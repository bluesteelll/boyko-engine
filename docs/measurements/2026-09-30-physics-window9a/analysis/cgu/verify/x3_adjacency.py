"""Verifier: adjacency of used pairs, order/position effect, receipt-median definitions, idle waits (post hoc)."""
import io, contextlib, json, math, os, sys, collections, datetime
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
with contextlib.redirect_stdout(io.StringIO()):
    import x1_recompute as X1
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a); OUT.append(s); print(s)
used, info, procs, warm = X1.used, X1.info, X1.procs, X1.warm
ts = datetime.datetime.fromisoformat
def mval(x, a0=100, b0=500):
    return sum(info[id(x)][2][a0:b0]) / (b0 - a0) / 1e6
pairs = collections.defaultdict(dict)
for x in used:
    pairs[(x['pass'], x['round'], x['W'])][x['binary']] = x
P('== pairs with a re-run member: executed neighbours ==')
allp = procs + warm
for k, d in sorted(pairs.items()):
    a, b = d['tip'], d['tipcgu1']
    if a['attempt'] == 'original' and b['attempt'] == 'original':
        continue
    ex = sorted([r for r in allp if r['pass'] == k[0]], key=lambda r: ts(r['start']))
    ia, ib = ex.index(a), ex.index(b)
    gap = abs((ts(a['start']) - ts(b['start'])).total_seconds())
    P(f"p{k[0]} r{k[1]} W{k[2]:<2} tip {a['attempt']}{a.get('rerun_no', 0)} cgu1 {b['attempt']}{b.get('rerun_no', 0)} executed-index distance {abs(ia - ib)} start gap {gap:.1f} s")
gaps_orig = [abs((ts(d['tip']['start']) - ts(d['tipcgu1']['start'])).total_seconds()) for d in pairs.values()
             if d['tip']['attempt'] == 'original' and d['tipcgu1']['attempt'] == 'original']
P('original-original used pairs:', len(gaps_orig), 'start gap s min/median/max', round(min(gaps_orig), 1), round(X1.med(gaps_orig), 1), round(max(gaps_orig), 1))

P('')
P('== position effect, original-original used pairs, [100,500) (post hoc) ==')
cf, tf = [], []
for k, d in pairs.items():
    a, b = d['tip'], d['tipcgu1']
    if a['attempt'] != 'original' or b['attempt'] != 'original':
        continue
    r = mval(b) / mval(a)
    (cf if b['seq'] < a['seq'] else tf).append(r)
P('cgu1-first n', len(cf), 'median', round(X1.med(cf), 4), 'mean', round(sum(cf) / len(cf), 4), '| tip-first n', len(tf), 'median', round(X1.med(tf), 4), 'mean', round(sum(tf) / len(tf), 4))
pos_med = (X1.med(tf) - X1.med(cf)) / 2
pos_mean = (sum(tf) / len(tf) - sum(cf) / len(cf)) / 2
P(f'position effect (second slower by) median-based {100*pos_med:+.2f} %, mean-based {100*pos_mean:+.2f} %; imbalance shift = (30-15)/45 x pos = {100*pos_mean/3:+.3f} % (mean-based)')

P('')
P('== receipt medians, used processes ==')
for p in (0, 1, 2):
    u = [r for r in used if r['pass'] == p]
    b = [r['receipt_before']['cpu_avg'] for r in u]; a = [r['receipt_after']['cpu_avg'] for r in u]
    P(f'p{p} before {X1.med(b):.2f} after {X1.med(a):.2f} combined {X1.med(b + a):.2f} mean-of-pair {X1.med([(x + y) / 2 for x, y in zip(b, a)]):.2f} max-of-pair {X1.med([max(x, y) for x, y in zip(b, a)]):.2f} witness {X1.med([r["others_busy_pct"] for r in u]):.2f}')
P('pooled before', X1.med([r['receipt_before']['cpu_avg'] for r in used]), 'after', X1.med([r['receipt_after']['cpu_avg'] for r in used]))
for W in (1, 2, 4, 8, 16):
    u = [r for r in used if r['W'] == W]
    P(f'W{W} before {X1.med([r["receipt_before"]["cpu_avg"] for r in u]):.2f} after {X1.med([r["receipt_after"]["cpu_avg"] for r in u]):.2f} witness {X1.med([r["others_busy_pct"] for r in u]):.2f}')
for b in ('tip', 'tipcgu1'):
    u = [r for r in used if r['binary'] == b]
    P(f'{b} before {X1.med([r["receipt_before"]["cpu_avg"] for r in u]):.2f} after {X1.med([r["receipt_after"]["cpu_avg"] for r in u]):.2f} witness {X1.med([r["others_busy_pct"] for r in u]):.2f}')
P('')
P('== waited_s on the first record of each pass ==')
for p in (0, 1, 2):
    ex = sorted([r for r in allp if r['pass'] == p], key=lambda r: ts(r['start']))
    P(f'p{p} first', ex[0]['attempt'], ex[0]['binary'], 'W', ex[0]['W'], 'waited_s', ex[0].get('waited_s'), 'start', ex[0]['start'])
open(os.path.join(HERE, 'x3_adjacency.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
