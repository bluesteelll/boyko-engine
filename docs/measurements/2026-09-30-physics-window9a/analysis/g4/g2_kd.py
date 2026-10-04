"""G2: C4-G4-kd (g4rT, own K 3 = one pass x three rounds, NOT gating) - tree_kd at 96/112/128 against tree (LeafList)
and all_pairs, both families. Recorded for F3 keep/freeze (RULINGS-2026-09-29-W8B ruling 4).

The tree_kd processes ran in their OWN block (05:41-05:49), after C4-G4 (04:28-05:38); tree and all_pairs exist only in
C4-G4. So every kd comparison is CROSS-BLOCK (different processes; the window-8b G4 block had all arms in one process).
Two readings, both at the claim arithmetic of ruling 1 with the kd block as its only pass (as window 8b read its K-3
G4 block: the pass is the pooled cell, claimed = i AND s, STRONG = also r):
  X9 : kd (n=3) against the C4-G4 pooled cell (n=9);
  X3 : kd (n=3) against C4-G4 pass 2 only (n=3; the adjacent pass, closest in time).
Not gating: no verdict here is a claim; labels are "would be claimed" readings.
The bench times one whole tree step (verify + active build + queries + assembly; bodies dynamic, so the active tree is
rebuilt every step and kd_order_builds > 0 every step on tree_kd); the query gain and the kd build cost are NOT
separable in this instrument - the ratio is the net."""
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_g4 as G  # noqa: E402

L = G.L
out = L.Out()
recs = G.all_records()
_, used4, d4 = G.select('C4-G4', recs)
_, usedk, dk = G.select('C4-G4-kd', recs)
assert not d4 and not dk and len(used4) == 9 and len(usedk) == 3
out('C4-G4 used %d, C4-G4-kd used %d (all originals, clean and valid)' % (len(used4), len(usedk)))
out('kd block wall: %s .. %s; C4-G4 pass 2: %s .. %s' % (
    usedk[0]['rec']['start'][11:19], usedk[-1]['rec']['end'][11:19],
    [p for p in used4 if p['pass'] == 2][0]['rec']['start'][11:19],
    [p for p in used4 if p['pass'] == 2][-1]['rec']['end'][11:19]))
out('')


def judge1(a_vals, b_vals):
    """one-pass reading: pooled = the pass."""
    a, b = L.cell(a_vals), L.cell(b_vals)
    c = L.cmp_(a, b)
    claimed = c['cl_i'] and c['cl_s']
    return a, b, c, claimed, claimed and c['cl_r']


def lab(c, cl, st):
    if not cl:
        return 'not claimed'
    return ('STRONG ' if st else '') + 'claimed ' + ('kd SLOWER' if c['ratio'] > 1 else 'kd FASTER')


RES = {}
for fam in G.FAMS:
    out('## %s' % fam)
    for n in G.KD_SIZES:
        kd = [p['v'][G.gid(fam, 'tree_kd', n)] for p in usedk]
        tr9 = [p['v'][G.gid(fam, 'tree', n)] for p in used4]
        tr3 = [p['v'][G.gid(fam, 'tree', n)] for p in used4 if p['pass'] == 2]
        ap9 = [p['v'][G.gid(fam, 'all_pairs', n)] for p in used4]
        r = {}
        for tag, tr in (('X9', tr9), ('X3', tr3)):
            a, b, c, cl, st = judge1(tr, kd)
            r[tag] = {'ratio': c['ratio'], 'delta': c['delta'], 'flags': L.yn(c), 'label': lab(c, cl, st),
                      'sep': c['minmax_sep'], 'bars': (c['bar_i'], c['bar_s'], c['bar_r'])}
        a, b, c, cl, st = judge1(ap9, kd)
        r['vs_all_pairs'] = {'ratio': c['ratio'], 'delta': c['delta'], 'flags': L.yn(c),
                             'label': (('STRONG ' if st else '') + 'claimed ' + (
                                 'kd SLOWER than all_pairs' if c['ratio'] > 1 else 'kd FASTER than all_pairs'))
                             if cl else 'not claimed'}
        RES['%s/%d' % (fam, n)] = r
        out('- n=%d (rows %d): tree_kd %s' % (n, n + G.ROWS_EXTRA[fam], G.fc(L.cell(kd))))
        out('    tree (C4-G4 pooled) %s; tree (C4-G4 p2) %s' % (G.fc(L.cell(tr9)), G.fc(L.cell(tr3))))
        for tag in ('X9', 'X3'):
            x = r[tag]
            out('    %s kd/tree %.4f (%+.2f %%, %+.3f us); bars 2i %.2f / 2s %.2f / 2r %.2f %%; flags i/s/r %s; min-max '
                'separated %s -> %s' % (tag, x['ratio'], 100 * (x['ratio'] - 1), x['delta'], 100 * x['bars'][0],
                                        100 * x['bars'][1], 100 * x['bars'][2], x['flags'], x['sep'], x['label']))
        x = r['vs_all_pairs']
        out('    kd/all_pairs (C4-G4 pooled) %.4f (%+.3f us), flags %s -> %s' % (
            x['ratio'], x['delta'], x['flags'], x['label']))
    out('')
out('## Window 8b beside (same-process kd and tree, 16191fda + g4ref, K 3, one pass; win8b/analysis/f3/g4.json)')
W8B = os.path.join(os.path.dirname(G.W9A), "win8b", 'analysis', 'f3', 'g4.json')
g8 = json.load(open(W8B, encoding='utf-8'))['cells']
for fam in G.FAMS:
    parts = []
    for n in (128, 136, 144, 152, 160, 176, 256, 1000):
        t = g8.get('bp_g4_%s/tree/%d' % (fam, n))
        k = g8.get('bp_g4_%s/tree_kd/%d' % (fam, n))
        if t and k:
            c = L.cmp_(t, k)
            parts.append('%d %.4f %s' % (n, c['ratio'], L.yn(c)))
    out('- %s kd/tree (8b): %s' % (fam, '; '.join(parts)))
out('')
out('## Reading for F3 (ruling 4: "F3 not the default ... Held opt-in until the tree C4 decision; if C4 does not use it, '
    'freeze and remove"; cut Q8: decide after the window-9 tree_kd rows)')
fast = [k for k, v in RES.items() if 'kd FASTER' in v['X9']['label'] and 'kd FASTER' in v['X3']['label']]
slow = [k for k, v in RES.items() if 'kd SLOWER' in v['X9']['label'] and 'kd SLOWER' in v['X3']['label']]
out('- cells where kd reads net FASTER than tree under both readings: %s' % (fast or 'none'))
out('- cells where kd reads net SLOWER than tree under both readings: %s' % (slow or 'none'))
out('- at 96/112/128 the shipped default runs NEITHER tree arm: n <= TREE_BRUTE_MAX_ROWS (128, re-read in g1) takes the '
    'brute path (all_pairs_into); and all_pairs is STRONG claimed faster than tree at 96..120 (g1), and kd against '
    'all_pairs is printed above')
json.dump(RES, open(os.path.join(G.HERE, 'g2_kd.json'), 'w'), indent=1)
out.save('g2_kd.txt')
