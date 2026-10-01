"""o1: omega_b v2. Per process: omega_b2 = (region_ns_median@72 - region_ns_median@36) / 36 for every
(route, P, blocks_per_participant, helper, gap); cell = median over the used processes (K); per pass and pooled.
Void rules (cut.md amended, W5) per cell; participation receipts; park price = park slope - spin slope (ruling 1 rule).
Derived (post hoc, arith.): overhead = slope - bpp x work_ns_calibrated (- gap); intercept = region@36 - 36 x slope."""
import json
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
procs = L.all_procs()
used, dropped, slots = L.select(procs)
V2ROWS = {'omega2-worker': 'worker', 'omega2-external': 'external', 'omega2-gap-worker': 'worker',
          'omega2-gap-external': 'external'}
v2 = [p for p in used if p['row'] in V2ROWS]
allv2 = [p for p in procs if p['row'] in V2ROWS and p['attempt'] != 'warmup']


def cfgs(p):
    """{(route, P, bpp, helper, gap): {stages: summary}} for one process."""
    d = {}
    for s in p['summaries']:
        k = (s['route'], s['participants'], s['blocks_per_participant'], s['helper'], s['gap_us'])
        d.setdefault(k, {})[s['stages']] = s
    return d


PER = []  # per-process records
for p in v2:
    wn = p['cal'][0]['work_ns_calibrated']
    for k, st in cfgs(p).items():
        a, b = st[36], st[72]
        slope = (b['region_ns_median'] - a['region_ns_median']) / 36.0
        slope_mean = (b['region_ns_mean'] - a['region_ns_mean']) / 36.0
        route, P, bpp, helper, gap = k
        PER.append({'row': p['row'], 'pass': p['pass'], 'round': p['round'], 'attempt': p['attempt'], 'key': k,
                    'slope': slope, 'slope_mean': slope_mean, 'r36': a['region_ns_median'], 'r72': b['region_ns_median'],
                    'intercept': a['region_ns_median'] - 36 * slope,
                    'overhead': slope - bpp * wn - gap * 1000.0, 'work_ns': wn,
                    'act36': a['active_median'], 'act72': b['active_median'],
                    'allact36': a['all_active_reps'], 'allact72': b['all_active_reps'],
                    'help36': a['helped_reps'], 'help72': b['helped_reps'],
                    'first36': a['first_helper_ns_median'], 'first72': b['first_helper_ns_median'],
                    'parks36': a['parks_per_region_median'], 'parks72': b['parks_per_region_median'],
                    'lost': a['lost_wakeups'] + b['lost_wakeups'],
                    'eo': a['exactly_once'] is True and b['exactly_once'] is True,
                    'wn36': a['work_ns_calibrated'], 'wn72': b['work_ns_calibrated'], 'regions': a['regions']})
json.dump(PER, open(L.HERE + '/o1_per_process.json', 'w'), indent=0)
KEYS = sorted({x['key'] for x in PER}, key=lambda k: (k[0], k[4], k[1], k[2], k[3]))


def get(k, f, passes=None):
    return L.cell([f(x) for x in PER if x['key'] == k and (passes is None or x['pass'] in passes)])


O('# o1 omega_b v2 (omega2 exe a3adc827), window 8b block omega-v2; ns; cell = median over K processes of the per-process value')
O('used v2 processes: %d; per-process records (process x config): %d; configs: %d' % (len(v2), len(PER), len(KEYS)))
O()
O('## Void rules (cut.md "omega_b v2 rows (W5)", amended), per (row, config) cell')
bad_wn = [x for x in PER if not (500 <= x['wn36'] <= 1000 and 500 <= x['wn72'] <= 1000)]
bad_eo = [x for x in PER if not x['eo']]
bad_lw = [x for x in PER if x['lost'] != 0]
O('work_ns_calibrated outside [500,1000]: %d records; range over used processes %.1f-%.1f ns' % (
    len(bad_wn), min(x['work_ns'] for x in PER), max(x['work_ns'] for x in PER)))
O('exactly_once not true: %d; lost_wakeups != 0: %d' % (len(bad_eo), len(bad_lw)))
for label, pool in (('used reps', v2), ('every rep incl. unused', allv2)):
    void, nopark, part = [], [], []
    seen = {}
    for p in pool:
        for s in p['summaries']:
            if s['helper'] != 'park':
                continue
            seen.setdefault((s['route'], s['participants'], s['blocks_per_participant'], s['gap_us'], s['stages']),
                            []).append(s['parks_per_region_median'])
    for k, ns in sorted(seen.items()):
        if all(n == 0 for n in ns):
            (void if k[3] > 0 else nopark).append((k, len(ns)))
        elif any(n == 0 for n in ns):
            part.append((k, ns))
    O('park rule over %s: %d park (route,P,bpp,gap,stages) cells; VOID (gap>0, 0 on every rep) %d; "no park taken" (gap 0, 0 on every rep) %d; mixed %d' % (
        label, len(seen), len(void), len(nopark), len(part)))
    for k, n in void:
        O('  VOID %s n=%d' % (str(k), n))
    for k, ns in part:
        O('  mixed (some reps parked) %s parks per rep %s' % (str(k), ns))
    if label == 'used reps':
        took = [(k, ns) for k, ns in sorted(seen.items()) if any(n > 0 for n in ns)]
        O('  park rows that parked on at least one used rep:')
        for k, ns in took:
            O('    %s parks_per_region_median per rep %s' % (str(k), sorted(ns)))
O()
O('## omega_b2 = (region@72 - region@36)/36 per process, then median; all configs (pooled K and per-pass cells)')
O('columns: key (route, P, bpp, helper, gap_us) | pooled slope cell | per pass medians (K) | slope from region means | overhead = slope - bpp*work - gap | intercept')
for k in KEYS:
    c = get(k, lambda x: x['slope'])
    pp = [get(k, lambda x: x['slope'], {q}) for q in (0, 1, 2)]
    cm = get(k, lambda x: x['slope_mean'])
    co = get(k, lambda x: x['overhead'])
    ci = get(k, lambda x: x['intercept'])
    O('%-34s slope %s | passes %s | mean-slope %.0f | overhead %.0f [%.0f-%.0f] | intercept %.0f' % (
        str(k), L.fc(c, 0), ' / '.join('%.0f (%d)' % (q['median'], q['K']) for q in pp if q), cm['median'],
        co['median'], co['min'], co['max'], ci['median']))
O()
O('## Participation receipts per config (cells over K of the per-process SUMMARY values; regions = 1000 per process)')
O('key | active_median 36/72 (cell medians, min) | all_active_reps 36/72 (median [min-max]) | helped_reps 36/72 min | first_helper_ns_median 36/72 | parks_per_region_median 36/72 (median [min-max])')
for k in KEYS:
    def f(n):
        return get(k, lambda x: x[n])
    a36, a72, aa36, aa72 = f('act36'), f('act72'), f('allact36'), f('allact72')
    h36, h72, f36, f72, p36, p72 = f('help36'), f('help72'), f('first36'), f('first72'), f('parks36'), f('parks72')
    O('%-34s act %g/%g (min %g/%g) | allact %g [%g-%g] / %g [%g-%g] | helped min %g/%g | first %g/%g ns [%g-%g / %g-%g] | parks %g [%g-%g] / %g [%g-%g]' % (
        str(k), a36['median'], a72['median'], a36['min'], a72['min'], aa36['median'], aa36['min'], aa36['max'],
        aa72['median'], aa72['min'], aa72['max'], h36['min'], h72['min'], f36['median'], f72['median'], f36['min'],
        f36['max'], f72['min'], f72['max'], p36['median'], p36['min'], p36['max'], p72['median'], p72['min'], p72['max']))
O()
O('## Park price: park slope vs spin slope at the same (route, P, bpp, gap); ruling 1 (i AND s pooled and in every pass)')
for k in KEYS:
    route, P, bpp, helper, gap = k
    if helper != 'spin':
        continue
    kp = (route, P, bpp, 'park', gap)
    a, b = get(k, lambda x: x['slope']), get(kp, lambda x: x['slope'])
    pooled = L.cmp_(a, b)
    per = {q: L.cmp_(get(k, lambda x: x['slope'], {q}), get(kp, lambda x: x['slope'], {q})) for q in (0, 1, 2)}
    v, det = L.rule1(pooled, per)
    O('%-30s spin %s | park %s | %s | %s -> %s' % (str((route, P, bpp, gap)), L.fc(a, 0), L.fc(b, 0), L.fcmp(pooled, 0), det, v))
O()
O('## Paired per-process park - spin slope difference (post hoc; same process, adjacent rows)')
for k in KEYS:
    route, P, bpp, helper, gap = k
    if helper != 'spin':
        continue
    kp = (route, P, bpp, 'park', gap)
    d = {}
    for x in PER:
        if x['key'] in (k, kp):
            d.setdefault((x['row'], x['pass'], x['round']), {})[x['key'][3]] = x['slope']
    diffs = [v['park'] - v['spin'] for v in d.values() if 'park' in v and 'spin' in v]
    c = L.cell(diffs)
    O('%-30s park-spin per process %s; positive in %d of %d' % (str((route, P, bpp, gap)), L.fc(c, 0),
                                                             sum(x > 0 for x in diffs), len(diffs)))
O()
O('## Route: external vs worker slope at the same (P, bpp, helper, gap) (ruling 1)')
for k in KEYS:
    route, P, bpp, helper, gap = k
    if route != 'worker':
        continue
    ke = ('external', P, bpp, helper, gap)
    a, b = get(k, lambda x: x['slope']), get(ke, lambda x: x['slope'])
    pooled = L.cmp_(a, b)
    per = {q: L.cmp_(get(k, lambda x: x['slope'], {q}), get(ke, lambda x: x['slope'], {q})) for q in (0, 1, 2)}
    v, det = L.rule1(pooled, per)
    O('%-26s worker %.0f external %.0f | %s | %s -> %s' % (str((P, bpp, helper, gap)), a['median'], b['median'],
                                                         L.fcmp(pooled, 0), det, v))
O.save('o1.txt')
