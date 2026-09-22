"""Reduce G5 (raw/runs.jsonl) to per-cell tables and the recipe's section 2.3 gate INPUTS. Pure reading; the
claims are the analyst's. Cell statistic, process selection, stats() and compare() are analyze_win3.py's
(window 3, imported): the MEDIAN over the K used processes of the process's [0,500) window mean (ms/step),
min-max / IQR / the median's SE as % of the median; a comparison is claimed iff |effect| > 2*hypot(spread_A,
spread_B), printed under the three readings.

Gate inputs (per armed process from run.csv, then the cell median over K):
  Delta-bp    = sys_physics_broadphase_ns (mean over [0,500) and median over [100,500)), allpairs - tree
  tree span   = phys_bp_verify + build + query + assemble per step, median over [100,500)
  t_q         = phys_bp_query_ns median over [100,500)
  structure   = void_steps, first_void, the set of (phys_bp_*_n) per step, queried + members per step
  canary      = sys_parity_canary_ns median over the steps vs canary_ns (= F*T); step rise = T-C-tree - T-A-tree
Reference cells (window 3, NOT re-run): D-L5 and JOLT56-T from
D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/analysis.json.

Usage: python -B reduce_g5.py [raw_dir]  -> writes <raw_dir>/g5_reduction.json and prints the tables.
"""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W4 = os.path.dirname(HERE)
RAW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(W4, 'raw')
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
import analyze_win3 as A  # noqa: E402
import driver as D  # noqa: E402

A.RAW = RAW
WIN3_ANALYSIS = 'D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/analysis.json'
WS = (1, 2, 4, 8, 16)
BP_SPANS = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def safe_stats(vals):
    """A.stats, or the raw values when the median is 0 (the brute path's spans are all 0)."""
    vals = [v for v in vals if v is not None]
    if not vals or statistics.median(vals) == 0:
        return {'n': len(vals), 'median': statistics.median(vals) if vals else None, 'values': vals, 'range_pct': None, 'se_med_pct': None}
    return A.stats(vals)


def ms(x):
    return None if x is None else x / 1e6


def mean_or_none(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def armed_fields(rec):
    c = D.load_csv(rec.get('csv'))
    if not c or 'wall_ns' not in c:
        return None
    out = {'n_steps': len(c['wall_ns'])}
    if 'sys_physics_broadphase_ns' in c:
        bp = c['sys_physics_broadphase_ns']
        out['bp_span_mean_0_500_ms'] = ms(mean_or_none(bp[0:500]))
        out['bp_span_med_100_500_ms'] = ms(med(bp[100:500]))
    if all(k in c for k in BP_SPANS):
        tot = [sum(c[k][i] for k in BP_SPANS) for i in range(len(c['wall_ns']))]
        out['tree_span_med_100_500_ms'] = ms(med(tot[100:500]))
        out['tree_span_mean_0_500_ms'] = ms(mean_or_none(tot[0:500]))
        out['t_q_med_100_500_ms'] = ms(med(c['phys_bp_query_ns'][100:500]))
        out['t_q_mean_100_500_ms'] = ms(mean_or_none(c['phys_bp_query_ns'][100:500]))
        out['span_n_sets'] = sorted({tuple(c[k.replace('_ns', '_n')][i] for k in BP_SPANS) for i in range(len(c['wall_ns']))})
    if 'phys_bp_queried' in c and 'phys_bp_members' in c:
        out['queried_plus_members_set'] = sorted({(c['phys_bp_queried'][i] or 0) + (c['phys_bp_members'][i] or 0) for i in range(len(c['wall_ns']))})
        out['members_set'] = sorted({c['phys_bp_members'][i] for i in range(len(c['wall_ns']))})
    if 'sys_parity_canary_ns' in c:
        cn = c['sys_parity_canary_ns']
        out['canary_span_med_ms'] = ms(med(cn[0:500]))
        out['canary_span_mean_ms'] = ms(mean_or_none(cn[0:500]))
    if 'void' in c:
        out['void_steps_csv'] = sum(1 for v in c['void'] if v)
    return out


def main():
    recs, warm, attempts, used, excluded = A.load()
    if os.environ.get('REDUCE_ALL') == '1':  # rehearsal only: the contaminated attempts too
        used = [r for r in attempts if r.get('valid')]
    cells, gate = {}, {}
    by_cell = {}
    for r in used:
        by_cell.setdefault((r['row'], r['W']), []).append(r)
    for (row, w), rs in sorted(by_cell.items(), key=lambda kv: (kv[0][1], kv[0][0])):
        s = A.stats([r['mean_ms'] for r in rs])
        s['processes'] = [{'pass': r['pass'], 'round': r['round'], 'seq': r['seq'], 'attempt': r['attempt'], 'mean_ms': r['mean_ms'],
                           'pose': r['summary']['pose_hash'], 'expect_pose': r['summary'].get('expect_pose'),
                           'rb': r['receipt_before']['cpu_avg'], 'ra': r['receipt_after']['cpu_avg'], 'others_pct': r.get('others_busy_pct'),
                           'void_steps': r['summary'].get('void_steps'), 'first_void': r['summary'].get('first_void'),
                           'canary_ns': r['summary'].get('canary_ns'), 'bp_tree': r['summary'].get('broadphase_tree')} for r in rs]
        subs = [A.sub_means(r) for r in rs]
        s['sub'] = {f'{a}_{b}': A.stats([x[f'{a}_{b}'] for x in subs if x and x.get(f'{a}_{b}') is not None]) for a, b in A.SUBWINDOWS}
        s['poses'] = sorted({r['summary']['pose_hash'] for r in rs})
        s['expect_pose_values'] = sorted({str(r['summary'].get('expect_pose')) for r in rs})
        s['void_steps_max'] = max(r['summary'].get('void_steps') or 0 for r in rs)
        if any(r.get('args') and '--arm-profiler' in r['args'] for r in rs):
            af = [armed_fields(r) for r in rs]
            s['armed'] = {'per_process': af}
            for k in ('bp_span_mean_0_500_ms', 'bp_span_med_100_500_ms', 'tree_span_med_100_500_ms', 'tree_span_mean_0_500_ms',
                      't_q_med_100_500_ms', 't_q_mean_100_500_ms', 'canary_span_med_ms', 'canary_span_mean_ms'):
                vals = [x[k] for x in af if x and x.get(k) is not None]
                if vals:
                    s['armed'][k] = safe_stats(vals)
            s['armed']['span_n_sets'] = sorted({str(x.get('span_n_sets')) for x in af if x})
            s['armed']['queried_plus_members_set'] = sorted({str(x.get('queried_plus_members_set')) for x in af if x})
            s['armed']['members_set'] = sorted({str(x.get('members_set')) for x in af if x})
        cells[f'{row}@W{w}'] = s

    def c(row, w):
        return cells.get(f'{row}@W{w}')

    def cmpx(a, b):
        r = A.compare(a, b)
        if r:
            r['a_median'], r['b_median'], r['a_n'], r['b_n'] = a['median'], b['median'], a['n'], b['n']
        return r

    cmp = {}
    for w in WS:
        cmp[f'T-A-tree vs T-A-allpairs @W{w}'] = cmpx(c('T-A-allpairs', w), c('T-A-tree', w))
    for w in (1, 8):
        cmp[f'T-D-tree vs T-D-allpairs @W{w}'] = cmpx(c('T-D-allpairs', w), c('T-D-tree', w))
        cmp[f'R-tree vs R-allpairs @W{w}'] = cmpx(c('R-allpairs', w), c('R-tree', w))
        cmp[f'T-A-tree-armed vs T-A-allpairs-armed @W{w}'] = cmpx(c('T-A-allpairs-armed', w), c('T-A-tree-armed', w))
        cmp[f'T-C-tree vs T-A-tree @W{w}'] = cmpx(c('T-A-tree', w), c('T-C-tree', w))
        cmp[f'T-A-tree-armed vs T-A-tree (arming cost) @W{w}'] = cmpx(c('T-A-tree', w), c('T-A-tree-armed', w))
    cmp['S16-tree vs S16-allpairs @W1'] = cmpx(c('S16-allpairs', 1), c('S16-tree', 1))
    # window-3 reference cells
    ref = {}
    if os.path.isfile(WIN3_ANALYSIS):
        w3 = json.load(open(WIN3_ANALYSIS, encoding='utf-8'))['cells']
        for k in ('D-L5@W1', 'D-L5@W8', 'JOLT56-T@W1', 'JOLT56-T@W2', 'JOLT56-T@W4', 'JOLT56-T@W8', 'JOLT56-T@W16'):
            if k in w3:
                ref[k] = {kk: w3[k][kk] for kk in ('n', 'median', 'min', 'max', 'range_pct', 'iqr_pct', 'se_med_pct')}
        for w in (1, 8):
            cmp[f'T-D-allpairs (this binary) vs window-3 D-L5 @W{w} (the bridge)'] = cmpx(ref.get(f'D-L5@W{w}'), c('T-D-allpairs', w))
            cmp[f'T-D-tree vs window-3 Jolt v5.6.0 @W{w}'] = cmpx(ref.get(f'JOLT56-T@W{w}'), c('T-D-tree', w))
        for w in WS:
            cmp[f'T-A-tree vs window-3 Jolt v5.6.0 @W{w}'] = cmpx(ref.get(f'JOLT56-T@W{w}'), c('T-A-tree', w))
    # gate inputs
    for w in (1, 8):
        ta, tt = c('T-A-allpairs-armed', w), c('T-A-tree-armed', w)
        if ta and tt and 'armed' in ta and 'armed' in tt:
            gate[f'Delta-bp @W{w}'] = {
                'allpairs_bp_span_mean_0_500_ms': ta['armed'].get('bp_span_mean_0_500_ms', {}).get('median'),
                'tree_bp_span_mean_0_500_ms': tt['armed'].get('bp_span_mean_0_500_ms', {}).get('median'),
                'allpairs_bp_span_med_100_500_ms': ta['armed'].get('bp_span_med_100_500_ms', {}).get('median'),
                'tree_bp_span_med_100_500_ms': tt['armed'].get('bp_span_med_100_500_ms', {}).get('median')}
            g = gate[f'Delta-bp @W{w}']
            if g['allpairs_bp_span_mean_0_500_ms'] is not None and g['tree_bp_span_mean_0_500_ms'] is not None:
                g['Delta_bp_mean_ms'] = g['allpairs_bp_span_mean_0_500_ms'] - g['tree_bp_span_mean_0_500_ms']
                if g['allpairs_bp_span_med_100_500_ms'] is not None and g['tree_bp_span_med_100_500_ms'] is not None:
                    g['Delta_bp_med_ms'] = g['allpairs_bp_span_med_100_500_ms'] - g['tree_bp_span_med_100_500_ms']
            gate[f'tree span @W{w}'] = {'median_over_K_of_per_process_median_100_500_ms': tt['armed'].get('tree_span_med_100_500_ms', {}).get('median'),
                                        'per_process': [x.get('tree_span_med_100_500_ms') for x in tt['armed']['per_process'] if x],
                                        'limit_ms': 0.36 if w == 1 else 0.35}
            gate[f't_q @W{w}'] = {'median_over_K_of_per_process_median_100_500_ms': tt['armed'].get('t_q_med_100_500_ms', {}).get('median'),
                                  'mean_100_500_ms': tt['armed'].get('t_q_mean_100_500_ms', {}).get('median')}
        for rid in ('T-A-tree-armed', 'T-A-allpairs-armed', 'T-C-tree'):
            x = c(rid, w)
            if x:
                gate[f'structure {rid} @W{w}'] = {'void_steps_max': x['void_steps_max'],
                                                  'first_void': sorted({str(p['first_void']) for p in x['processes']}),
                                                  'span_n_sets': x.get('armed', {}).get('span_n_sets'),
                                                  'queried_plus_members_set': x.get('armed', {}).get('queried_plus_members_set'),
                                                  'members_set': x.get('armed', {}).get('members_set')}
        tc, tr = c('T-C-tree', w), c('T-A-tree', w)
        if tc and tr:
            gate[f'canary @W{w}'] = {'canary_ns_per_process': [p['canary_ns'] for p in tc['processes']],
                                     'canary_span_med_ms_per_process': [x.get('canary_span_med_ms') for x in tc['armed']['per_process'] if x],
                                     'T-C-tree_median_ms': tc['median'], 'T-A-tree_median_ms': tr['median'],
                                     'step_rise_ms': tc['median'] - tr['median'],
                                     'canary_ns_median_ms': ms(med([p['canary_ns'] for p in tc['processes']]))}
    poses = {}
    for r in used:
        poses.setdefault(r['row'], set()).add(r['summary']['pose_hash'])
    gate['poses_per_row'] = {k: sorted(v) for k, v in poses.items()}
    gate['expect_pose_not_match_or_none'] = [(r['row'], r['W'], r['pass'], r['seq'], r['summary'].get('expect_pose')) for r in used
                                             if r['summary'].get('expect_pose') not in ('match', 'none')]
    gate['tree_diag_violations'] = []
    for r in used:
        bt = r['summary'].get('broadphase_tree') or {}
        is_tree = '--broadphase' in r['args'] and r['args'][r['args'].index('--broadphase') + 1] == 'tree'
        scene = r['args'][r['args'].index('--scene') + 1]
        if is_tree and scene in ('jolt', 'rest'):
            ok = (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0 and bt.get('translations') == 0
                  and bt.get('wide_rows') == 0 and bt.get('excluded_rows') == 0 and bt.get('sleeper_rebuilds') == 0 and bt.get('hint_candidates') == 0)
        else:
            ok = all(v == 0 for v in bt.values())
        if not ok:
            gate['tree_diag_violations'].append((r['row'], r['W'], r['pass'], r['seq'], bt))
    out = {'raw': RAW, 'records': len(recs), 'warmups': len(warm), 'attempts': len(attempts), 'used': len(used),
           'excluded_slots': [[{k: r.get(k) for k in ('row', 'W', 'pass', 'seq', 'attempt', 'contaminated', 'invalid')} for r in rs] for rs in excluded],
           'reruns': sum(1 for r in attempts if r.get('attempt') == 'rerun'),
           'contaminated_attempts': sum(1 for r in attempts if r.get('contaminated')),
           'invalid_attempts': [(r['row'], r['W'], r['pass'], r['seq'], r['attempt'], r['invalid']) for r in attempts if r.get('invalid')],
           'nonzero_exits': [(r['row'], r['W'], r['pass'], r['seq'], r['attempt'], r.get('exit')) for r in attempts if r.get('exit') not in (0, None)],
           'cells': cells, 'comparisons': cmp, 'window3_reference_cells': ref, 'gate_inputs': gate,
           'wall_clock': {'first_start': min(r['start'] for r in recs if r.get('start')), 'last_end': max(r['end'] for r in recs if r.get('end'))}}
    json.dump(out, open(os.path.join(RAW, 'g5_reduction.json'), 'w', encoding='utf-8'), indent=1, default=str)
    print(f"records {len(recs)} warm-ups {len(warm)} attempts {len(attempts)} used {len(used)} re-runs {out['reruns']} contaminated {out['contaminated_attempts']} excluded slots {len(excluded)}")
    print('| cell | K | median ms | min | max | range % | IQR % | SE % | [0,100) med | [100,500) med | poses | expect_pose | void max |')
    print('|---|---|---|---|---|---|---|---|---|---|---|---|---|')
    for k, s in cells.items():
        s0, s1 = s['sub'].get('0_100'), s['sub'].get('100_500')
        print(f"| {k} | {s['n']} | {s['median']:.4f} | {s['min']:.4f} | {s['max']:.4f} | {s['range_pct']:.2f} | {s['iqr_pct']:.2f} | {s['se_med_pct']:.2f} | "
              f"{s0['median'] if s0 else 'n/a'} | {s1['median'] if s1 else 'n/a'} | {' '.join(s['poses'])} | {' '.join(s['expect_pose_values'])} | {s['void_steps_max']} |")
    print()
    print('| comparison (B vs A) | A median | B median | ratio | effect % | bar range % | bar IQR % | bar SE % | claimed (range / IQR / SE) |')
    print('|---|---|---|---|---|---|---|---|---|')
    for k, v in cmp.items():
        if v is None:
            print(f'| {k} | n/a |')
            continue
        print(f"| {k} | {v['a_median']:.4f} (K={v['a_n']}) | {v['b_median']:.4f} (K={v['b_n']}) | {v['ratio']:.4f} | {v['effect_pct']:+.2f} | {v['bar_range_pct']:.2f} | {v['bar_iqr_pct']:.2f} | {v['bar_se_pct']:.2f} | {v['claim_range']} / {v['claim_iqr']} / {v['claim_se']} |")
    print()
    print('GATE INPUTS')
    print(json.dumps(gate, indent=1, default=str))


if __name__ == '__main__':
    main()
