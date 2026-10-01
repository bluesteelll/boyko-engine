"""C4-CGU POST HOC: the W8 / W16 pairs one by one (PRIMARY window), to see whether the two-level spread at W8 is a
machine state shared by both members of a pair (then the paired ratio is tight) or per process. Decides nothing."""
import statistics
from collections import defaultdict

import cgu_lib as C

o = C.Out()
recs = C.all_records()
_, used, _ = C.select(C.BLOCK, recs, C.RUN_CGU, row=C.ROW)
pairs = defaultdict(dict)
for p in used:
    pairs[(p['pass'], p['rec']['round'], p['W'])][p['binary']] = p
for W in C.WS:
    o(f'W{W} (pass, round): cgu16 ms, cgu1 ms, cgu1/cgu16, attempts')
    rs = []
    for (ps, rd, w), d in sorted(pairs.items()):
        if w != W:
            continue
        a, b = d[C.A_KEY], d[C.B_KEY]
        r = b['v'][C.PRIMARY] / a['v'][C.PRIMARY]
        rs.append(r)
        if W in (8, 16):
            o(f'  p{ps} r{rd}: {a["v"][C.PRIMARY]:.4f} {b["v"][C.PRIMARY]:.4f} {r:.4f} {a["attempt"][0]}/{b["attempt"][0]}')
    q = statistics.quantiles(rs, n=4, method='inclusive')
    o(f'  paired ratio: median {statistics.median(rs):.4f}, IQR {100*(q[2]-q[0]):.2f} %, min {min(rs):.4f}, max {max(rs):.4f} (n={len(rs)})')
o.save('c4_pairs.txt')
