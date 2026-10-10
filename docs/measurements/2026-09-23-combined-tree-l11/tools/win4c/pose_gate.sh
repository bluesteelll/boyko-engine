#!/usr/bin/env bash
# win4c pose gate on the merged tip. Usage: pose_gate.sh <runner.exe> <outdir>
# Untimed. Every row is the window-4 recipe (docs/measurements/2026-09-22-broadphase-tree/rows.json):
#   J    = --scene jolt --gap 0.5 --cfg default|as --broadphase allpairs|tree, 500 steps, W 1/8
#   rest = --scene rest --cfg default --broadphase allpairs|tree, 500 steps, W 1/8
#   s16  = --scene s16  --cfg default --broadphase allpairs|tree, 500 steps, W 1
#   red  = the tree twin at 501 steps against the 500-step pose: must exit 4
set -u
EXE="$1"; OUT="$2"; mkdir -p "$OUT"
run() { # name, expect_rc, args...
  local name="$1"; local want="$2"; shift 2
  "$EXE" "$@" > "$OUT/$name.out" 2> "$OUT/$name.err"; local rc=$?
  echo "$rc" > "$OUT/$name.rc"
  local hash; hash=$(grep -oE '"pose_hash":"[^"]+"' "$OUT/$name.out" | head -1)
  local ep;   ep=$(grep -oE '"expect_pose":"[^"]+"' "$OUT/$name.out" | head -1)
  local wk;   wk=$(grep -oE '"workers":[0-9]+' "$OUT/$name.out" | head -1)
  local st;   st=$(grep -oE '"steps":[0-9]+' "$OUT/$name.out" | head -1)
  local bp;   bp=$(grep -oE '"broadphase":"[^"]+"' "$OUT/$name.out" | head -1)
  local tree; tree=$(grep -oE '"broadphase_tree":\{[^}]*\}' "$OUT/$name.out" | head -1)
  local verdict="OK"; [ "$rc" = "$want" ] || verdict="RC-MISMATCH(want $want)"
  echo "$name: exit $rc [$verdict] $wk $st $bp $hash $ep $tree"
}
P="$OUT/pose"
# ── J, --cfg default ──
run j_def_allpairs_w1 0 --scene jolt --gap 0.5 --cfg default --broadphase allpairs --workers 1 --steps 500 --pose-out "$P"_j_default.bin --label j_def_allpairs_w1
run j_def_tree_w1     0 --scene jolt --gap 0.5 --cfg default --broadphase tree     --workers 1 --steps 500 --expect-pose "$P"_j_default.bin --label j_def_tree_w1
run j_def_allpairs_w8 0 --scene jolt --gap 0.5 --cfg default --broadphase allpairs --workers 8 --steps 500 --expect-pose "$P"_j_default.bin --label j_def_allpairs_w8
run j_def_tree_w8     0 --scene jolt --gap 0.5 --cfg default --broadphase tree     --workers 8 --steps 500 --expect-pose "$P"_j_default.bin --label j_def_tree_w8
run j_def_tree_red501 4 --scene jolt --gap 0.5 --cfg default --broadphase tree     --workers 1 --steps 501 --expect-pose "$P"_j_default.bin --label j_def_tree_red501
# ── J, --cfg as ──
run j_as_allpairs_w1  0 --scene jolt --gap 0.5 --cfg as --broadphase allpairs --workers 1 --steps 500 --pose-out "$P"_j_as.bin --expect-pose "$P"_j_default.bin --label j_as_allpairs_w1
run j_as_tree_w1      0 --scene jolt --gap 0.5 --cfg as --broadphase tree     --workers 1 --steps 500 --expect-pose "$P"_j_as.bin --label j_as_tree_w1
run j_as_allpairs_w8  0 --scene jolt --gap 0.5 --cfg as --broadphase allpairs --workers 8 --steps 500 --expect-pose "$P"_j_as.bin --label j_as_allpairs_w8
run j_as_tree_w8      0 --scene jolt --gap 0.5 --cfg as --broadphase tree     --workers 8 --steps 500 --expect-pose "$P"_j_as.bin --label j_as_tree_w8
run j_as_tree_red501  4 --scene jolt --gap 0.5 --cfg as --broadphase tree     --workers 1 --steps 501 --expect-pose "$P"_j_as.bin --label j_as_tree_red501
# ── rest, --cfg default ──
run rest_allpairs_w1  0 --scene rest --cfg default --broadphase allpairs --workers 1 --steps 500 --pose-out "$P"_rest.bin --label rest_allpairs_w1
run rest_tree_w1      0 --scene rest --cfg default --broadphase tree     --workers 1 --steps 500 --expect-pose "$P"_rest.bin --label rest_tree_w1
run rest_allpairs_w8  0 --scene rest --cfg default --broadphase allpairs --workers 8 --steps 500 --expect-pose "$P"_rest.bin --label rest_allpairs_w8
run rest_tree_w8      0 --scene rest --cfg default --broadphase tree     --workers 8 --steps 500 --expect-pose "$P"_rest.bin --label rest_tree_w8
run rest_tree_red501  4 --scene rest --cfg default --broadphase tree     --workers 1 --steps 501 --expect-pose "$P"_rest.bin --label rest_tree_red501
# ── s16 (brute path) ──
run s16_allpairs_w1   0 --scene s16 --cfg default --broadphase allpairs --workers 1 --steps 500 --pose-out "$P"_s16.bin --label s16_allpairs_w1
run s16_tree_w1       0 --scene s16 --cfg default --broadphase tree     --workers 1 --steps 500 --expect-pose "$P"_s16.bin --label s16_tree_w1
# ── controls ──
"$EXE" --scene jolt --broadphase foo > "$OUT/usage_bad.out" 2> "$OUT/usage_bad.err"; rc=$?; echo "$rc" > "$OUT/usage_bad.rc"; echo "usage_bad: exit $rc: $(cat "$OUT/usage_bad.err" | head -1)"
"$EXE" > "$OUT/self_check.out" 2> "$OUT/self_check.err"; rc=$?; echo "$rc" > "$OUT/self_check.rc"; echo "self_check: exit $rc: $(grep -oE 'steps [0-9]+ .*pose_hash 0x[0-9a-f]+' "$OUT/self_check.out" | head -1)"
sha256sum "$P"_*.bin
