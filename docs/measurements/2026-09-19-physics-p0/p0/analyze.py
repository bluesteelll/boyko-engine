"""Supplementary analysis of the P0 window (raw/), beside the driver's own reduce.

Per row and W: the window mean step time in every pass (boyko: the per-step CSV over the row's
window; Jolt: the -f per-frame CSV over all frames, cross-checked against Jolt's printed steps/s)
and the median, min and max over passes. Per armed run: a per-step closure census (g < 0, u < 0,
u > 3 % of the solve span, sum of system spans > wall, void steps). The receipts: every before
and after receipt of every attempt. The wall-clock window.

Usage: analyze.py [RAW_DIR]
"""
import collections
import json
import os
import statistics
import sys

sys.path.insert(0, r'D:/wt/joltab/tools/physics_parity')
import driver as D  # noqa: E402

RAW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    r'(session scratch)/p0', 'raw')


def load(path):
    return [json.loads(l) for l in open(path, encoding='utf-8')] if os.path.isfile(path) else []


def csv_cols(path):
    return D.load_csv(path)


def boyko_window_mean(rec):
    c = csv_cols(rec.get('csv'))
    if c is None:
        return None, None
    w = D.window_of(rec)
    xs = c['wall_ns'][w[0]:w[1]]
    return (sum(xs) / len(xs) if xs else None), c


def jolt_frame_mean(rec):
    c = csv_cols(rec.get('per_frame'))
    if c is None:
        return None, None
    xs = [x * 1e6 for x in c.get('Time (ms)', []) if x is not None]
    return (sum(xs) / len(xs) if xs else None), c


def fmt_ns(x):
    if x is None:
        return 'n/a'
    return f'{x / 1e6:.4f} ms'


def main():
    clean = [r for r in load(os.path.join(RAW, 'runs.jsonl')) if 'skipped' not in r]
    allr = load(os.path.join(RAW, 'runs_all.jsonl')) or load(os.path.join(RAW, 'runs.jsonl'))
    excluded = [r for r in load(os.path.join(RAW, 'runs.jsonl')) if 'skipped' in r]
    out = {'raw': RAW}
    lines = []
    P = lines.append

    # Wall clock.
    state = json.load(open(os.path.join(RAW, 'window_state.json'), encoding='utf-8')) \
        if os.path.isfile(os.path.join(RAW, 'window_state.json')) else {}
    out['window'] = {k: state.get(k) for k in ('window_start', 'window_end', 'status')}
    passes = sorted({r['pass'] for r in allr})
    per_pass = {}
    for p in passes:
        rs = [r for r in allr if r['pass'] == p and 'start' in r]
        if rs:
            per_pass[p] = {'first_start': min(r['start'] for r in rs), 'last_end': max(r['end'] for r in rs),
                           'attempts': len(rs), 'reruns': sum(1 for r in rs if r.get('attempt') == 'rerun')}
    out['per_pass'] = per_pass
    P(f'window: {out["window"]}')
    for p, v in per_pass.items():
        P(f'  pass {p}: {v["first_start"]} .. {v["last_end"]}; attempts {v["attempts"]} (re-runs {v["reruns"]})')

    # Per row and W: window means per pass.
    table = collections.OrderedDict()
    for r in clean:
        key = (r['row'], r['W'])
        if r['engine'] == 'boyko':
            m, _ = boyko_window_mean(r)
            s = r.get('summary') or {}
            extra = {'summary_window_mean_ns': s.get('window_mean_ns'), 'exit': r.get('exit'),
                     'void_steps': s.get('void_steps'), 'pose_hash': s.get('pose_hash')}
        else:
            m, _ = jolt_frame_mean(r)
            st = (r.get('jolt') or {}).get('stat_lines') or []
            extra = {'printed_ns_per_step': (1e9 / st[0]['steps_per_s']) if st else None, 'exit': r.get('exit'),
                     'hash': st[0]['hash'] if st else None, 'threads': st[0]['threads'] if st else None}
        table.setdefault(key, []).append({'pass': r['pass'], 'mean_ns': m, 'attempt': r.get('attempt'), **extra})
    rows_out = {}
    P('')
    P('per row and W: window mean step time, median over passes [min, max] (n passes); per pass in pass order')
    order = [row['id'] for row in D.BOYKO_ROWS] + [row['id'] for row in D.JOLT_ROWS]
    for rid in order:
        for w in D.WS:
            v = table.get((rid, w))
            if not v:
                continue
            ms = [x['mean_ns'] for x in v if x['mean_ns'] is not None]
            med = statistics.median(ms) if ms else None
            e = {'per_pass': v, 'median_ns': med, 'min_ns': min(ms) if ms else None, 'max_ns': max(ms) if ms else None,
                 'n': len(ms), 'spread': (max(ms) / min(ms) - 1) if ms else None}
            if rid.startswith('JOLT'):
                pr = [x['printed_ns_per_step'] for x in v if x.get('printed_ns_per_step')]
                e['printed_median_ns'] = statistics.median(pr) if pr else None
                e['csv_vs_printed_max_rel'] = max((abs(x['mean_ns'] / x['printed_ns_per_step'] - 1) for x in v
                                                   if x['mean_ns'] and x.get('printed_ns_per_step')), default=None)
            else:
                e['csv_vs_summary_max_rel'] = max((abs(x['mean_ns'] / x['summary_window_mean_ns'] - 1) for x in v
                                                   if x['mean_ns'] and x.get('summary_window_mean_ns')), default=None)
            rows_out[f'{rid}@W{w}'] = e
            pp = ' '.join(f'p{x["pass"]}={fmt_ns(x["mean_ns"])}{"(rerun)" if x["attempt"] == "rerun" else ""}'
                          for x in v)
            sp = f'{100 * e["spread"]:.2f}%' if e['spread'] is not None else 'n/a'
            P(f'  {rid:9s} W={w:<2d} median {fmt_ns(med)} [{fmt_ns(e["min_ns"])}, {fmt_ns(e["max_ns"])}] '
              f'n={e["n"]} spread {sp} | {pp}')
    out['rows'] = rows_out

    # Per-step closure census on every armed boyko run (clean set).
    census = []
    for r in clean:
        if r['engine'] != 'boyko' or not (r.get('summary') or {}).get('armed'):
            continue
        c = csv_cols(r.get('csv'))
        if c is None:
            census.append({'run': f'{r["row"]}@W{r["W"]} p{r["pass"]}', 'csv': 'missing'})
            continue
        n = len(c['wall_ns'])
        solve = c.get('sys_physics_solve_colored_ns') or [None] * n
        g_neg = sum(1 for x in c['g_ns'] if x is not None and x < 0)
        u_neg = sum(1 for x in c['u_ns'] if x is not None and x < 0)
        u_big = sum(1 for u, s in zip(c['u_ns'], solve) if u is not None and s and u > 0.03 * s)
        sys_over = sum(1 for s, w in zip(c['sys_sum_ns'], c['wall_ns']) if s is not None and w is not None and s > w)
        void = sum(1 for x in c['void'] if x)
        census.append({'run': f'{r["row"]}@W{r["W"]} p{r["pass"]}', 'steps': n, 'g_neg': g_neg, 'u_neg': u_neg,
                       'u_gt_3pct_solve': u_big, 'sys_sum_gt_wall': sys_over, 'void': void})
    out['closure_census'] = census
    bad = [x for x in census if x.get('csv') == 'missing' or x['g_neg'] or x['u_neg'] or x['u_gt_3pct_solve']
           or x['sys_sum_gt_wall'] or x['void']]
    P('')
    P(f'per-step closure census over {len(census)} armed runs: {len(bad)} with any step flagged')
    for x in bad:
        P(f'  {x}')

    # Receipts.
    recs = []
    for r in allr:
        for side in ('receipt_before', 'receipt_after'):
            if r.get(side):
                recs.append((r, side, r[side]))
    cpu = [x[2]['cpu_avg'] for x in recs if x[2].get('cpu_avg') is not None]
    top1 = collections.Counter(x[2]['top5'][0]['name'] for x in recs if x[2].get('top5'))
    build = collections.Counter(n for x in recs for n in x[2].get('build_procs_busy', []))
    over = [x for x in recs if (x[2].get('cpu_avg') or 0) > 5.0]
    orig = [r for r in allr if r.get('attempt') == 'original' and 'start' in r]
    cont_orig = [r for r in orig if r.get('contaminated_before') or r.get('contaminated_after')]
    reruns = [r for r in allr if r.get('attempt') == 'rerun']
    cont_rerun = [r for r in reruns if r.get('contaminated_before') or r.get('contaminated_after')]
    waited = sum(r.get('waited_s') or 0 for r in allr)
    q = sorted(cpu)
    out['receipts'] = {
        'n_receipts': len(recs), 'cpu_avg_min': q[0] if q else None, 'cpu_avg_median': statistics.median(q) if q else None,
        'cpu_avg_p90': q[int(0.9 * (len(q) - 1))] if q else None, 'cpu_avg_max': q[-1] if q else None,
        'over_5pct': len(over), 'top1_process_counts': top1.most_common(12), 'build_procs_busy': dict(build),
        'original_runs': len(orig), 'original_contaminated': [(r['row'], r['W'], r['pass']) for r in cont_orig],
        'reruns': len(reruns), 'reruns_contaminated': [(r['row'], r['W'], r['pass']) for r in cont_rerun],
        'excluded_from_reduction': [(r['row'], r['W'], r['pass'], r['skipped']) for r in excluded],
        'waited_s_total': round(waited, 1),
        'aborted': [(r['row'], r['W'], r['pass'], r.get('aborted')) for r in allr if 'aborted' in r]}
    rc = out['receipts']
    P('')
    P(f'receipts: {rc["n_receipts"]} (before+after of every attempt); cpu_avg min {rc["cpu_avg_min"]} median '
      f'{rc["cpu_avg_median"]} p90 {rc["cpu_avg_p90"]} max {rc["cpu_avg_max"]} (% of 16 logical CPUs); over 5 %: '
      f'{rc["over_5pct"]}')
    P(f'  top-1 process by CPU in the receipt window: {rc["top1_process_counts"]}')
    P(f'  build processes seen using CPU: {rc["build_procs_busy"] or "none"}')
    P(f'  original runs {rc["original_runs"]}, contaminated {len(rc["original_contaminated"])}: '
      f'{rc["original_contaminated"]}')
    P(f'  re-runs {rc["reruns"]}, contaminated again {len(rc["reruns_contaminated"])}: {rc["reruns_contaminated"]}')
    P(f'  excluded from the timed reduction: {rc["excluded_from_reduction"]}')
    P(f'  quiet-rule wait total {rc["waited_s_total"]} s; aborted {rc["aborted"]}')
    for x in over:
        r, side, rr = x
        P(f'    >5%: {r["row"]}@W{r["W"]} p{r["pass"]} {r.get("attempt")} {side} {rr["cpu_avg"]}% top '
          f'{[(t["name"], t["pct_of_machine"]) for t in rr["top5"][:3]]}')

    # Jolt determinism per (role, args, W) over every attempt.
    jh = collections.defaultdict(set)
    for r in allr:
        st = (r.get('jolt') or {}).get('stat_lines') or []
        if st:
            jh[(r['row'], r['W'])].add(st[0]['hash'])
    out['jolt_hashes'] = {f'{k[0]}@W{k[1]}': sorted(v) for k, v in jh.items()}
    # boyko pose hashes per row/W over every attempt.
    bh = collections.defaultdict(set)
    for r in allr:
        s = r.get('summary')
        if s:
            bh[(r['row'], r['W'])].add(s['pose_hash'])
    out['boyko_pose_hashes'] = {f'{k[0]}@W{k[1]}': sorted(v) for k, v in bh.items()}
    P('')
    P('boyko pose hash per row@W (every attempt): ' + json.dumps(out['boyko_pose_hashes']))
    P('Jolt hash per row@W (every attempt): ' + json.dumps(out['jolt_hashes']))
    exits = [(r['row'], r['W'], r['pass'], r.get('attempt'), r['exit']) for r in allr if r.get('exit') not in (0, None)]
    out['nonzero_exits_all_attempts'] = exits
    P(f'non-zero exits over every attempt: {exits}')

    with open(os.path.join(RAW, 'analysis.json'), 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1, default=str)
    text = '\n'.join(lines) + '\n'
    with open(os.path.join(RAW, 'analysis.txt'), 'wb') as f:
        f.write(text.encode('utf-8'))
    sys.stdout.buffer.write(text.encode('utf-8'))


if __name__ == '__main__':
    main()
