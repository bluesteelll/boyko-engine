"""o8: small checks quoted in omega.md: SUMMARY line count and tick granularity; parks per region vs helpers x stages;
min all_active_reps at P8 gap 0; the worker P8 b=4P recruitment-bias bound; omega_b as a share of the stage work."""
import json
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
procs = L.all_procs()
used, _, _ = L.select(procs)
v2 = [p for p in used if p['row'].startswith('omega2')]
ss = [s for p in v2 for s in p['summaries']]
O('used v2 processes %d, SUMMARY lines %d, CALIBRATION lines %d' % (len(v2), len(ss), sum(len(p['cal']) for p in v2)))
allnums = [s['region_ns_median'] for s in ss] + [s['scope_ns_median'] for p in used if p['row'].startswith('omega-wgap') for s in p['summaries']]
allnums += [s['region_ns_median'] for p in used if p['row'].startswith('omega1') for s in p['summaries']]
O('region/scope medians that are not multiples of 100 ns: %d of %d' % (sum(1 for x in allnums if x % 100), len(allnums)))
for s in ss:
    if s['helper'] == 'park' and s['gap_us'] > 0:
        assert s['parks_per_region_median'] == (s['participants'] - 1) * s['stages'], s
O('gap>0 park rows: parks_per_region_median == (P - 1) x stages on every used SUMMARY (7 x 36 = 252, 7 x 72 = 504): True')
p8 = [s for s in ss if s['participants'] == 8 and s['gap_us'] == 0]
O('P8 gap 0: active_median min %d; all_active_reps min %d of %d' % (min(s['active_median'] for s in p8), min(s['all_active_reps'] for s in p8), p8[0]['regions']))
PER = json.load(open(L.HERE + '/o1_per_process.json'))
for route in ('worker', 'external'):
    for P in (4, 8):
        xs = [x for x in PER if tuple(x['key'])[:3] == (route, P, 4) and tuple(x['key'])[4] == 0]
        d = [(x['first72'] - x['first36']) / 36.0 for x in xs]
        O('%s P%d b=4P gap 0: first72 - first36 per process %.0f..%.0f ns -> slope bias bound (first72-first36)/36 = %.0f..%.0f ns/stage' % (
            route, P, min(x['first72'] - x['first36'] for x in xs), max(x['first72'] - x['first36'] for x in xs), min(d), max(d)))
wk = sorted(x['work_ns'] for x in PER)[len(PER) // 2]
for name, ms, st in (('warm apply', 0.304, 36.44), ('integrate group', 0.1216 - 0.0027, 9)):
    per_part = ms * 1e6 / st / 8
    for bpp in (1, 2, 4):
        ws = [L.cell([x['overhead'] for x in PER if tuple(x['key']) == (r, 8, bpp, h, 0)])['median'] for r in ('worker', 'external') for h in ('spin', 'park')]
        O('%s: per-participant stage work at P8 %.0f ns; omega_b(b=%dP) %.0f-%.0f ns = %.0f-%.0f %% of it' % (
            name, per_part, bpp, min(ws), max(ws), 100 * min(ws) / per_part, 100 * max(ws) / per_part))
O('per-block work (median calibrated) %.1f ns; b=4P work per participant per stage %.0f ns; omega_b(4P) 857-1073 = %.0f-%.0f %% of it' % (
    wk, 4 * wk, 100 * 857 / (4 * wk), 100 * 1073 / (4 * wk)))
O('window 8 per-wave omega(8) 3.30 us vs region per-stage omega_b 0.346-1.073 us: ratio %.1f-%.1fx' % (3300 / 1073, 3300 / 346))
dirty = [p for p in procs if p['attempt'] != 'warmup' and p['clean_why']]
O('dirty timed processes %d of %d; omega-v2 used re-runs %d; omega-v1-cont used re-runs %d' % (
    len(dirty), len([p for p in procs if p['attempt'] != 'warmup']),
    sum(1 for p in used if p['block'] == 'omega-v2' and p['attempt'] == 'rerun'),
    sum(1 for p in used if p['block'] == 'omega-v1-cont' and p['attempt'] == 'rerun')))
O.save('o8.txt')
