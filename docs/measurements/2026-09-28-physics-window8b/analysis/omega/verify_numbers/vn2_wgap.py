"""omega(W, gap) with the N4 receipt, from raw/: per-process scope_ns_median, cell = median; ruling 1 per pass + pooled."""
import statistics
import sys
sys.dont_write_bytecode = True
import vn_lib as L

procs = L.load_procs()
slots, used, dropped = L.select(procs)


def val(p, W, gap, key='scope_ns_median'):
    for s in p['_summ']:
        if s['workers'] == W and s['gap_us'] == gap:
            return s[key]
    return None


print('## cells (pooled) and receipts')
for route in ('worker', 'external'):
    ps = [p for p in used if p['row'] == 'omega-wgap-' + route]
    for W in (8, 16):
        for gap in (0, 5, 20, 80):
            c = L.cell([val(p, W, gap) for p in ps])
            hr = [val(p, W, gap, 'helped_reps') for p in ps]
            fh = [val(p, W, gap, 'first_helper_ns_median') for p in ps]
            mm = L.cell([val(p, W, gap, 'scope_ns_mean') for p in ps])
            print('  %-8s W%-2d gap %2d: %s | helped %s-%s | first_helper med %s [%s-%s] | mean-cell %.0f'
                  % (route, W, gap, L.fcell(c), min(hr), max(hr), statistics.median(fh), min(fh), max(fh), mm['med']))
print()
print('## comparisons (B vs A), ruling 1')
for route in ('worker', 'external'):
    ps = [p for p in used if p['row'] == 'omega-wgap-' + route]
    for W in (8, 16):
        for ga, gb, tag in ((0, 80, 'N4 omega(80)-omega(0)'), (5, 80, 'AW omega(80)-omega(5)'),
                            (20, 80, 'omega(80)-omega(20)'), (0, 5, 'omega(5)-omega(0)'), (5, 20, 'omega(20)-omega(5)')):
            A = L.by_pass(ps, lambda p: val(p, W, ga))
            B = L.by_pass(ps, lambda p: val(p, W, gb))
            res_g, v_g = L.ruling1(A, B, True)
            res_n, v_n = L.ruling1(A, B, False)
            d = res_n['pooled']
            print('  %-8s W%-2d %-22s: %.0f -> %.0f = %+.0f (%+.1f %%) | p0 %s p1 %s p2 %s pooled %s | K %s | %s (no guard %s)'
                  % (route, W, tag, A['pooled']['med'], B['pooled']['med'], d['delta'], 100 * (d['ratio'] - 1),
                     L.flags(res_n['p0']), L.flags(res_n['p1']), L.flags(res_n['p2']), L.flags(res_n['pooled']),
                     [A[k]['K'] for k in ('p0', 'p1', 'p2', 'pooled')], v_g, v_n))
        # first helper 5 -> 80
        A = L.by_pass(ps, lambda p: val(p, W, 5, 'first_helper_ns_median'))
        B = L.by_pass(ps, lambda p: val(p, W, 80, 'first_helper_ns_median'))
        res_g, v_g = L.ruling1(A, B, True)
        print('  %-8s W%-2d first_helper 5->80: %.0f -> %.0f | pooled %s | %s (post hoc)'
              % (route, W, A['pooled']['med'], B['pooled']['med'], L.flags(res_g['pooled']), v_g))
