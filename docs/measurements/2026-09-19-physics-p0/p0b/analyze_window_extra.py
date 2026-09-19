"""Supplementary reduction of the P0b window (pure reading): the queue's quoting items H1 (sub-windows
[0,100) and [100,500)) and H9 (time per manifold per step), the disparity-scene broadphase crossover,
the pooled per-process spread per engine and W, and the clock witness per engine and W.
Writes raw/window/analysis_extra.json."""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, 'raw', 'window')
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, HERE)
import driver as D  # noqa: E402
import analyze_window as A  # noqa: E402

JOLT_MANIFOLDS_100_500 = 8456.0  # P0 JOLT-RCPT count over [100, 500) (a count; p0/wf_reduce.md section 4)


def main():
    recs, warm, attempts, used, excluded = A.load()
    cells = {}
    for r in used:
        cells.setdefault((r['row'], r['W']), []).append(r)
    X = {'h1': {}, 'h9': {}, 'pooled_spread': {}, 'witness_cells': {}}
    for w in (1, 2, 4, 8, 16):
        e = {}
        for row in ('J-A-d1', 'JOLT-T', 'JOLT56-T'):
            rs = cells.get((row, w), [])
            early, late = [], []
            for r in rs:
                c = A.csv_cols(r)
                early.append(D.wmean(c['wall_ns'], (0, 100)) / 1e6)
                late.append(D.wmean(c['wall_ns'], (100, 500)) / 1e6)
            e[row] = {'early_0_100': A.stats(early), 'late_100_500': A.stats(late)}
        e['ratio_v530_early'] = e['J-A-d1']['early_0_100']['median'] / e['JOLT-T']['early_0_100']['median']
        e['ratio_v530_late'] = e['J-A-d1']['late_100_500']['median'] / e['JOLT-T']['late_100_500']['median']
        e['ratio_v560_early'] = e['J-A-d1']['early_0_100']['median'] / e['JOLT56-T']['early_0_100']['median']
        e['ratio_v560_late'] = e['J-A-d1']['late_100_500']['median'] / e['JOLT56-T']['late_100_500']['median']
        X['h1'][w] = e
        # H9: time per manifold per step over [100, 500), where both counts are known.
        rs = cells[('J-A-d1', w)]
        man = statistics.median([D.wmean(A.csv_cols(r)['manifolds'], (100, 500)) for r in rs])
        tb = e['J-A-d1']['late_100_500']['median']
        tj = e['JOLT-T']['late_100_500']['median']
        X['h9'][w] = {'boyko_manifolds_100_500': man, 'jolt_manifolds_100_500': JOLT_MANIFOLDS_100_500,
                      'boyko_us_per_manifold': 1000 * tb / man, 'jolt_us_per_manifold': 1000 * tj / JOLT_MANIFOLDS_100_500,
                      'ratio_per_manifold': (tb / man) / (tj / JOLT_MANIFOLDS_100_500)}
    # Pooled per-process spread: each process's deviation from its cell median, pooled per (engine, W).
    pool = {}
    for (row, w), rs in cells.items():
        if len(rs) < 3:
            continue
        eng = 'jolt' if row.startswith('JOLT') else 'boyko'
        med = statistics.median([r['mean_ms'] for r in rs])
        pool.setdefault((eng, w), []).extend(100 * (r['mean_ms'] / med - 1) for r in rs)
    for (eng, w), devs in sorted(pool.items()):
        X['pooled_spread'][f'{eng}@W{w}'] = {'n': len(devs), 'sd_pct': statistics.pstdev(devs),
                                            'min_pct': min(devs), 'max_pct': max(devs)}
    # Witness per (engine, W): r of the time deviation against the busy-CPU clock deviation.
    wit = {}
    for (row, w), rs in cells.items():
        if row == 'JOLT-P' or len(rs) < 3:
            continue
        med = statistics.median([r['mean_ms'] for r in rs])
        pv = [A.perf_of(r) for r in rs]
        if any(p[0] is None for p in pv):
            continue
        mb = statistics.mean([p[0] for p in pv])
        eng = 'jolt' if row.startswith('JOLT') else 'boyko'
        k = f'{eng}@W{w}'
        wit.setdefault(k, ([], []))
        for r, p in zip(rs, pv):
            wit[k][0].append(100 * (r['mean_ms'] / med - 1))
            wit[k][1].append(100 * (p[0] / mb - 1))
    for k, (xs, ys) in sorted(wit.items()):
        X['witness_cells'][k] = {'n': len(xs), 'r': A.pearson(xs, ys), 'clock_dev_sd_pct': statistics.pstdev(ys),
                                 'time_dev_sd_pct': statistics.pstdev(xs)}
    # Disparity-scene crossover, same interpolation as the lattice group.
    R = json.load(open(os.path.join(RAW, 'analysis.json'), encoding='utf-8'))
    est = R['broadphase']['estimates']
    pts = [(n, est[f'broadphase_disparity/all_pairs/{n}']['median_ns'], est[f'broadphase_disparity/grid/{n}']['median_ns'])
           for n in (1000, 10000)]
    (n0, a0, g0), (n1, a1, g1) = pts
    r0, r1 = g0 / a0, g1 / a1
    x0, x1, y0, y1 = math.log(n0), math.log(n1), math.log(r0), math.log(r1)
    X['disparity_crossover'] = {'ratios_grid_over_all_pairs': {n0: r0, n1: r1},
                                'n_star': math.exp(x0 + (0 - y0) * (x1 - x0) / (y1 - y0))}
    par = {n: {w: est[f'broadphase_parallel/w{w}/{n}']['median_ns'] for w in (1, 2, 4)} for n in (1000, 10000, 100000)}
    X['parallel_speedup_w4_over_w1'] = {n: v[1] / v[4] for n, v in par.items()}
    X['parallel_speedup_w2_over_w1'] = {n: v[1] / v[2] for n, v in par.items()}
    X['parallel_w4_vs_serial_o2_100k'] = est['broadphase_parallel/serial_o2/100000']['median_ns'] / par[100000][4]
    with open(os.path.join(RAW, 'analysis_extra.json'), 'w', encoding='utf-8') as f:
        json.dump(X, f, indent=1)
    print(json.dumps(X, indent=1)[:6000])


if __name__ == '__main__':
    main()
