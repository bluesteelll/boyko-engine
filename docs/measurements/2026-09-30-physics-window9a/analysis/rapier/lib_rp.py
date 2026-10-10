"""Window 9a, block group "Rapier 0.36" (results-analyst, 2026-09-30). Read-only on raw/, rows9a*.json, bin/, gate/.

Method = window 8b's synthesis (win8b/analysis/synth/synthlib.py), statistics verbatim in arithmetic:
  cell   = median over K of per-process values; r = (max-min)/median; i = IQR (inclusive quartiles)/median;
           s = 1.2533*SD/sqrt(K)/median
  cmp_   = B against A: e = |B/A - 1|; bar_x = 2*hypot(A_x, B_x); flag x iff e > bar_x AND both K >= kmin
  ruling 1: CLAIMED iff i AND s flag pooled AND in every pass, one sign throughout; STRONG iff r also flags pooled and
           in every pass.  Ruling 8 (W8B): a pass-cell with K < 3 does not gate (LETTER: it sets no flag, so the
           every-pass requirement fails).  An alternative reading EXCL (a short pass-cell is dropped from the every-pass
           requirement; pooled keeps its processes) is computed beside it and labelled: it needs an orchestrator ruling.
Per-process value: the mean of wall_ns (runner, Rapier: run.csv) / Time (ms) (Jolt: per_frame_*.csv) over the
metric window, recomputed from the process own file.  Slot rule (ruling 8 reduction): the original if valid and
clean, else the FIRST clean valid re-run, else the slot is dropped.  Clean = the driver receipts (before/after cpu
<= 5 %, witness others_busy <= 2 %, no build process during) re-derived here, plus 8b presence check.
Only passes with a pass_done record are used."""
import csv
import json
import math
import os
import statistics
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W9A = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W9A, 'raw')

_R = json.load(open(os.path.join(W9A, 'rows9a.json'), encoding='utf-8'))
_X = json.load(open(os.path.join(W9A, 'rows9a.extra.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in _R['rows'] + _X['rows']}
BINS = dict(_R['binaries'])
BINS.update(_X['binaries'])
SUMS = {}
for _line in open(os.path.join(W9A, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if _line.strip():
        _sha, _name = _line.split(maxsplit=1)
        SUMS[os.path.basename(_name.strip().lstrip('*'))] = _sha
FIX = os.path.join(W9A, 'gate', 'fixtures')

# Row-iteration pins (window9a_rows.md rows_pin; audit.md section 1).  Ours is re-derived from C4-JD-armed#tip (G5).
RAPIER_ROWS = {'RP-D': {(0, 100): 303003, (100, 500): 354674}, 'RP-M': {(0, 100): 935729, (100, 500): 1071107}}
WINDOWS = [(100, 500), (0, 100), (0, 500)]


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(str(s))
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8').write(chr(10).join(self.lines) + chr(10))


def all_records():
    return [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]


def closed_passes(recs):
    return {(r['run_tag'], r['block'], r['pass'], r['pass_attempt']) for r in recs if r.get('pass_done')}


def procs_of(block, recs):
    cl = closed_passes(recs)
    return [r for r in recs if r.get('block') == block and 'row' in r and not r.get('passcell') and not r.get('r4')
            and r.get('attempt') != 'warmup' and (r['run_tag'], r['block'], r['pass'], r['pass_attempt']) in cl]


def clean_why(r):
    why = []
    for k in ('receipt_before', 'receipt_after'):
        rc = r.get(k) or {}
        v = rc.get('cpu_avg')
        if v is None or v > 5.0:
            why.append(f'{k} {v}')
        if rc.get('build_procs_busy'):
            why.append(f'{k} build busy')
        pr = rc.get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(f'{k} presence')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append(f'witness {ob}')
    if r.get('build_proc_during'):
        why.append('build during')
    return why


def exe_sha_ok(r):
    b = BINS[r['binary']]
    exe = b.get('exe') or b.get('path') or ''
    pin = b.get('sha256_pin') or SUMS.get(os.path.basename(exe))
    return pin is not None and r.get('exe_sha256') == pin


def load_csv(path):
    if not os.path.isfile(path):
        return None
    rows = list(csv.reader(open(path, encoding='utf-8', newline='')))
    hdr = [h.strip() for h in rows[0]]
    cols = {h: [] for h in hdr}
    for rr in rows[1:]:
        for h, v in zip(hdr, rr):
            v = v.strip()
            try:
                cols[h].append(float(v) if v != '' else None)
            except ValueError:
                cols[h].append(None)
    return cols, hdr


def parse_summary(path):
    try:
        for line in open(path, 'rb').read().decode('utf-8', 'replace').splitlines():
            if line.startswith('SUMMARY '):
                return json.loads(line[8:])
    except FileNotFoundError:
        return None
    return None


_FIXB = {}


def fixture_bytes(name):
    if name not in _FIXB:
        _FIXB[name] = open(os.path.join(FIX, name + '.pose'), 'rb').read()
    return _FIXB[name]


def load_process(r):
    """Independent light validity plus the per-window means, from the process files."""
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if r.get('hang'):
        why.append('hang')
    if not exe_sha_ok(r):
        why.append('sha256')
    p = {'rec': r, 'row': r['row'], 'binary': r['binary'], 'W': r['W'], 'pass': r['pass'], 'round': r['round'],
         'seq': r['seq'], 'attempt': r['attempt'], 'rerun_no': r.get('rerun_no', 0), 'block': r['block'],
         'start': r.get('start'), 'kind': r['kind']}
    wall = None
    if r['kind'] in ('runner', 'rapier'):
        s = parse_summary(os.path.join(d, 'stdout.txt'))
        lc = load_csv(os.path.join(d, 'run.csv'))
        if s is None or lc is None:
            why.append('no SUMMARY/csv')
        else:
            c, hdr = lc
            wall = c.get('wall_ns')
            if wall is None or len(wall) != 500 or any(x is None or x <= 0 for x in wall):
                why.append('csv wall_ns')
            if s.get('expect_pose') != 'match':
                why.append('expect_pose')
            if s.get('workers') != r['W']:
                why.append('workers')
            if r['kind'] == 'runner':
                if s.get('void_steps') != 0:
                    why.append('void_steps')
            else:
                if hdr != ['step', 'wall_ns', 'pool_threads']:
                    why.append('csv header')
                if any(x != r['W'] for x in c['pool_threads']):
                    why.append('pool_threads column')
                t = s.get('threads') or {}
                if not (t.get('pool_threads_min') == t.get('pool_threads_max') == r['W']
                        and t.get('install_receipt') == [r['W'], r['W']] and t.get('loop_on_pool_worker') is True):
                    why.append('V3 threads')
                if (s.get('config') or {}).get('counters_enabled') is not False:
                    why.append('V5 counters')
                if s.get('void') is not False or s.get('perturb') is not None:
                    why.append('V6 void/perturb')
                if s.get('arm') != BINS[r['binary']].get('arm'):
                    why.append('V4 arm')
                if '--receipt' in (s.get('args') or []):
                    why.append('V9 receipt')
                fx = row['pose_ref'][r['binary']]
                try:
                    if open(os.path.join(d, 'pose.bin'), 'rb').read() != fixture_bytes(fx):
                        why.append('V7 pose bytes')
                except FileNotFoundError:
                    why.append('V7 no pose')
            p['s'] = s
    elif r['kind'] == 'jolt':
        pf = [f for f in os.listdir(d) if f.startswith('per_frame_')]
        lc = load_csv(os.path.join(d, pf[0])) if pf else None
        if lc is None or 'Time (ms)' not in lc[0]:
            why.append('no per_frame csv')
        else:
            wall = [x * 1e6 for x in lc[0]['Time (ms)']]
            if len(wall) != 500:
                why.append('frames')
        so = open(os.path.join(d, 'stdout.txt'), 'rb').read().decode('utf-8', 'replace')
        stat = [l for l in so.splitlines() if l.startswith('Discrete,')]
        if len(stat) != 1 or int(stat[0].split(',')[1]) != r['W'] or stat[0].split(',')[3].strip() != row['jolt_hash']:
            why.append('jolt stat line / threads / hash')
        if 'allow_sleep=0' not in so or 'receipt=0' not in so:
            why.append('jolt banner')
    p['wall'] = wall
    p['vals'] = {w: (statistics.fmean(wall[w[0]:w[1]]) if wall and len(wall) == 500 else None) for w in WINDOWS}
    dc = r.get('cols') or {}
    p['driver_agree'] = all(abs(p['vals'][w] - dc[f'{w[0]}..{w[1]}']['wall_ns']['mean']) <= 1.0
                            for w in WINDOWS if p['vals'][w] is not None and f'{w[0]}..{w[1]}' in dc)
    p['valid_why'] = why
    p['driver_valid'] = bool(r.get('valid'))
    p['clean_why'] = clean_why(r)
    p['driver_clean'] = not r.get('contaminated')
    return p


def select(block, recs, rows=None):
    """Slot = (block, pass, pass_attempt, round, row, binary, W). Returns (procs, used, dropped)."""
    procs = [load_process(r) for r in procs_of(block, recs) if rows is None or r['row'] in rows]
    slots = {}
    for p in procs:
        rr = p['rec']
        slots.setdefault((rr['block'], rr['pass'], rr['pass_attempt'], rr['round'], rr['row'], rr['binary'], rr['W']),
                         []).append(p)
    used, dropped = [], []
    for k, ps in sorted(slots.items(), key=lambda kv: str(kv[0])):
        orig = [p for p in ps if p['attempt'] == 'original']
        rer = sorted([p for p in ps if p['attempt'] == 'rerun'], key=lambda p: p['rerun_no'])
        pick = next((c for c in orig + rer if not c['valid_why'] and not c['clean_why']), None)
        if pick:
            pick['used'] = True
            used.append(pick)
        else:
            dropped.append((k, ps))
    return procs, used, dropped


def by_pass(used, row, binary, W, window, scale=1.0):
    out = {0: [], 1: [], 2: []}
    for p in used:
        if p['row'] == row and p['binary'] == binary and p['W'] == W:
            out[p['pass']].append(p['vals'][window] / scale)
    return out


# ---- statistics (lib8 / synthlib verbatim in arithmetic) ----
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
    if a is None or b is None:
        return {'A': a and a['median'], 'B': b and b['median'], 'ratio': None, 'cl_r': False, 'cl_i': False,
                'cl_s': False, 'KA': a['K'] if a else 0, 'KB': b['K'] if b else 0, 'bar_r': None, 'bar_i': None,
                'bar_s': None, 'delta': None}
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= kmin and b['K'] >= kmin
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': ok and e > bars['r'],
            'cl_i': ok and e > bars['i'], 'cl_s': ok and e > bars['s'], 'KA': a['K'], 'KB': b['K']}


def yn(c):
    return ''.join('Y' if c[k] else 'n' for k in ('cl_i', 'cl_s', 'cl_r'))


def ruling1(pooled, per, gating=None):
    """gating: list of bools per pass (EXCL reading: a non-gating pass is skipped); None = LETTER (every pass)."""
    use = [c for k, c in enumerate(per) if gating is None or gating[k]]
    sign = 1 if pooled['ratio'] > 1 else -1
    same = all(c['ratio'] is not None and (1 if c['ratio'] > 1 else -1) == sign for c in use)
    claimed = bool(pooled['cl_i'] and pooled['cl_s'] and all(c['cl_i'] and c['cl_s'] for c in use) and same)
    strong = bool(claimed and pooled['cl_r'] and all(c['cl_r'] for c in use))
    return claimed, strong, ('B>A' if sign > 0 else 'B<A')


def judge(vals_a, vals_b, kmin=3):
    """vals_*: {pass: [values]}. Pooled over all passes plus per pass, under LETTER and EXCL."""
    pa = cell([v for k in sorted(vals_a) for v in vals_a[k]])
    pb = cell([v for k in sorted(vals_b) for v in vals_b[k]])
    pooled = cmp_(pa, pb, kmin)
    ks = sorted(set(vals_a) | set(vals_b))
    per = [cmp_(cell(vals_a.get(k, [])), cell(vals_b.get(k, [])), kmin) for k in ks]
    gating = [len(vals_a.get(k, [])) >= 3 and len(vals_b.get(k, [])) >= 3 for k in ks]
    cl, st, dr = ruling1(pooled, per)
    cle, ste, _ = ruling1(pooled, per, gating)
    return {'a': pa, 'b': pb, 'pooled': pooled, 'per': per, 'claimed': cl, 'strong': st, 'dir': dr,
            'claimed_excl': cle, 'strong_excl': ste, 'gating': gating,
            'Ks': [(len(vals_a.get(k, [])), len(vals_b.get(k, []))) for k in ks]}


def label(j, excl=False):
    c, s = (j['claimed_excl'], j['strong_excl']) if excl else (j['claimed'], j['strong'])
    if not c:
        return 'NOT CLAIMED'
    return 'CLAIMED STRONG' if s else 'CLAIMED'


def fcell(c, scale=1e6, d=4, unit=''):
    return (f"{c['median'] / scale:.{d}f}{unit} [{c['min'] / scale:.{d}f}-{c['max'] / scale:.{d}f}] n={c['K']} "
            f"(IQR {100 * c['i']:.2f} %, SE {100 * c['s']:.2f} %, range {100 * c['r']:.2f} %)")


def fyn(c):
    return yn(c) if c.get('ratio') is not None else '---'


def fj(j):
    p = j['pooled']
    per = ' '.join(f"p{k}:{fyn(c)}(K{c['KA']}/{c['KB']})" for k, c in enumerate(j['per']))
    return (f"B/A {p['ratio']:.4f} ({100 * (p['ratio'] - 1):+.2f} %) pooled {yn(p)} "
            f"(bars i {100 * p['bar_i']:.2f} / s {100 * p['bar_s']:.2f} / r {100 * p['bar_r']:.2f} %); passes {per} "
            f"-> LETTER {label(j)} | EXCL {label(j, True)} [{j['dir']}]")
