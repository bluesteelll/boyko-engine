"""Placement receipts at W1 and the TA-leaflist outlier (post hoc checks of the analyst's anomalies)."""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

P = [p for p in json.load(open(os.path.join(L.HERE, 'v_procs.json')))['procs'] if p.get('used')]
w1 = [p for p in P if p['W'] == 1]
print('used W1 processes:', len(w1))
est = [p['placement'].get('main_share_top_est') for p in w1]
print('main_share_top_est values:', sorted(set(est)))
top = [p['placement']['top3'][0][1] for p in w1]
print('top-CPU busy % at W1: min', min(top), 'max', max(top))
low = min(w1, key=lambda p: p['placement']['top3'][0][1])
print('lowest share process:', low['row'], low['pass'], low['round'], low['attempt'], low['placement']['top3'][0])
for row in ('F3-TA-armed-leaflist',):
    ps = [p for p in w1 if p['row'] == row]
    for nm in ('t_q_us', 't_b_us', 't_qb_us', 'four_us'):
        m = statistics.median(p[nm] for p in ps)
        mx = max(ps, key=lambda p: p[nm])
        print(f"  {row} {nm}: median {m:.2f} max {mx[nm]:.2f} ({100*(mx[nm]/m-1):+.2f} %) at p{mx['pass']} r{mx['round']} top {mx['placement']['top3'][0]}")
    mw = statistics.median(p['wall_ms']['0..500'] for p in ps)
    mx = max(ps, key=lambda p: p['wall_ms']['0..500'])
    print(f"  {row} wall: median {mw:.4f} max {mx['wall_ms']['0..500']:.4f} ({100*(mx['wall_ms']['0..500']/mw-1):+.2f} %) p{mx['pass']} r{mx['round']}")
# per-process mean vs median agreement for armed rows (post hoc)
for W in (1, 8):
    for nm in ('t_q', 't_qb'):
        a = [p for p in P if p['row'] == 'F3-TD-armed-leaflist' and p['W'] == W]
        b = [p for p in P if p['row'] == 'F3-TD-armed-kd' and p['W'] == W]
        ca, cb = L.cell([p[nm + '_mean_us'] for p in a]), L.cell([p[nm + '_mean_us'] for p in b])
        c = L.cmp(ca, cb)
        print(f"  post hoc, process MEAN statistic, W{W} {nm}: leaflist {ca['m']:.2f} kd {cb['m']:.2f} ratio {c['ratio']:.4f} {L.yn(c)}")
