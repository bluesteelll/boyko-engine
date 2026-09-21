"""Reduce window 3 (win3/raw/runs.jsonl) under P0b's ruled protocol. Pure reading: launches nothing.

Per (row, W) cell: the MEDIAN over the K used processes of each process's window mean (ms/step), with
min-max, range %, IQR % (inclusive quartiles), SD % and the median's SE (1.2533 SD / sqrt K) as % of the
median -- `stats()` and `compare()` are P0b's analyze_window.py functions verbatim. A comparison of two
cells is CLAIMED only if |effect| > 2 x hypot(spread_A, spread_B), read three ways (range, IQR, SE).

Also per cell: the same statistics over the sub-windows [0,100) and [100,500) recomputed from each
process's per-step CSV (boyko run.csv wall_ns; Jolt per_frame 'Time (ms)'); Jolt's own steps/s metric
(the printed stat line); the boyko manifold/pair columns (per-step means over the sub-windows).

Usage: python -B analyze_win3.py [raw_dir]   (default: win3/raw)
"""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W3 = os.path.dirname(HERE)
RAW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(W3, 'raw')
sys.path.insert(0, os.path.join(HERE, 'lib'))
import driver as D  # noqa: E402

SUBWINDOWS = ((0, 100), (100, 500))
WITNESS_MAX_PCT = 1.5
ROWS_JSON = json.load(open(os.path.join(W3, 'rows.json'), encoding='utf-8'))
P0B_REF = {r['id']: r.get('p0b_reference_ms') for r in ROWS_JSON['rows'] if r.get('p0b_reference_ms')}


# -- loading (P0b's rule: the original first, else its re-run; contaminated or invalid attempts are excluded)

def load():
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
    warm = [r for r in recs if r.get('attempt') == 'warmup']
    attempts = [r for r in recs if r.get('attempt') != 'warmup']
    by_slot = {}
    for r in attempts:
        by_slot.setdefault((r['pass'], r['seq']), []).append(r)
    used, excluded = [], []
    for slot, rs in sorted(by_slot.items()):
        chosen = None
        for r in rs:
            if 'aborted' in r or 'skipped' in r:
                continue
            if r.get('valid') and not r.get('contaminated'):
                chosen = r
                break
        if chosen is None:
            excluded.append(rs)
        else:
            used.append(chosen)
    return recs, warm, attempts, used, excluded


# -- P0b's statistics, verbatim ------------------------------------------------

def quart(xs):
    if len(xs) < 2:
        return xs[0], xs[0]
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[0], q[2]


def stats(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    med = statistics.median(xs)
    q1, q3 = quart(xs)
    sd = statistics.stdev(xs) if len(xs) >= 2 else 0.0
    return {'n': len(xs), 'median': med, 'min': xs[0], 'max': xs[-1], 'q1': q1, 'q3': q3,
            'range_pct': 100 * (xs[-1] - xs[0]) / med, 'iqr_pct': 100 * (q3 - q1) / med, 'sd_pct': 100 * sd / med,
            'se_med_pct': 1.2533 * 100 * sd / med / math.sqrt(len(xs))}


def compare(a, b):
    """Effect of B against A, and the three claim readings."""
    if not a or not b:
        return None
    eff = 100 * (b['median'] / a['median'] - 1)
    cr = math.hypot(a['range_pct'], b['range_pct'])
    ci = math.hypot(a['iqr_pct'], b['iqr_pct'])
    cs = math.hypot(a['se_med_pct'], b['se_med_pct'])
    return {'ratio': b['median'] / a['median'], 'effect_pct': eff, 'bar_range_pct': 2 * cr, 'bar_iqr_pct': 2 * ci,
            'bar_se_pct': 2 * cs, 'claim_range': abs(eff) > 2 * cr, 'claim_iqr': abs(eff) > 2 * ci,
            'claim_se': abs(eff) > 2 * cs}


# -- per-process CSV -------------------------------------------------------------

def csv_cols(rec):
    if rec['engine'] == 'boyko':
        return D.load_csv(rec.get('csv'))
    c = D.load_csv(rec.get('per_frame'))
    if c is not None and 'Time (ms)' in c:
        c['wall_ns'] = [x * 1e6 if x is not None else None for x in c['Time (ms)']]
    return c


def sub_means(rec):
    """window mean over each sub-window (ms), from the per-step CSV; plus the CSV's full-window mean."""
    c = csv_cols(rec)
    if not c or 'wall_ns' not in c:
        return None
    w = c['wall_ns']
    out = {'full': D.mean(w[0:500]) / 1e6 if len(w) >= 500 else None, 'n': len(w)}
    for a, b in SUBWINDOWS:
        out[f'{a}_{b}'] = D.mean(w[a:b]) / 1e6 if len(w) >= b else None
    if rec['engine'] == 'boyko':
        for col in ('manifolds', 'pairs'):
            if col in c:
                out[col] = {f'{a}_{b}': D.mean(c[col][a:b]) for a, b in SUBWINDOWS}
                out[col]['0_500'] = D.mean(c[col][0:500])
                out[col]['final'] = c[col][-1]
    return out


def main():
    recs, warm, attempts, used, excluded = load()
    R = {'raw': RAW, 'records': len(recs), 'attempts': len(attempts), 'used': len(used), 'warmups': len(warm),
         'nonzero_exits': [(r['row'], r['W'], r['pass'], r['seq'], r.get('exit')) for r in attempts if r.get('exit') not in (0, None)]}
    R['reruns'] = [{'pass': r['pass'], 'round': r['round'], 'seq': r['seq'], 'row': r['row'], 'W': r['W'],
                    'reason': r.get('rerun_reason'), 'rerun_valid': r.get('valid'), 'rerun_contaminated': r.get('contaminated'),
                    'rerun_mean_ms': r.get('mean_ms')} for r in attempts if r.get('attempt') == 'rerun']
    R['excluded_slots'] = [[{'row': r['row'], 'W': r['W'], 'pass': r['pass'], 'seq': r['seq'], 'attempt': r.get('attempt'),
                             'invalid': r.get('invalid'), 'contaminated': r.get('contaminated'), 'skipped': r.get('skipped'),
                             'aborted': r.get('aborted')} for r in rs] for rs in excluded]
    R['contaminated_attempts'] = [{'row': r['row'], 'W': r['W'], 'pass': r['pass'], 'round': r['round'], 'seq': r['seq'],
                                   'attempt': r.get('attempt'), 'rb': r['receipt_before']['cpu_avg'],
                                   'ra': r['receipt_after']['cpu_avg'],
                                   'build_before': r['receipt_before']['build_procs_busy'],
                                   'build_after': r['receipt_after']['build_procs_busy'],
                                   'top_after': r['receipt_after']['top5'][:3], 'top_before': r['receipt_before']['top5'][:3],
                                   'mean_ms': r.get('mean_ms')}
                                  for r in attempts if r.get('contaminated')]
    R['invalid_attempts'] = [{'row': r['row'], 'W': r['W'], 'pass': r['pass'], 'seq': r['seq'], 'why': r.get('invalid')}
                             for r in attempts if r.get('invalid')]
    R['warmups'] = [{'pass': r['pass'], 'mean_ms': r.get('mean_ms'), 'valid': r.get('valid')} for r in warm]

    # Receipts: every distinct receipt (the between-process ones are shared).
    seen = {}
    for r in recs:
        for k in ('receipt_before', 'receipt_after'):
            x = r.get(k)
            if x:
                seen[x['time']] = x
    rc = sorted(seen.values(), key=lambda x: x['time'])
    vals = [x['cpu_avg'] for x in rc if x['cpu_avg'] is not None]
    tops = {}
    for x in rc:
        if x['top5']:
            tops[x['top5'][0]['name']] = tops.get(x['top5'][0]['name'], 0) + 1
    R['receipts'] = {'n': len(vals), 'n_10s': sum(1 for x in rc if x['window_s'] >= 10), 'min': min(vals),
                     'median': statistics.median(vals),
                     'p90': statistics.quantiles(vals, n=10)[-1] if len(vals) > 10 else None,
                     'max': max(vals), 'over5': [(x['time'], x['cpu_avg'], [t['name'] for t in x['top5'][:3]])
                                                 for x in rc if (x['cpu_avg'] or 0) > 5],
                     'build_procs': [(x['time'], x['build_procs_busy']) for x in rc if x['build_procs_busy']],
                     'top_process_counts': dict(sorted(tops.items(), key=lambda kv: -kv[1])[:6])}
    ob = [r['others_busy_pct'] for r in used if r.get('others_busy_pct') is not None]
    R['others_during'] = {'n': len(ob), 'median': statistics.median(ob) if ob else None, 'max': max(ob) if ob else None,
                          'over5': [(r['row'], r['W'], r['pass'], r['seq'], r['others_busy_pct'],
                                     [t['name'] for t in r['others_top5'][:3]]) for r in used
                                    if (r.get('others_busy_pct') or 0) > 5]}

    # Cells.
    cells = {}
    for r in used:
        cells.setdefault((r['row'], r['W']), []).append(r)
    C = {}
    for (row, w), rs in sorted(cells.items(), key=lambda kv: (kv[0][1], kv[0][0])):
        rs = sorted(rs, key=lambda r: (r['g'], r['seq']))
        s = stats([r['mean_ms'] for r in rs])
        s['values_by_process'] = [round(r['mean_ms'], 4) for r in rs]
        s['process_ids'] = [f'p{r["pass"]}r{r["round"]}s{r["seq"]}{"R" if r.get("attempt") == "rerun" else ""}' for r in rs]
        s['per_pass_median'] = {p: statistics.median([r['mean_ms'] for r in rs if r['pass'] == p]) for p in (0, 1)
                                if any(r['pass'] == p for r in rs)}
        s['median_step'] = stats([r['median_ms'] for r in rs if r.get('median_ms') is not None])
        subs = [sub_means(r) for r in rs]
        s['csv_vs_record_max_abs_rel'] = max((abs(x['full'] / r['mean_ms'] - 1) for x, r in zip(subs, rs) if x and x['full']), default=None)
        s['others_busy_by_process'] = [r['others_busy_pct'] for r in rs]
        s['others_top_by_process'] = [(r['others_top5'][0]['name'] if r['others_top5'] else None) for r in rs]
        s['pass_by_process'] = [r['pass'] for r in rs]
        kept = [r['mean_ms'] for r in rs if (r.get('others_busy_pct') or 0) <= WITNESS_MAX_PCT]
        s['supp_witness_filtered'] = {'rule': f'SUPPLEMENTARY, not the ruled statistic: processes whose during-process others_busy_pct <= {WITNESS_MAX_PCT} (P0b p95 1.44)',
                                      'n_excluded': len(rs) - len(kept), 'stats': stats(kept)}
        s['sub'] = {}
        for a, b in SUBWINDOWS:
            k = f'{a}_{b}'
            s['sub'][k] = stats([x[k] for x in subs if x and x.get(k) is not None])
        if rs[0]['engine'] == 'boyko':
            s['summary_vs_csv_max_rel'] = max(abs(r['summary']['window_mean_ns'] / 1e6 / r['mean_ms'] - 1) for r in rs)
            s['manifolds'] = {k: sorted({round(x['manifolds'][k], 3) for x in subs if x and 'manifolds' in x and x['manifolds'][k] is not None})
                              for k in ('0_100', '100_500', '0_500', 'final')}
            s['pairs'] = {k: sorted({round(x['pairs'][k], 3) for x in subs if x and 'pairs' in x and x['pairs'][k] is not None})
                          for k in ('0_100', '100_500', '0_500', 'final')}
            s['final_manifolds'] = sorted({r['summary']['final_manifolds'] for r in rs})
            s['final_pairs'] = sorted({r['summary']['final_pairs'] for r in rs})
            s['pose'] = sorted({r['summary']['pose_hash'] for r in rs})
            s['config'] = rs[0]['summary']['config']
            s['expect_pose'] = sorted({r['summary'].get('expect_pose') for r in rs})
            s['pool_workers'] = sorted({r['summary']['threads']['pool_workers'] for r in rs})
        else:
            sps = [r['jolt']['stat_lines'][0]['steps_per_s'] for r in rs]
            s['steps_per_s'] = stats(sps)
            s['steps_per_s_values'] = [round(x, 3) for x in sps]
            s['ms_from_steps_per_s'] = stats([1000.0 / x for x in sps])
            s['hash'] = sorted({r['jolt']['stat_lines'][0]['hash'] for r in rs})
            s['threads'] = sorted({r['jolt']['stat_lines'][0]['threads'] for r in rs})
            s['patch_line'] = sorted({r['jolt'].get('patch_line') for r in rs})
        C[f'{row}@W{w}'] = s
    R['cells'] = C

    def c(row, w):
        return C.get(f'{row}@W{w}')

    cmp = {}
    for w in (1, 2, 4, 8, 16):
        cmp[f'boyko D-L4L2 / Jolt v5.3.0 @W{w}'] = compare(c('JOLT-T', w), c('D-L4L2', w))
        cmp[f'boyko D-L4L2 / Jolt v5.6.0 @W{w}'] = compare(c('JOLT56-T', w), c('D-L4L2', w))
        cmp[f'Jolt v5.6.0 vs v5.3.0 @W{w}'] = compare(c('JOLT-T', w), c('JOLT56-T', w))
        cmp[f'D-L5 vs D-L4L2 @W{w}'] = compare(c('D-L4L2', w), c('D-L5', w))
        cmp[f'boyko D-L5 / Jolt v5.3.0 @W{w}'] = compare(c('JOLT-T', w), c('D-L5', w))
        cmp[f'boyko D-L5 / Jolt v5.6.0 @W{w}'] = compare(c('JOLT56-T', w), c('D-L5', w))
    for w in (1, 8):
        cmp[f'D-L4L2 vs J-A (bridge) @W{w}'] = compare(c('J-A', w), c('D-L4L2', w))
        cmp[f'J-A / Jolt v5.3.0 @W{w}'] = compare(c('JOLT-T', w), c('J-A', w))
    cmp['D-L5-npoff vs D-L5 @W8'] = compare(c('D-L5', 8), c('D-L5-npoff', 8))
    cmp['D-L5-npoff vs D-L4L2 @W8 (same knobs, different binaries)'] = compare(c('D-L4L2', 8), c('D-L5-npoff', 8))
    cmp['D-L5 vs J-A (bridge) @W1'] = compare(c('J-A', 1), c('D-L5', 1))
    cmp['D-L5 vs J-A (bridge) @W8'] = compare(c('J-A', 8), c('D-L5', 8))
    for row in ('D-L4L2', 'D-L5', 'JOLT-T', 'JOLT56-T'):
        cmp[f'{row} W16 vs W8'] = compare(c(row, 8), c(row, 16))
    cmp['D-L5-npon vs D-L5 @W8'] = compare(c('D-L5', 8), c('D-L5-npon', 8))
    cmp['D-L5-npon vs D-L4L2 @W1'] = compare(c('D-L4L2', 1), c('D-L5-npon', 1))
    cmp['D-L5-npon vs D-L4L2 @W8'] = compare(c('D-L4L2', 8), c('D-L5-npon', 8))
    # sub-window ratios
    for w in (1, 2, 4, 8, 16):
        for k in ('0_100', '100_500'):
            a, b = c('JOLT-T', w), c('D-L4L2', w)
            if a and b:
                cmp[f'boyko D-L4L2 / Jolt v5.3.0 @W{w} [{k.replace("_", ",")})'] = compare(a['sub'][k], b['sub'][k])
    R['comparisons'] = {k: v for k, v in cmp.items() if v}
    def cf(row, w):
        x = C.get(f'{row}@W{w}')
        return x['supp_witness_filtered']['stats'] if x else None
    supp = {}
    for w in (1, 2, 4, 8, 16):
        supp[f'boyko D-L4L2 / Jolt v5.3.0 @W{w}'] = compare(cf('JOLT-T', w), cf('D-L4L2', w))
        supp[f'boyko D-L5 / Jolt v5.3.0 @W{w}'] = compare(cf('JOLT-T', w), cf('D-L5', w))
        supp[f'boyko D-L5 / Jolt v5.6.0 @W{w}'] = compare(cf('JOLT56-T', w), cf('D-L5', w))
        supp[f'D-L5 vs D-L4L2 @W{w}'] = compare(cf('D-L4L2', w), cf('D-L5', w))
    supp['D-L5-npoff vs D-L5 @W8'] = compare(cf('D-L5', 8), cf('D-L5-npoff', 8))
    supp['D-L5-npoff vs D-L4L2 @W8'] = compare(cf('D-L4L2', 8), cf('D-L5-npoff', 8))
    R['supp_comparisons_witness_filtered'] = {k: v for k, v in supp.items() if v}

    # The bridge to P0b: this window's cell against P0b's reference median (a single number; the P0b
    # spread is quoted in the report from ANALYSIS.md, not recomputed here).
    bridge = {}
    for rid, refs in P0B_REF.items():
        for w, ref in refs.items():
            cell = c(rid, int(w))
            if cell:
                bridge[f'{rid}@W{w}'] = {'p0b_median_ms': ref, 'win3_median_ms': cell['median'],
                                        'win3_min': cell['min'], 'win3_max': cell['max'],
                                        'shift_pct': 100 * (cell['median'] / ref - 1),
                                        'win3_range_pct': cell['range_pct'], 'win3_iqr_pct': cell['iqr_pct'],
                                        'win3_se_pct': cell['se_med_pct'],
                                        'p0b_inside_win3_minmax': cell['min'] <= ref <= cell['max']}
    R['bridge_to_p0b'] = bridge

    R['scaling'] = {}
    for row in ('D-L4L2', 'J-A', 'JOLT-T', 'JOLT56-T', 'D-L5'):
        t1 = c(row, 1)
        if t1:
            R['scaling'][row] = {w: t1['median'] / c(row, w)['median'] for w in (1, 2, 4, 8, 16) if c(row, w)}

    # Structural checks straight from the records.
    poses = {}
    for r in used:
        if r['engine'] == 'boyko':
            poses.setdefault(r['row'], set()).add(r['summary']['pose_hash'])
        else:
            poses.setdefault(r['row'], set()).add(r['jolt']['stat_lines'][0]['hash'])
    R['hashes_per_row'] = {k: sorted(v) for k, v in poses.items()}
    R['void_steps_nonzero'] = [(r['row'], r['W'], r['g']) for r in used if r['engine'] == 'boyko' and r['summary']['void_steps']]
    R['drops_nonzero'] = [(r['row'], r['W'], r['g']) for r in used if r['engine'] == 'boyko' and r['summary'].get('drops_total')]
    R['disarmed_ring_traffic_nonzero'] = [(r['row'], r['W']) for r in used if r['engine'] == 'boyko'
                                          and r['summary'].get('disarmed_ring_traffic')]
    R['jolt_threads'] = sorted({(r['row'], r['W'], r['jolt']['stat_lines'][0]['threads']) for r in used if r['engine'] == 'jolt'})
    R['masks'] = sorted({r.get('mask_readback') for r in used})
    R['target_env'] = sorted({r['summary']['target_env'] for r in used if r['engine'] == 'boyko'})
    R['profiles'] = sorted({(r['row'], r['summary']['profile_name']) for r in used if r['engine'] == 'boyko'})
    R['exe_sha_per_row'] = {k: sorted({r['exe_sha256'] for r in used if r['row'] == k}) for k in poses}
    R['expect_pose_gate'] = sorted({(r['row'], r['W'], r['summary'].get('expect_pose')) for r in used if r['engine'] == 'boyko'})

    # Jolt's own receipts (untimed).
    rp = os.path.join(RAW, 'receipts', 'receipts.json')
    if os.path.isfile(rp):
        R['jolt_receipts'] = [{k: r.get(k) for k in ('row', 'exit', 'manifolds', 'args')} | {'hash': r['jolt']['stat_lines'][0]['hash'] if r['jolt']['stat_lines'] else None}
                              for r in json.load(open(rp, encoding='utf-8'))]

    # Per-manifold times, both sides' own counts, over [100,500).
    pm = {}
    jr = {x['row']: x for x in R.get('jolt_receipts', [])}
    for w in (1, 2, 4, 8, 16):
        b = c('D-L4L2', w)
        if not b:
            continue
        bm = b['manifolds']['100_500'][0] if b.get('manifolds') and b['manifolds']['100_500'] else None
        e = {'boyko_manifolds_100_500': bm, 'boyko_us_per_manifold_100_500': 1000 * b['sub']['100_500']['median'] / bm if (bm and b['sub'].get('100_500')) else None}
        for rid in ('JOLT-T', 'JOLT56-T'):
            j = c(rid, w)
            jm = (jr.get(rid) or {}).get('manifolds', {}).get('mean_100_500') if jr.get(rid) else None
            if j and jm and j['sub'].get('100_500') and e['boyko_us_per_manifold_100_500']:
                e[f'{rid}_manifolds_100_500'] = jm
                e[f'{rid}_us_per_manifold_100_500'] = 1000 * j['sub']['100_500']['median'] / jm
                e[f'per_manifold_ratio_boyko_over_{rid}'] = e['boyko_us_per_manifold_100_500'] / e[f'{rid}_us_per_manifold_100_500']
        pm[w] = e
    R['per_manifold'] = pm

    # Witness: the 1-Hz '% Processor Performance' total per process (mean), by pass.
    def perf_total(r):
        t = [p['total'] for p in r.get('perf', []) if p.get('total') is not None]
        return statistics.mean(t) if t else None
    R['perf_total_by_pass'] = {p: statistics.median([perf_total(r) for r in used if r['pass'] == p and perf_total(r) is not None])
                               for p in (0, 1) if any(r['pass'] == p for r in used)}

    # Wall clock.
    st = json.load(open(os.path.join(RAW, 'window_state.json'), encoding='utf-8')) \
        if os.path.isfile(os.path.join(RAW, 'window_state.json')) else {}
    R['wall_clock'] = {'start': st.get('start'), 'end': st.get('end'), 'status': st.get('status'),
                       'first_process': min(r['start'] for r in attempts if r.get('start')),
                       'last_process_end': max(r['end'] for r in attempts if r.get('end')),
                       'binaries_after': st.get('binaries_after'),
                       'machine_before': st.get('machine_before'), 'machine_after': st.get('machine_after'),
                       'order_pass0': st.get('order_pass0'), 'cells_per_round': st.get('cells_per_round'),
                       'per_pass': {p: [min(r['start'] for r in recs if r['pass'] == p and r.get('start')),
                                        max(r['end'] for r in recs if r['pass'] == p and r.get('end'))]
                                    for p in sorted({r['pass'] for r in recs})}}
    with open(os.path.join(RAW, 'analysis.json'), 'w', encoding='utf-8') as f:
        json.dump(R, f, indent=1, default=str)
    print(f'analysis: {len(used)} used processes, {len(C)} cells -> {os.path.join(RAW, "analysis.json")}')
    for k, s in C.items():
        print(f'  {k:16s} K={s["n"]} median {s["median"]:.4f} [{s["min"]:.4f}-{s["max"]:.4f}] range {s["range_pct"]:.2f}% '
              f'iqr {s["iqr_pct"]:.2f}% se {s["se_med_pct"]:.2f}%')


if __name__ == '__main__':
    main()
