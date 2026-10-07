#!/usr/bin/env bash
# Dynamic parity scenes, C3: Jolt 5.6's dyn build (the parity patch v2), structural only - the timing
# column of Jolt's stat line and of -f's CSV is never read.
# usage: jolt_gates.sh <PerformanceTest.exe> <outdir> [<repo> [<9b stdout with the v1 banner>]]
#   G1 the repo patch applies to clean v5.6.0 sources (e77f175 from D:/tmp/jolt/JoltPhysics)
#   G3 the 9b build is untouched: 918fd2b7...
#   G4 J-T on this exe: hash 0xb8522b4e3fc62cfe at -t=1 and -t=8, the v1 banner byte-identical to 9b's,
#      no v2 banner, -f's CSV header `Frame, Time (ms)` and 500 rows (counted, never read)
#   G5 per program (kick, shoot, slide) and S-LAND (none): the v2 banner names it; the dump equals
#      dyn_spec_ref.py's canonical text; determinism (-t=1 twice, -t=8 twice); the pose at -t=1/2/4/8/16
#      (a receipt: Jolt promises no W-independence); -sanity changes no bit; dyn_sanity.py B1-B6 and
#      the premises at -t=1; the receipts line (readback_mismatches 0, update_errors 0); -receipt
#      runs for the work counts; the -t=1 pose and the stat line's hash = pins.json `jolt56.<P>`
#      (pose_pin.py: the fixture's sha256, then the bytes; the one check against the committed pin
#      rather than the run itself, triage r1 F2)
#   G6 ApplyEvents is called before clock_start inside the step loop (the patched source)
# The manifest receipt (G2: driver.py's source_is_repo_patch) is run by the caller at the lane head.
# Env: PROGRAMS (default "none kick shoot slide") narrows G5; JOLT_SRC points G6 at another source.
set -u
EXE=$1
OUT=$2
REPO=${3:-D:/wt/merge}
BANNER9B=${4:-D:/tmp/phys-orch/win9b/raw/V2-AB-p0_130001/001_r0_V2-jolt56_j56_W16/stdout.txt}
G=$(cd "$(dirname "$0")" && pwd)
PATCH=$REPO/crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch
mkdir -p "$OUT/canon"
python "$G/dyn_spec_ref.py" --write "$OUT/canon" > "$OUT/canon/fnv.txt"
T="$OUT/gates.tsv"
: > "$T"
check() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$T"; }
hash_of() { grep -o '0x[0-9a-f]*$' "$1" | tail -1; }
jrun() { # dir args...: one process per fresh working directory
  local d=$1; shift
  mkdir -p "$d"
  (cd "$d" && "$EXE" "$@" > out.txt 2>&1; echo $? > rc.txt)
}
# G1
S="$OUT/clean_v560"
mkdir -p "$S/PerformanceTest"
for f in PerformanceTest.cpp PerformanceTestScene.h PyramidScene.h; do
  git -C D:/tmp/jolt/JoltPhysics show e77f175:PerformanceTest/$f > "$S/PerformanceTest/$f"
done
(cd "$S" && git apply --check "$PATCH") && check "G1 patch applies to clean v5.6.0" PASS "" || check "G1 patch applies to clean v5.6.0" FAIL ""
# G3
h=$(sha256sum D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe | cut -c1-64)
[ "$h" = 918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad ] && check "G3 9b exe untouched" PASS "$h" || check "G3 9b exe untouched" FAIL "$h"
# G4
want_banner=$(grep '^boyko-parity-patch v1' "$BANNER9B" | tr -d '\r')
for w in 1 8; do
  d="$OUT/jt_t$w"
  jrun "$d" -s=Pyramid -q=Discrete -f -t=$w -i=500
  hh=$(hash_of "$d/out.txt"); rc=$(cat "$d/rc.txt")
  [ "$rc" = 0 ] && [ "$hh" = 0xb8522b4e3fc62cfe ] && check "G4 J-T hash -t=$w" PASS "$hh" || check "G4 J-T hash -t=$w" FAIL "rc $rc $hh"
  b=$(grep '^boyko-parity-patch v1' "$d/out.txt" | tr -d '\r')
  [ "$b" = "$want_banner" ] && ! grep -q 'boyko-parity-dyn' "$d/out.txt" && check "G4 J-T banner -t=$w" PASS "$b" || check "G4 J-T banner -t=$w" FAIL "$b"
  csv="$d/per_frame_discrete_th$w.csv"
  head1=$(head -1 "$csv" | tr -d '\r'); n=$(($(wc -l < "$csv") - 1))
  [ "$head1" = "Frame, Time (ms)" ] && [ "$n" = 500 ] && check "G4 J-T -f CSV shape -t=$w" PASS "$n rows" || check "G4 J-T -f CSV shape -t=$w" FAIL "$head1 / $n"
done
# G5
for P in ${PROGRAMS:-none kick shoot slide}; do
  if [ $P = none ]; then DYN=(); I=500; else DYN=(-dyn=$P); case $P in slide) I=500 ;; *) I=800 ;; esac; fi
  base=(-s=Pyramid "${DYN[@]}" -q=Discrete -i=$I)
  for w in 1 2 4 8 16; do jrun "$OUT/$P/t$w" "${base[@]}" -t=$w -pose_out=final.pose; done
  jrun "$OUT/$P/t1b" "${base[@]}" -t=1 -pose_out=final.pose
  jrun "$OUT/$P/t8b" "${base[@]}" -t=8 -pose_out=final.pose
  jrun "$OUT/$P/san1" "${base[@]}" -t=1 -pose_out=final.pose -sanity -scene_dump=dump.txt
  jrun "$OUT/$P/san8" "${base[@]}" -t=8 -pose_out=final.pose -sanity -scene_dump=dump.txt
  jrun "$OUT/$P/rcpt1" "${base[@]}" -t=1 -pose_out=final.pose -receipt
  rcs=$(cat "$OUT/$P"/*/rc.txt | sort -u | tr '\n' ' ')
  [ "$rcs" = "0 " ] && check "G5 $P exits" PASS "all 0" || check "G5 $P exits" FAIL "$rcs"
  ban=$(grep '^boyko-parity-dyn v1' "$OUT/$P/t1/out.txt" | tr -d '\r')
  [ "$ban" = "boyko-parity-dyn v1: program=$P, sanity=0, scene_dump=0, pose_out=1" ] && check "G5 $P banner" PASS "$ban" || check "G5 $P banner" FAIL "$ban"
  r=$(python "$G/compare_dumps.py" "$OUT/$P/san1/dump.txt" "$OUT/canon/$P.dump"); rc=$?
  [ $rc = 0 ] && check "G5 $P dump = canonical" PASS "$r" || check "G5 $P dump = canonical" FAIL "$r"
  cmp -s "$OUT/$P/san1/dump.txt" "$OUT/$P/san8/dump.txt" && check "G5 $P dump -t=1 = -t=8" PASS "" || check "G5 $P dump -t=1 = -t=8" FAIL ""
  ok=PASS; cmp -s "$OUT/$P/t1/final.pose" "$OUT/$P/t1b/final.pose" || ok=FAIL; cmp -s "$OUT/$P/t8/final.pose" "$OUT/$P/t8b/final.pose" || ok=FAIL
  check "G5 $P determinism t1x2 t8x2" $ok "$(hash_of "$OUT/$P/t1/out.txt") $(hash_of "$OUT/$P/t8/out.txt")"
  det=""; for w in 2 4 8 16; do cmp -s "$OUT/$P/t1/final.pose" "$OUT/$P/t$w/final.pose" || det="$det t$w"; done
  check "G5 $P cross-W pose (receipt)" "$([ -z "$det" ] && echo SAME || echo DIFFERS)" "${det:-all equal}"
  ok=PASS; cmp -s "$OUT/$P/t1/final.pose" "$OUT/$P/san1/final.pose" || ok=FAIL; cmp -s "$OUT/$P/t8/final.pose" "$OUT/$P/san8/final.pose" || ok=FAIL
  cmp -s "$OUT/$P/t1/final.pose" "$OUT/$P/rcpt1/final.pose" || ok=FAIL
  check "G5 $P -sanity / -receipt change no bit" $ok "$(hash_of "$OUT/$P/san1/out.txt") $(hash_of "$OUT/$P/rcpt1/out.txt")"
  r=$(python "$G/dyn_sanity.py" --program $P "$OUT/$P/san1/sanity_discrete_th1.csv" --json "$OUT/$P/bar_t1.json" > "$OUT/.last" 2>&1; echo $? > "$OUT/.rc"; head -1 "$OUT/.last"); rc=$(cat "$OUT/.rc")
  [ $rc = 0 ] && check "G5 $P bar B1-B6 + premise (-t=1)" PASS "$r" || check "G5 $P bar B1-B6 + premise (-t=1)" FAIL "$r"
  r=$(python "$G/dyn_sanity.py" --final $P "$OUT/$P/t1/final.pose" > "$OUT/.last" 2>&1; echo $? > "$OUT/.rc"; head -1 "$OUT/.last"); rc=$(cat "$OUT/.rc")
  [ $rc = 0 ] && check "G5 $P final B1-B3" PASS "$r" || check "G5 $P final B1-B3" FAIL "$r"
  rl=$(grep '^boyko-parity-dyn receipts' "$OUT/$P/san1/out.txt" | tr -d '\r')
  echo "$rl" | grep -q 'readback_mismatches=0, update_errors=0' && check "G5 $P receipts line" PASS "$rl" || check "G5 $P receipts line" FAIL "$rl"
  r=$(python "$G/pose_pin.py" jolt56 $P "$OUT/$P/t1/final.pose" --hash "$(hash_of "$OUT/$P/t1/out.txt")"); rc=$?
  [ $rc = 0 ] && check "G5 $P -t=1 pose = pin (pins.json)" PASS "$r" || check "G5 $P -t=1 pose = pin (pins.json)" FAIL "$r"
done
# G6
SRC=${JOLT_SRC:-D:/tmp/jolt/wt-v5.6.0-dyn/PerformanceTest/PerformanceTest.cpp}
python - "$SRC" > "$OUT/g6.txt" <<'PY'
import sys
lines = open(sys.argv[1], encoding='utf-8').read().replace('\r\n', '\n').split('\n')
loop = next(i for i, l in enumerate(lines) if 'for (uint iterations = 0; iterations < max_iterations; ++iterations)' in l)
call = next(i for i in range(loop, len(lines)) if 'scene->ApplyEvents(' in lines[i])
start = next(i for i in range(loop, len(lines)) if 'clock_start = chrono::high_resolution_clock::now()' in lines[i])
end = next(i for i in range(loop, len(lines)) if 'clock_end = chrono::high_resolution_clock::now()' in lines[i])
ok = loop < call < start < end
print('%s loop %d ApplyEvents %d clock_start %d clock_end %d' % ('PASS' if ok else 'FAIL', loop + 1, call + 1, start + 1, end + 1))
PY
r=$(cat "$OUT/g6.txt"); case $r in PASS*) check "G6 ApplyEvents before clock_start" PASS "$r" ;; *) check "G6 ApplyEvents before clock_start" FAIL "$r" ;; esac
cat "$T"
echo "checks $(wc -l < "$T"), pass $(grep -c "	PASS	" "$T"), fail $(grep -c "	FAIL	" "$T")"
