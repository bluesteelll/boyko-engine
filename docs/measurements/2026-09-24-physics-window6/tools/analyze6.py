#!/usr/bin/env python3
"""Window 6 analyst reduction, independent of tools/reduce6.py.

Recomputes every per-process statistic from the per-step `run.csv` files (and the class-bench /
criterion stdout), re-derives "clean" from the receipts, applies the slot rule, and prints every
cell, comparison and gate reading the analysis quotes. Bridges to windows 4b and 5 are recomputed
from those windows' own raw files with the same rule.

Statistic (as ruled, windows 3-5): a cell is the MEDIAN over K processes; spreads relative to the
median: r = (max - min), i = IQR (inclusive quartiles), s = 1.2533 * sample SD / sqrt K.
B against A is claimed iff |B/A - 1| > 2*hypot(sA, sB); this window's protocol requires the claim
under BOTH r and s (i printed). A cell against a constant bar uses the cell's own spread only.

Outputs: <win6>/analyst/reduction.json and <win6>/analyst/tables.txt.
"""
import csv
import json
import math
import os
import re
import statistics
import sys
from collections import Counter, defaultdict

WIN6 = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/7dc37fc3-7b1e-46f1-89da-ccd0dba7c788/scratchpad/win6'
OLD = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad'
OUTDIR = os.path.join(WIN6, 'analyst')
BUSY = 5.0

OUT = []


def say(*a):
    OUT.append(' '.join(str(x) for x in a))


# ---------------------------------------------------------------- statistics

def cell(xs):
    xs = sorted(x for x in xs if x is not None)
    K = len(xs)
    if K == 0:
        return None
    med = statistics.median(xs)
    if K >= 2:
        q = statistics.quantiles(xs, n=4, method='inclusive')
        iqr = q[2] - q[0]
        se = 1.2533 * statistics.stdev(xs) / math.sqrt(K)
    else:
        iqr = se = 0.0
    den = abs(med) if med != 0 else float('nan')
    return {'K': K, 'med': med, 'min': xs[0], 'max': xs[-1], 'vals': xs,
            'r': (xs[-1] - xs[0]) / den * 100, 'i': iqr / den * 100, 's': se / den * 100,
            'se_abs': se, 'range_abs': xs[-1] - xs[0]}


def cmp(A, B):
    """B against A (effect = B/A - 1)."""
    if A is None or B is None or A['med'] == 0:
        return None
    ratio = B['med'] / A['med']
    eff = (ratio - 1) * 100
    bars = {k: 2 * math.hypot(A[k], B[k]) for k in ('r', 'i', 's')}
    cl = {k: abs(eff) > bars[k] for k in bars}
    return {'ratio': ratio, 'eff': eff, 'delta': B['med'] - A['med'], 'bars': bars, 'cl': cl,
            'claimed': cl['r'] and cl['s'], 'se_ms': 2 * math.hypot(A['se_abs'], B['se_abs']),
            'worst_B_minus_A_hi': B['max'] - A['min'], 'worst_B_minus_A_lo': B['min'] - A['max']}


def vs_bar(C, bar):
    """A cell against a constant bar: claimed iff |med/bar - 1| > 2*spread (own spread only)."""
    eff = (C['med'] / bar - 1) * 100
    cl = {k: abs(eff) > 2 * C[k] for k in ('r', 'i', 's')}
    return {'eff': eff, 'cl': cl, 'claimed': cl['r'] and cl['s']}


def f(x, n=3):
    if x is None:
        return '-'
    return f'{x:.{n}f}'


def cstr(C, n=3):
    if C is None:
        return '-'
    return f"{C['med']:.{n}f} [{C['min']:.{n}f}-{C['max']:.{n}f}] K={C['K']} ({C['r']:.2f}/{C['i']:.2f}/{C['s']:.2f})"


def yn(b):
    return 'Y' if b else 'n'


def kstr(c):
    if c is None:
        return '-'
    return (f"ratio {c['ratio']:.4f} ({c['eff']:+.2f} %, d {c['delta']:+.4f}); bars {c['bars']['r']:.1f}/"
            f"{c['bars']['i']:.1f}/{c['bars']['s']:.1f} %; claimed r/i/s {yn(c['cl']['r'])}/{yn(c['cl']['i'])}/"
            f"{yn(c['cl']['s'])}{' CLAIMED(r&s)' if c['claimed'] else ''}; 2hypot(SE)={c['se_ms']:.4f}")


# ---------------------------------------------------------------- per-process CSV reduction

SKIP_COLS = {'step', 'top_y', 'void', 'disp_lane', 'worker_lane_max'}
BP4 = ['phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns']


def read_csv(path):
    with open(path, newline='') as fh:
        rd = csv.reader(fh)
        hdr = next(rd)
        rows = [r for r in rd]
    cols = {}
    for j, h in enumerate(hdr):
        if h in SKIP_COLS or (h.endswith('_n') and not h.endswith('_ns')):
            continue
        vals = []
        for r in rows:
            v = r[j] if j < len(r) else ''
            vals.append(float(v) if v != '' else None)
        cols[h] = vals
    steps = [int(r[0]) for r in rows]
    return steps, cols


def win_stats(steps, cols, a, b):
    idx = [k for k, s in enumerate(steps) if a <= s < b]
    out = {'n': len(idx)}
    for h, vals in cols.items():
        xs = [vals[k] for k in idx if vals[k] is not None]
        if xs:
            out[h] = (statistics.fmean(xs), statistics.median(xs))
    if all(h in cols for h in BP4):
        per = [sum(cols[h][k] or 0 for h in BP4) for k in idx]
        out['bp4_sum_of_medians'] = sum(out[h][1] for h in BP4 if h in out)
        out['bp4_perstep'] = (statistics.fmean(per), statistics.median(per))
    return out


def frozen_from(steps, cols):
    aw = cols.get('awake')
    if not aw or all(v is None for v in aw):
        return None
    last_awake = None
    for s, v in zip(steps, aw):
        if v is not None and v > 0:
            last_awake = s
    if last_awake is None:
        return steps[0]
    return last_awake + 1 if last_awake + 1 <= steps[-1] else None


def parse_summary(stdout_path):
    s = None
    with open(stdout_path, encoding='utf-8', errors='replace') as fh:
        for line in fh:
            if line.startswith('SUMMARY'):
                s = json.loads(line[len('SUMMARY'):].strip())
    return s


CRIT_RE = re.compile(r'(bp_g4_\S+)\s+time:\s+\[\s*([\d.]+)\s*(\S+)\s+([\d.]+)\s*(\S+)\s+([\d.]+)\s*(\S+)\s*\]', re.S)


def unit_ms(u):
    if u == 'ms':
        return 1.0
    if u == 'ns':
        return 1e-6
    if u == 's':
        return 1e3
    return 1e-3  # the micro sign in any encoding


def parse_criterion(stdout_path):
    txt = open(stdout_path, encoding='utf-8', errors='replace').read()
    est = {}
    for m in CRIT_RE.finditer(txt):
        est[m.group(1)] = float(m.group(4)) * unit_ms(m.group(5))
    return est


# ---------------------------------------------------------------- selection

def is_clean(r):
    """Recomputed from the receipts, independent of the driver's flags."""
    why = []
    for side in ('receipt_before', 'receipt_after'):
        rc = r.get(side) or {}
        if rc.get('cpu_avg', 0) > BUSY:
            why.append(f"{side} {rc.get('cpu_avg')}%")
        pres = rc.get('presence') or {}
        if pres.get('build') or pres.get('lane'):
            why.append(f'{side} presence {pres}')
        if rc.get('build_procs_busy'):
            why.append(f'{side} build busy')
    if r.get('build_proc_during'):
        why.append('build during')
    if r.get('void_names_new_during'):
        why.append('new void names during')
    return (len(why) == 0), why


def select(procs, slot_key):
    """Per slot: original if valid and clean, else its re-run if valid and clean, else dropped."""
    slots = defaultdict(dict)
    for r in procs:
        if not r.get('timed'):
            continue
        slots[slot_key(r)].setdefault(r['attempt'], []).append(r)
    chosen, dropped = [], []
    for k, d in slots.items():
        pick = None
        for att in ('original', 'rerun'):
            for r in d.get(att, []):
                ok, _ = is_clean(r)
                if r.get('valid') and r.get('exit') == 0 and ok:
                    pick = r
                    break
            if pick:
                break
        if pick:
            chosen.append(pick)
        else:
            dropped.append((k, d))
    return chosen, dropped, slots


# ---------------------------------------------------------------- window 6

def load_jsonl(p):
    return [json.loads(l) for l in open(p, encoding='utf-8')]


def main():
    rows6 = json.load(open(os.path.join(WIN6, 'rows6.json')))
    rowspec = {r['id']: r for r in rows6['rows']}
    fixtures = json.load(open(os.path.join(WIN6, 'gate/fixtures/fixtures.json')))
    sums = {}
    for line in open(os.path.join(WIN6, 'bin/SHA256SUMS')):
        h, name = line.split()
        sums[name.lstrip('*')] = h
    recs = load_jsonl(os.path.join(WIN6, 'raw/runs.jsonl'))
    procs = [r for r in recs if 'row' in r]
    markers = [r for r in recs if 'row' not in r]
    say('# Window 6 analyst reduction (analyze6.py)')
    say(f'records {len(recs)}: processes {len(procs)}, pass markers {len(markers)}')
    say('attempts', dict(Counter(r['attempt'] for r in procs)))

    # driver flag vs recomputed clean
    disagree = []
    for r in procs:
        ok, why = is_clean(r)
        if ok == bool(r['contaminated']):
            disagree.append((r['row'], r['binary'], r['W'], r['attempt'], ok, r['contaminated'], why))
    say(f'clean recomputed vs driver contaminated flag: disagreements {len(disagree)}')
    for d in disagree[:20]:
        say('  DISAGREE', d)

    key6 = lambda r: (r['run_tag'], r['block'], r['pass'], r['pass_attempt'], r['round'], r['row'], r['binary'], r['W'])
    chosen, dropped, slots = select(procs, key6)
    say(f'slots {len(slots)}; chosen {len(chosen)}; dropped {len(dropped)}')
    for k, d in dropped:
        say('  DROPPED', k, {a: [(x['receipt_before']['cpu_avg'], x['receipt_after']['cpu_avg'], round(x['mean_ms'], 4) if x.get('mean_ms') else None) for x in v] for a, v in d.items()})
    reruns_orphan = [k for k, d in slots.items() if 'original' not in d]
    say('slots without an original', len(reruns_orphan))
    say('re-runs chosen', sum(1 for r in chosen if r['attempt'] == 'rerun'))

    # receipts over all distinct receipts
    rtimes = {}
    for r in procs:
        for side in ('receipt_before', 'receipt_after'):
            rc = r[side]
            rtimes[rc['time']] = rc['cpu_avg']
    rv = sorted(rtimes.values())
    say(f'distinct 5/10-s receipts {len(rv)}: median {statistics.median(rv):.2f} %, p90 {statistics.quantiles(rv, n=10, method="inclusive")[8]:.2f} %, max {rv[-1]:.2f} %, over 5 %: {sum(v > BUSY for v in rv)}')
    wit = sorted(r['others_busy_pct'] for r in chosen)
    say(f'during-process witness over chosen: median {statistics.median(wit):.2f} %, p95 {statistics.quantiles(wit, n=20, method="inclusive")[18]:.2f} %, max {wit[-1]:.2f} %, over 5 %: {sum(w > BUSY for w in wit)}')
    hi_wit = [(r['row'], r['binary'], r['W'], r['others_busy_pct'], round(r['mean_ms'], 4) if r.get('mean_ms') else None) for r in chosen if r['others_busy_pct'] > 3.0]
    say('  witness > 3 %:', hi_wit)
    say('block times', {b: (min(r['start'] for r in chosen if r['block'] == b)[11:19], max(r['end'] for r in chosen if r['block'] == b)[11:19]) for b in sorted({r['block'] for r in chosen})})
    tops = Counter()
    for r in procs:
        if r['contaminated']:
            for t in r['receipt_after']['top5'][:1]:
                tops[t['name']] += 1
    say('top process in contaminated after-receipts', dict(tops))

    # ---- per-process reduction and structure checks
    P = []  # reduced processes
    problems = []
    for r in chosen:
        spec = rowspec[r['row']]
        kind = spec.get('kind', 'runner')
        exe = os.path.basename(r['exe'])
        if sums.get(exe) != r['exe_sha256']:
            problems.append((r['row'], 'sha256', exe))
        if r['commit'] != rows6['binaries'][r['binary']]['commit']:
            problems.append((r['row'], 'commit'))
        d = {'row': r['row'], 'bin': r['binary'], 'W': r['W'], 'pass': r['pass'], 'round': r['round'],
             'attempt': r['attempt'], 'block': r['block'], 'start': r['start'], 'wit': r['others_busy_pct'],
             'rb': r['receipt_before']['cpu_avg'], 'ra': r['receipt_after']['cpu_avg'], 'kind': kind}
        if kind == 'runner':
            s = r['summary']
            steps, cols = read_csv(os.path.join(r['cwd'], 'run.csv'))
            a, b = spec['window']
            d['steps'] = len(steps)
            d['win'] = win_stats(steps, cols, a, b)
            if abs(d['win']['wall_ns'][0] / 1e6 - r['mean_ms']) > 1e-9:
                problems.append((r['row'], 'mean_ms mismatch', d['win']['wall_ns'][0] / 1e6, r['mean_ms']))
            if abs(d['win']['wall_ns'][0] - s['window_mean_ns']) > 1e-3:
                problems.append((r['row'], 'window_mean_ns mismatch'))
            if d['win']['n'] != b - a:
                problems.append((r['row'], 'window steps', d['win']['n']))
            if spec['steps'] == 500:
                d['mw'] = win_stats(steps, cols, 100, 500)
                d['w0'] = win_stats(steps, cols, 0, 100)
            ff = frozen_from(steps, cols)
            d['ff_csv'] = ff
            d['ff_summary'] = s.get('first_frozen_step')
            if s.get('first_frozen_step') is not None:
                d['tail'] = win_stats(steps, cols, s['first_frozen_step'], spec['steps'])
                # The CSV's `awake` is the post-step count, so the last step that still had awake
                # bodies is (csv value - 1) and the runner's first all-frozen step is csv + 1.
                if ff is None or ff + 1 != s['first_frozen_step']:
                    problems.append((r['row'], 'frozen step csv vs summary', ff, s['first_frozen_step']))
            # structure
            fx = fixtures[spec['pose_ref']]['hash']
            if s['pose_hash'] != fx or s['expect_pose'] != 'match':
                problems.append((r['row'], 'pose', s['pose_hash'], s['expect_pose']))
            if s['void_steps'] != 0 or s.get('first_void') is not None:
                problems.append((r['row'], 'void'))
            if s['workers'] != r['W'] or s['threads']['pool_workers'] != r['W']:
                problems.append((r['row'], 'workers'))
            if s['target_env'] != 'msvc':
                problems.append((r['row'], 'env'))
            if bool(s['armed']) != bool(spec['armed']):
                problems.append((r['row'], 'armed'))
            if spec['armed'] and s.get('drops_total') != 0:
                problems.append((r['row'], 'drops', s.get('drops_total')))
            if not spec['armed'] and s.get('disarmed_ring_traffic') not in (None, 0):
                problems.append((r['row'], 'ring traffic'))
            if spec['armed'] and s['threads'].get('solve_on_dispatcher_steps', 0) != 0:
                problems.append((r['row'], 'solve on dispatcher'))
            if s['config']['broadphase'] != spec['broadphase']:
                problems.append((r['row'], 'broadphase', s['config']['broadphase']))
            td = s.get('broadphase_tree') or {}
            if spec['broadphase'] == 'Tree':
                want = dict(static_rebuilds=1, members=1, evictions=0)
                bad = {k: td.get(k) for k, v in want.items() if td.get(k) != v}
                if bad:
                    problems.append((r['row'], 'treediag', bad))
                d['treediag'] = td
            else:
                if any(v for v in td.values()):
                    problems.append((r['row'], 'treediag nonzero on AllPairs', td))
            if r['mask_readback'] != '0xffff':
                problems.append((r['row'], 'mask'))
            d['config'] = s['config']
            d['pair_classes'] = s.get('pair_classes')
            d['canary_ns'] = s.get('canary_ns')
            d['final_manifolds'] = s.get('final_manifolds')
            d['pose'] = s['pose_hash']
        elif kind == 'classes':
            s = parse_summary(os.path.join(r['cwd'], 'stdout.txt'))
            d['classes'] = s
            txt = open(os.path.join(r['cwd'], 'stdout.txt'), encoding='utf-8', errors='replace').read()
            m = re.findall(r'(\w+): design refutation reading dt_np\(1\) in \[([\d.]+), ([\d.]+)\] ms', txt)
            d['printed_band'] = {k: (float(lo), float(hi)) for k, lo, hi in m}
            if r['exit'] != 0 or s is None:
                problems.append((r['row'], 'classes no summary'))
        elif kind == 'criterion':
            est = parse_criterion(os.path.join(r['cwd'], 'stdout.txt'))
            d['crit'] = est
            if len(est) != 8:
                problems.append((r['row'], 'criterion estimates', len(est)))
        P.append(d)
    say(f'structure problems over {len(P)} chosen processes: {len(problems)}')
    for p in problems[:40]:
        say('  PROBLEM', p)
    poses = Counter((d['row'], d.get('pose')) for d in P if d['kind'] == 'runner')
    say('poses per row:', sorted({(k[0], k[1]) for k in poses}))

    # ---- indexing helpers
    def procs_of(row, b, W):
        return [d for d in P if d['row'] == row and d['bin'] == b and d['W'] == W]

    def C(row, b, W, fn):
        xs = []
        for d in procs_of(row, b, W):
            try:
                xs.append(fn(d))
            except (KeyError, TypeError):
                pass
        return cell(xs)

    T = lambda d: d['win']['wall_ns'][0] / 1e6
    Tmw = lambda d: d['mw']['wall_ns'][0] / 1e6
    Tw0 = lambda d: d['w0']['wall_ns'][0] / 1e6
    Ttail = lambda d: d['tail']['wall_ns'][0] / 1e6

    def span(col, win='win', stat=0):
        return lambda d: d[win][col][stat] / 1e6

    R = {}  # everything for the json

    # ================================================================ P1
    say('\n## P1 L10 pre-C0 refutation (bar 0.584 ms)')
    for row in ('L10-Offp', 'L10-Son'):
        for W in (1,):
            c = C(row, 'b4db', W, T)
            ct = C(row, 'b4db', W, Ttail)
            say(f'{row}@W{W} wall [264,1000): {cstr(c, 4)}')
            say(f'{row}@W{W} wall tail [ffs,1000): {cstr(ct, 4)}  ffs(summary)={sorted({d["ff_summary"] for d in procs_of(row, "b4db", W)})} ffs(csv)={sorted({d["ff_csv"] for d in procs_of(row, "b4db", W)})}')
            R[f'P1/{row}@{W}'] = {'win': c, 'tail': ct}
    off = R['P1/L10-Offp@1']
    for lab in ('win', 'tail'):
        vb = vs_bar(off[lab], 0.584)
        say(f'Off-prime {lab} vs 0.584: med {off[lab]["med"]:.4f} ({vb["eff"]:+.1f} % over the bar); below bar? {off[lab]["med"] < 0.584}; min {off[lab]["min"]:.4f}; claimed r/i/s {vb["cl"]}')
    say('Son vs Offp (Offp against Son):', kstr(cmp(R['P1/L10-Son@1']['win'], off['win'])))

    SPANS = [('bp', 'sys_physics_broadphase_ns'), ('np', 'sys_physics_narrowphase_ns'),
             ('graph', 'sys_physics_build_graph_ns'), ('solve', 'sys_physics_solve_colored_ns'),
             ('gather', 'sys_physics_gather_ns'), ('integrate', 'sys_physics_integrate_ns'),
             ('apply', 'sys_physics_apply_ns'), ('select_bp', 'sys_select_broadphase_ns'),
             ('sys_sum', 'sys_sum_ns'), ('g', 'g_ns'), ('u', 'u_ns'), ('r', 'r_ns'),
             ('bp_verify', 'phys_bp_verify_ns'), ('bp_build', 'phys_bp_build_ns'), ('bp_query', 'phys_bp_query_ns'),
             ('bp_assemble', 'phys_bp_assemble_ns'), ('solve_build', 'phys_solve_build_ns'),
             ('warm_apply', 'phys_warm_apply_ns'), ('store', 'phys_store_ns'), ('integrate_in_solve', 'phys_integrate_ns'),
             ('gravity', 'phys_gravity_ns'), ('wide', 'phys_color_wide_ns'), ('narrow', 'phys_color_narrow_ns'),
             ('biased', 'phys_pass_biased_ns'), ('relax', 'phys_pass_relax_ns'), ('write_back', 'phys_write_back_ns'),
             ('sleep_begin', 'phys_sleep_begin_ns'), ('sleep_freeze', 'phys_sleep_freeze_ns'), ('sleep_end', 'phys_sleep_end_ns'),
             ('np_dispatch', 'phys_np_dispatch_ns'), ('np_compact', 'phys_np_compact_ns'), ('np_axis_commit', 'phys_np_axis_commit_ns')]

    def span_table(rows_bins, win, tag, Ws):
        for row, b in rows_bins:
            for W in Ws:
                if not procs_of(row, b, W):
                    continue
                say(f'  spans {row}#{b}@W{W} over {win} (per-process mean -> median over K, ms; B = per-process median):')
                out = {}
                for nm, col in SPANS:
                    ca = C(row, b, W, span(col, win, 0))
                    cb = C(row, b, W, span(col, win, 1))
                    if ca is None or (ca['med'] == 0 and ca['max'] == 0):
                        continue
                    out[nm] = {'A': ca, 'B': cb}
                    say(f'    {nm:18s} A {cstr(ca, 4)} | B {cb["med"]:.4f}')
                if procs_of(row, b, W)[0][win].get('bp4_sum_of_medians') is not None:
                    cs = C(row, b, W, lambda d: d[win]['bp4_sum_of_medians'] / 1e6)
                    out['tree_span_B'] = cs
                    say(f'    tree span (sum of 4 per-col medians) {cstr(cs, 4)}')
                cw = C(row, b, W, span('wall_ns', win, 0))
                rem = C(row, b, W, lambda d: (d[win]['wall_ns'][0] - sum(d[win][c][0] for c in ('sys_physics_broadphase_ns', 'sys_physics_narrowphase_ns', 'sys_physics_build_graph_ns', 'sys_physics_solve_colored_ns'))) / 1e6)
                out['wall'] = cw
                out['rem_wall_minus_bp_np_graph_solve'] = rem
                say(f'    wall A {cstr(cw, 4)}; remainder (wall - bp - np - graph - solve) {cstr(rem, 4)}')
                mf = C(row, b, W, lambda d: d[win]['manifolds'][0])
                pr = C(row, b, W, lambda d: d[win]['pairs'][0])
                say(f'    manifolds {f(mf["med"], 2)} pairs {f(pr["med"], 2)}')
                out['manifolds'] = mf
                out['pairs'] = pr
                R[f'{tag}/{row}#{b}@{W}/{win}'] = out

    span_table([('L10-Offp', 'b4db'), ('L10-Son', 'b4db')], 'win', 'P1spans', (1,))
    span_table([('L10-Offp', 'b4db'), ('L10-Son', 'b4db')], 'tail', 'P1spans', (1,))

    # ================================================================ P2
    say('\n## P2 L9: (a) class bench band')
    cl = [d for d in P if d['row'] == 'L9-classes']
    per = []
    for d in cl:
        sc = {x['scene']: x for x in d['classes']['scenes']}
        j = sc['jolt']
        ts = j['timings_not_a_result']['separated']['ns_per_pair']
        tt = j['timings_not_a_result']['touching']['ns_per_pair']
        ns, nt = j['slow_separated'], j['slow_touching']
        lo = (ns * (ts - 60) + 0.80 * nt * (tt - 100)) * 1e-6 - 0.06
        hi = (ns * (ts - 35) + 0.97 * nt * (tt - 60)) * 1e-6 - 0.03
        rs = sc['rest']
        rts = rs['timings_not_a_result']['separated']['ns_per_pair']
        rtt = rs['timings_not_a_result']['touching']['ns_per_pair']
        rlo = (rs['slow_separated'] * (rts - 60) + 0.80 * rs['slow_touching'] * (rtt - 100)) * 1e-6 - 0.06
        rhi = (rs['slow_separated'] * (rts - 35) + 0.97 * rs['slow_touching'] * (rtt - 60)) * 1e-6 - 0.03
        per.append(dict(ts=ts, tt=tt, ns=ns, nt=nt, lo=lo, hi=hi, stream=j['timings_not_a_result']['stream']['ns_per_pair'],
                        rts=rts, rtt=rtt, rlo=rlo, rhi=rhi, rain=sc['rain']['timings_not_a_result']['fast']['ns_per_pair'],
                        printed=d['printed_band'], pass_=d['pass'], sep_axis_hits=j['sep_axis_hits']))
        if abs(round(lo, 3) - d['printed_band']['jolt'][0]) > 0.0015 or abs(round(hi, 3) - d['printed_band']['jolt'][1]) > 0.0015:
            say('  BAND MISMATCH vs printed', lo, hi, d['printed_band'])
    for k in ('ts', 'tt', 'stream', 'lo', 'hi', 'rts', 'rtt', 'rlo', 'rhi', 'rain'):
        say(f'  {k:6s} {cstr(cell([p[k] for p in per]), 4)}')
    say('  counts N_sep/N_touch', sorted({(p['ns'], p['nt'], p['sep_axis_hits']) for p in per}))
    lo_c = cell([p['lo'] for p in per])
    hi_c = cell([p['hi'] for p in per])
    ts_c = cell([p['ts'] for p in per])
    tt_c = cell([p['tt'] for p in per])
    ns0, nt0 = per[0]['ns'], per[0]['nt']
    lo_med = (ns0 * (ts_c['med'] - 60) + 0.80 * nt0 * (tt_c['med'] - 100)) * 1e-6 - 0.06
    hi_med = (ns0 * (ts_c['med'] - 35) + 0.97 * nt0 * (tt_c['med'] - 60)) * 1e-6 - 0.03
    say(f'  band from median t: [{lo_med:.4f}, {hi_med:.4f}]')
    vlo = vs_bar(lo_c, 1.0)
    say(f'  low vs 1.0: {vlo}')
    verdict_a = 'FIRES' if hi_c['med'] < 1.0 else ('AMBIGUOUS' if lo_c['med'] < 1.0 else 'CLEAR')
    say(f'  (a) verdict on the median band [{lo_c["med"]:.4f}, {hi_c["med"]:.4f}]: {verdict_a}; per-process lows {[round(p["lo"], 4) for p in per]}')
    # L9b-only split of the band: touching term alone, and the sep term
    say(f'  touching term alone low/high: {0.80 * nt0 * (tt_c["med"] - 100) * 1e-6:.4f} / {0.97 * nt0 * (tt_c["med"] - 60) * 1e-6:.4f}; sep term low/high {ns0 * (ts_c["med"] - 60) * 1e-6:.4f} / {ns0 * (ts_c["med"] - 35) * 1e-6:.4f}')
    R['P2/classes'] = {'per': per, 'lo': lo_c, 'hi': hi_c, 'ts': ts_c, 'tt': tt_c, 'verdict': verdict_a,
                       'band_from_median_t': [lo_med, hi_med]}

    say('\n## P2 (b) same-binary reuse off/on (b4db)')
    h = 0.9897
    pred_lo = h * nt0 * (tt_c['med'] - 100) * 1e-6
    pred_hi = h * nt0 * (tt_c['med'] - 60) * 1e-6
    bar = 0.6 * pred_lo
    say(f'  L9b-only prediction h*N_touch*(t_touch - t_hit): [{pred_lo:.4f}, {pred_hi:.4f}] ms; bar 0.6*low = {bar:.4f} ms')
    # sensitivity of the bar to t_touch's spread
    say(f'  bar at t_touch min/max: {0.6 * h * nt0 * (tt_c["min"] - 100) * 1e-6:.4f} / {0.6 * h * nt0 * (tt_c["max"] - 100) * 1e-6:.4f}')
    R['P2/pred'] = {'lo': pred_lo, 'hi': pred_hi, 'bar': bar}

    def reuse_pair(off_row, on_row, Ws, wins):
        res = {}
        for W in Ws:
            for wl, fn in wins:
                a = C(off_row, 'b4db', W, fn)
                bcell = C(on_row, 'b4db', W, fn)
                if a is None or bcell is None:
                    continue
                c = cmp(a, bcell)
                say(f'  {off_row} vs {on_row} @W{W} {wl}: off {cstr(a, 4)} | on {cstr(bcell, 4)}')
                say(f'      on/off {kstr(c)}; dT(off-on) = {-c["delta"]:+.4f} ms; worst pairing (min off - max on) {a["min"] - bcell["max"]:+.4f}')
                res[(W, wl)] = {'off': a, 'on': bcell, 'cmp': c}
        return res

    JW = [('[0,500)', T), ('[100,500)', Tmw), ('[0,100)', Tw0)]
    r_ja = reuse_pair('L9-JA-off', 'L9-JA-on', (1, 8), JW)
    r_jaa = reuse_pair('L9-JA-a-off', 'L9-JA-a-on', (1,), JW)
    R['P2/JA'] = {str(k): v for k, v in r_ja.items()}
    R['P2/JAa'] = {str(k): v for k, v in r_jaa.items()}
    say('  armed twins @W1 spans [100,500), off vs on:')
    npd = {}
    for nm, col in SPANS[:4] + [('sys_sum', 'sys_sum_ns'), ('g', 'g_ns'), ('solve_build', 'phys_solve_build_ns'), ('warm_apply', 'phys_warm_apply_ns'), ('store', 'phys_store_ns'), ('manifolds', 'manifolds')]:
        for st, stn in ((0, 'A-mean'), (1, 'B-median')):
            a = C('L9-JA-a-off', 'b4db', 1, lambda d, col=col, st=st: d['mw'][col][st] / (1e6 if col != 'manifolds' else 1))
            bcell = C('L9-JA-a-on', 'b4db', 1, lambda d, col=col, st=st: d['mw'][col][st] / (1e6 if col != 'manifolds' else 1))
            c = cmp(a, bcell)
            say(f'    {nm:12s} {stn}: off {a["med"]:.4f} on {bcell["med"]:.4f} d(off-on) {a["med"] - bcell["med"]:+.4f}; {kstr(c)}')
            npd[(nm, stn)] = {'off': a, 'on': bcell, 'cmp': c}
    R['P2/armed_spans'] = {str(k): v for k, v in npd.items()}
    # reuse counters on the on-rows
    for row in ('L9-JA-a-on', 'L9-JA-on', 'L9-JD-on', 'L9-R-on', 'L9-JA-on-mid'):
        pcs = [d['pair_classes'] for d in P if d['row'] == row and d.get('pair_classes')]
        if pcs:
            hs = sorted({round(p['h'], 6) for p in pcs})
            say(f'  pair_classes {row}: h over {pcs[0]["window"]} = {hs}; reused/full_contacts {pcs[0]["reused"]}/{pcs[0]["full_contacts"]}')
    for row in ('L9-JA-a-on', 'L9-JA-a-off'):
        d0 = [d for d in P if d['row'] == row][0]
        for k in ('phys_np_reused', 'phys_np_sep_hits', 'phys_np_full', 'manifolds', 'pairs'):
            if k in d0['mw']:
                say(f'  {row} [100,500) mean {k} = {d0["mw"][k][0]:.2f}')
    # h over [100,500) from the per-step counters
    for row in ('L9-JA-a-on',):
        for d in [d for d in P if d['row'] == row][:1]:
            pass

    say('\n## P2 extra G-TW rows (P5d J-D, P5e R, P5f J-A mid W)')
    r_jd = reuse_pair('L9-JD-off', 'L9-JD-on', (1, 8), JW)
    r_r = reuse_pair('L9-R-off', 'L9-R-on', (1, 8), [('[600,1100)', T)])
    r_jam = reuse_pair('L9-JA-off-mid', 'L9-JA-on-mid', (2, 4, 16), JW)
    R['P5d'] = {str(k): v for k, v in r_jd.items()}
    R['P5e'] = {str(k): v for k, v in r_r.items()}
    R['P5f'] = {str(k): v for k, v in r_jam.items()}
    for row in ('L9-R-off', 'L9-R-on'):
        for W in (1, 8):
            mf = C(row, 'b4db', W, lambda d: d['win']['manifolds'][0])
            say(f'  {row}@W{W} manifolds over [600,1100): {f(mf["med"], 2)}')

    # ================================================================ P3
    say('\n## P3 C3b (par 6dd1f916 RowWalk vs tip 983480a9 LeafList)')
    p3 = {}
    for W in (1, 8):
        tq = {b: C('C3b-TA-armed', b, W, lambda d: d['mw']['phys_bp_query_ns'][1] / 1e6) for b in ('par', 'tip')}
        tqa = {b: C('C3b-TA-armed', b, W, lambda d: d['mw']['phys_bp_query_ns'][0] / 1e6) for b in ('par', 'tip')}
        tsp = {b: C('C3b-TA-armed', b, W, lambda d: d['mw']['bp4_sum_of_medians'] / 1e6) for b in ('par', 'tip')}
        bpsys = {b: C('C3b-TA-armed', b, W, lambda d: d['mw']['sys_physics_broadphase_ns'][1] / 1e6) for b in ('par', 'tip')}
        q = {b: C('C3b-TA-armed', b, W, lambda d: d['mw']['phys_bp_queried'][0]) for b in ('par', 'tip')}
        Ta = {b: C('C3b-TA-armed', b, W, T) for b in ('par', 'tip')}
        Td = {b: C('C3b-TD-tree', b, W, T) for b in ('par', 'tip')}
        for b in ('par', 'tip'):
            say(f'  W{W} {b}: t_q(B) {cstr(tq[b], 4)} | t_q(A mean) {tqa[b]["med"]:.4f} | c_q {tq[b]["med"] / q[b]["med"] * 1e6:.1f} ns (queried {q[b]["med"]:.0f}) | tree span {cstr(tsp[b], 4)} | bp sys(B) {bpsys[b]["med"]:.4f}')
            say(f'        T armed {cstr(Ta[b], 4)} | T-D tree {cstr(Td[b], 4)}')
        for nm, dd in (('t_q', tq), ('tree span', tsp), ('bp sys', bpsys), ('T armed', Ta), ('T-D', Td)):
            say(f'    tip vs par {nm}@W{W}: {kstr(cmp(dd["par"], dd["tip"]))}')
        v186 = vs_bar(tq['tip'], 0.186)
        v21 = vs_bar(tq['tip'], 0.21)
        v235 = vs_bar(tq['tip'], 0.235)
        lim = 0.36 if W == 1 else 0.35
        vsp = vs_bar(tsp['tip'], lim)
        say(f'    tip t_q vs 0.186 {v186}; vs 0.21 {v21}; vs 0.235 {v235}; tip span vs {lim} {vsp}; par span vs {lim} {vs_bar(tsp["par"], lim)}')
        p3[W] = {'tq': tq, 'tq_mean': tqa, 'span': tsp, 'bpsys': bpsys, 'queried': q, 'Ta': Ta, 'Td': Td,
                 'cmp_tq': cmp(tq['par'], tq['tip']), 'cmp_span': cmp(tsp['par'], tsp['tip']),
                 'cmp_Ta': cmp(Ta['par'], Ta['tip']), 'cmp_Td': cmp(Td['par'], Td['tip'])}
    # per-process ratio (paired by pass/round) as a sensitivity
    R['P3'] = {str(k): v for k, v in p3.items()}

    # ================================================================ P5c criterion
    say('\n## P5c C3b G4 criterion (bpb, same binary)')
    cr = [d for d in P if d['row'] == 'C3b-G4']
    names = sorted({k for d in cr for k in d['crit']})
    crc = {}
    for nm in names:
        crc[nm] = cell([d['crit'][nm] for d in cr if nm in d['crit']])
        say(f'  {nm:40s} {cstr(crc[nm], 4)}')
    ratios = {}
    for fam in ('scene/{}/j100', 'scene/{}/1240', 'uniform/{}/1000', 'disparity/{}/1000'):
        a = 'bp_g4_' + fam.format('tree')
        b_ = 'bp_g4_' + fam.format('tree_rowwalk')
        rc = cell([d['crit'][a] / d['crit'][b_] for d in cr])
        c = cmp(crc[b_], crc[a])
        ratios[fam] = {'ratio_cell': rc, 'cmp': c}
        say(f'  LeafList/RowWalk {fam}: per-process ratio {cstr(rc, 4)}; cell ratio {kstr(c)}')
        say(f'      vs 0.55 {vs_bar(rc, 0.55)}; vs 1.0 {vs_bar(rc, 1.0)}; vs 0.30 {vs_bar(rc, 0.30)}; vs 0.60 {vs_bar(rc, 0.60)}')
    say(f'  rowwalk j100 bridge vs window 4 0.4430 ms: {crc["bp_g4_scene/tree_rowwalk/j100"]["med"]:.4f} ({(crc["bp_g4_scene/tree_rowwalk/j100"]["med"] / 0.4430 - 1) * 100:+.2f} %), own s {crc["bp_g4_scene/tree_rowwalk/j100"]["s"]:.2f} %')
    say(f'  LeafList j100 vs design prediction 0.213-0.216 [F1+F2 0.189-0.195]: {crc["bp_g4_scene/tree/j100"]["med"]:.4f}; rule 1 reading j100/0.30 = {crc["bp_g4_scene/tree/j100"]["med"] / 0.30:.3f}')
    R['P5c'] = {'cells': crc, 'ratios': ratios}

    # ================================================================ P4 G9
    say('\n## P4 L11 G9 remainder (g9p 0ca312bd vs g9t cbd86a65)')
    g9 = {}
    fails = []
    g9rows = [('G9-JA', (1, 8)), ('G9-R', (1, 8)), ('G9-RS', (1, 8)), ('G9-S16', (1,)), ('G9-JAs-mid', (2, 4, 16)),
              ('G9-JAs-a', (1,)), ('G9-JC', (1,))]
    for row, Ws in g9rows:
        for W in Ws:
            a = C(row, 'g9p', W, T)
            b = C(row, 'g9t', W, T)
            c = cmp(a, b)
            say(f'  {row}@W{W}: parent {cstr(a, 4)} | tip {cstr(b, 4)}')
            say(f'      tip/parent {kstr(c)}')
            slower_both = c['eff'] > 0 and c['claimed']
            slower_se = c['eff'] > 0 and c['cl']['s']
            if slower_both or slower_se:
                fails.append((row, W, c['eff'], slower_both, slower_se))
            g9[f'{row}@{W}'] = {'parent': a, 'tip': b, 'cmp': c}
            if spec_armed(rowspec, row) and row != 'G9-JC':
                wn = 'mw' if rowspec[row]['steps'] == 500 else 'win'
                for nm, col in (('solve_build', 'phys_solve_build_ns'), ('warm_apply', 'phys_warm_apply_ns'), ('store', 'phys_store_ns'),
                                ('wide', 'phys_color_wide_ns'), ('narrow', 'phys_color_narrow_ns'), ('solve_sys', 'sys_physics_solve_colored_ns'),
                                ('bp', 'sys_physics_broadphase_ns'), ('np', 'sys_physics_narrowphase_ns')):
                    for st, stn in ((1, 'B'), (0, 'A')):
                        ca = C(row, 'g9p', W, lambda d, col=col, st=st, wn=wn: d[wn][col][st] / 1e6)
                        cb = C(row, 'g9t', W, lambda d, col=col, st=st, wn=wn: d[wn][col][st] / 1e6)
                        if ca is None or cb is None or ca['med'] == 0:
                            continue
                        cc = cmp(ca, cb)
                        say(f'      {nm:11s} {stn}: {ca["med"]:.4f} -> {cb["med"]:.4f} d {cc["delta"]:+.4f}; {kstr(cc)}')
                        g9[f'{row}@{W}/{nm}/{stn}'] = {'parent': ca, 'tip': cb, 'cmp': cc}
                        if stn == 'B' and nm == 'wide' and cc['eff'] > 0 and cc['cl']['s']:
                            fails.append((row, W, 'wide colours slower', cc['eff']))
    say('  claimed-slower list (tip vs parent; both-spreads or SE-only):', fails)
    R['P4'] = g9
    R['P4/fails'] = fails
    # canary
    say('  canary J-C:')
    can = {}
    for b in ('g9p', 'g9t'):
        pcs = procs_of('G9-JC', b, 1)
        rows_ = []
        for d in pcs:
            inj = d['canary_ns']
            meas = d['win']['sys_parity_canary_ns'][0]
            rows_.append((inj, meas, meas / inj - 1))
        say(f'    {b}: injected/measured/rel per process: {[(round(x[0] / 1e6, 4), round(x[1] / 1e6, 4), round(x[2] * 100, 3)) for x in rows_]}')
        inj_c = cell([x[0] / 1e6 for x in rows_])
        meas_c = cell([x[1] / 1e6 for x in rows_])
        jc = C('G9-JC', b, 1, T)
        ref = C('G9-JAs-a', b, 1, T)
        c = cmp(ref, jc)
        rise = jc['med'] - ref['med']
        say(f'    {b}: span {cstr(meas_c, 4)} vs injected {cstr(inj_c, 4)} (span/inj {meas_c["med"] / inj_c["med"]:.4f}); wall J-C {jc["med"]:.4f} vs J-As-a {ref["med"]:.4f}: rise {rise:+.4f} ms = {rise / inj_c["med"] * 100:.0f} % of injected; {kstr(c)}')
        # step rise minus the canary span (does the rest of the step stay put?)
        rest = C('G9-JC', b, 1, lambda d: (d['win']['wall_ns'][0] - d['win']['sys_parity_canary_ns'][0]) / 1e6)
        say(f'      J-C wall minus canary span {cstr(rest, 4)} vs J-As-a {cstr(ref, 4)}: {kstr(cmp(ref, rest))}')
        can[b] = {'inj': inj_c, 'meas': meas_c, 'jc': jc, 'ref': ref, 'cmp': c, 'rise': rise, 'rest': rest}
    R['P4/canary'] = can

    # ================================================================ P5a bridge 2 in-window
    say('\n## P5a bridge 2 (w4bt f8873aae vs g9p 0ca312bd, J-As)')
    b2 = {}
    for W in (1, 8):
        a = C('B2-JAs', 'w4bt', W, T)
        b = C('B2-JAs', 'g9p', W, T)
        c = cmp(a, b)
        say(f'  W{W}: w4bt {cstr(a, 4)} | g9p {cstr(b, 4)}; g9p/w4bt {kstr(c)}')
        for ps in (0, 1):
            aa = cell([T(d) for d in procs_of('B2-JAs', 'w4bt', W) if d['pass'] == ps])
            bb = cell([T(d) for d in procs_of('B2-JAs', 'g9p', W) if d['pass'] == ps])
            say(f'      pass {ps}: w4bt {aa["med"]:.4f} g9p {bb["med"]:.4f} ratio {bb["med"] / aa["med"]:.4f}')
        b2[W] = {'w4bt': a, 'g9p': b, 'cmp': c}
    R['P5a'] = {str(k): v for k, v in b2.items()}

    # ================================================================ P5b L10 spans
    say('\n## P5b L10 armed Off-prime spans at W8 and R-S')
    for row, Ws in (('L10-Offp8', (8,)), ('L10-Son8', (8,)), ('L10-RS-Offp', (1, 8)), ('L10-RS', (1, 8))):
        for W in Ws:
            c = C(row, 'b4db', W, T)
            ct = C(row, 'b4db', W, Ttail)
            say(f'  {row}@W{W}: wall win {cstr(c, 4)}; tail {cstr(ct, 4)}; ffs {sorted({d["ff_summary"] for d in procs_of(row, "b4db", W)})}')
            R[f'P5b/{row}@{W}'] = {'win': c, 'tail': ct}
    span_table([('L10-Offp8', 'b4db'), ('L10-Son8', 'b4db')], 'win', 'P5bspans', (8,))
    span_table([('L10-RS-Offp', 'b4db'), ('L10-RS', 'b4db')], 'win', 'P5bspans', (1, 8))
    span_table([('L10-RS-Offp', 'b4db'), ('L10-RS', 'b4db')], 'tail', 'P5bspans', (1, 8))
    for W in (1, 8):
        o = R[f'P5b/L10-RS-Offp@{W}']['win']
        say(f'  R-S gate at W{W}: 0.6 x (Off-prime_RS {o["med"]:.4f} - 0.25) = {0.6 * (o["med"] - 0.25):.4f} ms; Off-prime_RS - Sets floor [0.12,0.25] = [{o["med"] - 0.25:.4f}, {o["med"] - 0.12:.4f}]')
        say(f'  R-S vs R-S Off-prime @W{W}: {kstr(cmp(R[f"P5b/L10-RS@{W}"]["win"], o))}')
    o8 = R['P5b/L10-Offp8@8']
    say(f'  Off-prime(8) {o8["win"]["med"]:.4f} vs design 0.42-0.93; J-Son(8) {R["P5b/L10-Son8@8"]["win"]["med"]:.4f}')

    # ================================================================ P5g G5
    say('\n## P5g C3b G5 on the tip, same binary')
    g5 = {}
    for W in (1, 8):
        bpt = C('G5-TA-tree-a', 'tip', W, lambda d: d['mw']['sys_physics_broadphase_ns'][1] / 1e6)
        bpa = C('G5-TA-ap-a', 'tip', W, lambda d: d['mw']['sys_physics_broadphase_ns'][1] / 1e6)
        bpt_a = C('G5-TA-tree-a', 'tip', W, lambda d: d['mw']['sys_physics_broadphase_ns'][0] / 1e6)
        bpa_a = C('G5-TA-ap-a', 'tip', W, lambda d: d['mw']['sys_physics_broadphase_ns'][0] / 1e6)
        tsp = C('G5-TA-tree-a', 'tip', W, lambda d: d['mw']['bp4_sum_of_medians'] / 1e6)
        tq = C('G5-TA-tree-a', 'tip', W, lambda d: d['mw']['phys_bp_query_ns'][1] / 1e6)
        Tt = C('G5-TA-tree-a', 'tip', W, T)
        Tap = C('G5-TA-ap-a', 'tip', W, T)
        Tdt = C('G5-TD-tree', 'tip', W, T)
        Tda = C('G5-TD-ap', 'tip', W, T)
        dbp = bpa['med'] - bpt['med']
        dbp_a = bpa_a['med'] - bpt_a['med']
        say(f'  W{W}: bp span (B) tree {cstr(bpt, 4)} | allpairs {cstr(bpa, 4)} | dbp(B) {dbp:.4f} dbp(A) {dbp_a:.4f}; 2hypot(SE) {2 * math.hypot(bpt["se_abs"], bpa["se_abs"]):.4f}; worst {bpa["min"] - bpt["max"]:.4f}')
        say(f'      tree span {cstr(tsp, 4)} t_q {cstr(tq, 4)}')
        say(f'      armed T tree {cstr(Tt, 4)} ap {cstr(Tap, 4)}: {kstr(cmp(Tap, Tt))}')
        say(f'      T-D tree {cstr(Tdt, 4)} ap {cstr(Tda, 4)}: {kstr(cmp(Tda, Tdt))}')
        # internal reproducibility: G5-TA-tree-a vs C3b-TA-armed tip (same binary, same row, two blocks)
        c3 = p3[W]
        say(f'      same cell two blocks: C3b-TA-armed tip T {c3["Ta"]["tip"]["med"]:.4f} vs G5-TA-tree-a T {Tt["med"]:.4f}: {kstr(cmp(c3["Ta"]["tip"], Tt))}')
        say(f'      same cell two blocks t_q: {kstr(cmp(c3["tq"]["tip"], tq))}; span {kstr(cmp(c3["span"]["tip"], tsp))}')
        say(f'      T-D tree two blocks (C3b-TD-tree tip vs G5-TD-tree): {kstr(cmp(c3["Td"]["tip"], Tdt))}')
        g5[W] = {'bp_tree': bpt, 'bp_ap': bpa, 'dbp_B': dbp, 'dbp_A': dbp_a, 'span': tsp, 'tq': tq, 'Tt': Tt, 'Tap': Tap,
                 'Tdt': Tdt, 'Tda': Tda, 'cmp_TD': cmp(Tda, Tdt), 'cmp_TA': cmp(Tap, Tt)}
    R['P5g'] = {str(k): v for k, v in g5.items()}

    # ---- the tip's armed tree row was taken in two blocks: pooled reading and the D6 arithmetic
    say('\n## P3 sensitivity: the tip armed tree row in two blocks (P3 and P5g), pooled K=12')
    pooled = {}
    for W in (1, 8):
        both = procs_of('C3b-TA-armed', 'tip', W) + procs_of('G5-TA-tree-a', 'tip', W)
        tq12 = cell([d['mw']['phys_bp_query_ns'][1] / 1e6 for d in both])
        tqA12 = cell([d['mw']['phys_bp_query_ns'][0] / 1e6 for d in both])
        sp12 = cell([d['mw']['bp4_sum_of_medians'] / 1e6 for d in both])
        say(f'  W{W}: t_q(B) P3 {p3[W]["tq"]["tip"]["med"]:.4f} | P5g {g5[W]["tq"]["med"]:.4f} | pooled {cstr(tq12, 4)} (c_q {tq12["med"] / 1240 * 1e6:.1f} ns)')
        say(f'       t_q(A mean) pooled {cstr(tqA12, 4)}; span pooled {cstr(sp12, 4)}')
        for bar in (0.186, 0.21, 0.235):
            say(f'       pooled t_q vs {bar}: {vs_bar(tq12, bar)}; P5g block vs {bar}: {vs_bar(g5[W]["tq"], bar)}; P3 A-reading vs {bar}: {vs_bar(p3[W]["tq_mean"]["tip"], bar)}')
        pooled[W] = {'tq': tq12, 'tqA': tqA12, 'span': sp12}
    R['P3/pooled'] = {str(k): v for k, v in pooled.items()}
    k6 = 1 - 1 / (8 * 0.681)
    for lab, t8, tq in (('P3 block', p3[8]['Td']['tip']['med'], p3[8]['tq']['tip']['med']),
                        ('P5g block', g5[8]['Tdt']['med'], g5[8]['tq']['med'])):
        trig = (0.05 * t8 + 0.00654) / k6
        save = tq * k6 - 0.00654
        say(f'  D6 at {lab}: T(8) {t8:.4f}, trigger t_q >= {trig:.4f}; measured t_q(8) {tq:.4f} -> C2 saving {save:.4f} ms = {save / t8 * 100:.2f} % of T(8); '
            f'C2 own gate needs realized >= {max(0.6 * save, 0.05 * t8):.4f} ms = {max(0.6 * save, 0.05 * t8) / save * 100:.0f} % of the prediction')

    # ---- Jolt v5.6.0 (window 3, never re-run) against the tip's tree default row
    say('\n## The tip default row with the tree against Jolt v5.6.0 (window 3 cells, recomputed from its runs.jsonl mean_ms)')
    w3 = load_jsonl('D:/wt/joltab/docs/measurements/2026-09-21-physics-window3/raw/runs.jsonl')
    w3p = []
    for r in w3:
        if r.get('row') != 'JOLT56-T':
            continue
        r = dict(r)
        r['timed'] = r['attempt'] in ('original', 'rerun')
        r['valid'] = str(r['valid']) == 'True'
        r['exit'] = int(r['exit'])
        r['W'] = int(r['W'])
        for side in ('receipt_before', 'receipt_after'):
            if isinstance(r.get(side), dict) and 'cpu_avg' in r[side]:
                r[side]['cpu_avg'] = float(r[side]['cpu_avg'])
        w3p.append(r)
    ch3, dr3, _ = select(w3p, lambda r: (r['pass'], r['seq'], r['row'], r['W']))
    say(f'  window 3 JOLT56-T: chosen {len(ch3)}, dropped {len(dr3)}')
    jolt = {}
    for W in (1, 8):
        jolt[W] = cell([float(r['mean_ms']) for r in ch3 if r['W'] == W])
        say(f'  Jolt v5.6.0 W{W}: {cstr(jolt[W], 4)}')
    for W in (1, 8):
        for lab, c in (('P3 block C3b-TD-tree tip', p3[W]['Td']['tip']), ('P5g block G5-TD-tree tip', g5[W]['Tdt']),
                       ('P3 block C3b-TD-tree par', p3[W]['Td']['par']), ('P5g block G5-TD-ap tip', g5[W]['Tda'])):
            say(f'  W{W} {lab} {c["med"]:.4f} vs Jolt: {kstr(cmp(jolt[W], c))}')
    R['jolt56'] = jolt

    # ================================================================ within-window cross-binary readings
    say('\n## Cross-binary readings inside the window')
    for W in (1, 8):
        a = C('G9-JA', 'g9t', W, T)
        b = C('L9-JA-off', 'b4db', W, T)
        say(f'  J-A W{W}: g9t (cbd86a65, pre-L9) {a["med"]:.4f} vs b4db reuse off (L9a+carry) {b["med"]:.4f}: {kstr(cmp(a, b))}')
        bon = C('L9-JA-on', 'b4db', W, T)
        say(f'  J-A W{W}: g9t vs b4db reuse on (L9a+L9b): {kstr(cmp(a, bon))}')
    npa = C('G9-JAs-a', 'g9t', 1, lambda d: d['mw']['sys_physics_narrowphase_ns'][0] / 1e6)
    npo = C('L9-JA-a-off', 'b4db', 1, lambda d: d['mw']['sys_physics_narrowphase_ns'][0] / 1e6)
    npn = C('L9-JA-a-on', 'b4db', 1, lambda d: d['mw']['sys_physics_narrowphase_ns'][0] / 1e6)
    say(f'  np span [100,500) mean, W1: g9t J-As-a (pre-L9) {cstr(npa, 4)}; b4db J-A-a off {cstr(npo, 4)}; on {cstr(npn, 4)}')
    say(f'     L9a+carry (pre-L9 -> off) {npa["med"] - npo["med"]:+.4f}; L9b (off -> on) {npo["med"] - npn["med"]:+.4f}; total {npa["med"] - npn["med"]:+.4f}')
    R['xbin/np'] = {'g9t_JAs_a': npa, 'off': npo, 'on': npn}
    for W in (1, 8):
        a = C('C3b-TD-tree', 'tip', W, T)
        b = C('G5-TD-ap', 'tip', W, T)
        say(f'  T-D tree (C3b block) vs T-D allpairs (G5 block) tip W{W}: {kstr(cmp(b, a))}')

    # ================================================================ bridges to windows 5 and 4b
    say('\n## Bridges')
    w5 = load_old('win5', lambda r: not (r['block'] == 'headline' and r['pass'] == 0 and r.get('pass_attempt') != 3))
    w4b = load_old('win4b', lambda r: True)

    def oc(P_, row, role, W, fn=lambda d: d['T']):
        return cell([fn(d) for d in P_ if d['row'] == row and d['role'] == role and d['W'] == W])

    br = {}

    def bridge(lab, A, B, note=''):
        c = cmp(A, B)
        hold = not c['claimed']
        se_only = (not c['claimed']) and c['cl']['s']
        say(f'  {lab}: earlier {cstr(A, 4)} | window 6 {cstr(B, 4)}; {kstr(c)} -> {"HOLDS" if hold else "FAILS"}{" (SE alone claims a shift)" if se_only else ""} {note}')
        br[lab] = {'earlier': A, 'w6': B, 'cmp': c, 'holds': hold, 'se_only': se_only}

    for W in (1, 8):
        bridge(f'w5 HL-D-tree tip(cbd86a65)@W{W} -> w6 C3b-TD-tree par(6dd1f916)', oc(w5, 'HL-D-tree', 'tip', W), C('C3b-TD-tree', 'par', W, T), '[same RowWalk kernel; plugin/ECS edits between]')
        bridge(f'w5 HL-D-allpairs tip@W{W} -> w6 G5-TD-ap tip(983480a9)', oc(w5, 'HL-D-allpairs', 'tip', W), C('G5-TD-ap', 'tip', W, T), '[AllPairs path; plugin/ECS edits + tree-only C3b between]')
        bridge(f'w5 J-As parent(0ca312bd)@W{W} -> w6 B2-JAs g9p (same binary)', oc(w5, 'J-As', 'parent', W), C('B2-JAs', 'g9p', W, T))
        bridge(f'w4b J-As tip(f8873aae)@W{W} -> w6 B2-JAs w4bt (same binary)', oc(w4b, 'J-As', 'tip', W), C('B2-JAs', 'w4bt', W, T))
    for role, b in (('parent', 'g9p'), ('tip', 'g9t')):
        bridge(f'w5 J-As-a {role}@W1 -> w6 G9-JAs-a {b} (same binary)', oc(w5, 'J-As-a', role, 1), C('G9-JAs-a', b, 1, T))
        bridge(f'w5 J-As-a {role}@W1 warm_apply(B) -> w6 {b}', oc(w5, 'J-As-a', role, 1, lambda d: d['mw']['phys_warm_apply_ns'][1] / 1e6),
               C('G9-JAs-a', b, 1, lambda d: d['mw']['phys_warm_apply_ns'][1] / 1e6))
    w5_wa = {role: oc(w5, 'J-As-a', role, 1, lambda d: d['mw']['phys_warm_apply_ns'][1] / 1e6) for role in ('parent', 'tip')}
    w6_wa = {b: C('G9-JAs-a', b, 1, lambda d: d['mw']['phys_warm_apply_ns'][1] / 1e6) for b in ('g9p', 'g9t')}
    say(f'  warm_apply gate re-read in window 6: {w6_wa["g9p"]["med"]:.4f} -> {w6_wa["g9t"]["med"]:.4f} = {w6_wa["g9t"]["med"] - w6_wa["g9p"]["med"]:+.4f} ms (bar -0.19); {kstr(cmp(w6_wa["g9p"], w6_wa["g9t"]))}')
    # window 4b cells that window 6 re-takes on other binaries (readings, not bridges)
    for row4, row6, Ws in (('J-A', 'G9-JA', (1, 8)), ('R', 'G9-R', (1, 8)), ('R-S', 'G9-RS', (1, 8)), ('S16', 'G9-S16', (1,)), ('J-As', 'G9-JAs-mid', (2, 4, 16))):
        for W in Ws:
            bridge(f'w4b {row4} tip(f8873aae)@W{W} -> w6 {row6} g9p(0ca312bd) [cross-binary: the line merge]', oc(w4b, row4, 'tip', W), C(row6, 'g9p', W, T))
    # window 4 (a46b8287, RowWalk) armed tree row -> window 6 par (6dd1f916, RowWalk): the 334 ns/row origin
    w4recs = load_jsonl(os.path.join(OLD, 'win4/raw/runs.jsonl'))
    w4p = []
    for r in w4recs:
        if r.get('row') != 'T-A-tree-armed':
            continue
        r = dict(r)
        r.setdefault('timed', r.get('attempt') in ('original', 'rerun'))
        w4p.append(r)
    ch4, dr4, _ = select(w4p, lambda r: (r.get('block'), r['pass'], r['round'], r['row'], r.get('role'), r['W']))
    say(f'  [win4] T-A-tree-armed: chosen {len(ch4)}, dropped {len(dr4)}')
    for W in (1, 8):
        xs, spans = [], []
        for r in ch4:
            if int(r['W']) != W:
                continue
            steps, cols = read_csv(r.get('csv') or os.path.join(r['cwd'], 'run.csv'))
            ws = win_stats(steps, cols, 100, 500)
            xs.append(ws['phys_bp_query_ns'][1] / 1e6)
            spans.append(ws['bp4_sum_of_medians'] / 1e6)
        c4 = cell(xs)
        bridge(f'w4 T-A-tree-armed t_q(B) (a46b8287, RowWalk)@W{W} -> w6 C3b-TA-armed par (6dd1f916, RowWalk)', c4, p3[W]['tq']['par'], '[cross-binary]')
        bridge(f'w4 T-A-tree-armed tree span(B)@W{W} -> w6 par', cell(spans), p3[W]['span']['par'], '[cross-binary]')
    w4j = {'K': 6, 'med': 0.4430, 'min': 0.4424, 'max': 0.4440, 'r': 0.36, 'i': float('nan'), 's': 0.13, 'se_abs': 0.4430 * 0.0013}
    bridge('w4 G4 bp_g4_scene/tree/j100 (a46b8287 RowWalk; cited from its analysis.md:134) -> w6 tree_rowwalk/j100 (983480a9)', w4j, crc['bp_g4_scene/tree_rowwalk/j100'], '[cross-binary, cited cell]')
    R['bridges'] = br

    os.makedirs(OUTDIR, exist_ok=True)
    with open(os.path.join(OUTDIR, 'tables.txt'), 'w', encoding='utf-8') as fh:
        fh.write('\n'.join(OUT) + '\n')
    with open(os.path.join(OUTDIR, 'reduction.json'), 'w', encoding='utf-8') as fh:
        json.dump(R, fh, indent=1, default=lambda o: str(o))
    print('\n'.join(OUT))


def spec_armed(rowspec, row):
    return bool(rowspec[row].get('armed'))


def load_old(win, keep):
    """Reduce an earlier window's protocol set with the same slot rule; returns per-process dicts."""
    recs = load_jsonl(os.path.join(OLD, win, 'raw/runs.jsonl'))
    procs = [r for r in recs if 'row' in r and keep(r)]
    key = lambda r: (r['block'], r['pass'], r.get('pass_attempt'), r['round'], r['row'], r['role'], r['W'])
    chosen, dropped, slots = select(procs, key)
    say(f'  [{win}] protocol processes {len(procs)}, slots {len(slots)}, chosen {len(chosen)}, dropped {len(dropped)}')
    out = []
    for r in chosen:
        csvp = r.get('csv') or os.path.join(r['cwd'], 'run.csv')
        steps, cols = read_csv(csvp)
        a, b = r['summary']['window']
        d = {'row': r['row'], 'role': r['role'], 'W': r['W'], 'T': win_stats(steps, cols, a, b)['wall_ns'][0] / 1e6}
        if abs(d['T'] - r['mean_ms']) > 1e-9:
            say('  [old] mean mismatch', win, r['row'], d['T'], r['mean_ms'])
        if len(steps) == 500:
            d['mw'] = win_stats(steps, cols, 100, 500)
        out.append(d)
    return out


if __name__ == '__main__':
    main()
