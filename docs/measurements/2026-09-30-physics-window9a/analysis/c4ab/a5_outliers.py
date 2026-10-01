"""Post hoc: the used processes of the cells with the widest spreads (Jolt W16, Jolt W1, ours J-D W1/W8/W16), each with
its pass, round, attempt, witness, receipts and wall; plus the p0 re-run budget (records only, no timing re-read)."""
import lib9a as L

O = L.Out()
recs = L.all_records()
_, used, dropped = L.select('C4-AB', recs)
for (row, W) in (('C4-jolt56', 16), ('C4-jolt56', 1), ('C4-jolt56', 8), ('C4-JD', 1), ('C4-JD', 8), ('C4-JD', 16)):
    O(f'\n## {row} W{W} (used)')
    for p in sorted([p for p in used if p['row'] == row and p['W'] == W], key=lambda p: p['v']['0..500']):
        r = p['rec']
        O(f"  {p['v']['0..500']:.4f} ms  p{p['pass']} r{p['round']} {p['attempt']}{p['rerun_no'] or ''} "
          f"witness {r.get('others_busy_pct')} % before {r['receipt_before']['cpu_avg']} after {r['receipt_after']['cpu_avg']} "
          f"start {r['start'][11:19]}")
O('\n## dropped C4-AB slots by pass')
from collections import Counter
O(str(Counter(k[1] for k, _ in dropped)))
O.save('a5_outliers.txt')
