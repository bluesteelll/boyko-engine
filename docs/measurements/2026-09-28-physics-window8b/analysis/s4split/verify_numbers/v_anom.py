"""Anomalies and receipts: slow W16 PARENT processes, pass drift, rung reference, placement. Output: v_anom.txt."""
import json, os, statistics, collections
import vlib as L

procs = json.load(open(os.path.join(L.HERE, 'v_proc.json')))
OUT = []
P = lambda *a: OUT.append(' '.join(str(x) for x in a))
S4 = [p for p in procs if p['block'] == 'S4-AB']

P('== W16 S4-AB processes [0,500): wall ms, median step ms, frac > 1.2x median, span ms, witness, receipts')
for row in ('S4-JT', 'S4-JT-a'):
    for b in ('parent', 'tip'):
        for p in sorted([p for p in S4 if p['row'] == row and p['bin'] == b and p['W'] == 16], key=lambda p: (p['pass'], p['round'])):
            sp = p['stats']['0..500'].get('phys_solve_build_ns')
            P('  %s %s p%d r%d %s: wall %.4f med %.4f frac %.3f span %s wit %.2f rb %.2f ra %.2f' % (
                row, b, p['pass'], p['round'], p['attempt'], p['stats']['0..500']['wall_ns'] / 1e6, p['median_step'] / 1e6,
                p['frac_gt_1p2_med'], ('%.4f' % (sp / 1e6)) if sp else '-', p['wit'], p['rb'], p['ra']))

P('')
P('== per-pass medians, S4-JT wall [0,500) (ms)')
for b in ('parent', 'tip'):
    for W in (1, 2, 4, 8, 16):
        m = [statistics.median(p['stats']['0..500']['wall_ns'] / 1e6 for p in S4 if p['row'] == 'S4-JT' and p['bin'] == b and p['W'] == W and p['pass'] == ps) for ps in (0, 1, 2)]
        P('  %s W%d: p0 %.4f p1 %.4f p2 %.4f' % (b, W, m[0], m[1], m[2]))

P('')
P('== rung reference S4-JT tip W8 processes')
for p in sorted([p for p in S4 if p['row'] == 'S4-JT' and p['bin'] == 'tip' and p['W'] == 8], key=lambda p: (p['pass'], p['round'])):
    P('  p%d r%d wall %.4f frac>1.2med %.3f' % (p['pass'], p['round'], p['stats']['0..500']['wall_ns'] / 1e6, p['frac_gt_1p2_med']))

P('')
P('== placement receipts over the 207 used processes')
top = collections.Counter(p['pl']['top_cpu'] for p in procs)
P('  top_cpu', dict(top))
mcf = [p['pl']['main_cycle_frac'] for p in procs]
mcf1 = [p['pl']['main_cycle_frac'] for p in procs if p['W'] == 1]
P('  main_cycle_frac all %.4f-%.4f; W1 %.4f-%.4f' % (min(mcf), max(mcf), min(mcf1), max(mcf1)))
P('  main_share_top_est values', dict(collections.Counter(p['pl']['main_share_top_est'] for p in procs)))
open(os.path.join(L.HERE, 'v_anom.txt'), 'w').write(chr(10).join(OUT) + chr(10))
print(chr(10).join(OUT))
