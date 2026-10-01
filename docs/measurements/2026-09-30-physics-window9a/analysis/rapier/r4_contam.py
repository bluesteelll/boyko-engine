"""r4: who made the unclean processes unclean (top other process of the witness / receipts), per block and pass."""
import collections
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_rp as L  # noqa: E402

o = L.Out()
recs = L.all_records()
for b, rows in (('C4-AB', {'C4-JT', 'C4-JD', 'C4-jolt56'}), ('C4-RAPIER', None)):
    procs, used, dropped = L.select(b, recs, rows)
    for ps in (0, 1, 2):
        un = [p for p in procs if p['pass'] == ps and p['clean_why']]
        top = collections.Counter()
        for p in un:
            r = p['rec']
            t = (r.get('others_top5') or [{}])[0].get('name')
            if any(w.startswith('witness') for w in p['clean_why']) and t:
                top[t] += 1
            for k in ('receipt_before', 'receipt_after'):
                if any(w.startswith(k) for w in p['clean_why']):
                    tt = ((r.get(k) or {}).get('top5') or [{}])[0].get('name')
                    if tt:
                        top[f'{k}:{tt}'] += 1
        o(f'{b}-p{ps}: processes {sum(1 for p in procs if p["pass"] == ps)}, unclean {len(un)}; top offenders {top.most_common(6)}')
o.save('r4_contam.txt')
