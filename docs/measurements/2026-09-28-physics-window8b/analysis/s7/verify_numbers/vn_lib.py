"""Independent verifier library for window 8b block S7-AB (written from raw/ and the ruling; imports nothing
from the analyst's s7/ scripts nor from the driver tools). Read-only on raw/."""
import csv
import hashlib
import json
import math
import os
import statistics

W8B = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b'
RAW = os.path.join(W8B, 'raw')
FIX = json.load(open(os.path.join(W8B, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))
EXTRA = json.load(open(os.path.join(W8B, 'rows8b.extra.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in EXTRA['rows']}
BINS = EXTRA['binaries']

_sha_cache = {}


def sha256_file(p):
    if p not in _sha_cache:
        h = hashlib.sha256()
        with open(p, 'rb') as f:
            for chunk in iter(lambda: f.read(1 << 20), b''):
                h.update(chunk)
        _sha_cache[p] = h.hexdigest()
    return _sha_cache[p]


def load_block(block='S7-AB'):
    out = []
    for line in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8'):
        r = json.loads(line)
        if r.get('pass_done') or r.get('block') != block:
            continue
        out.append(r)
    return out


def summary_of(txt):
    for line in txt.splitlines():
        if line.startswith('SUMMARY '):
            return json.loads(line[8:])
    return None


def read_csv(p):
    with open(p, newline='', encoding='utf-8') as f:
        rd = csv.DictReader(f)
        rows = list(rd)
    return rows


def validate(r):
    """Validity from the process's own files. Returns (why list, summary, csv rows)."""
    why = []
    row = ROWS[r['row']]
    d = r['cwd']
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    b = BINS[r['binary']]
    exe_path = os.path.join(W8B, b['exe'])
    if os.path.basename(r['exe']).lower() != os.path.basename(b['exe']).lower():
        why.append(f"exe name {r['exe']}")
    if r.get('exe_sha256') != b['sha256_pin']:
        why.append('record sha != pin')
    if sha256_file(exe_path) != b['sha256_pin']:
        why.append('file sha != pin')
    so = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
    s = summary_of(so)
    if s is None:
        return why + ['no SUMMARY'], None, None
    label = f"{r['row']}#{r['binary']}@W{r['W']}"
    if s.get('label') != label:
        why.append(f"label {s.get('label')} != {label}")
    exp_args = row['args'] + ['--workers', str(r['W']), '--steps', '500', '--window', '0..500']
    if s.get('args', [])[:len(exp_args)] != exp_args:
        why.append(f"args {s.get('args')[:len(exp_args)]}")
    a = s.get('args', [])
    ep = a[a.index('--expect-pose') + 1] if '--expect-pose' in a else None
    if ep is None or os.path.basename(ep) != row['pose_ref'] + '.pose':
        why.append(f'expect-pose path {ep}')
    if s.get('void_steps') != 0:
        why.append(f"void {s.get('void_steps')}")
    if s.get('pose_hash') != FIX[row['pose_ref']]['hash'] or s.get('expect_pose') != 'match':
        why.append(f"pose {s.get('pose_hash')} {s.get('expect_pose')}")
    pb = os.path.join(d, 'pose.bin')
    if not os.path.exists(pb) or sha256_file(pb) != FIX[row['pose_ref']]['sha256']:
        why.append('pose.bin sha')
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if s.get('armed'):
        why.append('armed')
    if s.get('disarmed_ring_traffic') != 0:
        why.append('ring')
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != row['broadphase']:
        why.append('bp')
    if cfg.get('sleeping'):
        why.append('sleeping on')
    if not cfg.get('contact_reuse'):
        why.append('reuse off')
    if '--canary-frac' in row['args']:
        f = float(row['args'][row['args'].index('--canary-frac') + 1])
        t = int(row['args'][row['args'].index('--canary-ref-ns') + 1])
        if s.get('canary_ns') != round(f * t):
            why.append(f"canary {s.get('canary_ns')}")
    elif s.get('canary_ns') is not None:
        why.append('canary on plain row')
    rows = read_csv(os.path.join(d, 'run.csv'))
    if len(rows) != 500:
        why.append(f'csv {len(rows)}')
    wm = sum(float(x['wall_ns']) for x in rows) / len(rows)
    if abs(wm - s['window_mean_ns']) > 1.0:
        why.append('window mean')
    if not r.get('placement'):
        why.append('NO placement receipt')
    return why, s, rows


def dirty(r):
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
            why.append(f'{k} presence')
    return why


WINDOWS = {'0..500': (0, 500), '0..100': (0, 100), '100..500': (100, 500)}


def wmean(rows, lo, hi, col='wall_ns'):
    xs = [float(x[col]) for x in rows[lo:hi]]
    return sum(xs) / len(xs)


def stats(xs):
    xs = sorted(xs)
    K = len(xs)
    m = statistics.median(xs)
    rng = xs[-1] - xs[0]
    if K >= 2:
        q = statistics.quantiles(xs, n=4, method='inclusive')
        iq = q[2] - q[0]
        se = 1.2533 * statistics.stdev(xs) / math.sqrt(K)
    else:
        iq = se = 0.0
    return {'K': K, 'med': m, 'min': xs[0], 'max': xs[-1], 'r': rng / m, 'i': iq / m, 's': se / m,
            'IQR': iq, 'SE': se, 'vals': xs}


def compare(a, b):
    ratio = b['med'] / a['med']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= 3 and b['K'] >= 3
    fl = {k: bool(ok and e > bars[k]) for k in bars}
    return {'ratio': ratio, 'delta': b['med'] - a['med'], 'bars': bars, 'fl': fl, 'sign': 1 if ratio > 1 else -1}


def yn(fl):
    return '/'.join('Y' if fl[k] else 'n' for k in ('r', 'i', 's'))
