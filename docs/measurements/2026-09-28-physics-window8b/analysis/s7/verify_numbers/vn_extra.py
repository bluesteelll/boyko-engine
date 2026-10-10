"""Independent side checks for S7-AB: placement receipts, outlier census, paired per round, engagement receipt
recomputed from the armed pre-flight SUMMARYs, IQR-method sensitivity, leave-one-out robustness."""
import collections
import glob
import json
import math
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vn_lib import (W8B, load_block, validate, dirty, wmean, compare, yn, summary_of)  # noqa: E402

recs = [r for r in load_block('S7-AB') if r.get('timed')]
info = {}
for r in recs:
    why, s, rows = validate(r)
    info[(r['pass'], r['round'], r['row'], r['binary'], r['W'], r['attempt'])] = (r, why, dirty(r), rows)
sel = {}
for k, (r, why, dw, rows) in info.items():
    sk = k[:5]
    if k[5] == 'original' and not why and not dw:
        sel[sk] = (r, rows)
for k, (r, why, dw, rows) in info.items():
    sk = k[:5]
    if sk not in sel and k[5] == 'rerun' and not why:
        sel[sk] = (r, rows)
print('selected slots', len(sel))

# 1. placement receipts
np_ = 0
miss_est = []
for sk, (r, rows) in sel.items():
    pl = r.get('placement')
    if not pl or not pl.get('top3'):
        np_ += 1
    elif pl.get('main_share_top_est') is None:
        miss_est.append((sk, pl.get('main_cpu_s')))
print('selected processes without a placement receipt (no top3):', np_)
print('selected processes lacking main_share_top_est:', len(miss_est))
for m in miss_est:
    print('   ', m)
allp = [r for (r, why, dw, rows) in info.values()]
print('all timed records (incl. dirty originals) without placement:', sum(1 for r in allp if not (r.get('placement') or {}).get('top3')))

# 2. outlier census (> 1.5 % from the pooled cell median, [0,500))
cells = collections.defaultdict(list)
for sk, (r, rows) in sel.items():
    cells[(sk[2], sk[3], sk[4])].append((sk, wmean(rows, 0, 500) / 1e6, r))
out = []
for ck, xs in cells.items():
    m = statistics.median(v for _, v, _ in xs)
    for sk, v, r in xs:
        dev = v / m - 1
        if abs(dev) > 0.015:
            out.append((ck, sk[0], sk[1], round(100 * dev, 2), (r.get('receipt_before') or {}).get('cpu_avg'),
                        (r.get('receipt_after') or {}).get('cpu_avg'), r.get('others_busy_pct')))
print(f'processes > 1.5 % from their cell median: {len(out)}; max {max(o[3] for o in out)} %, min {min(o[3] for o in out)} %')
for o in sorted(out):
    print('   ', o)
print('cells and n:', {f'{k[0]}#{k[1]}@W{k[2]}': len(v) for k, v in sorted(cells.items())})

# 3. paired per round (T vs P adjacent), [0,500)
for row in ('S7-JT', 'S7-JA'):
    for W in (8, 16):
        rat = []
        for p in (0, 1, 2):
            for rd in (0, 1, 2):
                t = wmean(sel[(p, rd, row, 's7t', W)][1], 0, 500)
                pp = wmean(sel[(p, rd, row, 's7p', W)][1], 0, 500)
                tr = sel[(p, rd, row, 's7t', W)][0]
                pr = sel[(p, rd, row, 's7p', W)][0]
                rat.append((p, rd, t / pp, 'T first' if tr['seq'] < pr['seq'] else 'P first', tr['attempt']))
        slower = sum(1 for x in rat if x[2] > 1)
        print(f'paired {row} W{W}: T slower in {slower}/9; median T/P {statistics.median(x[2] for x in rat):.4f}; '
              f'min {min(x[2] for x in rat):.4f} max {max(x[2] for x in rat):.4f}')
        print('    ', [(x[0], x[1], round(x[2], 4), x[3], x[4]) for x in rat])

# 4. engagement receipt recomputed from armed pre-flight SUMMARYs
PHYS = 8
for R in ('JT', 'JA'):
    for W in (8, 16):
        vals = {}
        for Bk in ('P', 'T'):
            d = os.path.join(W8B, 'gate', 's7', f'{Bk}-{R}-W{W}-armed')
            s = summary_of(open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read())
            w = s['w8s']
            vals[Bk] = (s, w)
        for Bk, (s, w) in vals.items():
            keys = {k: w.get(k) for k in w if any(t in k for t in ('lanes', 'waves', 'color_scopes', 'color_tasks', 'setup_steps', 'setup_tasks'))}
            print(f'  armed {Bk}-{R}-W{W}: exit-free SUMMARY pose {s.get("pose_hash")} void {s.get("void_steps")} steps {s.get("steps")} armed {s.get("armed")} w8s keys {keys}')
