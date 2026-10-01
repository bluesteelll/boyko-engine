"""Verifier post hoc: per-step quantile shift CGU vs AB (tip, W8/W16, [100,500)), three definitions."""
import io, contextlib, os, sys, math
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
with contextlib.redirect_stdout(io.StringIO()):
    import x1_recompute as X1
    import x4_crossblock as X4
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a); OUT.append(s); print(s)
q = X4.q
for W in (8, 16):
    A = [r for r in X4.abused if r['W'] == W]
    B = [r for r in X4.cgu_tip if r['W'] == W]
    for name, f in (('pooled steps', lambda rs, p: q([v for r in rs for v in X4.wall(r)[100:500]], p)),
                    ('median of per-process quantile', lambda rs, p: X1.med([q(X4.wall(r)[100:500], p) for r in rs])),
                    ('mean of per-process quantile', lambda rs, p: sum(q(X4.wall(r)[100:500], p) for r in rs) / len(rs))):
        P(f'W{W} {name}: ' + ' '.join(f"p{int(100*p)} {100*(f(B, p)/f(A, p)-1):+.1f} %" for p in (0.1, 0.5, 0.9)))
    # p0 of AB has n=1 at W8 - restrict to passes 1, 2 (both have K 3) as a sensitivity
    A12 = [r for r in A if r['pass'] in (1, 2)]; B12 = [r for r in B if r['pass'] in (1, 2)]
    f = lambda rs, p: q([v for r in rs for v in X4.wall(r)[100:500]], p)
    P(f'W{W} pooled steps, passes 1-2 only: ' + ' '.join(f"p{int(100*p)} {100*(f(B12, p)/f(A12, p)-1):+.1f} %" for p in (0.1, 0.5, 0.9)))
open(os.path.join(HERE, 'x5_quantiles.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
