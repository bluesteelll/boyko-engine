#!/usr/bin/env bash
# V2: the trunk's L10 pose gates under the overlap-only rule - the three scripts in l10/ unchanged,
# their fixtures unchanged (the pre-V2 references), every row run with both overlap-only flags
# through runner_d0.sh. Want: 264/264, both negatives exit 4, 26/26, 17/17.
# usage: pose_gates_d0.sh <exe> <outdir>
G=$(cd "$(dirname "$0")" && pwd)
export RUNNER_D0_EXE=$1
OUT=$2
W=$G/runner_d0.sh
mkdir -p "$OUT"
echo "== pose gates (sync, d = 0)" > "$OUT/all.txt"; bash "$G/l10/pose_gates_sync.sh" "$W" "$OUT/pose" >> "$OUT/all.txt" 2>&1
echo "== runner checks (c3c, d = 0)" >> "$OUT/all.txt"; bash "$G/l10/runner_checks_c3c.sh" "$W" "$OUT/runner" >> "$OUT/all.txt" 2>&1
echo "== runner checks (reuse-off, d = 0)" >> "$OUT/all.txt"; bash "$G/l10/runner_checks_off.sh" "$W" "$OUT/runner_off" >> "$OUT/all.txt" 2>&1
echo ALL_DONE >> "$OUT/all.txt"
