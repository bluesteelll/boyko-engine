#!/usr/bin/env bash
# V2: the trunk's L10 pose gates on the V2 fixtures and the V2 hashes - the three `*_v2.sh`
# scripts make_v2_scripts.py writes from l10/. Want: 264/264, both negatives exit 4, 26/26, 17/17
# (the rest pile's reuse-on `Sets` rows exited VOID on the runner's L9 probe until its triage-r1
# fix, which exempts a probe window that collided no box pair; see the README).
# usage: pose_gates_v2.sh <exe> <outdir>
# Exit (docs/physics/perf-campaign/GATE-KIT.md): 2 if <exe> is missing or a child could not run
# (exit 2; each child decides that before its first row, the other children still run); else 1 if
# a child is red or OUT/all.txt lacks one of the five green lines in its child's section; else 0.
# ALL_DONE is always the last line of all.txt: the wrapper ran to its end, not that it passed.
G=$(cd "$(dirname "$0")" && pwd)
if [ $# -ne 2 ]; then echo "usage: pose_gates_v2.sh <exe> <outdir>" >&2; exit 2; fi
EXE=$1
OUT=$2
if [ ! -f "$EXE" ]; then echo "pose_gates_v2.sh: no runner exe at $EXE" >&2; exit 2; fi
mkdir -p "$OUT"
echo "== pose gates (sync, V2 fixtures)" > "$OUT/all.txt"; bash "$G/pose_gates_sync_v2.sh" "$EXE" "$OUT/pose" >> "$OUT/all.txt" 2>&1; rc_sync=$?
echo "== runner checks (c3c, V2 fixtures)" >> "$OUT/all.txt"; bash "$G/runner_checks_c3c_v2.sh" "$EXE" "$OUT/runner" >> "$OUT/all.txt" 2>&1; rc_c3c=$?
echo "== runner checks (reuse-off, V2 fixtures)" >> "$OUT/all.txt"; bash "$G/runner_checks_off_v2.sh" "$EXE" "$OUT/runner_off" >> "$OUT/all.txt" 2>&1; rc_off=$?
echo ALL_DONE >> "$OUT/all.txt"
# The verifier: each green line, whole and fixed, inside the section its child wrote (so one child
# cannot supply another's tally).
section() { awk -v n="$1" '/^== /{ s++; next } s == n' "$OUT/all.txt"; }
want() { # section line
  if ! section "$1" | grep -Fqx -- "$2"; then echo "pose_gates_v2.sh: section $1 of $OUT/all.txt lacks '$2'" >&2; return 1; fi
}
v=0
want 1 'neg flagless R-S W1 vs fixtures-reuse-off: exit=4 (want 4)' || v=1
want 1 'neg reuse-off R-S W1 vs fixtures: exit=4 (want 4)' || v=1
want 1 'runs 264, pass 264, fail 0' || v=1
want 2 'checks 26, pass 26, fail 0' || v=1
want 3 'checks 17, pass 17, fail 0' || v=1
for rc in $rc_sync $rc_c3c $rc_off; do [ "$rc" = 2 ] && exit 2; done
[ "$rc_sync" = 0 ] && [ "$rc_c3c" = 0 ] && [ "$rc_off" = 0 ] && [ "$v" = 0 ] || exit 1
