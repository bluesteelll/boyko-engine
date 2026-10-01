"""Selection, validity and per-process statistics for block F3 (runner rows), recomputed from raw/.
Writes procs_f3.json and prints sel_f3.txt. The driver's valid/contaminated/mean_ms are compared, never used."""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
import lib_f3 as L

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


recs = L.load_recs()
f3 = [r for r in recs if r.get('block') == 'F3' and 'row' in r and not r.get('r4')]
warm = [r for r in f3 if r.get('attempt') == 'warmup']
timed = [r for r in f3 if r.get('attempt') != 'warmup']
P('F3 block: %d process records (%d warm-ups, %d timed: %d originals, %d re-runs); pass_done markers %d' % (
    len(f3), len(warm), len(timed), sum(1 for r in timed if r['attempt'] == 'original'),
    sum(1 for r in timed if r['attempt'] == 'rerun'),
    sum(1 for r in recs if r.get('block') == 'F3' and r.get('pass_done'))))
P('voided passes (any block): %d' % sum(1 for r in recs if r.get('voided_pass')))

procs = []
problems = []
for r in timed:
    rid = r['row']
    row = L.ROWS[rid]
    fam = L.row_family(rid)
    d = r['cwd']
    p = {k: r.get(k) for k in ('block', 'pass', 'round', 'row', 'binary', 'W', 'attempt', 'seq', 'start', 'end',
                               'others_busy_pct', 'exit', 'cwd', 'rerun_reason')}
    p['family'] = fam
    p['kernel'] = row['bp_kernel']
    p['receipt_before'] = (r.get('receipt_before') or {}).get('cpu_avg')
    p['receipt_after'] = (r.get('receipt_after') or {}).get('cpu_avg')
    p['witness_top'] = [(o['name'], o['cpu_s']) for o in (r.get('others_top5') or [])[:2]]
    p['placement'] = r.get('placement')
    p['clean_why'] = L.clean_why(r)
    why = []
    if r.get('exit') != 0:
        why.append('exit %s' % r.get('exit'))
    if not L.sha_ok(r):
        why.append('sha')
    if r.get('hang'):
        why.append('hang')
    ss = L.summaries_of(L.read_text(os.path.join(d, 'stdout.txt')))
    if len(ss) != 1:
        why.append('%d SUMMARY lines' % len(ss))
    s = ss[0] if ss else {}
    if s.get('void_steps') != 0:
        why.append('void %s' % s.get('void_steps'))
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']:
        why.append('workers')
    if s.get('target_env') != 'msvc':
        why.append('env')
    if bool(s.get('armed')) != bool(row['armed']):
        why.append('armed flag')
    if s.get('pose_hash') != L.ROW_HASH[fam]:
        why.append('pose_hash %s' % s.get('pose_hash'))
    if s.get('expect_pose') != 'match':
        why.append('expect_pose %s' % s.get('expect_pose'))
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != 'Tree':
        why.append('bp kind')
    bt = s.get('broadphase_tree') or {}
    if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
        why.append('treediag')
    kflag = L.KERNEL_FLAG[row['bp_kernel']]
    if bt.get('bp_kernel') != kflag or s.get('bp_kernel_flag') != kflag:
        why.append('kernel %s/%s' % (bt.get('bp_kernel'), s.get('bp_kernel_flag')))
    kd = bt.get('kd_order_builds')
    if row['bp_kernel'] == 'leaflist-kd':
        if not kd:
            why.append('kd_order_builds %s on kd' % kd)
    elif kd:
        why.append('kd_order_builds %s off kd' % kd)
    if row['armed']:
        if s.get('drops_total') != 0:
            why.append('drops')
        if not s.get('w8s') or s['w8s'].get('overflow') != 0:
            why.append('w8s')
    elif s.get('disarmed_ring_traffic'):
        why.append('ring')
    p['kd_order_builds'] = kd
    p['pose_hash'] = s.get('pose_hash')
    p['tree_brute_max_rows'] = cfg.get('tree_brute_max_rows')
    p['pose_md5'] = L.md5(os.path.join(d, 'pose.bin'))
    if p['pose_md5'] is None:
        why.append('no pose.bin')
    c = None
    try:
        c = L.load_csv(os.path.join(d, 'run.csv'))
    except FileNotFoundError:
        why.append('no csv')
    if c is not None:
        wall = c['wall_ns']
        if len(wall) != 500 or any(x is None for x in wall):
            why.append('csv steps %d' % len(wall))
        else:
            wm = statistics.fmean(wall)
            if abs(wm - s.get('window_mean_ns', -1)) > 1.0:
                why.append('window mean %s vs %s' % (wm, s.get('window_mean_ns')))
            p['wall_ms'] = {'0..500': statistics.fmean(wall) / 1e6, '0..100': statistics.fmean(wall[0:100]) / 1e6,
                            '100..500': statistics.fmean(wall[100:500]) / 1e6}
            p['manifolds_100_500'] = statistics.fmean(c['manifolds'][100:500])
            p['driver_mean_ms'] = r.get('mean_ms')
        if row['armed']:
            q = c['phys_bp_query_ns'][100:500]
            b = c['phys_bp_build_ns'][100:500]
            if any(x is None for x in q + b):
                why.append('span None')
            else:
                qb = [x + y for x, y in zip(q, b)]
                four = [sum(c[k][j] for k in L.SPANS4) for j in range(100, 500)]
                a = {}
                a['t_q_med'] = statistics.median(q) / 1e6
                a['t_b_med'] = statistics.median(b) / 1e6
                a['t_qb_med'] = statistics.median(qb) / 1e6
                a['four_med'] = statistics.median(four) / 1e6
                a['t_q_mean'] = statistics.fmean(q) / 1e6
                a['t_b_mean'] = statistics.fmean(b) / 1e6
                a['t_qb_mean'] = statistics.fmean(qb) / 1e6
                a['four_mean'] = statistics.fmean(four) / 1e6
                a['verify_med'] = statistics.median(c['phys_bp_verify_ns'][100:500]) / 1e6
                a['assemble_med'] = statistics.median(c['phys_bp_assemble_ns'][100:500]) / 1e6
                a['queried'] = sorted(set(c['phys_bp_queried'][100:500]))
                a['query_n'] = sorted(set(c['phys_bp_query_n'][100:500]))
                a['build_n'] = sorted(set(c['phys_bp_build_n'][100:500]))
                p['armed'] = a
                if a['queried'] != [1240.0]:
                    why.append('queried %s' % a['queried'][:4])
    p['valid_why'] = why
    procs.append(p)
    if why:
        problems.append((p['pass'], p['round'], rid, p['W'], p['attempt'], why))

# the slot rule: original if valid and clean, else its re-run if valid and clean, else dropped
slots = {}
for p in procs:
    slots.setdefault((p['pass'], p['round'], p['row'], p['binary'], p['W']), []).append(p)
used, dropped = [], []
for k in sorted(slots):
    ps = slots[k]
    pick = None
    for cand in [x for x in ps if x['attempt'] == 'original'] + [x for x in ps if x['attempt'] == 'rerun']:
        if not cand['valid_why'] and not cand['clean_why']:
            pick = cand
            break
    if pick:
        pick['used'] = True
        used.append(pick)
    else:
        dropped.append((k, [(x['attempt'], x['clean_why'], x['valid_why']) for x in ps]))
P('slots %d, used %d (%d by their re-run), dropped %d' % (
    len(slots), len(used), sum(1 for p in used if p['attempt'] == 'rerun'), len(dropped)))
P('validity problems (my re-derivation): %d' % len(problems))
for x in problems:
    P('  %s' % (x,))
P('dropped slots:')
for k, v in dropped:
    P('  pass %d round %d %s W%d: %s' % (k[0], k[1], k[2], k[4], v))
bycwd = {p['cwd']: p for p in procs}
dv = sum(1 for r in timed if bool(r.get('valid')) != (not bycwd[r['cwd']]['valid_why']))
dc = sum(1 for r in timed if bool(r.get('contaminated')) != bool(bycwd[r['cwd']]['clean_why']))
dm = max(abs(p['wall_ms']['0..500'] - p['driver_mean_ms']) for p in procs if p.get('wall_ms'))
P('driver agreement (compared, not used): valid flag differs on %d, clean flag differs on %d, max |mean_ms - mine| = %.3g ms'
  % (dv, dc, dm))
unclean = [p for p in procs if p['clean_why']]
P('unclean processes: %d of %d (witness-only %d)' % (
    len(unclean), len(procs), sum(1 for p in unclean if all(w.startswith('witness') for w in p['clean_why']))))
P('')
P('K per cell (used slots) p0/p1/p2 pooled:')
cells = {}
for p in used:
    cells.setdefault((p['row'], p['W']), []).append(p)
for k in sorted(cells):
    ps = cells[k]
    P('  %-22s W%-2d %s pooled %d' % (k[0], k[1], '/'.join(str(sum(1 for x in ps if x['pass'] == q)) for q in (0, 1, 2)),
                                     len(ps)))
P('tree_brute_max_rows in every SUMMARY: %s' % sorted({p['tree_brute_max_rows'] for p in procs}))
P('kd_order_builds per kernel: %s' % sorted({(p['kernel'], p['kd_order_builds']) for p in procs}, key=str))
arm = [p for p in procs if p.get('armed')]
P('armed rows: phys_bp_queried %s; query_n %s; build_n %s' % (
    sorted({tuple(p['armed']['queried']) for p in arm}), sorted({tuple(p['armed']['query_n']) for p in arm}),
    sorted({tuple(p['armed']['build_n']) for p in arm})))
mf = {}
for p in procs:
    if p.get('manifolds_100_500') is not None:
        mf.setdefault(p['family'], set()).add(round(p['manifolds_100_500'], 6))
P('manifolds per step over [100,500) per row family (every process): %s' % {k: sorted(v) for k, v in mf.items()})
json.dump({'procs': procs}, open(os.path.join(L.HERE, 'procs_f3.json'), 'w'))
open(os.path.join(L.HERE, 'sel_f3.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
