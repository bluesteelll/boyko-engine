"""C4-BR validity, census and receipts (read-only). Output: br0_validity.txt."""
import collections
import json
import statistics
import sys

sys.dont_write_bytecode = True
import lib_br as B

P = B.Out()
recs = B.recs_all()
ab = B.br_records(recs, '191354')
P('## 0. Records')
P('- aborted 2026-09-30 attempt (run_tag 191354): %d process records, pass_done %s -> NOT data, excluded' % (
    len(ab), bool(B.closed(recs, '191354'))))
rs = B.br_records(recs)
P('- resumed run (run_tag 033740): %d process records; closed passes %s' % (len(rs), sorted(B.closed(recs))))
procs, used, dropped = B.select(recs)
P('- timed processes in closed passes: %d (originals %d, re-runs %d); used %d; dropped slots %d' % (
    len(procs), sum(1 for p in procs if p['attempt'] == 'original'), sum(1 for p in procs if p['attempt'] == 'rerun'),
    len(used), len(dropped)))
P('')
P('## 1. Per process (execution order)')
for p in sorted(procs, key=lambda x: x['seq']):
    r = p['rec']
    P('- seq %d r%d %-5s %s: exit %s, sha %s, ids stdout %d / disk %d (need %d), sampling %s x%s, printed-vs-recomputed '
      'worst %.2e, driver est_ms-vs-recomputed worst %.2e; receipts %s / %s %%, witness %s %%, build %s; valid %s, '
      'clean %s; wall %.1f s; %s .. %s' % (
          p['seq'], p['round'], p['binary'], p['attempt'], r.get('exit'), B.L.sha_ok(r), len(p['printed']),
          len(p['on_disk']), len(B.NEED), sorted(set(p['modes'].values())), sorted(set(p['nsamp'].values())),
          p['worst_print'], p['worst_drv'], (r.get('receipt_before') or {}).get('cpu_avg'),
          (r.get('receipt_after') or {}).get('cpu_avg'), r.get('others_busy_pct'), r.get('build_proc_during'),
          not p['valid_why'], not p['clean_why'], r.get('wall_s'), r.get('start'), r.get('end')))
    if p['valid_why'] or p['clean_why']:
        P('    problems: %s %s' % (p['valid_why'], p['clean_why']))
    if r.get('valid') != (not p['valid_why']):
        P('    DISAGREES with the driver valid flag %s' % r.get('valid'))
P('')
P('## 2. Order: the three exes adjacent in every round (the one pass is REVERSED: g4rT, g4r8b, g4r7)')
by_round = collections.defaultdict(list)
for p in sorted(procs, key=lambda x: x['seq']):
    by_round[p['round']].append(p['binary'])
for k in sorted(by_round):
    P('- round %d: %s' % (k, ' -> '.join(by_round[k])))
P('')
P('## 3. Pass-cells: driver k_clean vs mine')
pcs = [r for r in recs if r.get('passcell') and r.get('block') == 'C4-BR' and r.get('run_tag') == B.RUN_TAG]
for pc in pcs:
    mine = sum(1 for p in used if p['binary'] == pc['pc_binary'])
    P('- %s: driver k_clean %d, gates %s, reruns %d; mine K %d -> %s' % (
        pc['pc_binary'], pc['k_clean'], pc['gates'], pc['reruns'], mine, 'AGREE' if mine == pc['k_clean'] else 'DIFFER'))
P('')
P('## 4. Workload identity at 144/256 (stderr kernel receipts: rows, pairs), every process')
ref = None
bad = 0
for p in procs:
    w = {k: v for k, v in p['work'].items() if k[2] in ('LeafList', 'RowWalk')}
    if ref is None:
        ref = w
    if w != ref:
        bad += 1
        P('- DIFFERS: seq %d %s %s' % (p['seq'], p['binary'], w))
P('- %d of %d processes carry the reference workload; reference: %s' % (len(procs) - bad, len(procs), sorted(
    (k, v) for k, v in ref.items() if k[2] == 'LeafList')))
P('')
P('## 5. Receipts of the used processes (recorded; clean thresholds 5 / 5 / 2 %)')
for b in B.BINARIES:
    us = [p for p in used if p['binary'] == b]
    P('- %-5s n=%d: witness %s %%; receipts before %s / after %s %%; perf _Total median per process %s (%% of nominal '
      'clock, PDH witness); main_share_top_est %s; top CPU %s' % (
          b, len(us), [p['rec']['others_busy_pct'] for p in us],
          [(p['rec'].get('receipt_before') or {}).get('cpu_avg') for p in us],
          [(p['rec'].get('receipt_after') or {}).get('cpu_avg') for p in us],
          ['%.1f' % p['perf_med'] for p in us], [(p['rec'].get('placement') or {}).get('main_share_top_est') for p in us],
          [(p['rec'].get('placement') or {}).get('top_cpu') for p in us]))
allperf = [p['perf_med'] for p in used]
P('- perf _Total over the 9 used processes: median %.1f, min %.1f, max %.1f' % (
    statistics.median(allperf), min(allperf), max(allperf)))
top = collections.Counter()
for p in used:
    for x in (p['rec'].get('others_top5') or [])[:1]:
        top[x['name']] += 1
P('- top other process (witness) per used process: %s' % top.most_common())
P('- block span: %s .. %s' % (min(p['rec']['start'] for p in procs), max(p['rec']['end'] for p in procs)))
B.Out.save(P, 'br0_validity.txt')
