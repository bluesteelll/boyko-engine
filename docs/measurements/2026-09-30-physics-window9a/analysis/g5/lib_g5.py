"""Window 9a, group C4-G5 (broadphase profiles): shared library (results-analyst, 2026-09-30). Read-only on raw/.

Parsers: the window's own tools/lib/driver.py (load_csv, parse_summary).
Statistics: window 8b's synthlib (win8b/analysis/synth/synthlib.py), copied verbatim in arithmetic:
  cell   = median over K of per-process values; i = IQR (inclusive quartiles)/median; s = 1.2533*SD/sqrt(K)/median;
           r = (max-min)/median
  cmp_   = B against A: e = |B/A - 1|; bar_x = 2*hypot(A_x, B_x); flag x iff e > bar_x AND both K >= 3
  vs_const = a cell against a constant bar: flag x iff |m/bar - 1| > 2*x_cell (the rule 8b used for R1's constant bar)
  ruling 1: CLAIMED iff i AND s flag pooled AND in every pass, one sign throughout; STRONG iff r also flags everywhere.
  ruling 8 (W8B): a pass-cell with K < 3 does not gate (the K guard of cmp_; it blocks the claim = the LETTER reading).
Slot rule (ruling 8 as implemented by the driver): the original if clean and valid, else the FIRST clean valid re-run,
else the slot is dropped. Only passes with a pass_done record are used.
Clean = receipt before/after cpu_avg <= 5 %, witness (others_busy_pct) <= 2 %, no build process, no build/lane presence
(the 8b clean_why, recomputed here independently of the driver flag 'contaminated'; agreement is printed).
"""
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W9A = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W9A, 'raw')
sys.path.insert(0, os.path.join(W9A, 'tools', 'lib'))
import driver as D  # noqa: E402

_R = json.load(open(os.path.join(W9A, 'rows9a.json'), encoding='utf-8'))
_X = json.load(open(os.path.join(W9A, 'rows9a.extra.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in _R['rows'] + _X['rows']}
BINS = dict(_R['binaries'])
BINS.update(_X['binaries'])
SUMS = {}
for _line in open(os.path.join(W9A, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if _line.strip():
        _sha, _name = _line.split(maxsplit=1)
        SUMS[_name.strip().lstrip('*')] = _sha

SPANS = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')
SYS = 'sys_physics_broadphase_colored_ns'
SEL = 'sys_select_broadphase_ns'
A, B = 100, 500


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(str(s))
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8', newline='\n').write('\n'.join(self.lines) + '\n')


def all_records(raw=RAW):
    return [json.loads(l) for l in open(os.path.join(raw, 'runs.jsonl'), encoding='utf-8') if l.strip()]


def done_passes(block, recs):
    return {(r['pass'], r.get('pass_attempt', 0)) for r in recs if r.get('block') == block and r.get('pass_done')}


def clean_why(r):
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
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
        rr = r.get(k) or {}
        if rr.get('build_procs_busy'):
            why.append(k + ' build busy')
        pr = rr.get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(k + ' presence')
    return why


def read(p):
    try:
        return open(p, 'rb').read()
    except FileNotFoundError:
        return b''


def sha_ok(r):
    exe = BINS[r['binary']]['exe']
    pin = BINS[r['binary']].get('sha256_pin')
    return r.get('exe_sha256') == pin and SUMS.get(exe) == pin


def load_process(r):
    """An independent light validity check of one runner process, plus its parsed CSV and SUMMARY."""
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    if r.get('hang'):
        why.append('hang')
    if not sha_ok(r):
        why.append('sha256')
    s = D.parse_summary(read(os.path.join(d, 'stdout.txt')))
    c = D.load_csv(os.path.join(d, 'run.csv'))
    if s is None or c is None or 'wall_ns' not in c:
        why.append('no SUMMARY/csv')
    else:
        if len(c['wall_ns']) != row['steps'] or any(x is None for x in c['wall_ns']):
            why.append('csv steps')
        if s.get('void_steps') != 0 or s.get('first_void') is not None:
            why.append('void_steps')
        if row.get('pose_ref') and s.get('expect_pose') != 'match':
            why.append('pose')
        if s.get('workers') != r['W']:
            why.append('workers')
        if abs(sum(c['wall_ns'][0:500]) / 500.0 - s['window_mean_ns']) > 1.0:
            why.append('window mean')
        cfg = s.get('config') or {}
        for k, v in (BINS[r['binary']].get('expect_config') or {}).items():
            if cfg.get(k) != v:
                why.append('config.' + k)
        if cfg.get('broadphase') != row['broadphase']:
            why.append('broadphase %s' % cfg.get('broadphase'))
        bt = s.get('broadphase_tree') or {}
        for k, v in (row.get('tree_diag_expect') or {}).items():
            if bt.get(k) != v:
                why.append('tree_diag.%s %s' % (k, bt.get(k)))
        if row['armed']:
            if not s.get('armed'):
                why.append('not armed')
            if any(v for v in c.get('void', []) if v):
                why.append('csv void')
            # structure (recipe 2.3, the four spans once a step): Tree rows once per step on [100,500), AllPairs never
            want = 1.0 if row['broadphase'] == 'Tree' else 0.0
            for k in SPANS:
                if any(x != want for x in c[k.replace('_ns', '_n')][A:B]):
                    why.append('%s count != %s' % (k, want))
            if any(x != 1.0 for x in c[SYS.replace('_ns', '_n')][A:B]):
                why.append('system span count')
    return {'rec': r, 'row': r['row'], 'binary': r['binary'], 'W': r['W'], 'pass': r['pass'], 'round': r['round'],
            'seq': r['seq'], 'attempt': r['attempt'], 'block': r['block'], 's': s, 'c': c, 'valid_why': why,
            'clean_why': clean_why(r)}


def select(block, recs=None, rows=None):
    """Slot = (block, pass, pass_attempt, seq); passes with pass_done only. Returns (procs, used, dropped)."""
    recs = recs if recs is not None else all_records()
    dp = done_passes(block, recs)
    rr = [r for r in recs if r.get('block') == block and 'row' in r and r.get('attempt') != 'warmup'
          and (r['pass'], r.get('pass_attempt', 0)) in dp and (rows is None or r['row'] in rows)]
    procs = [load_process(r) for r in rr]
    slots = {}
    for p in procs:
        slots.setdefault((p['block'], p['pass'], p['rec'].get('pass_attempt', 0), p['seq']), []).append(p)
    used, dropped = [], []
    for k, ps in sorted(slots.items()):
        orig = [p for p in ps if p['attempt'] == 'original']
        rer = sorted([p for p in ps if p['attempt'] == 'rerun'], key=lambda p: p['rec'].get('rerun_no', 0))
        pick = next((q for q in orig + rer if not q['valid_why'] and not q['clean_why']), None)
        if pick:
            pick['used'] = True
            used.append(pick)
        else:
            dropped.append((k, ps))
    return procs, used, dropped


# ---- per-process values (ms) ----
def wall_mean(p, a=0, b=500):
    return statistics.fmean(p['c']['wall_ns'][a:b]) / 1e6


def col_med(p, k, a=A, b=B):
    return statistics.median(p['c'][k][a:b]) / 1e6


def col_mean(p, k, a=A, b=B):
    return statistics.fmean(p['c'][k][a:b]) / 1e6


def sum4_series(p, a=A, b=B):
    c = p['c']
    return [sum(c[k][i] for k in SPANS) for i in range(a, b)]


def sum4_med(p, a=A, b=B):
    return statistics.median(sum4_series(p, a, b)) / 1e6


def sum4_mean(p, a=A, b=B):
    return statistics.fmean(sum4_series(p, a, b)) / 1e6


def other_series(p, a=A, b=B):
    c = p['c']
    return [c[SYS][i] - sum(c[k][i] for k in SPANS) for i in range(a, b)]


def other_med(p, a=A, b=B):
    return statistics.median(other_series(p, a, b)) / 1e6


def other_mean(p, a=A, b=B):
    return statistics.fmean(other_series(p, a, b)) / 1e6


def by_pass(used, row, W, f):
    v = {}
    for p in used:
        if p['row'] == row and p['W'] == W:
            v.setdefault(p['pass'], []).append(f(p))
    return v


# ---- statistics (the 8b synthlib, verbatim in arithmetic) ----
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
            's': se_med(xs) / dd, 'iqr_abs': iqr(xs), 'se_abs': se_med(xs)}


def cmp_(a, b, kmin=3):
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= kmin and b['K'] >= kmin
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': ok and e > bars['r'],
            'cl_i': ok and e > bars['i'], 'cl_s': ok and e > bars['s'], 'KA': a['K'], 'KB': b['K']}


def vs_const(c, bar, kmin=3):
    e = abs(c['median'] / bar - 1)
    ok = c['K'] >= kmin
    return {'A': bar, 'B': c['median'], 'delta': c['median'] - bar, 'ratio': c['median'] / bar,
            'bar_r': 2 * c['r'], 'bar_i': 2 * c['i'], 'bar_s': 2 * c['s'], 'cl_r': ok and e > 2 * c['r'],
            'cl_i': ok and e > 2 * c['i'], 'cl_s': ok and e > 2 * c['s'], 'KA': None, 'KB': c['K']}


def yn(c):
    return ''.join('Y' if c[k] else 'n' for k in ('cl_i', 'cl_s', 'cl_r'))


def ruling1(pooled, per):
    sign = 1 if pooled['ratio'] > 1 else -1
    same = all((1 if c['ratio'] > 1 else -1) == sign for c in per)
    claimed = pooled['cl_i'] and pooled['cl_s'] and all(c['cl_i'] and c['cl_s'] for c in per) and same
    strong = claimed and pooled['cl_r'] and all(c['cl_r'] for c in per)
    return claimed, strong, ('B>A' if sign > 0 else 'B<A')


def judge(vals_a, vals_b=None, kmin=3, const=None):
    pa = cell([v for k in sorted(vals_a) for v in vals_a[k]])
    if const is not None:
        pooled = vs_const(pa, const, kmin)
        per = [vs_const(cell(vals_a[k]), const, kmin) for k in sorted(vals_a)]
        cl, st, dr = ruling1(pooled, per)
        return {'a': pa, 'b': None, 'pooled': pooled, 'per': per, 'claimed': cl, 'strong': st, 'dir': dr,
                'Ks': [len(vals_a[k]) for k in sorted(vals_a)],
                'pcells': {k: cell(vals_a[k]) for k in sorted(vals_a)}}
    pb = cell([v for k in sorted(vals_b) for v in vals_b[k]])
    pooled = cmp_(pa, pb, kmin)
    ks = sorted(set(vals_a) | set(vals_b))
    per = [cmp_(cell(vals_a[k]), cell(vals_b[k]), kmin) for k in ks]
    cl, st, dr = ruling1(pooled, per)
    return {'a': pa, 'b': pb, 'pooled': pooled, 'per': per, 'claimed': cl, 'strong': st, 'dir': dr,
            'Ks': [(len(vals_a.get(k, [])), len(vals_b.get(k, []))) for k in ks]}


def label(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    return 'CLAIMED STRONG' if j['strong'] else 'CLAIMED'


def fcell(c, d=4, scale=1.0, unit=''):
    return ('%.*f%s [%.*f-%.*f] n=%d (IQR %.2f %%, SE %.2f %%, range %.2f %%)'
            % (d, c['median'] * scale, unit, d, c['min'] * scale, d, c['max'] * scale, c['K'],
               100 * c['i'], 100 * c['s'], 100 * c['r']))


def fj(j, d=4):
    p = j['pooled']
    s = ('B/A %.4f (%+.2f %%, B-A %+.*f) pooled %s (bars i %.2f / s %.2f / r %.2f %%); passes '
         % (p['ratio'], 100 * (p['ratio'] - 1), d, p['delta'], yn(p), 100 * p['bar_i'], 100 * p['bar_s'],
            100 * p['bar_r']))
    s += ' '.join('p%d:%s(K%s/%s)' % (k, yn(c), c['KA'], c['KB']) for k, c in enumerate(j['per']))
    return s + ' -> %s [%s]' % (label(j), j['dir'])
