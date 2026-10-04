"""Window 8b, F3 group: every runner comparison under the three readings of "every clean block" (read-only on raw/).

Reuses rc_decisive.py's selection and statistics (imported; that module prints its own D1-D3 report first, which this
script's output also carries). Per process: armed rows = median over steps [100,500) of the span (t_q, t_b, the per-step
sums t_qb and the four-span); unarmed rows = mean of wall_ns over [0,500). Readings: LETTER (every non-voided pass
gates, no flag at K<3), K2 (every non-voided pass gates, K=2 judged on its own i and s), CLEAN (the analyst's: only
passes with K=3 on both sides gate). Prints where the three readings differ.
"""
import contextlib
import io
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
with contextlib.redirect_stdout(io.StringIO()):
    import rc_decisive as D  # noqa: E402

SPANS4 = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')
_cache = {}


def cols(r):
    k = r['cwd']
    if k not in _cache:
        import csv
        with open(os.path.join(k, 'run.csv'), newline='', encoding='utf-8') as f:
            rd = csv.reader(f)
            head = [h.strip() for h in next(rd)]
            data = {h: [] for h in head}
            for row in rd:
                for h, v in zip(head, row):
                    data[h].append(float(v) if v.strip() else None)
        _cache[k] = data
    return _cache[k]


def metric(r, m):
    c = cols(r)
    steps = [int(s) for s in c['step']]
    if m == 'wall':
        return statistics.mean(v for s, v in zip(steps, c['wall_ns']) if 0 <= s < 500) / 1e6  # ms
    if m == 't_q':
        per = c['phys_bp_query_ns']
    elif m == 't_b':
        per = c['phys_bp_build_ns']
    elif m == 't_qb':
        per = [a + b for a, b in zip(c['phys_bp_query_ns'], c['phys_bp_build_ns'])]
    elif m == 'four':
        per = [sum(x) for x in zip(*(c[s] for s in SPANS4))]
    return statistics.median(v for s, v in zip(steps, per) if 100 <= s < 500) / 1000.0  # us


def cells(row, w, m):
    per = {p: [] for p in (0, 1, 2)}
    for (p, rnd, rw, ww), r in D.used.items():
        if rw == row and ww == w:
            per[p].append(metric(r, m))
    out = {p: D.cell(v) for p, v in per.items() if v}
    out['pooled'] = D.cell([x for v in per.values() for x in v])
    return out


def verdict(a, b, kmin, gating):
    cs = [('pooled', D.cmp_(a['pooled'], b['pooled'], kmin))] + [(f'p{p}', D.cmp_(a[p], b[p], kmin)) for p in gating]
    d = cs[0][1]['sign']
    claimed = all(c['i'] and c['s'] and c['sign'] == d for _, c in cs)
    strong = claimed and all(c['r'] and c['sign'] == d for _, c in cs)
    v = ('STRONG CLAIMED' if strong else 'CLAIMED' if claimed else 'NOT CLAIMED')
    if claimed:
        v += ' lower' if d < 0 else ' higher'
    return v


comps = []
for m in ('t_q', 't_b', 't_qb', 'four'):
    for w in (1, 8):
        comps.append((f'TD {m} W{w} kd/leaflist', 'F3-TD-armed-leaflist', 'F3-TD-armed-kd', w, m))
    comps.append((f'TA {m} W1 kd/leaflist', 'F3-TA-armed-leaflist', 'F3-TA-armed-kd', 1, m))
comps.append(('TR t_q W1 leaflist/rowwalk', 'F3-TR-armed', 'F3-TD-armed-leaflist', 1, 't_q'))
comps.append(('TR t_q W1 kd/rowwalk', 'F3-TR-armed', 'F3-TD-armed-kd', 1, 't_q'))
for fam in ('JT', 'RT'):
    for w in (1, 2, 4, 8, 16):
        comps.append((f'{fam} wall W{w} kd/leaflist', f'F3-{fam}-leaflist', f'F3-{fam}-kd', w, 'wall'))

print('comparison | B/A pooled | n A/B | K per pass A | K per pass B | flags r/i/s per pass (K>=3 rule) | '
      'LETTER | K2 | CLEAN')
differ = []
for name, ra, rb, w, m in comps:
    a, b = cells(ra, w, m), cells(rb, w, m)
    full = [p for p in (0, 1, 2) if a[p]['K'] >= 3 and b[p]['K'] >= 3]
    vl = verdict(a, b, 3, [0, 1, 2])
    vk = verdict(a, b, 2, [0, 1, 2])
    vc = verdict(a, b, 3, full)
    fl = ' '.join(f'p{p} {D.flags(D.cmp_(a[p], b[p], 3))}' for p in (0, 1, 2)) + \
        f" pooled {D.flags(D.cmp_(a['pooled'], b['pooled'], 3))}"
    ratio = b['pooled']['m'] / a['pooled']['m']
    print(f"{name} | {ratio:.4f} ({100 * (ratio - 1):+.2f} %) | {a['pooled']['K']}/{b['pooled']['K']} | "
          f"{'/'.join(str(a[p]['K']) for p in (0, 1, 2))} | {'/'.join(str(b[p]['K']) for p in (0, 1, 2))} | {fl} | "
          f"{vl} | {vk} | {vc}")
    if len({vl, vk, vc}) > 1:
        differ.append(name)
print(f'\ncomparisons where the readings differ: {len(differ)}: {differ}')
