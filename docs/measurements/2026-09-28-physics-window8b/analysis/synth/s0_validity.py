"""Window-wide validity for the synthesis: processes, validity (mine vs the driver's flag), clean, slot rule, drops,
K per pass-cell, idle waits, voided passes, R4, placement-receipt coverage, the witness's top processes."""
import collections
import datetime as dt
import re

import synthlib as L

O = L.Out()
recs = L.all_records()
BLOCKS = ['S4-AB', 'S7-AB', 'SPLIT', 'omega-v2', 'F3', 'F3-G4', 'J-Son-T', 'DM1', 'omega-v1-cont']


def ts(s):
    return dt.datetime.fromisoformat(s)


log = open(L.os.path.join(L.RAW, 'window_log.txt'), encoding='utf-8').read().splitlines()
start = next(l for l in log if 'WINDOW START' in l)
end = next(l for l in log if 'WINDOW END' in l)
O('# Window 8b validity (synthesis re-derivation)')
O(start)
O(end)
t0, t1 = ts(start.split()[0]), ts(end.split()[0])
O(f'window wall: {(t1 - t0).total_seconds() / 60:.1f} min')

# idle waits
O('\n## idle waits per pass (idle rule start -> idle reached)')
waits = []
pend = {}
for l in log:
    m = re.match(r'(\S+) (\S+-p\d): idle rule', l)
    if m:
        pend[m.group(2)] = ts(m.group(1))
    m = re.match(r'(\S+) (\S+-p\d): idle reached', l)
    if m:
        w = (ts(m.group(1)) - pend[m.group(2)]).total_seconds()
        waits.append((m.group(2), w))
for k, w in waits:
    O(f'  {k:22s} {w:6.1f} s')
ws = [w for _, w in waits]
O(f'passes {len(waits)}; wait min {min(ws):.1f} / median {sorted(ws)[len(ws) // 2]:.1f} / max {max(ws):.1f} s; '
  f'sum {sum(ws) / 60:.1f} min; waits > 135 s: {[(k, round(w)) for k, w in waits if w > 135]}')
O(f"voided_pass records: {sum(1 for r in recs if r.get('voided_pass'))}; pass_done records: "
  f"{sum(1 for r in recs if r.get('pass_done'))}")
r4 = [r for r in recs if r.get('r4')]
O(f"R4 records: {len(r4)}; verdicts {collections.Counter(r['verdict'] for r in r4)}")

# per block
O('\n## per block: processes, validity, clean, slot rule')
tot = collections.Counter()
witness_top = collections.Counter()
unclean_kinds = collections.Counter()
kdist_all = {}
for b in BLOCKS:
    procs, used, dropped = L.select(b, recs)
    warm = [r for r in recs if r.get('block') == b and r.get('attempt') == 'warmup']
    orig = [p for p in procs if p['attempt'] == 'original']
    rer = [p for p in procs if p['attempt'] == 'rerun']
    inval = [p for p in procs if p['valid_why']]
    drv_inval = [p for p in procs if not p['rec'].get('valid')]
    disagree = sum(1 for p in procs if bool(p['valid_why']) != (not p['rec'].get('valid')))
    uncl = [p for p in procs if p['clean_why']]
    for p in uncl:
        for w in p['clean_why']:
            unclean_kinds[(b, w.split()[0] + ' ' + w.split()[1] if w.startswith('receipt') else w.split()[0])] += 1
        top = (p['rec'].get('others_top5') or [{}])[0].get('name')
        witness_top[top] += 1
    rer_used = [p for p in used if p['attempt'] == 'rerun']
    times = [ts(p['rec']['start']) for p in procs] + [ts(p['rec']['end']) for p in procs]
    O(f"{b:14s} timed {len(procs):4d} (orig {len(orig):3d}, re-run {len(rer):3d}), warm-ups {len(warm)}, "
      f"invalid mine {len(inval)} / driver {len(drv_inval)} (disagree {disagree}), unclean {len(uncl):3d}, "
      f"slots {len(used) + len(dropped):3d}, used {len(used):3d} (re-runs {len(rer_used):2d}), dropped {len(dropped):2d}; "
      f"{min(times).strftime('%H:%M:%S')}-{max(times).strftime('%H:%M:%S')}")
    for k, ps in dropped:
        why = [(p['attempt'], p['clean_why'] or p['valid_why']) for p in ps]
        O(f"    dropped {k} {ps[0]['row']}#{ps[0]['binary']}@W{ps[0]['W']}: {why}")
    # K per pass-cell
    kc = collections.Counter()
    for p in used:
        kc[(p['row'], p['binary'], p['W'], p['pass'])] += 1
    if b not in ('DM1', 'F3-G4'):
        cells = {(r, bi, w) for (r, bi, w, _) in kc}
        npass = len({p['pass'] for p in procs})
        dist = collections.Counter(kc.get((r, bi, w, ps), 0) for (r, bi, w) in cells for ps in range(npass))
        ndist = collections.Counter(sum(kc.get((r, bi, w, ps), 0) for ps in range(npass)) for (r, bi, w) in cells)
        kdist_all[b] = (dist, ndist)
        O(f"    cells {len(cells)}; K per pass-cell {dict(sorted(dist.items()))}; n per cell {dict(sorted(ndist.items()))}")
    tot['timed'] += len(procs)
    tot['orig'] += len(orig)
    tot['rerun'] += len(rer)
    tot['warm'] += len(warm)
    tot['invalid'] += len(inval)
    tot['drv_invalid'] += len(drv_inval)
    tot['disagree'] += disagree
    tot['unclean'] += len(uncl)
    tot['slots'] += len(used) + len(dropped)
    tot['used'] += len(used)
    tot['dropped'] += len(dropped)
    tot['rerun_used'] += len(rer_used)
O(f"\nTOTAL {dict(tot)}")
O('\n## unclean reasons (process-level; one process may carry several)')
for k, v in sorted(unclean_kinds.items()):
    O(f'  {k}: {v}')
O(f"\n## top 'other' process in the witness of unclean processes: {witness_top.most_common(6)}")

# placement receipts
timed = [r for r in recs if 'row' in r]
pl = [r for r in timed if (r.get('placement') or {}).get('main_cycle_frac') is not None]
ps = [r for r in timed if (r.get('placement') or {}).get('main_share_top_est') is not None]
O(f"\nplacement receipts: {len(pl)} of {len(timed)} launched processes carry main_cycle_frac, {len(ps)} "
  f"main_share_top_est; used to drop: none (the slot rule above reads only validity and clean)")

# used-process receipt maxima
O('\n## receipts of the USED processes, per block (max before / after / witness %)')
for b in BLOCKS:
    _, used, _ = L.select(b, recs)
    rb = max((p['rec']['receipt_before'] or {}).get('cpu_avg') for p in used)
    ra = max((p['rec']['receipt_after'] or {}).get('cpu_avg') for p in used)
    wi = max(p['rec'].get('others_busy_pct') for p in used)
    O(f'  {b:14s} {rb:.2f} / {ra:.2f} / {wi:.2f}')
O('\n## binaries of the window (rows8b.json + rows8b.extra.json): ' +
  ', '.join(f"{k} ({v['kind']}, {v['exe'].split('/')[-1]})" for k, v in L.BINS.items()))
O('kinds of timed processes: ' + str(dict(collections.Counter(r['kind'] for r in recs if 'row' in r))))
O.save('s0_validity.txt')
