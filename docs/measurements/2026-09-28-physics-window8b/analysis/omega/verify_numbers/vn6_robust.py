"""Robustness (post hoc): PARK verdicts on region_ns_mean instead of region_ns_median; 100-ns quantum census; pass order."""
import sys
sys.dont_write_bytecode = True
import vn_lib as L

procs = L.load_procs()
slots, used, dropped = L.select(procs)


def summ_of(p, helper, P, b, gap, st):
    for s in p['_summ']:
        if (s['helper'], s['participants'], s['blocks_per_participant'], s['gap_us'], s['stages']) == (helper, P, b, gap, st):
            return s


def slope(p, helper, P, b, gap, key):
    return (summ_of(p, helper, P, b, gap, 72)[key] - summ_of(p, helper, P, b, gap, 36)[key]) / 36.0


diff = []
for key in ('region_ns_median', 'region_ns_mean'):
    out = {}
    for gap in (0, 20, 80):
        for route in ('worker', 'external'):
            ps = [p for p in used if p['row'] == ('omega2-' + route if gap == 0 else 'omega2-gap-' + route)]
            for P in ((2, 4, 8) if gap == 0 else (8,)):
                for b in ((1, 2, 4) if gap == 0 else (4,)):
                    A = L.by_pass(ps, lambda p: slope(p, 'spin', P, b, gap, key))
                    B = L.by_pass(ps, lambda p: slope(p, 'park', P, b, gap, key))
                    out[(gap, route, P, b)] = (L.ruling1(A, B, True)[1], L.ruling1(A, B, False)[1],
                                               B['pooled']['med'] - A['pooled']['med'])
    if key == 'region_ns_median':
        base = out
    else:
        for k in base:
            if base[k][:2] != out[k][:2]:
                diff.append((k, base[k], out[k]))
print('PARK verdicts that change when region_ns_mean replaces region_ns_median:', len(diff))
for d in diff:
    print('  ', d)
n = 0
bad = 0
for p in used:
    for s in p['_summ']:
        for k in ('region_ns_median', 'scope_ns_median'):
            if k in s:
                n += 1
                bad += (s[k] % 100) != 0
print('used-process region/scope medians:', n, 'not a multiple of 100 ns:', bad)
for b in ('omega-v2',):
    for ps in (0, 1, 2):
        seq = [p['row'] for p in sorted([p for p in procs if p['block'] == b and p['pass'] == ps and p['attempt'] == 'original'], key=lambda p: p['seq'])]
        print('pass', ps, 'round-0 order:', seq[:6])
