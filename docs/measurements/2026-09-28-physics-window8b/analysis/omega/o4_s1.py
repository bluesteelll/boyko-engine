"""o4: S1 inputs (arith., estimates; not a decision). From o1_per_process.json (8b, gap 0 rows):
(1) per-process least-squares fit slope(bpp) = a + b * bpp over bpp 1/2/4 at each (route, P, helper): a = the fixed
    per-stage cost, b = the per-block-per-participant cost; b - work_ns = per-block inflation over the calibrated work.
(2) omega_b as the design uses it (stage cost beyond ideal parallel work) = slope - bpp * work_ns (o1 'overhead').
(3) window 8's S1 arithmetic (analysis.md section 5) re-evaluated at these omega_b values: wide (3.30 - 0.3 - w - 0.7) x 96,
    S2 = 0.304 x 0.745 - 36.44 w, S3 = (0.1216 - 0.0027) x 0.745 - 9 w, narrow 0; T(8) = 2.0357 ms (window 8 J-T A).
(4) S2i (warm apply on today's scope path, W8 ruling 6) = 0.304 x 0.745 - 36.44 x omega(8) with omega(8) = 3.30 us."""
import json
import statistics
import sys

sys.dont_write_bytecode = True
import lib_om as L

PER = json.load(open(L.HERE + '/o1_per_process.json'))
for x in PER:
    x['key'] = tuple(x['key'])
O = L.Out()
O('# o4 S1 inputs (arith.); ns unless stated')
O('\n## (1) per-process fit slope(bpp) = a + b*bpp over bpp 1/2/4, gap 0 (post hoc decomposition)')
fits = {}
for route in ('worker', 'external'):
    for P in (2, 4, 8):
        for helper in ('spin', 'park'):
            by = {}
            for x in PER:
                r, p, bpp, h, g = x['key']
                if r == route and p == P and h == helper and g == 0:
                    by.setdefault((x['row'], x['pass'], x['round']), {})[bpp] = (x['slope'], x['work_ns'])
            aa, bb, infl = [], [], []
            for v in by.values():
                if len(v) != 3:
                    continue
                xs = [1, 2, 4]
                ys = [v[k][0] for k in xs]
                mx, my = statistics.mean(xs), statistics.mean(ys)
                b = sum((xi - mx) * (yi - my) for xi, yi in zip(xs, ys)) / sum((xi - mx) ** 2 for xi in xs)
                aa.append(my - b * mx)
                bb.append(b)
                infl.append(b - v[1][1])
            ca, cb, ci = L.cell(aa), L.cell(bb), L.cell(infl)
            fits[(route, P, helper)] = (ca, cb, ci)
            O('%-8s P%d %-4s a (fixed per stage) %s | b (per block per participant) %s | b - work %s' % (
                route, P, helper, L.fc(ca, 0), L.fc(cb, 0), L.fc(ci, 0)))


def cell(route, P, bpp, helper, f):
    return L.cell([f(x) for x in PER if x['key'] == (route, P, bpp, helper, 0)])


O('\n## (2) omega_b = slope - bpp x work_ns at P8, gap 0 (cell over K = 9), and the raw slope')
W = {}
for route in ('worker', 'external'):
    for helper in ('spin', 'park'):
        for bpp in (1, 2, 4):
            co = cell(route, 8, bpp, helper, lambda x: x['overhead'])
            cs = cell(route, 8, bpp, helper, lambda x: x['slope'])
            W[(route, helper, bpp)] = (co['median'], cs['median'])
            O('%-8s %-4s b=%dP: slope %s | omega_b %s' % (route, helper, bpp, L.fc(cs, 0), L.fc(co, 0)))
wk = statistics.median(x['work_ns'] for x in PER)
O('work_ns_calibrated median over used processes: %.1f ns (target 700)' % wk)


def bundle(w_us):
    wide = (3.30 - 0.3 - w_us - 0.7) * 96 / 1000.0
    s2 = 0.304 * 0.745 - 36.44 * w_us / 1000.0
    s3 = (0.1216 - 0.0027) * 0.745 - 9 * w_us / 1000.0
    tot = wide + s2 + s3
    return wide, s2, s3, tot, 100 * tot / 2.0357


O('\n## (3) window 8 section-5 arithmetic re-evaluated (ms; T(8) = 2.0357 ms); w = omega_b in us')
O('check: at w = 3.0 (window 8 v1 worker) -> wide %.3f S2 %.3f S3 %.3f bundle %.3f = %.1f %% (window 8 printed ~ -0.07 / 0.12 / 0.06 / 0.11-0.14 = 5.5-6.8 %%)' % bundle(3.0))
rows = []
for label, sel in (('raw slope b=4P (the pre-registered S1 input, includes 4 blocks of work)', lambda k, v: k[2] == 4 and v[1]),
                   ('omega_b = slope - bpp*work, b=4P', lambda k, v: k[2] == 4 and v[0]),
                   ('omega_b = slope - bpp*work, b=2P', lambda k, v: k[2] == 2 and v[0]),
                   ('omega_b = slope - bpp*work, b=1P', lambda k, v: k[2] == 1 and v[0])):
    ws = [sel(k, v) / 1000.0 for k, v in W.items() if sel(k, v)]
    lo, hi = min(ws), max(ws)
    b_lo, b_hi = bundle(hi), bundle(lo)
    O('%s: w %.3f-%.3f us -> wide %.3f..%.3f, S2 %.3f..%.3f, S3 %.3f..%.3f, bundle %.3f..%.3f ms = %.1f-%.1f %% of T(8)' % (
        label, lo, hi, b_lo[0], b_hi[0], b_lo[1], b_hi[1], b_lo[2], b_hi[2], b_lo[3], b_hi[3], b_lo[4], b_hi[4]))
O('build-if line (design: bundle >= 5 %% of T(8)) = %.3f ms' % (0.05 * 2.0357))
O('omega_b at which the bundle equals 5 %% of T(8): solve -> w = %.3f us' % (
    ((3.30 - 0.3 - 0.7) * 96 / 1000.0 + 0.304 * 0.745 + (0.1216 - 0.0027) * 0.745 - 0.05 * 2.0357) / ((96 + 36.44 + 9) / 1000.0)))

O('\n## (4) stage work the region would replace (window 8, W8, J-T) vs the per-stage cost measured here')
for name, ms, stages in (('warm apply (S2)', 0.304, 36.44), ('integrate group (S3)', 0.1216 - 0.0027, 9)):
    per = ms * 1000 / stages
    O('%s: %.3f ms over %.2f stages = %.2f us serial per stage = %.2f us per participant at P8 = %.1f blocks of %.0f ns' % (
        name, ms, stages, per, per / 8, per / 8 / (wk / 1000.0), wk))
s2i = 0.304 * 0.745 - 36.44 * 3.30 / 1000.0
O('S2i (warm apply on today\'s scope path, omega(8) = 3.30 us per wave, window 8): 0.304 x 0.745 - 36.44 x 3.30 us = %.3f ms' % s2i)
for bpp in (1, 2, 4):
    ws = [v[0] / 1000.0 for k, v in W.items() if k[2] == bpp]
    O('S2 on S1 at omega_b(b=%dP) %.3f-%.3f us: %.3f-%.3f ms (S2 - S2i = %+.3f..%+.3f ms)' % (
        bpp, min(ws), max(ws), 0.304 * 0.745 - 36.44 * max(ws) / 1000.0, 0.304 * 0.745 - 36.44 * min(ws) / 1000.0,
        0.304 * 0.745 - 36.44 * max(ws) / 1000.0 - s2i, 0.304 * 0.745 - 36.44 * min(ws) / 1000.0 - s2i))
O(chr(10) + '## (5) ruling 6 (09-26 owner value: spin vs spin-then-park), from o1 / o2')
O('back-to-back stages (gap 0): park price at P8 = park slope - spin slope, NOT CLAIMED on every bpp and route (o1)')
O('after a serial stretch >= 20 us: park price per stage = +6.0..+6.3 us (o1, P8 b=4P, both routes, gaps 20 and 80); the pool idle')
O('budget (127 PAUSE + 4 yields) expires between 5 and 20 us of idleness (o2: first helper 0.3-0.9 us at gap 0/5,')
O('3.4-3.7 us at gap 20/80). So spin-then-park charges ~6 us per region-internal serial stretch longer than the budget,')
O('and nothing measurable per back-to-back stage; the per-step price is that x the number of such stretches (a design quantity).')
O.save('o4.txt')
