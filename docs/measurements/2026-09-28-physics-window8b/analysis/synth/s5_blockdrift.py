"""Post hoc: the F3 block reads 4-11 % slower than S4-AB on the same binary and the same effective config (s1). What
the record says about it: per-block witness and receipt levels of the USED processes; per pass of F3-JT-leaflist; the
rank correlation, inside the F3 unarmed rows, between a process's wall relative to its cell median and its witness %."""
import statistics

import synthlib as L

O = L.Out()
recs = L.all_records()


def ranks(xs):
    s = sorted(range(len(xs)), key=lambda i: xs[i])
    r = [0] * len(xs)
    for k, i in enumerate(s):
        r[i] = k
    return r


def spearman(a, b):
    ra, rb = ranks(a), ranks(b)
    ma, mb = statistics.fmean(ra), statistics.fmean(rb)
    num = sum((x - ma) * (y - mb) for x, y in zip(ra, rb))
    den = (sum((x - ma) ** 2 for x in ra) * sum((y - mb) ** 2 for y in rb)) ** 0.5
    return num / den if den else float('nan')


O('# block-level machine state (used processes): witness % and receipts, median [max]')
for b in ('S4-AB', 'S7-AB', 'SPLIT', 'F3', 'J-Son-T'):
    _, used, _ = L.select(b, recs)
    wi = [p['rec']['others_busy_pct'] for p in used]
    rb = [p['rec']['receipt_before']['cpu_avg'] for p in used]
    ra = [p['rec']['receipt_after']['cpu_avg'] for p in used]
    O(f'  {b:8s} n={len(used):3d}: witness {statistics.median(wi):.2f} [{max(wi):.2f}] %, receipt before '
      f'{statistics.median(rb):.2f} [{max(rb):.2f}], after {statistics.median(ra):.2f} [{max(ra):.2f}]')

_, u3, _ = L.select('F3', recs)
O('\n# F3-JT-leaflist#tip per pass ([0,500) wall ms, used processes) vs S4-AB S4-JT#tip')
_, u4, _ = L.select('S4-AB', recs)
for W in (1, 8, 16):
    s4 = [statistics.fmean(p['c']['wall_ns'][0:500]) / 1e6 for p in u4 if p['row'] == 'S4-JT' and p['binary'] == 'tip'
          and p['W'] == W]
    by = {}
    for p in u3:
        if p['row'] == 'F3-JT-leaflist' and p['W'] == W:
            by.setdefault(p['pass'], []).append(statistics.fmean(p['c']['wall_ns'][0:500]) / 1e6)
    O(f'  W{W:2d}: S4-AB {statistics.median(s4):.4f} [{min(s4):.4f}-{max(s4):.4f}]; F3 per pass ' +
      '; '.join(f'p{k} {statistics.median(v):.4f} (K={len(v)})' for k, v in sorted(by.items())))

O('\n# inside F3 (unarmed JT/RT rows): process wall / its cell median vs witness % (Spearman, used processes)')
rel, wit = [], []
cells = {}
for p in u3:
    if p['row'].startswith(('F3-JT', 'F3-RT')):
        cells.setdefault((p['row'], p['W']), []).append(p)
for k, ps in cells.items():
    m = statistics.median(statistics.fmean(p['c']['wall_ns'][0:500]) for p in ps)
    for p in ps:
        rel.append(statistics.fmean(p['c']['wall_ns'][0:500]) / m)
        wit.append(p['rec']['others_busy_pct'])
O(f'  n={len(rel)}; Spearman rho(relative wall, witness %) = {spearman(rel, wit):+.3f}')
O.save('s5_blockdrift.txt')
