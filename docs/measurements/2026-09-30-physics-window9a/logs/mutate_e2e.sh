#!/usr/bin/env bash
# Mutation proof for ruling 8's rehearsals: each mutated copy of the driver must turn at least one e2e case red.
cd "$(dirname "$0")/.."
run() {  # name sed-expr
  cp tools/window9a_run.py tools/_mut_window9a_run.py
  python -B - "$2" "$3" <<'PY'
import sys
p='tools/_mut_window9a_run.py'
b=open(p,encoding='utf-8',newline='').read()
old,new=sys.argv[1],sys.argv[2]
assert b.count(old)==1,(old,b.count(old))
open(p,'w',encoding='utf-8',newline='').write(b.replace(old,new))
PY
  [ $? -eq 0 ] || { echo "MUTATION $1: could not apply"; return; }
  python -B tools/selftest9a.py --driver tools/_mut_window9a_run.py > logs/mut_$1.txt 2>&1
  echo "MUTATION $1: exit $? ; failing e2e cases: $(grep -c '^FAIL e2e' logs/mut_$1.txt) ; other failing: $(grep '^FAIL' logs/mut_$1.txt | grep -vc 'e2e')"
  rm -f tools/_mut_window9a_run.py
}
run waves1 "RERUN_MAX_WAVES = 4" "RERUN_MAX_WAVES = 1"
run gatesTrue "'gates': c['clean'] >= MIN_K_GATE" "'gates': True"
run rerunall "return sorted(((k, s) for k, s in slots.items() if slot_bad(s['rec'])), key=lambda kv: kv[1]['seq'])" "return sorted(((k, s) for k, s in slots.items() if slot_bad(s['rec']) or s['n_rerun'] < 1), key=lambda kv: kv[1]['seq'])"
run nounclean_counted "        if slot_bad(s['rec']) is None:
            c['clean'] += 1
        else:
            c['unclean_rounds'].append(rnd)" "        c['clean'] += 1"
echo done
