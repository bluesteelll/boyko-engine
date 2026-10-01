"""S1 arithmetic (not a claim) from MY v2 cells + window 8 / design constants; H1 post-hoc fit check."""
import statistics
import sys
sys.dont_write_bytecode = True
import vn_lib as L

procs = L.load_procs()
slots, used, dropped = L.select(procs)


def summ_of(p, helper, P, b, gap, st):
    for s in p['_summ']:
        if (s['helper'], s['participants'], s['blocks_per_participant'], s['gap_us'], s['stages']) == (helper, P, b, gap, st):
            return s


def slope(p, helper, P, b, gap=0):
    return (summ_of(p, helper, P, b, gap, 72)['region_ns_median'] - summ_of(p, helper, P, b, gap, 36)['region_ns_median']) / 36.0


net = {}
raw = {}
for route in ('worker', 'external'):
    ps = [p for p in used if p['row'] == 'omega2-' + route]
    for helper in ('spin', 'park'):
        for b in (1, 2, 4):
            net[(route, helper, b)] = L.cell([slope(p, helper, 8, b) - b * p['_cal'][0]['work_ns_calibrated'] for p in ps])['med'] / 1000
            raw[(route, helper, b)] = L.cell([slope(p, helper, 8, b) for p in ps])['med'] / 1000
OMEGA8 = 3.30        # us, window 8 section 3 (J-T-a W8 per-wave omega)
WARM = 0.304         # ms, window 8 section 2 warm apply at W8
STAGES_S2 = 36.44    # scale_design_rev: S2 stage count
INTEG = 0.1216 - 0.0027  # ms, scale_design_rev S3 (integrate group less write_back)
STAGES_S3 = 9
F = 0.745            # 1 - 1/(8E), scale_design
T8 = 2.0357          # ms, window 8 section 1 T(8) J-T
for b in (1, 2, 4):
    vals = [net[(r, h, b)] for r in ('worker', 'external') for h in ('spin', 'park')]
    print('b=%dP net us: %.3f-%.3f | omega(8)/net %.2f-%.2fx | / warm-per-participant (%.3f us) %.0f-%.0f %% | / integrate-per-participant (%.3f us) %.0f-%.0f %%'
          % (b, min(vals), max(vals), OMEGA8 / max(vals), OMEGA8 / min(vals), WARM * 1000 / STAGES_S2 / 8,
             100 * min(vals) / (WARM * 1000 / STAGES_S2 / 8), 100 * max(vals) / (WARM * 1000 / STAGES_S2 / 8),
             INTEG * 1000 / STAGES_S3 / 8, 100 * min(vals) / (INTEG * 1000 / STAGES_S3 / 8),
             100 * max(vals) / (INTEG * 1000 / STAGES_S3 / 8)))
allnet = list(net.values())
print('overall net range %.3f-%.3f us; omega(8)/net %.2f-%.2fx' % (min(allnet), max(allnet), OMEGA8 / max(allnet), OMEGA8 / min(allnet)))
print('warm apply per stage %.2f us serial, %.3f us per participant; integrate %.2f us / stage, %.3f per participant'
      % (WARM * 1000 / STAGES_S2, WARM * 1000 / STAGES_S2 / 8, INTEG * 1000 / STAGES_S3, INTEG * 1000 / STAGES_S3 / 8))


def bundle(w_us):
    wide = (OMEGA8 - 0.3 - w_us - 0.7) * 96 / 1000
    s2 = WARM * F - STAGES_S2 * w_us / 1000
    s3 = INTEG * F - STAGES_S3 * w_us / 1000
    return wide + s2 + s3, s2


for label, w in (('w=3.0 (window 8 v1)', 3.0), ('net min', min(allnet)), ('net max', max(allnet))):
    bb, s2 = bundle(w)
    print('%-22s bundle %.4f ms = %.2f %% of T(8); S2 on S1 %.4f ms' % (label, bb, 100 * bb / T8, s2))
rs = [raw[(r, h, 4)] for r in ('worker', 'external') for h in ('spin', 'park')]
for w in (min(rs), max(rs)):
    bb, s2 = bundle(w)
    print('raw slope b=4P w=%.3f: bundle %.4f ms = %.2f %%' % (w, bb, 100 * bb / T8))
lo, hi = 0.0, 10.0
for _ in range(60):
    mid = (lo + hi) / 2
    if bundle(mid)[0] / T8 > 0.05:
        lo = mid
    else:
        hi = mid
print('5 %% build-if crossed at w = %.3f us' % lo)
s2i = WARM * F - STAGES_S2 * OMEGA8 / 1000
print('S2i (warm apply on today scope path) gain %.4f ms; S2-on-S1 minus S2i: %.3f..%.3f ms'
      % (s2i, bundle(max(allnet))[1] - s2i, bundle(min(allnet))[1] - s2i))

print()
print('## H1 (post hoc): per-process least-squares slope_net? fit slope = a + c x bpp over bpp 1,2,4; c - work')
for P in (2, 4, 8):
    for route in ('worker', 'external'):
        ps = [p for p in used if p['row'] == 'omega2-' + route]
        for helper in ('spin', 'park'):
            A, C, X = [], [], []
            for p in ps:
                xs = [1, 2, 4]
                ys = [slope(p, helper, P, b) for b in xs]
                mx, my = statistics.mean(xs), statistics.mean(ys)
                c = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)
                A.append(my - c * mx)
                C.append(c)
                X.append(c - p['_cal'][0]['work_ns_calibrated'])
            print('  P%d %-8s %-4s a %.0f  c %.0f  c-work %.0f (ns, medians)' % (P, route, helper, statistics.median(A),
                                                                             statistics.median(C), statistics.median(X)))
