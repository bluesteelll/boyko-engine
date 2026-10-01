"""Verifier post hoc checks (labelled post hoc; decide nothing). Imports x1_recompute (own code) for the slot selection."""
import io, contextlib, json, math, os, sys, collections, datetime
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
with contextlib.redirect_stdout(io.StringIO()):
    import x1_recompute as X1
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a); OUT.append(s); print(s)
used, info, procs, warm, recs = X1.used, X1.info, X1.procs, X1.warm, X1.recs
def mval(x, a0, b0):
    return sum(info[id(x)][2][a0:b0]) / (b0 - a0) / 1e6
def ts(s):
    return datetime.datetime.fromisoformat(s)

P('== re-runs ==')
for x in sorted([r for r in procs if r['attempt'] == 'rerun'], key=lambda r: (r['pass'], r['seq'], r['rerun_no'])):
    v, c, _ = info[id(x)]
    P('p%d seq %02d %s W%d rerun_no %d clean=%s used=%s top_other=%s' % (x['pass'], x['seq'], x['binary'], x['W'], x['rerun_no'], not c, x in used,
      (x.get('others_top5') or [{}])[0].get('name')))
trig = [r for r in procs if info[id(r)][1]]
P('unclean attempts (each triggers a re-run):', len(trig), 'witness-unclean among them:', sum(1 for r in trig if any(w.startswith('witness') for w in info[id(r)][1])),
  'top other process names:', dict(collections.Counter((r.get('others_top5') or [{}])[0].get('name') for r in trig)))

pairs = collections.defaultdict(dict)
for x in used:
    pairs[(x['pass'], x['round'], x['W'])][x['binary']] = x
assert len(pairs) == 45 and all(len(v) == 2 for v in pairs.values())
nonadj = []
for k, d in sorted(pairs.items()):
    a, b = d['tip'], d['tipcgu1']
    if a['attempt'] != 'original' or b['attempt'] != 'original':
        gap = abs((ts(a['start']) - ts(b['start'])).total_seconds())
        starts = sorted(ts(r['start']) for r in procs + warm if r['pass'] == k[0])
        lo, hi = sorted([ts(a['start']), ts(b['start'])])
        between = sum(1 for s in starts if lo < s < hi)
        if between > 0:
            nonadj.append((k, round(gap, 1), between))
P('pairs with a re-run member:', sum(1 for d in pairs.values() if any(x['attempt'] != 'original' for x in d.values())),
  ' non-adjacent used pairs:', len(nonadj), ' gaps s min/max:', min(g for _, g, _ in nonadj) if nonadj else '-', max(g for _, g, _ in nonadj) if nonadj else '-')

origs = [r for r in procs if r['attempt'] == 'original']
op = collections.defaultdict(dict)
for r in origs:
    op[(r['pass'], r['round'], r['W'])][r['binary']] = r
first = collections.Counter()
adj = 0
for k, d in op.items():
    a, b = d['tip'], d['tipcgu1']
    adj += abs(a['seq'] - b['seq']) == 1
    first[(k[0], 'cgu1 first' if b['seq'] < a['seq'] else 'tip first')] += 1
P('original pairs adjacent by seq:', adj, 'of', len(op), ' order:', dict(sorted(first.items())))

for wn, (a0, b0) in (('[100,500)', (100, 500)), ('[0,500)', (0, 500))):
    P('')
    P('== paired cgu1/cgu16 per (pass, round, W), window', wn, '==')
    allr = []
    for W in (1, 2, 4, 8, 16):
        rs = [mval(pairs[(p, rd, W)]['tipcgu1'], a0, b0) / mval(pairs[(p, rd, W)]['tip'], a0, b0) for p in (0, 1, 2) for rd in (0, 1, 2)]
        allr += rs
        st = X1.stats(rs)
        nf = sum(1 for r in rs if r < 1)
        p2 = min(1.0, 2 * sum(math.comb(9, k) for k in range(0, min(nf, 9 - nf) + 1)) / 2 ** 9)
        P(f'W{W:<2} paired median {st["m"]:.4f} IQR {100*st["i"]:.2f} % SEmed {100*st["s"]:.2f} % mean {sum(rs)/9:.4f} cgu1 faster {nf}/9 sign p {p2:.3f}  ratios ' + ' '.join(f'{r:.4f}' for r in rs))
    gm = math.exp(sum(math.log(r) for r in allr) / len(allr))
    nf = sum(1 for r in allr if r < 1)
    p2 = min(1.0, 2 * sum(math.comb(45, k) for k in range(0, min(nf, 45 - nf) + 1)) / 2 ** 45)
    P(f'all W: n {len(allr)} geomean {gm:.4f} cgu1 faster {nf}/45 sign p {p2:.3f}')

P('')
P('== W8 / W16 levels, [100,500), per pair (tip/cgu1) ==')
for W in (8, 16):
    for p in (0, 1, 2):
        P(f'W{W} p{p} ' + '  '.join(f"r{rd}: {mval(pairs[(p, rd, W)]['tip'], 100, 500):.4f}/{mval(pairs[(p, rd, W)]['tipcgu1'], 100, 500):.4f}" for rd in (0, 1, 2)))

P('')
P('== re-run bias ([100,500)) ==')
rb_cell, rb_pc = [], []
for x in used:
    if x['attempt'] != 'rerun':
        continue
    cell_orig = [mval(o, 100, 500) for o in used if o['attempt'] == 'original' and o['binary'] == x['binary'] and o['W'] == x['W']]
    pc_orig = [mval(o, 100, 500) for o in used if o['attempt'] == 'original' and o['binary'] == x['binary'] and o['W'] == x['W'] and o['pass'] == x['pass']]
    rb_cell.append(mval(x, 100, 500) / X1.med(cell_orig))
    if pc_orig:
        rb_pc.append(mval(x, 100, 500) / X1.med(pc_orig))
P('vs cell originals: n', len(rb_cell), 'median', round(X1.med(rb_cell), 4), 'above 1:', sum(1 for r in rb_cell if r > 1),
  '| vs pass-cell originals: n', len(rb_pc), 'median', round(X1.med(rb_pc), 4) if rb_pc else '-', 'above 1:', sum(1 for r in rb_pc if r > 1))

P('')
P('== originals-only (post hoc) ==')
oc = [r for r in origs if not info[id(r)][0] and not info[id(r)][1]]
for wn, (a0, b0) in (('[100,500)', (100, 500)), ('[0,500)', (0, 500))):
    outs = []
    for W in (1, 2, 4, 8, 16):
        va = {p: [mval(r, a0, b0) for r in oc if r['binary'] == 'tip' and r['W'] == W and r['pass'] == p] for p in (0, 1, 2)}
        vb = {p: [mval(r, a0, b0) for r in oc if r['binary'] == 'tipcgu1' and r['W'] == W and r['pass'] == p] for p in (0, 1, 2)}
        A = X1.stats(sum(va.values(), [])); B = X1.stats(sum(vb.values(), []))
        pooled = X1.comp(A, B)
        per = [X1.comp(X1.stats(va[p]), X1.stats(vb[p])) if len(va[p]) >= 2 and len(vb[p]) >= 2 else None for p in (0, 1, 2)]
        claimed = pooled['fl']['i'] and pooled['fl']['s'] and all(c is not None and c['fl']['i'] and c['fl']['s'] for c in per)
        outs.append(f"W{W} {pooled['ratio']:.4f} K {[len(va[p]) for p in (0,1,2)]}/{[len(vb[p]) for p in (0,1,2)]} pooled {X1.yn(pooled)} {'CLAIMED' if claimed else 'NOT CLAIMED'}")
    P(wn, ' | '.join(outs))

P('')
P('== timeline / receipts ==')
starts = sorted((r['start'], r['pass'], r['attempt'], r['binary'], r['W']) for r in procs + warm)
P('first process start', starts[0], ' last', starts[-1])
man = json.load(open(os.path.join(X1.RAW, 'manifest_1790815060.json'), encoding='utf-8'))
s = json.dumps(man)
P('manifest has claim sentence:', 'cgu1 claimed faster than cgu16 at W8 AND not claimed slower at any W' in s, ' manifest keys:', list(man)[:15])
for p in (0, 1, 2):
    u = [r for r in used if r['pass'] == p]
    P(f"p{p} used n {len(u)} receipt_before med {X1.med([(r['receipt_before'] or {})['cpu_avg'] for r in u]):.2f} receipt_after med {X1.med([(r['receipt_after'] or {})['cpu_avg'] for r in u]):.2f} witness med {X1.med([r['others_busy_pct'] for r in u]):.2f}")
open(os.path.join(HERE, 'x2_posthoc.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
