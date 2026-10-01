"""Window 9a RESUME (run 033740, 2026-10-01 03:37-05:49): synthesis-level census and a re-derivation of every
decisive figure the resume section quotes (results-analyst synthesis, 2026-10-01). Read-only on raw/, rows9a*.json,
bin/, wait_log.txt, WINDOW_DONE.

Statistics and per-process parsers are the verified group libraries, unchanged:
  ../c4ab/lib9a.py (cell / cmp_ / ruling1 / judge / clean_why / load_process), ../cgu/cgu_lib.py (run-tag-aware select),
  ../br/lib_br.py (criterion loader for the three BR exes), ../g4/lib_g4.py (criterion loader + kernel receipts).
The census, the recipe reading and the cross-run comparison are written here.
"""
import json
import math
import os
import re
import statistics
import sys
from collections import Counter, defaultdict

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
AN = os.path.dirname(HERE)
for sub in ('c4ab', 'cgu', 'br', 'g4'):
    sys.path.insert(0, os.path.join(AN, sub))
import lib9a as L  # noqa: E402
import cgu_lib as C  # noqa: E402
import lib_br as B  # noqa: E402
import lib_g4 as G  # noqa: E402

W9A = L.W9A
TAG = '033740'
lines = []


def out(s=''):
    lines.append(str(s))
    print(s)


recs = L.all_records()
res = [r for r in recs if r.get('run_tag') == TAG]
procrecs = [r for r in res if 'row' in r]
pcs = [r for r in res if r.get('passcell')]
pdone = [r for r in res if r.get('pass_done')]
out('# 1. Census of the resumed run (run_tag %s)' % TAG)
out('records %d: process records %d, passcell %d, pass_done %d, other %d' % (
    len(res), len(procrecs), len(pcs), len(pdone), len(res) - len(procrecs) - len(pcs) - len(pdone)))
blocks = ['C4-CGU', 'C4-BR', 'C4-G4', 'C4-G4-kd']
for b in blocks:
    pr = [r for r in procrecs if r['block'] == b]
    att = Counter(r['attempt'] for r in pr)
    closed = sorted({r['pass'] for r in pdone if r['block'] == b})
    dval = sum(1 for r in pr if r.get('valid'))
    dcont = sum(1 for r in pr if r.get('contaminated') and r.get('attempt') != 'warmup')
    depth = max([r.get('rerun_no', 0) for r in pr] or [0])
    t0 = min(r['start'] for r in pr)
    t1 = max(r['end'] for r in pr)
    out('- %-9s passes closed %s; process records %d (%s); driver-valid %d/%d; driver-unclean timed %d; '
        'deepest re-run %d; %s .. %s' % (b, closed, len(pr), dict(att), dval, len(pr), dcont, depth, t0[11:19], t1[11:19]))
timed = [r for r in procrecs if r.get('attempt') != 'warmup']
out('timed processes %d (originals %d, re-runs %d), warm-ups %d; window log counters say timed 134, rerun 23, warm 6' % (
    len(timed), sum(r['attempt'] == 'original' for r in timed), sum(r['attempt'] == 'rerun' for r in timed),
    sum(r['attempt'] == 'warmup' for r in procrecs)))

# sha pins, every record incl. warm-ups
bad = [r for r in procrecs if not L.sha_ok(r)]
byb = Counter((r['binary'], r['exe_sha256'][:8]) for r in procrecs)
out('sha256 = pin AND = SHA256SUMS on %d/%d process records; by binary %s' % (
    len(procrecs) - len(bad), len(procrecs), dict(byb)))
for k in ('tip', 'tipcgu1', 'g4r7', 'g4r8b', 'g4rT'):
    exe = os.path.join(W9A, L.BINS[k]['exe'])
    out('  file %s sha256 now %s pin %s' % (k, C.file_sha256(exe)[:16], L.BINS[k]['sha256_pin'][:16]))

# independent validity + slot rule per block (group loaders), K per pass-cell vs the driver
out('')
out('## independent validity, slots, K vs the driver')
_, cu, cd = C.select('C4-CGU', recs, TAG)
cgu_all = [L.load_process(r) for r in C.procs_of('C4-CGU', recs, TAG)]
out('C4-CGU: timed %d, independently invalid %d, unclean %d; slots used %d (re-runs %d), dropped %d' % (
    len(cgu_all), sum(1 for p in cgu_all if p['valid_why']), sum(1 for p in cgu_all if p['clean_why']),
    len(cu), sum(p['attempt'] == 'rerun' for p in cu), len(cd)))
bp, bu, bd = B.select(recs)
out('C4-BR : timed %d, invalid %d, unclean %d; used %d, dropped %d' % (
    len(bp), sum(1 for p in bp if p['valid_why']), sum(1 for p in bp if p['clean_why']), len(bu), len(bd)))
gp, gu, gd = G.select('C4-G4', recs)
kp, ku, kd_ = G.select('C4-G4-kd', recs)
for nm, pp, uu, dd in (('C4-G4', gp, gu, gd), ('C4-G4-kd', kp, ku, kd_)):
    out('%-8s: timed %d, invalid %d (%s), unclean %d; used %d, dropped %d' % (
        nm, len(pp), sum(1 for p in pp if p['valid_why']), [p['valid_why'] for p in pp if p['valid_why']][:2],
        sum(1 for p in pp if p['clean_why']), len(uu), len(dd)))
# K per pass-cell: mine vs the driver's passcell records
mine = Counter()
for p in cu:
    mine[('C4-CGU', p['pass'], p['row'], p['binary'], p['W'])] += 1
for p in bu:
    mine[('C4-BR', p['pass'], 'C4-BR', p['binary'], 0)] += 1
for p in gu:
    mine[('C4-G4', p['pass'], 'C4-G4', 'g4rT', 0)] += 1
for p in ku:
    mine[('C4-G4-kd', p['pass'], 'C4-G4-kd', 'g4rT', 0)] += 1
agree = sum(1 for r in pcs if mine[(r['block'], r['pass'], r['pc_row'], r['pc_binary'], r['pc_W'])] == r['k_clean'])
short = [r for r in pcs if r.get('short')]
out('pass-cells %d: my K == driver k_clean on %d; short (K < 3) %d; K values %s' % (
    len(pcs), agree, len(short), dict(Counter(r['k_clean'] for r in pcs))))

# receipts of used processes per block
out('')
out('## receipts of the used processes (median [min-max], %)')
for nm, uu in (('C4-CGU', cu), ('C4-BR', bu), ('C4-G4', gu), ('C4-G4-kd', ku)):
    def m(key, sub=None):
        xs = [((p['rec'].get(key) or {}).get('cpu_avg') if sub else p['rec'].get(key)) for p in uu]
        xs = [x for x in xs if x is not None]
        return '%.2f [%.2f-%.2f]' % (statistics.median(xs), min(xs), max(xs))
    out('- %-8s before %s, after %s, witness %s' % (nm, m('receipt_before', 1), m('receipt_after', 1),
                                                        m('others_busy_pct')))
unclean_top = Counter()
for r in timed:
    if r.get('contaminated'):
        t5 = r.get('others_top5') or []
        if t5:
            nm0 = t5[0][0] if isinstance(t5[0], (list, tuple)) else (t5[0].get('name') if isinstance(t5[0], dict) else str(t5[0]))
            unclean_top[str(nm0)] += 1
out('top other process on the %d unclean timed processes: %s' % (sum(r.get('contaminated', False) for r in timed),
                                                                  dict(unclean_top)))

# idle waits of the resume (wait_log.txt) and D: free
out('')
out('## idle waits (wait_log.txt, starts at or after 03:37)')
wl = open(os.path.join(W9A, 'wait_log.txt'), encoding='utf-8', errors='replace').read().splitlines()
cur, waits, dfree = None, [], []
for ln in wl:
    m = re.match(r'# wait_idle9a\.ps1 start (\S+)', ln)
    if m:
        cur = {'start': m.group(1), 'polls': 0, 'end': None}
        waits.append(cur)
        continue
    m = re.match(r'poll\s+\d+ (\S+) .*D_free_GB=([\d.]+)', ln)
    if m and cur is not None:
        cur['polls'] += 1
        if m.group(1) >= '2026-10-01T03:37':
            dfree.append(float(m.group(2)))
    m = re.match(r'# IDLE reached at (\S+) after (\d+) polls', ln)
    if m and cur is not None:
        cur['end'] = m.group(1)
rw = [w for w in waits if w['start'] >= '2026-10-01T03:37']
out('resume idle waits %d; polls per wait %s; reached %d/%d; D: free during the resume %.2f..%.2f GB' % (
    len(rw), [w['polls'] for w in rw], sum(1 for w in rw if w['end']), len(rw), min(dfree), max(dfree)))
wd = open(os.path.join(W9A, 'WINDOW_DONE'), encoding='utf-8').read().split('\n')
out('WINDOW_DONE: %s' % ' / '.join(x for x in wd if x.strip()))
wlog = open(os.path.join(W9A, 'raw', 'window_log.txt'), encoding='utf-8', errors='replace').read()
out('window_log end line: %s' % [x for x in wlog.splitlines() if 'WINDOW END' in x][-1][30:])
ab = [r for r in recs if r.get('block') == 'C4-BR' and r.get('run_tag') == '191354' and 'row' in r]
out('aborted C4-BR attempt (run 191354): %d process records, pass_done %s -> excluded' % (
    len(ab), any(r.get('pass_done') and r.get('block') == 'C4-BR' and r.get('run_tag') == '191354' for r in recs)))

# ------------------------------------------------------------------------------------------------------------------
out('')
out('# 2. C4-CGU re-derivation (A = tip cgu16, B = tipcgu1; B/A < 1 = cgu1 faster)')
CG = {}
for win in ('100..500', '0..500'):
    for W in C.WS:
        a = C.vals(cu, 'tip', W, win)
        b = C.vals(cu, 'tipcgu1', W, win)
        j = L.judge(a, b, 'LETTER')
        CG[(win, W)] = j
        p = j['pooled']
        out('- %s W%-2d A %.4f (n %d) B %.4f (n %d) B/A %.4f (%+.2f %%) bars i %.2f s %.2f r %.2f; pooled %s; '
            'p0 %s p1 %s p2 %s -> %s %s' % (win, W, p['A'], p['KA'], p['B'], p['KB'], p['ratio'],
                                            100 * (p['ratio'] - 1), 100 * p['bar_i'], 100 * p['bar_s'],
                                            100 * p['bar_r'], L.yn(p), L.yn(j['per'][0]), L.yn(j['per'][1]),
                                            L.yn(j['per'][2]), L.label(j), j['dir']))
w8 = CG[('100..500', 8)]
faster8 = w8['claimed'] and w8['pooled']['ratio'] < 1
slower_any = [W for W in C.WS if CG[('100..500', W)]['claimed'] and CG[('100..500', W)]['pooled']['ratio'] > 1]
out('CGU-PROFILE ([100,500)): cgu1 claimed faster at W8 = %s; claimed slower at W %s -> profile moves to cgu 1: %s' % (
    faster8, slower_any or 'none', 'YES' if faster8 and not slower_any else 'NO'))
# paired ratios (post hoc): same pass, same round, adjacent original pairs and the used slots
pr_all = []
by = defaultdict(dict)
for p in cu:
    by[(p['pass'], p['round'], p['W'])][p['binary']] = p['v']['100..500']
for k, d in by.items():
    if 'tip' in d and 'tipcgu1' in d:
        pr_all.append((k[2], d['tipcgu1'] / d['tip']))
gm = math.exp(statistics.fmean(math.log(x) for _, x in pr_all))
out('post hoc paired (same pass+round, used slots) n %d: geomean cgu1/cgu16 %.4f; cgu1 faster in %d' % (
    len(pr_all), gm, sum(1 for _, x in pr_all if x < 1)))
for W in C.WS:
    xs = [x for w, x in pr_all if w == W]
    out('  W%-2d n %d median %.4f, cgu1 faster %d' % (W, len(xs), statistics.median(xs), sum(x < 1 for x in xs)))

# cross-run (post hoc): this block's tip vs C4-AB's C4-JD#tip (run 191354), same exe and args
out('')
out('## post hoc cross-run: C4-CGU tip (033740) against C4-AB C4-JD#tip (191354), [100,500)')
_, au, _ = C.select('C4-AB', recs, '191354', row='C4-JD', binary='tip')
for W in C.WS:
    a = {}
    for p in au:
        if p['W'] == W:
            a.setdefault(p['pass'], []).append(p['v']['100..500'])
    bb = C.vals(cu, 'tip', W, '100..500')
    bc = C.vals(cu, 'tipcgu1', W, '100..500')
    j = L.judge(a, bb, 'LETTER')
    j2 = L.judge(a, bc, 'LETTER')
    out('- W%-2d AB n %s -> CGU tip: %.4f -> %.4f (%+.2f %%) pooled %s, %s; cgu1 against AB tip %+.2f %%' % (
        W, '/'.join(str(len(a.get(k, []))) for k in L.PASSES), j['pooled']['A'], j['pooled']['B'],
        100 * (j['pooled']['ratio'] - 1), L.yn(j['pooled']), L.label(j), 100 * (j2['pooled']['ratio'] - 1)))

# ------------------------------------------------------------------------------------------------------------------
out('')
out('# 3. C4-G4 re-derivation (A = tree, B = all_pairs; ratio > 1 = tree faster)')
REC = {}
for fam in G.FAMS:
    s = []
    for n in G.SIZES:
        j = L.judge(G.by_pass(gu, G.gid(fam, 'tree', n)), G.by_pass(gu, G.gid(fam, 'all_pairs', n)), 'LETTER')
        p = j['pooled']
        tree_faster = j['claimed'] and p['ratio'] > 1
        s.append((n, tree_faster, j))
        out('- %s %d: ap %.3f tree %.3f ratio %.4f pooled %s p %s/%s/%s sep %s -> %s %s' % (
            fam, n, p['B'], p['A'], p['ratio'], L.yn(p), L.yn(j['per'][0]), L.yn(j['per'][1]), L.yn(j['per'][2]),
            p['minmax_sep'], L.label(j), j['dir']))
    lo = max([n for n, tf, _ in s if not tf], default=None)
    hi = min([n for n, tf, _ in s if tf], default=None)
    mono = all((n < hi) != tf for n, tf, _ in s) if hi else False
    REC[fam] = (lo, hi, mono)
    out('  %s: LO %s HI %s monotone %s' % (fam, lo, hi, mono))
los = [REC[f][0] for f in G.FAMS]
his = [REC[f][1] for f in G.FAMS]
edge = any(x is None or x < 96 for x in los) or any(x is None or x >= 160 for x in los) or any(h is None for h in his)
out('recipe: T = min LO = %s; AUTO = (%s, %s); edge fired %s; provisional %s -> %s' % (
    min(los), min(los), max(his), edge, G.PROV,
    'RE-READ, EQUAL to provisional' if (min(los), max(his)) == (128, 136) and not edge else 'DIFFERS / edge'))
out("window 7's LO 144: all_pairs claimed slower at 144 in %s" % [
    fam for fam in G.FAMS if L.judge(G.by_pass(gu, G.gid(fam, 'tree', 144)),
                                     G.by_pass(gu, G.gid(fam, 'all_pairs', 144)), 'LETTER')['claimed']])

out('')
out('# 4. C4-G4-kd (n 3, own block, cross-block against G4 pooled n 9; not gating)')
for fam in G.FAMS:
    for n in G.KD_SIZES:
        kd = [p['v'][G.gid(fam, 'tree_kd', n)] for p in ku]
        tr = [p['v'][G.gid(fam, 'tree', n)] for p in gu]
        ap = [p['v'][G.gid(fam, 'all_pairs', n)] for p in gu]
        c1 = L.cmp_(L.cell(tr), L.cell(kd))
        c2 = L.cmp_(L.cell(ap), L.cell(kd))
        out('- %s %d: kd %.3f; kd/tree %.4f %s; kd/all_pairs %.4f %s' % (
            fam, n, statistics.median(kd), c1['ratio'], L.yn(c1), c2['ratio'], L.yn(c2)))

out('')
out('# 5. C4-BR re-derivation (K 3, one pass, diagnostic; ratio = B/A)')
cells = B.cells(bu)
for arm in B.ARMS:
    for (bn, an) in (('g4r8b', 'g4r7'), ('g4rT', 'g4r7'), ('g4rT', 'g4r8b')):
        rs = []
        for fam in B.FAMS:
            for n in B.SIZES:
                i = B.ident(fam, arm, n)
                c = L.cmp_(cells[(an, i)], cells[(bn, i)])
                rs.append((c['ratio'], L.yn(c), c['minmax_sep']))
        out('- %-9s %s/%s: %s; median %.4f' % (arm, bn, an, ' '.join('%.4f %s%s' % (r, f, ' sep' if s else '')
                                                                      for r, f, s in rs),
                                               statistics.median(r for r, _, _ in rs)))

open(os.path.join(HERE, 'z0_resume.txt'), 'w', encoding='utf-8').write('\n'.join(lines) + '\n')
