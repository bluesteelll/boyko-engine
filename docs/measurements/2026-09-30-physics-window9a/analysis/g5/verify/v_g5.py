"""Independent verifier for window 9a C4-G5 (written BEFORE reading the analyst's g5 scripts).
Own CSV parser (csv module), own validity/clean check, own slot rule, statistics per 8b synthlib's definitions:
cell = median over K of per-process values; i = IQR(inclusive)/median; s = 1.2533*SD/sqrt(K)/median; r = (max-min)/median.
cmp B vs A: flag x iff |B/A-1| > 2*hypot(A_x,B_x) and both K>=3. vs_const: flag iff |m/bar-1| > 2*x and K>=3.
Ruling 1: CLAIMED iff i and s flag pooled and in every pass (pass-cells with K<3 set no flag), one sign; STRONG iff r too."""
import csv, json, math, os, statistics, hashlib, sys
from collections import defaultdict

W9 = r"C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win9a"
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = []
def P(s=''):
    OUT.append(str(s)); print(s)

recs = [json.loads(l) for l in open(os.path.join(W9, 'raw', 'runs.jsonl'), encoding='utf-8') if l.strip()]
rows = json.load(open(os.path.join(W9, 'rows9a.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in rows['rows']}
PIN = rows['binaries']['tip']['sha256_pin']
FIX = json.load(open(os.path.join(W9, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))

done = [r for r in recs if r.get('block') == 'C4-G5' and r.get('pass_done')]
P(f"G5 pass_done records: {[(r['pass'], r['pass_attempt'], r['run_tag']) for r in done]}")
done_keys = {(r['run_tag'], r['pass'], r['pass_attempt']) for r in done}

procs = [r for r in recs if r.get('block') == 'C4-G5' and 'row' in r and r.get('attempt') != 'warmup']
warm = [r for r in recs if r.get('block') == 'C4-G5' and r.get('attempt') == 'warmup']
P(f"G5 timed process records: {len(procs)} (original {sum(r['attempt']=='original' for r in procs)}, rerun {sum(r['attempt']=='rerun' for r in procs)}); warm-ups {len(warm)}")

exe_sha = hashlib.sha256(open(os.path.join(W9, rows['binaries']['tip']['exe']), 'rb').read()).hexdigest()
P(f"tip exe sha256 on disk == pin: {exe_sha == PIN}")
fixbytes = {k: open(os.path.join(W9, 'gate', 'fixtures', k + '.pose'), 'rb').read() for k in ('JT500', 'RT500', 'S16500')}

def load_csv(p):
    with open(p, newline='', encoding='utf-8') as f:
        rd = csv.reader(f)
        hdr = [h.strip() for h in next(rd)]
        cols = {h: [] for h in hdr}
        for row in rd:
            for h, v in zip(hdr, row):
                v = v.strip()
                cols[h].append(float(v) if v != '' else None)
    return cols

def summary(d):
    for line in open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace'):
        if line.startswith('SUMMARY '):
            return json.loads(line[8:])
    return None

def valid_why(r):
    row = ROWS[r['row']]
    d = r['cwd']
    why = []
    if r.get('exit') != 0: why.append('exit')
    if r.get('hang'): why.append('hang')
    if r.get('exe_sha256') != PIN: why.append('sha')
    s = summary(d); c = load_csv(os.path.join(d, 'run.csv'))
    if s is None: return ['no summary'], None, None
    if len(c['step']) != 500 or c['step'] != [float(i) for i in range(500)]: why.append('steps')
    if any(x is None or x <= 0 for x in c['wall_ns']): why.append('wall')
    if s.get('void_steps') != 0: why.append('void_steps')
    if s.get('first_void') is not None: why.append('first_void')
    if s.get('expect_pose') != 'match': why.append('expect_pose')
    if s.get('workers') != r['W']: why.append('workers')
    cfg = s['config']
    if cfg.get('broadphase') != row['broadphase']: why.append('broadphase ' + str(cfg.get('broadphase')))
    if cfg.get('tree_brute_max_rows') != 128: why.append('tbmr')
    if cfg.get('broadphase_select') != 'Manual': why.append('select')
    if bool(s.get('armed')) != bool(row['armed']): why.append('armed')
    if s.get('debug_assertions') is not False: why.append('debug')
    fx = row['pose_ref']
    if s.get('pose_hash') != FIX[fx]['hash']: why.append('pose_hash')
    pb = open(os.path.join(d, 'pose.bin'), 'rb').read()
    if pb != fixbytes[fx]: why.append('pose bytes')
    td = row.get('tree_diag_expect')
    bt = s.get('broadphase_tree') or {}
    if td:
        for k, v in td.items():
            if bt.get(k) != v: why.append(f'treediag {k}={bt.get(k)}')
    wm = sum(c['wall_ns'][0:500]) / 500
    if abs(wm - s['window_mean_ns']) > 1.0: why.append('window mean')
    if row['armed'] and any(v != 0 for v in c['void']): why.append('csv void')
    return why, s, c

def clean_why(r):
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg'); ra = (r.get('receipt_after') or {}).get('cpu_avg')
    if rb is None or rb > 5.0: why.append(f'before {rb}')
    if ra is None or ra > 5.0: why.append(f'after {ra}')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0: why.append(f'witness {ob}')
    if r.get('build_proc_during'): why.append('build')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'): why.append(k + ' presence')
    return why

allp = []
for r in procs:
    vw, s, c = valid_why(r)
    cw = clean_why(r)
    drv_clean = not r.get('contaminated')
    p = dict(rec=r, row=r['row'], W=r['W'], pass_=r['pass'], round=r['round'], seq=r['seq'], att=r['attempt'],
             rerun_no=r.get('rerun_no'), vw=vw, cw=cw, s=s, c=c, drv_valid=r.get('valid'), drv_clean=drv_clean,
             in_done=(r['run_tag'], r['pass'], r['pass_attempt']) in done_keys)
    allp.append(p)
P(f"invalid (mine): {[(p['row'],p['W'],p['pass_'],p['seq'],p['vw']) for p in allp if p['vw']]}")
P(f"driver valid disagreements: {[(p['row'],p['seq']) for p in allp if bool(p['drv_valid']) == bool(p['vw'])]}")
P("unclean (mine):")
for p in allp:
    if p['cw']:
        P(f"   p{p['pass_']} r{p['round']} seq{p['seq']} {p['row']} W{p['W']} {p['att']}#{p['rerun_no']}: {p['cw']} wall_s {p['rec'].get('wall_s')}")
P(f"driver clean disagreements: {[(p['row'],p['seq'],p['cw'],p['drv_clean']) for p in allp if p['drv_clean'] == bool(p['cw'])]}")
P(f"processes outside a pass_done pass: {sum(not p['in_done'] for p in allp)}")

slots = defaultdict(list)
for p in allp:
    if p['in_done']:
        slots[(p['pass_'], p['seq'])].append(p)
used = []
for k, ps in sorted(slots.items()):
    orig = [p for p in ps if p['att'] == 'original']
    rer = sorted([p for p in ps if p['att'] == 'rerun'], key=lambda p: p['rerun_no'])
    pick = next((p for p in orig + rer if not p['vw'] and not p['cw']), None)
    if pick: used.append(pick)
    else: P(f"DROPPED slot {k}")
P(f"slots {len(slots)}, used {len(used)}")
kc = defaultdict(lambda: defaultdict(int))
for p in used: kc[(p['row'], p['W'])][p['pass_']] += 1
for k in sorted(kc): P(f"   K {k}: {dict(kc[k])}")

A, B = 100, 500
def med(xs): return statistics.median(xs)
def iqr(xs):
    if len(xs) < 2: return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive'); return q[2] - q[0]
def se(xs): return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) if len(xs) >= 2 else 0.0
def cell(xs):
    xs = sorted(xs); m = med(xs)
    return dict(K=len(xs), m=m, mn=xs[0], mx=xs[-1], i=iqr(xs)/abs(m), s=se(xs)/abs(m), r=(xs[-1]-xs[0])/abs(m))
def cmpc(a, b):
    e = abs(b['m']/a['m'] - 1); ok = a['K'] >= 3 and b['K'] >= 3
    f = {x: ok and e > 2*math.hypot(a[x], b[x]) for x in 'isr'}
    return dict(ratio=b['m']/a['m'], d=b['m']-a['m'], f=f, bars={x: 2*math.hypot(a[x], b[x]) for x in 'isr'})
def vsc(c, bar):
    e = abs(c['m']/bar - 1); ok = c['K'] >= 3
    return dict(ratio=c['m']/bar, d=c['m']-bar, f={x: ok and e > 2*c[x] for x in 'isr'}, bars={x: 2*c[x] for x in 'isr'})
def yn(c): return ''.join('Y' if c['f'][x] else 'n' for x in 'isr')
def r1(pooled, per):
    sg = 1 if pooled['ratio'] > 1 else -1
    same = all((1 if c['ratio'] > 1 else -1) == sg for c in per)
    cl = pooled['f']['i'] and pooled['f']['s'] and all(c['f']['i'] and c['f']['s'] for c in per) and same
    st = cl and pooled['f']['r'] and all(c['f']['r'] for c in per)
    return ('CLAIMED' + (' STRONG' if st else '')) if cl else 'NOT CLAIMED', ('B>A' if sg > 0 else 'B<A')

def vals(row, W, fn, pas=None):
    return [fn(p) for p in used if p['row'] == row and p['W'] == W and (pas is None or p['pass_'] == pas)]
def fmtc(c, sc=1e6):
    return f"{c['m']/sc:.4f} [{c['mn']/sc:.4f}-{c['mx']/sc:.4f}] n={c['K']} (i {100*c['i']:.2f} s {100*c['s']:.2f} r {100*c['r']:.2f} %)"

def judge_pair(name, rowA, rowB, W, fn, sc=1e6, WB=None):
    WB = W if WB is None else WB
    a = cell(vals(rowA, W, fn)); b = cell(vals(rowB, WB, fn))
    pooled = cmpc(a, b)
    per = []; pp = []
    for q in (0, 1, 2):
        ca = cell(vals(rowA, W, fn, q)); cb = cell(vals(rowB, WB, fn, q))
        c = cmpc(ca, cb); per.append(c); pp.append(f"p{q} {ca['m']/sc:.4f}->{cb['m']/sc:.4f} {yn(c)} K{ca['K']}/{cb['K']}")
    v, dirn = r1(pooled, per)
    P(f"{name}: A {rowA} {fmtc(a, sc)} | B {rowB} {fmtc(b, sc)}")
    P(f"    B/A {pooled['ratio']:.4f} ({100*(pooled['ratio']-1):+.2f} %, d {pooled['d']/sc:+.4f}) pooled {yn(pooled)} bars i {100*pooled['bars']['i']:.2f} s {100*pooled['bars']['s']:.2f} r {100*pooled['bars']['r']:.2f}; {' | '.join(pp)} -> {v} [{dirn}]")
    return a, b, pooled, per, v, dirn

def judge_const(name, row, W, fn, bar, sc=1e6):
    c = cell(vals(row, W, fn)); pooled = vsc(c, bar)
    per = []; pp = []
    for q in (0, 1, 2):
        cq = cell(vals(row, W, fn, q)); x = vsc(cq, bar); per.append(x); pp.append(f"p{q} {cq['m']/sc:.4f} {yn(x)} K{cq['K']}")
    v, dirn = r1(pooled, per)
    P(f"{name}: {row}@W{W} {fmtc(c, sc)} vs bar {bar/sc:.4f}: {100*(pooled['ratio']-1):+.2f} % pooled {yn(pooled)}; {' | '.join(pp)} -> {v} [{dirn}]")
    return c, v, dirn

def smed(col):
    return lambda p: statistics.median(p['c'][col][A:B])
def smean(col):
    return lambda p: statistics.fmean(p['c'][col][A:B])
S4 = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')
def sum4_med(p):
    c = p['c']; return statistics.median([sum(c[k][t] for k in S4) for t in range(A, B)])
def sum4_mean(p):
    c = p['c']; return statistics.fmean([sum(c[k][t] for k in S4) for t in range(A, B)])
def wall_mean(p): return statistics.fmean(p['c']['wall_ns'][0:500])
def wall_mean_ab(p): return statistics.fmean(p['c']['wall_ns'][A:B])
SYS = 'sys_physics_broadphase_colored_ns'

P("\n=== B1 (tree span Sigma4, median over [100,500), per process) ===")
judge_const('B1-W1', 'C4-JD-armed', 1, sum4_med, 0.36e6)
judge_const('B1-W8', 'C4-JD-armed', 8, sum4_med, 0.35e6)
P("  (mean-over-steps variant)")
judge_const('B1-W1 mean', 'C4-JD-armed', 1, sum4_mean, 0.36e6)
judge_const('B1-W8 mean', 'C4-JD-armed', 8, sum4_mean, 0.35e6)

P("\n=== B2 realized dbp(8) = SYS(JDap-armed) - SYS(JD-armed), W8 ===")
for lab, fn in (('median over steps', smed(SYS)), ('mean over steps', smean(SYS))):
    a, b, pooled, per, v, d = judge_pair(f'B2 diff ({lab})', 'C4-JD-armed', 'C4-JDap-armed', 8, fn)
    pdl = [round((cell(vals('C4-JDap-armed', 8, fn, q))['m'] - cell(vals('C4-JD-armed', 8, fn, q))['m']) / 1e6, 4) for q in (0, 1, 2)]
    P(f"    delta of cell medians {pooled['d']/1e6:.4f}; per pass {pdl}")
    pr = defaultdict(dict)
    for p in used:
        if p['W'] == 8 and p['row'] in ('C4-JD-armed', 'C4-JDap-armed'):
            pr[(p['pass_'], p['round'])][p['row']] = fn(p)
    deltas = {k: v['C4-JDap-armed'] - v['C4-JD-armed'] for k, v in pr.items()}
    dc = cell(list(deltas.values()))
    P(f"    paired delta per (pass,round): {fmtc(dc)}")
    pooledc = vsc(dc, 1.03e6); per_c = []
    for q in (0, 1, 2):
        cq = cell([v for k, v in deltas.items() if k[0] == q]); per_c.append(vsc(cq, 1.03e6))
    P(f"    paired delta vs 1.03: {100*(pooledc['ratio']-1):+.2f} % pooled {yn(pooledc)} passes {[yn(x) for x in per_c]} -> {r1(pooledc, per_c)}")
    ps = cmpc(cell([x + 1.03e6 for x in vals('C4-JD-armed', 8, fn)]), cell(vals('C4-JDap-armed', 8, fn))); pers = []
    for q in (0, 1, 2):
        pers.append(cmpc(cell([x + 1.03e6 for x in vals('C4-JD-armed', 8, fn, q)]), cell(vals('C4-JDap-armed', 8, fn, q))))
    P(f"    JDap vs (JD+1.03): d {ps['d']/1e6:+.4f} pooled {yn(ps)} passes {[yn(x) for x in pers]} -> {r1(ps, pers)}")
    wc = (min(vals('C4-JDap-armed', 8, fn)) - max(vals('C4-JD-armed', 8, fn))) / 1e6
    P(f"    worst case: min(JDap) - max(JD) = {wc:.4f}")

P("\n=== B3: C4-R (Tree) vs C4-Rap, mean wall [0,500) ===")
for Wx in (1, 8):
    judge_pair(f'B3-W{Wx}', 'C4-Rap', 'C4-R', Wx, wall_mean)
P("  (window [100,500) variant, post hoc)")
for Wx in (1, 8):
    judge_pair(f'B3-W{Wx} [100,500)', 'C4-Rap', 'C4-R', Wx, wall_mean_ab)

P("\n=== B4: C4-S16 vs C4-S16ap W1, mean wall [0,500) (us) ===")
judge_pair('B4', 'C4-S16ap', 'C4-S16', 1, wall_mean, sc=1e3)
judge_pair('B4 [100,500)', 'C4-S16ap', 'C4-S16', 1, wall_mean_ab, sc=1e3)
judge_pair('B4 [0,100)', 'C4-S16ap', 'C4-S16', 1, lambda p: statistics.fmean(p['c']['wall_ns'][0:100]), sc=1e3)

P("\n=== B5 structure ===")
armed = [p for p in allp if ROWS[p['row']]['armed']]
nv = sum(p['s']['void_steps'] != 0 for p in armed)
nf = sum(p['s']['first_void'] is not None for p in armed)
nc = sum(any(v != 0 for v in p['c']['void']) for p in armed)
P(f"armed processes (all incl. unused): {len(armed)}; void_steps!=0: {nv}; first_void non-null: {nf}; csv void nonzero: {nc}")
badc = 0
for p in armed:
    c = p['c']
    tree = ROWS[p['row']]['broadphase'] == 'Tree'
    for k in S4:
        n = c[k.replace('_ns', '_n')]
        want = 1 if tree else 0
        bad = [t for t in range(500) if n[t] != want]
        if bad:
            badc += 1
            P(f"   span count {k} {p['row']} W{p['W']} seq{p['seq']}: bad steps {bad[:5]} ({len(bad)})")
        if not tree and any(v != 0 for v in c[k]):
            P(f"   nonzero AllPairs tree span {k} seq{p['seq']}")
    if tree:
        bad = [t for t in range(500) if c['phys_bp_queried'][t] + c['phys_bp_members'][t] != 1241]
        if bad:
            P(f"   queried+members != 1241 at {len(bad)} steps ({p['row']} W{p['W']} seq{p['seq']}) e.g. {[(t, c['phys_bp_queried'][t], c['phys_bp_members'][t]) for t in bad[:3]]}")
P(f"span-count anomalies: {badc}")
npo = sum(('pose bytes' not in p['vw']) and ('pose_hash' not in p['vw']) for p in allp)
P(f"pose ok (bytes+hash) over all G5 timed processes: {npo}/{len(allp)}")
r4 = [r for r in recs if r.get('block') == 'C4-G5' and 'r4' in r]
P(f"R4 records: {len(r4)}; sample: {json.dumps(r4[0])[:300] if r4 else None}")
pb = {}
for p in allp:
    if p['row'] in ('C4-S16', 'C4-S16ap'):
        pb.setdefault((p['pass_'], p['round'], p['row']), []).append(open(os.path.join(p['rec']['cwd'], 'pose.bin'), 'rb').read())
eq = [all(x == y for x in pb[(q, rr, 'C4-S16')] for y in pb[(q, rr, 'C4-S16ap')]) for (q, rr, row) in pb if row == 'C4-S16']
P(f"own R4 (S16 vs S16ap pose bytes per pass/round): {sum(eq)}/{len(eq)} equal")
bt = next(p['s'].get('broadphase_tree') for p in allp if p['row'] == 'C4-S16')
P(f"S16 TreeDiag (sample): {bt}")

P("\n=== Stage profile JD-armed (median over steps [100,500); cell median; ms) ===")
for Wx in (1, 8):
    parts = {}
    for k in S4:
        parts[k] = cell(vals('C4-JD-armed', Wx, smed(k)))['m'] / 1e6
    s4 = cell(vals('C4-JD-armed', Wx, sum4_med))['m'] / 1e6
    sy = cell(vals('C4-JD-armed', Wx, smed(SYS)))['m'] / 1e6
    wl = cell(vals('C4-JD-armed', Wx, wall_mean_ab))['m'] / 1e6
    wl0 = cell(vals('C4-JD-armed', Wx, wall_mean))['m'] / 1e6
    txt = ' '.join(f"{k[8:-3]} {v:.4f}" for k, v in parts.items())
    P(f"W{Wx}: {txt} | S4 {s4:.4f} sys {sy:.4f} sys-S4 {sy-s4:.5f} | armed wall [100,500) {wl:.4f} [0,500) {wl0:.4f} | query/S4 {100*parts['phys_bp_query_ns']/s4:.1f}% sys/wall {100*sy/wl:.1f}%")
    partsm = {}
    for k in S4:
        partsm[k] = cell(vals('C4-JD-armed', Wx, smean(k)))['m'] / 1e6
    s4m = cell(vals('C4-JD-armed', Wx, sum4_mean))['m'] / 1e6
    sym = cell(vals('C4-JD-armed', Wx, smean(SYS)))['m'] / 1e6
    txt = ' '.join(f"{k[8:-3]} {v:.4f}" for k, v in partsm.items())
    P(f"   mean over steps: {txt} S4 {s4m:.4f} sys {sym:.4f}")
    us = [p for p in used if p['row'] == 'C4-JD-armed' and p['W'] == Wx]
    q = set(); mem = set(); reb = set(); rbi = set()
    pmin = 1e18; pmax = 0; mmin = 1e18; mmax = 0
    diag = set()
    for p in us:
        c = p['c']
        q.update(c['phys_bp_queried'][A:B]); mem.update(c['phys_bp_members'][A:B]); reb.update(c['phys_bp_rebuilds'][A:B])
        rbi.update(i for i, v in enumerate(c['phys_bp_rebuilds']) if v)
        pmin = min(pmin, min(c['phys_bp_pairs'][A:B])); pmax = max(pmax, max(c['phys_bp_pairs'][A:B]))
        mmin = min(mmin, min(c['manifolds'][A:B])); mmax = max(mmax, max(c['manifolds'][A:B]))
        bt = p['s']['broadphase_tree']
        diag.add((bt['leaf_list_leaves'], bt['fallback_leaves'], bt['bp_kernel']))
    P(f"   queried {sorted(q)}; members {sorted(mem)}; rebuilds in [100,500) {sorted(reb)}; rebuild step idx {sorted(rbi)}; bp_pairs {pmin}-{pmax}; manifolds {mmin}-{mmax}; diag {diag}")
    P(f"   c_q = t_q(median over steps)/1240 = {parts['phys_bp_query_ns']*1e6/1240:.1f} ns")
for Wx in (1, 8):
    sy = cell(vals('C4-JDap-armed', Wx, smed(SYS)))['m']
    wl = cell(vals('C4-JDap-armed', Wx, wall_mean_ab))['m']
    pbs = set(p['s']['config']['parallel_broadphase'] for p in used if p['row'] == 'C4-JDap-armed')
    P(f"JDap-armed W{Wx}: sys {sy/1e6:.4f} ms; armed wall [100,500) {wl/1e6:.4f}; share {100*sy/wl:.1f}%; parallel_broadphase {pbs}")

P("\n=== S5 ===")
judge_const('S5 t_q W8 mean-over-steps vs 0.2069', 'C4-JD-armed', 8, smean('phys_bp_query_ns'), 0.2069e6)
judge_const('S5 t_q W8 median-over-steps vs 0.2069', 'C4-JD-armed', 8, smed('phys_bp_query_ns'), 0.2069e6)
judge_const('S5 t_q W8 median-over-steps vs 0.20693 (F3 figure)', 'C4-JD-armed', 8, smed('phys_bp_query_ns'), 0.20693e6)
def sh1(p):
    return statistics.fmean(p['c']['phys_bp_query_ns'][A:B]) / statistics.fmean(p['c']['wall_ns'][A:B])
def sh2(p):
    return statistics.median(p['c']['phys_bp_query_ns'][A:B]) / statistics.fmean(p['c']['wall_ns'][A:B])
def sh3(p):
    return statistics.fmean(p['c']['phys_bp_query_ns'][A:B]) / statistics.fmean(p['c']['wall_ns'][0:500])
judge_const('S5 letter: mean t_q / mean armed wall [100,500) vs 0.05', 'C4-JD-armed', 8, sh1, 0.05, sc=1)
judge_const('S5 letter: median t_q / mean armed wall [100,500) vs 0.05', 'C4-JD-armed', 8, sh2, 0.05, sc=1)
judge_const('S5 letter: mean t_q / mean armed wall [0,500) vs 0.05', 'C4-JD-armed', 8, sh3, 0.05, sc=1)

P("\n=== post hoc: W8 vs W1 per stage on JD-armed (median over steps) ===")
for k in S4 + ('SUM4',):
    fn = sum4_med if k == 'SUM4' else smed(k)
    judge_pair(f'W8 vs W1 {k}', 'C4-JD-armed', 'C4-JD-armed', 1, fn, sc=1e3, WB=8)

P("\n=== post hoc 5: armed wall JD vs JDap ===")
judge_pair('armed wall W8 [100,500)', 'C4-JDap-armed', 'C4-JD-armed', 8, wall_mean_ab)
judge_pair('armed wall W8 [0,500)', 'C4-JDap-armed', 'C4-JD-armed', 8, wall_mean)
judge_pair('armed wall W1 [0,500)', 'C4-JDap-armed', 'C4-JD-armed', 1, wall_mean)

P("\n=== receipts of used processes ===")
for q in (0, 1, 2):
    u = [p for p in used if p['pass_'] == q]
    b0 = statistics.median(p['rec']['receipt_before']['cpu_avg'] for p in u)
    a0 = statistics.median(p['rec']['receipt_after']['cpu_avg'] for p in u)
    w0 = statistics.median(p['rec']['others_busy_pct'] for p in u)
    P(f"p{q}: before median {b0:.2f} after {a0:.2f} witness {w0:.2f}; start {min(p['rec']['start'] for p in u)} end {max(p['rec']['end'] for p in u)}")
pn = set(p['s']['profile_name'] for p in allp)
P(f"profile_name values: {pn}")
open(os.path.join(HERE, 'v_g5.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
