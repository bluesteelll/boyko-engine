"""omega_b v2: void rules, per-process slopes, S1 input, park price, participation, route - from raw/."""
import collections
import statistics
import sys
sys.dont_write_bytecode = True
import vn_lib as L

procs = L.load_procs()
slots, used, dropped = L.select(procs)
timed = [p for p in procs if p.get('attempt') != 'warmup']
V2ROWS = ('omega2-worker', 'omega2-external', 'omega2-gap-worker', 'omega2-gap-external')
v2_used = [p for p in used if p['row'] in V2ROWS]
v2_all = [p for p in timed if p['row'] in V2ROWS]
ROUTE = {'omega2-worker': 'worker', 'omega2-external': 'external', 'omega2-gap-worker': 'worker',
         'omega2-gap-external': 'external'}

print('## V1 void rules (cut.md W5 amended)')
for label, group in (('used', v2_used), ('all timed', v2_all)):
    lines = sum(len(p['_summ']) for p in group)
    cals = sum(len(p['_cal']) for p in group)
    works = [s['work_ns_calibrated'] for p in group for s in p['_summ']] + \
            [c['work_ns_calibrated'] for p in group for c in p['_cal']]
    eo = all(s['exactly_once'] is True for p in group for s in p['_summ'])
    lw = all(s['lost_wakeups'] == 0 for p in group for s in p['_summ'])
    print(' ', label, 'processes', len(group), 'SUMMARY', lines, 'CALIBRATION', cals,
          'work_ns range %.1f-%.1f' % (min(works), max(works)), 'exactly_once all', eo, 'lost_wakeups 0 all', lw)
    cells = collections.defaultdict(list)
    for p in group:
        for s in p['_summ']:
            if s['helper'] == 'park':
                k = (ROUTE[p['row']], s['participants'], s['blocks_per_participant'], s['gap_us'], s['stages'])
                cells[k].append(s['parks_per_region_median'])
    void = [k for k, v in cells.items() if k[3] > 0 and all(x == 0 for x in v)]
    nopark0 = [k for k, v in cells.items() if k[3] == 0 and all(x == 0 for x in v)]
    mixed0 = [k for k, v in cells.items() if k[3] == 0 and any(x == 0 for x in v) and not all(x == 0 for x in v)]
    allpark0 = [(k, sorted(v)) for k, v in cells.items() if k[3] == 0 and all(x > 0 for x in v)]
    gp = sorted(set((k[3], k[4], x) for k, v in cells.items() if k[3] > 0 for x in v))
    print('   park (route,cfg,stages) cells', len(cells), 'VOID', len(void), '| gap-0 cells',
          sum(1 for k in cells if k[3] == 0), 'no-park-taken', len(nopark0), 'mixed', len(mixed0),
          'parked on every rep', allpark0)
    print('   gap>0 park rows (gap, stages, parks_per_region_median) values seen:', gp)
    for k in mixed0:
        print('   mixed', k, sorted(cells[k]))


def summ_of(p, helper, P, b, gap, stages):
    for s in p['_summ']:
        if (s['helper'], s['participants'], s['blocks_per_participant'], s['gap_us'], s['stages']) == \
                (helper, P, b, gap, stages):
            return s
    return None


def slope(p, helper, P, b, gap, key='region_ns_median'):
    a = summ_of(p, helper, P, b, gap, 36)
    c = summ_of(p, helper, P, b, gap, 72)
    if a is None or c is None:
        return None
    return (c[key] - a[key]) / 36.0


def rows_for(route, gap):
    if gap == 0:
        return 'omega2-' + route
    return 'omega2-gap-' + route


def pp(route, gap):
    return [p for p in v2_used if p['row'] == rows_for(route, gap)]


print()
print('## S1 input omega_b2(P=8, b=4P, gap 0): per-process slope (region_ns_median@72 - @36)/36, cell = median')
for route in ('worker', 'external'):
    for helper in ('spin', 'park'):
        ps = pp(route, 0)
        c = L.cell([slope(p, helper, 8, 4, 0) for p in ps])
        cm = L.cell([slope(p, helper, 8, 4, 0, 'region_ns_mean') for p in ps])
        print('  %-8s %-4s %s | on region_ns_mean: %.0f' % (route, helper, L.fcell(c), cm['med']))

print()
print('## all configs: slope cells (n) and net of work (slope - bpp x work_ns_calibrated - gap_ns, per process, median)')
for route in ('worker', 'external'):
    for P in (2, 4, 8):
        for b in (1, 2, 4):
            for gap in ((0, 20, 80) if (P, b) == (8, 4) else (0,)):
                ps = pp(route, gap)
                line = '  %-8s P%d b=%dP gap %2d:' % (route, P, b, gap)
                for helper in ('spin', 'park'):
                    c = L.cell([slope(p, helper, P, b, gap) for p in ps])
                    net = L.cell([slope(p, helper, P, b, gap) - b * p['_cal'][0]['work_ns_calibrated'] - gap * 1000
                                  for p in ps])
                    line += ' %s %.0f [%.0f-%.0f] n=%d net %.0f;' % (helper, c['med'], c['min'], c['max'], c['K'],
                                                                    net['med'])
                print(line)

print()
print('## PARK-g: park slope vs spin slope, ruling 1 (i AND s every pass and pooled; r -> STRONG)')
for gap in (0, 20, 80):
    for route in ('worker', 'external'):
        for P in ((2, 4, 8) if gap == 0 else (8,)):
            for b in ((1, 2, 4) if gap == 0 else (4,)):
                ps = pp(route, gap)
                A = L.by_pass(ps, lambda p: slope(p, 'spin', P, b, gap))
                B = L.by_pass(ps, lambda p: slope(p, 'park', P, b, gap))
                res_g, v_g = L.ruling1(A, B, guard=True)
                res_n, v_n = L.ruling1(A, B, guard=False)
                paired = [slope(p, 'park', P, b, gap) - slope(p, 'spin', P, b, gap) for p in ps]
                pos = sum(1 for x in paired if x > 0)
                d = res_n['pooled']
                print(('  gap %2d %-8s P%d b=%dP: spin %.0f park %.0f delta %+.0f (%+.1f %%) | p0 %s p1 %s p2 %s '
                       'pooled %s | K %s | guard: %s | no guard: %s | paired %d/%d pos, med %+.0f [%+.0f..%+.0f]')
                      % (gap, route, P, b, A['pooled']['med'], B['pooled']['med'], d['delta'],
                         100 * (d['ratio'] - 1), L.flags(res_n['p0']), L.flags(res_n['p1']), L.flags(res_n['p2']),
                         L.flags(res_n['pooled']), [A[k]['K'] for k in ('p0', 'p1', 'p2', 'pooled')], v_g, v_n,
                         pos, len(paired), statistics.median(paired), min(paired), max(paired)))

print()
print('## gap rows: cells with IQR/SE (pooled)')
for gap in (20, 80):
    for route in ('worker', 'external'):
        ps = pp(route, gap)
        for helper in ('spin', 'park'):
            c = L.cell([slope(p, helper, 8, 4, gap) for p in ps])
            print('  gap %d %-8s %-4s %s' % (gap, route, helper, L.fcell(c)))

print()
print('## PART: participation receipts at P8 b=4P, cell medians over used processes')
for route in ('worker', 'external'):
    for gap in (0, 20, 80):
        ps = pp(route, gap)
        for helper in ('spin', 'park'):
            for st in (36, 72):
                ss = [summ_of(p, helper, 8, 4, gap, st) for p in ps]
                am = [s['active_median'] for s in ss]
                aa = [s['all_active_reps'] for s in ss]
                fh = [s['first_helper_ns_median'] for s in ss]
                hr = [s['helped_reps'] for s in ss]
                print(('  %-8s gap %2d %-4s st%d: active_median med %s [%s-%s]; all_active_reps med %s [%s-%s]; '
                       'first_helper med %s [%s-%s]; helped %s-%s n=%d')
                      % (route, gap, helper, st, statistics.median(am), min(am), max(am), statistics.median(aa),
                         min(aa), max(aa), statistics.median(fh), min(fh), max(fh), min(hr), max(hr), len(ps)))
am_all = []
aa_all = []
for route in ('worker', 'external'):
    for p in pp(route, 0):
        for s in p['_summ']:
            am_all.append((s['participants'], s['active_median']))
            aa_all.append((s['participants'], s['all_active_reps']))
print('  gap 0, used: (P, active_median) seen', sorted(set(am_all)))
print('  gap 0, used: min all_active_reps per P', {P: min(v for q, v in aa_all if q == P) for P in (2, 4, 8)})

print()
print('## A3: first helper at 36 vs 72 stages, P8 b=4P gap 0, per process (ns)')
for route in ('worker', 'external'):
    for helper in ('spin', 'park'):
        f36 = [summ_of(p, helper, 8, 4, 0, 36)['first_helper_ns_median'] for p in pp(route, 0)]
        f72 = [summ_of(p, helper, 8, 4, 0, 72)['first_helper_ns_median'] for p in pp(route, 0)]
        bias = [(b_ - a_) / 36 for a_, b_ in zip(f36, f72)]
        print('  %-8s %-4s first36 %s-%s first72 %s-%s; (first72-first36)/36 median %.0f [%.0f-%.0f]; cell-median bound %.0f'
              % (route, helper, min(f36), max(f36), min(f72), max(f72), statistics.median(bias), min(bias),
                 max(bias), (statistics.median(f72) - statistics.median(f36)) / 36))

print()
print('## Route: worker vs external slope, same (P, b, helper, gap), ruling 1')
n_cl = 0
n = 0
for gap in (0, 20, 80):
    for P in ((2, 4, 8) if gap == 0 else (8,)):
        for b in ((1, 2, 4) if gap == 0 else (4,)):
            for helper in ('spin', 'park'):
                A = L.by_pass(pp('worker', gap), lambda p: slope(p, helper, P, b, gap))
                B = L.by_pass(pp('external', gap), lambda p: slope(p, helper, P, b, gap))
                res_g, v_g = L.ruling1(A, B, True)
                res_n, v_n = L.ruling1(A, B, False)
                n += 1
                n_cl += v_g != 'NOT CLAIMED'
                print('  gap %2d P%d b=%dP %-4s: worker %.0f external %.0f (ext/wk %.3f) p0 %s p1 %s p2 %s pooled %s -> %s (no guard %s)'
                      % (gap, P, b, helper, A['pooled']['med'], B['pooled']['med'], res_n['pooled']['ratio'],
                         L.flags(res_n['p0']), L.flags(res_n['p1']), L.flags(res_n['p2']), L.flags(res_n['pooled']),
                         v_g, v_n))
print('  route configs', n, 'claimed (guard)', n_cl)
