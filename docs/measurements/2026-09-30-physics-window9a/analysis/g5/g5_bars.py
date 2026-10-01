"""The pre-registered G5 bars (tree-c4/cut.md section 5 block 4 'G5 bars'; window9_c4_rows.json row gates; recipe
treebp/g4_g5_recipe.md:205-212), judged by ruling 1 + ruling 8 (LETTER: a pass-cell with K < 3 does not gate).
  B1  S4 = sum of phys_bp_{verify,build,query,assemble}_ns per step on C4-JD-armed, median over [100,500) per process:
      <= 0.36 ms at W1, <= 0.35 ms at W8. Letter = the cell (and every pass-cell) at or under the bar; ruling-1 reading
      = the cell CLAIMED below the bar (vs_const, 8b R1's rule for a constant bar).
  B2  realized dbp(8) = SYS(C4-JDap-armed, W8) - SYS(C4-JD-armed, W8) >= 1.03 ms, SYS = sys_physics_broadphase_colored_ns.
      Per-process statistic: the median over [100,500) (the sibling bar's statistic); the mean is the sensitivity.
      Letter = the difference CLAIMED (B > A) and the realized difference of cell medians >= 1.03 in the pool and in every
      pass; stricter reading = JDap CLAIMED above JD + 1.03 ms (per-process shift).
  B3  C4-R (Tree) CLAIMED faster than C4-Rap (AllPairs) at W1 and W8: wall mean over [0,500) per process.
  B4  C4-S16 (Tree, brute path) NOT CLAIMED slower than C4-S16ap at W1: wall mean over [0,500).
  B5  structure: void_steps 0 on every armed process; every pose = its fixture; R4 S16 vs S16ap bytes equal.
Writes g5_bars.txt and g5_bars.json."""
import json
import os

import lib_g5 as L

O = L.Out()
recs = L.all_records()
procs, used, dropped = L.select('C4-G5', recs)
RES = {}


def show_const(key, j, bar):
    O('  pooled cell %s' % L.fcell(j['a'], 4))
    for q, c in j['pcells'].items():
        O('    p%d %s' % (q, L.fcell(c, 4)))
    O('  vs bar %.2f: %s' % (bar, L.fj(j)))
    letter = j['a']['median'] <= bar and all(c['median'] <= bar for c in j['pcells'].values())
    below = j['claimed'] and j['dir'] == 'B<A'
    O('  -> letter (cell and every pass-cell <= bar): %s; ruling-1 CLAIMED below the bar: %s%s; headroom %.4f ms (%.1f %% of the bar)'
      % ('PASS' if letter else 'FAIL', 'YES' if below else 'NO', ' STRONG' if below and j['strong'] else '',
         bar - j['a']['median'], 100 * (bar - j['a']['median']) / bar))
    RES[key] = {'cell': j['a'], 'pcells': j['pcells'], 'pooled': j['pooled'], 'per': j['per'], 'letter': letter,
                'claimed_below': below, 'strong': j['strong']}


O('# B1: S4 on C4-JD-armed, median over [100,500) per process (ms)')
for W, bar in ((1, 0.36), (8, 0.35)):
    O('- W%d, bar %.2f' % (W, bar))
    show_const('B1 W%d' % W, L.judge(L.by_pass(used, 'C4-JD-armed', W, L.sum4_med), const=bar), bar)
O('  (mean-over-steps sensitivity)')
for W, bar in ((1, 0.36), (8, 0.35)):
    j = L.judge(L.by_pass(used, 'C4-JD-armed', W, L.sum4_mean), const=bar)
    O('  W%d mean conv: %s | %s' % (W, L.fcell(j['a'], 4), L.fj(j)))

O('\n# B2: realized dbp(8) = SYS(C4-JDap-armed) - SYS(C4-JD-armed) at W8, bar >= 1.03 ms')
for conv, f in (('median-over-steps', L.col_med), ('mean-over-steps', L.col_mean)):
    va = L.by_pass(used, 'C4-JD-armed', 8, lambda p: f(p, L.SYS))
    vb = L.by_pass(used, 'C4-JDap-armed', 8, lambda p: f(p, L.SYS))
    j = L.judge(va, vb)
    ca, cb = L.cell(sum(va.values(), [])), L.cell(sum(vb.values(), []))
    d_pool = cb['median'] - ca['median']
    d_pass = {q: L.cell(vb[q])['median'] - L.cell(va[q])['median'] for q in sorted(va)}
    vs = {q: [x + 1.03 for x in va[q]] for q in va}
    js = L.judge(vs, vb)
    O('- [%s] A = C4-JD-armed %s' % (conv, L.fcell(ca, 4)))
    O('  B = C4-JDap-armed %s' % L.fcell(cb, 4))
    O('  difference: %s' % L.fj(j))
    O('  realized dbp(8) pooled %.4f ms; per pass %s' % (d_pool, ', '.join('p%d %.4f' % (q, v) for q, v in d_pass.items())))
    O('  stricter: JDap vs (JD + 1.03): %s' % L.fj(js))
    letter = j['claimed'] and j['dir'] == 'B>A' and d_pool >= 1.03 and all(v >= 1.03 for v in d_pass.values())
    O('  -> letter (difference CLAIMED and realized >= 1.03 pooled and in every pass): %s; claimed above 1.03: %s%s; margin %.4f ms '
      '(%.1f %% over the bar); against window 6 cfg-A 1.698: %+.4f ms (%+.1f %%)' % (
          'PASS' if letter else 'FAIL', 'YES' if js['claimed'] and js['dir'] == 'B>A' else 'NO',
          ' STRONG' if js['strong'] and js['dir'] == 'B>A' else '', d_pool - 1.03, 100 * (d_pool / 1.03 - 1),
          d_pool - 1.698, 100 * (d_pool / 1.698 - 1)))
    RES['B2 ' + conv] = {'A': ca, 'B': cb, 'diff': j['pooled'], 'claimed': j['claimed'], 'strong': j['strong'],
                         'dir': j['dir'], 'd_pool': d_pool, 'd_pass': d_pass, 'shift_claimed': js['claimed'],
                         'shift_strong': js['strong'], 'shift_dir': js['dir'], 'shift_pooled': js['pooled'],
                         'shift_per': js['per'], 'letter': letter}
# context: the same at W1 (not a pre-registered bar) and the armed-wall delta at W8 (the cut's 'expected, arith.' line)
va = L.by_pass(used, 'C4-JD-armed', 1, lambda p: L.col_med(p, L.SYS))
vb = L.by_pass(used, 'C4-JDap-armed', 1, lambda p: L.col_med(p, L.SYS))
j = L.judge(va, vb)
O('  context (post hoc, no bar): dbp(1) median conv %s; realized %.4f ms' % (L.fj(j), j['b']['median'] - j['a']['median']))
RES['ctx dbp1'] = {'pooled': j['pooled'], 'claimed': j['claimed'], 'strong': j['strong']}
for W in (1, 8):
    for win in ((0, 500), (100, 500)):
        va = L.by_pass(used, 'C4-JD-armed', W, lambda p: L.wall_mean(p, *win))
        vb = L.by_pass(used, 'C4-JDap-armed', W, lambda p: L.wall_mean(p, *win))
        j = L.judge(va, vb)
        O('  context (the cut: "expected, arith., not a claim: Delta(W8) ~ -1.70 ms"): armed wall W%d %s, JD %.4f JDap %.4f: %s' % (
            W, win, j['a']['median'], j['b']['median'], L.fj(j)))
        RES['ctx armed wall W%d %s' % (W, win)] = {'A': j['a'], 'B': j['b'], 'pooled': j['pooled'],
                                                   'claimed': j['claimed'], 'strong': j['strong']}

O('\n# B3: C4-R (Tree) vs C4-Rap (AllPairs), rest scene, wall mean (ms); A = C4-Rap, B = C4-R; bar: B CLAIMED faster')
for W in (1, 8):
    for win in ((0, 500), (100, 500), (0, 100)):
        va = L.by_pass(used, 'C4-Rap', W, lambda p: L.wall_mean(p, *win))
        vb = L.by_pass(used, 'C4-R', W, lambda p: L.wall_mean(p, *win))
        j = L.judge(va, vb)
        tag = 'BAR' if win == (0, 500) else 'context'
        O('- W%d %s [%s]: A %s | B %s' % (W, win, tag, L.fcell(j['a'], 4), L.fcell(j['b'], 4)))
        O('    %s' % L.fj(j))
        if win == (0, 500):
            ok = j['claimed'] and j['dir'] == 'B<A'
            O('    -> C4-R claimed faster: %s%s' % ('YES' if ok else 'NO', ' STRONG' if ok and j['strong'] else ''))
        RES['B3 W%d %s' % (W, win)] = {'A': j['a'], 'B': j['b'], 'pooled': j['pooled'], 'per': j['per'],
                                       'claimed': j['claimed'], 'strong': j['strong'], 'dir': j['dir']}

O('\n# B4: C4-S16 (Tree, brute path) vs C4-S16ap (AllPairs), W1, wall mean; A = S16ap, B = S16; bar: B NOT claimed slower')
for win in ((0, 500), (100, 500), (0, 100)):
    va = L.by_pass(used, 'C4-S16ap', 1, lambda p: L.wall_mean(p, *win))
    vb = L.by_pass(used, 'C4-S16', 1, lambda p: L.wall_mean(p, *win))
    j = L.judge(va, vb)
    tag = 'BAR' if win == (0, 500) else 'context'
    O('- %s [%s]: A %s | B %s' % (win, tag, L.fcell(j['a'], 3, 1e3, ' us'), L.fcell(j['b'], 3, 1e3, ' us')))
    O('    %s' % L.fj(j, 6))
    if win == (0, 500):
        bad = j['claimed'] and j['dir'] == 'B>A'
        O('    -> C4-S16 claimed slower: %s => bar %s' % ('YES' if bad else 'NO', 'FAILS' if bad else 'HOLDS'))
    RES['B4 %s' % (win,)] = {'A': j['a'], 'B': j['b'], 'pooled': j['pooled'], 'per': j['per'],
                             'claimed': j['claimed'], 'strong': j['strong'], 'dir': j['dir']}

O('\n# B5: structure')
arm = [p for p in procs if L.ROWS[p['row']]['armed']]
O('- armed processes (all attempts) %d: void_steps %s, first_void %s, CSV void column all 0: %s, four-span counts ok: %s' % (
    len(arm), sorted({p['s'].get('void_steps') for p in arm}), sorted({str(p['s'].get('first_void')) for p in arm}),
    all(not any(p['c']['void']) for p in arm), all(not any('count' in w for w in p['valid_why']) for p in arm)))
O('- poses: every used process expect_pose = match: %s (n=%d)' % (
    all(p['s'].get('expect_pose') == 'match' for p in used), len(used)))
r4 = [r for r in recs if r.get('block') == 'C4-G5' and r.get('r4')]
O('- R4 S16 vs S16ap: %d of %d equal' % (sum(r['verdict'] == 'equal' for r in r4), len(r4)))
json.dump(RES, open(os.path.join(L.HERE, 'g5_bars.json'), 'w'), indent=1, default=str)
O.save('g5_bars.txt')
