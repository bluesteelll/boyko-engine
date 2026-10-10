"""J-Son-T (L10 C1b product row): sleeping ON (JSonT, JSonT-a) vs OFF (JSoffT, JSoffT-a), cfg a, TREE broadphase,
TIP binary, W1/W8, metric windows [0,100), [100,500), [274,500) (CSV row index = step, 0-based; the driver's
col_stats slices [a:b], window 8's convention). Writes jsont.txt and jsont.json."""
import json
import os
import statistics
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import libjd as L

OUT = []


def P(s=''):
    OUT.append(s)
    print(s)


WINS = {'0..100': (0, 100), '100..500': (100, 500), '274..500': (274, 500), '0..500': (0, 500),
        '100..274': (100, 274), '277..500': (277, 500)}
PREREG = ('0..100', '100..500', '274..500')

recs = L.load_recs('J-Son-T')
procs, used, dropped = L.select(recs, L.validate_runner)
P('# J-Son-T: sleeping ON vs OFF on the tree broadphase (TIP 16191fda, cfg a, gap 0.5), K = 9 = 3 passes x 3 rounds')
P(f'records (non-warm-up) {len(procs)}; slots {len(used) + len(dropped)}; used {len(used)}; dropped slots {len(dropped)}')
bad = [(p['pass'], p['round'], p['row'], p['W'], p['attempt'], p['valid_why']) for p in procs if p['valid_why']]
P(f'validity problems (independent re-check): {len(bad)}')
for b in bad:
    P(f'  INVALID {b}')
for k, why in dropped:
    P(f'  DROPPED slot pass {k[0]} round {k[1]} {k[2]}@W{k[4]} seq {k[5]}: {why}')
unclean = [(p['pass'], p['round'], p['row'], p['W'], p['attempt'], p['clean_why']) for p in procs if p['clean_why']]
P(f'unclean processes: {len(unclean)}')
for u in unclean:
    P(f'  {u}')
reruns_used = [(p['pass'], p['round'], p['row'], p['W']) for p in used if p['attempt'] == 'rerun']
P(f're-runs used: {len(reruns_used)} {reruns_used}')
# driver-mean cross-check and the per-process statistics
mism = 0
for p in used + [q for q in procs if not q.get('used')]:
    c = p['_c']
    if c is None:
        continue
    st = {}
    for wn, (a, b) in WINS.items():
        st[wn] = {h: L.mean(v[a:b]) for h, v in c.items() if h not in ('step', 'top_y', 'void')}
    p['stats'] = st
    p['wall_ms'] = {wn: st[wn]['wall_ns'] / 1e6 for wn in WINS}
    for wn in ('0..500', '0..100', '100..500', '274..500'):
        dv = (p.get('cols') or {}).get(wn, {}).get('wall_ns', {}).get('mean')
        if dv is None or abs(dv - st[wn]['wall_ns']) > 1.0:
            mism += 1
P(f'per-process means vs the driver cols (4 windows x {len(procs)} processes): mismatches {mism}')
P('receipts of used processes: before max %.2f, after max %.2f, witness max %.2f %%' % (
    max(p['receipt_before']['cpu_avg'] for p in used), max(p['receipt_after']['cpu_avg'] for p in used),
    max(p['others_busy_pct'] for p in used)))
P('first_frozen_step (SUMMARY) on ON rows: ' + str(sorted(set(p['_s']['first_frozen_step'] for p in used if p['row'].startswith('JSon')))))
P('pose hashes: ' + str(sorted(set((p['row'], p['_s']['pose_hash']) for p in used))))


def sel(row, W, passes=None, pool=None):
    pool = used if pool is None else pool
    return [p for p in pool if p['row'] == row and p['W'] == W and (passes is None or p['pass'] in passes)]


def C(row, W, f, passes=None, pool=None):
    return L.cell([f(p) for p in sel(row, W, passes, pool)])


def wall(win):
    return lambda p: p['wall_ms'][win]


RES = {}
P('\n## Cells (ms/step; cell = median over K of process means) and ON/OFF under ruling 1')
for arm, (on, off) in (('disarmed', ('JSonT', 'JSoffT')), ('armed', ('JSonT-a', 'JSoffT-a'))):
    for W in (1, 8):
        for win in list(PREREG) + ['0..500', '100..274', '277..500']:
            a = C(off, W, wall(win))
            b = C(on, W, wall(win))
            pooled = L.cmp_(a, b)
            pp = []
            for ps in (0, 1, 2):
                ca = C(off, W, wall(win), [ps])
                cb = C(on, W, wall(win), [ps])
                pp.append((ps, ca, cb, L.cmp_(ca, cb), L.cmp_(ca, cb, min_k=2)))
            v, sign = L.ruling1(pooled, [x[3] for x in pp])
            v2, sign2 = L.ruling1(pooled, [x[4] for x in pp])
            tag = '' if win in PREREG else ' (post hoc window)'
            P(f'\n[{win}) W{W} {arm}{tag}')
            P(f'  OFF {off}: {L.fcell(a)}')
            P(f'  ON  {on}:  {L.fcell(b)}')
            P(f'  ON/OFF pooled: {L.fcmp(pooled)}')
            for ps, ca, cb, c3, c2 in pp:
                P(f'  pass {ps}: OFF {ca["median"]:.4f} (K {ca["K"]}, IQR {100 * ca["i"]:.2f} %, SE {100 * ca["s"]:.2f} %) '
                  f'ON {cb["median"]:.4f} (K {cb["K"]}, IQR {100 * cb["i"]:.2f} %, SE {100 * cb["s"]:.2f} %) -> {L.fcmp(c3)}')
                if not c3['k_ok']:
                    P(f'          same pass with K >= 2 allowed: {L.fcmp(c2)}')
            dirn = '' if sign is None else ('ON slower' if sign > 0 else 'ON faster')
            dirn2 = '' if sign2 is None else ('ON slower' if sign2 > 0 else 'ON faster')
            P(f'  RULING 1 (K >= 3 per pass): {v} {dirn}; with a K = 2 pass admitted: {v2} {dirn2}')
            RES[f'{arm}|W{W}|{win}'] = {'off': a, 'on': b, 'pooled': pooled, 'passes': [x[3] for x in pp],
                                         'passes_k2': [x[4] for x in pp], 'verdict': v, 'sign': sign,
                                         'verdict_k2': v2, 'sign_k2': sign2}

json.dump({k: {kk: vv for kk, vv in v.items()} for k, v in RES.items()},
          open(os.path.join(L.HERE, 'jsont.json'), 'w'), indent=1, default=str)
json.dump([{k: p.get(k) for k in ('pass', 'round', 'seq', 'row', 'W', 'attempt', 'start', 'others_busy_pct',
                                  'placement', 'wall_ms', 'valid_why', 'clean_why', 'used')}
           for p in procs], open(os.path.join(L.HERE, 'jsont_procs.json'), 'w'), indent=1, default=str)
open(os.path.join(L.HERE, 'jsont.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
