"""Second reduction of p0b/raw/runs.jsonl: is the per-process spread a per-PROCESS property or a
per-PASS (time / machine-state) property? Two-way decomposition pass x policy per (engine, W) on
log(window mean), and the spread of within-pass PAIRED ratios against the unpaired spread.
Writes raw/analysis2.md and raw/analysis2.json. Pure reading."""
import json
import math
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, 'raw')
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
import analyze as A  # noqa: E402

F_2_10_95 = 4.10   # F(2,10) upper 5 %
F_5_10_95 = 3.33   # F(5,10) upper 5 %


def rng(xs):
    return 100 * (max(xs) - min(xs)) / statistics.median(xs)


def iqr(xs):
    q1, q3 = A.q(xs)
    return 100 * (q3 - q1) / statistics.median(xs)


def main():
    recs, attempts, used, excluded, warm = A.load()
    T = {(r['engine'], r['W'], r['policy'], r['pass']): r['mean_ms'] for r in used}
    passes = sorted({r['pass'] for r in used})
    out = {'anova': {}, 'paired_policy': {}, 'paired_ratio': {}, 'pass_effects': {}}
    L = ['## Two-way decomposition of log(window mean): pass x policy (no replication)', '',
         '| engine | W | SD pass effect % | SD policy effect % | residual SD % | F pass (5,10) | F policy (2,10) |',
         '|---|---|---|---|---|---|---|']
    for eng in A.ENGINES:
        for w in A.WS:
            y = {(p, pol): math.log(T[(eng, w, pol, p)]) for p in passes for pol in A.POLICIES}
            g = statistics.mean(y.values())
            pe = {p: statistics.mean(y[(p, pol)] for pol in A.POLICIES) - g for p in passes}
            qe = {pol: statistics.mean(y[(p, pol)] for p in passes) - g for pol in A.POLICIES}
            res = {k: y[k] - g - pe[k[0]] - qe[k[1]] for k in y}
            a, b = len(passes), len(A.POLICIES)
            ss_p = b * sum(v * v for v in pe.values())
            ss_q = a * sum(v * v for v in qe.values())
            ss_r = sum(v * v for v in res.values())
            ms_p, ms_q, ms_r = ss_p / (a - 1), ss_q / (b - 1), ss_r / ((a - 1) * (b - 1))
            fp, fq = ms_p / ms_r, ms_q / ms_r
            sd_r = 100 * math.sqrt(ms_r)
            # the pass component of a single process's variance: (ms_p - ms_r) / b, floored at 0
            sd_pass = 100 * math.sqrt(max(0.0, (ms_p - ms_r) / b))
            sd_pol = 100 * math.sqrt(max(0.0, (ms_q - ms_r) / a))
            out['anova'][f'{eng}|W{w}'] = {'F_pass': fp, 'F_policy': fq, 'sd_pass_pct': sd_pass, 'sd_policy_pct': sd_pol,
                                           'sd_residual_pct': sd_r,
                                           'pass_effects_pct': {p: 100 * v for p, v in pe.items()},
                                           'policy_effects_pct': {k: 100 * v for k, v in qe.items()}}
            L.append(f'| {eng} | {w} | {sd_pass:.2f} | {sd_pol:.2f} | {sd_r:.2f} | {fp:.2f}{" *" if fp > F_5_10_95 else ""} | '
                     f'{fq:.2f}{" *" if fq > F_2_10_95 else ""} |')
    L += ['', '(* = significant at 5 %: F(5,10) > 3.33, F(2,10) > 4.10)', '', '### Pass effects (% of the grand mean), per (engine, W)', '',
          '| engine | W | ' + ' | '.join(f'p{p}' for p in passes) + ' |', '|---|---|' + '---|' * len(passes)]
    for k, v in out['anova'].items():
        eng, w = k.split('|')
        L.append(f'| {eng} | {w[1:]} | ' + ' | '.join(f'{v["pass_effects_pct"][p]:+.2f}' for p in passes) + ' |')
    L += ['', '## Paired (same pass) vs unpaired spread of the policy effect', '',
          '| comparison | unpaired: ratio of medians | paired: median of per-pass ratios | paired range % | paired IQR % |',
          '|---|---|---|---|---|']
    for eng in A.ENGINES:
        for w in A.WS:
            for pol in ('P-phys', 'P-full'):
                rs = [T[(eng, w, pol, p)] / T[(eng, w, 'P-none', p)] for p in passes]
                um = statistics.median([T[(eng, w, pol, p)] for p in passes]) / statistics.median(
                    [T[(eng, w, 'P-none', p)] for p in passes])
                out['paired_policy'][f'{pol}/P-none|{eng}|W{w}'] = {'ratios': rs, 'median': statistics.median(rs),
                                                                     'range_pct': rng(rs), 'iqr_pct': iqr(rs), 'unpaired': um}
                L.append(f'| {pol}/P-none {eng} W={w} | {um:.4f} | {statistics.median(rs):.4f} | {rng(rs):.2f} | {iqr(rs):.2f} |')
    L += ['', '## boyko/Jolt per policy: paired within the pass vs unpaired', '',
          '| policy | W | unpaired (median/median) | paired median | paired range % | paired IQR % | per-pass ratios |',
          '|---|---|---|---|---|---|---|']
    for pol in A.POLICIES:
        for w in A.WS:
            rs = [T[('boyko', w, pol, p)] / T[('jolt', w, pol, p)] for p in passes]
            um = statistics.median([T[('boyko', w, pol, p)] for p in passes]) / statistics.median(
                [T[('jolt', w, pol, p)] for p in passes])
            out['paired_ratio'][f'{pol}|W{w}'] = {'ratios': rs, 'median': statistics.median(rs), 'range_pct': rng(rs),
                                                  'iqr_pct': iqr(rs), 'unpaired': um}
            L.append(f'| {pol} | {w} | {um:.4f} | {statistics.median(rs):.4f} | {rng(rs):.2f} | {iqr(rs):.2f} | '
                     f'{[round(x, 4) for x in rs]} |')
    # Pooled over policies (placement does not matter, see above): 18 processes per (engine, W).
    L += ['', '## Pooled over the three policies (18 processes per engine and W)', '',
          '| engine | W | n | median ms | min | max | range % | IQR % | SD % |', '|---|---|---|---|---|---|---|---|---|']
    out['pooled'] = {}
    for eng in A.ENGINES:
        for w in A.WS:
            xs = [T[(eng, w, pol, p)] for pol in A.POLICIES for p in passes]
            c = A.cell_stats(xs)
            out['pooled'][f'{eng}|W{w}'] = c
            L.append(f'| {eng} | {w} | {c["n"]} | {c["median"]:.4f} | {c["min"]:.4f} | {c["max"]:.4f} | {c["range_pct"]:.2f} | '
                     f'{c["iqr_pct"]:.2f} | {c["sd_pct"]:.2f} |')
    # Whole-run shift vs transient: per process, mean over [100,500) vs [0,100) ... use median_ms vs mean_ms
    L += ['', '## Whole-run shift or transient? (per-process median step against the window mean, pooled)', '',
          '| engine | W | range % of means | range % of per-process medians | corr(mean, median) |', '|---|---|---|---|---|']
    for eng in A.ENGINES:
        for w in A.WS:
            rs = [r for r in used if r['engine'] == eng and r['W'] == w]
            m1 = [r['mean_ms'] for r in rs]
            m2 = [r['median_ms'] for r in rs]
            L.append(f'| {eng} | {w} | {rng(m1):.2f} | {rng(m2):.2f} | {statistics.correlation(m1, m2):.2f} |')
    with open(os.path.join(RAW, 'analysis2.json'), 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1, default=str)
    with open(os.path.join(RAW, 'analysis2.md'), 'w', encoding='utf-8') as f:
        f.write('\n'.join(L) + '\n')
    print('\n'.join(L))


if __name__ == '__main__':
    main()
