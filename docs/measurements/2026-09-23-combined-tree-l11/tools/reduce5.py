"""Window 5 reduction (own script; the arithmetic of window 4b's reduce.py, the gates removed).

Per (row, W, binary) cell: the MEDIAN over K of the process window mean ([0, 500) wall_ns mean), min, max,
range = max - min, SE of the median = 1.2533 * SD / sqrt(K), and both as fractions of the median.
Comparisons print only arithmetic: ratio = B / A, |ratio - 1|, and the two thresholds of the task's rule
2 * sqrt(sA^2 + sB^2) with s = SE/median (SE reading) and s = (max - min)/median (min-max reading).
No verdict column: this script makes no claim.
Armed rows: per process the median over steps [100, 500) of every *_ns zone column, then the median over K
(min, max, SE beside it).

Slot policy (window 3's / 4b's): records of a VOIDED pass attempt are excluded; then a (block, pass, round,
row, binary, W) slot contributes its re-run if valid and uncontaminated, else its original if valid and
uncontaminated, else nothing (listed as dropped).
Usage: reduce5.py [raw_dir]  -> writes <raw>/analysis.json and <raw>/tables.md, prints the tables."""
import csv
import json
import math
import os
import statistics
import sys

ARGS = [a for a in sys.argv[1:] if not a.startswith('--')]
WITH_MAKEUP = '--with-makeup' in sys.argv
SUFFIX = '_with_makeup' if WITH_MAKEUP else ''
RAW = ARGS[0] if ARGS else os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'raw')
JOLT56 = {1: 9.828, 2: 5.770, 4: 3.581, 8: 2.569, 16: 2.388}  # window 3's Jolt v5.6.0 cells (ms/step), never re-run
WIN4B_C2_WARM_APPLY_MS = 0.519  # window 4b, J-As-a W=1 tip (C2) phys_warm_apply_ns median over [100,500)
WARM_APPLY_BAR_MS = -0.19       # the design's realized-gain bar for warm_apply at W=1 (already the 0.6x bar)
STAGES = ['phys_solve_build_ns', 'phys_gravity_ns', 'phys_warm_apply_ns', 'phys_integrate_ns', 'phys_pass_biased_ns',
          'phys_pass_relax_ns', 'phys_color_wide_ns', 'phys_color_narrow_ns', 'phys_restitution_ns', 'phys_store_ns',
          'phys_write_back_ns', 'phys_np_dispatch_ns', 'phys_np_compact_ns', 'phys_np_axis_commit_ns',
          'sys_physics_integrate_ns', 'sys_physics_gather_ns', 'sys_select_broadphase_ns', 'sys_physics_broadphase_ns',
          'sys_physics_narrowphase_ns', 'sys_physics_build_graph_ns', 'sys_physics_solve_colored_ns', 'sys_physics_apply_ns',
          'u_ns', 'g_ns', 'r_ns', 'sys_sum_ns', 'wall_ns']
COUNTERS = ['manifolds', 'pairs', 'colors', 'wide_colors', 'waves', 'phys_np_points', 'phys_slots_wide', 'phys_slots_narrow']
OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


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
        return 0.0
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs))


def cell_stats(xs):
    xs = sorted(xs)
    med = statistics.median(xs)
    se = se_median(xs)
    return {'K': len(xs), 'median': med, 'mean': statistics.mean(xs), 'min': xs[0], 'max': xs[-1],
            'range': xs[-1] - xs[0], 'sd': statistics.stdev(xs) if len(xs) >= 2 else 0.0, 'se_median': se,
            'rel_se': se / med if med else None, 'rel_range': (xs[-1] - xs[0]) / med if med else None, 'values': xs}


def compare(a, b):
    """ratio = b / a, and the two thresholds of 2*sqrt(sA^2 + sB^2)."""
    r = b['median'] / a['median']
    return {'a_median': a['median'], 'b_median': b['median'], 'delta_ms': b['median'] - a['median'], 'ratio': r,
            'abs_ratio_minus_1': abs(r - 1),
            'thr_se': 2 * math.hypot(a['rel_se'], b['rel_se']),
            'thr_minmax': 2 * math.hypot(a['rel_range'], b['rel_range']),
            'K': [a['K'], b['K']]}


def perf_med(r):
    v = [p['total'] for p in r.get('perf', []) if p.get('total') is not None]
    return statistics.mean(v) if v else None


def main():
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
    voided = {(r['block'], r['pass'], r['pass_attempt']) for r in recs if r.get('voided_pass')}
    proc = [r for r in recs if not r.get('voided_pass') and 'aborted' not in r and 'mean_ms' in r]
    voided_procs = [r for r in proc if (r['block'], r['pass'], r['pass_attempt']) in voided]
    live = [r for r in proc if (r['block'], r['pass'], r['pass_attempt']) not in voided]
    timed = [r for r in live if r.get('timed')]
    warm = [r for r in live if not r.get('timed')]
    slots = {}
    for r in timed:
        slots.setdefault((r['block'], r['pass'], r['round'], r['row'], r['binary'], r['W']), {})[r['attempt']] = r
    chosen, dropped = {}, []
    for key, att in slots.items():
        pick = None
        for tag in ('rerun', 'original'):
            r = att.get(tag)
            if r and r['valid'] and not r['contaminated']:
                pick = r
                break
        if pick is None:
            dropped.append({'slot': key, 'attempts': {t: {'valid': a['valid'], 'invalid': a['invalid'], 'contaminated': a['contaminated']}
                                                       for t, a in att.items()}})
        else:
            chosen[key] = pick
            pick['_chosen'] = True
    makeup_fills = []
    if WITH_MAKEUP:
        mk = [json.loads(l) for l in open(os.path.join(RAW, 'makeup', 'runs_makeup.jsonl'), encoding='utf-8')]
        mvoid = {(r['block'], r['pass'], r['pass_attempt']) for r in mk if r.get('voided_pass')}
        mk = [r for r in mk if not r.get('voided_pass') and r.get('timed') and 'mean_ms' in r
              and (r['block'], r['pass'], r['pass_attempt']) not in mvoid]
        for d in dropped:
            key = d['slot']
            cand = [r for r in mk if (r['row'], r['binary'], r['W']) == key[3:] and not r.get('_used')]
            fill = cand[0] if cand else None
            ok = bool(fill and fill['valid'] and not fill['contaminated'])
            makeup_fills.append({'slot': key, 'makeup_found': bool(fill), 'makeup_valid_clean': ok,
                                 'makeup_mean_ms': fill['mean_ms'] if fill else None,
                                 'rb': (fill or {}).get('receipt_before', {}).get('cpu_avg'),
                                 'ra': ((fill or {}).get('receipt_after') or {}).get('cpu_avg')})
            if fill:
                fill['_used'] = True
            if ok:
                chosen[key] = fill
                fill['_chosen'] = True
        dropped = [d for d, f in zip(dropped, makeup_fills) if not f['makeup_valid_clean']]
    cells = {}
    for key, r in chosen.items():
        cells.setdefault((r['row'], r['W'], r['binary']), []).append(r)
    dry = bool(timed and timed[0].get('dry'))
    A = {'raw': RAW, 'dry': dry, 'n_records': len(recs), 'n_timed_processes': len(timed), 'n_warmups': len(warm),
         'n_reruns': sum(1 for r in timed if r['attempt'] == 'rerun'), 'voided_pass_attempts': sorted(voided),
         'n_voided_processes': len(voided_procs),
         'n_contaminated_before': sum(1 for r in timed if r.get('contaminated_before')),
         'n_contaminated_after': sum(1 for r in timed if r.get('contaminated_after')),
         'n_build_proc_during': sum(1 for r in timed if r.get('contaminated_during')),
         'n_invalid': sum(1 for r in timed if not r['valid']),
         'invalid_list': [{'row': r['row'], 'binary': r['binary'], 'W': r['W'], 'block': r['block'], 'pass': r['pass'],
                           'seq': r['seq'], 'attempt': r['attempt'], 'why': r['invalid']} for r in timed if not r['valid']],
         'dropped_slots': dropped, 'makeup_fills': makeup_fills, 'with_makeup': WITH_MAKEUP, 'cells': {}, 'headline': {}, 'g9': {}, 'stages': {}, 'stage_effects': {}, 'receipts': {}}
    order = {'HL-D-tree': 0, 'HL-D-allpairs': 1, 'HL-R-tree': 2, 'HL-R-allpairs': 3, 'J-As': 4, 'J-As-a': 5}
    for (row, w, b), rs in sorted(cells.items(), key=lambda kv: (order.get(kv[0][0], 9), kv[0][1], kv[0][2])):
        st = cell_stats([r['mean_ms'] for r in rs])
        st['poses'] = sorted({r['summary']['pose_hash'] for r in rs})
        st['expect_pose'] = sorted({r['summary'].get('expect_pose') for r in rs})
        st['tree_diag'] = sorted({json.dumps(r['summary'].get('broadphase_tree'), sort_keys=True, separators=(',', ':')) for r in rs})
        st['broadphase'] = sorted({str((r['summary'].get('config') or {}).get('broadphase')) for r in rs})
        st['others_busy_pct_median'] = statistics.median([(r.get('others_busy_pct') or 0) for r in rs])
        st['others_busy_pct_max'] = max((r.get('others_busy_pct') or 0) for r in rs)
        pm = [perf_med(r) for r in rs if perf_med(r) is not None]
        st['perf_pct_median'] = statistics.median(pm) if pm else None
        st['masks'] = sorted({r.get('mask_readback') for r in rs})
        st['void_steps'] = sorted({r['summary'].get('void_steps') for r in rs})
        st['passes'] = sorted({r['pass'] for r in rs})
        A['cells'][f'{row}@W{w}#{b}'] = st
    C = A['cells']
    # headline: tree vs allpairs (tip), and each against Jolt v5.6.0's window-3 cells
    for scene, pre in (('J', 'HL-D'), ('R', 'HL-R')):
        for w in (1, 2, 4, 8):
            t, a = C.get(f'{pre}-tree@W{w}#tip'), C.get(f'{pre}-allpairs@W{w}#tip')
            if not t or not a:
                continue
            e = compare(a, t)  # ratio = tree / allpairs
            if scene == 'J':
                e['jolt56_ms'] = JOLT56[w]
                e['tree_over_jolt'] = t['median'] / JOLT56[w]
                e['allpairs_over_jolt'] = a['median'] / JOLT56[w]
            A['headline'][f'{scene}@W{w}'] = e
    # G9: tip vs parent
    for row in ('J-As', 'J-As-a'):
        for w in (1, 8):
            p, t = C.get(f'{row}@W{w}#parent'), C.get(f'{row}@W{w}#tip')
            if p and t:
                A['g9'][f'{row}@W{w}'] = compare(p, t)  # ratio = tip / parent

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

    for (row, w, b), rs in cells.items():
        if not rs[0]['summary'].get('armed'):
            continue
        lo, hi = (100, 500) if not rs[0].get('dry') else tuple(rs[0]['summary']['window'])
        per_proc = [stage_medians(r, lo, hi) for r in rs]
        agg = {}
        for s in STAGES + COUNTERS:
            vals = [p_[s] for p_ in per_proc if p_.get(s) is not None]
            if vals:
                agg[s] = {'median': statistics.median(vals), 'min': min(vals), 'max': max(vals), 'se_median': se_median(vals), 'K': len(vals)}
        A['stages'][f'{row}@W{w}#{b}'] = {'steps': [lo, hi], 'stages': agg,
                                          'waves_total': sorted({r['summary'].get('waves_total') for r in rs}),
                                          'drops_total': sorted({r['summary'].get('drops_total') for r in rs}),
                                          'solve_on_dispatcher_steps': sorted({(r['summary'].get('threads') or {}).get('solve_on_dispatcher_steps') for r in rs})}
    for w in (1, 8):
        p = A['stages'].get(f'J-As-a@W{w}#parent', {}).get('stages')
        t = A['stages'].get(f'J-As-a@W{w}#tip', {}).get('stages')
        if not p or not t:
            continue
        eff = {}
        for s in STAGES:
            if s in p and s in t:
                pm_, tm_ = p[s]['median'], t[s]['median']
                eff[s] = {'parent_ms': pm_ / 1e6, 'tip_ms': tm_ / 1e6, 'delta_ms': (tm_ - pm_) / 1e6,
                          'ratio': tm_ / pm_ if pm_ else None,
                          'two_se_ms': 2 * math.hypot(p[s]['se_median'], t[s]['se_median']) / 1e6,
                          'parent_min_max_ms': [p[s]['min'] / 1e6, p[s]['max'] / 1e6],
                          'tip_min_max_ms': [t[s]['min'] / 1e6, t[s]['max'] / 1e6],
                          'parent_se_ms': p[s]['se_median'] / 1e6, 'tip_se_ms': t[s]['se_median'] / 1e6}
        A['stage_effects'][f'J-As-a@W{w}'] = eff
    # receipts
    cpu_b = [r['receipt_before']['cpu_avg'] for r in timed if r.get('receipt_before') and r['receipt_before'].get('cpu_avg') is not None]
    cpu_a = [r['receipt_after']['cpu_avg'] for r in timed if r.get('receipt_after') and r['receipt_after'].get('cpu_avg') is not None]
    ob = [r.get('others_busy_pct') or 0 for r in timed]
    ch = [r for r in timed if r.get('_chosen')]
    A['receipts'] = {'n': len(timed), 'before_median': statistics.median(cpu_b) if cpu_b else None, 'before_max': max(cpu_b) if cpu_b else None,
                     'after_median': statistics.median(cpu_a) if cpu_a else None, 'after_max': max(cpu_a) if cpu_a else None,
                     'n_over_5_before': sum(1 for x in cpu_b if x > 5), 'n_over_5_after': sum(1 for x in cpu_a if x > 5),
                     'others_median': statistics.median(ob) if ob else None, 'others_max': max(ob) if ob else None,
                     'others_over_5': sum(1 for x in ob if x > 5),
                     'others_over_5_in_chosen': sum(1 for r in ch if (r.get('others_busy_pct') or 0) > 5),
                     'build_busy_in_receipts': sorted({n for r in timed for k in ('receipt_before', 'receipt_after') for n in (r.get(k) or {}).get('build_procs_busy', [])}),
                     'presence_in_receipts': sorted({x for r in timed for k in ('receipt_before', 'receipt_after') for kk in ('build', 'lane') for x in ((r.get(k) or {}).get('presence') or {}).get(kk, [])}),
                     'build_during': sorted({n for r in timed for n in r.get('build_proc_during', [])}),
                     'waited_s_total': sum(r.get('waited_s') or 0 for r in timed),
                     'masks': sorted({r.get('mask_readback') for r in timed})}
    with open(os.path.join(RAW, f'analysis{SUFFIX}.json'), 'w', encoding='utf-8') as f:
        json.dump(A, f, indent=1, default=str)

    # ---- render
    if WITH_MAKEUP:
        P(f'MAKE-UP VARIANT: dropped protocol slots filled from raw/makeup/runs_makeup.jsonl where the make-up process is valid and clean: {makeup_fills}')
    P(f'timed processes {A["n_timed_processes"]} (re-runs {A["n_reruns"]}), warm-ups {A["n_warmups"]}, voided pass attempts '
      f'{len(voided)} ({A["n_voided_processes"]} processes discarded), invalid {A["n_invalid"]}, contaminated before '
      f'{A["n_contaminated_before"]} / after {A["n_contaminated_after"]} / build-proc-during {A["n_build_proc_during"]}, dropped slots {len(dropped)}')
    P()
    P('### Cells (ms/step; MEDIAN over K of the process [0,500) window mean; min-max; SE of the median = 1.2533*SD/sqrt(K))')
    P()
    P('| row | W | binary | K | median | min | max | range | SE_med | range/med | SE/med | pose | TreeDiag (static_rebuilds/members/evictions; all other fields) | others busy % med/max | perf % med |')
    P('|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|')
    for k, s in C.items():
        row, rest = k.split('@W')
        w, b = rest.split('#')
        td = []
        for j in s['tree_diag']:
            d = json.loads(j) if j != 'null' else {}
            others = sum(v for kk, v in d.items() if kk not in ('static_rebuilds', 'members', 'evictions'))
            td.append(f'{d.get("static_rebuilds")}/{d.get("members")}/{d.get("evictions")}; others sum {others}')
        pp = f'{s["perf_pct_median"]:.1f}' if s['perf_pct_median'] is not None else '-'
        P(f'| {row} | {w} | {b} | {s["K"]} | {s["median"]:.4f} | {s["min"]:.4f} | {s["max"]:.4f} | {s["range"]:.4f} | {s["se_median"]:.4f} | '
          f'{100 * s["rel_range"]:.2f} % | {100 * s["rel_se"]:.2f} % | {",".join(s["poses"])} | {" | ".join(td)} | '
          f'{s["others_busy_pct_median"]:.2f}/{s["others_busy_pct_max"]:.2f} | {pp} |')
    P()
    P('### (1) Headline, tip only - tree vs allpairs (ratio = tree / allpairs), and each against Jolt v5.6.0 (window 3 cells, not re-run)')
    P()
    P('Thresholds are the rule\'s right-hand side 2*sqrt(sA^2 + sB^2), s as a fraction of the median: SE reading and min-max (range) reading. Arithmetic only.')
    P()
    P('| scene | W | allpairs median | tree median | tree - allpairs ms | ratio tree/allpairs | abs(ratio-1) | thr SE | thr min-max | Jolt v5.6.0 ms | tree / Jolt | allpairs / Jolt |')
    P('|---|---|---|---|---|---|---|---|---|---|---|---|')
    for k, e in A['headline'].items():
        sc, w = k.split('@W')
        j = f'{e["jolt56_ms"]:.3f}' if 'jolt56_ms' in e else '-'
        tj = f'{e["tree_over_jolt"]:.4f}' if 'tree_over_jolt' in e else '-'
        aj = f'{e["allpairs_over_jolt"]:.4f}' if 'allpairs_over_jolt' in e else '-'
        P(f'| {"J (--cfg default)" if sc == "J" else "rest (--cfg default)"} | {w} | {e["a_median"]:.4f} | {e["b_median"]:.4f} | {e["delta_ms"]:+.4f} | '
          f'{e["ratio"]:.4f} | {e["abs_ratio_minus_1"]:.4f} | {e["thr_se"]:.4f} | {e["thr_minmax"]:.4f} | {j} | {tj} | {aj} |')
    P()
    P('### (2) G9-on-C3 - parent 0ca312bd vs tip cbd86a65 (ratio = tip / parent)')
    P()
    P('| row | W | parent median | tip median | tip - parent ms | ratio | abs(ratio-1) | thr SE | thr min-max | K p/t |')
    P('|---|---|---|---|---|---|---|---|---|---|')
    for k, e in A['g9'].items():
        row, w = k.split('@W')
        P(f'| {row} | {w} | {e["a_median"]:.4f} | {e["b_median"]:.4f} | {e["delta_ms"]:+.4f} | {e["ratio"]:.4f} | {e["abs_ratio_minus_1"]:.4f} | '
          f'{e["thr_se"]:.4f} | {e["thr_minmax"]:.4f} | {e["K"][0]}/{e["K"][1]} |')
    for w in (1, 8):
        eff = A['stage_effects'].get(f'J-As-a@W{w}')
        if not eff:
            continue
        stp = A['stages'][f'J-As-a@W{w}#parent']['steps']
        P()
        P(f'#### J-As-a W={w}: per-stage spans (per process the median over steps [{stp[0]}, {stp[1]}), then the median over K; ms)')
        P()
        P('| zone | parent median | parent min-max | parent SE | tip median | tip min-max | tip SE | tip - parent | ratio | 2*hypot(SE) |')
        P('|---|---|---|---|---|---|---|---|---|---|')
        for s, e in eff.items():
            rt = f'{e["ratio"]:.4f}' if e['ratio'] is not None else '-'
            P(f'| {s} | {e["parent_ms"]:.4f} | {e["parent_min_max_ms"][0]:.4f}-{e["parent_min_max_ms"][1]:.4f} | {e["parent_se_ms"]:.4f} | '
              f'{e["tip_ms"]:.4f} | {e["tip_min_max_ms"][0]:.4f}-{e["tip_min_max_ms"][1]:.4f} | {e["tip_se_ms"]:.4f} | {e["delta_ms"]:+.4f} | {rt} | {e["two_se_ms"]:.4f} |')
        cp = A['stages'][f'J-As-a@W{w}#parent']
        ct = A['stages'][f'J-As-a@W{w}#tip']
        P()
        P('counters (median over K of the per-process median): ' + '; '.join(
            f'{c} parent {cp["stages"][c]["median"]:.1f} / tip {ct["stages"][c]["median"]:.1f}' for c in COUNTERS if c in cp['stages'] and c in ct['stages']))
        P(f'waves_total parent {cp["waves_total"]} / tip {ct["waves_total"]}; drops_total parent {cp["drops_total"]} / tip {ct["drops_total"]}; '
          f'solve_on_dispatcher_steps parent {cp["solve_on_dispatcher_steps"]} / tip {ct["solve_on_dispatcher_steps"]}')
        if w == 1 and 'phys_warm_apply_ns' in eff:
            e = eff['phys_warm_apply_ns']
            P()
            P(f'warm_apply at W=1, reference numbers side by side (arithmetic only): window 4b C2 {WIN4B_C2_WARM_APPLY_MS:.3f} ms; '
              f'this window parent {e["parent_ms"]:.4f} ms, tip {e["tip_ms"]:.4f} ms; tip - parent {e["delta_ms"]:+.4f} ms; '
              f'tip - 0.519 {e["tip_ms"] - WIN4B_C2_WARM_APPLY_MS:+.4f} ms; design bar {WARM_APPLY_BAR_MS:+.2f} ms; 2*hypot(SE) {e["two_se_ms"]:.4f} ms')
    P()
    P('### Load receipts')
    P()
    r = A['receipts']
    if r['n']:
        P(f'- {r["n"]} timed processes (non-voided); 5-s receipt before: median {r["before_median"]:.2f} %, max {r["before_max"]:.2f} %, > 5 %: {r["n_over_5_before"]}; '
          f'after: median {r["after_median"]:.2f} %, max {r["after_max"]:.2f} %, > 5 %: {r["n_over_5_after"]}')
        P(f'- during-process witness others_busy_pct: median {r["others_median"]:.2f} %, max {r["others_max"]:.2f} %, > 5 %: {r["others_over_5"]} '
          f'(of which in the chosen set: {r["others_over_5_in_chosen"]})')
        P(f'- build processes busy in receipts: {r["build_busy_in_receipts"] or "none"}; build/lane processes present at receipts: {r["presence_in_receipts"] or "none"}; '
          f'during a process: {r["build_during"] or "none"}; total wait for quiet before processes {r["waited_s_total"]:.0f} s; affinity masks read back {r["masks"]}')
    if A['invalid_list']:
        P(f'- INVALID processes: {A["invalid_list"]}')
    if dropped:
        P(f'- dropped slots: {dropped}')
    with open(os.path.join(RAW, f'tables{SUFFIX}.md'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(OUT) + '\n')

    # per-process receipt listing (every timed + warm-up + voided process)
    L = ['| block | pass/attempt | round | seq | row | binary | W | attempt | mean ms | receipt before % | receipt after % | others busy % | perf % | wall s | flags | in cell |',
         '|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|']
    for x in proc:
        flags = []
        if x.get('contaminated_before'):
            flags.append('CB')
        if x.get('contaminated_after'):
            flags.append('CA')
        if x.get('contaminated_during'):
            flags.append('BUILD-DURING')
        if x.get('invalid'):
            flags.append('INVALID ' + ';'.join(x['invalid']))
        if (x['block'], x['pass'], x['pass_attempt']) in voided:
            flags.append('VOIDED-PASS')
        if x.get('void'):
            flags.append('VOID-TRIGGER')
        pmx = perf_med(x)
        rb = (x.get('receipt_before') or {}).get('cpu_avg')
        ra = (x.get('receipt_after') or {}).get('cpu_avg')
        L.append(f'| {x["block"]} | {x["pass"]}/{x["pass_attempt"]} | {x["round"]} | {x["seq"]} | {x["row"]} | {x["binary"]} | {x["W"]} | {x["attempt"]} | '
                 f'{x["mean_ms"]:.4f} | {rb} | {ra} | {x.get("others_busy_pct")} | {f"{pmx:.1f}" if pmx is not None else "-"} | {x.get("wall_s")} | '
                 f'{" ".join(flags) or "-"} | {"yes" if x.get("_chosen") else "no"} |')
    if WITH_MAKEUP:
        return
    with open(os.path.join(RAW, 'process_receipts.md'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(L) + '\n')


if __name__ == '__main__':
    main()
