"""C4-AB (+ the C4-JD-armed census rows of C4-G5): processes, validity (independent check vs the driver's flag), clean,
slots, K per pass-cell (vs the driver's passcell records), what made processes unclean, pass timing."""
import collections
import json

import lib9a as L

O = L.Out()
recs = L.all_records()
O('closed passes: ' + ', '.join(f'{b}-p{p}' for b, p, _ in sorted(L.closed_passes(recs))))
for block in ('C4-AB', 'C4-G5'):
    procs, used, dropped = L.select(block, recs)
    inv = [p for p in procs if p['valid_why']]
    drv_inv = [p for p in procs if not p['rec'].get('valid', False) or p['rec'].get('invalid')]
    agree = sum(1 for p in procs if bool(p['valid_why']) == bool(p['rec'].get('invalid')))
    cln_agree = sum(1 for p in procs if bool(p['clean_why']) == bool(p['rec'].get('contaminated')))
    O(f'\n## {block}: timed processes {len(procs)} (originals {sum(p["attempt"] == "original" for p in procs)}, '
      f're-runs {sum(p["attempt"] == "rerun" for p in procs)}); invalid by my check {len(inv)}, by the driver '
      f'{len(drv_inv)}; validity agreement {agree}/{len(procs)}; clean agreement with the driver {cln_agree}/{len(procs)}')
    for p in inv[:10]:
        O(f"  INVALID {p['row']}#{p['binary']}@W{p['W']} p{p['pass']} r{p['round']} {p['attempt']}: {p['valid_why']}")
    O(f'  slots {len(used) + len(dropped)}: used {len(used)} (re-runs used {sum(p["attempt"] == "rerun" for p in used)}), '
      f'dropped {len(dropped)}')
    per_pass = collections.Counter()
    unclean_pass = collections.Counter()
    for p in procs:
        per_pass[p['pass']] += 1
        if p['clean_why']:
            unclean_pass[p['pass']] += 1
    O('  per pass: processes / unclean: ' + ', '.join(f'p{k} {per_pass[k]}/{unclean_pass[k]}' for k in sorted(per_pass)))
    # K per pass-cell, mine vs the driver's passcell records
    mine = collections.Counter((p['pass'], p['row'], p['binary'], p['W']) for p in used)
    drv = {(r['pass'], r['pc_row'], r['pc_binary'], r['pc_W']): r['k_clean'] for r in recs
           if r.get('passcell') and r['block'] == block}
    diff = [(k, mine.get(k, 0), v) for k, v in drv.items() if mine.get(k, 0) != v]
    O(f'  pass-cells {len(drv)}; my K == the driver k_clean on {len(drv) - len(diff)}; differ: {diff}')
    short = sorted((k, v) for k, v in drv.items() if v < 3)
    O(f'  short pass-cells (K < 3): {len(short)}: ' + '; '.join(f'p{k[0]} {k[1]}#{k[2]}@W{k[3]} K{v}' for k, v in short))
    # what made them unclean
    why = collections.Counter()
    top = collections.Counter()
    wit = collections.defaultdict(list)
    for p in procs:
        for w in p['clean_why']:
            why[w.split(' ')[0] + ' ' + w.split(' ')[1] if w.startswith('receipt') else w.split(' ')[0]] += 1
        if p['clean_why']:
            t5 = p['rec'].get('others_top5') or []
            if t5:
                top[t5[0]['name']] += 1
        wit[p['pass']].append(p['rec'].get('others_busy_pct') or 0)
    O(f'  unclean reasons (a process can have several): {dict(why)}')
    O(f'  top other process of the witness on unclean processes: {dict(top.most_common(8))}')
    for k in sorted(wit):
        xs = sorted(wit[k])
        O(f'  witness p{k}: median {xs[len(xs) // 2]:.2f} %, max {xs[-1]:.2f} %, > 2 %: {sum(x > 2 for x in xs)} of {len(xs)}')
    # pass wall times
    for r in recs:
        if r.get('pass_done') and r['block'] == block:
            O(f"  pass_done {block}-p{r['pass']} at {r['time']}")
    # poses / hashes on every used runner process
    runners = [p for p in used if p['rec']['kind'] == 'runner']
    ph = collections.Counter((p['row'], p['s']['pose_hash']) for p in runners)
    O(f'  used runner processes {len(runners)}; pose_hash by row: {dict(ph)}')
    jo = [p for p in used if p['rec']['kind'] == 'jolt']
    O(f"  used Jolt processes {len(jo)}; hashes {collections.Counter(p['j']['stat_lines'][0]['hash'] for p in jo)}")
    cfgs = collections.Counter((p['row'], p['binary'], p['s']['config']['broadphase'], p['s']['config']['tree_brute_max_rows'])
                               for p in runners)
    O(f'  (row, binary, broadphase, tree_brute_max_rows): {dict(cfgs)}')
    if block == 'C4-G5':
        arm = [p for p in used if 'armed' in p['row']]
        O(f"  armed used {len(arm)}; void_steps {collections.Counter(p['s']['void_steps'] for p in arm)}")
# all processes of every block with a pass_done: invalid count
allp = [r for r in recs if 'row' in r and r.get('attempt') != 'warmup']
O(f"\nall launched timed process records {len(allp)}; driver-invalid {sum(1 for r in allp if r.get('invalid'))}")
O.save('a0_validity.txt')
