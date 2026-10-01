"""S1 arithmetic on window 8b's own inputs (post hoc, arith., no claim). Inputs recomputed from raw/:
  - PARENT (S4 off) armed SPLIT-J-T-a W1/W8 [100,500): wave ramp / join / tail / first ramp / pass ramps per step (TSC
    ticks / SUMMARY ticks_per_ns, the SPLIT method), waves per step, wide(W), L_wide = wide(8) - wide(1)/8, omega(8),
    warm apply span, integrate group (gravity + integrate + store + write_back), as q_split.py defines them;
  - TIP (trunk, S4 on) armed S4-JT-a#tip@W8 [100,500): the same spans (S4 changes setup, not these);
  - T(8) disarmed [0,500) and [100,500): S4-JT PARENT and TIP (S4-AB);
  - w = omega_b v2 net stage cost at P8 (slope - b x work), from s2_micro's cells (recomputed here from raw).
Formulas: window 8 section 5 (constants 0.3, 0.7, 0.745, 36.44, 9, 0.0027 are window 8's, not re-derived here):
  wide = (omega(8) - 0.3 - w - 0.7) x waves; S2 = warm x 0.745 - 36.44 w; S3 = (integ - 0.0027) x 0.745 - 9 w.
  S2i = warm x 0.745 - 36.44 x omega(8) (W8 ruling 6); S3i by analogy = (integ - 0.0027) x 0.745 - 9 x omega(8).
SPLIT framing of the colour-wave term: S1 removes at most ramp + join - one recruitment (the first ramp) per step
(s4split section 3), and pays waves x w: wide_split = (ramp + join - first ramp) - waves x w."""
import collections
import statistics

import synthlib as L

O = L.Out()
recs = L.all_records()
A, B = 100, 500


def split_terms(p):
    c = p['c']
    s = p['s']
    tpn = s['ticks_per_ns']
    n = B - A

    def sm(k):
        return sum(v for v in c[k][A:B] if v is not None)

    def mn(k):
        return sm(k) / n / 1e6  # ms per step (span columns are ns)

    waves = sm('phys_route_worker') + sm('phys_route_external')
    t = {'waves': waves / n,
         'ramp': sm('phys_wave_ramp') / tpn / n / 1e3, 'join': sm('phys_wave_join') / tpn / n / 1e3,
         'tail': sm('phys_wave_tail') / tpn / n / 1e3, 'first': sm('phys_wave_first_ramp') / tpn / n / 1e3,
         'pass': sm('phys_wave_pass_ramp') / tpn / n / 1e3,
         'wide': mn('phys_color_wide_ns'), 'warm': mn('phys_warm_apply_ns'),
         'integ': mn('phys_gravity_ns') + mn('phys_integrate_ns') + mn('phys_store_ns') + mn('phys_write_back_ns'),
         'scopes': sm('phys_color_scopes') / n, 'setup': mn('phys_solve_build_ns'),
         'tasks_per_scope': sm('phys_color_tasks') / sm('phys_color_scopes') if sm('phys_color_scopes') else 0.0}
    return t


def med(ps, k):
    return statistics.median(x[k] for x in ps)


_, us, _ = L.select('SPLIT', recs)
_, u4, _ = L.select('S4-AB', recs)
par = {W: [split_terms(p) for p in us if p['row'] == 'SPLIT-J-T-a' and p['W'] == W] for W in (1, 8)}
tip8 = [split_terms(p) for p in u4 if p['row'] == 'S4-JT-a' and p['binary'] == 'tip' and p['W'] == 8]
T8 = {}
for b in ('parent', 'tip'):
    for (a, e) in ((0, 500), (100, 500)):
        T8[(b, a)] = statistics.median(statistics.fmean(p['c']['wall_ns'][a:e]) / 1e6 for p in u4
                                       if p['row'] == 'S4-JT' and p['binary'] == b and p['W'] == 8)

# w from omega v2 (net), P8 gap 0
_, u2, _ = L.select('omega-v2', recs)
NET = collections.defaultdict(list)
for p in u2:
    if p['row'].startswith('omega-wgap'):
        continue
    by = {}
    for s in p['ss']:
        by.setdefault((s['route'], s['helper'], s['participants'], s['blocks_per_participant'], s['gap_us']), {})[
            s['stages']] = s
    for k, st in by.items():
        if k[2] == 8 and k[4] == 0:
            sl = (st[72]['region_ns_median'] - st[36]['region_ns_median']) / 36.0
            NET[(k[0], k[1], k[3])].append(sl - k[3] * st[36]['work_ns_calibrated'])
            NET[('raw', k[0], k[1], k[3])].append(sl)
W_NET = {b: [statistics.median(NET[(r, h, b)]) / 1e3 for r in ('worker', 'external') for h in ('spin', 'park')]
         for b in (1, 2, 4)}

O('# S1 arithmetic on window 8b inputs (post hoc, arith.; no claim)')
O(f'n: SPLIT-J-T-a PARENT W1 {len(par[1])}, W8 {len(par[8])}; S4-JT-a TIP W8 {len(tip8)}')
p8 = {k: med(par[8], k) for k in par[8][0]}
p1 = {k: med(par[1], k) for k in par[1][0]}
t8 = {k: med(tip8, k) for k in tip8[0]}
Lw = p8['wide'] - p1['wide'] / 8
om8 = 1e3 * Lw / p8['scopes']
O(f'PARENT W8 [100,500): waves/step {p8["waves"]:.2f}, colour scopes/step {p8["scopes"]:.2f}; L_wide {Lw * 1e3:.1f} us; '
  f'omega(8) {om8:.3f} us; ramp {p8["ramp"]:.1f}, join {p8["join"]:.1f}, tail {p8["tail"]:.1f}, first ramp '
  f'{p8["first"]:.2f}, pass ramps {p8["pass"]:.1f} us/step; warm {p8["warm"]:.4f} ms; integ group {p8["integ"]:.4f} ms; '
  f'setup {p8["setup"]:.4f} ms')
Lw_t = t8['wide'] - p1['wide'] / 8
O(f'TIP W8 [100,500): waves/step {t8["waves"]:.2f}, colour scopes/step {t8["scopes"]:.2f}; L_wide (wide(1) from '
  f'PARENT W1, an estimate) {Lw_t * 1e3:.1f} us; omega(8) {1e3 * Lw_t / t8["scopes"]:.3f} us; ramp {t8["ramp"]:.1f}, '
  f'join {t8["join"]:.1f}, first {t8["first"]:.2f} us/step; warm {t8["warm"]:.4f}; integ {t8["integ"]:.4f}; setup '
  f'{t8["setup"]:.4f} ms')
O('T(8) disarmed S4-JT (S4-AB): ' + ', '.join(f'{b} [{a},500) {T8[(b, a)]:.4f}' for (b, a) in T8))
for b in (1, 2, 4):
    O(f'w (net omega_b v2, P8, b={b}P) over routes x helpers: {min(W_NET[b]):.3f}-{max(W_NET[b]):.3f} us')

O(f'colour tasks per scope W8 [100,500): PARENT {p8["tasks_per_scope"]:.2f}, TIP {t8["tasks_per_scope"]:.2f} '
  f'= {p8["tasks_per_scope"] / 8:.2f} blocks per participant at P8')
disp = p8['ramp'] + p8['join'] - p8['first']
O(f'\nSPLIT dispatch target (ramp + join - first ramp): {disp:.1f} us/step (PARENT); TIP {t8["ramp"] + t8["join"] - t8["first"]:.1f}')


def bundle(w, src):
    om = 1e3 * (src['wide'] - p1['wide'] / 8) / src['scopes']
    wide_f = (om - 0.3 - w - 0.7) * src['scopes'] / 1e3
    wide_s = (src['ramp'] + src['join'] - src['first'] - src['scopes'] * w) / 1e3
    s2 = src['warm'] * 0.745 - 36.44 * w / 1e3
    s3 = (src['integ'] - 0.0027) * 0.745 - 9 * w / 1e3
    s2i = src['warm'] * 0.745 - 36.44 * om / 1e3
    s3i = (src['integ'] - 0.0027) * 0.745 - 9 * om / 1e3
    return wide_f, wide_s, s2, s3, s2i, s3i


for name, src, T in (('PARENT inputs (S4 off, the design baseline)', p8, T8[('parent', 0)]),
                     ('TIP inputs (trunk 16191fda, S4 on)', t8, T8[('tip', 0)])):
    O(f'\n## {name}; T(8) [0,500) = {T:.4f} ms; 5 % build-if = {0.05 * T:.4f} ms')
    for b in (1, 2, 4):
        lo, hi = min(W_NET[b]), max(W_NET[b])
        rows = [bundle(w, src) for w in (hi, lo)]
        wf = [r[0] for r in rows]
        ws = [r[1] for r in rows]
        s2 = [r[2] for r in rows]
        s3 = [r[3] for r in rows]
        bf = [r[0] + r[2] + r[3] for r in rows]
        bs = [r[1] + r[2] + r[3] for r in rows]
        O(f'  w {lo:.3f}-{hi:.3f} (b={b}P): wide formula {wf[0]:.3f}..{wf[1]:.3f}, wide SPLIT {ws[0]:+.3f}..{ws[1]:+.3f}, '
          f'S2 {s2[0]:.3f}..{s2[1]:.3f}, S3 {s3[0]:.3f}..{s3[1]:.3f} ms; bundle formula {bf[0]:.3f}..{bf[1]:.3f} = '
          f'{100 * bf[0] / T:.1f}-{100 * bf[1] / T:.1f} %; bundle SPLIT-wide {bs[0]:.3f}..{bs[1]:.3f} = '
          f'{100 * bs[0] / T:.1f}-{100 * bs[1] / T:.1f} %')
    r = bundle(0.0, src)
    O(f'  S2i {r[4]:.3f} ms, S3i (by analogy) {r[5]:.3f} ms, S2i + S3i {r[4] + r[5]:.3f} ms = {100 * (r[4] + r[5]) / T:.1f} %')
    for b in (1, 2, 4):
        lo, hi = min(W_NET[b]), max(W_NET[b])
        inc = []
        for w in (hi, lo):
            q = bundle(w, src)
            inc.append((q[1] + q[2] + q[3]) - (q[4] + q[5]))
        O(f'  S1 bundle (SPLIT-wide) minus S2i + S3i at b={b}P: {inc[0]:+.3f}..{inc[1]:+.3f} ms = '
          f'{100 * inc[0] / T:+.1f}..{100 * inc[1] / T:+.1f} % of T(8)')
    # S2i priced at the dispatch-only per-wave cost d = (ramp + join) / colour scopes (SPLIT), since the 0.745 factor
    # (= 1 - 1/(8E), scale_design.md section 6) already charges the in-wave efficiency E; omega(8) = L_wide/waves
    # charges it a second time. A framing question for the S1 design review, not a measurement.
    d = (src['ramp'] + src['join']) / src['scopes']
    s2i_d = src['warm'] * 0.745 - 36.44 * d / 1e3
    s3i_d = (src['integ'] - 0.0027) * 0.745 - 9 * d / 1e3
    O(f'  dispatch-only per-wave cost d = (ramp + join)/scopes = {d:.3f} us; omega(8) = {1e3 * (src["wide"] - p1["wide"] / 8) / src["scopes"]:.3f} us; '
      f'in-wave remainder + imbalance per wave = {1e3 * (src["wide"] - p1["wide"] / 8) / src["scopes"] - d:.3f} us')
    O(f'  S2i_d {s2i_d:.3f} ms, S3i_d {s3i_d:.3f} ms, sum {s2i_d + s3i_d:.3f} ms = {100 * (s2i_d + s3i_d) / T:.1f} %')
    for b in (1, 2, 4):
        lo, hi = min(W_NET[b]), max(W_NET[b])
        inc = []
        for w in (hi, lo):
            q = bundle(w, src)
            inc.append((q[1] + q[2] + q[3]) - (s2i_d + s3i_d))
        O(f'  S1 bundle (SPLIT-wide) minus S2i_d + S3i_d at b={b}P: {inc[0]:+.3f}..{inc[1]:+.3f} ms = '
          f'{100 * inc[0] / T:+.1f}..{100 * inc[1] / T:+.1f} % of T(8)')
    # the pre-registered S1 input is the RAW slope at b=4P (it holds 4 blocks of work); same formulas
    raws = [statistics.median(NET[('raw', r, h, 4)]) / 1e3 for r in ('worker', 'external') for h in ('spin', 'park')]
    lo, hi = min(raws), max(raws)
    q = [bundle(w, src) for w in (hi, lo)]
    O(f'  RAW slope b=4P {lo:.3f}-{hi:.3f} us (pre-registered input, not like-for-like): bundle formula '
      f'{q[0][0] + q[0][2] + q[0][3]:.3f}..{q[1][0] + q[1][2] + q[1][3]:.3f} ms = '
      f'{100 * (q[0][0] + q[0][2] + q[0][3]) / T:.1f}-{100 * (q[1][0] + q[1][2] + q[1][3]) / T:.1f} %; bundle SPLIT-wide '
      f'{q[0][1] + q[0][2] + q[0][3]:.3f}..{q[1][1] + q[1][2] + q[1][3]:.3f} ms = '
      f'{100 * (q[0][1] + q[0][2] + q[0][3]) / T:.1f}-{100 * (q[1][1] + q[1][2] + q[1][3]) / T:.1f} %')
O.save('s4_s1_arith.txt')
