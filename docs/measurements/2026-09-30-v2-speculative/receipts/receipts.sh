#!/usr/bin/env bash
# V2 C6: the work receipts window 9 re-reads (untimed): armed W1 rows, 500 steps, per-step CSV;
# receipts.py reduces steps [100, 500) to manifolds, points, colours, colour passes and
# velocity rows. The tip runs at its default (V2) and with both overlap-only flags; the parent
# (16191fda) runs the same rows flagless. usage: receipts.sh <tip exe> <parent exe> <outdir>
set -u
TIP=$1; PAR=$2; OUT=$3
mkdir -p "$OUT"
D0="--speculative-distance 0 --speculative-velocity-cap 0"
JT="--scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off"
row() { # name exe flags...
  local name=$1 exe=$2; shift 2
  "$exe" "$@" --arm-profiler --workers 1 --steps 500 --window 0..500 --csv "$OUT/$name.csv" > "$OUT/$name.out" 2>&1
  echo "$name exit=$? $*" >> "$OUT/rows.txt"
}
: > "$OUT/rows.txt"
row JT_v2 "$TIP" $JT
row JT_d0 "$TIP" $JT $D0
row JT_parent "$PAR" $JT
row JToff_v2 "$TIP" $JT --contact-reuse off
row JToff_d0 "$TIP" $JT --contact-reuse off $D0
row JToff_parent "$PAR" $JT --contact-reuse off
python "$(dirname "$0")/receipts.py" "$OUT" > "$OUT/receipts.txt"
cat "$OUT/receipts.txt"
