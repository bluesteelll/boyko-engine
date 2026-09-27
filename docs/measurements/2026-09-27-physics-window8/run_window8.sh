#!/usr/bin/env bash
# PHYSICS WINDOW 8 (2026-09-27): W8S-A -> W8S-R -> micro -> DM1 A/B -> P-jolt56-prof, alone on an idle machine.
# Window 7 wave 2's launcher (run_window.sh), extended. Launch ONCE, from any cwd, with no agent working:
#   bash run_window8.sh --cutoff HH:MM [--resume]
#   --dry-run [--start HH:MM]  print the schedule and the estimated minutes per block; run nothing
#   --resume                   skip the passes listed in raw/passes_done.txt (after a cut, a STOP or exit 3)
#   --test                     untimed control-flow rehearsal under test/window (12 steps, one round, one pass, no idle wait)
#   --blocks a,b               only these blocks (W8S-A, W8S-R, micro, DM1, P-jolt56-prof)
#   --cutoff HH:MM             hard stop: no process starts whose estimated end passes it
# HARD STOP FLAG: create win8/STOP and the driver stops before its next process or pass (exit 2); --resume continues.
# Outputs: raw/runs.jsonl (one record per process), raw/<block>-p<n>_<runtag>/<seq>_.../ (stdout.txt, stderr.txt, run.csv |
# per_frame csv + profile dumps + wfb csv | artifact.toml), raw/window_log.txt, wait_log.txt, progress.txt, raw/passes_done.txt,
# raw/counts.txt (untimed validity counts) and WINDOW_DONE last:
#   line 1 'exit <n>' (0 complete, 2 cut at the cutoff / STOP flag / blocks skipped, 3 stopped: idle never came, binaries
#   changed or repeated voids, 5 driver crashed), line 2 the status.
# Never builds, never touches a worktree, never sets RUSTFLAGS. One windowed (DM1) process at a time; a DM1 process
# alive past 300 s is terminated by its own handle.
set -u
W8="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$W8" || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
PY="${PYTHON:-python}"

if [[ " $* " == *" --dry-run "* ]]; then
  exec "$PY" -B tools/window8_run.py "$@"
fi
if [[ " $* " == *" --test "* ]]; then RAWD="$W8/test/window"; DONE="$RAWD/WINDOW_DONE"; else RAWD="$W8/raw"; DONE="$W8/WINDOW_DONE"; fi
mkdir -p "$RAWD"
if [ -f "$DONE" ]; then mv "$DONE" "$DONE.prev-$(date +%s)"; fi
rm -f "$RAWD/driver_done.txt"

echo "$(date '+%F %T') run_window8.sh start: $*" >> "$RAWD/shell_log.txt"
"$PY" -B tools/window8_run.py "$@" >> "$RAWD/driver_stdout.txt" 2>&1
PYRC=$?
echo "$(date '+%F %T') driver exit $PYRC" >> "$RAWD/shell_log.txt"

# Untimed, after the last timed process: validity counts only (no statistic).
"$PY" -B tools/counts8.py "$RAWD" > "$RAWD/counts.txt" 2>&1
CRC=$?

if [ -f "$RAWD/driver_done.txt" ]; then
  STATUS="$(cat "$RAWD/driver_done.txt")"
else
  STATUS="exit 5
driver crashed (python exit $PYRC); see $RAWD/driver_stdout.txt; last progress: $(tail -n 1 "$W8/progress.txt" 2>/dev/null)"
fi
printf '%s\ncounts exit %s (%s/counts.txt)\n' "$STATUS" "$CRC" "${RAWD#"$W8"/}" > "$DONE"
echo "$(date '+%F %T') WINDOW_DONE written: $(head -n 2 "$DONE" | tr '\n' ' ')" >> "$RAWD/shell_log.txt"
exit 0
