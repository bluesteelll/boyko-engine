"""C4-BR, POST HOC cross-block consistency only (no verdict on C4-G4, which another group analyses): the g4rT cells at
144 from C4-G4 (same exe, same window, K 9 = 3 passes x 3 rounds) against BR's g4rT and g4r7 cells at 144. It asks one
thing: does the binary term BR reads (g4rT all_pairs ~12 % over g4r7) sit in the binary G4 re-reads, at the same
level? Output: br3_g4link.txt."""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
import lib_br as B

L = B.L
P = B.Out()
recs = B.recs_all()
procs, used, dropped = B.select(recs)
C = B.cells(used)
cp = {r['pass'] for r in recs if r.get('pass_done') and r.get('block') == 'C4-G4' and r.get('run_tag') == B.RUN_TAG}
g = [r for r in recs if r.get('block') == 'C4-G4' and 'row' in r and r.get('run_tag') == B.RUN_TAG and r['pass'] in cp]
vals = {}
ok = 0
for r in g:
    if r.get('exit') != 0 or not L.sha_ok(r) or L.clean_why(r):
        continue
    ok += 1
    for fam in B.FAMS:
        for arm in B.ARMS:
            i = B.ident(fam, arm, 144)
            sp = os.path.join(r['cwd'], 'criterion', *i.split('/'), 'new', 'sample.json')
            s = json.load(open(sp, encoding='utf-8'))
            vals.setdefault(i, []).append(statistics.fmean(t / n for t, n in zip(s['times'], s['iters'])) / 1e3)
P('C4-G4 processes read: %d of %d (exit 0, sha, clean); passes closed %s' % (ok, len(g), sorted(cp)))
for fam in B.FAMS:
    for arm in B.ARMS:
        i = B.ident(fam, arm, 144)
        cg = L.cell(vals[i])
        c1 = L.cmp_(C[('g4rT', i)], cg)
        c2 = L.cmp_(C[('g4r7', i)], cg)
        P('- %-30s G4 g4rT %s' % (i, L.fcell(cg, 3)))
        P('    vs BR g4rT %.3f: %.4f (%+.2f %%) %s | vs BR g4r7 %.3f: %.4f (%+.2f %%) %s%s' % (
            c1['A'], c1['ratio'], 100 * (c1['ratio'] - 1), L.yn(c1), c2['A'], c2['ratio'], 100 * (c2['ratio'] - 1),
            L.yn(c2), ' sep' if c2['minmax_sep'] else ''))
for fam in B.FAMS:
    ap = L.cell(vals[B.ident(fam, 'all_pairs', 144)])
    tr = L.cell(vals[B.ident(fam, 'tree', 144)])
    c = L.cmp_(tr, ap)
    P('- G4 g4rT in-binary all_pairs/tree at 144, %s: %.4f %s (BR g4rT %.4f, BR g4r7 %.4f)' % (
        fam, c['ratio'], L.yn(c),
        C[('g4rT', B.ident(fam, 'all_pairs', 144))]['median'] / C[('g4rT', B.ident(fam, 'tree', 144))]['median'],
        C[('g4r7', B.ident(fam, 'all_pairs', 144))]['median'] / C[('g4r7', B.ident(fam, 'tree', 144))]['median']))
B.Out.save(P, 'br3_g4link.txt')
