"""Window 8 analyst library: selection, validity from the files, per-process statistics, cells, claims.
Imports nothing from the driver tools. Read-only on raw/."""
import csv
import json
import math
import os
import re
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)
RAW = os.path.join(W8, 'raw')
BIN = os.path.join(W8, 'bin')
FIX = json.load(open(os.path.join(W8, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in json.load(open(os.path.join(W8, 'rows8.json'), encoding='utf-8'))['rows']}
SUMS = {}
for line in open(os.path.join(BIN, 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS[name.strip().lstrip('*')] = sha
KEY2EXE = {'instr': 'bin/runner_226bd99e.exe', 'omega': 'bin/omega_b_region_226bd99e.exe', 'dmA': 'bin/dm1_A.exe',
           'dmB': 'bin/dm1_B.exe', 'j56': 'D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe',
           'j56p': 'bin/jolt56prof8/PerformanceTest.exe'}


def load_recs():
    out = []
    for line in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8'):
        r = json.loads(line)
        if r.get('pass_done'):
            continue
        out.append(r)
    return out


def pdir(r):
    return r['cwd']


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


def clean(r):
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    why = []
    if rb is None or rb > 5.0:
        why.append(f'receipt before {rb}')
    if ra is None or ra > 5.0:
        why.append(f'receipt after {ra}')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append(f'witness {ob}')
    if r.get('build_proc_during'):
        why.append('build during')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(f'{k} presence {pr}')
    return why


def sha_ok(r):
    return r.get('exe_sha256') == SUMS.get(KEY2EXE[r['binary']])


def validate_runner(r):
    row = ROWS[r['row']]
    d = pdir(r)
    why = []
    so = read_text(os.path.join(d, 'stdout.txt'))
    s = summary_of(so)
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if not sha_ok(r):
        why.append('sha')
    if s is None:
        return why + ['no SUMMARY'], None, None
    c = load_csv(os.path.join(d, 'run.csv'))
    n = len(c['wall_ns'])
    if n != row['steps']:
        why.append(f'csv steps {n}')
    if s.get('void_steps') != 0:
        why.append(f"void {s.get('void_steps')}")
    fx = FIX[row['pose_ref']]['hash']
    if s.get('pose_hash') != fx or s.get('expect_pose') != 'match':
        why.append(f"pose {s.get('pose_hash')} {s.get('expect_pose')}")
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if bool(s.get('armed')) != bool(row['armed']):
        why.append('armed flag')
    if row['armed']:
        if s.get('drops_total') != 0:
            why.append(f"drops {s.get('drops_total')}")
        w8 = s.get('w8s')
        if not w8:
            why.append('no w8s')
        elif w8.get('overflow') != 0:
            why.append('overflow')
    else:
        if s.get('disarmed_ring_traffic') != 0:
            why.append(f"ring {s.get('disarmed_ring_traffic')}")
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != row['broadphase']:
        why.append('bp kind')
    bt = s.get('broadphase_tree') or {}
    if row['broadphase'] == 'Tree':
        if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
            why.append(f'treediag {bt}')
    else:
        if any(v != 0 for v in bt.values()):
            why.append(f'treediag nonzero {bt}')
    a = row['args']
    if '--canary-frac' in a:
        f = float(a[a.index('--canary-frac') + 1])
        t = int(a[a.index('--canary-ref-ns') + 1])
        if s.get('canary_ns') != round(f * t):
            why.append(f"canary_ns {s.get('canary_ns')} != {f * t}")
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
        why.append(f"window mean {wm} vs {s['window_mean_ns']}")
    sl = '--sleeping' in a and a[a.index('--sleeping') + 1] == 'on'
    if bool(cfg.get('sleeping')) != sl:
        why.append('sleeping flag')
    cr = not ('--contact-reuse' in a and a[a.index('--contact-reuse') + 1] == 'off')
    if bool(cfg.get('contact_reuse')) != cr:
        why.append('reuse flag')
    return why, s, c


JOLT_STAT = re.compile(r'^Discrete, (\d+), ([\d.]+), (0x[0-9a-f]+)', re.M)


def validate_jolt(r):
    row = ROWS[r['row']]
    d = pdir(r)
    why = []
    so = read_text(os.path.join(d, 'stdout.txt'))
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if not sha_ok(r):
        why.append('sha')
    st = JOLT_STAT.findall(so)
    if len(st) != 1:
        why.append(f'{len(st)} stat lines')
    else:
        if int(st[0][0]) != r['W']:
            why.append('threads')
        if st[0][2] != row['jolt_hash']:
            why.append(f'hash {st[0][2]}')
    if 'boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=0, receipt=0' not in so:
        why.append('banner')
    pf = [f for f in os.listdir(d) if f.startswith('per_frame_')]
    frames = None
    if len(pf) != 1:
        why.append('per_frame csv')
    else:
        c = load_csv(os.path.join(d, pf[0]))
        frames = c.get('Time (ms)')
        if frames is None or len(frames) != 500 or any(x is None for x in frames):
            why.append('frames')
    if r['kind'] == 'joltprof':
        if 'boyko-w8s-patch v1: lighter profile' not in so or 'wfb=1' not in so:
            why.append('w8s banner')
        for it in (100, 200, 300, 400):
            if not os.path.exists(os.path.join(d, f"profile_chart_discrete_th{r['W']}_it{it}.html")):
                why.append(f'dump {it}')
        wf = [f for f in os.listdir(d) if f.startswith('wfb_')]
        if len(wf) != 1:
            why.append('wfb')
    return why, so, frames


def se_med(xs):
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
            's': se_med(xs) / dd, 'values': xs}


def cmp_(a, b):
    """B against A (window 7 rule)."""
    if not a or not b or not a['median']:
        return None
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= 3 and b['K'] >= 3
    cl = {k: ok and e > bars[k] for k in bars}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'], 'cl_i': cl['i'],
            'cl_s': cl['s'], 'claimed': cl['r'] and cl['s'], 'KA': a['K'], 'KB': b['K']}


def yn(c):
    return ('Y' if c['cl_r'] else 'n') + '/' + ('Y' if c['cl_i'] else 'n') + '/' + ('Y' if c['cl_s'] else 'n')


def fc(c, d=4, scale=1.0):
    if not c:
        return 'n/a'
    return (f"{c['median'] * scale:.{d}f} [{c['min'] * scale:.{d}f}-{c['max'] * scale:.{d}f}] K={c['K']} "
            f"(r {100 * c['r']:.2f} / i {100 * c['i']:.2f} / s {100 * c['s']:.2f} %)")


def fcmp(c):
    return (f"{c['ratio']:.4f} ({100 * (c['ratio'] - 1):+.2f} %, d {c['delta']:+.4f}) {yn(c)} "
            f"bars r {100 * c['bar_r']:.2f} / i {100 * c['bar_i']:.2f} / s {100 * c['bar_s']:.2f} %")
