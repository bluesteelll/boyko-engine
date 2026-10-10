#!/usr/bin/env python3
"""Window 4b (L11 C2 tip f8873aae against its parent C0 146a1125) - the results-analyst's own reduction.

Independent of the tester's reduce.py: reads win4b/raw/runs.jsonl and every per-process run.csv, re-derives
every window mean, selects per slot, forms the cells, applies G9 of 02-DESIGN-REV1.md, reduces the armed
spans, recomputes window 3's cells from its own raw for the bridge and the Jolt v5.6.0 reference, and writes
reduction.json + tables_analyst.md beside this script's output directory.

Statistic (the P0/window-3 ruling, restated in rows.json): a cell = MEDIAN over K separate processes of the
process's window mean (ms/step). Spreads = min-max range, IQR (inclusive quartiles), and the median's SE
(1.2533 * sample SD / sqrt K), each as % of the median. B against A is claimed iff
|median_B / median_A - 1| > 2 * hypot(spread_A %, spread_B %), read three ways (range / IQR / SE); G9 names
the SE form as the gate, and rows.json asks for the min-max reading to be printed beside it.

Selection: per (block, pass, seq) slot the ORIGINAL attempt is used when valid and uncontaminated
(5-s receipt before/after <= 5 % busy and no build process, per the driver's flags), else its RE-RUN when
valid and uncontaminated, else the slot is dropped and reported. (A re-run exists only after a contaminated
original, so this equals the tester's "prefer the re-run" rule; both are checked.)

Spans (armed rows): reading A (P0b's convention, the one the design's targets were derived in) = per process
the MEAN over the row's window of the per-step span, then the median over K; reading B (the tester's) = per
process the MEDIAN over steps [100,500) (J rows) / the row's window (others), then the median over K.
Per manifold = ms / 4,524.2 (the [0,500) manifold count, as the design specifies).
"""
import csv
import json
import math
import os
import statistics as st
import sys
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
WIN = os.path.abspath(os.path.join(HERE, '..'))
RAW = os.path.join(WIN, 'raw')
WIN3_RAW = 'D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl'
OUT_DIR = os.path.join(WIN, 'analyst')
os.makedirs(OUT_DIR, exist_ok=True)

DENOM = 4524.2  # the design's manifold denominator, [0,500) mean, J rows
BUSY_PCT = 5.0

# Design G9 bars (02-DESIGN-REV1.md 'Gates', G9): the listed numbers are ALREADY 0.6 x the lower predicted
# delta (targets table: solve_build 1.038 -> 0.55 => -0.488, x0.6 = -0.29; store 0.270 -> 0.08 => -0.19,
# x0.6 = -0.11; T(1) -1.35 x0.6 = -0.81; T(8) -1.01 x0.6 = -0.61).
GATES_DESIGN = {'solve_build': 0.29, 'store': 0.11, 'T1_JAs': 0.81, 'T8_JAs': 0.61}
# The task text's bars, which apply 0.6 a second time; printed beside the design's.
GATES_TASK = {k: round(v * 0.6, 3) for k, v in GATES_DESIGN.items()}
PRED_LOWER = {'solve_build': 0.488, 'store': 0.19, 'T1_JAs': 1.35, 'T8_JAs': 1.01}

SPAN_COLS = {
    'T_wall': 'wall_ns',
    'sys_sum': 'sys_sum_ns',
    'gather': 'sys_physics_gather_ns',
    'broadphase': 'sys_physics_broadphase_ns',
    'narrowphase': 'sys_physics_narrowphase_ns',
    'build_graph': 'sys_physics_build_graph_ns',
    'solve_colored': 'sys_physics_solve_colored_ns',
    'apply': 'sys_physics_apply_ns',
    'solve_build': 'phys_solve_build_ns',
    'gravity': 'phys_gravity_ns',
    'warm_apply': 'phys_warm_apply_ns',
    'integrate': 'phys_integrate_ns',
    'pass_biased': 'phys_pass_biased_ns',
    'pass_relax': 'phys_pass_relax_ns',
    'color_wide': 'phys_color_wide_ns',
    'color_narrow': 'phys_color_narrow_ns',
    'restitution': 'phys_restitution_ns',
    'store': 'phys_store_ns',
    'write_back': 'phys_write_back_ns',
    'sleep_begin': 'phys_sleep_begin_ns',
    'sleep_freeze': 'phys_sleep_freeze_ns',
    'sleep_end': 'phys_sleep_end_ns',
    'np_dispatch': 'phys_np_dispatch_ns',
    'np_compact': 'phys_np_compact_ns',
    'np_axis_commit': 'phys_np_axis_commit_ns',
    'g_gap': 'g_ns',
    'u': 'u_ns',
    'r': 'r_ns',
    'canary': 'sys_parity_canary_ns',
}
COUNT_COLS = ['manifolds', 'pairs', 'colors', 'wide_colors', 'waves', 'phys_slots_wide', 'phys_slots_narrow',
              'phys_np_points', 'phys_np_pairs', 'phys_np_manifolds', 'phys_bp_pairs', 'phys_np_chunks', 'awake']


def q_inclusive(xs):
    q = st.quantiles(xs, n=4, method='inclusive')
    return q[0], q[2]


def cell_stats(xs):
    xs = sorted(xs)
    n = len(xs)
    med = st.median(xs)
    if n >= 2:
        q1, q3 = q_inclusive(xs)
        sd = st.stdev(xs)
    else:
        q1 = q3 = med
        sd = 0.0
    se = 1.2533 * sd / math.sqrt(n)
    return {'n': n, 'median': med, 'min': xs[0], 'max': xs[-1], 'q1': q1, 'q3': q3, 'sd': sd, 'se': se,
            'range_pct': 100 * (xs[-1] - xs[0]) / med if med else None,
            'iqr_pct': 100 * (q3 - q1) / med if med else None,
            'se_pct': 100 * se / med if med else None,
            'values': xs}


def compare(a, b):
    """b against a. Effect = ratio - 1; bars = 2 * hypot(spread_a %, spread_b %); claimed per reading.
    Also the ms form: delta = med_b - med_a, bar_ms = 2 * hypot(se_a_ms, se_b_ms)."""
    if not a or not b or not a['median'] or not b['median']:
        return None
    ratio = b['median'] / a['median']
    eff = 100 * (ratio - 1)
    out = {'ratio': ratio, 'effect_pct': eff, 'delta_ms': b['median'] - a['median']}
    for k in ('range', 'iqr', 'se'):
        bar = 2 * math.hypot(a[f'{k}_pct'], b[f'{k}_pct'])
        out[f'bar_{k}_pct'] = bar
        out[f'claimed_{k}'] = abs(eff) > bar
    out['bar_se_ms'] = 2 * math.hypot(a['se'], b['se'])
    out['bar_range_ms'] = 2 * math.hypot(a['max'] - a['min'], b['max'] - b['min'])
    out['minmax_disjoint'] = (b['max'] < a['min']) or (a['max'] < b['min'])
    return out


def load_csv(path):
    with open(path, newline='') as f:
        rd = csv.DictReader(f)
        rows = list(rd)
    cols = {}
    for k in rows[0].keys():
        vals = []
        for r in rows:
            v = r[k]
            vals.append(float(v) if v not in ('', None) else None)
        cols[k] = vals
    return cols


def window_of(rec):
    a = rec['args']
    w = a[a.index('--window') + 1]
    lo, hi = w.split('..')
    return int(lo), int(hi)


def wmean(col, lo, hi):
    xs = [v for v in col[lo:hi] if v is not None]
    return st.mean(xs) if xs else None


def wmedian(col, lo, hi):
    xs = [v for v in col[lo:hi] if v is not None]
    return st.median(xs) if xs else None


def fmt(x, d=3):
    return '-' if x is None else f'{x:.{d}f}'


def main():
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
    timed = [r for r in recs if r.get('timed')]
    warm = [r for r in recs if not r.get('timed')]
    R = {'n_records': len(recs), 'n_timed': len(timed), 'n_warmups': len(warm)}

    # ---- selection ------------------------------------------------------------------------------------
    slots = defaultdict(dict)
    for r in timed:
        slots[(r['block'], r['pass'], r['seq'])][r['attempt']] = r
    used, dropped, tester_diff = [], [], []
    for key, att in slots.items():
        pick = None
        for tag in ('original', 'rerun'):
            a = att.get(tag)
            if a and a['valid'] and not a['contaminated']:
                pick = a
                break
        # the tester's rule: prefer the re-run
        tpick = None
        for tag in ('rerun', 'original'):
            a = att.get(tag)
            if a and a['valid'] and not a['contaminated']:
                tpick = a
                break
        if pick is None:
            dropped.append({'slot': key, 'attempts': {t: {'valid': a['valid'], 'contaminated': a['contaminated']} for t, a in att.items()}})
        else:
            used.append(pick)
            if tpick is not pick:
                tester_diff.append(key)
    R['selection'] = {'n_slots': len(slots), 'n_used': len(used), 'n_dropped': len(dropped), 'dropped': dropped,
                      'n_originals': sum(1 for r in timed if r['attempt'] == 'original'),
                      'n_reruns': sum(1 for r in timed if r['attempt'] == 'rerun'),
                      'n_used_reruns': sum(1 for r in used if r['attempt'] == 'rerun'),
                      'n_contaminated_attempts': sum(1 for r in timed if r['contaminated']),
                      'n_contam_before': sum(1 for r in timed if r['contaminated_before']),
                      'n_contam_after': sum(1 for r in timed if r['contaminated_after']),
                      'n_build_proc_during': sum(1 for r in timed if r['contaminated_during']),
                      'n_invalid': sum(1 for r in timed if not r['valid']),
                      'n_nonzero_exit': sum(1 for r in timed if r['exit'] != 0),
                      'slots_where_tester_rule_differs': tester_diff}

    # ---- per-process re-derivation from the CSVs --------------------------------------------------------
    manifest = json.load(open(os.path.join(RAW, 'manifest.json'), encoding='utf-8'))
    exp_pose = {rid: row['expected_pose'] for rid, row in manifest['rows'].items()}
    checks = {'mean_vs_driver_max_abs_ms': 0.0, 'mean_vs_runner_max_abs_ns': 0.0, 'pose_mismatch': [], 'expect_pose_not_match': [],
              'nonzero_exit': [], 'void_or_drops': [], 'workers_mismatch': [], 'mask_not_ffff': [], 'sha_mismatch': [],
              'n_steps_bad': [], 'armed_flag_bad': [], 'config_mismatch_across_binaries': []}
    per_proc = []
    cfg_by = {}
    for r in used:
        cols = load_csv(r['csv'])
        lo, hi = window_of(r)
        wall = cols['wall_ns']
        m = wmean(wall, lo, hi) / 1e6
        checks['mean_vs_driver_max_abs_ms'] = max(checks['mean_vs_driver_max_abs_ms'], abs(m - r['mean_ms']))
        checks['mean_vs_runner_max_abs_ns'] = max(checks['mean_vs_runner_max_abs_ns'], abs(m * 1e6 - r['summary']['window_mean_ns']))
        s = r['summary']
        tag = f"{r['row']}#{r['binary']}@W{r['W']} p{r['pass']} s{r['seq']} {r['attempt']}"
        if s['pose_hash'] != exp_pose[r['row']]:
            checks['pose_mismatch'].append(tag)
        if '--expect-pose' in r['args'] and s.get('expect_pose') != 'match':
            checks['expect_pose_not_match'].append(tag)
        if r['exit'] != 0:
            checks['nonzero_exit'].append(tag)
        if s.get('void_steps') or s.get('drops_total') or s.get('disarmed_ring_traffic'):
            checks['void_or_drops'].append(tag)
        if s['threads']['pool_workers'] != r['W']:
            checks['workers_mismatch'].append(tag)
        if r['mask_readback'] != '0xffff':
            checks['mask_not_ffff'].append(tag)
        if r['exe_sha256'] != manifest['binaries'][r['binary']]['sha256']:
            checks['sha_mismatch'].append(tag)
        if (hi - lo) != r['n_steps'] or r['n_steps'] != r['expect_steps'] or len(wall) < hi:
            checks['n_steps_bad'].append(tag)
        if bool(s.get('armed')) != bool(manifest['rows'][r['row']]['armed']):
            checks['armed_flag_bad'].append(tag)
        key = (r['row'], r['W'])
        cfg = json.dumps(s['config'], sort_keys=True)
        cfg_by.setdefault(key, {})[r['binary']] = cfg
        sub_lo = max(lo, 100) if lo == 0 else lo
        p = {'row': r['row'], 'W': r['W'], 'binary': r['binary'], 'pass': r['pass'], 'seq': r['seq'], 'round': r['round'],
             'attempt': r['attempt'], 'start': r['start'], 'mean_ms': m, 'median_step_ms': wmedian(wall, lo, hi) / 1e6,
             'mean_sub_ms': wmean(wall, sub_lo, hi) / 1e6, 'others_busy_pct': r['others_busy_pct'],
             'rb': r['receipt_before']['cpu_avg'], 'ra': r['receipt_after']['cpu_avg'],
             'manifolds_window': wmean(cols['manifolds'], lo, hi), 'manifolds_sub': wmean(cols['manifolds'], sub_lo, hi),
             'pairs_window': wmean(cols['pairs'], lo, hi), 'armed': bool(s.get('armed')), 'canary_ns': s.get('canary_ns'),
             'wave_total': s.get('waves_total'), 'cfg': s.get('cfg')}
        if s.get('armed'):
            spans_a, spans_b, counts = {}, {}, {}
            for name, c in SPAN_COLS.items():
                if c in cols:
                    spans_a[name] = wmean(cols[c], lo, hi)
                    spans_b[name] = wmedian(cols[c], sub_lo, hi)
            for c in COUNT_COLS:
                if c in cols:
                    counts[c] = wmean(cols[c], lo, hi)
            p['spans_mean'] = {k: (v / 1e6 if v is not None else None) for k, v in spans_a.items()}
            p['spans_med'] = {k: (v / 1e6 if v is not None else None) for k, v in spans_b.items()}
            p['counts'] = counts
        per_proc.append(p)
    for key, d in cfg_by.items():
        if len(set(d.values())) != 1:
            checks['config_mismatch_across_binaries'].append({'row_W': key, 'cfgs': d})
    R['checks'] = checks
    R['configs'] = {f'{k[0]}@W{k[1]}': json.loads(list(v.values())[0]) for k, v in cfg_by.items()}

    # ---- cells ----------------------------------------------------------------------------------------
    by_cell = defaultdict(list)
    for p in per_proc:
        by_cell[(p['row'], p['W'], p['binary'])].append(p)
    cells = {}
    for key, ps in by_cell.items():
        c = cell_stats([p['mean_ms'] for p in ps])
        c['per_pass_median'] = {pa: st.median([p['mean_ms'] for p in ps if p['pass'] == pa]) for pa in sorted({p['pass'] for p in ps})}
        c['per_pass_n'] = {pa: sum(1 for p in ps if p['pass'] == pa) for pa in sorted({p['pass'] for p in ps})}
        c['sub_median'] = st.median([p['mean_sub_ms'] for p in ps])
        c['manifolds_window'] = st.median([p['manifolds_window'] for p in ps])
        c['manifolds_sub'] = st.median([p['manifolds_sub'] for p in ps])
        c['witness_max'] = max(p['others_busy_pct'] for p in ps)
        c['witness_over5'] = [f"p{p['pass']}s{p['seq']}:{p['others_busy_pct']}" for p in ps if p['others_busy_pct'] > 5]
        c['procs'] = sorted([(p['pass'], p['seq'], p['attempt'], round(p['mean_ms'], 4), p['others_busy_pct']) for p in ps])
        cells[key] = c
    R['cells'] = {f'{k[0]}@W{k[1]}#{k[2]}': v for k, v in cells.items()}

    def cell(row, w, b):
        return cells.get((row, w, b))

    # ---- tip against parent, every row/W ---------------------------------------------------------------
    rowsW = sorted({(k[0], k[1]) for k in cells}, key=lambda x: (x[0], x[1]))
    comps = {}
    for row, w in rowsW:
        a, b = cell(row, w, 'parent'), cell(row, w, 'tip')
        if a and b:
            cmp_ = compare(a, b)
            # pass-split deltas
            cmp_['delta_pass'] = {pa: b['per_pass_median'].get(pa, float('nan')) - a['per_pass_median'].get(pa, float('nan'))
                                  for pa in a['per_pass_median']}
            # sensitivity: drop processes whose during-witness > 5 %
            a2 = [p['mean_ms'] for p in by_cell[(row, w, 'parent')] if p['others_busy_pct'] <= 5]
            b2 = [p['mean_ms'] for p in by_cell[(row, w, 'tip')] if p['others_busy_pct'] <= 5]
            if len(a2) >= 2 and len(b2) >= 2:
                cmp_['sens_witness5'] = compare(cell_stats(a2), cell_stats(b2)) | {'nA': len(a2), 'nB': len(b2)}
            # sub-window [100,500) reading
            a3 = cell_stats([p['mean_sub_ms'] for p in by_cell[(row, w, 'parent')]])
            b3 = cell_stats([p['mean_sub_ms'] for p in by_cell[(row, w, 'tip')]])
            cmp_['sub_window'] = compare(a3, b3) | {'medA': a3['median'], 'medB': b3['median']}
            comps[f'{row}@W{w}'] = cmp_
    R['tip_vs_parent'] = comps

    # ---- G9 realized-gain gates on T ---------------------------------------------------------------------
    gates = {}
    for name, row, w in (('T1_JAs', 'J-As', 1), ('T8_JAs', 'J-As', 8)):
        c = comps[f'{row}@W{w}']
        gates[name] = {'row': f'{row}@W{w}', 'parent': cell(row, w, 'parent')['median'], 'tip': cell(row, w, 'tip')['median'],
                       'delta_ms': c['delta_ms'], 'bar_se_ms': c['bar_se_ms'], 'claimed_se': c['claimed_se'], 'claimed_range': c['claimed_range'],
                       'claimed_iqr': c['claimed_iqr'], 'predicted_lower_delta': PRED_LOWER[name],
                       'gate_design': GATES_DESIGN[name], 'gate_task': GATES_TASK[name]}
        gates[name]['verdict_design'] = ('PASS' if c['claimed_se'] and c['delta_ms'] <= -GATES_DESIGN[name] else
                                         ('NOT CLAIMED' if not c['claimed_se'] else 'FAIL'))
        gates[name]['verdict_task'] = ('PASS' if c['claimed_se'] and c['delta_ms'] <= -GATES_TASK[name] else
                                       ('NOT CLAIMED' if not c['claimed_se'] else 'FAIL'))
        gates[name]['margin_design_ms'] = -c['delta_ms'] - GATES_DESIGN[name]
        gates[name]['delta_minus_bar_se'] = -c['delta_ms'] - c['bar_se_ms']

    # ---- armed spans -----------------------------------------------------------------------------------
    armed_cells = {}
    for key, ps in by_cell.items():
        if not all(p['armed'] for p in ps):
            continue
        row, w, b = key
        out = {'n': len(ps), 'A': {}, 'B': {}, 'counts': {}}
        for name in SPAN_COLS:
            va = [p['spans_mean'][name] for p in ps if p['spans_mean'].get(name) is not None]
            vb = [p['spans_med'][name] for p in ps if p['spans_med'].get(name) is not None]
            if va:
                out['A'][name] = cell_stats(va)
            if vb:
                out['B'][name] = cell_stats(vb)
        for cname in COUNT_COLS:
            v = [p['counts'][cname] for p in ps if p['counts'].get(cname) is not None]
            if v:
                out['counts'][cname] = {'median': st.median(v), 'min': min(v), 'max': max(v)}
        # derived: setup = solve_build + warm_apply + store ; kernel = wide + narrow ; per process then stats
        for rd in ('A', 'B'):
            k = 'spans_mean' if rd == 'A' else 'spans_med'
            setup = [p[k]['solve_build'] + p[k]['warm_apply'] + p[k]['store'] for p in ps]
            kern = [p[k]['color_wide'] + p[k]['color_narrow'] for p in ps]
            out[rd]['setup_total'] = cell_stats(setup)
            out[rd]['kernel_total'] = cell_stats(kern)
            solve_serial = [p[k]['solve_build'] + p[k]['gravity'] + p[k]['warm_apply'] + p[k]['integrate'] + p[k]['restitution']
                            + p[k]['store'] + p[k]['write_back'] + p[k]['sleep_begin'] + p[k]['sleep_freeze'] + p[k]['sleep_end'] for p in ps]
            out[rd]['solve_serial_sum'] = cell_stats(solve_serial)
        armed_cells[key] = out
    R['armed_cells'] = {f'{k[0]}@W{k[1]}#{k[2]}': v for k, v in armed_cells.items()}

    span_comps = {}
    for (row, w, b) in list(armed_cells):
        if b != 'parent' or (row, w, 'tip') not in armed_cells:
            continue
        A, B = armed_cells[(row, w, 'parent')], armed_cells[(row, w, 'tip')]
        sc = {}
        for rd in ('A', 'B'):
            sc[rd] = {}
            for name in A[rd]:
                if name in B[rd]:
                    c = compare(A[rd][name], B[rd][name])
                    if c:
                        c['parent_ms'] = A[rd][name]['median']
                        c['tip_ms'] = B[rd][name]['median']
                        c['parent_us_per_manifold'] = A[rd][name]['median'] * 1000 / DENOM
                        c['tip_us_per_manifold'] = B[rd][name]['median'] * 1000 / DENOM
                        c['parent_se_ms'] = A[rd][name]['se']
                        c['tip_se_ms'] = B[rd][name]['se']
                        sc[rd][name] = c
        span_comps[f'{row}@W{w}'] = sc
    R['span_comps'] = span_comps

    # stage gates: solve_build, store (J-As-a at W1 and W8), warm_apply reported not gated; wide colours not slower
    stage_gates = {}
    for w in (1, 8):
        key = f'J-As-a@W{w}'
        if key not in span_comps:
            continue
        for rd in ('A', 'B'):
            for stage in ('solve_build', 'store', 'warm_apply', 'setup_total', 'color_wide', 'kernel_total', 'color_narrow', 'solve_colored'):
                c = span_comps[key][rd].get(stage)
                if not c:
                    continue
                g = {'parent_ms': c['parent_ms'], 'tip_ms': c['tip_ms'], 'delta_ms': c['delta_ms'], 'effect_pct': c['effect_pct'],
                     'bar_se_ms': c['bar_se_ms'], 'bar_se_pct': c['bar_se_pct'], 'bar_range_pct': c['bar_range_pct'], 'bar_iqr_pct': c['bar_iqr_pct'],
                     'claimed_se': c['claimed_se'], 'claimed_range': c['claimed_range'], 'claimed_iqr': c['claimed_iqr'],
                     'parent_us_pm': c['parent_us_per_manifold'], 'tip_us_pm': c['tip_us_per_manifold']}
                if stage in GATES_DESIGN:
                    g['gate_design'] = GATES_DESIGN[stage]
                    g['gate_task'] = GATES_TASK[stage]
                    g['verdict_design'] = ('PASS' if c['claimed_se'] and c['delta_ms'] <= -GATES_DESIGN[stage] else
                                           ('NOT CLAIMED' if not c['claimed_se'] else 'FAIL'))
                    g['verdict_task'] = ('PASS' if c['claimed_se'] and c['delta_ms'] <= -GATES_TASK[stage] else
                                         ('NOT CLAIMED' if not c['claimed_se'] else 'FAIL'))
                if stage in ('color_wide', 'kernel_total', 'color_narrow'):
                    g['not_claimed_slower'] = not (c['delta_ms'] > 0 and c['claimed_se'])
                    g['claimed_faster'] = c['delta_ms'] < 0 and c['claimed_se']
                stage_gates[f'{key}/{rd}/{stage}'] = g
    R['stage_gates'] = stage_gates
    R['T_gates'] = gates

    # cfg-A not slower at any W; wide colours; J-As-a vs J-As (armed vs disarmed)
    cfgA = {}
    for w in (1, 2, 4, 8, 16):
        c = comps.get(f'J-A@W{w}')
        if c:
            cfgA[w] = {'delta_ms': c['delta_ms'], 'effect_pct': c['effect_pct'], 'claimed_se': c['claimed_se'], 'claimed_range': c['claimed_range'],
                       'claimed_iqr': c['claimed_iqr'], 'not_claimed_slower': not (c['delta_ms'] > 0 and c['claimed_se']),
                       'claimed_faster_se': c['delta_ms'] < 0 and c['claimed_se']}
    R['cfgA_gate'] = cfgA
    arm_vs_dis = {}
    for w in (1, 8):
        for b in ('parent', 'tip'):
            a, d = cell('J-As', w, b), cell('J-As-a', w, b)
            if a and d:
                arm_vs_dis[f'W{w}#{b}'] = compare(a, d)
    R['armed_vs_disarmed'] = arm_vs_dis

    # scaling
    scal = {}
    for row in ('J-As', 'J-A'):
        for b in ('parent', 'tip'):
            t1 = cell(row, 1, b)
            if t1:
                scal[f'{row}#{b}'] = {f'T1/T{w}': t1['median'] / cell(row, w, b)['median'] for w in (2, 4, 8, 16) if cell(row, w, b)}
                c16 = compare(cell(row, 8, b), cell(row, 16, b)) if cell(row, 16, b) else None
                scal[f'{row}#{b}']['W16_vs_W8'] = c16
    R['scaling'] = scal

    # ---- canary ----------------------------------------------------------------------------------------
    can = {}
    for b in ('parent', 'tip'):
        jc = [p for p in per_proc if p['row'] == 'J-C' and p['binary'] == b]
        ref = armed_cells.get(('J-As-a', 1, b))
        refc = cell('J-As-a', 1, b)
        for p in jc:
            span = p['spans_mean'].get('canary')
            can[b] = {'canary_ns': p['canary_ns'], 'span_mean_ms': span, 'span_rel_err_pct': (span * 1e6 / p['canary_ns'] - 1) * 100 if span and p['canary_ns'] else None,
                      'wall_ms': p['mean_ms'], 'jasa_W1_median': refc['median'] if refc else None,
                      'wall_rise_ms': p['mean_ms'] - refc['median'] if refc else None,
                      'expected_rise_ms': p['canary_ns'] / 1e6 if p['canary_ns'] else None,
                      'jasa_W1_minmax': [refc['min'], refc['max']] if refc else None,
                      'inside_jasa_minmax': (refc['min'] <= p['mean_ms'] <= refc['max']) if refc else None,
                      'others_busy_pct': p['others_busy_pct']}
    R['canary'] = can

    # ---- receipts --------------------------------------------------------------------------------------
    rc = {}
    seen = {}
    for r in timed:
        for k in ('receipt_before', 'receipt_after'):
            rr = r[k]
            seen[rr['time']] = rr
    vals = sorted(v['cpu_avg'] for v in seen.values())
    rc['n_distinct'] = len(vals)
    rc['median'] = st.median(vals)
    rc['p90'] = st.quantiles(vals, n=10)[-1]
    rc['max'] = vals[-1]
    rc['over5'] = sum(1 for v in vals if v > BUSY_PCT)
    rc['with_build_proc'] = sum(1 for v in seen.values() if v['build_procs_busy'])
    rc['over5_by_block_pass'] = {}
    for r in timed:
        bp = f"{r['block']}-p{r['pass']}"
        rc['over5_by_block_pass'].setdefault(bp, 0)
        rc['over5_by_block_pass'][bp] += int(r['contaminated_before']) + int(r['contaminated_after'])
    wit = sorted(p['others_busy_pct'] for p in per_proc)
    rc['witness_used'] = {'n': len(wit), 'median': st.median(wit), 'p95': st.quantiles(wit, n=20)[-1], 'max': wit[-1],
                          'over5': sum(1 for v in wit if v > 5), 'over2': sum(1 for v in wit if v > 2)}
    rc['build_proc_during_any'] = sum(1 for r in timed if r['build_proc_during'])
    # top background names in the used processes' others_top5
    names = defaultdict(float)
    for r in used:
        for t in r['others_top5']:
            names[t['name']] += t['cpu_s']
    rc['background_cpu_s_by_name_top8'] = sorted(names.items(), key=lambda x: -x[1])[:8]
    rc['timing'] = {'first_start': min(r['start'] for r in timed), 'last_end': max(r['end'] for r in timed)}
    for blk in sorted({(r['block'], r['pass']) for r in timed}):
        rs = [r for r in timed if (r['block'], r['pass']) == blk]
        rc['timing'][f'{blk[0]}-p{blk[1]}'] = [min(r['start'] for r in rs)[11:19], max(r['end'] for r in rs)[11:19], len(rs)]
    R['receipts'] = rc

    # ---- window 3 recomputed: the bridge and the Jolt v5.6.0 reference -----------------------------------
    w3 = [json.loads(l) for l in open(WIN3_RAW, encoding='utf-8')]
    w3slots = defaultdict(dict)
    for r in w3:
        if r['attempt'] == 'warmup':
            continue
        w3slots[(r['pass'], r['seq'])][r['attempt']] = r
    w3cells = defaultdict(list)
    for key, att in w3slots.items():
        for tag in ('original', 'rerun'):
            a = att.get(tag)
            if a and a['valid'] and not a['contaminated']:
                w3cells[(a['row'], a['W'])].append(a['mean_ms'])
                break
    w3c = {k: cell_stats(v) for k, v in w3cells.items()}
    R['win3_cells'] = {f'{k[0]}@W{k[1]}': v for k, v in w3c.items()}
    bridge = {}
    pairs = [('J-A@W1 parent vs win3 J-A@W1 (equal knobs)', cell('J-A', 1, 'parent'), w3c[('J-A', 1)]),
             ('J-As@W1 parent vs win3 D-L5@W1 (same code path at one worker; knobs print differently)', cell('J-As', 1, 'parent'), w3c[('D-L5', 1)]),
             ('J-As@W8 parent vs win3 D-L5@W8 (KNOB MISMATCH: parallel_broadphase on in `as`, off in default)', cell('J-As', 8, 'parent'), w3c[('D-L5', 8)]),
             ('J-A@W8 parent vs win3 J-A@W8 (L5 parallel narrowphase landed between; expected about -2.4 ms)', cell('J-A', 8, 'parent'), w3c[('J-A', 8)]),
             ('J-A@W1 tip vs win3 J-A@W1', cell('J-A', 1, 'tip'), w3c[('J-A', 1)]),
             ('J-As@W1 tip vs win3 D-L5@W1', cell('J-As', 1, 'tip'), w3c[('D-L5', 1)])]
    for name, a, b in pairs:
        c = compare(b, a)  # this window against window 3
        c['win3_median'] = b['median']
        c['win3_minmax'] = [b['min'], b['max']]
        c['this_median'] = a['median']
        c['this_minmax'] = [a['min'], a['max']]
        c['this_inside_win3_minmax'] = b['min'] <= a['median'] <= b['max']
        c['win3_inside_this_minmax'] = a['min'] <= b['median'] <= a['max']
        bridge[name] = c
    R['bridge'] = bridge

    jolt = {}
    for w in (1, 2, 4, 8, 16):
        j = w3c[('JOLT56-T', w)]
        for row in ('J-As', 'J-A'):
            for b in ('parent', 'tip'):
                c0 = cell(row, w, b)
                if c0:
                    cc = compare(j, c0)
                    cc['boyko'] = c0['median']
                    cc['jolt56'] = j['median']
                    cc['jolt_minmax'] = [j['min'], j['max']]
                    jolt[f'{row}#{b}@W{w}'] = cc
    R['jolt56_reference'] = jolt
    # per manifold [100,500), each side's own count: boyko from its CSVs; Jolt v5.6.0 from window 3's per-frame files
    # (the used JOLT56-T processes, mean over frames [100,500), median over K) and its `-receipt` manifold column.
    w3base = os.path.dirname(WIN3_RAW)
    jolt_sub = defaultdict(list)
    for key, att in w3slots.items():
        for tag in ('original', 'rerun'):
            a = att.get(tag)
            if a and a['valid'] and not a['contaminated']:
                if a['row'] == 'JOLT56-T':
                    p = os.path.join(w3base, f"pass-{a['pass']:02d}", os.path.basename(a['cwd']), os.path.basename(a['per_frame']))
                    with open(p) as f:
                        t = [float(x[1]) for x in csv.reader(f) if x and x[0].strip().isdigit()]
                    jolt_sub[a['W']].append((st.mean(t[0:500]), st.mean(t[100:500])))
                break
    with open(os.path.join(w3base, 'receipts', 'JOLT56-T_receipt_W1', 'receipt_discrete_th1.csv')) as f:
        rows_ = list(csv.DictReader(f))
    mk = [k for k in rows_[0] if 'Manifold' in k][0]
    jm = [float(r[mk]) for r in rows_]
    JOLT56_MANIFOLDS_SUB = st.mean(jm[100:500])
    JOLT56_MANIFOLDS_FULL = st.mean(jm[0:500])
    R['jolt56_sub_window'] = {w: {'K': len(v), 'full_median': st.median([a for a, b in v]), 'sub_median': st.median([b for a, b in v]),
                                  'us_pm_sub': st.median([b for a, b in v]) * 1000 / JOLT56_MANIFOLDS_SUB} for w, v in jolt_sub.items()}
    R['jolt56_manifolds'] = {'sub_100_500': JOLT56_MANIFOLDS_SUB, 'full_0_500': JOLT56_MANIFOLDS_FULL}
    pm = {}
    for w in (1, 8):
        js = R['jolt56_sub_window'][w]
        for b in ('parent', 'tip'):
            for row in ('J-As', 'J-A'):
                c0 = cell(row, w, b)
                us = c0['sub_median'] * 1000 / c0['manifolds_sub']
                pm[f'{row}#{b}@W{w}'] = {'boyko_sub_median_ms': c0['sub_median'], 'boyko_us_pm': us, 'boyko_manifolds_sub': c0['manifolds_sub'],
                                         'jolt56_sub_median_ms': js['sub_median'], 'jolt56_us_pm': js['us_pm_sub'], 'jolt56_manifolds_sub': JOLT56_MANIFOLDS_SUB,
                                         'ratio_us_pm': us / js['us_pm_sub'], 'ratio_step_sub': c0['sub_median'] / js['sub_median']}
    R['per_manifold_vs_jolt56'] = pm

    json.dump(R, open(os.path.join(OUT_DIR, 'reduction.json'), 'w'), indent=1, default=str)

    # ---- tables ---------------------------------------------------------------------------------------
    L = []
    P = L.append
    P('# analyst tables (win4b) - generated by tools/analyze_win4b.py')
    P('')
    S = R['selection']
    P(f"records {R['n_records']}: warm-ups {R['n_warmups']}, timed attempts {R['n_timed']} (originals {S['n_originals']}, re-runs {S['n_reruns']}); "
      f"slots {S['n_slots']}, used {S['n_used']} (re-runs used {S['n_used_reruns']}), dropped {S['n_dropped']}; contaminated attempts {S['n_contaminated_attempts']} "
      f"(before {S['n_contam_before']}, after {S['n_contam_after']}, build-proc-during {S['n_build_proc_during']}); invalid {S['n_invalid']}; non-zero exits {S['n_nonzero_exit']}; "
      f"slots where the tester's prefer-rerun rule picks differently: {S['slots_where_tester_rule_differs']}")
    P(f"checks: {json.dumps({k: (v if not isinstance(v, list) else len(v)) for k, v in checks.items()})}")
    P('')
    P('## cells (median [min-max]; range/IQR/SE %; per-pass medians; values)')
    P('| cell | K | median | min-max | r / i / s % | pass0 / pass1 | manif [0,500) / sub | witness max | values |')
    P('|---|---|---|---|---|---|---|---|---|')
    for row, w in rowsW:
        for b in ('parent', 'tip'):
            c = cell(row, w, b)
            if not c:
                continue
            P(f"| {row}@W{w}#{b} | {c['n']} | {c['median']:.4f} | [{c['min']:.3f}-{c['max']:.3f}] | {c['range_pct']:.2f} / {c['iqr_pct']:.2f} / {c['se_pct']:.2f} | "
              f"{' / '.join(f'{v:.3f}' for v in c['per_pass_median'].values())} | {c['manifolds_window']:.1f} / {c['manifolds_sub']:.1f} | {c['witness_max']:.2f} | "
              f"{', '.join(f'{v:.3f}' for v in c['values'])} |")
    P('')
    P('## tip against parent')
    P('| row@W | parent | tip | ratio | effect % | delta ms | bars r / i / s % | bar_se ms | claimed r/i/s | minmax disjoint | delta pass0 / pass1 | sens(witness<=5): eff %, claimed r/i/s, nA/nB | sub-window [100,500) eff %, claimed s |')
    P('|---|---|---|---|---|---|---|---|---|---|---|---|---|')
    for k, c in comps.items():
        row, w = k.split('@W')
        a, b = cell(row, int(w), 'parent'), cell(row, int(w), 'tip')
        sw = c.get('sens_witness5')
        yn = lambda cc: f"{'Y' if cc['claimed_range'] else 'n'}/{'Y' if cc['claimed_iqr'] else 'n'}/{'Y' if cc['claimed_se'] else 'n'}"
        sw_txt = f"{sw['effect_pct']:+.2f}, {yn(sw)}, {sw['nA']}/{sw['nB']}" if sw else '-'
        pass_txt = ' / '.join(f'{v:+.3f}' for v in c['delta_pass'].values())
        P(f"| {k} | {a['median']:.3f} | {b['median']:.3f} | {c['ratio']:.4f} | {c['effect_pct']:+.2f} | {c['delta_ms']:+.3f} | "
          f"{c['bar_range_pct']:.2f} / {c['bar_iqr_pct']:.2f} / {c['bar_se_pct']:.2f} | {c['bar_se_ms']:.3f} | "
          f"{yn(c)} | {c['minmax_disjoint']} | {pass_txt} | {sw_txt} | "
          f"{c['sub_window']['effect_pct']:+.2f}, {'Y' if c['sub_window']['claimed_se'] else 'n'} |")
    P('')
    P('## G9 T gates')
    for name, g in gates.items():
        P(f"- {name} ({g['row']}): parent {g['parent']:.3f} -> tip {g['tip']:.3f}, delta {g['delta_ms']:+.3f} ms (bar_se {g['bar_se_ms']:.3f} ms; claimed r/i/s "
          f"{'Y' if g['claimed_range'] else 'n'}/{'Y' if g['claimed_iqr'] else 'n'}/{'Y' if g['claimed_se'] else 'n'}); lower predicted delta -{g['predicted_lower_delta']}; "
          f"design bar -{g['gate_design']} => {g['verdict_design']} (margin {g['margin_design_ms']:+.3f} ms); task bar -{g['gate_task']} => {g['verdict_task']}")
    P('')
    P('## armed spans, J-As-a (reading A = window MEAN per process then median over K [P0b convention]; reading B = MEDIAN over steps [100,500) per process then median over K [tester])')
    for key in ('J-As-a@W1', 'J-As-a@W8', 'R@W1', 'R@W8', 'R-S@W1', 'R-S@W8', 'S16@W1'):
        if key not in span_comps:
            continue
        for rd in ('A', 'B'):
            P(f'### {key} reading {rd}')
            P('| stage | parent ms | tip ms | delta ms | effect % | parent us/m | tip us/m | bars r/i/s % | bar_se ms | claimed r/i/s |')
            P('|---|---|---|---|---|---|---|---|---|---|')
            for stage in ('T_wall', 'sys_sum', 'gather', 'broadphase', 'narrowphase', 'build_graph', 'solve_colored', 'apply', 'solve_build', 'warm_apply', 'store',
                          'setup_total', 'gravity', 'integrate', 'restitution', 'write_back', 'pass_biased', 'pass_relax', 'color_wide', 'color_narrow', 'kernel_total',
                          'solve_serial_sum', 'sleep_begin', 'sleep_freeze', 'sleep_end', 'np_dispatch', 'np_compact', 'np_axis_commit', 'g_gap', 'u', 'r'):
                c = span_comps[key][rd].get(stage)
                if not c:
                    continue
                P(f"| {stage} | {c['parent_ms']:.4f} | {c['tip_ms']:.4f} | {c['delta_ms']:+.4f} | {c['effect_pct']:+.2f} | {c['parent_us_per_manifold']:.4f} | {c['tip_us_per_manifold']:.4f} | "
                  f"{c['bar_range_pct']:.2f} / {c['bar_iqr_pct']:.2f} / {c['bar_se_pct']:.2f} | {c['bar_se_ms']:.4f} | "
                  f"{'Y' if c['claimed_range'] else 'n'}/{'Y' if c['claimed_iqr'] else 'n'}/{'Y' if c['claimed_se'] else 'n'} |")
            P('')
        A = armed_cells[(key.split('@W')[0], int(key.split('@W')[1]), 'parent')]
        B = armed_cells[(key.split('@W')[0], int(key.split('@W')[1]), 'tip')]
        P(f"counts parent: {json.dumps({k: round(v['median'], 3) for k, v in A['counts'].items()})}")
        P(f"counts tip:    {json.dumps({k: round(v['median'], 3) for k, v in B['counts'].items()})}")
        P('')
    P('## stage gates')
    for k, g in stage_gates.items():
        extra = ''
        if 'verdict_design' in g:
            extra = f"; design bar -{g['gate_design']} => {g['verdict_design']}; task bar -{g['gate_task']} => {g['verdict_task']}"
        if 'not_claimed_slower' in g:
            extra = f"; not claimed slower: {g['not_claimed_slower']}; claimed faster (SE): {g['claimed_faster']}"
        P(f"- {k}: {g['parent_ms']:.4f} -> {g['tip_ms']:.4f} ms ({g['parent_us_pm']:.4f} -> {g['tip_us_pm']:.4f} us/m), delta {g['delta_ms']:+.4f} ({g['effect_pct']:+.1f} %), bar_se {g['bar_se_ms']:.4f} ms, "
          f"claimed r/i/s {'Y' if g['claimed_range'] else 'n'}/{'Y' if g['claimed_iqr'] else 'n'}/{'Y' if g['claimed_se'] else 'n'}{extra}")
    P('')
    P('## cfg-A gate (J-A, tip vs parent)')
    for w, g in cfgA.items():
        P(f"- W{w}: delta {g['delta_ms']:+.3f} ms ({g['effect_pct']:+.2f} %), claimed r/i/s {'Y' if g['claimed_range'] else 'n'}/{'Y' if g['claimed_iqr'] else 'n'}/{'Y' if g['claimed_se'] else 'n'}; not claimed slower: {g['not_claimed_slower']}; claimed faster (SE): {g['claimed_faster_se']}")
    P('')
    P('## armed vs disarmed (J-As-a against J-As, same binary)')
    for k, c in arm_vs_dis.items():
        P(f"- {k}: {c['effect_pct']:+.2f} % ({c['delta_ms']:+.3f} ms), claimed r/i/s {'Y' if c['claimed_range'] else 'n'}/{'Y' if c['claimed_iqr'] else 'n'}/{'Y' if c['claimed_se'] else 'n'}")
    P('')
    P('## scaling')
    for k, s in scal.items():
        c16 = s.get('W16_vs_W8')
        P(f"- {k}: " + ', '.join(f'{kk} {vv:.3f}' for kk, vv in s.items() if kk != 'W16_vs_W8') +
          (f"; W16 vs W8 {c16['effect_pct']:+.2f} % ({c16['delta_ms']:+.3f} ms), claimed r/i/s {'Y' if c16['claimed_range'] else 'n'}/{'Y' if c16['claimed_iqr'] else 'n'}/{'Y' if c16['claimed_se'] else 'n'}" if c16 else ''))
    P('')
    P('## canary')
    for b, c in can.items():
        P(f"- {b}: canary_ns {c['canary_ns']}, span mean {c['span_mean_ms']:.4f} ms (rel err {c['span_rel_err_pct']:+.3f} %), wall {c['wall_ms']:.3f} vs J-As-a@W1 median {c['jasa_W1_median']:.3f} "
          f"[{c['jasa_W1_minmax'][0]:.3f}-{c['jasa_W1_minmax'][1]:.3f}] => rise {c['wall_rise_ms']:+.3f} ms (expected {c['expected_rise_ms']:.3f}); inside min-max: {c['inside_jasa_minmax']}; witness {c['others_busy_pct']}")
    P('')
    P('## receipts')
    P(json.dumps(rc, default=str))
    P('')
    P('## window 3 cells recomputed (this analyst, same selection rule)')
    for k in sorted(R['win3_cells']):
        c = R['win3_cells'][k]
        P(f"- {k}: K={c['n']} {c['median']:.3f} [{c['min']:.3f}-{c['max']:.3f}] r/i/s {c['range_pct']:.2f} / {c['iqr_pct']:.2f} / {c['se_pct']:.2f}")
    P('')
    P('## bridge (this window against window 3)')
    for k, c in bridge.items():
        P(f"- {k}: win3 {c['win3_median']:.3f} [{c['win3_minmax'][0]:.3f}-{c['win3_minmax'][1]:.3f}] -> this {c['this_median']:.3f} [{c['this_minmax'][0]:.3f}-{c['this_minmax'][1]:.3f}]: "
          f"{c['effect_pct']:+.2f} % ({c['delta_ms']:+.3f} ms), bars r/i/s {c['bar_range_pct']:.2f} / {c['bar_iqr_pct']:.2f} / {c['bar_se_pct']:.2f}, claimed "
          f"{'Y' if c['claimed_range'] else 'n'}/{'Y' if c['claimed_iqr'] else 'n'}/{'Y' if c['claimed_se'] else 'n'}; this median inside win3 min-max {c['this_inside_win3_minmax']}, win3 median inside this min-max {c['win3_inside_this_minmax']}")
    P('')
    P('## Jolt v5.6.0 reference (window 3 cells; boyko / Jolt)')
    for k, c in jolt.items():
        P(f"- {k}: boyko {c['boyko']:.3f} / Jolt {c['jolt56']:.3f} [{c['jolt_minmax'][0]:.3f}-{c['jolt_minmax'][1]:.3f}] = {c['ratio']:.3f} ({c['effect_pct']:+.1f} %), bars r/i/s {c['bar_range_pct']:.1f} / {c['bar_iqr_pct']:.1f} / {c['bar_se_pct']:.1f}, claimed "
          f"{'Y' if c['claimed_range'] else 'n'}/{'Y' if c['claimed_iqr'] else 'n'}/{'Y' if c['claimed_se'] else 'n'}")
    P(f"Jolt v5.6.0 manifolds: [100,500) {R['jolt56_manifolds']['sub_100_500']:.1f}, [0,500) {R['jolt56_manifolds']['full_0_500']:.3f}; sub-window medians " +
      ', '.join(f"W{w}: full {v['full_median']:.4f} sub {v['sub_median']:.4f} ({v['us_pm_sub']:.4f} us/m)" for w, v in sorted(R['jolt56_sub_window'].items())))
    for k, c in pm.items():
        P(f"- per manifold [100,500) {k}: boyko {c['boyko_sub_median_ms']:.3f} ms, {c['boyko_us_pm']:.3f} us/m ({c['boyko_manifolds_sub']:.1f}) vs Jolt v5.6.0 {c['jolt56_sub_median_ms']:.3f} ms, "
          f"{c['jolt56_us_pm']:.4f} us/m ({c['jolt56_manifolds_sub']:.1f}) = {c['ratio_us_pm']:.2f}x per manifold, {c['ratio_step_sub']:.3f}x per step over [100,500)")
    open(os.path.join(OUT_DIR, 'tables_analyst.md'), 'w', encoding='utf-8').write('\n'.join(L) + '\n')
    print('\n'.join(L))


if __name__ == '__main__':
    main()
