"""Window 8b, block group F3 + F3-G4: the analyst's library (read-only on raw/).

Statistics follow window 8's analysis (win8/analysis/lib8.py) verbatim: a cell is the median over K of per-process
values; r = (max-min)/median, i = IQR/median (linear 'inclusive' quartiles), s = 1.2533*SD/sqrt(K)/median.
B against A: e = |B/A - 1|, bar_x = 2*hypot(x_A, x_B); flag x set iff K_A >= 3 and K_B >= 3 and e > bar_x.
Against a constant bar b (window 7 wave 2's rule, the spread of the cell alone): e = |m/b - 1|, bar_x = 2*x.
Ruling 1 (RULINGS-2026-09-27-W8): CLAIMED iff i AND s hold, in the same direction, in every clean pass AND pooled;
STRONG iff r also holds everywhere i and s were required.
Slot rule (windows 6-8): per (block, pass, round, row, binary, W) the original if valid and clean, else its re-run if
valid and clean, else the slot is dropped. Clean = receipts before/after <= 5.0 pct, witness (others_busy_pct) <= 2.0
pct, no build process during, no build/lane presence in either receipt.
Validity is re-derived from the files each process left (SUMMARY line, run.csv, pose.bin, criterion/), never from
the driver fields valid/contaminated/mean_ms/cols (those are compared afterwards)."""
import csv
import hashlib
import json
import math
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W8B, 'raw')
ROWS = {r['id']: r for r in json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))['rows']}
SUMS = {}
for _l in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if _l.strip():
        _h, _n = _l.split(maxsplit=1)
        SUMS[_n.strip().lstrip('*')] = _h
KEY2EXE = {'tip': 'bin/runner_tip_16191fda.exe', 'g4ref': 'bin/broadphase_g4ref_16191fda.exe'}
# the pre-registered hashes (tree-f3/window_cmds.md section 3, the table's last column)
ROW_HASH = {'F3-TD-armed': '0x30c5438bc6ad9ffa', 'F3-TA-armed': '0x30c5438bc6ad9ffa', 'F3-TR-armed': '0x30c5438bc6ad9ffa',
            'F3-JT': '0x30c5438bc6ad9ffa', 'F3-RT': '0x6cbe24bf8fafda26'}
KERNEL_FLAG = {'leaflist': 'LeafList', 'leaflist-kd': 'LeafListKd', 'rowwalk': 'RowWalk'}
SPANS4 = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')


def row_family(rid):
    for f in ROW_HASH:
        if rid == f or rid.startswith(f + '-'):
            return f
    raise KeyError(rid)


def load_recs():
    return [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]


def read_text(p):
    try:
        return open(p, encoding='utf-8', errors='replace').read()
    except FileNotFoundError:
        return ''


def summaries_of(txt):
    return [json.loads(line[8:]) for line in txt.splitlines() if line.startswith('SUMMARY ')]


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


def md5(p):
    return hashlib.md5(open(p, 'rb').read()).hexdigest() if os.path.exists(p) else None


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
    d = abs(m) if m else float('inf')
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / d, 'i': iqr(xs) / d,
            's': se_(xs) / d, 'values': xs}


def sgn(x):
    return 1 if x > 0 else (-1 if x < 0 else 0)


def cmp_(a, b, kmin=3):
    """B against A (window 8's rule): bars 2*hypot of the two cells' relative spreads."""
    if not a or not b:
        return None
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    ok = a['K'] >= kmin and b['K'] >= kmin
    out = {'A': a['median'], 'B': b['median'], 'ratio': ratio, 'delta': b['median'] - a['median'], 'KA': a['K'],
           'KB': b['K'], 'sign': sgn(ratio - 1)}
    for k in 'ris':
        out['bar_' + k] = 2 * math.hypot(a[k], b[k])
        out['cl_' + k] = ok and e > out['bar_' + k]
    return out


def vs_bar(c, b, kmin=3):
    """A cell against a constant bar b: bars 2*x of the cell alone."""
    if not c:
        return None
    e = c['median'] / b - 1
    ok = c['K'] >= kmin
    out = {'A': b, 'B': c['median'], 'ratio': c['median'] / b, 'delta': c['median'] - b, 'KA': None, 'KB': c['K'],
           'sign': sgn(e)}
    for k in 'ris':
        out['bar_' + k] = 2 * c[k]
        out['cl_' + k] = ok and abs(e) > out['bar_' + k]
    return out


def yn(c):
    return '/'.join(('Y' if c['cl_' + k] else 'n') for k in 'ris')


def ruling1(per_pass, pooled):
    """per_pass: [(pass, cmp)] over every clean pass; pooled: the cmp over the union. Direction = pooled sign."""
    allc = [('pooled', pooled)] + [('p%d' % p, c) for p, c in per_pass]
    d = pooled['sign']
    is_ = all(c['cl_i'] and c['cl_s'] and c['sign'] == d for _, c in allc)
    r_ = all(c['cl_r'] and c['sign'] == d for _, c in allc)
    rs_every = all(c['cl_r'] and c['cl_s'] and c['sign'] == d for _, c in allc)
    notes = []
    for n, c in allc:
        if c['KB'] < 3 or (c['KA'] is not None and c['KA'] < 3):
            notes.append('%s K %s/%s' % (n, c['KA'], c['KB']))
    flags = {}
    for n, c in allc:
        flags[n] = yn(c) + ('' if c['sign'] == d else '(opp)')
    return {'dir': d, 'claimed': is_, 'strong': is_ and r_, 'rs_every': rs_every,
            'rs_pooled': pooled['cl_r'] and pooled['cl_s'], 'flags': flags, 'notes': notes}


def fcell(c, scale=1.0, d=4):
    if not c:
        return 'n/a'
    return '%.*f [%.*f-%.*f] n=%d (r %.2f / i %.2f / s %.2f %%)' % (
        d, c['median'] * scale, d, c['min'] * scale, d, c['max'] * scale, c['K'], 100 * c['r'], 100 * c['i'],
        100 * c['s'])


def fcmp(c):
    return '%.4f (%+.2f %%) %s bars r/i/s %.2f/%.2f/%.2f %%' % (
        c['ratio'], 100 * (c['ratio'] - 1), yn(c), 100 * c['bar_r'], 100 * c['bar_i'], 100 * c['bar_s'])
