"""Sensitivity of the five pre-registered verdicts: (a) IQR quantile method exclusive instead of window 8's inclusive;
(b) leave-one-process-out (drop any single process from either cell, every pass/pooled re-judged);
(c) pre-flight: unarmed pose.bin P vs T byte-compare and fixture sha, recomputed."""
import hashlib
import itertools
import json
import math
import os
import statistics
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vn_lib import (W8B, FIX, load_block, validate, dirty, wmean, summary_of)  # noqa: E402

recs = [r for r in load_block('S7-AB') if r.get('timed')]
sel = {}
info = {}
for r in recs:
    why, s, rows = validate(r)
    info[(r['pass'], r['round'], r['row'], r['binary'], r['W'], r['attempt'])] = (why, dirty(r), rows)
for k, (why, dw, rows) in info.items():
    if k[5] == 'original' and not why and not dw:
        sel[k[:5]] = rows
for k, (why, dw, rows) in info.items():
    if k[:5] not in sel and k[5] == 'rerun' and not why:
        sel[k[:5]] = rows


def vals(row, b, W, passes):
    return {(p, rd): wmean(sel[(p, rd, row, b, W)], 0, 500) / 1e6 for p in passes for rd in (0, 1, 2)}


def st(xs, method):
    xs = sorted(xs)
    m = statistics.median(xs)
    q = statistics.quantiles(xs, n=4, method=method)
    return {'K': len(xs), 'med': m, 'r': (xs[-1] - xs[0]) / m, 'i': (q[2] - q[0]) / m,
            's': 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) / m}


def judge(A, B, method='inclusive', drop=None):
    signs = set()
    allis = True
    allr = True
    for passes in ((0, 1, 2), (0,), (1,), (2,)):
        va = vals(*A, passes)
        vb = vals(*B, passes)
        if drop:
            side, key = drop
            (va if side == 'A' else vb).pop(key, None)
        a = st(list(va.values()), method)
        b = st(list(vb.values()), method)
        ratio = b['med'] / a['med']
        e = abs(ratio - 1)
        ok = lambda k: e > 2 * math.hypot(a[k], b[k])  # noqa: E731
        allis &= ok('i') and ok('s')
        allr &= ok('r')
        signs.add(ratio > 1)
    return allis and len(signs) == 1, allis and allr and len(signs) == 1, signs


CL = {'C1-JT': (('S7-JT', 's7t', 8), ('S7-JT', 's7t', 16)), 'C1-JA': (('S7-JA', 's7t', 8), ('S7-JA', 's7t', 16)),
      'C2-JT': (('S7-JT', 's7p', 8), ('S7-JT', 's7t', 8)), 'C2-JA': (('S7-JA', 's7p', 8), ('S7-JA', 's7t', 8)),
      'C3-JT': (('S7-JT', 's7p', 16), ('S7-JT', 's7t', 16))}
print('(a) quantile method sensitivity (claimed i&s every pass+pooled / STRONG / signs B>A):')
for method in ('inclusive', 'exclusive'):
    for n, (A, B) in CL.items():
        print(f'   {method:9s} {n}: {judge(A, B, method)}')
print('(b) leave-one-process-out: does any single dropped process change claimed / STRONG?')
for n, (A, B) in CL.items():
    base = judge(A, B)
    flips = []
    for side in ('A', 'B'):
        for key in itertools.product((0, 1, 2), (0, 1, 2)):
            res = judge(A, B, drop=(side, key))
            if res[:2] != base[:2]:
                flips.append((side, key, res))
    print(f'   {n}: base {base[:2]}; flips {flips}')

print('(c) pre-flight recomputed:')
g = os.path.join(W8B, 'gate', 's7')
for R in ('JT', 'JA'):
    for W in (1, 2, 4, 8, 16):
        ps = open(os.path.join(g, f'P-{R}-W{W}', 'pose.bin'), 'rb').read()
        ts = open(os.path.join(g, f'T-{R}-W{W}', 'pose.bin'), 'rb').read()
        sp = summary_of(open(os.path.join(g, f'P-{R}-W{W}', 'stdout.txt'), encoding='utf-8', errors='replace').read())
        stt = summary_of(open(os.path.join(g, f'T-{R}-W{W}', 'stdout.txt'), encoding='utf-8', errors='replace').read())
        fx = FIX[R + '500']
        print(f'   {R} W{W}: P==T bytes {ps == ts}; sha fixture {hashlib.sha256(ps).hexdigest() == fx["sha256"]}; '
              f'P {sp["pose_hash"]} {sp["expect_pose"]} void {sp["void_steps"]} W {sp["workers"]}; '
              f'T {stt["pose_hash"]} {stt["expect_pose"]} void {stt["void_steps"]} W {stt["workers"]}')
for R in ('JT', 'JA'):
    for W in (8, 16):
        bnd = 8 * 49404 + (W + 1) * 500
        for Bk in ('P', 'T'):
            s = summary_of(open(os.path.join(g, f'{Bk}-{R}-W{W}-armed', 'stdout.txt'), encoding='utf-8', errors='replace').read())
            w = s['w8s']
            ls = w['lanes_mean'] * w['waves']
            bnd = 8 * w['color_scopes'] + (W + 1) * w['setup_steps']
            print(f'   engagement {Bk}-{R}-W{W}: lanes_sum {ls:.0f} bound {bnd} within {ls <= bnd} '
                  f'scopes {w["color_scopes"]} tasks {w["color_tasks"]} pose {s["pose_hash"]} void {s["void_steps"]} steps {s["steps"]}')

print('(d) ladders under both quartile methods, [0,500) (seen = i&s every pass + pooled, rung slower):')
LAD = {'ladder8': [(('S7-ladder8-F' + f, 's7p', 8), ('S7-JT', 's7p', 8), float(f) * 0.060) for f in ('0.5', '1', '1.5', '2')],
       'ladder16': [(('S7-ladder16-F' + f, 's7p', 16), ('S7-JT', 's7p', 16), float(f) * 0.105) for f in ('0.5', '1', '1.5', '2')],
       'JA-ladder16': [(('S7-JA-ladder16-F1', 's7p', 16), ('S7-JA', 's7p', 16), 0.105)]}
for method in ('inclusive', 'exclusive'):
    for ln, rungs in LAD.items():
        seen = []
        for rung, ref, inj in rungs:
            cl, strong, signs = judge(ref, rung, method)
            seen.append((inj, cl and signs == {True}))
        R = next((seen[i][0] for i in range(len(seen)) if all(s for _, s in seen[i:])), None)
        print(f'   {method:9s} {ln}: seen {[(round(a, 4), b) for a, b in seen]} -> R = {R}')
