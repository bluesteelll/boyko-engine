#!/usr/bin/env python3
"""Dynamic parity scenes, C2 gate 3: per-scene WORK receipts (counts, never times) for ruling 21(b).

usage: work_receipts.py OURS_GATES_OUTDIR [--json OUT]
Reads, per program P, OUTDIR/P/armed_w1.csv (our runner, --arm-profiler, W1) and OUTDIR/P/san1.out
(the --sanity run's SUMMARY) and prints the means per step over P's metric window: candidate pairs,
manifolds, contact points, solver slots (wide + narrow), colours, wide colours, colour waves
(wide-colour spans), the narrowphase classes, and the static-pair receipts of ruling 22 (candidate
pairs with a static endpoint, static-static pairs, their manifolds). The armed CSV's columns are the
runner's recounted structure (`step_shape`), so these are the engine's own counts. No timing column
is read.
"""
import csv
import json
import os
import sys

WINDOW = {'none': (0, 100), 'kick': (200, 800), 'shoot': (200, 800), 'slide': (0, 500)}
COLS = ['pairs', 'manifolds', 'phys_np_points', 'phys_slots_wide', 'phys_slots_narrow', 'colors', 'wide_colors',
        'waves', 'phys_np_full', 'phys_np_reused', 'phys_np_sep_hits']


def main():
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    root = sys.argv[1]
    out = {}
    for p in ('none', 'kick', 'shoot', 'slide'):
        path = os.path.join(root, p, 'armed_w1.csv')
        if not os.path.exists(path):
            continue
        a, b = WINDOW[p]
        with open(path, newline='') as f:
            rows = [r for r in csv.DictReader(f) if a <= int(r['step']) < b]
        n = len(rows)
        mean = {c: sum(float(r[c]) for r in rows) / n for c in COLS}
        mean['slots'] = mean.pop('phys_slots_wide') + mean.pop('phys_slots_narrow')
        mean['points'] = mean.pop('phys_np_points')
        summary = None
        san = os.path.join(root, p, 'san1.out')
        if os.path.exists(san):
            for ln in open(san, encoding='utf-8', errors='replace'):
                if ln.startswith('SUMMARY '):
                    summary = json.loads(ln[8:])
        statics = None
        if summary and summary.get('dyn', {}).get('sanity'):
            s = summary['dyn']['sanity']['statics']
            statics = {k: v / n for k, v in s.items()}
        out[p] = {'window': [a, b], 'steps': n, 'per_step': mean, 'statics_per_step': statics}
        print('%-6s [%d,%d) pairs %.0f manifolds %.0f points %.0f slots %.0f colors %.1f wide %.1f waves %.1f'
              % (p, a, b, mean['pairs'], mean['manifolds'], mean['points'], mean['slots'], mean['colors'],
                 mean['wide_colors'], mean['waves']))
        if statics:
            print('       statics: pairs_static %.0f pairs_static_static %.1f manifolds_static %.1f '
                  'manifolds_static_static %.1f points_static %.1f'
                  % (statics['pairs_static'], statics['pairs_static_static'], statics['manifolds_static'],
                     statics['manifolds_static_static'], statics['points_static']))
    if len(sys.argv) > 3 and sys.argv[2] == '--json':
        with open(sys.argv[3], 'w') as f:
            json.dump(out, f, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
