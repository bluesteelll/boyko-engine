"""S4-AB verdicts under ruling 1, recomputed from v_proc.json. Output: v_s4.txt."""
import json, os
import vlib as L

procs = json.load(open(os.path.join(L.HERE, 'v_proc.json')))
OUT = []
P = lambda *a: OUT.append(' '.join(str(x) for x in a))


def vals(row, b, W, metric, win, ps=None):
    out = []
    for p in procs:
        if p['block'] == 'S4-AB' and p['row'] == row and p['bin'] == b and p['W'] == W and (ps is None or p['pass'] == ps):
            out.append(p['stats'][win][metric] / 1e6)
    return out


def comp(name, A, B, metric, win):
    """A, B = (row, bin, W)."""
    pa = L.cell(vals(*A, metric, win))
    pb = L.cell(vals(*B, metric, win))
    pooled = L.cmp_(pa, pb)
    passes = []
    for ps in (0, 1, 2):
        ca = L.cell(vals(*A, metric, win, ps))
        cb = L.cell(vals(*B, metric, win, ps))
        passes.append(L.cmp_(ca, cb))
    v = L.ruling1(pooled, passes)
    P('%-22s [%s) %s n %d/%d' % (name, win, metric, pa['K'], pb['K']))
    P('   A %.4f [%.4f-%.4f] IQR %.4f SE %.4f | B %.4f [%.4f-%.4f] IQR %.4f SE %.4f' % (
        pa['med'], pa['min'], pa['max'], pa['iqr'], pa['se'], pb['med'], pb['min'], pb['max'], pb['iqr'], pb['se']))
    P('   B-A %+.4f ms (%+.2f %%); bars r %.2f i %.2f s %.2f %%; pooled %s; passes %s; pass B-A %s' % (
        pooled['d'], pooled['pct'], 100 * pooled['bars']['r'], 100 * pooled['bars']['i'], 100 * pooled['bars']['s'],
        L.yn(pooled), ' '.join(L.yn(c) for c in passes), ' / '.join('%+.4f' % c['d'] for c in passes)))
    P('   pass bars i/s: ' + '; '.join('p%d %.2f/%.2f (e %.2f)' % (i, 100 * c['bars']['i'], 100 * c['bars']['s'], abs(c['pct'])) for i, c in enumerate(passes)))
    P('   -> %s (direction %s)' % (v, 'B slower' if pooled['dir'] > 0 else 'B faster'))
    return pooled, passes, v, pa, pb


SPAN = 'phys_solve_build_ns'
res = {}
for win in ('0..500', '100..500'):
    P('\n==== window [%s) ====' % win)
    res[(win, 'W8wall')] = comp('S4-W8 wall', ('S4-JT', 'parent', 8), ('S4-JT', 'tip', 8), 'wall_ns', win)
    res[(win, 'W8span')] = comp('S4-W8 span', ('S4-JT-a', 'parent', 8), ('S4-JT-a', 'tip', 8), SPAN, win)
    res[(win, 'rung')] = comp('R_8 rung', ('S4-JT', 'tip', 8), ('S4-rung', 'tip', 8), 'wall_ns', win)
    res[(win, 'z30')] = comp('zone N30000 span', ('S4-JT-a', 'tip', 8), ('S4-zone-N30000', 'tip', 8), SPAN, win)
    res[(win, 'z60')] = comp('zone N60000 span', ('S4-JT-a', 'tip', 8), ('S4-zone-N60000', 'tip', 8), SPAN, win)
    for W in (1, 2, 4, 16):
        res[(win, 'W%d' % W)] = comp('no-slower W%d' % W, ('S4-JT', 'parent', W), ('S4-JT', 'tip', W), 'wall_ns', win)
    comp('W16 armed wall', ('S4-JT-a', 'parent', 16), ('S4-JT-a', 'tip', 16), 'wall_ns', win)
    comp('W16 span', ('S4-JT-a', 'parent', 16), ('S4-JT-a', 'tip', 16), SPAN, win)
    comp('W8 armed wall', ('S4-JT-a', 'parent', 8), ('S4-JT-a', 'tip', 8), 'wall_ns', win)
    comp('zone N30000 wall', ('S4-JT-a', 'tip', 8), ('S4-zone-N30000', 'tip', 8), 'wall_ns', win)
    comp('zone N60000 wall', ('S4-JT-a', 'tip', 8), ('S4-zone-N60000', 'tip', 8), 'wall_ns', win)
    comp('P-c (sb_pc) W8', ('S4-JT-a', 'parent', 8), ('S4-JT-a', 'tip', 8), 'phys_sb_pc_ns', win)

P('\n==== derived (window [0,500) unless stated) ====')
wall = res[('0..500', 'W8wall')][0]
span = res[('0..500', 'W8span')][0]
P('wall gain %.4f ms = %.2fx the 0.060 bar; span gain %.4f; wall/span %.3f' % (-wall['d'], -wall['d'] / 0.060, -span['d'], wall['d'] / span['d']))
P('wall gain minus pooled i-bar (ms, relative bar x A): %.4f; span: %.4f' % (
    -wall['d'] - wall['bars']['i'] * wall['A'], -span['d'] - span['bars']['i'] * span['A']))
P('below window-8 arithmetic low 0.19: %.4f' % (0.19 + wall['d']))
# per manifold on [100,500), disarmed W8
for b in ('parent', 'tip'):
    m = [p['stats']['100..500']['manifolds'] for p in procs if p['block'] == 'S4-AB' and p['row'] == 'S4-JT' and p['bin'] == b and p['W'] == 8]
    w = L.cell(vals('S4-JT', b, 8, 'wall_ns', '100..500'))
    P('per manifold W8 [100,500) %s: wall %.4f ms, manifolds per step %s -> %.1f ns/manifold' % (b, w['med'], sorted(set(m)), 1e6 * w['med'] / m[0]))
open(os.path.join(L.HERE, 'v_s4.txt'), 'w').write('\n'.join(OUT) + '\n')
print('\n'.join(OUT))
