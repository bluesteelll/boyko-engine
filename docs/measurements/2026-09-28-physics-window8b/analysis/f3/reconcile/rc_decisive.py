"""Window 8b, F3 group: reconciliation of the analyst/verifier disputes D1-D3 (read-only on raw/).

Independent of lib_f3.py and verify_numbers/vlib.py: selection, validity and statistics are re-derived here.
Statistics are window 8's lib8.py definitions (cell = median over K; r = (max-min)/median; i = IQR/median with
'inclusive' linear quartiles; s = 1.2533*SD/sqrt(K)/median; B vs A clears x iff |B/A-1| > 2*hypot(x_A, x_B)).
The K<3 clause (window 7 wave 2 README "No claim when either side has K < 3"; lib8 cmp_ `ok = K_A >= 3 and K_B >= 3`)
is applied or lifted explicitly, per reading.
D1: K per pass-cell after the slot rule.  D2: R2 per pass and pooled, under each reading.  D3: attribution A vs B.
"""
import collections
import csv
import json
import math
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
RAW = os.path.join(W8B, 'raw')
SUMS = {}
for line in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        h, n = line.split(maxsplit=1)
        SUMS[n.strip().lstrip('*')] = h
TIP_SHA = SUMS['bin/runner_tip_16191fda.exe']
ROW_HASH = {'F3-RT': '0x6cbe24bf8fafda26'}  # every other F3 row: J500 (window_cmds.md section 3)
J_HASH = '0x30c5438bc6ad9ffa'


def row_hash(row):
    return ROW_HASH['F3-RT'] if row.startswith('F3-RT') else J_HASH


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


def summary(d):
    p = os.path.join(d, 'stdout.txt')
    if not os.path.exists(p):
        return None
    for line in open(p, encoding='utf-8', errors='replace'):
        if line.startswith('SUMMARY '):
            return json.loads(line[8:])
    return None


def read_col(d, col):
    with open(os.path.join(d, 'run.csv'), newline='', encoding='utf-8') as f:
        rd = csv.reader(f)
        head = [h.strip() for h in next(rd)]
        ix = head.index(col)
        ixs = head.index('step')
        return [(int(float(row[ixs])), float(row[ix])) for row in rd]


def valid_why(r):
    why = []
    if r.get('exit') != 0:
        why.append(f"exit {r.get('exit')}")
    if r.get('exe_sha256') != TIP_SHA:
        why.append('sha')
    s = summary(r['cwd'])
    if s is None:
        return why + ['no SUMMARY']
    if s.get('void_steps') != 0:
        why.append('void steps')
    if s.get('pose_hash') != row_hash(r['row']):
        why.append(f"pose {s.get('pose_hash')}")
    n = len(read_col(r['cwd'], 'wall_ns'))
    if n != 500:
        why.append(f'csv steps {n}')
    return why


recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]
f3 = [r for r in recs if r.get('block') == 'F3' and 'row' in r and not r.get('r4')]
passmarks = [r for r in recs if r.get('block') == 'F3' and (r.get('pass_done') or r.get('voided_pass'))]
timed = [r for r in f3 if r.get('attempt') != 'warmup']
print(f"F3 records: {len(f3)} processes ({len(timed)} timed: "
      f"{sum(r['attempt'] == 'original' for r in timed)} originals, {sum(r['attempt'] == 'rerun' for r in timed)} re-runs; "
      f"{len(f3) - len(timed)} warm-ups)")
print(f"F3 pass markers: pass_done {sum(1 for r in passmarks if r.get('pass_done'))}, "
      f"voided_pass {sum(1 for r in passmarks if r.get('voided_pass'))}")

slots = collections.defaultdict(dict)
for r in timed:
    slots[(r['pass'], r['round'], r['row'], r['W'])][r['attempt']] = r
used, dropped = {}, []
nvalid_bad = 0
for k, att in slots.items():
    pick = None
    for a in ('original', 'rerun'):
        if a in att:
            vw = valid_why(att[a])
            nvalid_bad += bool(vw)
            if not vw and not clean_why(att[a]):
                pick = att[a]
                break
    if pick is None:
        dropped.append(k)
    else:
        used[k] = pick
print(f'slots {len(slots)}, used {len(used)} ({sum(r["attempt"] == "rerun" for r in used.values())} by re-run), '
      f'dropped {len(dropped)}; attempts failing my validity re-derivation: {nvalid_bad}')

# ---------------- D1: K per pass-cell ----------------
print('\n== D1: K per pass-cell (row, W, pass) after the slot rule')
K = collections.Counter()
for (p, rnd, row, w) in slots:
    K[(row, w, p)] += 0
for (p, rnd, row, w) in used:
    K[(row, w, p)] += 1
dist = collections.Counter(K.values())
print(f'pass-cells {len(K)}; K distribution {dict(sorted(dist.items()))}')
k2 = sorted(k for k, v in K.items() if v < 3)
print(f'pass-cells with K < 3: {len(k2)}')
for row, w, p in k2:
    print(f'  {row:24s} W{w:<3d} p{p}  K={K[(row, w, p)]}')
drop_cells = collections.Counter((row, w, p) for (p, rnd, row, w) in dropped)
print(f'dropped slots {len(dropped)} fall in {len(drop_cells)} distinct pass-cells; max drops in one pass-cell '
      f'{max(drop_cells.values())}')
cellsN = collections.Counter()
for (row, w, p), v in K.items():
    cellsN[(row, w)] += v
print(f'cells {len(cellsN)}; cells with n < 9: {sum(1 for v in cellsN.values() if v < 9)}; '
      f'n values {dict(sorted(collections.Counter(cellsN.values()).items()))}')
# kd-vs-leaflist pass-pairs with K<3 on either side (the only other count a "pass-cell" figure could mean)
pairs = []
for (row, w, p), v in K.items():
    if row.endswith('-kd'):
        tw = row[:-3] + '-leaflist'
        if (tw, w, p) in K and (v < 3 or K[(tw, w, p)] < 3):
            pairs.append((row[:-3], w, p, K[(tw, w, p)], v))
print(f'kd/leaflist pass-pairs with K < 3 on either side: {len(pairs)}')
for fam, w, p, ka, kb in sorted(pairs):
    print(f'  {fam:20s} W{w:<3d} p{p}  K leaflist/kd {ka}/{kb}')


# ---------------- D2: R2 ----------------
def med_window(r, col, lo=100, hi=500):
    return statistics.median(v for s, v in read_col(r['cwd'], col) if lo <= s < hi)


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(xs)
    m = statistics.median(xs)
    sd = statistics.stdev(xs) if len(xs) >= 2 else 0.0
    return {'K': len(xs), 'm': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / m, 'i': iqr(xs) / m,
            's': 1.2533 * sd / math.sqrt(len(xs)) / m, 'v': xs}


def cmp_(a, b, kmin):
    ratio = b['m'] / a['m']
    e = abs(ratio - 1)
    ok = a['K'] >= kmin and b['K'] >= kmin
    out = {'ratio': ratio, 'sign': (ratio > 1) - (ratio < 1), 'KA': a['K'], 'KB': b['K']}
    for x in 'ris':
        out['bar_' + x] = 2 * math.hypot(a[x], b[x])
        out[x] = ok and e > out['bar_' + x]
    return out


def flags(c):
    return ''.join('Y' if c[x] else 'n' for x in 'ris')


print('\n== D2: R2 = t_q(kd) / t_q(leaflist) < 1 claimed, F3-TD-armed, W1 (t_q = per-process median over [100,500) '
      'of phys_bp_query_ns)')
vals = collections.defaultdict(list)
for (p, rnd, row, w), r in used.items():
    if row in ('F3-TD-armed-leaflist', 'F3-TD-armed-kd') and w == 1:
        vals[(row, p)].append((rnd, med_window(r, 'phys_bp_query_ns') / 1000.0, r['attempt']))
for row in ('F3-TD-armed-leaflist', 'F3-TD-armed-kd'):
    for p in (0, 1, 2):
        print(f'  {row:22s} p{p}: ' + ', '.join(f'r{rnd} {v:.3f} us ({a})' for rnd, v, a in sorted(vals[(row, p)])))
A = {p: cell([v for _, v, _ in vals[('F3-TD-armed-leaflist', p)]]) for p in (0, 1, 2)}
B = {p: cell([v for _, v, _ in vals[('F3-TD-armed-kd', p)]]) for p in (0, 1, 2)}
A['pooled'] = cell([v for p in (0, 1, 2) for _, v, _ in vals[('F3-TD-armed-leaflist', p)]])
B['pooled'] = cell([v for p in (0, 1, 2) for _, v, _ in vals[('F3-TD-armed-kd', p)]])
for key in (0, 1, 2, 'pooled'):
    a, b = A[key], B[key]
    c3, c2 = cmp_(a, b, 3), cmp_(a, b, 2)
    tag = key if key == 'pooled' else f'p{key}'
    print(f'  {tag:6s} A {a["m"]:.2f} [{a["min"]:.2f}-{a["max"]:.2f}] K={a["K"]} i {100 * a["i"]:.2f} s {100 * a["s"]:.2f} | '
          f'B {b["m"]:.2f} [{b["min"]:.2f}-{b["max"]:.2f}] K={b["K"]} i {100 * b["i"]:.2f} s {100 * b["s"]:.2f} | '
          f'B/A {c3["ratio"]:.4f} ({100 * (c3["ratio"] - 1):+.2f} %) bars 2r/2i/2s {100 * c3["bar_r"]:.2f}/'
          f'{100 * c3["bar_i"]:.2f}/{100 * c3["bar_s"]:.2f} % | flags r/i/s K>=3 rule {flags(c3)}, K>=2 {flags(c2)}')
print(f'  pooled delta {A["pooled"]["m"] - B["pooled"]["m"]:.2f} us less on kd; leaflist min {A["pooled"]["min"]:.2f} '
      f'vs kd max {B["pooled"]["max"]:.2f}')


def ruling1(kmin, gating):
    cs = [('pooled', cmp_(A['pooled'], B['pooled'], kmin))] + [(f'p{p}', cmp_(A[p], B[p], kmin)) for p in gating]
    d = cs[0][1]['sign']
    claimed = all(c['i'] and c['s'] and c['sign'] == d for _, c in cs)
    strong = claimed and all(c['r'] and c['sign'] == d for _, c in cs)
    return ('STRONG CLAIMED' if strong else 'CLAIMED' if claimed else 'NOT CLAIMED'), \
        ' '.join(f'{n} {flags(c)}' for n, c in cs)


voided = {r['pass'] for r in passmarks if r.get('voided_pass')}
clean_passes = [p for p in (0, 1, 2) if p not in voided]
full_passes = [p for p in clean_passes if A[p]['K'] >= 3 and B[p]['K'] >= 3]
print(f'  clean (not voided) F3 passes: {clean_passes}; passes where both R2 cells kept 3 slots: {full_passes}')
for name, kmin, gating in (('LETTER (every non-voided pass gates; no flag at K<3)', 3, clean_passes),
                           ('K2 (every non-voided pass gates; a K=2 pass-cell judged on its own i and s)', 2, clean_passes),
                           ("analyst's CLEAN (only passes with K=3 on both sides gate)", 3, full_passes)):
    v, f = ruling1(kmin, gating)
    print(f'  R2 under {name}: {v}   [{f}]')

# ---------------- D3: attribution A vs B (post hoc; no pre-registered rule) ----------------
print('\n== D3: attribution A (proportional) vs B (fixed residual), J, W1 -- post hoc, no rule')
NQ = 1240
cq_a, cq_b = 1000 * A['pooled']['m'] / NQ, 1000 * B['pooled']['m'] / NQ
ratio = B['pooled']['m'] / A['pooled']['m']
dtq = B['pooled']['m'] - A['pooled']['m']
print(f'observed: c_q leaflist {cq_a:.2f} ns, kd {cq_b:.2f} ns; ratio {ratio:.4f}; dt_q {dtq:+.2f} us')
# the model (tree-f3/f3_arith.txt, cut.md 3.6): base 167.6 ns; model ratio from A's printed band; B's modelled part
BASE_M = 167.6
R_LO, R_HI = 121.6 / BASE_M, 135.5 / BASE_M
M_LO, M_HI = 122.0, 130.0
print(f'model: base {BASE_M} ns, kd/F1+F2 ratio {R_LO:.4f}..{R_HI:.4f} (from A 121.6..135.5), B modelled part '
      f'{M_LO:.0f}..{M_HI:.0f} ns')


def bands(base):
    a_cq = (base * R_LO, base * R_HI)
    a_rt = (R_LO, R_HI)
    a_dt = (-(1 - R_LO) * base * NQ / 1000, -(1 - R_HI) * base * NQ / 1000)
    b_cq = (base - M_HI * (1 - R_LO), base - M_LO * (1 - R_HI))
    b_rt = (b_cq[0] / base, b_cq[1] / base)
    b_dt = (-M_HI * (1 - R_LO) * NQ / 1000, -M_LO * (1 - R_HI) * NQ / 1000)
    return {'A': (a_cq, a_rt, a_dt), 'B': (b_cq, b_rt, b_dt)}


def inside(x, lohi):
    lo, hi = min(lohi), max(lohi)
    return 'IN ' if lo <= x <= hi else 'out'


for label, base in (('model base 167.6 ns (cut.md table, as printed)', BASE_M),
                    (f'this window\'s leaflist base {cq_a:.2f} ns (same attribution rule applied)', cq_a)):
    bd = bands(base)
    print(f'-- {label}')
    for att in ('A', 'B'):
        cq, rt, dt = bd[att]
        print(f'   {att}: c_q {min(cq):.1f}..{max(cq):.1f} ns [{inside(cq_b, cq)}]  ratio {min(rt):.4f}..{max(rt):.4f} '
              f'[{inside(ratio, rt)}]  dt_q {min(dt):.1f}..{max(dt):.1f} us [{inside(dtq, dt)}]')
print(f'base shift window 8b vs model: {100 * (cq_a / BASE_M - 1):+.2f} % (leaflist c_q {cq_a:.2f} vs {BASE_M})')
print(f'observed ratio vs A upper edge {R_HI:.4f}: {100 * (ratio / R_HI - 1):+.2f} %; pooled 2i bar '
      f'{100 * cmp_(A["pooled"], B["pooled"], 3)["bar_i"]:.2f} %, 2s bar {100 * cmp_(A["pooled"], B["pooled"], 3)["bar_s"]:.2f} %')
