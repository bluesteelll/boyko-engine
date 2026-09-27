from common8 import *

OUT.clear()
P('# Q5 micro: omega_b (region protocol) and omega(W, gap); cell = median over K=6 processes of each SUMMARY statistic')


def mcell(row, pred, key):
    xs = []
    for p in PROCS:
        if p['row'] != row:
            continue
        for s in p['summaries']:
            if pred(s):
                xs.append(s[key])
    return L.cell(xs)


for route in ('worker', 'external'):
    P(f'\n## omega_b, route {route}')
    for n in (2, 4, 8, 16):
        c36 = mcell(f'omega-b-s36-{route}', lambda s: s['participants'] == n, 'region_ns_median')
        c72 = mcell(f'omega-b-s72-{route}', lambda s: s['participants'] == n, 'region_ns_median')
        m36 = mcell(f'omega-b-s36-{route}', lambda s: s['participants'] == n, 'region_ns_mean')
        m72 = mcell(f'omega-b-s72-{route}', lambda s: s['participants'] == n, 'region_ns_mean')
        slope = (c72['median'] - c36['median']) / 36
        inter = c36['median'] - 36 * slope
        slope_lo = (c72['min'] - c36['max']) / 36
        slope_hi = (c72['max'] - c36['min']) / 36
        # per-process pairing by (pass, round)
        pp = {}
        for p in PROCS:
            if p['row'] in (f'omega-b-s36-{route}', f'omega-b-s72-{route}'):
                for s in p['summaries']:
                    if s['participants'] == n:
                        pp.setdefault((p['pass'], p['round']), {})[s['stages']] = s['region_ns_median']
        sl = L.cell([(v[72] - v[36]) / 36 for v in pp.values() if 36 in v and 72 in v])
        P(f'  P{n}: region36 {L.fc(c36, 0)}; region72 {L.fc(c72, 0)} ns')
        P(f'       slope (72-36)/36 = {slope:.1f} ns/stage [worst pairing {slope_lo:.1f} .. {slope_hi:.1f}]; per-round paired slope {L.fc(sl, 1)}; intercept (region cost at 0 stages) {inter:.0f} ns;'
          f' stage_ns_median(36) as printed {c36["median"] / 36:.1f}; means: 36 {m36["median"]:.0f} 72 {m72["median"]:.0f} -> slope(mean) {(m72["median"] - m36["median"]) / 36:.1f}')
for route in ('worker', 'external'):
    P(f'\n## omega(W, gap), route {route}: zero-work scope of 32 tasks after a gap; scope_ns_median (ns)')
    for Wk in (8, 16):
        cs = {}
        for gap in (0, 5, 20, 80):
            cs[gap] = mcell(f'omega-{route}', lambda s: s['workers'] == Wk and s['gap_us'] == gap, 'scope_ns_median')
            mm = mcell(f'omega-{route}', lambda s: s['workers'] == Wk and s['gap_us'] == gap, 'scope_ns_mean')
            P(f'  W{Wk} gap {gap:2d} us: median {L.fc(cs[gap], 0)}; mean-of-reps cell {mm["median"]:.0f}')
        d = L.cmp_(cs[0], cs[80])
        P(f'  W{Wk}: omega(80) - omega(0) = {cs[80]["median"] - cs[0]["median"]:+.0f} ns ({L.yn(d)}); omega(20)-omega(5) = {cs[20]["median"] - cs[5]["median"]:+.0f}; min over gaps {min(c["median"] for c in cs.values()):.0f}')
save('q5.txt')
