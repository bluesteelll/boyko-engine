"""Markdown rows for the verdict table, straight from q_s4.pkl (ruling 1 judgements)."""
from common_s4 import *
import pickle

R = pickle.load(open(os.path.join(L.HERE, 'q_s4.pkl'), 'rb'))


def cellstr(c):
    return '%.4f [%.4f-%.4f] IQR %.4f SE %.4f' % (c['median'], c['min'], c['max'], c['iqr_abs'], c['se_abs'])


def flags(j):
    s = 'pooled %s' % L.yn(j['pooled'])
    s += '; p0 %s p1 %s p2 %s' % tuple(L.yn(c) for c in j['per'])
    return s


def row(cid, bar, cells, key, verdict_fn):
    j = R[str(key)]
    d = j['pooled']
    nums = 'A %s; B %s; B-A %+.4f ms (%+.2f %%); pass B-A %s' % (
        cellstr(j['a']), cellstr(j['b']), d['delta'], 100 * (d['ratio'] - 1),
        ' / '.join('%+.4f' % c['delta'] for c in j['per']))
    bars = 'bars pooled i %.2f s %.2f r %.2f %%' % (100 * d['bar_i'], 100 * d['bar_s'], 100 * d['bar_r'])
    P('| %s | %s | %s | %s; %s | %s | %s |' % (cid, bar, cells, nums, bars, flags(j), verdict_fn(j)))


def v(j):
    return j['verdict'] + (' STRONG' if j['strong'] else '')


P('| claim | pre-registered bar | cells, n | numbers (ms; A = reference, B = test) | i/s/r flags (r/i/s order) | verdict |')
P('|---|---|---|---|---|---|')
for win in ('0..500',):
    row('S4-W8 wall', 'gain >= 0.060 ms, TIP faster claimed; wall route needs R_8 <= 0.060', 'S4-JT PARENT (A) vs TIP (B), W8, [0,500); n 9/9 (3+3+3)', (win, 'JT', 8), v)
    row('S4-W8 span', 'span gain >= 0.060 ms claimed, with the 30/60 us zone canary seen', 'S4-JT-a PARENT vs TIP phys_solve_build, W8, [0,500); n 9/9', (win, 'span', 8), v)
    row('R_8 rung', 'rise of the 0.060 ms rung claimed = R_8 <= 0.060', 'S4-JT#tip (A) vs S4-rung#tip (B), W8, [0,500); n 9/9', (win, 'rung', 8), v)
    row('zone N30000', 'span rise claimed (canary seen)', 'S4-JT-a#tip vs S4-zone-N30000, W8, phys_solve_build, [0,500); n 9/9', (win, 'zone', 30000), v)
    row('zone N60000', 'span rise claimed (canary seen)', 'S4-JT-a#tip vs S4-zone-N60000, W8, phys_solve_build, [0,500); n 9/9', (win, 'zone', 60000), v)
    for W in (1, 2, 4, 16):
        row('no-slower W%d' % W, 'TIP not claimed slower (W1: any W1 difference is layout)', 'S4-JT PARENT vs TIP, W%d, [0,500); n 9/9' % W, (win, 'JT', W),
            lambda j: 'not slower (' + {'CLAIMED': 'TIP faster CLAIMED', 'NOT CLAIMED': 'no resolved difference', 'REFUTED': 'TIP SLOWER CLAIMED'}[j['verdict']] + (' STRONG' if j['strong'] else '') + ')' if j['verdict'] != 'REFUTED' else 'TIP SLOWER CLAIMED')
    row('W16 wall gain (no bar)', 'none pre-registered at W16 (design: -)', 'S4-JT PARENT vs TIP, W16, [0,500); n 9/9', (win, 'JT', 16), v)
    row('W16 armed wall (no bar)', 'none', 'S4-JT-a PARENT vs TIP, W16, [0,500); n 9/9', (win, 'JTa', 16), v)
    row('W16 span (no bar)', 'none', 'S4-JT-a PARENT vs TIP phys_solve_build, W16, [0,500); n 9/9', (win, 'span', 16), v)
    row('W8 armed wall', 'beside', 'S4-JT-a PARENT vs TIP, W8, [0,500); n 9/9', (win, 'JTa', 8), v)
P('\n[100,500) beside:')
for k, lab in ((('100..500', 'JT', 8), 'S4-W8 wall'), (('100..500', 'span', 8), 'S4-W8 span'), (('100..500', 'rung', 8), 'rung'),
               (('100..500', 'JT', 1), 'W1'), (('100..500', 'JT', 2), 'W2'), (('100..500', 'JT', 4), 'W4'),
               (('100..500', 'JT', 16), 'W16'), (('100..500', 'zone', 30000), 'zone30'), (('100..500', 'zone', 60000), 'zone60')):
    j = R[str(k)]
    P('%s: B-A %+.4f ms (%+.2f %%) %s -> %s%s' % (lab, j['pooled']['delta'], 100 * (j['pooled']['ratio'] - 1), flags(j), j['verdict'], ' STRONG' if j['strong'] else ''))
save('mk_table.txt')
