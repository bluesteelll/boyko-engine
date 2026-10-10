"""C4-G5 broadphase stage profile on the TIP (50e31f1a): C4-JD-armed (Tree, the default) and C4-JDap-armed (AllPairs),
W 1 and 8, window [100,500) unless named. Per process: the MEDIAN over steps (the pre-registered G5 statistic, and F3's
t_q convention) and the MEAN over steps (8b's s6_extras convention behind 'bp query 0.2069 ms at W8'); cell = median
over K = 9 of the per-process values. Writes g5_profile.txt and g5_profile.json."""
import json
import os
import statistics

import lib_g5 as L

O = L.Out()
recs = L.all_records()
procs, used, dropped = L.select('C4-G5', recs)
RES = {}
ITEMS = [('verify', lambda p, f: f(p, 'phys_bp_verify_ns')), ('build', lambda p, f: f(p, 'phys_bp_build_ns')),
         ('query', lambda p, f: f(p, 'phys_bp_query_ns')), ('assemble', lambda p, f: f(p, 'phys_bp_assemble_ns'))]
O('# Broadphase stage profile, TIP 50e31f1a, [100,500), ms per step; cell = median over K of per-process values')
O('# columns: phys_bp_{verify,build,query,assemble}_ns, their sum S4 per step, the system span '
  'sys_physics_broadphase_colored_ns (SYS), OTHER = SYS - S4 per step, sys_select_broadphase_ns (SEL)')
for row in ('C4-JD-armed', 'C4-JDap-armed'):
    for W in (1, 8):
        ps = [p for p in used if p['row'] == row and p['W'] == W]
        O('\n## %s W%d (n=%d, passes %s)' % (row, W, len(ps), sorted({p['pass'] for p in ps})))
        res = {}
        for conv, f, fs, fo in (('median-over-steps', L.col_med, L.sum4_med, L.other_med),
                                ('mean-over-steps', L.col_mean, L.sum4_mean, L.other_mean)):
            O('  [%s]' % conv)
            d = {}
            for name, g in ITEMS:
                d[name] = L.cell([g(p, f) for p in ps])
            d['S4'] = L.cell([fs(p) for p in ps])
            d['SYS'] = L.cell([f(p, L.SYS) for p in ps])
            d['OTHER'] = L.cell([fo(p) for p in ps])
            d['SEL'] = L.cell([f(p, L.SEL) for p in ps])
            d['wall_armed'] = L.cell([f(p, 'wall_ns') for p in ps])
            for k, c in d.items():
                O('    %-10s %s' % (k, L.fcell(c, 4)))
            if d['S4']['median'] > 0:
                O('    shares (of cell medians): query/S4 %.1f %%, S4/SYS %.1f %%, SYS/wall %.1f %%, query/wall %.1f %%' % (
                    100 * d['query']['median'] / d['S4']['median'], 100 * d['S4']['median'] / d['SYS']['median'],
                    100 * d['SYS']['median'] / d['wall_armed']['median'],
                    100 * d['query']['median'] / d['wall_armed']['median']))
            else:
                O('    shares: S4 = 0 (the AllPairs path runs none of the four tree stages); SYS/wall %.1f %%' % (
                    100 * d['SYS']['median'] / d['wall_armed']['median']))
            res[conv] = d
        # [0,100) context for S4 and query (median over steps)
        res['S4_0_100'] = L.cell([L.sum4_med(p, 0, 100) for p in ps])
        res['query_0_100'] = L.cell([L.col_med(p, 'phys_bp_query_ns', 0, 100) for p in ps])
        res['SYS_0_100'] = L.cell([L.col_med(p, L.SYS, 0, 100) for p in ps])
        O('  [0,100) median-over-steps: S4 %s; query %s; SYS %s' % (
            L.fcell(res['S4_0_100'], 4), L.fcell(res['query_0_100'], 4), L.fcell(res['SYS_0_100'], 4)))
        # row-iteration census (counters, per step, [100,500))
        cen = {}
        for k in ('phys_bp_queried', 'phys_bp_members', 'phys_bp_rebuilds', 'phys_bp_pairs', 'manifolds', 'pairs'):
            vals = sorted({x for p in ps for x in p['c'][k][100:500]})
            meds = [statistics.median(p['c'][k][100:500]) for p in ps]
            cen[k] = {'distinct': vals if len(vals) <= 6 else [vals[0], '...', vals[-1]], 'median': statistics.median(meds)}
            O('  census %-18s median/step %s; distinct values over [100,500) %s' % (k, cen[k]['median'], cen[k]['distinct']))
        steps_rebuild = sorted({i for p in ps for i, x in enumerate(p['c']['phys_bp_rebuilds']) if x})
        O('  census: steps with phys_bp_rebuilds > 0 over [0,500): %s' % steps_rebuild)
        if res['median-over-steps']['query']['median'] > 0:
            q = cen['phys_bp_queried']['median']
            cq = L.cell([L.col_med(p, 'phys_bp_query_ns') * 1e6 / statistics.median(p['c']['phys_bp_queried'][100:500])
                         for p in ps])
            O('  c_q = t_q / queried rows (per process, median-over-steps t_q): %s ns per row (queried %s)' % (
                L.fcell(cq, 1), q))
            res['c_q_ns'] = cq
        res['census'] = cen
        RES['%s W%d' % (row, W)] = res
json.dump(RES, open(os.path.join(L.HERE, 'g5_profile.json'), 'w'), indent=1, default=str)
O.save('g5_profile.txt')
