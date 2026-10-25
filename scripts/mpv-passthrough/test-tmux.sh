#!/usr/bin/env bash
# Headless tmux test of mpv --vo=kitty passthrough.
#
# usage: test-tmux.sh <mpv-bin> <url> [expect] [duration-sec]
#   expect: pass (patched binary) | fail (stock binary control)
#
# The mpv invocation is written to a run-script (one quoting layer)
# and is the tmux session start-command. The pane starts in / so a
# direnv-managed CWD cannot load a devshell menu. The pane stdout
# stream is captured with tmux pipe-pane: the tmux client pty
# swallows the in-band DCS envelopes, so a captured client attach
# would not show them.
set -u
MPV=${1:?mpv binary}
URL=${2:?stream url}
EXPECT=${3:-pass}
DUR=${4:-12}
HERE=$(cd "$(dirname "$0")" && pwd)

W=$(mktemp -d /tmp/mpv-pt.XXXXXX)
cleanup() { tmux -S "$W/sock" kill-server 2>/dev/null; }
trap cleanup EXIT

printf 'set -g allow-passthrough on\n' > "$W/tmux.conf"

cat > "$W/run.sh" <<EOF
exec $MPV --no-config \
  --vo=kitty \
  --vo-kitty-auto-multiplexer-passthrough=yes \
  --vo-kitty-use-shm=no \
  --vo-kitty-alt-screen=no \
  --vo-kitty-config-clear=no \
  --vo-kitty-top=0 --vo-kitty-left=0 \
  --vo-kitty-width=96 --vo-kitty-height=54 \
  --really-quiet --msg-level=all=no \
  "$URL"
EOF
chmod +x "$W/run.sh"

# The keepalive holds the pane after mpv exits so the pipe-pane
# stop and the pane capture can run.
cat > "$W/keep.sh" <<EOF
bash "$W/run.sh"
echo "rc=\$?" > "$W/rc.txt"
sleep 30
EOF

tmux -S "$W/sock" -f "$W/tmux.conf" new-session -d -s vt -x 100 -y 30 \
  -c / "bash -c \"bash $W/keep.sh\""
sleep 3
tmux -S "$W/sock" pipe-pane -o -t vt:0 "cat > $W/pane-stdout.raw"
sleep "$DUR"
tmux -S "$W/sock" pipe-pane -t vt:0 ""
tmux -S "$W/sock" capture-pane -p -t vt:0 > "$W/pane.txt" 2>/dev/null

python3 "$HERE/analyze-client.py" "$W" "$EXPECT"
RC=$?
echo "mpv rc file: $(cat "$W/rc.txt" 2>/dev/null || echo none)"
exit "$RC"
