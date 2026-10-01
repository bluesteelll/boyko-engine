"""G1: C4-G4 (g4rT, K 9 = 3 passes x 3 rounds) - all_pairs/tree per family and size 96..160 under ruling 1, and the
recipe as pre-registered (rows9a.json lane_rules C4-G4 = window9_c4_rows.json C4-G4 rule = cut.md 5 block 2 = the
window-8b recipe, f3.md:113-114):
  LO = the largest n at which all_pairs is NOT claimed slower than tree; HI = the smallest n at which tree is claimed
  faster; per family; TREE_BRUTE_MAX_ROWS = min LO; AUTO_TREE_LO/HI = (min LO, max HI); LO >= T.
  Bottom edge: LO < 96 -> report, keep 128/136, extend down. Top edge: LO >= 160, or no HI in the grid -> report,
  keep 128/136 marked NOT RE-READ, extend up. Never set a constant from an edge.
"all_pairs claimed slower" and "tree claimed faster" are the same event: ruling 1 CLAIMED on all_pairs/tree with
ratio > 1 (the window-8b g4.py reading). The window-7 rule (r AND s; the rule 144/152 were set under) is printed beside,
deciding nothing."""
import json
import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_g4 as G  # noqa: E402

L = G.L
out = L.Out()
recs = G.all_records()
procs, used, dropped = G.select('C4-G4', recs)
assert not dropped and len(used) == 9, (len(used), len(dropped))
out('C4-G4: %d used processes (3 per pass), 0 dropped; per-process value = Flat mean of times/iters (us)' % len(used))
out('A = tree, B = all_pairs; ratio = all_pairs/tree (> 1: tree faster); flags i/s/r per pass and pooled')
out('')


def w7(j):
    """window-7 rule beside: r AND s in every pass and pooled, one sign."""
    per = [j['per'][k] for k in j['gates']]
    sign = 1 if j['pooled']['ratio'] > 1 else -1
    same = all((1 if c['ratio'] > 1 else -1) == sign for c in per)
    return same and j['pooled']['cl_r'] and j['pooled']['cl_s'] and all(c['cl_r'] and c['cl_s'] for c in per)


RES = {}
for fam in G.FAMS:
    out('## %s' % fam)
    rows = []
    for n in G.SIZES:
        a = G.by_pass(used, G.gid(fam, 'tree', n))
        b = G.by_pass(used, G.gid(fam, 'all_pairs', n))
        j = L.judge(a, b, 'LETTER')
        slower = j['claimed'] and j['pooled']['ratio'] > 1
        faster_ap = j['claimed'] and j['pooled']['ratio'] < 1
        rows.append((n, j, slower, faster_ap, w7(j) and j['pooled']['ratio'] > 1))
        p = j['pooled']
        out('- n=%d (rows %d): all_pairs %s | tree %s' % (n, n + G.ROWS_EXTRA[fam], G.fc(j['b']), G.fc(j['a'])))
        out('    all_pairs/tree %.4f (%+.2f %%, all_pairs - tree %+.3f us); pooled bars 2i %.2f / 2s %.2f / 2r %.2f %%; '
            'flags pooled %s, p0 %s, p1 %s, p2 %s; min-max separated pooled %s -> %s%s [w7 rule: %s]' % (
                p['ratio'], 100 * (p['ratio'] - 1), p['delta'], 100 * p['bar_i'], 100 * p['bar_s'], 100 * p['bar_r'],
                L.yn(p), L.fper(j['per'][0]), L.fper(j['per'][1]), L.fper(j['per'][2]), p['minmax_sep'],
                G.verdict(j), (' all_pairs slower = tree faster' if slower else
                               (' all_pairs FASTER' if faster_ap else '')),
                'claimed' if w7(j) else 'not claimed'))
    rec = {}
    for rule, idx in (('ruling1', 2), ('w7', 4)):
        s = [(x[0], x[idx]) for x in rows]
        lo = max((n for n, x in s if not x), default=None)
        hi = min((n for n, x in s if x), default=None)
        mono = hi is not None and all((n >= hi) == x for n, x in s)
        rec[rule] = {'LO': lo, 'HI': hi, 'monotone': mono, 'flags': {n: x for n, x in s}}
    cross = None
    for (n0, j0, *_), (n1, j1, *_) in zip(rows, rows[1:]):
        r0, r1 = j0['pooled']['ratio'], j1['pooled']['ratio']
        if r0 <= 1 < r1:
            y0, y1 = math.log(r0), math.log(r1)
            cross = math.exp(math.log(n0) - y0 * (math.log(n1) - math.log(n0)) / (y1 - y0))
            break
    rec['loglog'] = cross
    rec['ratios'] = {n: j['pooled']['ratio'] for n, j, *_ in rows}
    rec['delta_us'] = {n: j['pooled']['delta'] for n, j, *_ in rows}
    rec['ap_us'] = {n: j['b']['median'] for n, j, *_ in rows}
    rec['tree_us'] = {n: j['a']['median'] for n, j, *_ in rows}
    rec['verdict'] = {n: G.verdict(j) + (' ' + j['dir']) for n, j, *_ in rows}
    rec['all_pairs_faster'] = [n for n, j, s_, f_, _ in rows if f_]
    RES[fam] = rec
    out('  => %s: ruling 1 LO %s / HI %s (monotone %s); window-7 rule LO %s / HI %s (monotone %s); log-log crossover '
        '(pooled medians) %s; sizes where all_pairs is CLAIMED FASTER: %s' % (
            fam, rec['ruling1']['LO'], rec['ruling1']['HI'], rec['ruling1']['monotone'], rec['w7']['LO'],
            rec['w7']['HI'], rec['w7']['monotone'], '%.1f' % cross if cross else 'none in grid',
            rec['all_pairs_faster']))
    out('')
out('## The recipe (ruling 1, the headline; window-7 rule beside)')
FINAL = {}
for rule in ('ruling1', 'w7'):
    los = {f: RES[f][rule]['LO'] for f in G.FAMS}
    his = {f: RES[f][rule]['HI'] for f in G.FAMS}
    edges = []
    for f in G.FAMS:
        if los[f] is None:
            edges.append('%s: LO < 96 (bottom edge)' % f)
        elif los[f] >= 160:
            edges.append('%s: LO >= 160 (top edge)' % f)
        if his[f] is None:
            edges.append('%s: no HI in the grid (top edge)' % f)
    if edges:
        res = {'edge': edges, 'TREE_BRUTE_MAX_ROWS': G.PROV['TREE_BRUTE_MAX_ROWS'],
               'AUTO_TREE_LO': G.PROV['AUTO_TREE_LO'], 'AUTO_TREE_HI': G.PROV['AUTO_TREE_HI'],
               'status': 'EDGE: keep the provisional 128/136 (NOT RE-READ if top edge)'}
    else:
        t = min(los.values())
        res = {'edge': [], 'TREE_BRUTE_MAX_ROWS': t, 'AUTO_TREE_LO': t, 'AUTO_TREE_HI': max(his.values()),
               'status': 'RE-READ'}
        res['LO>=T'] = res['AUTO_TREE_LO'] >= res['TREE_BRUTE_MAX_ROWS']
        res['LO<HI'] = res['AUTO_TREE_LO'] < res['AUTO_TREE_HI']
    res['LO'] = los
    res['HI'] = his
    FINAL[rule] = res
    out('- %s: LO %s, HI %s -> %s: TREE_BRUTE_MAX_ROWS %s, AUTO_TREE_LO/HI %s/%s%s; const-asserts LO >= T %s, LO < HI %s' % (
        rule, los, his, res['status'], res['TREE_BRUTE_MAX_ROWS'], res['AUTO_TREE_LO'], res['AUTO_TREE_HI'],
        ('; edges %s' % res['edge']) if res['edge'] else '', res.get('LO>=T'), res.get('LO<HI')))
f1 = FINAL['ruling1']
same = all(f1[k] == G.PROV[k] for k in G.PROV)
out('- provisional 128/136 %s by the ruling-1 reading' % ('STANDS' if same else 'is REPLACED by %d / %d/%d' % (
    f1['TREE_BRUTE_MAX_ROWS'], f1['AUTO_TREE_LO'], f1['AUTO_TREE_HI'])))
out('')
out('## Per-step arithmetic: all_pairs - tree (pooled medians, us per broadphase step) per size')
for f in G.FAMS:
    out('- %s: ' % f + '; '.join('%d %+.3f (ratio %.4f)' % (n, RES[f]['delta_us'][n], RES[f]['ratios'][n])
                                 for n in G.SIZES))
out('- one grid step (8 rows) in the band 120..144, the slope of (all_pairs - tree) per 8 rows: ' + '; '.join(
    '%s %s' % (f, ' '.join('%d->%d %+.3f' % (n0, n1, RES[f]['delta_us'][n1] - RES[f]['delta_us'][n0])
                           for n0, n1 in zip(G.SIZES, G.SIZES[1:]) if 112 <= n0 <= 144)) for f in G.FAMS))
t_new = f1['TREE_BRUTE_MAX_ROWS']
t_old = G.PROV['TREE_BRUTE_MAX_ROWS']
lo_, hi_ = sorted((t_new, t_old))
band = [n for n in G.SIZES if lo_ < n <= hi_]
if band:
    out('- rows (%d, %d] change path between the provisional T %d and the re-read T %d; at the grid sizes in that band, '
        'brute minus tree: %s' % (lo_, hi_, t_old, t_new, '; '.join(
            '%s %s' % (f, ' '.join('%d %+.3f us' % (n, RES[f]['delta_us'][n]) for n in band)) for f in G.FAMS)))
else:
    out('- the re-read T equals the provisional T: no row changes path (difference 0 us per step by construction)')
out('- for scale: at the crossover sizes the whole broadphase step is ~10-15 us (the cells above); the J pyramid '
    '(1,240 rows) is far above every threshold read here, so no gated runner row changes path at any value in 96..160')
json.dump({'RES': {f: {k: (v if not isinstance(v, dict) else {str(a): b for a, b in v.items()})
                       for k, v in r.items()} for f, r in RES.items()},
           'FINAL': {k: {a: (b if not isinstance(b, dict) else {str(x): y for x, y in b.items()})
                         for a, b in v.items()} for k, v in FINAL.items()}},
          open(os.path.join(G.HERE, 'g1_thresholds.json'), 'w'), indent=1, default=str)
out.save('g1_thresholds.txt')
