"""Window 8b omega group analyst library (results-analyst, 2026-09-29). Read-only on raw/.
Re-parses every micro process's stdout.txt itself; validity = tools/micro8b.py's rules (reused, not re-invented) plus
exit 0 and the sha256 pin; clean = window 8's rule (lib8.clean: receipts <= 5 %, witness <= 2 %, no build process);
slot rule = window 8's and the 8b protocol's: original if valid and clean, else its re-run if valid and clean, else the
slot is dropped. Statistics = window 8's lib8 (cell: median, IQR inclusive, SE = 1.2533 SD / sqrt(K), min-max, all
relative to the median; cmp: |B/A - 1| > 2 hypot(rel_A, rel_B) per spread)."""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W8B, 'raw')
sys.path.insert(0, os.path.join(W8B, 'tools'))
import micro8b  # noqa: E402  (the window's own micro rules)

EXTRA = json.load(open(os.path.join(W8B, 'rows8b.extra.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in EXTRA['rows']}
PIN = EXTRA['binaries']['omega2']['sha256_pin']
SUMS = {}
for line in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS[name.strip().lstrip('*')] = sha


def load_recs(blocks=('omega-v2', 'omega-v1-cont')):
    out = []
    for line in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8'):
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


def clean_why(r):
    """window 8 lib8.clean, same thresholds."""
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


def process(r):
    """One process: SUMMARY/CALIBRATION parsed from its own stdout.txt, validity reasons, park notes, clean reasons."""
    so = read_text(os.path.join(r['cwd'], 'stdout.txt'))
    ss, cal = micro8b.parse(so)
    row = ROWS[r['row']]
    why, notes = micro8b.process_rules(row, ss, cal)
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if r.get('exe_sha256') != PIN or SUMS.get('bin/omega_b_region_v2_a3adc827.exe') != PIN:
        why.append('sha')
    return {'block': r['block'], 'pass': r['pass'], 'round': r['round'], 'row': r['row'], 'attempt': r.get('attempt'),
            'seq': r.get('seq'), 'start': r.get('start'), 'cwd': r['cwd'], 'summaries': ss, 'cal': cal,
            'valid_why': why, 'notes': notes, 'clean_why': clean_why(r), 'driver_valid': r.get('valid'),
            'driver_contaminated': r.get('contaminated'), 'placement': r.get('placement') or {},
            'others_busy_pct': r.get('others_busy_pct'),
            'rb': (r.get('receipt_before') or {}).get('cpu_avg'), 'ra': (r.get('receipt_after') or {}).get('cpu_avg')}


def select(procs):
    """window 8 / 8b slot rule. Returns (used, dropped_slots, slots)."""
    slots = {}
    for p in procs:
        if p['attempt'] == 'warmup':
            continue
        k = (p['block'], p['pass'], p['round'], p['row'])
        slots.setdefault(k, []).append(p)
    used, dropped = [], []
    for k, ps in sorted(slots.items()):
        orig = [p for p in ps if p['attempt'] == 'original']
        rer = [p for p in ps if p['attempt'] == 'rerun']
        pick = next((c for c in orig + rer if not c['valid_why'] and not c['clean_why']), None)
        if pick:
            pick['used'] = True
            used.append(pick)
        else:
            dropped.append((k, ps))
    return used, dropped, slots


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
            's': se_med(xs) / dd, 'IQR': iqr(xs), 'SE': se_med(xs), 'values': xs}


def cmp_(a, b):
    """B against A (window 7/8 rule); claimed flags per spread; K >= 3 on both sides required (lib8)."""
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
    if c is None:
        return 'n/a'
    return ('Y' if c['cl_r'] else 'n') + '/' + ('Y' if c['cl_i'] else 'n') + '/' + ('Y' if c['cl_s'] else 'n')


def rule1(pooled, per_pass):
    """Ruling 1: CLAIMED iff i AND s clear pooled AND in every clean pass (same direction); STRONG if r also clears
    in all of them. per_pass: {pass: cmp or None}."""
    parts = [('pooled', pooled)] + [('p%d' % k, v) for k, v in sorted(per_pass.items())]
    if pooled is None:
        return 'n/a', ''
    ok_is = all(c is not None and c['cl_i'] and c['cl_s'] for _, c in parts)
    ok_r = all(c is not None and c['cl_r'] for _, c in parts)
    same_dir = all(c is not None and (c['delta'] > 0) == (pooled['delta'] > 0) for _, c in parts)
    det = ' '.join('%s %s' % (n, yn(c)) for n, c in parts)
    if ok_is and same_dir:
        return ('CLAIMED STRONG' if ok_r else 'CLAIMED'), det
    return 'NOT CLAIMED', det


def fc(c, d=0, scale=1.0, unit=''):
    if not c:
        return 'n/a'
    f = '%.' + str(d) + 'f'
    return ((f % (c['median'] * scale)) + unit + ' [IQR ' + (f % (c['IQR'] * scale)) + ', SE ' + (f % (c['SE'] * scale))
            + ', ' + (f % (c['min'] * scale)) + '-' + (f % (c['max'] * scale)) + '] n=' + str(c['K']))


def fcmp(c, d=0, scale=1.0):
    if not c:
        return 'n/a'
    f = '%+.' + str(d) + 'f'
    return ('B-A ' + (f % (c['delta'] * scale)) + ' (%+.1f %%) ' % (100 * (c['ratio'] - 1)) + yn(c)
            + ' bars r %.1f / i %.1f / s %.1f %%' % (100 * c['bar_r'], 100 * c['bar_i'], 100 * c['bar_s']))


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(s)
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8').write('\n'.join(self.lines) + '\n')


def all_procs():
    return [process(r) for r in load_recs()]
