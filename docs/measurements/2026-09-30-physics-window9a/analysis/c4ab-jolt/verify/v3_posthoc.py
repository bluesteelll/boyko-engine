"""v3: post hoc checks - witness medians, re-run bias, Jolt W16 modes, paired rounds, scaling flags."""
import collections, statistics
import vlib as L

o = L.Out()
recs = L.records()
procs, used, dropped, slots = L.select('C4-AB', recs)
for ps in (0, 1, 2):
    allp = [p['rec'].get('others_busy_pct') for p in procs if p['pass_'] == ps]
    o(f'p{ps}: witness median over ALL processes {statistics.median(allp):.2f} % (n={len(allp)})')
o(f"re-run waves p0: {collections.Counter(p['rerun_no'] for p in procs if p['pass_'] == 0 and p['attempt'] == 'rerun')}")
o(f"re-run waves p2: {collections.Counter(p['rerun_no'] for p in procs if p['pass_'] == 2 and p['attempt'] == 'rerun')}")
o(f"p0 slots with unclean original: {sum(1 for k, ps in slots.items() if k[0] == 0 and not [p for p in ps if p['attempt'] == 'original'][0]['clean'])}")


def key(p):
    return (p['row'], p['binary'], p['W'])


orig_by = collections.defaultdict(list)
for p in used:
    if p['attempt'] == 'original':
        orig_by[key(p)].append(p)
rr = []
for p in used:
    if p['attempt'] == 'rerun':
        m = statistics.median(q['m0_500'] for q in orig_by[key(p)])
        rr.append((p['m0_500'] / m, p['pass_']))
o(f're-runs used {len(rr)}: median ratio to cell originals {statistics.median(x for x, _ in rr):.4f}; above {sum(1 for x, _ in rr if x > 1)}')
for ps in (0, 1, 2):
    xs = [x for x, q in rr if q == ps]
    if xs:
        o(f'   p{ps}: n={len(xs)} median {statistics.median(xs):.4f}')
loo = []
for k, ps in orig_by.items():
    for i, p in enumerate(ps):
        rest = [q['m0_500'] for j, q in enumerate(ps) if j != i]
        if rest:
            loo.append(p['m0_500'] / statistics.median(rest))
o(f'control originals leave-one-out: n={len(loo)} median {statistics.median(loo):.4f}; above {sum(1 for x in loo if x > 1)}')
j16 = sorted((p['m0_500'], p['pass_'], p['attempt']) for p in used if p['row'] == 'C4-jolt56' and p['W'] == 16)
o(f'Jolt W16 used: {[(round(a, 4), b, c) for a, b, c in j16]}')
# paired rounds: Jolt and JD#tip in the same pass and round, both originals used, adjacent seq
o('paired rounds (JD#tip vs jolt56, same pass+round, both originals used, |seq diff|==1):')
for W in (1, 2, 4, 8, 16):
    jd = {(p['pass_'], p['round']): p for p in used if p['row'] == 'C4-JD' and p['binary'] == 'tip' and p['W'] == W and p['attempt'] == 'original'}
    jo = {(p['pass_'], p['round']): p for p in used if p['row'] == 'C4-jolt56' and p['W'] == W and p['attempt'] == 'original'}
    pr = [(jd[k]['m0_500'] / jo[k]['m0_500'], abs(jd[k]['seq'] - jo[k]['seq'])) for k in jd if k in jo]
    adj = [x for x, d in pr if d == 1]
    o(f'  W{W}: pairs {len(pr)} (adjacent {len(adj)}); ours faster {sum(1 for x in adj if x < 1)}/{len(adj)}; median {statistics.median(adj):.3f}')
# scaling T16 vs T8 ours, jolt
for row, b in (('C4-JD', 'tip'), ('C4-jolt56', 'j56')):
    va, vb = {}, {}
    for p in used:
        if p['row'] == row and p['binary'] == b and p['W'] == 8:
            va.setdefault(p['pass_'], []).append(p['m0_500'])
        if p['row'] == row and p['binary'] == b and p['W'] == 16:
            vb.setdefault(p['pass_'], []).append(p['m0_500'])
    o(f'T16 vs T8 {row}: {L.fj(L.judge(va, vb))}')
o.save('v3_posthoc.txt')
