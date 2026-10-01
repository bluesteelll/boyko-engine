"""Independent re-derivation of the Rapier group of window 9a (verifier, 2026-09-30).
Written BEFORE reading the analyst's lib_rp.py / r*_*.py. Reads raw/runs.jsonl, the process dirs, rows9a*.json,
gate/fixtures, gate/rapier/pins.json, bin/. Imports nothing from the analyst or from 8b's synthlib (the arithmetic
of 8b's lib is re-implemented here from its docstring: cell = median over K of per-process means; i = IQR
(inclusive quartiles)/median; s = 1.2533*SD/sqrt(K)/median; r = (max-min)/median; flag x iff |B/A-1| > 2*hypot)."""
import csv, hashlib, json, math, os, statistics, sys
from collections import defaultdict, Counter

HERE = os.path.dirname(os.path.abspath(__file__))
W9 = os.path.abspath(os.path.join(HERE, '..', '..', '..'))
RAW = os.path.join(W9, 'raw')
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a)
    OUT.append(s)
    print(s)

R = json.load(open(os.path.join(W9, 'rows9a.json'), encoding='utf-8'))
X = json.load(open(os.path.join(W9, 'rows9a.extra.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in R['rows'] + X['rows']}
BINS = dict(R['binaries'])
BINS.update(X['binaries'])
PINS = json.load(open(os.path.join(W9, 'gate', 'rapier', 'pins.json'), encoding='utf-8'))
FIX = {n: open(os.path.join(W9, 'gate', 'fixtures', n + '.pose'), 'rb').read() for n in ('JT500', 'RD8', 'RD4', 'RM8', 'RM4')}

def sha_file(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for ch in iter(lambda: f.read(1 << 20), b''):
            h.update(ch)
    return h.hexdigest()

SHA_DISK = {}
for k in ('tip', 'parent', 'j56', 'rs8', 'rs4'):
    exe = BINS[k]['exe']
    p = exe if os.path.isabs(exe) else os.path.join(W9, exe)
    SHA_DISK[k] = sha_file(p)
    P('exe', k, 'disk sha == pin:', SHA_DISK[k] == BINS[k]['sha256_pin'])

recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]
closed = {(r['run_tag'], r['block'], r['pass'], r['pass_attempt']) for r in recs if r.get('pass_done')}
P('closed passes:', sorted((c[1], c[2]) for c in closed))
procs = [r for r in recs if 'row' in r and r.get('timed') and r.get('attempt') in ('original', 'rerun')]

def summary_lines(txt):
    return [l for l in txt.splitlines() if l.startswith('SUMMARY')]

def load_rows_csv(path):
    with open(path, newline='') as f:
        rd = csv.reader(f)
        hdr = [h.strip() for h in next(rd)]
        rows = [r for r in rd if r]
    return hdr, rows

def means(vals):
    return (statistics.fmean(vals[0:100]) / 1e6, statistics.fmean(vals[100:500]) / 1e6)

def check_rapier(r, s, hdr, rows, pose, W, why):
    ref = ROWS[r['row']]['pose_ref'][r['binary']]
    if pose != FIX[ref] or len(pose) == 0:
        why.append('pose bytes')
    if s.get('expect_pose') != 'match':
        why.append('expect_pose')
    if hdr != ['step', 'wall_ns', 'pool_threads']:
        why.append('csv header')
    ip = hdr.index('pool_threads')
    if any(int(x[ip]) != W for x in rows):
        why.append('csv pool_threads')
    t = s.get('threads', {})
    ok3 = (t.get('pool_threads_min') == W and t.get('pool_threads_max') == W and t.get('install_receipt') == [W, W]
           and t.get('loop_on_pool_worker') is True and t.get('pool_threads_requested') == W)
    if not ok3:
        why.append('V3 threads')
    if s.get('workers') != W:
        why.append('workers')
    arm = BINS[r['binary']]['arm']
    fe = s.get('features', {})
    ok4 = (s.get('arm') == arm and fe.get('simd8') is (arm == 'simd8') and fe.get('simd4') is (arm == 'simd4')
           and fe.get('parallel') is True and fe.get('enhanced_determinism') is False
           and s.get('rapier_version') == '0.36.0' and s.get('debug_assertions') is False)
    if not ok4:
        why.append('V4 build')
    if (s.get('config') or {}).get('counters_enabled') is not False:
        why.append('counters')
    if '--receipt' in r['args'] or '--receipt' in (s.get('args') or []):
        why.append('--receipt')
    if s.get('void') is not False or s.get('voids'):
        why.append('void')
    if s.get('perturb') is not None:
        why.append('perturb')
    si = s.get('scene_identity')
    if isinstance(si, dict):
        bad = [k for k, v in si.items() if isinstance(v, bool) and v is not True]
        if bad:
            why.append('scene_identity ' + ','.join(bad))

def check(r):
    """(why_invalid list, values (w0, w1) in ms)."""
    why = []
    d = r['cwd']
    kind = r['kind']
    W = r['W']
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    if r.get('hang'):
        why.append('hang')
    pin = BINS[r['binary']]['sha256_pin']
    if r.get('exe_sha256') != pin or SHA_DISK[r['binary']] != pin:
        why.append('sha')
    vals = None
    if kind in ('runner', 'rapier'):
        txt = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
        sl = summary_lines(txt)
        if len(sl) != 1:
            why.append('SUMMARY x%d' % len(sl))
            return why, None
        s = json.loads(sl[0][len('SUMMARY'):].strip())
        hdr, rows = load_rows_csv(os.path.join(d, 'run.csv'))
        iw = hdr.index('wall_ns')
        ist = hdr.index('step')
        wall = [float(x[iw]) for x in rows]
        if len(rows) != 500 or [int(x[ist]) for x in rows] != list(range(500)) or any(v <= 0 for v in wall):
            why.append('csv shape')
        pose = open(os.path.join(d, 'pose.bin'), 'rb').read()
        if kind == 'runner':
            if s.get('workers') != W:
                why.append('workers')
            if s.get('void_steps') != 0:
                why.append('void_steps')
            if s.get('expect_pose') != 'match':
                why.append('expect_pose')
            if pose != FIX['JT500']:
                why.append('pose bytes')
            if abs(statistics.fmean(wall) - s['window_mean_ns']) > 1.0:
                why.append('window mean')
        else:
            check_rapier(r, s, hdr, rows, pose, W, why)
        vals = means(wall)
    elif kind == 'jolt':
        txt = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
        stat = [l for l in txt.splitlines() if l.startswith('Discrete,')]
        if len(stat) != 1:
            why.append('stat lines')
        else:
            f = [x.strip() for x in stat[0].split(',')]
            if int(f[1]) != W or f[3] != ROWS['C4-jolt56']['jolt_hash']:
                why.append('jolt threads/hash')
        if 'allow_sleep=0' not in txt or 'receipt=0' not in txt:
            why.append('banner')
        hdr, rows = load_rows_csv(os.path.join(d, 'per_frame_discrete_th%d.csv' % W))
        ms = [float(x[1]) for x in rows]
        if len(ms) != 500 or [int(x[0]) for x in rows] != list(range(500)):
            why.append('jolt csv')
        vals = (statistics.fmean(ms[0:100]), statistics.fmean(ms[100:500]))
    return why, vals

def unclean(r):
    w = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    if rb is None or rb > 5.0:
        w.append('before')
    if ra is None or ra > 5.0:
        w.append('after')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        w.append('witness')
    if r.get('build_proc_during'):
        w.append('build')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            w.append('presence')
    return w

WANT = {'C4-RAPIER': None, 'C4-AB': {'C4-JT', 'C4-JD', 'C4-jolt56'}}
sel = [r for r in procs if r['block'] in WANT and (WANT[r['block']] is None or r['row'] in WANT[r['block']])]
P('')
P('== processes read:', len(sel))
info = {}
dis_valid = 0
dis_clean = 0
for r in sel:
    why, vals = check(r)
    uc = unclean(r)
    info[id(r)] = (why, uc, vals)
    if (not why) != bool(r.get('valid')):
        dis_valid += 1
    if bool(uc) != bool(r.get('contaminated')):
        dis_clean += 1
    if vals is not None and r.get('cols') and r['kind'] != 'jolt':
        c = r['cols']
        if abs(c['100..500']['wall_ns']['mean'] / 1e6 - vals[1]) > 1e-6 or abs(c['0..100']['wall_ns']['mean'] / 1e6 - vals[0]) > 1e-6:
            P('  cols mismatch', r['row'], r['binary'], r['W'], r['pass'], r['seq'])
for blk in ('C4-RAPIER', 'C4-AB'):
    rs = [r for r in sel if r['block'] == blk]
    orig = sum(r['attempt'] == 'original' for r in rs)
    inval = [r for r in rs if info[id(r)][0]]
    unc = [r for r in rs if info[id(r)][1]]
    P('%s: n %d (orig %d, rerun %d); invalid %d; unclean %d' % (blk, len(rs), orig, len(rs) - orig, len(inval), len(unc)))
    for p in (0, 1, 2):
        rp = [r for r in rs if r['pass'] == p]
        tops = Counter()
        for r in rp:
            if 'witness' in info[id(r)][1]:
                tops[(r.get('others_top5') or [{}])[0].get('name')] += 1
        P('  p%d: %d of %d unclean; witness top-other %s' % (p, sum(1 for r in rp if info[id(r)][1]), len(rp), dict(tops)))
    cause = Counter(c for r in unc for c in set(info[id(r)][1]))
    P('  unclean causes (a process may carry >1):', dict(cause))
    for r in inval[:10]:
        P('  INVALID', r['row'], r['binary'], r['W'], r['pass'], info[id(r)][0])
P('driver valid-flag disagreements:', dis_valid, '; driver contaminated-flag disagreements:', dis_clean)

slots = defaultdict(list)
for r in sel:
    if (r['run_tag'], r['block'], r['pass'], r['pass_attempt']) not in closed:
        continue
    slots[(r['run_tag'], r['block'], r['pass'], r['pass_attempt'], r['seq'])].append(r)
used = []
dropped = []
for k, rs in slots.items():
    rs.sort(key=lambda r: (r['attempt'] != 'original', r.get('rerun_no') or 0))
    pick = next((r for r in rs if not info[id(r)][0] and not info[id(r)][1]), None)
    if pick is None:
        dropped.append((k, rs[0]))
    else:
        used.append(pick)
P('slots', len(slots), 'used', len(used), 'dropped', len(dropped), dict(Counter((k[1], k[2]) for k, _ in dropped)))
P('  dropped slots:', sorted('%s p%d %s#%s@W%d' % (k[1], k[2], r0['row'], r0['binary'], r0['W']) for k, r0 in dropped))
for blk in ('C4-RAPIER', 'C4-AB'):
    u = [r for r in used if r['block'] == blk]
    rb = statistics.median((r['receipt_before'] or {}).get('cpu_avg') for r in u)
    ra = statistics.median((r['receipt_after'] or {}).get('cpu_avg') for r in u)
    wi = statistics.median(r['others_busy_pct'] for r in u)
    P('  used receipts (medians) %s: before %.2f after %.2f witness %.2f; %s .. %s' % (blk, rb, ra, wi, min(r['start'] for r in u)[11:16], max(r['end'] for r in u)[11:16]))

pcs = {(r['block'], r['pass'], r['pc_row'], r['pc_binary'], r['pc_W']): r['k_clean'] for r in recs if r.get('passcell')}
cells = defaultdict(lambda: defaultdict(list))
for r in used:
    cells[(r['block'], r['row'], r['binary'], r['W'])][r['pass']].append(info[id(r)][2])
mism = 0
for (b, row, bi, W), per in list(cells.items()):
    for p in (0, 1, 2):
        if pcs.get((b, p, row, bi, W)) != len(per.get(p, [])):
            mism += 1
            P('  K mismatch', b, row, bi, W, p, pcs.get((b, p, row, bi, W)), len(per.get(p, [])))
P('pass-cell K vs driver k_clean mismatches:', mism)
KEYS = [('C4-AB', 'C4-JT', 'tip', w) for w in (1, 8, 16)] + [('C4-AB', 'C4-JD', 'tip', w) for w in (1, 2, 4, 8, 16)]
KEYS += [('C4-AB', 'C4-JT', 'parent', w) for w in (1, 8, 16)] + [('C4-AB', 'C4-jolt56', 'j56', w) for w in (1, 2, 4, 8, 16)]
for key in KEYS:
    P('  K', key[1:], [len(cells[key].get(p, [])) for p in (0, 1, 2)])
rk = Counter(tuple(len(cells[k].get(p, [])) for p in (0, 1, 2)) for k in cells if k[0] == 'C4-RAPIER')
P('  Rapier cells K pattern:', dict(rk))

def cell(xs):
    xs = sorted(xs)
    K = len(xs)
    if K == 0:
        return None
    m = statistics.median(xs)
    if K >= 2:
        q = statistics.quantiles(xs, n=4, method='inclusive')
        iq = q[2] - q[0]
        sd = statistics.stdev(xs)
    else:
        iq = 0.0
        sd = 0.0
    return dict(K=K, m=m, mn=xs[0], mx=xs[-1], i=iq / m, s=1.2533 * sd / math.sqrt(K) / m, r=(xs[-1] - xs[0]) / m)

def cmpc(a, b, kmin):
    ratio = b['m'] / a['m']
    e = abs(ratio - 1)
    ok = a['K'] >= kmin and b['K'] >= kmin
    fl = {x: ok and e > 2 * math.hypot(a[x], b[x]) for x in 'isr'}
    return dict(ratio=ratio, ok=ok, KA=a['K'], KB=b['K'], **fl)

def yn(c):
    return ''.join('Y' if c[x] else 'n' for x in 'isr')

def judge(pa, pb, mode):
    A = cell([v for p in sorted(pa) for v in pa[p]])
    B = cell([v for p in sorted(pb) for v in pb[p]])
    pooled = cmpc(A, B, 3)
    per = {p: cmpc(cell(pa[p]), cell(pb[p]), 3) for p in (0, 1, 2)}
    gating = per if mode == 'LETTER' else {p: c for p, c in per.items() if c['ok']}
    sign = pooled['ratio'] > 1
    same = all((c['ratio'] > 1) == sign for c in gating.values())
    claimed = bool(gating) and pooled['i'] and pooled['s'] and all(c['i'] and c['s'] for c in gating.values()) and same
    strong = claimed and pooled['r'] and all(c['r'] for c in gating.values())
    return dict(A=A, B=B, pooled=pooled, per=per, claimed=claimed, strong=strong, dir=('B>A' if sign else 'B<A'))

def lab(j):
    if not j['claimed']:
        return 'NOT CLAIMED'
    return 'CLAIMED STRONG' if j['strong'] else 'CLAIMED'

def fl(j):
    return yn(j['pooled']) + '; ' + ' '.join('p%d%s(K%d/%d)' % (p, yn(c), c['KA'], c['KB']) for p, c in j['per'].items())

def series(block, row, bi, W, win, scale=1.0):
    per = cells[(block, row, bi, W)]
    return {p: [v[win] / scale for v in per.get(p, [])] for p in (0, 1, 2)}

WS = (1, 2, 4, 8, 16)
WIN = {0: '[0,100)', 1: '[100,500)'}
RESULT = {}
P('')
P('== R-ARM (A = rs8, B = rs4; B/A > 1 = rs8 faster)')
for win in (1, 0):
    for cfg in ('RP-D', 'RP-M'):
        for W in WS:
            j = judge(series('C4-RAPIER', cfg, 'rs8', W, win), series('C4-RAPIER', cfg, 'rs4', W, win), 'LETTER')
            RESULT['R-ARM %s W%d %s' % (cfg, W, WIN[win])] = lab(j) + ' ' + j['dir']
            P('%s %s W%d: rs8 %.4f [%.4f-%.4f] (K%d) rs4 %.4f [%.4f-%.4f] (K%d) B/A %.3f %s -> %s %s' % (
                WIN[win], cfg, W, j['A']['m'], j['A']['mn'], j['A']['mx'], j['A']['K'], j['B']['m'], j['B']['mn'], j['B']['mx'], j['B']['K'],
                j['pooled']['ratio'], fl(j), lab(j), j['dir']))

P('')
P('== R-ARM, reversed orientation (A = rs4, B = rs8): verdict changes only')
for win in (1, 0):
    for cfg in ('RP-D', 'RP-M'):
        for W in WS:
            j = judge(series('C4-RAPIER', cfg, 'rs4', W, win), series('C4-RAPIER', cfg, 'rs8', W, win), 'LETTER')
            rev = lab(j) + ' ' + ('B>A' if j['dir'] == 'B<A' else 'B<A')
            fwd = RESULT['R-ARM %s W%d %s' % (cfg, W, WIN[win])]
            P('  %s %s W%d: rs8-first %s | rs4-first %s (%s)%s' % (WIN[win], cfg, W, fwd, rev, fl(j), '  <-- DIFFERS' if (fwd.startswith('NOT')) != (rev.startswith('NOT')) or ('STRONG' in fwd) != ('STRONG' in rev) else ''))

arm = [r for r in procs if r['block'] == 'C4-G5' and r['row'] == 'C4-JD-armed' and r['binary'] == 'tip']
nps = []
for r in arm:
    hdr, rows = load_rows_csv(os.path.join(r['cwd'], 'run.csv'))
    ip = hdr.index('phys_np_points')
    v = [float(x[ip]) for x in rows]
    nps.append((statistics.fmean(v[0:100]), statistics.fmean(v[100:500])))
P('')
P('rows census: %d JD-armed#tip processes, W %s; distinct Np[0,100) %s; distinct Np[100,500) %s' % (
    len(arm), sorted(Counter(r['W'] for r in arm).items()), sorted(set(round(a, 4) for a, _ in nps)), sorted(set(round(b, 4) for _, b in nps))))
OURS_ROWS = {0: 36 * nps[0][0], 1: 36 * nps[0][1]}
P('ours rows = 36*Np: [0,100) %.2f  [100,500) %.2f' % (OURS_ROWS[0], OURS_ROWS[1]))
RP_ROWS = {('RP-D', 0): 303003, ('RP-D', 1): 354674, ('RP-M', 0): 935729, ('RP-M', 1): 1071107}
for (c, w), v in RP_ROWS.items():
    key = '0..100' if w == 0 else '100..500'
    assert ROWS[c]['rows_pin']['rs8'][key] == v and ROWS[c]['rows_pin']['rs4'][key] == v
P('row ratio RP-M/ours [100,500) %.4f, [0,100) %.4f; RP-D/ours [100,500) %.4f' % (
    RP_ROWS[('RP-M', 1)] / OURS_ROWS[1], RP_ROWS[('RP-M', 0)] / OURS_ROWS[0], RP_ROWS[('RP-D', 1)] / OURS_ROWS[1]))

OURS = {1: ('C4-JT', 'tip'), 2: ('C4-JD', 'tip'), 4: ('C4-JD', 'tip'), 8: ('C4-JT', 'tip'), 16: ('C4-JT', 'tip')}

def claim(cfg, W, win, mode, unit, ours=None, flip=False):
    orow, obin = ours or OURS[W]
    so = 1.0 if unit == 'wall' else OURS_ROWS[win] / 1e6
    a = series('C4-AB', orow, obin, W, win, so)
    js = {}
    for b in ('rs8', 'rs4'):
        sr = 1.0 if unit == 'wall' else RP_ROWS[(cfg, win)] / 1e6
        rb = series('C4-RAPIER', cfg, b, W, win, sr)
        js[b] = judge(rb, a, mode) if flip else judge(a, rb, mode)
    ours_dir = 'B<A' if flip else 'B>A'
    ours_faster = all(j['claimed'] and j['dir'] == ours_dir for j in js.values())
    rap = [b for b, j in js.items() if j['claimed'] and j['dir'] != ours_dir]
    if ours_faster:
        v = 'ours faster' + (' STRONG' if all(j['strong'] for j in js.values()) else '')
    elif rap:
        v = 'Rapier faster (' + ','.join(rap) + ')' + (' STRONG' if any(js[b]['strong'] for b in rap) else '')
    else:
        v = 'NOT CLAIMED'
    return js, v

CLAIMS = (('R-WALL-D', 'RP-D', 'wall'), ('R-WALL-M', 'RP-M', 'wall'), ('R-ROW-D', 'RP-D', 'row'), ('R-ROW-M', 'RP-M', 'row'))
for win in (1, 0):
    P('')
    P('== R-WALL / R-ROW %s (A = ours, B = Rapier; ratio = Rapier/ours; >1 = ours faster/cheaper)' % WIN[win])
    for name, cfg, unit in CLAIMS:
        for W in WS:
            js, vL = claim(cfg, W, win, 'LETTER', unit)
            _, vE = claim(cfg, W, win, 'EXCL', unit)
            _, vEf = claim(cfg, W, win, 'EXCL', unit, flip=True)
            _, vLf = claim(cfg, W, win, 'LETTER', unit, flip=True)
            a = js['rs8']['A']
            RESULT['%s W%d %s' % (name, W, WIN[win])] = dict(LETTER=vL, LETTER_rfirst=vLf, EXCL=vE, EXCL_rfirst=vEf)
            P('%s W%d: ours %s#%s %.4f [%.4f-%.4f] K%d (%s) i%.1f s%.1f' % (name, W, OURS[W][0], OURS[W][1], a['m'], a['mn'], a['mx'], a['K'],
              '/'.join(str(len(series('C4-AB', OURS[W][0], OURS[W][1], W, win)[p])) for p in (0, 1, 2)), 100 * a['i'], 100 * a['s']))
            for b, j in js.items():
                P('     %s %.4f [%.4f-%.4f] K%d i%.1f s%.1f ratio %.3f  %s' % (b, j['B']['m'], j['B']['mn'], j['B']['mx'], j['B']['K'], 100 * j['B']['i'], 100 * j['B']['s'], j['pooled']['ratio'], fl(j)))
            P('     -> LETTER %s | LETTER Rapier-first %s | EXCL %s | EXCL Rapier-first %s' % (vL, vLf, vE, vEf))

P('')
P('== post hoc: J-D#tip as ours at every W (the only K 3/3/3 cell is expected at W16)')
for win in (1, 0):
    for name, cfg, unit in CLAIMS:
        for W in WS:
            js, vL = claim(cfg, W, win, 'LETTER', unit, ours=('C4-JD', 'tip'))
            Ks = '/'.join(str(len(series('C4-AB', 'C4-JD', 'tip', W, win)[p])) for p in (0, 1, 2))
            if Ks == '3/3/3':
                P('  %s %s W%d (K %s): LETTER %s; ratios rs8 %.3f rs4 %.3f; %s | %s' % (WIN[win], name, W, Ks, vL, js['rs8']['pooled']['ratio'], js['rs4']['pooled']['ratio'], fl(js['rs8']), fl(js['rs4'])))

P('')
P('== reported: cells [100,500) and [0,100), ms; scaling T1/TW')
TAB = {'ours': {W: ('C4-AB',) + OURS[W] for W in WS}, 'RP-D rs8': {W: ('C4-RAPIER', 'RP-D', 'rs8') for W in WS},
       'RP-D rs4': {W: ('C4-RAPIER', 'RP-D', 'rs4') for W in WS}, 'RP-M rs8': {W: ('C4-RAPIER', 'RP-M', 'rs8') for W in WS},
       'RP-M rs4': {W: ('C4-RAPIER', 'RP-M', 'rs4') for W in WS}, 'Jolt 5.6': {W: ('C4-AB', 'C4-jolt56', 'j56') for W in WS}}
MED = {}
for win in (1, 0):
    for eng, m in TAB.items():
        meds = {}
        ns = {}
        for W in WS:
            s = series(m[W][0], m[W][1], m[W][2], W, win)
            xs = [v for p in (0, 1, 2) for v in s[p]]
            meds[W] = statistics.median(xs)
            ns[W] = '%d(%s)' % (len(xs), '/'.join(str(len(s[p])) for p in (0, 1, 2)))
        MED[(eng, win)] = meds
        P('%s %-9s T: %s | n %s | T1/TW: %s | T16/T8 %.3f' % (WIN[win], eng, ' / '.join('%.3f' % meds[W] for W in WS), ' '.join(ns[W] for W in WS),
          ' / '.join('%.2f' % (meds[1] / meds[W]) for W in WS[1:]), meds[16] / meds[8]))
for eng in ('RP-D rs8', 'RP-M rs8', 'ours'):
    P('  %s / Jolt [100,500): %s' % (eng, ' / '.join('%.3f' % (MED[(eng, 1)][W] / MED[('Jolt 5.6', 1)][W]) for W in WS)))

json.dump(RESULT, open(os.path.join(HERE, 'v1_recompute.json'), 'w'), indent=1)
open(os.path.join(HERE, 'v1_recompute.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
