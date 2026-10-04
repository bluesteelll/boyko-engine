"""Post hoc: do used re-runs (ruling 8's re-run stage, at the end of a pass) read slower than the used originals of the
same cell? Per used re-run: value / median of the cell's used originals (all passes), [0,500). Also per block C4-G5 and
C4-RAPIER is NOT read here (other groups). Plus: the same statistic for the originals (leave-one-out), as the control."""
import statistics

import lib9a as L

O = L.Out()
recs = L.all_records()
_, used, _ = L.select('C4-AB', recs)
cells = {}
for p in used:
    cells.setdefault((p['row'], p['binary'], p['W']), []).append(p)
rr, oo = [], []
for k, ps in cells.items():
    orig = [p['v']['0..500'] for p in ps if p['attempt'] == 'original']
    for p in ps:
        if p['attempt'] == 'rerun' and len(orig) >= 3:
            rr.append((p['v']['0..500'] / statistics.median(orig), k, p['pass'], p['round']))
        elif p['attempt'] == 'original':
            rest = [v for v in orig if v is not p['v']['0..500']]
            rest = list(orig)
            rest.remove(p['v']['0..500'])
            if len(rest) >= 3:
                oo.append(p['v']['0..500'] / statistics.median(rest))
x = sorted(r[0] for r in rr)
O(f'used re-runs with >= 3 used originals in the cell: n = {len(x)}; ratio to the originals median: median {statistics.median(x):.4f}, '
  f'quartiles {statistics.quantiles(x, n=4)[0]:.4f} / {statistics.quantiles(x, n=4)[2]:.4f}, > 1 in {sum(v > 1 for v in x)} of {len(x)}, '
  f'> 1.05 in {sum(v > 1.05 for v in x)}')
y = sorted(oo)
O(f'control, used originals (leave-one-out): n = {len(y)}; median {statistics.median(y):.4f}, quartiles '
  f'{statistics.quantiles(y, n=4)[0]:.4f} / {statistics.quantiles(y, n=4)[2]:.4f}, > 1 in {sum(v > 1 for v in y)} of {len(y)}, '
  f'> 1.05 in {sum(v > 1.05 for v in y)}')
by_pass = {}
for v, k, ps, rnd in rr:
    by_pass.setdefault(ps, []).append(v)
for ps in sorted(by_pass):
    O(f'  re-runs used in p{ps}: n {len(by_pass[ps])}, median ratio {statistics.median(by_pass[ps]):.4f}')
O.save('a6_rerun_bias.txt')
