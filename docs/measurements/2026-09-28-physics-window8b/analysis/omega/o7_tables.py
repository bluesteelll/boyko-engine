"""o7: markdown tables for omega.md, from o1_per_process.json and the used omega-wgap processes (no new statistic)."""
import json
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
PER = json.load(open(L.HERE + '/o1_per_process.json'))
for x in PER:
    x['key'] = tuple(x['key'])


def c(k, f):
    return L.cell([f(x) for x in PER if x['key'] == k])


O('| route | P | b | gap us | spin slope med [min-max] n | park slope med [min-max] n | omega_b spin / park (slope - b x work - gap) | active 36/72 (spin; park) | first helper ns 36/72 (spin; park) | parks/region 36/72 (park) |')
O('|---|---|---|---|---|---|---|---|---|---|')
for route in ('worker', 'external'):
    for P, bpp, gap in [(p, b, 0) for p in (2, 4, 8) for b in (1, 2, 4)] + [(8, 4, 20), (8, 4, 80)]:
        ks, kp = (route, P, bpp, 'spin', gap), (route, P, bpp, 'park', gap)
        s, p = c(ks, lambda x: x['slope']), c(kp, lambda x: x['slope'])
        os_, op = c(ks, lambda x: x['overhead']), c(kp, lambda x: x['overhead'])
        a = [c(k, lambda x, n=n: x[n])['median'] for k in (ks, kp) for n in ('act36', 'act72')]
        f = [c(k, lambda x, n=n: x[n])['median'] for k in (ks, kp) for n in ('first36', 'first72')]
        pk = [c(kp, lambda x, n=n: x[n]) for n in ('parks36', 'parks72')]
        O('| %s | %d | %dP | %d | %.0f [%.0f-%.0f] %d | %.0f [%.0f-%.0f] %d | %.0f / %.0f | %g/%g; %g/%g | %g/%g; %g/%g | %g [%g-%g] / %g [%g-%g] |' % (
            route, P, bpp, gap, s['median'], s['min'], s['max'], s['K'], p['median'], p['min'], p['max'], p['K'],
            os_['median'], op['median'], a[0], a[1], a[2], a[3], f[0], f[1], f[2], f[3],
            pk[0]['median'], pk[0]['min'], pk[0]['max'], pk[1]['median'], pk[1]['min'], pk[1]['max']))
O()
procs = L.all_procs()
used, _, _ = L.select(procs)
O('| route | W | gap us | scope ns med [IQR, SE, min-max] n | helped_reps of 2000 [min-max] | first_helper_ns_median med [min-max] |')
O('|---|---|---|---|---|---|')
for route in ('worker', 'external'):
    for W in (8, 16):
        for gap in (0, 5, 20, 80):
            ss = [s for p in used if p['row'] == 'omega-wgap-' + route for s in p['summaries'] if s['workers'] == W and s['gap_us'] == gap]
            sc = L.cell([s['scope_ns_median'] for s in ss])
            hr = [s['helped_reps'] for s in ss]
            fh = L.cell([s['first_helper_ns_median'] for s in ss if s['first_helper_ns_median'] is not None])
            O('| %s | %d | %d | %s | %d-%d | %.0f [%.0f-%.0f] |' % (route, W, gap, L.fc(sc, 0), min(hr), max(hr), fh['median'], fh['min'], fh['max']))
O.save('o7.txt')
