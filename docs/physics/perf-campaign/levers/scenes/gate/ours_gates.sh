#!/usr/bin/env bash
# Dynamic parity scenes, C2 gate 2: our runner on the dynamic scenes (structural; no timing is read).
# usage: ours_gates.sh <runner exe> <outdir> [programs...]   (default: none kick shoot slide)
# Per program P (none = S-LAND, the J-T row itself):
#   W-identity       the pose at W 1/2/4/8/16 equals W1's (the engine's {1,N} promise)
#   determinism      W1 twice and W8 twice give one pose
#   sanity-neutral   --sanity changes no bit (W1 and W8)
#   dump             --scene-dump equals dyn_spec_ref.py's canonical text byte for byte
#   bar              dyn_sanity.py B1-B6 and the premises on the W1 --sanity CSV
#   armed            --arm-profiler at W1 and W8 exits 0 with void_steps 0, same pose
#   counts           kick 1488 kicks / 24 events; shoot 60 launches / 1300 bodies; slide 0; none 0
#   prefix (kick)    --dyn kick --steps 200 == --scene jolt --steps 200 (same exe)
# Writes OUT/gates.tsv (one line per check, PASS/FAIL) and OUT/<P>/ (every process's output).
# A scorer's verdict is its exit code (the first cut piped it through `head`, whose status `$?` read:
# that gate could not fail, which red-first (a) of C3 exposed).
set -u
EXE=$1
OUT=$2
shift 2
PROGRAMS=${*:-none kick shoot slide}
G=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT/canon"
python "$G/dyn_spec_ref.py" --write "$OUT/canon" > "$OUT/canon/fnv.txt"
T="$OUT/gates.tsv"
: > "$T"
check() { # name verdict detail
  printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$T"
}
hash_of() { grep -o '"pose_hash":"0x[0-9a-f]*"' "$1" | head -1 | grep -o '0x[0-9a-f]*'; }
voids_of() { grep -o '"void_steps":[0-9]*' "$1" | head -1 | grep -o '[0-9]*$'; }
run() { # tag args... ; runs EXE, records exit in $D/$tag.rc
  local tag=$1; shift
  "$EXE" "$@" > "$D/$tag.out" 2>&1
  echo $? > "$D/$tag.rc"
}
for P in $PROGRAMS; do
  D="$OUT/$P"
  mkdir -p "$D"
  if [ "$P" = none ]; then BASE=(--scene jolt); else BASE=(--scene jolt --dyn "$P"); fi
  for w in 1 2 4 8 16; do run "w$w" "${BASE[@]}" --workers $w --pose-out "$D/w$w.pose"; done
  run w1b "${BASE[@]}" --workers 1 --pose-out "$D/w1b.pose"
  run w8b "${BASE[@]}" --workers 8 --pose-out "$D/w8b.pose"
  run san1 "${BASE[@]}" --workers 1 --pose-out "$D/san1.pose" --sanity "$D/sanity_w1.csv" --scene-dump "$D/dump_w1.txt"
  run san8 "${BASE[@]}" --workers 8 --pose-out "$D/san8.pose" --sanity "$D/sanity_w8.csv" --scene-dump "$D/dump_w8.txt"
  run arm1 "${BASE[@]}" --workers 1 --pose-out "$D/arm1.pose" --arm-profiler --csv "$D/armed_w1.csv"
  run arm8 "${BASE[@]}" --workers 8 --pose-out "$D/arm8.pose" --arm-profiler --csv "$D/armed_w8.csv"
  rcs=$(cat "$D"/*.rc | sort -u | tr '\n' ' ')
  [ "$rcs" = "0 " ] && check "$P exits" PASS "all 0" || check "$P exits" FAIL "exit codes: $rcs"
  ok=PASS; det=""
  for w in 2 4 8 16; do cmp -s "$D/w1.pose" "$D/w$w.pose" || { ok=FAIL; det="$det W$w"; }; done
  check "$P W-identity 1/2/4/8/16" $ok "$(hash_of "$D/w1.out")${det:+ differs at$det}"
  ok=PASS; cmp -s "$D/w1.pose" "$D/w1b.pose" || ok=FAIL; cmp -s "$D/w8.pose" "$D/w8b.pose" || ok=FAIL
  check "$P determinism W1x2 W8x2" $ok "$(hash_of "$D/w1b.out") $(hash_of "$D/w8b.out")"
  ok=PASS; cmp -s "$D/w1.pose" "$D/san1.pose" || ok=FAIL; cmp -s "$D/w8.pose" "$D/san8.pose" || ok=FAIL
  check "$P sanity changes no bit" $ok "$(hash_of "$D/san1.out") $(hash_of "$D/san8.out")"
  ok=PASS; cmp -s "$D/w1.pose" "$D/arm1.pose" || ok=FAIL; cmp -s "$D/w8.pose" "$D/arm8.pose" || ok=FAIL
  check "$P armed pose" $ok "$(hash_of "$D/arm1.out") $(hash_of "$D/arm8.out")"
  v1=$(voids_of "$D/arm1.out"); v8=$(voids_of "$D/arm8.out")
  [ "$v1" = 0 ] && [ "$v8" = 0 ] && check "$P armed void_steps" PASS "0 0" || check "$P armed void_steps" FAIL "$v1 $v8: $(grep -h VOID "$D/arm1.out" "$D/arm8.out" | head -2 | tr '\n' ' ')"
  r=$(python "$G/compare_dumps.py" "$D/dump_w1.txt" "$OUT/canon/$P.dump"); rc=$?
  [ $rc = 0 ] && check "$P dump = canonical (W1)" PASS "$r" || check "$P dump = canonical (W1)" FAIL "$r"
  cmp -s "$D/dump_w1.txt" "$D/dump_w8.txt" && check "$P dump W1 = W8" PASS "" || check "$P dump W1 = W8" FAIL ""
  r=$(python "$G/dyn_sanity.py" --program "$P" "$D/sanity_w1.csv" --json "$D/bar_w1.json" > "$OUT/.last" 2>&1; echo $? > "$OUT/.rc"; head -1 "$OUT/.last"); rc=$(cat "$OUT/.rc")
  [ $rc = 0 ] && check "$P bar B1-B6 + premise (W1)" PASS "$r" || check "$P bar B1-B6 + premise (W1)" FAIL "$r"
  r=$(python "$G/dyn_sanity.py" --final "$P" "$D/w1.pose" > "$OUT/.last" 2>&1; echo $? > "$OUT/.rc"; head -1 "$OUT/.last"); rc=$(cat "$OUT/.rc")
  [ $rc = 0 ] && check "$P final B1-B3 (W1 pose)" PASS "$r" || check "$P final B1-B3 (W1 pose)" FAIL "$r"
  counts=$(grep -o '"kicks":[0-9]*,"launches":[0-9]*,"readback_mismatches":[0-9]*,"bad_statics":[0-9]*,"bodies":[0-9]*' "$D/san1.out")
  case $P in
    none)  want='"kicks":0,"launches":0,"readback_mismatches":0,"bad_statics":0,"bodies":1240' ;;
    kick)  want='"kicks":1488,"launches":0,"readback_mismatches":0,"bad_statics":0,"bodies":1240' ;;
    shoot) want='"kicks":0,"launches":60,"readback_mismatches":0,"bad_statics":0,"bodies":1300' ;;
    slide) want='"kicks":0,"launches":0,"readback_mismatches":0,"bad_statics":0,"bodies":1240' ;;
  esac
  [ "$counts" = "$want" ] && check "$P counts" PASS "$counts" || check "$P counts" FAIL "$counts want $want"
  if [ "$P" = kick ]; then
    run pre_dyn --scene jolt --dyn kick --steps 200 --workers 1 --pose-out "$D/pre_dyn.pose"
    run pre_jt --scene jolt --steps 200 --workers 1 --pose-out "$D/pre_jt.pose"
    cmp -s "$D/pre_dyn.pose" "$D/pre_jt.pose" && check "kick prefix [0,200) = J-T" PASS "$(hash_of "$D/pre_dyn.out")" \
      || check "kick prefix [0,200) = J-T" FAIL "$(hash_of "$D/pre_dyn.out") vs $(hash_of "$D/pre_jt.out")"
  fi
done
cat "$T"
echo "checks $(wc -l < "$T"), pass $(grep -c "	PASS	" "$T"), fail $(grep -c "	FAIL	" "$T")"
