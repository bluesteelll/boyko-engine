"""Window 6 reduction (untimed; run after the driver). Reads raw/runs.jsonl (or test/window/runs.jsonl with --test),
selects per slot the original if valid and clean, else its re-run if valid and clean (voided passes and warm-ups
excluded), and prints per cell: median over K, [min-max], and the three relative spreads r (min-max range), i (IQR),
s (SE of the median = 1.2533 * sd / sqrt K), each over the median. B against A is CLAIMED iff
|B/A - 1| > 2*hypot(sA, sB) under BOTH r and s (i printed). Then each item's decision rule.
Writes raw/reduction.json and raw/tables.md.
"""
import json
import math
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
W6 = os.path.dirname(HERE)
TEST = '--test' in sys.argv
RAW = os.path.join(W6, 'test', 'window') if TEST else os.path.join(W6, 'raw')
OUT = []


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
    if not a or not b or not a['median']:
        return None
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    cl = {k: e > bars[k] for k in bars}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'effect': ratio - 1, 'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'],
            'cl_i': cl['i'], 'cl_s': cl['s'], 'claimed': cl['r'] and cl['s'], 'KA': a['K'], 'KB': b['K']}


def fc(c, d=4):
    if not c:
        return 'n/a'
    return f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] K={c['K']} r/i/s {100 * c['r']:.2f}/{100 * c['i']:.2f}/{100 * c['s']:.2f} %"


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
    # a pass counts from ONE driver invocation: the one that completed it (a 'pass_done' marker), else the latest
    # invocation that ran any of it (a pass cut at the hard stop keeps its complete processes)
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


def by_cell(chosen):
    out = {}
    for r in chosen:
        out.setdefault((r['row'], r['binary'], r['W']), []).append(r)
    return out


def zone(r, win, col, stat='mean'):
    c = (r.get('cols') or {}).get(win, {}).get(col)
    return c[stat] / 1e6 if c else None


def main():
    recs, chosen, dropped, voided = load()
    cells = by_cell(chosen)
    R = {'n_records': len(recs), 'n_chosen': len(chosen), 'dropped_slots': [list(k) for k in dropped],
         'voided_passes': [list(v) for v in voided], 'cells': {}, 'cmp': {}, 'decisions': {}}
    P(f'# Window 6 reduction ({RAW})')
    P(f'{len(recs)} records, {len(chosen)} chosen processes, {len(dropped)} dropped slots, {len(voided)} voided passes')

    def C(row, key, w, fn):
        rs = cells.get((row, key, w), [])
        return cell([fn(r) for r in rs])

    def wall(r):
        return r.get('mean_ms')

    def show(name, c):
        R['cells'][name] = c
        P(f'- {name}: {fc(c)}')

    # ---------------- P1
    P('\n## P1 L10 pre-C0 refutation (armed J-Son tail [264,1000), W=1; bar 0.584 ms)')
    offp = C('L10-Offp', 'b4db', 1, wall)
    son = C('L10-Son', 'b4db', 1, wall)
    show('L10-Offp@W1 wall mean [264,1000) ms', offp)
    show('L10-Son@W1 wall mean [264,1000) ms', son)
    for col in ('sys_sum_ns', 'sys_physics_broadphase_ns', 'sys_physics_narrowphase_ns', 'sys_physics_build_graph_ns',
                'sys_physics_solve_colored_ns', 'g_ns'):
        show(f'L10-Offp@W1 {col} mean [264,1000) ms', C('L10-Offp', 'b4db', 1, lambda r, c=col: zone(r, '264..1000', c)))
    offp_frozen = C('L10-Offp', 'b4db', 1, lambda r: zone(r, 'frozen..end', 'wall_ns'))
    show('L10-Offp@W1 wall mean over the all-frozen tail [first_frozen_step,1000) ms', offp_frozen)
    show('L10-Offp@W1 first_frozen_step', C('L10-Offp', 'b4db', 1, lambda r: r.get('frozen_from')))
    show('L10-Son@W1 wall mean over the all-frozen tail ms', C('L10-Son', 'b4db', 1, lambda r: zone(r, 'frozen..end', 'wall_ns')))
    c = cmp_(son, offp)
    R['cmp']['P1 Offp vs Son W1'] = c
    P(f'- Off-prime against the lane J-Son: {fcmp(c)}')
    if offp:
        dec = ('STOP L10 (refuted): Off-prime(1) median < 0.584 ms' if offp['median'] < 0.584 else
               'NOT refuted: Off-prime(1) median >= 0.584 ms')
        dec += f"; min {offp['min']:.4f} max {offp['max']:.4f} (the bar is {'inside' if offp['min'] < 0.584 <= offp['max'] else 'outside'} [min, max])"
        if offp_frozen:
            dec += f"; all-frozen tail median {offp_frozen['median']:.4f} ms ({'< 0.584: the literal window and the frozen tail DISAGREE' if (offp_frozen['median'] < 0.584) != (offp['median'] < 0.584) else 'same verdict'})"
    else:
        dec = 'NO DATA'
    R['decisions']['P1'] = dec
    P(f'**Decision P1:** {dec}')

    # ---------------- P2a
    P('\n## P2a L9 C0 refutation reading (class bench --bench; STOP C4 if the predicted dt_np(1) band lies below 1.0 ms)')
    lo = C('L9-classes', 'c4db', 1, lambda r: (r.get('classes') or {}).get('band', {}).get('jolt', [None, None])[0])
    hi = C('L9-classes', 'c4db', 1, lambda r: (r.get('classes') or {}).get('band', {}).get('jolt', [None, None])[1])
    show('classes jolt band low ms', lo)
    show('classes jolt band high ms', hi)
    ns = {}
    for k in ('jolt/separated', 'jolt/touching', 'jolt/stream', 'rest/separated', 'rest/touching', 'rain/fast'):
        ns[k] = C('L9-classes', 'c4db', 1, lambda r, k=k: ((r.get('classes') or {}).get('ns', {}).get(k) or {}).get('ns_per_pair'))
        show(f'classes {k} ns/pair', ns[k])
    h_meas = 0.9897  # l9/h_tau.md: h_J(1 mm) over [100,500), counters (untimed, a result)
    n_touch = 4515
    pred = None
    if ns['jolt/touching']:
        t = ns['jolt/touching']['median']
        pred = (h_meas * n_touch * (t - 100.0) * 1e-6, h_meas * n_touch * (t - 60.0) * 1e-6)
        P(f'- L9b-only prediction with the measured h = {h_meas} and N_touch = {n_touch}: [{pred[0]:.3f}, {pred[1]:.3f}] ms '
          f'(t_hit in [60, 100] ns; the carry is in both arms of the same-binary A/B)')
    R['cells']['L9b prediction ms'] = pred
    if lo and hi:
        if hi['median'] < 1.0:
            dec = 'FIRES: the whole predicted band is below 1.0 ms -> STOP C4 and escalate'
        elif lo['median'] < 1.0:
            dec = 'AMBIGUOUS: the band straddles 1.0 ms -> escalate with the numbers'
        else:
            dec = 'DOES NOT FIRE: the band lies above 1.0 ms'
        dec += f" (median band [{lo['median']:.3f}, {hi['median']:.3f}] ms)"
    else:
        dec = 'NO DATA'
    R['decisions']['P2a'] = dec
    P(f'**Decision P2a:** {dec}')

    # ---------------- P2b
    P('\n## P2b L9 same-binary reuse off vs on (4db26681)')
    for w in (1, 8):
        a = C('L9-JA-off', 'b4db', w, wall)
        b = C('L9-JA-on', 'b4db', w, wall)
        show(f'L9-JA-off@W{w} [0,500) ms', a)
        show(f'L9-JA-on@W{w} [0,500) ms', b)
        c = cmp_(a, b)
        R['cmp'][f'P2b JA on vs off W{w} [0,500)'] = c
        P(f'- on vs off W{w} [0,500): {fcmp(c)}')
        a2 = C('L9-JA-off', 'b4db', w, lambda r: zone(r, '100..500', 'wall_ns'))
        b2 = C('L9-JA-on', 'b4db', w, lambda r: zone(r, '100..500', 'wall_ns'))
        c2 = cmp_(a2, b2)
        R['cmp'][f'P2b JA on vs off W{w} [100,500)'] = c2
        P(f'- on vs off W{w} [100,500): off {fc(a2)}; on {fc(b2)}; {fcmp(c2)}')
        if w == 1 and c2 and pred:
            bar = 0.6 * pred[0]
            real = -c2['delta']
            dec = (f"realized dT(1)[100,500) = {real:.3f} ms against 0.6 x {pred[0]:.3f} = {bar:.3f} ms: "
                   f"{'PASS' if real >= bar and c2['claimed'] else 'FAIL (refuted or not claimed) -> investigate before C4'}")
            R['decisions']['P2b realized gain'] = dec
            P(f'**Decision P2b (realized-gain rule):** {dec}')
    for col in ('sys_physics_narrowphase_ns', 'wall_ns', 'sys_sum_ns'):
        a = C('L9-JA-a-off', 'b4db', 1, lambda r, c=col: zone(r, '100..500', c))
        b = C('L9-JA-a-on', 'b4db', 1, lambda r, c=col: zone(r, '100..500', c))
        c = cmp_(a, b)
        R['cmp'][f'P2b armed {col} [100,500) W1'] = c
        P(f'- armed W1 {col} [100,500): off {fc(a)}; on {fc(b)}; {fcmp(c)}')
        if col == 'sys_physics_narrowphase_ns' and c:
            d = -c['delta']
            R['decisions']['P2b measured dt_np(1)'] = f'{d:.3f} ms (armed narrowphase span, off - on, [100,500))'
            P(f'**Measured dt_np(1) (L9b alone):** {d:.3f} ms against the 1.0 ms bar (L9a is in both arms)')

    # ---------------- P3
    P('\n## P3 tree broadphase C3b: parent 6dd1f916 (RowWalk) vs tip 983480a9 (LeafList)')
    bp4 = ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')
    for w in (1, 8):
        tq = {k: C('C3b-TA-armed', k, w, lambda r: zone(r, '100..500', 'phys_bp_query_ns', 'median')) for k in ('par', 'tip')}
        for k in ('par', 'tip'):
            show(f'C3b t_q {k}@W{w} (median over [100,500) of phys_bp_query_ns) ms', tq[k])
        c = cmp_(tq['par'], tq['tip'])
        R['cmp'][f'P3 t_q tip vs par W{w}'] = c
        P(f'- t_q tip vs par W{w}: {fcmp(c)}')
        def span_of(r):
            v = [zone(r, '100..500', z, 'median') for z in bp4]
            return None if any(x is None for x in v) else sum(v)
        span = {k: C('C3b-TA-armed', k, w, span_of) for k in ('par', 'tip')}
        for k in ('par', 'tip'):
            show(f'C3b tree span (sum of four phys_bp_* per-step medians) {k}@W{w} ms', span[k])
        R['cmp'][f'P3 tree span tip vs par W{w}'] = cmp_(span['par'], span['tip'])
        for row in ('C3b-TA-armed', 'C3b-TD-tree'):
            a = C(row, 'par', w, wall)
            b = C(row, 'tip', w, wall)
            show(f'{row} par@W{w} wall [0,500) ms', a)
            show(f'{row} tip@W{w} wall [0,500) ms', b)
            c = cmp_(a, b)
            R['cmp'][f'P3 {row} tip vs par W{w}'] = c
            P(f'- {row} W{w} tip vs par: {fcmp(c)}')
    tq1 = R['cells'].get('C3b t_q tip@W1 (median over [100,500) of phys_bp_query_ns) ms')
    c1 = R['cmp'].get('P3 t_q tip vs par W1')
    if tq1 and c1:
        t = tq1['median']
        parts = [f"t_q(J,W=1) tip = {t:.4f} ms (c_q = {t * 1e6 / 1240:.0f} ns/row)",
                 'kernel SHIPS (ratio < 1)' if c1['ratio'] < 1 else 'kernel REJECTED (ratio >= 1): default back to RowWalk',
                 'attribution A REFUTED (ratio > 0.55 or t_q > 0.21)' if (c1['ratio'] > 0.55 or t > 0.21) else 'attribution A stands',
                 'C2 BUILT (t_q >= 0.235)' if t >= 0.235 else 'C2 not built (t_q < 0.235)',
                 'into the band CLAIMED (t_q <= 0.186)' if (t <= 0.186 and c1['claimed']) else 'into the band not claimed',
                 'F3 taken up (c_q > 150 ns)' if t > 0.186 else 'F3 not needed']
        s1 = R['cells'].get('C3b tree span (sum of four phys_bp_* per-step medians) tip@W1 ms')
        s8 = R['cells'].get('C3b tree span (sum of four phys_bp_* per-step medians) tip@W8 ms')
        if s1 and s8:
            parts.append(f"tree span {s1['median']:.4f} / {s8['median']:.4f} ms vs limits 0.36 / 0.35: "
                         f"{'PASS' if s1['median'] <= 0.36 and s8['median'] <= 0.35 else 'STOP'}")
        dec = '; '.join(parts)
    else:
        dec = 'NO DATA'
    R['decisions']['P3'] = dec
    P(f'**Decision P3:** {dec}')

    # ---------------- P4
    P('\n## P4 L11 G9 remainder: parent 0ca312bd vs tip cbd86a65')
    for row, ws, win in (('G9-JA', (1, 8), None), ('G9-R', (1, 8), '600..1100'), ('G9-RS', (1, 8), '300..800'),
                         ('G9-S16', (1,), '0..300'), ('G9-JAs-mid', (2, 4, 16), None)):
        for w in ws:
            a = C(row, 'g9p', w, wall)
            b = C(row, 'g9t', w, wall)
            show(f'{row} g9p@W{w} ms', a)
            show(f'{row} g9t@W{w} ms', b)
            c = cmp_(a, b)
            R['cmp'][f'P4 {row} tip vs parent W{w}'] = c
            slower = c and c['claimed'] and c['ratio'] > 1
            P(f'- {row} W{w} tip vs parent: {fcmp(c)}{"  <-- CLAIMED SLOWER" if slower else ""}')
            if win:
                for col in ('phys_solve_build_ns', 'phys_warm_apply_ns', 'phys_store_ns', 'phys_color_wide_ns'):
                    ca = C(row, 'g9p', w, lambda r, c_=col: zone(r, win, c_))
                    cb = C(row, 'g9t', w, lambda r, c_=col: zone(r, win, c_))
                    cc = cmp_(ca, cb)
                    R['cmp'][f'P4 {row} {col} W{w}'] = cc
                    P(f'  - {col}: parent {fc(ca)}; tip {fc(cb)}; {fcmp(cc)}')
    # the canary (P0 recipe): its own span reads F*T, and the step rises by it against the reference row
    for key in ('g9p', 'g9t'):
        ref = C('G9-JAs-a', key, 1, wall)
        jc = C('G9-JC', key, 1, wall)
        cns = C('G9-JC', key, 1, lambda r: ((r.get('summary') or {}).get('canary_ns') or 0) / 1e6 or None)
        span = C('G9-JC', key, 1, lambda r: zone(r, '0..500', 'sys_parity_canary_ns'))
        show(f'G9-JAs-a {key}@W1 ms', ref)
        show(f'G9-JC {key}@W1 ms', jc)
        show(f'G9-JC {key}@W1 injected canary ms', cns)
        show(f'G9-JC {key}@W1 canary span ms', span)
        c = cmp_(ref, jc)
        R['cmp'][f'P4 canary rise {key} W1'] = c
        if ref and jc and cns and span:
            rise = jc['median'] - ref['median']
            span_ok = abs(span['median'] / cns['median'] - 1) <= 0.05
            seen = bool(c and c['claimed'] and c['ratio'] > 1)
            R['decisions'][f'P4 canary {key}'] = (f"span {span['median']:.4f} vs injected {cns['median']:.4f} ms "
                                                  f"({'ok' if span_ok else 'OFF'}); step rise {rise:+.4f} ms "
                                                  f"({'SEEN (claimed)' if seen else 'NOT SEEN under the claim rule'})")
            P(f"- canary {key}: {R['decisions'][f'P4 canary {key}']}; {fcmp(c)}")
    slow = [k for k, v in R['cmp'].items() if k.startswith('P4 ') and 'phys_' not in k and 'canary' not in k
            and v and v['claimed'] and v['ratio'] > 1]
    R['decisions']['P4'] = ('G9 remainder: no row claimed slower' if not slow else f'CLAIMED SLOWER: {slow}')
    P(f"**Decision P4:** {R['decisions']['P4']}")

    # ---------------- P5
    P('\n## P5 extras')
    for w in (1, 8):
        a = C('B2-JAs', 'w4bt', w, wall)
        b = C('B2-JAs', 'g9p', w, wall)
        show(f'B2-JAs w4bt(L11 C2)@W{w} ms', a)
        show(f'B2-JAs g9p(C2+line merge)@W{w} ms', b)
        c = cmp_(a, b)
        R['cmp'][f'P5a bridge2 W{w}'] = c
        P(f'- bridge 2 W{w} (C2+merge vs C2): {fcmp(c)}')
    for row, ws, win in (('L10-Offp8', (8,), '264..1000'), ('L10-Son8', (8,), '264..1000'),
                         ('L10-RS-Offp', (1, 8), '300..800'), ('L10-RS', (1, 8), '300..800')):
        for w in ws:
            show(f'{row}@W{w} wall ms', C(row, 'b4db', w, wall))
            show(f'{row}@W{w} sys_sum ms', C(row, 'b4db', w, lambda r: zone(r, win, 'sys_sum_ns')))
    for nm in ('bp_g4_scene/tree/j100', 'bp_g4_scene/tree_rowwalk/j100', 'bp_g4_scene/tree/1240', 'bp_g4_scene/tree_rowwalk/1240',
               'bp_g4_uniform/tree/1000', 'bp_g4_uniform/tree_rowwalk/1000', 'bp_g4_disparity/tree/1000',
               'bp_g4_disparity/tree_rowwalk/1000'):
        show(f'C3b-G4 {nm} ms', C('C3b-G4', 'bpb', 1, lambda r, n=nm: ((r.get('criterion') or {}).get('est_ms', {}).get(n) or [None, None])[1]))
    for fam, sc, lo_b, hi_b in (('scene', 'j100', None, 0.55), ('scene', '1240', None, 0.55),
                                ('uniform', '1000', 0.30, 0.60), ('disparity', '1000', 0.30, 0.60)):
        a_id, b_id = f'bp_g4_{fam}/tree/{sc}', f'bp_g4_{fam}/tree_rowwalk/{sc}'
        rat = C('C3b-G4', 'bpb', 1, lambda r, a_id=a_id, b_id=b_id: (lambda e: e[a_id][1] / e[b_id][1]
                                                                   if a_id in e and b_id in e else None)(
            (r.get('criterion') or {}).get('est_ms', {})))
        show(f'C3b-G4 LeafList/RowWalk {fam}/{sc} (per-process ratio)', rat)
        if rat:
            m = rat['median']
            if lo_b is None:
                verdict = 'REFUTES attribution A (> 0.55)' if m > 0.55 else 'within the design (<= 0.55)'
            else:
                verdict = ('the model is STRUCTURALLY WRONG (outside [0.30, 0.60])' if not (lo_b <= m <= hi_b)
                           else 'inside [0.30, 0.60]')
            verdict += '; the kernel ships (< 1)' if m < 1 else '; the kernel is REJECTED (>= 1)'
            R['decisions'][f'P5c {fam}/{sc}'] = f'ratio {m:.3f}: {verdict}'
    for w in (1, 8):
        a = C('L9-JD-off', 'b4db', w, wall)
        b = C('L9-JD-on', 'b4db', w, wall)
        c = cmp_(a, b)
        R['cmp'][f'P5d JD on vs off W{w}'] = c
        P(f'- L9 J-D on vs off W{w}: off {fc(a)}; on {fc(b)}; {fcmp(c)}')

    P('')
    P('## P5g C3b G5 on the tip, same binary: tree vs allpairs')
    for w in (1, 8):
        tr = C('G5-TA-tree-a', 'tip', w, lambda r: zone(r, '100..500', 'sys_physics_broadphase_ns'))
        ap = C('G5-TA-ap-a', 'tip', w, lambda r: zone(r, '100..500', 'sys_physics_broadphase_ns'))
        show(f'G5 broadphase span tree tip@W{w} [100,500) ms', tr)
        show(f'G5 broadphase span allpairs tip@W{w} [100,500) ms', ap)
        if tr and ap:
            R['decisions'][f'P5g dbp W{w}'] = (f"allpairs - tree broadphase span {ap['median'] - tr['median']:.4f} ms "
                                               f"(window 4 bar at W8: >= 1.03 ms)")
            P(f"- dbp(W{w}) = {ap['median'] - tr['median']:.4f} ms")
        tq = C('G5-TA-tree-a', 'tip', w, lambda r: zone(r, '100..500', 'phys_bp_query_ns', 'median'))
        show(f'G5 t_q tip@W{w} (same-block repeat of P3) ms', tq)
        for a_id, b_id in (('G5-TA-ap-a', 'G5-TA-tree-a'), ('G5-TD-ap', 'G5-TD-tree')):
            a = C(a_id, 'tip', w, wall)
            b = C(b_id, 'tip', w, wall)
            show(f'{a_id} tip@W{w} ms', a)
            show(f'{b_id} tip@W{w} ms', b)
            c = cmp_(a, b)
            R['cmp'][f'P5g {b_id} vs {a_id} W{w}'] = c
            P(f'- {b_id} vs {a_id} W{w}: {fcmp(c)}')
    P('')
    P('## P5e / P5f L9 reuse off vs on: R W1/8, J-A W2/4/16')
    for a_id, b_id, ws in (('L9-R-off', 'L9-R-on', (1, 8)), ('L9-JA-off-mid', 'L9-JA-on-mid', (2, 4, 16))):
        for w in ws:
            a = C(a_id, 'b4db', w, wall)
            b = C(b_id, 'b4db', w, wall)
            c = cmp_(a, b)
            R['cmp'][f'P5 {b_id} vs {a_id} W{w}'] = c
            P(f'- {b_id} vs {a_id} W{w}: off {fc(a)}; on {fc(b)}; {fcmp(c)}')

    with open(os.path.join(RAW, 'reduction.json'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(R, f, indent=1, default=str)
    with open(os.path.join(RAW, 'tables.md'), 'w', encoding='utf-8', newline='\n') as f:
        f.write('\n'.join(OUT) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
