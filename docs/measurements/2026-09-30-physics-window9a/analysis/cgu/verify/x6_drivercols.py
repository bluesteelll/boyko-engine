"""Verifier: own run.csv window means vs the driver's per-record cols means, all 119 C4-CGU processes."""
import io, contextlib, os, sys
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
with contextlib.redirect_stdout(io.StringIO()):
    import x1_recompute as X1
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a); OUT.append(s); print(s)
bad = 0
n = 0
for r in X1.procs + X1.warm:
    w = X1.info[id(r)][2]
    for key, (a0, b0) in (('100..500', (100, 500)), ('0..500', (0, 500))):
        mine = sum(w[a0:b0]) / (b0 - a0)
        drv = r['cols'][key]['wall_ns']['mean']
        n += 1
        if abs(mine - drv) > 0.5:
            bad += 1
            P('MISMATCH', r['pass'], r['seq'], r['attempt'], key, mine, drv)
P('window means compared:', n, 'mismatches:', bad)
open(os.path.join(HERE, 'x6_drivercols.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
