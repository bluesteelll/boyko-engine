"""Counts behind the verifier's disputes, from v1_recompute.json (the verifier's own output)."""
import json, os
HERE = os.path.dirname(os.path.abspath(__file__))
R = json.load(open(os.path.join(HERE, 'v1_recompute.json')))
out = []
claims = {k: v for k, v in R.items() if isinstance(v, dict)}
nc_ours = [k for k, v in claims.items() if v['EXCL'] == 'NOT CLAIMED']
nc_both = [k for k, v in claims.items() if v['EXCL'] == 'NOT CLAIMED' and v['EXCL_rfirst'] == 'NOT CLAIMED']
out.append('LETTER NOT CLAIMED (ours-first and Rapier-first): %d of %d' % (sum(v['LETTER'] == 'NOT CLAIMED' and v['LETTER_rfirst'] == 'NOT CLAIMED' for v in claims.values()), len(claims)))
out.append('still NOT CLAIMED under EXCL, ours-first (K guard is not the only reason): %d: %s' % (len(nc_ours), nc_ours))
out.append('still NOT CLAIMED under EXCL in BOTH orientations: %d: %s' % (len(nc_both), nc_both))
p = [k for k in claims if k.endswith('[100,500)')]
same = [k for k in p if claims[k]['EXCL'] == claims[k]['EXCL_rfirst']]
out.append('[100,500) EXCL identical in both orientations (strength included): %d of %d' % (len(same), len(p)))
open(os.path.join(HERE, 'v3_disputes.txt'), 'w').write('\n'.join(out) + '\n')
print('\n'.join(out))
