"""Window 8b, block S7-AB: analyst library (results-analyst, 2026-09-29).
Selection, validity recomputed from the per-process files, per-process window means from run.csv, cells, the window-8
comparison (lib8.cmp_ verbatim in its arithmetic) and ruling 1's claim rule. Read-only on raw/ and gate/.
Parsers reused from the window's own tools: tools/lib/driver.py (load_csv, parse_summary), tools/s7pre8b.py (lanes_bound).
"""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W8B, 'raw')
GATE = os.path.join(W8B, 'gate')
sys.path.insert(0, os.path.join(W8B, 'tools', 'lib'))
sys.path.insert(0, os.path.join(W8B, 'tools'))
import driver as D  # noqa: E402  (load_csv, parse_summary)

EXTRA = json.load(open(os.path.join(W8B, 'rows8b.extra.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in EXTRA['rows']}
BINS = EXTRA['binaries']
FIX = json.load(open(os.path.join(GATE, 'fixtures', 'fixtures.json'), encoding='utf-8'))
SUMS = {}
for line in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS[name.strip().lstrip('*')] = sha
WINS = {'0..500': (0, 500), '0..100': (0, 100), '100..500': (100, 500)}
BLOCK = 'S7-AB'


def load_recs():
    out = []
    for line in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8'):
        if not line.strip():
            continue
        r = json.loads(line)
        if r.get('block') != BLOCK or r.get('pass_done') or r.get('voided_pass') or r.get('r4'):
            continue
        out.append(r)
    return out


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def clean_why(r):
    """Window 8's clean rule (receipts <= 5 %, witness <= 2 %, no build process, no build/lane presence)."""
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
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


def validate(r):
    """Validity recomputed from the files (not from the driver's 'invalid' field, which is cross-checked)."""
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if r.get('hang'):
        why.append('hang')
    pin = BINS[r['binary']]['sha256_pin']
    if r.get('exe_sha256') != pin or SUMS.get(BINS[r['binary']]['exe']) != pin:
        why.append('sha256 vs pin / SHA256SUMS')
    so = open(os.path.join(d, 'stdout.txt'), 'rb').read()
    s = D.parse_summary(so)
    c = D.load_csv(os.path.join(d, 'run.csv'))
    if s is None:
        return why + ['no SUMMARY'], None, c
    if c is None or 'wall_ns' not in c:
        return why + ['no csv'], s, None
    n = len(c['wall_ns'])
    if n != row['steps'] or any(x is None for x in c['wall_ns']):
        why.append(f'csv steps {n}')
    if s.get('void_steps') != 0:
        why.append(f"void_steps {s.get('void_steps')}")
    fx = FIX[row['pose_ref']]['hash']
    if s.get('pose_hash') != fx or s.get('expect_pose') != 'match':
        why.append(f"pose {s.get('pose_hash')} {s.get('expect_pose')}")
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') not in (None, r['W']):
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if bool(s.get('armed')):
        why.append('armed on a disarmed row')
    if s.get('disarmed_ring_traffic'):
        why.append(f"ring {s.get('disarmed_ring_traffic')}")
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != row['broadphase']:
        why.append(f"bp {cfg.get('broadphase')}")
    bt = s.get('broadphase_tree') or {}
    if row['broadphase'] == 'Tree':
        exp = row.get('tree_diag_expect') or {}
        if any(bt.get(k) != v for k, v in exp.items()):
            why.append(f'treediag {bt}')
        if bt.get('kd_order_builds'):
            why.append('kd builds on the default kernel')
    else:
        if any(bt.get(k) for k in ('static_rebuilds', 'members', 'evictions')):
            why.append(f'treediag nonzero {bt}')
    a = row['args']
    if '--canary-frac' in a:
        f = float(a[a.index('--canary-frac') + 1])
        t = int(a[a.index('--canary-ref-ns') + 1])
        if s.get('canary_ns') != round(f * t):
            why.append(f"canary_ns {s.get('canary_ns')} vs {f * t}")
    elif s.get('canary_ns'):
        why.append('canary on a non-canary row')
    wm = mean(c['wall_ns'][0:500])
    if abs(wm - s['window_mean_ns']) > 1.0:
        why.append(f"window mean {wm} vs SUMMARY {s['window_mean_ns']}")
    if r.get('mean_ms') is None or abs(wm / 1e6 - r['mean_ms']) > 1e-9:
        why.append(f"driver mean_ms {r.get('mean_ms')} vs {wm / 1e6}")
    return why, s, c


def se_med(xs):
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) if len(xs) >= 2 else 0.0


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    """Window 8 (lib8.cell): median over K of process means; r = range, i = IQR, s = 1.2533 SD / sqrt(K), all / median."""
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    m = statistics.median(xs)
    dd = abs(m) if m else float('inf')
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / dd, 'i': iqr(xs) / dd,
            's': se_med(xs) / dd, 'iqr_abs': iqr(xs), 'se_abs': se_med(xs), 'values': xs}


def cmp_(a, b):
    """B against A, window 8's rule (lib8.cmp_): e = |B/A - 1|; bar_x = 2 hypot(A_x, B_x); flag x iff e > bar_x."""
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


def ruling1(a_pooled, b_pooled, a_pass, b_pass):
    """RULINGS-2026-09-27-W8 ruling 1: a difference is CLAIMED iff it clears i AND s in every clean pass and pooled,
    in one direction; STRONG iff it also clears r (reported both pooled-only and every-pass-and-pooled)."""
    pooled = cmp_(a_pooled, b_pooled)
    per = {p: cmp_(a_pass[p], b_pass[p]) for p in sorted(a_pass)}
    sign = 1 if pooled['ratio'] > 1 else -1
    same = all((1 if c['ratio'] > 1 else -1) == sign for c in per.values())
    claimed = pooled['cl_i'] and pooled['cl_s'] and all(c['cl_i'] and c['cl_s'] for c in per.values()) and same
    strong_all = claimed and pooled['cl_r'] and all(c['cl_r'] for c in per.values())
    strong_pooled = claimed and pooled['cl_r']
    return {'pooled': pooled, 'per': per, 'direction': 'B slower' if sign > 0 else 'B faster', 'same_sign': same,
            'claimed': claimed, 'strong_all': strong_all, 'strong_pooled_r': strong_pooled}


def fcell(c, d=4):
    return (f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] K={c['K']} "
            f"(r {100 * c['r']:.2f} / i {100 * c['i']:.2f} / s {100 * c['s']:.2f} %)")


def fcmp(c):
    return (f"{c['ratio']:.4f} ({100 * (c['ratio'] - 1):+.2f} %, d {c['delta']:+.4f} ms) {yn(c)} "
            f"bars r {100 * c['bar_r']:.2f} / i {100 * c['bar_i']:.2f} / s {100 * c['bar_s']:.2f} %")
