"""C4-CGU validity: sha per record, poses per record, re-runs, slots, K vs the driver's passcell records,
order balance, adjacency of the used pairs, receipts, timeline, args identity with C4-JD, pre-registration
timestamps. Read-only."""
import json
import os
import statistics
from collections import Counter, defaultdict
from datetime import datetime

import cgu_lib as C

L = C.L
o = C.Out()
recs = C.all_records()

cg = [r for r in recs if r.get('block') == C.BLOCK]
proc = [r for r in cg if 'row' in r]
warm = [r for r in proc if r.get('attempt') == 'warmup']
timed = [r for r in proc if r.get('attempt') != 'warmup']
pcs = [r for r in cg if r.get('passcell')]
pdone = [r for r in cg if r.get('pass_done')]
o('# C4-CGU validity (run tag %s)' % C.RUN_CGU)
o(f'records in block: {len(cg)}; process records {len(proc)} (timed {len(timed)}, warm-ups {len(warm)}); '
  f'passcell {len(pcs)}; pass_done {len(pdone)} -> passes {sorted((r["pass"], r["pass_attempt"]) for r in pdone)}')
o(f'run tags of block records: {Counter(r.get("run_tag") for r in cg)}')
o(f'voided records in block: {sum(1 for r in cg if r.get("voided_pass") or r.get("void"))}')
o(f'originals {sum(1 for r in timed if r["attempt"]=="original")}, re-runs {sum(1 for r in timed if r["attempt"]=="rerun")}, '
  f'deepest re-run {max(r.get("rerun_no",0) for r in timed)}')

# ---- 1. sha256 per record + file + manifest ----
o("\n## 1. Binaries: every record's exe and sha256 against its key's pin")
man = json.load(open(os.path.join(C.RAW, 'manifest_1790815060.json'), encoding='utf-8'))
for k in (C.A_KEY, C.B_KEY):
    exe = L.BINS[k]['exe']
    fsha = C.file_sha256(os.path.join(C.W9A, exe))
    allv = {L.BINS[k]['sha256_pin'], C.PIN[k], L.SUMS.get(exe), fsha, man['binaries'][k]['sha256_pin'],
            man['binaries'][k].get('sha256')}
    o(f'{k}: pin(rows) {L.BINS[k]["sha256_pin"][:16]}.. pin(const) {C.PIN[k][:16]}.. SHA256SUMS {L.SUMS.get(exe, "-")[:16]}.. '
      f'file-now {fsha[:16]}.. manifest pin {man["binaries"][k]["sha256_pin"][:16]}.. manifest sha {man["binaries"][k].get("sha256", "-")[:16]}.. '
      f'all equal: {len(allv) == 1}')
bad_sha = []
for r in proc:
    k = r['binary']
    fname = r['exe'].replace(chr(92), '/').split('/')[-1]
    ok = (r.get('exe_sha256') == C.PIN[k] and fname == L.BINS[k]['exe'].split('/')[-1] and r.get('role') == k
          and ((k == 'tip' and r['commit'] == '50e31f1a78bd53adf31628a3fa03ff28c2647275') or
               (k == 'tipcgu1' and r['commit'].endswith('+ CARGO_PROFILE_PARITY_CODEGEN_UNITS=1'))))
    if not ok:
        bad_sha.append((r['pass'], r['seq'], k, r.get('exe_sha256', '')[:8], fname))
o(f'process records checked {len(proc)} (incl. warm-ups and re-runs): sha == pin AND exe filename == the key exe AND role == key '
  f'AND commit field: {len(proc) - len(bad_sha)} OK, {len(bad_sha)} BAD {bad_sha}')
o(f'per key: {dict(Counter((r["binary"], r["exe_sha256"][:8]) for r in proc))}')
o(f'the two pins differ: {C.PIN["tip"] != C.PIN["tipcgu1"]}')

# ---- 2. poses ----
o('\n## 2. Poses: every process (incl. warm-ups, re-runs, unclean) against JT500')
fx = L.FIX['JT500']
fb = L.FIXBYTES['JT500']
pose_bad = []
for r in proc:
    d = r['cwd']
    s = r.get('summary') or {}
    pb = L.read(os.path.join(d, 'pose.bin'))
    if not (s.get('pose_hash') == fx['hash'] and s.get('expect_pose') == 'match' and pb == fb):
        pose_bad.append((r['pass'], r['seq'], r['binary'], r['W'], r['attempt'], s.get('pose_hash'), len(pb)))
o(f'JT500 fixture hash {fx["hash"]}, {len(fb)} bytes. pose_hash == fixture AND expect_pose match AND pose.bin bytes == fixture: '
  f'{len(proc) - len(pose_bad)} of {len(proc)}; bad {pose_bad}')
o(f'void_steps != 0: {sum(1 for r in proc if (r.get("summary") or {}).get("void_steps") != 0)}; exit != 0: '
  f'{sum(1 for r in proc if r.get("exit") != 0)}; hang: {sum(1 for r in proc if r.get("hang"))}; driver invalid: '
  f'{sum(1 for r in proc if not r.get("valid", True) or r.get("invalid"))}')
o(f'profile_name in SUMMARY (a runner label): {dict(Counter((r["binary"], (r.get("summary") or {}).get("profile_name")) for r in proc))}')
o(f'final_manifolds / final_pairs: {dict(Counter(((r.get("summary") or {}).get("final_manifolds"), (r.get("summary") or {}).get("final_pairs")) for r in proc))}')

# ---- 3. independent validity + clean + slots ----
o('\n## 3. Independent validity (lib9a.load_process), clean (ruling 8), slot rule')
procs, used, dropped = C.select(C.BLOCK, recs, C.RUN_CGU, row=C.ROW)
inval = [(p['pass'], p['seq'], p['binary'], p['W'], p['attempt'], p['valid_why']) for p in procs if p['valid_why']]
o(f'timed processes in closed passes: {len(procs)}; independently invalid: {len(inval)} {inval}')
o(f'unclean: {sum(1 for p in procs if p["clean_why"])}; used slots {len(used)} (originals {sum(1 for p in used if p["attempt"]=="original")}, '
  f're-runs {sum(1 for p in used if p["attempt"]=="rerun")}); dropped {len(dropped)}')
why = Counter()
for p in procs:
    for w in p['clean_why']:
        why[' '.join(w.split(' ')[:2]) if w.startswith('receipt') else w.split(' ')[0]] += 1
o(f'unclean reasons (a process may carry several): {dict(why)}')
dis = [(p['pass'], p['seq'], p['attempt'], p['rerun_no']) for p in procs if bool(p['clean_why']) != bool(p['rec'].get('contaminated'))]
o(f'my clean flag vs the driver contaminated flag: disagree on {len(dis)} {dis}')
o('\npass-cell K (mine vs driver k_clean):')
kk = defaultdict(int)
for p in used:
    kk[(p['pass'], p['binary'], p['W'])] += 1
mism = 0
for r in sorted(pcs, key=lambda r: (r['pass'], r['pc_binary'], r['pc_W'])):
    mine = kk[(r['pass'], r['pc_binary'], r['pc_W'])]
    if mine != r['k_clean']:
        mism += 1
    o(f'  p{r["pass"]} {r["pc_binary"]:8s} W{r["pc_W"]:<2d}: K mine {mine} driver {r["k_clean"]} gates {r["gates"]} short {r["short"]} reruns {r["reruns"]}')
o(f'K mismatches: {mism} of {len(pcs)}; pass-cells with K < 3: {sum(1 for r in pcs if r["k_clean"] < 3)}')
o(f'used re-runs per cell: {dict(Counter((p["binary"], p["W"]) for p in used if p["attempt"]=="rerun"))}')

# ---- 4. order balance & adjacency ----
o('\n## 4. Order: which exe ran first in each (pass, round, W) pair of ORIGINALS (executed order by seq), and the used pairs')
origs = [r for r in timed if r['attempt'] == 'original']
pairs = defaultdict(list)
for r in origs:
    pairs[(r['pass'], r['round'], r['W'])].append(r)
first = Counter()
adj_bad = []
for k, g in sorted(pairs.items()):
    g = sorted(g, key=lambda r: r['seq'])
    if len(g) != 2 or g[1]['seq'] - g[0]['seq'] != 1 or {x['binary'] for x in g} != {C.A_KEY, C.B_KEY}:
        adj_bad.append(k)
    first[(k[0], g[0]['binary'])] += 1
o(f'original pairs {len(pairs)}; non-adjacent or malformed pairs: {adj_bad}')
o(f'first-of-pair by pass: {dict(first)}  (expected: p0 and p2 tipcgu1 first, p1 tip first, 30 to 15)')
wo = {}
for ps in (0, 1, 2):
    r0 = sorted([x for x in origs if x['pass'] == ps and x['round'] == 0], key=lambda r: r['seq'])
    wo[ps] = [r['W'] for r in r0][::2]
o(f'W order of round 0 by pass: {wo}')
up = defaultdict(dict)
for p in used:
    up[(p['pass'], p['rec']['round'], p['W'])][p['binary']] = p
both_orig = sum(1 for d in up.values() if len(d) == 2 and all(x['attempt'] == 'original' for x in d.values()))
o(f'used (pass, round, W) pairs: {len(up)}; both members originals (adjacent): {both_orig}; at least one member a re-run '
  f'(not adjacent: re-run at the end of its pass): {len(up) - both_orig}')
for k, d in sorted(up.items()):
    if len(d) == 2 and any(x['attempt'] == 'rerun' for x in d.values()):
        t = [datetime.fromisoformat(x['rec']['start']) for x in d.values()]
        g = abs((t[0] - t[1]).total_seconds())
        o(f'  non-adjacent used pair p{k[0]} r{k[1]} W{k[2]}: start gap {g:.0f} s, ' + str({b: x['attempt'] for b, x in d.items()}))

# ---- 5. receipts, timeline ----
o('\n## 5. Receipts and timeline of the block')
for ps in (0, 1, 2):
    pr = [r for r in proc if r['pass'] == ps]
    t0 = min(r['start'] for r in pr)[11:19]
    t1 = max(r['end'] for r in pr)[11:19]
    up_ = [p['rec'] for p in used if p['pass'] == ps]
    rb = [r['receipt_before']['cpu_avg'] for r in up_]
    ra = [r['receipt_after']['cpu_avg'] for r in up_]
    ob = [r['others_busy_pct'] for r in up_]
    allob = [r['others_busy_pct'] for r in pr if r['attempt'] != 'warmup']
    o(f'p{ps}: {t0}-{t1}; processes {len(pr)} (warm-ups {sum(1 for r in pr if r["attempt"]=="warmup")}, re-runs '
      f'{sum(1 for r in pr if r["attempt"]=="rerun")}); used: receipt before median {statistics.median(rb):.2f} max {max(rb):.2f}, '
      f'after median {statistics.median(ra):.2f} max {max(ra):.2f}, witness median {statistics.median(ob):.2f} max {max(ob):.2f}; '
      f'witness median over all timed {statistics.median(allob):.2f}')
tops = Counter()
for r in timed:
    if r.get('contaminated'):
        for t in (r.get('others_top5') or [])[:1]:
            tops[t['name']] += 1
o(f'top other process on unclean timed runs: {dict(tops)}')
wl = open(os.path.join(C.RAW, 'window_log.txt'), encoding='utf-8').read().splitlines()
for ln in wl:
    if 'C4-CGU' in ln and ('idle' in ln or 'opening receipt' in ln or 'wave' in ln):
        o('  ' + ln[:200])

# ---- 6. args identity ----
o('\n## 6. Args: C4-JDcgu vs C4-JD (rows), and per record (output paths stripped)')
o(f'row args C4-JDcgu {L.ROWS["C4-JDcgu"]["args"]} == C4-JD {L.ROWS["C4-JD"]["args"]}: '
  f'{L.ROWS["C4-JDcgu"]["args"] == L.ROWS["C4-JD"]["args"]}')


def strip(a):
    out, skip = [], False
    for x in a:
        if skip:
            skip = False
            continue
        if x in ('--csv', '--pose-out', '--label'):
            skip = True
            continue
        out.append(os.path.basename(x) if x.endswith('.pose') else x)
    return tuple(out)


same_w = all(len({strip(r['args']) for r in proc if r['W'] == W}) == 1 for W in C.WS)
o(f'distinct stripped arg vectors: {len({strip(r["args"]) for r in proc})} (5 W values); both binaries identical at each W: {same_w}')
abrec = [r for r in recs if r.get('block') == 'C4-AB' and r.get('row') == 'C4-JD' and r.get('binary') == 'tip']
eq_ab = all(strip(r['args']) in {strip(x['args']) for x in proc if x['W'] == r['W']} for r in abrec)
o(f'C4-AB C4-JD#tip records {len(abrec)}: exe sha {dict(Counter(r["exe_sha256"][:8] for r in abrec))}; stripped args equal to '
  f'C4-CGU at the same W: {eq_ab}')

# ---- 7. pre-registration timing ----
o('\n## 7. Pre-registration')
ms = json.dumps(man)
o(f'manifest generated {man["generated"]}; first C4-CGU process start {min(r["start"] for r in proc)}; '
  f'claim sentence present in the manifest: {"cgu1 claimed faster than cgu16 at W8 AND not claimed slower at any W" in ms}')
o(f'row metric windows {L.ROWS["C4-JDcgu"].get("metric_windows")}; PREP addendum: [100,500) PRIMARY, [0,500) beside')
o.save('c0_validity.txt')
