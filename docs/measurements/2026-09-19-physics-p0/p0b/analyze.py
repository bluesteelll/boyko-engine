"""Reduce p0b/raw/runs.jsonl: per (policy, engine, W) cell, the distribution over processes of the
window mean (the quoted statistic), plus the per-CPU placement witness. Writes raw/analysis.json and
raw/analysis.md. Pure reading; no process is launched."""
import json
import math
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, sys.argv[1] if len(sys.argv) > 1 else 'raw')
POLICIES = ('P-none', 'P-phys', 'P-full')
ENGINES = ('boyko', 'jolt')
WS = (2, 8)
F_5_5_975 = 7.146  # F(5,5) upper 2.5 % point: a variance ratio beyond it (or below 1/it) is a 5 % two-sided difference


def load():
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
    final = {}
    attempts = []
    for r in recs:
        if r.get('attempt') == 'warmup':
            continue
        attempts.append(r)
        key = (r['pass'], r['seq'])
        final.setdefault(key, []).append(r)
    used, excluded = [], []
    for key, rs in sorted(final.items()):
        chosen = None
        for r in rs:  # original first, then its re-run
            if 'aborted' in r:
                continue
            if r.get('valid') and not r.get('contaminated'):
                chosen = r
                break
        if chosen is None:
            excluded.append(rs[-1])
        else:
            used.append(chosen)
    warm = [r for r in recs if r.get('attempt') == 'warmup']
    return recs, attempts, used, excluded, warm


def q(xs):
    qs = statistics.quantiles(xs, n=4, method='inclusive')
    return qs[0], qs[2]


def cell_stats(xs):
    xs = sorted(xs)
    med = statistics.median(xs)
    q1, q3 = q(xs) if len(xs) >= 2 else (xs[0], xs[0])
    sd = statistics.stdev(xs) if len(xs) >= 2 else 0.0
    return {'n': len(xs), 'values': [round(x, 4) for x in xs], 'median': med, 'min': xs[0], 'max': xs[-1],
            'range_pct': 100 * (xs[-1] - xs[0]) / med, 'iqr_pct': 100 * (q3 - q1) / med, 'sd_pct': 100 * sd / med,
            'var': sd * sd}


def witness(r):
    b = r.get('percpu_busy')
    if not b:
        return None
    b = [x or 0.0 for x in b]
    cores = [(b[2 * k], b[2 * k + 1]) for k in range(len(b) // 2)]
    busy_cpus = [i for i, x in enumerate(b) if x >= 50]
    both = sum(1 for a, c in cores if a >= 30 and c >= 30)
    return {'sum_busy_cpus': round(sum(b) / 100, 2), 'cpus_ge50': busy_cpus,
            'cores_ge50': sorted({i // 2 for i in busy_cpus}), 'smt_pairs_both_ge30': both,
            'proc_cpu_per_wall': round(r['proc_cpu_s'] / r['wall_s'], 2) if r.get('proc_cpu_s') and r.get('wall_s') else None}


def main():
    recs, attempts, used, excluded, warm = load()
    out = {'attempts': len(attempts), 'used': len(used), 'excluded': [(r['pass'], r['seq'], r['engine'], r['W'], r['policy'])
                                                                         for r in excluded],
           'reruns': [(r['pass'], r['seq'], r['engine'], r['W'], r['policy'], r.get('rerun_reason'))
                      for r in attempts if r.get('attempt') == 'rerun'],
           'cells': {}, 'median_cells': {}, 'comparisons': {}, 'ratios': {}, 'witness': {}, 'drift': {}}
    rb = [r['receipt_before']['cpu_avg'] for r in attempts if r.get('receipt_before')] + \
         [r['receipt_after']['cpu_avg'] for r in attempts if r.get('receipt_after')]
    out['receipts'] = {'n': len(rb), 'median': statistics.median(rb), 'max': max(rb), 'min': min(rb),
                       'over5': sum(1 for x in rb if x > 5)}
    out['warmups'] = [(r['pass'], round(r.get('mean_ms', float('nan')), 4)) for r in warm]
    for pol in POLICIES:
        for eng in ENGINES:
            for w in WS:
                rs = [r for r in used if r['policy'] == pol and r['engine'] == eng and r['W'] == w]
                if not rs:
                    continue
                k = f'{pol}|{eng}|W{w}'
                out['cells'][k] = cell_stats([r['mean_ms'] for r in rs])
                out['median_cells'][k] = cell_stats([r['median_ms'] for r in rs])
                out['drift'][k] = [(r['pass'], round(r['mean_ms'], 4)) for r in sorted(rs, key=lambda x: x['pass'])]
                out['witness'][k] = [dict(pass_=r['pass'], mean_ms=round(r['mean_ms'], 4), **(witness(r) or {}))
                                     for r in sorted(rs, key=lambda x: x['pass'])]
    C = out['cells']
    for eng in ENGINES:
        for w in WS:
            base = C.get(f'P-none|{eng}|W{w}')
            for pol in ('P-phys', 'P-full'):
                c = C.get(f'{pol}|{eng}|W{w}')
                if not base or not c:
                    continue
                eff = 100 * (c['median'] / base['median'] - 1)
                comb_range = math.hypot(c['range_pct'], base['range_pct'])
                comb_iqr = math.hypot(c['iqr_pct'], base['iqr_pct'])
                fr = base['var'] / c['var'] if c['var'] > 0 else float('inf')
                out['comparisons'][f'{pol} vs P-none|{eng}|W{w}'] = {
                    'level_effect_pct': eff, 'combined_range_pct': comb_range, 'combined_iqr_pct': comb_iqr,
                    'level_claimable_2x_range': abs(eff) > 2 * comb_range,
                    'level_claimable_2x_iqr': abs(eff) > 2 * comb_iqr,
                    'var_ratio_none_over_pol': fr,
                    'spread_diff_significant_F5_5': fr > F_5_5_975 or fr < 1 / F_5_5_975}
    for pol in POLICIES:
        for w in WS:
            b, j = C.get(f'{pol}|boyko|W{w}'), C.get(f'{pol}|jolt|W{w}')
            if b and j:
                out['ratios'][f'{pol}|W{w}'] = {'boyko_over_jolt': b['median'] / j['median'],
                                                'resolution_pct_range': math.hypot(b['range_pct'], j['range_pct']),
                                                'resolution_pct_iqr': math.hypot(b['iqr_pct'], j['iqr_pct'])}
    with open(os.path.join(RAW, 'analysis.json'), 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1, default=str)
    L = []
    L.append(f'attempts {out["attempts"]}, used {out["used"]}, excluded {out["excluded"]}, reruns {out["reruns"]}')
    L.append(f'receipts {out["receipts"]}')
    L.append(f'warm-ups (pass, mean ms, boyko W=8 P-none): {out["warmups"]}')
    L.append('')
    L.append('| W | engine | policy | n | median ms | min | max | range % | IQR % | SD % | values (sorted) |')
    L.append('|---|---|---|---|---|---|---|---|---|---|---|')
    for w in WS:
        for eng in ENGINES:
            for pol in POLICIES:
                c = C.get(f'{pol}|{eng}|W{w}')
                if c:
                    L.append(f'| {w} | {eng} | {pol} | {c["n"]} | {c["median"]:.4f} | {c["min"]:.4f} | {c["max"]:.4f} | '
                             f'{c["range_pct"]:.2f} | {c["iqr_pct"]:.2f} | {c["sd_pct"]:.2f} | {c["values"]} |')
    L.append('')
    L.append('Per-process MEDIAN step (robustness: a whole-run shift moves it, a transient does not)')
    L.append('| W | engine | policy | median of medians ms | range % | IQR % |')
    L.append('|---|---|---|---|---|---|')
    for w in WS:
        for eng in ENGINES:
            for pol in POLICIES:
                c = out['median_cells'].get(f'{pol}|{eng}|W{w}')
                if c:
                    L.append(f'| {w} | {eng} | {pol} | {c["median"]:.4f} | {c["range_pct"]:.2f} | {c["iqr_pct"]:.2f} |')
    L.append('')
    L.append('| comparison | level effect % | 2x comb. range % | 2x comb. IQR % | var ratio none/pol | F(5,5) 5 % |')
    L.append('|---|---|---|---|---|---|')
    for k, v in out['comparisons'].items():
        L.append(f'| {k} | {v["level_effect_pct"]:+.2f} | {2 * v["combined_range_pct"]:.2f} | {2 * v["combined_iqr_pct"]:.2f} | '
                 f'{v["var_ratio_none_over_pol"]:.2f} | {"sig" if v["spread_diff_significant_F5_5"] else "ns"} |')
    L.append('')
    L.append('| policy | W | boyko/Jolt (medians) | resolution (RSS of ranges) % | (RSS of IQRs) % |')
    L.append('|---|---|---|---|---|')
    for k, v in out['ratios'].items():
        pol, w = k.split('|')
        L.append(f'| {pol} | {w} | {v["boyko_over_jolt"]:.4f} | {v["resolution_pct_range"]:.2f} | {v["resolution_pct_iqr"]:.2f} |')
    L.append('')
    L.append('Witness per process (pass order): mean ms, sum of per-CPU busy (CPUs), CPUs >= 50 % busy, SMT pairs both >= 30 %, proc CPU-s per wall-s')
    for k, ws_ in out['witness'].items():
        L.append(f'{k}:')
        for x in ws_:
            L.append(f'   p{x["pass_"]} {x["mean_ms"]:.4f} ms  sum {x.get("sum_busy_cpus")}  cpus>=50 {x.get("cpus_ge50")}  '
                     f'smt-both {x.get("smt_pairs_both_ge30")}  cpu/wall {x.get("proc_cpu_per_wall")}')
    with open(os.path.join(RAW, 'analysis.md'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(L) + '\n')
    print('\n'.join(L))


if __name__ == '__main__':
    main()
