"""Checks: poses, setup counters, n per cell, re-runs used, placement receipt summary (recorded, never used to drop),
and the 'unresolved above' bar per W (the largest i or s bar over pooled + passes)."""
from common_s4 import *
import pickle

for b in ('S4-AB', 'SPLIT'):
    ps = [p for p in PROCS if p['block'] == b]
    P('%s: used %d; pose hashes %s; expect_pose %s' % (b, len(ps), sorted(set(p['summary']['pose_hash'] for p in ps)),
                                                     sorted(set(p['valid_why'] == [] for p in ps))))
    armed = [p for p in ps if p['summary']['armed']]
    P('   armed processes %d; (binary, W, setup_steps, setup_tasks): %s' % (len(armed), sorted(set(
        (p['binary'], p['W'], p['summary']['w8s']['setup_steps'], p['summary']['w8s']['setup_tasks']) for p in armed))))
    cells = {}
    for p in ps:
        cells.setdefault((p['row'], p['binary'], p['W']), []).append(p)
    P('   cells %d; n per cell %s; per pass %s' % (len(cells), sorted(set(len(v) for v in cells.values())),
                                                 sorted(set(tuple(sorted(sum(1 for q in v if q['pass'] == k) for k in (0, 1, 2))) for v in cells.values()))))
    P('   re-runs used: %s' % [(p['row'], p['binary'], p['W'], p['pass'], p['round']) for p in ps if p['attempt'] == 'rerun'])
    ms = [(p['placement'] or {}).get('main_share_top_est') for p in ps]
    P('   placement main_share_top_est: %d of %d = 0.0 (sub-tick main CPU time), others %s..%s' % (
        sum(1 for x in ms if x == 0), len(ms), min(x for x in ms if x), max(x for x in ms if x)))
    top = [((p['placement'] or {}).get('top3') or [[None]])[0][0] for p in ps]
    P('   top logical CPU: %s' % sorted(((t, top.count(t)) for t in set(top)), key=lambda x: -x[1]))

R = pickle.load(open(os.path.join(L.HERE, 'q_s4.pkl'), 'rb'))
P('\n# unresolved-above bar (ruling 1): the largest of bar_i, bar_s over pooled and the three passes')
for win in ('0..500', '100..500'):
    for W in (1, 2, 4, 8, 16):
        j = R[str((win, 'JT', W))]
        allc = [j['pooled']] + j['per']
        mx = max(max(c['bar_i'], c['bar_s']) for c in allc)
        P('[%s) S4-JT W%d: delta TIP-PARENT %+.4f ms (%+.2f %%), verdict(TIP faster) %s; largest i/s bar %.2f %% = %.4f ms' % (
            win, W, j['pooled']['delta'], 100 * (j['pooled']['ratio'] - 1), j['verdict'], 100 * mx,
            mx * j['pooled']['A']))
save('q_checks.txt')

P('\n# placement receipts (recorded, never used to drop)')
for b in ('S4-AB', 'SPLIT'):
    ps = [p for p in PROCS if p['block'] == b]
    ms = [(p['placement'] or {}).get('main_share_top_est') for p in ps]
    cf = [(p['placement'] or {}).get('main_cycle_frac') for p in ps]
    P('%s: main_share_top_est null %d, 1.0 %d, other %d; main_cycle_frac %.4f..%.4f (W1 %.4f..%.4f)' % (
        b, sum(1 for x in ms if x is None), sum(1 for x in ms if x == 1.0),
        sum(1 for x in ms if x is not None and x != 1.0), min(cf), max(cf),
        min(p['placement']['main_cycle_frac'] for p in ps if p['W'] == 1),
        max(p['placement']['main_cycle_frac'] for p in ps if p['W'] == 1)))
save('q_checks.txt')
