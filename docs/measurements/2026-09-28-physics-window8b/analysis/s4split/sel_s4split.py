"""Selection + per-process statistics for S4-AB and SPLIT (and S7-AB's TIP-binary rows, for a post-hoc cross-check
only), recomputed from raw/*/run.csv and stdout SUMMARY. Writes proc_s4split.json and prints the selection record."""
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_s4 as L

BLOCKS = ('S4-AB', 'SPLIT', 'S7-AB')
recs = L.load_recs(BLOCKS)
out, problems = [], []
for r in recs:
    if r.get('attempt') == 'warmup':
        continue
    p = {k: r.get(k) for k in ('block', 'pass', 'round', 'row', 'binary', 'W', 'attempt', 'kind', 'seq', 'start', 'end',
                               'others_busy_pct', 'exit', 'cwd', 'valid')}
    p['driver_invalid'] = r.get('invalid')
    p['receipt_before'] = (r.get('receipt_before') or {}).get('cpu_avg')
    p['receipt_after'] = (r.get('receipt_after') or {}).get('cpu_avg')
    p['witness_top'] = [(o['name'], o['cpu_s']) for o in (r.get('others_top5') or [])[:3]]
    p['placement'] = r.get('placement')
    p['clean_why'] = L.clean(r)
    why, s, c = L.validate_runner(r)
    if s is not None:
        why = why + L.validate_more(r, s, c)
    p['valid_why'] = why
    if c is not None:
        st = {}
        sums = {}
        for wn, (a, b) in L.WINS.items():
            st[wn] = {h: L.mean(v[a:b]) for h, v in c.items() if h not in ('step', 'top_y', 'void', 'awake')}
            sums[wn] = {h: L.ssum(v[a:b]) for h, v in c.items() if h not in ('step', 'top_y', 'void', 'awake')}
        p['stats'] = st
        p['sums'] = sums
        p['wall_ms'] = {wn: st[wn]['wall_ns'] / 1e6 for wn in L.WINS}
    if s is not None:
        p['summary'] = {k: s.get(k) for k in ('pose_hash', 'armed', 'canary_ns', 'canary_zone', 'canary_zone_ns',
                                             'w8s', 'threads', 'final_manifolds', 'waves_total', 'ticks_per_ns',
                                             'window_mean_ns')}
    out.append(p)
    if why:
        problems.append((p['block'], p['pass'], p['round'], p['row'], p['binary'], p['W'], p['attempt'], why))

slots = {}
for p in out:
    k = (p['block'], p['pass'], p['round'], p['row'], p['binary'], p['W'])
    slots.setdefault(k, []).append(p)
used, dropped = [], []
for k, ps in slots.items():
    orig = [p for p in ps if p['attempt'] == 'original']
    rer = [p for p in ps if p['attempt'] == 'rerun']
    pick = None
    for cand in orig + rer:
        if not cand['valid_why'] and not cand['clean_why']:
            pick = cand
            break
    if pick:
        pick['used'] = True
        used.append(pick)
    else:
        dropped.append(k)
for b in BLOCKS:
    ob = [p for p in out if p['block'] == b]
    print('%s: processes (non-warm-up) %d, originals %d, re-runs %d, slots %d, used %d, dropped slots %d' % (
        b, len(ob), sum(p['attempt'] == 'original' for p in ob), sum(p['attempt'] == 'rerun' for p in ob),
        len([k for k in slots if k[0] == b]), sum(1 for p in used if p['block'] == b),
        sum(1 for k in dropped if k[0] == b)))
print('own validity problems:', len(problems))
for x in problems:
    print('  ', x)
print('driver-invalid (valid false):', sum(1 for p in out if not p.get('valid')))
unclean = [p for p in out if p['clean_why']]
print('unclean processes:', len(unclean))
for p in unclean:
    rr = [q for q in slots[(p['block'], p['pass'], p['round'], p['row'], p['binary'], p['W'])] if q['attempt'] == 'rerun']
    print('   %s p%s r%s %s#%s@W%s %s: %s; witness top %s; re-run: %s' % (
        p['block'], p['pass'], p['round'], p['row'], p['binary'], p['W'], p['attempt'], p['clean_why'],
        p['witness_top'], ['%s used=%s others %.2f %%' % (q['start'], q.get('used', False), q['others_busy_pct'])
                           for q in rr]))
rb = [p['receipt_before'] for p in used if p['block'] in ('S4-AB', 'SPLIT')]
ra = [p['receipt_after'] for p in used if p['block'] in ('S4-AB', 'SPLIT')]
wi = [p['others_busy_pct'] for p in used if p['block'] in ('S4-AB', 'SPLIT')]
print('used S4-AB+SPLIT: receipt before max %.2f, after max %.2f, witness max %.2f %%' % (max(rb), max(ra), max(wi)))
json.dump({'procs': out}, open(os.path.join(L.HERE, 'proc_s4split.json'), 'w'))
