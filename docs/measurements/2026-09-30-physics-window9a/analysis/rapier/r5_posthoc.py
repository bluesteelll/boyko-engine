"""r5 POST HOC: (a) ours [0,500) cells against window 8b's trunk table (win8b/analysis.md section 5, 16191fda, J-T,
S4-AB TIP; a different binary and window, so a sanity check, never a claim); (b) the size of each [100,500) median gap
against 8b's largest block-level drift (F3 / S4-AB 1.1066 at W8) - which directional readings survive a drift of that size."""
import json
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import lib_rp as L  # noqa: E402

o = L.Out()
J = json.load(open(os.path.join(HERE, 'r1_claims.json')))
W8B = {1: 4.4500, 2: 2.9998, 4: 2.1981, 8: 1.8154, 16: 2.0502}
o('(a) ours 9a [0,500) / 8b trunk 16191fda [0,500):')
for W in (1, 2, 4, 8, 16):
    name = 'ours C4-JT#tip' if W in (1, 8, 16) else 'ours C4-JD#tip'
    v = J['cells'][f'{name}|W{W}|[0,500)']['median_ms']
    o(f'  W{W}: {v:.4f} / {W8B[W]:.4f} = {v / W8B[W]:.4f}')
DRIFT = 1.1066
o(f'(b) [100,500) Rapier/ours median ratios vs a block drift of {DRIFT} (8b F3/S4-AB at W8): a gap survives if ratio > {DRIFT:.4f} or < {1 / DRIFT:.4f}')
for kind in ('WALL', 'ROW'):
    for cfg in 'DM':
        for W in (1, 2, 4, 8, 16):
            d = J['claims'][f'R-{kind}-{cfg}(W{W})|[100,500)']
            rs = [d[a]['ratio_rapier_over_ours'] for a in ('rs8', 'rs4')]
            surv = ['yes' if (r > DRIFT or r < 1 / DRIFT) else 'no' for r in rs]
            o(f'  R-{kind}-{cfg} W{W}: rs8 {rs[0]:.3f} ({surv[0]}), rs4 {rs[1]:.3f} ({surv[1]})')
o.save('r5_posthoc.txt')
