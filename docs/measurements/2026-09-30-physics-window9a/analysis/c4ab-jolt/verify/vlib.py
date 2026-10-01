"""Independent verification library for window 9a C4-AB + Jolt 5.6 (adversarial verifier, 2026-09-30).

Written WITHOUT reading analysis/c4ab/*.py. Read-only on raw/, rows9a*.json, gate/fixtures, bin/.
Statistics = window 8 lib8 / 8b synthlib arithmetic (re-typed, not imported):
  cell   = median over K of per-process values; i = IQR (statistics.quantiles inclusive)/median;
           s = 1.2533*stdev(sample)/sqrt(K)/median; r = (max-min)/median
  cmp    = B against A: e = |B/A - 1|; flag x iff e > 2*hypot(A_x, B_x) AND both K >= kmin
  ruling 1: CLAIMED iff i AND s flag pooled AND in every pass (same sign); STRONG iff r also, pooled and every pass
  LETTER : kmin = 3 (ruling 8: a K<3 pass-cell does not gate -> sets no flag -> blocks a claim, 8b reading)
Slot rule (ruling 8 as implemented): slot = (block, pass, seq); the original if valid and clean, else the FIRST
valid clean re-run (by rerun_no), else dropped.  Clean: receipts before/after cpu_avg <= 5, witness <= 2,
no build process during, no build/lane presence in either receipt."""
import csv, hashlib, json, math, os, statistics, sys
sys.dont_write_bytecode = True
W9A = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a'
HERE = os.path.dirname(os.path.abspath(__file__))
_R = json.load(open(os.path.join(W9A, 'rows9a.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in _R['rows']}
BINS = _R['binaries']
FIX = json.load(open(os.path.join(W9A, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))
JOLT_HASH = '0xb8522b4e3fc62cfe'
JOLT_BANNER = 'boyko-parity-patch v1: damping 0 (Pyramid), no_pair_cache=0, allow_sleep=0, receipt=0'


class Out:
    def __init__(self):
        self.lines = []

    def __call__(self, s=''):
        self.lines.append(str(s))
        print(s)

    def save(self, name):
        open(os.path.join(HERE, name), 'w', encoding='utf-8').write('\n'.join(self.lines) + '\n')


def records():
    return [json.loads(l) for l in open(os.path.join(W9A, 'raw', 'runs.jsonl'), encoding='utf-8') if l.strip()]


_shacache = {}


def file_sha(p):
    if p not in _shacache:
        h = hashlib.sha256()
        with open(p, 'rb') as f:
            for ch in iter(lambda: f.read(1 << 20), b''):
                h.update(ch)
        _shacache[p] = h.hexdigest()
    return _shacache[p]


def exe_path(binkey):
    e = BINS[binkey]['exe']
    return e if os.path.isabs(e) else os.path.join(W9A, e)


_fixbytes = {}


def fixture_bytes(name):
    if name not in _fixbytes:
        _fixbytes[name] = open(os.path.join(W9A, 'gate', 'fixtures', name + '.pose'), 'rb').read()
    return _fixbytes[name]


def parse_summary(txt):
    for line in txt.splitlines():
        if line.startswith('SUMMARY '):
            try:
                return json.loads(line[8:])
            except json.JSONDecodeError:
                return None
    return None


def read_csv(p):
    if not os.path.isfile(p):
        return None
    rows = list(csv.reader(open(p, encoding='utf-8', newline='')))
    hdr = [h.strip() for h in rows[0]]
    return hdr, [[c.strip() for c in r] for r in rows[1:] if r]


def clean_why(r):
    why = []
    for k in ('receipt_before', 'receipt_after'):
        v = (r.get(k) or {}).get('cpu_avg')
        if v is None or v > 5.0:
            why.append(f'{k} {v}')
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(f'{k} presence')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append(f'witness {ob}')
    if r.get('build_proc_during'):
        why.append('build during')
    return why


def _read_text(p):
    if not os.path.isfile(p):
        return ""
    return open(p, encoding='utf-8', errors='replace').read()


def load(r):
    """Independent validity + per-process values."""
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    W = r['W']
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if r.get('hang'):
        why.append('hang')
    pin = BINS[r['binary']]['sha256_pin']
    if r.get('exe_sha256') != pin:
        why.append('record sha != pin')
    if file_sha(exe_path(r['binary'])) != pin:
        why.append('file sha != pin')
    p = dict(row=r['row'], binary=r['binary'], W=W, pass_=r['pass'], round=r['round'], seq=r['seq'],
             attempt=r['attempt'], rerun_no=r.get('rerun_no', 0), kind=r['kind'], rec=r,
             driver_valid=r.get('valid'))
    stdout = _read_text(os.path.join(d, 'stdout.txt'))
    if r['kind'] == 'runner':
        s = parse_summary(stdout)
        c = read_csv(os.path.join(d, 'run.csv'))
        if s is None or c is None:
            why.append('no summary/csv')
        else:
            hdr, rows = c
            iw, ist, im = hdr.index('wall_ns'), hdr.index('step'), hdr.index('manifolds')
            if len(rows) != 500 or [int(x[ist]) for x in rows] != list(range(500)):
                why.append('csv steps')
            wall = [float(x[iw]) for x in rows]
            if any(v <= 0 for v in wall):
                why.append('wall<=0')
            man = [float(x[im]) for x in rows]
            p['wall'] = wall
            p['m0_500'] = sum(wall[0:500]) / 500 / 1e6
            p['m100_500'] = sum(wall[100:500]) / 400 / 1e6
            p['m0_100'] = sum(wall[0:100]) / 100 / 1e6
            p['man100'] = sum(man[100:500]) / 400
            if abs(sum(wall) / 500 - s.get('window_mean_ns', -1)) > 1.0:
                why.append('window mean')
            if s.get('steps') != 500:
                why.append('summary steps')
            if s.get('void_steps') != 0:
                why.append('void_steps')
            if s.get('workers') != W:
                why.append('workers')
            if (s.get('threads') or {}).get('pool_workers') != W:
                why.append('pool_workers')
            if s.get('debug_assertions') is not False:
                why.append('debug_assertions')
            cfg = s.get('config') or {}
            for k, v in BINS[r['binary']]['expect_config'].items():
                if cfg.get(k) != v:
                    why.append(f'config {k}={cfg.get(k)}')
            if cfg.get('broadphase') != row['broadphase']:
                why.append(f"broadphase {cfg.get('broadphase')}")
            p['sleeping'] = cfg.get('sleeping')
            if s.get('expect_pose') != 'match':
                why.append('expect_pose')
            fx = row['pose_ref']
            if s.get('pose_hash') != FIX[fx]['hash']:
                why.append('pose_hash')
            pp = os.path.join(d, 'pose.bin')
            pb = open(pp, 'rb').read() if os.path.isfile(pp) else b''
            if pb != fixture_bytes(fx):
                why.append('pose bytes')
            p['pose_hash'] = s.get('pose_hash')
            if row.get('canary'):
                if s.get('canary_ns') != 60000:
                    why.append(f"canary {s.get('canary_ns')}")
            elif s.get('canary_ns') not in (None, 0):
                why.append(f"canary on non-rung {s.get('canary_ns')}")
            td = s.get('broadphase_tree') or {}
            if row.get('tree_diag_expect'):
                for k, v in row['tree_diag_expect'].items():
                    if td.get(k) != v:
                        why.append(f'treediag {k}')
            elif td.get('static_rebuilds') or td.get('members'):
                why.append('allpairs row ran tree')
            a = r['args']
            if a[:len(row['args'])] != row['args'] or a[a.index('--workers') + 1] != str(W):
                why.append('args')
            p['void'] = s.get('void_steps')
    elif r['kind'] == 'jolt':
        c = read_csv(os.path.join(d, f'per_frame_discrete_th{W}.csv'))
        if c is None:
            why.append('no per-frame csv')
        else:
            hdr, rows = c
            if len(rows) != 500 or [int(x[0]) for x in rows] != list(range(500)):
                why.append('frames')
            t = [float(x[1]) for x in rows]
            if any(v <= 0 for v in t):
                why.append('t<=0')
            p['wall'] = [v * 1e6 for v in t]
            p['m0_500'] = sum(t[0:500]) / 500
            p['m100_500'] = sum(t[100:500]) / 400
            p['m0_100'] = sum(t[0:100]) / 100
        stat = [l for l in stdout.splitlines() if l.startswith('Discrete,') or l.startswith('LinearCast,')]
        if len(stat) != 1:
            why.append(f'stat lines {len(stat)}')
        else:
            q, th, sps, hsh = [x.strip() for x in stat[0].split(',')]
            if int(th) != W:
                why.append('threads')
            if hsh != JOLT_HASH:
                why.append('hash')
            p['pose_hash'] = hsh
        if JOLT_BANNER not in [l.strip() for l in stdout.splitlines()]:
            why.append('banner')
        if f'-t={W}' not in r['args'] or '-i=500' not in r['args'] or r['args'][:3] != row['args']:
            why.append('args')
    p['valid_why'] = why
    p['clean_why'] = clean_why(r)
    p['valid'] = not why
    p['clean'] = not p['clean_why']
    return p


def select(block, recs):
    procs = [load(r) for r in recs
             if r.get('block') == block and 'row' in r and r.get('attempt') in ('original', 'rerun')]
    slots = {}
    for p in procs:
        slots.setdefault((p['pass_'], p['seq']), []).append(p)
    used, dropped = [], []
    for k in sorted(slots):
        ps = slots[k]
        orig = [p for p in ps if p['attempt'] == 'original']
        rer = sorted([p for p in ps if p['attempt'] == 'rerun'], key=lambda p: p['rerun_no'])
        assert len(orig) == 1, k
        pick = next((c for c in orig + rer if c['valid'] and c['clean']), None)
        if pick:
            pick['used'] = True
            used.append(pick)
        else:
            dropped.append((k, ps))
    return procs, used, dropped, slots


# ---------------- statistics ----------------
def se_med(xs):
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) if len(xs) >= 2 else 0.0


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(xs)
    if not xs:
        return {'K': 0, 'median': float('nan'), 'min': float('nan'), 'max': float('nan'), 'r': 0.0, 'i': 0.0, 's': 0.0}
    m = statistics.median(xs)
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / m,
            'i': iqr(xs) / m, 's': se_med(xs) / m}


def cmp_(a, b, kmin=3, scale=1.0):
    """B against A; scale multiplies B/A (per-unit re-expression). Relative spreads are scale-free."""
    if a['K'] == 0 or b['K'] == 0:
        return {'ratio': float('nan'), 'delta': float('nan'), 'cl_i': False, 'cl_s': False, 'cl_r': False,
                'KA': a['K'], 'KB': b['K'], 'bar_i': 0.0, 'bar_s': 0.0, 'bar_r': 0.0, 'sep': False}
    ok = a['K'] >= kmin and b['K'] >= kmin
    ratio = b['median'] / a['median'] * scale
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'ris'}
    sep = (b['min'] * scale > a['max']) or (b['max'] * scale < a['min'])
    return {'ratio': ratio, 'delta': b['median'] - a['median'], 'bar_i': bars['i'], 'bar_s': bars['s'],
            'bar_r': bars['r'], 'cl_i': ok and e > bars['i'], 'cl_s': ok and e > bars['s'],
            'cl_r': ok and e > bars['r'], 'KA': a['K'], 'KB': b['K'], 'sep': sep}


def yn(c):
    return ''.join('Y' if c[k] else 'n' for k in ('cl_i', 'cl_s', 'cl_r'))


def judge(va, vb, kmin=3, scale=1.0, passes=(0, 1, 2)):
    pa = cell([x for k in passes for x in va.get(k, [])])
    pb = cell([x for k in passes for x in vb.get(k, [])])
    pooled = cmp_(pa, pb, kmin, scale)
    per = [cmp_(cell(va.get(k, [])), cell(vb.get(k, [])), kmin, scale) for k in passes]
    sign = 1 if pooled['ratio'] > 1 else -1
    same = all((1 if c['ratio'] > 1 else -1) == sign for c in per if not math.isnan(c['ratio']))
    claimed = pooled['cl_i'] and pooled['cl_s'] and all(c['cl_i'] and c['cl_s'] for c in per) and same
    strong = claimed and pooled['cl_r'] and all(c['cl_r'] for c in per)
    return {'a': pa, 'b': pb, 'pooled': pooled, 'per': per, 'claimed': claimed, 'strong': strong,
            'dir': 'B>A' if sign > 0 else 'B<A'}


def label(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    return 'CLAIMED STRONG' if j['strong'] else 'CLAIMED'


def fj(j):
    p = j['pooled']
    return (f"B/A {p['ratio']:.4f} ({100 * (p['ratio'] - 1):+.2f} %, B-A {p['delta']:+.4f}) pooled {yn(p)}"
            f" (bars i {100 * p['bar_i']:.2f} s {100 * p['bar_s']:.2f} r {100 * p['bar_r']:.2f}) passes "
            + ' '.join(f"p{k}:{yn(c)}{'(sep)' if c['sep'] else ''}(K{c['KA']}/{c['KB']})" for k, c in enumerate(j['per']))
            + f" -> {label(j)} [{j['dir']}]")


def fc(c, d=4):
    return (f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] n={c['K']} "
            f"(i {100 * c['i']:.2f} s {100 * c['s']:.2f} r {100 * c['r']:.2f} %)")
