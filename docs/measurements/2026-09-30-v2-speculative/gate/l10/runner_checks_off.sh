#!/usr/bin/env bash
# The reuse-off arm of the lane's runner_checks_c3c.sh armed rows (untimed): each armed
# expect-pose row with --contact-reuse off against the pre-C4 fixtures; void steps must be 0.
# usage: runner_checks_off.sh <exe> <outdir>
set -u
EXE=$1
OUT=$2
L10OFF=D:/wt/merge/docs/measurements/2026-09-23-l10-sleeping/fixtures-reuse-off
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
echo "checks $(wc -l < "$T"), pass $(grep -c 'PASS' "$T"), fail $(grep -c 'FAIL' "$T")"
