"""Reduce G4 (raw/g4/runs.jsonl + the criterion estimates) to per-cell tables and the recipe's section 1.2
stop-rule inputs. Pure reading. The statistic (recipe 1.1): per cell the MEDIAN over the K processes of
criterion's median.point_estimate (ns), spread = min-max as % of the median, the median's SE
(1.2533 SD / sqrt K) as % of the median; a comparison B against A is claimed iff
|median_B / median_A - 1| > 2 * hypot(spread_A, spread_B), under both spreads (range, SE).
Process selection = window 3's rule: the original if clean, else its re-run if clean, else excluded.

Usage: python -B reduce_g4.py [raw_g4_dir]  -> writes <raw_g4_dir>/g4_reduction.json and prints the tables.
"""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W4 = os.path.dirname(HERE)
RAW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(W4, 'raw', 'g4')
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
from analyze_win3 import compare, stats  # noqa: E402

TREE_BRUTE_MAX_ROWS = 64
MS = 1e6


def load():
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
    by_slot = {}
    for r in recs:
        by_slot.setdefault((r['block'], r['seq']), []).append(r)
    used, excluded = [], []
    for slot, rs in sorted(by_slot.items()):
        chosen = None
        for r in rs:  # the original first (attempt 'original'), then the re-run
            if 'aborted' in r:
                continue
            if r.get('valid') and not r.get('contaminated'):
                chosen = r
                break
        if chosen is None:
            excluded.append([{k: r.get(k) for k in ('block', 'k', 'seq', 'attempt', 'filter_root', 'contaminated', 'valid', 'exit')} for r in rs])
        else:
            used.append(chosen)
    return recs, used, excluded


def cells_of(used):
    per_cell = {}
    for r in used:
        for e in r['estimates']:
            per_cell.setdefault(e['cell'], []).append({'k': r['k'], 'attempt': r['attempt'], 'median_ns': e['median_ns'],
                                                      'seq': r['seq'], 'block': r['block']})
    out = {}
    for cell, xs in sorted(per_cell.items()):
        s = stats([x['median_ns'] for x in xs])
        s['k_values'] = sorted(xs, key=lambda x: x['k'])
        out[cell] = s
    return out


def med(cells, cell):
    return cells[cell]['median'] if cell in cells else None


def main():
    recs, used, excluded = load()
    cells = cells_of(used)
    rules = []

    def rule(name, cell_desc, value_ns, limit_ns, source, extra=None):
        fired = value_ns is not None and value_ns > limit_ns
        rules.append({'rule': name, 'cell': cell_desc, 'value_ms': None if value_ns is None else value_ns / MS,
                      'limit_ms': limit_ns / MS, 'fired': fired, 'source': source, 'inputs': extra or {}})

    # J snapshot
    rule('J snapshot', 'bp_g4_scene/tree/j100', med(cells, 'bp_g4_scene/tree/j100'), 0.30 * MS, 'G4: > 0.30 ms')
    # Tree vs Grid (claimed slower) at n > TREE_BRUTE_MAX_ROWS, W=1
    tvg = []
    for fam in ('bp_g4_uniform', 'bp_g4_disparity', 'bp_g4_scene'):
        for n in ('128', '256', '1000', '1240', '10000', '100000', 'j100'):
            t, g = cells.get(f'{fam}/tree/{n}'), cells.get(f'{fam}/grid_w1/{n}')
            if t and g:
                c = compare(g, t)  # effect of tree against grid_w1
                tvg.append({'family': fam, 'n': n, 'tree_ms': t['median'] / MS, 'grid_w1_ms': g['median'] / MS,
                            'ratio_tree_over_grid': c['ratio'], 'bar_range_pct': c['bar_range_pct'], 'bar_se_pct': c['bar_se_pct'],
                            'claimed_slower_range': c['effect_pct'] > 0 and c['claim_range'],
                            'claimed_slower_se': c['effect_pct'] > 0 and c['claim_se']})
    fired = [x for x in tvg if x['claimed_slower_range'] or x['claimed_slower_se']]
    rules.append({'rule': 'Tree vs Grid', 'cell': 'bp_g4_*/tree/n vs grid_w1/n, n > 64', 'fired': bool(fired),
                  'source': 'G4: tree slower than grid_w1 (claimed) at W=1', 'inputs': tvg})
    # maintenance rules: (cell - stable) against 2x the D3.5 upper values, per m
    lim = {'stable': {'1240': 58e3, '10000': 0.46 * MS, '100000': 4.6 * MS},
           'shift_translation': {'1240': 62e3, '10000': 0.5 * MS, '100000': 5.0 * MS},
           'eviction_filter': {'1240': 48e3, '10000': 0.38 * MS, '100000': 3.8 * MS},
           'compaction': {'1240': 50e3, '10000': 0.40 * MS, '100000': 6.0 * MS},
           'admission_of_64': {'1240': 120e3, '10000': 0.80 * MS, '100000': 9.8 * MS},
           'admission_from_empty': {'1240': 0.46 * MS, '10000': 4.8 * MS, '100000': 60 * MS},
           'high_jumper': {'1240': 62e3, '10000': 0.5 * MS, '100000': 5.0 * MS}}
    src = {'stable': 'D3.5 verify + copy: 2 x (5+24) us / 2 x (40+190) us / 2 x (0.4+1.9) ms',
           'shift_translation': 'D3.5 translation: 2 x 31 us / 0.25 ms / 2.5 ms (minus stable)',
           'eviction_filter': 'D3.5 eviction filter: 2 x 24 us / 0.19 ms / 1.9 ms (minus stable)',
           'compaction': 'D3.5 compaction: 2 x 25 us / 0.20 ms / 3 ms (minus eviction_filter)',
           'admission_of_64': 'D3.5 admission of 64: 2 x 60 us / 0.40 ms / 4.9 ms (minus stable)',
           'admission_from_empty': 'D3.5 admission from empty: 2 x 0.23 ms / 2.4 ms / 30 ms (minus stable)',
           'high_jumper': 'ruling W1: the translation row bound 2 x 31 us / 0.25 ms / 2.5 ms (minus stable)'}
    for arm in ('stable', 'shift_translation', 'eviction_filter', 'compaction', 'admission_of_64', 'admission_from_empty', 'high_jumper'):
        for m in ('1240', '10000', '100000'):
            v = med(cells, f'bp_g4_maintenance/{arm}/{m}')
            base_name = 'eviction_filter' if arm == 'compaction' else 'stable'
            base = 0 if arm == 'stable' else med(cells, f'bp_g4_maintenance/{base_name}/{m}')
            diff = None if v is None or base is None else v - base
            rule(arm, f'bp_g4_maintenance/{arm}/{m}' + ('' if arm == 'stable' else f' - {base_name}/{m}'), diff, lim[arm][m], src[arm],
                 {'cell_ms': None if v is None else v / MS, 'base_ms': None if base is None else base / MS})
    # churn claim inputs (not stop rules): tree vs allpairs per arm x sleeping; tree(arm) - tree(stable)
    churn = []
    for sl in ('sleeping_off', 'sleeping_on'):
        for arm in ('stable', 'swap_churn', 'archetype_shift', 'first_archetype_spawn', 'burst_despawn', 'burst_migrate'):
            a, t = cells.get(f'row_identity_churn/{arm}/{sl}_allpairs'), cells.get(f'row_identity_churn/{arm}/{sl}_tree')
            ts = cells.get(f'row_identity_churn/stable/{sl}_tree')
            c = compare(a, t) if a and t else None
            churn.append({'arm': arm, 'sleeping': sl, 'allpairs_ms': a['median'] / MS if a else None, 'tree_ms': t['median'] / MS if t else None,
                          'ratio_tree_over_allpairs': c['ratio'] if c else None,
                          'claim_faster_range': (c['effect_pct'] < 0 and c['claim_range']) if c else None,
                          'claim_faster_se': (c['effect_pct'] < 0 and c['claim_se']) if c else None,
                          'bar_range_pct': c['bar_range_pct'] if c else None, 'bar_se_pct': c['bar_se_pct'] if c else None,
                          'tree_minus_tree_stable_ms': (t['median'] - ts['median']) / MS if t and ts else None})
    out = {'raw': RAW, 'processes_total': len(recs), 'processes_used': len(used), 'slots_excluded': excluded,
           'cells': cells, 'stop_rules': rules, 'churn_inputs': churn,
           'statistic': 'median over K of criterion median.point_estimate; range_pct = (max-min)/median; se_med_pct = 1.2533*SD/sqrt(K)/median; claim iff |effect| > 2*hypot(spread_A, spread_B)'}
    json.dump(out, open(os.path.join(RAW, 'g4_reduction.json'), 'w', encoding='utf-8'), indent=1)
    print(f'processes {len(recs)} used {len(used)} excluded slots {len(excluded)}')
    print('| cell | K | median | min | max | range % | SE % | per-k (ms) |')
    print('|---|---|---|---|---|---|---|---|')
    for cell, s in cells.items():
        unit = 'ms' if s['median'] >= 1e5 else 'us'
        f = MS if unit == 'ms' else 1e3
        ks = ' '.join(f"k{x['k']}{'r' if x['attempt'] != 'original' else ''}={x['median_ns'] / f:.4g}" for x in s['k_values'])
        print(f"| {cell} | {s['n']} | {s['median'] / f:.4g} {unit} | {s['min'] / f:.4g} | {s['max'] / f:.4g} | {s['range_pct']:.2f} | {s['se_med_pct']:.2f} | {ks} |")
    print()
    print('| stop rule | cell | value ms | limit ms | fired |')
    print('|---|---|---|---|---|')
    for r in rules:
        if r['rule'] == 'Tree vs Grid':
            for x in r['inputs']:
                print(f"| Tree vs Grid | {x['family']}/{x['n']} | tree {x['tree_ms']:.4g} vs grid_w1 {x['grid_w1_ms']:.4g} (ratio {x['ratio_tree_over_grid']:.3f}) | bar range {x['bar_range_pct']:.1f} % / SE {x['bar_se_pct']:.1f} % | claimed slower: range {x['claimed_slower_range']} SE {x['claimed_slower_se']} |")
        else:
            v = 'n/a' if r['value_ms'] is None else f"{r['value_ms']:.4g}"
            print(f"| {r['rule']} | {r['cell']} | {v} | {r['limit_ms']:.4g} | {r['fired']} |")
    print()
    print('| churn arm | sleeping | allpairs ms | tree ms | ratio | claimed faster (range / SE) | tree - tree(stable) ms |')
    print('|---|---|---|---|---|---|---|')
    for x in churn:
        print(f"| {x['arm']} | {x['sleeping']} | {x['allpairs_ms']} | {x['tree_ms']} | {x['ratio_tree_over_allpairs']} | {x['claim_faster_range']} / {x['claim_faster_se']} | {x['tree_minus_tree_stable_ms']} |")
    any_fired = [r['rule'] for r in rules if r['fired']]
    print(f'\nSTOP RULES FIRED: {any_fired if any_fired else "none"}')


if __name__ == '__main__':
    main()
