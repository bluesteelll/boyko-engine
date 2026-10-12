#!/usr/bin/env bash
# Lane SL, SL-1 (ruling 39 Q1 + Q2): record both pose-gate fixture sets into this directory with the
# lane's parity runner, at W1 (every W8 file of a W1/W8 pair is a copy of its W1 recording, so a W8
# gate row checks worker-count identity against the W1 bytes, as V2's recorder does).
#   v2/  - the V2 set: record_v2_fixtures.sh's rows, verbatim (the V2 gate scripts' rows).
#   d0/  - the overlap-only set: the rows l10/*.sh check, run through gate/runner_d0.sh (both
#          overlap-only flags appended), mirroring the pre-V2 record directories the d0 gate read.
# Every row is recorded, sleeping off as well as on: a sleeping-off file must come out byte-equal to
# the record directory's original (SL changes no sleeping-off value), which README.md's table checks.
# The record directories themselves are never edited (V2's precedent: originals read, never edited).
# usage: record_sl_fixtures.sh <exe> <outdir> <logdir>  (outdir = docs/measurements/2026-10-11-sl-sleeping;
# logdir, outside the checkout, takes every row's runner output; logs/record.tsv is the committed log)
set -u
if [ $# -ne 3 ]; then echo "usage: record_sl_fixtures.sh <exe> <outdir> <logdir>" >&2; exit 2; fi
EXE=$1
OUT=$2
RUNLOG=$3
if [ ! -f "$EXE" ]; then echo "record_sl_fixtures.sh: no runner exe at $EXE" >&2; exit 2; fi
HERE=$(cd "$(dirname "$0")" && pwd)
D0=$HERE/../2026-09-30-v2-speculative/gate/runner_d0.sh
if [ ! -f "$D0" ]; then echo "record_sl_fixtures.sh: no runner_d0.sh at $D0" >&2; exit 2; fi
SON="--sleeping on"
for d in v2/l9-reuse-on v2/l9-reuse-off v2/l10-reuse-on v2/l10-reuse-off v2/w6 v2/w8 \
  d0/l9-fixtures d0/l9-fixtures-c4 d0/l10-fixtures d0/l10-fixtures-reuse-off d0/w6-fixtures logs; do
  mkdir -p "$OUT/$d"
done
mkdir -p "$RUNLOG"
LOG="$OUT/logs/record.tsv"
: > "$LOG"
rec() { # set dir name flags...  (set: v2 runs the exe, d0 runs it through runner_d0.sh)
  local set=$1 dir=$2 name=$3; shift 3
  local o="$RUNLOG/${set}_${dir}_$name.out"
  if [ "$set" = d0 ]; then
    RUNNER_D0_EXE=$EXE bash "$D0" "$@" --workers 1 --pose-out "$OUT/$set/$dir/$name.pose" > "$o" 2>&1
  else
    "$EXE" "$@" --workers 1 --pose-out "$OUT/$set/$dir/$name.pose" > "$o" 2>&1
  fi
  local rc=$?
  local h; h=$(grep -o 'pose_hash [0-9a-fx]*' "$o" | awk '{print $2}')
  printf '%s\t%s\texit=%s\t%s\t%s\n' "$set/$dir" "$name" "$rc" "$h" "$*" >> "$LOG"
}
w1w8() { # set dir name flags... : W1 recording, W8 copy
  local set=$1 dir=$2 name=$3; shift 3
  rec "$set" "$dir" "${name}_W1" "$@"
  cp "$OUT/$set/$dir/${name}_W1.pose" "$OUT/$set/$dir/${name}_W8.pose"
}
for arm in on off; do
  if [ "$arm" = on ]; then RU=; L9=l9-fixtures-c4; L10=l10-fixtures; else RU="--contact-reuse off"; L9=l9-fixtures; L10=l10-fixtures-reuse-off; fi
  for set in v2 d0; do
    if [ "$set" = v2 ]; then F9=l9-reuse-$arm; F10=l10-reuse-$arm; else F9=$L9; F10=$L10; fi
    # L9's C0 family (pose_gates_sync.sh G2), 600 steps.
    w1w8 $set $F9 J-A --scene jolt --gap 0.5 --cfg a --sleeping off --steps 600 $RU
    w1w8 $set $F9 J-D --scene jolt --gap 0.5 --cfg default --sleeping off --steps 600 $RU
    w1w8 $set $F9 R --scene rest --solver colored --sleeping off --steps 600 $RU
    w1w8 $set $F9 R-S --scene rest $SON --steps 600 $RU
    w1w8 $set $F9 J-Son --scene jolt --gap 0.5 --cfg a $SON --steps 600 $RU
    # L10's fixtures (G3 and the runner checks), 600 steps.
    w1w8 $set $F10 R --scene rest --solver colored --sleeping off --steps 600 $RU
    w1w8 $set $F10 R-S --scene rest $SON --steps 600 $RU
    w1w8 $set $F10 J-Son --scene jolt --gap 0.5 --cfg a $SON --steps 600 $RU
    w1w8 $set $F10 J-A-on --scene jolt --gap 0.5 --cfg a $SON --steps 600 $RU
    w1w8 $set $F10 R-on --scene rest --solver colored $SON --steps 600 $RU
    w1w8 $set $F10 J-D-on --scene jolt --gap 0.5 --cfg default $SON --steps 600 $RU
  done
done
# Window 6's gate fixtures (G4), 1000 / 800 steps, in both sets.
for set in v2 d0; do
  if [ "$set" = v2 ]; then F6=w6; else F6=w6-fixtures; fi
  rec $set $F6 Offp-J1000 --scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on --steps 1000
  rec $set $F6 Son-J1000 --scene jolt --gap 0.5 --cfg a --sleeping --contact-reuse off --steps 1000
  rec $set $F6 RSOffp800 --scene rest --sleeping --broadphase tree --contact-reuse on --steps 800
  rec $set $F6 RS800 --scene rest --sleeping --contact-reuse off --steps 800
done
# Window 8 / 8b's gate fixtures (V2 set only), 500 steps.
rec v2 w8 JT500 --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off --steps 500
rec v2 w8 JToff500 --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off --contact-reuse off --steps 500
rec v2 w8 JA500 --scene jolt --gap 0.5 --cfg a --sleeping off --steps 500
rec v2 w8 JSonT500 --scene jolt --gap 0.5 --cfg a --broadphase tree --sleeping on --steps 500
# The fidelity gate's fourth anchor (gap 0.5, sleeping on, contact reuse off), 500 steps.
rec v2 w8 JSonToff500 --scene jolt --gap 0.5 --cfg a --broadphase tree --sleeping on --contact-reuse off --steps 500
rec v2 w8 RT500 --scene rest --cfg default --broadphase tree --bp-kernel leaflist --sleeping off --steps 500
# The parity runner's R1100 (window 6's Ron1100 row).
rec v2 w6 R1100 --scene rest --solver colored --sleeping off --steps 1100
# The J500 hashes the sync gate pins (G1).
rec v2 w8 J500 --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off --steps 500
rec v2 w8 J500off --scene jolt --gap 0.5 --cfg default --broadphase tree --sleeping off --steps 500 --contact-reuse off
(cd "$OUT" && find v2 d0 -name '*.pose' | LC_ALL=C sort | xargs sha256sum) > "$OUT/SHA256SUMS"
echo "recorded $(wc -l < "$LOG"), nonzero exits $(grep -vc 'exit=0' "$LOG")"
