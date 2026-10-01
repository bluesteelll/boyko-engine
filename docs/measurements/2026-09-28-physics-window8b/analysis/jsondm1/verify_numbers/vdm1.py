"""Independent verifier (numbers lens): DM1 C6 re-read of window 8b, from raw/ only.
Gate letter (window 8, q8): per ABBA, d = median(B1-A1, B2-A2) (pairs = adjacent A/B), band = max(|A1-A2|, |B1-B2|);
B<=A if d<=0, inside band if d<=band, RED if d>band.  Ruling 9: ABBA x 2 per row; C6 dropped if VB_EARLY_CULL B-A
stays above its band.  Metric = the zone median_ns over 220 frames from artifact.toml.
Readings computed: H1/H2 = the letter applied per ABBA half; POOL = median of all complete position pairs vs band
max(range A, range B) over all used repeats; ALLORIG = POOL on the originals only (ignoring cleanliness)."""
import json, os, re, statistics

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.abspath(os.path.join(HERE, '..', '..', '..'))
RAW = os.path.join(W8B, 'raw')
SUMS = {}
for ln in open(os.path.join(W8B, 'bin', 'SHA256SUMS'), encoding='utf-8'):
    if ln.strip():
        h, n = ln.split(maxsplit=1)
        SUMS[os.path.basename(n.strip().lstrip('*'))] = h
SHA = {'dmA': SUMS['dm1_A.exe'], 'dmB': SUMS['dm1_B.exe']}
ROWS = {r['id']: r for r in json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))['rows']}
ZN = {2: 'VB_SHADE', 4: 'VB_EARLY_CULL', 9: 'VB_RUN', 14: 'VB_PRODUCE_NET', 17: 'GBUF_DEFERRED_RESOLVE'}
OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


def rd(p):
    try:
        return open(p, encoding='utf-8', errors='replace').read()
    except FileNotFoundError:
        return ''


def zones(d):
    art = rd(os.path.join(d, 'artifact.toml'))
    z = {}
    for blk in re.findall(r'\[\[zone\]\](.*?)(?=\[\[zone\]\]|\Z)', art, re.S):
        kv = dict(re.findall(r'^(\w+) = (.+)$', blk, re.M))
        z[int(kv['id'])] = {k: float(kv[k]) for k in ('n', 'median_ns', 'mean_ns', 'p95_ns', 'stddev_ns')}
    pm = re.search(r'^present_mode = "(\w+)"', art, re.M)
    return z, (pm.group(1) if pm else None)


def validate(r):
    why = []
    row = ROWS[r['row']]
    d = r['cwd']
    if r.get('exit') != 0:
        why.append('exit')
    if r.get('exe_sha256') != SHA[r['binary']]:
        why.append('sha')
    txt = rd(os.path.join(d, 'stdout.txt')) + '\n' + rd(os.path.join(d, 'stderr.txt'))
    if 'test result: ok. 1 passed' not in txt:
        why.append('libtest')
    if 'NOT MEASURED' in txt:
        why.append('not measured')
    m = re.search(r'DM1 timing: path=(\w+) res=(\d+x\d+) edit_rows=(\d+) grow_at=(\S+)', txt)
    exp_path = {'vb': 'VisibilityBuffer', 'deferred': 'Deferred'}[row['path']]
    if not m or m.group(1) != exp_path or m.group(2) != row['res'] or int(m.group(3)) != row['edit_rows'] or m.group(4) != 'None':
        why.append('DM1 line')
    z, pm = zones(d)
    for name, zid in row['need_zones'].items():
        if zid not in z or int(z[zid]['n']) != 220:
            why.append(f'zone {zid}')
    if pm != 'fifo':
        why.append(f'present {pm}')
    return why, z


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
dm = [r for r in recs if r.get('block') == 'DM1' and r.get('attempt') in ('original', 'rerun')]
P(f'# DM1 verifier: {len(dm)} timed records ({sum(r["attempt"] == "original" for r in dm)} originals, '
  f'{sum(r["attempt"] == "rerun" for r in dm)} re-runs)')
procs = {}
nbad = 0
wit = []
for r in dm:
    why, z = validate(r)
    cw = clean(r)
    if why:
        nbad += 1
        P(f'INVALID seq {r["seq"]} {r["attempt"]}: {why}')
    if cw:
        wit.append((r['seq'], r['attempt'], cw, [o['name'] for o in (r.get('others_top5') or [])[:1]]))
    procs.setdefault(r['seq'], []).append(dict(seq=r['seq'], row=r['row'], bin=r['binary'], att=r['attempt'],
                                               ok=(not why) and (not cw), z=z, start=r['start']))
P(f'validity problems: {nbad}; unclean processes: {len(wit)}')
for w in wit:
    P(f'  unclean {w}')
order = json.load(open(os.path.join(W8B, 'rows8b.json'), encoding='utf-8'))
blk = [b for b in order['blocks'] if b['name'] == 'DM1'][0]['order']
used = {}
orig = {}
for i, (row, b, _) in enumerate(blk):
    seq = i + 1
    ps = procs[seq]
    assert all(p['row'] == row and p['bin'] == b for p in ps), seq
    orig[seq] = [p for p in ps if p['att'] == 'original'][0]
    cands = [x for x in ps if x['att'] == 'original'] + [x for x in ps if x['att'] == 'rerun']
    used[seq] = next((p for p in cands if p['ok']), None)
P(f'slots 24, used {sum(1 for v in used.values() if v)}; dropped seqs {[s for s, v in used.items() if not v]}; '
  f're-runs used {[s for s, v in used.items() if v and v["att"] == "rerun"]}')


def analyse(base, zid, src):
    def g(pos):
        p = src[base + pos]
        return None if p is None else p['z'][zid]['median_ns'] / 1e3
    A = {0: g(0), 3: g(3), 4: g(4), 7: g(7)}
    Bv = {1: g(1), 2: g(2), 5: g(5), 6: g(6)}
    pairs = [(0, 1), (3, 2), (4, 5), (7, 6)]
    diffs = [(Bv[b] - A[a]) if (A[a] is not None and Bv[b] is not None) else None for a, b in pairs]
    res = {}
    for hname, hp, ha, hb in (('H1', diffs[:2], (0, 3), (1, 2)), ('H2', diffs[2:], (4, 7), (5, 6))):
        dd = [x for x in hp if x is not None]
        av = [A[k] for k in ha if A[k] is not None]
        bv = [Bv[k] for k in hb if Bv[k] is not None]
        ra = (max(av) - min(av)) if len(av) == 2 else 0.0
        rb = (max(bv) - min(bv)) if len(bv) == 2 else 0.0
        res[hname] = (statistics.median(dd) if dd else None, max(ra, rb), len(dd), len(av), len(bv))
    dd = [x for x in diffs if x is not None]
    av = [v for v in A.values() if v is not None]
    bv = [v for v in Bv.values() if v is not None]
    res['POOL'] = (statistics.median(dd), max(max(av) - min(av), max(bv) - min(bv)), len(dd), len(av), len(bv))
    res['_A'] = A
    res['_B'] = Bv
    res['_diffs'] = diffs
    res['_meanA'] = statistics.mean(av)
    res['_zone_med'] = statistics.median(av + bv)
    return res


def verdict(d, band):
    if d is None:
        return 'n/a'
    if d <= 0:
        return 'B<=A (d<=0)'
    return 'B<=A (inside band)' if d <= band else 'RED (above band)'


def fm(v):
    return '   --   ' if v is None else f'{v:8.2f}'


RES = {}
for row, base, zs in (('dm-vb-1920x1080-idle', 1, (4, 9, 2, 14)), ('dm-deferred-1920x1080-idle', 9, (17,)),
                      ('dm-vb-1920x1080-edit100', 17, (4, 9, 2, 14))):
    P(f'\n## {row} (seq {base}..{base + 7}; A B B A A B B A)')
    for src_name, src in (('USED', used), ('ALLORIG', orig)):
        for zid in zs:
            res = analyse(base, zid, src)
            A, Bv = res['_A'], res['_B']
            P(f'  [{src_name}] zone {zid:2d} {ZN[zid]:22s} A0 {fm(A[0])} B1 {fm(Bv[1])} B2 {fm(Bv[2])} A3 {fm(A[3])} | '
              f'A4 {fm(A[4])} B5 {fm(Bv[5])} B6 {fm(Bv[6])} A7 {fm(A[7])} us')
            nf = sum(1 for x in res['_diffs'] if x is not None and x < 0)
            nn = sum(1 for x in res['_diffs'] if x is not None)
            P(f'       pair diffs B-A {[None if x is None else round(x, 2) for x in res["_diffs"]]}; B faster in {nf} of {nn}')
            for k in ('H1', 'H2', 'POOL'):
                d, band, npair, na, nb = res[k]
                ds = 'n/a' if d is None else f'{d:+.2f} us ({100 * d / res["_meanA"]:+.2f} % of mean A)'
                P(f'       {k}: d {ds}, band {band:.2f} us ({100 * band / res["_zone_med"]:.1f} % of zone), '
                  f'pairs {npair}, A {na}, B {nb} -> {verdict(d, band)}')
            RES[f'{src_name}|{row}|{ZN[zid]}'] = {k: res[k] for k in ('H1', 'H2', 'POOL')}
json.dump(RES, open(os.path.join(HERE, 'vdm1.json'), 'w'), indent=1)
open(os.path.join(HERE, 'vdm1.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')


# post hoc: ruling-1 style i/s/r on DM1 (one block = pooled), USED slots, zone medians in us
import math


def iqr(xs):
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(xs)
    m = statistics.median(xs)
    return dict(K=len(xs), med=m, r=(xs[-1] - xs[0]) / m, i=iqr(xs) / m, s=1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) / m)


P('\n## post hoc: ruling-1 i/s/r on DM1 (single block), USED slots')
for row, base, zs in (('dm-vb-1920x1080-idle', 1, (4, 9, 2, 14)), ('dm-deferred-1920x1080-idle', 9, (17,)),
                      ('dm-vb-1920x1080-edit100', 17, (4, 9, 2, 14))):
    for zid in zs:
        av = [used[base + p]['z'][zid]['median_ns'] / 1e3 for p in (0, 3, 4, 7) if used[base + p]]
        bv = [used[base + p]['z'][zid]['median_ns'] / 1e3 for p in (1, 2, 5, 6) if used[base + p]]
        a, b = cell(av), cell(bv)
        e = b['med'] / a['med'] - 1
        bars = {k: 2 * math.hypot(a[k], b[k]) for k in 'ris'}
        fl = ''.join('Y' if abs(e) > bars[k] else 'n' for k in 'ris')
        P(f'  {row} {ZN[zid]}: A med {a["med"]:.2f} K{a["K"]} B med {b["med"]:.2f} K{b["K"]} B/A-1 {100 * e:+.2f} % '
          f'bars r {100 * bars["r"]:.2f} i {100 * bars["i"]:.2f} s {100 * bars["s"]:.2f} -> r/i/s {fl}')
open(os.path.join(HERE, 'vdm1.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
