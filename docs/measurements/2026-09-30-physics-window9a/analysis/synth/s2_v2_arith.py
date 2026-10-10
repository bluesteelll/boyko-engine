"""ARITHMETIC, NOT A MEASUREMENT: where the standing would sit once V2 (owner rulings 2026-09-30 items 1, 2, 6, 9:
S20 + K3 + VMARGIN) is on the trunk. 9a measured the pre-V2 trunk (window9a_rows.md precondition "V2 is on the trunk"
NOT met), so every "ours faster" in 9a is at unequal fidelity.

Inputs (all recorded, none timed here):
  - 9a cells, [100,500), ours = C4-JD#tip, Jolt, RP-D / RP-M faster build (synth9a / s1_standing).
  - ours today: C4-JD-armed#tip census (points, manifolds per step over [100,500)).
  - V2 work: jolt-gap/f0/f0g/work.txt, gap 0.5, [100,500): arm a = K3+S20, arm b = K3+S20+VMARGIN (reuse on / off);
    F0d's trunk row (VERDICT.md; v2-spec/cut.md s5 item 3): 4483 manifolds / 16608 points / 130.0 colour passes;
    S20: 7617 / 27831 / 192.0.
  - The F0 VERDICT's timing model (VERDICT.md 'Rough timing model'): dT = d(velocity row-iterations) x c_row
    + d(colour passes) x c_pass, c_row = 1.04 ns (W8), 1.32 ns (W16), c_pass = 0.907-0.949 us (W8); setup and
    narrowphase growth left out (so a LOWER bound of the cost). Reproduced here on S20 first (VERDICT: 1.774 -> 2.25
    ms at W8, 2.012 -> 2.54 at W16).
  - Upper bound: the whole step scales with rows (constant total ns per row-iteration; the Rapier group's method)."""
import os
import re

import synth9a as S

O = S.Out()
pts9a, man9a, vrow9a = S.ours_census()
F0D_B = {'manif': 4483.0, 'points': 16608.0, 'passes': 130.0}
S20 = {'manif': 7617.0, 'points': 27831.0, 'passes': 192.0}
work = {}
for line in open(os.path.join(S.SCRATCH, 'jolt-gap', 'f0', 'f0g', 'work.txt'), encoding='utf-8'):
    m = re.match(r'(\S+)\s+(on|off)\s+\[100,500\)\s+(a|b)\s+(\d+)\s+(\d+)\s+([\d.]+)\s+(\d+)\s+([\d.]+)', line)
    if m and m.group(1) == '0.5':
        work[(m.group(2), m.group(3))] = {'manif': float(m.group(4)), 'points': float(m.group(5)),
                                          'passes': float(m.group(8))}
O(f'9a today (C4-JD-armed#tip): points {pts9a:.3f}, manifolds {man9a:.3f}; F0d trunk row: {F0D_B}')
O(f'F0g gap 0.5 [100,500): {work}')
V2 = {k: v for k, v in work.items() if k[1] == 'b'}
fac = {k: v['points'] / F0D_B['points'] for k, v in V2.items()}
O('V2 (K3+S20+VMARGIN) work / F0d trunk: ' + '; '.join(
    f"reuse {k[0]}: points x{fac[k]:.4f}, manifolds x{V2[k]['manif'] / F0D_B['manif']:.4f} ({V2[k]['manif']:.0f} vs Jolt "
    f"{S.JOLT_MANIF:.0f})" for k in V2))
flo, fhi = min(fac.values()), max(fac.values())
v2rows = (pts9a * flo * 36, pts9a * fhi * 36)
O(f'V2 velocity row-iterations on 9a basis: {v2rows[0]:,.0f}-{v2rows[1]:,.0f}; Jolt/ours_V2 velocity rows '
  f'{S.JOLT_VROWS / v2rows[1]:.4f}-{S.JOLT_VROWS / v2rows[0]:.4f} (today {S.JOLT_VROWS / vrow9a:.4f}); '
  f'RP-M/ours_V2 rows {S.RAPIER_ROWS["RP-M"]["100..500"] / v2rows[1]:.4f}-{S.RAPIER_ROWS["RP-M"]["100..500"] / v2rows[0]:.4f} '
  f'(equal-work band [0.90, 1.10])')
O(f'ours_V2 rows / RP-D rows (Rapier defaults, rows_pin 354,674): {v2rows[0] / S.RAPIER_ROWS["RP-D"]["100..500"]:.3f}-'
  f'{v2rows[1] / S.RAPIER_ROWS["RP-D"]["100..500"]:.3f} (today {vrow9a / S.RAPIER_ROWS["RP-D"]["100..500"]:.3f}); '
  f'ours_V2 / Jolt velocity rows {v2rows[0] / S.JOLT_VROWS:.3f}-{v2rows[1] / S.JOLT_VROWS:.3f}; points per manifold V2 '
  + ', '.join(f"{v['points'] / v['manif']:.3f}" for v in V2.values()) + f'; today {pts9a / man9a:.3f}')

C_ROW = {8: 1.04e-9, 16: 1.32e-9}
C_PASS = {8: (0.907e-6, 0.949e-6)}


def model(dpts, dpasses, W):
    rows = dpts * 36 * C_ROW[W] * 1e3        # ms
    if W in C_PASS:
        return rows + dpasses * C_PASS[W][0] * 1e3, rows + dpasses * C_PASS[W][1] * 1e3
    return rows, rows


O('\n# Model check on S20 (VERDICT: +0.476 ms at W8, +0.528 ms at W16)')
for W in (8, 16):
    lo, hi = model(S20['points'] - F0D_B['points'], S20['passes'] - F0D_B['passes'], W)
    O(f'  W{W}: S20 dT = {lo:.4f}-{hi:.4f} ms' + ('' if W in C_PASS else ' (rows only: no colour-pass cost at W16)'))

O('\n# Ours after V2, [100,500), against 9a\'s other engines (ours = C4-JD#tip)')
cells = {}
for W in S.WS:
    g = lambda r, b: S.pooled_cell(S.vals(r, b, W, '100..500'))['median']  # noqa: E731
    cells[W] = {'ours': g('C4-JD', 'tip'), 'jolt': g('C4-jolt56', 'j56'),
                'rpd': min(g('RP-D', 'rs8'), g('RP-D', 'rs4')), 'rpm': min(g('RP-M', 'rs8'), g('RP-M', 'rs4'))}
O('W | ours now | model (lower) ours_V2 | const-ns/row (upper) ours_V2 | vs Jolt lower-upper | vs RP-D best | vs RP-M best')
for W in S.WS:
    c = cells[W]
    up = (c['ours'] * flo, c['ours'] * fhi)
    if W in C_ROW:
        dp = [(V2[k]['points'] - F0D_B['points'], V2[k]['passes'] - F0D_B['passes']) for k in V2]
        d_lo = min(model(a, b, W)[0] for a, b in dp)
        d_hi = max(model(a, b, W)[1] for a, b in dp)
        lo = (c['ours'] + d_lo, c['ours'] + d_hi)
        O(f"W{W} | {c['ours']:.4f} | {lo[0]:.3f}-{lo[1]:.3f} (dT {d_lo:.3f}-{d_hi:.3f}) | {up[0]:.3f}-{up[1]:.3f} | "
          f"{lo[0] / c['jolt']:.3f}-{up[1] / c['jolt']:.3f} | {lo[0] / c['rpd']:.3f}-{up[1] / c['rpd']:.3f} | "
          f"{lo[0] / c['rpm']:.3f}-{up[1] / c['rpm']:.3f}")
    else:
        O(f"W{W} | {c['ours']:.4f} | (no c_row at W{W}) | {up[0]:.3f}-{up[1]:.3f} | <= {up[1] / c['jolt']:.3f} | "
          f"<= {up[1] / c['rpd']:.3f} | <= {up[1] / c['rpm']:.3f}")
O('\n(ratios are ours_V2 / other; < 1 = ours faster. The upper bound scales the WHOLE step with rows, the lower bound '
  'omits setup and narrowphase growth; neither is a measurement. Window 9b\'s V2-AB block measures it.)')

O('\n# The lever budget at W8 against the V2 cost (arithmetic; plan and G5 figures, not measured here)')
sr, s5 = 0.29, (0.142, 0.145)
d8 = [model(V2[k]['points'] - F0D_B['points'], V2[k]['passes'] - F0D_B['passes'], 8) for k in V2]
O(f"  V2 dT(W8) model {min(x[0] for x in d8):.3f}-{max(x[1] for x in d8):.3f} ms; SR predicted 0.29 ms (PLAN M7, "
  f"today's contact set); S5 re-priced {s5[0]}-{s5[1]} ms (G5 group); SR + S5 = {sr + s5[0]:.3f}-{sr + s5[1]:.3f} ms")

c8 = cells[8]
lo8 = (c8['ours'] + min(x[0] for x in d8), c8['ours'] + max(x[1] for x in d8))
up8 = (c8['ours'] * flo, c8['ours'] * fhi)
for name, (a, b) in (('model (lower)', lo8), ('const-ns/row (upper)', up8)):
    ra, rb = a - (sr + s5[1]), b - (sr + s5[0])
    O(f"  W8 V2 {name} {a:.3f}-{b:.3f} minus SR+S5 -> {ra:.3f}-{rb:.3f} ms: vs Jolt {ra / c8['jolt']:.3f}-{rb / c8['jolt']:.3f}, "
      f"vs RP-D best {ra / c8['rpd']:.3f}-{rb / c8['rpd']:.3f}, vs RP-M best {ra / c8['rpm']:.3f}-{rb / c8['rpm']:.3f} "
      "(chained arithmetic; the lever gains were priced on the pre-V2 contact set)")
O(f"  plan end state T(8) 1.28-1.42 ms (PLAN rev 3, pre-V2 contact set) vs 9a J-D T(8) [100,500) {c8['ours']:.4f}")
O.save('s2_v2_arith.txt')
