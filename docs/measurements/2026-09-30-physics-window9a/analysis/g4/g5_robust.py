"""G5: robustness of the recipe to the per-process statistic (post hoc; the pre-registered value is the Flat mean, g1).
Re-runs the ruling-1 recipe with (a) the driver est_ms point estimate (its own stdout parse, printed to 5 digits) and
(b) the per-process MEDIAN of the 10 samples (times/iters) instead of their mean. Independent of g1 code paths except
the lib9a statistics."""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_g4 as G  # noqa: E402

L = G.L
out = L.Out()
recs = G.all_records()
_, used, _ = G.select('C4-G4', recs)


def per_proc(p, i, how):
    if how == 'driver':
        return p['rec']['criterion']['est_ms'][i][1] * 1e3
    s = json.load(open(os.path.join(p['rec']['cwd'], 'criterion', *i.split('/'), 'new', 'sample.json'), encoding='utf-8'))
    xs = [t / n / 1e3 for t, n in zip(s['times'], s['iters'])]
    return statistics.median(xs) if how == 'median' else statistics.fmean(xs)


for how in ('mean', 'driver', 'median'):
    out('## per-process value = %s' % how)
    rec = {}
    for fam in G.FAMS:
        flags = []
        for n in G.SIZES:
            va, vb = {}, {}
            for p in used:
                va.setdefault(p['pass'], []).append(per_proc(p, G.gid(fam, 'tree', n), how))
                vb.setdefault(p['pass'], []).append(per_proc(p, G.gid(fam, 'all_pairs', n), how))
            j = L.judge(va, vb, 'LETTER')
            flags.append((n, j['claimed'] and j['pooled']['ratio'] > 1, j['pooled']['ratio'], G.verdict(j), j['dir']))
        lo = max(n for n, x, *_ in flags if not x)
        hi = min(n for n, x, *_ in flags if x)
        rec[fam] = (lo, hi)
        out('- %s: LO %d / HI %d; %s' % (fam, lo, hi, '; '.join('%d %.4f %s %s' % (n, r, v, d)
                                                              for n, x, r, v, d in flags)))
    out('  => TREE_BRUTE_MAX_ROWS %d, AUTO_TREE_LO/HI %d/%d' % (
        min(v[0] for v in rec.values()), min(v[0] for v in rec.values()), max(v[1] for v in rec.values())))
out.save('g5_robust.txt')
