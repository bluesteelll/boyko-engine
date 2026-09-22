#!/bin/bash
# Pose gate (recipe section 2.2), untimed. Every process's stdout/stderr and exit code land in gate/.
W4=C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win4
R=$W4/bin/runner_c3.exe
G=$W4/gate
cd $G || exit 9
run() { # name, args...
  local name=$1; shift
  "$R" "$@" > $G/$name.out 2> $G/$name.err
  local rc=$?
  echo "$rc" > $G/$name.rc
  local pose=$(grep -o '"pose_hash":"[^"]*"' $G/$name.out | head -1)
  local exp=$(grep -o '"expect_pose":"[^"]*"' $G/$name.out | head -1)
  local bp=$(grep -o '"broadphase_tree":{[^}]*}' $G/$name.out | head -1)
  local w=$(grep -o '"workers":[0-9]*' $G/$name.out | head -1)
  echo "$name: exit $rc $w $pose $exp $bp"
}
# 1. J default: allpairs reference at W1, then tree at W1/8/16, allpairs at 8/16, grid at 1/8/16, all --expect-pose against the allpairs W1 pose
run j_def_allpairs_w1 --scene jolt --gap 0.5 --cfg default --broadphase allpairs --steps 500 --workers 1 --pose-out $G/pose_j_default.bin
for w in 1 8 16; do
  run j_def_tree_w$w --scene jolt --gap 0.5 --cfg default --broadphase tree --steps 500 --workers $w --expect-pose $G/pose_j_default.bin
  run j_def_grid_w$w --scene jolt --gap 0.5 --cfg default --broadphase grid --steps 500 --workers $w --expect-pose $G/pose_j_default.bin
done
for w in 8 16; do
  run j_def_allpairs_w$w --scene jolt --gap 0.5 --cfg default --broadphase allpairs --steps 500 --workers $w --expect-pose $G/pose_j_default.bin
done
# red control: 501 steps against the 500-step pose must exit 4
run j_def_tree_red501 --scene jolt --gap 0.5 --cfg default --broadphase tree --steps 501 --workers 1 --expect-pose $G/pose_j_default.bin
# 2. J cfg a: allpairs reference (its own hash), tree at W1/8/16, allpairs at 8/16, red control
run j_a_allpairs_w1 --scene jolt --gap 0.5 --cfg a --broadphase allpairs --steps 500 --workers 1 --pose-out $G/pose_j_a.bin
for w in 1 8 16; do
  run j_a_tree_w$w --scene jolt --gap 0.5 --cfg a --broadphase tree --steps 500 --workers $w --expect-pose $G/pose_j_a.bin
done
for w in 8 16; do
  run j_a_allpairs_w$w --scene jolt --gap 0.5 --cfg a --broadphase allpairs --steps 500 --workers $w --expect-pose $G/pose_j_a.bin
done
run j_a_tree_red501 --scene jolt --gap 0.5 --cfg a --broadphase tree --steps 501 --workers 1 --expect-pose $G/pose_j_a.bin
# 3. rest: allpairs reference, tree at W1/8/16, allpairs at 8, red control
run rest_allpairs_w1 --scene rest --cfg default --broadphase allpairs --steps 500 --workers 1 --pose-out $G/pose_rest.bin
for w in 1 8 16; do
  run rest_tree_w$w --scene rest --cfg default --broadphase tree --steps 500 --workers $w --expect-pose $G/pose_rest.bin
done
run rest_allpairs_w8 --scene rest --cfg default --broadphase allpairs --steps 500 --workers 8 --expect-pose $G/pose_rest.bin
run rest_tree_red501 --scene rest --cfg default --broadphase tree --steps 501 --workers 1 --expect-pose $G/pose_rest.bin
# 4. s16: allpairs reference, tree at W1 (the brute path), red control
run s16_allpairs_w1 --scene s16 --cfg default --broadphase allpairs --steps 500 --workers 1 --pose-out $G/pose_s16.bin
run s16_tree_w1 --scene s16 --cfg default --broadphase tree --steps 500 --workers 1 --expect-pose $G/pose_s16.bin
run s16_tree_red501 --scene s16 --cfg default --broadphase tree --steps 501 --workers 1 --expect-pose $G/pose_s16.bin
# 5. usage text and self-check
"$R" --scene jolt --broadphase foo > $G/usage_bad.out 2> $G/usage_bad.err; echo $? > $G/usage_bad.rc; echo "usage_bad: exit $(cat $G/usage_bad.rc): $(head -1 $G/usage_bad.err)$(head -1 $G/usage_bad.out)"
"$R" > $G/self_check.out 2> $G/self_check.err; echo $? > $G/self_check.rc; echo "self_check: exit $(cat $G/self_check.rc): $(tail -2 $G/self_check.out | tr '\n' ' ')"
