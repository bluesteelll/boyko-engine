"""Window 8b untimed gate (runner + criterion). Model: win8/tools/gate8.py. Reads NO timing statistic: exits, poses,
voids, armed/canary receipts, the S4 setup counters, TreeDiag / kd_order_builds, structural CSV columns, the
freeze step, criterion ids. Each process's wall is recorded ONLY as the driver's schedule estimate (estimates.json).
  python -B tools/gate8b.py fixtures   # pose fixtures from TIP: JT500 JToff500 JTA500 RT500 JSonT500 JSoffT500
  python -B tools/gate8b.py s4         # both exes x {J-T, J-T armed, J-T-off} x W 1/2/4/8/16: poses, voids, setup counters,
                                       # and every non-timing CSV column TIP vs PARENT (armed J-T)
  python -B tools/gate8b.py red        # red controls on both exes: 501 steps vs JT500, J-T-off vs JT500 -> exit 4
  python -B tools/gate8b.py rows [ids] # every runner row of rows8b.json (+ extra) at its listed W, --steps 500, --expect-pose
  python -B tools/gate8b.py f3pre      # window_cmds.md §2 pre-flight (W1, 20 steps, both kernels, cmp) + R4 over the rows part
  python -B tools/gate8b.py freeze     # J-Son-T: the freeze step from the rows part's CSVs -> gate/jsont_freeze.json
  python -B tools/gate8b.py criterion  # one full-size G4 process (CRITERION_HOME per process): the 60 ids, no other
  python -B tools/gate8b.py micro [ids]# reserved micro rows (rows8b.extra.json): exit, SUMMARY count and fields
Writes gate/fixtures/, gate/gate_<part>.json, gate/gate_run.log, gate/estimates.json (merged)."""
import json
import os
import shutil
import subprocess
import sys
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
import driver as D  # noqa: E402
sys.path.insert(0, HERE)
import micro8b as M  # noqa: E402  (2026-09-29: the micro rows' validity rules, shared with the driver)

GATE = os.path.join(W8, 'gate')
FIX = os.path.join(GATE, 'fixtures')
os.makedirs(FIX, exist_ok=True)
RJ = json.load(open(os.path.join(W8, 'rows8b.json'), encoding='utf-8'))
EXTRA = os.path.join(W8, 'rows8b.extra.json')
if os.path.exists(EXTRA):
    ex = json.load(open(EXTRA, encoding='utf-8'))
    RJ['binaries'].update(ex.get('binaries', {}))
    RJ['rows'] = [r for r in RJ['rows'] if r['id'] not in {x['id'] for x in ex.get('rows', [])}] + ex.get('rows', [])
ROWS = {r['id']: r for r in RJ['rows']}
BINS = {k: (e['exe'] if ':' in e['exe'] else os.path.join(W8, e['exe'])) for k, e in RJ['binaries'].items()}
S4 = {k: e.get('s4') for k, e in RJ['binaries'].items()}
KNOWN = {'JT500': '0x30c5438bc6ad9ffa', 'JToff500': '0x32d5e235342b4143', 'JTA500': '0x30c5438bc6ad9ffa',
         'RT500': '0x6cbe24bf8fafda26'}
JT = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', 'tree', '--sleeping', 'off']
JTOFF = JT + ['--contact-reuse', 'off']
# fixture -> (recording args, armed): TIP, W1, 500 steps
FIX_SRC = {'JT500': (JT, False), 'JToff500': (JTOFF, False),
           'JTA500': (ROWS['F3-TA-armed-leaflist']['args'], True), 'RT500': (ROWS['F3-RT-leaflist']['args'], False),
           'JSonT500': (ROWS['JSonT']['args'], False), 'JSoffT500': (ROWS['JSoffT']['args'], False)}
WS = [1, 2, 4, 8, 16]
LOG = os.path.join(GATE, 'gate_run.log')
EST = os.path.join(GATE, 'estimates.json')
CUT_SETUP = {1: (0, 0), 2: (500, 6000), 4: (500, 12000), 8: (500, 15980), 16: (500, 15980)}  # cut.md §5 (c4/rows)
env = dict(os.environ)
for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
    env.pop(k, None)


def log(msg):
    line = f'{D.now()} {msg}'
    print(line, flush=True)
    with open(LOG, 'a', encoding='utf-8', newline='\n') as f:
        f.write(line + '\n')


def run(cmd, cwd, timeout=900, extra_env=None):
    if os.path.isdir(cwd):
        shutil.rmtree(cwd)
    os.makedirs(cwd, exist_ok=True)
    e = dict(env)
    e.update(extra_env or {})
    t0 = time.perf_counter()
    p = subprocess.run(cmd, cwd=cwd, env=e, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    wall = round(time.perf_counter() - t0, 3)
    open(os.path.join(cwd, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(cwd, 'stderr.txt'), 'wb').write(p.stderr)
    return p.returncode, p.stdout, p.stderr, wall


def est_update(d):
    cur = json.load(open(EST, encoding='utf-8')) if os.path.exists(EST) else {}
    cur.update(d)
    json.dump(cur, open(EST, 'w', encoding='utf-8', newline='\n'), indent=1, sort_keys=True)


def runner_cmd(key, args, armed, w, cwd, label, steps=500, expect=None):
    a = list(args) + ['--workers', str(w), '--steps', str(steps), '--window', f'0..{steps}',
                      '--csv', os.path.join(cwd, 'run.csv'), '--label', label, '--pose-out', os.path.join(cwd, 'pose.bin')]
    if armed:
        a.append('--arm-profiler')
    if expect:
        a += ['--expect-pose', expect]
    return [BINS[key]] + a


def structural(s):
    """The SUMMARY fields the gate reads (no timing field: window_mean_ns / window_steps_per_s are never copied)."""
    s = s or {}
    w8 = s.get('w8s') if isinstance(s.get('w8s'), dict) else None
    bt = s.get('broadphase_tree') if isinstance(s.get('broadphase_tree'), dict) else None
    return {'pose_hash': s.get('pose_hash'), 'expect_pose': s.get('expect_pose'), 'void_steps': s.get('void_steps'),
            'first_void': s.get('first_void'), 'armed': s.get('armed'), 'workers': s.get('workers'),
            'target_env': s.get('target_env'), 'drops_total': s.get('drops_total'),
            'disarmed_ring_traffic': s.get('disarmed_ring_traffic'), 'canary_ns': s.get('canary_ns'),
            'canary_zone': s.get('canary_zone'), 'canary_zone_ns': s.get('canary_zone_ns'),
            'broadphase': (s.get('config') or {}).get('broadphase') if isinstance(s.get('config'), dict) else None,
            'tree_diag': bt, 'kd_order_builds': (bt or {}).get('kd_order_builds'), 'bp_kernel_flag': s.get('bp_kernel_flag'),
            'setup_steps': (w8 or {}).get('setup_steps'), 'setup_tasks': (w8 or {}).get('setup_tasks'),
            'w8s_present': w8 is not None, 'first_frozen_step': s.get('first_frozen_step')}


def check(key, row, w, rc, st, want_hash, expect_exit=0):
    why = []
    if rc != expect_exit:
        why.append(f'exit {rc} (expected {expect_exit})')
    if st['pose_hash'] is None:
        return why + ['no SUMMARY']
    if want_hash and st['pose_hash'] != want_hash:
        why.append(f'pose {st["pose_hash"]} != {want_hash}')
    if st['workers'] != w:
        why.append(f'workers {st["workers"]}')
    if st['target_env'] != 'msvc':
        why.append(f'target_env {st["target_env"]}')
    if st['void_steps'] != 0:
        why.append(f'void_steps {st["void_steps"]} first {st["first_void"]}')
    armed = row.get('armed')
    if bool(st['armed']) != bool(armed):
        why.append(f'armed {st["armed"]}')
    if not armed and st['disarmed_ring_traffic']:
        why.append(f'disarmed_ring_traffic {st["disarmed_ring_traffic"]}')
    if armed and st['drops_total']:
        why.append(f'drops_total {st["drops_total"]}')
    if armed and not st['w8s_present']:
        why.append('armed but no w8s object')
    if not armed and st['w8s_present']:
        why.append('disarmed but a w8s object')
    if row.get('broadphase') and st['broadphase'] != row['broadphase']:
        why.append(f'broadphase {st["broadphase"]} != {row["broadphase"]}')
    kd = st['kd_order_builds']
    if row.get('bp_kernel') == 'leaflist-kd' and not kd:
        why.append(f'kd_order_builds {kd} on leaflist-kd')
    if row.get('bp_kernel') != 'leaflist-kd' and kd:
        why.append(f'kd_order_builds {kd} on {row.get("bp_kernel")}')
    exp = row.get('tree_diag_expect')
    if exp and row.get('broadphase') == 'Tree' and any((st['tree_diag'] or {}).get(k) != v for k, v in exp.items()):
        why.append(f'TreeDiag {st["tree_diag"]} != {exp}')
    if row.get('canary') and not st['canary_ns']:
        why.append(f'canary_ns {st["canary_ns"]}')
    zc = row.get('zone_canary')
    if zc and (st['canary_zone'] != zc[0] or st['canary_zone_ns'] != zc[1]):
        why.append(f'canary_zone {st["canary_zone"]} {st["canary_zone_ns"]} != {zc}')
    if not zc and st['canary_zone'] is not None:
        why.append(f'unexpected canary_zone {st["canary_zone"]}')
    s4 = S4.get(key)
    if armed and s4:
        pair = (st['setup_steps'], st['setup_tasks'])
        if s4 == 'off' and pair != (0, 0):
            why.append(f'S4 off: setup {pair}')
        if s4 == 'on' and w == 1 and pair != (0, 0):
            why.append(f'S4 W1 not inline: setup {pair}')
        if s4 == 'on' and w >= 2 and not (pair[1] or 0) > 0:
            why.append(f'S4 on W{w}: setup {pair}')
    return why


def fixture_hash(name):
    fx = json.load(open(os.path.join(FIX, 'fixtures.json'), encoding='utf-8'))
    return fx[name]['hash']


def part_fixtures():
    out = {}
    for name, (args, armed) in FIX_SRC.items():
        cwd = os.path.join(GATE, 'fixture_runs', name)
        row = {'armed': armed, 'broadphase': 'Tree'}
        rc, so, se, wall = run(runner_cmd('tip', args, armed, 1, cwd, f'gate-fixture:{name}'), cwd)
        st = structural(D.parse_summary(so))
        why = check('tip', row, 1, rc, st, KNOWN.get(name))
        pb = os.path.join(cwd, 'pose.bin')
        if not why and os.path.exists(pb):
            shutil.copyfile(pb, os.path.join(FIX, f'{name}.pose'))
        out[name] = {'hash': st['pose_hash'], 'known': KNOWN.get(name), 'recorded_by': 'tip@W1 500 steps',
                     'args': list(args) + (['--arm-profiler'] if armed else []),
                     'sha256': D.sha256(pb) if os.path.exists(pb) else None, 'first_frozen_step': st['first_frozen_step'],
                     'tree_diag': st['tree_diag'], 'fail': why}
        log(f'fixture {name:10s} pose {st["pose_hash"]} known {KNOWN.get(name)} frozen {st["first_frozen_step"]} '
            f'tree {st["tree_diag"]} {"OK" if not why else "FAIL " + "; ".join(why)}')
    json.dump(out, open(os.path.join(FIX, 'fixtures.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def csv_struct(path):
    """Every CSV column that is not a timing column (wall_ns, *_ns, *tick*), as a list per column."""
    c = D.load_csv(path) or {}
    return {k: v for k, v in c.items() if not (k == 'wall_ns' or k.endswith('_ns') or 'tick' in k)}


def part_s4():
    recs = []
    cols = {}
    for w in WS:
        for key in ('parent', 'tip'):
            for tag, args, armed, fixture in (('J-T', JT, False, 'JT500'), ('J-T-a', JT, True, 'JT500'),
                                              ('J-T-off', JTOFF, False, 'JToff500')):
                cwd = os.path.join(GATE, 's4', f'{tag}_{key}_W{w}')
                row = {'armed': armed, 'broadphase': 'Tree', 'bp_kernel': None,
                       'tree_diag_expect': {'static_rebuilds': 1, 'members': 1, 'evictions': 0}}
                rc, so, se, wall = run(runner_cmd(key, args, armed, w, cwd, f'gate-s4:{tag}#{key}@W{w}',
                                                  expect=os.path.join(FIX, f'{fixture}.pose')), cwd)
                st = structural(D.parse_summary(so))
                why = check(key, row, w, rc, st, KNOWN[fixture])
                if st['expect_pose'] != 'match':
                    why.append(f'expect_pose {st["expect_pose"]}')
                rec = {'tag': tag, 'binary': key, 'W': w, 'exit': rc, **st, 'fail': why}
                if armed:
                    rec['cut_recorded_setup'] = CUT_SETUP[w] if key == 'tip' else (0, 0)
                    rec['setup_equals_cut'] = (st['setup_steps'], st['setup_tasks']) == tuple(rec['cut_recorded_setup'])
                    cols[(key, w)] = csv_struct(os.path.join(cwd, 'run.csv'))
                recs.append(rec)
                log(f's4 {tag:8s} {key:6s} W{w:<2d} exit {rc} pose {st["pose_hash"]} expect {st["expect_pose"]} void '
                    f'{st["void_steps"]} setup {st["setup_steps"]}/{st["setup_tasks"]}'
                    f'{" (cut: " + str(rec.get("cut_recorded_setup")) + ")" if armed else ""} '
                    f'{"PASS" if not why else "FAIL " + "; ".join(why)}')
    # "PARENT disables S4 and nothing else": every non-timing per-step column, TIP vs PARENT, armed J-T.
    diff = {}
    for w in WS:
        a, b = cols.get(('parent', w)) or {}, cols.get(('tip', w)) or {}
        names = sorted(set(a) | set(b))
        differ = [n for n in names if a.get(n) != b.get(n)]
        diff[w] = {'columns_compared': len(names), 'differ': differ,
                   'only_in': sorted(set(a) ^ set(b))}
        log(f's4 columns TIP vs PARENT W{w}: {len(names)} non-timing columns, differ {differ or "none"}')
    json.dump({'processes': recs, 'columns_tip_vs_parent': diff},
              open(os.path.join(GATE, 'gate_s4.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    log(f's4 gate: {len(recs)} processes, {sum(1 for r in recs if r["fail"])} failing')


def part_red():
    recs = []
    for key in ('parent', 'tip'):
        for tag, args, steps in (('501-steps-vs-JT500', JT, 501), ('J-T-off-vs-JT500', JTOFF, 500)):
            cwd = os.path.join(GATE, 'red', f'{tag}_{key}')
            rc, so, se, wall = run(runner_cmd(key, args, False, 1, cwd, f'gate-red:{tag}', steps=steps,
                                              expect=os.path.join(FIX, 'JT500.pose')), cwd)
            st = structural(D.parse_summary(so))
            ok = rc == 4 and st['expect_pose'] not in (None, 'match')
            recs.append({'tag': tag, 'binary': key, 'exit': rc, 'expect_pose': st['expect_pose'], 'pass': ok})
            log(f'red {tag:20s} {key:6s} exit {rc} expect_pose {st["expect_pose"]} {"PASS (red as required)" if ok else "FAIL"}')
    json.dump(recs, open(os.path.join(GATE, 'gate_red.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def part_rows(only):
    recs, est = [], {}
    for rid, row in ROWS.items():
        if row['kind'] != 'runner' or (only and rid not in only):
            continue
        for key in row['binaries']:
            for w in row['workers']:
                cwd = os.path.join(GATE, 'cells', f'{rid}_{key}_W{w}')
                fixture = os.path.join(FIX, f'{row["pose_ref"]}.pose')
                rc, so, se, wall = run(runner_cmd(key, row['args'], row['armed'], w, cwd, f'gate:{rid}#{key}@W{w}',
                                                  steps=row['steps'], expect=fixture), cwd)
                st = structural(D.parse_summary(so))
                why = check(key, row, w, rc, st, fixture_hash(row['pose_ref']))
                if st['expect_pose'] != 'match':
                    why.append(f'expect_pose {st["expect_pose"]}')
                ef = row.get('expect_first_frozen')
                if ef is not None and st['first_frozen_step'] != ef:
                    why.append(f'first_frozen_step {st["first_frozen_step"]} != {ef}')
                rec = {'row': rid, 'binary': key, 'W': w, 'exit': rc, **st, 'cwd': cwd, 'wall_s_for_schedule': wall,
                       'fail': why}
                recs.append(rec)
                est[f'{rid}#{key}@W{w}'] = wall
                log(f'row {rid:22s} {key:6s} W{w:<2d} exit {rc} pose {st["pose_hash"]} expect {st["expect_pose"]} void '
                    f'{st["void_steps"]} kd {st["kd_order_builds"]} setup {st["setup_steps"]}/{st["setup_tasks"]} frozen '
                    f'{st["first_frozen_step"]} {"PASS" if not why else "FAIL " + "; ".join(why)}')
    p = os.path.join(GATE, 'gate_rows.json')
    old = json.load(open(p, encoding='utf-8')) if os.path.exists(p) and only else []
    keep = [r for r in old if r['row'] not in {x['row'] for x in recs}]
    json.dump(keep + recs, open(p, 'w', encoding='utf-8', newline='\n'), indent=1)
    est_update(est)
    log(f'rows gate: {len(recs)} processes, {sum(1 for r in recs if r["fail"])} failing')


def part_f3pre():
    out = {'preflight': [], 'r4_rows': []}
    for k in ('leaflist', 'leaflist-kd'):
        cwd = os.path.join(GATE, 'f3pre', k)
        args = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', 'tree', '--bp-kernel', k]
        rc, so, se, wall = run(runner_cmd('tip', args, True, 1, cwd, f'gate-f3pre:{k}', steps=20), cwd)
        st = structural(D.parse_summary(so))
        out['preflight'].append({'kernel': k, 'exit': rc, 'void_steps': st['void_steps'], 'kd_order_builds': st['kd_order_builds']})
        log(f'f3pre {k:12s} exit {rc} void {st["void_steps"]} kd_order_builds {st["kd_order_builds"]}')
    a = open(os.path.join(GATE, 'f3pre', 'leaflist', 'pose.bin'), 'rb').read()
    b = open(os.path.join(GATE, 'f3pre', 'leaflist-kd', 'pose.bin'), 'rb').read()
    pf = out['preflight']
    out['preflight_pass'] = (a == b and all(x['exit'] == 0 and x['void_steps'] == 0 for x in pf)
                             and not pf[0]['kd_order_builds'] and (pf[1]['kd_order_builds'] or 0) > 0)
    log(f'f3pre cmp {"silent (equal)" if a == b else "DIFFER"}; pre-flight {"PASS" if out["preflight_pass"] else "FAIL"}')
    for rid, row in ROWS.items():
        twin = row.get('r4_twin')
        if not twin:
            continue
        for w in row['workers']:
            pa = os.path.join(GATE, 'cells', f'{rid}_tip_W{w}', 'pose.bin')
            pb = os.path.join(GATE, 'cells', f'{twin}_tip_W{w}', 'pose.bin')
            eq = os.path.exists(pa) and os.path.exists(pb) and open(pa, 'rb').read() == open(pb, 'rb').read()
            out['r4_rows'].append({'row': rid, 'twin': twin, 'W': w, 'equal': eq})
            log(f'R4 {rid:20s} vs {twin:22s} W{w:<2d} {"equal" if eq else "DIFFER/MISSING"}')
    json.dump(out, open(os.path.join(GATE, 'gate_f3pre.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def part_freeze():
    """The freeze step of J-Son-T from the rows part's CSVs (the first step whose awake count is 0; the runner's
    first_frozen_step is that step + 1, the first step of the frozen pile), cross-checked with the SUMMARY, per W."""
    res = {}
    for rid in ('JSonT', 'JSonT-a'):
        for w in ROWS[rid]['workers']:
            cwd = os.path.join(GATE, 'cells', f'{rid}_tip_W{w}')
            c = D.load_csv(os.path.join(cwd, 'run.csv')) or {}
            aw = c.get('awake') or []
            z = next((i for i, v in enumerate(aw) if v == 0), None)
            stays = z is not None and all(v == 0 for v in aw[z:])
            s = structural(D.parse_summary(open(os.path.join(cwd, 'stdout.txt'), 'rb').read()))
            res[f'{rid}@W{w}'] = {'first_zero_awake_csv_row': z, 'csv_first_frozen_step': (z + 1) if z is not None else None,
                                  'summary_first_frozen_step': s['first_frozen_step'], 'awake_stays_zero': stays,
                                  'rows': len(aw)}
            log(f'freeze {rid}@W{w}: CSV first awake==0 at step {z} (frozen from {None if z is None else z + 1}), '
                f'SUMMARY first_frozen_step {s["first_frozen_step"]}, stays zero {stays}')
    vals = {v['summary_first_frozen_step'] for v in res.values()} | {v['csv_first_frozen_step'] for v in res.values()}
    ok = len(vals) == 1 and None not in vals and all(v['awake_stays_zero'] for v in res.values())
    out = {'per_process': res, 'consistent': ok}
    if ok:
        f = vals.pop()
        out.update({'first_frozen_step': f, 'post_freeze_window': [f, 500],
                    'note': 'the post-freeze window starts at the first step of the frozen pile (first_frozen_step), '
                            "window 8's [274, 500) convention"})
    json.dump(out, open(os.path.join(GATE, 'jsont_freeze.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    log(f'freeze: consistent {ok} -> {out.get("post_freeze_window")}')


def part_criterion():
    row = ROWS['G4']
    cwd = os.path.join(GATE, 'criterion_sample')
    rc, so, se, wall = run([BINS['g4ref']] + row['args'], cwd, timeout=3600,
                           extra_env={'CRITERION_HOME': os.path.join(cwd, 'criterion')})
    ids = []
    for root, dirs, files in os.walk(os.path.join(cwd, 'criterion')):
        if 'benchmark.json' in files and os.path.basename(root) == 'new':
            ids.append(json.load(open(os.path.join(root, 'benchmark.json'), encoding='utf-8')).get('full_id'))
    ids.sort()
    miss = sorted(set(row['expect']) - set(ids))
    extra = sorted(set(ids) - set(row['expect']))
    ok = rc == 0 and not miss and not extra
    json.dump({'exit': rc, 'ids_on_disk': len(ids), 'missing': miss, 'unexpected': extra, 'pass': ok,
               'wall_s_for_schedule': wall}, open(os.path.join(GATE, 'gate_criterion.json'), 'w', encoding='utf-8',
                                                  newline='\n'), indent=1)
    est_update({'G4#g4ref@W0': wall})
    log(f'criterion G4 exit {rc} ids on disk {len(ids)} of {len(row["expect"])} missing {miss[:3]} unexpected {extra[:3]} '
        f'{"PASS" if ok else "FAIL"}')


def part_micro(only):
    """For a reserved micro block (omega_b v2 from rows8b.extra.json): each micro row once at full size; exit 0, the
    SUMMARY count if the row names it, and every SUMMARY matching the row's expect_fields / expect_bench / expect_route
    / expect_stages. The wall goes to estimates.json only."""
    recs, est = [], {}
    for rid, row in ROWS.items():
        if row['kind'] != 'micro' or (only and rid not in only):
            continue
        key = row['binaries'][0]
        cwd = os.path.join(GATE, 'micro', rid)
        rc, so, se, wall = run([BINS[key]] + list(row['args']), cwd, timeout=1800)
        # 2026-09-29: the same rules as the driver (micro8b.py); with one sample, "0 on every rep" is this sample.
        ss, cal = M.parse(D.decode(so))
        why = [] if rc == 0 else [f'exit {rc}']
        rules, notes = M.process_rules(row, ss, cal)
        why += rules
        void, nopark, _ = M.cell_park_rule([{'row': rid, 'micro_notes': notes}])
        why += [f'park row {k} at gap > 0 took no park (void)' for _, k, _ in void]
        recs.append({'row': rid, 'exit': rc, 'summaries': len(ss), 'calibration_lines': len(cal), 'micro_notes': notes,
                     'no_park_taken_reported': [k for _, k, _ in nopark], 'wall_s_for_schedule': wall, 'fail': why})
        est[f'{rid}#{key}@W0'] = wall
        log(f'micro {rid:24s} exit {rc} summaries {len(ss)} calibration lines {len(cal)} park rows '
            f'{len(notes.get("park", []))} (no park taken at gap 0: {len(nopark)}) '
            f'{"PASS" if not why else "FAIL " + "; ".join(why)}')
    p = os.path.join(GATE, 'gate_micro.json')
    old = json.load(open(p, encoding='utf-8')) if os.path.exists(p) and only else []
    recs = [r for r in old if r['row'] not in {x['row'] for x in recs}] + recs
    json.dump(recs, open(p, 'w', encoding='utf-8', newline='\n'), indent=1)
    est_update(est)


if __name__ == '__main__':
    part = sys.argv[1]
    log(f'=== gate8b {part} start')
    if part == 'rows':
        part_rows(set(sys.argv[2:]))
    elif part == 'micro':
        part_micro(set(sys.argv[2:]))
    else:
        {'fixtures': part_fixtures, 's4': part_s4, 'red': part_red, 'f3pre': part_f3pre, 'freeze': part_freeze,
         'criterion': part_criterion}[part]()
    log(f'=== gate8b {part} end')
