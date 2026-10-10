"""Independent verifier library (window 8b, F3 + F3-G4). Written from raw/ and the rulings only.
Imports nothing from the analyst's f3/*.py nor from the driver tools. Read-only on raw/."""
import csv
import json
import math
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.normpath(os.path.join(HERE, '..', '..', '..'))
RAW = os.path.join(W8B, 'raw')
TIP_SHA = 'aa34fadff73923bcc69daa3e517bbbf1de693f01659ab86900460f84e0a371a1'
G4_SHA = 'b887850f38633d62419344c41080d1bada92bc931c3307989b6f6fb314a33b6d'
# pre-registered row hashes (tree-f3/window_cmds.md section 3)
HASH_J = '0x30c5438bc6ad9ffa'
HASH_R = '0x6cbe24bf8fafda26'
ROWHASH = {'F3-TD-armed-leaflist': HASH_J, 'F3-TD-armed-kd': HASH_J, 'F3-TA-armed-leaflist': HASH_J,
           'F3-TA-armed-kd': HASH_J, 'F3-TR-armed': HASH_J, 'F3-JT-leaflist': HASH_J, 'F3-JT-kd': HASH_J,
           'F3-RT-leaflist': HASH_R, 'F3-RT-kd': HASH_R}
KERNEL = {'F3-TD-armed-leaflist': 'LeafList', 'F3-TD-armed-kd': 'LeafListKd', 'F3-TA-armed-leaflist': 'LeafList',
          'F3-TA-armed-kd': 'LeafListKd', 'F3-TR-armed': 'RowWalk', 'F3-JT-leaflist': 'LeafList',
          'F3-JT-kd': 'LeafListKd', 'F3-RT-leaflist': 'LeafList', 'F3-RT-kd': 'LeafListKd'}
ARMED = {k: ('armed' in k) for k in ROWHASH}


def load_recs():
    return [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]


def rtext(p):
    try:
        return open(p, encoding='utf-8', errors='replace').read()
    except FileNotFoundError:
        return ''


def summary(d):
    for line in rtext(os.path.join(d, 'stdout.txt')).splitlines():
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
                cols[h].append(float(v) if v != '' else None)
    return cols


def clean_why(r):
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    if rb is None or rb > 5.0:
        why.append(('before', rb))
    if ra is None or ra > 5.0:
        why.append(('after', ra))
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append(('witness', ob))
    if r.get('build_proc_during'):
        why.append(('build', r.get('build_proc_during')))
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(('presence', k))
    return why


def validate(r, s, c):
    row = r['row']
    why = []
    if r.get('exit') != 0:
        why.append('exit')
    if r.get('exe_sha256') != TIP_SHA or r.get('binary') != 'tip':
        why.append('sha/binary')
    if s is None:
        return why + ['no summary']
    if c is None or len(c['wall_ns']) != 500:
        why.append('csv steps')
    if s.get('void_steps') != 0:
        why.append('void')
    if s.get('pose_hash') != ROWHASH[row]:
        why.append(f"pose {s.get('pose_hash')}")
    if s.get('expect_pose') != 'match':
        why.append(f"expect_pose {s.get('expect_pose')}")
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if bool(s.get('armed')) != ARMED[row]:
        why.append('armed')
    if ARMED[row]:
        if s.get('drops_total') != 0:
            why.append('drops')
        if not s.get('w8s') or s['w8s'].get('overflow') != 0:
            why.append('w8s overflow')
    else:
        if s.get('disarmed_ring_traffic') != 0:
            why.append('ring')
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != 'Tree':
        why.append('bp')
    bt = s.get('broadphase_tree') or {}
    if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
        why.append('treediag')
    if bt.get('bp_kernel') != KERNEL[row] or s.get('bp_kernel_flag') != KERNEL[row]:
        why.append(f"kernel {bt.get('bp_kernel')} {s.get('bp_kernel_flag')}")
    kd = bt.get('kd_order_builds')
    if KERNEL[row] == 'LeafListKd':
        if not kd or kd <= 0:
            why.append('kd builds 0 on kd')
    elif kd != 0:
        why.append('kd builds on non-kd')
    if c is not None:
        wm = sum(c['wall_ns'][0:500]) / 500
        if abs(wm - s['window_mean_ns']) > 1.0:
            why.append('window mean')
    if cfg.get('sleeping'):
        why.append('sleeping')
    if not cfg.get('contact_reuse'):
        why.append('reuse')
    a = s.get('args') or []
    want_cfg = 'a' if '-TA-' in row else 'default'
    if s.get('cfg') != want_cfg:
        why.append('cfg')
    want_scene = 'rest' if row.startswith('F3-RT') else 'jolt'
    if s.get('scene') != want_scene:
        why.append('scene')
    return why


def med(xs):
    return statistics.median(xs)


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def se(xs):
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) if len(xs) >= 2 else 0.0


def cell(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    m = med(xs)
    return {'K': len(xs), 'm': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / abs(m),
            'i': iqr(xs) / abs(m), 's': se(xs) / abs(m), 'v': xs}


def cmp(a, b, kmin=3):
    """B against A; bars 2*hypot of the relative spreads; claim needs K >= kmin on both sides."""
    ratio = b['m'] / a['m']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'ris'}
    ok = a['K'] >= kmin and b['K'] >= kmin
    fl = {k: ok and e > bars[k] for k in 'ris'}
    return {'ratio': ratio, 'e': e, 'bars': bars, 'fl': fl, 'KA': a['K'], 'KB': b['K'], 'd': b['m'] - a['m']}




def const(a, bar, kmin=3):
    """A cell against a constant bar b: |m/b - 1| > 2x of the cell's own relative spread."""
    ratio = a['m'] / bar
    e = abs(ratio - 1)
    bars = {k: 2 * a[k] for k in 'ris'}
    ok = a['K'] >= kmin
    fl = {k: ok and e > bars[k] for k in 'ris'}
    return {'ratio': ratio, 'e': e, 'bars': bars, 'fl': fl, 'KA': a['K']}


def yn(c):
    return ''.join('Y' if c['fl'][k] else 'n' for k in 'ris')
