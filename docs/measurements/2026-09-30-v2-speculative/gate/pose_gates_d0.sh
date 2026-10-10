#!/usr/bin/env bash
# V2: the trunk's L10 pose gates under the overlap-only rule - the three scripts in l10/ unchanged,
# their fixtures unchanged (the pre-V2 references), every row run with both overlap-only flags
# through runner_d0.sh. Want: 264/264, both negatives exit 4, 26/26, 17/17.
# usage: pose_gates_d0.sh [--root DIR] <exe> <outdir>   (ROOT from the gate-root block, passed to every child)
# Exit (docs/physics/perf-campaign/GATE-KIT.md): 2 if <exe> is missing or a child could not run
# (exit 2; each child decides that before its first row, the other children still run); else 1 if
# a child is red or OUT/all.txt lacks one of the five green lines in its child's section; else 0.
# ALL_DONE is always the last line of all.txt: the wrapper ran to its end, not that it passed.
G=$(cd "$(dirname "$0")" && pwd)
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
if [ $# -ne 2 ]; then echo "usage: pose_gates_d0.sh [--root DIR] <exe> <outdir>" >&2; exit 2; fi
if [ ! -f "$1" ]; then echo "pose_gates_d0.sh: no runner exe at $1" >&2; exit 2; fi
export RUNNER_D0_EXE=$1
OUT=$2
W=$G/runner_d0.sh
mkdir -p "$OUT"
echo "== pose gates (sync, d = 0)" > "$OUT/all.txt"; bash "$G/l10/pose_gates_sync.sh" --root "$ROOT" "$W" "$OUT/pose" >> "$OUT/all.txt" 2>&1; rc_sync=$?
echo "== runner checks (c3c, d = 0)" >> "$OUT/all.txt"; bash "$G/l10/runner_checks_c3c.sh" --root "$ROOT" "$W" "$OUT/runner" >> "$OUT/all.txt" 2>&1; rc_c3c=$?
echo "== runner checks (reuse-off, d = 0)" >> "$OUT/all.txt"; bash "$G/l10/runner_checks_off.sh" --root "$ROOT" "$W" "$OUT/runner_off" >> "$OUT/all.txt" 2>&1; rc_off=$?
echo ALL_DONE >> "$OUT/all.txt"
# The verifier: each green line, whole and fixed, inside the section its child wrote (so one child
# cannot supply another's tally).
section() { awk -v n="$1" '/^== /{ s++; next } s == n' "$OUT/all.txt"; }
want() { # section line
  if ! section "$1" | grep -Fqx -- "$2"; then echo "pose_gates_d0.sh: section $1 of $OUT/all.txt lacks '$2'" >&2; return 1; fi
}
v=0
want 1 'neg flagless R-S W1 vs fixtures-reuse-off: exit=4 (want 4)' || v=1
want 1 'neg reuse-off R-S W1 vs fixtures: exit=4 (want 4)' || v=1
want 1 'runs 264, pass 264, fail 0' || v=1
want 2 'checks 26, pass 26, fail 0' || v=1
want 3 'checks 17, pass 17, fail 0' || v=1
for rc in $rc_sync $rc_c3c $rc_off; do [ "$rc" = 2 ] && exit 2; done
[ "$rc_sync" = 0 ] && [ "$rc_c3c" = 0 ] && [ "$rc_off" = 0 ] && [ "$v" = 0 ] || exit 1
