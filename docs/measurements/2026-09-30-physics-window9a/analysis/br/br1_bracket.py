"""C4-BR: the all_pairs bracket (cut.md section 5 block 1). K 3 = 1 pass x 3 rounds, DIAGNOSTIC ONLY (ruling (b)).
Cells per binary per id; the ratios g4r8b/g4r7, g4rT/g4r7, g4rT/g4r8b for all_pairs (pre-registered read: the first
two; the third is task-requested) and tree (the in-binary control); in-binary all_pairs/tree. With one pass the pass
IS the pooled cell, so the flags i/s/r are the pooled flags; i AND s = RESOLVED (diagnostic), never CLAIMED.
Outputs: br1_bracket.txt, br1_bracket.json."""
import json
import statistics
import sys

sys.dont_write_bytecode = True
import lib_br as B

L = B.L
P = B.Out()
recs = B.recs_all()
procs, used, dropped = B.select(recs)
C = B.cells(used)
P('used processes: %d (per binary: %s); dropped slots %d' % (
    len(used), {b: sum(1 for p in used if p['binary'] == b) for b in B.BINARIES}, len(dropped)))
P('')
P('## 1. Cells, us per step (median of K 3 [min-max], IQR %, SE %, range %)')
for fam in B.FAMS:
    for arm in B.ARMS:
        for n in B.SIZES:
            i = B.ident(fam, arm, n)
            P('- %s' % i)
            for b in B.BINARIES:
                c = C[(b, i)]
                vals = sorted(p['est'][i] for p in used if p['binary'] == b)
                P('    %-5s %s  values %s' % (b, L.fcell(c, 3), ['%.3f' % v for v in vals]))
P('')
PAIRS = (('g4r7', 'g4r8b', 'pre-registered (cut 5.1: binary effect iff > 1 in the same window)'),
         ('g4r7', 'g4rT', 'pre-registered (row read)'),
         ('g4r8b', 'g4rT', 'task-requested, no pre-registered rule'))
RES = {'cells': {'%s %s' % k: v for k, v in C.items()}, 'cmp': {}}
P('## 2. Ratios B/A (flags i/s/r at K 3/3; sep = min-max separated; label by ruling (b))')
summary = {}
for arm in B.ARMS:
    P('### %s%s' % (arm, '' if arm == 'all_pairs' else ' (the in-binary control)'))
    for a, b, why in PAIRS:
        rats = []
        P('- %s/%s  [%s]' % (b, a, why))
        for fam in B.FAMS:
            for n in B.SIZES:
                i = B.ident(fam, arm, n)
                c = L.cmp_(C[(a, i)], C[(b, i)])
                rats.append(c['ratio'])
                RES['cmp']['%s %s/%s %s' % (arm, b, a, i)] = c
                P('    %-26s %.3f -> %.3f us: %.4f (%+.2f %%); %s%s; bars 2i %.2f / 2s %.2f / 2r %.2f %% -> %s' % (
                    '%s %d' % (fam, n), c['A'], c['B'], c['ratio'], 100 * (c['ratio'] - 1), L.yn(c),
                    ' sep' if c['minmax_sep'] else ' no-sep', 100 * c['bar_i'], 100 * c['bar_s'], 100 * c['bar_r'],
                    B.diag_label(c)))
        summary[(arm, b, a)] = rats
        P('    over the 4 (family, size): median %.4f, range %.4f-%.4f' % (
            statistics.median(rats), min(rats), max(rats)))
P('')
P('## 3. In-binary all_pairs/tree (A = tree, B = all_pairs), per binary; window 8b read 144 at 1.1294 / 1.1003, '
  'window 7 at 1.0076 / 0.9929 (uniform / disparity)')
for fam in B.FAMS:
    for n in B.SIZES:
        parts = []
        for b in B.BINARIES:
            c = L.cmp_(C[(b, B.ident(fam, 'tree', n))], C[(b, B.ident(fam, 'all_pairs', n))])
            RES['cmp']['inbin %s %s %d' % (b, fam, n)] = c
            parts.append('%s %.4f %s%s' % (b, c['ratio'], L.yn(c), ' sep' if c['minmax_sep'] else ''))
        P('- %s %d: %s' % (fam, n, '; '.join(parts)))
P('')
P('## 4. Diagnostic summary')
for arm in B.ARMS:
    for a, b, why in PAIRS:
        n_res = sum(1 for fam in B.FAMS for n in B.SIZES
                    if B.diag_label(RES['cmp']['%s %s/%s %s' % (arm, b, a, B.ident(fam, arm, n))]).startswith('RES'))
        n_up = sum(1 for fam in B.FAMS for n in B.SIZES
                   if RES['cmp']['%s %s/%s %s' % (arm, b, a, B.ident(fam, arm, n))]['ratio'] > 1)
        n_sep = sum(1 for fam in B.FAMS for n in B.SIZES
                    if RES['cmp']['%s %s/%s %s' % (arm, b, a, B.ident(fam, arm, n))]['minmax_sep'])
        rats = summary[(arm, b, a)]
        P('- %-9s %s/%s: median %.4f [%.4f-%.4f]; > 1 at %d of 4; RESOLVED (i AND s) at %d of 4; min-max separated at '
          '%d of 4' % (arm, b, a, statistics.median(rats), min(rats), max(rats), n_up, n_res, n_sep))
json.dump(RES, open(B.HERE + '/br1_bracket.json', 'w'), indent=1)
B.Out.save(P, 'br1_bracket.txt')
