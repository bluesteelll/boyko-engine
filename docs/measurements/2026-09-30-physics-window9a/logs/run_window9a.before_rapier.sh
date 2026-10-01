#!/usr/bin/env bash
# PHYSICS WINDOW 9a (2026-09-30; PARENT = trunk 3d9433ae, TIP = u/phys-tree-c4 50e31f1a): C4-AB (with the in-block Jolt
# 5.6 row) -> C4-G5 -> C4-BR -> C4-G4 -> C4-G4-kd (blocks run in the priority order of rows9a.json, which is this one; the
# lane's own order BR, G4, G4-kd, AB, G5 is `python -B tools/mkrows9a.py --lane-order`; reserved blocks from
# rows9a.extra.json, e.g. Rapier, sort in by their priority), alone on an idle machine. Window 8b's run_window8b.sh with
# the 9a names. Claim protocol: ruling 1 of RULINGS-2026-09-27-W8 (K 9 = 3 passes x 3 rounds, p0 and p2 reversed,
# placement receipts) + ruling 8 of RULINGS-2026-09-29-W8B (a dropped slot is re-run until its pass-cell has K = 3 clean
# slots or the re-run allowance of the pass is spent; a pass-cell with K < 3 does not gate). Launch ONCE, from any cwd,
# with no agent working:
#   bash run_window9a.sh --cutoff HH:MM [--resume]
#   --dry-run [--start HH:MM]  print the schedule and the estimated minutes per block; run nothing
#   --resume                   skip the passes listed in raw/passes_done.txt (after a cut, a STOP or exit 3)
#   --test                     untimed control-flow rehearsal under test/window (12 steps, one round, one pass, no idle wait)
#   --blocks a,b               only these blocks (C4-AB, C4-G5, C4-BR, C4-G4, C4-G4-kd, + any rows9a.extra.json block)
#   --cutoff HH:MM             hard stop: no process starts whose estimated end passes it
# HARD STOP FLAG: create win9a/STOP and the driver stops before its next process or pass (exit 2); --resume continues.
# Outputs: raw/runs.jsonl (one record per process), raw/<block>-p<n>_<runtag>/<seq>_.../ (stdout.txt, stderr.txt, run.csv |
# per_frame csv | criterion/ | pose.bin), raw/window_log.txt, wait_log.txt, progress.txt, raw/passes_done.txt,
# raw/counts.txt (untimed validity counts) and WINDOW_DONE last:
#   line 1 'exit <n>' (0 complete, 2 cut at the cutoff / STOP flag / blocks skipped, 3 stopped: idle never came, binaries
#   changed or repeated voids, 5 driver crashed), line 2 the status.
# Never builds, never touches a worktree, never sets RUSTFLAGS.
set -u
W9="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$W9" || exit 9
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
PY="${PYTHON:-python}"

if [[ " $* " == *" --dry-run "* ]]; then
  exec "$PY" -B tools/window9a_run.py "$@"
fi
if [[ " $* " == *" --test "* ]]; then RAWD="$W9/test/window"; DONE="$RAWD/WINDOW_DONE"; else RAWD="$W9/raw"; DONE="$W9/WINDOW_DONE"; fi
mkdir -p "$RAWD"
if [ -f "$DONE" ]; then mv "$DONE" "$DONE.prev-$(date +%s)"; fi
rm -f "$RAWD/driver_done.txt"

echo "$(date '+%F %T') run_window9a.sh start: $*" >> "$RAWD/shell_log.txt"
"$PY" -B tools/window9a_run.py "$@" >> "$RAWD/driver_stdout.txt" 2>&1
PYRC=$?
echo "$(date '+%F %T') driver exit $PYRC" >> "$RAWD/shell_log.txt"

# Untimed, after the last timed process: validity counts only (no statistic).
"$PY" -B tools/counts9a.py "$RAWD" > "$RAWD/counts.txt" 2>&1
CRC=$?

if [ -f "$RAWD/driver_done.txt" ]; then
  STATUS="$(cat "$RAWD/driver_done.txt")"
else
  STATUS="exit 5
driver crashed (python exit $PYRC); see $RAWD/driver_stdout.txt; last progress: $(tail -n 1 "$W9/progress.txt" 2>/dev/null)"
fi
printf '%s\ncounts exit %s (%s/counts.txt)\n' "$STATUS" "$CRC" "${RAWD#"$W9"/}" > "$DONE"
echo "$(date '+%F %T') WINDOW_DONE written: $(head -n 2 "$DONE" | tr '\n' ' ')" >> "$RAWD/shell_log.txt"
exit 0
