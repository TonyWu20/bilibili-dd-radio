#!/usr/bin/env bash
# video-perf.sh: performance gate for N concurrent mpv kitty-protocol
# video overlays. See docs/decisions.md, session 8.
#
# Run on the machine that will carry the overlays:
#   headless host:  scripts/video-perf.sh -q 6 280446 6557640
#   terminal host:  scripts/video-perf.sh -d 60 6 280446 6557640
#   (a terminal host must speak the kitty graphics protocol:
#    ghostty or kitty)
#
# It reports per stream: the active hardware decoder
# (hwdec-current), dropped frames, and CPU. It answers two questions:
#   1. Does GPU hardware decode engage?
#   2. Does the machine hold N concurrent overlays?
#
# Options:
#   -d seconds   measurement window, default 60
#   -g gap-rows  vertical offset per overlay, in terminal rows,
#                default 0 (all overlays start at the top-left)
#   -q           headless: mpv stdout goes to /dev/null
#   room ...     live room id, one per overlay; default 6
#
# Extra mpv flags for experiments:
#   VIDEO_PERF_MPV_ARGS="--hwdec=cuda" scripts/video-perf.sh -q 6
#
# Inside tmux the images reach the outer terminal only when
# ~/.tmux.conf has `set -g allow-passthrough on` and either
# VIDEO_PERF_MPV_ARGS="--vo-kitty-auto-multiplexer-passthrough=yes"
# is set, or the whole session is wrapped in kitty-tmux.
set -euo pipefail

cd "$(dirname "$0")/.."

DURATION=60
GAP=0
QUIET=0
ROOMS=()

while [ $# -gt 0 ]; do
  case "$1" in
    -d) DURATION="$2"; shift 2 ;;
    -g) GAP="$2"; shift 2 ;;
    -q) QUIET=1; shift ;;
    *) ROOMS+=("$1"); shift ;;
  esac
done
[ ${#ROOMS[@]} -eq 0 ] && ROOMS=(6)

command -v mpv >/dev/null || {
  echo "mpv not on PATH; enter the nix devshell first"
  exit 1
}
cargo build -p bili-api --example playurl --quiet

WORK="$(mktemp -d)"
PIDS=()
cleanup() {
  [ ${#PIDS[@]} -gt 0 ] && kill "${PIDS[@]}" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

gpu_snap() {
  nvidia-smi --query-gpu=utilization.gpu,memory.used \
    --format=csv,noheader,nounits 2>/dev/null || echo "no nvidia-smi"
}
GPU_BEFORE="$(gpu_snap)"

echo "starting ${#ROOMS[@]} overlays (headless=${QUIET}), ${DURATION}s window"
for i in "${!ROOMS[@]}"; do
  url="$(./target/debug/examples/playurl "${ROOMS[$i]}")" || {
    echo "room ${ROOMS[$i]}: no stream url (offline?)"
    exit 1
  }
  sock="$WORK/s$i.sock"
  log="$WORK/s$i.log"
  # shellcheck disable=SC2086
  if [ "$QUIET" -eq 1 ]; then
    mpv --vo=kitty --vo-kitty-use-shm=no \
        --profile=low-latency --hwdec=auto-safe --vd-lavc-fast \
        --vo-kitty-width=720 --vo-kitty-height=405 \
        --vo-kitty-top="$((i * GAP))" --vo-kitty-left=0 \
        --vo-kitty-alt-screen=no --vo-kitty-config-clear=no \
        --msg-level=all=no --term-status-msg="" --term-osd-bar=no \
        --really-quiet --input-ipc-server="$sock" \
        --referrer=https://live.bilibili.com/ \
        ${VIDEO_PERF_MPV_ARGS:-} "$url" \
        </dev/null >/dev/null 2>"$log" &
  else
    mpv --vo=kitty --vo-kitty-use-shm=no \
        --profile=low-latency --hwdec=auto-safe --vd-lavc-fast \
        --vo-kitty-width=720 --vo-kitty-height=405 \
        --vo-kitty-top="$((i * GAP))" --vo-kitty-left=0 \
        --vo-kitty-alt-screen=no --vo-kitty-config-clear=no \
        --msg-level=all=no --term-status-msg="" --term-osd-bar=no \
        --really-quiet --input-ipc-server="$sock" \
        --referrer=https://live.bilibili.com/ \
        ${VIDEO_PERF_MPV_ARGS:-} "$url" \
        </dev/null 2>"$log" &
  fi
  PIDS+=("$!")
  echo "  room ${ROOMS[$i]}: pid ${PIDS[$i]}, socket s$i"
done

# Settle: socket open, stream buffered, hwdec negotiated.
sleep 5
echo "measuring ${DURATION}s ..."
sleep "$DURATION"

mpv_query() {
  python3 - "$1" "$2" <<'PY'
import json, socket, sys
sock_path, prop = sys.argv[1], sys.argv[2]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(3)
try:
    s.connect(sock_path)
    req = json.dumps({"command": ["get_property", prop], "request_id": 1})
    s.sendall(req.encode() + b"\n")
    d = json.loads(s.recv(65536).decode())
    print(d.get("data", d.get("error")))
except Exception as e:
    print(f"<unavailable: {e}>")
PY
}

echo
printf '%-10s %-8s %-16s %-8s %-6s\n' room pid hwdec drops cpu%
TOTAL_CPU=0
for i in "${!ROOMS[@]}"; do
  hwdec="$(mpv_query "$WORK/s$i.sock" hwdec-current)"
  drops="$(mpv_query "$WORK/s$i.sock" frame-drop-count)"
  pos="$(mpv_query "$WORK/s$i.sock" time-pos)"
  cpu="$(ps -o %cpu= -p "${PIDS[$i]}" | tr -d ' ')"
  TOTAL_CPU="$(python3 -c "print(f'{$TOTAL_CPU + float('$cpu'):6.1f}')" )"
  printf '%-10s %-8s %-16s %-8s %-6s\n' \
    "${ROOMS[$i]}" "${PIDS[$i]}" "$hwdec" "$drops" "$cpu"
  echo "  room ${ROOMS[$i]}: time-pos ${pos}s"
done
echo "total cpu: ${TOTAL_CPU}%"
echo "gpu (util gpu%, mem MiB) before: $(echo $GPU_BEFORE)"
echo "gpu (util gpu%, mem MiB) after:  $(gpu_snap)"
if [ "$QUIET" -eq 0 ]; then
  echo
  echo "Count the columns and rows one image occupies on screen."
  echo "That count is the pixel-to-cell mapping of this font."
fi
echo "Paste this summary into docs/decisions.md (session 8)."
