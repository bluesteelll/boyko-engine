"""Selection, validity, cleanliness and per-process statistics for the F3 block (independent)."""
import collections
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vlib as L

recs = L.load_recs()
f3 = [r for r in recs if r.get('block') == 'F3' and r.get('row')]
timed = [r for r in f3 if r.get('attempt') != 'warmup']
warm = [r for r in f3 if r.get('attempt') == 'warmup']
print('F3 runner records:', len(f3), 'timed:', len(timed), 'warm-ups:', len(warm),
      'originals:', sum(r['attempt'] == 'original' for r in timed), 're-runs:', sum(r['attempt'] == 'rerun' for r in timed))

SPANS = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')
procs = []
missing_span_steps = 0
queried_vals = collections.Counter()
for r in f3:
    d = r['cwd']
    s = L.summary(d)
    cp = os.path.join(d, 'run.csv')
    c = L.load_csv(cp) if os.path.exists(cp) else None
    why = L.validate(r, s, c)
    p = {k: r.get(k) for k in ('pass', 'round', 'row', 'W', 'attempt', 'seq', 'start', 'others_busy_pct', 'cwd', 'binary')}
    p['valid_why'] = why
    p['clean_why'] = L.clean_why(r)
    p['rb'] = (r.get('receipt_before') or {}).get('cpu_avg')
    p['ra'] = (r.get('receipt_after') or {}).get('cpu_avg')
    p['witness_top'] = [(o['name'], o['cpu_s']) for o in (r.get('others_top5') or [])[:1]]
    p['placement'] = r.get('placement')
    p['pose_hash'] = (s or {}).get('pose_hash')
    if c is not None:
        w = c['wall_ns']
        p['wall_ms'] = {'0..500': statistics.fmean(w[0:500]) / 1e6, '0..100': statistics.fmean(w[0:100]) / 1e6,
                        '100..500': statistics.fmean(w[100:500]) / 1e6}
        if L.ARMED[r['row']]:
            sp = {}
            for k in SPANS:
                sp[k] = c[k][100:500]
            miss = sum(1 for i in range(400) if any(sp[k][i] is None for k in SPANS))
            missing_span_steps += miss
            q = [x for x in sp['phys_bp_query_ns'] if x is not None]
            b = [x for x in sp['phys_bp_build_ns'] if x is not None]
            qb = [sp['phys_bp_query_ns'][i] + sp['phys_bp_build_ns'][i] for i in range(400)
                  if sp['phys_bp_query_ns'][i] is not None and sp['phys_bp_build_ns'][i] is not None]
            four = [sum(sp[k][i] for k in SPANS) for i in range(400) if all(sp[k][i] is not None for k in SPANS)]
            p['t_q_us'] = statistics.median(q) / 1e3
            p['t_b_us'] = statistics.median(b) / 1e3
            p['t_qb_us'] = statistics.median(qb) / 1e3
            p['four_us'] = statistics.median(four) / 1e3
            p['n_steps_span'] = (len(q), len(b), len(qb), len(four))
            # post hoc: process means too
            p['t_q_mean_us'] = statistics.fmean(q) / 1e3
            p['t_b_mean_us'] = statistics.fmean(b) / 1e3
            p['t_qb_mean_us'] = statistics.fmean(qb) / 1e3
            p['four_mean_us'] = statistics.fmean(four) / 1e3
            p['verify_us'] = statistics.median([x for x in sp['phys_bp_verify_ns'] if x is not None]) / 1e3
            p['assemble_us'] = statistics.median([x for x in sp['phys_bp_assemble_ns'] if x is not None]) / 1e3
            for v in c['phys_bp_queried'][100:500]:
                queried_vals[v] += 1
    procs.append(p)

print('missing span steps in [100,500) over all armed processes:', missing_span_steps)
print('phys_bp_queried values over [100,500) on armed processes:', dict(queried_vals))
bad = [(p['pass'], p['round'], p['row'], p['W'], p['attempt'], p['valid_why']) for p in procs if p['valid_why']]
print('validity problems (incl. warm-ups):', len(bad))
for b in bad:
    print('  ', b)

# slot rule: original if valid and clean, else its re-run if valid and clean, else dropped
slots = collections.defaultdict(list)
for p in procs:
    if p['attempt'] == 'warmup':
        continue
    slots[(p['pass'], p['round'], p['row'], p['W'])].append(p)
used, dropped = [], []
for k, ps in sorted(slots.items(), key=lambda kv: str(kv[0])):
    orig = [p for p in ps if p['attempt'] == 'original']
    rer = [p for p in ps if p['attempt'] == 'rerun']
    assert len(orig) == 1 and len(rer) <= 1, (k, len(orig), len(rer))
    pick = None
    for cand in orig + rer:
        if not cand['valid_why'] and not cand['clean_why']:
            pick = cand
            break
    if pick:
        pick['used'] = True
        used.append(pick)
    else:
        dropped.append((k, ps))
print('slots:', len(slots), 'used:', len(used), 'used by re-run:', sum(p['attempt'] == 'rerun' for p in used),
      'dropped:', len(dropped))
reasons = collections.Counter()
for k, ps in dropped:
    rr = [p for p in ps if p['attempt'] == 'rerun']
    if not rr:
        why = 'no re-run'
    else:
        why = ','.join(sorted(set(w[0] for w in rr[0]['clean_why']))) + (' +invalid' if rr[0]['valid_why'] else '')
    reasons[why] += 1
    print('  dropped', k, 're-run clean_why:', rr[0]['clean_why'] if rr else None,
          'orig clean_why:', [p['clean_why'] for p in ps if p['attempt'] == 'original'][0])
print('drop reasons (re-run):', dict(reasons))
# rerun count consistency: every re-run has an unclean or invalid original
for k, ps in slots.items():
    o = [p for p in ps if p['attempt'] == 'original'][0]
    if any(p['attempt'] == 'rerun' for p in ps) and not (o['clean_why'] or o['valid_why']):
        print('  re-run without cause:', k)
unclean = [p for p in procs if p['attempt'] != 'warmup' and p['clean_why']]
print('unclean timed processes:', len(unclean))
tops = collections.Counter((p['witness_top'][0][0] if p['witness_top'] else None) for p in unclean)
print('  top other process among unclean:', dict(tops))

# n per cell and K per pass-cell
cells = collections.defaultdict(lambda: [0, 0, 0])
for p in used:
    cells[(p['row'], p['W'])][p['pass']] += 1
k2 = 0
short = 0
for k in sorted(cells, key=str):
    v = cells[k]
    n = sum(v)
    if n < 9:
        short += 1
    k2 += sum(1 for x in v if x < 3)
    print(f'  cell {k[0]:24s} W{k[1]:<3d} n={n} per pass {v}')
print('cells:', len(cells), 'cells with n<9:', short, 'pass-cells with K<3:', k2,
      'K distribution:', collections.Counter(x for v in cells.values() for x in v))
json.dump({'procs': procs}, open(os.path.join(L.HERE, 'v_procs.json'), 'w'), default=str)
