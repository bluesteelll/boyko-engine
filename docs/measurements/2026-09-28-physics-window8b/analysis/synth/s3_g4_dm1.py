"""Synthesis re-derivation: F3-G4 (TH 144/152 on `tree`, R3d scene legs, the recipe's LO/HI on the grid) and DM1
(ruling 9 / dm1 cut 11.5 gate, reading R1: pairs by ABBA position, d = median(B-A) over complete pairs, band =
max(range A, range B); per-frame median over 220 frames)."""
import statistics

import synthlib as L

O = L.Out()
recs = L.all_records()

# ---------------- G4 ----------------
_, g4, gdrop = L.select('F3-G4', recs)
O(f'# F3-G4: used {len(g4)} processes, dropped {len(gdrop)} (own K = 3, one pass; the pass is the pooled cell)')


def gcell(i):
    return L.cell([p['est'][i] for p in g4])


def gj(a_id, b_id):
    a, b = gcell(a_id), gcell(b_id)
    c = L.cmp_(a, b, kmin=3)
    claimed = c['cl_i'] and c['cl_s']
    return a, b, c, claimed


GRID = (128, 136, 144, 152, 160, 176, 256, 1000)
for fam in ('uniform', 'disparity'):
    O(f'\n## {fam}: all_pairs/tree (B = all_pairs, A = tree); claimed = i AND s (ruling 1); window 7 rule = r AND s')
    lo1 = lo7 = None
    hi1 = hi7 = None
    for n in GRID:
        a, b, c, cl = gj(f'bp_g4_{fam}/tree/{n}', f'bp_g4_{fam}/all_pairs/{n}')
        cl7 = c['cl_r'] and c['cl_s']
        slower = c['ratio'] > 1
        O(f'  {n:5d}: tree {a["median"]:.3f} us, all_pairs {b["median"]:.3f} us, ratio {c["ratio"]:.4f} '
          f'flags i/s/r {L.yn(c)}; ruling1 {"all_pairs slower CLAIMED" if cl and slower else "-"}; '
          f'w7 {"CLAIMED" if cl7 and slower else "-"}')
        if not (cl and slower) and hi1 is None:
            lo1 = n
        if cl and slower and hi1 is None:
            hi1 = n
        if not (cl7 and slower) and hi7 is None:
            lo7 = n
        if cl7 and slower and hi7 is None:
            hi7 = n
    O(f'  recipe on `tree`: ruling 1 LO {lo1 if lo1 else "< 128 (off the grid)"} / HI {hi1}; '
      f'window-7 rule LO {lo7 if lo7 else "< 128"} / HI {hi7}  (pre-registered to reproduce 144 / 152)')

O('\n## the same recipe on `tree_kd` (recorded for the flip; no bar)')
for fam in ('uniform', 'disparity'):
    lo = hi = None
    for n in GRID:
        a, b, c, cl = gj(f'bp_g4_{fam}/tree_kd/{n}', f'bp_g4_{fam}/all_pairs/{n}')
        if cl and c['ratio'] > 1:
            hi = n
            break
        lo = n
    O(f'  {fam}: ruling 1 LO {lo} / HI {hi}')

O('\n## R3d: bp_g4_scene tree_kd not claimed slower than tree')
for n in ('10000', '100000', '1240', 'j100'):
    a, b, c, cl = gj(f'bp_g4_scene/tree/{n}', f'bp_g4_scene/tree_kd/{n}')
    O(f'  {n:>6s}: tree {a["median"]:.2f}, tree_kd {b["median"]:.2f} us, kd/tree {c["ratio"]:.4f} flags i/s/r '
      f'{L.yn(c)} -> {"kd faster CLAIMED" if cl and c["ratio"] < 1 else ("kd SLOWER CLAIMED" if cl else "not claimed")}'
      f'{" STRONG" if cl and c["cl_r"] else ""}')

# ---------------- DM1 ----------------
procs, used, dropped = L.select('DM1', recs)
O(f'\n# DM1: processes {len(procs)}, slots {len(used) + len(dropped)}, used {len(used)}, dropped {len(dropped)}')
POS = ['A1', 'B1', 'B2', 'A2', 'A3', 'B3', 'B4', 'A4']
ZN = {2: 'VB_SHADE', 4: 'VB_EARLY_CULL', 9: 'VB_RUN', 14: 'VB_PRODUCE_NET', 17: 'GBUF_DEFERRED_RESOLVE'}
GATED = {'vb': (4, 9, 2, 14), 'deferred': (17,)}
for row in ('dm-vb-1920x1080-idle', 'dm-deferred-1920x1080-idle', 'dm-vb-1920x1080-edit100'):
    orig = sorted([p for p in procs if p['row'] == row and p['attempt'] == 'original'], key=lambda p: p['seq'])
    order = ''.join(p['binary'][-1] for p in orig)
    slot = {}
    for pos, p0 in zip(POS, orig):
        assert pos[0] == p0['binary'][-1]
        c = [p for p in used if p['seq'] == p0['seq']]
        slot[pos] = c[0] if c else None
    path = 'vb' if '-vb-' in row else 'deferred'
    O(f'\n## {row}: order {order}; used ' + ' '.join(f'{k}={"-" if slot[k] is None else slot[k]["attempt"][0]}'
                                                      for k in POS))
    for z in GATED[path]:
        v = {k: (slot[k]['zones'][z]['median_ns'] / 1e3 if slot[k] is not None else None) for k in POS}
        pairs = [(v[f'A{i}'], v[f'B{i}']) for i in (1, 2, 3, 4)]
        diffs = [b - a for a, b in pairs if a is not None and b is not None]
        As = [v[k] for k in POS if k[0] == 'A' and v[k] is not None]
        Bs = [v[k] for k in POS if k[0] == 'B' and v[k] is not None]
        d = statistics.median(diffs)
        band = max(max(As) - min(As), max(Bs) - min(Bs))
        g = 'B<=A (d<=0)' if d <= 0 else ('B<=A (inside band)' if d <= band else 'RED: above band')
        am = statistics.mean(As)
        O(f'  zone {z:2d} {ZN[z]:22s}: A n={len(As)} median {statistics.median(As):.1f}, B n={len(Bs)} median '
          f'{statistics.median(Bs):.1f} us; pairs {len(diffs)}; d = {d:+.2f} us ({100 * d / am:+.2f} % of mean A); '
          f'band {band:.2f} us ({100 * band / am:.1f} % of mean A) -> {g}')
O.save('s3_g4_dm1.txt')
