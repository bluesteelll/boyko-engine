"""Window 7, the results-analyst's reduction (untimed). Independent of tools/reduce7.py: every number is recomputed
from the raw files (run.csv, per_frame_*.csv, stdout.txt SUMMARY / stat lines, the Jolt profile_chart_*.html dumps),
validity and cleanliness are re-derived from the files and the receipts, and nothing is imported from reduce7.py or
from the driver's `mean_ms` / `cols` / `valid` / `contaminated` fields (those are only compared against).

Writes <win7>/analyst/tables.txt and <win7>/analyst/reduction.json; prints the same text.

Statistic (plan.md section 1): per process the mean of the per-step wall over the row's window (ours: `wall_ns` of
run.csv; Jolt: `Time (ms)` of per_frame_discrete_thW.csv), per metric window likewise; per cell the median over K,
[min-max], r = range/median, i = IQR/median, s = 1.2533*sd/sqrt(K)/median. B against A is CLAIMED iff
|B/A - 1| > 2*hypot(sA, sB) under BOTH r and s (i printed); no claim when either side has K < 3.
Slot rule: per (block, pass, round, row, binary, W) the original if valid and clean, else its re-run if valid and
clean, else the slot is dropped. Clean = both 5-s receipts (before, after) <= 5.0 % machine busy.
"""
import csv
import glob
import json
import math
import os
import re
import statistics
import sys
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
W7 = os.path.dirname(HERE)
RAW = os.path.join(W7, 'raw')
GATE = os.path.join(W7, 'gate')
OUTD = os.path.join(W7, 'analyst')
WS = (1, 2, 4, 8, 16)
JOLT_HASH = '0xb8522b4e3fc62cfe'
BUSY = 5.0
# window 3's JOLT56-T cells (context only; docs/measurements/2026-09-21-physics-window3/, window 6 analysis.md:13)
WIN3_JOLT56 = {1: 9.8281962, 2: 5.7704603, 4: 3.5814395, 8: 2.5692559, 16: 2.3884695}
# window 6 analysis.md:129-130: the L9b prediction and the realized-gain bar
L9_PRED = (1.4994, 1.6782)
L9_BAR = 0.8997
# d2(K): the expected range of K normal draws in units of sigma (for the resolution table)
D2 = {3: 1.693, 4: 2.059, 5: 2.326, 6: 2.534, 8: 2.847, 10: 3.078, 12: 3.258, 24: 3.895}

OUT = []
RES = {'checks': {}, 'cells': {}, 'cmp': {}, 'decisions': {}, 'p5': {}, 'stages': {}}


def P(s=''):
    OUT.append(s)
    print(s)


# ----------------------------------------------------------------------------------------------- statistics
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
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'r': (xs[-1] - xs[0]) / d, 'i': iqr(xs) / d,
            's': se_med(xs) / d, 'sd_rel': (statistics.stdev(xs) / d if len(xs) >= 2 else 0.0), 'values': xs}


def cmp_(a, b):
    """B against A."""
    if not a or not b or not a['median']:
        return None
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    ok = a['K'] >= 3 and b['K'] >= 3
    cl = {k: ok and e > bars[k] for k in bars}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'], 'cl_i': cl['i'],
            'cl_s': cl['s'], 'claimed': cl['r'] and cl['s'], 'KA': a['K'], 'KB': b['K']}


def yn(c):
    return f"{'Y' if c['cl_r'] else 'n'}/{'Y' if c['cl_i'] else 'n'}/{'Y' if c['cl_s'] else 'n'}"


def fc(c, d=4):
    if not c:
        return 'n/a'
    return (f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}] K={c['K']} "
            f"({100 * c['r']:.2f}/{100 * c['i']:.2f}/{100 * c['s']:.2f})")


def fcmp(c):
    if not c:
        return 'n/a'
    return (f"{c['ratio']:.4f} ({100 * (c['ratio'] - 1):+.2f} %, {c['delta']:+.4f}); bars "
            f"{100 * c['bar_r']:.2f}/{100 * c['bar_i']:.2f}/{100 * c['bar_s']:.2f} %; {yn(c)}"
            f"{' CLAIMED' if c['claimed'] else ''}")


# ----------------------------------------------------------------------------------------------- inputs
def local_dir(rec):
    cwd = rec['cwd'].replace('\\', '/')
    parts = cwd.split('/')
    return os.path.join(RAW, parts[-2], parts[-1])


def read_sums():
    out = {}
    for line in open(os.path.join(W7, 'bin', 'SHA256SUMS'), encoding='utf-8'):
        if line.strip():
            h, p = line.split(None, 1)
            out[p.strip().lstrip('*').replace('\\', '/')] = h
    return out


ROWS = json.load(open(os.path.join(W7, 'rows7.json'), encoding='utf-8'))
ROWDEF = {r['id']: r for r in ROWS['rows']}
BINS = ROWS['binaries']
FIX = {k: v['hash'] for k, v in json.load(open(os.path.join(GATE, 'fixtures', 'fixtures.json'), encoding='utf-8')).items()}
SUMS = read_sums()


def bin_sha(key):
    exe = BINS[key]['exe'].replace('\\', '/')
    for p, h in SUMS.items():
        if p == exe or p.endswith(exe):
            return h
    return None


def summary_of(d):
    p = os.path.join(d, 'stdout.txt')
    if not os.path.exists(p):
        return None
    for line in open(p, encoding='utf-8', errors='replace'):
        if line.startswith('SUMMARY '):
            return json.loads(line[len('SUMMARY '):])
    return None


def runner_csv(d):
    p = os.path.join(d, 'run.csv')
    if not os.path.exists(p):
        return None
    with open(p, encoding='utf-8') as f:
        rows = list(csv.DictReader(f))
    return rows


def colmean(rows, a, b, col, stat='mean'):
    xs = []
    for r in rows:
        s = int(r['step'])
        if a <= s < b:
            v = r.get(col)
            if v not in (None, ''):
                xs.append(float(v))
    if not xs:
        return None
    return statistics.mean(xs) if stat == 'mean' else statistics.median(xs)


def jolt_frames(d, W):
    p = os.path.join(d, f'per_frame_discrete_th{W}.csv')
    if not os.path.exists(p):
        return None
    with open(p, encoding='utf-8') as f:
        rd = csv.reader(f)
        next(rd)
        return [(int(x[0]), float(x[1])) for x in rd if x]


def jolt_stdout(d):
    p = os.path.join(d, 'stdout.txt')
    txt = open(p, encoding='utf-8', errors='replace').read() if os.path.exists(p) else ''
    stats = re.findall(r'^\s*Discrete,\s*(\d+),\s*([\d.]+),\s*(0x[0-9a-f]+)\s*$', txt, re.M)
    banner = [l for l in txt.splitlines() if 'boyko-parity-patch' in l]
    return stats, banner


# ----------------------------------------------------------------------------------------------- Jolt profile
def _js_to_json(txt):
    txt = re.sub(r'(?m)^(\s*)(\w+):', r'\1"\2":', txt)
    txt = re.sub(r',\s*([}\]])', r'\1', txt)
    return json.loads(txt)


def _grab(s, head, open_ch, close_ch):
    i = s.find(head)
    if i < 0:
        return None
    j = s.find(open_ch, i)
    depth, k, in_str = 0, j, False
    while k < len(s):
        ch = s[k]
        if in_str:
            if ch == '\\':
                k += 2
                continue
            if ch == '"':
                in_str = False
        elif ch == '"':
            in_str = True
        elif ch == open_ch:
            depth += 1
        elif ch == close_ch:
            depth -= 1
            if depth == 0:
                return s[j:k + 1]
        k += 1
    return None


JOB_STAGE = {
    'UpdateBroadPhasePrepare': 'bp', 'UpdateBroadPhaseFinalize': 'bp',
    'FindCollisions': 'np_rest',
    'ApplyGravity': 'integrate', 'PreIntegrateVelocity': 'integrate', 'IntegrateVelocity': 'integrate',
    'PostIntegrateVelocity': 'integrate',
    'BuildIslandsFromConstraints': 'islands', 'FinalizeIslands': 'islands', 'BodySetIslandIndex': 'islands',
    'DetermineActiveConstraints': 'islands',
    'SetupVelocityConstraints': 'setup',
    'SolveVelocityConstraints': 'solve_vel',
    'SolvePositionConstraints': 'solve_pos',
    'ContactRemovedCallbacks': 'other', 'ResolveCCDContacts': 'other', 'FindCCDContacts': 'other',
    'SoftBodyPrepare': 'other', 'SoftBodyCollide': 'other', 'SoftBodySimulate': 'other', 'SoftBodyFinalize': 'other',
}


def sub_stage(name, inherited):
    """A scope inside a job that is re-attributed to another stage."""
    if name.startswith('virtual void JPH::BroadPhaseQuadTree::FindCollidingPairs'):
        return 'bp'
    if name.startswith('virtual void JPH::BroadPhaseQuadTree::NotifyBodiesAABBChanged'):
        return 'bp'
    if name == 'Add Constraint From Cached Manifold':
        return 'np_cached'
    if 'CollideConvexVsConvex' in name or 'CollideShape' in name or name.startswith('static void JPH::CollisionDispatch'):
        return 'np_collide'
    if 'WarmStartVelocityConstraints' in name:
        return 'warm'
    if name in ('Check Sleeping', 'Update Bounds'):
        return 'integrate'
    if name.startswith('void JPH::ContactConstraintManager::SortContacts') or \
            name.startswith('bool JPH::LargeIslandSplitter::SplitIsland'):
        return 'solve_prep'
    return inherited


def parse_profile(path):
    """Per-scope and per-stage numbers of one single-frame Jolt profile dump. Exclusive (self) cycles are
    attributed to the sample's stage: the stage of its nearest job ancestor, overridden by sub_stage for the
    re-attributed scopes (and inherited by their descendants). cpu = sum over threads of self time; wall of a
    stage = the union over threads of the intervals during which a thread's innermost open sample is in it."""
    s = open(path, encoding='utf-8', errors='replace').read()
    cps = float(re.search(r'var\s+cycles_per_second\s*=\s*([\d.]+)', s).group(1))
    threads = _js_to_json(_grab(s, 'var threads', '[', ']'))
    agg = _js_to_json(_grab(s, 'var aggregated', '{', '}'))
    names = [n.replace('&lt;', '<').replace('&gt;', '>').replace('&amp;', '&') for n in agg['name']]
    stage_self = defaultdict(float)
    stage_n = defaultdict(int)
    stage_iv = defaultdict(list)
    scope_incl = defaultdict(float)
    scope_calls = defaultdict(int)
    job_iv = defaultdict(list)
    n_samples = 0
    update_cycles = 0
    for t in threads:
        smp = sorted(zip(t['start'], t['depth'], t['cycles'], t['aggregator']))
        stack = []  # entries: [start, end, depth, stage, child_cycles, name]
        closed = []

        def close(e):
            closed.append(e)
        for st, dp, cy, ag in smp:
            n_samples += 1
            nm = names[ag]
            while stack and (stack[-1][2] >= dp or stack[-1][1] <= st):
                close(stack.pop())
            parent_stage = stack[-1][3] if stack else 'frame'
            stg = JOB_STAGE.get(nm, parent_stage)
            stg = sub_stage(nm, stg)
            if stack:
                stack[-1][4] += cy
            stack.append([st, st + cy, dp, stg, 0, nm])
            scope_incl[nm] += cy
            scope_calls[nm] += 1
            stage_n[stg] += 1
            if nm in JOB_STAGE:
                job_iv[nm].append((st, st + cy))
            if nm.startswith('JPH::EPhysicsUpdateError JPH::PhysicsSystem::Update'):
                update_cycles += cy
        while stack:
            close(stack.pop())
        # self time and the innermost-open intervals: a sample's interval minus its children's intervals
        closed.sort(key=lambda e: (e[0], e[2]))
        kids = defaultdict(list)
        stack = []
        for idx, e in enumerate(closed):
            while stack and (closed[stack[-1]][2] >= e[2] or closed[stack[-1]][1] <= e[0]):
                stack.pop()
            if stack:
                kids[stack[-1]].append(idx)
            stack.append(idx)
        for idx, e in enumerate(closed):
            stage_self[e[3]] += e[1] - e[0] - e[4]
            cur = e[0]
            for k in sorted(kids[idx], key=lambda j: closed[j][0]):
                ks, ke = closed[k][0], closed[k][1]
                if ks > cur:
                    stage_iv[e[3]].append((cur, ks))
                cur = max(cur, ke)
            if e[1] > cur:
                stage_iv[e[3]].append((cur, e[1]))

    def union(iv):
        tot, cs, ce = 0, None, None
        for a, b in sorted(iv):
            if ce is None or a > ce:
                if ce is not None:
                    tot += ce - cs
                cs, ce = a, b
            else:
                ce = max(ce, b)
        if ce is not None:
            tot += ce - cs
        return tot
    # the wall partition: sweep over every thread's innermost intervals of the non-'frame' stages; each elementary
    # segment is split between the stages open in it, in proportion to how many threads are in each. Instants with
    # no thread in any stage are 'sched' (the main thread inside Update/WaitForJobs only: scheduling, barriers).
    ev = []
    for stg, iv in stage_iv.items():
        if stg == 'frame':
            continue
        for a_, b_ in iv:
            if b_ > a_:
                ev.append((a_, 1, stg))
                ev.append((b_, -1, stg))
    ev.sort()
    part = defaultdict(float)
    cnt = defaultdict(int)
    tot_open = 0
    last = None
    for t_, d_, stg in ev:
        if last is not None and t_ > last and tot_open > 0:
            dt = t_ - last
            for s_, n_ in cnt.items():
                if n_:
                    part[s_] += dt * n_ / tot_open
        cnt[stg] += d_
        tot_open += d_
        last = t_
    ms = 1e3 / cps
    part_ms = {k: v * ms for k, v in part.items()}
    part_ms['sched'] = update_cycles * ms - sum(part_ms.values())
    return {'cps': cps, 'threads': len(threads), 'n_samples': n_samples, 'update_ms': update_cycles * ms,
            'stage_part': part_ms,
            'stage_cpu': {k: v * ms for k, v in stage_self.items()},
            'stage_wall': {k: union(v) * ms for k, v in stage_iv.items()},
            'stage_n': dict(stage_n),
            'job_wall': {k: union(v) * ms for k, v in job_iv.items()},
            'job_cpu': {k: sum(b - a for a, b in v) * ms for k, v in job_iv.items()},
            'scope_cpu': {k: v * ms for k, v in scope_incl.items()}, 'scope_calls': dict(scope_calls)}


# ----------------------------------------------------------------------------------------------- per process
def recompute(rec):
    """Returns (mine: dict of recomputed statistics, problems: list of validity failures)."""
    row = ROWDEF[rec['row']]
    d = local_dir(rec)
    W = rec['W']
    a, b = row['window']
    probs = []
    mine = {'dir': d, 'cols': {}}
    if rec.get('exit') != 0:
        probs.append(f"exit {rec.get('exit')}")
    sha = bin_sha(rec['binary'])
    if sha is None or rec.get('exe_sha256') != sha:
        probs.append('exe sha256 != SHA256SUMS')
    if rec.get('mask_readback') != '0xffff':
        probs.append(f"mask {rec.get('mask_readback')}")
    if rec['kind'] == 'runner':
        sm = summary_of(d)
        rows = runner_csv(d)
        if not sm or not rows:
            return mine, probs + ['no SUMMARY or run.csv']
        mine['summary'] = sm
        if len(rows) != row['steps']:
            probs.append(f'{len(rows)} csv rows != {row["steps"]}')
        if sm.get('void_steps') != 0:
            probs.append(f"void_steps {sm.get('void_steps')}")
        if sm.get('expect_pose') != 'match':
            probs.append(f"expect_pose {sm.get('expect_pose')}")
        if sm.get('pose_hash') != FIX.get(row['pose_ref']):
            probs.append(f"pose {sm.get('pose_hash')} != {row['pose_ref']}")
        if sm.get('workers') != W or (sm.get('threads') or {}).get('pool_workers') != W:
            probs.append('workers != W')
        if sm.get('target_env') != 'msvc':
            probs.append('target_env')
        if bool(sm.get('armed')) != bool(row['armed']):
            probs.append('armed flag')
        if row['armed']:
            if sm.get('drops_total') != 0:
                probs.append(f"drops {sm.get('drops_total')}")
        if sm.get('disarmed_ring_traffic') not in (None, 0):
            probs.append('disarmed ring traffic')
        cfg = sm.get('config') or {}
        if cfg.get('broadphase') != row['broadphase']:
            probs.append(f"broadphase {cfg.get('broadphase')}")
        td = sm.get('broadphase_tree') or {}
        if row['broadphase'] == 'Tree':
            if (td.get('static_rebuilds'), td.get('members'), td.get('evictions')) != (1, 1, 0):
                probs.append(f'TreeDiag {td}')
        elif any(v for v in td.values()):
            probs.append(f'TreeDiag on AllPairs {td}')
        args = row['args']
        if '--contact-reuse' in args:
            want = args[args.index('--contact-reuse') + 1] == 'on'
            if cfg.get('contact_reuse') != want:
                probs.append('contact_reuse config')
        elif rec['binary'] == 'trk' and cfg.get('contact_reuse') is not False:
            probs.append('trunk default reuse not off')
        if row.get('canary_of') and not sm.get('canary_ns'):
            probs.append('canary_ns missing')
        mine['mean_ms'] = colmean(rows, a, b, 'wall_ns') / 1e6
        wins = [(a, b)] + [tuple(w) for w in (row.get('metric_windows') or [])]
        cols = [c for c in rows[0].keys() if c != 'step']
        for (x, y) in wins:
            key = f'{x}..{y}'
            mine['cols'][key] = {}
            for c in cols:
                mm = colmean(rows, x, y, c, 'mean')
                if mm is None:
                    continue
                md = colmean(rows, x, y, c, 'median')
                mine['cols'][key][c] = {'mean': mm, 'median': md}
    else:
        fr = jolt_frames(d, W)
        stats, banner = jolt_stdout(d)
        if not fr:
            return mine, probs + ['no per_frame csv']
        if len(fr) != 500:
            probs.append(f'{len(fr)} frames')
        if len(stats) != 1:
            probs.append(f'{len(stats)} stat lines')
        else:
            th, sps, h = stats[0]
            if int(th) != W:
                probs.append('threads != W')
            if h != JOLT_HASH:
                probs.append(f'hash {h}')
        if not banner or 'boyko-parity-patch v1' not in banner[0] or 'allow_sleep=0, receipt=0' not in banner[0]:
            probs.append('patch banner')
        mine['mean_ms'] = statistics.mean(t for f, t in fr if a <= f < b)
        for (x, y) in [(a, b)] + [tuple(w) for w in (row.get('metric_windows') or [])]:
            ts = [t for f, t in fr if x <= f < y]
            mine['cols'][f'{x}..{y}'] = {'wall_ns': {'mean': statistics.mean(ts) * 1e6,
                                                     'median': statistics.median(ts) * 1e6}}
        mine['frames'] = dict(fr)
        if rec['kind'] == 'joltprof':
            prof = {}
            for it in (100, 200, 300, 400):
                p = os.path.join(d, f'profile_chart_discrete_th{W}_it{it}.html')
                if not os.path.exists(p):
                    probs.append(f'dump it{it} missing')
                    continue
                prof[it] = parse_profile(p)
            mine['profile'] = prof
    return mine, probs


def main():
    try:
        sys.stdout.reconfigure(encoding='utf-8', newline='\n')
    except (AttributeError, ValueError):
        pass
    os.makedirs(OUTD, exist_ok=True)
    recs = [json.loads(l) for l in open(os.path.join(RAW, 'runs.jsonl'), encoding='utf-8') if l.strip()]
    procs = [r for r in recs if 'row' in r]
    markers = [r for r in recs if 'row' not in r]
    P('# Window 7: the analyst\'s reduction (tools/analyze7.py), recomputed from raw/')
    P()
    P('## 0. Selection and input checks')
    att = defaultdict(int)
    for r in procs:
        att[r['attempt']] += 1
    P(f'- {len(recs)} records = {len(procs)} processes ({dict(att)}) + {len(markers)} pass markers')
    # one run tag per (block, pass), taken from the pass markers
    done_tag = {(m['block'], m['pass']): m['run_tag'] for m in markers if m.get('pass_done')}
    tags = defaultdict(set)
    for r in procs:
        tags[(r['block'], r['pass'])].add(r['run_tag'])
    P(f"- passes done: {len(done_tag)}; run tags per pass: "
      + ', '.join(f'{k[0]}-p{k[1]}={sorted(v)}' for k, v in sorted(tags.items())))
    voided = [r for r in recs if r.get('voided_pass')]
    P(f'- voided pass records: {len(voided)}')
    # recompute every non-warm-up process
    mism = defaultdict(int)
    nproc = 0
    my_valid, my_clean = {}, {}
    probs_all = []
    presence = 0
    busy_rec = []
    for r in procs:
        if r['attempt'] == 'warmup':
            continue
        nproc += 1
        mine, probs = recompute(r)
        r['_mine'] = mine
        rb, ra = r['receipt_before']['cpu_avg'], r['receipt_after']['cpu_avg']
        busy_rec += [rb, ra]
        for rc in (r['receipt_before'], r['receipt_after']):
            if (rc.get('presence') or {}).get('build') or (rc.get('presence') or {}).get('lane') or rc.get('build_procs_busy'):
                presence += 1
        if r.get('build_proc_during') or r.get('void_names_new_during'):
            presence += 1
        clean = rb <= BUSY and ra <= BUSY
        valid = not probs
        my_valid[id(r)] = valid
        my_clean[id(r)] = clean
        r['_valid'], r['_clean'] = valid, clean
        if probs:
            probs_all.append((r['block'], r['pass'], r['seq'], r['attempt'], r['row'], r['W'], probs))
        if valid != bool(r.get('valid')):
            mism['valid flag'] += 1
        if clean != (not r.get('contaminated')):
            mism['clean flag'] += 1
        if 'mean_ms' in mine and r.get('mean_ms') is not None and abs(mine['mean_ms'] - r['mean_ms']) > 1e-6:
            mism['mean_ms'] += 1
        for win, cols in (r.get('cols') or {}).items():
            for c, v in cols.items():
                m2 = (mine['cols'].get(win) or {}).get(c)
                if m2 is None:
                    mism['col missing'] += 1
                    continue
                for st in ('mean', 'median'):
                    if st in v and abs(m2[st] - v[st]) > 1e-3 * max(1.0, abs(v[st])):
                        mism[f'col {st}'] += 1
    P(f'- {nproc} non-warm-up processes recomputed; validity problems in {len(probs_all)}: '
      + ('; '.join(f'{p}' for p in probs_all[:10]) if probs_all else 'none'))
    P(f"- against the driver's per-process fields: {dict(mism) if mism else 'every valid flag, clean flag, mean_ms and cols value reproduces'}")
    P(f'- build / lane presence at any receipt or during any process: {presence}')
    rcp = {}
    for r in procs:
        for rc in (r['receipt_before'], r['receipt_after']):
            rcp[rc['time']] = rc
    busy_rec = sorted(rc['cpu_avg'] for rc in rcp.values())
    over = [rc for rc in rcp.values() if rc['cpu_avg'] > BUSY]
    tops = defaultdict(int)
    for rc in over:
        if rc['top5']:
            tops[rc['top5'][0]['name']] += 1
    P(f'- receipts (distinct, every process incl. warm-ups and the pass openings): {len(busy_rec)}; median '
      f'{statistics.median(busy_rec):.2f} %, p90 {busy_rec[int(0.9 * len(busy_rec))]:.2f} %, max {busy_rec[-1]:.2f} %, '
      f'{len(over)} over 5 % (top process in those: {dict(tops)})')
    RES['checks']['receipts'] = {'n': len(busy_rec), 'median': statistics.median(busy_rec), 'max': busy_rec[-1], 'over5': len(over),
                                 'over5_top': dict(tops)}
    # slots
    slots = defaultdict(dict)
    for r in procs:
        if r['attempt'] not in ('original', 'rerun'):
            continue
        if r['run_tag'] != done_tag.get((r['block'], r['pass'])):
            continue
        slots[(r['block'], r['pass'], r['round'], r['row'], r['binary'], r['W'])][r['attempt']] = r
    chosen, dropped, by_rerun = [], [], 0
    for k, a in slots.items():
        pick = None
        for tag in ('original', 'rerun'):
            x = a.get(tag)
            if x and x['_valid'] and x['_clean']:
                pick = x
                break
        if pick:
            chosen.append(pick)
            by_rerun += pick['attempt'] == 'rerun'
        else:
            dropped.append(k)
    P(f'- slots {len(slots)}: used {len(chosen)} ({by_rerun} by their re-run), dropped {len(dropped)} (both attempts hot):')
    for k in sorted(dropped):
        P(f'  - {k[0]} p{k[1]} r{k[2]} {k[3]} {k[4]} W{k[5]}')
    RES['checks'] = {'records': len(recs), 'processes': len(procs), 'slots': len(slots), 'used': len(chosen),
                     'used_by_rerun': by_rerun, 'dropped': [list(k) for k in sorted(dropped)],
                     'validity_problems': probs_all, 'driver_mismatch': dict(mism), 'presence': presence}
    # the during-process witness by pass, over the used set
    P('- the during-process witness (others_busy_pct), used set, by pass: median / p90 / max; the top "other" process')
    byp = defaultdict(list)
    topn = defaultdict(lambda: defaultdict(int))
    for r in chosen:
        byp[(r['block'], r['pass'])].append(r['others_busy_pct'])
        if r['others_top5']:
            topn[(r['block'], r['pass'])][r['others_top5'][0]['name']] += 1
    order = ['P1A-jolt', 'P2-L9GTW', 'P3-G9JA', 'P4-trkspans', 'P5-joltprof', 'P1B-jolt']
    for blk in order:
        for ps in (0, 1):
            v = sorted(byp.get((blk, ps), []))
            if v:
                t = sorted(topn[(blk, ps)].items(), key=lambda kv: -kv[1])[:2]
                ts = [r['start'][11:19] for r in chosen if (r['block'], r['pass']) == (blk, ps)]
                P(f'  - {blk}-p{ps} ({min(ts)}-{max(ts)}, n={len(v)}): {statistics.median(v):.2f} / '
                  f'{v[int(0.9 * len(v))]:.2f} / {v[-1]:.2f} %; top {t}')

    def C(row, key, w, fn, blocks=None, passes=None):
        rs = [r for r in chosen if r['row'] == row and r['binary'] == key and r['W'] == w
              and (blocks is None or r['block'] in blocks) and (passes is None or r['pass'] in passes)]
        return cell([fn(r) for r in rs])

    def wall(r):
        return r['_mine']['mean_ms']

    def zone(win, col, stat='mean', scale=1e-6):
        def f(r):
            c = (r['_mine']['cols'].get(win) or {}).get(col)
            return c[stat] * scale if c else None
        return f

    # =========================================================================================== P1
    P()
    P('## 1. P1: the Jolt headline (H-trk-tree / H-trk-ap on trk 93b2615b against H-jolt56, Jolt v5.6.0 918fd2b7)')
    rc = json.load(open(os.path.join(GATE, 'jolt_receipt.json'), encoding='utf-8'))
    # recompute Jolt's receipt counts from the receipt csv
    jm = {}
    for w in (1, 8):
        p = os.path.join(GATE, 'jolt_receipt', f'j56_W{w}', f'receipt_discrete_th{w}.csv')
        with open(p, encoding='utf-8') as f:
            rd = list(csv.reader(f))[1:]
        man = [float(x[1]) for x in rd]
        pts = [float(x[2]) for x in rd]
        jm[w] = {'m_100_500': statistics.mean(man[100:500]), 'p_100_500': statistics.mean(pts[100:500]),
                 'm_0': man[0], 'p_0': pts[0], 'm_final': man[-1]}
    MJ, PJ = jm[1]['m_100_500'], jm[1]['p_100_500']
    P(f"- Jolt receipt (gate, untimed), recomputed from receipt_discrete_thW.csv: manifolds [100,500) W1 {jm[1]['m_100_500']:.4f}, "
      f"W8 {jm[8]['m_100_500']:.4f} (json {rc['W1']['manifolds_mean_100_500']}); points {PJ:.4f}; frame 0: {jm[1]['m_0']:.0f} manifolds, "
      f"{jm[1]['p_0']:.0f} points")
    # our counts over [100,500), per process, from run.csv
    ourM = sorted({round(zone('100..500', 'manifolds', scale=1)(r), 4) for r in chosen
                   if r['row'] in ('H-trk-tree', 'H-trk-ap', 'S-trk-tree-a') and zone('100..500', 'manifolds', scale=1)(r) is not None})
    ptsP4 = sorted({round(zone('100..500', 'phys_np_points', scale=1)(r), 4) for r in chosen if r['row'] == 'S-trk-tree-a'})
    MO = ourM[0] if len(ourM) == 1 else None
    PO = ptsP4[0] if len(ptsP4) == 1 else None
    m0 = sorted({float(r['_mine']['cols']['0..500']['manifolds']['mean']) for r in chosen if r['row'] == 'H-trk-tree'})
    P(f'- ours, per process over [100,500) (every used H-trk-* and S-trk-tree-a process): manifolds {ourM}, points (armed rows) {ptsP4}')
    if MO and PO:
        P(f'  - Jolt/ours: manifolds {MJ / MO:.4f}x, points {PJ / PO:.4f}x; points per manifold ours {PO / MO:.4f}, Jolt {PJ / MJ:.4f}')
    RES['decisions']['counts'] = {'ours_manifolds': MO, 'ours_points': PO, 'jolt_manifolds': MJ, 'jolt_points': PJ}
    BL = {'A': ('P1A-jolt',), 'B': ('P1B-jolt',), 'pooled': ('P1A-jolt', 'P1B-jolt')}
    rowsP1 = (('H-trk-tree', 'trk'), ('H-trk-ap', 'trk'), ('H-jolt56', 'j56'))
    head = {}
    for w in WS:
        P()
        P(f'### W={w}')
        cw = {}
        for row, key in rowsP1:
            for bn, bl in BL.items():
                cw[(row, bn, '0..500')] = C(row, key, w, wall, bl)
                for sub in ('0..100', '100..500'):
                    cw[(row, bn, sub)] = C(row, key, w, zone(sub, 'wall_ns'), bl)
                RES['cells'][f'{row}@W{w} [{bn}] [0,500) ms'] = cw[(row, bn, '0..500')]
            P(f"- {row:<10} [0,500): A {fc(cw[(row, 'A', '0..500')])}; B {fc(cw[(row, 'B', '0..500')])}; "
              f"pooled {fc(cw[(row, 'pooled', '0..500')])}")
        # per pass, to locate the block term
        pp = []
        for row, key in rowsP1:
            vals = []
            for blk in ('P1A-jolt', 'P1B-jolt'):
                for ps in (0, 1):
                    c = C(row, key, w, wall, (blk,), (ps,))
                    vals.append(f"{blk[1:3]}p{ps} {c['median']:.3f}(K{c['K']})" if c else f'{blk[1:3]}p{ps} n/a')
            pp.append(f'{row}: ' + ', '.join(vals))
        P('- per pass medians: ' + ' | '.join(pp))
        for ours in ('H-trk-tree', 'H-trk-ap'):
            res = {}
            for bn in BL:
                for sub in ('0..500', '0..100', '100..500'):
                    c = cmp_(cw[('H-jolt56', bn, sub)], cw[(ours, bn, sub)])
                    RES['cmp'][f'P1 {ours}/jolt56 W{w} [{bn}] [{sub})'] = c
                    res[(bn, sub)] = c
            for sub in ('0..500', '100..500', '0..100'):
                P(f"- {ours}/Jolt [{sub}): A {fcmp(res[('A', sub)])} | B {fcmp(res[('B', sub)])} | pooled {fcmp(res[('pooled', sub)])}")
            ok = [res[(b, '0..500')] for b in ('A', 'B', 'pooled')]
            holds = all(x and x['claimed'] for x in ok) and len({x['ratio'] > 1 for x in ok}) == 1
            v = ('HOLDS: ours ' + ('SLOWER' if ok[2]['ratio'] > 1 else 'FASTER')) if holds else 'does NOT hold'
            RES['decisions'][f'P1 two-block {ours} W{w}'] = {'A': ok[0]['ratio'], 'B': ok[1]['ratio'], 'pooled': ok[2]['ratio'],
                                                            'claimed': [x['claimed'] for x in ok], 'verdict': v}
            P(f"  - **two-block rule [0,500): A {ok[0]['ratio']:.4f} {yn(ok[0])}, B {ok[1]['ratio']:.4f} {yn(ok[1])}, "
              f"pooled {ok[2]['ratio']:.4f} {yn(ok[2])} -> {v}**")
            # per manifold [100,500), each side's own count
            for bn in BL:
                to, tj = cw[(ours, bn, '100..500')], cw[('H-jolt56', bn, '100..500')]
                if to and tj and MO:
                    uo, uj = to['median'] * 1e3 / MO, tj['median'] * 1e3 / MJ
                    RES['decisions'][f'P1 per-manifold {ours} W{w} [{bn}]'] = {'ours_us': uo, 'jolt_us': uj, 'ratio': uo / uj,
                                                                                'ours_ns_per_point': to['median'] * 1e6 / PO,
                                                                                'jolt_ns_per_point': tj['median'] * 1e6 / PJ}
            pm = [RES['decisions'][f'P1 per-manifold {ours} W{w} [{bn}]'] for bn in BL]
            P(f"  - per manifold [100,500) (ours /{MO}, Jolt /{MJ}): "
              + '; '.join(f"{bn} {x['ours_us']:.4f} vs {x['jolt_us']:.4f} us = {x['ratio']:.3f}x" for bn, x in zip(BL, pm))
              + f"; per point pooled {pm[2]['ours_ns_per_point']:.1f} vs {pm[2]['jolt_ns_per_point']:.1f} ns = "
              f"{pm[2]['ours_ns_per_point'] / pm[2]['jolt_ns_per_point']:.3f}x")
        c = cmp_(cw[('H-trk-ap', 'pooled', '0..500')], cw[('H-trk-tree', 'pooled', '0..500')])
        cb = cmp_(cw[('H-trk-ap', 'B', '0..500')], cw[('H-trk-tree', 'B', '0..500')])
        RES['cmp'][f'P1 tree vs allpairs W{w} pooled'] = c
        P(f'- tree vs allpairs [0,500): pooled {fcmp(c)} | B {fcmp(cb)}')
        for row, key in rowsP1:
            cab = cmp_(cw[(row, 'A', '0..500')], cw[(row, 'B', '0..500')])
            RES['cmp'][f'P1 block B vs A {row} W{w}'] = cab
            P(f'- block term B vs A, {row}: {fcmp(cab)}')
        jB, jP = cw[('H-jolt56', 'B', '0..500')], cw[('H-jolt56', 'pooled', '0..500')]
        P(f"- window term (context): Jolt this window / window 3 {WIN3_JOLT56[w]:.4f}: B {jB['median'] / WIN3_JOLT56[w]:.4f}, "
          f"pooled {jP['median'] / WIN3_JOLT56[w]:.4f}")
        head[w] = {(row, bn): cw[(row, bn, '0..500')]['median'] for row, _ in rowsP1 for bn in BL}
        head[w].update({(row, bn, '100'): cw[(row, bn, '100..500')]['median'] for row, _ in rowsP1 for bn in BL})
    P()
    P('### Scaling T(1)/T(W) [0,500) (efficiency T(1)/(W T(W)) in brackets)')
    for bn in BL:
        for row, _ in rowsP1:
            t1 = head[1][(row, bn)]
            P(f'- {bn:<6} {row:<10} ' + ', '.join(f'W{w} {t1 / head[w][(row, bn)]:.3f} ({t1 / (w * head[w][(row, bn)]):.2f})' for w in WS))
    RES['decisions']['P1 scaling'] = {f'{row} {bn}': {w: head[1][(row, bn)] / head[w][(row, bn)] for w in WS}
                                     for bn in BL for row, _ in rowsP1}

    # =========================================================================================== P2
    P()
    P('## 2. P2: L9 C4 G-TW on c4 989ca0f0, --contact-reuse off vs on (on against off)')
    regress = []
    bars_tbl = []
    for sc, wins in (('JA', ('0..500', '0..100', '100..500')), ('JD', ('0..500', '0..100', '100..500')), ('R', ('600..1100',))):
        for w in WS:
            for win in wins:
                fn = wall if win in ('0..500', '600..1100') else zone(win, 'wall_ns')
                a = C(f'L9-{sc}-off', 'c4', w, fn)
                b = C(f'L9-{sc}-on', 'c4', w, fn)
                c = cmp_(a, b)
                RES['cells'][f'L9-{sc}-off@W{w} [{win})'] = a
                RES['cells'][f'L9-{sc}-on@W{w} [{win})'] = b
                RES['cmp'][f'P2 {sc} on vs off W{w} [{win})'] = c
                if c and c['claimed'] and c['ratio'] > 1:
                    regress.append(f'{sc} W{w} [{win})')
                P(f'- {sc} W{w} [{win}): off {fc(a)}; on {fc(b)}; {fcmp(c)}')
                if win in ('0..500', '600..1100'):
                    bars_tbl.append((sc, w, c))
    P()
    P('- G-TW resolution: the smallest on/off effect each cell could claim (the bar = 2*hypot of the two spreads), r / s:')
    for sc in ('JA', 'JD', 'R'):
        P(f'  - {sc}: ' + ', '.join(f"W{w} {100 * c['bar_r']:.1f} / {100 * c['bar_s']:.1f} % (effect {100 * (c['ratio'] - 1):+.1f})"
                                   for s2, w, c in bars_tbl if s2 == sc))
    RES['decisions']['P2 regressions'] = regress
    P(f"- **claimed regressions (on slower, claimed) at any W and window: {regress if regress else 'none'}**")
    jaW1 = RES['cmp']['P2 JA on vs off W1 [100..500)']
    offc, onc = RES['cells']['L9-JA-off@W1 [100..500)'], RES['cells']['L9-JA-on@W1 [100..500)']
    dT = -jaW1['delta']
    worst = offc['min'] - onc['max']
    passed = dT >= L9_BAR and jaW1['claimed'] and jaW1['ratio'] < 1
    RES['decisions']['P2 realized gain'] = {'dT': dT, 'bar': L9_BAR, 'worst_pairing': worst, 'pass': passed,
                                            'x_bar': dT / L9_BAR, 'x_pred_low': dT / L9_PRED[0], 'x_pred_high': dT / L9_PRED[1]}
    P(f"- **realized gain: dT(1)[100,500) on J-A = {offc['median']:.4f} - {onc['median']:.4f} = {dT:.4f} ms "
      f"({100 * (jaW1['ratio'] - 1):+.2f} %, {yn(jaW1)}) against the bar 0.6 x {L9_PRED[0]} = {L9_BAR} ms: "
      f"{'PASS' if passed else 'FAIL'}; {dT / L9_BAR:.2f}x the bar; worst pairing min(off) - max(on) = {worst:+.4f}; "
      f"against the prediction [{L9_PRED[0]}, {L9_PRED[1]}]: {dT / L9_PRED[0]:.3f}x / {dT / L9_PRED[1]:.3f}x**")
    # poses per row across W
    P('- poses (used processes, from SUMMARY): ' + '; '.join(
        f"{rid} {sorted({r['_mine']['summary']['pose_hash'] for r in chosen if r['row'] == rid})} over W "
        f"{sorted({r['W'] for r in chosen if r['row'] == rid})}"
        for rid in ('L9-JA-off', 'L9-JA-on', 'L9-JD-off', 'L9-JD-on', 'L9-R-off', 'L9-R-on', 'L9-JA-a-off', 'L9-JA-a-on', 'L9-JC')))
    # armed table
    P()
    P('### 2.1 The armed per-contact table (J-A, [100,500); reading A = per-process mean, median over K; own counts)')
    stages = (('wall', 'wall_ns'), ('sys_sum', 'sys_sum_ns'), ('bp', 'sys_physics_broadphase_ns'),
              ('np', 'sys_physics_narrowphase_ns'), ('graph', 'sys_physics_build_graph_ns'),
              ('solve (system)', 'sys_physics_solve_colored_ns'), ('  setup = solve_build', 'phys_solve_build_ns'),
              ('  warm_apply', 'phys_warm_apply_ns'), ('  colours wide', 'phys_color_wide_ns'),
              ('  colours narrow', 'phys_color_narrow_ns'), ('  store', 'phys_store_ns'),
              ('  integrate', 'phys_integrate_ns'), ('gather', 'sys_physics_gather_ns'), ('apply', 'sys_physics_apply_ns'))
    for w in (1, 8):
        cnt = {}
        for arm in ('off', 'on'):
            rid = f'L9-JA-a-{arm}'
            cnt[arm] = {k: C(rid, 'c4', w, zone('100..500', k, scale=1)) for k in ('manifolds', 'pairs', 'phys_np_reused', 'phys_np_full', 'phys_np_sep_hits')}
        P(f"- W{w}: manifolds off {cnt['off']['manifolds']['median']:.2f} / on {cnt['on']['manifolds']['median']:.2f}; pairs "
          f"{cnt['off']['pairs']['median']:.2f} / {cnt['on']['pairs']['median']:.2f}; reused on {cnt['on']['phys_np_reused']['median']:.2f}, "
          f"full on {cnt['on']['phys_np_full']['median']:.2f}, sep hits on {cnt['on']['phys_np_sep_hits']['median']:.2f}")
        P('  | stage | off ms | on ms | on vs off | off us/manifold | on us/manifold | off ns/pair | on ns/pair |')
        P('  |---|---|---|---|---|---|---|---|')
        for nm, col in stages:
            a = C('L9-JA-a-off', 'c4', w, zone('100..500', col))
            b = C('L9-JA-a-on', 'c4', w, zone('100..500', col))
            c = cmp_(a, b)
            RES['cells'][f'L9-JA-a-off@W{w} {nm.strip()} ms'] = a
            RES['cells'][f'L9-JA-a-on@W{w} {nm.strip()} ms'] = b
            RES['cmp'][f'P2 armed {nm.strip()} on vs off W{w}'] = c
            mo, mn = cnt['off']['manifolds']['median'], cnt['on']['manifolds']['median']
            po, pn = cnt['off']['pairs']['median'], cnt['on']['pairs']['median']
            P(f"  | {nm} | {a['median']:.4f} | {b['median']:.4f} | {100 * (c['ratio'] - 1):+.2f} % {yn(c)} | "
              f"{a['median'] * 1e3 / mo:.4f} | {b['median'] * 1e3 / mn:.4f} | {a['median'] * 1e6 / po:.1f} | {b['median'] * 1e6 / pn:.1f} |")
    # canary
    P()
    P('### 2.2 The canary (L9-JC against its reference L9-JA-a-on, W1, [0,500) wall)')
    ref = C('L9-JA-a-on', 'c4', 1, wall)
    jc = C('L9-JC', 'c4', 1, wall)
    inj = C('L9-JC', 'c4', 1, lambda r: (r['_mine']['summary'].get('canary_ns') or 0) / 1e6 or None)
    span = C('L9-JC', 'c4', 1, zone('0..500', 'sys_parity_canary_ns'))
    ratios = sorted(zone('0..500', 'sys_parity_canary_ns')(r) / (r['_mine']['summary']['canary_ns'] / 1e6)
                    for r in chosen if r['row'] == 'L9-JC')
    refns = sorted(r.get('canary_ref_ns') for r in chosen if r['row'] == 'L9-JC')
    c = cmp_(ref, jc)
    RES['cmp']['P2 canary rise W1'] = c
    rise = jc['median'] - ref['median']
    P(f'- reference {fc(ref)}; canary row {fc(jc)}; injected {fc(inj)}; span {fc(span)}')
    P(f'- per-process span/injected: {[round(x, 4) for x in ratios]}; canary_ref_ns used: {refns}')
    P(f"- step rise {rise:+.4f} ms = {100 * rise / inj['median']:.1f} % of the injection; {fcmp(c)} -> "
      f"{'SEEN (r and s)' if c['claimed'] else 'NOT SEEN under the two-spread rule'}; SE alone {'seen' if c['cl_s'] else 'not seen'}")
    # resolution: sigma from the two cells' sample sd, the expected r-bar and s-bar at K
    sa, sb = ref['sd_rel'], jc['sd_rel']
    sig = math.hypot(sa, sb)
    P(f"- resolution model: sigma_rel ref {100 * sa:.2f} %, canary row {100 * sb:.2f} %; hypot {100 * sig:.2f} %. "
      f"Expected bars at K: r = 2 d2(K) hypot, s = 2 x 1.2533 hypot / sqrt(K):")
    res_tbl = {}
    for K in (3, 6, 12, 24):
        br, bs = 2 * D2[K] * sig, 2 * 1.2533 * sig / math.sqrt(K)
        res_tbl[K] = (br, bs)
        P(f'  - K={K}: r-bar {100 * br:.2f} %, s-bar {100 * bs:.2f} %')
    obs_eff = c['ratio'] - 1
    need_r = c['bar_r']
    P(f"- the injected effect {100 * inj['median'] / ref['median']:.2f} % of the reference (observed rise {100 * obs_eff:.2f} %); "
      f"the observed r-bar {100 * need_r:.2f} % -> a canary is seen under r at these spreads only above ~{100 * need_r:.1f} % "
      f"of the step (canary_frac >= {need_r * 1.06:.3f} allowing the 6 % over-read seen here), i.e. >= "
      f"{need_r * ref['median']:.2f} ms at W1")
    RES['decisions']['P2 canary'] = {'rise_ms': rise, 'injected_ms': inj['median'], 'span_ms': span['median'],
                                     'span_over_injected': [min(ratios), max(ratios)], 'cmp': c,
                                     'seen_two_spread': c['claimed'], 'seen_se': c['cl_s'],
                                     'resolution': {K: {'r_bar': v[0], 's_bar': v[1]} for K, v in res_tbl.items()}}
    # the ref and canary processes with their witness
    for rid in ('L9-JA-a-on', 'L9-JC'):
        P(f'  - {rid} W1 processes (pass, round, start, wall ms, others_busy %): ' + '; '.join(
            f"p{r['pass']}r{r['round']} {r['start'][11:19]} {wall(r):.3f} {r['others_busy_pct']:.2f}"
            for r in sorted(chosen, key=lambda r: r['start']) if r['row'] == rid and r['W'] == 1))
    # per-pass split of the P2 W1 J-A cells (pass 0 ran with the browser active)
    P('- P2 pass 0 vs pass 1 (the witness differs): ' + '; '.join(
        f"{rid}@W{w} p0 {C(rid, 'c4', w, wall, None, (0,))['median']:.3f} / p1 {C(rid, 'c4', w, wall, None, (1,))['median']:.3f}"
        for rid, w in (('L9-JA-off', 1), ('L9-JA-on', 1), ('L9-JA-a-on', 1), ('L9-JC', 1), ('L9-JA-off', 8), ('L9-JA-on', 8),
                       ('L9-JD-off', 8), ('L9-JD-on', 8))))

    P('- diagnostic, NOT the protocol cell: pass 1 only (04:02-04:20, witness median 0.65 %), K=3 per arm, on vs off, '
      '[0,500) / [600,1100):')
    for sc in ('JA', 'JD', 'R'):
        parts = []
        for w in WS:
            a = C(f'L9-{sc}-off', 'c4', w, wall, None, (1,))
            b = C(f'L9-{sc}-on', 'c4', w, wall, None, (1,))
            c = cmp_(a, b)
            RES['cmp'][f'P2 diag p1 {sc} W{w}'] = c
            parts.append(f"W{w} {100 * (c['ratio'] - 1):+.2f} % ({c['delta']:+.3f} ms; bars {100 * c['bar_r']:.1f}/{100 * c['bar_s']:.1f}; {yn(c)})")
        P(f'  - {sc}: ' + '; '.join(parts))
    a = C('L9-JA-a-on', 'c4', 1, wall, None, (1,))
    b = C('L9-JC', 'c4', 1, wall, None, (1,))
    c = cmp_(a, b)
    RES['cmp']['P2 diag p1 canary'] = c
    P(f"  - canary, pass 1 only: reference {fc(a)}, canary row {fc(b)}; rise {c['delta']:+.4f} ms, {fcmp(c)}")

    # =========================================================================================== P3
    P()
    P('## 3. P3: L11 G9 remainder, J-A at W 2/4/16, tip cbd86a65 against parent 0ca312bd')
    slow = []
    for w in (2, 4, 16):
        a = C('G9-JA-mid', 'g9p', w, wall)
        b = C('G9-JA-mid', 'g9t', w, wall)
        c = cmp_(a, b)
        RES['cells'][f'G9-JA g9p@W{w}'] = a
        RES['cells'][f'G9-JA g9t@W{w}'] = b
        RES['cmp'][f'P3 G9-JA tip vs parent W{w}'] = c
        if c['claimed'] and c['ratio'] > 1:
            slow.append(w)
        P(f'- W{w}: parent {fc(a)}; tip {fc(b)}; tip vs parent {fcmp(c)}')
    RES['decisions']['P3'] = {'claimed_slower_at': slow}
    P(f"- **not claimed slower at any of W 2/4/16: {'yes' if not slow else 'NO: ' + str(slow)}**")

    # =========================================================================================== P4
    P()
    P('## 4. P4: our per-stage spans, trunk 93b2615b default row (--cfg default --broadphase tree, reuse off), [100,500)')
    P('Reading B = per-process median over steps, reading A = per-process mean; per cell the median over K.')
    p4cols = sorted({c for r in chosen if r['row'] == 'S-trk-tree-a' for c in r['_mine']['cols']['100..500'] if c.endswith('_ns')})
    p4 = {}
    for w in (1, 8):
        P(f'### W={w}')
        for col in ['wall_ns', 'sys_sum_ns'] + [c for c in p4cols if c not in ('wall_ns', 'sys_sum_ns')]:
            b_ = C('S-trk-tree-a', 'trk', w, zone('100..500', col, 'median'))
            a_ = C('S-trk-tree-a', 'trk', w, zone('100..500', col, 'mean'))
            p4[(w, col)] = (a_, b_)
            if a_ and a_['median'] > 0.00005:
                P(f"- {col:<32} B {b_['median']:.4f} [{b_['min']:.4f}-{b_['max']:.4f}]  A {a_['median']:.4f} ms  "
                  f"A per manifold {a_['median'] * 1e6 / MO:.1f} ns")
        RES['stages'][f'ours W{w}'] = {col: {'A': v[0]['median'] if v[0] else None, 'B': v[1]['median'] if v[1] else None}
                                       for (ww, col), v in p4.items() if ww == w}

    # =========================================================================================== P5
    P()
    P('## 5. P5: Jolt v5.6.0 per-stage profile (profiled build j56p; frames 100/200/300/400; own parser)')
    P('Stage = the nearest job ancestor, with FindCollidingPairs and NotifyBodiesAABBChanged moved to bp, '
      '"Add Constraint From Cached Manifold" to np_cached, WarmStart* to warm, SortContacts/SplitIsland to solve_prep, '
      'Check Sleeping to integrate. cpu = sum over threads of exclusive time; wall = union of the intervals in which '
      'a thread\'s innermost scope is in the stage.')
    STG = ('bp', 'np_rest', 'np_cached', 'np_collide', 'islands', 'setup', 'warm', 'solve_prep', 'solve_vel',
           'solve_pos', 'integrate', 'other', 'frame')
    p5 = {}
    for w in (1, 8):
        prs = [r for r in chosen if r['row'] == 'P-jolt56-prof' and r['W'] == w]

        def pm(fn):
            return cell([statistics.mean(fn(f) for f in r['_mine']['profile'].values()) for r in prs])
        upd = pm(lambda f: f['update_ms'])
        frame_csv = cell([statistics.mean(r['_mine']['frames'][it] for it in (100, 200, 300, 400)) for r in prs])
        whole = cell([r['_mine']['mean_ms'] for r in prs])
        w100 = cell([r['_mine']['cols']['100..500']['wall_ns']['mean'] / 1e6 for r in prs])
        nsm = pm(lambda f: f['n_samples'])
        P(f'### W={w} (K={len(prs)})')
        P(f"- Update scope {fc(upd)} ms; the harness's Time at the same frames {fc(frame_csv)}; the profiled [100,500) mean "
          f"{fc(w100)}; samples per frame {nsm['median']:.0f}")
        cpu, wl, pt = {}, {}, {}
        for s in STG + ('sched',):
            cpu[s] = pm(lambda f, s_=s: f['stage_cpu'].get(s_, 0.0))
            wl[s] = pm(lambda f, s_=s: f['stage_wall'].get(s_, 0.0))
            pt[s] = pm(lambda f, s_=s: f['stage_part'].get(s_, 0.0))
        tot_cpu = sum(cpu[s]['median'] for s in STG)
        jobs = ('UpdateBroadPhasePrepare', 'FindCollisions', 'ApplyGravity', 'SetupVelocityConstraints',
                'SolveVelocityConstraints', 'SolvePositionConstraints', 'IntegrateVelocity', 'FinalizeIslands')
        jw = {j: pm(lambda f, j_=j: f['job_wall'].get(j_, 0.0)) for j in jobs}
        jc_ = {j: pm(lambda f, j_=j: f['job_cpu'].get(j_, 0.0)) for j in jobs}
        sc = {nm: pm(lambda f, n_=nm: sum(v for k, v in f['scope_cpu'].items() if k.startswith(n_)))
              for nm in ('virtual void JPH::BroadPhaseQuadTree::FindCollidingPairs', 'Add Constraint From Cached Manifold',
                         'static void JPH::ConvexShape::sCollideConvexVsConvex',
                         'bool JPH::ContactConstraintManager::SolveVelocityConstraints',
                         'void JPH::ContactConstraintManager::WarmStartVelocityConstraints',
                         'bool JPH::ContactConstraintManager::SolvePositionConstraints')}
        calls = {nm: pm(lambda f, n_=nm: sum(v for k, v in f['scope_calls'].items() if k.startswith(n_)))
                 for nm in ('Add Constraint From Cached Manifold', 'static void JPH::ConvexShape::sCollideConvexVsConvex',
                            'bool JPH::ContactConstraintManager::SolveVelocityConstraints')}
        P('| stage | cpu ms | share of cpu | wall (union) ms | wall partition ms | share of Update | samples |')
        P('|---|---|---|---|---|---|---|')
        nst = {s: pm(lambda f, s_=s: f['stage_n'].get(s_, 0)) for s in STG}
        for s in STG:
            P(f"| {s} | {cpu[s]['median']:.4f} | {100 * cpu[s]['median'] / tot_cpu:.2f} % | {wl[s]['median']:.4f} | "
              f"{pt[s]['median']:.4f} | {100 * pt[s]['median'] / upd['median']:.2f} % | {nst[s]['median']:.0f} |")
        P(f"| sched (no thread in a stage) | | | | {pt['sched']['median']:.4f} | {100 * pt['sched']['median'] / upd['median']:.2f} % | |")
        P(f'| total (exclusive, all threads) | {tot_cpu:.4f} | | | {sum(pt[s]["median"] for s in STG + ("sched",)):.4f} | | |')
        P('- jobs (inclusive): ' + '; '.join(f"{j} wall {jw[j]['median']:.4f} cpu {jc_[j]['median']:.4f}" for j in jobs))
        P('- scopes (inclusive cpu): ' + '; '.join(f"{k.split('::')[-1][:40]} {v['median']:.4f}" for k, v in sc.items()))
        P('- calls per frame: ' + '; '.join(f"{k.split('::')[-1][:40]} {v['median']:.0f}" for k, v in calls.items()))
        p5[w] = {'update': upd['median'], 'frame_csv': frame_csv['median'], 'w100': w100['median'],
                 'cpu': {s: cpu[s]['median'] for s in STG}, 'wall': {s: wl[s]['median'] for s in STG},
                 'part': {s: pt[s]['median'] for s in STG + ('sched',)},
                 'n': {s: nst[s]['median'] for s in STG}, 'samples': nsm['median'], 'tot_cpu': tot_cpu,
                 'job_wall': {j: jw[j]['median'] for j in jobs}, 'job_cpu': {j: jc_[j]['median'] for j in jobs}}
    RES['p5'] = p5

    # =========================================================================================== stage comparison
    P()
    P('## 6. Stage by stage, per manifold: ours (P4, reading A) against Jolt (P5 shares, scaled to the unprofiled step)')
    jref = {w: {bn: C('H-jolt56', 'j56', w, zone('100..500', 'wall_ns'), BL[bn])['median'] for bn in BL} for w in (1, 8)}
    oref = {w: {bn: C('H-trk-tree', 'trk', w, zone('100..500', 'wall_ns'), BL[bn])['median'] for bn in BL} for w in (1, 8)}
    for w in (1, 8):
        P(f"- W{w}: unprofiled Jolt [100,500): B {jref[w]['B']:.4f}, pooled {jref[w]['pooled']:.4f}; profiled Update "
          f"{p5[w]['update']:.4f} -> profiler overhead {100 * (p5[w]['update'] / jref[w]['B'] - 1):.1f} % (vs B)")
    # reuse-on arithmetic from the P2 armed ratio (same W), applied to P4's np
    np_ratio = {w: RES['cmp'][f'P2 armed np on vs off W{w}']['ratio'] for w in (1, 8)}
    MON = RES['cells']['L9-JA-a-on@W1 wall ms'] and C('L9-JA-a-on', 'c4', 1, zone('100..500', 'manifolds', scale=1))['median']

    def ours(w, cols):
        return sum(p4[(w, c)][0]['median'] for c in cols if p4.get((w, c)) and p4[(w, c)][0])
    OG = {'bp': ['sys_physics_broadphase_ns'],
          'np': ['sys_physics_narrowphase_ns'],
          'setup': ['phys_solve_build_ns'],
          'warm': ['phys_warm_apply_ns'],
          'solve': ['phys_color_wide_ns', 'phys_color_narrow_ns', 'phys_restitution_ns'],
          'islands': ['sys_physics_build_graph_ns'],
          'integrate': ['phys_integrate_ns', 'phys_gravity_ns', 'phys_store_ns', 'phys_write_back_ns'],
          'other': ['sys_physics_gather_ns', 'sys_physics_apply_ns', 'sys_select_broadphase_ns', 'sys_physics_integrate_ns']}
    JG = {'bp': ['bp'], 'np': ['np_rest', 'np_cached', 'np_collide'], 'setup': ['setup'], 'warm': ['warm'],
          'solve': ['solve_prep', 'solve_vel', 'solve_pos'], 'islands': ['islands'], 'integrate': ['integrate'],
          'other': ['other', 'frame']}
    RES['stages']['comparison'] = {}
    JG['other'] = ['other', 'frame', 'sched']
    for w in (1, 8):
        tot_o = ours(w, [c for g in OG.values() for c in g])
        wall_o = p4[(w, 'wall_ns')][0]['median']
        # Jolt: the wall partition of the profiled Update (at W1 it is the exclusive cpu), scaled uniformly to the
        # unprofiled block-B step [100,500) (model U). Model M (W1): part of the overhead is a constant c per recorded
        # sample, the rest uniform; c is capped at the largest value that leaves every stage >= 0.
        part = p5[w]['part']
        psum = sum(part[s] for g in JG.values() for s in g)
        scaleU = jref[w]['B'] / psum
        cmax = None
        if w == 1:
            cmax = min(part[s] / p5[w]['n'][s] for s in part if p5[w]['n'].get(s, 0) >= 500 and s != 'sched')
            Ntot = p5[w]['samples']
            scaleM = jref[w]['B'] / (psum - cmax * Ntot)
        P()
        P(f'### W={w}: ours wall {wall_o:.4f} ms (sum of the stage spans {tot_o:.4f}); Jolt partition sum {psum:.4f} ms '
          f'(Update {p5[w]["update"]:.4f}), model U scale {scaleU:.4f}'
          + (f'; model M: c_max = {1e6 * cmax:.1f} ns/sample (the value that zeroes the tightest high-call stage), '
             f'{cmax * p5[w]["samples"]:.3f} ms of the {psum - jref[w]["B"]:.3f} ms overhead per sample, scale {scaleM:.4f}' if cmax else ''))
        P('| stage | ours ms | ours ns/manifold | ours reuse-on ns/manifold (arith.) | Jolt ms (U) | Jolt ns/manifold (U) '
          + ('| Jolt ns/manifold (M) ' if cmax else '') + '| ours - Jolt (off) | ours - Jolt (on) |')
        P('|---|---|---|---|---|---|' + ('---|' if cmax else '') + '---|---|')
        rows_ = {}
        for g in OG:
            o = ours(w, OG[g])
            o_on = o * np_ratio[w] if g == 'np' else o
            j_raw = sum(part[s] for s in JG[g])
            j = j_raw * scaleU
            jm_ = None
            if cmax:
                nsmp = sum(p5[w]['n'].get(s, 0) for s in JG[g] if s != 'sched')
                jm_ = (j_raw - cmax * nsmp) * scaleM
            rows_[g] = {'ours_ms': o, 'ours_ns_m': o * 1e6 / MO, 'ours_on_ns_m': o_on * 1e6 / MON,
                        'jolt_ms': j, 'jolt_ns_m': j * 1e6 / MJ, 'jolt_ns_m_M': (jm_ * 1e6 / MJ if jm_ is not None else None)}
            x = rows_[g]
            P(f"| {g} | {o:.4f} | {x['ours_ns_m']:.1f} | {x['ours_on_ns_m']:.1f} | {j:.4f} | {x['jolt_ns_m']:.1f} | "
              + (f"{x['jolt_ns_m_M']:.1f} | " if cmax else '')
              + f"{x['ours_ns_m'] - x['jolt_ns_m']:+.1f} | {x['ours_on_ns_m'] - x['jolt_ns_m']:+.1f} |")
        so = sum(x['ours_ns_m'] for x in rows_.values())
        son = sum(x['ours_on_ns_m'] for x in rows_.values())
        sj = sum(x['jolt_ns_m'] for x in rows_.values())
        sjm = sum(x['jolt_ns_m_M'] for x in rows_.values()) if cmax else None
        P(f'| sum | {tot_o:.4f} | {so:.1f} | {son:.1f} | {jref[w]["B"]:.4f} | {sj:.1f} | ' + (f'{sjm:.1f} | ' if cmax else '')
          + f'{so - sj:+.1f} | {son - sj:+.1f} |')
        P(f"- the step per manifold [100,500): block B ours {oref[w]['B'] * 1e6 / MO:.1f} ns, Jolt {jref[w]['B'] * 1e6 / MJ:.1f} ns "
          f"({oref[w]['B'] / MO / (jref[w]['B'] / MJ):.3f}x); pooled ours {oref[w]['pooled'] * 1e6 / MO:.1f}, Jolt "
          f"{jref[w]['pooled'] * 1e6 / MJ:.1f} ({oref[w]['pooled'] / MO / (jref[w]['pooled'] / MJ):.3f}x)")
        comb = ('np', 'setup', 'warm')
        P(f"- np + setup + warm combined (the boundary differs: Jolt builds a contact constraint inside its cached-manifold add): "
          f"ours off {sum(rows_[g]['ours_ns_m'] for g in comb):.1f}, on {sum(rows_[g]['ours_on_ns_m'] for g in comb):.1f}, Jolt U "
          f"{sum(rows_[g]['jolt_ns_m'] for g in comb):.1f}" + (f", M {sum(rows_[g]['jolt_ns_m_M'] for g in comb):.1f}" if cmax else '')
          + ' ns/manifold')
        P(f"- Jolt sub-split (U, ns/manifold): np_rest {part['np_rest'] * scaleU * 1e6 / MJ:.1f}, np_cached {part['np_cached'] * scaleU * 1e6 / MJ:.1f}, "
          f"np_collide {part['np_collide'] * scaleU * 1e6 / MJ:.1f}; solve_prep {part['solve_prep'] * scaleU * 1e6 / MJ:.1f}, "
          f"solve_vel {part['solve_vel'] * scaleU * 1e6 / MJ:.1f}, solve_pos {part['solve_pos'] * scaleU * 1e6 / MJ:.1f}; "
          f"sched {part['sched'] * scaleU * 1e6 / MJ:.1f}")
        P(f"- ours sub-split (ns/manifold): colours wide {p4[(w, 'phys_color_wide_ns')][0]['median'] * 1e6 / MO:.1f} (biased "
          f"{p4[(w, 'phys_pass_biased_ns')][0]['median'] * 1e6 / MO:.1f}, relax {p4[(w, 'phys_pass_relax_ns')][0]['median'] * 1e6 / MO:.1f}); "
          f"bp query {p4[(w, 'phys_bp_query_ns')][0]['median'] * 1e6 / MO:.1f}, assemble {p4[(w, 'phys_bp_assemble_ns')][0]['median'] * 1e6 / MO:.1f}, "
          f"build {p4[(w, 'phys_bp_build_ns')][0]['median'] * 1e6 / MO:.1f}, verify {p4[(w, 'phys_bp_verify_ns')][0]['median'] * 1e6 / MO:.1f}")
        RES['stages']['comparison'][f'W{w}'] = {'rows': rows_, 'scaleU': scaleU, 'cmax_ms': cmax,
                                                'np_ratio_on_off': np_ratio[w], 'manifolds_on': MON,
                                                'sum_ours': so, 'sum_ours_on': son, 'sum_jolt': sj, 'sum_jolt_M': sjm}
        if w == 8:
            P('- Jolt W8 cpu (all threads, exclusive, profiled) per group, ms: ' + '; '.join(
                f"{g} {sum(p5[8]['cpu'].get(s, 0) for s in JG[g]):.4f}" for g in JG))
        # the W scaling of each of our stages (serial stages do not shrink)
    P()
    P('- our stage spans W1 -> W8 (reading A, ms; ratio): ' + '; '.join(
        f"{g} {ours(1, OG[g]):.4f} -> {ours(8, OG[g]):.4f} ({ours(1, OG[g]) / ours(8, OG[g]):.2f}x)" for g in OG))
    P('- Jolt stage partitions W1 -> W8 (profiled, ms; ratio): ' + '; '.join(
        f"{g} {sum(p5[1]['part'].get(s, 0) for s in JG[g]):.4f} -> {sum(p5[8]['part'].get(s, 0) for s in JG[g]):.4f}"
        f" ({sum(p5[1]['part'].get(s, 0) for s in JG[g]) / max(1e-9, sum(p5[8]['part'].get(s, 0) for s in JG[g])):.2f}x)" for g in JG))

    # ------------------------------------------------------------------------------------ driver comparison
    P()
    P("## 7. Against the driver's raw/reduction.json")
    drv = json.load(open(os.path.join(RAW, 'reduction.json'), encoding='utf-8'))
    n_eq, n_ne, diffs = 0, 0, []
    for k, v in drv['cells'].items():
        m = RES['cells'].get(k)
        if not isinstance(v, dict) or m is None:
            continue
        same = v['K'] == m['K'] and abs(v['median'] - m['median']) < 1e-6 * max(1, abs(v['median'])) \
            and abs(v['min'] - m['min']) < 1e-6 * max(1, abs(v['min'])) and abs(v['max'] - m['max']) < 1e-6 * max(1, abs(v['max']))
        n_eq += same
        n_ne += not same
        if not same:
            diffs.append((k, v['K'], v['median'], m['K'], m['median']))
    P(f"- cells compared by key: {n_eq} equal, {n_ne} differ" + (f': {diffs[:8]}' if diffs else ''))
    c_eq, c_ne, cd = 0, 0, []
    for k, v in drv['cmp'].items():
        kk = k.replace('P2 armed setup(solve_build)', 'P2 armed setup = solve_build')
        m = RES['cmp'].get(kk) or RES['cmp'].get(k)
        if not v or not m:
            continue
        same = abs(v['ratio'] - m['ratio']) < 1e-9 and (v['cl_r'], v['cl_i'], v['cl_s']) == (m['cl_r'], m['cl_i'], m['cl_s'])
        c_eq += same
        c_ne += not same
        if not same:
            cd.append(k)
    P(f'- comparisons compared by key: {c_eq} equal (ratio and all three claim flags), {c_ne} differ' + (f': {cd[:8]}' if cd else ''))
    RES['checks']['driver_cells_equal'] = n_eq
    RES['checks']['driver_cells_differ'] = diffs
    RES['checks']['driver_cmp_equal'] = c_eq
    RES['checks']['driver_cmp_differ'] = cd

    with open(os.path.join(OUTD, 'tables.txt'), 'w', encoding='utf-8', newline='\n') as f:
        f.write('\n'.join(OUT) + '\n')

    def clean_json(o):
        if isinstance(o, dict):
            return {str(k): clean_json(v) for k, v in o.items()}
        if isinstance(o, (list, tuple)):
            return [clean_json(v) for v in o]
        return o
    with open(os.path.join(OUTD, 'reduction.json'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(clean_json(RES), f, indent=1, default=str)
    return 0


if __name__ == '__main__':
    sys.exit(main())
