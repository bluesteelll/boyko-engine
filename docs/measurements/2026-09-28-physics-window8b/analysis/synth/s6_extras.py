"""Extras the synthesis states: SPLIT per-wave shares (PARENT W8/W16, [100,500)), F3 c_q, J-Son-T freeze step and the
L10 serial pieces (armed, [0,100)), S7 paired T/P per round at W16, S4 per-manifold. All from raw/ via synthlib."""
import statistics

import synthlib as L

O = L.Out()
recs = L.all_records()
A, B = 100, 500

_, us, _ = L.select('SPLIT', recs)
O('# SPLIT (PARENT, SPLIT-J-T-a, [100,500)): per-step us and per-wave ns, cell medians over K = 9')
for W in (8, 16):
    rows = []
    for p in us:
        if p['row'] != 'SPLIT-J-T-a' or p['W'] != W:
            continue
        c, tpn = p['c'], p['s']['ticks_per_ns']

        def sm(k):
            return sum(v for v in c[k][A:B] if v is not None)
        n = B - A
        waves = sm('phys_route_worker') + sm('phys_route_external')
        disp = sum(1 for v in c['phys_color_scopes'][A:B] if v and v > 0)
        r = {'ramp': sm('phys_wave_ramp') / tpn / n / 1e3, 'join': sm('phys_wave_join') / tpn / n / 1e3,
             'tail': sm('phys_wave_tail') / tpn / n / 1e3, 'first': sm('phys_wave_first_ramp') / tpn / n / 1e3,
             'pass': sm('phys_wave_pass_ramp') / tpn / n / 1e3, 'waves': waves / n,
             'first_ns': sm('phys_wave_first_ramp') / tpn / disp, 'pass_ns': sm('phys_wave_pass_ramp') / tpn / (12 * disp),
             'other_ns': (sm('phys_wave_ramp') - sm('phys_wave_pass_ramp')) / tpn / (waves - 12 * disp),
             'helped': sm('phys_wave_helped') / waves}
        rows.append(r)
    m = {k: statistics.median(x[k] for x in rows) for k in rows[0]}
    imb = m['tail'] - m['join']
    O(f'  W{W}: waves/step {m["waves"]:.2f}, helped {m["helped"]:.3f}; ramp {m["ramp"]:.1f} (pass-first {m["pass"]:.1f} = '
      f'{100 * m["pass"] / m["ramp"]:.0f} %), tail {m["tail"]:.1f} = imbalance {imb:.1f} ({100 * imb / m["tail"]:.0f} %) + '
      f'join {m["join"]:.1f} ({100 * m["join"] / m["tail"]:.0f} %) us/step; first ramp {m["first_ns"]:.0f} ns, pass-first '
      f'{m["pass_ns"]:.0f} ns = {m["pass_ns"] / m["other_ns"]:.1f}x the other waves {m["other_ns"]:.0f} ns')

_, u3, _ = L.select('F3', recs)
O('\n# F3 c_q = t_q / queried rows per step (TD-armed W1, median over steps [100,500), per process; cell median)')
for row in ('F3-TD-armed-leaflist', 'F3-TD-armed-kd'):
    v = []
    q = set()
    for p in u3:
        if p['row'] == row and p['W'] == 1:
            c = p['c']
            v.append(statistics.median(c['phys_bp_query_ns'][A:B]))
            q |= {x for x in c['phys_bp_queried'][A:B]}
    O(f'  {row}: t_q {statistics.median(v) / 1e3:.2f} us; queried rows per step {sorted(q)}; c_q '
      f'{statistics.median(v) / 1240:.1f} ns (n={len(v)})')

_, uj, _ = L.select('J-Son-T', recs)
O('\n# J-Son-T: first_frozen_step over ON records; L10 serial pieces (armed JSonT-a, us/step, [0,100) and [274,500))')
ff = sorted({p['s'].get('first_frozen_step') for p in uj if p['row'].startswith('JSonT')})
O(f'  first_frozen_step values: {ff}')
for W in (1, 8):
    for (a, e) in ((0, 100), (274, 500)):
        v = []
        off = []
        for p in uj:
            c = p['c']
            if p['row'] == 'JSonT-a' and p['W'] == W:
                v.append(sum(statistics.fmean(c[k][a:e]) for k in ('phys_sleep_begin_ns', 'phys_sleep_freeze_ns',
                                                                   'phys_sleep_end_ns', 'phys_sleep_classify_ns')) / 1e3)
            if p['row'] == 'JSoffT' and p['W'] == W:
                off.append(statistics.fmean(c['wall_ns'][a:e]) / 1e3)
        O(f'  W{W} [{a},{e}): L10 pieces {statistics.median(v):.2f} us/step = {100 * statistics.median(v) / statistics.median(off):.3f} % '
          f'of the OFF disarmed step {statistics.median(off):.1f} us')

_, u7, _ = L.select('S7-AB', recs)
O('\n# S7 paired per (pass, round): T/P at W16 and W8, [0,500)')
for row in ('S7-JT', 'S7-JA'):
    for W in (8, 16):
        pr = {}
        for p in u7:
            if p['row'] == row and p['W'] == W:
                pr.setdefault((p['pass'], p['round']), {})[p['binary']] = statistics.fmean(p['c']['wall_ns'][0:500])
        rat = [v['s7t'] / v['s7p'] for v in pr.values() if len(v) == 2]
        O(f'  {row} W{W}: T/P median {statistics.median(rat):.4f} [{min(rat):.4f}-{max(rat):.4f}], T slower in '
          f'{sum(r > 1 for r in rat)} of {len(rat)} rounds')
O.save('s6_extras.txt')

O2 = L.Out()
O2('# post hoc: trunk scaling W8 -> W16 (P = s7p = trunk; ruling 1, LETTER), and serial spans at W8 on TIP (armed)')
for row in ('S7-JT', 'S7-JA'):
    va, vb = {}, {}
    for p in u7:
        if p['row'] == row and p['binary'] == 's7p' and p['W'] in (8, 16):
            (va if p['W'] == 8 else vb).setdefault(p['pass'], []).append(statistics.fmean(p['c']['wall_ns'][0:500]) / 1e6)
    j = L.judge(va, vb, kmin=3)
    O2(f'  {row} P(16) vs P(8): {L.fj(j)}')
_, u4, _ = L.select('S4-AB', recs)
for col in ('phys_bp_query_ns', 'sys_physics_broadphase_colored_ns', 'phys_warm_apply_ns', 'phys_solve_build_ns'):
    for b in ('parent', 'tip'):
        v = [statistics.fmean(p['c'][col][A:B]) / 1e6 for p in u4 if p['row'] == 'S4-JT-a' and p['binary'] == b and p['W'] == 8]
        O2(f'  S4-JT-a#{b}@W8 [100,500) {col}: {statistics.median(v):.4f} ms (n={len(v)})')
O.lines += [''] + O2.lines
O.save('s6_extras.txt')

O3 = L.Out()
O3('# SPLIT L_wide decomposition W8 vs W16 (PARENT armed, [100,500), us/step; L_wide = wide(W) - wide(1)/W)')
_, us2, _ = L.select('SPLIT', recs)


def wide_mean(p):
    return statistics.fmean(p['c']['phys_color_wide_ns'][A:B]) / 1e3


w1 = statistics.median(wide_mean(p) for p in us2 if p['row'] == 'SPLIT-J-T-a' and p['W'] == 1)
dec = {}
for W in (8, 16):
    ps = [p for p in us2 if p['row'] == 'SPLIT-J-T-a' and p['W'] == W]
    lw = statistics.median(wide_mean(p) for p in ps) - w1 / W

    def per(k):
        return statistics.median(sum(v for v in p['c'][k][A:B] if v is not None) / p['s']['ticks_per_ns'] / (B - A) / 1e3
                                 for p in ps)
    ramp, tail, join = per('phys_wave_ramp'), per('phys_wave_tail'), per('phys_wave_join')
    dec[W] = (lw, ramp, join, tail - join, lw - ramp - tail)
    O3(f'  W{W}: L_wide {lw:.1f} = ramp {ramp:.1f} + join {join:.1f} + imbalance {tail - join:.1f} + in-wave remainder '
       f'{lw - ramp - tail:.1f} ({100 * (lw - ramp - tail) / lw:.1f} %)')
g = [dec[16][i] - dec[8][i] for i in range(5)]
O3(f'  W16 - W8: L_wide {g[0]:+.1f}; ramp {g[1]:+.1f}, join {g[2]:+.1f}, imbalance {g[3]:+.1f}, in-wave {g[4]:+.1f} '
   f'({100 * g[4] / g[0]:.0f} % of the growth)')
O.lines += [''] + O3.lines
O.save('s6_extras.txt')
