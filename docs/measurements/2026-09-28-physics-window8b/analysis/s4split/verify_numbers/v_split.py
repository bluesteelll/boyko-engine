"""SPLIT per-wave split, D(W), omega and the derived shares, from v_proc.json. Output: v_split.txt."""
import json, os, statistics
import vlib as L

procs = json.load(open(os.path.join(L.HERE, 'v_proc.json')))
OUT = []
P = lambda *a: OUT.append(' '.join(str(x) for x in a))
INS = ['phys_solve_build_ns', 'phys_gravity_ns', 'phys_warm_apply_ns', 'phys_integrate_ns', 'phys_restitution_ns',
       'phys_store_ns', 'phys_write_back_ns', 'phys_sleep_begin_ns', 'phys_sleep_freeze_ns', 'phys_sleep_end_ns']


def perwave(p, win):
    s = p['sums'][win]
    lo, hi = (0, 500) if win == '0..500' else (100, 500)
    tp = p['tpn']
    steps = hi - lo
    wv = s['waves']
    if wv == 0:
        return None
    d = {
        'first_ramp': s['phys_wave_first_ramp'] / steps / tp,
        'first_tail': s['phys_wave_first_tail'] / steps / tp,
        'pass_ramp': s['phys_wave_pass_ramp'] / (12 * steps) / tp,
        'ramp': s['phys_wave_ramp'] / wv / tp,
        'ramp_helped': s['phys_wave_ramp'] / s['phys_wave_helped'] / tp,
        'tail': s['phys_wave_tail'] / wv / tp,
        'join': s['phys_wave_join'] / wv / tp,
        'imb': (s['phys_wave_tail'] - s['phys_wave_join']) / wv / tp,
        'other_ramp': (s['phys_wave_ramp'] - s['phys_wave_pass_ramp']) / (wv - 12 * steps) / tp,
        'np_ramp': s['phys_np_wave_ramp'] / steps / tp,
        'np_tail': s['phys_np_wave_tail'] / steps / tp,
        'np_join': s['phys_np_wave_join'] / steps / tp,
        'np_imb': (s['phys_np_wave_tail'] - s['phys_np_wave_join']) / steps / tp,
        'np_route_worker': s['phys_np_route_worker'],
        'helped': s['phys_wave_helped'] / steps, 'waves': wv / steps,
        'route_ext': s['phys_route_external'],
        'inflight': s['phys_wave_inflight'] / wv, 'lanes': s['phys_wave_lanes'] / wv,
    }
    return d


def sel(row, W, block='SPLIT', b='parent'):
    return [p for p in procs if p['block'] == block and p['row'] == row and p['W'] == W and p['bin'] == b]


P('== units: CSV [0,500) per wave (ticks / ticks_per_ns) vs SUMMARY w8s fields, max rel diff over armed PARENT processes')
pairs = [('first_ramp', 'first_ramp_ns_mean'), ('first_tail', 'first_tail_ns_mean'), ('pass_ramp', 'pass_ramp_ns_mean'),
         ('ramp', 'ramp_ns_mean'), ('tail', 'tail_ns_mean'), ('join', 'join_ns_mean'), ('imb', 'imbalance_ns_mean'),
         ('np_ramp', 'np_ramp_ns_mean'), ('np_tail', 'np_tail_ns_mean'), ('np_join', 'np_join_ns_mean'),
         ('np_imb', 'np_imbalance_ns_mean'), ('ramp_helped', 'ramp_ns_mean_helped')]
mx = dict((k, 0.0) for k, _ in pairs)
n = 0
raw_ratio = []
for p in procs:
    if not p['armed'] or p['W'] == 1 or p['bin'] != 'parent':
        continue
    d = perwave(p, '0..500')
    w = p['w8s']
    n += 1
    for k, sk in pairs:
        mx[k] = max(mx[k], abs(d[k] / w[sk] - 1))
    raw_ratio.append((p['sums']['0..500']['phys_wave_ramp'] / p['sums']['0..500']['waves']) / w['ramp_ns_mean'])
    assert w['first_waves'] == 500 and w['pass_waves'] == 6000 and w['np_waves'] == 500, w
P('processes', n, dict((k, '%.1e' % v) for k, v in mx.items()))
P('raw CSV ramp per wave / SUMMARY ns ramp: min %.4f max %.4f (equals ticks_per_ns when the CSV is in ticks)' % (min(raw_ratio), max(raw_ratio)))
P('ticks_per_ns range:', min(p['tpn'] for p in procs if p['tpn']), max(p['tpn'] for p in procs if p['tpn']))

for p in sel('SPLIT-J-T-a', 1):
    s = p['sums']['0..500']
    assert all(v == 0 for k, v in s.items() if k != 'waves'), (p['dir'], s)
    wv_w1 = s['waves']
P('')
P('W1 SPLIT-J-T-a: every dispatch counter is 0 over [0,500) in', len(sel('SPLIT-J-T-a', 1)), 'processes; the waves column (inline colour waves) reads', wv_w1)

for win in ('100..500', '0..500'):
    P('')
    P('== SPLIT-J-T-a per wave, window [%s), ns, median [min-max], n' % win)
    for W in (8, 16):
        ds = [perwave(p, win) for p in sel('SPLIT-J-T-a', W)]
        for k in ('first_ramp', 'first_tail', 'pass_ramp', 'other_ramp', 'helped', 'waves', 'ramp_helped', 'ramp', 'tail',
                  'join', 'imb', 'np_ramp', 'np_tail', 'np_join', 'np_imb', 'np_route_worker', 'route_ext', 'inflight', 'lanes'):
            xs = sorted(d[k] for d in ds)
            P('  W%d %-16s %.3f [%.3f-%.3f] n %d' % (W, k, statistics.median(xs), xs[0], xs[-1], len(xs)))


def terms(p, win):
    s = p['stats'][win]
    g = lambda k: (s.get(k) or 0.0) / 1e6
    t = {'T': g('wall_ns'), 'wide': g('phys_color_wide_ns'), 'narrow': g('phys_color_narrow_ns'),
         'passes': g('phys_pass_biased_ns') + g('phys_pass_relax_ns'), 'solve': g('sys_physics_solve_colored_ns'),
         'r_col': g('r_ns'), 'u_col': g('u_ns'), 'waves': s.get('waves'),
         'setup': g('phys_solve_build_ns'), 'pc': g('phys_sb_pc_ns'), 'warm': g('phys_warm_apply_ns'),
         'bp': g('sys_physics_broadphase_colored_ns'), 'bp_query': g('phys_bp_query_ns'),
         'graph': g('sys_physics_build_graph_ns'),
         'integ_grp': g('phys_gravity_ns') + g('phys_integrate_ns') + g('phys_store_ns') + g('phys_write_back_ns'),
         'np_ser': g('sys_physics_narrowphase_colored_ns') - g('phys_np_dispatch_ns')}
    t['r'] = t['passes'] - t['wide'] - t['narrow']
    t['u'] = t['solve'] - sum(g(k) for k in INS) - t['passes']
    return t


def med(row, W, key, win):
    return statistics.median(terms(p, win)[key] for p in sel(row, W))


for win in ('100..500', '0..500'):
    P('')
    P('== D(W), omega; window [%s) (ms, medians over K=9)' % win)
    Td = dict((W, med('SPLIT-J-T', W, 'T', win)) for W in (1, 8, 16))
    Ta = dict((W, med('SPLIT-J-T-a', W, 'T', win)) for W in (1, 8, 16))
    P('  T_d', dict((W, round(v, 4)) for W, v in Td.items()), 'T_a', dict((W, round(v, 4)) for W, v in Ta.items()))
    r1, u1, wide1 = med('SPLIT-J-T-a', 1, 'r', win), med('SPLIT-J-T-a', 1, 'u', win), med('SPLIT-J-T-a', 1, 'wide', win)
    for W in (8, 16):
        rW, uW = med('SPLIT-J-T-a', W, 'r', win), med('SPLIT-J-T-a', W, 'u', win)
        rc, uc = med('SPLIT-J-T-a', W, 'r_col', win), med('SPLIT-J-T-a', W, 'u_col', win)
        dr, du = rW - r1, uW - u1
        D = (Ta[W] - Td[W]) - (Ta[1] - Td[1]) - dr - du
        wv = med('SPLIT-J-T-a', W, 'waves', win)
        Lw = med('SPLIT-J-T-a', W, 'wide', win) - wide1 / W
        P('  W%d: D = [%+.4f] - [%+.4f] - dr %.4f - du %.4f = %+.4f ms (%.3f us/wave); r %.5f (col %.5f) u %.5f (col %.5f)' % (
            W, Ta[W] - Td[W], Ta[1] - Td[1], dr, du, D, 1e3 * D / wv, rW, rc, uW, uc))
        P('      L_wide %.4f ms, waves %.2f, omega %.3f us, after D %.3f us' % (Lw, wv, 1e3 * Lw / wv, 1e3 * (Lw - D) / wv))
    if win == '100..500':
        P('')
        P('== serial stages, SPLIT-J-T-a W8 [100,500) (ms)')
        for k in ('setup', 'pc', 'warm', 'bp', 'bp_query', 'graph', 'integ_grp', 'narrow', 'np_ser'):
            P('  %-10s %.4f' % (k, med('SPLIT-J-T-a', 8, k, win)))

open(os.path.join(L.HERE, 'v_split.txt'), 'w').write(chr(10).join(OUT) + chr(10))
print(chr(10).join(OUT))
