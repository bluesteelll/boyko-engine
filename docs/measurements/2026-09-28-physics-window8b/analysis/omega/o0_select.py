"""o0: selection, validity (micro8b rules re-applied on stdout.txt), cleanliness, slot rule, K per row and pass,
cross-check against the driver's own flags, placement receipts (recorded only)."""
import collections
import sys

sys.dont_write_bytecode = True
import lib_om as L

O = L.Out()
procs = L.all_procs()
O('# o0 selection: omega-v2 + omega-v1-cont (window 8b, raw/runs.jsonl + per-process stdout.txt)')
wu = [p for p in procs if p['attempt'] == 'warmup']
tp = [p for p in procs if p['attempt'] != 'warmup']
O('processes: %d timed (%d original, %d re-run) + %d warm-ups' % (
    len(tp), sum(p['attempt'] == 'original' for p in tp), sum(p['attempt'] == 'rerun' for p in tp), len(wu)))
inval = [p for p in tp if p['valid_why']]
O('validity (micro8b rules + exit 0 + sha pin, re-applied): %d invalid of %d' % (len(inval), len(tp)))
for p in inval:
    O('  INVALID %s p%s r%s %s %s: %s' % (p['block'], p['pass'], p['round'], p['row'], p['attempt'], p['valid_why']))
dv = [p for p in tp if bool(p['driver_valid']) != (not p['valid_why'])]
O('driver valid flag disagrees with mine: %d' % len(dv))
dc = [p for p in tp if bool(p['driver_contaminated']) != bool(p['clean_why'])]
O('driver contaminated flag disagrees with window-8 clean rule: %d' % len(dc))
for p in dc:
    O('  %s p%s r%s %s %s driver %s mine %s' % (p['block'], p['pass'], p['round'], p['row'], p['attempt'],
                                             p['driver_contaminated'], p['clean_why']))
used, dropped, slots = L.select(procs)
O('slots %d, used %d (%d originals, %d re-runs), dropped %d' % (
    len(slots), len(used), sum(p['attempt'] == 'original' for p in used), sum(p['attempt'] == 'rerun' for p in used),
    len(dropped)))
for k, ps in dropped:
    O('  DROPPED slot %s: %s' % (k, ['%s %s' % (p['attempt'], p['clean_why'] or p['valid_why']) for p in ps]))
O()
O('K per (block, row) and pass (used processes):')
kk = collections.defaultdict(lambda: collections.Counter())
for p in used:
    kk[(p['block'], p['row'])][p['pass']] += 1
for (b, r), c in sorted(kk.items()):
    O('  %-14s %-22s total %d  per pass %s' % (b, r, sum(c.values()), dict(sorted(c.items()))))
O()
O('re-runs used, and why the original was not:')
for p in used:
    if p['attempt'] == 'rerun':
        k = (p['block'], p['pass'], p['round'], p['row'])
        o = [q for q in slots[k] if q['attempt'] == 'original'][0]
        O('  %s p%s r%s %s: original %s' % (p['block'], p['pass'], p['round'], p['row'], o['clean_why'] or o['valid_why']))
O()
O('receipts of used processes: before max %.2f %%, after max %.2f %%, witness max %.2f %%, witness median %.2f %%' % (
    max(p['rb'] for p in used), max(p['ra'] for p in used), max(p['others_busy_pct'] for p in used),
    sorted(p['others_busy_pct'] for p in used)[len(used) // 2]))
O()
O('placement receipts (RECORDED, never used to drop): top CPU and its busy %, main-thread cycle share of the process')
for p in sorted(used, key=lambda q: (q['block'], q['row'], q['pass'], q['round'])):
    pl = p['placement']
    t3 = pl.get('top3') or []
    O('  %-13s %-20s p%d r%d %-8s top3 %s main_cycle_frac %s main_share_top_est %s' % (
        p['block'], p['row'], p['pass'], p['round'], p['attempt'], t3, pl.get('main_cycle_frac'),
        pl.get('main_share_top_est')))
O()
O('calibration of used v2 processes (work_ns_calibrated, work_iters):')
for p in used:
    if p['cal']:
        c = p['cal'][0]
        O('  %-20s p%d r%d %-8s work_ns_calibrated %.1f iters %d probes %s' % (
            p['row'], p['pass'], p['round'], p['attempt'], c['work_ns_calibrated'], c['work_iters'], c['probe_ns_per_iter']))
O.save('o0.txt')
