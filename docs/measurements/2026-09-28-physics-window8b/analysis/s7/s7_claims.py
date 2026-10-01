"""S7-AB pre-registered claims C1, C2, C3 and the resolution ladders (cut.md §5 item 3 as amended), under ruling 1.
Reads s7_procs.json (s7_select.py). Writes s7_claims.txt and s7_results.json."""
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import s7lib as L  # noqa: E402

PROCS = [p for p in json.load(open(os.path.join(L.HERE, 's7_procs.json'), encoding='utf-8'))['procs'] if p.get('used')]
PASSES = (0, 1, 2)
OUT = []
RES = {}


def P(s=''):
    OUT.append(s)
    print(s)


def vals(row, b, W, win, passes=PASSES):
    return [p['wall_ms'][win] for p in PROCS if p['row'] == row and p['binary'] == b and p['W'] == W
            and p['pass'] in passes]


def cellof(row, b, W, win, passes=PASSES):
    return L.cell(vals(row, b, W, win, passes))


def judge(a, b, win):
    """a, b = (row, binary, W). Returns ruling1's result plus the cells; B against A."""
    ap, bp = cellof(*a, win), cellof(*b, win)
    apass = {q: cellof(*a, win, (q,)) for q in PASSES}
    bpass = {q: cellof(*b, win, (q,)) for q in PASSES}
    j = L.ruling1(ap, bp, apass, bpass)
    j['A_cell'], j['B_cell'], j['A_pass'], j['B_pass'] = ap, bp, apass, bpass
    thr = max(max(c['bar_i'], c['bar_s']) for c in [j['pooled']] + list(j['per'].values()))
    j['rule_threshold_rel'] = thr
    j['rule_threshold_ms'] = thr * ap['median']
    return j


def show(tag, j):
    c = j['pooled']
    P(f"  {tag}: A {L.fcell(j['A_cell'])}")
    P(f"  {' ' * len(tag)}  B {L.fcell(j['B_cell'])}")
    P(f"  {' ' * len(tag)}  pooled B/A {L.fcmp(c)}")
    for q, cq in j['per'].items():
        P(f"  {' ' * len(tag)}  p{q}: A {j['A_pass'][q]['median']:.4f} B {j['B_pass'][q]['median']:.4f} "
          f"(A r {100 * j['A_pass'][q]['r']:.2f} / i {100 * j['A_pass'][q]['i']:.2f} / s {100 * j['A_pass'][q]['s']:.2f}; "
          f"B r {100 * j['B_pass'][q]['r']:.2f} / i {100 * j['B_pass'][q]['i']:.2f} / s {100 * j['B_pass'][q]['s']:.2f} %) "
          f"B/A {L.fcmp(cq)}")
    P(f"  {' ' * len(tag)}  ruling 1: direction {j['direction']}, same sign in every pass {j['same_sign']}, "
      f"CLAIMED {j['claimed']}, STRONG (r every pass + pooled) {j['strong_all']}, STRONG (r pooled) {j['strong_pooled_r']}; "
      f"the rule's binding threshold here {100 * j['rule_threshold_rel']:.2f} % = {j['rule_threshold_ms']:.4f} ms of A")


def ser(j):
    keep = {k: v for k, v in j.items() if k not in ('A_pass', 'B_pass')}
    keep['A_pass'] = {str(q): {k: v for k, v in c.items()} for q, c in j['A_pass'].items()}
    keep['B_pass'] = {str(q): {k: v for k, v in c.items()} for q, c in j['B_pass'].items()}
    keep['per'] = {str(q): c for q, c in j['per'].items()}
    return keep


WINDOWS = ('0..500', '0..100', '100..500')
T8JT, T16JT = ('S7-JT', 's7t', 8), ('S7-JT', 's7t', 16)
P8JT, P16JT = ('S7-JT', 's7p', 8), ('S7-JT', 's7p', 16)
T8JA, T16JA = ('S7-JA', 's7t', 8), ('S7-JA', 's7t', 16)
P8JA, P16JA = ('S7-JA', 's7p', 8), ('S7-JA', 's7p', 16)
CLAIMS = {
    'C1-JT': (T8JT, T16JT, 'T(16) vs T(8), J-T: C1 holds unless T(16) is CLAIMED slower'),
    'C1-JA': (T8JA, T16JA, 'T(16) vs T(8), J-A: C1 holds unless T(16) is CLAIMED slower'),
    'C2-JT': (P8JT, T8JT, 'T(8) vs P(8), J-T: C2 holds unless T(8) is CLAIMED slower'),
    'C2-JA': (P8JA, T8JA, 'T(8) vs P(8), J-A: C2 holds unless T(8) is CLAIMED slower'),
    'C3-JT': (P16JT, T16JT, 'T(16) vs P(16), J-T: C3 = P(16) - T(16), claimed if T(16) is CLAIMED faster'),
}

# ---------------------------------------------------------------- ladders first (R_8, R_16 are reported with C1-C3)
LADDERS = {
    'ladder8': (P8JT, [(f, f'S7-ladder8-F{f}') for f in ('0.5', '1', '1.5', '2')], 60000),
    'ladder16': (P16JT, [(f, f'S7-ladder16-F{f}') for f in ('0.5', '1', '1.5', '2')], 105000),
    'JA-ladder16': (P16JA, [('1', 'S7-JA-ladder16-F1')], 105000),
}
RES['ladders'] = {}
for win in WINDOWS:
    P(f'\n# Resolution ladders, window [{win}) (reference = P plain row at the same W; a rung is SEEN iff its rise is '
      f'CLAIMED under ruling 1 with the rung slower)')
    RES['ladders'][win] = {}
    for name, (ref, rungs, ref_ns) in LADDERS.items():
        seen = {}
        det = {}
        P(f'## {name} (ref {ref[0]}#{ref[1]}@W{ref[2]}, rung = F x {ref_ns / 1e3:.0f} us per step)')
        for f, rid in rungs:
            j = judge(ref, (rid, ref[1], ref[2]), win)
            inj = float(f) * ref_ns / 1e6
            ok = j['claimed'] and j['pooled']['ratio'] > 1
            seen[float(f)] = ok
            det[f] = {'injected_ms': inj, 'rise_ms': j['pooled']['delta'], 'seen': ok, 'strong_all': j['strong_all'],
                      'judge': ser(j)}
            show(f'{rid} (inj {inj:.4f} ms)', j)
            P(f"      -> rise {j['pooled']['delta']:+.4f} ms, rise/injected {j['pooled']['delta'] / inj:.3f}: "
              f"{'SEEN' if ok else 'not seen'}")
        ks = sorted(seen)
        demo = [k for k in ks if all(seen[q] for q in ks if q >= k)]
        rk = min(demo) if demo else None
        R = rk * ref_ns / 1e6 if rk else None
        P(f"  => R ({name}) = {'%.4f ms (F %s)' % (R, rk) if R else 'NOT DEMONSTRATED (the largest rung is not seen)'}")
        RES['ladders'][win][name] = {'R_ms': R, 'F': rk, 'rungs': det}

# ---------------------------------------------------------------- the claims
RES['claims'] = {}
for win in WINDOWS:
    P(f'\n# Claims, window [{win}) (B against A; ms per step; cell = median over K of process means)')
    RES['claims'][win] = {}
    for cid, (a, b, what) in CLAIMS.items():
        j = judge(a, b, win)
        P(f'## {cid}: {what}')
        show(f'{b[0]}#{b[1]}@W{b[2]} vs {a[0]}#{a[1]}@W{a[2]}', j)
        slower = j['claimed'] and j['pooled']['ratio'] > 1
        faster = j['claimed'] and j['pooled']['ratio'] < 1
        if cid.startswith('C3'):
            gain = -j['pooled']['delta']
            st = 'CLAIMED (T faster)' if faster else ('REFUTED (T slower claimed)' if slower else 'NOT CLAIMED')
            P(f"  => P(16) - T(16) = {gain:+.4f} ms ({100 * (1 - j['pooled']['ratio']):+.2f} % of P(16)); {st}; "
              f"bar 0.105 ms {'met' if gain >= 0.105 else 'NOT met'} by the point value; predicted +0.166..+0.175")
            RES['claims'][win][cid] = {'status': st, 'gain_ms': gain, 'judge': ser(j)}
        else:
            if slower:
                st = 'REFUTED: B slower CLAIMED'
            elif faster:
                st = 'HOLDS: B faster CLAIMED (the opposite of slower)'
            else:
                st = 'HOLDS: no resolved difference'
            P(f'  => {st}')
            RES['claims'][win][cid] = {'status': st, 'judge': ser(j)}

open(os.path.join(L.HERE, 's7_claims.txt'), 'w', encoding='utf-8').write('\n'.join(OUT) + '\n')
json.dump(RES, open(os.path.join(L.HERE, 's7_results.json'), 'w', encoding='utf-8'), indent=1)
