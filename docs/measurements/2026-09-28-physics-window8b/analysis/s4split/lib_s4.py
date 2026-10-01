"""Window 8b, group S4-AB + SPLIT: analyst library. Read-only on raw/; imports nothing from the driver tools.
Conventions of window 8's analysis (win8/analysis/lib8.py): per-process mean over a step window recomputed from
run.csv; cell = median over K of per-process means; r = (max-min)/median, i = IQR (inclusive quartiles)/median,
s = 1.2533*SD/sqrt(K)/median; a comparison B vs A flags x in {r,i,s} iff |B/A - 1| > 2*hypot(x_A, x_B).
Claim rule (RULINGS-2026-09-27-W8 ruling 1): CLAIMED iff i AND s flag, same direction, in every clean pass (K=3)
AND pooled (K=9); STRONG iff r also flags in every pass and pooled."""
import csv
import json
import math
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W8B, 'raw')
ROWS = {r['id']: r for r in json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))['rows']}
_extra = json.load(open(os.path.join(W8B, 'rows8b.extra.json'), encoding='utf-8'))
for r in _extra.get('rows', []):
    ROWS[r['id']] = r
BINS = dict(json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))['binaries'])
BINS.update(_extra.get('binaries', {}))
FIX = json.load(open(os.path.join(W8B, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))
SUMS = {}
for line in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS[name.strip().lstrip('*')] = sha
WINS = {'0..500': (0, 500), '0..100': (0, 100), '100..500': (100, 500)}
# Counter columns are written RAW by the runner's csv_text (index >= n_ticks), so the wave timings are TSC ticks;
# spans / systems are converted to ns. These wave-timing counters need division by ticks_per_ns.
TICK_COUNTERS = ('phys_wave_ramp', 'phys_wave_tail', 'phys_wave_join', 'phys_wave_first_ramp', 'phys_wave_first_tail',
                 'phys_wave_pass_ramp', 'phys_np_wave_ramp', 'phys_np_wave_tail', 'phys_np_wave_join')


def load_recs(blocks):
    out = []
    for line in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8'):
        if not line.strip():
            continue
        r = json.loads(line)
        if r.get('pass_done') or r.get('r4') or 'row' not in r:
            continue
        if r.get('block') in blocks:
            out.append(r)
    return out


def read_text(p):
    try:
        return open(p, encoding='utf-8', errors='replace').read()
    except FileNotFoundError:
        return ''


def summary_of(txt):
    for line in txt.splitlines():
        if line.startswith('SUMMARY '):
            return json.loads(line[8:])
    return None


def load_csv(p):
    with open(p, newline='', encoding='utf-8') as f:
        rd = csv.reader(f)
        head = [h.strip() for h in next(rd)]
        cols = {h: [] for h in head}
        for row in rd:
            for h, v in zip(head, row):
                v = v.strip()
                if v == '':
                    cols[h].append(None)
                else:
                    try:
                        cols[h].append(float(v))
                    except ValueError:
                        cols[h].append(None)
    return cols


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def ssum(xs):
    return sum(x for x in xs if x is not None)


def clean(r):
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    why = []
    if rb is None or rb > 5.0:
        why.append('receipt before %s' % rb)
    if ra is None or ra > 5.0:
        why.append('receipt after %s' % ra)
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append('witness %s' % ob)
    if r.get('build_proc_during'):
        why.append('build during')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append('%s presence %s' % (k, pr))
    return why


def sha_ok(r):
    exe = BINS[r['binary']]['exe']
    return r.get('exe_sha256') == SUMS.get(exe)


def validate_runner(r):
    """Own re-validation from the files (window 8 list plus the 8b S4 counters)."""
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    s = summary_of(read_text(os.path.join(d, 'stdout.txt')))
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    if r.get('hang'):
        why.append('hang')
    if not sha_ok(r):
        why.append('sha')
    if s is None:
        return why + ['no SUMMARY'], None, None
    c = load_csv(os.path.join(d, 'run.csv'))
    n = len(c['wall_ns'])
    if n != row['steps']:
        why.append('csv steps %d' % n)
    if s.get('void_steps') != 0:
        why.append('void %s' % s.get('void_steps'))
    fx = FIX[row['pose_ref']]['hash']
    if s.get('pose_hash') != fx or s.get('expect_pose') != 'match':
        why.append('pose %s %s' % (s.get('pose_hash'), s.get('expect_pose')))
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if bool(s.get('armed')) != bool(row['armed']):
        why.append('armed flag')
    if row['armed']:
        if s.get('drops_total') != 0:
            why.append('drops %s' % s.get('drops_total'))
        w8 = s.get('w8s')
        if not w8:
            why.append('no w8s')
        else:
            if w8.get('overflow') != 0:
                why.append('overflow')
            s4 = BINS[r['binary']].get('s4')
            st, sk = w8.get('setup_steps'), w8.get('setup_tasks')
            if s4 == 'off' and (st, sk) != (0, 0):
                why.append('S4 off, setup %s/%s' % (st, sk))
            if s4 == 'on' and r['W'] == 1 and (st, sk) != (0, 0):
                why.append('S4 W1 not inline %s/%s' % (st, sk))
            if s4 == 'on' and r['W'] >= 2 and not (sk or 0) > 0:
                why.append('S4 on, setup_tasks %s' % sk)
    else:
        if s.get('disarmed_ring_traffic') != 0:
            why.append('ring %s' % s.get('disarmed_ring_traffic'))
    return why, s, c


def validate_more(r, s, c):
    row = ROWS[r['row']]
    why = []
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != row['broadphase']:
        why.append('bp kind')
    bt = s.get('broadphase_tree') or {}
    exp = row.get('tree_diag_expect') or {}
    if any(bt.get(k) != v for k, v in exp.items()):
        why.append('treediag %s' % bt)
    if bt.get('kd_order_builds'):
        why.append('kd builds on default kernel')
    a = row['args']
    if '--canary-frac' in a:
        f = float(a[a.index('--canary-frac') + 1])
        t = int(a[a.index('--canary-ref-ns') + 1])
        if s.get('canary_ns') != round(f * t):
            why.append('canary_ns %s' % s.get('canary_ns'))
    elif s.get('canary_ns') is not None:
        why.append('canary on a non-canary row')
    if '--canary-zone' in a:
        z = a[a.index('--canary-zone') + 1]
        nn = int(a[a.index('--canary-ns') + 1])
        if s.get('canary_zone') != z or s.get('canary_zone_ns') != nn:
            why.append('zone canary')
    elif s.get('canary_zone') is not None:
        why.append('zone canary on a plain row')
    wm = mean(c['wall_ns'][0:500])
    if abs(wm - s['window_mean_ns']) > 1.0:
        why.append('window mean %s vs %s' % (wm, s['window_mean_ns']))
    if bool(cfg.get('sleeping')):
        why.append('sleeping on')
    if not bool(cfg.get('contact_reuse')):
        why.append('reuse off')
    return why


def se_(xs):
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
    dd = abs(m) if m else float('inf')
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / dd, 'i': iqr(xs) / dd,
            's': se_(xs) / dd, 'iqr_abs': iqr(xs), 'se_abs': se_(xs), 'values': xs}


def cmp_(a, b):
    """B against A."""
    if not a or not b or not a['median']:
        return None
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= 3 and b['K'] >= 3
    cl = {k: ok and e > bars[k] for k in bars}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'], 'cl_i': cl['i'],
            'cl_s': cl['s'], 'KA': a['K'], 'KB': b['K']}


def yn(c):
    return ('Y' if c['cl_r'] else 'n') + '/' + ('Y' if c['cl_i'] else 'n') + '/' + ('Y' if c['cl_s'] else 'n')


def ruling1(pooled, per_pass, direction):
    """direction +1: B above A (a rise); -1: B below A (a fall). CLAIMED: i and s flag in that direction in pooled
    and in every pass. REFUTED: the opposite direction is CLAIMED by the same rule. Else NOT CLAIMED.
    STRONG: r also flags (same direction) in pooled and every pass."""
    allc = [pooled] + per_pass

    def holds(c, dd):
        return c['cl_i'] and c['cl_s'] and (c['ratio'] - 1) * dd > 0

    def strong(dd):
        return all(c['cl_r'] and (c['ratio'] - 1) * dd > 0 for c in allc)
    if all(holds(c, direction) for c in allc):
        return 'CLAIMED', strong(direction)
    if all(holds(c, -direction) for c in allc):
        return 'REFUTED', strong(-direction)
    return 'NOT CLAIMED', False


def fc(c, d=4, scale=1.0):
    if not c:
        return 'n/a'
    f = '%.' + str(d) + 'f'
    tmpl = f + ' [' + f + '-' + f + '] K=%d (IQR ' + f + ', SE ' + f + '; r %.2f / i %.2f / s %.2f %%)'
    return tmpl % (c['median'] * scale, c['min'] * scale, c['max'] * scale, c['K'], c['iqr_abs'] * scale,
                   c['se_abs'] * scale, 100 * c['r'], 100 * c['i'], 100 * c['s'])


def fcmp(c):
    return ('%.4f (%+.2f %%, d %+.4f) %s bars r %.2f / i %.2f / s %.2f %%' %
            (c['ratio'], 100 * (c['ratio'] - 1), c['delta'], yn(c), 100 * c['bar_r'], 100 * c['bar_i'],
             100 * c['bar_s']))
