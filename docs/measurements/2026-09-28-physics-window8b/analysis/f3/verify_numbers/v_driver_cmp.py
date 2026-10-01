"""Compare (after the fact) my validity/cleanliness/means with the driver's record fields."""
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

recs = {(r['pass'], r['round'], r['row'], r['W'], r['attempt']): r for r in L.load_recs() if r.get('block') == 'F3' and r.get('row')}
P = json.load(open(os.path.join(L.HERE, 'v_procs.json')))['procs']
dv = dc = 0
dm = 0.0
for p in P:
    r = recs[(p['pass'], p['round'], p['row'], p['W'], p['attempt'])]
    if bool(r.get('valid')) != (not p['valid_why']):
        dv += 1
    if p['attempt'] != 'warmup' and bool(r.get('contaminated')) != bool(p['clean_why']):
        dc += 1
        print('  clean differs', p['pass'], p['round'], p['row'], p['W'], p['attempt'], r.get('contaminated'), p['clean_why'])
    dm = max(dm, abs(r['mean_ms'] - p['wall_ms']['0..500']))
print('valid flag differs:', dv, 'clean flag differs:', dc, 'max |mean_ms - mine| ms:', dm)
