"""Independent validity + claim recomputation for window 8b block S7-AB (verifier, VALIDITY lens).
Written from raw/ and the ruling's definitions; does not import or run the analyst's s7*.py.
Sources of the rule: RULINGS-2026-09-27-W8.md ruling 1; win8/analysis.md Method (cell = median over K of process
means; r = min-max, i = IQR (inclusive quartiles, as win8 lib8.py), s = 1.2533 SD / sqrt(K), all relative to the
median; flag x iff |B/A - 1| > 2 hypot(A_x, B_x); K >= 3 per side). Claim (ruling 1): i AND s set pooled AND in
every clean pass; STRONG iff r also set. Slot rule (win8): original if valid and clean, else its re-run if valid and
clean, else dropped; clean = receipts before/after <= 5 %, witness <= 2 %, no build process, no build/lane presence."""
import collections
import hashlib
import json
import math
import os
import statistics
import sys

W = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b'
RAW = W + '/raw'
FIX = W + '/gate/fixtures'
EX = json.load(open(W + '/rows8b.extra.json', encoding='utf-8'))
ROWS = {r['id']: r for r in EX['rows']}
BINS = EX['binaries']
POSE = '0x30c5438bc6ad9ffa'
WINDOWS = [(0, 500), (0, 100), (100, 500)]
OUT = []


def say(*a):
    s = ' '.join(str(x) for x in a)
    OUT.append(s)
    print(s)


def sha256(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()


def norm(p):
    return os.path.normcase(os.path.normpath(p))


def parse_summary(txt):
    out = None
    for line in txt.splitlines():
        if line.startswith('SUMMARY '):
            if out is not None:
                return 'MULTIPLE'
            out = json.loads(line[len('SUMMARY '):])
    return out


def read_csv(p):
    lines = open(p, encoding='utf-8').read().splitlines()
    hdr = lines[0].split(',')
    rows = [l.split(',') for l in lines[1:] if l.strip()]
    cols = {h: [] for h in hdr}
    for r in rows:
        for h, v in zip(hdr, r):
            cols[h].append(v)
    return cols


# ---------------------------------------------------------------- records
recs = [json.loads(l) for l in open(RAW + '/runs.jsonl', encoding='utf-8') if l.strip()]
s7recs = [r for r in recs if r.get('block') == 'S7-AB']
procs = [r for r in s7recs if 'row' in r and not r.get('r4')]
timed = [r for r in procs if r.get('attempt') in ('original', 'rerun')]
warm = [r for r in procs if r.get('attempt') == 'warmup']
other = [r for r in procs if r.get('attempt') not in ('original', 'rerun', 'warmup')]
say(f'S7-AB records {len(s7recs)}: processes {len(procs)} (timed {len(timed)}: originals '
    f'{sum(r["attempt"] == "original" for r in timed)}, re-runs {sum(r["attempt"] == "rerun" for r in timed)}; '
    f'warm-ups {len(warm)}; other {len(other)}); pass_done {sum(1 for r in s7recs if r.get("pass_done"))}; '
    f'voided_pass {sum(1 for r in s7recs if r.get("voided_pass"))}; r4 {sum(1 for r in s7recs if r.get("r4"))}')
stray = [r for r in recs if 'row' in r and str(r.get('row', '')).startswith('S7') and r.get('block') != 'S7-AB']
say(f'S7 rows recorded outside block S7-AB: {len(stray)}')
say(f'S7-AB processes carrying a "void" key: {sum(1 for r in procs if r.get("void"))}; "aborted": '
    f'{sum(1 for r in procs if "aborted" in r)}')
pd = sorted((r['pass'], r.get('pass_attempt'), r.get('time')) for r in s7recs if r.get('pass_done'))
say(f'pass_done records: {pd}')
say(f'pass_attempt values over processes: {sorted(collections.Counter(r.get("pass_attempt") for r in procs).items())}')

# ---------------------------------------------------------------- binaries
say('\n## Binary identity')
for k in ('s7p', 's7t'):
    p = os.path.join(W, BINS[k]['exe'])
    got = sha256(p)
    say(f'{k}: {BINS[k]["exe"]} sha256 now {got} pin {BINS[k]["sha256_pin"]} match {got == BINS[k]["sha256_pin"]}; '
        f'commit {BINS[k]["commit"][:8]}; s7 label {BINS[k].get("s7")}')
sums = {}
for line in open(W + '/bin/SHA256SUMS', encoding='utf-8'):
    h, p = line.split()
    sums[p.lstrip('*')] = h
say(f'SHA256SUMS: s7p path {sums.get(BINS["s7p"]["exe"])} ; s7t path {sums.get(BINS["s7t"]["exe"])}')


# ---------------------------------------------------------------- per-process validation
def expected_args(r, row):
    cwd = r['cwd']
    w = r['W']
    return list(row['args']) + ['--workers', str(w), '--steps', '500', '--window', '0..500',
                                '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
                                '--label', f'{r["row"]}#{r["binary"]}@W{w}',
                                '--expect-pose', os.path.join(FIX, row['pose_ref'] + '.pose')]


def same_args(a, b):
    if len(a) != len(b):
        return False
    for x, y in zip(a, b):
        if (chr(92) in x or '/' in x) and (chr(92) in y or '/' in y):
            if norm(x) != norm(y):
                return False
        elif x != y:
            return False
    return True


fixbytes = {k: open(os.path.join(FIX, k + '.pose'), 'rb').read() for k in ('JT500', 'JA500')}
fixj = json.load(open(FIX + '/fixtures.json', encoding='utf-8'))
say(f'fixtures.json hashes: JT500 {fixj["JT500"]["hash"]}, JA500 {fixj["JA500"]["hash"]}; '
    f'JT500.pose == JA500.pose bytes: {fixbytes["JT500"] == fixbytes["JA500"]}')


def validate(r):
    row = ROWS[r['row']]
    why = []
    key, w = r['binary'], r['W']
    if key not in row['binaries']:
        why.append(f'binary {key} not in row')
    if w not in row['workers']:
        why.append(f'W {w} not in row')
    if r.get('role') != key:
        why.append('role != binary')
    if os.path.basename(r.get('exe', '')).lower() != os.path.basename(BINS[key]['exe']).lower():
        why.append(f'exe {r.get("exe")}')
    if r.get('exe_sha256') != BINS[key]['sha256_pin']:
        why.append('exe_sha256 != pin')
    if r.get('commit') != BINS[key]['commit']:
        why.append('commit')
    if not same_args(r.get('args') or [], expected_args(r, row)):
        why.append('args differ from the row')
    if r.get('exit') != 0:
        why.append(f'exit {r.get("exit")}')
    if r.get('hang'):
        why.append('hang')
    d = r['cwd']
    so = open(os.path.join(d, 'stdout.txt'), 'rb').read().decode('utf-8', 'replace')
    s = parse_summary(so)
    if not isinstance(s, dict):
        return why + [f'SUMMARY {s}'], None, None
    if not same_args(s.get('args') or [], r.get('args') or []):
        why.append('SUMMARY args != record args')
    if s.get('label') != f'{r["row"]}#{key}@W{w}':
        why.append(f'label {s.get("label")}')
    if s.get('void_steps') != 0:
        why.append(f'void_steps {s.get("void_steps")}')
    if s.get('pose_hash') != POSE or s.get('expect_pose') != 'match':
        why.append(f'pose {s.get("pose_hash")} {s.get("expect_pose")}')
    if s.get('workers') != w or (s.get('threads') or {}).get('pool_workers') != w:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('target_env')
    if s.get('armed'):
        why.append('armed')
    if s.get('disarmed_ring_traffic') not in (0, None):
        why.append(f'ring {s.get("disarmed_ring_traffic")}')
    if s.get('steps') != 500 or s.get('window') != [0, 500]:
        why.append('steps/window')
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != row['broadphase']:
        why.append(f'broadphase {cfg.get("broadphase")}')
    a = row['args']
    want_cfg = a[a.index('--cfg') + 1]
    if s.get('cfg') != want_cfg:
        why.append(f'cfg {s.get("cfg")}')
    if cfg.get('sleeping') is not False or cfg.get('contact_reuse') is not True:
        why.append('sleeping/reuse')
    bt = s.get('broadphase_tree') or {}
    if row['broadphase'] == 'Tree':
        if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0
                and not bt.get('kd_order_builds')):
            why.append(f'treediag {bt}')
    else:
        if any(v not in (0, None) for k2, v in bt.items() if isinstance(v, (int, float))):
            why.append(f'treediag nonzero {bt}')
    if '--canary-frac' in a:
        f = float(a[a.index('--canary-frac') + 1])
        t = int(a[a.index('--canary-ref-ns') + 1])
        if s.get('canary_ns') != round(f * t):
            why.append(f'canary_ns {s.get("canary_ns")} != {f * t}')
    elif s.get('canary_ns') is not None:
        why.append('canary on a plain row')
    if s.get('canary_zone') is not None:
        why.append('zone canary')
    pb = open(os.path.join(d, 'pose.bin'), 'rb').read()
    if pb != fixbytes[row['pose_ref']]:
        why.append('pose.bin bytes != fixture')
    c = read_csv(os.path.join(d, 'run.csv'))
    wall = [float(x) for x in c['wall_ns'] if x != '']
    if len(c['wall_ns']) != 500 or len(wall) != 500:
        why.append(f'csv rows {len(c["wall_ns"])}')
    if [int(x) for x in c['step']] != list(range(500)):
        why.append('csv step column')
    m500 = sum(wall[0:500]) / 500
    if abs(m500 - s['window_mean_ns']) > 1.0:
        why.append(f'window mean {m500} vs SUMMARY {s["window_mean_ns"]}')
    if r.get('mean_ms') is None or abs(m500 / 1e6 - r['mean_ms']) > 1e-6:
        why.append('driver mean_ms mismatch')
    means = {wn: sum(wall[wn[0]:wn[1]]) / (wn[1] - wn[0]) / 1e6 for wn in WINDOWS}
    man = tuple(c['manifolds'])
    return why, s, {'means': means, 'manifolds': man}


def clean(r):
    why = []
    rb = (r.get('receipt_before') or {}).get('cpu_avg')
    ra = (r.get('receipt_after') or {}).get('cpu_avg')
    if rb is None or rb > 5.0:
        why.append(f'receipt before {rb}')
    if ra is None or ra > 5.0:
        why.append(f'receipt after {ra}')
    if (r.get('receipt_before') or {}).get('build_procs_busy') or (r.get('receipt_after') or {}).get('build_procs_busy'):
        why.append('build procs busy')
    ob = r.get('others_busy_pct')
    if ob is None or ob > 2.0:
        why.append(f'witness {ob}')
    if r.get('build_proc_during'):
        why.append('build during')
    if r.get('void_names_new_during'):
        why.append('void names during')
    for k in ('receipt_before', 'receipt_after'):
        pr = (r.get(k) or {}).get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            why.append(f'{k} presence {pr}')
    return why


say('\n## Per-process validity (all timed S7-AB processes, originals and re-runs)')
info = {}
bad = []
drv_disagree = []
clean_disagree = []
manseq = collections.defaultdict(set)
summ_keys = collections.defaultdict(set)
for r in timed:
    why, s, x = validate(r)
    cw = clean(r)
    info[id(r)] = {'why': why, 'clean': cw, 's': s, 'x': x}
    if why:
        bad.append((r['pass'], r['round'], r['seq'], r['attempt'], r['row'], r['binary'], r['W'], why))
    if bool(why) != (not r.get('valid')):
        drv_disagree.append((r['pass'], r['seq'], r['row'], r['binary'], r['W'], why, r.get('invalid')))
    if bool(cw) != bool(r.get('contaminated')):
        clean_disagree.append((r['pass'], r['seq'], r['row'], r['binary'], r['W'], cw, r.get('contaminated')))
    if x:
        manseq[ROWS[r['row']]['pose_ref']].add(x['manifolds'])
    if s:
        summ_keys[r['binary']].add((s.get('profile_name'), s.get('debug_assertions'), s.get('zones_compiled'),
                                    s.get('system_zones_compiled'), s.get('runner')))
say(f'invalid (my checks): {len(bad)}')
for b in bad:
    say('  INVALID', b)
say(f'driver valid flag disagrees with my checks: {len(drv_disagree)} {drv_disagree[:5]}')
say(f'driver contaminated flag disagrees with my clean rule: {len(clean_disagree)} {clean_disagree[:5]}')
say('distinct per-step manifold sequences: ' + ', '.join(f'{k} {len(v)}' for k, v in manseq.items())
    + f'; JT == JA sequence: {manseq["JT500"] == manseq["JA500"]}')
say(f'build-identity fields in SUMMARY (profile_name, debug_assertions, zones, system_zones, runner) by key: '
    f'{ {k: sorted(v) for k, v in summ_keys.items()} }')
hot = [(r['pass'], r['round'], r['seq'], r['attempt'], r['row'], r['binary'], r['W'], info[id(r)]['clean'])
       for r in timed if info[id(r)]['clean']]
say(f'not clean: {len(hot)}')
for h in hot:
    say('  HOT', h)
t0 = min(r['start'] for r in timed)
t1 = max(r['end'] for r in timed)
say(f'timed S7-AB span: {t0} .. {t1}')

# ---------------------------------------------------------------- slot rule
say('\n## Slot rule')
slots = collections.defaultdict(lambda: {'original': [], 'rerun': []})
for r in timed:
    slots[(r['pass'], r['round'], r['row'], r['binary'], r['W'])][r['attempt']].append(r)
used, used_orig_only = {}, {}
dropped = []
multi = [k for k, v in slots.items() if len(v['original']) != 1 or len(v['rerun']) > 1]
say(f'slots {len(slots)}; slots with != 1 original or > 1 re-run: {multi}')
reruns_used = []
for k, v in slots.items():
    o = v['original'][0]
    ok_o = not info[id(o)]['why'] and not info[id(o)]['clean']
    used_orig_only[k] = o if not info[id(o)]['why'] else None
    if ok_o:
        used[k] = o
        continue
    rr = v['rerun'][0] if v['rerun'] else None
    if rr is not None and not info[id(rr)]['why'] and not info[id(rr)]['clean']:
        used[k] = rr
        reruns_used.append((k, info[id(o)]['why'] + info[id(o)]['clean'], o['start'], rr['start'],
                            rr.get('others_busy_pct'), rr['receipt_before']['cpu_avg'], rr['receipt_after']['cpu_avg']))
    else:
        dropped.append((k, info[id(o)]['why'] + info[id(o)]['clean']))
say(f'used {len(used)}; dropped {len(dropped)} {dropped}; re-runs used {len(reruns_used)}')
for x in reruns_used:
    say('  RERUN USED', x)
unused_rerun = [(k, v['original'][0]['seq']) for k, v in slots.items() if v['rerun'] and used.get(k) is not v['rerun'][0]]
say(f're-runs NOT used: {unused_rerun}')
cells = collections.defaultdict(list)
for k, r in used.items():
    cells[(k[2], k[3], k[4])].append((k[0], k[1], r))
say('K per cell: ' + str(sorted(collections.Counter(len(v) for v in cells.values()).items())) +
    '; K per (cell, pass): ' + str(sorted(collections.Counter(
        sum(1 for p, _, _ in v if p == pp) for v in cells.values() for pp in (0, 1, 2)).items())))
say(f'cells: {len(cells)}')

# ---------------------------------------------------------------- placement receipt
say('\n## Placement receipts (recorded, never used to drop)')
nopl = [r for r in used.values() if not isinstance(r.get('placement'), dict)]
notop3 = [r for r in used.values() if not (r.get('placement') or {}).get('top3')]
nocyc = [r for r in used.values() if (r.get('placement') or {}).get('main_cycle_frac') is None]
noshare = [r for r in used.values() if (r.get('placement') or {}).get('main_share_top_est') is None]
say(f'used processes: no placement object {len(nopl)}; no top3 {len(notop3)}; no main_cycle_frac {len(nocyc)}; '
    f'main_share_top_est null {len(noshare)}')
for r in noshare:
    pl = r['placement']
    say(f'  share-null: p{r["pass"]} r{r["round"]} {r["row"]}#{r["binary"]}@W{r["W"]} main_cpu_s {pl.get("main_cpu_s")} '
        f'main_cycle_frac {pl.get("main_cycle_frac")} top3 {pl.get("top3")}')
allnoshare = [r for r in timed if (r.get('placement') or {}).get('main_share_top_est') is None]
say(f'all timed (incl. unused originals): main_share_top_est null {len(allnoshare)}')

# ---------------------------------------------------------------- adjacency and order
say('\n## Launch order and P/T adjacency (originals)')
ids = [r['id'] for r in EX['rows'] if 'S7-AB' in r['blocks']]
order0 = [(rid, key, w) for w in (0, 1, 2, 4, 8, 16) for rid in ids if w in ROWS[rid]['workers']
          for key in ROWS[rid]['binaries']]
for p in (0, 1, 2):
    exp = order0[::-1] if p % 2 == 0 else order0
    for rnd in (0, 1, 2):
        orig = [r for r in timed if r['pass'] == p and r['round'] == rnd and r['attempt'] == 'original']
        got = [(r['row'], r['binary'], r['W']) for r in sorted(orig, key=lambda r: r['seq'])]
        pairs = []
        for rid in ('S7-JT', 'S7-JA', 'S7-JT-W1'):
            for w in ROWS[rid]['workers']:
                sp = [r['seq'] for r in orig if r['row'] == rid and r['W'] == w and r['binary'] == 's7p'][0]
                st = [r['seq'] for r in orig if r['row'] == rid and r['W'] == w and r['binary'] == 's7t'][0]
                pairs.append(f'{rid}@W{w}:{"T" if st < sp else "P"}first,adj={abs(st - sp) == 1}')
        say(f'p{p} r{rnd}: order == expected {"reversed" if p % 2 == 0 else "forward"} cell_order: {got == exp}; '
            + ' '.join(pairs))


# ---------------------------------------------------------------- statistics
def cellstat(xs):
    xs = sorted(xs)
    m = statistics.median(xs)
    iq = statistics.quantiles(xs, n=4, method='inclusive')
    sd = statistics.stdev(xs)
    return {'K': len(xs), 'med': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / m,
            'i': (iq[2] - iq[0]) / m, 's': 1.2533 * sd / math.sqrt(len(xs)) / m, 'iqr_ms': iq[2] - iq[0],
            'se_ms': 1.2533 * sd / math.sqrt(len(xs))}


def cmpc(a, b):
    ratio = b['med'] / a['med']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'ris'}
    ok = a['K'] >= 3 and b['K'] >= 3
    return {'ratio': ratio, 'd': b['med'] - a['med'], 'bars': bars, 'fl': {k: ok and e > bars[k] for k in 'ris'}}


def vals(sel, cellkey, wn, p=None):
    return [info[id(r)]['x']['means'][wn] for (pp, rr, r) in sel[cellkey] if p is None or pp == p]


def judge(sel, A, B, wn):
    res = {'A': cellstat(vals(sel, A, wn)), 'B': cellstat(vals(sel, B, wn))}
    res['pooled'] = cmpc(res['A'], res['B'])
    for p in (0, 1, 2):
        res[p] = cmpc(cellstat(vals(sel, A, wn, p)), cellstat(vals(sel, B, wn, p)))
    comps = [res['pooled'], res[0], res[1], res[2]]
    one_sign = len(set(c['ratio'] > 1 for c in comps)) == 1
    claimed = all(c['fl']['i'] and c['fl']['s'] for c in comps) and one_sign
    strong = claimed and all(c['fl']['r'] for c in comps)
    res.update({'claimed': claimed, 'claimed_nosign': all(c['fl']['i'] and c['fl']['s'] for c in comps),
                'strong': strong, 'strong_pooled_r': claimed and res['pooled']['fl']['r'],
                'pooled_only': res['pooled']['fl']['i'] and res['pooled']['fl']['s'],
                'dir': 'B slower' if res['pooled']['ratio'] > 1 else 'B faster',
                'binding': max(max(c['bars']['i'], c['bars']['s']) for c in comps)})
    return res


def yn(c):
    return '/'.join('Y' if c['fl'][k] else 'n' for k in 'ris')


def show(name, res):
    a, b, pc = res['A'], res['B'], res['pooled']
    passes = ' '.join('p%d %.4f %s' % (p, res[p]['ratio'], yn(res[p])) for p in (0, 1, 2))
    say('%s: A %.4f [%.4f-%.4f] K=%d IQR %.4f SE %.4f | B %.4f [%.4f-%.4f] K=%d IQR %.4f SE %.4f | '
        'B/A %.4f (B-A %+.4f ms) pooled %s bars r/i/s %.2f/%.2f/%.2f %%; passes %s | CLAIMED %s (%s) STRONG %s '
        '(pooled-r reading %s); pooled-only i&s %s; binding %.2f %% = %.4f ms' % (
            name, a['med'], a['min'], a['max'], a['K'], a['iqr_ms'], a['se_ms'],
            b['med'], b['min'], b['max'], b['K'], b['iqr_ms'], b['se_ms'],
            pc['ratio'], pc['d'], yn(pc), 100 * pc['bars']['r'], 100 * pc['bars']['i'], 100 * pc['bars']['s'],
            passes, res['claimed'], res['dir'], res['strong'], res['strong_pooled_r'], res['pooled_only'],
            100 * res['binding'], res['binding'] * a['med']))


JT8P, JT8T, JT16P, JT16T = ('S7-JT', 's7p', 8), ('S7-JT', 's7t', 8), ('S7-JT', 's7p', 16), ('S7-JT', 's7t', 16)
JA8P, JA8T, JA16P, JA16T = ('S7-JA', 's7p', 8), ('S7-JA', 's7t', 8), ('S7-JA', 's7p', 16), ('S7-JA', 's7t', 16)
CLAIMS = [('C1-JT T8->T16', JT8T, JT16T, 'notslower'), ('C1-JA T8->T16', JA8T, JA16T, 'notslower'),
          ('C2-JT P8->T8', JT8P, JT8T, 'notslower'), ('C2-JA P8->T8', JA8P, JA8T, 'notslower'),
          ('C3-JT P16->T16', JT16P, JT16T, 'faster'),
          ('W1 P1->T1 (optional)', ('S7-JT-W1', 's7p', 1), ('S7-JT-W1', 's7t', 1), 'notslower')]
LADDERS = [('ladder8', JT8P, [('S7-ladder8-F0.5', 0.030), ('S7-ladder8-F1', 0.060), ('S7-ladder8-F1.5', 0.090),
                             ('S7-ladder8-F2', 0.120)]),
           ('ladder16', JT16P, [('S7-ladder16-F0.5', 0.0525), ('S7-ladder16-F1', 0.105), ('S7-ladder16-F1.5', 0.1575),
                               ('S7-ladder16-F2', 0.210)]),
           ('JA-rung16', JA16P, [('S7-JA-ladder16-F1', 0.105)])]


def verdict_of(res, kind):
    slower = res['claimed'] and res['dir'] == 'B slower'
    faster = res['claimed'] and res['dir'] == 'B faster'
    tag = ' STRONG' if res['strong'] else ''
    if kind == 'notslower':
        return ('FAILS/REFUTED (B slower claimed%s)' % tag) if slower else 'HOLDS (B slower not claimed)'
    gain = -res['pooled']['d']
    if faster:
        return 'CLAIMED T faster%s, gain %.4f ms, bar 0.105 met %s' % (tag, gain, gain >= 0.105)
    if slower:
        return 'REFUTED (T slower claimed%s), P16-T16 = %+.4f ms' % (tag, gain)
    return 'NOT CLAIMED, P16-T16 = %+.4f ms' % gain


def analyse(sel, label):
    say('\n## Claims [%s]' % label)
    verdict = {}
    for wn in WINDOWS:
        say('-- window [%d,%d)' % wn)
        for name, A, B, kind in CLAIMS:
            res = judge(sel, A, B, wn)
            show(name, res)
            v = verdict_of(res, kind)
            say('   => ' + v)
            verdict[(name, wn)] = v
        for lname, ref, rungs in LADDERS:
            seen = []
            for rid, inj in rungs:
                res = judge(sel, ref, (rid, 's7p', ref[2]), wn)
                ok = res['claimed'] and res['dir'] == 'B slower'
                seen.append((inj, ok))
                say('   %s %s inj %.4f: rise %+.4f ms (x%.2f) pooled %s %s -> %s' % (
                    lname, rid, inj, res['pooled']['d'], res['pooled']['d'] / inj, yn(res['pooled']),
                    ' '.join('p%d %s' % (p, yn(res[p])) for p in (0, 1, 2)), 'SEEN' if ok else 'not seen'))
            R = None
            for j, (inj, ok) in enumerate(seen):
                if ok and all(o for _, o in seen[j:]):
                    R = inj
                    break
            say('   %s: R = %s' % (lname, R))
            verdict[(lname, wn)] = R
    return verdict


v_main = analyse(cells, 'slot rule: clean re-runs replace hot originals')

cells_o = collections.defaultdict(list)
for k, o in used_orig_only.items():
    if o is not None:
        cells_o[(k[2], k[3], k[4])].append((k[0], k[1], o))
v_orig = analyse(cells_o, 'post hoc sensitivity: originals only (hot originals kept)')

cells_s = collections.defaultdict(list)
for k, r in used.items():
    if (r.get('placement') or {}).get('main_share_top_est') is not None:
        cells_s[(k[2], k[3], k[4])].append((k[0], k[1], r))
short = []
for c, v in sorted(cells_s.items()):
    per = [sum(1 for p, _, _ in v if p == pp) for pp in (0, 1, 2)]
    if min(per) < 3:
        short.append((c, per))
say('\nK per pass of cells that lose a process if share-null processes were dropped: %s' % short)
v_share = None
try:
    v_share = analyse(cells_s, 'post hoc: share-null processes dropped (NOT the protocol; receipts never drop)')
except Exception as e:
    say('share-null drop analysis not computable: %r' % e)

say('\n## Verdict agreement across selections ([0,500))')
for key in [k for k in v_main if k[1] == (0, 500)]:
    say('%s: main %s | originals %s | share-drop %s' % (key[0], v_main[key], v_orig.get(key),
                                                         None if v_share is None else v_share.get(key)))
for wn in WINDOWS[1:]:
    say('-- [%d,%d): ' % wn + '; '.join('%s: %s' % (k[0], v) for k, v in v_main.items() if k[1] == wn))

with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'vv_out.txt'), 'w', encoding='utf-8') as f:
    f.write('\n'.join(OUT) + '\n')
