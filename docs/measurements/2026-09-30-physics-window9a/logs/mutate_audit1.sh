#!/usr/bin/env bash
# Audit-1 mutation proof: every NEW rule of the audit-1 fixes (W1 allowance floor, W2 insert_after, O2 INCOMPLETE line) must turn at
# least one selftest case red when it is broken. Each mutation is applied to a COPY of the driver (tools/_mut_window9a_run.py) and the
# selftest is run against it with --driver (the pure cases load the copy too); the counts9a.py mutation is applied in place with a
# byte-checked restore. Prints, per mutation, the number of failing cases and the first failing names.
cd "$(dirname "$0")/.."
run() {  # name old new
  cp tools/window9a_run.py tools/_mut_window9a_run.py
  python -B - "$2" "$3" <<'PY'
import sys
p='tools/_mut_window9a_run.py'
b=open(p,encoding='utf-8',newline='').read()
old,new=sys.argv[1],sys.argv[2]
assert b.count(old)==1,(old,b.count(old))
open(p,'w',encoding='utf-8',newline='').write(b.replace(old,new))
PY
  [ $? -eq 0 ] || { echo "MUTATION $1: could not apply"; rm -f tools/_mut_window9a_run.py; return; }
  python -B tools/selftest9a.py --driver tools/_mut_window9a_run.py > logs/mut1_$1.txt 2>&1
  echo "MUTATION $1: selftest exit $? ; FAIL cases: $(grep -c '^FAIL' logs/mut1_$1.txt)"
  grep '^FAIL' logs/mut1_$1.txt | head -4 | cut -c1-170 | sed 's/^/    /'
  rm -f tools/_mut_window9a_run.py
}
# W1: the floor term dropped (the pre-fix allowance = the fraction alone)
run nofloor "return max(frac_s, floor_s), frac_s, floor_s" "return frac_s, frac_s, floor_s"
# W1: the loop does not use rerun_allowance (falls back to the pre-fix formula inline): only an END-TO-END case can see this
run loopfrac "allowance_s = rerun_allowance(block, rounds)[0]" "allowance_s = RERUN_STAGE_FRAC * rounds * block_estimate(block)['per_round_s']"
# W1: the admission test admits everything
run admitall "return elapsed_s + process_est_s + RECEIPT_S <= allowance_s" "return True"
# W1: the slack removed (the model's +-30 % no longer covered)
run slack1 "RERUN_EST_SLACK = 1.3
# --dry-run prints" "RERUN_EST_SLACK = 1.0
# --dry-run prints"
# W1: the wave cap lifted (the floor now decides alone)
run waves9 "RERUN_MAX_WAVES = 4" "RERUN_MAX_WAVES = 9"
# W2: insert_after ignored (the row is appended)
run anchorignored "rj['rows'].insert(ids[anchor] + 1, r)" "rj['rows'].append(r)"
# W2: insert_after inserts BEFORE the anchor
run anchorbefore "rj['rows'].insert(ids[anchor] + 1, r)" "rj['rows'].insert(ids[anchor], r)"
# O2: counts9a.py loses its INCOMPLETE rule (in place, restored byte for byte)
cp tools/counts9a.py logs/_counts9a.keep
python -B - <<'PY'
p='tools/counts9a.py'
b=open(p,encoding='utf-8',newline='').read()
old="incomplete = sorted((k for k in started if k not in closed), key=str)"
assert b.count(old)==1
open(p,'w',encoding='utf-8',newline='').write(b.replace(old,"incomplete = []"))
PY
python -B tools/selftest9a.py > logs/mut1_noincomplete.txt 2>&1
echo "MUTATION noincomplete (counts9a.py): selftest exit $? ; FAIL cases: $(grep -c '^FAIL' logs/mut1_noincomplete.txt)"
grep '^FAIL' logs/mut1_noincomplete.txt | head -4 | cut -c1-170 | sed 's/^/    /'
cp logs/_counts9a.keep tools/counts9a.py
cmp logs/_counts9a.keep tools/counts9a.py && echo "counts9a.py restored byte for byte" && rm -f logs/_counts9a.keep
echo done
