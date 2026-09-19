"""Render the report's tables from raw/window/analysis.json and analysis_extra.json (pure reading)."""
import json, os, sys
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
R = json.load(open(os.path.join(HERE, 'raw', 'window', 'analysis.json'), encoding='utf-8'))
X = json.load(open(os.path.join(HERE, 'raw', 'window', 'analysis_extra.json'), encoding='utf-8'))
L = []
P = L.append
ORDER = ['J-A-d1', 'J-A-a', 'J-C', 'J-B', 'J-P1', 'J-S0', 'L1-B', 'L1-A1', 'AA2-pre', 'SHIP', 'J-Son', 'R', 'R-ref',
         'R-S', 'S16', 'JOLT-T', 'JOLT56-T', 'JOLT-NPC', 'JOLT-SLP', 'JOLT-P']
NAME = {'J-A-d1': 'J-A disarmed', 'J-A-a': 'J-A armed', 'J-C': 'J-C canary', 'L1-B': 'L1-B (tip, J-S0 disarmed)',
        'L1-A1': 'L1-A (pre-L1, J-S0 disarmed)', 'JOLT-T': 'Jolt v5.3.0', 'JOLT56-T': 'Jolt v5.6.0',
        'JOLT-NPC': 'Jolt -no_pair_cache', 'JOLT-SLP': 'Jolt -allow_sleep', 'JOLT-P': 'Jolt Release -p (profiled)'}
P('| row | W | K | median ms | min–max ms | range % | IQR % | SD % |')
P('|---|---|---|---|---|---|---|---|')
for row in ORDER:
    for w in (1, 2, 4, 8, 16):
        s = R['cells'].get(f'{row}@W{w}')
        if not s:
            continue
        dec = 4 if s['median'] >= 1 else 5
        P(f'| {NAME.get(row, row)} | {w} | {s["n"]} | {s["median"]:.{dec}f} | {s["min"]:.{dec}f}–{s["max"]:.{dec}f} | '
          f'{s["range_pct"]:.2f} | {s["iqr_pct"]:.2f} | {s["sd_pct"]:.2f} |')
P('')
P('| comparison (B against A) | B/A | effect % | 2× range bar | 2× IQR bar | 2× SE bar | claimed (range / IQR / SE) |')
P('|---|---|---|---|---|---|---|')
for k, v in R['comparisons'].items():
    yn = lambda b: 'yes' if b else 'no'
    P(f'| {k} | {v["ratio"]:.4f} | {v["effect_pct"]:+.2f} | {v["bar_range_pct"]:.2f} | {v["bar_iqr_pct"]:.2f} | '
      f'{v["bar_se_pct"]:.2f} | {yn(v["claim_range"])} / {yn(v["claim_iqr"])} / {yn(v["claim_se"])} |')
P('')
P('| W | boyko [0,100) | boyko [100,500) | v5.3.0 [0,100) | v5.3.0 [100,500) | v5.6.0 [0,100) | v5.6.0 [100,500) | ratio v5.3.0 early / late | ratio v5.6.0 early / late |')
P('|---|---|---|---|---|---|---|---|---|')
for w, e in X['h1'].items():
    f = lambda r, k: e[r][k]['median']
    P(f'| {w} | {f("J-A-d1","early_0_100"):.3f} | {f("J-A-d1","late_100_500"):.3f} | {f("JOLT-T","early_0_100"):.3f} | '
      f'{f("JOLT-T","late_100_500"):.3f} | {f("JOLT56-T","early_0_100"):.3f} | {f("JOLT56-T","late_100_500"):.3f} | '
      f'{e["ratio_v530_early"]:.3f} / {e["ratio_v530_late"]:.3f} | {e["ratio_v560_early"]:.3f} / {e["ratio_v560_late"]:.3f} |')
P('')
P('| W | boyko µs per manifold per step | Jolt v5.3.0 µs per manifold per step | ratio |')
P('|---|---|---|---|')
for w, e in X['h9'].items():
    P(f'| {w} | {e["boyko_us_per_manifold"]:.3f} | {e["jolt_us_per_manifold"]:.3f} | {e["ratio_per_manifold"]:.2f} |')
P('')
for w, c in R['jolt_p'].items():
    P(f'Jolt -p, W={w}: K={c["K"]}, frames {c["frames_used"]} per process; profiled T {c["profiled_T_ms"]["median"]:.3f} ms; '
      f'Update span {c["update_ms"]["median"]:.3f} ms; summed thread time in scopes {c["total_work_ms"]["median"]:.3f} ms')
    P('')
    P('| stage | wall coverage % of the step | thread-time share % (min–max over K) | thread-time ms |')
    P('|---|---|---|---|')
    for k, v in sorted(c['stages'].items(), key=lambda kv: -kv[1]['work_share_pct']):
        if v['work_share_pct'] < 0.04 and v['wall_cover_pct'] < 0.1:
            continue
        P(f'| {k} | {v["wall_cover_pct"]:.2f} | {v["work_share_pct"]:.2f} ({v["work_share_min"]:.2f}–{v["work_share_max"]:.2f}) | {v["work_ms"]:.3f} |')
    P('')
b = R['broadphase']
P('| benchmark | median | 95 % CI of the median |')
P('|---|---|---|')
def fmt(ns):
    return f'{ns/1e6:.3f} ms' if ns >= 1e6 else f'{ns/1e3:.2f} µs'
for k, v in sorted(b['estimates'].items()):
    P(f'| {k} | {fmt(v["median_ns"])} | {fmt(v["median_lo"])} – {fmt(v["median_hi"])} |')
open(os.path.join(HERE, 'raw', 'window', 'tables.md'), 'w', encoding='utf-8').write('\n'.join(L) + '\n')
print('\n'.join(L))
