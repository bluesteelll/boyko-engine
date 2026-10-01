"""Window 8b, group J-Son-T + DM1: selection, independent validity re-check from the files, per-process statistics,
cells and the ruling-1 claim test. Statistics definitions copied verbatim from window 8's analysis/lib8.py
(cell = median over K of per-process means; r = min-max, i = IQR (inclusive quartiles), s = 1.2533*SD/sqrt(K), each
relative to the median; a comparison clears bar x iff |B/A-1| > 2*hypot(A.x, B.x) and both K >= 3).
Read-only on raw/. Imports nothing from the driver."""
import csv
import json
import math
import os
import re
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(os.path.dirname(HERE))
RAW = os.path.join(W8B, 'raw')
RJ = json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in RJ['rows']}
FIX = json.load(open(os.path.join(W8B, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))
SUMS = {}
for line in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS[name.strip().lstrip('*')] = sha
KEY2EXE = {'tip': 'bin/runner_tip_16191fda.exe',
           'dmA': RJ['binaries']['dmA']['exe'], 'dmB': RJ['binaries']['dmB']['exe']}


def load_recs(block):
    out = []
    for line in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8'):
        if not line.strip():
            continue
        r = json.loads(line)
        if r.get('pass_done') or r.get('r4') or 'row' not in r:
            continue
        if r.get('block') == block and r.get('attempt') != 'warmup':
            out.append(r)
    return out


def read_text(p):
    try:
        return open(p, encoding='utf-8', errors='replace').read()
    except FileNotFoundError:
        return ''


def summary_of(txt):
    for line in txt.splitlines():
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
                if v == '':
                    cols[h].append(None)
                else:
                    try:
                        cols[h].append(float(v))
                    except ValueError:
                        cols[h].append(None)
    return cols


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def clean(r):
    """window 8 clean rule: both receipts <= 5 %, witness <= 2 %, no build process, no build/lane presence."""
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


def sha_ok(r):
    return r.get('exe_sha256') == SUMS.get(KEY2EXE[r['binary']])


def validate_runner(r):
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    s = summary_of(read_text(os.path.join(d, 'stdout.txt')))
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if not sha_ok(r):
        why.append('sha')
    if s is None:
        return why + ['no SUMMARY'], None, None
    c = load_csv(os.path.join(d, 'run.csv'))
    n = len(c['wall_ns'])
    if n != row['steps'] or any(x is None for x in c['wall_ns']):
        why.append(f'csv steps {n}')
    if c['step'][:3] != [0.0, 1.0, 2.0]:
        why.append('step column not 0-based')
    if s.get('void_steps') != 0:
        why.append(f"void {s.get('void_steps')}")
    fx = FIX[row['pose_ref']]['hash']
    if s.get('pose_hash') != fx or s.get('expect_pose') != 'match':
        why.append(f"pose {s.get('pose_hash')} {s.get('expect_pose')}")
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if bool(s.get('armed')) != bool(row['armed']):
        why.append('armed flag')
    if row['armed']:
        if s.get('drops_total') != 0:
            why.append(f"drops {s.get('drops_total')}")
        w8 = s.get('w8s')
        if not w8:
            why.append('no w8s')
        elif w8.get('overflow') not in (0, None):
            why.append(f"overflow {w8.get('overflow')}")
    else:
        if s.get('disarmed_ring_traffic') != 0:
            why.append(f"ring {s.get('disarmed_ring_traffic')}")
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != row['broadphase']:
        why.append('bp kind')
    bt = s.get('broadphase_tree') or {}
    for k, v in (row.get('tree_diag_expect') or {}).items():
        if bt.get(k) != v:
            why.append(f'treediag {k} {bt.get(k)} != {v}')
    if (bt.get('kd_order_builds') or 0) != 0:
        why.append('kd builds on the default kernel')
    a = row['args']
    sl = a[a.index('--sleeping') + 1] == 'on'
    if bool(cfg.get('sleeping')) != sl:
        why.append('sleeping flag')
    if sl:
        if s.get('first_frozen_step') != row.get('expect_first_frozen'):
            why.append(f"first_frozen_step {s.get('first_frozen_step')}")
        aw = c['awake']
        fz = next((i for i, x in enumerate(aw) if x == 0), None)
        if fz != 273 or any(x != 0 for x in aw[273:]):
            why.append(f'awake column: first zero at row {fz} / not zero after')
    else:
        if s.get('first_frozen_step') is not None:
            why.append('froze with sleeping off')
        if any(x is not None for x in c['awake']):
            why.append('awake column written with sleeping off')
    wm = mean(c['wall_ns'][0:500])
    if abs(wm - s['window_mean_ns']) > 1.0:
        why.append(f"window mean {wm} vs {s['window_mean_ns']}")
    return why, s, c


LINE = re.compile(r'DM1 timing: path=(\w+) res=(\d+)x(\d+) edit_rows=(\d+) grow_at=(\S+) frames=(\d+)')


def dm1_zones(art_txt):
    zones = {}
    for blk in re.findall(r'\[\[zone\]\](.*?)(?=\[\[zone\]\]|\Z)', art_txt, re.S):
        kv = dict(re.findall(r'(\w+) = ([^\n]+)', blk))
        zid = int(kv['id'])
        z = {}
        for k, v in kv.items():
            v = v.strip()
            try:
                z[k] = float(v)
            except ValueError:
                z[k] = v.strip('"')
        zones[zid] = z
    return zones


def validate_dm1(r):
    row = ROWS[r['row']]
    d = r['cwd']
    txt = read_text(os.path.join(d, 'stdout.txt')) + '\n' + read_text(os.path.join(d, 'stderr.txt'))
    why = []
    if r.get('exit') != 0:
        why.append('exit')
    if not sha_ok(r):
        why.append('sha')
    if 'test result: ok. 1 passed' not in txt:
        why.append('libtest')
    if 'NOT MEASURED' in txt:
        why.append('not measured')
    m = LINE.search(txt)
    if not m:
        why.append('no DM1 line')
    else:
        path = {'vb': 'VisibilityBuffer', 'deferred': 'Deferred'}[row['path']]
        bad = m.group(1) != path or f'{m.group(2)}x{m.group(3)}' != row['res']
        bad = bad or int(m.group(4)) != row['edit_rows'] or m.group(5) != 'None'
        if bad:
            why.append('DM1 line mismatch')
    art = read_text(os.path.join(d, 'artifact.toml'))
    zones = dm1_zones(art)
    for name, z in row['need_zones'].items():
        if z not in zones or int(zones[z]['n']) != 220:
            why.append(f'zone {name} {z} n')
    pm = re.search(r'^present_mode = "(\w+)"', art, re.M)
    pm = pm.group(1) if pm else None
    if pm != 'fifo':
        why.append(f'present {pm}')
    return why, zones, (m.group(0) if m else None), pm


def select(recs, validator, dm1=False):
    """The slot rule (window 7/8): original if valid and clean, else its re-run if valid and clean, else the slot is
    dropped. Slot = (pass, round, row, binary, W, seq); seq keeps DM1's repeated ABBA positions apart."""
    procs = []
    for r in recs:
        p = dict(r)
        res = validator(r)
        p['valid_why'] = res[0]
        if dm1:
            p['_zones'], p['_line'], p['_pm'] = res[1], res[2], res[3]
        else:
            p['_s'], p['_c'] = res[1], res[2]
        p['clean_why'] = clean(r)
        procs.append(p)
    slots = {}
    for p in procs:
        k = (p['pass'], p['round'], p['row'], p['binary'], p['W'], p['seq'])
        slots.setdefault(k, []).append(p)
    used, dropped = [], []
    for k, ps in sorted(slots.items(), key=lambda kv: (kv[0][0], kv[0][5])):
        orig = [p for p in ps if p['attempt'] == 'original']
        rer = [p for p in ps if p['attempt'] == 'rerun']
        pick = None
        for cand in orig + rer:
            if not cand['valid_why'] and not cand['clean_why']:
                pick = cand
                break
        if pick:
            pick['used'] = True
            pick['slot_n'] = len(ps)
            used.append(pick)
        else:
            dropped.append((k, [(p['attempt'], p['valid_why'], p['clean_why']) for p in ps]))
    return procs, used, dropped


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
            's': se_med(xs) / dd, 'iqr_abs': iqr(xs), 'se_abs': se_med(xs), 'values': xs}


def cmp_(a, b, min_k=3):
    """B against A (window 7/8 rule)."""
    if not a or not b or not a['median']:
        return None
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= min_k and b['K'] >= min_k
    cl = {k: ok and e > bars[k] for k in bars}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'], 'cl_i': cl['i'],
            'cl_s': cl['s'], 'KA': a['K'], 'KB': b['K'], 'k_ok': ok}


def yn(c):
    return ('Y' if c['cl_r'] else 'n') + '/' + ('Y' if c['cl_i'] else 'n') + '/' + ('Y' if c['cl_s'] else 'n')


def ruling1(pooled, per_pass):
    """Ruling 1: CLAIMED iff i AND s clear in every clean pass and pooled, one sign throughout; STRONG iff r also
    clears everywhere. The caller maps the sign to CLAIMED / REFUTED against the pre-registered direction."""
    allc = [pooled] + list(per_pass)
    if any(c is None for c in allc):
        return 'NOT CLAIMED (missing cell)', None
    sign = [1 if c['ratio'] > 1 else -1 for c in allc]
    is_ok = all(c['cl_i'] and c['cl_s'] for c in allc) and len(set(sign)) == 1
    strong = is_ok and all(c['cl_r'] for c in allc)
    if is_ok:
        return ('CLAIMED STRONG' if strong else 'CLAIMED'), sign[0]
    return 'NOT CLAIMED', None


def fcell(c, d=4, scale=1.0):
    if not c:
        return 'n/a'
    return (f"{c['median'] * scale:.{d}f} [{c['min'] * scale:.{d}f}-{c['max'] * scale:.{d}f}] K={c['K']} "
            f"(IQR {c['iqr_abs'] * scale:.{d}f} = {100 * c['i']:.2f} %, SE {c['se_abs'] * scale:.{d}f} = "
            f"{100 * c['s']:.2f} %, range {100 * c['r']:.2f} %)")


def fcmp(c):
    if c is None:
        return 'n/a'
    tail = '' if c['k_ok'] else f" [K {c['KA']}/{c['KB']} < 3: no claim possible]"
    return (f"{c['ratio']:.4f} ({100 * (c['ratio'] - 1):+.2f} %, d {c['delta']:+.4f}) r/i/s {yn(c)} "
            f"bars r {100 * c['bar_r']:.2f} / i {100 * c['bar_i']:.2f} / s {100 * c['bar_s']:.2f} %" + tail)
