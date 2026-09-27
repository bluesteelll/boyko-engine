from q2_identity import *

OUT.clear()
P('# Q3 per wave (armed rows; per-process means over the window, cell = median over K)')
for win in ('100..500', '0..100'):
    P(f'\n## window [{win})')
    for row, W, bl in (('J-T-a', 8, ['W8S-A']), ('R-J-T-a', 8, ['W8S-R']), ('J-T-a', 16, ['W8S-A']),
                       ('J-T-off-a', 8, ['W8S-A']), ('J-T-off-a', 16, ['W8S-A']), ('J-A-a', 8, ['W8S-A']),
                       ('J-A-a', 16, ['W8S-A']), ('J-Son-a', 8, ['W8S-A'])):
        m = medterms(row, W, bl, win)
        wv = m['waves']
        sc = m['scopes']
        P(f"{row} W{W}: waves {wv:.2f} scopes {sc:.2f} tasks/scope {m['tasks'] / sc:.2f}; wide {m['wide']:.4f} ms -> {1e3 * m['wide'] / wv:.3f} us/wave;"
          f" ramp {1e3 * m['ramp_sum'] / sc:.3f} us/wave (sum {m['ramp_sum']:.4f} ms); tail {1e3 * m['tail_sum'] / sc:.3f} us/wave (sum {m['tail_sum']:.4f} ms);"
          f" inflight {m['inflight_sum'] / sc:.2f}; lanes {m['lanes_sum'] / sc:.2f}; route worker {m['route_w']:.2f} external {m['route_x']:.2f};"
          f" np: disp {m['np_disp']:.4f} ramp {1e3 * m['np_ramp']:.3f} us tail {1e3 * m['np_tail']:.3f} us inflight {m['np_inflight']} lanes {m['np_lanes']}")
    # omega per row, with W1 twin
    P('\n  omega(W) = [wide(W) - wide(1)/W]/waves, and the ramp+tail share of it')
    for row in ('J-T-a', 'J-T-off-a', 'J-A-a'):
        m1 = medterms(row, 1, ['W8S-A'], win)
        for W in (8, 16):
            m = medterms(row, W, ['W8S-A'], win)
            Lw = m['wide'] - m1['wide'] / W
            wv = m['waves']
            rt = (m['ramp_sum'] + m['tail_sum'])
            P(f"  {row} W{W}: L_wide {Lw:.4f} ms, waves {wv:.2f}, omega {1e3 * Lw / wv:.3f} us; ramp+tail per wave {1e3 * rt / wv:.3f} us = {100 * rt / Lw:.1f} % of L_wide;"
              f" E_wide {m1['wide'] / (W * m['wide']):.3f}; work/wave at W1 {1e3 * m1['wide'] / m1['waves']:.2f} us")
P('\n# colour histogram per step (colours; slots) [100,500) and [0,100)')
for win in ('100..500', '0..100'):
    for row, W in (('J-T-a', 8), ('J-T-off-a', 8), ('J-A-a', 8), ('J-Son-a', 8), ('J-T-a', 1)):
        m = medterms(row, W, ['W8S-A'], win)
        hc = [m['hc_' + k] for k in ('lt32', 'lt64', 'lt128', 'lt256', 'ge256')]
        hs = [m['hs_' + k] for k in ('lt32', 'lt64', 'lt128', 'lt256', 'ge256')]
        P(f"[{win}) {row} W{W}: colours {m['colors']:.2f} wide {m['wide_colors']:.2f}; hist colours {['%.2f' % x for x in hc]}; slots {['%.1f' % x for x in hs]}; slots wide {m['slots_wide']:.1f} narrow {m['slots_narrow']:.1f}; narrow span {m['narrow']:.4f} ms")
P('\n# S6 hit rates (per solving step, armed; mean of the 0/1 counters)')
for win in ('100..500', '0..100'):
    for row, W in (('J-T-a', 1), ('J-T-a', 8), ('J-T-a', 16), ('J-T-off-a', 8), ('J-A-a', 8), ('J-Son-a', 1), ('J-Son-a', 8)):
        ps = sel(row, W, ['W8S-A'])
        g = L.cell([p['stats'][win]['phys_s6_graph_hit'] for p in ps])
        b = L.cell([p['stats'][win]['phys_s6_pb_hit'] for p in ps])
        P(f"[{win}) {row} W{W}: graph hit {g['median']:.4f} [{g['min']:.4f}-{g['max']:.4f}], P-b hit {b['median']:.4f} [{b['min']:.4f}-{b['max']:.4f}] K={g['K']}")
save('q3.txt')
