"""The verdict-table rows for f3.md, printed from q_runner.json, g4.json, r4.txt and procs_f3.json (flags in i/s/r
order here; 'Y' = the difference clears that bar). Writes tables.txt."""
import json
import os
import sys

sys.dont_write_bytecode = True
import lib_f3 as L

Q = json.load(open(os.path.join(L.HERE, 'q_runner.json')))
G = json.load(open(os.path.join(L.HERE, 'g4.json')))
procs = json.load(open(os.path.join(L.HERE, 'procs_f3.json')))['procs']
OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


def isr(c):
    return ''.join('Y' if c['cl_' + k] else 'n' for k in 'isr')


def flags(e):
    s = []
    for q in ('0', '1', '2'):
        c = e['per'][q]
        s.append('p%s %s%s' % (q, isr(c), '' if c['KB'] >= 3 and (c['KA'] is None or c['KA'] >= 3) else ' (K%s/%s)' % (
            c['KA'] if c['KA'] is not None else '-', c['KB'])))
    s.append('pooled %s' % isr(e['pooled']))
    return '; '.join(s)


def kpp(cell_pass):
    return '/'.join(str(cell_pass[q]['K']) if cell_pass[q] else '0' for q in ('0', '1', '2'))


def cellstr(c, pc, scale, unit, d):
    return '%.*f %s [%.*f-%.*f], IQR %.2f %%, SE %.2f %%, n=%d (%s)' % (
        d, c['median'] * scale, unit, d, c['min'] * scale, d, c['max'] * scale, 100 * c['i'], 100 * c['s'], c['K'],
        kpp(pc))


def verdict(e):
    out = []
    for nm in ('clean', 'k2', 'letter'):
        j = e[nm]
        if j['claimed']:
            w = ('STRONG ' if j['strong'] else '') + 'CLAIMED ' + ('higher' if j['dir'] > 0 else 'lower')
        else:
            w = 'NOT CLAIMED'
        out.append('%s %s' % (nm.upper(), w))
    return '; '.join(out)


def cmprow(cid, bar, key, scale=1e3, unit='us', d=2):
    e = Q[key]
    p = e['pooled']
    P('| %s | %s | A %s; B %s | B/A %.4f (%+.2f %%, %+.2f %s); pooled bars 2i %.2f / 2s %.2f / 2r %.2f %% | %s | %s |' % (
        cid, bar, cellstr(e['A'], e['pass_cells_A'], scale, unit, d), cellstr(e['B'], e['pass_cells_B'], scale, unit, d),
        p['ratio'], 100 * (p['ratio'] - 1), p['delta'] * scale, unit, 100 * p['bar_i'], 100 * p['bar_s'],
        100 * p['bar_r'], flags(e), verdict(e)))


e = Q['R1 t_q(kd) W1']
P('| R1 | t_q(kd), TD-armed W1 <= 0.186 ms (c_q <= 150 ns); constant bar | %s | m/bar %.4f (%+.2f %%); c_q %.1f ns; '
  'pooled bars 2i %.2f / 2s %.2f / 2r %.2f %% | %s | %s |' % (
      cellstr(e['cell'], e['pass_cells'], 1e3, 'us', 2), e['pooled']['ratio'], 100 * (e['pooled']['ratio'] - 1),
      e['cell']['median'] * 1e6 / 1240, 100 * e['pooled']['bar_i'], 100 * e['pooled']['bar_s'],
      100 * e['pooled']['bar_r'], flags(e), verdict(e)))
cmprow('R2', 't_q(kd)/t_q(leaflist) < 1, TD-armed W1, claimed', 'R2 t_q kd vs leaflist W1')
cmprow('R3a', 't_qb(kd) < t_qb(leaflist) claimed, TD-armed W1', 'R3a t_qb kd vs leaflist W1')
cmprow('R3b', 't_qb(kd) not claimed slower, TD-armed W8', 'R3b t_qb kd vs leaflist W8')
for fam in ('F3-JT', 'F3-RT'):
    for W in (1, 2, 4, 8, 16):
        cmprow('R3c %s W%d' % (fam[3:], W), 'wall [0,500) kd not claimed slower', 'R3c %s wall[0,500) kd vs leaflist W%d' % (fam, W),
               1.0, 'ms', 4)
GC = G['cells']


def gcell(i, d=2):
    c = GC[i]
    return '%.*f us [%.*f-%.*f], IQR %.2f %%, SE %.2f %%, n=%d' % (d, c['median'], d, c['min'], d, c['max'], 100 * c['i'],
                                                                   100 * c['s'], c['K'])


def gverdict(c, slower_word='SLOWER'):
    if c['cl_i'] and c['cl_s']:
        return ('STRONG ' if c['cl_r'] else '') + 'CLAIMED ' + (slower_word if c['ratio'] > 1 else 'faster')
    return 'NOT CLAIMED'


for n in ('10000', '100000'):
    c = G['scene'][n]
    P('| R3d scene/%s | bp_g4_scene tree_kd not claimed slower than tree (G4, K 3, one pass) | tree %s; tree_kd %s | '
      'kd/tree %.4f (%+.2f %%); bars 2i %.2f / 2s %.2f / 2r %.2f %% | %s | %s (kd faster) |' % (
          n, gcell('bp_g4_scene/tree/' + n), gcell('bp_g4_scene/tree_kd/' + n), c['ratio'], 100 * (c['ratio'] - 1),
          100 * c['bar_i'], 100 * c['bar_s'], 100 * c['bar_r'], isr(c), gverdict(c)))
for fam in ('uniform', 'disparity'):
    for n, what in (('144', 'all_pairs NOT claimed slower than tree at 144'), ('152', 'tree claimed faster at 152')):
        c = G['families'][fam]['tree']['cmp'][n]
        P('| TH %s %s | %s | all_pairs %s; tree %s | all_pairs/tree %.4f (%+.2f %%); bars 2i %.2f / 2s %.2f / 2r %.2f %% '
          '| %s | %s |' % (fam, n, what, gcell('bp_g4_%s/all_pairs/%s' % (fam, n), 3), gcell('bp_g4_%s/tree/%s' % (fam, n), 3),
                           c['ratio'], 100 * (c['ratio'] - 1), 100 * c['bar_i'], 100 * c['bar_s'], 100 * c['bar_r'],
                           isr(c), gverdict(c, 'all_pairs slower = tree faster')))
for fam in ('uniform', 'disparity'):
    for arm in ('tree', 'tree_kd'):
        f = G['families'][fam][arm]
        P('- recipe %s %s: ruling 1 LO %s / HI %s; window-7 rule LO %s / HI %s; monotone %s/%s; log-log crossover %s; '
          'ratios all_pairs/%s at 128..1000: %s' % (
              fam, arm, f['ruling1']['LO'] if f['ruling1']['LO'] is not None else '< 128 (grid edge)', f['ruling1']['HI'],
              f['w7']['LO'], f['w7']['HI'], f['ruling1']['monotone'], f['w7']['monotone'],
              '%.1f' % f['loglog'] if f['loglog'] else 'below 128 (no ratio <= 1 in the grid)', arm,
              ', '.join('%s %.4f %s' % (n, f['cmp'][n]['ratio'], isr(f['cmp'][n])) for n in
                        ('128', '136', '144', '152', '160', '176', '256', '1000'))))
P('- tree_kd/tree per size (the build-inclusive step; recorded for R2-without-R3): ' + '; '.join(
    '%s %s' % (fam, ', '.join('%s %.4f %s' % (n, GC['bp_g4_%s/tree_kd/%s' % (fam, n)]['median'] /
                                             GC['bp_g4_%s/tree/%s' % (fam, n)]['median'],
                                             isr(L.cmp_(GC['bp_g4_%s/tree/%s' % (fam, n)], GC['bp_g4_%s/tree_kd/%s' % (fam, n)])))
                              for n in ('128', '136', '144', '152', '160', '176', '256', '1000')))
    for fam in ('uniform', 'disparity')))
P('- scene tree_kd/tree: ' + ', '.join('%s %.4f %s' % (n, G['scene'][n]['ratio'], isr(G['scene'][n]))
                                      for n in ('j100', '1240', '10000', '100000')))
used = [p for p in procs if p.get('used')]
P('- selection: %d timed processes (%d re-runs), %d slots, %d used (%d by re-run), %d dropped; cells K<9: %d of 27' % (
    len(procs), sum(1 for p in procs if p['attempt'] == 'rerun'), 243, len(used),
    sum(1 for p in used if p['attempt'] == 'rerun'), 243 - len(used),
    sum(1 for k in {(p['row'], p['W']) for p in used} if sum(1 for p in used if (p['row'], p['W']) == k) < 9)))
open(os.path.join(L.HERE, 'tables.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
