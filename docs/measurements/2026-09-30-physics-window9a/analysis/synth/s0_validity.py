"""Window-wide validity census for the synthesis: record counts per block, driver validity vs the group libraries'
independent re-check, clean/unclean, slots used/dropped, K per pass-cell (vs the driver's passcell records), pass
timeline and idle waits (raw/window_log.txt), receipts of the used processes, top other processes on unclean runs,
the aborted C4-BR pass and the last idle attempt (wait_log.txt). Counts and timestamps only; no timing verdict."""
import collections
import datetime
import os
import re
import statistics

import synth9a as S

O = S.Out()
d = S.load()
recs = d['recs']

O('# 1. Records')
kinds = collections.Counter()
for r in recs:
    if r.get('pass_done'):
        kinds['pass_done'] += 1
    elif r.get('passcell'):
        kinds['passcell'] += 1
    elif r.get('r4'):
        kinds['r4'] += 1
    elif 'row' in r:
        kinds['process'] += 1
    else:
        kinds['other'] += 1
O(f'runs.jsonl lines {len(recs)}: {dict(kinds)}')
per = collections.defaultdict(collections.Counter)
for r in recs:
    if 'row' in r and not r.get('passcell') and not r.get('r4'):
        b = r['block']
        per[b][r.get('attempt')] += 1
        if r.get('aborted'):
            per[b]['aborted'] += 1
        if r.get('attempt') != 'warmup' and r.get('valid') is False:
            per[b]['driver_invalid'] += 1
        if r.get('attempt') != 'warmup' and r.get('contaminated'):
            per[b]['driver_unclean'] += 1
for b in ('C4-AB', 'C4-RAPIER', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-G4-kd'):
    O(f'  {b:10s} {dict(per.get(b, {})) or "NO RECORDS (never ran)"}')
tot = sum(v['original'] + v['rerun'] for v in per.values())
O(f'  timed process records (originals + re-runs, all blocks): {tot}; re-runs {sum(v["rerun"] for v in per.values())}; '
  f'warm-ups {sum(v["warmup"] for v in per.values())}')

O('\n# 2. Closed-pass blocks: independent validity, clean, slots, K')
for b, key in (('C4-AB', 'AB'), ('C4-RAPIER', 'RAP'), ('C4-G5', 'G5')):
    procs, used, dropped = d[key]
    inval = [p for p in procs if p['valid_why']]
    drv_inval = [p for p in procs if p['rec'].get('valid') is False]
    uncl = [p for p in procs if p['clean_why']]
    drv_uncl = [p for p in procs if p['rec'].get('contaminated')]
    agree_c = sum(1 for p in procs if bool(p['clean_why']) == bool(p['rec'].get('contaminated')))
    orig = sum(1 for p in procs if p['attempt'] == 'original')
    rer = sum(1 for p in procs if p['attempt'] == 'rerun')
    used_rr = sum(1 for p in used if p['attempt'] == 'rerun')
    O(f'{b}: processes {len(procs)} (originals {orig}, re-runs {rer}); invalid: mine {len(inval)}, driver {len(drv_inval)}; '
      f'unclean: mine {len(uncl)}, driver {len(drv_uncl)}, agree {agree_c}/{len(procs)}; slots {len(used) + len(dropped)}: '
      f'used {len(used)} ({used_rr} re-runs), dropped {len(dropped)}')
    byp = collections.Counter((p['pass'], bool(p['clean_why'])) for p in procs)
    O('   unclean by pass: ' + ', '.join(f"p{k}: {byp[(k, True)]} of {byp[(k, True)] + byp[(k, False)]}" for k in S.PASSES))
    dp = collections.Counter(k[1] for k, _ in dropped)
    O(f'   dropped slots by pass: {dict(dp)}')
    # K per pass-cell vs the driver's passcell records
    kc = collections.Counter((p['pass'], p['row'], p['binary'], p['W']) for p in used)
    pcs = [r for r in recs if r.get('passcell') and r.get('block') == b]
    agree = sum(1 for r in pcs if kc.get((r['pass'], r['pc_row'], r['pc_binary'], r['pc_W']), 0) == r['k_clean'])
    short = [(r['pass'], r['pc_row'], r['pc_binary'], r['pc_W'], r['k_clean']) for r in pcs if r['k_clean'] < 3]
    O(f'   pass-cells {len(pcs)}; my K == driver k_clean on {agree}/{len(pcs)}; short (K<3) {len(short)}; '
      f'by pass {dict(collections.Counter(s[0] for s in short))}')
    if short:
        full0 = sorted({(r['pc_row'], r['pc_binary'], r['pc_W']) for r in pcs if r['pass'] == 0 and r['k_clean'] >= 3})
        O(f'   pass-0 pass-cells with K = 3: {full0}')
        O(f'   pass-0 pass-cells with K = 0: {[s for s in short if s[4] == 0]}')
    # receipts of the used processes
    for k in S.PASSES:
        u = [p['rec'] for p in used if p['pass'] == k]
        rb = statistics.median((x.get('receipt_before') or {}).get('cpu_avg') for x in u)
        ra = statistics.median((x.get('receipt_after') or {}).get('cpu_avg') for x in u)
        w = statistics.median(x.get('others_busy_pct') for x in u)
        a = [p['rec'] for p in procs if p['pass'] == k]
        wa = statistics.median(x.get('others_busy_pct') for x in a)
        O(f'   p{k}: used n {len(u)}: receipt before {rb:.2f} %, after {ra:.2f} %, witness {w:.2f} % | all processes '
          f'witness median {wa:.2f} %')
    tops = collections.Counter()
    for p in procs:
        if p['clean_why']:
            t = p['rec'].get('others_top5') or []
            if t:
                tops[t[0]['name']] += 1
    O(f'   top other process on unclean runs: {tops.most_common(5)}')
    # poses / hashes of the used processes
    if key in ('AB', 'G5'):
        ph = collections.Counter((p['row'], p['s'].get('pose_hash')) for p in used if 's' in p)
        jh = collections.Counter(p['j']['stat_lines'][0]['hash'] for p in used if 'j' in p)
        vs = collections.Counter(p['s'].get('void_steps') for p in used if 's' in p)
        O(f'   used runner pose hashes: {sorted(set(h for _, h in ph))}; void_steps values {dict(vs)}; '
          f'Jolt hashes {dict(jh)}')
    else:
        O(f'   used Rapier processes with a V1-V9-shaped failure: {sum(1 for p in used if p["valid_why"])}; '
          f'processes with any: {len(inval)}')

O('\n# 3. Timeline and idle waits (raw/window_log.txt)')
ts = lambda s: datetime.datetime.fromisoformat(s)  # noqa: E731
ev = collections.defaultdict(dict)
for line in open(os.path.join(S.RAW, 'window_log.txt'), encoding='utf-8'):
    m = re.match(r'(\S+) (C4-[A-Z0-9-]+-p\d): (idle rule|idle reached|opening receipt|done|re-run stage ended after (\d+))',
                 line)
    if m:
        k = m.group(3) if not m.group(3).startswith('re-run') else 'rerun_end'
        ev[m.group(2)][k] = ts(m.group(1))
        if m.group(4):
            ev[m.group(2)]['waves'] = int(m.group(4))
tw = 0.0
for ps, e in ev.items():
    wait = (e['idle reached'] - e['idle rule']).total_seconds() if 'idle reached' in e else None
    tw += wait or 0
    dur = (e['done'] - e['idle rule']).total_seconds() / 60 if 'done' in e else None
    O(f"  {ps:14s} idle wait {wait if wait is not None else '-':>6} s; start {e['idle rule'].strftime('%H:%M:%S')}"
      f"; end {e['done'].strftime('%H:%M:%S') if 'done' in e else 'ABORTED'}; "
      f"{f'{dur:.1f} min' if dur else '-'}; re-run waves {e.get('waves', '-')}")
O(f'  idle waits total {tw / 60:.1f} min')
t0 = min(e['idle rule'] for e in ev.values())
O(f'  window start {t0.strftime("%H:%M:%S")}')
for line in open(os.path.join(S.RAW, 'window_log.txt'), encoding='utf-8'):
    if 'WINDOW END' in line or 'quiet budget exhausted' in line or 'ABORTED' in line or 'allowance spent' in line:
        O('  ' + line.strip()[:330])

O('\n# 4. C4-BR (NOT analysed: its only pass has no pass_done record)')
br = [r for r in recs if r.get('block') == 'C4-BR' and 'row' in r]
for r in br:
    O(f"  r{r['round']} {r['binary']:6s} exit {r.get('exit')} aborted {r.get('aborted')} contaminated {r.get('contaminated')}")
O(f"  pass_done for C4-BR: {any(r.get('pass_done') and r.get('block') == 'C4-BR' for r in recs)}")

O('\n# 5. The last idle attempt (wait_log.txt, from 22:09)')
lines = open(os.path.join(S.W9A, 'wait_log.txt'), encoding='utf-8').read().splitlines()
start = max(i for i, l in enumerate(lines) if l.startswith('# wait_idle9a.ps1 start'))
for l in lines[start:]:
    m = re.search(r'poll\s+(\d+) (\S+) cpu10=([\d.]+)%.*D_free_GB=([\d.]+).*top3\(cpu s in ~10s\): (\S+)', l)
    if m:
        O(f'  poll {m.group(1)} {m.group(2)[11:19]} cpu10 {m.group(3)} % D_free {m.group(4)} GB top {m.group(5)}')
    elif l.startswith('#'):
        O('  ' + l[:200])
stop = os.path.join(S.W9A, 'STOP')
O(f"  STOP flag mtime {datetime.datetime.fromtimestamp(os.path.getmtime(stop)).strftime('%H:%M:%S')}")
O.save('s0_validity.txt')
