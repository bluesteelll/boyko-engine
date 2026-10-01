"""Two small post hoc statements of the analyst: the [0,100) ladder8-F2 pass-1 cell range; armed pre-flight T(8) vs P(8)."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vn_lib import (W8B, load_block, validate, dirty, wmean, summary_of)  # noqa: E402

recs = [r for r in load_block('S7-AB') if r.get('timed')]
sel = {}
for r in recs:
    if r['attempt'] == 'original' and not dirty(r):
        sel[(r['pass'], r['round'], r['row'], r['binary'], r['W'])] = validate(r)[2]
for r in recs:
    k = (r['pass'], r['round'], r['row'], r['binary'], r['W'])
    if k not in sel and r['attempt'] == 'rerun':
        sel[k] = validate(r)[2]
for row in ('S7-ladder8-F2', 'S7-JT'):
    xs = sorted(wmean(sel[(1, rd, row, 's7p', 8)], 0, 100) / 1e6 for rd in (0, 1, 2))
    m = xs[1]
    print(f'[0,100) p1 {row}#s7p@W8: {[round(x, 4) for x in xs]} range {100 * (xs[-1] - xs[0]) / m:.2f} %')
g = os.path.join(W8B, 'gate', 's7')
for R in ('JT', 'JA'):
    for W in (8, 16):
        v = {}
        for B in ('P', 'T'):
            s = summary_of(open(os.path.join(g, f'{B}-{R}-W{W}-armed', 'stdout.txt'), encoding='utf-8', errors='replace').read())
            v[B] = s['window_mean_ns'] / 1e6
        print(f'armed pre-flight {R} W{W}: P {v["P"]:.4f} T {v["T"]:.4f} T/P {v["T"] / v["P"]:.4f} (n = 1 each)')
