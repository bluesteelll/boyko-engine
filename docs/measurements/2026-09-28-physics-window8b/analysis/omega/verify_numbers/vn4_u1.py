"""U1 sensitivity for PARK-20/80: pass-0 K=2 margins; the dropped slots restored (original / re-run); guard on/off."""
import sys
sys.dont_write_bytecode = True
import vn_lib as L

procs = L.load_procs()
slots, used, dropped = L.select(procs)
dropped_v2 = {k: ps for k, ps in dropped if k[0] == 'omega-v2'}


def summ_of(p, helper, gap, st):
    for s in p['_summ']:
        if (s['helper'], s['participants'], s['blocks_per_participant'], s['gap_us'], s['stages']) == (helper, 8, 4, gap, st):
            return s


def slope(p, helper, gap):
    return (summ_of(p, helper, gap, 72)['region_ns_median'] - summ_of(p, helper, gap, 36)['region_ns_median']) / 36.0


for variant in ('as selected', 'restore original', 'restore re-run'):
    group = list(used)
    if variant != 'as selected':
        for k, ps in dropped_v2.items():
            want = 'original' if variant == 'restore original' else 'rerun'
            group.append([p for p in ps if p['attempt'] == want][0])
    for gap in (20, 80):
        for route in ('worker', 'external'):
            ps = [p for p in group if p['row'] == 'omega2-gap-' + route]
            A = L.by_pass(ps, lambda p: slope(p, 'spin', gap))
            B = L.by_pass(ps, lambda p: slope(p, 'park', gap))
            rg, vg = L.ruling1(A, B, True)
            rn, vn = L.ruling1(A, B, False)
            p0 = rn['p0']
            print('%-16s gap %d %-8s: K %s | p0 e %.3f bars r/i/s %.3f/%.3f/%.3f | pooled %s p0 %s p1 %s p2 %s | guard %s | no guard %s | delta %+.0f'
                  % (variant, gap, route, [A[k]['K'] for k in ('p0', 'p1', 'p2', 'pooled')], p0['e'], p0['bar_r'],
                     p0['bar_i'], p0['bar_s'], L.flags(rn['pooled']), L.flags(rn['p0']), L.flags(rn['p1']),
                     L.flags(rn['p2']), vg, vn, rn['pooled']['delta']))
print()
print('dropped omega-v2 slots and their processes (slope spin/park at gap 20, 80):')
for k, ps in dropped_v2.items():
    for p in ps:
        print(' ', k, p['attempt'], p['_clean'], ['%.0f/%.0f' % (slope(p, 'spin', g), slope(p, 'park', g)) for g in (20, 80)])
