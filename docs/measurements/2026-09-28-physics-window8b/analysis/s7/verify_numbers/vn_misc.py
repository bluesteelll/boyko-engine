"""Misc cross-checks: my per-process [0,500) means vs the driver's mean_ms; main_cycle_frac by cell; W1 paired."""
import collections
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vn_lib import (load_block, validate, dirty, wmean)  # noqa: E402

recs = [r for r in load_block('S7-AB') if r.get('timed')]
sel = {}
info = {}
for r in recs:
    why, s, rows = validate(r)
    info[(r['pass'], r['round'], r['row'], r['binary'], r['W'], r['attempt'])] = (r, why, dirty(r), rows)
for k, v in info.items():
    if k[5] == 'original' and not v[1] and not v[2]:
        sel[k[:5]] = v
for k, v in info.items():
    if k[:5] not in sel and k[5] == 'rerun' and not v[1]:
        sel[k[:5]] = v
eq = sum(1 for (r, _, _, rows) in sel.values() if abs(wmean(rows, 0, 500) / 1e6 - r['mean_ms']) < 1e-9)
print(f'my [0,500) mean == driver mean_ms: {eq}/{len(sel)}')
mf = collections.defaultdict(list)
for sk, (r, _, _, rows) in sel.items():
    mf[(sk[2], sk[3], sk[4])].append(r['placement'].get('main_cycle_frac'))
meds = {k: statistics.median(v) for k, v in mf.items()}
allv = [x for v in mf.values() for x in v]
print(f'main_cycle_frac: cell medians {min(meds.values()):.4f}..{max(meds.values()):.4f}; per process {min(allv):.4f}..{max(allv):.4f}')
rat = []
for p in (0, 1, 2):
    for rd in (0, 1, 2):
        t = wmean(sel[(p, rd, 'S7-JT-W1', 's7t', 1)][3], 0, 500)
        pp = wmean(sel[(p, rd, 'S7-JT-W1', 's7p', 1)][3], 0, 500)
        rat.append(t / pp)
print(f'paired W1 T/P median {statistics.median(rat):.4f} [{min(rat):.4f}-{max(rat):.4f}], T slower {sum(x > 1 for x in rat)}/9')
