"""S4-AB: TIP (S4 on) vs PARENT (SETUP_MAX_TASKS = 1) per W, ruling 1; the rung; the zone canaries."""
from common_s4 import *

B = 'S4-AB'
P('# S4-AB (block S4-AB, K = 9 = 3 passes x 3 rounds; B = TIP, A = PARENT; flags r/i/s)')
res = {}
for win in ('0..500', '100..500', '0..100'):
    P('\n## S4-JT disarmed wall, window [%s) ms' % win)
    for W in (1, 2, 4, 8, 16):
        j = judge('S4-JT', 'parent', 'S4-JT', 'tip', W, B, wall(win), -1)
        gain = -j['pooled']['delta']
        P('W%d: gain PARENT-TIP %+.4f ms (%+.2f %%)' % (W, gain, -100 * (j['pooled']['ratio'] - 1)))
        P(fj(j))
        slower = 'CLAIMED SLOWER' if j['verdict'] == 'REFUTED' else 'not claimed slower'
        P('  TIP faster: %s; TIP slower: %s' % (j['verdict'], slower))
        res[(win, 'JT', W)] = j
    P('\n## S4-JT-a armed wall, window [%s) ms' % win)
    for W in (8, 16):
        j = judge('S4-JT-a', 'parent', 'S4-JT-a', 'tip', W, B, wall(win), -1)
        P('W%d: gain PARENT-TIP %+.4f ms' % (W, -j['pooled']['delta']))
        P(fj(j))
        res[(win, 'JTa', W)] = j
    P('\n## S4-JT-a phys_solve_build span (armed), window [%s) ms' % win)
    for W in (8, 16):
        j = judge('S4-JT-a', 'parent', 'S4-JT-a', 'tip', W, B, col('phys_solve_build_ns', win), -1)
        P('W%d: span gain PARENT-TIP %+.4f ms' % (W, -j['pooled']['delta']))
        P(fj(j))
        for z in ('phys_sb_bodies_ns', 'phys_sb_pa_ns', 'phys_sb_pb_ns', 'phys_sb_pc_ns'):
            a = C('S4-JT-a', W, B, col(z, win), 'parent')
            b = C('S4-JT-a', W, B, col(z, win), 'tip')
            P('    %s: PARENT %.4f TIP %.4f d %+.4f ms' % (z, a['median'], b['median'], b['median'] - a['median']))
        res[(win, 'span', W)] = j

P('\n# Resolution (TIP only; same block, same rounds)')
for win in ('0..500', '100..500'):
    P('\n## S4-rung (TIP W8, --canary-frac 1 --canary-ref-ns 60000 = 0.060 ms serial) vs S4-JT#tip@W8, [%s)' % win)
    j = judge('S4-JT', 'tip', 'S4-rung', 'tip', 8, B, wall(win), +1)
    P('rise %+.4f ms (injected 0.0600; rise/inj %.3f)' % (j['pooled']['delta'], j['pooled']['delta'] / 0.06))
    P(fj(j))
    P('  R_8 <= 0.060 ms demonstrated: %s' % ('YES' if j['verdict'] == 'CLAIMED' else 'NO'))
    res[(win, 'rung', 8)] = j
    for N in (30000, 60000):
        rid = 'S4-zone-N%d' % N
        P('\n## %s vs S4-JT-a#tip@W8, [%s): phys_solve_build span' % (rid, win))
        j = judge('S4-JT-a', 'tip', rid, 'tip', 8, B, col('phys_solve_build_ns', win), +1)
        P('span rise %+.4f ms (injected %.4f; rise/inj %.3f)' % (j['pooled']['delta'], N / 1e6,
                                                                 j['pooled']['delta'] / (N / 1e6)))
        P(fj(j))
        res[(win, 'zone', N)] = j
        P('## %s wall vs S4-JT-a#tip@W8, [%s)' % (rid, win))
        jw = judge('S4-JT-a', 'tip', rid, 'tip', 8, B, wall(win), +1)
        P('wall rise %+.4f ms' % jw['pooled']['delta'])
        P(fj(jw))
        res[(win, 'zonewall', N)] = jw

P('\n# Setup counters (armed, [0,500) summaries): setup_steps / setup_tasks per process')
for W in (8, 16):
    for b in ('parent', 'tip'):
        v = sorted(set((p['summary']['w8s']['setup_steps'], p['summary']['w8s']['setup_tasks'])
                       for p in sel('S4-JT-a', W, B, b)))
        P('S4-JT-a#%s@W%d: %s (n=%d)' % (b, W, v, len(sel('S4-JT-a', W, B, b))))
for rid in ('S4-zone-N30000', 'S4-zone-N60000'):
    v = sorted(set((p['summary']['w8s']['setup_steps'], p['summary']['w8s']['setup_tasks']) for p in sel(rid, 8, B)))
    P('%s#tip@W8: %s' % (rid, v))
P('\n# poses: distinct pose hashes over every used S4-AB process: %s' %
  sorted(set(p['summary']['pose_hash'] for p in PROCS if p['block'] == B)))

P('\n# per manifold, [100,500): ns per manifold on each process own manifold mean (S4-JT)')
for W in (1, 2, 4, 8, 16):
    for b in ('parent', 'tip'):
        c = C('S4-JT', W, B, lambda p: 1e6 * p['wall_ms']['100..500'] / p['stats']['100..500']['manifolds'], b)
        m = C('S4-JT', W, B, lambda p: p['stats']['100..500']['manifolds'], b)
        P('W%d %s: %.1f ns/manifold [%.1f-%.1f], manifolds %.3f [%.3f-%.3f]' % (
            W, b, c['median'], c['min'], c['max'], m['median'], m['min'], m['max']))
import pickle
pickle.dump({str(k): v for k, v in res.items()}, open(os.path.join(L.HERE, 'q_s4.pkl'), 'wb'))
save('q_s4.txt')
