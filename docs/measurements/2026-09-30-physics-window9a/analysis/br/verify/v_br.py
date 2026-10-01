"""Independent verification of the C4-BR group (window 9a resume, run_tag 033740). Read-only on raw/.
Own reader + own statistics (re-implemented from lib9a's documented arithmetic; cross-checked against lib9a at the end).
"""
import hashlib, json, math, os, re, statistics, sys
sys.dont_write_bytecode = True
W9A = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a'
W8B = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b'
W7RED = 'D:/wt/merge/docs/measurements/2026-09-25-physics-window7/wave2/analyst/reduction.json'
RAW = W9A + '/raw'
OUT = []
def P(s=''):
    OUT.append(str(s)); print(s)

recs = [json.loads(l) for l in open(RAW + '/runs.jsonl', encoding='utf-8') if l.strip()]
R = json.load(open(W9A + '/rows9a.json', encoding='utf-8'))
row = [r for r in R['rows'] if r['id'] == 'C4-BR'][0]
BIN = R['binaries']
SUMS = {}
for l in open(W9A + '/bin/SHA256SUMS', encoding='utf-8'):
    if l.strip():
        h, n = l.split(maxsplit=1); SUMS[n.strip().lstrip('*')] = h
IDS = sorted(row['expect'])
P(f'expected ids ({len(IDS)}): {IDS}')
RX = re.compile(row['args'][-1])
assert all(RX.match(i) for i in IDS)

# ---------- census
br_all = [r for r in recs if r.get('block') == 'C4-BR']
tags = {}
for r in br_all:
    tags.setdefault(r.get('run_tag'), []).append(r)
for t, rs in tags.items():
    P(f"run_tag {t}: {len(rs)} records; process recs {sum(1 for r in rs if 'row' in r)}; pass_done {sum(1 for r in rs if r.get('pass_done'))}; passcell {sum(1 for r in rs if r.get('passcell'))}; aborted {sum(1 for r in rs if r.get('aborted'))}")
closed = {(r['run_tag'], r['pass'], r['pass_attempt']) for r in br_all if r.get('pass_done')}
P(f'closed passes: {sorted(closed)}')
procs = [r for r in br_all if 'row' in r and r.get('attempt') != 'warmup' and r.get('timed', True)
         and (r['run_tag'], r['pass'], r['pass_attempt']) in closed]
P(f'processes in closed passes: {len(procs)}; attempts {sorted(set(r["attempt"] for r in procs))}')
assert all(r['run_tag'] == '033740' for r in procs)
for r in br_all:
    if r.get('passcell'):
        P(f"passcell {r['run_tag']} {r['pc_binary']} k_target {r['k_target']} k_clean {r['k_clean']} reruns {r['reruns']} gates {r['gates']} short {r['short']}")

def sha_file(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()
FILESHA = {k: sha_file(W9A + '/' + BIN[k]['exe']) for k in ('g4r7', 'g4r8b', 'g4rT')}
P(f'file sha == pin: {[(k, FILESHA[k] == BIN[k]["sha256_pin"]) for k in FILESHA]}')

def clean_why(r):
    why = []
    for k in ('receipt_before', 'receipt_after'):
        rc = r.get(k) or {}
        v = rc.get('cpu_avg')
        if v is None or v > 5.0: why.append(f'{k} {v}')
        if rc.get('build_procs_busy'): why.append(f'{k} build busy')
        pr = rc.get('presence') or {}
        if pr.get('build') or pr.get('lane'): why.append(f'{k} presence')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0: why.append(f'witness {ob}')
    if r.get('build_proc_during'): why.append('build during')
    if r.get('void_names_new_during'): why.append('void names during')
    return why

UNIT = {'\u00b5s': 1.0, 'us': 1.0, 'ns': 1e-3, 'ms': 1e3}
EST_RX = re.compile(r'(bp_g4_\S+)\s+time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]')
BASE_RX = re.compile(r'change:|Performance has|No change|regressed|improved')

def read_proc(r):
    why = []
    if r.get('exit') != 0: why.append(f"exit {r.get('exit')}")
    if r.get('hang'): why.append('hang')
    b = r['binary']
    if r.get('exe_sha256') != BIN[b]['sha256_pin'] or SUMS.get(BIN[b]['exe']) != BIN[b]['sha256_pin'] or FILESHA[b] != BIN[b]['sha256_pin']:
        why.append('sha')
    if os.path.basename(r['exe'].replace(chr(92), '/')) != os.path.basename(BIN[b]['exe']): why.append('exe path')
    if r['args'] != row['args']: why.append('args')
    cwd = r['cwd'].replace(chr(92), '/')
    ch = (r.get('env_extra') or {}).get('CRITERION_HOME', '').replace(chr(92), '/')
    if ch != cwd + '/criterion': why.append('CRITERION_HOME')
    crit = cwd + '/criterion'
    on_disk = []
    for g in os.listdir(crit):
        for f in os.listdir(crit + '/' + g):
            for s in os.listdir(crit + '/' + g + '/' + f):
                on_disk.append(g + '/' + f + '/' + s)
    if sorted(on_disk) != IDS: why.append('ids on disk ' + str(sorted(on_disk)))
    so = open(cwd + '/stdout.txt', encoding='utf-8', errors='replace').read()
    est = {}
    for m in EST_RX.finditer(so):
        est[m.group(1)] = float(m.group(4)) * UNIT[m.group(5)]
    if sorted(est) != IDS: why.append('stdout ids ' + str(sorted(est)))
    if BASE_RX.search(so): why.append('baseline comparison in stdout')
    vals = {}
    for i in IDS:
        sj = json.load(open(crit + '/' + i + '/new/sample.json'))
        if sj['sampling_mode'] != 'Flat' or len(sj['times']) != 10 or len(sj['iters']) != 10: why.append(i + ' sample shape')
        vals[i] = statistics.fmean(t / n for t, n in zip(sj['times'], sj['iters'])) / 1000.0
        if i in est and abs(est[i] - vals[i]) / vals[i] > 1e-3: why.append(i + ' printed vs sample')
        dm = (r.get('criterion') or {}).get('est_ms', {}).get(i)
        if dm is None or abs(dm[1] * 1000 - vals[i]) > 1e-3: why.append(i + ' driver est')
    maxdev = max(abs(est[i] - vals[i]) for i in IDS if i in est) if est else float('nan')
    return {'r': r, 'b': b, 'round': r['round'], 'seq': r['seq'], 'vals': vals, 'valid_why': why,
            'clean_why': clean_why(r), 'maxdev_print': maxdev, 'driver_valid': r.get('valid'),
            'driver_contam': r.get('contaminated')}

PS = [read_proc(r) for r in procs]
P('\n== per process')
for p in sorted(PS, key=lambda p: p['seq']):
    r = p['r']
    perf = [x['total'] for x in (r.get('perf') or [])]
    P(f"seq {p['seq']} r{p['round']} {p['b']:6s} valid_why={p['valid_why']} clean_why={p['clean_why']} drv valid={p['driver_valid']} contam={p['driver_contam']} "
      f"before {r['receipt_before']['cpu_avg']} after {r['receipt_after']['cpu_avg']} witness {r['others_busy_pct']} top={r['others_top5'][0]['name']} "
      f"clock mean {statistics.fmean(perf):.1f} (n {len(perf)}) start {r['start'][11:19]} end {r['end'][11:19]} maxdev_print_us {p['maxdev_print']:.2e} wall {r['wall_s']}")
used = [p for p in PS if not p['valid_why'] and not p['clean_why']]
P(f'used {len(used)} of {len(PS)}')
P('executed order: ' + str([(p['seq'], p['round'], p['b']) for p in sorted(PS, key=lambda p: p['seq'])]))

def cell(xs):
    xs = sorted(xs); m = statistics.median(xs)
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return {'K': len(xs), 'med': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / m, 'i': (q[2] - q[0]) / m,
            's': 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) / m, 'xs': xs}
def cmp(a, b):
    ratio = b['med'] / a['med']; e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'isr'}
    ok = a['K'] >= 3 and b['K'] >= 3
    fl = {k: ok and e > bars[k] for k in 'isr'}
    sep = b['max'] < a['min'] or b['min'] > a['max']
    return {'ratio': ratio, 'bars': bars, 'fl': fl, 'sep': sep}
def yn(c):
    return ''.join('Y' if c['fl'][k] else 'n' for k in 'isr')

BINS3 = ('g4r7', 'g4r8b', 'g4rT')
V = {(b, i): [p['vals'][i] for p in used if p['b'] == b] for b in BINS3 for i in IDS}
C = {k: cell(v) for k, v in V.items()}
ORD = ['bp_g4_uniform/all_pairs/144', 'bp_g4_uniform/all_pairs/256', 'bp_g4_disparity/all_pairs/144', 'bp_g4_disparity/all_pairs/256',
       'bp_g4_uniform/tree/144', 'bp_g4_uniform/tree/256', 'bp_g4_disparity/tree/144', 'bp_g4_disparity/tree/256']
AP = [i for i in ORD if '/all_pairs/' in i]
TR = [i for i in ORD if '/tree/' in i]
def fc(c):
    return '%.3f [%.3f-%.3f] n%d %.2f/%.2f/%.2f' % (c['med'], c['min'], c['max'], c['K'], 100 * c['i'], 100 * c['s'], 100 * c['r'])
P('\n== cells (us/step; median [min-max] n; IQR% / SE% / range%)')
for i in ORD:
    P(i + ': ' + ' | '.join(b + ' ' + fc(C[(b, i)]) for b in BINS3))

P('\n== comparisons B/A (bars 2i/2s/2r %) flags i s r, sep')
res = {}
CMPS = [('BR-1', 'g4r7', 'g4r8b', AP), ('BR-2', 'g4r7', 'g4rT', AP), ('BR-C 8b/7', 'g4r7', 'g4r8b', TR),
        ('BR-C T/7', 'g4r7', 'g4rT', TR), ('BR-3 ap T/8b', 'g4r8b', 'g4rT', AP), ('BR-3 tree T/8b', 'g4r8b', 'g4rT', TR)]
for name, A, B, ids in CMPS:
    rr = []
    for i in ids:
        c = cmp(C[(A, i)], C[(B, i)]); rr.append(c['ratio']); res[(name, i)] = (A, B, c)
        P('%s %s/%s %s: %.4f (%.2f/%.2f/%.2f) %s sep=%s' % (name, B, A, i, c['ratio'], 100 * c['bars']['i'], 100 * c['bars']['s'], 100 * c['bars']['r'], yn(c), c['sep']))
    P('  %s median of ratios %.4f [%.4f-%.4f]' % (name, statistics.median(rr), min(rr), max(rr)))

P('\n== in-binary all_pairs/tree')
for b in BINS3:
    for fam in ('uniform', 'disparity'):
        for n in (144, 256):
            c = cmp(C[(b, 'bp_g4_%s/tree/%d' % (fam, n))], C[(b, 'bp_g4_%s/all_pairs/%d' % (fam, n))])
            P('%s %s %d: ap/tree %.4f %s sep=%s' % (b, fam, n, c['ratio'], yn(c), c['sep']))

P('\n== paired per-round ratios (same round)')
byr = {(p['b'], p['round']): p for p in used}
for A, B, ids, nm in [('g4r7', 'g4r8b', AP, 'ap'), ('g4r7', 'g4rT', AP, 'ap'), ('g4r8b', 'g4rT', AP, 'ap'), ('g4r7', 'g4r8b', TR, 'tree'), ('g4r7', 'g4rT', TR, 'tree')]:
    rs = [byr[(B, k)]['vals'][i] / byr[(A, k)]['vals'][i] for k in range(3) for i in ids]
    P('%s/%s %s: n %d min %.4f max %.4f median %.4f' % (B, A, nm, len(rs), min(rs), max(rs), statistics.median(rs)))

P('\n== per-process values (us)')
for p in sorted(used, key=lambda p: p['seq']):
    P('seq %d %s: ' % (p['seq'], p['b']) + ' '.join('%s=%.3f' % (i.split('bp_g4_')[1], p['vals'][i]) for i in ORD))

P('\n== POST HOC: cross-window')
w7 = json.load(open(W7RED))['cells']
w7c = {}
for i in ORD:
    fam, arm, n = i.split('bp_g4_')[1].split('/')
    w7c[i] = cell([x * 1000 for x in w7['Q3 %s %s/%s' % (fam, arm, n)]['values']])
r8 = [json.loads(l) for l in open(W8B + '/raw/runs.jsonl', encoding='utf-8') if l.strip()]
g8 = [r for r in r8 if r.get('block') == 'F3-G4' and 'row' in r]
P('8b F3-G4 procs %d: shas %s contaminated %s valid %s' % (len(g8), set(r['exe_sha256'] for r in g8), [r['contaminated'] for r in g8], [r['valid'] for r in g8]))
v8 = {i: [] for i in ORD}
for r in g8:
    cw = r['cwd'].replace(chr(92), '/')
    for i in ORD:
        sj = json.load(open(cw + '/criterion/' + i + '/new/sample.json'))
        v8[i].append(statistics.fmean(t / n for t, n in zip(sj['times'], sj['iters'])) / 1000.0)
c8 = {i: cell(v8[i]) for i in ORD}
for i in ORD:
    a = cmp(w7c[i], C[('g4r7', i)]); b = cmp(c8[i], C[('g4r8b', i)]); x = cmp(w7c[i], c8[i])
    P('%s: w7 %s | 8b %s | g4r7 9a/w7 %.4f %s sep=%s | g4r8b 9a/8b %.4f %s sep=%s | 8b/w7 %.4f %s' % (
        i, fc(w7c[i]), fc(c8[i]), a['ratio'], yn(a), a['sep'], b['ratio'], yn(b), b['sep'], x['ratio'], yn(x)))
for nm, ids in (('all_pairs', AP), ('tree', TR)):
    g7 = [C[('g4r7', i)]['med'] / w7c[i]['med'] for i in ids]
    g8r = [C[('g4r8b', i)]['med'] / c8[i]['med'] for i in ids]
    s87 = [c8[i]['med'] / w7c[i]['med'] for i in ids]
    P('%s: g4r7 9a/w7 median %.4f [%.4f-%.4f]; g4r8b 9a/8b median %.4f [%.4f-%.4f]; 8b/w7 median %.4f [%.4f-%.4f]' % (
        nm, statistics.median(g7), min(g7), max(g7), statistics.median(g8r), min(g8r), max(g8r), statistics.median(s87), min(s87), max(s87)))
P('decomposition per id (all_pairs): 8b/7 = binary(9a g4r8b/g4r7) x window(g4r8b 8b/9a) x window(g4r7 9a/7)')
fr = []
for i in AP:
    tot = c8[i]['med'] / w7c[i]['med']
    binr = C[('g4r8b', i)]['med'] / C[('g4r7', i)]['med']
    wb = c8[i]['med'] / C[('g4r8b', i)]['med']
    w7r = C[('g4r7', i)]['med'] / w7c[i]['med']
    f = math.log(binr) / math.log(tot); fr.append(f)
    P('  %s: total %.4f = %.4f x %.4f x %.4f (prod %.4f); binary share of log %.1f %%' % (i, tot, binr, wb, w7r, binr * wb * w7r, 100 * f))
tots = [c8[i]['med'] / w7c[i]['med'] for i in AP]
bb = [C[('g4r8b', i)]['med'] / C[('g4r7', i)]['med'] for i in AP]
wbs = [c8[i]['med'] / C[('g4r8b', i)]['med'] for i in AP]
w7s = [C[('g4r7', i)]['med'] / w7c[i]['med'] for i in AP]
P('  medians: total %.4f, binary %.4f, window 8b/9a %.4f, window 9a/7 %.4f; product of medians %.4f; binary share %.1f-%.1f %%' % (
    statistics.median(tots), statistics.median(bb), statistics.median(wbs), statistics.median(w7s),
    statistics.median(bb) * statistics.median(wbs) * statistics.median(w7s), 100 * min(fr), 100 * max(fr)))
P('  share of the median log: %.1f %%' % (100 * math.log(statistics.median(bb)) / math.log(statistics.median(tots))))
P('\n== POST HOC: net of tree')
for B in ('g4r8b', 'g4rT'):
    a = statistics.median([C[(B, i)]['med'] / C[('g4r7', i)]['med'] for i in AP])
    t = statistics.median([C[(B, i)]['med'] / C[('g4r7', i)]['med'] for i in TR])
    per = [(C[(B, i)]['med'] / C[('g4r7', i)]['med']) / (C[(B, i.replace('all_pairs', 'tree'))]['med'] / C[('g4r7', i.replace('all_pairs', 'tree'))]['med']) for i in AP]
    P('%s: median ap ratio %.4f / median tree ratio %.4f = %.4f; per-id ratio-of-ratios median %.4f [%.4f-%.4f]' % (B, a, t, a / t, statistics.median(per), min(per), max(per)))
P('\n== POST HOC: per pair test (uniform)')
for i in ('bp_g4_uniform/all_pairs/144', 'bp_g4_uniform/all_pairs/256'):
    n = int(i.rsplit('/', 1)[1]); npairs = n * (n - 1) / 2
    P('%s: pairs %d; g4r7 %.3f ns/pair, g4r8b %.3f, g4rT %.3f' % (i, npairs, 1000 * C[('g4r7', i)]['med'] / npairs, 1000 * C[('g4r8b', i)]['med'] / npairs, 1000 * C[('g4rT', i)]['med'] / npairs))

P('\n== POST HOC: C4-G4 g4rT at 144 vs BR')
g4 = [r for r in recs if r.get('block') == 'C4-G4']
cl4 = {(r['run_tag'], r['pass'], r['pass_attempt']) for r in g4 if r.get('pass_done')}
pg4 = [r for r in g4 if 'row' in r and r.get('attempt') != 'warmup' and (r['run_tag'], r['pass'], r['pass_attempt']) in cl4]
P('C4-G4 closed passes %s; process records %d; attempts %s; shas %s' % (sorted(cl4), len(pg4), sorted(set(r['attempt'] for r in pg4)), set(r['exe_sha256'] for r in pg4)))
slots = {}
for r in pg4:
    slots.setdefault((r['pass'], r['seq'] if r['attempt'] == 'original' else r.get('slot_seq', r.get('orig_seq', r['seq']))), []).append(r)
P('re-run records: %s' % [(r['pass'], r['seq'], r.get('rerun_no'), {k: r.get(k) for k in r if 'orig' in k or 'slot' in k}) for r in pg4 if r['attempt'] == 'rerun'])
I144 = [i for i in ORD if i.endswith('/144')]
g4v = {i: [] for i in I144}
g4used = 0
for r in pg4:
    if r['attempt'] != 'original':
        continue
    ok = (r.get('exit') == 0 and not clean_why(r) and r.get('exe_sha256') == BIN['g4rT']['sha256_pin'])
    P('  G4 p%d seq %d r%d ok=%s clean_why=%s valid=%s' % (r['pass'], r['seq'], r['round'], ok, clean_why(r), r.get('valid')))
    if not ok:
        continue
    g4used += 1
    cw = r['cwd'].replace(chr(92), '/')
    for i in I144:
        sj = json.load(open(cw + '/criterion/' + i + '/new/sample.json'))
        g4v[i].append(statistics.fmean(t / n for t, n in zip(sj['times'], sj['iters'])) / 1000.0)
P('G4 originals used %d' % g4used)
for i in I144:
    cg = cell(g4v[i])
    a = cmp(C[('g4rT', i)], cg); b = cmp(C[('g4r7', i)], cg)
    P('%s: G4 g4rT %s | G4/BR-g4rT %.4f %s sep=%s | G4/BR-g4r7 %.4f %s sep=%s' % (i, fc(cg), a['ratio'], yn(a), a['sep'], b['ratio'], yn(b), b['sep']))
for fam in ('uniform', 'disparity'):
    cg_ap = cell(g4v['bp_g4_%s/all_pairs/144' % fam]); cg_t = cell(g4v['bp_g4_%s/tree/144' % fam])
    c = cmp(cg_t, cg_ap)
    P('G4 g4rT %s ap/tree at 144: %.4f %s' % (fam, c['ratio'], yn(c)))

sys.path.insert(0, W9A + '/analysis/c4ab')
import lib9a as L
bad = 0
for (name, i), (A, B, c) in res.items():
    lc = L.cmp_(L.cell(V[(A, i)]), L.cell(V[(B, i)]))
    if abs(lc['ratio'] - c['ratio']) > 1e-12 or L.yn(lc) != yn(c) or lc['minmax_sep'] != c['sep']:
        bad += 1
P('\nlib9a cross-check: %d disagreements over %d comparisons' % (bad, len(res)))
HERE = os.path.dirname(os.path.abspath(__file__))
open(HERE + '/v_br.txt', 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
