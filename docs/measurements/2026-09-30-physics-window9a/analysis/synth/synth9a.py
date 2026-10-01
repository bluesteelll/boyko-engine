"""Window 9a synthesis library (results-analyst, 2026-09-30). Read-only on raw/, rows9a*.json, bin/, gate/.

It reuses the verified group libraries instead of re-implementing them:
  - analysis/c4ab/lib9a.py  (C4-AB and C4-G5 runner + Jolt processes; per-process value = mean wall in ms over a
    window; the ruling-8 slot rule; 8b synthlib arithmetic; readings LETTER and GATING-ONLY),
  - analysis/rapier/lib_rp.py (C4-RAPIER processes, V1-V9-shaped re-check; per-process value in ns, converted here).
GATING-ONLY (lib9a) and EXCL (lib_rp) are the same reading: only passes where both pass-cells have K >= 3 gate.
Only passes with a pass_done record are used (PREP.md)."""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W9A = os.path.dirname(os.path.dirname(HERE))
SCRATCH = os.path.dirname(W9A)
RAW = os.path.join(W9A, 'raw')
sys.path.insert(0, os.path.join(W9A, 'analysis', 'c4ab'))
sys.path.insert(0, os.path.join(W9A, 'analysis', 'rapier'))
import lib9a as L  # noqa: E402
import lib_rp as RP  # noqa: E402

WS = (1, 2, 4, 8, 16)
PASSES = (0, 1, 2)
WIN = {'0..500': (0, 500), '100..500': (100, 500), '0..100': (0, 100)}

# Work per step over [100,500) (row-iterations = points x 3 rows x 12 sweeps for ours; C4-AB a3_census.json).
JOLT_VROWS, JOLT_PROWS, JOLT_MANIF = 565790.0, 62224.0, 8489.0   # DOSSIER equal-work census (hash 0xb8522b4e3fc62cfe)
RAPIER_ROWS = {'RP-D': {'100..500': 354674.0, '0..100': 303003.0},
               'RP-M': {'100..500': 1071107.0, '0..100': 935729.0}}     # window9a_rows.md rows_pin


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(str(s))
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8', newline='\n').write('\n'.join(self.lines) + '\n')


_CACHE = {}


def load():
    if _CACHE:
        return _CACHE
    recs = L.all_records()
    _CACHE['recs'] = recs
    _CACHE['AB'] = L.select('C4-AB', recs)
    _CACHE['G5'] = L.select('C4-G5', recs)
    _CACHE['RAP'] = RP.select('C4-RAPIER', recs)
    return _CACHE


def vals(row, binary, W, win):
    """{pass: [per-process value in ms]} over the used processes of the block that carries the row."""
    d = load()
    out = {k: [] for k in PASSES}
    if row.startswith('RP-'):
        a, b = WIN[win]
        for p in d['RAP'][1]:
            if p['row'] == row and p['binary'] == binary and p['W'] == W:
                out[p['pass']].append(p['vals'][(a, b)] / 1e6)
    else:
        blk = 'G5' if row in ('C4-JD-armed', 'C4-JDap-armed', 'C4-R', 'C4-Rap', 'C4-S16', 'C4-S16ap') else 'AB'
        for p in d[blk][1]:
            if p['row'] == row and p['binary'] == binary and p['W'] == W:
                out[p['pass']].append(p['v'][win])
    return out


def pooled_cell(v):
    return L.cell([x for k in PASSES for x in v.get(k, [])])


def ks(v):
    return '/'.join(str(len(v.get(k, []))) for k in PASSES)


def fcell(v, d=4):
    c = pooled_cell(v)
    if c['K'] == 0:
        return 'n=0'
    return f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] n={c['K']} ({ks(v)})"


def ours_census():
    """Points / manifolds per step over [100,500) on C4-JD-armed#tip (C4-G5), every used process; must be one value."""
    d = load()
    s = set()
    for p in d['G5'][1]:
        if p['row'] == 'C4-JD-armed':
            c = p['c']
            s.add((round(L.mean(c['phys_np_points'][100:500]), 6), round(L.mean(c['phys_np_manifolds'][100:500]), 6)))
    assert len(s) == 1, s
    pts, man = next(iter(s))
    return pts, man, pts * 36.0


def verdict(j):
    return L.label(j)


def both(va, vb, scale=1.0):
    """LETTER and GATING-ONLY judgements of B against A (lib9a.judge)."""
    return L.judge(va, vb, 'LETTER', scale=scale), L.judge(va, vb, 'GATING-ONLY', scale=scale)
