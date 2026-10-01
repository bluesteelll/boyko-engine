"""Independent verifier (numbers lens): J-Son-T block of window 8b, recomputed from raw/ only.
Written from ruling 1 + the window 8 analysis.md definitions; imports nothing from the analyst or the driver tools.
cell = median over K of per-process means (ms/step) of run.csv wall_ns over [a,b) (CSV row index = step).
r = (max-min)/median; i = IQR (inclusive quartiles)/median; s = 1.2533*SD/sqrt(K)/median.
bar_k = 2*hypot(A_k, B_k); cl_k iff |B/A-1| > bar_k.  CLAIMED iff i AND s clear in every pass AND pooled.
STRONG iff also r clears in every pass and pooled. Guard variant: lib8.cmp_ (window 8) refuses K<3."""
import csv, json, math, os, statistics

W8B = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', '..'))
RAW = os.path.join(W8B, 'raw')
FIX = json.load(open(os.path.join(W8B, 'gate', 'fixtures', 'fixtures.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))['rows']}
SUMS = {}
for ln in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if ln.strip():
        h, n = ln.split(maxsplit=1)
        SUMS[n.strip().lstrip('*')] = h
TIP_SHA = SUMS['bin/runner_tip_16191fda.exe']
OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


def summary(d):
    for ln in open(os.path.join(d, 'stdout.txt'), encoding='utf-8', errors='replace'):
        if ln.startswith('SUMMARY '):
            return json.loads(ln[8:])
    return None


def load_csv(d):
    with open(os.path.join(d, 'run.csv'), newline='') as f:
        rd = csv.reader(f)
        h = next(rd)
        rows = list(rd)
    return h, rows


def colmean(h, rows, name, a, b):
    j = h.index(name)
    xs = [float(r[j]) for r in rows[a:b] if r[j] != '']
    return sum(xs) / len(xs)


def validate(r):
    why = []
    row = ROWS[r['row']]
    d = r['cwd']
    if r.get('exit') != 0:
        why.append('exit')
    if r.get('exe_sha256') != TIP_SHA or r.get('binary') != 'tip':
        why.append('sha/binary')
    s = summary(d)
    if s is None:
        return why + ['no SUMMARY'], None, None, None
    h, rows = load_csv(d)
    if len(rows) != 500 or [int(x[0]) for x in rows] != list(range(500)):
        why.append('csv steps')
    if s.get('void_steps') != 0:
        why.append('void')
    if s.get('pose_hash') != FIX[row['pose_ref']]['hash'] or s.get('expect_pose') != 'match':
        why.append('pose')
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if bool(s.get('armed')) != bool(row['armed']):
        why.append('armed flag')
    if row['armed']:
        if s.get('drops_total') != 0:
            why.append('drops')
        if not s.get('w8s') or s['w8s'].get('overflow') != 0:
            why.append('w8s')
    else:
        if s.get('disarmed_ring_traffic') != 0:
            why.append('ring')
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != 'Tree':
        why.append('bp')
    if s.get('cfg') != 'a':
        why.append('cfg')
    sl = row['args'][row['args'].index('--sleeping') + 1] == 'on'
    if bool(cfg.get('sleeping')) != sl:
        why.append('sleeping flag')
    bt = s.get('broadphase_tree') or {}
    for k, v in row['tree_diag_expect'].items():
        if bt.get(k) != v:
            why.append(f'treediag {k}={bt.get(k)}')
    aw = [(int(x[h.index('awake')]) if x[h.index('awake')] != '' else None) for x in rows]
    if sl:
        if s.get('first_frozen_step') != 274:
            why.append(f"freeze {s.get('first_frozen_step')}")
        if aw.index(0) != 273 or any(v != 0 for v in aw[273:]):
            why.append('awake csv')
    else:
        if s.get('first_frozen_step') is not None or 0 in aw:
            why.append('freeze on OFF')
    if abs(colmean(h, rows, 'wall_ns', 0, 500) - s['window_mean_ns']) > 1.0:
        why.append('window mean')
    return why, s, h, rows


def clean(r):
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    if rb is None or rb > 5.0:
        why.append(f'before {rb}')
    if ra is None or ra > 5.0:
        why.append(f'after {ra}')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append(f'witness {ob}')
    if r.get('build_proc_during'):
        why.append('build')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append('presence')
    return why


recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
js = [r for r in recs if r.get('block') == 'J-Son-T' and r.get('attempt') in ('original', 'rerun')]
P(f'# J-Son-T verifier: {len(js)} timed records (originals {sum(r["attempt"] == "original" for r in js)}, '
  f're-runs {sum(r["attempt"] == "rerun" for r in js)})')
WINS = {'0..100': (0, 100), '100..500': (100, 500), '274..500': (274, 500), '277..500': (277, 500)}
L10C = ('phys_sleep_begin_ns', 'phys_sleep_freeze_ns', 'phys_sleep_end_ns', 'phys_sleep_classify_ns')
procs = []
nbad = 0
for r in js:
    why, s, h, rows = validate(r)
    cw = clean(r)
    if why:
        nbad += 1
        P(f'INVALID {r["pass"]} {r["round"]} {r["row"]} W{r["W"]} {r["attempt"]}: {why}')
    p = dict(pass_=r['pass'], round=r['round'], row=r['row'], W=r['W'], attempt=r['attempt'], seq=r['seq'],
             valid=not why, clean=not cw, clean_why=cw,
             witness_top=[o['name'] for o in (r.get('others_top5') or [])[:1]],
             freeze=(s or {}).get('first_frozen_step'), driver_mean=r.get('mean_ms'))
    if h is not None:
        p['wall'] = {wn: colmean(h, rows, 'wall_ns', a, b) / 1e6 for wn, (a, b) in WINS.items()}
        p['wall500'] = colmean(h, rows, 'wall_ns', 0, 500) / 1e6
        if r['row'] == 'JSonT-a':
            p['l10'] = {wn: sum(colmean(h, rows, c, a, b) for c in L10C) / 1e3 for wn, (a, b) in WINS.items()}
    procs.append(p)
P(f'validity problems: {nbad}')
P(f'driver mean_ms vs my [0,500) mean: max |diff| = {max(abs(p["driver_mean"] - p["wall500"]) for p in procs):.3e} ms')
unclean = [p for p in procs if not p['clean']]
P(f'unclean processes: {len(unclean)}; witness top among them: {sorted(set(tuple(p["witness_top"]) for p in unclean))}')
slots = {}
for p in procs:
    slots.setdefault((p['pass_'], p['round'], p['row'], p['W']), []).append(p)
used, dropped = [], []
for k, ps in sorted(slots.items()):
    cands = [x for x in ps if x['attempt'] == 'original'] + [x for x in ps if x['attempt'] == 'rerun']
    pick = next((c for c in cands if c['valid'] and c['clean']), None)
    if pick:
        used.append(pick)
    else:
        dropped.append((k, [(x['attempt'], x['clean_why']) for x in ps]))
P(f'slots {len(slots)}, used {len(used)}, re-runs used {sum(p["attempt"] == "rerun" for p in used)}, dropped {len(dropped)}')
for d in dropped:
    P(f'  dropped slot {d}')
P(f'freeze step over used ON processes: {sorted(set(p["freeze"] for p in used if p["row"].startswith("JSonT")))}; '
  f'over ALL ON records: {sorted(set(p["freeze"] for p in procs if p["row"].startswith("JSonT")))}')


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(xs)
    m = statistics.median(xs)
    sd = statistics.stdev(xs) if len(xs) >= 2 else 0.0
    return dict(K=len(xs), med=m, mn=xs[0], mx=xs[-1], r=(xs[-1] - xs[0]) / m, i=iqr(xs) / m,
                s=1.2533 * sd / math.sqrt(len(xs)) / m, xs=xs)


def cmp(a, b, guard):
    ratio = b['med'] / a['med']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'ris'}
    ok = (a['K'] >= 3 and b['K'] >= 3) if guard else True
    return dict(ratio=ratio, e=e, bars=bars, cl={k: ok and e > bars[k] for k in 'ris'})


def sel(row, W, pas=None, win='0..100'):
    return [p['wall'][win] for p in used if p['row'] == row and p['W'] == W and (pas is None or p['pass_'] == pas)]


def yn(c):
    return ''.join('Y' if c['cl'][k] else 'n' for k in 'ris')


RESULT = {}
for armed in ('', '-a'):
    for W in (1, 8):
        for win, tag in (('0..100', 'AW'), ('100..500', 'G'), ('274..500', 'F'), ('277..500', 'F277posthoc')):
            key = f'JST-{tag}-W{W}{armed}'
            P(f'\n## {key}: JSoffT{armed} -> JSonT{armed}, W{W}, [{win})')
            per = {}
            for pas in (0, 1, 2, None):
                a = cell(sel('JSoffT' + armed, W, pas, win))
                b = cell(sel('JSonT' + armed, W, pas, win))
                c0 = cmp(a, b, False)
                c1 = cmp(a, b, True)
                lab = 'pooled' if pas is None else f'p{pas}'
                per[lab] = (a, b, c0, c1)
                P(f'  {lab}: OFF {a["med"]:.4f} K={a["K"]} (r {100 * a["r"]:.2f} i {100 * a["i"]:.2f} s {100 * a["s"]:.2f}) | '
                  f'ON {b["med"]:.4f} K={b["K"]} (r {100 * b["r"]:.2f} i {100 * b["i"]:.2f} s {100 * b["s"]:.2f}) | '
                  f'{100 * (c0["ratio"] - 1):+.2f} % bars r {100 * c0["bars"]["r"]:.2f} i {100 * c0["bars"]["i"]:.2f} '
                  f's {100 * c0["bars"]["s"]:.2f} | r/i/s {yn(c0)} (K>=3 guard {yn(c1)})')
            for g, gi in (('noguard', 2), ('guard', 3)):
                claimed = all(per[l][gi]['cl']['i'] and per[l][gi]['cl']['s'] for l in per)
                strong = claimed and all(per[l][gi]['cl']['r'] for l in per)
                sign = set((per[l][gi]['ratio'] > 1) for l in per)
                v = ('CLAIMED STRONG' if strong else 'CLAIMED') if claimed else 'NOT CLAIMED'
                if claimed and len(sign) > 1:
                    v += ' (MIXED SIGN)'
                if claimed:
                    v += ' (ON slower)' if per['pooled'][gi]['ratio'] > 1 else ' (ON faster)'
                RESULT.setdefault(key, {})[g] = v
                P(f'  verdict [{g}]: {v}')
            mc = max(max(per[l][2]['bars']['i'], per[l][2]['bars']['s']) for l in per)
            P(f'  smallest claimable |effect| (max over passes+pooled of max(bar_i, bar_s), no guard): {100 * mc:.2f} %')
            RESULT[key]['pooled'] = dict(off=per['pooled'][0]['med'], on=per['pooled'][1]['med'],
                                         delta_pct=100 * (per['pooled'][2]['ratio'] - 1),
                                         Koff=per['pooled'][0]['K'], Kon=per['pooled'][1]['K'], min_claimable_pct=100 * mc)
P('\n## post hoc: spread of the [0,100) cells (pooled), % of median')
for row in ('JSoffT', 'JSonT', 'JSoffT-a', 'JSonT-a'):
    for W in (1, 8):
        c = cell(sel(row, W, None, '0..100'))
        P(f'  {row} W{W}: i {100 * c["i"]:.2f} % r {100 * c["r"]:.2f} % K {c["K"]}')
P('\n## L10 serial pieces (JSonT-a, begin+freeze+end+classify, us/step), median over K of per-process means')
for W in (1, 8):
    for win in ('0..100', '100..500'):
        xs = [p['l10'][win] for p in used if p['row'] == 'JSonT-a' and p['W'] == W]
        m = statistics.median(xs)
        stepon = statistics.median(sel('JSonT-a', W, None, win))
        stepoff = statistics.median(sel('JSoffT', W, None, win))
        P(f'  W{W} [{win}): {m:.2f} us/step (K {len(xs)}) = {100 * m / 1e3 / stepon:.3f} % of the armed ON step, '
          f'{100 * m / 1e3 / stepoff:.3f} % of the disarmed OFF step')
P('\n## derived: frozen-pile cost on the tree ([274,500) ON / OFF, disarmed pooled)')
for W in (1, 8):
    on = statistics.median(sel('JSonT', W, None, '274..500'))
    off = statistics.median(sel('JSoffT', W, None, '274..500'))
    P(f'  W{W}: ON {on:.4f} ms = {100 * on / off:.2f} % of OFF [274,500) {off:.4f}')
HERE = os.path.dirname(os.path.abspath(__file__))
json.dump(dict(result=RESULT, used=used), open(os.path.join(HERE, 'vjs.json'), 'w'), indent=1)
open(os.path.join(HERE, 'vjs.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
