"""The verdict table in window 8b's form (markdown rows), recomputed from raw/ with lib9a: claim | bar | cells + n |
median [min-max] with IQR/SE | pooled i/s/r and per pass | LETTER verdict | GATING-ONLY (post hoc)."""
import json

import lib9a as L

O = L.Out()
recs = L.all_records()
_, used, _ = L.select('C4-AB', recs)
K = json.load(open(L.os.path.join(L.HERE, 'a3_census.json')))
V = {}
for p in used:
    for wn, v in p['v'].items():
        V.setdefault((p['row'], p['binary'], p['W'], wn), {}).setdefault(p['pass'], []).append(v)


def ns(va):
    return '/'.join(str(len(va.get(k, []))) for k in L.PASSES)


def cellstr(c):
    return f"{c['median']:.4f} [{c['min']:.4f}-{c['max']:.4f}] i {100 * c['i']:.2f} s {100 * c['s']:.2f}"


def v_of(kind, j):
    if kind == 'not_slower':
        if j['claimed'] and j['dir'] == 'B>A':
            return 'FAILS'
        return 'HOLDS' + (' (B faster' + (' STRONG)' if j['strong'] else ')') if j['claimed'] else '')
    if kind in ('faster', 'ours_vs_jolt'):
        if not j['claimed']:
            return 'NOT CLAIMED'
        s = ' STRONG' if j['strong'] else ''
        if kind == 'faster':
            return ('CLAIMED' + s) if j['dir'] == 'B<A' else ('REFUTED' + s)
        return ('ours faster' + s) if j['dir'] == 'B<A' else ('Jolt faster' + s)
    if kind == 'same':
        return ('CLAIMED DIFFERENT' + (' STRONG' if j['strong'] else '')) if j['claimed'] else 'NOT CLAIMED'
    if kind == 'rise':
        return ('SEEN' + (' STRONG' if j['strong'] else '')) if (j['claimed'] and j['dir'] == 'B>A') else 'NOT SEEN'


def row(name, bar, ka, kb, kind, wn='0..500', scale=1.0):
    va, vb = V[(*ka, wn)], V[(*kb, wn)]
    jl = L.judge(va, vb, 'LETTER', scale=scale)
    jg = L.judge(va, vb, 'GATING-ONLY', scale=scale)
    p = jl['pooled']
    per = ' '.join(f"{L.yn(c)}" for c in jl['per'].values())
    O(f"| {name} | {bar} | A {ka[0]}#{ka[1]} n={jl['a']['K']} ({ns(va)}); B {kb[0]}#{kb[1]} n={jl['b']['K']} ({ns(vb)}) | "
      f"A {cellstr(jl['a'])}; B {cellstr(jl['b'])}; B/A {p['ratio']:.4f} ({100 * (p['ratio'] - 1):+.2f} %, "
      f"{'' if scale != 1.0 else f'B-A {p[chr(100) + chr(101) + chr(108) + chr(116) + chr(97)]:+.4f}'}) | "
      f"pooled {L.yn(p)}; p0/p1/p2 {per} | **{v_of(kind, jl)}** | {v_of(kind, jg)} (gates p{','.join(map(str, jg['gates']))}) |")


O('| claim | bar | cells, n (p0/p1/p2) | ms: median [min-max] i s (%); B/A | i/s/r | verdict (LETTER) | post hoc GATING-ONLY |')
O('|---|---|---|---|---|---|---|')
for W in (1, 2, 4, 8, 16):
    row(f'MG-ap W{W}', 'JD not claimed slower', ('C4-JDap', 'tip', W), ('C4-JD', 'tip', W), 'not_slower')
for W in (1, 2, 4, 8, 16):
    row(f'MG-par W{W}', 'JD not claimed slower', ('C4-JDpar', 'parent', W), ('C4-JD', 'tip', W), 'not_slower')
for W in (1, 8, 16):
    row(f'CTL J-A W{W}', 'not claimed different', ('C4-JA', 'parent', W), ('C4-JA', 'tip', W), 'same')
for W in (1, 8, 16):
    row(f'CTL J-T W{W}', 'not claimed different', ('C4-JT', 'parent', W), ('C4-JT', 'tip', W), 'same')
for W in (8, 16):
    row(f'RUNG W{W}', 'rise of 0.060 ms claimed', ('C4-JD', 'tip', W), ('C4-rung', 'tip', W), 'rise')
for W in (1, 2, 4, 8, 16):
    row(f'JOLT wall W{W}', 'none (M8)', ('C4-jolt56', 'j56', W), ('C4-JD', 'tip', W), 'ours_vs_jolt')
for un, sc in (('vel.rows', K['k']['velocity']), ('all rows', K['k']['all']), ('manifold', K['k']['manifold'])):
    for W in (1, 2, 4, 8, 16):
        row(f'JOLT {un} W{W} [100,500)', 'none', ('C4-jolt56', 'j56', W), ('C4-JD', 'tip', W), 'ours_vs_jolt', '100..500', sc)
O.save('a7_table.txt')
