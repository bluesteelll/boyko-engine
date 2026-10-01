"""The owner's question in one place: ours against Jolt 5.6 and Rapier 0.36, per W, on the wall and per unit of work.

ours = C4-JD#tip (the shipped default after C4; its tip twin C4-JT#tip is printed beside it where it exists).
Cells: median over K of per-process means, pooled over the three passes, used processes only (ruling-8 slot rule).
Ratios are printed as ours/other (< 1 = ours faster or cheaper). Per-unit ratios scale the wall ratio by the constant
work factor rows_other/rows_ours (Jolt: DOSSIER census; Rapier: rows_pin; ours: C4-JD-armed census in C4-G5).

Judgements (lib9a.judge, ruling 1 + ruling 8) in each group's orientation:
  Jolt  : A = Jolt, B = ours (the C4-AB group's orientation; no pre-registered bar, PLAN M8).
  Rapier: A = ours, B = Rapier (window9a_rows.md: the first-named side is A). Pre-registered comparator = J-T#tip at
          W 1/8/16 and J-D#tip at W 2/4 ('PRE'); the J-D#tip-everywhere reading ('JD') is post hoc.
LETTER = the verdict of record; GATING-ONLY (= the Rapier group's EXCL) = post hoc sizing only."""
import json

import synth9a as S

O = S.Out()
pts, man, vrow = S.ours_census()
O(f'ours census (C4-JD-armed#tip, [100,500)): points {pts:.3f}, manifolds {man:.3f}, velocity row-iterations {vrow:,.2f}')
KJ = {'wall': 1.0, 'velocity rows': S.JOLT_VROWS / vrow, 'all rows': (S.JOLT_VROWS + S.JOLT_PROWS) / vrow,
      'manifold': S.JOLT_MANIF / man}
O('Jolt/ours work factors: ' + ', '.join(f'{k} {v:.4f}' for k, v in KJ.items()))
KR = {cfg: S.RAPIER_ROWS[cfg]['100..500'] / vrow for cfg in S.RAPIER_ROWS}
O('Rapier/ours row factors [100,500): ' + ', '.join(f'{k} {v:.4f}' for k, v in KR.items()))


def ours_pre(W):
    return ('C4-JT', 'tip') if W in (1, 8, 16) else ('C4-JD', 'tip')


def tag(jl, jg, inv=False):
    """Verdict text for ours-vs-other with direction words; inv=True when A = ours."""
    def one(j):
        lab = S.verdict(j)
        if lab == 'NOT CLAIMED':
            return 'NOT CLAIMED'
        ours_better = (j['dir'] == 'B<A') != inv
        return f"{lab} ({'ours' if ours_better else 'other'} faster)"
    return f'LETTER {one(jl)} | GATING-ONLY {one(jg)}'


RES = {}
O('\n# 1. Cells, ms (median [min-max] n (p0/p1/p2))')
for win in ('0..500', '100..500', '0..100'):
    O(f'\n## window [{win})')
    for W in S.WS:
        O(f'W{W}:')
        rows = [('C4-JD', 'tip'), ('C4-JT', 'tip'), ('C4-jolt56', 'j56'), ('RP-D', 'rs8'), ('RP-D', 'rs4'),
                ('RP-M', 'rs8'), ('RP-M', 'rs4')]
        for row, b in rows:
            v = S.vals(row, b, W, win)
            if sum(len(x) for x in v.values()):
                O(f'   {row + "#" + b:14s} {S.fcell(v)}')
                RES[(row, b, W, win)] = S.pooled_cell(v)['median']

O('\n# 2. Ours / Jolt 5.6 (ours = C4-JD#tip)')
JOLT = {}
for win, units in (('0..500', ('wall',)), ('100..500', ('wall', 'velocity rows', 'all rows', 'manifold'))):
    for un in units:
        line = []
        for W in S.WS:
            va = S.vals('C4-jolt56', 'j56', W, win)
            vb = S.vals('C4-JD', 'tip', W, win)
            jl, jg = S.both(va, vb, KJ[un])
            JOLT[(win, un, W)] = (jl['pooled']['ratio'], S.verdict(jl), S.verdict(jg), jl['dir'])
            flags = S.L.yn(jl['pooled']) + '; ' + ' '.join(S.L.yn(jl['per'][k]) + f"(K{jl['per'][k]['KA']}/{jl['per'][k]['KB']})"
                                                         for k in S.PASSES)
            line.append(f"W{W} {jl['pooled']['ratio']:.4f} [{tag(jl, jg)}] i/s/r pooled {flags}")
        O(f'[{win}) {un}:')
        for s in line:
            O('   ' + s)

O('\n# 3. Ours / Rapier 0.36, [100,500) (A = ours, B = Rapier)')
RAPR = {}
for mode in ('PRE', 'JD'):
    O(f'\n## ours = {"J-T#tip at W1/8/16, J-D#tip at W2/4 (pre-registered)" if mode == "PRE" else "J-D#tip at every W (post hoc)"}')
    for cfg in ('RP-D', 'RP-M'):
        for un in ('wall', 'row'):
            O(f'{cfg} {un}:')
            for W in S.WS:
                orow = ours_pre(W) if mode == 'PRE' else ('C4-JD', 'tip')
                va = S.vals(orow[0], orow[1], W, '100..500')
                parts = []
                for b in ('rs8', 'rs4'):
                    vb = S.vals(cfg, b, W, '100..500')
                    sc = 1.0 if un == 'wall' else 1.0 / KR[cfg]   # per-row B/A = (T_R/R_R)/(T_o/R_o) = B/A * R_o/R_R
                    jl, jg = S.both(va, vb, sc)
                    r_ours_other = 1.0 / jl['pooled']['ratio']
                    RAPR[(mode, cfg, un, W, b)] = (r_ours_other, S.verdict(jl), S.verdict(jg), jl['dir'])
                    parts.append(f"{b}: ours/Rapier {r_ours_other:.4f} [{tag(jl, jg, inv=True)}]")
                O(f'   W{W}: ' + ' ; '.join(parts))

O('\n# 4. Headline table, [100,500): ours = C4-JD#tip; Rapier = its faster build (lower median) per cell')
O('W | ours ms | Jolt ms | RP-D best ms | RP-M best ms | ours/Jolt wall | ours/Jolt per vel. row | ours/Jolt per manifold '
  '| ours/RP-D wall | ours/RP-M wall | ours/RP-M per row | ours/RP-D per row')
HEAD = {}
for W in S.WS:
    o = RES[('C4-JD', 'tip', W, '100..500')]
    j = RES[('C4-jolt56', 'j56', W, '100..500')]
    bd = min(('rs8', 'rs4'), key=lambda b: RES[('RP-D', b, W, '100..500')])
    bm = min(('rs8', 'rs4'), key=lambda b: RES[('RP-M', b, W, '100..500')])
    d, m = RES[('RP-D', bd, W, '100..500')], RES[('RP-M', bm, W, '100..500')]
    h = {'ours': o, 'jolt': j, 'rpd': d, 'rpm': m, 'bd': bd, 'bm': bm,
         'oj': o / j, 'oj_v': o / j * KJ['velocity rows'], 'oj_m': o / j * KJ['manifold'],
         'od': o / d, 'om': o / m, 'om_row': o / m * KR['RP-M'], 'od_row': o / d * KR['RP-D']}
    HEAD[W] = h
    O(f"W{W} | {o:.4f} | {j:.4f} | {d:.4f} ({bd}) | {m:.4f} ({bm}) | {h['oj']:.3f} | {h['oj_v']:.3f} | {h['oj_m']:.3f} | "
      f"{h['od']:.3f} | {h['om']:.3f} | {h['om_row']:.3f} | {h['od_row']:.3f}")
O('\n[0,500) wall, ours = C4-JD#tip: ' + ' / '.join(
    f"W{W} {RES[('C4-JD', 'tip', W, '0..500')] / RES[('C4-jolt56', 'j56', W, '0..500')]:.4f}" for W in S.WS))

O('\n# 5. Scaling T(1)/T(W), pooled medians')
for win in ('0..500', '100..500'):
    for name, row, b in (('ours J-D', 'C4-JD', 'tip'), ('Jolt', 'C4-jolt56', 'j56'), ('RP-D rs8', 'RP-D', 'rs8'),
                         ('RP-M rs8', 'RP-M', 'rs8')):
        t = [RES[(row, b, W, win)] for W in S.WS]
        O(f'[{win}) {name:9s}: T1/TW ' + ' / '.join(f'{t[0] / x:.3f}' for x in t[1:]) + f'; T16/T8 {t[4] / t[3]:.4f}')

O('\n# 6. PRE vs JD comparator: largest |difference| of the ours/Rapier ratio, and any verdict that differs')
mx = 0.0
diff = []
for (mode, cfg, un, W, b), v in RAPR.items():
    if mode == 'PRE':
        w = RAPR[('JD', cfg, un, W, b)]
        mx = max(mx, abs(w[0] / v[0] - 1))
        if v[1:3] != w[1:3]:
            diff.append((cfg, un, W, b, v[1:3], w[1:3]))
O(f'max relative difference {100 * mx:.2f} %; verdicts differing (LETTER, GATING-ONLY): {diff}')
json.dump({'head': HEAD, 'jolt': {str(k): v for k, v in JOLT.items()}, 'rapier': {str(k): v for k, v in RAPR.items()}},
          open(S.os.path.join(S.HERE, 's1_standing.json'), 'w'), indent=1)
O.save('s1_standing.txt')
