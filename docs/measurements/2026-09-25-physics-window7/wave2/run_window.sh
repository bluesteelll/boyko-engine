#!/usr/bin/env bash
# WINDOW 7 WAVE 2 (2026-09-25): every timed pass of Q1-Q4, alone on an idle machine. Window 7's driver, extended.
# Launch ONCE, from any cwd, with no agent working:   bash run_window.sh
#   --dry-run          print the schedule, the estimated minutes per block and per item, and the launch-context
#                      lengths of the padded rows; run nothing timed
#   --dry-run --start HH:MM   the same, assuming the window starts at HH:MM
#   --resume           skip the passes listed in raw/passes_done.txt (after a stop or a cut); restores the canary
#                      reference from raw/runs.jsonl
#   --test             untimed rehearsal of the control flow (12 steps, one round, one pass, no idle wait) under test/
#   --test-real        untimed rehearsal with REAL steps, windows and pose checks (one round, one pass) under test/real/
#   --blocks a,b       only these blocks (names in rows7b.json: Q1C, Q2C, Q3, Q4, Q2D, Q1D)
#   --cutoff HH:MM     hard stop (default 11:45 local): no process starts whose estimated end passes it
# HARD STOP FLAG: create the file win7b/STOP (any content) and the driver stops before its next process or pass
# (WINDOW_DONE: exit 2 'cut by the STOP flag ...'); --resume continues after the flag is removed.
# PRIORITY: Q1 1, Q2 2, Q3 3, Q4 4; a block that would not leave room for the later higher-priority blocks before the
# cutoff is SKIPPED (progress line 'SKIPPED for priority'; WINDOW_DONE exit 2 'complete except ...').
# Block order (rows7b.json): Q1C, Q2C, Q3, Q4, Q2D, Q1D.
# Outputs: raw/runs.jsonl (one record per process, appended at once), raw/<block>-p<n>_<runtag>/<seq>_.../ (stdout,
# stderr, run.csv or per_frame csv or criterion output, pose.bin), raw/window_log.txt, wait_log.txt, progress.txt
# ('<time> <item> <row> p<n> done' per pass), raw/passes_done.txt, raw/reduction.json + raw/tables.md (untimed,
# after the last pass), and WINDOW_DONE last:
#   line 1 'exit <n>' (0 complete, 2 cut at the hard stop / by the STOP flag / blocks skipped for priority,
#   3 stopped: idle never came / binaries changed / voids, 5 driver crashed), line 2 the status.
# Never builds, never touches a worktree; never sets RUSTFLAGS.
set -u
W7="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$W7" || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
PY="${PYTHON:-python}"

if [[ " $* " == *" --dry-run "* ]]; then
  exec "$PY" -B tools/window7b_run.py "$@"
fi

TESTMODE=0
RFLAG=""
if [[ " $* " == *" --test-real "* ]]; then
  TESTMODE=1; RAWD="$W7/test/real"; RFLAG="--test-real"
elif [[ " $* " == *" --test "* ]]; then
  TESTMODE=1; RAWD="$W7/test/window"; RFLAG="--test"
else
  RAWD="$W7/raw"
fi
if [ "$TESTMODE" = 1 ]; then DONE="$RAWD/WINDOW_DONE"; else DONE="$W7/WINDOW_DONE"; fi
mkdir -p "$RAWD"
if [ -f "$DONE" ]; then mv "$DONE" "$DONE.prev-$(date +%s)"; fi
rm -f "$RAWD/driver_done.txt"

echo "$(date '+%F %T') run_window.sh start: $*" >> "$RAWD/shell_log.txt"
"$PY" -B tools/window7b_run.py "$@" >> "$RAWD/driver_stdout.txt" 2>&1
PYRC=$?
echo "$(date '+%F %T') driver exit $PYRC" >> "$RAWD/shell_log.txt"

# The reduction is untimed and runs only after the last timed process.
"$PY" -B tools/reduce7b.py $RFLAG > "$RAWD/reduce_stdout.txt" 2>&1
RDRC=$?

if [ -f "$RAWD/driver_done.txt" ]; then
  STATUS="$(cat "$RAWD/driver_done.txt")"
else
  STATUS="exit 5
driver crashed (python exit $PYRC); see $RAWD/driver_stdout.txt; last progress: $(tail -n 1 "$W7/progress.txt" 2>/dev/null)"
fi
printf '%s\nreduction exit %s (%s/tables.md)\n' "$STATUS" "$RDRC" "${RAWD#"$W7"/}" > "$DONE"
echo "$(date '+%F %T') WINDOW_DONE written: $(head -n 2 "$DONE" | tr '\n' ' ')" >> "$RAWD/shell_log.txt"
exit 0
