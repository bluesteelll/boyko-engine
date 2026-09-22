"""Reduce the P0b window (raw/window/runs.jsonl) under the ruled protocol. Pure reading: launches no
measured process (it runs the driver copy's `reduce` for the structural checks only).

Per (row, W) cell: the MEDIAN over the K used processes of each process's window mean, with min-max,
range %, IQR % (inclusive quartiles) and SD % of the median. A comparison of two cells is CLAIMED only if
|effect| > 2 x the combined spread, combined = hypot(spread_A, spread_B) (the P0b diagnostic's form),
read three ways: min-max range (the ruling's reading), IQR, and the standard error of the median
(1.2533 SD / sqrt K per side; supplementary, the diagnostic's suggested gate).

Usage: python -B analyze_window.py [raw/window]
"""
import json
import math
import os
import re
import shutil
import statistics
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, sys.argv[1] if len(sys.argv) > 1 else os.path.join('raw', 'window'))
sys.path.insert(0, os.path.join(HERE, 'lib'))
import driver as D  # noqa: E402

GRID_LO, GRID_HI = 96, 192  # D:/wt/joltab/crates/boyko_physics/src/broadphase_policy.rs:79,88
NCPU = os.cpu_count()


# ── loading ──────────────────────────────────────────────────────────────────

def load():
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8')]
    warm = [r for r in recs if r.get('attempt') == 'warmup']
    attempts = [r for r in recs if r.get('attempt') != 'warmup']
    by_slot = {}
    for r in attempts:
        by_slot.setdefault((r['pass'], r['seq']), []).append(r)
    used, excluded = [], []
    for slot, rs in sorted(by_slot.items()):
        chosen = None
        for r in rs:  # the original first, then its re-run
            if 'aborted' in r or 'skipped' in r:
                continue
            if r.get('valid') and not r.get('contaminated'):
                chosen = r
                break
        if chosen is None:
            excluded.append(rs)
        else:
            used.append(chosen)
    return recs, warm, attempts, used, excluded


def quart(xs):
    if len(xs) < 2:
        return xs[0], xs[0]
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[0], q[2]


def stats(xs):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    med = statistics.median(xs)
    q1, q3 = quart(xs)
    sd = statistics.stdev(xs) if len(xs) >= 2 else 0.0
    return {'n': len(xs), 'median': med, 'min': xs[0], 'max': xs[-1], 'q1': q1, 'q3': q3,
            'range_pct': 100 * (xs[-1] - xs[0]) / med, 'iqr_pct': 100 * (q3 - q1) / med, 'sd_pct': 100 * sd / med,
            'se_med_pct': 1.2533 * 100 * sd / med / math.sqrt(len(xs))}


def compare(a, b):
    """Effect of B against A, and the three claim readings."""
    if not a or not b:
        return None
    eff = 100 * (b['median'] / a['median'] - 1)
    cr = math.hypot(a['range_pct'], b['range_pct'])
    ci = math.hypot(a['iqr_pct'], b['iqr_pct'])
    cs = math.hypot(a['se_med_pct'], b['se_med_pct'])
    return {'ratio': b['median'] / a['median'], 'effect_pct': eff, 'bar_range_pct': 2 * cr, 'bar_iqr_pct': 2 * ci,
            'bar_se_pct': 2 * cs, 'claim_range': abs(eff) > 2 * cr, 'claim_iqr': abs(eff) > 2 * ci,
            'claim_se': abs(eff) > 2 * cs}


def csv_cols(rec):
    if rec['engine'] == 'boyko':
        return D.load_csv(rec.get('csv'))
    c = D.load_csv(rec.get('per_frame'))
    if c is not None and 'Time (ms)' in c:
        c['wall_ns'] = [x * 1e6 if x is not None else None for x in c['Time (ms)']]
        c['awake'] = c.get('Active Bodies')
    return c


# ── Jolt -p stage shares ─────────────────────────────────────────────────────

def load_chart(path):
    t = open(path, encoding='utf-8', errors='replace').read()
    i = t.index('var cycles_per_second')
    body = t[i:t.index('</script>', i)]
    cps = int(re.search(r'var cycles_per_second = (\d+);', body).group(1))
    thr = body[body.index('var threads = ') + len('var threads = '):body.index('var aggregated = ')].strip().rstrip(';')
    agg = body[body.index('var aggregated = ') + len('var aggregated = '):].strip().rstrip(';')

    def js2json(s):
        s = re.sub(r'(^|[{,\n])\s*([a-z_]+):', r'\1"\2":', s)
        s = re.sub(r',\s*([}\]])', r'\1', s)
        return json.loads(s)
    return cps, js2json(thr), js2json(agg)


def union_len(iv):
    iv = sorted(iv)
    tot, cs, ce = 0, None, None
    for s, e in iv:
        if cs is None or s > ce:
            if cs is not None:
                tot += ce - cs
            cs, ce = s, e
        else:
            ce = max(ce, e)
    if cs is not None:
        tot += ce - cs
    return tot


def chart_shares(path):
    """One dumped frame: per job-level stage, summed cycles over threads (work) and the union of its
    intervals (wall coverage). Job level = the children of 'Executing Jobs' on a worker and of
    'Execute Jobs' on the main thread (where it helps inside WaitForJobs). Main-thread work outside
    WaitForJobs is 'main serial'. Scopes at job level that are job-system functions ('JPH::' names) are
    'job-system overhead'."""
    cps, threads, agg = load_chart(path)
    names = agg['name']
    upd = [i for i, n in enumerate(names) if 'PhysicsSystem::Update' in n]
    work, wall_iv = {}, {}
    update_cycles = None
    main_serial = 0
    nsamples = {t['thread_name']: len(t['start']) for t in threads}
    for t in threads:
        a, st, cy, dp = t['aggregator'], t['start'], t['cycles'], t['depth']
        n = len(a)
        # parent stack to find job-level scopes
        stack = []  # (depth, name, end)
        for k in range(n):
            nm = names[a[k]]
            while stack and stack[-1][0] >= dp[k]:
                stack.pop()
            parent = stack[-1][1] if stack else None
            stack.append((dp[k], nm, st[k] + cy[k]))
            if a[k] in upd and dp[k] == 0:
                update_cycles = cy[k]
            if t['thread_name'] == 'Main' and dp[k] == 1 and parent is not None and 'PhysicsSystem::Update' in parent \
                    and 'WaitForJobs' not in nm:
                main_serial += cy[k]
            if parent in ('Executing Jobs', 'Execute Jobs'):
                key = 'job-system overhead' if 'JPH::' in nm else nm
                work[key] = work.get(key, 0) + cy[k]
                wall_iv.setdefault(key, []).append((st[k], st[k] + cy[k]))
    total_work = sum(work.values()) + main_serial
    out = {'cps': cps, 'update_ms': 1000 * update_cycles / cps if update_cycles else None,
           'samples_per_thread_max': max(nsamples.values()) if nsamples else 0,
           'truncated': any(v >= 65536 for v in nsamples.values()),
           'work_share_pct': {k: 100 * v / total_work for k, v in work.items()},
           'wall_cover_pct': {k: 100 * union_len(v) / update_cycles for k, v in wall_iv.items()} if update_cycles else {},
           'work_ms': {k: 1000 * v / cps for k, v in work.items()}}
    out['work_share_pct']['main serial (outside WaitForJobs)'] = 100 * main_serial / total_work
    out['work_ms']['main serial (outside WaitForJobs)'] = 1000 * main_serial / cps
    out['total_work_ms'] = 1000 * total_work / cps
    return out


def jolt_p(used):
    res = {}
    for w in (1, 8):
        procs = []
        for r in [r for r in used if r['row'] == 'JOLT-P' and r['W'] == w]:
            frames = {}
            for f in sorted(os.listdir(r['cwd'])):
                m = re.match(r'profile_chart_.*_it(\d+)\.html$', f)
                if m:
                    frames[int(m.group(1))] = chart_shares(os.path.join(r['cwd'], f))
            procs.append({'g': r['g'], 'mean_ms_profiled': r['mean_ms'], 'frames': frames,
                          'too_many_samples_warning': b'Too many samples' in open(os.path.join(r['cwd'], 'stdout.txt'), 'rb').read()
                          + open(os.path.join(r['cwd'], 'stderr.txt'), 'rb').read()})
        if not procs:
            continue
        # per process: median over frames 100..400 (frame 0 is the first step), then median over processes
        keys = sorted({k for p in procs for it, fr in p['frames'].items() if it > 0 for k in fr['work_share_pct']})
        cell = {'K': len(procs), 'frames_used': sorted({it for p in procs for it in p['frames'] if it > 0}),
                'truncated_frames': sum(1 for p in procs for fr in p['frames'].values() if fr['truncated']),
                'warning_processes': sum(1 for p in procs if p['too_many_samples_warning']),
                'profiled_T_ms': stats([p['mean_ms_profiled'] for p in procs]),
                'update_ms': stats([statistics.median([fr['update_ms'] for it, fr in p['frames'].items() if it > 0])
                                    for p in procs]),
                'total_work_ms': stats([statistics.median([fr['total_work_ms'] for it, fr in p['frames'].items() if it > 0])
                                        for p in procs]),
                'frame0_update_ms': stats([p['frames'][0]['update_ms'] for p in procs if 0 in p['frames']]),
                'stages': {}}
        for k in keys:
            ws = [statistics.median([fr['work_share_pct'].get(k, 0.0) for it, fr in p['frames'].items() if it > 0])
                  for p in procs]
            wc = [statistics.median([fr['wall_cover_pct'].get(k, 0.0) for it, fr in p['frames'].items() if it > 0])
                  for p in procs]
            wm = [statistics.median([fr['work_ms'].get(k, 0.0) for it, fr in p['frames'].items() if it > 0])
                  for p in procs]
            cell['stages'][k] = {'work_share_pct': statistics.median(ws), 'work_share_min': min(ws),
                                 'work_share_max': max(ws), 'wall_cover_pct': statistics.median(wc),
                                 'work_ms': statistics.median(wm)}
        res[w] = cell
    return res


# ── broadphase ───────────────────────────────────────────────────────────────

def broadphase():
    bdir = os.path.join(RAW, 'broadphase')
    runp = os.path.join(bdir, 'broadphase_run.json')
    if not os.path.isfile(runp):
        return None
    run = json.load(open(runp, encoding='utf-8'))
    est = {}
    root = os.path.join(bdir, 'criterion')
    for dirpath, dirnames, filenames in os.walk(root):
        if os.path.basename(dirpath) == 'new' and 'estimates.json' in filenames:
            rel = os.path.relpath(os.path.dirname(dirpath), root).replace('\\', '/')
            e = json.load(open(os.path.join(dirpath, 'estimates.json'), encoding='utf-8'))
            bm = json.load(open(os.path.join(dirpath, 'benchmark.json'), encoding='utf-8')) \
                if os.path.isfile(os.path.join(dirpath, 'benchmark.json')) else {}
            est[rel] = {'median_ns': e['median']['point_estimate'], 'median_lo': e['median']['confidence_interval']['lower_bound'],
                        'median_hi': e['median']['confidence_interval']['upper_bound'],
                        'mean_ns': e['mean']['point_estimate'], 'full_id': bm.get('full_id')}
    out = {'run': {k: run.get(k) for k in ('exit', 'wall_s', 'start', 'end', 'contaminated', 'others_busy_pct',
                                           'others_top5', 'proc_cpu_s', 'aborted', 'args')},
           'receipt_before': (run.get('receipt_before') or {}).get('cpu_avg'),
           'receipt_after': (run.get('receipt_after') or {}).get('cpu_avg'),
           'perf_total_mean': statistics.mean([p['total'] for p in run.get('perf', []) if p.get('total')])
           if run.get('perf') else None,
           'estimates': est}
    # The crossover on the uniform-lattice group: log-log interpolation of t_grid / t_all_pairs over n.
    pts = []
    for n in (100, 1000, 10000):
        a, g = est.get(f'broadphase/all_pairs/{n}'), est.get(f'broadphase/grid/{n}')
        if a and g:
            pts.append((n, a['median_ns'], g['median_ns'], g['median_ns'] / a['median_ns']))
    out['points'] = pts
    cross = None
    for (n0, a0, g0, r0), (n1, a1, g1, r1) in zip(pts, pts[1:]):
        if (r0 - 1) * (r1 - 1) <= 0:
            x0, x1, y0, y1 = math.log(n0), math.log(n1), math.log(r0), math.log(r1)
            cross = {'n_star': math.exp(x0 + (0 - y0) * (x1 - x0) / (y1 - y0)), 'between': [n0, n1],
                     'method': 'log-log interpolation of t_grid/t_all_pairs between the two bracketing sizes'}
            break
    if cross is None and len(pts) >= 2:
        (n0, a0, g0, r0), (n1, a1, g1, r1) = pts[0], pts[1]
        x0, x1, y0, y1 = math.log(n0), math.log(n1), math.log(r0), math.log(r1)
        if y1 != y0:
            cross = {'n_star': math.exp(x0 + (0 - y0) * (x1 - x0) / (y1 - y0)), 'between': None,
                     'method': 'EXTRAPOLATED from the 100 and 1000 points (outside the measured range)'}
    out['crossover'] = cross
    out['grid_lo'], out['grid_hi'] = GRID_LO, GRID_HI
    return out


# ── witnesses ────────────────────────────────────────────────────────────────

def perf_of(r):
    """Mean '% Processor Performance' of the CPUs this process kept busy (lifetime busy >= 50 %), and of
    the machine total, over the process's samples."""
    ps = r.get('perf') or []
    busy = r.get('percpu_busy') or []
    cpus = [i for i, b in enumerate(busy) if b is not None and b >= 50]
    if not cpus and busy:
        cpus = sorted(range(len(busy)), key=lambda i: -(busy[i] or 0))[:max(1, r['W'])]
    vals = [statistics.mean([p['cpu'][i] for i in cpus if p['cpu'][i] is not None]) for p in ps
            if any(p['cpu'][i] is not None for i in cpus)]
    tot = [p['total'] for p in ps if p.get('total') is not None]
    return (statistics.mean(vals) if vals else None), (statistics.mean(tot) if tot else None), cpus


def pearson(xs, ys):
    if len(xs) < 3:
        return None
    mx, my = statistics.mean(xs), statistics.mean(ys)
    sx = math.sqrt(sum((x - mx) ** 2 for x in xs))
    sy = math.sqrt(sum((y - my) ** 2 for y in ys))
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / (sx * sy) if sx and sy else None


# ── main ─────────────────────────────────────────────────────────────────────

def main():
    recs, warm, attempts, used, excluded = load()
    R = {'raw': RAW, 'attempts': len(attempts), 'used': len(used), 'warmups': len(warm)}
    R['reruns'] = [{'pass': r['pass'], 'round': r['round'], 'seq': r['seq'], 'row': r['row'], 'W': r['W'],
                    'reason': r.get('rerun_reason'), 'rerun_valid': r.get('valid'), 'rerun_contaminated': r.get('contaminated'),
                    'rerun_mean_ms': r.get('mean_ms')} for r in attempts if r.get('attempt') == 'rerun']
    R['excluded'] = [[{'row': r['row'], 'W': r['W'], 'pass': r['pass'], 'seq': r['seq'], 'attempt': r.get('attempt'),
                       'invalid': r.get('invalid'), 'contaminated': r.get('contaminated'), 'skipped': r.get('skipped'),
                       'aborted': r.get('aborted')} for r in rs] for rs in excluded]
    R['contaminated_attempts'] = [{'row': r['row'], 'W': r['W'], 'pass': r['pass'], 'round': r['round'], 'seq': r['seq'],
                                   'attempt': r.get('attempt'), 'rb': r['receipt_before']['cpu_avg'],
                                   'ra': r['receipt_after']['cpu_avg'],
                                   'build_before': r['receipt_before']['build_procs_busy'],
                                   'build_after': r['receipt_after']['build_procs_busy'],
                                   'top_after': r['receipt_after']['top5'][:3], 'mean_ms': r.get('mean_ms')}
                                  for r in attempts if r.get('contaminated')]
    R['invalid_attempts'] = [{'row': r['row'], 'W': r['W'], 'pass': r['pass'], 'seq': r['seq'], 'why': r.get('invalid')}
                             for r in attempts if r.get('invalid')]
    R['warmup_means'] = [(r['pass'], r.get('mean_ms')) for r in warm]

    # Receipts: every distinct receipt (the between-process ones are shared).
    seen = {}
    for r in recs:
        for k in ('receipt_before', 'receipt_after'):
            x = r.get(k)
            if x:
                seen[x['time']] = x
    rc = sorted(seen.values(), key=lambda x: x['time'])
    vals = [x['cpu_avg'] for x in rc if x['cpu_avg'] is not None]
    tops = {}
    for x in rc:
        if x['top5']:
            tops[x['top5'][0]['name']] = tops.get(x['top5'][0]['name'], 0) + 1
    R['receipts'] = {'n': len(vals), 'n_10s': sum(1 for x in rc if x['window_s'] >= 10), 'min': min(vals),
                     'median': statistics.median(vals), 'p90': statistics.quantiles(vals, n=10)[-1] if len(vals) > 10 else None,
                     'max': max(vals), 'over5': [(x['time'], x['cpu_avg'], [t['name'] for t in x['top5'][:3]])
                                                 for x in rc if (x['cpu_avg'] or 0) > 5],
                     'build_procs': [(x['time'], x['build_procs_busy']) for x in rc if x['build_procs_busy']],
                     'top_process_counts': dict(sorted(tops.items(), key=lambda kv: -kv[1])[:6])}
    ob = [r['others_busy_pct'] for r in used if r.get('others_busy_pct') is not None]
    R['others_during'] = {'n': len(ob), 'median': statistics.median(ob), 'max': max(ob),
                          'over5': [(r['row'], r['W'], r['pass'], r['seq'], r['others_busy_pct'],
                                     [t['name'] for t in r['others_top5'][:3]]) for r in used
                                    if (r.get('others_busy_pct') or 0) > 5]}

    # Cells.
    cells = {}
    for r in used:
        cells.setdefault((r['row'], r['W']), []).append(r)
    C = {}
    for (row, w), rs in sorted(cells.items(), key=lambda kv: (kv[0][1], kv[0][0])):
        rs = sorted(rs, key=lambda r: (r['g'], r['seq']))
        s = stats([r['mean_ms'] for r in rs])
        s['values_by_process'] = [round(r['mean_ms'], 4) for r in rs]
        s['per_pass_median'] = [statistics.median([r['mean_ms'] for r in rs if r['pass'] == p]) for p in (0, 1)
                                if any(r['pass'] == p for r in rs)]
        s['median_step'] = stats([r['median_ms'] for r in rs if r.get('median_ms') is not None])
        if rs[0]['engine'] == 'boyko':
            s['summary_vs_csv_max_rel'] = max(abs(r['summary']['window_mean_ns'] / 1e6 / r['mean_ms'] - 1) for r in rs)
        C[f'{row}@W{w}'] = s
    R['cells'] = C

    def c(row, w):
        return C.get(f'{row}@W{w}')

    cmp = {}
    for w in (1, 2, 4, 8, 16):
        cmp[f'boyko/Jolt v5.3.0 @W{w} (J-A-d1 vs JOLT-T)'] = compare(c('JOLT-T', w), c('J-A-d1', w))
        cmp[f'boyko/Jolt v5.6.0 @W{w} (J-A-d1 vs JOLT56-T)'] = compare(c('JOLT56-T', w), c('J-A-d1', w))
        cmp[f'Jolt v5.6.0 vs v5.3.0 @W{w}'] = compare(c('JOLT-T', w), c('JOLT56-T', w))
    for w in (1, 8):
        cmp[f'A/A1 armed vs disarmed @W{w} (J-A-a vs J-A-d1)'] = compare(c('J-A-d1', w), c('J-A-a', w))
        cmp[f'A/A2 tip vs pre-instrument @W{w} (J-A-d1 vs AA2-pre)'] = compare(c('AA2-pre', w), c('J-A-d1', w))
        cmp[f'shipping vs dev (disarmed) @W{w} (SHIP vs J-A-d1)'] = compare(c('J-A-d1', w), c('SHIP', w))
        cmp[f'canary @W{w} (J-C vs J-A-a)'] = compare(c('J-A-a', w), c('J-C', w))
        cmp[f'cfg-B vs cfg-A, armed @W{w} (J-B vs J-A-a)'] = compare(c('J-A-a', w), c('J-B', w))
        cmp[f'Jolt -allow_sleep vs timed @W{w}'] = compare(c('JOLT-T', w), c('JOLT-SLP', w))
    cmp['L1 gate: tip vs pre-L1, J-S0 disarmed @W1 (L1-B vs L1-A1)'] = compare(c('L1-A1', 1), c('L1-B', 1))
    cmp['arming on J-S0 @W1 (J-S0 armed vs L1-B disarmed tip)'] = compare(c('L1-B', 1), c('J-S0', 1))
    cmp['sleep bookkeeping, threshold 0, armed @W1 (J-S0 vs J-A-a)'] = compare(c('J-A-a', 1), c('J-S0', 1))
    cmp['J-P1 vs J-A-a @W1 (parallel_solve at one worker)'] = compare(c('J-A-a', 1), c('J-P1', 1))
    cmp['D1 price @W1 (R-ref vs R)'] = compare(c('R', 1), c('R-ref', 1))
    cmp['Delta_J @W8 (JOLT-NPC vs JOLT-T)'] = compare(c('JOLT-T', 8), c('JOLT-NPC', 8))
    R['comparisons'] = {k: v for k, v in cmp.items() if v}
    R['delta_J_ms'] = (c('JOLT-NPC', 8)['median'] - c('JOLT-T', 8)['median']) if c('JOLT-NPC', 8) and c('JOLT-T', 8) else None
    R['scaling'] = {}
    for row in ('J-A-d1', 'JOLT-T', 'JOLT56-T'):
        t1 = c(row, 1)
        if t1:
            R['scaling'][row] = {w: t1['median'] / c(row, w)['median'] for w in (1, 2, 4, 8, 16) if c(row, w)}
    for row in ('R', 'S16', 'J-Son', 'R-S', 'J-A-a', 'J-B', 'J-C', 'AA2-pre', 'SHIP'):
        if c(row, 1) and c(row, 8):
            R['scaling'][row] = {8: c(row, 1)['median'] / c(row, 8)['median']}

    # The canary, in full.
    can = {}
    for w in (1, 8):
        jc = [r for r in used if r['row'] == 'J-C' and r['W'] == w]
        ja = c('J-A-a', w)
        if not jc or not ja:
            continue
        cns = [r['summary']['canary_ns'] for r in jc]
        spans = []
        for r in jc:
            col = csv_cols(r)
            win = D.ROWS['J-C']['dry_window'] if r['dry'] else D.ROWS['J-C']['window']
            sp = D.wmean(col.get('sys_parity_canary_ns'), win) if col else None
            if sp is not None:
                spans.append(sp / r['summary']['canary_ns'] - 1)
        expected = 100 * statistics.median(cns) / 1e6 / ja['median']
        k = cmp[f'canary @W{w} (J-C vs J-A-a)']
        can[w] = {'canary_ns': cns, 'canary_ns_median': statistics.median(cns), 'expected_rise_pct': expected,
                  'measured_rise_pct': k['effect_pct'], 'rise_minus_expected_pct': k['effect_pct'] - expected,
                  'span_rel_err_median': statistics.median(spans) if spans else None,
                  'span_rel_err_minmax': [min(spans), max(spans)] if spans else None,
                  'span_ok_5pct': bool(spans) and abs(statistics.median(spans)) <= 0.05,
                  'seen_range': k['claim_range'], 'seen_iqr': k['claim_iqr'], 'seen_se': k['claim_se'],
                  'bars': {'range': k['bar_range_pct'], 'iqr': k['bar_iqr_pct'], 'se': k['bar_se_pct']},
                  'rise_matches_within_range_bar': abs(k['effect_pct'] - expected) <= k['bar_range_pct'],
                  'rise_matches_within_iqr_bar': abs(k['effect_pct'] - expected) <= k['bar_iqr_pct'],
                  'rise_matches_within_se_bar': abs(k['effect_pct'] - expected) <= k['bar_se_pct']}
    R['canary'] = can

    # O5: sleeping floors on the tails after everything is asleep (J-Son against Jolt -allow_sleep).
    o5 = {}
    for w in (1, 8):
        bs = [r for r in used if r['row'] == 'J-Son' and r['W'] == w]
        js = [r for r in used if r['row'] == 'JOLT-SLP' and r['W'] == w]
        if not bs or not js:
            continue
        fb = [next((i for i, x in enumerate(csv_cols(r).get('awake') or []) if x == 0), None) for r in bs]
        fj = [next((i for i, x in enumerate(csv_cols(r).get('awake') or []) if x == 0), None) for r in js]
        if None in fb or None in fj:
            o5[w] = {'boyko_all_asleep_from': fb, 'jolt_all_asleep_from': fj, 'tail': None}
            continue
        start = max(fb + fj)
        tb = stats([D.wmean(csv_cols(r)['wall_ns'], (start, 1000)) / 1e6 for r in bs])
        tj = stats([D.wmean(csv_cols(r)['wall_ns'], (start, 1000)) / 1e6 for r in js])
        o5[w] = {'boyko_all_asleep_from': sorted(set(fb)), 'jolt_all_asleep_from': sorted(set(fj)), 'tail': [start, 1000],
                 'boyko_tail_ms': tb, 'jolt_tail_ms': tj, 'boyko_over_jolt_tail': compare(tj, tb)}
    R['o5'] = o5

    # Structural checks straight from the records.
    poses = {}
    for r in used:
        if r['engine'] == 'boyko':
            poses.setdefault(r['row'], set()).add(r['summary']['pose_hash'])
        else:
            poses.setdefault(r['row'], set()).add(r['jolt']['stat_lines'][0]['hash'])
    R['hashes_per_row'] = {k: sorted(v) for k, v in poses.items()}
    R['h7'] = sorted({(r['W'], r['summary'].get('expect_pose')) for r in used if r['row'] == 'J-B'})
    R['void_steps_nonzero'] = [(r['row'], r['W'], r['g']) for r in used if r['engine'] == 'boyko' and r['summary']['void_steps']]
    R['drops_nonzero'] = [(r['row'], r['W'], r['g']) for r in used if r['engine'] == 'boyko' and r['summary'].get('drops_total')]
    R['rs_first_frozen'] = sorted({(r['W'], r['summary'].get('first_frozen_step')) for r in used if r['row'] == 'R-S'})
    R['solve_on_dispatcher'] = sorted({(r['row'], r['W'], r['summary']['threads']['solve_on_dispatcher_steps'])
                                       for r in used if r['engine'] == 'boyko' and r['summary'].get('armed')})
    R['jolt_threads'] = sorted({(r['row'], r['W'], r['jolt']['stat_lines'][0]['threads']) for r in used if r['engine'] == 'jolt'})
    R['masks'] = sorted({r.get('mask_readback') for r in used})
    R['disarmed_ring_traffic_nonzero'] = [(r['row'], r['W']) for r in used if r['engine'] == 'boyko'
                                          and not r['summary'].get('armed') and r['summary'].get('disarmed_ring_traffic')]
    R['profiles'] = sorted({(r['row'], r['summary']['profile_name']) for r in used if r['engine'] == 'boyko'})

    # Jolt -p.
    R['jolt_p'] = jolt_p(used)

    # Witnesses: the per-process deviation from its cell median against the perf and busy witnesses.
    xs, ys, zs, rows = [], [], [], []
    for (row, w), rs in cells.items():
        if row == 'JOLT-P' or len(rs) < 3:
            continue
        med = statistics.median([r['mean_ms'] for r in rs])
        pv = [perf_of(r) for r in rs]
        pb = [p[0] for p in pv if p[0] is not None]
        pt = [p[1] for p in pv if p[1] is not None]
        if len(pb) != len(rs) or len(pt) != len(rs):
            continue
        mb, mt = statistics.mean(pb), statistics.mean(pt)
        for r, (b, t, _) in zip(rs, pv):
            xs.append(100 * (r['mean_ms'] / med - 1))
            ys.append(100 * (b / mb - 1))
            zs.append(100 * (t / mt - 1))
            rows.append((row, w))
    R['witness'] = {'n': len(xs), 'r_dev_vs_busy_cpu_perf': pearson(xs, ys), 'r_dev_vs_total_perf': pearson(xs, zs),
                    'busy_cpu_perf_dev_sd_pct': statistics.pstdev(ys) if ys else None,
                    'time_dev_sd_pct': statistics.pstdev(xs) if xs else None,
                    'note': 'exploratory: a faster clock should show as NEGATIVE r (less time, more perf)'}
    by_engine = {}
    for (row, w), x, y in zip(rows, xs, ys):
        e = 'jolt' if row.startswith('JOLT') else 'boyko'
        by_engine.setdefault(e, ([], []))
        by_engine[e][0].append(x)
        by_engine[e][1].append(y)
    R['witness']['by_engine'] = {e: {'n': len(v[0]), 'r': pearson(v[0], v[1])} for e, v in by_engine.items()}
    # Lag-1 autocorrelation of deviations in run order, and the perf level per pass.
    order = sorted(used, key=lambda r: (r['pass'], r['seq']))
    devs = []
    for r in order:
        cs = cells[(r['row'], r['W'])]
        if len(cs) >= 3 and r['row'] != 'JOLT-P':
            devs.append(100 * (r['mean_ms'] / statistics.median([x['mean_ms'] for x in cs]) - 1))
    R['witness']['lag1_autocorr_dev'] = pearson(devs[:-1], devs[1:]) if len(devs) > 3 else None
    R['witness']['perf_total_by_pass'] = {p: statistics.median([perf_of(r)[1] for r in used if r['pass'] == p
                                                                 and perf_of(r)[1] is not None]) for p in (0, 1)
                                          if any(r['pass'] == p for r in used)}

    # Wall clock.
    st = json.load(open(os.path.join(RAW, 'window_state.json'), encoding='utf-8')) \
        if os.path.isfile(os.path.join(RAW, 'window_state.json')) else {}
    R['wall_clock'] = {'start': st.get('start'), 'end': st.get('end'), 'status': st.get('status'),
                       'first_process': min(r['start'] for r in attempts if r.get('start')),
                       'last_process_end': max(r['end'] for r in attempts if r.get('end')),
                       'binaries_after': st.get('binaries_after'),
                       'machine_before': st.get('machine_before'), 'machine_after': st.get('machine_after'),
                       'per_pass': {p: [min(r['start'] for r in recs if r['pass'] == p and r.get('start')),
                                        max(r['end'] for r in recs if r['pass'] == p and r.get('end'))]
                                    for p in sorted({r['pass'] for r in recs})}}

    R['broadphase'] = broadphase()

    # The driver's own structural reduction over the used processes, one synthetic 'pass' per round (g).
    dred = os.path.join(RAW, 'driver_reduce')
    os.makedirs(dred, exist_ok=True)
    with open(os.path.join(dred, 'runs.jsonl'), 'w', encoding='utf-8') as f:
        for r in used:
            x = dict(r)
            x['pass'] = r['g']
            f.write(json.dumps(x) + '\n')
    shutil.copyfile(os.path.join(RAW, 'manifest.json'), os.path.join(dred, 'manifest.json'))
    p = subprocess.run([sys.executable, '-B', os.path.join(HERE, 'lib', 'driver.py'), 'reduce', '--out', dred]
                       + (['--dry'] if used and used[0].get('dry') else []), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    R['driver_reduce_exit'] = p.returncode
    R['driver_reduce_stderr_tail'] = p.stderr.decode('utf-8', 'replace')[-800:]
    if p.returncode == 0:
        DR = json.load(open(os.path.join(dred, 'reduction.json'), encoding='utf-8'))
        R['driver'] = {k: DR.get(k) for k in ('determinism_boyko', 'determinism_jolt_per_W', 'h7_expect_pose',
                                              'anti_vacuity_void', 'closure', 'identity', 'rs_first_frozen_step',
                                              'rs_void', 'o5', 'threads', 'build_checks', 'nonzero_exits', 'h6')}
        R['driver']['profile_J-A'] = DR.get('profile_J-A')

    with open(os.path.join(RAW, 'analysis.json'), 'w', encoding='utf-8') as f:
        json.dump(R, f, indent=1, default=str)
    print(f'analysis: {len(used)} used processes, {len(C)} cells -> {os.path.join(RAW, "analysis.json")}')


if __name__ == '__main__':
    main()
