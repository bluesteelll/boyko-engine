"""Window 4b reduction: per (row, W, binary) the MEDIAN over K of the process window mean, min-max, IQR and
the median's SE (1.2533*SD/sqrt(K)); tip-vs-parent effects under the SE reading (G9: claim iff
|effect| > 2*hypot(SE_A, SE_B)) and the min-max reading (intervals disjoint); the realized-gain gates
(>= 0.6 x the lower predicted delta); per-stage spans from the armed CSVs (per process the median over
steps [100, 500), then the median over K; us per manifold on the design's 4,524.2 denominator); the canary;
the load receipts. Writes <raw>/analysis.json and prints Markdown tables.

Slot policy (window 3's): a (block, pass, round, cell) slot contributes its re-run if one exists and is
valid and uncontaminated, else its original if valid and uncontaminated, else nothing (reported).
"""
import csv
import json
import math
import os
import statistics
import sys

RAW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'raw')
MANIFOLD_DENOM = 4524.2
GATES = {  # lower predicted delta (ms), gate = 0.6 x
    'solve_build': -0.29, 'warm_apply': -0.19, 'store': -0.11, 'T1_J-As': -0.81, 'T8_J-As': -0.61}
STAGES = ['phys_solve_build_ns', 'phys_gravity_ns', 'phys_warm_apply_ns', 'phys_integrate_ns', 'phys_pass_biased_ns',
          'phys_pass_relax_ns', 'phys_color_wide_ns', 'phys_color_narrow_ns', 'phys_restitution_ns', 'phys_store_ns',
          'phys_write_back_ns', 'phys_np_dispatch_ns', 'phys_np_compact_ns', 'phys_np_axis_commit_ns',
          'sys_physics_integrate_ns', 'sys_physics_gather_ns', 'sys_select_broadphase_ns', 'sys_physics_broadphase_ns',
          'sys_physics_narrowphase_ns', 'sys_physics_build_graph_ns', 'sys_physics_solve_colored_ns', 'sys_physics_apply_ns',
          'u_ns', 'g_ns', 'r_ns', 'sys_sum_ns', 'wall_ns']
COUNTERS = ['manifolds', 'pairs', 'colors', 'wide_colors', 'waves', 'phys_np_points', 'phys_slots_wide', 'phys_slots_narrow']


def load_csv(path):
    if not path or not os.path.isfile(path):
        return None
    rows = list(csv.reader(open(path, encoding='utf-8', newline='')))
    hdr = [h.strip() for h in rows[0]]
    cols = {h: [] for h in hdr}
    for r in rows[1:]:
        for h, v in zip(hdr, r):
            v = v.strip()
            try:
                cols[h].append(float(v) if v != '' else None)
            except ValueError:
                cols[h].append(None)
    return cols


def se_median(xs):
    if len(xs) < 2:
        return 0.0  # a single process: no spread is measurable; printed as 0 and reported as K=1
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs))


def cell_stats(xs):
    xs = sorted(xs)
    if not xs:
        return None
    q = statistics.quantiles(xs, n=4) if len(xs) >= 2 else [xs[0], xs[0], xs[0]]
    return {'K': len(xs), 'median': statistics.median(xs), 'mean': statistics.mean(xs), 'min': xs[0], 'max': xs[-1],
            'q1': q[0], 'q3': q[2], 'iqr': q[2] - q[0], 'sd': statistics.stdev(xs) if len(xs) >= 2 else 0.0,
            'se_median': se_median(xs), 'values': xs}


def main():
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
    timed = [r for r in recs if r.get('timed') and 'skipped' not in r and 'aborted' not in r]
    warm = [r for r in recs if not r.get('timed')]
    # slots
    slots = {}
    for r in timed:
        key = (r['block'], r['pass'], r['round'], r['row'], r['binary'], r['W'])
        slots.setdefault(key, {})[r['attempt']] = r
    chosen, dropped = {}, []
    for key, att in slots.items():
        pick = None
        for tag in ('rerun', 'original'):
            r = att.get(tag)
            if r and r['valid'] and not r['contaminated']:
                pick = r
                break
        if pick is None:
            dropped.append({'slot': key, 'attempts': {t: {'valid': a['valid'], 'invalid': a['invalid'], 'contaminated': a['contaminated'],
                                                       'cb': a.get('contaminated_before'), 'ca': a.get('contaminated_after'),
                                                       'cd': a.get('contaminated_during')} for t, a in att.items()}})
        else:
            chosen[key] = pick
    # cells
    cells = {}
    for key, r in chosen.items():
        cells.setdefault((r['row'], r['W'], r['binary']), []).append(r)
    A = {'n_records': len(recs), 'n_timed_processes': len(timed), 'n_warmups': len(warm),
         'n_reruns': sum(1 for r in timed if r['attempt'] == 'rerun'),
         'n_contaminated_before': sum(1 for r in timed if r.get('contaminated_before')),
         'n_contaminated_after': sum(1 for r in timed if r.get('contaminated_after')),
         'n_build_proc_during': sum(1 for r in timed if r.get('contaminated_during')),
         'n_invalid': sum(1 for r in timed if not r['valid']),
         'invalid_list': [{'row': r['row'], 'binary': r['binary'], 'W': r['W'], 'pass': r['pass'], 'seq': r['seq'], 'attempt': r['attempt'], 'why': r['invalid']}
                          for r in timed if not r['valid']],
         'dropped_slots': dropped, 'cells': {}, 'effects': {}, 'gates': {}, 'stages': {}, 'canary': {}, 'receipts': {}}
    for (row, w, b), rs in sorted(cells.items()):
        st = cell_stats([r['mean_ms'] for r in rs])
        st['poses'] = sorted({r['summary']['pose_hash'] for r in rs})
        st['expect_pose'] = sorted({r['summary'].get('expect_pose') for r in rs})
        st['others_busy_pct_max'] = max((r.get('others_busy_pct') or 0) for r in rs)
        st['others_busy_pct_median'] = statistics.median([(r.get('others_busy_pct') or 0) for r in rs])
        st['perf_pct_median'] = statistics.median([statistics.mean([p['total'] for p in r['perf'] if p.get('total') is not None]) for r in rs if r.get('perf') and any(p.get('total') is not None for p in r['perf'])] or [0])
        st['void_steps'] = sorted({r['summary'].get('void_steps') for r in rs})
        A['cells'][f'{row}@W{w}#{b}'] = st
    # effects tip - parent
    rows_ws = sorted({(row, w) for (row, w, b) in cells})
    for row, w in rows_ws:
        p, t = A['cells'].get(f'{row}@W{w}#parent'), A['cells'].get(f'{row}@W{w}#tip')
        if not p or not t:
            continue
        d = t['median'] - p['median']
        se = math.hypot(p['se_median'] or 0, t['se_median'] or 0)
        e = {'parent_median': p['median'], 'tip_median': t['median'], 'delta_ms': d, 'ratio': t['median'] / p['median'],
             'pct': 100 * (t['median'] / p['median'] - 1), 'se_pair': se, 'two_se': 2 * se,
             'claimed_se': abs(d) > 2 * se, 'direction': 'tip faster' if d < 0 else ('tip slower' if d > 0 else 'equal'),
             'minmax_disjoint': (t['max'] < p['min']) or (t['min'] > p['max']),
             'iqr_disjoint': (t['q3'] < p['q1']) or (t['q1'] > p['q3']),
             'K': [p['K'], t['K']]}
        e['claimed_minmax'] = e['minmax_disjoint']
        A['effects'][f'{row}@W{w}'] = e
    # end-to-end gates
    g = A['gates']
    for name, key in (('T1_J-As', 'J-As@W1'), ('T8_J-As', 'J-As@W8')):
        e = A['effects'].get(key)
        if e:
            thr = 0.6 * GATES[name]
            g[name] = {'delta_ms': e['delta_ms'], 'threshold_ms': thr, 'pass': e['delta_ms'] <= thr and e['claimed_se'],
                       'claimed_se': e['claimed_se'], 'two_se': e['two_se']}
    for w in (1, 2, 4, 8, 16):
        e = A['effects'].get(f'J-A@W{w}')
        if e:
            g[f'cfgA_not_slower_W{w}'] = {'delta_ms': e['delta_ms'], 'two_se': e['two_se'],
                                          'slower_claimed': e['delta_ms'] > 0 and e['claimed_se'],
                                          'pass': not (e['delta_ms'] > 0 and e['claimed_se'])}
    for row in ('R', 'R-S', 'S16', 'J-As'):
        for w in (1, 2, 4, 8, 16):
            e = A['effects'].get(f'{row}@W{w}')
            if e and f'cfgA_not_slower_W{w}' not in (f'{row}',):
                g[f'{row}_W{w}_not_slower'] = {'delta_ms': e['delta_ms'], 'two_se': e['two_se'],
                                               'slower_claimed': e['delta_ms'] > 0 and e['claimed_se'],
                                               'pass': not (e['delta_ms'] > 0 and e['claimed_se'])}

    # per-stage spans from the armed CSVs
    def stage_medians(r, lo, hi):
        c = load_csv(r.get('csv'))
        out = {}
        if not c:
            return out
        for s in STAGES + COUNTERS:
            if s in c:
                v = [x for x in c[s][lo:hi] if x is not None]
                out[s] = statistics.median(v) if v else None
        return out

    stage_cells = {}
    for (row, w, b), rs in cells.items():
        if not rs[0]['summary'].get('armed'):
            continue
        lo, hi = (100, 500) if row in ('J-As-a', 'J-C') and not rs[0].get('dry') else tuple(rs[0]['summary']['window'])
        per_proc = [stage_medians(r, lo, hi) for r in rs]
        agg = {}
        for s in STAGES + COUNTERS:
            vals = [p[s] for p in per_proc if p.get(s) is not None]
            if vals:
                agg[s] = {'median': statistics.median(vals), 'min': min(vals), 'max': max(vals), 'se_median': se_median(vals), 'K': len(vals)}
        stage_cells[(row, w, b)] = {'steps': [lo, hi], 'stages': agg}
        A['stages'][f'{row}@W{w}#{b}'] = stage_cells[(row, w, b)]
    # stage effects (tip - parent) for the armed J-As-a rows and R / R-S
    A['stage_effects'] = {}
    for (row, w, b) in sorted(stage_cells):
        if b != 'parent':
            continue
        p = stage_cells[(row, w, 'parent')]['stages']
        t = stage_cells.get((row, w, 'tip'), {}).get('stages')
        if not t:
            continue
        eff = {}
        for s in STAGES:
            if s in p and s in t:
                d_ns = t[s]['median'] - p[s]['median']
                se = math.hypot(p[s]['se_median'] or 0, t[s]['se_median'] or 0)
                eff[s] = {'parent_ms': p[s]['median'] / 1e6, 'tip_ms': t[s]['median'] / 1e6, 'delta_ms': d_ns / 1e6,
                          'parent_us_per_manifold': p[s]['median'] / 1e3 / MANIFOLD_DENOM if row.startswith('J-') else None,
                          'tip_us_per_manifold': t[s]['median'] / 1e3 / MANIFOLD_DENOM if row.startswith('J-') else None,
                          'two_se_ms': 2 * se / 1e6, 'claimed_se': abs(d_ns) > 2 * se,
                          'pct': 100 * (t[s]['median'] / p[s]['median'] - 1) if p[s]['median'] else float('nan')}
        A['stage_effects'][f'{row}@W{w}'] = eff
        if row == 'J-As-a':
            for gname, s in (('solve_build', 'phys_solve_build_ns'), ('warm_apply', 'phys_warm_apply_ns'), ('store', 'phys_store_ns')):
                if s in eff:
                    thr = 0.6 * GATES[gname]
                    g[f'{gname}_W{w}'] = {'delta_ms': eff[s]['delta_ms'], 'threshold_ms': thr, 'pass': eff[s]['delta_ms'] <= thr and eff[s]['claimed_se'],
                                          'claimed_se': eff[s]['claimed_se'], 'two_se_ms': eff[s]['two_se_ms'],
                                          'note': 'C3 lever, NOT measurable in this window (C2 tip); reported, not gated' if gname == 'warm_apply' else None}
            if 'phys_color_wide_ns' in eff:
                e = eff['phys_color_wide_ns']
                g[f'wide_colours_not_slower_W{w}'] = {'delta_ms': e['delta_ms'], 'two_se_ms': e['two_se_ms'], 'pct': e['pct'],
                                                      'slower_claimed': e['delta_ms'] > 0 and e['claimed_se'],
                                                      'faster_claimed': e['delta_ms'] < 0 and e['claimed_se'],
                                                      'pass': not (e['delta_ms'] > 0 and e['claimed_se'])}
    # canary
    for b in ('parent', 'tip'):
        rs = cells.get(('J-C', 1, b), [])
        ref = cells.get(('J-As-a', 1, b), [])
        for r in rs:
            cn = r['summary'].get('canary_ns')
            c = load_csv(r['csv'])
            lo, hi = (100, 500) if not r.get('dry') else tuple(r['summary']['window'])
            sv = [x for x in c['sys_parity_canary_ns'][lo:hi] if x is not None] if c and 'sys_parity_canary_ns' in c else []
            span = statistics.median(sv) if sv else None
            ref_med = statistics.median([x['mean_ms'] for x in ref]) if ref else None
            A['canary'][b] = {'canary_ns': cn, 'canary_ref_ns': r.get('canary_ref', {}).get('window_mean_ns') if r.get('canary_ref') else None,
                              'span_median_ns_100_500': span, 'span_rel_err': (span / cn - 1) if span and cn else None,
                              'span_ok_5pct': (abs(span / cn - 1) <= 0.05) if span and cn else None,
                              'T_JC_ms': r['mean_ms'], 'T_JAsa_median_ms': ref_med,
                              'rise_ms': (r['mean_ms'] - ref_med) if ref_med else None,
                              'rise_minus_canary_rel_of_T': ((r['mean_ms'] - ref_med - cn / 1e6) / ref_med) if ref_med and cn else None,
                              'seen': bool(span and cn and abs(span / cn - 1) <= 0.05 and ref_med and (r['mean_ms'] - ref_med) > 0.5 * cn / 1e6)}
    # receipts
    cpu_b = [r['receipt_before']['cpu_avg'] for r in timed if r.get('receipt_before') and r['receipt_before']['cpu_avg'] is not None]
    cpu_a = [r['receipt_after']['cpu_avg'] for r in timed if r.get('receipt_after') and r['receipt_after']['cpu_avg'] is not None]
    ob = [r.get('others_busy_pct') or 0 for r in timed]
    A['receipts'] = {'n': len(timed), 'before_cpu_avg_median': statistics.median(cpu_b) if cpu_b else None, 'before_cpu_avg_max': max(cpu_b) if cpu_b else None,
                     'after_cpu_avg_median': statistics.median(cpu_a) if cpu_a else None, 'after_cpu_avg_max': max(cpu_a) if cpu_a else None,
                     'n_over_5pct_before': sum(1 for x in cpu_b if x > 5), 'n_over_5pct_after': sum(1 for x in cpu_a if x > 5),
                     'others_busy_pct_median': statistics.median(ob) if ob else None, 'others_busy_pct_max': max(ob) if ob else None,
                     'others_busy_over_5pct': sum(1 for x in ob if x > 5),
                     'build_procs_seen_in_receipts': sorted({n for r in timed for k in ('receipt_before', 'receipt_after') for n in (r.get(k) or {}).get('build_procs_busy', [])}),
                     'build_procs_seen_during': sorted({n for r in timed for n in r.get('build_proc_during', [])}),
                     'waited_s_total': sum(r.get('waited_s') or 0 for r in timed)}
    with open(os.path.join(RAW, 'analysis.json'), 'w', encoding='utf-8') as f:
        json.dump(A, f, indent=1, default=str)

    # ---- render
    P = print
    P(f'timed processes {A["n_timed_processes"]} (re-runs {A["n_reruns"]}), warm-ups {A["n_warmups"]}, invalid {A["n_invalid"]}, '
      f'contaminated before {A["n_contaminated_before"]} / after {A["n_contaminated_after"]} / build-proc-during {A["n_build_proc_during"]}, dropped slots {len(dropped)}')
    P()
    P('### Cells (ms/step; MEDIAN over K of the process window mean; min-max; IQR; SE of the median)')
    P()
    P('| row | W | binary | K | median | mean | min | max | IQR | SE_med | poses | others busy % (med/max) | perf % (med) |')
    P('|---|---|---|---|---|---|---|---|---|---|---|---|---|')
    for k, s in A['cells'].items():
        row, rest = k.split('@W')
        w, b = rest.split('#')
        P(f'| {row} | {w} | {b} | {s["K"]} | {s["median"]:.4f} | {s["mean"]:.4f} | {s["min"]:.4f} | {s["max"]:.4f} | {s["iqr"]:.4f} | {s["se_median"]:.4f} | {",".join(s["poses"])} | {s["others_busy_pct_median"]:.2f}/{s["others_busy_pct_max"]:.2f} | {s["perf_pct_median"]:.1f} |')
    P()
    P('### Effects, tip - parent (ms/step). SE reading: claimed iff |delta| > 2*hypot(SE_p, SE_t). Min-max reading: intervals disjoint.')
    P()
    P('| row | W | parent | tip | delta ms | % | 2*SE | claimed (SE) | min-max disjoint | IQR disjoint | direction |')
    P('|---|---|---|---|---|---|---|---|---|---|---|')
    for k, e in A['effects'].items():
        row, w = k.split('@W')
        P(f'| {row} | {w} | {e["parent_median"]:.4f} | {e["tip_median"]:.4f} | {e["delta_ms"]:+.4f} | {e["pct"]:+.2f} | {e["two_se"]:.4f} | {"YES" if e["claimed_se"] else "no"} | {"yes" if e["minmax_disjoint"] else "no"} | {"yes" if e["iqr_disjoint"] else "no"} | {e["direction"]} |')
    P()
    P('### Per-stage spans (armed rows; per process the median over the steps named, then the median over K; ms and us per manifold on 4,524.2)')
    for k, se_ in A['stage_effects'].items():
        row, w = k.split('@W')
        st = A['stages'][f'{k}#parent']['steps']
        P()
        P(f'#### {row} W={w} (steps [{st[0]}, {st[1]}))')
        P()
        P('| stage | parent ms | tip ms | delta ms | % | 2*SE ms | claimed (SE) | parent us/manifold | tip us/manifold |')
        P('|---|---|---|---|---|---|---|---|---|')
        for s, e in se_.items():
            pu = f'{e["parent_us_per_manifold"]:.4f}' if e['parent_us_per_manifold'] is not None else '-'
            tu = f'{e["tip_us_per_manifold"]:.4f}' if e['tip_us_per_manifold'] is not None else '-'
            P(f'| {s} | {e["parent_ms"]:.4f} | {e["tip_ms"]:.4f} | {e["delta_ms"]:+.4f} | {e["pct"]:+.2f} | {e["two_se_ms"]:.4f} | {"YES" if e["claimed_se"] else "no"} | {pu} | {tu} |')
        cp = A['stages'][f'{k}#parent']['stages']
        ct = A['stages'][f'{k}#tip']['stages']
        P()
        P('counters (median): ' + '; '.join(f'{c} parent {cp[c]["median"]:.1f} / tip {ct[c]["median"]:.1f}' for c in COUNTERS if c in cp and c in ct))
    P()
    P('### Gates (G9)')
    P()
    P('| gate | delta ms | threshold ms (0.6 x lower predicted) / 2*SE | verdict |')
    P('|---|---|---|---|')
    for k, v in A['gates'].items():
        thr = f'{v["threshold_ms"]:+.4f}' if 'threshold_ms' in v else '-'
        two = v.get('two_se', v.get('two_se_ms'))
        note = f' ({v["note"]})' if v.get('note') else ''
        if 'threshold_ms' in v:
            verdict = ('PASS' if v['pass'] else 'FAIL') + note
        else:
            verdict = ('PASS (not claimed slower)' if v['pass'] else 'FAIL (claimed slower)') + (' [faster claimed]' if v.get('faster_claimed') else '')
        P(f'| {k} | {v["delta_ms"]:+.4f} | {thr} / {two:.4f} | {verdict} |')
    P()
    P('### Canary')
    P()
    for b, c in A['canary'].items():
        P(f'- {b}: canary_ns {c["canary_ns"]} (ref {c["canary_ref_ns"]}); span median [100,500) {c["span_median_ns_100_500"]} ns, rel err {c["span_rel_err"]:+.4f} (ok <= 5 %: {c["span_ok_5pct"]}); '
          f'T(J-C) {c["T_JC_ms"]:.4f} ms vs J-As-a median {c["T_JAsa_median_ms"]:.4f} ms: rise {c["rise_ms"]:+.4f} ms (canary {c["canary_ns"]/1e6:.4f} ms; (rise - canary)/T = {c["rise_minus_canary_rel_of_T"]:+.4f}); seen: {c["seen"]}')
    P()
    P('### Load receipts')
    P()
    r = A['receipts']
    P(f'- {r["n"]} timed processes; 5-s receipt before: median {r["before_cpu_avg_median"]:.2f} %, max {r["before_cpu_avg_max"]:.2f} %, > 5 %: {r["n_over_5pct_before"]}; '
      f'after: median {r["after_cpu_avg_median"]:.2f} %, max {r["after_cpu_avg_max"]:.2f} %, > 5 %: {r["n_over_5pct_after"]}')
    P(f'- during-process witness (others_busy_pct): median {r["others_busy_pct_median"]:.2f} %, max {r["others_busy_pct_max"]:.2f} %, > 5 %: {r["others_busy_over_5pct"]}')
    P(f'- build processes seen in receipts: {r["build_procs_seen_in_receipts"] or "none"}; during a process: {r["build_procs_seen_during"] or "none"}; total wait for quiet {r["waited_s_total"]:.0f} s')
    if A['invalid_list']:
        P(f'- INVALID processes: {A["invalid_list"]}')
    if dropped:
        P(f'- dropped slots (both attempts contaminated/invalid): {dropped}')


if __name__ == '__main__':
    main()
