#!/usr/bin/env bash
# V2: the trunk's L10 pose gates on the V2 fixtures and the V2 hashes - the three `*_v2.sh`
# scripts make_v2_scripts.py writes from l10/. Want: 264/264, both negatives exit 4, 26/26, 17/17
# (see the README: the rest pile's reuse-on `Sets` rows exit VOID on the runner's L9 probe).
# usage: pose_gates_v2.sh <exe> <outdir>
G=$(cd "$(dirname "$0")" && pwd)
EXE=$1
OUT=$2
mkdir -p "$OUT"
echo "== pose gates (sync, V2 fixtures)" > "$OUT/all.txt"; bash "$G/pose_gates_sync_v2.sh" "$EXE" "$OUT/pose" >> "$OUT/all.txt" 2>&1
echo "== runner checks (c3c, V2 fixtures)" >> "$OUT/all.txt"; bash "$G/runner_checks_c3c_v2.sh" "$EXE" "$OUT/runner" >> "$OUT/all.txt" 2>&1
echo "== runner checks (reuse-off, V2 fixtures)" >> "$OUT/all.txt"; bash "$G/runner_checks_off_v2.sh" "$EXE" "$OUT/runner_off" >> "$OUT/all.txt" 2>&1
echo ALL_DONE >> "$OUT/all.txt"
