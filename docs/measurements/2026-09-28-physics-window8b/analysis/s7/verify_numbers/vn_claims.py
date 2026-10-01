"""Independent recomputation of S7-AB C1/C2/C3 and ladders from raw/ (ruling 1 rule: claim = i AND s in every clean
pass (K=3) AND pooled (K=9), one sign; r reported, all three = STRONG)."""
import collections
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from vn_lib import (load_block, validate, dirty, WINDOWS, wmean, stats, compare, yn)  # noqa: E402

OUT = os.path.dirname(os.path.abspath(__file__))
recs = load_block('S7-AB')
timed = [r for r in recs if r.get('timed')]
warm = [r for r in recs if not r.get('timed')]
print(f'S7-AB records: {len(recs)} ({len(timed)} timed, {len(warm)} warm-up)')

# ---- validity and cleanliness from files ----
info = {}
nvalid = 0
for r in timed:
    why, s, rows = validate(r)
    dw = dirty(r)
    key = (r['pass'], r['round'], r['row'], r['binary'], r['W'], r['attempt'])
    assert key not in info, key
    info[key] = {'rec': r, 'why': why, 'dirty': dw, 's': s, 'rows': rows}
    nvalid += not why
    if why or dw:
        print('  ', key, 'INVALID' if why else 'valid', why, 'DIRTY' if dw else 'clean', dw,
              'driver valid', r.get('valid'), 'driver contaminated', r.get('contaminated'))
print(f'valid from files: {nvalid}/{len(timed)}')
bad_drv = [k for k, v in info.items() if bool(not v['why']) != bool(v['rec'].get('valid'))]
print('validity disagreements with the driver:', bad_drv)

# ---- slot selection ----
slots = collections.defaultdict(dict)
for k, v in info.items():
    p, rd, row, b, W, att = k
    slots[(p, rd, row, b, W)][att] = v
sel = {}
sel_orig = {}
for sk, d in slots.items():
    o = d.get('original')
    rr = d.get('rerun')
    if o and not o['why'] and not o['dirty']:
        sel[sk] = o
    elif rr and not rr['why']:
        sel[sk] = rr
    elif o and not o['why']:
        sel[sk] = o
    if o and not o['why']:
        sel_orig[sk] = o
used_rerun = [sk for sk, v in sel.items() if v['rec']['attempt'] == 'rerun']
print('slots:', len(slots), 'selected:', len(sel), 're-runs used:', used_rerun)
for sk, v in sel.items():
    if v['dirty']:
        print('  SELECTED BUT DIRTY', sk, v['dirty'])

# ---- manifolds sequence identity ----
fam = collections.defaultdict(set)
for sk, v in sel.items():
    fk = 'JA' if 'JA' in v['rec']['row'] else 'JT'
    fam[fk].add(tuple(int(x['manifolds']) for x in v['rows']))
print('distinct per-step manifold sequences per scene family:', {k: len(v) for k, v in fam.items()})
if all(len(v) == 1 for v in fam.values()) and len(fam) == 2:
    print('JT and JA manifold sequences identical to each other:', list(fam['JT'])[0] == list(fam['JA'])[0])


def pm(v, win):
    lo, hi = WINDOWS[win]
    return wmean(v['rows'], lo, hi) / 1e6


def cellvals(selmap, row, b, W, win, passes=(0, 1, 2)):
    xs = []
    for (p, rd, rw, bb, WW), v in selmap.items():
        if rw == row and bb == b and WW == W and p in passes:
            xs.append(pm(v, win))
    return xs


TAGS = ('pooled', 'p0', 'p1', 'p2')


def judge(selmap, A, B, win):
    res = {}
    for tag, passes in (('pooled', (0, 1, 2)), ('p0', (0,)), ('p1', (1,)), ('p2', (2,))):
        a = stats(cellvals(selmap, *A, win, passes))
        b = stats(cellvals(selmap, *B, win, passes))
        res[tag] = {'A': a, 'B': b, 'c': compare(a, b)}
    signs = {res[t]['c']['sign'] for t in TAGS}
    cl = all(res[t]['c']['fl']['i'] and res[t]['c']['fl']['s'] for t in TAGS) and len(signs) == 1
    st = cl and all(res[t]['c']['fl']['r'] for t in TAGS)
    stp = cl and res['pooled']['c']['fl']['r']
    clp = res['pooled']['c']['fl']['i'] and res['pooled']['c']['fl']['s']
    binding = max(max(res[t]['c']['bars']['i'], res[t]['c']['bars']['s']) for t in TAGS)
    return res, cl, st, stp, clp, binding


def show(name, res, cl, st, stp, clp, binding):
    po = res['pooled']
    a, b, c = po['A'], po['B'], po['c']
    print(f"{name}: A {a['med']:.4f} [{a['min']:.4f}-{a['max']:.4f}] IQR {a['IQR']:.4f} SE {a['SE']:.4f} K={a['K']} | "
          f"B {b['med']:.4f} [{b['min']:.4f}-{b['max']:.4f}] IQR {b['IQR']:.4f} SE {b['SE']:.4f} K={b['K']}")
    print(f"   pooled B/A {c['ratio']:.4f} (B-A {c['delta']:+.4f} ms) {yn(c['fl'])} bars r {100*c['bars']['r']:.2f} "
          f"i {100*c['bars']['i']:.2f} s {100*c['bars']['s']:.2f} %")
    for t in ('p0', 'p1', 'p2'):
        cc = res[t]['c']
        print(f"   {t}: A {res[t]['A']['med']:.4f} B {res[t]['B']['med']:.4f} K={res[t]['A']['K']}/{res[t]['B']['K']} "
              f"B/A {cc['ratio']:.4f} ({cc['delta']:+.4f}) {yn(cc['fl'])} bars r {100*cc['bars']['r']:.2f} "
              f"i {100*cc['bars']['i']:.2f} s {100*cc['bars']['s']:.2f} %")
    print(f"   CLAIMED(i&s every pass+pooled, one sign)={cl} STRONG(r every pass+pooled)={st} "
          f"STRONG(r pooled)={stp} pooled-only i&s={clp} binding max(i,s) bar {100*binding:.2f} % "
          f"= {binding*a['med']:.4f} ms of A")


CLAIMS = {
    'C1-JT': (('S7-JT', 's7t', 8), ('S7-JT', 's7t', 16)),
    'C1-JA': (('S7-JA', 's7t', 8), ('S7-JA', 's7t', 16)),
    'C2-JT': (('S7-JT', 's7p', 8), ('S7-JT', 's7t', 8)),
    'C2-JA': (('S7-JA', 's7p', 8), ('S7-JA', 's7t', 8)),
    'C3-JT': (('S7-JT', 's7p', 16), ('S7-JT', 's7t', 16)),
    'PH-P16vsP8-JT': (('S7-JT', 's7p', 8), ('S7-JT', 's7p', 16)),
    'PH-P16vsP8-JA': (('S7-JA', 's7p', 8), ('S7-JA', 's7p', 16)),
    'PH-JA-P16vsT16': (('S7-JA', 's7p', 16), ('S7-JA', 's7t', 16)),
    'PH-W1-PvsT': (('S7-JT-W1', 's7p', 1), ('S7-JT-W1', 's7t', 1)),
}
LADDERS = {
    'ladder8': [(f, ('S7-ladder8-F' + f, 's7p', 8), ('S7-JT', 's7p', 8), float(f) * 0.060) for f in ('0.5', '1', '1.5', '2')],
    'ladder16': [(f, ('S7-ladder16-F' + f, 's7p', 16), ('S7-JT', 's7p', 16), float(f) * 0.105) for f in ('0.5', '1', '1.5', '2')],
    'JA-ladder16': [('1', ('S7-JA-ladder16-F1', 's7p', 16), ('S7-JA', 's7p', 16), 0.105)],
}

results = {}
for win in ('0..500', '0..100', '100..500'):
    for label, selmap in (('SELECTED', sel), ('ORIGINALS-ONLY', sel_orig)):
        if label == 'ORIGINALS-ONLY' and win != '0..500':
            continue
        print(f"\n===== window [{win}) , {label} =====")
        for name, (A, B) in CLAIMS.items():
            res, cl, st, stp, clp, bind = judge(selmap, A, B, win)
            show(name, res, cl, st, stp, clp, bind)
            slower = res['pooled']['c']['sign'] > 0
            if name.startswith('C1') or name.startswith('C2'):
                if cl and slower:
                    verdict = 'FAILS (B claimed slower)'
                else:
                    verdict = 'HOLDS' + (' (B claimed faster)' if cl else ' (no resolved difference)')
            elif name == 'C3-JT':
                gain = -res['pooled']['c']['delta']
                if cl and not slower:
                    verdict = f'gain claimed {gain:.4f} ms ' + ('>= bar 0.105' if gain >= 0.105 else '< bar 0.105')
                elif cl and slower:
                    verdict = f'REFUTED: T16 claimed SLOWER than P16 by {-gain:.4f} ms'
                else:
                    verdict = 'not claimed'
            else:
                verdict = 'post hoc: ' + ('claimed' if cl else 'not claimed')
            print('   VERDICT:', verdict)
            results['|'.join((win, label, name))] = {
                'A': res['pooled']['A']['med'], 'B': res['pooled']['B']['med'], 'ratio': res['pooled']['c']['ratio'],
                'delta': res['pooled']['c']['delta'], 'claimed': cl, 'strong': st, 'strong_pooled': stp,
                'binding': bind, 'verdict': verdict,
                'per_pass_ratio': [res[t]['c']['ratio'] for t in ('p0', 'p1', 'p2')],
                'flags': {t: yn(res[t]['c']['fl']) for t in TAGS}}
        print(f"  -- ladders [{win}) {label} --")
        for lname, rungs in LADDERS.items():
            seen = []
            for f, rung, ref, inj in rungs:
                res, cl, st, stp, clp, bind = judge(selmap, ref, rung, win)
                rise = res['pooled']['c']['delta']
                ok = cl and res['pooled']['c']['sign'] > 0
                seen.append((inj, ok))
                print(f"   {lname} F{f} inj {inj:.4f} rise {rise:+.4f} ({rise/inj:.2f}x) pooled {yn(res['pooled']['c']['fl'])} "
                      f"p0 {yn(res['p0']['c']['fl'])} p1 {yn(res['p1']['c']['fl'])} p2 {yn(res['p2']['c']['fl'])} seen={ok}")
            R = None
            for idx in range(len(seen)):
                if all(s for _, s in seen[idx:]):
                    R = seen[idx][0]
                    break
            print(f"   {lname}: R = {R}")
            results['|'.join((win, label, lname))] = R

json.dump(results, open(os.path.join(OUT, 'vn_results.json'), 'w'), indent=1)
