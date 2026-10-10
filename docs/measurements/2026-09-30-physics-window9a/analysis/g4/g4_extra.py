"""G4x: post hoc only. (6) the recipe read in ROWS instead of bench labels (disparity label n = n + 4 rows);
(7) arithmetic: the crossover if all_pairs ran at window-7-binary speed (BR g4rT/g4r7 all_pairs factor at 144 applied
to every size - an assumption, labelled); (8) tree arm g4rT/g4r7 in BR; (9) other-process CPU in the G4/kd processes."""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_g4 as G  # noqa: E402

L = G.L
out = L.Out()
g1 = json.load(open(os.path.join(G.HERE, 'g1_thresholds.json'), encoding='utf-8'))
RES = g1['RES']
out('## 6. The recipe read in rows (post hoc; the pre-registered reading uses the bench label n, as window 8b did)')
for fam in G.FAMS:
    f = RES[fam]['ruling1']
    lo, hi = f['LO'], f['HI']
    out('- %s: label LO %s / HI %s = rows %s / %s' % (fam, lo, hi, lo + G.ROWS_EXTRA[fam], hi + G.ROWS_EXTRA[fam]))
los = {fam: RES[fam]['ruling1']['LO'] + G.ROWS_EXTRA[fam] for fam in G.FAMS}
his = {fam: RES[fam]['ruling1']['HI'] + G.ROWS_EXTRA[fam] for fam in G.FAMS}
out('- in rows: TREE_BRUTE_MAX_ROWS = min LO = %d, AUTO_TREE_LO/HI = %d/%d (label reading: 128 / 128/136)' % (
    min(los.values()), min(los.values()), max(his.values())))
cross = {}
for fam in G.FAMS:
    r = {int(k): v for k, v in RES[fam]['ratios'].items()}
    ns = sorted(r)
    for n0, n1 in zip(ns, ns[1:]):
        if r[n0] <= 1 < r[n1]:
            y0, y1 = math.log(r[n0]), math.log(r[n1])
            x0, x1 = math.log(n0 + G.ROWS_EXTRA[fam]), math.log(n1 + G.ROWS_EXTRA[fam])
            cross[fam] = math.exp(x0 - y0 * (x1 - x0) / (y1 - y0))
out('- log-log crossover in rows: %s' % {k: round(v, 1) for k, v in cross.items()})
out('')
out('## 7. Arithmetic: all_pairs at the window-7 binary speed (BR, 144: g4rT/g4r7 = 1.1289 uniform, 1.1239 disparity;'
    ' applied to every size - an ASSUMPTION, not measured below 144)')
F = {'uniform': 1.1289, 'disparity': 1.1239}
for fam in G.FAMS:
    r = {int(k): v / F[fam] for k, v in RES[fam]['ratios'].items()}
    ns = sorted(r)
    cr = None
    for n0, n1 in zip(ns, ns[1:]):
        if r[n0] <= 1 < r[n1]:
            y0, y1 = math.log(r[n0]), math.log(r[n1])
            cr = math.exp(math.log(n0) - y0 * (math.log(n1) - math.log(n0)) / (y1 - y0))
    out('- %s: scaled all_pairs/tree %s; log-log crossover %s' % (
        fam, ' '.join('%d %.3f' % (n, r[n]) for n in ns), '%.1f' % cr if cr else 'none in grid'))
out('')
out('## 8. C4-BR tree arm, g4rT/g4r7 and g4r8b/g4r7 at 144 and 256 (diagnostic, n=3 each; another group owns BR)')
recs = G.all_records()
vals = {}
for r in G.procs_of('C4-BR', recs):
    if r.get('exit') != 0 or not L.sha_ok(r) or L.clean_why(r) or r.get('attempt') != 'original':
        continue
    v = {}
    for i in L.ROWS['C4-BR']['expect']:
        s = json.load(open(os.path.join(r['cwd'], 'criterion', *i.split('/'), 'new', 'sample.json'), encoding='utf-8'))
        v[i] = statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3
    vals.setdefault(r['binary'], []).append(v)
for fam in G.FAMS:
    for n in (144, 256):
        parts = []
        for arm in ('all_pairs', 'tree'):
            i = 'bp_g4_%s/%s/%d' % (fam, arm, n)
            c7 = L.cell([v[i] for v in vals['g4r7']])
            for b in ('g4r8b', 'g4rT'):
                cb = L.cell([v[i] for v in vals[b]])
                c = L.cmp_(c7, cb)
                parts.append('%s %s/g4r7 %.4f %s' % (arm, b, c['ratio'], L.yn(c)))
        out('- %s %d: %s' % (fam, n, '; '.join(parts)))
out('')
out('## 9. Other-process CPU during the used G4 / kd processes (witness %, and the top other process CPU-seconds)')
for block in ('C4-G4', 'C4-G4-kd'):
    _, used, _ = G.select(block, recs)
    w = [p['rec']['others_busy_pct'] for p in used]
    tops = {}
    for p in used:
        per = {}
        for t in p['rec'].get('others_top5') or []:
            per[t['name']] = per.get(t['name'], 0.0) + t['cpu_s'] / p['rec']['wall_s']
        for k, v in per.items():
            tops.setdefault(k, []).append(v)
    out('- %s: witness %.2f-%.2f %% (median %.2f); top-5 others summed by name per process (CPU-s per wall-s = cores): %s' % (
        block, min(w), max(w), statistics.median(w),
        '; '.join('%s median %.3f max %.3f (in %d of %d)' % (k, statistics.median(v), max(v), len(v), len(used))
                  for k, v in sorted(tops.items(), key=lambda kv: -statistics.median(kv[1]))[:4])))
out.save('g4_extra.txt')
