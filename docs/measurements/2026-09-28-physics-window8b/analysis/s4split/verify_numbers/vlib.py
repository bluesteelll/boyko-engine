"""Independent verifier library (NUMBERS lens) for window 8b S4-AB + SPLIT.
Written from raw/ and the ruling's definitions; imports nothing from the analyst's s4split scripts or the driver.
Read-only on raw/."""
import csv, json, math, os, statistics, hashlib, ntpath

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.normpath(os.path.join(HERE, '..', '..', '..'))
RAW = os.path.join(W8B, 'raw')
BIN = os.path.join(W8B, 'bin')
POSE = '0x30c5438bc6ad9ffa'
EXE = {'parent': 'runner_parent_16191fda_s4off.exe', 'tip': 'runner_tip_16191fda.exe'}
SUMS = {}
for line in open(os.path.join(BIN, 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS[os.path.basename(name.strip().lstrip('*'))] = sha
ROWS = {r['id']: r for r in json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))['rows']}


def file_sha(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()


def load_recs(blocks):
    out = []
    for line in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8'):
        r = json.loads(line)
        if r.get('pass_done') or 'r4' in r:
            continue
        if r.get('block') in blocks:
            out.append(r)
    return out


def summary_of(txt):
    s = None
    n = 0
    for line in txt.splitlines():
        if line.startswith('SUMMARY '):
            s = json.loads(line[8:])
            n += 1
    return s, n


def load_csv(p):
    with open(p, newline='', encoding='utf-8') as f:
        rd = csv.reader(f)
        head = [h.strip() for h in next(rd)]
        cols = {h: [] for h in head}
        for row in rd:
            for h, v in zip(head, row):
                v = v.strip()
                try:
                    cols[h].append(float(v) if v != '' else None)
                except ValueError:
                    cols[h].append(None)
    return cols


def clean_why(r):
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    if rb is None or rb > 5.0:
        why.append('before %s' % rb)
    if ra is None or ra > 5.0:
        why.append('after %s' % ra)
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append('witness %s' % ob)
    if r.get('build_proc_during'):
        why.append('build during')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(k + ' presence')
    if r.get('hang'):
        why.append('hang')
    return why


def validate(r):
    """Own validity check from the process's own files."""
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    exe = EXE[r['binary']]
    if ntpath.basename(r['exe']) != exe:
        why.append('exe ' + r['exe'])
    if r.get('exe_sha256') != SUMS[exe]:
        why.append('sha')
    so = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
    s, ns = summary_of(so)
    if s is None or ns != 1:
        return why + ['SUMMARY count %d' % ns], None, None
    c = load_csv(os.path.join(d, 'run.csv'))
    if len(c['wall_ns']) != 500 or any(x is None for x in c['wall_ns']):
        why.append('csv steps %d' % len(c['wall_ns']))
    if [int(x) for x in c['step']] != list(range(500)):
        why.append('step column')
    if s.get('void_steps') != 0:
        why.append('void %s' % s.get('void_steps'))
    if s.get('pose_hash') != POSE or s.get('expect_pose') != 'match':
        why.append('pose %s %s' % (s.get('pose_hash'), s.get('expect_pose')))
    if s.get('steps') != 500 or s.get('window') != [0, 500]:
        why.append('steps/window')
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    lab = '%s%s%s@W%d' % (r['row'], chr(35), r['binary'], r['W'])
    if s.get('label') != lab:
        why.append('label %s' % s.get('label'))
    armed = '--arm-profiler' in s.get('args', [])
    if bool(s.get('armed')) != bool(row['armed']) or armed != bool(row['armed']):
        why.append('armed flag')
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != 'Tree' or cfg.get('sleeping') or not cfg.get('contact_reuse') or s.get('cfg') != 'default':
        why.append('cfg')
    bt = s.get('broadphase_tree') or {}
    if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0
            and bt.get('kd_order_builds') == 0):
        why.append('treediag')
    w8 = s.get('w8s')
    if row['armed']:
        if s.get('drops_total') != 0:
            why.append('drops')
        if not w8:
            why.append('no w8s')
        else:
            if w8.get('overflow') != 0:
                why.append('overflow')
            st, sk = w8.get('setup_steps'), w8.get('setup_tasks')
            if r['binary'] == 'parent' and (st or 0) != 0:
                why.append('parent setup %s/%s' % (st, sk))
            if r['binary'] == 'tip' and r['W'] >= 8 and (st, sk) != (500, 15980):
                why.append('tip setup %s/%s' % (st, sk))
            if (s.get('threads') or {}).get('solve_on_dispatcher_steps') != 0:
                why.append('solve on dispatcher')
    else:
        if s.get('disarmed_ring_traffic') != 0:
            why.append('ring traffic')
        if w8 is not None:
            why.append('w8s on disarmed')
    a = row['args']
    if '--canary-frac' in a:
        f = float(a[a.index('--canary-frac') + 1])
        t = int(a[a.index('--canary-ref-ns') + 1])
        if s.get('canary_ns') != round(f * t):
            why.append('canary_ns %s' % s.get('canary_ns'))
    elif s.get('canary_ns') is not None:
        why.append('canary on plain row')
    if '--canary-zone' in a:
        z = a[a.index('--canary-zone') + 1]
        nn = int(a[a.index('--canary-ns') + 1])
        if s.get('canary_zone') != z or s.get('canary_zone_ns') != nn:
            why.append('zone canary')
    elif s.get('canary_zone') is not None:
        why.append('zone canary on plain row')
    wm = sum(c['wall_ns']) / 500.0
    if abs(wm - s['window_mean_ns']) > 1.0:
        why.append('window mean %s vs %s' % (wm, s['window_mean_ns']))
    return why, s, c


def select(recs):
    """Slot rule of window 8: original if valid and clean, else its re-run if valid and clean, else dropped."""
    slots = {}
    for r in recs:
        if not r.get('timed') or r.get('attempt') not in ('original', 'rerun'):
            continue
        k = (r['block'], r['pass'], r['round'], r['row'], r['binary'], r['W'])
        slots.setdefault(k, {})[r['attempt']] = r
    used, dropped, log = [], [], []
    for k in sorted(slots, key=str):
        cand = slots[k]
        pick = None
        for att in ('original', 'rerun'):
            r = cand.get(att)
            if r is None:
                continue
            why, s, c = validate(r)
            cw = clean_why(r)
            if not why and not cw:
                pick = (r, s, c)
                break
            log.append((k, att, why, cw))
        if pick:
            used.append(pick)
        else:
            dropped.append(k)
    return slots, used, dropped, log


def win_mean(c, col, lo, hi):
    xs = [x for x in c[col][lo:hi] if x is not None]
    return sum(xs) / len(xs) if xs else None


def win_sum(c, col, lo, hi):
    return sum(x for x in c[col][lo:hi] if x is not None)


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(xs)
    m = statistics.median(xs)
    se = 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs))
    return {'K': len(xs), 'med': m, 'min': xs[0], 'max': xs[-1], 'iqr': iqr(xs), 'se': se,
            'r': (xs[-1] - xs[0]) / abs(m), 'i': iqr(xs) / abs(m), 's': se / abs(m), 'vals': xs}


def cmp_(a, b):
    ratio = b['med'] / a['med']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'ris'}
    fl = {k: e > bars[k] for k in 'ris'}
    return {'A': a['med'], 'B': b['med'], 'd': b['med'] - a['med'], 'pct': 100 * (ratio - 1), 'bars': bars, 'fl': fl,
            'dir': (1 if b['med'] > a['med'] else -1)}


def yn(c):
    return '/'.join('Y' if c['fl'][k] else 'n' for k in 'ris')


def ruling1(pooled, passes):
    """CLAIMED iff i and s flag (same direction) pooled and in every pass; STRONG iff r too, pooled and every pass."""
    allc = [pooled] + passes
    dirs = set(c['dir'] for c in allc)
    claimed = all(c['fl']['i'] and c['fl']['s'] for c in allc) and len(dirs) == 1
    strong = claimed and all(c['fl']['r'] for c in allc)
    return 'CLAIMED STRONG' if strong else ('CLAIMED' if claimed else 'NOT CLAIMED')
