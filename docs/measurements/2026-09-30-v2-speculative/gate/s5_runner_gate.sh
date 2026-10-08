#!/usr/bin/env bash
# S5's runner {1, N} gate (cut §3 C3): every row at W 1 2 3 4 5 8 16 (R1100 at 1 8 16), arms
# --parallel-tree-query on / off; want exit 0, the pinned pose hash, and the receipt's dispatch
# count (counted == recomputed; on at W >= 2 dispatches on every step after the first).
# usage: s5_runner_gate.sh [--root DIR] <exe> <outdir>   (fixtures from ROOT: the gate-root block)
# Committed from D:/tmp/phys-orch/s5/r3/tools/s5_runner_gate.sh (md5 7c692a8ff641ae7137586ea824dfa4fa); the rows,
# their hashes and the row format are r3's byte for byte.
# Exit (docs/physics/perf-campaign/GATE-KIT.md): 0 = every row PASS and all EXPECT_ROWS rows ran; 1 = red;
# 2 = could not run (usage, no exe, a fixture missing), decided before the first row.
set -u
EXPECT_ROWS=54 # 7 W x 2 arms x 3 + 3 W x 2 arms (R1100) + 3 W x 2 arms (rowwalk)
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
V2=$ROOT/docs/measurements/2026-09-30-v2-speculative/w8
W9A=$ROOT/docs/measurements/2026-09-30-physics-window9a/gate/fixtures
if [ ! -f "$EXE" ]; then echo "$ME: no runner exe at $EXE" >&2; exit 2; fi
missing=0
for f in "$V2/JT500.pose" "$V2/JToff500.pose" "$W9A/JT500.pose"; do
  [ -f "$f" ] || { echo "$ME: missing fixture $f" >&2; missing=1; }
done
if [ "$missing" != 0 ]; then exit 2; fi
mkdir -p "$OUT"
T="$OUT/rows.tsv"
: > "$T"
JT="--scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off --steps 500"
D0="--speculative-distance 0 --speculative-velocity-cap 0"
row() { # label W arm want flags...
  local label=$1 w=$2 arm=$3 want=$4; shift 4
  local f="$OUT/${label}_${arm}_W${w}.out"
  "$EXE" "$@" --parallel-tree-query "$arm" --workers "$w" > "$f" 2>&1
  local rc=$?
  local h; h=$(grep -o 'pose_hash 0x[0-9a-f]*' "$f" | head -1 | awk '{print $2}')
  local s5; s5=$(grep -o '"bp_query_dispatches":{[^}]*}' "$f" | head -1)
  local tail; tail=$(grep -o '"bp_query_tail_leaves":{[^}]*}' "$f" | head -1)
  local v=PASS
  if [ "$rc" != 0 ] || [ "$h" != "$want" ]; then v=FAIL; fi
  printf '%s\t%s\tW%s\texit=%s\t%s\twant=%s\t%s\t%s\t%s\n' "$label" "$arm" "$w" "$rc" "${h:-none}" "$want" "$v" "$s5" "$tail" >> "$T"
}
for w in 1 2 3 4 5 8 16; do
  for arm in on off; do
    row JT500_v2 $w $arm 0x441a568e91a4f9c9 $JT --expect-pose $V2/JT500.pose
    row JToff500_v2 $w $arm 0xbc09a1faf7b413a8 $JT --contact-reuse off --expect-pose $V2/JToff500.pose
    row JT500_d0 $w $arm 0x30c5438bc6ad9ffa $JT $D0 --expect-pose $W9A/JT500.pose
  done
done
for w in 1 8 16; do
  for arm in on off; do
    row R1100_v2 $w $arm 0x21bc7ecb642b0090 --scene rest --solver colored --steps 1100
  done
done
# Fix r1 (triage G3): the RowWalk kernel with the switch on stays serial - exit 0, the fixture pose,
# 0 dispatches and 0 leaf-list leaves (the runner's own --bp-kernel receipt voids with exit 3 otherwise).
rowwalk() { # W arm
  local w=$1 arm=$2
  local f="$OUT/JT500_v2_rowwalk_${arm}_W${w}.out"
  "$EXE" $JT --bp-kernel rowwalk --expect-pose $V2/JT500.pose --parallel-tree-query "$arm" --workers "$w" > "$f" 2>&1
  local rc=$?
  local h; h=$(grep -o 'pose_hash 0x[0-9a-f]*' "$f" | head -1 | awk '{print $2}')
  local s5; s5=$(grep -o '"bp_query_dispatches":{[^}]*}' "$f" | head -1)
  local ll; ll=$(grep -o '"leaf_list_leaves":[0-9]*' "$f" | head -1)
  local rw; rw=$(grep -o '"row_walk_leaves":[0-9]*' "$f" | head -1)
  local v=PASS
  if [ "$rc" != 0 ] || [ "$h" != 0x441a568e91a4f9c9 ] || [ "$ll" != '"leaf_list_leaves":0' ] || [ "$rw" = '"row_walk_leaves":0' ] || ! echo "$s5" | grep -q '"run":0,'; then v=FAIL; fi
  printf '%s	%s	W%s	exit=%s	%s	want=%s	%s	%s	%s %s
' JT500_v2_rowwalk "$arm" "$w" "$rc" "${h:-none}" 0x441a568e91a4f9c9 "$v" "$s5" "$ll" "$rw" >> "$T"
}
for w in 1 8 16; do
  for arm in on off; do
    rowwalk $w $arm
  done
done
cat "$T"
rows=$(wc -l < "$T"); pass=$(grep -c PASS "$T"); fail=$(grep -c FAIL "$T")
echo "rows $rows, pass $pass, fail $fail"
[ "$rows" = "$EXPECT_ROWS" ] && [ "$pass" = "$EXPECT_ROWS" ] && [ "$fail" = 0 ] || exit 1
