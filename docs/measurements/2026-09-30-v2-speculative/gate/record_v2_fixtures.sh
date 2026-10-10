#!/usr/bin/env bash
# V2 C6: record the V2 pose fixtures with the tip's parity runner, at W1 (every W8 file is a copy
# of its W1 recording, so a W8 gate row checks worker-count identity against the W1 bytes, never a
# W8 recording against itself - critique W5). Rows and args are the ones the L9/L10/W6/W8 gate
# scripts check, verbatim, minus --workers / --expect-pose.
# usage: record_v2_fixtures.sh <exe> <outdir>  (outdir = docs/measurements/2026-09-30-v2-speculative)
set -u
EXE=$1
OUT=$2
SON="--sleeping on"
mkdir -p "$OUT/l9-reuse-on" "$OUT/l9-reuse-off" "$OUT/l10-reuse-on" "$OUT/l10-reuse-off" "$OUT/w6" "$OUT/w8" "$OUT/logs"
LOG="$OUT/logs/record.tsv"
: > "$LOG"
rec() { # dir name flags...
  local dir=$1 name=$2; shift 2
  "$EXE" "$@" --workers 1 --pose-out "$OUT/$dir/$name.pose" > "$OUT/logs/${dir}_$name.out" 2>&1
  local rc=$?
  local h; h=$(grep -o 'pose_hash [0-9a-fx]*' "$OUT/logs/${dir}_$name.out" | awk '{print $2}')
  printf '%s\t%s\texit=%s\t%s\t%s\n' "$dir" "$name" "$rc" "$h" "$*" >> "$LOG"
}
w1w8() { # dir name flags... : W1 recording, W8 copy
  local dir=$1 name=$2; shift 2
  rec "$dir" "${name}_W1" "$@"
  cp "$OUT/$dir/${name}_W1.pose" "$OUT/$dir/${name}_W8.pose"
}
for arm in on off; do
  if [ "$arm" = on ]; then RU=; else RU="--contact-reuse off"; fi
  # L9's C0 family (pose_gates_sync.sh G2), 600 steps.
  w1w8 l9-reuse-$arm J-A --scene jolt --gap 0.5 --cfg a --steps 600 $RU
  w1w8 l9-reuse-$arm J-D --scene jolt --gap 0.5 --cfg default --steps 600 $RU
  w1w8 l9-reuse-$arm R --scene rest --solver colored --steps 600 $RU
  w1w8 l9-reuse-$arm R-S --scene rest $SON --steps 600 $RU
  w1w8 l9-reuse-$arm J-Son --scene jolt --gap 0.5 --cfg a $SON --steps 600 $RU
  # L10's fixtures (G3 and the runner checks), 600 steps.
  w1w8 l10-reuse-$arm R --scene rest --solver colored --steps 600 $RU
  w1w8 l10-reuse-$arm R-S --scene rest $SON --steps 600 $RU
  w1w8 l10-reuse-$arm J-Son --scene jolt --gap 0.5 --cfg a $SON --steps 600 $RU
  w1w8 l10-reuse-$arm J-A-on --scene jolt --gap 0.5 --cfg a $SON --steps 600 $RU
  w1w8 l10-reuse-$arm R-on --scene rest --solver colored $SON --steps 600 $RU
  w1w8 l10-reuse-$arm J-D-on --scene jolt --gap 0.5 --cfg default $SON --steps 600 $RU
done
# Window 6's gate fixtures (G4), 1000 / 800 steps.
rec w6 Offp-J1000 --scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on --steps 1000
rec w6 Son-J1000 --scene jolt --gap 0.5 --cfg a --sleeping --contact-reuse off --steps 1000
rec w6 RSOffp800 --scene rest --sleeping --broadphase tree --contact-reuse on --steps 800
rec w6 RS800 --scene rest --sleeping --contact-reuse off --steps 800
# Window 8 / 8b's gate fixtures, 500 steps.
rec w8 JT500 --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off --steps 500
rec w8 JToff500 --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off --contact-reuse off --steps 500
rec w8 JA500 --scene jolt --gap 0.5 --cfg a --steps 500
rec w8 JSonT500 --scene jolt --gap 0.5 --cfg a --broadphase tree --sleeping on --steps 500
# The fidelity gate's fourth anchor (gap 0.5, sleeping on, contact reuse off), 500 steps.
rec w8 JSonToff500 --scene jolt --gap 0.5 --cfg a --broadphase tree --sleeping on --contact-reuse off --steps 500
rec w8 RT500 --scene rest --cfg default --broadphase tree --bp-kernel leaflist --steps 500
# The parity runner's R1100 (window 6's Ron1100 row, flagless on the trunk).
rec w6 R1100 --scene rest --solver colored --steps 1100
# The flagless J500 hashes the sync gate pins (G1).
rec w8 J500 --scene jolt --gap 0.5 --cfg default --broadphase tree --steps 500
rec w8 J500off --scene jolt --gap 0.5 --cfg default --broadphase tree --steps 500 --contact-reuse off
echo "recorded $(wc -l < "$LOG"), nonzero exits $(grep -vc 'exit=0' "$LOG")"
