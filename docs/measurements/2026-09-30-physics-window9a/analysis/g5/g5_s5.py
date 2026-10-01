"""S5 (parallel tree query) inputs from window 9a's G5 block, plus cross-window context from window 8b's raw/.
1. t_q = phys_bp_query_ns on C4-JD-armed#tip (Tree, the default) at W1/W8, both per-process conventions, against 8b's
   0.2069 ms (S4-JT-a#tip@W8, mean over [100,500); 8b s6_extras) and 206.93 us (F3-TD-armed-leaflist W8, median).
   A cell against a constant from ANOTHER window: vs_const flags printed as a description, not a claim.
2. post hoc: t_q(W8) vs t_q(W1) in 9a (ruling 1).
3. S5's W8 letter (RULINGS-2026-09-26 #1; scale_design.md 6.5: query >= 5 % of T(8), above the SE bar), re-read on 9a:
   per process t_q / armed wall over [100,500) at W8 against 0.05 (vs_const); and t_q against the disarmed C4-JD#tip
   T(8) of block C4-AB (cross-block, arithmetic).
4. S5's delta arithmetic (scale_design 6.5: t_q x 0.745 - one ramp 0.01; E = 0.6 variant) on 9a's t_q. Not a claim.
5. W16: NOT MEASURED in 9a (no armed row at W16). Context from 8b raw (post hoc, cross-window): the tip 16191fda armed
   S4-JT-a at W8/W16 and PARENT SPLIT-J-T-a at W1/W8/W16, 8b's own slot rule; W16 vs W8 by ruling 1 inside 8b.
Writes g5_s5.txt."""
import os
import statistics
import sys

import lib_g5 as L

O = L.Out()
recs = L.all_records()
procs, used, dropped = L.select('C4-G5', recs)
Q = 'phys_bp_query_ns'
O('# 1. t_q on C4-JD-armed#tip (50e31f1a), [100,500), ms')
cells = {}
for W in (1, 8):
    for conv, f in (('median', L.col_med), ('mean', L.col_mean)):
        v = L.by_pass(used, 'C4-JD-armed', W, lambda p: f(p, Q))
        cells[(W, conv)] = v
        O('- W%d %s-over-steps: %s; passes %s' % (W, conv, L.fcell(L.cell(sum(v.values(), [])), 4),
                                                  ', '.join('p%d %.4f' % (q, L.cell(x)['median']) for q, x in sorted(v.items()))))
for conv, ref, name in (('mean', 0.2069, "8b S4-JT-a#tip@W8 mean conv (the S5 target, analysis.md 4.7)"),
                        ('median', 0.20693, "8b F3-TD-armed-leaflist W8 median conv (s1_runner_claims.txt)")):
    j = L.judge(cells[(8, conv)], const=ref)
    O('- W8 %s vs %.5f (%s): %s' % (conv, ref, name, L.fj(j)))
    O('    cross-window description only: delta %+.4f ms (%+.2f %%)' % (j['a']['median'] - ref, 100 * (j['a']['median'] / ref - 1)))

O('\n# 2. post hoc: t_q(W8) vs t_q(W1) in 9a (A = W1, B = W8)')
for conv in ('median', 'mean'):
    j = L.judge(cells[(1, conv)], cells[(8, conv)])
    O('- %s conv: %s' % (conv, L.fj(j)))
for name, f in (('build', lambda p: L.col_med(p, 'phys_bp_build_ns')), ('assemble', lambda p: L.col_med(p, 'phys_bp_assemble_ns')),
                ('verify', lambda p: L.col_med(p, 'phys_bp_verify_ns')), ('S4', L.sum4_med)):
    j = L.judge(L.by_pass(used, 'C4-JD-armed', 1, f), L.by_pass(used, 'C4-JD-armed', 8, f))
    O('- %s (median conv): %s' % (name, L.fj(j)))

O('\n# 3. S5 W8 letter re-read: t_q share of T(8)')
sh = L.by_pass(used, 'C4-JD-armed', 8, lambda p: L.col_mean(p, Q) / L.col_mean(p, 'wall_ns'))
j = L.judge(sh, const=0.05)
O('- per process t_q / armed wall (means over [100,500)) at W8: %s' % L.fcell(j['a'], 4))
O('  vs 0.05: %s' % L.fj(j))
sh1 = L.by_pass(used, 'C4-JD-armed', 1, lambda p: L.col_mean(p, Q) / L.col_mean(p, 'wall_ns'))
O('- the same at W1 (context): %s' % L.fcell(L.cell(sum(sh1.values(), [])), 4))
sb = L.by_pass(used, 'C4-JD-armed', 8, lambda p: L.col_mean(p, L.SYS) / L.col_mean(p, 'wall_ns'))
O('- whole serial bp system span / armed wall at W8: %s' % L.fcell(L.cell(sum(sb.values(), [])), 4))
# the disarmed T(W) of the tip default, block C4-AB (cross-block, arithmetic; pooled used processes, all passes)
_, uab, dab = L.select('C4-AB', recs, rows={'C4-JD'})
TW = {}
for W in (1, 2, 4, 8, 16):
    v = [L.wall_mean(p, 100, 500) for p in uab if p['W'] == W]
    v0 = [L.wall_mean(p, 0, 500) for p in uab if p['W'] == W]
    if v:
        TW[W] = L.cell(v)
        O('- C4-AB C4-JD#tip@W%d disarmed wall [100,500): %s; [0,500): %.4f' % (W, L.fcell(TW[W], 4), L.cell(v0)['median']))
tq8 = L.cell(sum(cells[(8, 'mean')].values(), []))['median']
tq8m = L.cell(sum(cells[(8, 'median')].values(), []))['median']
O('- arithmetic: t_q(W8, mean conv) %.4f / T(8) disarmed %.4f = %.2f %% (letter: >= 5 %%)' % (
    tq8, TW[8]['median'], 100 * tq8 / TW[8]['median']))

O('\n# 4. S5 delta arithmetic on 9a (scale_design 6.5 formula; not a claim)')
for lab, tq in (('mean conv', tq8), ('median conv', tq8m)):
    d1 = tq * 0.745 - 0.01
    d2 = tq * (1 - 1 / (8 * 0.6)) - 0.01
    O('- %s t_q %.4f: x0.745 - 0.01 = %.4f ms (%.2f %% of T(8) %.4f); E=0.6: x%.4f - 0.01 = %.4f ms (%.2f %%)' % (
        lab, tq, d1, 100 * d1 / TW[8]['median'], TW[8]['median'], 1 - 1 / 4.8, d2, 100 * d2 / TW[8]['median']))
    O('  ceiling (perfect 8-way split, no ramp): %.4f ms (%.2f %%)' % (tq * 7 / 8, 100 * tq * 7 / 8 / TW[8]['median']))

O('\n# 5. W16: NOT MEASURED in 9a. Cross-window context from 8b raw/ (post hoc; 8b slot rule, LETTER)')
W8B = os.path.join(os.path.dirname(L.W9A), 'win8b', 'analysis', 'synth')
sys.path.insert(0, W8B)
import synthlib as S  # noqa: E402

r8 = S.all_records()
for blk, row, bin_, Ws in (('S4-AB', 'S4-JT-a', 'tip', (8, 16)), ('S4-AB', 'S4-JT-a', 'parent', (8, 16)),
                           ('SPLIT', 'SPLIT-J-T-a', 'parent', (1, 8, 16))):
    _, u8, d8 = S.select(blk, r8)
    vals = {}
    for W in Ws:
        for conv in ('mean', 'median'):
            fn = statistics.fmean if conv == 'mean' else statistics.median
            v = {}
            for p in u8:
                if p['row'] == row and p['binary'] == bin_ and p['W'] == W:
                    v.setdefault(p['pass'], []).append(fn(p['c'][Q][100:500]) / 1e6)
            vals[(W, conv)] = v
            sh = [statistics.fmean(p['c'][Q][100:500]) / statistics.fmean(p['c']['wall_ns'][100:500])
                  for p in u8 if p['row'] == row and p['binary'] == bin_ and p['W'] == W]
            if conv == 'mean':
                O('- 8b %s#%s@W%d t_q %s-over-steps: %s; t_q / armed wall %.2f %%' % (
                    row, bin_, W, conv, S.fcell(S.cell(sum(v.values(), []))), 100 * statistics.median(sh)))
            else:
                O('- 8b %s#%s@W%d t_q %s-over-steps: %s' % (row, bin_, W, conv, S.fcell(S.cell(sum(v.values(), [])))))
    for conv in ('mean', 'median'):
        j = S.judge(vals[(8, conv)], vals[(16, conv)])
        O('  8b %s#%s W16 vs W8 (%s conv): %s' % (row, bin_, conv, S.fj(j)))
    if 1 in Ws:
        j = S.judge(vals[(1, 'median')], vals[(8, 'median')])
        O('  8b %s#%s W8 vs W1 (median conv): %s' % (row, bin_, S.fj(j)))
O.save('g5_s5.txt')
