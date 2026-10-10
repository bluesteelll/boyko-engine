"""o2: omega(W, gap) with the N4 participation receipt (rows omega-wgap-worker / -external, omega2 exe, v1 omega mode).
Per process: scope_ns_median, scope_ns_mean, helped_reps (of reps), first_helper_ns_median per (W, gap).
Cells over K; N4's lower bound omega(80) - omega(0) and the awake-baseline form omega(80) - omega(5), each judged by
ruling 1 (i AND s pooled and in every pass). Window 8's cells recomputed from win8/raw (post hoc continuity)."""
import json
import os
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
procs = L.all_procs()
used, dropped, slots = L.select(procs)
rows = {'worker': 'omega-wgap-worker', 'external': 'omega-wgap-external'}


def val(route, W, gap, key, passes=None):
    xs = []
    for p in used:
        if p['row'] != rows[route] or (passes is not None and p['pass'] not in passes):
            continue
        for s in p['summaries']:
            if s['workers'] == W and s['gap_us'] == gap:
                xs.append(s[key])
    return xs


O('# o2 omega(W, gap), 32 zero-work tasks after a gap; ns; cell = median over K processes of the per-process SUMMARY value')
for route in ('worker', 'external'):
    for W in (8, 16):
        O('\n## route %s, W%d' % (route, W))
        cs = {}
        for gap in (0, 5, 20, 80):
            cs[gap] = L.cell(val(route, W, gap, 'scope_ns_median'))
            cm = L.cell(val(route, W, gap, 'scope_ns_mean'))
            hr = val(route, W, gap, 'helped_reps')
            reps = val(route, W, gap, 'reps')
            fh = [x for x in val(route, W, gap, 'first_helper_ns_median') if x is not None]
            cf = L.cell(fh)
            O('  gap %2d us: scope %s | mean-of-reps cell %.0f | helped_reps %s of %s | first_helper_ns_median %s | scope - first %.0f' % (
                gap, L.fc(cs[gap], 0), cm['median'], sorted(hr), sorted(set(reps)), L.fc(cf, 0),
                cs[gap]['median'] - cf['median']))
            O('           per pass scope medians: %s' % ' / '.join(
                '%.0f (K %d)' % (L.cell(val(route, W, gap, 'scope_ns_median', {q}))['median'],
                                 L.cell(val(route, W, gap, 'scope_ns_median', {q}))['K']) for q in (0, 1, 2)))
        for a, b, name in ((0, 80, 'N4 lower bound omega(80) - omega(0)'), (5, 80, 'omega(80) - omega(5) (gap 5 = awake baseline)'),
                           (5, 20, 'omega(20) - omega(5)'), (20, 80, 'omega(80) - omega(20)'), (0, 5, 'omega(5) - omega(0)')):
            pooled = L.cmp_(cs[a], cs[b])
            per = {q: L.cmp_(L.cell(val(route, W, a, 'scope_ns_median', {q})), L.cell(val(route, W, b, 'scope_ns_median', {q})))
                   for q in (0, 1, 2)}
            v, det = L.rule1(pooled, per)
            O('  %s: %s | %s -> %s' % (name, L.fcmp(pooled, 0), det, v))
        # first-helper latency by gap, judged the same way (post hoc)
        fc0 = {g: L.cell([x for x in val(route, W, g, 'first_helper_ns_median') if x is not None]) for g in (0, 5, 20, 80)}
        pooled = L.cmp_(fc0[5], fc0[80])
        per = {q: L.cmp_(L.cell([x for x in val(route, W, 5, 'first_helper_ns_median', {q}) if x is not None]),
                         L.cell([x for x in val(route, W, 80, 'first_helper_ns_median', {q}) if x is not None])) for q in (0, 1, 2)}
        v, det = L.rule1(pooled, per)
        O('  first_helper(80) - first_helper(5) (post hoc): %s | %s -> %s' % (L.fcmp(pooled, 0), det, v))
O.save('o2.txt')
