"""S7-AB selection and per-process statistics, recomputed from raw/ files. Writes s7_procs.json and s7_select.txt.
Slot rule (window 7/8): original if valid and clean, else its re-run if valid and clean, else the slot is dropped.
Placement receipts are recorded per process and never used to select."""
import collections
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s7lib as L  # noqa: E402

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


recs = L.load_recs()
procs = []
for r in recs:
    p = {k: r.get(k) for k in ('pass', 'round', 'seq', 'row', 'binary', 'W', 'attempt', 'start', 'end', 'others_busy_pct',
                               'exit', 'cwd', 'contaminated', 'rerun_reason', 'mean_ms')}
    p['receipt_before'] = (r.get('receipt_before') or {}).get('cpu_avg')
    p['receipt_after'] = (r.get('receipt_after') or {}).get('cpu_avg')
    p['driver_invalid'] = r.get('invalid')
    p['driver_valid'] = r.get('valid')
    pl = r.get('placement') or {}
    p['placement'] = {k: pl.get(k) for k in ('top3', 'main_cpu_s', 'main_cycle_frac', 'main_share_top_est',
                                             'top_share_of_proc', 'top_cpu', 'top_busy_s')}
    p['placement_present'] = bool(pl) and pl.get('main_cycle_frac') is not None and bool(pl.get('top3'))
    p['clean_why'] = L.clean_why(r)
    why, s, c = L.validate(r)
    p['valid_why'] = why
    if c is not None:
        p['wall_ms'] = {w: L.mean(c['wall_ns'][a:b]) / 1e6 for w, (a, b) in L.WINS.items()}
        p['manifolds'] = {w: L.mean(c['manifolds'][a:b]) for w, (a, b) in L.WINS.items()}
        p['manifolds_seq_hash'] = hash(tuple(c['manifolds']))
    if s is not None:
        p['pose_hash'] = s.get('pose_hash')
        p['canary_ns'] = s.get('canary_ns')
        p['host'] = s.get('host')
    procs.append(p)

timed = [p for p in procs if p['attempt'] != 'warmup']
warm = [p for p in procs if p['attempt'] == 'warmup']
P(f'# S7-AB selection: {len(recs)} records = {len(timed)} timed processes + {len(warm)} warm-ups')
P(f"attempts: {dict(collections.Counter(p['attempt'] for p in timed))}")
bad = [p for p in timed if p['valid_why']]
P(f'validity problems (recomputed): {len(bad)}')
for p in bad:
    P(f"  INVALID p{p['pass']} r{p['round']} {p['row']}#{p['binary']}@W{p['W']} {p['attempt']}: {p['valid_why']}")
dis = [p for p in timed if bool(p['driver_invalid']) != bool(p['valid_why'])]
P(f"driver 'invalid' field disagrees with the recomputation: {len(dis)}")
P(f"warm-ups valid: {sum(1 for p in warm if not p['valid_why'])} of {len(warm)} (untimed; not used)")
unclean = [p for p in timed if p['clean_why']]
P(f'unclean processes: {len(unclean)}')
for p in unclean:
    P(f"  p{p['pass']} r{p['round']} seq {p['seq']} {p['row']}#{p['binary']}@W{p['W']} {p['attempt']} {p['start']}: "
      f"{p['clean_why']}")
nopl = [p for p in timed if not p['placement_present']]
P(f'processes without a placement receipt (main_cycle_frac + top3): {len(nopl)}')
P(f"processes with main_share_top_est null (main_cpu_s 0 at a 15.6-ms tick): "
  f"{sum(1 for p in timed if p['placement']['main_share_top_est'] is None)}")

slots = collections.defaultdict(list)
for p in timed:
    slots[(p['pass'], p['round'], p['row'], p['binary'], p['W'])].append(p)
used, dropped, replaced = [], [], []
for k, ps in sorted(slots.items(), key=lambda kv: str(kv[0])):
    orig = [p for p in ps if p['attempt'] == 'original']
    rer = [p for p in ps if p['attempt'] == 'rerun']
    pick = None
    for cand in orig + rer:
        if not cand['valid_why'] and not cand['clean_why']:
            pick = cand
            break
    if pick is None:
        dropped.append(k)
        continue
    pick['used'] = True
    used.append(pick)
    if pick['attempt'] == 'rerun':
        replaced.append((k, [o['clean_why'] for o in orig]))
P(f'slots {len(slots)}; used {len(used)}; dropped {len(dropped)}; re-runs used in place of a hot original: {len(replaced)}')
for k, w in replaced:
    P(f'  slot {k}: original {w} -> re-run used')
cells = collections.Counter((p['row'], p['binary'], p['W']) for p in used)
P(f'cells {len(cells)}; K per cell: {sorted(set(cells.values()))}')
perpass = collections.Counter((p['row'], p['binary'], p['W'], p['pass']) for p in used)
P(f'K per (cell, pass): {sorted(set(perpass.values()))}')

# pose / manifolds identity across the processes of one scene (bit-identical trajectory => identical manifolds)
for scene_rows, name in ((('S7-JT', 'S7-JT-W1') + tuple(f'S7-ladder{w}-F{f}' for w in (8, 16) for f in ('0.5', '1', '1.5', '2')), 'J-T'),
                         (('S7-JA', 'S7-JA-ladder16-F1'), 'J-A')):
    ps = [p for p in used if p['row'] in scene_rows]
    P(f"{name}: {len(ps)} used processes; pose hashes {sorted(set(p['pose_hash'] for p in ps))}; distinct per-step "
      f"manifold sequences {len(set(p['manifolds_seq_hash'] for p in ps))}; manifolds/step [0,500) "
      f"{sorted(set(round(p['manifolds']['0..500'], 4) for p in ps))}, [100,500) "
      f"{sorted(set(round(p['manifolds']['100..500'], 4) for p in ps))}")
hosts = collections.Counter(json.dumps(p.get('host'), sort_keys=True) for p in used)
P(f'SUMMARY host objects: {dict(hosts)}')
mism = [p for p in used if abs(p['wall_ms']['0..500'] - p['mean_ms']) > 1e-9]
P(f"per-process [0,500) means equal the driver's mean_ms: {len(used) - len(mism)}/{len(used)}")

for p in procs:
    p.pop('manifolds_seq_hash', None)
json.dump({'procs': procs}, open(os.path.join(L.HERE, 's7_procs.json'), 'w', encoding='utf-8'), indent=0)
open(os.path.join(L.HERE, 's7_select.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
