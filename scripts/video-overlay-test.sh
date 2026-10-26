#!/usr/bin/env bash
# Headless test of the bdr per-room video overlay.
#
# usage: video-overlay-test.sh <bdr-bin> <mpv-bin> <room>
#
# Runs bdr inside a private tmux session with a 120x30 pane.
# tmux owns the pty and its size, which a pty made by this
# script cannot hold. The test drives the TUI with send-keys
# and checks the mpv process table, which records each respawn:
#
#   1. v in the grid view: an mpv runs the kitty driver with
#      the flags of the focused cell.
#   2. d, the danmaku view: the same mpv process survives. The
#      focused room keeps its overlay, so no respawn happens
#      when the position is unchanged.
#   3. v again: no mpv runs the kitty driver.
#
# The frame bytes do not pass through tmux: the tmux parser
# consumes the DCS. Frame flow is covered by the standalone
# mpv test and the Mac smoke test. The two-room position-change
# path also needs a second live room and is covered there.
set -u
BDR=${1:?bdr binary}
MPV=${2:?mpv binary}
ROOM=${3:?room id}
MPVDIR=$(dirname "$MPV")
TMP=$(mktemp -d /tmp/bdr-video-test.XXXXXX)
CAP="$TMP/cap.log"
touch "$CAP"
S=bdrtest-$$
cleanup() {
    tmux -L "$S" kill-server 2>/dev/null
    pkill -f "bdr-room-[0-9]+.sock" 2>/dev/null
    rm -rf "$TMP"
}
trap cleanup EXIT
# Fixed delay. tail -f on /dev/null never produces output, so
# the timeout bounds the wait.
delay() {
    timeout "$1" tail -f /dev/null 2>/dev/null || true
}

# A killed earlier run can leave bdr mpv children behind. The
# bracket keeps the pattern away from our own command line.
pkill -f "bdr-room-[0-9]+.sock" 2>/dev/null

tmux -L "$S" new-session -d -s vt -x 120 -y 30 \
    "exec env BDR_TRACE=1 PATH=$MPVDIR:$PATH $BDR room $ROOM"
# Start the pipe after the pane exists, so nothing is missed.
tmux -L "$S" pipe-pane -t vt -o "cat >> $CAP"

# Wait for the audio pipeline. The trace line marks it.
deadline=$((SECONDS + 90))
until rg -q "mpv start: Ok" "$CAP" 2>/dev/null; do
    [ "$SECONDS" -ge "$deadline" ] && {
        echo "FAIL: audio pipeline did not start"
        exit 1
    }
    delay 1
done
delay 2

# Grid view, video on. A single room fills the pane, so the
# cell position is top 1, left 1.
tmux -L "$S" send-keys -t vt v
delay 12
GRID_PID=""
if ps -eo pid,args | rg -- "--vo=kitt[y]" > /dev/null; then
    GRID_PID=$(ps -eo pid,args | rg -- "--vo=kitt[y]" | awk '{print $1}' | head -1)
fi
# Danmaku view: the overlay keeps the same position. The same
# mpv must survive.
tmux -L "$S" send-keys -t vt d
delay 10
DM_PID=""
if ps -eo pid,args | rg -- "--vo=kitt[y]" > /dev/null; then
    DM_PID=$(ps -eo pid,args | rg -- "--vo=kitt[y]" | awk '{print $1}' | head -1)
fi
# Video off: no kitty mpv at all.
tmux -L "$S" send-keys -t vt v
delay 8
KITTY_LEFT=0
ps -eo args | rg -q -- "--vo=kitt[y]" && KITTY_LEFT=1
# Quit.
tmux -L "$S" send-keys -t vt Escape
delay 1
tmux -L "$S" send-keys -t vt q
delay 2

echo "kitty mpv pid in grid window:   ${GRID_PID:-none}"
echo "kitty mpv pid in danmaku window: ${DM_PID:-none}"
echo "kitty mpv still alive after off: $KITTY_LEFT"
ok=1
[ -n "$GRID_PID" ] || { echo "RESULT: FAIL"; exit 1; }
[ "$GRID_PID" = "$DM_PID" ] || { echo "RESULT: FAIL (danmaku respawned the overlay)"; exit 1; }
[ "$KITTY_LEFT" -eq 0 ] || { echo "RESULT: FAIL"; exit 1; }
echo "RESULT: PASS"
exit 0
