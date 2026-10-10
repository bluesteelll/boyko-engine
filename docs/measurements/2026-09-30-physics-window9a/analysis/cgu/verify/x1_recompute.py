"""Adversarial verifier, window 9a block C4-CGU. Written from the rulings and raw/ BEFORE reading the analyst's scripts.
Shares no code with analysis/c4ab/lib9a.py or analysis/cgu/*: own CSV parser, own validity/clean/slot rules, own
statistics (recipe: win8b/analysis.md 'Statistics' + ruling 1 of RULINGS-2026-09-27-W8 + ruling 8 of RULINGS-2026-09-29-W8B).
Read-only on raw/, rows9a*.json, bin/, gate/fixtures/."""
import hashlib, json, math, os, sys, collections
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W9A = os.path.abspath(os.path.join(HERE, '..', '..', '..'))
RAW = os.path.join(W9A, 'raw')
TAG = '033740'
BLOCK = 'C4-CGU'
ROW = 'C4-JDcgu'
OUT = []
def P(*a):
    s = ' '.join(str(x) for x in a); OUT.append(s); print(s)

X = json.load(open(os.path.join(W9A, 'rows9a.extra.json'), encoding='utf-8'))
R0 = json.load(open(os.path.join(W9A, 'rows9a.json'), encoding='utf-8'))
BINS = dict(R0['binaries']); BINS.update(X['binaries'])
row = [r for r in X['rows'] if r['id'] == ROW][0]
jd = [r for r in R0['rows'] if r['id'] == 'C4-JD'][0]
# typed from PREP.md / the task brief, not read from the rows
PIN = {'tip': '8d6e7d4173857ca83b47cd350890b8e178451fb48f030ac3d25aaef33cd268c3',
       'tipcgu1': '3e1f72babf76987260988481f12ddbe409ecdc308ce141326529ef89b020d771'}
EXEF = {'tip': 'runner_tip_50e31f1a.exe', 'tipcgu1': 'runner_tipcgu1_50e31f1a.exe'}
sums = {}
for line in open(os.path.join(W9A, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        h, n = line.split(maxsplit=1); sums[n.strip().lstrip('*')] = h
P('== binary identity ==')
for k in ('tip', 'tipcgu1'):
    fp = os.path.join(W9A, 'bin', EXEF[k])
    h = hashlib.sha256(open(fp, 'rb').read()).hexdigest()
    allv = {h, PIN[k], BINS[k]['sha256_pin'], sums.get('bin/' + EXEF[k])}
    P(k, 'file', h[:12], 'size', os.path.getsize(fp), 'pin(brief)', PIN[k][:12], 'rows pin', BINS[k]['sha256_pin'][:12],
      'SHA256SUMS', sums.get('bin/' + EXEF[k], '?')[:12], 'ALL EQUAL' if len(allv) == 1 else 'MISMATCH')
P('row args == C4-JD args:', row['args'] == jd['args'], row['args'])
P('row metric_windows', row['metric_windows'], 'workers', row['workers'], 'steps', row['steps'], 'pose_ref', row['pose_ref'])

fixb = open(os.path.join(W9A, 'gate', 'fixtures', 'JT500.pose'), 'rb').read()
P('JT500.pose sha256', hashlib.sha256(fixb).hexdigest()[:12], 'len', len(fixb))
FIXHASH = json.load(open(os.path.join(W9A, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))['JT500']['hash']

recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]
cg = [r for r in recs if r.get('block') == BLOCK]
P('records with block C4-CGU:', len(cg), dict(collections.Counter(r.get('run_tag') for r in cg)))
done = {(r['pass'], r['pass_attempt']) for r in cg if r.get('pass_done') and r.get('run_tag') == TAG}
P('pass_done:', sorted(done))
allattempts = collections.Counter((r.get('pass'), r.get('pass_attempt')) for r in cg if 'row' in r)
P('process records by (pass, pass_attempt):', dict(allattempts))
procs = [r for r in cg if 'row' in r and r.get('run_tag') == TAG and r.get('attempt') != 'warmup']
warm = [r for r in cg if r.get('attempt') == 'warmup']
P('process records (non-warm-up):', len(procs), dict(collections.Counter(r['attempt'] for r in procs)), 'rows', dict(collections.Counter(r['row'] for r in procs)), 'warm-ups', len(warm), 'timed flags', dict(collections.Counter(r.get('timed') for r in procs)))

def csv_wall(path):
    lines = open(path, encoding='utf-8').read().splitlines()
    hdr = lines[0].split(',')
    iw, istep = hdr.index('wall_ns'), hdr.index('step')
    vals = []
    for i, l in enumerate(lines[1:]):
        f = l.split(',')
        assert int(f[istep]) == i, (path, i)
        vals.append(int(f[iw]))
    return vals

def strip_paths(args):
    out, skip = [], False
    for a in args:
        if skip:
            skip = False; continue
        if a in ('--csv', '--pose-out', '--expect-pose', '--label'):
            skip = True; continue
        out.append(a)
    return out

def validity(r):
    why = []
    b = r['binary']
    if r.get('exit') != 0: why.append('exit')
    if r.get('hang'): why.append('hang')
    if r.get('exe_sha256') != PIN.get(b): why.append('sha!=pin')
    if os.path.basename(r.get('exe', '')) != EXEF.get(b): why.append('exe name')
    if r.get('role') != b: why.append('role')
    if r.get('commit') != BINS[b]['commit']: why.append('commit field')
    if r.get('row') != ROW: why.append('row')
    exp = row['args'] + ['--workers', str(r['W']), '--steps', '500', '--window', '0..500']
    if strip_paths(r.get('args', [])) != exp: why.append('args %s' % strip_paths(r.get('args', [])))
    s = r.get('summary') or {}
    d = r['cwd']
    so = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
    if FIXHASH not in so: why.append('stdout lacks pose hash')
    if s.get('workers') != r['W']: why.append('workers')
    if s.get('void_steps') != 0: why.append('void')
    if s.get('debug_assertions') is not False: why.append('debug')
    if s.get('target_env') != 'msvc': why.append('env')
    if s.get('expect_pose') != 'match': why.append('expect_pose')
    if s.get('pose_hash') != FIXHASH: why.append('pose_hash')
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != 'Tree': why.append('bp')
    if cfg.get('broadphase_select') != 'Manual': why.append('select')
    if cfg.get('tree_brute_max_rows') != 128: why.append('brute')
    if cfg.get('sleeping') is not False: why.append('sleep')
    td = s.get('broadphase_tree') or {}
    for k, v in row['tree_diag_expect'].items():
        if td.get(k) != v: why.append('td ' + k)
    if s.get('armed'): why.append('armed')
    if s.get('canary_ns') is not None: why.append('canary')
    pp = os.path.join(d, 'pose.bin')
    pb = open(pp, 'rb').read() if os.path.exists(pp) else b''
    if pb != fixb: why.append('pose bytes')
    wall = csv_wall(os.path.join(d, 'run.csv'))
    if len(wall) != 500: why.append('csv len')
    wm = sum(wall) / 500.0
    if abs(wm - s.get('window_mean_ns', -1)) > 1.0: why.append('window mean')
    return why, wall, s

def clean(r):
    why = []
    for k in ('receipt_before', 'receipt_after'):
        rc = r.get(k) or {}
        a = rc.get('cpu_avg')
        if a is None or a > 5.0: why.append('%s %s' % (k, a))
        if rc.get('build_procs_busy'): why.append(k + ' build busy')
        pr = rc.get('presence') or {}
        if pr.get('build') or pr.get('lane'): why.append(k + ' presence')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0: why.append('witness %s' % ob)
    if r.get('build_proc_during'): why.append('build during')
    return why

info = {}
nvalid_fail = 0
drv_mismatch = 0
for r in procs + warm:
    v, wall, s = validity(r)
    c = clean(r)
    info[id(r)] = (v, c, wall)
    if v:
        nvalid_fail += 1; P('INVALID', r['pass'], r['seq'], r['attempt'], r['binary'], r['W'], v)
    if bool(c) != bool(r.get('contaminated')) or (not v) != bool(r.get('valid')):
        drv_mismatch += 1; P('DRIVER DISAGREES', r['pass'], r['seq'], r['attempt'], c, r.get('contaminated'), r.get('valid'))
P('invalid processes (incl. warm-ups):', nvalid_fail, ' driver disagreements on clean/valid:', drv_mismatch)
P('exe sha by binary:', dict(collections.Counter((r['binary'], r['exe_sha256'][:8]) for r in procs + warm)))
P('final manifolds/pairs:', dict(collections.Counter((r['summary']['final_manifolds'], r['summary']['final_pairs']) for r in procs + warm)))

slots = collections.defaultdict(list)
for r in procs:
    if (r['pass'], r['pass_attempt']) not in done:
        continue
    slots[(r['pass'], r['seq'])].append(r)
used, dropped = [], []
for k, rs in sorted(slots.items()):
    ids = {(x['binary'], x['W'], x['round']) for x in rs}
    assert len(ids) == 1, ('slot identity', k, ids)
    orig = [x for x in rs if x['attempt'] == 'original']
    assert len(orig) == 1, k
    rer = sorted([x for x in rs if x['attempt'] == 'rerun'], key=lambda x: x['rerun_no'])
    pick = None
    for x in orig + rer:
        v, c, _ = info[id(x)]
        if not v and not c:
            pick = x
            break
    if pick is not None:
        used.append(pick)
    else:
        dropped.append(k)
P('slots', len(slots), 'used', len(used), 'dropped', len(dropped), 'used reruns', sum(1 for x in used if x['attempt'] == 'rerun'),
  'max rerun_no', max((x.get('rerun_no', 0) for x in procs), default=0))
P('unclean originals:', sum(1 for k, rs in slots.items() for x in rs if x['attempt'] == 'original' and info[id(x)][1]),
  ' reruns total', sum(1 for x in procs if x['attempt'] == 'rerun'))
why_counter = collections.Counter()
for x in procs:
    for w in info[id(x)][1]:
        why_counter['witness' if w.startswith('witness') else w.split(' ')[0]] += 1
P('unclean reasons (all attempts):', dict(why_counter))

pcs = {(r['pass'], r['pc_binary'], r['pc_W']): r for r in recs if r.get('passcell') and r.get('block') == BLOCK and r.get('run_tag') == TAG}
mineK = collections.Counter((x['pass'], x['binary'], x['W']) for x in used)
P('passcells', len(pcs), 'K==driver k_clean on', sum(1 for k, r in pcs.items() if mineK.get(k, 0) == r['k_clean']), 'min K', min(mineK.values()),
  'any short', any(r['short'] for r in pcs.values()), 'all gates', all(r['gates'] for r in pcs.values()))

def q_incl(xs, p):
    xs = sorted(xs); h = (len(xs) - 1) * p; lo = math.floor(h); hi = math.ceil(h)
    return xs[lo] + (xs[hi] - xs[lo]) * (h - lo)
def med(xs):
    xs = sorted(xs); n = len(xs)
    return xs[n // 2] if n % 2 else 0.5 * (xs[n // 2 - 1] + xs[n // 2])
def sd(xs):
    m = sum(xs) / len(xs)
    return math.sqrt(sum((x - m) ** 2 for x in xs) / (len(xs) - 1))
def stats(xs):
    m = med(xs)
    return dict(K=len(xs), m=m, lo=min(xs), hi=max(xs), i=(q_incl(xs, .75) - q_incl(xs, .25)) / m,
                s=1.2533 * sd(xs) / math.sqrt(len(xs)) / m, r=(max(xs) - min(xs)) / m)
def comp(a, b):
    ratio = b['m'] / a['m']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'isr'}
    ok = a['K'] >= 3 and b['K'] >= 3
    fl = {k: ok and e > bars[k] for k in 'isr'}
    sep = b['hi'] < a['lo'] or b['lo'] > a['hi']
    return dict(ratio=ratio, bars=bars, fl=fl, sep=sep)
def yn(c):
    return ''.join(('Y' if c['fl'][k] else 'n') for k in 'isr')

WINS = {'[100,500)': (100, 500), '[0,500)': (0, 500)}
res = {}
VALS = {}
for wn, (a0, b0) in WINS.items():
    P('')
    P('== window', wn, '(B = tipcgu1 / A = tip) ==')
    for W in (1, 2, 4, 8, 16):
        vals = {}
        for bkey in ('tip', 'tipcgu1'):
            for p in (0, 1, 2):
                vals[(bkey, p)] = [sum(info[id(x)][2][a0:b0]) / (b0 - a0) / 1e6 for x in used
                                   if x['binary'] == bkey and x['W'] == W and x['pass'] == p]
        VALS[(wn, W)] = vals
        A = stats([v for p in (0, 1, 2) for v in vals[('tip', p)]])
        B = stats([v for p in (0, 1, 2) for v in vals[('tipcgu1', p)]])
        pooled = comp(A, B)
        per = [comp(stats(vals[('tip', p)]), stats(vals[('tipcgu1', p)])) for p in (0, 1, 2)]
        sign = pooled['ratio'] > 1
        same = all((c['ratio'] > 1) == sign for c in per)
        claimed = pooled['fl']['i'] and pooled['fl']['s'] and all(c['fl']['i'] and c['fl']['s'] for c in per) and same
        strong = claimed and pooled['fl']['r'] and all(c['fl']['r'] for c in per)
        dirn = 'cgu1 slower' if sign else 'cgu1 faster'
        res[(wn, W)] = dict(claimed=claimed, strong=strong, dirn=dirn, ratio=pooled['ratio'], A=A['m'], B=B['m'],
                            bar_i=pooled['bars']['i'], bar_s=pooled['bars']['s'], bar_r=pooled['bars']['r'],
                            pass_ratio=[c['ratio'] for c in per], pooled_flags=yn(pooled), pass_flags=[yn(c) for c in per],
                            A_iqr=A['i'], A_se=A['s'], B_iqr=B['i'], B_se=B['s'], A_rng=[A['lo'], A['hi']], B_rng=[B['lo'], B['hi']])
        line = (f"W{W:<2} A {A['m']:.4f} [{A['lo']:.4f}-{A['hi']:.4f}] n={A['K']} i {100*A['i']:.2f}% s {100*A['s']:.2f}% | "
                f"B {B['m']:.4f} [{B['lo']:.4f}-{B['hi']:.4f}] n={B['K']} i {100*B['i']:.2f}% s {100*B['s']:.2f}% | "
                f"B/A {pooled['ratio']:.4f} ({100*(pooled['ratio']-1):+.2f} %, {1000*(B['m']-A['m']):+.1f} us) bars i/s/r "
                f"{100*pooled['bars']['i']:.2f}/{100*pooled['bars']['s']:.2f}/{100*pooled['bars']['r']:.2f} pooled {yn(pooled)}")
        if pooled['sep']:
            line += ' sep'
        line += '; ' + ' '.join(f"p{p}:{yn(c)}({len(vals[('tip', p)])}/{len(vals[('tipcgu1', p)])} {c['ratio']:.4f})" for p, c in zip((0, 1, 2), per))
        verdict = ('CLAIMED' + (' STRONG' if strong else '') + ' ' + dirn) if claimed else 'NOT CLAIMED'
        P(line + ' -> ' + verdict)
P('')
P('== pre-registered claim (primary [100,500); the beside window for reference only) ==')
for wn in WINS:
    w8 = res[(wn, 8)]
    faster_w8 = w8['claimed'] and w8['dirn'] == 'cgu1 faster'
    slower_any = [W for W in (1, 2, 4, 8, 16) if res[(wn, W)]['claimed'] and res[(wn, W)]['dirn'] == 'cgu1 slower']
    faster_any = [W for W in (1, 2, 4, 8, 16) if res[(wn, W)]['claimed'] and res[(wn, W)]['dirn'] == 'cgu1 faster']
    P(wn, 'cgu1 claimed faster at W8:', faster_w8, '| claimed slower at W:', slower_any, '| claimed faster at W:', faster_any,
      '-> PROFILE MOVES TO cgu1:', faster_w8 and not slower_any)
open(os.path.join(HERE, 'x1_recompute.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
json.dump({f'{k[0]} W{k[1]}': v for k, v in res.items()}, open(os.path.join(HERE, 'x1_recompute.json'), 'w'), indent=1)
