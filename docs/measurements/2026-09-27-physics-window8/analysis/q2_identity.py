"""Q2/Q3: the identity at W 8 and 16 (N1, N2 applied), per-stage ratios, per-wave telemetry."""
from common8 import *

SYS = ['sys_physics_gather_ns', 'sys_select_broadphase_ns', 'sys_physics_broadphase_colored_ns',
       'sys_physics_narrowphase_colored_ns', 'sys_physics_build_graph_ns', 'sys_physics_solve_colored_ns',
       'sys_physics_apply_ns', 'sys_physics_integrate_ns']
INSOLVE_SERIAL = ['phys_solve_build_ns', 'phys_gravity_ns', 'phys_warm_apply_ns', 'phys_integrate_ns',
                  'phys_restitution_ns', 'phys_store_ns', 'phys_write_back_ns', 'phys_sleep_begin_ns',
                  'phys_sleep_freeze_ns', 'phys_sleep_end_ns']


def terms(p, win='100..500'):
    s = p['stats'][win]
    g = lambda k: (s.get(k) or 0.0) / 1e6
    t = {}
    t['T'] = g('wall_ns')
    t['gather'] = g('sys_physics_gather_ns') + g('sys_select_broadphase_ns') + g('sys_physics_apply_ns') + g('sys_physics_integrate_ns')
    t['bp'] = g('sys_physics_broadphase_colored_ns')
    t['np'] = g('sys_physics_narrowphase_colored_ns')
    t['np_disp'] = g('phys_np_dispatch_ns')
    t['np_ser'] = t['np'] - t['np_disp']
    t['graph'] = g('sys_physics_build_graph_ns')
    t['solve'] = g('sys_physics_solve_colored_ns')
    t['setup'] = g('phys_solve_build_ns')
    t['sb_bodies'] = g('phys_sb_bodies_ns')
    t['sb_pa'] = g('phys_sb_pa_ns')
    t['sb_pb'] = g('phys_sb_pb_ns')
    t['sb_pc'] = g('phys_sb_pc_ns')
    t['warm'] = g('phys_warm_apply_ns')
    t['integ_grp'] = g('phys_gravity_ns') + g('phys_integrate_ns') + g('phys_store_ns') + g('phys_write_back_ns')
    t['gravity'] = g('phys_gravity_ns')
    t['integrate'] = g('phys_integrate_ns')
    t['store'] = g('phys_store_ns')
    t['write_back'] = g('phys_write_back_ns')
    t['restitution'] = g('phys_restitution_ns')
    t['sleep'] = g('phys_sleep_begin_ns') + g('phys_sleep_freeze_ns') + g('phys_sleep_end_ns')
    t['wide'] = g('phys_color_wide_ns')
    t['narrow'] = g('phys_color_narrow_ns')
    t['passes'] = g('phys_pass_biased_ns') + g('phys_pass_relax_ns')
    t['r'] = t['passes'] - t['wide'] - t['narrow']
    t['g'] = t['T'] - sum(g(k) for k in SYS)
    t['u'] = t['solve'] - sum(g(k) for k in INSOLVE_SERIAL) - t['passes']
    t['r_col'] = g('r_ns')
    t['g_col'] = g('g_ns')
    t['u_col'] = g('u_ns')
    t['bp_query'] = g('phys_bp_query_ns')
    t['bp_assemble'] = g('phys_bp_assemble_ns')
    t['bp_build'] = g('phys_bp_build_ns')
    t['bp_verify'] = g('phys_bp_verify_ns')
    t['np_compact'] = g('phys_np_compact_ns')
    t['np_axis'] = g('phys_np_axis_commit_ns')
    t['S_meas'] = (t['gather'] + t['bp'] + t['graph'] + t['np_ser'] + t['narrow']
                   + sum(g(k) for k in INSOLVE_SERIAL))
    t['closure_exact'] = t['T'] - (t['S_meas'] + t['np_disp'] + t['wide'] + t['r'] + t['u'] + t['g'])
    c = s
    t['waves'] = c.get('waves')
    t['scopes'] = c.get('phys_color_scopes')
    t['ramp_sum'] = (c.get('phys_wave_ramp') or 0) / 1e6
    t['tail_sum'] = (c.get('phys_wave_tail') or 0) / 1e6
    t['inflight_sum'] = c.get('phys_wave_inflight')
    t['lanes_sum'] = c.get('phys_wave_lanes')
    t['tasks'] = c.get('phys_color_tasks')
    t['route_w'] = c.get('phys_route_worker')
    t['route_x'] = c.get('phys_route_external')
    t['np_ramp'] = (c.get('phys_np_wave_ramp') or 0) / 1e6
    t['np_tail'] = (c.get('phys_np_wave_tail') or 0) / 1e6
    t['np_inflight'] = c.get('phys_np_wave_inflight')
    t['np_lanes'] = c.get('phys_np_wave_lanes')
    t['s6g'] = c.get('phys_s6_graph_hit')
    t['s6pb'] = c.get('phys_s6_pb_hit')
    t['colors'] = c.get('colors')
    t['wide_colors'] = c.get('wide_colors')
    t['slots_wide'] = c.get('phys_slots_wide')
    t['slots_narrow'] = c.get('phys_slots_narrow')
    for k in ('lt32', 'lt64', 'lt128', 'lt256', 'ge256'):
        t['hc_' + k] = c.get('phys_hist_colors_' + k)
        t['hs_' + k] = c.get('phys_hist_slots_' + k)
    t['manifolds'] = c.get('manifolds')
    return t


def TC(row, W, blocks, key, win='100..500'):
    return L.cell([terms(p, win)[key] for p in sel(row, W, blocks)])


def medterms(row, W, blocks, win):
    ts = [terms(p, win) for p in sel(row, W, blocks)]
    m = {k: statistics.median([t[k] for t in ts]) for k in ts[0] if ts[0][k] is not None}
    m['_K'] = len(ts)
    m['closure_exact_max'] = max(abs(t['closure_exact']) for t in ts)
    return m
