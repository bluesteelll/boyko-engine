#!/usr/bin/env bash
# The trunk's parity-runner value pins at W 1 and 8 (merge-l10 verify2's runner_pins.sh rows):
# J500, J500 --contact-reuse off, R1100. Flagless they read V2's recordings (the fixture directory's
# w8/J500, w8/J500off, w6/R1100); with both overlap-only flags they must read the pre-V2 trunk
# pins exactly. usage: runner_pins.sh <exe> <outdir>
# Exit (docs/physics/perf-campaign/GATE-KIT.md): 0 = every row PASS and all EXPECT_ROWS rows ran;
# 1 = red (a FAIL row or a short tally); 2 = could not run (usage, or no exe), before the first row.
set -u
EXPECT_ROWS=12 # 2 W x 6 rows
if [ $# -ne 2 ]; then echo "usage: runner_pins.sh <exe> <outdir>" >&2; exit 2; fi
EXE=$1
OUT=$2
if [ ! -f "$EXE" ]; then echo "runner_pins.sh: no runner exe at $EXE" >&2; exit 2; fi
mkdir -p "$OUT"
T="$OUT/pins.tsv"
: > "$T"
D0="--speculative-distance 0 --speculative-velocity-cap 0"
row() { # label W want flags...
  local label=$1 w=$2 want=$3; shift 3
  "$EXE" "$@" --workers "$w" > "$OUT/${label}_W${w}.out" 2>&1
  local rc=$?
  # Exactly one human-readable hash line (the SUMMARY JSON carries the hash too, as a quoted field).
  local n; n=$(grep -c 'pose_hash 0x' "$OUT/${label}_W${w}.out")
  local h; h=$(grep -o 'pose_hash [0-9a-fx]*' "$OUT/${label}_W${w}.out" | head -1 | awk '{print $2}')
  local v=PASS
  if [ "$rc" != 0 ] || [ "$h" != "$want" ] || [ "$n" != 1 ]; then v=FAIL; fi
  printf '%s\tW%s\texit=%s\tlines=%s\t%s\twant=%s\t%s\t%s\n' "$label" "$w" "$rc" "$n" "${h:-none}" "$want" "$v" "$*" >> "$T"
}
for w in 1 8; do
  row J500_v2 $w 0x441a568e91a4f9c9 --scene jolt --steps 500
  row J500off_v2 $w 0xbc09a1faf7b413a8 --scene jolt --steps 500 --contact-reuse off
  row R1100_v2 $w 0x21bc7ecb642b0090 --scene rest --solver colored --steps 1100
  row J500_d0 $w 0x30c5438bc6ad9ffa --scene jolt --steps 500 $D0
  row J500off_d0 $w 0x32d5e235342b4143 --scene jolt --steps 500 --contact-reuse off $D0
  row R1100_d0 $w 0xc8bbe34cf6a8afc6 --scene rest --solver colored --steps 1100 $D0
done
cat "$T"
rows=$(wc -l < "$T"); pass=$(grep -c PASS "$T"); fail=$(grep -c FAIL "$T")
echo "rows $rows, pass $pass, fail $fail"
[ "$rows" = "$EXPECT_ROWS" ] && [ "$pass" = "$EXPECT_ROWS" ] && [ "$fail" = 0 ] || exit 1
