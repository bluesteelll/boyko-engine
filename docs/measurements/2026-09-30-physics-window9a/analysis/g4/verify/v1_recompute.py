"""Independent verification of window 9a C4-G4 + C4-G4-kd (adversarial verifier, 2026-10-01).
Written from raw/ and the rulings ONLY; does not import the analyst's lib_g4.py / lib9a.py.
Read-only on raw/, rows9a*.json, bin/."""
import hashlib, json, math, os, re, statistics, sys
from collections import defaultdict
sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W = os.path.abspath(os.path.join(HERE, '..', '..', '..'))
RAW = os.path.join(W, 'raw')
OUT = []
def P(s=''):
    OUT.append(str(s)); print(s)

R = json.load(open(os.path.join(W, 'rows9a.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in R['rows']}
PIN = R['binaries']['g4rT']['sha256_pin']
SUMS = {}
for l in open(os.path.join(W, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if l.strip():
        a, b = l.split(maxsplit=1); SUMS[b.strip().lstrip('*')] = a
exe_sha = hashlib.sha256(open(os.path.join(W, 'bin', 'broadphase_g4rT_50e31f1a.exe'), 'rb').read()).hexdigest()
P(f'g4rT pin {PIN}; SHA256SUMS {SUMS.get("bin/broadphase_g4rT_50e31f1a.exe")}; file on disk {exe_sha}; equal {PIN == exe_sha == SUMS.get("bin/broadphase_g4rT_50e31f1a.exe")}')

recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]
allg = [r for r in recs if (r.get('block') or '').startswith('C4-G4')]
P(f'records with block C4-G4*: {len(allg)}; run tags {sorted({r.get("run_tag") for r in allg})}')
done = {(r['run_tag'], r['block'], r['pass'], r['pass_attempt']) for r in recs if r.get('pass_done')}
P(f'pass_done for G4*: {sorted(k for k in done if k[1].startswith("C4-G4"))}')
voided = [r for r in recs if r.get('voided_pass') and (r.get('block') or '').startswith('C4-G4')]
P(f'voided_pass records for G4*: {len(voided)}')
procs = [r for r in allg if 'row' in r]
P(f'process records: {len(procs)}; attempts {sorted({r["attempt"] for r in procs})}; timed {sorted({r.get("timed") for r in procs})}')
pcs = [r for r in allg if r.get('passcell')]
for r in pcs:
    P(f'  passcell {r["block"]} p{r["pass"]}: k_clean {r["k_clean"]}/{r["k_target"]} reruns {r["reruns"]} gates {r["gates"]}')

TIME_RE = re.compile(r'^(bp_g4_\S+)\s*\n?\s*time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]', re.M)
UNITS = {'ns': 1e-3, 'us': 1.0, 'ms': 1e3, 's': 1e6, 'ps': 1e-6}
def to_us(v, u):
    if u not in UNITS and len(u) == 2 and u.endswith('s'):
        return float(v)  # the micro sign
    return float(v) * UNITS[u]
KRE = re.compile(r'^(bp_g4_\w+)/(\w+): kernel (\w+) rows (\d+) pairs (\d+) tree members (\d+) .*kd_order_builds (\d+)\s*$', re.M)

def clean_reasons(r):
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
    return why

def load(r):
    d = r['cwd']
    row = ROWS[r['row']]
    need = sorted(row['expect'])
    why = []
    if r.get('exit') != 0: why.append('exit')
    if r.get('hang'): why.append('hang')
    if r.get('exe_sha256') != PIN or exe_sha != PIN: why.append('sha')
    if not r['exe'].replace(chr(92), '/').endswith('bin/broadphase_g4rT_50e31f1a.exe'): why.append('exe path')
    if r['args'] != row['args']: why.append('args')
    so = open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace').read()
    se = open(os.path.join(d, 'stderr.txt'), encoding='utf-8', errors='replace').read()
    printed = defaultdict(list)
    for m in TIME_RE.finditer(so):
        printed[m.group(1)].append(to_us(m.group(4), m.group(5)))
    if 'change:' in so: why.append('stdout has change: lines (stale baseline)')
    ondisk = set(); vals = {}; med = {}; basediff = 0
    crit = os.path.join(d, 'criterion')
    for root, dirs, files in os.walk(crit):
        if os.path.basename(root) == 'new' and 'benchmark.json' in files:
            fid = json.load(open(os.path.join(root, 'benchmark.json'), encoding='utf-8'))['full_id']
            ondisk.add(fid)
            s = json.load(open(os.path.join(root, 'sample.json'), encoding='utf-8'))
            if s.get('sampling_mode') != 'Flat' or len(s['times']) != 10 or len(s['iters']) != 10:
                why.append(f'{fid} sampling')
            per = [t / n for t, n in zip(s['times'], s['iters'])]
            vals[fid] = sum(per) / len(per) / 1e3
            med[fid] = statistics.median(per) / 1e3
            est = json.load(open(os.path.join(root, 'estimates.json'), encoding='utf-8'))
            if est.get('slope') is not None: why.append(f'{fid} slope not null')
            bp = os.path.join(os.path.dirname(root), 'base', 'sample.json')
            if os.path.exists(bp) and open(bp, 'rb').read() != open(os.path.join(root, 'sample.json'), 'rb').read():
                basediff += 1
    miss = [i for i in need if i not in vals or i not in printed]
    extra = sorted((set(printed) | ondisk) - set(need))
    dup = [i for i, v in printed.items() if len(v) != 1]
    if miss: why.append(f'missing {miss}')
    if extra: why.append(f'extra {extra}')
    if dup: why.append(f'dup {dup}')
    worst_pr = max(abs(printed[i][0] / vals[i] - 1) for i in vals if i in printed) if vals else None
    em = (r.get('criterion') or {}).get('est_ms') or {}
    worst_drv = max((abs(em[i][1] * 1e3 / vals[i] - 1) for i in vals if i in em), default=None)
    kr = defaultdict(dict)
    for fam, n, kern, rows_, pairs, members, kdb in KRE.findall(se):
        kr[(fam, int(n) if n.isdigit() else n)][kern] = (int(rows_), int(pairs), int(members), int(kdb))
    kbad = []
    for k, v in kr.items():
        if set(v) != {'LeafList', 'RowWalk', 'LeafListKd'}:
            kbad.append((k, 'kernels')); continue
        if len({x[1] for x in v.values()}) != 1: kbad.append((k, 'pairs'))
        if v['LeafListKd'][3] == 0 or v['LeafList'][3] != 0 or v['RowWalk'][3] != 0: kbad.append((k, 'kd builds'))
        if k[0] != 'bp_g4_scene' and any(x[2] != 0 for x in v.values()): kbad.append((k, 'members'))
    if kbad: why.append(f'kernel receipts {kbad[:3]}')
    return {'r': r, 'valid_why': why, 'clean_why': clean_reasons(r), 'v': vals, 'med': med, 'printed': {k: v[0] for k, v in printed.items()},
            'worst_pr': worst_pr, 'worst_drv': worst_drv, 'kr': kr, 'basediff': basediff, 'oracle_ok': se.count('oracle ok'),
            'driver_valid': r.get('valid'), 'driver_contam': r.get('contaminated')}

P('\n## Per-process validity / cleanliness')
L = {}
for r in procs:
    key = (r['run_tag'], r['block'], r['pass'], r['pass_attempt'])
    x = load(r)
    x['closed'] = key in done
    L[(r['block'], r['pass'], r['seq'], r['attempt'], r.get('rerun_no', 0))] = x
    P(f"- {r['block']} p{r['pass']} r{r['round']} seq{r['seq']} {r['attempt']}: closed {x['closed']} valid {not x['valid_why']} {x['valid_why']} clean {not x['clean_why']} {x['clean_why']} "
      f"| driver valid {x['driver_valid']} contaminated {x['driver_contam']} | ids {len(x['v'])} printed-vs-recomp {x['worst_pr']:.2e} drv-vs-recomp {x['worst_drv']:.2e} "
      f"| receipts {(r['receipt_before'] or {}).get('cpu_avg')}/{(r['receipt_after'] or {}).get('cpu_avg')} witness {r['others_busy_pct']} | kernel keys {len(x['kr'])} base!=new {x['basediff']} oracle-ok {x['oracle_ok']} | {r['start'][11:19]}-{r['end'][11:19]}")

def used_of(block):
    slots = defaultdict(list)
    for k, x in L.items():
        if k[0] == block and x['closed']:
            slots[(k[1], k[2])].append(x)
    used = {}
    for s, xs in sorted(slots.items()):
        orig = [x for x in xs if x['r']['attempt'] == 'original']
        rer = sorted([x for x in xs if x['r']['attempt'] == 'rerun'], key=lambda x: x['r'].get('rerun_no', 0))
        pick = next((x for x in orig + rer if not x['valid_why'] and not x['clean_why']), None)
        if pick: used[s] = pick
    return used, slots
uG, sG = used_of('C4-G4'); uK, sK = used_of('C4-G4-kd')
P(f'\nslot rule: C4-G4 slots {len(sG)} used {len(uG)}; C4-G4-kd slots {len(sK)} used {len(uK)}')
for blk, u in (('C4-G4', uG), ('C4-G4-kd', uK)):
    byp = defaultdict(int)
    for (p, s) in u: byp[p] += 1
    P(f'  {blk} K per pass {dict(byp)}')

# ---- statistics, written from the ruling text ----
def q_incl(xs):
    xs = sorted(xs); n = len(xs)
    def q(p):
        h = (n - 1) * p; lo = math.floor(h); hi = min(lo + 1, n - 1)
        return xs[lo] + (h - lo) * (xs[hi] - xs[lo])
    return q(0.25), q(0.75)
def cell(xs):
    xs = sorted(xs); n = len(xs); m = statistics.median(xs)
    q1, q3 = q_incl(xs) if n >= 2 else (m, m)
    sd = statistics.stdev(xs) if n >= 2 else 0.0
    return {'n': n, 'med': m, 'min': xs[0], 'max': xs[-1], 'i': (q3 - q1) / m, 's': 1.2533 * sd / math.sqrt(n) / m, 'r': (xs[-1] - xs[0]) / m, 'xs': xs}
def cmp(a, b):
    """B against A."""
    ratio = b['med'] / a['med']; e = abs(ratio - 1)
    ok = a['n'] >= 3 and b['n'] >= 3
    f = {x: ok and e > 2 * math.hypot(a[x], b[x]) for x in 'isr'}
    bars = {x: 2 * math.hypot(a[x], b[x]) for x in 'isr'}
    sep = b['max'] < a['min'] or b['min'] > a['max']
    return {'ratio': ratio, 'f': f, 'bars': bars, 'sep': sep, 'nA': a['n'], 'nB': b['n']}
def yn(c, keys='isr'):
    return ''.join('Y' if c['f'][k] else 'n' for k in keys)
def r1(pooled, per):
    sg = pooled['ratio'] > 1
    same = all((c['ratio'] > 1) == sg for c in per)
    cl = pooled['f']['i'] and pooled['f']['s'] and all(c['f']['i'] and c['f']['s'] for c in per) and same and len(per) == 3
    st = cl and pooled['f']['r'] and all(c['f']['r'] for c in per)
    return cl, st
def w7(pooled, per):
    sg = pooled['ratio'] > 1
    same = all((c['ratio'] > 1) == sg for c in per)
    return pooled['f']['r'] and pooled['f']['s'] and all(c['f']['r'] and c['f']['s'] for c in per) and same
def vals(u, i, stat='v', passes=(0, 1, 2)):
    return {p: [x[stat][i] for (pp, s), x in sorted(u.items()) if pp == p] for p in passes}

SIZES = list(range(96, 161, 8))
RES = {}
for stat in ('v', 'med', 'printed'):
    P(f'\n## all_pairs/tree under ruling 1, per-process statistic = {stat} (A = tree, B = all_pairs)')
    RES[stat] = {}
    for fam in ('uniform', 'disparity'):
        RES[stat][fam] = {}
        for n in SIZES:
            ia, it = f'bp_g4_{fam}/all_pairs/{n}', f'bp_g4_{fam}/tree/{n}'
            va, vt = vals(uG, ia, stat), vals(uG, it, stat)
            pa = cell(sum(va.values(), [])); pt = cell(sum(vt.values(), []))
            pooled = cmp(pt, pa)
            per = [cmp(cell(vt[p]), cell(va[p])) for p in (0, 1, 2)]
            cl, st = r1(pooled, per)
            pooled2 = cmp(pa, pt); per2 = [cmp(cell(va[p]), cell(vt[p])) for p in (0, 1, 2)]
            cl2, st2 = r1(pooled2, per2)
            wcl = w7(pooled, per)
            RES[stat][fam][n] = {'cl': cl, 'st': st, 'ratio': pooled['ratio'], 'cl_swapped': cl2, 'w7': wcl, 'pa': pa, 'pt': pt, 'pooled': pooled, 'per': per}
            if stat == 'v':
                P(f"- {fam} {n}: all_pairs {pa['med']:.3f} [{pa['min']:.3f}-{pa['max']:.3f}] i {100*pa['i']:.2f} s {100*pa['s']:.2f} n={pa['n']} | "
                  f"tree {pt['med']:.3f} [{pt['min']:.3f}-{pt['max']:.3f}] i {100*pt['i']:.2f} s {100*pt['s']:.2f} n={pt['n']} | ratio {pooled['ratio']:.4f} ({pa['med']-pt['med']:+.3f} us) "
                  f"bars {100*pooled['bars']['i']:.2f}/{100*pooled['bars']['s']:.2f}/{100*pooled['bars']['r']:.2f} | pooled {yn(pooled)} "
                  + ' '.join(f"p{p}:{yn(c)}({c['ratio']:.4f},n{c['nA']}/{c['nB']})" for p, c in enumerate(per))
                  + f" | R1 {'STRONG ' if st else ''}{'CLAIMED' if cl else 'NOT CLAIMED'} {'ap slower' if pooled['ratio'] > 1 else 'ap faster'} | swapped {cl2}/{st2} | w7 {wcl} | sep {pooled['sep']}")
            else:
                P(f"- {fam} {n}: ratio {pooled['ratio']:.4f} pooled {yn(pooled)} " + ' '.join(f"p{p}:{yn(c)}" for p, c in enumerate(per)) + f" R1 {'STRONG ' if st else ''}{'CLAIMED' if cl else 'NOT CLAIMED'}")

def recipe(fr, key='cl'):
    slower = {n: fr[n][key] and fr[n]['ratio'] > 1 for n in SIZES}
    lo = max((n for n in SIZES if not slower[n]), default=None)
    hi = min((n for n in SIZES if slower[n]), default=None)
    mono = hi is not None and all(slower[n] == (n >= hi) for n in SIZES)
    return lo, hi, mono
P('\n## The recipe')
for stat in ('v', 'med', 'printed'):
    out = {}
    for fam in ('uniform', 'disparity'):
        out[fam] = {'r1': recipe(RES[stat][fam]), 'swapped': recipe(RES[stat][fam], 'cl_swapped'), 'w7': recipe(RES[stat][fam], 'w7')}
    for rule in ('r1', 'swapped', 'w7'):
        los = [out[f][rule][0] for f in out]; his = [out[f][rule][1] for f in out]
        edge = []
        for f in out:
            lo, hi, _ = out[f][rule]
            if lo is None: edge.append(f'{f}: LO below 96 (bottom edge)')
            if lo is not None and lo >= 160: edge.append(f'{f}: LO at or above 160 (top edge)')
            if hi is None: edge.append(f'{f}: no HI (top edge)')
        T = min(los) if None not in los else None
        AUTO = (T, max(his)) if (None not in his and T is not None) else None
        P(f"- stat {stat}, rule {rule}: " + '; '.join(f"{f} LO {out[f][rule][0]} HI {out[f][rule][1]} monotone {out[f][rule][2]}" for f in out)
          + f" => T {T}, AUTO {AUTO}; edges {edge or 'none'}")

P('\n## log-log crossover (8b g4.py method, label n)')
for fam in ('uniform', 'disparity'):
    fr = RES['v'][fam]; cross = None
    for n0, n1 in zip(SIZES, SIZES[1:]):
        if fr[n0]['ratio'] <= 1.0 and fr[n1]['ratio'] > 1.0:
            y0, y1 = math.log(fr[n0]['ratio']), math.log(fr[n1]['ratio'])
            cross = math.exp(math.log(n0) - y0 * (math.log(n1) - math.log(n0)) / (y1 - y0)); break
    P(f'- {fam}: {cross}')

P('\n## Paired per-process all_pairs/tree (post hoc)')
for fam in ('uniform', 'disparity'):
    for n in (120, 128, 136):
        rs = [x['v'][f'bp_g4_{fam}/all_pairs/{n}'] / x['v'][f'bp_g4_{fam}/tree/{n}'] for s, x in sorted(uG.items())]
        nb = len([v for v in rs if v < 1.0])
        P(f"- {fam} {n}: " + ' '.join(f'{v:.4f}' for v in rs) + f"  (min {min(rs):.4f} max {max(rs):.4f}, below one: {nb}/9)")

P('\n## tree_kd (C4-G4-kd, own K 3, not gating; cross-block)')
for fam in ('uniform', 'disparity'):
    for n in (96, 112, 128):
        ik, it, ia = f'bp_g4_{fam}/tree_kd/{n}', f'bp_g4_{fam}/tree/{n}', f'bp_g4_{fam}/all_pairs/{n}'
        k = cell([x['v'][ik] for x in uK.values()])
        t9 = cell(sum(vals(uG, it).values(), [])); a9 = cell(sum(vals(uG, ia).values(), []))
        t3 = cell(vals(uG, it)[2])
        c9 = cmp(t9, k); c3 = cmp(t3, k); ca = cmp(a9, k)
        P(f"- {fam} {n}: kd {k['med']:.3f} [{k['min']:.3f}-{k['max']:.3f}] i {100*k['i']:.2f} s {100*k['s']:.2f} n={k['n']} | kd/tree X9 {c9['ratio']:.4f} ({k['med']-t9['med']:+.3f} us) {yn(c9)} | "
          f"kd/tree X3 {c3['ratio']:.4f} {yn(c3)} | kd/all_pairs {ca['ratio']:.4f} {yn(ca)} ({100*(ca['ratio']-1):+.1f} pct)")

P('\n## rows per label (kernel receipts, first G4 process)')
x0 = next(iter(uG.values()))
for fam in ('uniform', 'disparity'):
    parts = []
    for n in SIZES:
        kk = (f'bp_g4_{fam}', n)
        if kk in x0['kr']:
            parts.append(f"{n}->{x0['kr'][kk]['LeafList'][0]} rows/{x0['kr'][kk]['LeafList'][1]} pairs")
    P(f"- {fam}: " + ', '.join(parts))
inc = 0
for x in list(uG.values()) + list(uK.values()):
    for k, v in x['kr'].items():
        if v != x0['kr'].get(k): inc += 1
P(f'kernel receipt tuples differing from the first process: {inc}; keys per process {sorted({len(x["kr"]) for x in list(uG.values()) + list(uK.values())})}')

P('\n## per-pass medians (drift, post hoc)')
for fam in ('uniform', 'disparity'):
    for arm in ('all_pairs', 'tree'):
        for n in (128, 136):
            i = f'bp_g4_{fam}/{arm}/{n}'
            ms = [statistics.median(vals(uG, i)[p]) for p in (0, 1, 2)]
            P(f'- {i}: ' + ' '.join(f'{m:.3f}' for m in ms) + f'  spread {100*(max(ms)-min(ms))/min(ms):.2f} pct')
open(os.path.join(HERE, 'v1_recompute.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
