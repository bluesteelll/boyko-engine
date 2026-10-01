"""C4-G5 validity: processes, my validity check vs the driver's flag, clean vs the driver's flag, slots used/dropped,
K per pass-cell, R4, pass_done, idle waits, receipts of the used processes. Writes g5_validity.txt."""
import collections
import statistics

import lib_g5 as L

O = L.Out()
recs = L.all_records()
BLK = 'C4-G5'
procs, used, dropped = L.select(BLK, recs)
O('# C4-G5 validity (passes with pass_done: %s)' % sorted(L.done_passes(BLK, recs)))
warm = [r for r in recs if r.get('block') == BLK and r.get('attempt') == 'warmup']
O('processes (timed) %d: original %d, rerun %d; warm-ups %d' % (
    len(procs), sum(p['attempt'] == 'original' for p in procs), sum(p['attempt'] == 'rerun' for p in procs), len(warm)))
inv = [p for p in procs if p['valid_why']]
O('invalid by my check: %d; driver valid flag false: %d; disagreements: %d' % (
    len(inv), sum(not p['rec'].get('valid') for p in procs),
    sum(bool(p['valid_why']) == bool(p['rec'].get('valid')) for p in procs)))
for p in inv:
    O('  INVALID %s p%d seq %d %s W%d: %s' % (p['attempt'], p['pass'], p['seq'], p['row'], p['W'], p['valid_why']))
unc = [p for p in procs if p['clean_why']]
O('unclean by my check: %d; driver contaminated: %d; disagreements: %d' % (
    len(unc), sum(bool(p['rec'].get('contaminated')) for p in procs),
    sum(bool(p['clean_why']) != bool(p['rec'].get('contaminated')) for p in procs)))
for p in unc:
    O('  UNCLEAN %s p%d seq %d %s W%d wall %.3f s: %s' % (p['attempt'], p['pass'], p['seq'], p['row'], p['W'],
                                                         p['rec']['wall_s'], p['clean_why']))
O('slots used %d, dropped %d' % (len(used), len(dropped)))
O('re-runs used (slot filled by a re-run): %d' % sum(p['attempt'] == 'rerun' for p in used))
kc = collections.defaultdict(dict)
for p in used:
    kc[(p['row'], p['W'])][p['pass']] = kc[(p['row'], p['W'])].get(p['pass'], 0) + 1
O('K per cell (pass0/pass1/pass2 -> pooled):')
for (row, W), d in sorted(kc.items()):
    O('  %-14s W%-2d %s -> %d' % (row, W, '/'.join(str(d.get(q, 0)) for q in (0, 1, 2)), sum(d.values())))
r4 = [r for r in recs if r.get('block') == BLK and r.get('r4')]
O('R4 (C4-S16 vs C4-S16ap pose bytes): %d records, verdicts %s' % (len(r4), dict(collections.Counter(r['verdict'] for r in r4))))
# pose hashes and tree_diag of used processes
ph = collections.Counter((p['row'], p['W'], p['s'].get('pose_hash')) for p in used)
O('pose hashes of used processes: %s' % sorted(ph.items()))
td = collections.Counter((p['row'], tuple(sorted((k, v) for k, v in (p['s'].get('broadphase_tree') or {}).items()
                                                  if k in ('static_rebuilds', 'members', 'evictions', 'leaf_list_leaves',
                                                           'fallback_leaves', 'bp_kernel')))) for p in used)
for k, n in sorted(td.items()):
    O('  tree_diag %s x%d' % (k, n))
O('void_steps on armed used processes: %s' % sorted({p['s'].get('void_steps') for p in used if L.ROWS[p['row']]['armed']}))
# receipts of used processes
for W in (1, 8):
    u = [p for p in used if p['W'] == W]
    O('receipts, used W%d (n=%d): before median %.2f %% max %.2f; after median %.2f max %.2f; witness median %.2f max %.2f' % (
        W, len(u), statistics.median(p['rec']['receipt_before']['cpu_avg'] for p in u),
        max(p['rec']['receipt_before']['cpu_avg'] for p in u),
        statistics.median(p['rec']['receipt_after']['cpu_avg'] for p in u),
        max(p['rec']['receipt_after']['cpu_avg'] for p in u),
        statistics.median(p['rec']['others_busy_pct'] for p in u), max(p['rec']['others_busy_pct'] for p in u)))
# pass time spans
for q in (0, 1, 2):
    u = [p['rec'] for p in procs if p['pass'] == q]
    O('pass %d: first start %s, last end %s' % (q, min(r['start'] for r in u)[11:19], max(r['end'] for r in u)[11:19]))
O.save('g5_validity.txt')
