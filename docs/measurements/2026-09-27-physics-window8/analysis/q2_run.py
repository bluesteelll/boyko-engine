from q2_identity import *
import json as _j

STAGES = ('T', 'gather', 'bp', 'bp_query', 'bp_assemble', 'bp_build', 'bp_verify', 'np', 'np_disp', 'np_ser',
          'np_compact', 'np_axis', 'graph', 'solve', 'setup', 'sb_bodies', 'sb_pa', 'sb_pb', 'sb_pc', 'warm',
          'integ_grp', 'gravity', 'integrate', 'store', 'write_back', 'restitution', 'sleep', 'wide', 'narrow',
          'passes', 'r', 'u', 'g', 'S_meas')
for win in ('100..500', '0..100'):
    P(f'\n# the identity, J-T armed, window [{win}) (cells = median over K of per-process means, ms)')
    med = {W: medterms('J-T-a', W, ['W8S-A'], win) for W in (1, 8, 16)}
    medR = medterms('R-J-T-a', 8, ['W8S-R'], win)
    Td = {W: C('J-T', W, wall(win), ['W8S-A'])['median'] for W in (1, 8, 16)}
    TdR = C('R-J-T', 8, wall(win), ['W8S-R'])['median']
    P('stage | W1 | W8 | W16 | W8R | W1/W8 | W1/W16 | W8/W16')
    for k in STAGES:
        a, b, c2, d = med[1][k], med[8][k], med[16][k], medR[k]
        rr = lambda x, y: (x / y) if y else float('nan')
        P(f'{k} | {a:.4f} | {b:.4f} | {c2:.4f} | {d:.4f} | {rr(a, b):.2f} | {rr(a, c2):.2f} | {rr(b, c2):.2f}')
    P(f"per-step exact closure max: W1 {med[1]['closure_exact_max']:.2e} W8 {med[8]['closure_exact_max']:.2e} W16 {med[16]['closure_exact_max']:.2e} ms")
    P(f"r g u recomputed vs runner columns, W8 medians: r {med[8]['r']:.5f} vs {med[8]['r_col']:.5f}; g {med[8]['g']:.5f} vs {med[8]['g_col']:.5f}; u {med[8]['u']:.5f} vs {med[8]['u_col']:.5f}")
    np_ser1 = med[8]['np_ser']
    np_par1 = med[1]['np'] - np_ser1
    S1_sp = med[1]['S_meas'] - med[1]['np'] + np_ser1
    S1 = S1_sp + med[1]['g'] + med[1]['u']
    P1 = np_par1 + med[1]['wide']
    P(f'\nS(1) = {S1:.4f} ms (serial spans {S1_sp:.4f} + g(1) {med[1]["g"]:.4f} + u(1) {med[1]["u"]:.4f}); np_ser(1) := np_ser(8) {np_ser1:.4f}')
    P(f'P(1) = {P1:.4f} ms (np parallel {np_par1:.4f} + wide {med[1]["wide"]:.4f}); r(1) {med[1]["r"]:.4f}')
    P(f'T(1) armed {med[1]["T"]:.4f}, disarmed {Td[1]:.4f}; s = S(1)/T_a(1) = {S1 / med[1]["T"]:.3f}')
    res = {}
    for W, M, T_d in ((8, med[8], Td[8]), (16, med[16], Td[16]), ('8R', medR, TdR)):
        Wn = 8 if W == '8R' else W
        S_W = M['S_meas']
        I = S_W - S1_sp
        Lnp = M['np_disp'] - np_par1 / Wn
        Lw = M['wide'] - med[1]['wide'] / Wn
        Lt = Lnp + Lw
        dg = M['g'] - med[1]['g']
        du = M['u'] - med[1]['u']
        ident = S1 + I + P1 / Wn + Lt + dg + du
        resid = M['T'] - ident
        dr = M['r'] - med[1]['r']
        DW = (M['T'] - T_d) - (med[1]['T'] - Td[1]) - dr - du
        res[str(W)] = dict(S1=S1, I=I, P1W=P1 / Wn, L=Lt, Lnp=Lnp, Lw=Lw, dg=dg, du=du, ident=ident, resid=resid, dr=dr, D=DW,
                           T=M['T'], Td=T_d, solve=M['solve'], waves=M['waves'])
        P(f'\n## W{W}: T_a {M["T"]:.4f} (T_d {T_d:.4f}; armed-disarmed {M["T"] - T_d:+.4f}; at W1 {med[1]["T"] - Td[1]:+.4f})')
        P(f'   S(1) {S1:.4f} + I {I:+.4f} + P(1)/W {P1 / Wn:.4f} + L {Lt:.4f} (np {Lnp:.4f}, wide {Lw:.4f}) + dg {dg:+.4f} + du {du:+.4f} = {ident:.4f}')
        P(f'   residual = {resid:+.4f} ms = {100 * resid / M["solve"]:+.2f} % of solve span {M["solve"]:.4f}; r(W) {M["r"]:.4f}, r(1) {med[1]["r"]:.4f}')
        P(f'   N1 dr = {dr:+.4f}; residual after N1 = {resid - dr:+.4f} ms = {100 * (resid - dr) / M["solve"]:+.2f} % of solve span')
        P(f'   u(W) = {M["u"]:.4f} = {100 * M["u"] / M["solve"]:.2f} % of solve span; g(W) = {M["g"]:.4f}')
        P(f'   N2 D(W) = [{M["T"] - T_d:+.4f}] - [{med[1]["T"] - Td[1]:+.4f}] - dr {dr:+.4f} - du {du:+.4f} = {DW:+.4f} ms')
        wv = M['waves']
        P(f'   waves/step {wv:.2f}; omega = L_wide/waves = {1e3 * Lw / wv:.3f} us; after D: {1e3 * (Lw - DW) / wv:.3f} us; D/waves {1e3 * DW / wv:.3f} us')
        ideal = med[1]['T'] / Wn
        ex = M['T'] - ideal
        P(f'   ideal T(1)/W {ideal:.4f}; excess {ex:.4f}: S(1)(1-1/W) {S1 * (1 - 1 / Wn):.4f} ({100 * S1 * (1 - 1 / Wn) / ex:.1f} %), L {Lt:.4f} ({100 * Lt / ex:.1f} %), I {I:.4f} ({100 * I / ex:.1f} %), dg+du {dg + du:.4f}, r-part {M["r"] - med[1]["r"] / Wn:.4f}')
        P(f'   E = P(1)/(W t_par) = {P1 / (Wn * (M["np_disp"] + M["wide"])):.3f}; wide {med[1]["wide"] / (Wn * M["wide"]):.3f}; np {np_par1 / (Wn * M["np_disp"]):.3f}')
    P('\n## W16 - W8 by term')
    for k in ('T', 'S_meas', 'np_disp', 'wide', 'r', 'u', 'g', 'bp', 'np_ser', 'graph', 'setup', 'warm', 'integ_grp', 'narrow', 'gather'):
        P(f'   {k}: W8 {med[8][k]:.4f} W16 {med[16][k]:.4f} d {med[16][k] - med[8][k]:+.4f}')
    _j.dump({'med': med, 'medR': medR, 'Td': Td, 'TdR': TdR, 'res': res, 'S1': S1, 'P1': P1}, open(os.path.join(L.HERE, f'q2_{win}.json'), 'w'), default=float)
save('q2.txt')
