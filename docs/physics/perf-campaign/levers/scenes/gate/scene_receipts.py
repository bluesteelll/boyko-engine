#!/usr/bin/env python3
"""Dynamic parity scenes: the per-scene work and static-pair receipts of all three engines (counts only).

usage: scene_receipts.py --ours DIR[,DIR...] --jolt DIR --rapier PINS.json [--json OUT]
  --ours    ours_gates.sh output roots (P/armed_w1.csv and P/san1.out under each)
  --jolt    jolt_gates.sh output root (P/rcpt1/receipt_discrete_th1.csv, P/san1/statics_discrete_th1.csv)
  --rapier  rapier_gates.py's rapier_pins.json (per row and program: rows, Np, Nm, statics)
Means per step over each scene's metric window. Ours: candidate pairs, manifolds, contact points (the
armed CSV's recount) and the static-pair receipts (--sanity); Jolt: the counting listener's manifolds
and points (-receipt) and the static-pair receipts (-sanity: the broadphase predicate recomputed per
active-static pair; the listener's manifolds with a static endpoint); Rapier: solver rows, points and
manifolds (--receipt) and the static-pair receipts (--sanity: contact_pairs with a fixed endpoint).
The engines count different things under one name (a Jolt manifold is reduced, a Rapier "pair" is a
broad-phase pair); the record states each definition. No timing column is read.
"""
import argparse
import csv
import json
import os
import sys

WINDOW = {'none': (0, 100), 'kick': (200, 800), 'shoot': (200, 800), 'slide': (0, 500)}


def mean_rows(path, cols, a, b, key='step'):
    with open(path, newline='') as f:
        rd = csv.DictReader((ln.replace(', ', ',') for ln in f))
        rows = [r for r in rd if a <= int(r[key]) < b]
    n = len(rows)
    return {c: (sum(float(r[c]) for r in rows) / n if n else None) for c in cols}, n


def ours(roots):
    out = {}
    for root in roots:
        for p in WINDOW:
            arm = os.path.join(root, p, 'armed_w1.csv')
            if not os.path.exists(arm):
                continue
            a, b = WINDOW[p]
            m, n = mean_rows(arm, ['pairs', 'manifolds', 'phys_np_points'], a, b)
            s = None
            for ln in open(os.path.join(root, p, 'san1.out'), encoding='utf-8', errors='replace'):
                if ln.startswith('SUMMARY '):
                    st = json.loads(ln[8:])['dyn']['sanity']['statics']
                    s = {k: v / n for k, v in st.items()}
            out[p] = {'steps': n, 'pairs': m['pairs'], 'manifolds': m['manifolds'], 'points': m['phys_np_points'],
                      'statics': s}
    return out


def jolt(root):
    out = {}
    for p in WINDOW:
        rc = os.path.join(root, p, 'rcpt1', 'receipt_discrete_th1.csv')
        st = os.path.join(root, p, 'san1', 'statics_discrete_th1.csv')
        if not os.path.exists(rc):
            continue
        a, b = WINDOW[p]
        m, n = mean_rows(rc, ['Manifolds', 'Points'], a, b, key='Frame')
        s, _ = mean_rows(st, ['pairs_static', 'pairs_static_static', 'manifolds', 'manifolds_static',
                              'manifolds_static_static', 'points_static'], a, b, key='frame')
        out[p] = {'steps': n, 'manifolds': m['Manifolds'], 'points': m['Points'], 'statics': s}
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--ours')
    ap.add_argument('--jolt')
    ap.add_argument('--rapier')
    ap.add_argument('--json')
    a = ap.parse_args()
    res = {}
    if a.ours:
        res['ours'] = ours(a.ours.split(','))
    if a.jolt:
        res['jolt56'] = jolt(a.jolt)
    if a.rapier:
        pins = json.load(open(a.rapier))
        res['rapier'] = {}
        for rid, progs in pins.items():
            for p, v in progs.items():
                st = v.get('statics')
                n = WINDOW[p][1] - WINDOW[p][0]
                res['rapier'].setdefault(p, {})[rid] = {'rows': v.get('rows'), 'points': v.get('Np'), 'manifolds': v.get('Nm'),
                                                         'statics': {k: x / n for k, x in st.items()} if st else None}
    for eng, scenes in res.items():
        for p, v in sorted(scenes.items()):
            if eng == 'rapier':
                for rid, x in sorted(v.items()):
                    s = x['statics'] or {}
                    print('%-7s %-6s %-24s rows %s points %s manifolds %s | pairs_static %.0f static-static %.1f manifolds_static %.1f'
                          % (eng, p, rid, x['rows'], x['points'], x['manifolds'], s.get('pairs_static', float('nan')),
                             s.get('pairs_static_static', float('nan')), s.get('manifolds_static', float('nan'))))
            else:
                s = v['statics'] or {}
                print('%-7s %-6s pairs %s manifolds %.0f points %.0f | pairs_static %.0f static-static %.1f manifolds_static %.1f static-static manifolds %.1f'
                      % (eng, p, ('%.0f' % v['pairs']) if v.get('pairs') is not None else '-', v['manifolds'], v['points'],
                         s.get('pairs_static', float('nan')), s.get('pairs_static_static', float('nan')),
                         s.get('manifolds_static', float('nan')), s.get('manifolds_static_static', float('nan'))))
    if a.json:
        with open(a.json, 'w') as f:
            json.dump(res, f, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
