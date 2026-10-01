"""C4-BR, POST HOC only (no claim, no pre-registered rule): (1) the same exes across windows (g4r7: window 7 wave 2's
reduction; g4r8b: window 8b's F3-G4 cells), and the decomposition of window 8b's cross-window 8b/7 shift into the
same-window binary term and the window terms; (2) per-round paired ratios (the exes ran adjacent); (3) the all_pairs
shift net of the tree control; (4) the static placement of all_pairs_into in the three exes (M9's place.py, read-only,
on the bin/ byte copies; the 16191fda object listing with relocations wildcarded). Output: br2_posthoc.txt."""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
import lib_br as B

L = B.L
P = B.Out()
recs = B.recs_all()
procs, used, dropped = B.select(recs)
C = B.cells(used)
SP = os.path.dirname(B.W9A)
W7 = json.load(open('D:/wt/merge/docs/measurements/2026-09-25-physics-window7/wave2/analyst/reduction.json'))['cells']
G8 = json.load(open(os.path.join(SP, 'win8b', 'analysis', 'f3', 'g4.json')))['cells']


def w7(i):
    fam, arm, n = i[len('bp_g4_'):].split('/')
    c = W7['Q3 %s %s/%s' % (fam, arm, n)]
    return {k: (c[k] * 1e3 if k in ('median', 'min', 'max') else c[k]) for k in ('K', 'median', 'min', 'max', 'r', 'i', 's')}


def g8(i):
    return G8[i]


P('## 1. The same exe in two windows (cross-window: no claim; flags i/s/r printed for scale only)')
for b, name, ref in (('g4r7', 'window 7 wave 2 (Q3)', w7), ('g4r8b', 'window 8b (F3-G4)', g8)):
    P('### %s: 9a / %s' % (b, name))
    rr = {}
    for arm in B.ARMS:
        rr[arm] = []
        for fam in B.FAMS:
            for n in B.SIZES:
                i = B.ident(fam, arm, n)
                a = ref(i)
                c = L.cmp_(a, C[(b, i)])
                rr[arm].append(c['ratio'])
                P('- %-30s %.3f [%.3f-%.3f] -> %.3f [%.3f-%.3f]: %.4f (%+.2f %%) %s%s' % (
                    i, a['median'], a['min'], a['max'], c['B'], C[(b, i)]['min'], C[(b, i)]['max'], c['ratio'],
                    100 * (c['ratio'] - 1), L.yn(c), ' sep' if c['minmax_sep'] else ''))
        P('  %s: median %.4f, range %.4f-%.4f' % (arm, statistics.median(rr[arm]), min(rr[arm]), max(rr[arm])))
P('')
P('## 2. Decomposition of window 8b\'s cross-window shift on the 4 BR ids (log-additive: X87 = window term of g4r8b')
P('##    (8b / 9a) x binary term in 9a (g4r8b / g4r7) x window term of g4r7 (9a / window 7))')
for arm in B.ARMS:
    rows = []
    for fam in B.FAMS:
        for n in B.SIZES:
            i = B.ident(fam, arm, n)
            x87 = g8(i)['median'] / w7(i)['median']
            win8 = g8(i)['median'] / C[('g4r8b', i)]['median']
            binr = C[('g4r8b', i)]['median'] / C[('g4r7', i)]['median']
            win7 = C[('g4r7', i)]['median'] / w7(i)['median']
            share = math.log(binr) / math.log(x87) if abs(math.log(x87)) > 1e-9 else float('nan')
            rows.append((x87, binr, win8, win7, share))
            P('- %-30s 8b/7 %.4f = window(g4r8b, 8b/9a) %.4f x binary(9a, g4r8b/g4r7) %.4f x window(g4r7, 9a/7) %.4f '
              '(check %.4f); binary share of the log shift %.1f %%' % (
                  i, x87, win8, binr, win7, win8 * binr * win7, 100 * share))
    P('  %s medians: 8b/7 %.4f; binary %.4f; window g4r8b %.4f; window g4r7 %.4f' % (
        arm, *(statistics.median(r[k] for r in rows) for k in range(4))))
P('')
P('## 3. Per-round paired ratios (adjacent processes of one round; min / median / max over the 3 rounds)')
by = {(p['binary'], p['round']): p for p in used}
for arm in B.ARMS:
    for a, b in (('g4r7', 'g4r8b'), ('g4r7', 'g4rT'), ('g4r8b', 'g4rT')):
        parts = []
        for fam in B.FAMS:
            for n in B.SIZES:
                i = B.ident(fam, arm, n)
                rs = sorted(by[(b, k)]['est'][i] / by[(a, k)]['est'][i] for k in range(3))
                parts.append('%s %d %.4f/%.4f/%.4f' % (fam[0], n, rs[0], rs[1], rs[2]))
        P('- %-9s %s/%s: %s' % (arm, b, a, '; '.join(parts)))
P('')
P('## 4. The all_pairs shift net of the tree control (ratio_all_pairs / ratio_tree, cell medians)')
for a, b in (('g4r7', 'g4r8b'), ('g4r7', 'g4rT')):
    xs = []
    for fam in B.FAMS:
        for n in B.SIZES:
            ap = C[(b, B.ident(fam, 'all_pairs', n))]['median'] / C[(a, B.ident(fam, 'all_pairs', n))]['median']
            tr = C[(b, B.ident(fam, 'tree', n))]['median'] / C[(a, B.ident(fam, 'tree', n))]['median']
            xs.append(ap / tr)
    P('- %s/%s: %s; median %.4f' % (b, a, ' '.join('%.4f' % x for x in xs), statistics.median(xs)))
P('')
P('## 5. Per pair test (all_pairs = n(n-1)/2 sphere tests; uniform rows = n, disparity rows from the receipts)')
rows_of = {('uniform', 144): 144, ('uniform', 256): 256, ('disparity', 144): 148, ('disparity', 256): 260}
for fam in B.FAMS:
    for n in B.SIZES:
        m = rows_of[(fam, n)]
        t = m * (m - 1) / 2
        P('- %s %d (rows %d, %d tests): %s ns per test' % (fam, n, m, t, ', '.join(
            '%s %.4f' % (b, 1e3 * C[(b, B.ident(fam, 'all_pairs', n))]['median'] / t) for b in B.BINARIES)))
P('')
P('## 6. Static placement of all_pairs_into (M9 place.py over the bin/ byte copies; loop = object offsets 0xe0..0x192)')
sys.path.insert(0, os.path.join(SP, 'tree-c4', 'm9'))
import place  # noqa: E402
fb, rel, ins = place.parse_dis(os.path.join(SP, 'tree-c4', 'm9', 'apinto_16191fda.dis'))
for b in B.BINARIES:
    exe = os.path.join(B.W9A, L.BINS[b]['exe'])
    hits, base = place.find(exe, fb, rel)
    if len(hits) != 1:
        P('- %s: %d hits (not unique): %s' % (b, len(hits), [hex(h) for h in hits]))
        continue
    rva = hits[0]
    lo, hi = 0xe0, 0x192
    P('- %-5s all_pairs_into at RVA %s (mod 64 = %d); loop head mod 64 = %d; loop spans %d x 64-B lines, %d x 32-B '
      'blocks; jcc crossing / ending on a 32-B boundary: %d' % (
          b, hex(rva), rva % 64, (rva + lo) % 64, (rva + hi - 1) // 64 - (rva + lo) // 64 + 1,
          (rva + hi - 1) // 32 - (rva + lo) // 32 + 1,
          sum(1 for j in place.jcc_report(rva, [x for x in ins if lo <= x[0] < hi]) if j['crosses32'] or j['ends_on32'])))
B.Out.save(P, 'br2_posthoc.txt')
