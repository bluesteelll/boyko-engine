"""G3: post hoc only (no claim, no rule): (1) paired per-process all_pairs/tree; (2) pass-to-pass drift per arm;
(3) all_pairs cost per pair test n(n-1)/2 across the grid (the step between 120 and 128); (4) 9a against window 8b at
the shared sizes (different binaries and windows); (5) context from C4-BR (DIAGNOSTIC ONLY, ruling (b); another group
owns its reading): g4rT at 144 in BR against g4rT at 144 in G4, and all_pairs/tree at 144 per binary."""
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
out('## 1. Paired per-process all_pairs/tree (same process; n=9), against the ratio of cell medians (g1)')
for fam in G.FAMS:
    parts = []
    for n in G.SIZES:
        pr = sorted(p['v'][G.gid(fam, 'all_pairs', n)] / p['v'][G.gid(fam, 'tree', n)] for p in used)
        parts.append('%d %.4f [%.4f-%.4f]%s' % (n, statistics.median(pr), pr[0], pr[-1],
                                                ' (all > 1)' if pr[0] > 1 else (' (all < 1)' if pr[-1] < 1 else
                                                                                ' (straddles 1)')))
    out('- %s: %s' % (fam, '; '.join(parts)))
out('')
out('## 2. Pass medians per arm (us), p0 / p1 / p2 (drift across the 70-min block)')
for fam in G.FAMS:
    for arm in ('all_pairs', 'tree'):
        parts = []
        for n in G.SIZES:
            bp = G.by_pass(used, G.gid(fam, arm, n))
            parts.append('%d %s' % (n, '/'.join('%.3f' % statistics.median(bp[k]) for k in (0, 1, 2))))
        out('- %s %s: %s' % (fam, arm, '; '.join(parts)))
out('')
out('## 3. all_pairs per pair test, ns (median us / (rows*(rows-1)/2)); tree per row, ns')
for fam in G.FAMS:
    ap, tr = [], []
    for n in G.SIZES:
        rows = n + G.ROWS_EXTRA[fam]
        a = L.cell([p['v'][G.gid(fam, 'all_pairs', n)] for p in used])['median']
        t = L.cell([p['v'][G.gid(fam, 'tree', n)] for p in used])['median']
        ap.append('%d %.4f' % (n, 1e3 * a / (rows * (rows - 1) / 2)))
        tr.append('%d %.2f' % (n, 1e3 * t / rows))
    out('- %s all_pairs ns/test: %s' % (fam, '; '.join(ap)))
    out('- %s tree ns/row: %s' % (fam, '; '.join(tr)))
out('')
out('## 4. 9a (50e31f1a + g4ref, K 9) against window 8b (16191fda + g4ref, K 3): cell medians, 9a/8b per arm')
W8B = os.path.join(os.path.dirname(G.W9A), 'win8b', 'analysis', 'f3', 'g4.json')
g8 = json.load(open(W8B, encoding='utf-8'))['cells']
for fam in G.FAMS:
    for arm in ('all_pairs', 'tree'):
        parts = []
        for n in (128, 136, 144, 152, 160):
            c8 = g8['bp_g4_%s/%s/%d' % (fam, arm, n)]['median']
            c9 = L.cell([p['v'][G.gid(fam, arm, n)] for p in used])['median']
            parts.append('%d %.3f/%.3f = %.4f' % (n, c9, c8, c9 / c8))
        out('- %s %s: %s' % (fam, arm, '; '.join(parts)))
    parts = []
    for n in (128, 136, 144, 152, 160):
        r8 = g8['bp_g4_%s/all_pairs/%d' % (fam, n)]['median'] / g8['bp_g4_%s/tree/%d' % (fam, n)]['median']
        a9 = L.cell([p['v'][G.gid(fam, 'all_pairs', n)] for p in used])['median']
        t9 = L.cell([p['v'][G.gid(fam, 'tree', n)] for p in used])['median']
        parts.append('%d 8b %.4f -> 9a %.4f' % (n, r8, a9 / t9))
    out('- %s all_pairs/tree: %s' % (fam, '; '.join(parts)))
out('')
out('## 5. C4-BR context (DIAGNOSTIC ONLY, K 3 = one pass x 3 rounds, NOT claim-bearing; read here only at 144)')
br = [r for r in G.procs_of('C4-BR', recs)]
vals = {}
for r in br:
    d = r['cwd']
    why = []
    if r.get('exit') != 0 or not L.sha_ok(r):
        why.append('exit/sha')
    why += L.clean_why(r)
    v = {}
    for i in L.ROWS['C4-BR']['expect']:
        sp = os.path.join(d, 'criterion', *i.split('/'), 'new', 'sample.json')
        if os.path.exists(sp):
            s = json.load(open(sp, encoding='utf-8'))
            v[i] = statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3
    if len(v) != 8:
        why.append('ids %d' % len(v))
    if r.get('attempt') == 'original' and not why:
        vals.setdefault(r['binary'], []).append(v)
    out('- %s p%d r%d %s: valid+clean %s %s' % (r['binary'], r['pass'], r['round'], r['attempt'], not why, why))
for fam in G.FAMS:
    for b in ('g4r7', 'g4r8b', 'g4rT'):
        vs = vals.get(b, [])
        a = L.cell([v['bp_g4_%s/all_pairs/144' % fam] for v in vs])
        t = L.cell([v['bp_g4_%s/tree/144' % fam] for v in vs])
        c = L.cmp_(t, a)
        out('- %s 144 %s: all_pairs %.3f [%.3f-%.3f] tree %.3f [%.3f-%.3f] n=%d; all_pairs/tree %.4f flags %s' % (
            fam, b, a['median'], a['min'], a['max'], t['median'], t['min'], t['max'], a['K'], c['ratio'], L.yn(c)))
    r7 = L.cell([v['bp_g4_%s/all_pairs/144' % fam] for v in vals['g4r7']])
    rT = L.cell([v['bp_g4_%s/all_pairs/144' % fam] for v in vals['g4rT']])
    c = L.cmp_(r7, rT)
    out('  all_pairs g4rT/g4r7 at 144 (same window, adjacent processes): %.4f flags %s' % (c['ratio'], L.yn(c)))
    g4c = L.cell([p['v'][G.gid(fam, 'all_pairs', 144)] for p in used])
    g4t = L.cell([p['v'][G.gid(fam, 'tree', 144)] for p in used])
    out('  g4rT in BR vs g4rT in G4 at 144: all_pairs %.3f vs %.3f (%.4f); tree %.3f vs %.3f (%.4f)' % (
        rT['median'], g4c['median'], rT['median'] / g4c['median'],
        L.cell([v['bp_g4_%s/tree/144' % fam] for v in vals['g4rT']])['median'], g4t['median'],
        L.cell([v['bp_g4_%s/tree/144' % fam] for v in vals['g4rT']])['median'] / g4t['median']))
out.save('g3_posthoc.txt')
