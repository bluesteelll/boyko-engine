"""Window 5 reduction, results-analyst (own script; nothing imported from the tester's reduce5.py).

Inputs (read-only):
  win5/raw/runs.jsonl, every per-process run.csv / pose.bin, win5/raw/makeup/runs_makeup.jsonl,
  win5/rows.json, win5/bin/*, win5/gate/gate.json;
  window 3 raw (Jolt v5.6.0 cells + its manifold receipt), window 4 raw (T-D-allpairs),
  window 4b raw (J-As / J-As-a, parent C0 and tip C2) - for the bridges and the Jolt reference.
Outputs: win5/analyst/reduction.json and win5/analyst/tables.txt (data for the analysis, not a report).

Statistic (the ruled one): a cell = MEDIAN over K processes of the process's [0,500) window mean (ms/step).
Spreads, each as a fraction of the median: min-max range, IQR (inclusive quartiles), SE of the median
(1.2533 * sample SD / sqrt K). B against A: claimed iff |B/A - 1| > 2*hypot(sA, sB); this window's protocol
requires BOTH the min-max and the SE reading ("claimed"), IQR is printed beside them as context.

Selection: records of a voided pass attempt are excluded. Per slot (block, pass, pass_attempt, round, row,
binary, W) the ORIGINAL is used when valid and clean, else its re-run when valid and clean, else the slot
is dropped. "Clean" is recomputed here from the receipts (both 5-s receipts <= 5 % busy, no build/lane
process busy, present or appearing) and cross-checked against the driver's own flags.
"""
import csv
import hashlib
import json
import math
import os
import statistics
import sys

S = r'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad'
W5 = S + '/win5'
W4 = S + '/win4'
W4B = S + '/win4b'
W3 = r'D:/wt/joltab/docs/measurements/2026-09-21-physics-window3'
W3_SCRATCH = S + '/win3'
OUTD = W5 + '/analyst'
BUSY = 5.0
JOLT56_TASK = {1: 9.828, 2: 5.770, 4: 3.581, 8: 2.569, 16: 2.388}
WARM_BAR = -0.19          # G9 realized-gain bar for warm_apply (already 0.6 x the lower predicted delta)
WIN4B_C2_WARM = 0.519     # window 4b, J-As-a W=1 tip (C2), warm_apply
ZONES = ['phys_solve_build_ns', 'phys_gravity_ns', 'phys_warm_apply_ns', 'phys_integrate_ns', 'phys_pass_biased_ns',
         'phys_pass_relax_ns', 'phys_color_wide_ns', 'phys_color_narrow_ns', 'phys_restitution_ns', 'phys_store_ns',
         'phys_write_back_ns', 'phys_sleep_begin_ns', 'phys_sleep_freeze_ns', 'phys_sleep_end_ns',
         'phys_np_dispatch_ns', 'phys_np_compact_ns', 'phys_np_axis_commit_ns',
         'phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns',
         'sys_physics_integrate_ns', 'sys_physics_gather_ns', 'sys_select_broadphase_ns', 'sys_physics_broadphase_ns',
         'sys_physics_narrowphase_ns', 'sys_physics_build_graph_ns', 'sys_physics_solve_colored_ns', 'sys_physics_apply_ns',
         'u_ns', 'g_ns', 'r_ns', 'sys_sum_ns', 'wall_ns']
DERIVED = {
    'setup (build+warm+store)': ['phys_solve_build_ns', 'phys_warm_apply_ns', 'phys_store_ns'],
    'serial-in-solve (build+gravity+warm+integrate+restitution+store+write_back+sleep)':
        ['phys_solve_build_ns', 'phys_gravity_ns', 'phys_warm_apply_ns', 'phys_integrate_ns', 'phys_restitution_ns',
         'phys_store_ns', 'phys_write_back_ns', 'phys_sleep_begin_ns', 'phys_sleep_freeze_ns', 'phys_sleep_end_ns'],
    'kernel (wide+narrow colours)': ['phys_color_wide_ns', 'phys_color_narrow_ns'],
}
COUNTERS = ['manifolds', 'pairs', 'colors', 'wide_colors', 'waves', 'phys_np_points', 'phys_slots_wide', 'phys_slots_narrow']
LINES = []


def P(s=''):
    LINES.append(s)
    print(s)


# ---------------------------------------------------------------- helpers
def jl(path):
    with open(path, encoding='utf-8') as f:
        return [json.loads(l) for l in f if l.strip()]


def remap(p, frm, to):
    if p and not os.path.exists(p):
        q = p.replace('\\', '/').replace(frm.replace('\\', '/'), to)
        if os.path.exists(q):
            return q
    return p


def load_csv(path):
    with open(path, encoding='utf-8', newline='') as f:
        rows = list(csv.reader(f))
    hdr = [h.strip() for h in rows[0]]
    cols = {h: [] for h in hdr}
    for r in rows[1:]:
        for h, v in zip(hdr, r):
            v = v.strip()
            try:
                cols[h].append(float(v) if v != '' else None)
            except ValueError:
                cols[h].append(None)
    return cols


def se_med(xs):
    return 1.2533 * statistics.stdev(xs) / math.sqrt(len(xs)) if len(xs) >= 2 else 0.0


def iqr(xs):
    if len(xs) < 2:
        return 0.0
    q = statistics.quantiles(xs, n=4, method='inclusive')
    return q[2] - q[0]


def cell(xs):
    xs = sorted(xs)
    m = statistics.median(xs)
    d = m if m else float('inf')  # an all-zero zone: relative spreads 0, never claimed
    return {'K': len(xs), 'median': m, 'min': xs[0], 'max': xs[-1], 'range': xs[-1] - xs[0], 'iqr': iqr(xs),
            'se': se_med(xs), 'r': (xs[-1] - xs[0]) / d, 'i': iqr(xs) / d, 's': se_med(xs) / d, 'values': xs}


def cmp_(a, b):
    """B against A: ratio B/A and the three bars 2*hypot(sA, sB)."""
    ratio = b['median'] / a['median']
    e = abs(ratio - 1)
    bars = {k: 2 * math.hypot(a[k], b[k]) for k in ('r', 'i', 's')}
    cl = {k: e > bars[k] for k in bars}
    return {'A': a['median'], 'B': b['median'], 'delta': b['median'] - a['median'], 'ratio': ratio, 'effect': ratio - 1,
            'bar_r': bars['r'], 'bar_i': bars['i'], 'bar_s': bars['s'], 'cl_r': cl['r'], 'cl_i': cl['i'], 'cl_s': cl['s'],
            'claimed': cl['r'] and cl['s'], 'KA': a['K'], 'KB': b['K']}


def yn(c):
    return f"{'Y' if c['cl_r'] else 'n'}/{'Y' if c['cl_i'] else 'n'}/{'Y' if c['cl_s'] else 'n'}"


def fc(c, d=4):
    return f"{c['median']:.{d}f} [{c['min']:.{d}f}-{c['max']:.{d}f}]"


def spreads(c):
    return f"{100 * c['r']:.2f} / {100 * c['i']:.2f} / {100 * c['s']:.2f}"


def fcmp(c):
    return (f"{c['ratio']:.4f} ({100 * c['effect']:+.2f} %; {c['delta']:+.4f} ms; bars r/i/s {100 * c['bar_r']:.2f} / "
            f"{100 * c['bar_i']:.2f} / {100 * c['bar_s']:.2f} %; {yn(c)})")


def sha256(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()


def perf_mean(r):
    v = [p['total'] for p in r.get('perf', []) or [] if p.get('total') is not None]
    return statistics.mean(v) if v else None


def binkey(r):
    return r.get('binary') or r.get('role') or r.get('engine')


def slotkey(r):
    return (r.get('block'), r['pass'], r.get('pass_attempt', 0), r['round'], r['row'], binkey(r), r['W'])


def clean_recomputed(r):
    bad = []
    for k in ('receipt_before', 'receipt_after'):
        rc = r.get(k) or {}
        if rc.get('cpu_avg') is not None and rc['cpu_avg'] > BUSY:
            bad.append(k + '>5')
        if rc.get('build_procs_busy'):
            bad.append(k + ' build busy')
        pr = rc.get('presence') or {}
        if pr.get('build') or pr.get('lane'):
            bad.append(k + ' build/lane present')
    if r.get('build_proc_during'):
        bad.append('build during')
    if r.get('void_names_new_during'):
        bad.append('void name new during')
    return not bad, bad


def select(recs, rowfilter=None, driver_flags=True):
    """Original if valid and clean, else its re-run if valid and clean, else dropped."""
    voided = {(r['block'], r['pass'], r['pass_attempt']) for r in recs if r.get('voided_pass')}
    live = [r for r in recs if 'mean_ms' in r and not r.get('voided_pass')
            and (r.get('block'), r['pass'], r.get('pass_attempt', 0)) not in voided
            and r['attempt'] in ('original', 'rerun') and r.get('timed', True)]
    if rowfilter:
        live = [r for r in live if rowfilter(r)]
    slots = {}
    for r in live:
        slots.setdefault(slotkey(r), {})[r['attempt']] = r
    chosen, dropped, disagree = {}, [], []
    for k, att in slots.items():
        pick = None
        for tag in ('original', 'rerun'):
            r = att.get(tag)
            if not r:
                continue
            ok_mine, _ = clean_recomputed(r)
            ok_drv = not r.get('contaminated')
            if ok_mine != ok_drv:
                disagree.append((k, tag, ok_mine, ok_drv))
            ok = ok_drv if driver_flags else ok_mine
            if r['valid'] and ok:
                pick = r
                break
        if pick is None:
            dropped.append(k)
        else:
            chosen[k] = pick
    return chosen, dropped, disagree, live, voided


def cells_of(chosen, key=lambda r: (r['row'], r['W'], binkey(r)), val=lambda r: r['mean_ms']):
    out = {}
    for r in chosen.values():
        out.setdefault(key(r), []).append(r)
    return {k: (cell([val(r) for r in rs]), rs) for k, rs in out.items()}


def win_mean(r, lo, hi, col='wall_ns', base=None):
    c = load_csv(remap(r['csv'], base[0], base[1]) if base else r['csv'])
    v = [x for x in c[col][lo:hi] if x is not None]
    return statistics.mean(v), c


# ---------------------------------------------------------------- window 5 inputs
recs = jl(W5 + '/raw/runs.jsonl')
rows_json = json.load(open(W5 + '/rows.json', encoding='utf-8'))
ROWS = {r['id']: r for r in rows_json['rows']}
BIN = rows_json['binaries']
R = {'inputs': {}, 'checks': {}, 'cells': {}, 'headline': {}, 'g9': {}, 'spans': {}, 'bridge': {}, 'receipts': {}, 'makeup': {}}

P('# window 5 reduction (results-analyst, analyze_win5.py)')
P()
# binaries
binck = {}
for b, d in BIN.items():
    p = W5 + '/' + d['exe']
    binck[b] = {'file_sha256': sha256(p), 'rows_json_sha256': d['sha256'], 'commit': d['commit']}
sums = open(W5 + '/bin/SHA256SUMS', encoding='utf-8').read()
P('## 0. inputs and checks')
for b, d in binck.items():
    P(f"- binary {b}: file sha256 {d['file_sha256']} == rows.json {d['file_sha256'] == d['rows_json_sha256']}; in SHA256SUMS {d['file_sha256'] in sums}; commit {d['commit']}")
R['checks']['binaries'] = binck

chosen, dropped, disagree, live, voided = select(recs)
chosen_mine, dropped_mine, _, _, _ = select(recs, driver_flags=False)
P(f'- records {len(recs)}; voided pass attempts {sorted(voided)}')
vproc = [r for r in recs if 'mean_ms' in r and (r.get('block'), r['pass'], r.get('pass_attempt', 0)) in voided]
P(f"- processes in voided attempts {len(vproc)} (timed {sum(1 for r in vproc if r.get('timed'))}, warm-up {sum(1 for r in vproc if not r.get('timed'))})")
timed = [r for r in live if r.get('timed')]
P(f"- protocol set: timed {len(timed)} = originals {sum(1 for r in timed if r['attempt'] == 'original')} + re-runs {sum(1 for r in timed if r['attempt'] == 'rerun')}; "
  f"warm-ups {sum(1 for r in recs if 'mean_ms' in r and r['attempt'] == 'warmup' and (r['block'], r['pass'], r['pass_attempt']) not in voided)}")
P(f'- slots {len(chosen) + len(dropped)}: used {len(chosen)}, dropped {len(dropped)}: {sorted(dropped)}')
P(f'- clean flag, driver vs recomputed from receipts: {len(disagree)} disagreements {disagree}')
P(f'- selection by recomputed flags picks the same processes: {set(id(r) for r in chosen.values()) == set(id(r) for r in chosen_mine.values())}, dropped {sorted(dropped_mine) == sorted(dropped)}')
# tester's rule (re-run first) would pick the same? a re-run exists only after a contaminated original
diff_rule = [k for k, r in chosen.items() if r['attempt'] == 'original' and any(
    x['attempt'] == 'rerun' and slotkey(x) == k and x['valid'] and not x['contaminated'] for x in live)]
P(f'- slots where "re-run first" (the tester\'s rule) would differ: {len(diff_rule)}')
R['checks']['selection'] = {'used': len(chosen), 'dropped': [list(k) for k in dropped], 'disagree': disagree, 'rule_diff': len(diff_rule)}

# per-process structural checks over every process with a result (protocol set, voided, warm-ups)
allproc = [r for r in recs if 'mean_ms' in r]
bad = []
maxd_mean, maxd_ns = 0.0, 0.0
for r in allproc:
    row = ROWS[r['row']]
    s = r['summary']
    wm, c = win_mean(r, 0, 500)
    maxd_mean = max(maxd_mean, abs(wm / 1e6 - r['mean_ms']))
    maxd_ns = max(maxd_ns, abs(wm - s['window_mean_ns']))
    probs = []
    if r['exit'] != 0: probs.append('exit')
    if len([x for x in c['wall_ns'][0:500] if x is not None]) != 500: probs.append('steps')
    if s['pose_hash'] != row['expected_pose']: probs.append('pose')
    if s.get('expect_pose') != 'match': probs.append('expect_pose')
    if s.get('void_steps') != 0: probs.append('void')
    if s.get('target_env') != 'msvc': probs.append('env')
    if s.get('workers') != r['W'] or (s.get('threads') or {}).get('pool_workers') != r['W']: probs.append('workers')
    if r['exe_sha256'] != BIN[r['binary']]['sha256'] or r['commit'] != BIN[r['binary']]['commit']: probs.append('binary')
    bp = (s.get('config') or {}).get('broadphase')
    if bp != ('Tree' if row['tree'] else 'AllPairs'): probs.append('broadphase ' + str(bp))
    td = s.get('broadphase_tree') or {}
    exp_td = {'static_rebuilds': 1, 'members': 1} if row['tree'] else {}
    for k, v in td.items():
        if v != exp_td.get(k, 0): probs.append(f'treediag {k}={v}')
    if bool(s.get('armed')) != row['armed']: probs.append('armed')
    if row['armed'] and (s.get('drops_total') not in (0,) or (s.get('threads') or {}).get('solve_on_dispatcher_steps') != 0): probs.append('armed-structure')
    if r.get('mask_readback') != '0xffff': probs.append('mask')
    if s.get('disarmed_ring_traffic') not in (0, None): probs.append('ring')
    if probs:
        bad.append((slotkey(r), r['attempt'], probs))
P(f'- structural checks over all {len(allproc)} processes with a result (protocol + voided + warm-ups): problems {len(bad)} {bad[:5]}')
P(f'- window mean recomputed from run.csv vs driver mean_ms: max |d| {maxd_mean:.3g} ms; vs runner window_mean_ns: max |d| {maxd_ns:.3g} ns')
# config parity parent vs tip
par = {}
for r in allproc:
    par.setdefault((r['row'], r['W'], r['binary']), set()).add(json.dumps(r['summary']['config'], sort_keys=True))
pairs = [(row, w) for (row, w, b) in par if b == 'parent']
eq = sum(1 for row, w in pairs if par.get((row, w, 'tip')) == par[(row, w, 'parent')] and len(par[(row, w, 'parent')]) == 1)
P(f'- printed config parent == tip (one config per cell): {eq}/{len(pairs)} (row, W) pairs')
cfg_of = {}
for r in allproc:
    cfg_of.setdefault((r['row'], r['W']), r['summary']['config'])
for k in sorted(cfg_of, key=str):
    c = cfg_of[k]
    P(f"  cfg {k}: bp {c['broadphase']} select {c['broadphase_select']} simd_solve {c['simd_solve']} par_solve {c['parallel_solve']} "
      f"par_bp {c['parallel_broadphase']} par_np {c['parallel_narrowphase']} sleeping {c['sleeping']} substeps {c['substeps']} relax {c['relax_iterations']}")
R['checks']['structural_problems'] = bad
R['checks']['max_abs_d_mean_ms'] = maxd_mean

# makeup
mk = jl(W5 + '/raw/makeup/runs_makeup.jsonl')
mk_t = [r for r in mk if r.get('timed') and 'mean_ms' in r]

# ---------------------------------------------------------------- cells
C = cells_of(chosen)
order = ['HL-D-tree', 'HL-D-allpairs', 'HL-R-tree', 'HL-R-allpairs', 'J-As', 'J-As-a']
P()
P('## 1. cells (ms/step; median [min-max]; spreads r / i / s % of median; per pass p0 / p1; perf % med; others % med/max)')
P()
P('| row | W | bin | K | median [min-max] | r / i / s % | p0 / p1 | values | perf % | others % |')
P('|---|---|---|---|---|---|---|---|---|---|')
for (row, w, b) in sorted(C, key=lambda k: (order.index(k[0]), k[1], k[2])):
    c, rs = C[(row, w, b)]
    pp = []
    for p_ in (0, 1):
        v = [r['mean_ms'] for r in rs if r['pass'] == p_]
        pp.append(f'{statistics.median(v):.4f}' if v else '-')
    perf = statistics.median([perf_mean(r) for r in rs if perf_mean(r) is not None])
    ob = [r.get('others_busy_pct') or 0 for r in rs]
    P(f"| {row} | {w} | {b} | {c['K']} | {fc(c)} | {spreads(c)} | {' / '.join(pp)} | {', '.join(f'{x:.3f}' for x in c['values'])} | {perf:.1f} | {statistics.median(ob):.2f}/{max(ob):.2f} |")
    R['cells'][f'{row}@W{w}#{b}'] = dict(c, perf=perf, p0p1=pp)

# ---------------------------------------------------------------- Jolt v5.6.0 (window 3, not re-run)
w3 = jl(W3 + '/raw/runs.jsonl')
jch, jdrop, _, _, _ = select(w3, rowfilter=lambda r: r['row'] == 'JOLT56-T')
JC = cells_of(jch, key=lambda r: r['W'])
j_frame_d = 0.0
jolt_sub = {}
for w, (c, rs) in JC.items():
    subs = []
    for r in rs:
        pf = remap(r['per_frame'], W3_SCRATCH, W3)
        cc = load_csv(pf)
        col = [k for k in cc if k.lower().startswith('time')][0]
        v = cc[col]
        j_frame_d = max(j_frame_d, abs(statistics.mean(v[0:500]) - r['mean_ms']))
        subs.append(statistics.mean(v[100:500]))
    jolt_sub[w] = cell(subs)
rc = load_csv(W3 + '/raw/receipts/JOLT56-T_receipt_W1/receipt_discrete_th1.csv')
mcol = [k for k in rc if k.lower().startswith('manifold')][0]
JOLT_M = statistics.mean(rc[mcol][100:500])
P()
P(f'## 2. Jolt v5.6.0, window 3 (recomputed from its raw; K per W {[JC[w][0]["K"] for w in sorted(JC)]}; dropped {jdrop}); per-frame mean == mean_ms max |d| {j_frame_d:.3g} ms; manifolds/step [100,500) {JOLT_M:.4f}')
for w in sorted(JC):
    c = JC[w][0]
    P(f"- W{w}: {fc(c, 3)} ({spreads(c)}); task value {JOLT56_TASK[w]} equal: {abs(c['median'] - JOLT56_TASK[w]) < 5e-4}; [100,500) {jolt_sub[w]['median']:.4f}")
R['jolt56'] = {w: JC[w][0] for w in JC}
R['jolt56_sub'] = jolt_sub
R['jolt_manifolds'] = JOLT_M

# ---------------------------------------------------------------- headline
P()
P('## 3. headline (tip): tree vs allpairs, each vs Jolt v5.6.0; claimed = min-max AND SE')
P()
P('| scene | W | allpairs | tree | tree/allpairs | tree/Jolt | allpairs/Jolt |')
P('|---|---|---|---|---|---|---|')
for scene, pre, ws in (('J', 'HL-D', (1, 2, 4, 8)), ('R', 'HL-R', (1, 8))):
    for w in ws:
        a = C[(f'{pre}-allpairs', w, 'tip')][0]
        t = C[(f'{pre}-tree', w, 'tip')][0]
        e = {'tree_vs_allpairs': cmp_(a, t)}
        if scene == 'J':
            j = JC[w][0]
            e['tree_vs_jolt'] = cmp_(j, t)
            e['allpairs_vs_jolt'] = cmp_(j, a)
        R['headline'][f'{scene}@W{w}'] = e
        P(f"| {scene} | {w} | {fc(a)} | {fc(t)} | {fcmp(e['tree_vs_allpairs'])} | "
          f"{fcmp(e['tree_vs_jolt']) if scene == 'J' else '-'} | {fcmp(e['allpairs_vs_jolt']) if scene == 'J' else '-'} |")

# per manifold [100,500) and sub-window ratios
P()
P('per manifold per step over [100,500) (boyko: per process mean wall / mean manifolds over [100,500), median over K; Jolt: per process [100,500) mean / receipted manifolds):')
pm = {}
for row in ('HL-D-tree', 'HL-D-allpairs'):
    for w in (1, 2, 4, 8):
        rs = C[(row, w, 'tip')][1]
        us, sub, man = [], [], []
        for r in rs:
            cc = load_csv(r['csv'])
            wv = statistics.mean(cc['wall_ns'][100:500]) / 1e6
            mv = statistics.mean(cc['manifolds'][100:500])
            us.append(wv * 1e3 / mv)
            sub.append(wv)
            man.append(mv)
        pm[(row, w)] = {'us_per_manifold': cell(us), 'sub_100_500': cell(sub), 'manifolds': sorted(set(round(m, 4) for m in man))}
for w in (1, 2, 4, 8):
    j_us = jolt_sub[w]['median'] * 1e3 / JOLT_M
    t = pm[('HL-D-tree', w)]
    a = pm[('HL-D-allpairs', w)]
    P(f"- W{w}: tree {t['us_per_manifold']['median']:.4f} us (manifolds {t['manifolds']}), allpairs {a['us_per_manifold']['median']:.4f} us, Jolt {j_us:.4f} us "
      f"-> tree/Jolt per manifold {t['us_per_manifold']['median'] / j_us:.3f}, allpairs/Jolt {a['us_per_manifold']['median'] / j_us:.3f}; "
      f"[100,500) step ratio tree/Jolt {t['sub_100_500']['median'] / jolt_sub[w]['median']:.4f}, allpairs/Jolt {a['sub_100_500']['median'] / jolt_sub[w]['median']:.4f}")
    R['headline'][f'J@W{w}']['per_manifold'] = {'tree_us': t['us_per_manifold']['median'], 'allpairs_us': a['us_per_manifold']['median'], 'jolt_us': j_us}
# scaling
P()
for row in ('HL-D-tree', 'HL-D-allpairs'):
    t1 = C[(row, 1, 'tip')][0]['median']
    P(f"- scaling {row}: T(1)/T(W) = " + ', '.join(f"W{w} {t1 / C[(row, w, 'tip')][0]['median']:.3f}" for w in (2, 4, 8)))
j1 = JC[1][0]['median']
P('- scaling Jolt v5.6.0: T(1)/T(W) = ' + ', '.join(f"W{w} {j1 / JC[w][0]['median']:.3f}" for w in (2, 4, 8)))
for row in ('HL-R-tree', 'HL-R-allpairs'):
    P(f"- scaling {row}: T(1)/T(8) = {C[(row, 1, 'tip')][0]['median'] / C[(row, 8, 'tip')][0]['median']:.3f}")
# in-window, same binary, W=1 same code path: HL-D-allpairs vs J-As tip
for w in (1, 8):
    e = cmp_(C[('J-As', w, 'tip')][0], C[('HL-D-allpairs', w, 'tip')][0])
    R['headline'][f'default_vs_as@W{w}'] = e
    P(f'- same binary (tip), HL-D-allpairs against J-As at W{w} (W=1: same code path; W=8: cfg as adds parallel_broadphase): {fcmp(e)}')

# ---------------------------------------------------------------- G9: T cells
P()
P('## 4. G9-on-C3: tip against parent')
P()
for row in ('J-As', 'J-As-a'):
    for w in (1, 8):
        p = C[(row, w, 'parent')][0]
        t = C[(row, w, 'tip')][0]
        e = cmp_(p, t)
        R['g9'][f'{row}@W{w}'] = e
        # worst pairing
        P(f"- {row} W{w}: parent {fc(p)} ({spreads(p)}) -> tip {fc(t)} ({spreads(t)}): {fcmp(e)}; worst pairing (tip max - parent min) {t['max'] - p['min']:+.4f} ms; K {p['K']}/{t['K']}")

# ---------------------------------------------------------------- spans


def span_table(rs, reading):
    """reading A: per-process mean over [0,500); reading B: per-process median over [100,500)."""
    per = []
    for r in rs:
        cc = load_csv(r['csv']) if isinstance(r, dict) and 'csv' in r else r
        d = {}
        for z in ZONES + COUNTERS:
            if z in cc:
                v = [x for x in (cc[z][0:500] if reading == 'A' else cc[z][100:500]) if x is not None]
                if v:
                    d[z] = statistics.mean(v) if reading == 'A' else statistics.median(v)
        for name, parts in DERIVED.items():
            if all(p_ in cc for p_ in parts if not p_.startswith('phys_sleep')):
                steps = range(0, 500) if reading == 'A' else range(100, 500)
                vals = [sum((cc[p_][i] or 0) for p_ in parts if p_ in cc) for i in steps]
                d[name] = statistics.mean(vals) if reading == 'A' else statistics.median(vals)
        per.append(d)
    agg = {}
    keys = set().union(*[set(d) for d in per])
    for z in keys:
        v = [d[z] for d in per if z in d]
        if len(v) == len(per):
            agg[z] = cell(v)
    return agg


def span_cmp(pa, ta, keys):
    out = {}
    for z in keys:
        if z in pa and z in ta:
            p, t = pa[z], ta[z]
            if p['median'] == 0 and t['median'] == 0:
                continue
            d = {'parent': p['median'] / 1e6 if z.endswith('_ns') or z in DERIVED else p['median'],
                 'tip': t['median'] / 1e6 if z.endswith('_ns') or z in DERIVED else t['median'],
                 'pmin': p['min'] / 1e6, 'pmax': p['max'] / 1e6, 'tmin': t['min'] / 1e6, 'tmax': t['max'] / 1e6,
                 'pse': p['se'] / 1e6, 'tse': t['se'] / 1e6}
            d['delta'] = d['tip'] - d['parent']
            d['two_se'] = 2 * math.hypot(d['pse'], d['tse'])
            if p['median'] > 0:
                c = cmp_(p, t)
                d.update({'ratio': c['ratio'], 'bar_r': c['bar_r'], 'bar_s': c['bar_s'], 'bar_i': c['bar_i'], 'yn': yn(c), 'claimed': c['claimed']})
            out[z] = d
    return out


for reading in ('B', 'A'):
    for w in (1, 8):
        pa = span_table(C[('J-As-a', w, 'parent')][1], reading)
        ta = span_table(C[('J-As-a', w, 'tip')][1], reading)
        keys = ZONES + list(DERIVED)
        R['spans'][f'{reading}@W{w}'] = span_cmp(pa, ta, keys)
        R['spans'][f'{reading}@W{w}_counters'] = {z: (pa[z]['median'], ta[z]['median']) for z in COUNTERS if z in pa and z in ta}
        if reading == 'B':
            R['spans'][f'B@W{w}_raw_parent'] = pa
            R['spans'][f'B@W{w}_raw_tip'] = ta

P()
P('### armed spans J-As-a (ms); reading B = per process median over [100,500), then median over K; reading A = per process mean over [0,500)')
for reading in ('B', 'A'):
    for w in (1, 8):
        P()
        P(f'#### reading {reading}, W={w}')
        P('| zone | parent [min-max] | tip [min-max] | delta | ratio | 2*hypot(SE) ms | bars r / s % | r/i/s |')
        P('|---|---|---|---|---|---|---|---|')
        for z, d in R['spans'][f'{reading}@W{w}'].items():
            P(f"| {z} | {d['parent']:.4f} [{d['pmin']:.4f}-{d['pmax']:.4f}] | {d['tip']:.4f} [{d['tmin']:.4f}-{d['tmax']:.4f}] | {d['delta']:+.4f} | "
              f"{d.get('ratio', float('nan')):.4f} | {d['two_se']:.4f} | {100 * d.get('bar_r', 0):.2f} / {100 * d.get('bar_s', 0):.2f} | {d.get('yn', '-')} |")
        P('counters (parent / tip): ' + '; '.join(f'{z} {v[0]:.2f}/{v[1]:.2f}' for z, v in R['spans'][f'{reading}@W{w}_counters'].items()))

# warm_apply gate
P()
P('### the warm_apply gate (G9: realized gain <= -0.19 ms, C3, J-As; armed row, W=1)')
for reading in ('B', 'A'):
    for w in (1, 8):
        d = R['spans'][f'{reading}@W{w}']['phys_warm_apply_ns']
        worst = d['tmax'] - d['pmin']
        verdict = 'PASS' if d['delta'] <= WARM_BAR and d['claimed'] else ('NOT CLAIMED' if not d['claimed'] else 'FAIL')
        P(f"- reading {reading} W{w}: parent {d['parent']:.4f} -> tip {d['tip']:.4f}: delta {d['delta']:+.4f} ms (bar {WARM_BAR}); margin {WARM_BAR - d['delta']:+.4f}; "
          f"2*hypot(SE) {d['two_se']:.4f}; bars r/s {100 * d['bar_r']:.2f}/{100 * d['bar_s']:.2f} % vs effect {100 * (d['ratio'] - 1):+.2f} % ({d['yn']}); "
          f"worst pairing {worst:+.4f}; tip - 0.519 (win4b C2) {d['tip'] - WIN4B_C2_WARM:+.4f}; delta / lower predicted (-0.314) {d['delta'] / -0.314:.3f}; verdict {verdict}")
        R['g9'][f'warm_apply_{reading}@W{w}'] = dict(d, worst=worst, verdict=verdict)

# ---------------------------------------------------------------- bridges
P()
P('## 5. bridges')
w4 = jl(W4 + '/raw/runs.jsonl')
w4ch, w4drop, _, _, _ = select(w4, rowfilter=lambda r: r['row'] == 'T-D-allpairs')
W4C = cells_of(w4ch, key=lambda r: r['W'])
w4b = jl(W4B + '/raw/runs.jsonl')
w4bch, w4bdrop, _, _, _ = select(w4b, rowfilter=lambda r: r['row'] in ('J-As', 'J-As-a'))
W4BC = cells_of(w4bch)
P(f'- window 4 T-D-allpairs (binary {sorted(set(r["exe_sha256"][:8] for r in w4ch.values()))}): ' + '; '.join(f'W{w} {fc(W4C[w][0], 3)} K={W4C[w][0]["K"]} ({spreads(W4C[w][0])})' for w in sorted(W4C)) + f'; dropped {w4drop}')
for k in sorted(W4BC, key=str):
    c = W4BC[k][0]
    P(f"- window 4b {k}: {fc(c, 3)} K={c['K']} ({spreads(c)}); commit {sorted(set(r['commit'][:8] for r in W4BC[k][1]))}")
P(f'- window 4b dropped {w4bdrop}')

# bridge 1: literal
for w in (1, 8):
    a4 = W4C[w][0]
    t5 = C[('HL-D-allpairs', w, 'tip')][0]
    e = cmp_(a4, t5)
    R['bridge'][f'b1_literal@W{w}'] = e
    P(f'- bridge 1 literal, window 5 tip HL-D-allpairs against window 4 T-D-allpairs, W{w}: {fcmp(e)}')
# bridge 1: chain with the in-window lever deltas (4b: C0 -> C2 on J-As; 5: C2+line -> C3 on J-As)
for w in (1, 8):
    parts = {'w4_ap': W4C[w][0], 'w4b_par': W4BC[('J-As', w, 'parent')][0], 'w4b_tip': W4BC[('J-As', w, 'tip')][0],
             'w5_par': C[('J-As', w, 'parent')][0], 'w5_tip': C[('J-As', w, 'tip')][0], 'w5_ap': C[('HL-D-allpairs', w, 'tip')][0]}
    pred = parts['w4_ap']['median'] + (parts['w4b_tip']['median'] - parts['w4b_par']['median']) + (parts['w5_tip']['median'] - parts['w5_par']['median'])
    resid = parts['w5_ap']['median'] - pred
    bar_s = 2 * math.sqrt(sum(p_['se'] ** 2 for p_ in parts.values()))
    bar_r = 2 * math.sqrt(sum(p_['range'] ** 2 for p_ in parts.values()))
    R['bridge'][f'b1_chain@W{w}'] = {'pred': pred, 'meas': parts['w5_ap']['median'], 'resid': resid, 'bar_s': bar_s, 'bar_r': bar_r}
    P(f"- bridge 1 chain W{w}: predicted {pred:.4f} = {parts['w4_ap']['median']:.4f} + ({parts['w4b_tip']['median']:.4f} - {parts['w4b_par']['median']:.4f}) + "
      f"({parts['w5_tip']['median']:.4f} - {parts['w5_par']['median']:.4f}); measured {parts['w5_ap']['median']:.4f}; residual {resid:+.4f} ms ({100 * resid / pred:+.2f} %); "
      f"abs bars 2*sqrt(sum SE^2) {bar_s:.4f} / 2*sqrt(sum range^2) {bar_r:.4f}")
# bridge 2: literal (window 5 parent J-As against window 4b tip J-As), and the armed twin
for row in ('J-As', 'J-As-a'):
    for w in (1, 8):
        b4 = W4BC[(row, w, 'tip')][0]
        p5 = C[(row, w, 'parent')][0]
        e = cmp_(b4, p5)
        R['bridge'][f'b2_{row}@W{w}'] = e
        P(f'- bridge 2 {row} W{w}: window 5 parent {fc(p5)} against window 4b tip {fc(b4)} (K {b4["K"]}/{p5["K"]}): {fcmp(e)}; '
          f'4b tip per pass {[round(statistics.median([r["mean_ms"] for r in W4BC[(row, w, "tip")][1] if r["pass"] == q]), 4) for q in (0, 1)]}')
# perf % and witness for the bridge-2 cells
for row in ('J-As', 'J-As-a'):
    for w in (1, 8):
        rs4 = W4BC[(row, w, 'tip')][1]
        rs5 = C[(row, w, 'parent')][1]
        rs5t = C[(row, w, 'tip')][1]
        f = lambda rs: statistics.median([perf_mean(r) for r in rs if perf_mean(r) is not None])
        g = lambda rs: statistics.median([r.get('others_busy_pct') or 0 for r in rs])
        P(f'  perf % median: 4b tip {f(rs4):.1f}, w5 parent {f(rs5):.1f}, w5 tip {f(rs5t):.1f}; others busy % median: {g(rs4):.2f} / {g(rs5):.2f} / {g(rs5t):.2f}')
        R['bridge'][f'b2_perf_{row}@W{w}'] = [f(rs4), f(rs5), f(rs5t)]
# bridge 2 at the stage level: 4b tip armed (reading B) against window 5 parent armed
P()
P('bridge 2 per stage, armed J-As-a, reading B (window 4b tip C2 -> window 5 parent C2+line); ms')
for w in (1, 8):
    b4 = span_table(W4BC[('J-As-a', w, 'tip')][1], 'B')
    p5 = R['spans'][f'B@W{w}_raw_parent']
    sc = span_cmp(b4, p5, ['wall_ns', 'sys_physics_broadphase_ns', 'sys_physics_narrowphase_ns', 'sys_physics_solve_colored_ns',
                           'phys_solve_build_ns', 'phys_warm_apply_ns', 'phys_store_ns', 'phys_color_wide_ns', 'phys_pass_biased_ns',
                           'phys_pass_relax_ns', 'phys_integrate_ns', 'sys_physics_gather_ns', 'sys_physics_build_graph_ns', 'g_ns',
                           'setup (build+warm+store)'])
    R['bridge'][f'b2_stages@W{w}'] = sc
    P(f'W{w}: ' + '; '.join(f"{z.replace('_ns', '')} {d['parent']:.4f}->{d['tip']:.4f} ({100 * (d['ratio'] - 1):+.1f} %, {d['yn']})" for z, d in sc.items()))

# ---------------------------------------------------------------- receipts
P()
P('## 6. receipts (protocol set, timed)')
rb = [r['receipt_before']['cpu_avg'] for r in timed]
ra = [r['receipt_after']['cpu_avg'] for r in timed]
ob = [r.get('others_busy_pct') or 0 for r in timed]
used = list(chosen.values())
obu = [r.get('others_busy_pct') or 0 for r in used]
q = lambda xs, p_: statistics.quantiles(xs, n=100, method='inclusive')[p_ - 1]
P(f'- before: median {statistics.median(rb):.2f} %, max {max(rb):.2f}, >5: {sum(x > 5 for x in rb)}; after: median {statistics.median(ra):.2f}, p90 {q(ra, 90):.2f}, max {max(ra):.2f}, >5: {sum(x > 5 for x in ra)}')
P(f'- witness others_busy_pct, all timed: median {statistics.median(ob):.2f}, p95 {q(ob, 95):.2f}, max {max(ob):.2f}, >5: {sum(x > 5 for x in ob)}; used: median {statistics.median(obu):.2f}, p95 {q(obu, 95):.2f}, max {max(obu):.2f}, >5: {sum(x > 5 for x in obu)}')
hi = sorted([(r.get('others_busy_pct'), slotkey(r), r['mean_ms']) for r in used if (r.get('others_busy_pct') or 0) > 5], key=str)
P(f'- used processes with witness > 5 %: {hi}')
R['receipts'] = {'before_med': statistics.median(rb), 'after_med': statistics.median(ra), 'after_max': max(ra), 'n_after_over5': sum(x > 5 for x in ra),
                 'wit_used_med': statistics.median(obu), 'wit_used_max': max(obu), 'wit_used_over5': len(hi)}
# per-block time spans
for blk in ('headline', 'g9', 'armed'):
    ts = sorted(r['start'] for r in used if r['block'] == blk)
    P(f'- block {blk}: used processes {len(ts)}, first {ts[0]}, last {ts[-1]}')

# ---------------------------------------------------------------- make-up sensitivity
P()
P('## 7. make-up sensitivity (dropped slots filled by the make-up processes; not the protocol set)')
filled = dict(chosen)
for k in dropped:
    m = [r for r in mk_t if (r['row'], r['binary'], r['W']) == (k[4], k[5], k[6]) and r['valid'] and not r['contaminated']]
    if m:
        filled[k] = m[0]
        P(f"- slot {k}: make-up {m[0]['mean_ms']:.4f} ms (receipts {m[0]['receipt_before']['cpu_avg']} / {m[0]['receipt_after']['cpu_avg']} %, witness {m[0].get('others_busy_pct')})")
CM = cells_of(filled)
for key in [('HL-D-tree', 1, 'tip'), ('HL-D-allpairs', 2, 'tip'), ('J-As', 8, 'tip')]:
    P(f'  cell {key}: {fc(CM[key][0])} K={CM[key][0]["K"]} ({spreads(CM[key][0])})')
for w in (1, 2):
    a = CM[('HL-D-allpairs', w, 'tip')][0]; t = CM[('HL-D-tree', w, 'tip')][0]; j = JC[w][0]
    P(f'  J W{w}: tree/allpairs {fcmp(cmp_(a, t))}; tree/Jolt {fcmp(cmp_(j, t))}; allpairs/Jolt {fcmp(cmp_(j, a))}')
e = cmp_(CM[('J-As', 8, 'parent')][0], CM[('J-As', 8, 'tip')][0])
P(f'  G9 J-As W8 with make-up: {fcmp(e)}')
R['makeup'] = {'J-As@W8': e}

# ---------------------------------------------------------------- extra: sensitivity, chain link, tree budget
P()
P('## 8. sensitivity (not the ruled statistic) and the tree-row budget')


def drop_max(c):
    return cell(c['values'][:-1])


for w in (1, 8):
    j = JC[w][0]
    jd = drop_max(j)
    t = C[('HL-D-tree', w, 'tip')][0]
    a = C[('HL-D-allpairs', w, 'tip')][0]
    P(f"- Jolt W{w} without its max process ({j['max']:.3f}): {fc(jd, 3)} ({spreads(jd)}); tree/Jolt {fcmp(cmp_(jd, t))}; allpairs/Jolt {fcmp(cmp_(jd, a))}")
    R['headline'][f'J@W{w}']['sens_jolt_dropmax'] = {'tree': cmp_(jd, t), 'allpairs': cmp_(jd, a)}
t2 = C[('HL-D-tree', 2, 'tip')][0]
P(f"- HL-D-tree W2 without its max ({t2['max']:.4f}): {fc(drop_max(t2))}; tree/Jolt {fcmp(cmp_(JC[2][0], drop_max(t2)))}; tree/allpairs {fcmp(cmp_(C[('HL-D-allpairs', 2, 'tip')][0], drop_max(t2)))}")
t8 = C[('HL-D-tree', 8, 'tip')][0]
P(f"- HL-D-tree W8 per pass: {R['cells']['HL-D-tree@W8#tip']['p0p1']}; tree/Jolt with each pass's median: " +
  ', '.join(f'{float(x) / JC[8][0]["median"]:.4f}' for x in R['cells']['HL-D-tree@W8#tip']['p0p1']))
# the chain link window 4 -> window 4b parent (A1b + C0 + two windows), W1 (same code path at W=1)
e = cmp_(W4C[1][0], W4BC[('J-As', 1, 'parent')][0])
P(f'- chain link W1: window 4b parent J-As (C0 + A1b) against window 4 T-D-allpairs: {fcmp(e)}')
R['bridge']['link_w4_w4bpar@W1'] = e
# window 4 tree span, recomputed (T-A-tree-armed, cfg-A; the tree span does not depend on simd_solve)
w4t, _, _, _, _ = select(w4, rowfilter=lambda r: r['row'] in ('T-A-tree-armed', 'T-A-allpairs-armed'))
W4T = cells_of(w4t)
tree_span = {}
for w in (1, 8):
    rs = W4T[('T-A-tree-armed', w, 'c3')][1]
    per_sig, per_q, per_sys = [], [], []
    for r in rs:
        cc = load_csv(remap(r['csv'], 'x', 'x'))
        sig = [sum(cc[z][i] or 0 for z in ('phys_bp_verify_ns', 'phys_bp_build_ns', 'phys_bp_query_ns', 'phys_bp_assemble_ns')) for i in range(100, 500)]
        per_sig.append(statistics.median(sig))
        per_q.append(statistics.median(cc['phys_bp_query_ns'][100:500]))
        per_sys.append(statistics.median(cc['sys_physics_broadphase_ns'][100:500]))
    tree_span[w] = {'sigma': cell(per_sig), 'query': cell(per_q), 'sys': cell(per_sys)}
    q = tree_span[w]['query']['median']
    P(f"- window 4 tree span (recomputed, reading B) W{w}: sigma {tree_span[w]['sigma']['median'] / 1e6:.4f} ms, query {q / 1e6:.4f} ms = {q / 1240:.1f} ns/row (1240 rows), "
      f"sys_physics_broadphase {tree_span[w]['sys']['median'] / 1e6:.4f}; design c_q 100-150 ns/row -> t_q {0.124:.3f}-{0.186:.3f} ms, excess {q / 1e6 - 0.186:.3f}-{q / 1e6 - 0.124:.3f} ms")
R['tree_span_w4'] = tree_span
# tree-row budget: tip J-As-a spans (reading B) with the AllPairs broadphase span replaced by window 4's tree broadphase span
for w in (1, 8):
    ta = R['spans'][f'B@W{w}_raw_tip']
    pred = (ta['wall_ns']['median'] - ta['sys_physics_broadphase_ns']['median'] + tree_span[w]['sys']['median']) / 1e6
    rs = C[('HL-D-tree', w, 'tip')][1]
    meas = statistics.median([statistics.median(load_csv(r['csv'])['wall_ns'][100:500]) for r in rs]) / 1e6
    z = lambda k: ta[k]['median'] / 1e6
    ser = z('serial-in-solve (build+gravity+warm+integrate+restitution+store+write_back+sleep)')
    wind = tree_span[w]['sys']['median'] / 1e6 + ser + z('sys_physics_build_graph_ns') + z('sys_physics_gather_ns') + z('sys_physics_apply_ns')
    P(f"- tree-row budget W{w} (ms; tip J-As-a reading B, AllPairs span {z('sys_physics_broadphase_ns'):.4f} swapped for the tree's {tree_span[w]['sys']['median'] / 1e6:.4f}): "
      f"predicted step {pred:.4f}, measured HL-D-tree median-over-steps [100,500) {meas:.4f} ({100 * (meas / pred - 1):+.1f} %); "
      f"narrowphase {z('sys_physics_narrowphase_ns'):.4f}, solve_colored {z('sys_physics_solve_colored_ns'):.4f} (serial-in-solve {ser:.4f}: build {z('phys_solve_build_ns'):.4f}, warm {z('phys_warm_apply_ns'):.4f}, store {z('phys_store_ns'):.4f}, integrate {z('phys_integrate_ns'):.4f}; "
      f"kernel {z('kernel (wide+narrow colours)'):.4f}), build_graph {z('sys_physics_build_graph_ns'):.4f}, gather {z('sys_physics_gather_ns'):.4f}, apply {z('sys_physics_apply_ns'):.4f}, g {z('g_ns'):.4f}, u+r {z('u_ns') + z('r_ns'):.4f}; "
      f"W-independent block (tree span + serial-in-solve + build_graph + gather + apply) {wind:.4f} = {100 * wind / meas:.0f} % of the measured row")
    R['headline'][f'budget@W{w}'] = {'pred': pred, 'meas': meas, 'w_independent': wind}
# L11's own C4 (D10 parallel fill) build-if arithmetic - not the tree's C4
sb8 = R['spans']['B@W8']['phys_solve_build_ns']
P(f"- L11 D10 (L11's C4, NOT the tree lane's C4) arithmetic: solve_build(8) after C3 {sb8['tip']:.4f} ms (SE {sb8['tse']:.4f}) = "
  f"{100 * sb8['tip'] / C[('J-As', 8, 'tip')][0]['median']:.2f} % of T(8) J-As tip, {100 * sb8['tip'] / C[('J-As-a', 8, 'tip')][0]['median']:.2f} % of the armed T(8)")

os.makedirs(OUTD, exist_ok=True)
with open(OUTD + '/reduction.json', 'w', encoding='utf-8') as f:
    json.dump(R, f, indent=1, default=str)
with open(OUTD + '/tables.txt', 'w', encoding='utf-8') as f:
    f.write('\n'.join(LINES) + '\n')
