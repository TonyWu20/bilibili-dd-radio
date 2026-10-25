#!/usr/bin/env bash
# Block on the mpv patch build, poke the session once on exit.
# No polling. No liveness check.
set -u
SESSION="video-in-kitty"
PROJ=/home/tony/programming/bilibili-dd-radio
SESS_ROOT="$PROJ/sessions"
LOG=/home/tony/programming/mpv-kitty-passthrough/build.log
bash /home/tony/programming/mpv-kitty-passthrough/build.sh > "$LOG" 2>&1 &
TASK_PID=$!
echo "$TASK_PID" > /home/tony/programming/mpv-kitty-passthrough/build.task.pid
wait "$TASK_PID"
RC=$?
if [ "$RC" -eq 0 ]; then
  rushi run "$SESSION" "mpv patch build finished cleanly. Binary at /home/tony/programming/mpv-kitty-passthrough/src/build/mpv. Run the tmux passthrough test." \
    --sessions-root "$SESS_ROOT" --cwd "$PROJ"
else
  rushi run "$SESSION" "mpv patch build failed (exit $RC). Last log lines: $(tail -12 "$LOG" | tr '\n' ' ')" \
    --sessions-root "$SESS_ROOT" --cwd "$PROJ"
fi
