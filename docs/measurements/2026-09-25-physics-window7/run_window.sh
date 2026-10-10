#!/usr/bin/env bash
# WINDOW 7 (2026-09-25): every timed pass of the quiet window, alone on an idle machine. Window 6's driver, extended.
# Launch ONCE, from any cwd, with no agent working:   bash run_window.sh
#   --dry-run          print the schedule and the estimated minutes per block; run nothing timed
#   --dry-run --start HH:MM   the same, assuming the window starts at HH:MM
#   --resume           skip the passes listed in raw/passes_done.txt (after a stop or a cut)
#   --test             untimed rehearsal of the control flow (12 steps, one round, one pass, no idle wait) under test/
#   --blocks a,b       only these blocks (names in rows7.json)
#   --cutoff HH:MM     hard stop (default 04:15 local): no process starts whose estimated end passes it
# Block order (rows7.json): P1A-jolt, P2-L9GTW, P3-G9JA, P4-trkspans, P5-joltprof, P1B-jolt.
# Outputs: raw/runs.jsonl (one record per process, appended at once), raw/<block>-p<n>_<runtag>/<seq>_.../ (stdout,
# stderr, run.csv or per_frame csv, pose.bin, profile dumps), raw/window_log.txt, wait_log.txt, progress.txt
# ('<time> <item> <row> p<n> done' per pass), raw/passes_done.txt, raw/reduction.json + raw/tables.md (untimed,
# after the last pass), and WINDOW_DONE last:
#   line 1 'exit <n>' (0 complete, 2 cut at the hard stop, 3 stopped: idle never came / binaries changed / voids,
#   5 driver crashed), line 2 the status.
# Never builds, never touches a worktree; never sets RUSTFLAGS.
set -u
W7="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$W7" || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
PY="${PYTHON:-python}"

if [[ " $* " == *" --dry-run "* ]]; then
  exec "$PY" -B tools/window7_run.py "$@"
fi

TESTMODE=0
[[ " $* " == *" --test "* ]] && TESTMODE=1
if [ "$TESTMODE" = 1 ]; then
  RAWD="$W7/test/window"; DONE="$RAWD/WINDOW_DONE"; RFLAG="--test"
else
  RAWD="$W7/raw"; DONE="$W7/WINDOW_DONE"; RFLAG=""
fi
mkdir -p "$RAWD"
if [ -f "$DONE" ]; then mv "$DONE" "$DONE.prev-$(date +%s)"; fi
rm -f "$RAWD/driver_done.txt"

echo "$(date '+%F %T') run_window.sh start: $*" >> "$RAWD/shell_log.txt"
"$PY" -B tools/window7_run.py "$@" >> "$RAWD/driver_stdout.txt" 2>&1
PYRC=$?
echo "$(date '+%F %T') driver exit $PYRC" >> "$RAWD/shell_log.txt"

# The reduction is untimed and runs only after the last timed process.
"$PY" -B tools/reduce7.py $RFLAG > "$RAWD/reduce_stdout.txt" 2>&1
RDRC=$?

if [ -f "$RAWD/driver_done.txt" ]; then
  STATUS="$(cat "$RAWD/driver_done.txt")"
else
  STATUS="exit 5
driver crashed (python exit $PYRC); see $RAWD/driver_stdout.txt; last progress: $(tail -n 1 "$W7/progress.txt" 2>/dev/null)"
fi
printf '%s\nreduction exit %s (raw/tables.md)\n' "$STATUS" "$RDRC" > "$DONE"
echo "$(date '+%F %T') WINDOW_DONE written: $(head -n 2 "$DONE" | tr '\n' ' ')" >> "$RAWD/shell_log.txt"
exit 0
