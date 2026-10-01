"""Window 9a, group "C4-AB + Jolt 5.6": analysis library (results-analyst, 2026-09-30). Read-only on raw/, rows9a*.json,
bin/SHA256SUMS, gate/fixtures/.

Parsers: the window's own tools/lib/driver.py (load_csv, parse_summary, parse_jolt_stdout). Statistics: window 8b's
synthlib (= window 8's lib8) verbatim in arithmetic:

  cell   = median over K of per-process values; r = (max-min)/median; i = IQR (inclusive quartiles)/median;
           s = 1.2533*SD/sqrt(K)/median
  cmp_   = B against A: e = |B/A - 1|; bar_x = 2*hypot(A_x, B_x); flag x iff e > bar_x AND both K >= kmin (3)
  ruling 1: CLAIMED iff i AND s flag pooled AND in every pass, one sign throughout; STRONG iff r also flags pooled and
           in every pass.

Passes: only passes with a pass_done record (PREP.md: the analysis uses passes with a pass_done record only).
Slot rule (ruling 8, PREP.md): the original if clean and valid, else the FIRST clean valid re-run (rerun_no order),
else the slot is dropped. Clean = receipts before/after <= 5 %, witness <= 2 %, no build process during, no build/lane
presence in either receipt (win8b synthlib.clean_why, identical to the driver's contaminated flag).

Two readings of "every clean block" are carried:
  LETTER      : every closed pass is a gate; a pass-cell with K < 3 sets no flag (window 7's clause, win8b's LETTER,
                09-29 ruling 8 "the letter stands"), so any comparison with a short pass-cell is NOT CLAIMED.
  GATING-ONLY : (post hoc) the passes in which BOTH pass-cells have K >= 3 are the gates (at least one required);
                pooled over every used process, as in LETTER.
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
import driver as D  # noqa: E402  load_csv, parse_summary, parse_jolt_stdout

RUN_TAG = '191354'
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
FIX = json.load(open(os.path.join(W9A, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))
FIXBYTES = {k: open(os.path.join(W9A, 'gate', 'fixtures', k + '.pose'), 'rb').read() for k in FIX}
PASSES = (0, 1, 2)


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


def closed_passes(recs):
    return {(r['block'], r['pass'], r['pass_attempt']) for r in recs
            if r.get('pass_done') and r.get('run_tag') == RUN_TAG}


def procs_of(block, recs):
    cp = closed_passes(recs)
    return [r for r in recs if r.get('block') == block and 'row' in r and r.get('attempt') != 'warmup'
            and r.get('timed', True) and (r['block'], r['pass'], r['pass_attempt']) in cp]


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
        rc = r.get(k) or {}
        if rc.get('build_procs_busy'):
            why.append(f'{k} build busy')
        pr = rc.get('presence') or {}
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


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


WINS = {'0..500': (0, 500), '0..100': (0, 100), '100..500': (100, 500)}


def load_process(r):
    """Parse the process's own files; an independent validity check (not the driver's flag) and the payload:
    p['v'][window] = the process mean wall per step in ms (runner: run.csv wall_ns; Jolt: per_frame csv Time (ms))."""
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
         'seq': r['seq'], 'attempt': r['attempt'], 'rerun_no': r.get('rerun_no', 0), 'block': r['block'], 'v': {}}
    if r['kind'] == 'runner':
        s = D.parse_summary(read(os.path.join(d, 'stdout.txt')))
        c = D.load_csv(os.path.join(d, 'run.csv'))
        if s is None or c is None or 'wall_ns' not in c:
            why.append('no SUMMARY/csv')
        else:
            if len(c['wall_ns']) != row['steps'] or any(x is None for x in c['wall_ns']):
                why.append('csv steps')
            if s.get('void_steps') != 0:
                why.append('void_steps')
            if s.get('workers') != r['W']:
                why.append('workers')
            if s.get('debug_assertions') is not False or s.get('target_env') != 'msvc':
                why.append('build receipt')
            cfg = s.get('config') or {}
            if cfg.get('broadphase') != row.get('broadphase'):
                why.append(f"broadphase {cfg.get('broadphase')} != {row.get('broadphase')}")
            ec = BINS[r['binary']].get('expect_config') or {}
            for k, v in ec.items():
                if cfg.get(k) != v:
                    why.append(f'config {k} {cfg.get(k)} != {v}')
            if cfg.get('sleeping') is not False:
                why.append('sleeping not off')
            if row.get('canary'):
                if s.get('canary_ns') != 60000:
                    why.append(f"canary_ns {s.get('canary_ns')}")
            elif s.get('canary_ns') is not None:
                why.append('unexpected canary')
            if bool(s.get('armed')) != bool(row.get('armed')):
                why.append('armed flag')
            ref = row.get('pose_ref')
            if ref:
                if s.get('expect_pose') != 'match':
                    why.append('expect_pose')
                if s.get('pose_hash') != FIX[ref]['hash']:
                    why.append(f"pose_hash {s.get('pose_hash')}")
                if read(os.path.join(d, 'pose.bin')) != FIXBYTES[ref]:
                    why.append('pose bytes != fixture')
            tde = row.get('tree_diag_expect')
            if tde:
                td = s.get('broadphase_tree') or {}
                for k, v in tde.items():
                    if td.get(k) != v:
                        why.append(f'tree_diag {k} {td.get(k)} != {v}')
            wm = sum(c['wall_ns'][0:500]) / 500.0
            if abs(wm - s['window_mean_ns']) > 1.0:
                why.append('window mean')
            for wn, (a, b) in WINS.items():
                p['v'][wn] = mean(c['wall_ns'][a:b]) / 1e6
        p['s'], p['c'] = s, c
    elif r['kind'] == 'jolt':
        so = read(os.path.join(d, 'stdout.txt'))
        j = D.parse_jolt_stdout(so)
        st = j.get('stat_lines') or []
        if len(st) != 1:
            why.append(f'{len(st)} stat lines')
        else:
            if st[0]['threads'] != r['W']:
                why.append('threads')
            if st[0]['hash'] != row['jolt_hash']:
                why.append(f"hash {st[0]['hash']}")
        pl = j.get('patch_line') or ''
        if not pl.startswith('boyko-parity-patch v1') or 'receipt=0' not in pl or 'allow_sleep=0' not in pl:
            why.append('banner')
        pf = [f for f in sorted(os.listdir(d)) if f.startswith('per_frame_')]
        c = D.load_csv(os.path.join(d, pf[0])) if pf else None
        if not c or 'Time (ms)' not in c or len(c['Time (ms)']) != 500 or any(x is None for x in c['Time (ms)']):
            why.append('per_frame csv')
        else:
            for wn, (a, b) in WINS.items():
                p['v'][wn] = mean(c['Time (ms)'][a:b])
            if abs(p['v']['0..500'] - r.get('mean_ms', -1)) > 1e-6:
                why.append('mean vs driver')
        p['j'], p['c'] = j, c
    p['valid_why'] = why
    p['clean_why'] = clean_why(r)
    return p


def select(block, recs):
    """Slot = (block, pass, seq). Returns (procs, used, dropped)."""
    procs = [load_process(r) for r in procs_of(block, recs)]
    slots = {}
    for p in procs:
        slots.setdefault((p['block'], p['pass'], p['seq']), []).append(p)
    used, dropped = [], []
    for k, ps in sorted(slots.items()):
        orig = [p for p in ps if p['attempt'] == 'original']
        rer = sorted([p for p in ps if p['attempt'] == 'rerun'], key=lambda p: p['rerun_no'])
        pick = next((c for c in orig + rer if not c['valid_why'] and not c['clean_why']), None)
        if pick:
            pick['used'] = True
            used.append(pick)
        else:
            dropped.append((k, ps))
    return procs, used, dropped


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
        return {'K': 0, 'median': None}
    m = statistics.median(xs)
    dd = abs(m) if m else float('inf')
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / dd, 'i': iqr(xs) / dd,
            's': se_med(xs) / dd, 'iqr_abs': iqr(xs), 'se_abs': se_med(xs)}


def cmp_(a, b, kmin=3, scale=1.0):
    """B against A. scale multiplies B/A (a per-unit ratio with constant per-process work counts: the relative
    spreads are unchanged, e = |scale*B/A - 1|). minmax_sep: every scaled B process on one side of every A process."""
    if a['K'] == 0 or b['K'] == 0:
        return {'A': a['median'], 'B': b['median'], 'delta': None, 'ratio': None, 'bar_r': None, 'bar_i': None,
                'bar_s': None, 'cl_r': False, 'cl_i': False, 'cl_s': False, 'KA': a['K'], 'KB': b['K'],
                'minmax_sep': False}
    ratio = scale * b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= kmin and b['K'] >= kmin
    sep = (b['max'] * scale < a['min']) or (b['min'] * scale > a['max'])
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': ok and e > bars['r'],
            'cl_i': ok and e > bars['i'], 'cl_s': ok and e > bars['s'], 'KA': a['K'], 'KB': b['K'],
            'minmax_sep': sep}


def yn(c):
    return ''.join('Y' if c[k] else 'n' for k in ('cl_i', 'cl_s', 'cl_r'))


def ruling1(pooled, per):
    """pooled: cmp dict; per: list of cmp dicts (the gating passes). Returns (claimed, strong, direction)."""
    sign = 1 if pooled['ratio'] > 1 else -1
    same = all(c['ratio'] is not None and (1 if c['ratio'] > 1 else -1) == sign for c in per)
    claimed = bool(per) and pooled['cl_i'] and pooled['cl_s'] and all(c['cl_i'] and c['cl_s'] for c in per) and same
    strong = claimed and pooled['cl_r'] and all(c['cl_r'] for c in per)
    return claimed, strong, ('B>A' if sign > 0 else 'B<A')


def judge(vals_a, vals_b, reading='LETTER', kmin=3, scale=1.0):
    """vals_*: {pass: [values]}. Pooled over all passes + per pass (0, 1, 2). LETTER: every pass gates;
    GATING-ONLY: only passes where both sides have K >= kmin gate."""
    pa = cell([v for k in PASSES for v in vals_a.get(k, [])])
    pb = cell([v for k in PASSES for v in vals_b.get(k, [])])
    pooled = cmp_(pa, pb, kmin, scale)
    per_all = {k: cmp_(cell(vals_a.get(k, [])), cell(vals_b.get(k, [])), kmin, scale) for k in PASSES}
    if reading == 'LETTER':
        gates = list(PASSES)
    else:
        gates = [k for k in PASSES if per_all[k]['KA'] >= kmin and per_all[k]['KB'] >= kmin]
    cl, st, dr = ruling1(pooled, [per_all[k] for k in gates])
    return {'a': pa, 'b': pb, 'pooled': pooled, 'per': per_all, 'gates': gates, 'claimed': cl, 'strong': st,
            'dir': dr, 'reading': reading}


def label(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    return 'CLAIMED STRONG' if j['strong'] else 'CLAIMED'


def fcell(c, d=4):
    if c['K'] == 0:
        return 'n=0'
    return (f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] n={c['K']} "
            f"(IQR {100 * c['i']:.2f} %, SE {100 * c['s']:.2f} %, range {100 * c['r']:.2f} %)")


def fper(c):
    sep = ',sep' if c['minmax_sep'] else ''
    rr = '-' if c['ratio'] is None else f"{c['ratio']:.4f}"
    return f"{yn(c)}(K{c['KA']}/{c['KB']} {rr}{sep})"


def fj(j, d=4):
    p = j['pooled']
    s = (f"B/A {p['ratio']:.4f} ({100 * (p['ratio'] - 1):+.2f} %, B-A {p['delta']:+.{d}f}) pooled {yn(p)} "
         f"(bars i {100 * p['bar_i']:.2f} / s {100 * p['bar_s']:.2f} / r {100 * p['bar_r']:.2f} %); passes "
         + ' '.join(f"p{k}:{fper(c)}" for k, c in j['per'].items())
         + f" gates {j['gates']} -> {label(j)} [{j['dir']}]")
    return s
