"""Window 8b synthesis library (results-analyst, 2026-09-29). Read-only on raw/, rows8b*.json, bin/SHA256SUMS.

An independent re-derivation of the numbers the window synthesis (analysis.md) states, so that every number there is
computed by a script run for the synthesis itself, not only copied from the per-group sections. Parsers are the
window's own (tools/lib/driver.py load_csv / parse_summary, tools/micro8b.py parse / process_rules); statistics are
window 8's lib8 (win8/analysis/lib8.py) verbatim in arithmetic, with the K guard made a parameter:

  cell   = median over K of per-process values; r = (max-min)/median; i = IQR (inclusive quartiles)/median;
           s = 1.2533*SD/sqrt(K)/median
  cmp_   = B against A: e = |B/A - 1|; bar_x = 2*hypot(A_x, B_x); flag x iff e > bar_x AND both K >= kmin
  ruling 1 (RULINGS-2026-09-27-W8 #1): CLAIMED iff i AND s flag pooled AND in every pass, one sign throughout;
           STRONG iff r also flags pooled and in every pass.

Two readings of the K guard are carried everywhere (the F3 reconciliation D2):
  LETTER: kmin = 3, window 7's "No claim when either side has K < 3" (lib8.cmp_), never lifted by ruling 1;
  K2    : kmin = 2, a K = 2 pass-cell gates on its own i and s (needs an orchestrator ruling).
Slot rule (windows 7/8): the original if valid and clean, else its re-run if valid and clean, else the slot is dropped.
Clean = receipts before/after <= 5 %, witness <= 2 %, no build process, no build/lane presence (lib8.clean)."""
import json
import math
import os
import re
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W8B, 'raw')
sys.path.insert(0, os.path.join(W8B, 'tools', 'lib'))
sys.path.insert(0, os.path.join(W8B, 'tools'))
import driver as D  # noqa: E402  load_csv, parse_summary
import micro8b  # noqa: E402  parse, process_rules

_R = json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))
_X = json.load(open(os.path.join(W8B, 'rows8b.extra.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in _R['rows'] + _X['rows']}
BINS = dict(_R['binaries'])
BINS.update(_X['binaries'])
SUMS = {}
for _line in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if _line.strip():
        _sha, _name = _line.split(maxsplit=1)
        SUMS[_name.strip().lstrip('*')] = _sha


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(str(s))
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8').write('\n'.join(self.lines) + '\n')


def all_records():
    return [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]


def procs_of(block, recs=None):
    recs = recs if recs is not None else all_records()
    return [r for r in recs if r.get('block') == block and 'row' in r and r.get('attempt') != 'warmup']


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
            why.append(f'{k} presence')
    return why


def read(p, mode='rb'):
    try:
        return open(p, mode).read()
    except FileNotFoundError:
        return b'' if 'b' in mode else ''


def sha_ok(r):
    exe = BINS[r['binary']]['exe']
    pin = BINS[r['binary']].get('sha256_pin') or SUMS.get(exe)
    return r.get('exe_sha256') == pin and SUMS.get(exe) == pin


def dm1_zones(art):
    zones = {}
    for blk in re.findall(r'\[\[zone\]\](.*?)(?=\[\[zone\]\]|\Z)', art, re.S):
        kv = dict(re.findall(r'(\w+) = ([^\n]+)', blk))
        z = {}
        for k, v in kv.items():
            v = v.strip()
            try:
                z[k] = float(v)
            except ValueError:
                z[k] = v.strip('"')
        zones[int(z['id'])] = z
    return zones


def load_process(r):
    """Parse the process's own files; returns dict with validity reasons (a light, independent check) and payload."""
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if r.get('hang'):
        why.append('hang')
    if not sha_ok(r):
        why.append('sha256')
    p = {'rec': r, 'row': r['row'], 'binary': r['binary'], 'W': r['W'], 'pass': r['pass'], 'round': r['round'],
         'seq': r['seq'], 'attempt': r['attempt'], 'block': r['block']}
    kind = r['kind']
    if kind == 'runner':
        s = D.parse_summary(read(os.path.join(d, 'stdout.txt')))
        c = D.load_csv(os.path.join(d, 'run.csv'))
        if s is None or c is None or 'wall_ns' not in c:
            why.append('no SUMMARY/csv')
        else:
            if len(c['wall_ns']) != row['steps'] or any(x is None for x in c['wall_ns']):
                why.append('csv steps')
            if s.get('void_steps') != 0:
                why.append('void_steps')
            if '--expect-pose' in row['args'] and s.get('expect_pose') != 'match':
                why.append('pose')
            if s.get('workers') != r['W']:
                why.append('workers')
            wm = sum(c['wall_ns'][0:500]) / 500.0
            if abs(wm - s['window_mean_ns']) > 1.0:
                why.append('window mean')
        p['s'], p['c'] = s, c
    elif kind == 'micro':
        ss, cal = micro8b.parse(read(os.path.join(d, 'stdout.txt'), 'r'))
        w, _notes = micro8b.process_rules(row, ss, cal)
        why += w
        p['ss'], p['cal'] = ss, cal
    elif kind == 'dm1':
        art = read(os.path.join(d, 'artifact.toml'), 'r')
        z = dm1_zones(art) if art else {}
        pm = re.search(r'^present_mode = "(\w+)"', art, re.M)
        if not pm or pm.group(1) != 'fifo':
            why.append('present mode')
        for name, zid in row['need_zones'].items():
            if zid not in z or int(z[zid]['n']) != 220:
                why.append(f'zone {name}')
        p['zones'] = z
    elif kind == 'criterion':
        est = {}
        for i in row['expect']:
            sp = os.path.join(d, 'criterion', *i.split('/'), 'new', 'sample.json')
            if not os.path.exists(sp):
                why.append(f'missing {i}')
                continue
            smp = json.load(open(sp, encoding='utf-8'))
            est[i] = statistics.fmean(t / n for t, n in zip(smp['times'], smp['iters'])) / 1e3  # us
        p['est'] = est
    p['valid_why'] = why
    p['clean_why'] = clean_why(r)
    return p


def select(block, recs=None):
    """Slot = (block, pass, seq). Returns (procs, used, dropped)."""
    procs = [load_process(r) for r in procs_of(block, recs)]
    slots = {}
    for p in procs:
        slots.setdefault((p['block'], p['pass'], p['seq']), []).append(p)
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
    return procs, used, dropped


# ---- statistics (lib8 verbatim in arithmetic) ----
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
    """A cell against a constant bar (window_cmds R1): flag x iff |m/bar - 1| > 2*x_cell."""
    e = abs(c['median'] / bar - 1)
    ok = c['K'] >= kmin
    return {'A': bar, 'B': c['median'], 'delta': c['median'] - bar, 'ratio': c['median'] / bar,
            'bar_r': 2 * c['r'], 'bar_i': 2 * c['i'], 'bar_s': 2 * c['s'], 'cl_r': ok and e > 2 * c['r'],
            'cl_i': ok and e > 2 * c['i'], 'cl_s': ok and e > 2 * c['s'], 'KA': None, 'KB': c['K']}


def yn(c):
    return ''.join('Y' if c[k] else 'n' for k in ('cl_i', 'cl_s', 'cl_r'))


def ruling1(pooled, per):
    """pooled: cmp dict; per: list of cmp dicts (one per pass). Returns (verdict, strong, direction)."""
    sign = 1 if pooled['ratio'] > 1 else -1
    same = all((1 if c['ratio'] > 1 else -1) == sign for c in per)
    claimed = pooled['cl_i'] and pooled['cl_s'] and all(c['cl_i'] and c['cl_s'] for c in per) and same
    strong = claimed and pooled['cl_r'] and all(c['cl_r'] for c in per)
    return claimed, strong, ('B>A' if sign > 0 else 'B<A')


def judge(vals_a, vals_b, kmin=3, const=None):
    """vals_*: {pass: [values]}. Pooled over all passes + per pass. Returns dict."""
    pa = cell([v for k in sorted(vals_a) for v in vals_a[k]])
    if const is not None:
        pooled = vs_const(pa, const, kmin)
        per = [vs_const(cell(vals_a[k]), const, kmin) for k in sorted(vals_a)]
        cl, st, dr = ruling1(pooled, per)
        return {'a': pa, 'b': None, 'pooled': pooled, 'per': per, 'claimed': cl, 'strong': st, 'dir': dr,
                'Ks': [len(vals_a[k]) for k in sorted(vals_a)]}
    pb = cell([v for k in sorted(vals_b) for v in vals_b[k]])
    pooled = cmp_(pa, pb, kmin)
    per = [cmp_(cell(vals_a[k]), cell(vals_b[k]), kmin) for k in sorted(set(vals_a) | set(vals_b))]
    cl, st, dr = ruling1(pooled, per)
    return {'a': pa, 'b': pb, 'pooled': pooled, 'per': per, 'claimed': cl, 'strong': st, 'dir': dr,
            'Ks': [(len(vals_a.get(k, [])), len(vals_b.get(k, []))) for k in sorted(set(vals_a) | set(vals_b))]}


def label(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    return 'CLAIMED STRONG' if j['strong'] else 'CLAIMED'


def fcell(c, d=4):
    return (f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] n={c['K']} "
            f"(IQR {100 * c['i']:.2f} %, SE {100 * c['s']:.2f} %, range {100 * c['r']:.2f} %)")


def fj(j, d=4):
    p = j['pooled']
    s = (f"B/A {p['ratio']:.4f} ({100 * (p['ratio'] - 1):+.2f} %, B-A {p['delta']:+.{d}f}) pooled {yn(p)} "
         f"(bars i {100 * p['bar_i']:.2f} / s {100 * p['bar_s']:.2f} / r {100 * p['bar_r']:.2f} %); passes "
         + ' '.join(f"p{k}:{yn(c)}(K{c['KA']}/{c['KB']})" for k, c in enumerate(j['per']))
         + f" -> {label(j)} [{j['dir']}]")
    return s
