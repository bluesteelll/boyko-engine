#!/usr/bin/env bash
# WINDOW 6 (2026-09-24): every timed pass of the quiet window, alone on an idle machine.
# Launch ONCE, from any cwd, with no agent working:   bash run_window.sh
#   --dry-run          print the schedule and the estimated minutes per item; run nothing timed
#   --dry-run --start HH:MM   the same, assuming the window starts at HH:MM
#   --resume           skip the passes listed in raw/passes_done.txt (after a stop or a cut)
#   --test             untimed rehearsal of the control flow (12 steps, one round, one pass, no idle wait) under test/
#   --blocks a,b       only these blocks (names in rows6.json)
#   --cutoff HH:MM     hard stop (default 12:45 local): no process starts whose estimated end passes it
# Outputs: raw/runs.jsonl (one record per process, appended at once), raw/<block>-p<n>/<seq>_.../ (stdout, stderr,
# run.csv, pose.bin), raw/window_log.txt, wait_log.txt, progress.txt ('<time> <item> <row> p<n> done' per pass),
# raw/passes_done.txt, raw/reduction.json + raw/tables.md (untimed, after the last pass), and WINDOW_DONE last:
#   line 1 'exit <n>' (0 complete, 2 cut at the cutoff, 3 stopped: idle rule / binaries / voids, 5 driver crashed),
#   line 2 the status ('complete' | 'cut at 12:45 after <block>-p<n> r<k> <row>#<bin>@W<w> ...' | 'STOP ...').
# Never builds, never touches a worktree; never sets RUSTFLAGS.
set -u
W6="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$W6" || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
PY="${PYTHON:-python}"

if [[ " $* " == *" --dry-run "* ]]; then
  exec "$PY" tools/window6_run.py "$@"
fi

TESTMODE=0
[[ " $* " == *" --test "* ]] && TESTMODE=1
if [ "$TESTMODE" = 1 ]; then
  RAWD="$W6/test/window"; DONE="$RAWD/WINDOW_DONE"; RFLAG="--test"
else
  RAWD="$W6/raw"; DONE="$W6/WINDOW_DONE"; RFLAG=""
fi
mkdir -p "$RAWD"
if [ -f "$DONE" ]; then mv "$DONE" "$DONE.prev-$(date +%s)"; fi
rm -f "$RAWD/driver_done.txt"

echo "$(date '+%F %T') run_window.sh start: $*" >> "$RAWD/shell_log.txt"
"$PY" tools/window6_run.py "$@" >> "$RAWD/driver_stdout.txt" 2>&1
PYRC=$?
echo "$(date '+%F %T') driver exit $PYRC" >> "$RAWD/shell_log.txt"

# The reduction is untimed and runs only after the last timed process.
"$PY" tools/reduce6.py $RFLAG > "$RAWD/reduce_stdout.txt" 2>&1
RDRC=$?

if [ -f "$RAWD/driver_done.txt" ]; then
  STATUS="$(cat "$RAWD/driver_done.txt")"
else
  STATUS="exit 5
driver crashed (python exit $PYRC); see $RAWD/driver_stdout.txt; last progress: $(tail -n 1 "$W6/progress.txt" 2>/dev/null)"
fi
printf '%s\nreduction exit %s (raw/tables.md)\n' "$STATUS" "$RDRC" > "$DONE"
echo "$(date '+%F %T') WINDOW_DONE written: $(head -n 2 "$DONE" | tr '\n' ' ')" >> "$RAWD/shell_log.txt"
exit 0
