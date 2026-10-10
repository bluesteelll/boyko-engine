"""Window 7 wave 2 reduction (untimed; run after the driver). Reads win7b/raw/runs.jsonl (test/window or test/real
with --test / --test-real) and, for Q1, window 7's own win7/raw/runs.jsonl (its blocks P1A-jolt = A and P1B-jolt = B).
Slot selection per process: the original if valid and clean, else its re-run if valid and clean (voided passes and
warm-ups excluded) - window 6/7's selection verbatim. Per cell: median over K, [min-max], relative spreads r (range),
i (IQR), s (SE of the median = 1.2533 sd / sqrt K), each over the median.
CLAIM RULE (window 7's): B against A is CLAIMED iff |B/A - 1| > 2 hypot(sA, sB) under BOTH r and s (i printed); no
claim when either side has K < 3. Against a constant bar b: CLAIMED iff |m/b - 1| > 2 r and > 2 s of the cell.
Decision rules (stated in plan.md, applied here):
  Q1  ours/Jolt at W 4/8/16: a claim HOLDS iff it is claimed in EVERY block (A, B, C, D) AND pooled over A-D, all in
      the same direction; C, D and pooled C+D are printed beside it.
  Q2  t_q = the per-process median over [100,500) of phys_bp_query_ns, median over K. Block term = D vs C (same row,
      binary, layout); layout term = pad06 vs armed (same block, binary). C2 (c3b/design.md:372): BUILT iff
      t_q(J, W=1) on the tip is >= 0.235 ms claimed above the bar in C, in D and pooled; NOT BUILT iff below the bar
      claimed in C, in D and pooled; otherwise UNRESOLVED. Read on the default row (the letter, :231/:372) and on the
      cfg-A row (window 6's reading), plain layout; the padded layout printed beside. Also > 0.21 (attribution A),
      <= 0.186 claimed (into the band), c_q = t_q / 1240 > 150 ns (F3), the tree span limits 0.36 / 0.35 ms.
  Q3  recipe 1.4 (g4_g5_recipe.md:131-132): per family, LO_f = the largest n at which all_pairs is not claimed slower
      than tree, HI_f = the smallest n at which tree is claimed faster; TREE_BRUTE_MAX_ROWS = min(LO_uniform,
      LO_disparity); AUTO_TREE_LO / HI = the wider band (min LO_f, max HI_f), LO >= TREE_BRUTE_MAX_ROWS. Beside it,
      L2's procedure (window 4 analysis 6.2): HI = the larger family's log-log crossover to two significant figures,
      LO = 0.9 HI.
  Q4  a canary rung is SEEN iff its step rise over the reference is claimed (r and s) upward and canary_ns is printed;
      G-TW's resolution is DEMONSTRATED iff the 1.5 R and 2 R rungs are SEEN; the smallest rung seen with every
      larger rung seen is the resolution demonstrated in this block.
Writes raw/reduction.json and raw/tables.md.
"""
import json
import math
import os
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
W7B = os.path.dirname(HERE)
WIN7 = os.path.join(os.path.dirname(W7B), 'win7')
TEST = '--test' in sys.argv or '--test-real' in sys.argv
RAW = os.path.join(W7B, 'test', 'real' if '--test-real' in sys.argv else 'window') if TEST else os.path.join(W7B, 'raw')
if '--raw' in sys.argv:  # rehearsal only: reduce another directory (e.g. a synthetic code-path check)
    RAW = sys.argv[sys.argv.index('--raw') + 1]
RAW7 = os.path.join(WIN7, 'raw')
GATE7 = os.path.join(WIN7, 'gate')
KEEP_MISSES = '--keep-misses' in sys.argv  # rehearsal only: test/real paths cannot hit the pinned lengths
ROWS = json.load(open(os.path.join(W7B, 'rows7b.json'), encoding='utf-8'))
OUT = []
# window 3's JOLT56-T cells (context only, as window 7)
WIN3_JOLT56 = {1: 9.8281962, 2: 5.7704603, 4: 3.5814395, 8: 2.5692559, 16: 2.3884695}
# window 6 (D:/wt/joltab/docs/measurements/2026-09-24-physics-window6/analysis.md:214, :232): tip t_q at W1
WIN6_TQ = {'P3 (plain layout, 648 chars)': 0.2102, 'P5g (padded layout, 654 chars)': 0.2354}
# window 4 (docs/measurements/2026-09-22-broadphase-tree/analysis.md:219): all_pairs / tree before C3b's F1
WIN4_G4 = {'uniform': {64: 0.619, 128: 1.137, 256: 1.997}, 'disparity': {64: 0.500, 128: 0.946, 256: 1.543}}
C2_BAR, ATTR_A, BAND, ROWS_Q = 0.235, 0.21, 0.186, 1240
SPAN_LIMIT = {1: 0.36, 8: 0.35}


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
    cl = {k: e > bars[k] and a['K'] >= 3 and b['K'] >= 3 for k in bars}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'effect': ratio - 1, 'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'],
            'cl_i': cl['i'], 'cl_s': cl['s'], 'claimed': cl['r'] and cl['s'], 'KA': a['K'], 'KB': b['K']}


def vs_bar(c, bar):
    """A cell against a constant bar: claimed iff |m/bar - 1| > 2 r and > 2 s (only the cell has a spread)."""
    if not c:
        return None
    e = c['median'] / bar - 1
    cl_r = abs(e) > 2 * c['r'] and c['K'] >= 3
    cl_s = abs(e) > 2 * c['s'] and c['K'] >= 3
    side = 'above' if e > 0 else 'below'
    return {'median': c['median'], 'bar': bar, 'effect': e, 'side': side, 'cl_r': cl_r, 'cl_s': cl_s,
            'claimed': cl_r and cl_s, 'bar_r': 2 * c['r'], 'bar_s': 2 * c['s']}


def fbar(v):
    if not v:
        return 'n/a'
    return (f"{v['median']:.4f} vs {v['bar']} ({100 * v['effect']:+.2f} %; bars 2r/2s {100 * v['bar_r']:.2f}/"
            f"{100 * v['bar_s']:.2f} %; claimed r/s {'Y' if v['cl_r'] else 'n'}/{'Y' if v['cl_s'] else 'n'}) -> "
            f"{v['side'] + ' CLAIMED' if v['claimed'] else 'on the bar (not claimed)'}")


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


def load(raw):
    path = os.path.join(raw, 'runs.jsonl')
    recs = [json.loads(l) for l in open(path, encoding='utf-8') if l.strip()] if os.path.exists(path) else []
    voided = {(r.get('run_tag'), r['block'], r['pass'], r['pass_attempt']) for r in recs if r.get('voided_pass')}
    done_by, latest = {}, {}
    for r in recs:
        if r.get('pass_done'):
            done_by[(r['block'], r['pass'])] = r['run_tag']
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
    skipped = [r for r in recs if r.get('block_skipped')]
    return recs, chosen, dropped, voided, skipped


def zone(r, win, col, stat='mean'):
    c = (r.get('cols') or {}).get(win, {}).get(col)
    return c[stat] / 1e6 if c else None


def zcount(r, win, col, stat='mean'):
    c = (r.get('cols') or {}).get(win, {}).get(col)
    return c[stat] if c else None


def main():
    recs, chosen, dropped, voided, skipped = load(RAW)
    # window 7's blocks A and B are read in the real reduction and in the --test-real rehearsal (read-only)
    recs7, chosen7, dropped7, voided7, _ = load(RAW7) if '--test' not in sys.argv else ([], [], [], set(), [])
    R = {'n_records': len(recs), 'n_chosen': len(chosen), 'dropped_slots': [list(k) for k in dropped],
         'voided_passes': [list(v) for v in voided], 'skipped_blocks': [s['block'] for s in skipped],
         'cells': {}, 'cmp': {}, 'decisions': {}}
    P(f'# Window 7 wave 2 reduction ({RAW})')
    P(f'{len(recs)} records, {len(chosen)} chosen processes, {len(dropped)} dropped slots, {len(voided)} voided passes, '
      f'blocks skipped for priority: {[s["block"] for s in skipped] or "none"}')
    if dropped:
        P(f'dropped slots (no valid clean attempt): {dropped}')
    miss = [r for r in chosen if r.get('cwd_len_miss')]
    P(f'launch-context length misses among chosen processes: {len(miss)} '
      f'{[(r["block"], r["row"], r["binary"], r["W"], r.get("cwd_len_actual")) for r in miss][:8]}')
    P(f'window 7 records (Q1 blocks A, B): {len(recs7)} records, {len(chosen7)} chosen, {len(dropped7)} dropped slots')
    pool = chosen + [dict(r, block={'P1A-jolt': 'A7', 'P1B-jolt': 'B7'}[r['block']]) for r in chosen7
                     if r['block'] in ('P1A-jolt', 'P1B-jolt')]

    def C(row, key, w, fn, blocks, excl_miss=False):
        rs = [r for r in pool if r['row'] == row and r['binary'] == key and r['W'] == w and r['block'] in blocks
              and not (excl_miss and r.get('cwd_len_miss') and not KEEP_MISSES)]
        return cell([fn(r) for r in rs])

    def wall(r):
        return r.get('mean_ms')

    def show(name, c):
        R['cells'][name] = c
        P(f'- {name}: {fc(c)}')

    # ------------------------------------------------------------------ Q1
    P('\n## Q1 The W8 headline over four separated blocks: trunk 93b2615b default tree row vs Jolt v5.6.0 918fd2b7')
    P('Blocks: A = window 7 P1A-jolt, B = window 7 P1B-jolt (win7/raw/runs.jsonl), C = Q1C (first block of this run), '
      'D = Q1D (last). Statistic: the process mean over [0,500) (window 3). Rule: HOLDS iff claimed in A, B, C, D and '
      'pooled over A-D, same direction.')
    BL = {'A': ('A7',), 'B': ('B7',), 'C': ('Q1C',), 'D': ('Q1D',), 'pooled A-D': ('A7', 'B7', 'Q1C', 'Q1D'),
          'pooled C+D': ('Q1C', 'Q1D')}
    rc = {}
    rp = os.path.join(GATE7, 'jolt_receipt.json')
    if os.path.exists(rp):
        rc = json.load(open(rp, encoding='utf-8'))
    mj = (rc.get('W8') or {}).get('manifolds_mean_100_500')
    for w in (4, 8, 16):
        P(f'\n### W={w}')
        cw = {}
        for row, key in (('H-trk-tree', 'trk'), ('H-trk-ap', 'trk'), ('H-jolt56', 'j56')):
            for bn, bl in BL.items():
                c = C(row, key, w, wall, bl)
                cw[(row, bn)] = c
                show(f'{row}@W{w} [{bn}] [0,500) ms', c)
                for sub in ('0..100', '100..500'):
                    cw[(row, bn, sub)] = C(row, key, w, lambda r, s=sub: zone(r, s, 'wall_ns'), bl)
        for ours in ('H-trk-tree', 'H-trk-ap'):
            res = {}
            for bn in BL:
                c = cmp_(cw[('H-jolt56', bn)], cw[(ours, bn)])
                res[bn] = c
                R['cmp'][f'Q1 {ours}/jolt56 W{w} [{bn}] [0,500)'] = c
                P(f'- {ours} / Jolt56 W{w} [{bn}] [0,500): {fcmp(c)}')
            for sub in ('0..100', '100..500'):
                cs = cmp_(cw[('H-jolt56', 'pooled A-D', sub)], cw[(ours, 'pooled A-D', sub)])
                R['cmp'][f'Q1 {ours}/jolt56 W{w} [pooled A-D] [{sub})'] = cs
                P(f'  - [{sub}) pooled A-D: ours {fc(cw[(ours, "pooled A-D", sub)])}; jolt '
                  f'{fc(cw[("H-jolt56", "pooled A-D", sub)])}; {fcmp(cs)}')
            need = [res[b] for b in ('A', 'B', 'C', 'D', 'pooled A-D')]
            if all(need):
                dirs = {x['ratio'] > 1 for x in need}
                holds = all(x['claimed'] for x in need) and len(dirs) == 1
                verdict = ((f'HOLDS: ours {"SLOWER" if need[-1]["ratio"] > 1 else "FASTER"} than Jolt v5.6.0 '
                            f'(claimed in A, B, C, D and pooled A-D)') if holds else
                           'does NOT hold: not claimed in every block and pooled, same direction (' +
                           ', '.join(f'{b} {"Y" if res[b]["claimed"] else "n"}' for b in ('A', 'B', 'C', 'D', 'pooled A-D')) + ')')
            else:
                verdict = 'NO DATA for ' + ', '.join(b for b in ('A', 'B', 'C', 'D', 'pooled A-D') if not res[b])
            cd = [res[b] for b in ('C', 'D', 'pooled C+D')]
            cd_holds = all(cd) and all(x['claimed'] for x in cd) and len({x['ratio'] > 1 for x in cd}) == 1
            dec = {b: (res[b]['ratio'] if res[b] else None) for b in BL}
            dec.update({'verdict': verdict, 'this_run_C_D_pooledCD': 'holds' if cd_holds else 'does not hold'})
            R['decisions'][f'Q1 {ours}/jolt56 W{w}'] = dec
            P(f'  **{ours} / Jolt56 W{w}: ' + ', '.join(f'{b} {dec[b]:.4f}' if dec[b] else f'{b} n/a' for b in BL)
              + f' -> {verdict}; this run alone (C, D, pooled C+D): {dec["this_run_C_D_pooledCD"]}**')
            mo = C(ours, 'trk', w, lambda r: zcount(r, '100..500', 'manifolds'), BL['pooled A-D'])
            to, tj = cw[(ours, 'pooled A-D', '100..500')], cw[('H-jolt56', 'pooled A-D', '100..500')]
            if mo and to and tj and mj:
                uo, uj = to['median'] * 1e3 / mo['median'], tj['median'] * 1e3 / mj
                P(f'  - per manifold [100,500) pooled A-D: ours {uo:.4f} us ({mo["median"]:.2f} manifolds/step), Jolt '
                  f'{uj:.4f} us ({mj} manifolds/step, window 7 -receipt) -> ours/Jolt {uo / uj:.3f}x')
        for row in ('H-jolt56', 'H-trk-tree', 'H-trk-ap'):
            parts = []
            for a, b in (('A', 'B'), ('A', 'C'), ('C', 'D'), ('B', 'D')):
                c = cmp_(cw[(row, a)], cw[(row, b)])
                R['cmp'][f'Q1 block term {row} W{w} {b} vs {a}'] = c
                parts.append(f'{b}/{a} {c["ratio"]:.4f} ({"C" if c["claimed"] else "n"})' if c else f'{b}/{a} n/a')
            P(f'- block terms {row} W{w}: ' + '; '.join(parts))
        jc = cw[('H-jolt56', 'pooled A-D')]
        if jc:
            P(f'- window term: Jolt56 pooled A-D / window 3 = {jc["median"] / WIN3_JOLT56[w]:.4f} (context only)')
    w8 = R['decisions'].get('Q1 H-trk-tree/jolt56 W8', {})
    R['decisions']['Q1 W8 headline'] = w8
    P(f"\n**Decision Q1 (W8, the default row against Jolt v5.6.0, blocks A-D and pooled):** {w8.get('verdict')}")

    # ------------------------------------------------------------------ Q2
    P('\n## Q2 C3b t_q block-shift test: parent 6dd1f916 (RowWalk) vs tip 983480a9 (LeafList), blocks C and D')
    P('t_q = per-process median over [100,500) of phys_bp_query_ns, median over K. Layouts: armed = window 6 P3 '
      '(cwd 163, 648 chars), pad06 = window 6 P5g (cwd 166, 654 chars); length misses excluded. '
      f'Window 6 context (tip, W1): {WIN6_TQ}')
    QB = {'C': ('Q2C',), 'D': ('Q2D',), 'pooled': ('Q2C', 'Q2D')}

    def tq(r):
        return zone(r, '100..500', 'phys_bp_query_ns', 'median')

    def span(r):
        v = [zone(r, '100..500', c, 'median') for c in ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns',
                                                         'phys_bp_assemble_ns')]
        return sum(v) if all(x is not None for x in v) else None

    T = {}
    for rid, ws in (('C3b-TA-armed', (1, 8)), ('C3b-TA-pad06', (1,)), ('C3b-TD-armed', (1,))):
        for w in ws:
            for key in ('par', 'tip'):
                for bn, bl in QB.items():
                    c = C(rid, key, w, tq, bl, excl_miss=True)
                    T[(rid, key, w, bn)] = c
                    show(f't_q {rid} {key}@W{w} [{bn}] ms', c)
                sp = C(rid, key, w, span, QB['pooled'], excl_miss=True)
                show(f'tree span {rid} {key}@W{w} [pooled] ms (limit {SPAN_LIMIT.get(w)})', sp)
                st = C(rid, key, w, wall, QB['pooled'], excl_miss=True)
                show(f'step {rid} {key}@W{w} [pooled] [0,500) ms', st)
            for bn in QB:
                c = cmp_(T[(rid, 'par', w, bn)], T[(rid, 'tip', w, bn)])
                R['cmp'][f'Q2 t_q tip vs par {rid} W{w} [{bn}]'] = c
                P(f'- t_q tip vs par {rid} W{w} [{bn}]: {fcmp(c)}')
    for key in ('par', 'tip'):
        for rid, w in (('C3b-TA-armed', 1), ('C3b-TA-armed', 8), ('C3b-TA-pad06', 1), ('C3b-TD-armed', 1)):
            c = cmp_(T[(rid, key, w, 'C')], T[(rid, key, w, 'D')])
            R['cmp'][f'Q2 block term D vs C {rid} {key} W{w}'] = c
            P(f'- BLOCK TERM t_q D vs C {rid} {key}@W{w}: {fcmp(c)}')
        for bn in QB:
            c = cmp_(T[('C3b-TA-armed', key, 1, bn)], T[('C3b-TA-pad06', key, 1, bn)])
            R['cmp'][f'Q2 layout term pad06 vs armed {key} W1 [{bn}]'] = c
            P(f'- LAYOUT TERM t_q pad06 vs armed {key}@W1 [{bn}]: {fcmp(c)}')
    lay = [R['cmp'].get(f'Q2 layout term pad06 vs armed tip W1 [{b}]') for b in QB]
    blk = R['cmp'].get('Q2 block term D vs C C3b-TA-armed tip W1')
    lay_claimed = [b for b, c in zip(QB, lay) if c and c['claimed']]
    R['decisions']['Q2 shift attribution'] = (
        f"layout term (tip W1) claimed in {lay_claimed or 'no block'}; block term D vs C (tip W1, plain) "
        f"{'CLAIMED' if blk and blk['claimed'] else 'not claimed'}")
    P(f"**Q2 attribution:** {R['decisions']['Q2 shift attribution']}")

    def c2_verdict(rid):
        vs = {bn: vs_bar(T[(rid, 'tip', 1, bn)], C2_BAR) for bn in QB}
        if not all(vs.values()):
            return vs, 'NO DATA'
        if all(v['claimed'] and v['side'] == 'above' for v in vs.values()):
            return vs, 'C2 BUILT (>= 0.235 claimed above in C, D and pooled)'
        if all(v['claimed'] and v['side'] == 'below' for v in vs.values()):
            return vs, 'C2 NOT BUILT (< 0.235 claimed below in C, D and pooled)'
        return vs, 'UNRESOLVED (on the bar in at least one of C, D, pooled) - the orchestrator decides'

    for rid, what in (('C3b-TD-armed', "the letter's row: the DEFAULT row armed (c3b/design.md:231, :372)"),
                      ('C3b-TA-armed', "window 6's reading: the cfg-A armed row, plain layout (window 6 P3)"),
                      ('C3b-TA-pad06', 'the cfg-A armed row, padded layout (window 6 P5g), for robustness')):
        vs, verdict = c2_verdict(rid)
        for bn, v in vs.items():
            P(f'- C2 bar, {rid} tip W1 [{bn}]: {fbar(v)}')
        t = T[(rid, 'tip', 1, 'pooled')]
        extra = ''
        if t:
            a = vs_bar(t, ATTR_A)
            b = vs_bar(t, BAND)
            extra = (f"; > 0.21 (attribution A): {fbar(a)}; <= 0.186 (into the band): {fbar(b)}; "
                     f"c_q = {t['median'] * 1e6 / ROWS_Q:.1f} ns/row ({'> 150: F3' if t['median'] * 1e6 / ROWS_Q > 150 else '<= 150'})")
        R['decisions'][f'Q2 C2 {rid}'] = verdict
        P(f'**Decision Q2 C2 on {what}: {verdict}**{extra}')

    # ------------------------------------------------------------------ Q3
    P('\n## Q3 The tree thresholds between 64 and 256 (recipe 1.4), instrument 93b2615b + G4_SIZES, K = 3')
    crow = next(r for r in ROWS['rows'] if r['kind'] == 'criterion')
    sizes = sorted({int(n.rsplit('/', 1)[1]) for n in crow['expect']})
    fam_res = {}
    for fam in ('uniform', 'disparity'):
        P(f'\n### {fam}')
        per = []
        for n in sizes:
            a = C('G4-refine', 'g4r', 1, lambda r, i=f'bp_g4_{fam}/all_pairs/{n}': ((r.get('criterion') or {}).get('est_ms') or {}).get(i, [None, None])[1], ('Q3',))
            t = C('G4-refine', 'g4r', 1, lambda r, i=f'bp_g4_{fam}/tree/{n}': ((r.get('criterion') or {}).get('est_ms') or {}).get(i, [None, None])[1], ('Q3',))
            c = cmp_(t, a)  # all_pairs against tree: ratio = all_pairs / tree
            R['cmp'][f'Q3 {fam} all_pairs/tree n={n}'] = c
            slower = bool(c and c['claimed'] and c['ratio'] > 1)
            faster = bool(c and c['claimed'] and c['ratio'] < 1)
            per.append((n, c, slower, faster))
            w4 = WIN4_G4[fam].get(n)
            P(f'- n={n}: all_pairs {fc(a, 6) if a else "n/a"} ms; tree {fc(t, 6) if t else "n/a"} ms; all_pairs/tree '
              f'{fcmp(c)}{"  <- all_pairs SLOWER (tree claimed faster)" if slower else ""}'
              f'{f"  [window 4, before F1: {w4}]" if w4 else ""}')
        have = [x for x in per if x[1]]
        lo = max((n for n, c, s, f in have if not s), default=None)
        hi = min((n for n, c, s, f in have if s), default=None)
        mono = hi is None or all((n >= hi) == s for n, c, s, f in have)
        cross = None
        for (n0, c0, _, _), (n1, c1, _, _) in zip(have, have[1:]):
            if c0['ratio'] <= 1 < c1['ratio']:
                x0, x1, y0, y1 = math.log(n0), math.log(n1), math.log(c0['ratio']), math.log(c1['ratio'])
                cross = math.exp(x0 + (0 - y0) * (x1 - x0) / (y1 - y0))
                break
        fam_res[fam] = {'LO': lo, 'HI': hi, 'monotone': mono, 'loglog_crossover': cross}
        P(f'**{fam}: largest n with all_pairs not claimed slower = {lo}; smallest n with tree claimed faster = {hi}; '
          f'monotone {mono}; log-log crossover {cross if cross is None else round(cross, 1)}**')
        # the bridge: C1's per-row walk (tree_rowwalk, same binary) at window 4's grid points. The design
        # (c3b/design.md:244) predicted "the tree gets cheaper at every n, so the crossovers move down".
        for n in sorted({int(x.rsplit('/', 1)[1]) for x in crow['expect'] if '/tree_rowwalk/' in x}):
            def est(r, i):
                return ((r.get('criterion') or {}).get('est_ms') or {}).get(i, [None, None])[1]
            rw = C('G4-refine', 'g4r', 1, lambda r, i=f'bp_g4_{fam}/tree_rowwalk/{n}': est(r, i), ('Q3',))
            ll = C('G4-refine', 'g4r', 1, lambda r, i=f'bp_g4_{fam}/tree/{n}': est(r, i), ('Q3',))
            ap = C('G4-refine', 'g4r', 1, lambda r, i=f'bp_g4_{fam}/all_pairs/{n}': est(r, i), ('Q3',))
            c1 = cmp_(rw, ll)
            c2 = cmp_(rw, ap)
            R['cmp'][f'Q3 {fam} LeafList/RowWalk n={n}'] = c1
            R['cmp'][f'Q3 {fam} all_pairs/tree_rowwalk n={n}'] = c2
            P(f'  - bridge n={n}: tree_rowwalk {fc(rw, 6) if rw else "n/a"} ms; LeafList/RowWalk {fcmp(c1)}; '
              f'all_pairs/tree_rowwalk {c2["ratio"] if c2 else float("nan"):.3f} (window 4, the same kernel: '
              f'{WIN4_G4[fam].get(n)})')
    if all(fam_res[f]['LO'] is not None for f in fam_res):
        tbmr = min(fam_res[f]['LO'] for f in fam_res)
        his = [fam_res[f]['HI'] for f in fam_res]
        auto_hi = max(his) if all(h is not None for h in his) else None
        auto_lo = tbmr
        crosses = [fam_res[f]['loglog_crossover'] for f in fam_res]
        l2_hi = None
        if all(x is not None for x in crosses):
            big = max(crosses)
            mag = 10 ** (math.floor(math.log10(big)) - 1)
            l2_hi = round(big / mag) * mag
        dec = {'TREE_BRUTE_MAX_ROWS': tbmr, 'AUTO_TREE_LO': auto_lo, 'AUTO_TREE_HI': auto_hi,
               'L2_HI': l2_hi, 'L2_LO': round(0.9 * l2_hi) if l2_hi else None, 'families': fam_res,
               'grid_edge': (f'the answer sits on the grid edge ({tbmr} or {auto_hi}): extend the grid'
                             if tbmr in (sizes[0], sizes[-1]) or auto_hi in (None, sizes[0]) else 'inside the grid')}
    else:
        dec = {'verdict': 'NO DATA or a family is claimed slower at the smallest size', 'families': fam_res}
    R['decisions']['Q3 constants'] = dec
    P(f'**Decision Q3 (recipe 1.4 grid rule; L2 procedure beside it):** {dec}')

    # ------------------------------------------------------------------ Q4
    P("\n## Q4 G-TW's canary resolution on the C4 binary 989ca0f0 (J-A W1, reuse on, unarmed)")
    cp = ROWS['protocol']['canary']
    P(f"R = {100 * cp['R_gtw']:.3f} % = window 7's G-TW J-A W1 [0,500) two-spread bar (2 hypot(3.352, 1.728) %); "
      f"rise/injected assumed {cp['rise_over_injected']:.4f} (window 7's armed canary). Reference = G-TW's own 'on' "
      'row; window 7 read it 16.6838 ms.')
    ref = C('L9-JA-on', 'c4', 1, wall, ('Q4',))
    show('L9-JA-on c4@W1 (reference) [0,500) ms', ref)
    seen = {}
    for r in ROWS['rows']:
        if not r.get('canary_of'):
            continue
        k = r['canary_k']
        jc = C(r['id'], 'c4', 1, wall, ('Q4',))
        cns = C(r['id'], 'c4', 1, lambda x: ((x.get('summary') or {}).get('canary_ns') or 0) / 1e6 or None, ('Q4',))
        show(f"{r['id']} c4@W1 (frac {r['canary_frac']} = {k} R) [0,500) ms", jc)
        show(f"{r['id']} injected canary ms", cns)
        c = cmp_(ref, jc)
        R['cmp'][f'Q4 canary {k}R rise'] = c
        ok = bool(c and c['claimed'] and c['ratio'] > 1 and cns)
        ok_s = bool(c and c['cl_s'] and c['ratio'] > 1)
        seen[k] = ok
        if c and cns:
            P(f"- rung {k} R: rise {c['delta']:+.4f} ms = {100 * c['effect']:+.2f} % (injected {cns['median']:.4f} ms, "
              f"rise/injected {c['delta'] / cns['median']:.3f}); {fcmp(c)} -> {'SEEN' if ok else 'NOT SEEN'} "
              f"(SE alone: {'seen' if ok_s else 'not seen'})")
    ks = sorted(seen)
    demo = [k for k in ks if all(seen[j] for j in ks if j >= k)]
    res_k = min(demo) if demo else None
    verdict = ('DEMONSTRATED: the 1.5 R and 2 R rungs are SEEN' if seen.get(1.5) and seen.get(2.0) else
               'NOT DEMONSTRATED: the 1.5 R or 2 R rung is not seen')
    R['decisions']['Q4 canary'] = {'seen': seen, 'resolution_demonstrated_k': res_k,
                                   'resolution_demonstrated_pct': 100 * res_k * cp['R_gtw'] if res_k else None,
                                   'verdict': verdict}
    res_txt = f'{res_k} R = {100 * res_k * cp["R_gtw"]:.2f} % of the step' if res_k else 'none'
    P(f'**Decision Q4:** {verdict}; smallest rung seen with every larger rung seen: {res_txt}')

    json.dump(R, open(os.path.join(RAW, 'reduction.json'), 'w', encoding='utf-8', newline='\n'), indent=1, default=str)
    open(os.path.join(RAW, 'tables.md'), 'w', encoding='utf-8', newline='\n').write('\n'.join(OUT) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
