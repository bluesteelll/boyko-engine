#!/usr/bin/env bash
# The reuse-off arm of the lane's runner_checks_c3c.sh armed rows (untimed): each armed
# expect-pose row with --contact-reuse off against the pre-C4 fixtures; void steps must be 0.
# usage: runner_checks_off.sh [--root DIR] <exe> <outdir>   (fixtures from ROOT: the gate-root block)
# Exit (docs/physics/perf-campaign/GATE-KIT.md): 0 = every check PASS and all EXPECT_CHECKS ran;
# 1 = red; 2 = could not run (usage, no exe, a fixture missing), decided before the first check.
set -u
EXPECT_CHECKS=17 # 7 + 2 W x 5
ME=$(basename "$0")
# gate-root BEGIN (identical text in every gate script that reads fixtures; GATE-KIT.md, "Root rule")
# ROOT = DIR when `--root DIR` comes first, else the git toplevel of this script's own checkout
# (GIT_DIR and GIT_WORK_TREE ignored); in the mixed form D:/..., which the native runner reads.
if [ "${1:-}" = --root ]; then
  if [ $# -lt 2 ]; then echo "$(basename "$0"): --root needs a directory" >&2; exit 2; fi
  ROOT=$2
  shift 2
else
  HERE=$(cd "$(dirname "$0")" && pwd)
  if command -v cygpath > /dev/null 2>&1; then HERE=$(cygpath -m "$HERE"); fi
  if ! ROOT=$(unset GIT_DIR GIT_WORK_TREE; git -C "$HERE" rev-parse --show-toplevel 2> /dev/null); then
    echo "$(basename "$0"): cannot resolve the repo root from $0; pass --root DIR" >&2; exit 2
  fi
fi
if command -v cygpath > /dev/null 2>&1; then ROOT=$(cygpath -m "$ROOT" 2> /dev/null); fi
case $ROOT in *' '*) echo "$(basename "$0"): the root '$ROOT' contains a space" >&2; exit 2 ;; esac
if [ ! -d "$ROOT/docs/measurements" ]; then echo "$(basename "$0"): '$ROOT' is not a boyko-engine checkout (no docs/measurements)" >&2; exit 2; fi
# gate-root END
if [ $# -ne 2 ]; then echo "usage: $ME [--root DIR] <exe> <outdir>" >&2; exit 2; fi
EXE=$1
OUT=$2
L10OFF=$ROOT/docs/measurements/2026-09-23-l10-sleeping/fixtures-reuse-off
if [ ! -f "$EXE" ]; then echo "$ME: no runner exe at $EXE" >&2; exit 2; fi
missing=0
for w in 1 8; do for n in R-S J-Son J-D-on; do
  [ -f "$L10OFF/${n}_W$w.pose" ] || { echo "$ME: missing fixture $L10OFF/${n}_W$w.pose" >&2; missing=1; }
done; done
if [ "$missing" != 0 ]; then exit 2; fi
mkdir -p "$OUT"
T="$OUT/runner_checks_off.tsv"
: > "$T"
run() { # label want-exit flags...
  local label=$1 want=$2; shift 2
  "$EXE" "$@" > "$OUT/$label.out" 2>&1
  local rc=$?
  local h; h=$(grep -o 'pose_hash [0-9a-fx]*' "$OUT/$label.out" | awk '{print $2}')
  local verdict=PASS
  if [ "$rc" != "$want" ]; then verdict=FAIL; fi
  if [ "$want" = 0 ] && grep -q '^profile:' "$OUT/$label.out" && ! grep -q 'void steps 0,' "$OUT/$label.out"; then verdict=FAIL; fi
  printf '%s\texit=%s\twant=%s\t%s\t%s\t%s\n' "$label" "$rc" "$want" "${h:--}" "$verdict" "$*" >> "$T"
}
RU="--contact-reuse off"
run off_accept_skip_sets 0 --scene rest --sleeping on --sleep-skip sets $RU --steps 600 --workers 1 --expect-pose $L10OFF/R-S_W1.pose
run off_accept_skip_off 0 --scene rest --sleeping on --sleep-skip off $RU --steps 600 --workers 1 --expect-pose $L10OFF/R-S_W1.pose
run off_armed_R-S_sets_W1 0 --scene rest --sleeping on $RU --arm-profiler --steps 600 --workers 1 --expect-pose $L10OFF/R-S_W1.pose
run off_armed_R-S_sets_W8 0 --scene rest --sleeping on $RU --arm-profiler --steps 600 --workers 8 --expect-pose $L10OFF/R-S_W8.pose
run off_armed_R-S_off_W1 0 --scene rest --sleeping on --sleep-skip off $RU --arm-profiler --steps 600 --workers 1 --expect-pose $L10OFF/R-S_W1.pose
run off_armed_J-Son_sets_W8 0 --scene jolt --gap 0.5 --cfg a --sleeping on $RU --arm-profiler --steps 600 --workers 8 --expect-pose $L10OFF/J-Son_W8.pose
run off_armed_J-A_sleep_off_W8 0 --scene jolt --gap 0.5 --cfg a $RU --arm-profiler --steps 300 --workers 8
for w in 1 8; do
  run off_armed_R-S_tree_sets_W$w 0 --scene rest --sleeping on --broadphase tree $RU --arm-profiler --steps 600 --workers $w --expect-pose $L10OFF/R-S_W$w.pose
  run off_armed_R-S_tree_off_W$w 0 --scene rest --sleeping on --sleep-skip off --broadphase tree $RU --arm-profiler --steps 600 --workers $w --expect-pose $L10OFF/R-S_W$w.pose
  run off_armed_J-Son_tree_sets_W$w 0 --scene jolt --gap 0.5 --cfg a --sleeping on --broadphase tree $RU --arm-profiler --steps 600 --workers $w --expect-pose $L10OFF/J-Son_W$w.pose
  run off_armed_J-D-on_tree_sets_W$w 0 --scene jolt --gap 0.5 --cfg default --sleeping on --broadphase tree $RU --arm-profiler --steps 600 --workers $w --expect-pose $L10OFF/J-D-on_W$w.pose
  run off_armed_R-S_grid_sets_W$w 0 --scene rest --sleeping on --broadphase grid $RU --arm-profiler --steps 600 --workers $w --expect-pose $L10OFF/R-S_W$w.pose
done
checks=$(wc -l < "$T"); pass=$(grep -c 'PASS' "$T"); fail=$(grep -c 'FAIL' "$T")
echo "checks $checks, pass $pass, fail $fail"
[ "$checks" = "$EXPECT_CHECKS" ] && [ "$pass" = "$EXPECT_CHECKS" ] && [ "$fail" = 0 ] || exit 1
