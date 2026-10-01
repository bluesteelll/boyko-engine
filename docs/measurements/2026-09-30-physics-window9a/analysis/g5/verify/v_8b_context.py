"""Check of the analyst's cross-window 8b context (post hoc 2, SPLIT W8 vs W1, the 0.2069 reference) from win8b raw/,
using 8b's own slot selection (synthlib.select) and my own statistics; plus a TWO-CELL comparison of 9a's t_q against
the 8b reference cells (instead of vs_const, which drops the reference cell's spread)."""
import json, math, os, statistics, sys
sys.dont_write_bytecode = True
S = r"C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad"
sys.path.insert(0, S + "/win8b/analysis/synth")
import synthlib as L
O = []
def Q(s=''):
    O.append(str(s)); print(s)
A, B = 100, 500
def iqr(xs):
    q = statistics.quantiles(xs, n=4, method='inclusive'); return q[2] - q[0]
def cell(xs):
    xs = sorted(xs); m = statistics.median(xs)
    return dict(K=len(xs), m=m, mn=xs[0], mx=xs[-1], i=iqr(xs)/m, s=1.2533*statistics.stdev(xs)/math.sqrt(len(xs))/m, r=(xs[-1]-xs[0])/m)
def cmpc(a, b):
    e = abs(b['m']/a['m'] - 1); ok = a['K'] >= 3 and b['K'] >= 3
    return dict(ratio=b['m']/a['m'], f={x: ok and e > 2*math.hypot(a[x], b[x]) for x in 'isr'}, bars={x: 2*math.hypot(a[x], b[x]) for x in 'isr'})
def yn(c): return ''.join('Y' if c['f'][x] else 'n' for x in 'isr')
def r1(pooled, per):
    sg = 1 if pooled['ratio'] > 1 else -1
    same = all((1 if c['ratio'] > 1 else -1) == sg for c in per)
    cl = pooled['f']['i'] and pooled['f']['s'] and all(c['f']['i'] and c['f']['s'] for c in per) and same
    st = cl and pooled['f']['r'] and all(c['f']['r'] for c in per)
    return ('CLAIMED' + (' STRONG' if st else '')) if cl else 'NOT CLAIMED'

recs = L.all_records()
blocks = sorted({r.get('block') for r in recs if r.get('block')})
Q(f"8b blocks: {blocks}")
def used_of(block):
    procs, used, dropped = L.select(block, recs)
    return used
cache = {}
def U(block):
    if block not in cache: cache[block] = used_of(block)
    return cache[block]
def vals(block, row, binary, W, fn, pas=None):
    return [fn(p) for p in U(block) if p['row'] == row and p['binary'] == binary and p['W'] == W and (pas is None or p['pass'] == pas)]
mean_q = lambda p: statistics.fmean(p['c']['phys_bp_query_ns'][A:B]) / 1e6
med_q = lambda p: statistics.median(p['c']['phys_bp_query_ns'][A:B]) / 1e6
share = lambda p: statistics.fmean(p['c']['phys_bp_query_ns'][A:B]) / statistics.fmean(p['c']['wall_ns'][A:B])
def show(block, row, binary, W, fn, lab):
    c = cell(vals(block, row, binary, W, fn))
    Q(f"{block} {row}#{binary}@W{W} {lab}: {c['m']:.4f} [{c['mn']:.4f}-{c['mx']:.4f}] n={c['K']} (i {100*c['i']:.2f} s {100*c['s']:.2f} r {100*c['r']:.2f})")
    return c
def pair(block, row, binary, WA, WB, fn, lab):
    a = cell(vals(block, row, binary, WA, fn)); b = cell(vals(block, row, binary, WB, fn)); pooled = cmpc(a, b)
    per = [cmpc(cell(vals(block, row, binary, WA, fn, q)), cell(vals(block, row, binary, WB, fn, q))) for q in (0, 1, 2)]
    Q(f"  {row}#{binary} W{WB} vs W{WA} ({lab}): {100*(pooled['ratio']-1):+.2f} % pooled {yn(pooled)} passes {[yn(x) for x in per]} -> {r1(pooled, per)}")
for blk, row, b in (('S4-AB', 'S4-JT-a', 'tip'), ('S4-AB', 'S4-JT-a', 'parent'), ('SPLIT', 'SPLIT-J-T-a', 'parent')):
    if blk not in blocks:
        cand = [x for x in blocks if row.split('-')[0] in x]
        Q(f"block {blk} not found; candidates {cand}"); continue
    for W in (1, 8, 16):
        if vals(blk, row, b, W, mean_q):
            show(blk, row, b, W, mean_q, 'mean-over-steps t_q ms'); show(blk, row, b, W, med_q, 'median-over-steps t_q ms'); show(blk, row, b, W, share, 't_q / armed wall')
    pair(blk, row, b, 8, 16, mean_q, 'mean'); pair(blk, row, b, 8, 16, med_q, 'median')
    if vals(blk, row, b, 1, med_q): pair(blk, row, b, 1, 8, med_q, 'median')

Q("\n--- two-cell comparison 9a t_q(W8) vs the 8b reference cells (pooled only; cross-window, cross-binary) ---")
g5 = json.load(open(S + "/win9a/analysis/g5/verify/v_g5_tq.json"))
for lab, key9, blk, row, b, fn in (('mean conv vs 8b S4-JT-a#tip (0.2069)', 'mean', 'S4-AB', 'S4-JT-a', 'tip', mean_q),
                                 ('median conv vs 8b S4-JT-a#tip', 'median', 'S4-AB', 'S4-JT-a', 'tip', med_q),
                                 ('median conv vs 8b F3-TD-armed-leaflist (206.93 us)', 'median', 'F3', 'F3-TD-armed-leaflist', 'tip', med_q)):
    if blk not in blocks:
        Q(f"{lab}: block {blk} missing"); continue
    ref = cell(vals(blk, row, b, 8, fn)); new = cell(g5[key9])
    c = cmpc(ref, new)
    Q(f"{lab}: ref {ref['m']:.5f} (i {100*ref['i']:.2f} s {100*ref['s']:.2f} r {100*ref['r']:.2f}, n {ref['K']}) -> 9a {new['m']:.5f}: {100*(c['ratio']-1):+.2f} % pooled {yn(c)} bars i {100*c['bars']['i']:.2f} s {100*c['bars']['s']:.2f} r {100*c['bars']['r']:.2f}")
open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'v_8b_context.txt'), 'w', encoding='utf-8').write('\n'.join(O) + '\n')
