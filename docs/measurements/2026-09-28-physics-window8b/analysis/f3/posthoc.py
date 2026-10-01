"""POST HOC observations for F3 / F3-G4 (no claim is made from anything here). Reads procs_f3.json, q_runner.json,
g4.json, and window 7 wave 2's committed reduction (D:/wt/merge, read only) for the cross-window G4 lines."""
import collections
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
import lib_f3 as L

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


procs = json.load(open(os.path.join(L.HERE, 'procs_f3.json')))['procs']
used = [p for p in procs if p.get('used')]
Q = json.load(open(os.path.join(L.HERE, 'q_runner.json')))
G = json.load(open(os.path.join(L.HERE, 'g4.json')))

P('## 1. The model against the window (cut.md 3.4 / 3.6; window_cmds.md last line) - W1, TD-armed, pooled cells')
tq = Q['R2 t_q kd vs leaflist W1']
tb = Q['ctx t_b kd vs leaflist W1']
qb = Q['R3a t_qb kd vs leaflist W1']
fs = Q['ctx four-span kd vs leaflist W1']
P('- t_q leaflist %.2f us, kd %.2f us: delta %+.2f us (%+.2f %%); model A -39.8..-57.0 us, B -29.0..-44.2 us' % (
    tq['A']['median'] * 1e3, tq['B']['median'] * 1e3, tq['pooled']['delta'] * 1e3, 100 * (tq['pooled']['ratio'] - 1)))
base = 167.6
for nm, lo, hi in (('A (proportional)', 121.6, 135.5), ('B (fixed residual)', 131.9, 144.2)):
    P('  - %s: c_q %.1f-%.1f ns from the model base %.1f = ratio %.4f-%.4f; measured ratio %.4f (bars i %.2f %% / s '
      '%.2f %%) -> %s' % (nm, lo, hi, base, lo / base, hi / base, tq['pooled']['ratio'], 100 * tq['pooled']['bar_i'],
                          100 * tq['pooled']['bar_s'],
                          'inside' if lo / base <= tq['pooled']['ratio'] <= hi / base else 'outside'))
P('- c_q: leaflist %.1f ns, kd %.1f ns (model predicted kd 121.6-144.2 ns)' % (
    tq['A']['median'] * 1e6 / 1240, tq['B']['median'] * 1e6 / 1240))
P('- t_b leaflist %.2f us, kd %.2f us: delta %+.2f us (x%.3f); model +14..+39 us' % (
    tb['A']['median'] * 1e3, tb['B']['median'] * 1e3, tb['pooled']['delta'] * 1e3, tb['pooled']['ratio']))
P('- t_qb leaflist %.2f us, kd %.2f us: delta %+.2f us (%+.2f %%); model net -43..+10 us (cut 3.6), -36..+8 us '
  '(window_cmds)' % (qb['A']['median'] * 1e3, qb['B']['median'] * 1e3, qb['pooled']['delta'] * 1e3,
                     100 * (qb['pooled']['ratio'] - 1)))
P('- four-span leaflist %.2f us, kd %.2f us: delta %+.2f us (%+.2f %%)' % (
    fs['A']['median'] * 1e3, fs['B']['median'] * 1e3, fs['pooled']['delta'] * 1e3, 100 * (fs['pooled']['ratio'] - 1)))
for W in (8,):
    for key, nm in (('ctx t_q kd vs leaflist W8', 't_q'), ('ctx t_b kd vs leaflist W8', 't_b'),
                    ('R3b t_qb kd vs leaflist W8', 't_qb'), ('ctx four-span kd vs leaflist W8', 'four-span')):
        c = Q[key]
        P('- W8 %s leaflist %.2f us, kd %.2f us: %+.2f us (%+.2f %%) pooled flags %s' % (
            nm, c['A']['median'] * 1e3, c['B']['median'] * 1e3, c['pooled']['delta'] * 1e3,
            100 * (c['pooled']['ratio'] - 1), L.yn(c['pooled'])))

P('')
P('## 2. G4 across windows (window 7 wave 2 Q3, instrument 93b2615b + G4_SIZES, K = 3; this block 16191fda + g4ref)')
g4recs = [r for r in L.load_recs() if r.get('block') == 'F3-G4' and 'row' in r]


def est8b(i):
    xs = []
    for r in g4recs:
        s = json.load(open(os.path.join(r['cwd'], 'criterion', *i.split('/'), 'new', 'sample.json'), encoding='utf-8'))
        xs.append(statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3)
    return L.cell(xs)


w7 = json.load(open('D:/wt/merge/docs/measurements/2026-09-25-physics-window7/wave2/analyst/reduction.json'))['cells']
rat = collections.defaultdict(list)
for fam in ('uniform', 'disparity'):
    for arm, sizes in (('all_pairs', (128, 136, 144, 152, 160, 176, 256)), ('tree', (128, 136, 144, 152, 160, 176, 256)),
                       ('tree_rowwalk', (256,))):
        parts = []
        for n in sizes:
            a = w7['Q3 %s %s/%d' % (fam, arm, n)]['median'] * 1e3
            b = est8b('bp_g4_%s/%s/%d' % (fam, arm, n))['median']
            rat[arm].append(b / a)
            parts.append('%d: %.3f -> %.3f (%+.1f %%)' % (n, a, b, 100 * (b / a - 1)))
        P('- %s %s (us): %s' % (fam, arm, '; '.join(parts)))
for arm, v in rat.items():
    P('- %s: 8b / window 7 over every shared (family, size): median %.4f, range %.4f-%.4f (n=%d)' % (
        arm, statistics.median(v), min(v), max(v), len(v)))
P('- window 7 all_pairs/tree at 144: uniform 1.0076, disparity 0.9929 (analysis 3.1); 8b: uniform %.4f, disparity '
  '%.4f' % (G['families']['uniform']['tree']['cmp']['144']['ratio'], G['families']['disparity']['tree']['cmp']['144']['ratio']))

P('')
P('## 3. The G4 J scenes against the runner (W1, TD-armed; the bench times one steady-state step_direct = verify + build')
P('##    + queries + assembly on a fixed body set, hot; the runner times the same spans inside the full step)')
for n in ('1240', 'j100'):
    c = G['scene'][n]
    P('- bench scene/%s: tree %.2f us, tree_kd %.2f us, kd/tree %.4f (%s)' % (n, c['A'], c['B'], c['ratio'], L.yn(c)))
P('- runner four-span [100,500): leaflist %.2f us, kd %.2f us, kd/leaflist %.4f (%s)' % (
    fs['A']['median'] * 1e3, fs['B']['median'] * 1e3, fs['pooled']['ratio'], L.yn(fs['pooled'])))

P('')
P('## 4. Placement receipts beside the slow processes (recorded, never used to drop): used processes > 1.04 x cell median')


def metric(p):
    if p.get('armed'):
        return p['armed']['t_qb_med']
    return p['wall_ms']['0..500']


cells = collections.defaultdict(list)
for p in used:
    cells[(p['row'], p['W'])].append(p)
n_out = 0
for k in sorted(cells):
    ps = cells[k]
    m = statistics.median(metric(p) for p in ps)
    for p in ps:
        if metric(p) > 1.04 * m:
            n_out += 1
            pl = p.get('placement') or {}
            P('- %s W%d p%d r%d %s: %.4f vs cell median %.4f (%+.1f %%); main_share_top_est %s, main_cycle_frac %s, '
              'top3 %s; witness %s %%' % (k[0], k[1], p['pass'], p['round'], p['attempt'], metric(p), m,
                                         100 * (metric(p) / m - 1), pl.get('main_share_top_est'),
                                         pl.get('main_cycle_frac'), pl.get('top3'), p['others_busy_pct']))
P('(%d of %d used processes)' % (n_out, len(used)))
w1 = [p for p in used if p['W'] == 1 and (p.get('placement') or {}).get('main_share_top_est') is not None]
for fam in ('F3-JT', 'F3-RT', 'F3-TD-armed'):
    xs = [(p['placement']['main_share_top_est'], metric(p) / statistics.median(metric(q) for q in cells[(p['row'], 1)]))
          for p in w1 if p['family'] == fam]
    lo = [r for s, r in xs if s < 0.6]
    hi = [r for s, r in xs if s >= 0.6]
    P('- W1 %s: main_share_top_est < 0.6 on %d processes (median relative value %s), >= 0.6 on %d (%s)' % (
        fam, len(lo), '%.4f' % statistics.median(lo) if lo else '-', len(hi), '%.4f' % statistics.median(hi) if hi else '-'))

tops = sorted((p['placement'] or {}).get('top3', [[None, None]])[0][1] for p in used if p['W'] == 1)
P('- W1 used processes (n=%d): top logical CPU busy %% over the process lifetime: median %.1f, min %.1f, max %.1f; '
  'main_share_top_est is 1.0 (the cap) on %d of them' % (
      len(tops), statistics.median(tops), tops[0], tops[-1],
      sum(1 for p in used if p['W'] == 1 and (p['placement'] or {}).get('main_share_top_est') == 1.0)))
for p in used:
    if p['W'] == 1 and (p['placement'] or {}).get('top3', [[0, 100]])[0][1] < 75:
        m = statistics.median(metric(q) for q in cells[(p['row'], 1)])
        P('  - W1 top-CPU < 75 %%: %s p%d r%d: top3 %s, value/cell median %.4f' % (
            p['row'], p['pass'], p['round'], p['placement']['top3'], metric(p) / m))

P('')
P('## 5. The witness (others_busy_pct) over the F3 block')
ob = sorted(p['others_busy_pct'] for p in procs)
P('- all %d timed processes: median %.2f %%, p90 %.2f %%, max %.2f %%; over 2 %%: %d' % (
    len(ob), statistics.median(ob), ob[int(0.9 * len(ob))], ob[-1], sum(1 for x in ob if x > 2.0)))
top = collections.Counter()
for p in procs:
    if p['clean_why'] and p['witness_top']:
        top[p['witness_top'][0][0]] += 1
P('- top other process on the unclean ones: %s' % top.most_common(5))
ou = sorted(p['others_busy_pct'] for p in used)
P('- used processes: witness median %.2f %%, max %.2f %%' % (statistics.median(ou), ou[-1]))
P('- block span: %s .. %s' % (min(p['start'] for p in procs), max(p['end'] for p in procs)))
open(os.path.join(L.HERE, 'posthoc.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
