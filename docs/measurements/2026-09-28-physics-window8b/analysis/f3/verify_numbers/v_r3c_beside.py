"""Context: R3c walls over the beside windows [0,100) and [100,500) (not the pre-registered metric)."""
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

P = [p for p in json.load(open(os.path.join(L.HERE, 'v_procs.json')))['procs'] if p.get('used')]
for win in ('0..100', '100..500'):
    for fam in ('JT', 'RT'):
        for W in (1, 2, 4, 8, 16):
            def cc(row, ps=None):
                return L.cell([p['wall_ms'][win] for p in P if p['row'] == row and p['W'] == W and (ps is None or p['pass'] in ps)])
            A, B = f'F3-{fam}-leaflist', f'F3-{fam}-kd'
            pooled = L.cmp(cc(A), cc(B))
            passes = [L.cmp(cc(A, [k]), cc(B, [k]), 2) for k in (0, 1, 2)]
            slower_pooled = pooled['ratio'] > 1 and pooled['fl']['i'] and pooled['fl']['s']
            slower_k2 = slower_pooled and all(c['ratio'] > 1 and c['fl']['i'] and c['fl']['s'] for c in passes)
            print(f"{win} {fam} W{W}: kd/leaflist {pooled['ratio']:.4f} {L.yn(pooled)} pooled-slower(i&s) {slower_pooled} K2-every-pass {slower_k2}")
