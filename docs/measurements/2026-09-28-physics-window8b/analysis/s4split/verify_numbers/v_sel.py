"""Selection, own validation, per-process statistics for S4-AB and SPLIT. Output: v_sel.txt, v_proc.json."""
import json, os, collections
import vlib as L

OUT = []
P = lambda *a: OUT.append(' '.join(str(x) for x in a))
for k, f in L.EXE.items():
    sh = L.file_sha(os.path.join(L.BIN, f))
    P('bin', k, f, sh, 'SHA256SUMS match' if sh == L.SUMS[f] else 'MISMATCH')

recs = L.load_recs(('S4-AB', 'SPLIT'))
P('records (S4-AB, SPLIT, no pass markers):', len(recs), collections.Counter((r['block'], r.get('attempt')) for r in recs))
slots, used, dropped, log = L.select(recs)
P('slots', len(slots), 'used', len(used), 'dropped', len(dropped), dropped)
for b in ('S4-AB', 'SPLIT'):
    P(b, 'slots', sum(1 for k in slots if k[0] == b), 'processes', sum(len(v) for k, v in slots.items() if k[0] == b),
      'used', sum(1 for r, s, c in used if r['block'] == b))
P('rejected candidates (valid-why, clean-why):')
for x in log:
    P('  ', x)
# every candidate's own validity, not only the used
nbad = 0
for k, cand in slots.items():
    for att, r in cand.items():
        why, s, c = L.validate(r)
        if why:
            nbad += 1
            P('  INVALID', k, att, why)
P('invalid candidates (own check):', nbad)
# re-run details
for k, cand in slots.items():
    if 'rerun' in cand:
        o, rr = cand['original'], cand['rerun']
        P('rerun slot', k, 'orig witness', o['others_busy_pct'], 'top', o.get('others_top5'), 'mean_ms', o.get('mean_ms'),
          '| rerun witness', rr['others_busy_pct'], 'mean_ms', rr.get('mean_ms'))

WINS = {'0..500': (0, 500), '100..500': (100, 500), '0..100': (0, 100)}
CNT = ['waves', 'phys_wave_ramp', 'phys_wave_tail', 'phys_wave_join', 'phys_wave_helped', 'phys_wave_first_ramp',
       'phys_wave_first_tail', 'phys_wave_pass_ramp', 'phys_np_wave_ramp', 'phys_np_wave_tail', 'phys_np_wave_join',
       'phys_np_route_worker', 'phys_wave_inflight', 'phys_wave_lanes', 'phys_route_worker', 'phys_route_external',
       'phys_color_scopes', 'phys_setup_chunks']
procs = []
mx = collections.defaultdict(float)
for r, s, c in used:
    p = {'block': r['block'], 'pass': r['pass'], 'round': r['round'], 'row': r['row'], 'bin': r['binary'], 'W': r['W'],
         'attempt': r['attempt'], 'start': r['start'], 'dir': os.path.basename(r['cwd']),
         'rb': r['receipt_before']['cpu_avg'], 'ra': r['receipt_after']['cpu_avg'], 'wit': r['others_busy_pct'],
         'pl': r.get('placement'), 'tpn': s.get('ticks_per_ns'), 'w8s': s.get('w8s'), 'pose': s['pose_hash'],
         'expect': s['expect_pose'], 'armed': s['armed'], 'stats': {}, 'sums': {}}
    for wn, (lo, hi) in WINS.items():
        p['stats'][wn] = {k: L.win_mean(c, k, lo, hi) for k in c if not k.endswith('_n') and k != 'step'}
        p['sums'][wn] = {k: L.win_sum(c, k, lo, hi) for k in CNT if k in c}
        p['stats'][wn]['wall_median'] = sorted(c['wall_ns'][lo:hi])[(hi - lo) // 2]
    p['frac_gt_1p2_med'] = None
    ws = c['wall_ns']
    med = sorted(ws)[250]
    p['frac_gt_1p2_med'] = sum(1 for x in ws if x > 1.2 * med) / 500.0
    p['median_step'] = (sorted(ws)[249] + sorted(ws)[250]) / 2
    procs.append(p)
    for k in ('rb', 'ra', 'wit'):
        mx[k] = max(mx[k], p[k])
P('used receipts max: before %.2f after %.2f witness %.2f' % (mx['rb'], mx['ra'], mx['wit']))
P('poses:', collections.Counter((p['pose'], p['expect']) for p in procs))
# S4 counters by binary/W on armed
sc = collections.Counter((p['block'], p['row'], p['bin'], p['W'], (p['w8s'] or {}).get('setup_steps'), (p['w8s'] or {}).get('setup_tasks'))
                         for p in procs if p['armed'])
for k, v in sorted(sc.items(), key=str):
    P('  setup counters', k, 'n', v)
# n per cell per pass
cells = collections.Counter((p['block'], p['row'], p['bin'], p['W'], p['pass']) for p in procs)
bad = {k: v for k, v in cells.items() if v != 3}
P('cells x pass with n != 3:', bad)
P('cells:', len(set(k[:4] for k in cells)))
json.dump(procs, open(os.path.join(L.HERE, 'v_proc.json'), 'w'))
open(os.path.join(L.HERE, 'v_sel.txt'), 'w').write('\n'.join(OUT) + '\n')
print('\n'.join(OUT))
