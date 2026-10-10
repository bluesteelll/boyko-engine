"""SPLIT (PARENT, S4 off): the per-wave split fields per W on SPLIT-J-T-a, recomputed from run.csv sums (the wave
timing counters are TSC ticks in the CSV, divided by the SUMMARY ticks_per_ns), checked against the SUMMARY w8s
object over [0,500); D(W) with the W1 pair (review N2); omega(W). Cell = median over K of per-process values."""
from common_s4 import *

B = 'SPLIT'
SYS = ['sys_physics_gather_ns', 'sys_select_broadphase_ns', 'sys_physics_broadphase_colored_ns',
       'sys_physics_narrowphase_colored_ns', 'sys_physics_build_graph_ns', 'sys_physics_solve_colored_ns',
       'sys_physics_apply_ns', 'sys_physics_integrate_ns']
INSOLVE_SERIAL = ['phys_solve_build_ns', 'phys_gravity_ns', 'phys_warm_apply_ns', 'phys_integrate_ns',
                  'phys_restitution_ns', 'phys_store_ns', 'phys_write_back_ns', 'phys_sleep_begin_ns',
                  'phys_sleep_freeze_ns', 'phys_sleep_end_ns']


def split(p, win):
    a, b = L.WINS[win]
    n = b - a
    S = p['sums'][win]
    tpn = p['summary']['ticks_per_ns']
    c = L.load_csv(os.path.join(p['cwd'], 'run.csv'))
    scopes = S['phys_color_scopes']
    waves = S['phys_route_worker'] + S['phys_route_external']
    dispatched = sum(1 for v in c['phys_color_scopes'][a:b] if v and v > 0)
    npw = sum(1 for v in c['phys_np_wave_ramp_n'][a:b] if v and v > 0)

    def t(k):
        return S[k] / tpn

    m = {}
    m['steps'] = n
    m['waves_per_step'] = waves / n
    m['scopes_per_step'] = scopes / n
    m['helped_per_step'] = S['phys_wave_helped'] / n
    m['helped_frac'] = S['phys_wave_helped'] / waves if waves else None
    m['first_waves'] = dispatched
    sweeps = 12
    m['pass_waves'] = sweeps * dispatched
    m['ramp_ns_mean'] = t('phys_wave_ramp') / waves if waves else None
    m['tail_ns_mean'] = t('phys_wave_tail') / waves if waves else None
    m['join_ns_mean'] = t('phys_wave_join') / waves if waves else None
    m['imbalance_ns_mean'] = (t('phys_wave_tail') - t('phys_wave_join')) / waves if waves else None
    m['ramp_ns_mean_helped'] = t('phys_wave_ramp') / S['phys_wave_helped'] if S['phys_wave_helped'] else None
    m['first_ramp_ns_mean'] = t('phys_wave_first_ramp') / dispatched if dispatched else None
    m['first_tail_ns_mean'] = t('phys_wave_first_tail') / dispatched if dispatched else None
    m['pass_ramp_ns_mean'] = t('phys_wave_pass_ramp') / m['pass_waves'] if dispatched else None
    ow = waves - m['pass_waves']
    m['other_ramp_ns_mean'] = (t('phys_wave_ramp') - t('phys_wave_pass_ramp')) / ow if ow else None
    m['ramp_us_step'] = t('phys_wave_ramp') / n / 1e3
    m['tail_us_step'] = t('phys_wave_tail') / n / 1e3
    m['join_us_step'] = t('phys_wave_join') / n / 1e3
    m['imb_us_step'] = (t('phys_wave_tail') - t('phys_wave_join')) / n / 1e3
    m['first_ramp_us_step'] = t('phys_wave_first_ramp') / n / 1e3
    m['pass_ramp_us_step'] = t('phys_wave_pass_ramp') / n / 1e3
    m['np_waves'] = npw
    m['np_ramp_ns_mean'] = t('phys_np_wave_ramp') / npw if npw else None
    m['np_tail_ns_mean'] = t('phys_np_wave_tail') / npw if npw else None
    m['np_join_ns_mean'] = t('phys_np_wave_join') / npw if npw else None
    m['np_imbalance_ns_mean'] = (t('phys_np_wave_tail') - t('phys_np_wave_join')) / npw if npw else None
    m['np_route_worker'] = S['phys_np_route_worker']
    m['np_route_worker_frac'] = S['phys_np_route_worker'] / npw if npw else None
    m['route_external'] = S['phys_route_external']
    m['inflight_mean'] = S['phys_wave_inflight'] / waves if waves else None
    m['lanes_mean'] = S['phys_wave_lanes'] / waves if waves else None
    return m


def terms(p, win):
    s = p['stats'][win]

    def g(k):
        return (s.get(k) or 0.0) / 1e6

    t = {'T': g('wall_ns'), 'wide': g('phys_color_wide_ns'), 'narrow': g('phys_color_narrow_ns'),
         'passes': g('phys_pass_biased_ns') + g('phys_pass_relax_ns'), 'solve': g('sys_physics_solve_colored_ns'),
         'np': g('sys_physics_narrowphase_colored_ns'), 'np_disp': g('phys_np_dispatch_ns'),
         'setup': g('phys_solve_build_ns'), 'sb_pc': g('phys_sb_pc_ns'), 'warm': g('phys_warm_apply_ns'),
         'integ_grp': g('phys_gravity_ns') + g('phys_integrate_ns') + g('phys_store_ns') + g('phys_write_back_ns'),
         'bp': g('sys_physics_broadphase_colored_ns'), 'bp_query': g('phys_bp_query_ns'),
         'graph': g('sys_physics_build_graph_ns'),
         'gather': (g('sys_physics_gather_ns') + g('sys_select_broadphase_ns') + g('sys_physics_apply_ns')
                    + g('sys_physics_integrate_ns'))}
    t['np_ser'] = t['np'] - t['np_disp']
    t['r'] = t['passes'] - t['wide'] - t['narrow']
    t['u'] = t['solve'] - sum(g(k) for k in INSOLVE_SERIAL) - t['passes']
    t['g'] = t['T'] - sum(g(k) for k in SYS)
    t['r_col'] = g('r_ns')
    t['u_col'] = g('u_ns')
    t['g_col'] = g('g_ns')
    return t


KEYS = ['waves_per_step', 'scopes_per_step', 'helped_per_step', 'helped_frac', 'first_waves', 'pass_waves',
        'ramp_ns_mean', 'ramp_ns_mean_helped', 'first_ramp_ns_mean', 'first_tail_ns_mean', 'pass_ramp_ns_mean',
        'other_ramp_ns_mean', 'tail_ns_mean', 'join_ns_mean', 'imbalance_ns_mean', 'ramp_us_step', 'tail_us_step',
        'join_us_step', 'imb_us_step', 'first_ramp_us_step', 'pass_ramp_us_step', 'np_waves', 'np_ramp_ns_mean',
        'np_tail_ns_mean', 'np_join_ns_mean', 'np_imbalance_ns_mean', 'np_route_worker', 'np_route_worker_frac',
        'route_external', 'inflight_mean', 'lanes_mean']
SUMKEY = ['ramp_ns_mean', 'tail_ns_mean', 'join_ns_mean', 'imbalance_ns_mean', 'ramp_ns_mean_helped',
          'first_ramp_ns_mean', 'first_tail_ns_mean', 'pass_ramp_ns_mean', 'np_ramp_ns_mean', 'np_tail_ns_mean',
          'np_join_ns_mean', 'np_imbalance_ns_mean']

P('# SPLIT: SPLIT-J-T-a on PARENT (S4 off), K = 9 per W; ns per wave unless stated; cell = median [min-max]')
P('\n## check: CSV-derived [0,500) values vs the SUMMARY w8s object, over every process at W 8 and 16')
worst = 0.0
nchk = 0
for W in (8, 16):
    for p in sel('SPLIT-J-T-a', W, B):
        m = split(p, '0..500')
        w = p['summary']['w8s']
        nchk += 1
        for k in SUMKEY:
            if m[k] is None:
                continue
            worst = max(worst, abs(m[k] - w[k]) / abs(w[k]))
        for k in ('first_waves', 'pass_waves', 'np_waves', 'np_route_worker'):
            if m[k] != w[k]:
                P('MISMATCH %s %s %s' % (k, m[k], w[k]))
        if abs(m['helped_per_step'] * 500 - w['helped_waves']) > 0.5:
            P('MISMATCH helped %s %s' % (m['helped_per_step'] * 500, w['helped_waves']))
P('%d processes checked; max rel diff of the 12 per-wave means %.2e' % (nchk, worst))

res = {}
for win in ('100..500', '0..500', '0..100'):
    P('\n## window [%s)' % win)
    for W in (1, 8, 16):
        ms = [split(p, win) for p in sel('SPLIT-J-T-a', W, B)]
        P('\n### W%d (K=%d)' % (W, len(ms)))
        res[(win, W)] = {}
        for k in KEYS:
            vals = [m[k] for m in ms if m[k] is not None]
            if not vals:
                P('%s: n/a (no dispatched wave)' % k)
                continue
            c = L.cell(vals)
            res[(win, W)][k] = c
            P('%s: %.3f [%.3f-%.3f] K=%d (IQR %.3f, SE %.3f)' % (k, c['median'], c['min'], c['max'], c['K'],
                                                                c['iqr_abs'], c['se_abs']))

P('\n## D(W) (review N2) and omega(W); J-T-a = armed, J-T = its disarmed twin, same block')
STG = ('setup', 'sb_pc', 'warm', 'integ_grp', 'bp', 'bp_query', 'graph', 'np_ser', 'narrow', 'np_disp', 'wide', 'gather')
for win in ('100..500', '0..500'):
    ta = {}
    for W in (1, 8, 16):
        ts = [terms(p, win) for p in sel('SPLIT-J-T-a', W, B)]
        ta[W] = {k: statistics.median([t[k] for t in ts]) for k in ts[0]}
    Td = {W: C('SPLIT-J-T', W, B, wall(win))['median'] for W in (1, 8, 16)}
    P('\n[%s) T_d W1/8/16 %.4f / %.4f / %.4f; T_a %.4f / %.4f / %.4f' % (
        win, Td[1], Td[8], Td[16], ta[1]['T'], ta[8]['T'], ta[16]['T']))
    P('   r recomputed vs runner r_ns (W8 medians) %.5f vs %.5f; u %.5f vs %.5f' % (
        ta[8]['r'], ta[8]['r_col'], ta[8]['u'], ta[8]['u_col']))
    for W in (8, 16):
        dr = ta[W]['r'] - ta[1]['r']
        du = ta[W]['u'] - ta[1]['u']
        D = (ta[W]['T'] - Td[W]) - (ta[1]['T'] - Td[1]) - dr - du
        wv = statistics.median([p['stats'][win]['phys_color_scopes'] for p in sel('SPLIT-J-T-a', W, B)])
        Lw = ta[W]['wide'] - ta[1]['wide'] / W
        P('W%d: D = [%+.4f] - [%+.4f] - dr %+.4f - du %+.4f = %+.4f ms; waves %.2f; D per wave %.3f us; '
          'L_wide %.4f ms; omega %.3f us; omega after D %.3f us; wide(W) %.4f, wide(1) %.4f' % (
              W, ta[W]['T'] - Td[W], ta[1]['T'] - Td[1], dr, du, D, wv, 1e3 * D / wv, Lw, 1e3 * Lw / wv,
              1e3 * (Lw - D) / wv, ta[W]['wide'], ta[1]['wide']))
        res[(win, 'D', W)] = {'D': D, 'Lw': Lw, 'waves': wv, 'omega': 1e3 * Lw / wv, 'omega_D': 1e3 * (Lw - D) / wv}
    P('   armed - disarmed: W1 %+.4f, W8 %+.4f, W16 %+.4f ms' % (
        ta[1]['T'] - Td[1], ta[8]['T'] - Td[8], ta[16]['T'] - Td[16]))
    for k in STG:
        P('   stage %s (armed median, ms): W1 %.4f, W8 %.4f, W16 %.4f' % (k, ta[1][k], ta[8][k], ta[16][k]))
    res[(win, 'terms')] = ta
    res[(win, 'Td')] = Td
import pickle
pickle.dump({str(k): v for k, v in res.items()}, open(os.path.join(L.HERE, 'q_split.pkl'), 'wb'))
save('q_split.txt')
