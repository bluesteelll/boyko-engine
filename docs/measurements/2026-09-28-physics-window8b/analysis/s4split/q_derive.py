"""Arithmetic on q_split's cells (medians; no spread, no claim): the B1/B2 decomposition per step and per wave,
shares of L_wide, W16 - W8 growth, and the S1-addressable dispatch loss (upper bound, arith.)."""
from common_s4 import *
import pickle

R = pickle.load(open(os.path.join(L.HERE, 'q_split.pkl'), 'rb'))
for win in ('100..500', '0..500'):
    P('\n## [%s)' % win)
    rows = {}
    for W in (8, 16):
        c = R[str((win, W))]
        m = {k: v['median'] for k, v in c.items()}
        D = R[str((win, 'D', W))]
        Lw = D['Lw'] * 1e3  # us per step
        rest = Lw - m['ramp_us_step'] - m['tail_us_step']
        rows[W] = dict(m, Lw=Lw, rest=rest)
        P('W%d per step (us): L_wide %.1f = ramp %.1f (%.1f %%) + join %.1f (%.1f %%) + imbalance %.1f (%.1f %%) + '
          'in-wave remainder %.1f (%.1f %%)' % (W, Lw, m['ramp_us_step'], 100 * m['ramp_us_step'] / Lw,
                                               m['join_us_step'], 100 * m['join_us_step'] / Lw, m['imb_us_step'],
                                               100 * m['imb_us_step'] / Lw, rest, 100 * rest / Lw))
        P('   ramp per step = first-wave %.2f + other pass-first %.2f + non-pass-first %.2f; pass-first share %.1f %%' % (
            m['first_ramp_us_step'], m['pass_ramp_us_step'] - m['first_ramp_us_step'],
            m['ramp_us_step'] - m['pass_ramp_us_step'], 100 * m['pass_ramp_us_step'] / m['ramp_us_step']))
        P('   per wave (ns): first ramp %.0f = %.1fx the helped-wave mean %.0f; pass-first mean %.0f = %.1fx the '
          'other waves %.0f; tail %.0f = imbalance %.0f (%.1f %%) + join %.0f (%.1f %%); ramp+tail %.0f = %.1f %% of '
          'omega %.0f' % (m['first_ramp_ns_mean'], m['first_ramp_ns_mean'] / m['ramp_ns_mean_helped'],
                          m['ramp_ns_mean_helped'], m['pass_ramp_ns_mean'],
                          m['pass_ramp_ns_mean'] / m['other_ramp_ns_mean'], m['other_ramp_ns_mean'], m['tail_ns_mean'],
                          m['imbalance_ns_mean'], 100 * m['imbalance_ns_mean'] / m['tail_ns_mean'], m['join_ns_mean'],
                          100 * m['join_ns_mean'] / m['tail_ns_mean'], m['ramp_ns_mean'] + m['tail_ns_mean'],
                          100 * (m['ramp_ns_mean'] + m['tail_ns_mean']) / (1e3 * D['omega']), 1e3 * D['omega']))
        P('   S1-addressable dispatch (arith., upper bound before S1 own costs): ramp %.1f + join %.1f - one '
          'recruitment %.1f = %.1f us/step; imbalance %.1f us/step is not dispatch' % (
              m['ramp_us_step'], m['join_us_step'], m['first_ramp_us_step'],
              m['ramp_us_step'] + m['join_us_step'] - m['first_ramp_us_step'], m['imb_us_step']))
        P('   np wave: ramp %.0f ns, tail %.0f ns = imbalance %.0f (%.1f %%) + join %.0f; route worker %.3f' % (
            m['np_ramp_ns_mean'], m['np_tail_ns_mean'], m['np_imbalance_ns_mean'],
            100 * m['np_imbalance_ns_mean'] / m['np_tail_ns_mean'], m['np_join_ns_mean'], m['np_route_worker_frac']))
    a, b = rows[8], rows[16]
    P('W16 - W8 per step (us): L_wide %+.1f; ramp %+.1f; join %+.1f; imbalance %+.1f; in-wave remainder %+.1f' % (
        b['Lw'] - a['Lw'], b['ramp_us_step'] - a['ramp_us_step'], b['join_us_step'] - a['join_us_step'],
        b['imb_us_step'] - a['imb_us_step'], b['rest'] - a['rest']))
    ta = R[str((win, 'terms'))]
    P('serial stages W8 (armed PARENT, ms) -> lever: setup %.4f (P-c %.4f) -> S4; warm %.4f -> S2; integrate group '
      '%.4f -> S3; bp query %.4f (bp %.4f) -> S5; graph %.4f -> S6 (not built); narrow colours %.4f -> S1 narrow '
      'term; np serial %.4f' % (ta[8]['setup'], ta[8]['sb_pc'], ta[8]['warm'], ta[8]['integ_grp'], ta[8]['bp_query'],
                                ta[8]['bp'], ta[8]['graph'], ta[8]['narrow'], ta[8]['np_ser']))
save('q_derive.txt')
