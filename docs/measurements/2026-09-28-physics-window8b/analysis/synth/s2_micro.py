"""Synthesis re-derivation of the omega group's S1 inputs: omega_b v2 slopes at P8 (raw and net of block work), the
park price at gap 0/20/80, omega(W, gap) with the N4 receipt (first helper), v1 continuity at P8. Ruling 1 in both
K-guard readings. Per-process slope = (region_ns_median@72 - region_ns_median@36) / 36 (same process); net = slope -
blocks_per_participant x work_ns_calibrated - gap_us x 1000 (the omega group's post hoc omega_b)."""
import collections
import statistics

import synthlib as L

O = L.Out()
recs = L.all_records()
_, used2, _ = L.select('omega-v2', recs)
_, used1, _ = L.select('omega-v1-cont', recs)

SL = collections.defaultdict(lambda: collections.defaultdict(list))   # key -> pass -> [slope]
NET = collections.defaultdict(lambda: collections.defaultdict(list))
FH = collections.defaultdict(lambda: collections.defaultdict(list))   # first helper by (key, stages)
ACT = collections.defaultdict(list)
WG = collections.defaultdict(lambda: collections.defaultdict(list))   # (route, W, gap) -> pass -> [scope ns]
WGF = collections.defaultdict(list)
WORK = []
for p in used2:
    ss = p['ss']
    if p['row'].startswith('omega-wgap'):
        for s in ss:
            WG[(s['route'], s['workers'], s['gap_us'])][p['pass']].append(s['scope_ns_median'])
            WGF[(s['route'], s['workers'], s['gap_us'])].append(s['first_helper_ns_median'])
        continue
    by = {}
    for s in ss:
        k = (s['route'], s['helper'], s['participants'], s['blocks_per_participant'], s['gap_us'])
        by.setdefault(k, {})[s['stages']] = s
        WORK.append(s['work_ns_calibrated'])
    for k, st in by.items():
        sl = (st[72]['region_ns_median'] - st[36]['region_ns_median']) / 36.0
        SL[k][p['pass']].append(sl)
        NET[k][p['pass']].append(sl - k[3] * st[36]['work_ns_calibrated'] - k[4] * 1000.0)
        for n in (36, 72):
            FH[(k, n)][p['pass']].append(st[n]['first_helper_ns_median'])
            ACT[(k, n)].append((st[n]['active_median'], st[n]['all_active_reps']))

O('# omega_b v2 / omega(W, gap) / v1 continuity (synthesis re-derivation), ns')
O(f'used processes: omega-v2 {len(used2)}, omega-v1-cont {len(used1)}; work_ns_calibrated over used SUMMARY lines '
  f'{min(WORK):.1f}-{max(WORK):.1f}')


def pooled(d):
    return L.cell([v for k in sorted(d) for v in d[k]])


O('\n## P8 slopes and net stage cost (cell medians over K; n)')
for route in ('worker', 'external'):
    for b in (1, 2, 4):
        for helper in ('spin', 'park'):
            k = (route, helper, 8, b, 0)
            c, n = pooled(SL[k]), pooled(NET[k])
            O(f'  {route:8s} b={b}P {helper}: slope {c["median"]:.0f} [{c["min"]:.0f}-{c["max"]:.0f}] '
              f'(IQR {c["iqr_abs"]:.0f}, SE {c["se_abs"]:.0f}) n={c["K"]}; net {n["median"]:.0f}')
for b in (1, 2, 4):
    nets = [pooled(NET[(r, h, 8, b, 0)])['median'] for r in ('worker', 'external') for h in ('spin', 'park')]
    O(f'  P8 b={b}P net range over routes x helpers: {min(nets):.0f}-{max(nets):.0f} ns')
raw4 = [pooled(SL[(r, h, 8, 4, 0)])['median'] for r in ('worker', 'external') for h in ('spin', 'park')]
O(f'  P8 b=4P raw slope range: {min(raw4):.0f}-{max(raw4):.0f} ns; 4 x work = {4 * statistics.median(WORK):.0f} ns')

O('\n## park vs spin (B = park, A = spin), ruling 1')
for route in ('worker', 'external'):
    for P, b, g in [(8, bb, 0) for bb in (1, 2, 4)] + [(8, 4, 20), (8, 4, 80)] + \
                   [(4, bb, 0) for bb in (1, 2, 4)] + [(2, bb, 0) for bb in (1, 2, 4)]:
        a, bb_ = SL[(route, 'spin', P, b, g)], SL[(route, 'park', P, b, g)]
        res = {}
        for rd, km in (('LETTER', 3), ('K2', 2)):
            j = L.judge(a, bb_, kmin=km)
            res[rd] = L.label(j) + (' park dearer' if j['dir'] == 'B>A' else ' park cheaper') if j['claimed'] else 'NOT CLAIMED'
        j = L.judge(a, bb_, kmin=3)
        O(f'  {route:8s} P{P} b={b}P gap {g:2d}: spin {j["a"]["median"]:.0f} n={j["a"]["K"]}, park {j["b"]["median"]:.0f} '
          f'n={j["b"]["K"]}; park-spin {j["pooled"]["delta"]:+.0f} ({100 * (j["pooled"]["ratio"] - 1):+.1f} %); '
          f'Ks {j["Ks"]}; LETTER {res["LETTER"]} | K2 {res["K2"]}')

O('\n## participation at P8 b=4P (active_median, all_active_reps median over used processes)')
for route in ('worker', 'external'):
    for helper in ('spin', 'park'):
        for g in (0, 20, 80):
            for n in (36, 72):
                v = ACT.get(((route, helper, 8, 4, g), n))
                if v:
                    O(f'  {route:8s} {helper} gap {g:2d} st {n}: active {statistics.median(x[0] for x in v)}, '
                      f'all_active_reps {statistics.median(x[1] for x in v)} [{min(x[1] for x in v)}-{max(x[1] for x in v)}]')
O('\n## first helper inside the region, P8 b=4P gap 0 (cell median of per-process medians, ns)')
for route in ('worker', 'external'):
    for helper in ('spin', 'park'):
        f36 = pooled(FH[((route, helper, 8, 4, 0), 36)])
        f72 = pooled(FH[((route, helper, 8, 4, 0), 72)])
        O(f'  {route:8s} {helper}: 36 st {f36["median"]:.0f} [{f36["min"]:.0f}-{f36["max"]:.0f}], 72 st '
          f'{f72["median"]:.0f} [{f72["min"]:.0f}-{f72["max"]:.0f}]; bias bound (72-36)/36 = '
          f'{(f72["median"] - f36["median"]) / 36:.0f} ns/stage')

O('\n## omega(W, gap): scope ns (cell median), first helper ns; N4 = omega(80) - omega(0); AW = omega(80) - omega(5)')
for route in ('worker', 'external'):
    for W in (8, 16):
        cells = {g: pooled(WG[(route, W, g)]) for g in (0, 5, 20, 80)}
        fh = {g: statistics.median(WGF[(route, W, g)]) for g in (0, 5, 20, 80)}
        O(f'  {route:8s} W{W:2d}: ' + ', '.join(f'gap {g}: {cells[g]["median"]:.0f} (fh {fh[g]:.0f}) n={cells[g]["K"]}'
                                               for g in (0, 5, 20, 80)))
        for nm, g0 in (('N4', 0), ('AW', 5)):
            out = []
            for rd, km in (('LETTER', 3), ('K2', 2)):
                j = L.judge(WG[(route, W, g0)], WG[(route, W, 80)], kmin=km)
                out.append(f'{rd} {L.label(j)} [{j["dir"]}]')
            O(f'      {nm}: {j["pooled"]["delta"]:+.0f} ns; ' + ' | '.join(out))
        j = L.judge(WG[(route, W, 20)], WG[(route, W, 80)], kmin=3)
        O(f'      omega(80)-omega(20): {j["pooled"]["delta"]:+.0f} ns; LETTER {L.label(j)}')

O('\n## park void rule re-read (park rows; per (config, stages) cell over the USED processes)')
PK = collections.defaultdict(list)
for p in used2:
    if p['row'].startswith('omega-wgap'):
        continue
    for s in p['ss']:
        if s['helper'] == 'park':
            PK[(s['route'], s['participants'], s['blocks_per_participant'], s['gap_us'], s['stages'])].append(
                s['parks_per_region_median'])
void = [k for k, v in PK.items() if k[3] > 0 and all(x == 0 for x in v)]
nopark0 = [k for k, v in PK.items() if k[3] == 0 and all(x == 0 for x in v)]
park0 = [(k, sorted(set(v))) for k, v in PK.items() if k[3] == 0 and any(x != 0 for x in v)]
gp = sorted({(k[3], k[4], tuple(sorted(set(v)))) for k, v in PK.items() if k[3] > 0})
O(f'  park cells {len(PK)}; gap>0 cells VOID (0 parks on every used process): {len(void)}; gap-0 cells with no park '
  f'taken: {len(nopark0)} of {sum(1 for k in PK if k[3] == 0)}; gap-0 cells that parked: {park0}')
O(f'  gap>0 parks per region (gap, stages, values): {gp}')

O('\n## v1 continuity (window 8 N5 method: slope = (cell region@72 - cell region@36)/36), P8')
V1 = collections.defaultdict(list)
for p in used1:
    for s in p['ss']:
        V1[(s['route'], s['participants'], s['stages'])].append(s['region_ns_median'])
for route in ('worker', 'external'):
    for P in (2, 4, 8, 16):
        c36 = statistics.median(V1[(route, P, 36)])
        c72 = statistics.median(V1[(route, P, 72)])
        O(f'  {route:8s} P{P:2d}: region36 {c36:.0f} (K={len(V1[(route, P, 36)])}), region72 {c72:.0f} '
          f'(K={len(V1[(route, P, 72)])}); slope {(c72 - c36) / 36:.1f}')
O.save('s2_micro.txt')
