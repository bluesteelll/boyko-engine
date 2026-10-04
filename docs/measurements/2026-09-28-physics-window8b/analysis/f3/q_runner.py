"""R1-R3 on the F3 runner rows (tree-f3/window_cmds.md section 5), judged by ruling 1.
Reads procs_f3.json (sel_f3.py). Prints q_runner.txt, writes q_runner.json.

Two readings of ruling 1's 'every clean block' are computed side by side (the letter does not settle it when a pass
lost a slot to a hot witness and its cell has K = 2):
  LETTER : every one of the three passes gates; a pass where either side has K < 3 cannot claim (window 7/8's
           'no claim when either side has K < 3'), so it blocks the claim.
  CLEAN  : a pass gates only if both of its cells kept all three slots (a pass that lost a slot is not a clean block
           for that cell); pooled always gates.
  K2     : every pass gates, a K = 2 pass cell judged on its own i and s (the K >= 3 guard dropped per pass only).
Verdict words: CLAIMED (dir) / NOT CLAIMED / STRONG = CLAIMED and r in every gating test."""
import json
import os
import sys

sys.dont_write_bytecode = True
import lib_f3 as L

PROCS = [p for p in json.load(open(os.path.join(L.HERE, 'procs_f3.json')))['procs'] if p.get('used')]
OUT = []
RES = {}


def P(s=''):
    OUT.append(s)
    print(s)


def vals(row, W, f, passes=(0, 1, 2)):
    return [f(p) for p in PROCS if p['row'] == row and p['W'] == W and p['pass'] in passes]


def cells(row, W, f):
    return {q: L.cell(vals(row, W, f, (q,))) for q in (0, 1, 2)}, L.cell(vals(row, W, f))


def judge(per, pooled, per2=None):
    """per: {pass: cmp}; per2: the same with the K guard at 2. Returns (LETTER, CLEAN, K2)."""
    letter = L.ruling1([(q, per[q]) for q in (0, 1, 2)], pooled)
    gate = [q for q in (0, 1, 2) if per[q]['KB'] >= 3 and (per[q]['KA'] is None or per[q]['KA'] >= 3)]
    clean = L.ruling1([(q, per[q]) for q in gate], pooled)
    clean['gating_passes'] = gate
    k2 = L.ruling1([(q, per2[q]) for q in (0, 1, 2)], pooled) if per2 else None
    return letter, clean, k2


def word(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    return ('STRONG ' if j['strong'] else '') + 'CLAIMED ' + ('higher' if j['dir'] > 0 else 'lower')


def compare(label, rowA, rowB, W, f, scale=1.0, d=4):
    """B against A, per pass and pooled."""
    pa, ca = cells(rowA, W, f)
    pb, cb = cells(rowB, W, f)
    per = {q: L.cmp_(pa[q], pb[q]) for q in (0, 1, 2)}
    per2 = {q: L.cmp_(pa[q], pb[q], 2) for q in (0, 1, 2)}
    pooled = L.cmp_(ca, cb)
    letter, clean, k2 = judge(per, pooled, per2)
    P('- %s (W%d)' % (label, W))
    P('    A %s: %s' % (rowA, L.fcell(ca, scale, d)))
    P('    B %s: %s' % (rowB, L.fcell(cb, scale, d)))
    for q in (0, 1, 2):
        P('    p%d: A %s | B %s | B/A %s' % (q, L.fcell(pa[q], scale, d), L.fcell(pb[q], scale, d), L.fcmp(per[q])))
    P('    pooled B/A %s (delta %+.4f)' % (L.fcmp(pooled), pooled['delta'] * scale))
    P('    LETTER: %s %s | CLEAN (gating %s): %s %s | K2: %s %s | window-7 r AND s pooled: %s' % (
        word(letter), letter['notes'] or '', clean['gating_passes'], word(clean), clean['flags'], word(k2),
        k2['flags'], 'Y' if letter['rs_pooled'] else 'n'))
    RES[label + ' W%d' % W] = {'A': ca, 'B': cb, 'per': per, 'pooled': pooled, 'letter': letter, 'clean': clean,
                              'k2': k2, 'pass_cells_A': pa, 'pass_cells_B': pb}
    return letter, clean, k2


def against_bar(label, row, W, f, bar, scale=1.0, d=4):
    pc, cc = cells(row, W, f)
    per = {q: L.vs_bar(pc[q], bar) for q in (0, 1, 2)}
    per2 = {q: L.vs_bar(pc[q], bar, 2) for q in (0, 1, 2)}
    pooled = L.vs_bar(cc, bar)
    letter, clean, k2 = judge(per, pooled, per2)
    P('- %s (W%d) against the constant bar %s' % (label, W, bar))
    P('    cell %s' % L.fcell(cc, scale, d))
    for q in (0, 1, 2):
        P('    p%d: %s | m/bar %s' % (q, L.fcell(pc[q], scale, d), L.fcmp(per[q])))
    P('    pooled m/bar %s' % L.fcmp(pooled))
    P('    LETTER: %s %s | CLEAN (gating %s): %s | K2: %s' % (word(letter), letter['notes'] or '',
                                                            clean['gating_passes'], word(clean), word(k2)))
    RES[label + ' W%d' % W] = {'cell': cc, 'per': per, 'pooled': pooled, 'letter': letter, 'clean': clean,
                              'k2': k2, 'pass_cells': pc}
    return letter, clean, k2


def tq(p):
    return p['armed']['t_q_med']


def tb(p):
    return p['armed']['t_b_med']


def tqb(p):
    return p['armed']['t_qb_med']


def four(p):
    return p['armed']['four_med']


def wall(win):
    return lambda p: p['wall_ms'][win]


TDL, TDK = 'F3-TD-armed-leaflist', 'F3-TD-armed-kd'
P('# F3 runner rows: R1-R3 (units ms unless stated; per process = median over steps [100,500) for armed spans, mean of')
P('# wall_ns over the window for unarmed rows; cell = median over the used processes)')
P('')
P('## R1: F3-TD-armed W1 leaflist-kd t_q <= 0.186 ms (c_q <= 150 ns), the constant-bar rule')
r1 = against_bar('R1 t_q(kd)', TDK, 1, tq, 0.186, 1.0, 5)
c = RES['R1 t_q(kd) W1']['cell']
P('    c_q = t_q / 1240: %.1f ns [%.1f-%.1f]' % (c['median'] * 1e6 / 1240, c['min'] * 1e6 / 1240, c['max'] * 1e6 / 1240))
P('')
P('## R2: t_q(kd) / t_q(leaflist) < 1, W1, same binary, same block')
r2 = compare('R2 t_q kd vs leaflist', TDL, TDK, 1, tq, 1.0, 5)
P('')
P('## R3 (a): t_qb(kd) < t_qb(leaflist) claimed at W1')
r3a = compare('R3a t_qb kd vs leaflist', TDL, TDK, 1, tqb, 1.0, 5)
P('')
P('## R3 (b): t_qb(kd) not claimed slower at W8')
r3b = compare('R3b t_qb kd vs leaflist', TDL, TDK, 8, tqb, 1.0, 5)
P('')
P('## R3 (c): F3-JT / F3-RT kd not claimed slower at any W (metric [0,500); [0,100) and [100,500) beside)')
r3c = {}
for fam in ('F3-JT', 'F3-RT'):
    for W in (1, 2, 4, 8, 16):
        r3c[(fam, W)] = compare('R3c %s wall[0,500) kd vs leaflist' % fam, fam + '-leaflist', fam + '-kd', W,
                                wall('0..500'), 1.0, 4)
P('')
P('### beside: [0,100) and [100,500)')
beside = {}
for fam in ('F3-JT', 'F3-RT'):
    for W in (1, 2, 4, 8, 16):
        for win in ('0..100', '100..500'):
            beside[(fam, W, win)] = compare('R3c-beside %s wall[%s] kd vs leaflist' % (fam, win), fam + '-leaflist',
                                            fam + '-kd', W, wall(win), 1.0, 4)
P('')
P('## Context rows (not claims): the other spans, W8, the cfg-A bridge, the RowWalk bridge')
for W in (1, 8):
    compare('ctx t_b kd vs leaflist', TDL, TDK, W, tb, 1.0, 5)
    compare('ctx four-span kd vs leaflist', TDL, TDK, W, four, 1.0, 5)
compare('ctx t_q kd vs leaflist', TDL, TDK, 8, tq, 1.0, 5)
compare('ctx TA t_q kd vs leaflist', 'F3-TA-armed-leaflist', 'F3-TA-armed-kd', 1, tq, 1.0, 5)
compare('ctx TA t_qb kd vs leaflist', 'F3-TA-armed-leaflist', 'F3-TA-armed-kd', 1, tqb, 1.0, 5)
compare('ctx TA t_b kd vs leaflist', 'F3-TA-armed-leaflist', 'F3-TA-armed-kd', 1, tb, 1.0, 5)
compare('ctx TR rowwalk vs TD leaflist t_q', 'F3-TR-armed', TDL, 1, tq, 1.0, 5)
compare('ctx TR rowwalk vs TD kd t_q', 'F3-TR-armed', TDK, 1, tq, 1.0, 5)
compare('ctx TR rowwalk vs TD leaflist t_qb', 'F3-TR-armed', TDL, 1, tqb, 1.0, 5)
compare('ctx TR rowwalk vs TD kd t_qb', 'F3-TR-armed', TDK, 1, tqb, 1.0, 5)
P('')
P('## Means beside the medians (post hoc: the pre-registered per-process statistic is the median over steps)')
for W in (1, 8):
    for nm, f in (('t_q', lambda p: p['armed']['t_q_mean']), ('t_b', lambda p: p['armed']['t_b_mean']),
                  ('t_qb', lambda p: p['armed']['t_qb_mean'])):
        compare('posthoc mean %s kd vs leaflist' % nm, TDL, TDK, W, f, 1.0, 5)
P('')
P('## Summary lines')


def s(j):
    return word(j)


P('R1 letter %s / clean %s / k2 %s' % (s(r1[0]), s(r1[1]), s(r1[2])))
P('R2 letter %s / clean %s / k2 %s' % (s(r2[0]), s(r2[1]), s(r2[2])))
P('R3a letter %s / clean %s / k2 %s' % (s(r3a[0]), s(r3a[1]), s(r3a[2])))
P('R3b letter %s / clean %s / k2 %s' % (s(r3b[0]), s(r3b[1]), s(r3b[2])))
for k in sorted(r3c):
    P('R3c %s W%d letter %s / clean %s / k2 %s' % (k[0], k[1], s(r3c[k][0]), s(r3c[k][1]), s(r3c[k][2])))
for k in sorted(beside):
    P('R3c-beside %s W%d [%s] letter %s / clean %s / k2 %s' % (k[0], k[1], k[2], s(beside[k][0]), s(beside[k][1]),
                                                               s(beside[k][2])))


def dump(o):
    if isinstance(o, dict):
        return {str(k): dump(v) for k, v in o.items()}
    if isinstance(o, (list, tuple)):
        return [dump(v) for v in o]
    return o


json.dump(dump(RES), open(os.path.join(L.HERE, 'q_runner.json'), 'w'), indent=1)
open(os.path.join(L.HERE, 'q_runner.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
