"""r1: the pre-registered claims of window9a_rows.md: R-ARM(cfg, W); R-WALL-D / R-WALL-M / R-ROW-D / R-ROW-M (W),
windows [100,500) (primary) and [0,100) (claimed under the same rule, reported separately); [0,500) is post hoc.
Ours = C4-JT#tip at W 1/8/16 and C4-JD#tip at W 2/4 (C4-AB; the tip A/A twin of J-T, orchestrator brief).
Passes are paired by index across blocks (C4-AB pN with C4-RAPIER pN; both p0/p2 reversed, p1 forward).
Orientation (pre-registered text: ours J-T vs RP-D, rs8 vs rs4): A = the first named, B = the second.
Verdicts under LETTER (ruling 8: a K < 3 pass-cell sets no flag) and, labelled, EXCL (it is skipped)."""
import json
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lib_rp as L  # noqa: E402

o = L.Out()
recs = L.all_records()
_, U_R, _ = L.select('C4-RAPIER', recs)
_, U_A, _ = L.select('C4-AB', recs, {'C4-JT', 'C4-JD', 'C4-jolt56'})
OURS_ROWS = {(0, 100): 588764.88, (100, 500): 595935.18}  # r0: 36 * mean Np, C4-JD-armed#tip, identical in all 18
WS = (1, 2, 4, 8, 16)
CFG = {'D': 'RP-D', 'M': 'RP-M'}
ARMS = ('rs8', 'rs4')
WN = {(100, 500): '[100,500)', (0, 100): '[0,100)', (0, 500): '[0,500)'}


def ours_row(W):
    return ('C4-JT', 'tip') if W in (1, 8, 16) else ('C4-JD', 'tip')


def vals_ours(W, win, scale=1.0, row=None):
    rid, b = row or ours_row(W)
    return L.by_pass(U_A, rid, b, W, win, scale)


def vals_rap(cfg, arm, W, win, scale=1.0):
    return L.by_pass(U_R, CFG[cfg], arm, W, win, scale)


OUT = {'cells': {}, 'arm': {}, 'claims': {}}

o('=== cells (ms per step): median [min-max] n (IQR, SE, range) ; per-pass medians p0/p1/p2 ===')
for win in L.WINDOWS:
    o(f'--- window {WN[win]} ---')
    for W in WS:
        items = [(f'ours {ours_row(W)[0]}#tip', vals_ours(W, win))]
        items += [(f'RP-{c} {a}', vals_rap(c, a, W, win)) for c in 'DM' for a in ARMS]
        items += [('Jolt 5.6', L.by_pass(U_A, 'C4-jolt56', 'j56', W, win))]
        for name, v in items:
            c = L.cell([x for k in (0, 1, 2) for x in v[k]])
            pp = ' / '.join((f'{L.cell(v[k])["median"] / 1e6:.4f}(K{len(v[k])})' if v[k] else '-(K0)') for k in (0, 1, 2))
            o(f'  W{W:<2d} {name:18s} {L.fcell(c)}  passes {pp}')
            OUT['cells'][f'{name}|W{W}|{WN[win]}'] = {'median_ms': c['median'] / 1e6, 'min_ms': c['min'] / 1e6,
                                                      'max_ms': c['max'] / 1e6, 'K': c['K'], 'i': c['i'], 's': c['s'],
                                                      'r': c['r'], 'Kpass': [len(v[k]) for k in (0, 1, 2)]}

o()
o('=== R-ARM(cfg, W): A = rs8, B = rs4 (B/A > 1: rs8 faster) ===')
FASTER = {}
for win in L.WINDOWS:
    tag = '' if win != (0, 500) else '  (post hoc window)'
    o(f'--- window {WN[win]}{tag} ---')
    for c in 'DM':
        for W in WS:
            j = L.judge(vals_rap(c, 'rs8', W, win), vals_rap(c, 'rs4', W, win))
            if j['claimed']:
                f = 'rs8' if j['pooled']['ratio'] > 1 else 'rs4'
            else:
                f = 'tied'
            FASTER[(c, W, win)] = f
            o(f'  R-ARM({c}, W{W:<2d}) rs8 {j["a"]["median"] / 1e6:.4f} rs4 {j["b"]["median"] / 1e6:.4f}: {L.fj(j)} => faster arm: {f}')
            OUT['arm'][f'{c}|W{W}|{WN[win]}'] = {'rs8': j['a']['median'] / 1e6, 'rs4': j['b']['median'] / 1e6,
                                                'ratio_rs4_over_rs8': j['pooled']['ratio'], 'verdict': L.label(j),
                                                'faster': f, 'pooled_flags': L.yn(j['pooled']),
                                                'pass_flags': [L.fyn(x) for x in j['per']]}


def claim_block(kind, cfg, W, win, orient='ours_first', row=None):
    """kind WALL or ROW. Returns per-arm judgements and the combined pre-registered verdict."""
    res = {}
    for a in ARMS:
        if kind == 'WALL':
            vo = vals_ours(W, win, 1.0, row)
            vr = vals_rap(cfg, a, W, win)
        else:
            vo = vals_ours(W, win, OURS_ROWS[win], row)
            vr = vals_rap(cfg, a, W, win, L.RAPIER_ROWS[CFG[cfg]][win])
        res[a] = L.judge(vo, vr) if orient == 'ours_first' else L.judge(vr, vo)
    out = {}
    for reading, key, skey in (('LETTER', 'claimed', 'strong'), ('EXCL', 'claimed_excl', 'strong_excl')):
        ours_faster = []
        rap_faster = []
        for a in ARMS:
            j = res[a]
            r = j['pooled']['ratio']
            ours_is_faster = (r > 1) if orient == 'ours_first' else (r < 1)
            if j[key]:
                (ours_faster if ours_is_faster else rap_faster).append(a)
        if rap_faster:
            v = 'RAPIER FASTER CLAIMED vs ' + '+'.join(rap_faster)
        elif len(ours_faster) == len(ARMS):
            v = 'OURS FASTER CLAIMED vs every build'
        elif ours_faster:
            v = 'NOT CLAIMED (ours faster claimed vs ' + '+'.join(ours_faster) + ' only)'
        else:
            v = 'NOT CLAIMED'
        strong = [a for a in ARMS if res[a][skey]]
        out[reading] = v + ((' [STRONG vs ' + '+'.join(strong) + ']') if strong else '')
    return res, out


def cellsum(c):
    return {k: c[k] for k in ('K', 'median', 'min', 'max', 'i', 's', 'r')}


for orient in ('ours_first', 'rapier_first'):
    if orient == 'ours_first':
        hdr = 'A = ours, B = Rapier build: B/A = Rapier/ours (above 1: ours faster)'
    else:
        hdr = 'ROBUSTNESS, A = Rapier build, B = ours: B/A = ours/Rapier (below 1: ours faster)'
    o()
    o(f'=== R-WALL / R-ROW claims; {hdr} ===')
    for win in L.WINDOWS:
        kinds = ('WALL', 'ROW') if win != (0, 500) else ('WALL',)
        tag = '' if win != (0, 500) else '  (post hoc window: no row pin for Rapier; WALL only)'
        o(f'--- window {WN[win]}{tag} ---')
        for kind in kinds:
            for cfg in 'DM':
                for W in WS:
                    res, out = claim_block(kind, cfg, W, win, orient)
                    name = f'R-{kind}-{cfg}(W{W})'
                    for a in ARMS:
                        o(f'  {name:14s} vs {a}: {L.fj(res[a])}')
                    o(f'  {name:14s} => LETTER: {out["LETTER"]} | EXCL: {out["EXCL"]}')
                    key = f'{name}|{WN[win]}'
                    if orient == 'ours_first':
                        d = {'LETTER': out['LETTER'], 'EXCL': out['EXCL']}
                        for a in ARMS:
                            j = res[a]
                            d[a] = {'ratio_rapier_over_ours': j['pooled']['ratio'], 'pooled': L.yn(j['pooled']),
                                    'passes': [L.fyn(x) for x in j['per']], 'Ks': j['Ks'],
                                    'bars_pooled_isr': [j['pooled']['bar_i'], j['pooled']['bar_s'], j['pooled']['bar_r']],
                                    'ours_cell': cellsum(j['a']), 'rap_cell': cellsum(j['b'])}
                        OUT['claims'][key] = d
                    else:
                        OUT['claims'][key]['robust_rapier_first'] = {'LETTER': out['LETTER'], 'EXCL': out['EXCL']}

o()
o('=== POST HOC sensitivity: ours = C4-JD#tip at every W (the tip A/A twin; W16 has K 3/3/3), A = ours ===')
for win in ((100, 500), (0, 100)):
    o(f'--- window {WN[win]} ---')
    for kind in ('WALL', 'ROW'):
        for cfg in 'DM':
            for W in WS:
                res, out = claim_block(kind, cfg, W, win, 'ours_first', ('C4-JD', 'tip'))
                o(f'  R-{kind}-{cfg}(W{W}) JD vs rs8: {L.fj(res["rs8"])}')
                o(f'  R-{kind}-{cfg}(W{W}) JD vs rs4: {L.fj(res["rs4"])}')
                o(f'  => LETTER: {out["LETTER"]} | EXCL: {out["EXCL"]}')
                OUT['claims'][f'POSTHOC-JD R-{kind}-{cfg}(W{W})|{WN[win]}'] = out

o()
o('=== POST HOC: J-T#tip vs J-D#tip A/A (C4-AB, W 1/8/16), A = J-T, B = J-D ===')
for win in ((100, 500), (0, 100)):
    for W in (1, 8, 16):
        j = L.judge(L.by_pass(U_A, 'C4-JT', 'tip', W, win), L.by_pass(U_A, 'C4-JD', 'tip', W, win))
        o(f'  {WN[win]} W{W}: {L.fj(j)}')

json.dump(OUT, open(os.path.join(L.HERE, 'r1_claims.json'), 'w'), indent=1)
o.save('r1_claims.txt')
