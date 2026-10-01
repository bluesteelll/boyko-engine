"""Independent verifier library (verify_numbers lens). Written from raw/ and the ruling's definitions only.
Imports nothing from the analyst's scripts or the driver tools. Read-only on raw/."""
import hashlib
import json
import math
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.abspath(os.path.join(HERE, '..', '..', '..'))
RAW = os.path.join(W8B, 'raw')
W8 = os.path.join(os.path.dirname(W8B), 'win8')
PIN = '48e9486ac04a36c0f16426c0b6f3eaa50e782f4ea6951d099df392fb2ba954f4'
EXE = os.path.join(W8B, 'bin', 'omega_b_region_v2_a3adc827.exe')
ROWS = {}
for f in ('rows8b.json', 'rows8b.extra.json'):
    for r in json.load(open(os.path.join(W8B, f), encoding='utf-8'))['rows']:
        ROWS[r['id']] = r


def sha256_file(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        while True:
            chunk = f.read(1048576)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def read_lines(d):
    p = os.path.join(d, 'stdout.txt')
    try:
        return open(p, encoding='utf-8', errors='replace').read().splitlines()
    except FileNotFoundError:
        return []


def parse(d):
    summ, cal = [], []
    for ln in read_lines(d):
        if ln.startswith('SUMMARY '):
            summ.append(json.loads(ln[len('SUMMARY '):]))
        elif ln.startswith('CALIBRATION '):
            cal.append(json.loads(ln[len('CALIBRATION '):]))
    return summ, cal


def load_procs(raw=RAW, blocks=('omega-v2', 'omega-v1-cont')):
    out = []
    for ln in open(os.path.join(raw, 'runs.jsonl'), encoding='utf-8'):
        r = json.loads(ln)
        if r.get('pass_done') or r.get('block') not in blocks:
            continue
        out.append(r)
    return out


def clean_why(r):
    """Window 8's clean rule (analysis.md Method): both receipts <= 5 %, witness <= 2 %, no build process."""
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    ob = r.get('others_busy_pct')
    if rb is None or rb > 5.0:
        why.append(f'before {rb}')
    if ra is None or ra > 5.0:
        why.append(f'after {ra}')
    if ob is None or ob > 2.0:
        why.append(f'witness {ob}')
    if r.get('build_proc_during'):
        why.append('build during')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(f'presence {k}')
    return why


def valid_why(r, summ, cal):
    row = ROWS[r['row']]
    why = []
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    if r.get('exe_sha256') != PIN:
        why.append('sha')
    a = row['args']
    mode = a[a.index('--mode') + 1]
    route = a[a.index('--route') + 1]
    if len(summ) != row['expect_summaries']:
        why.append('%d SUMMARY' % len(summ))
    if mode == 'omega-b2':
        if len(cal) != 1:
            why.append('%d CALIBRATION' % len(cal))
        for c in cal:
            if c.get('bench') != 'omega_b2' or c.get('version') != 2:
                why.append('cal bench/version')
            w = c.get('work_ns_calibrated')
            if w is None or not (500 <= w <= 1000):
                why.append('cal work %s' % w)
        parts = [int(x) for x in a[a.index('--participants') + 1].split(',')]
        stages = [int(x) for x in a[a.index('--stages') + 1].split(',')]
        bpps = [int(x) for x in a[a.index('--blocks-per-participant') + 1].split(',')]
        helpers = a[a.index('--helper') + 1].split(',')
        gaps = [int(x) for x in a[a.index('--gap-us') + 1].split(',')] if '--gap-us' in a else [0]
        want = {(p, s, b, h, g) for p in parts for s in stages for b in bpps for h in helpers for g in gaps}
        got = []
        for s in summ:
            if s.get('bench') != 'omega_b2' or s.get('version') != 2 or s.get('route') != route:
                why.append('bench/version/route')
            if s.get('exactly_once') is not True:
                why.append('exactly_once')
            if s.get('lost_wakeups') != 0:
                why.append('lost_wakeups')
            w = s.get('work_ns_calibrated')
            if w is None or not (500 <= w <= 1000):
                why.append('work %s' % w)
            if cal and s.get('work_ns_calibrated') != cal[0].get('work_ns_calibrated'):
                why.append('work != cal')
            got.append((s['participants'], s['stages'], s['blocks_per_participant'], s['helper'], s['gap_us']))
        if sorted(got) != sorted(want):
            why.append('grid')
    elif mode == 'omega':
        workers = [int(x) for x in a[a.index('--workers') + 1].split(',')]
        gaps = [int(x) for x in a[a.index('--gap-us') + 1].split(',')]
        want = {(w, g) for w in workers for g in gaps}
        got = []
        for s in summ:
            if s.get('bench') != 'omega' or s.get('route') != route:
                why.append('bench/route')
            for k in ('helped_reps', 'first_helper_ns_median'):
                if k not in s:
                    why.append('missing ' + k)
            got.append((s['workers'], s['gap_us']))
        if sorted(got) != sorted(want):
            why.append('grid')
    elif mode == 'omega-b':
        st = int(a[a.index('--stages') + 1])
        parts = [int(x) for x in a[a.index('--participants') + 1].split(',')]
        got = []
        for s in summ:
            if s.get('bench') != 'omega_b' or s.get('route') != route or s.get('stages') != st:
                why.append('bench/route/stages')
            got.append(s['participants'])
        if sorted(got) != sorted(parts):
            why.append('grid')
    return why


def select(procs):
    """Window 8's slot rule: the original if valid and clean, else its re-run if valid and clean, else dropped."""
    for r in procs:
        summ, cal = parse(r['cwd'])
        r['_summ'], r['_cal'] = summ, cal
        r['_valid'] = valid_why(r, summ, cal)
        r['_clean'] = clean_why(r)
    slots = {}
    for r in procs:
        if r.get('attempt') == 'warmup':
            continue
        k = (r['block'], r['pass'], r['round'], r['row'])
        slots.setdefault(k, []).append(r)
    used, dropped = [], []
    for k in sorted(slots):
        ps = slots[k]
        cand = [p for p in ps if p['attempt'] == 'original'] + [p for p in ps if p['attempt'] == 'rerun']
        pick = None
        for p in cand:
            if not p['_valid'] and not p['_clean']:
                pick = p
                break
        if pick:
            used.append(pick)
        else:
            dropped.append((k, ps))
    return slots, used, dropped


def se(xs):
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) if len(xs) >= 2 else 0.0


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    m = statistics.median(xs)
    d = abs(m) if m else float('inf')
    return {'K': len(xs), 'med': m, 'min': xs[0], 'max': xs[-1], 'IQR': iqr(xs), 'SE': se(xs),
            'r': (xs[-1] - xs[0]) / d, 'i': iqr(xs) / d, 's': se(xs) / d, 'xs': xs}


def cmp2(a, b, guard=True):
    """B against A: e = |B/A - 1| against 2 x hypot of the relative spreads (window 7/8 rule)."""
    ratio = b['med'] / a['med']
    e = abs(ratio - 1)
    ok = (a['K'] >= 3 and b['K'] >= 3) if guard else True
    res = {'ratio': ratio, 'delta': b['med'] - a['med'], 'KA': a['K'], 'KB': b['K'], 'e': e}
    for k in ('r', 'i', 's'):
        bar = 2 * math.hypot(a[k], b[k])
        res['bar_' + k] = bar
        res[k] = bool(ok and e > bar)
    return res


def flags(c):
    if c is None:
        return '-'
    return ('Y' if c['r'] else 'n') + '/' + ('Y' if c['i'] else 'n') + '/' + ('Y' if c['s'] else 'n')


def ruling1(cells_a, cells_b, guard=True):
    """cells_x: key in ('p0','p1','p2','pooled') -> cell. CLAIMED iff i and s clear in every pass present and
    pooled, all with one sign; STRONG iff r clears everywhere too."""
    res = {}
    signs = set()
    for k in ('p0', 'p1', 'p2', 'pooled'):
        a, b = cells_a.get(k), cells_b.get(k)
        if a is None or b is None:
            res[k] = None
            continue
        c = cmp2(a, b, guard)
        res[k] = c
        signs.add(1 if c['delta'] > 0 else (-1 if c['delta'] < 0 else 0))
    present = [v for v in res.values() if v is not None]
    claimed = bool(present) and all(v['i'] and v['s'] for v in present) and len(signs) == 1 and 0 not in signs
    strong = claimed and all(v['r'] for v in present)
    return res, ('CLAIMED STRONG' if strong else ('CLAIMED' if claimed else 'NOT CLAIMED'))


def fcell(c, d=0):
    if c is None:
        return 'n/a'
    fmt = '%.' + str(d) + 'f'
    return ((fmt % c['med']) + ' [IQR ' + (fmt % c['IQR']) + ', SE ' + (fmt % c['SE']) + ', ' + (fmt % c['min']) +
            '-' + (fmt % c['max']) + '] K=' + str(c['K']))


def by_pass(procs, f):
    """Cells per pass and pooled for a per-process metric f (None-valued processes skipped)."""
    out = {}
    for k, ps in (('p0', 0), ('p1', 1), ('p2', 2)):
        xs = [f(p) for p in procs if p['pass'] == ps]
        xs = [x for x in xs if x is not None]
        out[k] = cell(xs) if xs else None
    xs = [f(p) for p in procs]
    xs = [x for x in xs if x is not None]
    out['pooled'] = cell(xs) if xs else None
    return out
