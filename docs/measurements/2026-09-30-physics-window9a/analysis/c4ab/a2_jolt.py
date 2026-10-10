"""Standing against Jolt 5.6, in-block (C4-AB): ours = C4-JD#tip (the new default; = J-T after the flip) against
C4-jolt56#j56, per W, on the wall and per unit of work (velocity row-iterations, all row-iterations, manifolds), under
ruling 1 + ruling 8 (LETTER = the reading of record; GATING-ONLY = post hoc). No pre-registered bar (PLAN M8: the Jolt
row in-block, a measurement). Per-unit ratios scale B/A by the constant Jolt/ours work factor (a3_census.json); the
relative spreads and so the bars are unchanged. Scaling T(1)/T(W) from pooled cell medians. Window 8 / 8b figures are
quoted from their analyses (cross-window, no claim)."""
import json
import statistics

import lib9a as L

O = L.Out()
recs = L.all_records()
_, used, _ = L.select('C4-AB', recs)
K = json.load(open(L.os.path.join(L.HERE, 'a3_census.json')))
V = {}
ORIG = {}
for p in used:
    for wn, v in p['v'].items():
        V.setdefault((p['row'], p['binary'], p['W'], wn), {}).setdefault(p['pass'], []).append(v)
    if p['attempt'] == 'original':
        ORIG[(p['row'], p['W'], p['pass'], p['round'], p['seq'])] = p['v']
RES = {}
WS = (1, 2, 4, 8, 16)
units = {'wall': 1.0, 'velocity rows': K['k']['velocity'], 'all rows': K['k']['all'], 'manifold': K['k']['manifold']}
for wn in ('0..500', '100..500', '0..100'):
    O(f'\n## ours (C4-JD#tip) / Jolt 5.6, window [{wn})')
    for W in WS:
        va = V.get(('C4-jolt56', 'j56', W, wn), {})
        vb = V.get(('C4-JD', 'tip', W, wn), {})
        for un, sc in units.items():
            if wn != '100..500' and un != 'wall':
                continue
            out = {rd: L.judge(va, vb, reading=rd, scale=sc) for rd in ('LETTER', 'GATING-ONLY')}
            j = out['LETTER']
            if un == 'wall':
                O(f'\n### W{W}')
                O(f'  Jolt : {L.fcell(j["a"])}')
                O(f'  ours : {L.fcell(j["b"])}')
                for k in L.PASSES:
                    O(f'    p{k}: Jolt {L.fcell(L.cell(va.get(k, [])))} | ours {L.fcell(L.cell(vb.get(k, [])))}')
            for rd in ('LETTER', 'GATING-ONLY'):
                jj = out[rd]
                pp = jj['pooled']
                who = ('ours faster' if jj['dir'] == 'B<A' else 'Jolt faster') if jj['claimed'] else 'no resolved difference'
                O(f'  [{un}] {rd:11s}: ours/Jolt {pp["ratio"]:.4f} pooled {L.yn(pp)} (bars i {100 * pp["bar_i"]:.2f} / '
                  f's {100 * pp["bar_s"]:.2f} / r {100 * pp["bar_r"]:.2f} %); passes '
                  + ' '.join(f'p{k}:{L.fper(c)}' for k, c in jj['per'].items())
                  + f' gates {jj["gates"]} -> {L.label(jj)} ({who})')
                RES[f'{wn}|W{W}|{un}|{rd}'] = {'ratio': pp['ratio'], 'flags': L.yn(pp), 'label': L.label(jj),
                                               'dir': jj['dir'], 'gates': jj['gates'],
                                               'per': {k: L.fper(c) for k, c in jj['per'].items()},
                                               'jolt': jj['a']['median'], 'ours': jj['b']['median'],
                                               'nJ': jj['a']['K'], 'nO': jj['b']['K']}
# ns per manifold and per row-iteration [100,500)
O('\n## per unit, absolute ([100,500), pooled medians)')
for W in WS:
    jo = L.cell([v for k in L.PASSES for v in V[('C4-jolt56', 'j56', W, '100..500')].get(k, [])])['median']
    ou = L.cell([v for k in L.PASSES for v in V[('C4-JD', 'tip', W, '100..500')].get(k, [])])['median']
    O(f'  W{W:2d}: ns/manifold ours {1e6 * ou / K["manifolds"]:.1f}  Jolt {1e6 * jo / 8489.0:.1f} | '
      f'ns/velocity row-it ours {1e6 * ou / K["vrow_ours"]:.3f}  Jolt {1e6 * jo / 565790.0:.3f} | '
      f'ns/all row-it Jolt {1e6 * jo / (565790.0 + 62224.0):.3f}')
# scaling
O('\n## scaling T(1)/T(W), pooled cell medians')
for wn in ('0..500', '100..500'):
    med = {}
    for who, key in (('ours', ('C4-JD', 'tip')), ('Jolt', ('C4-jolt56', 'j56'))):
        for W in WS:
            med[(who, W)] = L.cell([v for k in L.PASSES for v in V[(*key, W, wn)].get(k, [])])['median']
    for W in WS[1:]:
        so, sj = med[('ours', 1)] / med[('ours', W)], med[('Jolt', 1)] / med[('Jolt', W)]
        O(f'  [{wn}] T1/T{W}: ours {so:.3f}  Jolt {sj:.3f}  (ours/Jolt scaling {so / sj:.3f}); '
          f'T(W) ours {med[("ours", W)]:.4f} Jolt {med[("Jolt", W)]:.4f} ms')
    O(f'  [{wn}] T16/T8: ours {med[("ours", 16)] / med[("ours", 8)]:.4f}  Jolt {med[("Jolt", 16)] / med[("Jolt", 8)]:.4f}')
    RES[f'scaling|{wn}'] = {f'{w}|{W}': med[(w, W)] for (w, W) in med}
# W16 vs W8 inside each engine, ruling 1
O('\n## W16 against W8 inside each engine (ruling 1), [0,500)')
for who, key in (('ours', ('C4-JD', 'tip')), ('Jolt', ('C4-jolt56', 'j56'))):
    for rd in ('LETTER', 'GATING-ONLY'):
        j = L.judge(V[(*key, 8, '0..500')], V[(*key, 16, '0..500')], reading=rd)
        O(f'  {who} T16 vs T8 {rd}: {L.fj(j)}')
# paired rounds (post hoc): ours/Jolt in rounds where both ORIGINALS were used (adjacent in time)
O('\n## post hoc: paired rounds (both originals used, adjacent cells), ours/Jolt on [0,500)')
for W in WS:
    pr = []
    for (row, w, ps, rnd, seq), v in ORIG.items():
        if row == 'C4-JD' and w == W:
            jv = [vv for (r2, w2, p2, rn2, _), vv in ORIG.items() if r2 == 'C4-jolt56' and w2 == W and p2 == ps and rn2 == rnd]
            if jv:
                pr.append(v['0..500'] / jv[0]['0..500'])
    if pr:
        O(f'  W{W}: n={len(pr)} paired ratios median {statistics.median(pr):.4f} [{min(pr):.4f}-{max(pr):.4f}]; '
          f'ours faster in {sum(x < 1 for x in pr)} of {len(pr)}')
# cross-window sentinels (post hoc, no claim)
O('\n## post hoc cross-window (no claim; window 8 = win8/analysis.md section 1, window 8b = win8b/analysis.md section 5)')
w8 = {'ours J-T': {1: 4.4701, 8: 2.0357, 16: 2.2233}, 'Jolt': {1: 9.9612, 8: 2.5598, 16: 2.5674}}
w8b = {1: 4.4500, 2: 2.9998, 4: 2.1981, 8: 1.8154, 16: 2.0502}
for W in WS:
    ou = L.cell([v for k in L.PASSES for v in V[('C4-JD', 'tip', W, '0..500')].get(k, [])])['median']
    jo = L.cell([v for k in L.PASSES for v in V[('C4-jolt56', 'j56', W, '0..500')].get(k, [])])['median']
    s = f'  W{W:2d}: 9a ours J-D {ou:.4f} (8b trunk J-T {w8b[W]:.4f}, 9a/8b {ou / w8b[W]:.4f}'
    if W in w8['Jolt']:
        s += f'; w8 J-T {w8["ours J-T"][W]:.4f}, 9a/w8 {ou / w8["ours J-T"][W]:.4f}) | Jolt 9a {jo:.4f} (w8 {w8["Jolt"][W]:.4f}, 9a/w8 {jo / w8["Jolt"][W]:.4f})'
    else:
        s += f') | Jolt 9a {jo:.4f}'
    O(s)
O.save('a2_jolt.txt')
json.dump(RES, open(L.os.path.join(L.HERE, 'a2_jolt.json'), 'w'), indent=1, default=str)
