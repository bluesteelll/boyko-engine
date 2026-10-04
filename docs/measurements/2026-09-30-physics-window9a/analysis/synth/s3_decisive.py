"""Re-derivation of the decisive figures the synthesis's decisions rest on (a second computation beside the group
scripts; all three verifications agreed, so this checks the synthesis's own reading, not a dispute):
  1. the C4 merge rule M7 ('no J-D W is slower'): J-D#tip vs J-Dap#tip and vs J-Dpar#parent at W 1/2/4/8/16, [0,500),
     both readings, plus the p0 min-max separation;
  2. the controls J-A / J-T parent vs tip at W 1/8/16;
  3. G5 B1 (sum of the four tree spans on C4-JD-armed, median over [100,500) steps) and B2 (realised dbp(8) on the
     system span) through the G5 group's library;
  4. the one LETTER claim of the standing (Jolt per manifold at W16), the Jolt W16 process values (bimodality);
  5. figures two groups report on different bases (ours T1/T8; ours/Jolt at W1)."""
import os
import sys

import synth9a as S

sys.path.insert(0, os.path.join(S.W9A, 'analysis', 'g5'))
import lib_g5 as G  # noqa: E402

O = S.Out()
L = S.L

O('# 1. M7: J-D#tip not claimed slower (A = the AllPairs row, B = J-D#tip), [0,500) ms')
worst = None
for arow, ab in (('C4-JDap', 'tip'), ('C4-JDpar', 'parent')):
    for W in S.WS:
        va, vb = S.vals(arow, ab, W, '0..500'), S.vals('C4-JD', 'tip', W, '0..500')
        jl, jg = S.both(va, vb)
        p0a, p0b = va[0], vb[0]
        sep0 = bool(p0a and p0b and max(p0b) < min(p0a))
        slower = (jl['claimed'] and jl['dir'] == 'B>A') or (jg['claimed'] and jg['dir'] == 'B>A')
        d = jl['pooled']['delta']
        worst = d if worst is None else max(worst, d)
        O(f"  {arow}#{ab} W{W}: A {S.fcell(va)} | B {S.fcell(vb)} | B/A {jl['pooled']['ratio']:.4f} "
          f"B-A {d:+.4f} | LETTER {S.verdict(jl)} [{jl['dir']}] GATING-ONLY {S.verdict(jg)} | p0 every J-D below every "
          f"AllPairs: {sep0} | claimed slower in any reading: {slower}")
O(f'  largest B-A over the 10 comparisons: {worst:+.4f} ms (negative = J-D faster everywhere)')

O('\n# 2. Controls, parent (A) vs tip (B), [0,500)')
for row in ('C4-JA', 'C4-JT'):
    for W in (1, 8, 16):
        jl, jg = S.both(S.vals(row, 'parent', W, '0..500'), S.vals(row, 'tip', W, '0..500'))
        O(f"  {row} W{W}: B/A {jl['pooled']['ratio']:.4f} ({100 * (jl['pooled']['ratio'] - 1):+.2f} %) LETTER "
          f"{S.verdict(jl)} GATING-ONLY {S.verdict(jg)}")

O('\n# 3. G5 bars through lib_g5 (median over steps [100,500), ms)')
_, gused, _ = G.select('C4-G5')
for W, bar in ((1, 0.36), (8, 0.35)):
    v = G.by_pass(gused, 'C4-JD-armed', W, G.sum4_med)
    c = G.cell([x for k in v for x in v[k]])
    O(f"  B1 W{W}: sum4 {c['median']:.4f} [{c['min']:.4f}-{c['max']:.4f}] n={c['K']} per pass "
      f"{[round(G.cell(v[k])['median'], 4) for k in sorted(v)]}; bar {bar}; headroom {bar - c['median']:.4f}")
va = G.by_pass(gused, 'C4-JD-armed', 8, lambda p: G.col_med(p, G.SYS))
vb = G.by_pass(gused, 'C4-JDap-armed', 8, lambda p: G.col_med(p, G.SYS))
ca, cb = G.cell([x for k in va for x in va[k]]), G.cell([x for k in vb for x in vb[k]])
O(f"  B2: SYS JD-armed {ca['median']:.4f}, JDap-armed {cb['median']:.4f}, dbp(8) {cb['median'] - ca['median']:+.4f} "
  f"(bar 1.03); per pass " + ', '.join(f"p{k} {G.cell(vb[k])['median'] - G.cell(va[k])['median']:+.4f}" for k in sorted(va)))
vq = G.by_pass(gused, 'C4-JD-armed', 8, lambda p: G.col_mean(p, 'phys_bp_query_ns'))
cq = G.cell([x for k in vq for x in vq[k]])
vw = G.by_pass(gused, 'C4-JD-armed', 8, lambda p: G.wall_mean(p, 100, 500))
cw = G.cell([x for k in vw for x in vw[k]])
O(f"  t_q(W8) mean over steps {cq['median']:.4f} ms; armed wall [100,500) {cw['median']:.4f}; share "
  f"{cq['median'] / cw['median']:.4f}; of the disarmed C4-JD#tip T(8) [100,500) "
  f"{S.pooled_cell(S.vals('C4-JD', 'tip', 8, '100..500'))['median']:.4f}: "
  f"{cq['median'] / S.pooled_cell(S.vals('C4-JD', 'tip', 8, '100..500'))['median']:.4f}")
O(f"  S5 re-price (scale_design s6.5 formula as the G5 group used it: t_q x 0.745 - 0.01): {cq['median'] * 0.745 - 0.01:.4f} ms")

O('\n# 4. The LETTER claim: ours/Jolt per manifold at W16 (A = Jolt, B = ours, [100,500))')
pts, man, vrow = S.ours_census()
va, vb = S.vals('C4-jolt56', 'j56', 16, '100..500'), S.vals('C4-JD', 'tip', 16, '100..500')
jl, _ = S.both(va, vb, S.JOLT_MANIF / man)
O(f"  scale {S.JOLT_MANIF / man:.4f}; ours/Jolt per manifold {jl['pooled']['ratio']:.4f}; LETTER {S.verdict(jl)} "
  f"[{jl['dir']}]; pooled {L.yn(jl['pooled'])}; passes " + ' '.join(
      f"{L.yn(jl['per'][k])}(K{jl['per'][k]['KA']}/{jl['per'][k]['KB']})" for k in S.PASSES))
v16 = sorted(x for k in S.PASSES for x in S.vals('C4-jolt56', 'j56', 16, '0..500')[k])
O(f"  Jolt W16 [0,500) process means: {[round(x, 3) for x in v16]}")

O('\n# 5. Figures reported on different bases')
t = {W: S.pooled_cell(S.vals('C4-JD', 'tip', W, '0..500'))['median'] for W in S.WS}
tt = {W: S.pooled_cell(S.vals(r, 'tip', W, '100..500'))['median']
      for W, r in ((1, 'C4-JT'), (2, 'C4-JD'), (4, 'C4-JD'), (8, 'C4-JT'), (16, 'C4-JT'))}
O(f"  ours T1/T8: C4-AB basis (J-D, [0,500)) {t[1] / t[8]:.3f}; Rapier basis (J-T/J-D, [100,500)) {tt[1] / tt[8]:.3f}")
j = {w: S.pooled_cell(S.vals('C4-jolt56', 'j56', 1, w))['median'] for w in ('0..500', '100..500')}
O(f"  ours/Jolt W1: J-D [0,500) {t[1] / j['0..500']:.4f}; J-T [100,500) {tt[1] / j['100..500']:.4f}")
O.save('s3_decisive.txt')
