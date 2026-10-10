"""Our equal-work census on the tip (C4-JD-armed#tip, W1 and W8, block C4-G5; used processes only), counts only (no
timing): per-step means over [100,500) of phys_np_points, phys_np_manifolds and the disarmed 'manifolds' column; the
velocity row-iterations per step = points x 3 rows x 12 sweeps (jolt-gap/equal-work/units.py: substeps 4 x (1 biased +
2 relax), 3 rows per point; the SUMMARY config is re-checked here). Plus the disarmed C4-JD#tip manifolds column."""
import collections
import json

import lib9a as L

O = L.Out()
recs = L.all_records()
_, used, _ = L.select('C4-G5', recs)
arm = [p for p in used if p['row'] == 'C4-JD-armed']
res = collections.defaultdict(set)
for p in arm:
    c, s = p['c'], p['s']
    cfg = s['config']
    assert cfg['substeps'] == 4 and cfg['relax_iterations'] == 2, cfg
    pts = L.mean(c['phys_np_points'][100:500])
    man = L.mean(c['phys_np_manifolds'][100:500])
    man2 = L.mean(c['manifolds'][100:500])
    res[p['W']].add((round(pts, 6), round(man, 6), round(man2, 6), s['pose_hash']))
for W in sorted(res):
    O(f'C4-JD-armed#tip W{W}: distinct (points, np_manifolds, manifolds, pose) over the used processes: {sorted(res[W])}')
allv = set().union(*res.values())
assert len(allv) == 1, allv
pts, man, man2, ph = next(iter(allv))
vrow = pts * 3 * 12
O(f'ours per step over [100,500): points {pts:.2f}, manifolds {man:.3f} (disarmed column {man2:.3f}); '
  f'velocity row-iterations {vrow:,.1f} (DOSSIER trunk contact set: 595,935); pose {ph}')
_, usedab, _ = L.select('C4-AB', recs)
jd = {round(L.mean(p['c']['manifolds'][100:500]), 6) for p in usedab if p['row'] == 'C4-JD'}
O(f'C4-JD#tip disarmed (C4-AB) manifolds [100,500): distinct values {sorted(jd)}')
J_V, J_P, J_M = 565790.0, 62224.0, 8489.0
O(f'Jolt (DOSSIER equal-work, hash 0xb8522b4e3fc62cfe): velocity row-iterations {J_V:,.0f} + position rows {J_P:,.0f}; '
  f'manifolds {J_M:,.0f}')
k = {'velocity': J_V / vrow, 'all': (J_V + J_P) / vrow, 'manifold': J_M / man}
O(f'Jolt/ours work: velocity rows {k["velocity"]:.4f}, all rows {k["all"]:.4f}, manifolds {k["manifold"]:.4f}')
json.dump({'points': pts, 'manifolds': man, 'vrow_ours': vrow, 'k': k}, open(L.os.path.join(L.HERE, 'a3_census.json'), 'w'),
          indent=1)
O.save('a3_census.txt')
