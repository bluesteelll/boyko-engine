"""DM1 two-state check (post hoc): per-frame medians of the vb SHADE/PRODUCE_NET zones and the deferred zone over
every DM1 process (originals and re-runs), split at 290 us (vb) / 500 us (deferred). Writes states.txt."""
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import libjd as L

OUT = []
procs, used, dropped = L.select(L.load_recs('DM1'), L.validate_dm1, dm1=True)
for z, cut in ((2, 290.0), (14, 290.0), (4, None), (9, None), (17, 500.0)):
    vals = [(p['_zones'][z]['median_ns'] / 1e3, p['binary'], p['seq'], p['attempt']) for p in procs if z in p['_zones']]
    if cut is None:
        v = [x[0] for x in vals]
        OUT.append(f'zone {z}: all {len(v)} processes span {min(v):.1f}-{max(v):.1f} us')
        continue
    lo = [x for x in vals if x[0] < cut]
    hi = [x for x in vals if x[0] >= cut]
    OUT.append(f'zone {z}: low state n {len(lo)} span {min(x[0] for x in lo):.1f}-{max(x[0] for x in lo):.1f} us '
               f'(A {sum(x[1] == "dmA" for x in lo)}, B {sum(x[1] == "dmB" for x in lo)}); high state n {len(hi)} span '
               f'{min(x[0] for x in hi):.1f}-{max(x[0] for x in hi):.1f} us (A {sum(x[1] == "dmA" for x in hi)}, '
               f'B {sum(x[1] == "dmB" for x in hi)})')
    for b in ('dmA', 'dmB'):
        v = [x[0] for x in vals if x[1] == b]
        OUT.append(f'   {b}: {sorted(round(x, 1) for x in v)}')
print('\n'.join(OUT))
open(os.path.join(L.HERE, 'states.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
