"""Window 7 reduction (untimed; run after the driver). Reads raw/runs.jsonl (or test/window/runs.jsonl with --test),
selects per slot the original if valid and clean, else its re-run if valid and clean (voided passes and warm-ups
excluded; window 6's reduce6.py selection verbatim), and prints per cell: median over K, [min-max], and the three
relative spreads r (min-max range), i (IQR), s (SE of the median = 1.2533 * sd / sqrt K), each over the median.
B against A is CLAIMED iff |B/A - 1| > 2*hypot(sA, sB) under BOTH r and s (i printed).
P1 (the Jolt headline) is reduced per block (A, B) and pooled (K=12); a claim HOLDS iff it is claimed pooled AND in
each block separately, in the same direction (window 6's lesson: a +/-4-6 % window term at W8).
Writes raw/reduction.json and raw/tables.md.
"""
import json
import math
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
W7 = os.path.dirname(HERE)
TEST = '--test' in sys.argv or '--test-real' in sys.argv
RAW = os.path.join(W7, 'test', 'real' if '--test-real' in sys.argv else 'window') if TEST else os.path.join(W7, 'raw')
GATE = os.path.join(W7, 'gate')
OUT = []
WS = (1, 2, 4, 8, 16)
# window 3's JOLT56-T cells (D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/analysis.json, 'cells'):
# the cross-window reference for the window term only
WIN3_JOLT56 = {1: 9.8281962, 2: 5.7704603, 4: 3.5814395, 8: 2.5692559, 16: 2.3884695}
WIN3_JOLT56_MANIFOLDS = 8489.0
# window 6 (D:/wt/joltab/docs/measurements/2026-09-24-physics-window6/analysis.md:129-130): the L9b-only prediction
# 0.9897 x 4515 x (435.56 - [100, 60]) ns and the realized-gain bar 0.6 x its low end
L9_PRED = (1.4994, 1.6782)
L9_BAR = 0.8997


def P(s=''):
    OUT.append(s)
    print(s)


def se_med(xs):
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) if len(xs) >= 2 else 0.0


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    m = statistics.median(xs)
    d = abs(m) if m else float('inf')
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'range': xs[-1] - xs[0], 'iqr': iqr(xs),
            'se': se_med(xs), 'r': (xs[-1] - xs[0]) / d, 'i': iqr(xs) / d, 's': se_med(xs) / d, 'values': xs}


def cmp_(a, b):
    """B against A."""
    if not a or not b or not a['median']:
        return None
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    cl = {k: e > bars[k] and a['K'] >= 3 and b['K'] >= 3 for k in bars}  # K < 3: no spread, no claim
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'effect': ratio - 1, 'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'],
            'cl_i': cl['i'], 'cl_s': cl['s'], 'claimed': cl['r'] and cl['s'], 'KA': a['K'], 'KB': b['K']}


def fc(c, d=4):
    if not c:
        return 'n/a'
    return (f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] K={c['K']} r/i/s "
            f"{100 * c['r']:.2f}/{100 * c['i']:.2f}/{100 * c['s']:.2f} %")


def fcmp(c):
    if not c:
        return 'n/a'
    yn = f"{'Y' if c['cl_r'] else 'n'}/{'Y' if c['cl_i'] else 'n'}/{'Y' if c['cl_s'] else 'n'}"
    return (f"ratio {c['ratio']:.4f} ({100 * c['effect']:+.2f} %, {c['delta']:+.4f}); bars r/i/s "
            f"{100 * c['bar_r']:.2f}/{100 * c['bar_i']:.2f}/{100 * c['bar_s']:.2f} %; claimed r/i/s {yn} -> "
            f"{'CLAIMED' if c['claimed'] else 'not claimed'}")


def load():
    path = os.path.join(RAW, 'runs.jsonl')
    recs = [json.loads(l) for l in open(path, encoding='utf-8') if l.strip()] if os.path.exists(path) else []
    voided = {(r.get('run_tag'), r['block'], r['pass'], r['pass_attempt']) for r in recs if r.get('voided_pass')}
    done_by = {}
    for r in recs:
        if r.get('pass_done'):
            done_by[(r['block'], r['pass'])] = r['run_tag']
    latest = {}
    for r in recs:
        if 'row' in r and r.get('run_tag'):
            latest[(r['block'], r['pass'])] = max(latest.get((r['block'], r['pass']), ''), r['run_tag'])
    use = {k: done_by.get(k, latest.get(k)) for k in set(done_by) | set(latest)}
    live = [r for r in recs if 'row' in r and r.get('attempt') in ('original', 'rerun')
            and (r.get('run_tag'), r['block'], r['pass'], r.get('pass_attempt', 0)) not in voided
            and r.get('run_tag') == use.get((r['block'], r['pass'])) and 'aborted' not in r and 'void' not in r]
    slots = {}
    for r in live:
        k = (r['block'], r['pass'], r.get('pass_attempt', 0), r['round'], r['row'], r['binary'], r['W'])
        slots.setdefault(k, {})[r['attempt']] = r
    chosen, dropped = [], []
    for k, att in slots.items():
        pick = None
        for tag in ('original', 'rerun'):
            r = att.get(tag)
            if r and r.get('valid') and not r.get('contaminated'):
                pick = r
                break
        if pick:
            chosen.append(pick)
        else:
            dropped.append(k)
    return recs, chosen, dropped, voided


def zone(r, win, col, stat='mean'):
    c = (r.get('cols') or {}).get(win, {}).get(col)
    return c[stat] / 1e6 if c else None


def zcount(r, win, col, stat='mean'):
    c = (r.get('cols') or {}).get(win, {}).get(col)
    return c[stat] if c else None


def main():
    recs, chosen, dropped, voided = load()
    R = {'n_records': len(recs), 'n_chosen': len(chosen), 'dropped_slots': [list(k) for k in dropped],
         'voided_passes': [list(v) for v in voided], 'cells': {}, 'cmp': {}, 'decisions': {}}
    P(f'# Window 7 reduction ({RAW})')
    P(f'{len(recs)} records, {len(chosen)} chosen processes, {len(dropped)} dropped slots, {len(voided)} voided passes')
    if dropped:
        P(f'dropped slots (no valid clean attempt): {dropped}')

    def C(row, key, w, fn, blocks=None):
        rs = [r for r in chosen if r['row'] == row and r['binary'] == key and r['W'] == w
              and (blocks is None or r['block'] in blocks)]
        return cell([fn(r) for r in rs])

    def wall(r):
        return r.get('mean_ms')

    def show(name, c):
        R['cells'][name] = c
        P(f'- {name}: {fc(c)}')

    # ------------------------------------------------------------------ P1
    P('\n## P1 The Jolt headline, same window (trunk integ/unified 93b2615b vs Jolt v5.6.0 918fd2b7)')
    P('Rows: H-trk-tree = --cfg default --broadphase tree (reuse off on this tree); H-trk-ap = --cfg default '
      '--broadphase allpairs; H-jolt56 = -s=Pyramid -q=Discrete -f. Statistic: the process mean over [0,500) (window 3).')
    rc = {}
    rp = os.path.join(GATE, 'jolt_receipt.json')
    if os.path.exists(rp):
        rc = json.load(open(rp, encoding='utf-8'))
    mj = (rc.get('W1') or {}).get('manifolds_mean_100_500')
    P(f"- Jolt manifolds per step over [100,500) (-receipt, untimed, gate): W1 {mj}, W8 {(rc.get('W8') or {}).get('manifolds_mean_100_500')} "
      f"(window 3: {WIN3_JOLT56_MANIFOLDS})")
    BA, BB, BP = ('P1A-jolt',), ('P1B-jolt',), ('P1A-jolt', 'P1B-jolt')
    head = {}
    for w in WS:
        P(f'\n### W={w}')
        cells_w = {}
        for row, key in (('H-trk-tree', 'trk'), ('H-trk-ap', 'trk'), ('H-jolt56', 'j56')):
            for bn, bl in (('A', BA), ('B', BB), ('pooled', BP)):
                c = C(row, key, w, wall, bl)
                cells_w[(row, bn)] = c
                show(f'{row}@W{w} [{bn}] [0,500) ms', c)
        for bn, bl in (('A', BA), ('B', BB), ('pooled', BP)):
            for sub in ('0..100', '100..500'):
                for row, key in (('H-trk-tree', 'trk'), ('H-trk-ap', 'trk'), ('H-jolt56', 'j56')):
                    cells_w[(row, bn, sub)] = C(row, key, w, lambda r, s=sub: zone(r, s, 'wall_ns'), bl)
        hw = {}
        for ours in ('H-trk-tree', 'H-trk-ap'):
            res = {}
            for bn in ('A', 'B', 'pooled'):
                c = cmp_(cells_w[('H-jolt56', bn)], cells_w[(ours, bn)])
                res[bn] = c
                R['cmp'][f'P1 {ours}/jolt56 W{w} [{bn}] [0,500)'] = c
                P(f'- {ours} / Jolt56 W{w} [{bn}] [0,500): {fcmp(c)}')
                for sub in ('0..100', '100..500'):
                    cs = cmp_(cells_w[('H-jolt56', bn, sub)], cells_w[(ours, bn, sub)])
                    R['cmp'][f'P1 {ours}/jolt56 W{w} [{bn}] [{sub})'] = cs
                    if bn == 'pooled':
                        P(f'  - [{sub}) pooled: ours {fc(cells_w[(ours, bn, sub)])}; jolt {fc(cells_w[("H-jolt56", bn, sub)])}; {fcmp(cs)}')
            ok = [res[b] for b in ('A', 'B', 'pooled')]
            if all(ok):
                dirs = {x['ratio'] > 1 for x in ok}
                holds = all(x['claimed'] for x in ok) and len(dirs) == 1
                verdict = (('HOLDS: ours SLOWER than Jolt' if ok[2]['ratio'] > 1 else 'HOLDS: ours FASTER than Jolt')
                           if holds else 'does NOT hold (not claimed in both blocks and pooled, same direction)')
            else:
                verdict = 'NO DATA'
            hw[ours] = {'ratio_pooled': ok[2]['ratio'] if ok[2] else None, 'ratio_A': ok[0]['ratio'] if ok[0] else None,
                        'ratio_B': ok[1]['ratio'] if ok[1] else None, 'verdict': verdict}
            R['decisions'][f'P1 {ours}/jolt56 W{w}'] = hw[ours]
            P(f'  **{ours} / Jolt56 W{w}: pooled {hw[ours]["ratio_pooled"]}, A {hw[ours]["ratio_A"]}, B {hw[ours]["ratio_B"]} -> {verdict}**')
            # per manifold per step over [100,500): each side's own count
            mo = C(ours, 'trk', w, lambda r: zcount(r, '100..500', 'manifolds'), BP)
            to = cells_w[(ours, 'pooled', '100..500')]
            tj = cells_w[('H-jolt56', 'pooled', '100..500')]
            if mo and to and tj and mj:
                upm_o = to['median'] * 1e3 / mo['median']
                upm_j = tj['median'] * 1e3 / mj
                pm = {'ours_manifolds_100_500': mo['median'], 'ours_manifolds_minmax': [mo['min'], mo['max']],
                      'jolt_manifolds_100_500': mj, 'ours_us_per_manifold': upm_o, 'jolt_us_per_manifold': upm_j,
                      'per_manifold_ratio': upm_o / upm_j,
                      'manifold_ratio_jolt_over_ours': mj / mo['median']}
                R['decisions'][f'P1 per-manifold {ours} W{w}'] = pm
                P(f'  - per manifold [100,500) pooled: ours {upm_o:.4f} us ({mo["median"]:.4f} manifolds/step), '
                  f'Jolt {upm_j:.4f} us ({mj} manifolds/step) -> ours/Jolt {upm_o / upm_j:.3f}x '
                  f'(Jolt/ours manifolds {mj / mo["median"]:.3f}); the claim is the [100,500) step claim above')
        c = cmp_(cells_w[('H-trk-ap', 'pooled')], cells_w[('H-trk-tree', 'pooled')])
        R['cmp'][f'P1 tree vs allpairs W{w} pooled'] = c
        P(f'- tree vs allpairs W{w} pooled: {fcmp(c)}')
        jc = cells_w[('H-jolt56', 'pooled')]
        if jc:
            P(f'- window term: Jolt56 this window / window 3 = {jc["median"] / WIN3_JOLT56[w]:.4f} '
              f'({jc["median"]:.4f} vs {WIN3_JOLT56[w]:.4f} ms; cross-window, context only)')
            ca, cb = cells_w[('H-jolt56', 'A')], cells_w[('H-jolt56', 'B')]
            cab = cmp_(ca, cb)
            R['cmp'][f'P1 jolt56 block B vs A W{w}'] = cab
            P(f'- block term (same window): Jolt56 B vs A: {fcmp(cab)}')
            for ours in ('H-trk-tree', 'H-trk-ap'):
                cab = cmp_(cells_w[(ours, 'A')], cells_w[(ours, 'B')])
                R['cmp'][f'P1 {ours} block B vs A W{w}'] = cab
                P(f'- block term: {ours} B vs A: {fcmp(cab)}')
        head[w] = {k: (cells_w[(k, 'pooled')] or {}).get('median') for k in ('H-trk-tree', 'H-trk-ap', 'H-jolt56')}
    P('\n### Scaling T(1)/T(W), pooled medians')
    for k in ('H-trk-tree', 'H-trk-ap', 'H-jolt56'):
        t1 = head.get(1, {}).get(k)
        P(f'- {k}: ' + ', '.join(f'W{w} {t1 / head[w][k]:.3f}' if t1 and head.get(w, {}).get(k) else f'W{w} n/a' for w in WS))
    w8 = R['decisions'].get('P1 H-trk-tree/jolt56 W8', {})
    R['decisions']['P1 W8 headline'] = w8
    P(f"\n**Decision P1 (W8, the default row against Jolt v5.6.0, pooled K=12, must hold in each block):** {w8}")

    # ------------------------------------------------------------------ P2
    P('\n## P2 L9 C4 G-TW on 989ca0f0: same binary, --contact-reuse off vs on')
    regress = []
    for sc, wins in (('JA', ('0..500', '0..100', '100..500')), ('JD', ('0..500', '0..100', '100..500')), ('R', ('600..1100',))):
        for w in WS:
            for win in wins:
                if win in ('0..500', '600..1100'):
                    a = C(f'L9-{sc}-off', 'c4', w, wall)
                    b = C(f'L9-{sc}-on', 'c4', w, wall)
                else:
                    a = C(f'L9-{sc}-off', 'c4', w, lambda r, s=win: zone(r, s, 'wall_ns'))
                    b = C(f'L9-{sc}-on', 'c4', w, lambda r, s=win: zone(r, s, 'wall_ns'))
                R['cells'][f'L9-{sc}-off@W{w} [{win})'] = a
                R['cells'][f'L9-{sc}-on@W{w} [{win})'] = b
                c = cmp_(a, b)
                R['cmp'][f'P2 {sc} on vs off W{w} [{win})'] = c
                slower = bool(c and c['claimed'] and c['ratio'] > 1)
                if slower:
                    regress.append(f'{sc} W{w} [{win})')
                P(f'- {sc} W{w} [{win}): off {fc(a)}; on {fc(b)}; on vs off {fcmp(c)}{"  <-- CLAIMED REGRESSION" if slower else ""}')
    R['decisions']['P2 regressions'] = regress or 'none claimed'
    c2 = R['cmp'].get('P2 JA on vs off W1 [100..500)')
    if c2:
        real = -c2['delta']
        ok = real >= L9_BAR and c2['claimed'] and c2['ratio'] < 1
        dec = (f'realized dT(1)[100,500) on J-A = {real:.4f} ms against the bar 0.6 x {L9_PRED[0]} = {L9_BAR} ms '
               f'(window 6 prediction [{L9_PRED[0]}, {L9_PRED[1]}] ms): {"PASS" if ok else "FAIL (refuted or not claimed)"}')
    else:
        dec = 'NO DATA'
    R['decisions']['P2 realized gain'] = dec
    P(f'**Decision P2 (realized-gain rule):** {dec}')
    P(f"**Decision P2 (G-TW: a claimed regression at any W blocks):** {'BLOCKS: ' + ', '.join(regress) if regress else 'no claimed regression at any W'}")
    P('\n### The armed table (J-A, [100,500), per-process mean per step, median over K)')
    stages = (('wall', 'wall_ns'), ('bp', 'sys_physics_broadphase_ns'), ('np', 'sys_physics_narrowphase_ns'),
              ('graph', 'sys_physics_build_graph_ns'), ('solve', 'sys_physics_solve_colored_ns'),
              ('setup(solve_build)', 'phys_solve_build_ns'), ('colours wide', 'phys_color_wide_ns'),
              ('colours narrow', 'phys_color_narrow_ns'), ('warm_apply', 'phys_warm_apply_ns'), ('store', 'phys_store_ns'),
              ('sys_sum', 'sys_sum_ns'))
    for w in (1, 8):
        for arm in ('off', 'on'):
            rid = f'L9-JA-a-{arm}'
            m = C(rid, 'c4', w, lambda r: zcount(r, '100..500', 'manifolds'))
            pr = C(rid, 'c4', w, lambda r: zcount(r, '100..500', 'pairs'))
            reu = C(rid, 'c4', w, lambda r: zcount(r, '100..500', 'phys_np_reused'))
            P(f'- {rid} W{w}: manifolds/step {fc(m, 2)}; pairs/step {fc(pr, 2)}; reused/step {fc(reu, 2)}')
            for nm, col in stages:
                cc = C(rid, 'c4', w, lambda r, c_=col: zone(r, '100..500', c_))
                R['cells'][f'{rid}@W{w} {nm} ms'] = cc
                if cc and m and pr:
                    P(f'  - {nm:<20} {cc["median"]:.4f} ms/step [{cc["min"]:.4f}-{cc["max"]:.4f}]; '
                      f'{cc["median"] * 1e3 / m["median"]:.4f} us/manifold; {cc["median"] * 1e3 / pr["median"]:.4f} us/pair')
        for nm, col in stages:
            a = R['cells'].get(f'L9-JA-a-off@W{w} {nm} ms')
            b = R['cells'].get(f'L9-JA-a-on@W{w} {nm} ms')
            c = cmp_(a, b)
            R['cmp'][f'P2 armed {nm} on vs off W{w}'] = c
            P(f'  - {nm} on vs off W{w}: {fcmp(c)}')
    ref = C('L9-JA-a-on', 'c4', 1, wall)
    jc = C('L9-JC', 'c4', 1, wall)
    cns = C('L9-JC', 'c4', 1, lambda r: ((r.get('summary') or {}).get('canary_ns') or 0) / 1e6 or None)
    span = C('L9-JC', 'c4', 1, lambda r: zone(r, '0..500', 'sys_parity_canary_ns'))
    show('L9-JA-a-on c4@W1 (canary reference) ms', ref)
    show('L9-JC c4@W1 ms', jc)
    show('L9-JC c4@W1 injected canary ms', cns)
    show('L9-JC c4@W1 canary span ms', span)
    c = cmp_(ref, jc)
    R['cmp']['P2 canary rise W1'] = c
    if ref and jc and cns:
        rise = jc['median'] - ref['median']
        span_ok = bool(span and abs(span['median'] / cns['median'] - 1) <= 0.05)
        seen = bool(c and c['claimed'] and c['ratio'] > 1)
        seen_s = bool(c and c['cl_s'] and c['ratio'] > 1)
        dec = (f"span {span['median'] if span else None} vs injected {cns['median']:.4f} ms ({'ok' if span_ok else 'OFF or absent'}); "
               f"step rise {rise:+.4f} ms ({'SEEN (claimed r and s)' if seen else 'NOT SEEN under the two-spread rule'}; "
               f"SE alone: {'seen' if seen_s else 'not seen'})")
    else:
        dec = 'NO DATA'
    R['decisions']['P2 canary'] = dec
    P(f'**Decision P2 canary:** {dec}; {fcmp(c)}')

    # ------------------------------------------------------------------ P3
    P('\n## P3 L11 G9 remainder: J-A at W 2/4/16, parent 0ca312bd vs tip cbd86a65')
    slow = []
    for w in (2, 4, 16):
        a = C('G9-JA-mid', 'g9p', w, wall)
        b = C('G9-JA-mid', 'g9t', w, wall)
        show(f'G9-JA g9p@W{w} ms', a)
        show(f'G9-JA g9t@W{w} ms', b)
        c = cmp_(a, b)
        R['cmp'][f'P3 G9-JA tip vs parent W{w}'] = c
        if c and c['claimed'] and c['ratio'] > 1:
            slow.append(f'W{w}')
        P(f'- G9-JA W{w} tip vs parent: {fcmp(c)}')
    R['decisions']['P3'] = 'not claimed slower at W 2/4/16' if not slow else f'CLAIMED SLOWER at {slow}'
    P(f"**Decision P3:** {R['decisions']['P3']}")

    # ------------------------------------------------------------------ P4
    P('\n## P4 Our per-stage armed spans, trunk 93b2615b default row (--cfg default --broadphase tree), [100,500)')
    P('Per process: the MEDIAN over steps [100,500) of each span column (and the mean beside it); per cell: the median over K.')
    cols = set()
    for r in chosen:
        if r['row'] == 'S-trk-tree-a':
            cols |= set(((r.get('cols') or {}).get('100..500') or {}).keys())
    order = sorted(c for c in cols if c.endswith('_ns') and (c.startswith('phys_') or c.startswith('sys_')
                                                              or c in ('wall_ns', 'g_ns', 'u_ns', 'r_ns')))
    for w in (1, 8):
        P(f'\n### W={w}')
        for col in ['wall_ns'] + [c for c in order if c != 'wall_ns']:
            med = C('S-trk-tree-a', 'trk', w, lambda r, c_=col: zone(r, '100..500', c_, 'median'))
            mean = C('S-trk-tree-a', 'trk', w, lambda r, c_=col: zone(r, '100..500', c_, 'mean'))
            R['cells'][f'S-trk-tree-a@W{w} {col} median'] = med
            R['cells'][f'S-trk-tree-a@W{w} {col} mean'] = mean
            if med:
                P(f'- {col:<34} median {med["median"]:.4f} ms [{med["min"]:.4f}-{med["max"]:.4f}] (s {100 * med["s"]:.2f} %); '
                  f'mean {mean["median"]:.4f} ms')
        for col in ('manifolds', 'pairs', 'phys_np_pairs', 'phys_bp_pairs', 'phys_np_manifolds'):
            cc = C('S-trk-tree-a', 'trk', w, lambda r, c_=col: zcount(r, '100..500', c_))
            if cc:
                P(f'- {col} per step (mean over [100,500)): {cc["median"]:.2f}')

    # ------------------------------------------------------------------ P5
    P('\n## P5 Jolt v5.6.0 per-stage profile (the profiled Distribution build, frames 100/200/300/400 per process)')
    P('Per process: the mean over its four dumped frames of each scope\'s wall (union of its intervals across threads) '
      'and cpu (sum over threads); per cell: the median over K. Profiler on: the frame time carries its overhead.')
    jobs = [('update', lambda n: n.startswith('JPH::EPhysicsUpdateError JPH::PhysicsSystem::Update')),
            ('bp prepare (UpdateBroadPhasePrepare)', lambda n: n == 'UpdateBroadPhasePrepare'),
            ('bp finalize (UpdateBroadPhaseFinalize)', lambda n: n == 'UpdateBroadPhaseFinalize'),
            ('FindCollisions job (bp pairs + narrowphase + contact add)', lambda n: n == 'FindCollisions'),
            ('  bp FindCollidingPairs', lambda n: n.startswith('virtual void JPH::BroadPhaseQuadTree::FindCollidingPairs')),
            ('  np Add Constraint From Cached Manifold', lambda n: n == 'Add Constraint From Cached Manifold'),
            ('  np sCollideConvexVsConvex', lambda n: n.startswith('static void JPH::ConvexShape::sCollideConvexVsConvex')),
            ('islands (BuildIslandsFromConstraints)', lambda n: n == 'BuildIslandsFromConstraints'),
            ('islands (FinalizeIslands)', lambda n: n == 'FinalizeIslands'),
            ('setup (SetupVelocityConstraints)', lambda n: n == 'SetupVelocityConstraints'),
            ('solve velocity (SolveVelocityConstraints job)', lambda n: n == 'SolveVelocityConstraints'),
            ('solve position (SolvePositionConstraints job)', lambda n: n == 'SolvePositionConstraints'),
            ('integrate (IntegrateVelocity)', lambda n: n == 'IntegrateVelocity'),
            ('contact removed callbacks', lambda n: n == 'ContactRemovedCallbacks')]

    def prof(r, pred, idx):
        pf = r.get('profile') or {}
        vals = []
        for it, d in pf.items():
            if int(it) < 100 or 'scopes' not in d:
                continue
            v = sum(s[idx] for n, s in d['scopes'].items() if pred(n))
            vals.append(v)
        return statistics.mean(vals) if vals else None

    for w in (1, 8):
        P(f'\n### W={w}')
        fr = C('P-jolt56-prof', 'j56p', w, lambda r: statistics.mean([d['frame_ms'] for it, d in (r.get('profile') or {}).items()
                                                                        if int(it) >= 100 and d.get('frame_ms')] or [0]) or None)
        show(f'P-jolt56-prof@W{w} frame ms at the dumped frames', fr)
        show(f'P-jolt56-prof@W{w} [0,500) mean ms (profiled build)', C('P-jolt56-prof', 'j56p', w, wall))
        for nm, pred in jobs:
            cw = C('P-jolt56-prof', 'j56p', w, lambda r, p_=pred: prof(r, p_, 2))
            cc = C('P-jolt56-prof', 'j56p', w, lambda r, p_=pred: prof(r, p_, 1))
            R['cells'][f'P5 W{w} {nm} wall ms'] = cw
            R['cells'][f'P5 W{w} {nm} cpu ms'] = cc
            if cw:
                P(f'- {nm:<58} wall {cw["median"]:.4f} ms [{cw["min"]:.4f}-{cw["max"]:.4f}]; cpu {cc["median"]:.4f} ms')
        fc_ = R['cells'].get(f'P5 W{w} FindCollisions job (bp pairs + narrowphase + contact add) cpu ms')
        bp_ = R['cells'].get(f'P5 W{w}   bp FindCollidingPairs cpu ms')
        if fc_ and bp_:
            P(f'- derived: narrowphase + contact add (cpu) = FindCollisions - FindCollidingPairs = '
              f'{fc_["median"] - bp_["median"]:.4f} ms')

    with open(os.path.join(RAW, 'reduction.json'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(R, f, indent=1, default=str)
    with open(os.path.join(RAW, 'tables.md'), 'w', encoding='utf-8', newline='\n') as f:
        f.write('\n'.join(OUT) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
