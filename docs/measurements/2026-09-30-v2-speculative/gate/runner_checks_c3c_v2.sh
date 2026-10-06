#!/usr/bin/env bash
# V2 (C6): generated from l10/runner_checks_c3c.sh by make_v2_scripts.py - the fixtures and pinned
# hashes are the V2 recordings (D:/wt/merge/docs/measurements/2026-09-30-v2-speculative).
# L10 C3c runner checks (untimed): C3b's fourteen, plus the tree seam's armed rows (every step's
# structure checked, void steps must be 0) and its TreeDiag receipts.
# usage: runner_checks_c3c.sh <exe> <outdir>
set -u
EXE=$1
OUT=$2
L10=D:/wt/merge/docs/measurements/2026-09-30-v2-speculative/l10-reuse-on
mkdir -p "$OUT"
T="$OUT/runner_checks.tsv"
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
# C3b's fourteen.
run refuse_skip_default_cfg_sleeping_unset 2 --scene rest --sleep-skip sets --steps 10
run refuse_skip_sleeping_off 2 --scene rest --sleeping off --sleep-skip off --steps 10
run refuse_skip_cfg_a_unset 2 --scene jolt --gap 0.5 --cfg a --sleep-skip sets --steps 10
run refuse_skip_reference 2 --scene rest --solver reference --sleep-skip off --steps 10
run refuse_skip_bad 2 --scene rest --sleeping on --sleep-skip replay --steps 10
run accept_skip_sets 0 --scene rest --sleeping on --sleep-skip sets --steps 600 --workers 1 --expect-pose $L10/R-S_W1.pose
run accept_skip_off 0 --scene rest --sleeping on --sleep-skip off --steps 600 --workers 1 --expect-pose $L10/R-S_W1.pose
run armed_R-S_sets_W1 0 --scene rest --sleeping on --arm-profiler --steps 600 --workers 1 --expect-pose $L10/R-S_W1.pose
run armed_R-S_sets_W8 0 --scene rest --sleeping on --arm-profiler --steps 600 --workers 8 --expect-pose $L10/R-S_W8.pose
run armed_R-S_off_W1 0 --scene rest --sleeping on --sleep-skip off --arm-profiler --steps 600 --workers 1 --expect-pose $L10/R-S_W1.pose
run armed_J-Son_sets_W8 0 --scene jolt --gap 0.5 --cfg a --sleeping on --arm-profiler --steps 600 --workers 8 --expect-pose $L10/J-Son_W8.pose
run armed_J-A_sleep_off_W8 0 --scene jolt --gap 0.5 --cfg a --arm-profiler --steps 300 --workers 8
run armed_reference_s16 0 --scene s16 --solver reference --arm-profiler --steps 50 --workers 1
run self_check 0
# C3c: the tree seam, armed (the per-path structure on every step, PHYS_NP_PAIRS from the stream,
# PHYS_BP_PAIRS the logical count, q + m == N with the sleepers counted).
for w in 1 8; do
  run armed_R-S_tree_sets_W$w 0 --scene rest --sleeping on --broadphase tree --arm-profiler --steps 600 --workers $w --expect-pose $L10/R-S_W$w.pose
  run armed_R-S_tree_off_W$w 0 --scene rest --sleeping on --sleep-skip off --broadphase tree --arm-profiler --steps 600 --workers $w --expect-pose $L10/R-S_W$w.pose
  run armed_J-Son_tree_sets_W$w 0 --scene jolt --gap 0.5 --cfg a --sleeping on --broadphase tree --arm-profiler --steps 600 --workers $w --expect-pose $L10/J-Son_W$w.pose
  run armed_J-D-on_tree_sets_W$w 0 --scene jolt --gap 0.5 --cfg default --sleeping on --broadphase tree --arm-profiler --steps 600 --workers $w --expect-pose $L10/J-D-on_W$w.pose
  run armed_R-S_grid_sets_W$w 0 --scene rest --sleeping on --broadphase grid --arm-profiler --steps 600 --workers $w --expect-pose $L10/R-S_W$w.pose
  # The receipt with sleeping off on the tree: sleeper_rebuilds 0 (design 04 T(W) row, untimed).
  run J-A_sleep_off_tree_W$w 0 --scene jolt --gap 0.5 --cfg a --broadphase tree --steps 600 --workers $w
done
echo "checks $(wc -l < "$T"), pass $(grep -c 'PASS' "$T"), fail $(grep -c 'FAIL' "$T")"
