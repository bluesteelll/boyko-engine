"""Window 9a untimed gate. Model: win8b/tools/gate8b.py, rewritten for the C4 rows; the validators are the DRIVER's own
(window9a_run imported without running: validate_runner / validate_jolt / criterion_check / validate_rapier), so the gate and the
window cannot disagree. Reads NO timing statistic: exits, poses, voids, armed/canary receipts, the S4 setup counters, TreeDiag,
config receipts, criterion ids. Each process's wall is recorded ONLY as the driver's schedule estimate (gate/estimates.json).
  python -B tools/gate9a.py fixtures      # record JT500 / JA500 / RT500 / S16500 on TIP (W1, 500 steps); JT/JA/RT must equal window 8b's bytes
  python -B tools/gate9a.py rows [ids]    # every runner + jolt row of rows9a.json (+ extra) once at each of its W, --steps 500, --expect-pose
  python -B tools/gate9a.py preflight     # the lane's pre-flight (cut.md §5): flagless TIP/PARENT receipts, --broadphase allpairs == PARENT bytes
  python -B tools/gate9a.py red           # red controls on both exes: 501 steps vs JT500, contact-reuse off vs JT500 -> exit 4
  python -B tools/gate9a.py r4            # C4-S16 vs C4-S16ap pose bytes at W1 (from the rows part)
  python -B tools/gate9a.py list          # `--list` of every criterion row on every one of its exes: the expected ids, no other
  python -B tools/gate9a.py criterion [ids]  # one full-size process per (criterion row, exe), CRITERION_HOME per process
  python -B tools/gate9a.py rapier [ids]  # reserved rapier rows (rows9a.extra.json): each once at its W, the driver's validate_rapier
  python -B tools/gate9a.py estimates     # criterion schedule estimates from the 8b-rate model (the loaded dry samples are kept in gate_criterion.json)
Writes gate/fixtures/, gate/gate_<part>.json, gate/gate_run.log, gate/estimates.json (merged)."""
import json
import os
import re
import shutil
import subprocess
import sys
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W9 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, HERE)
_argv = sys.argv
sys.argv = [_argv[0], '--dry-run']
import window9a_run as R  # noqa: E402  (the driver's validators; main() is not called)
sys.argv = _argv
D = R.D

GATE = os.path.join(W9, 'gate')
FIX = os.path.join(GATE, 'fixtures')
os.makedirs(FIX, exist_ok=True)
ROWS = R.ROWS
BINS = {k: e['path'] for k, e in R.BINS.items()}
LOG = os.path.join(GATE, 'gate_run.log')
EST = os.path.join(GATE, 'estimates.json')
WS = [1, 2, 4, 8, 16]
B8B_FIX = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad/win8b/gate/fixtures'
KNOWN = {'JT500': '0x30c5438bc6ad9ffa', 'JA500': '0x30c5438bc6ad9ffa', 'RT500': '0x6cbe24bf8fafda26'}   # cut.md §5, PREP 8b
# fixture -> recording args (TIP, W1, 500 steps): the C4 spellings of the rows that use them
FIX_SRC = {
    'JT500': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--sleeping', 'off'],
    'JA500': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a'],
    'RT500': ['--scene', 'rest', '--cfg', 'default', '--sleeping', 'off'],
    'S16500': ['--scene', 's16', '--cfg', 'default', '--sleeping', 'off', '--broadphase', 'allpairs'],
}
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


def runner_cmd(key, row, w, cwd, label, steps=500, expect=None, args=None, armed=None):
    a = list(args if args is not None else row['args']) + ['--workers', str(w), '--steps', str(steps), '--window', f'0..{steps}',
                                                           '--csv', os.path.join(cwd, 'run.csv'), '--label', label,
                                                           '--pose-out', os.path.join(cwd, 'pose.bin')]
    if (row['armed'] if armed is None and row else armed):
        a.append('--arm-profiler')
    if expect:
        a += ['--expect-pose', expect]
    return [BINS[key]] + a


def record(key, row, w, rc, so, cwd):
    """A driver-shaped record of one gate process, judged by the driver's own validator for its kind."""
    rec = {'binary': key, 'exit': rc, 'hang': False}
    if row['kind'] in ('runner', 'rapier'):
        rec['summary'] = D.parse_summary(so)
        rec.update(R.runner_stats(row, cwd))
        rec['invalid'] = R.validate_runner(rec, row, w) if row['kind'] == 'runner' else R.validate_rapier(rec, row, w)
    else:
        rec.update(R.jolt_stats(row, cwd, so))
        rec['invalid'] = R.validate_jolt(rec, row, w)
    return rec


def structural(s):
    s = s or {}
    w8 = s.get('w8s') if isinstance(s.get('w8s'), dict) else None
    bt = s.get('broadphase_tree') if isinstance(s.get('broadphase_tree'), dict) else None
    cfg = s.get('config') if isinstance(s.get('config'), dict) else {}
    return {'pose_hash': s.get('pose_hash'), 'expect_pose': s.get('expect_pose'), 'void_steps': s.get('void_steps'),
            'armed': s.get('armed'), 'workers': s.get('workers'), 'target_env': s.get('target_env'),
            'broadphase': cfg.get('broadphase'), 'tree_brute_max_rows': cfg.get('tree_brute_max_rows'),
            'broadphase_select': cfg.get('broadphase_select'), 'tree_diag': bt,
            'kd_order_builds': (bt or {}).get('kd_order_builds'), 'canary_ns': s.get('canary_ns'),
            'setup_steps': (w8 or {}).get('setup_steps'), 'setup_tasks': (w8 or {}).get('setup_tasks'),
            'w8s_present': w8 is not None}


# ---------------------------------------------------------------------------------------------------- fixtures
def part_fixtures():
    out = {}
    for name, args in FIX_SRC.items():
        cwd = os.path.join(GATE, 'fixture_runs', name)
        rc, so, se, wall = run(runner_cmd('tip', None, 1, cwd, f'gate-fixture:{name}', args=args, armed=False), cwd)
        st = structural(D.parse_summary(so))
        why = []
        if rc != 0:
            why.append(f'exit {rc}')
        if st['void_steps'] != 0:
            why.append(f'void_steps {st["void_steps"]}')
        if st['target_env'] != 'msvc':
            why.append(f'target_env {st["target_env"]}')
        if name in KNOWN and st['pose_hash'] != KNOWN[name]:
            why.append(f'pose {st["pose_hash"]} != known {KNOWN[name]}')
        pb = os.path.join(cwd, 'pose.bin')
        eq8b = None
        if name in KNOWN and os.path.exists(pb):     # C4 moves no pose: TIP's bytes equal window 8b's fixture bytes
            eq8b = open(pb, 'rb').read() == open(os.path.join(B8B_FIX, f'{name}.pose'), 'rb').read()
            if not eq8b:
                why.append('pose bytes differ from window 8b\'s fixture')
        if not why:
            shutil.copyfile(pb, os.path.join(FIX, f'{name}.pose'))
        out[name] = {'hash': st['pose_hash'], 'known': KNOWN.get(name), 'recorded_by': 'tip@W1 500 steps', 'args': args,
                     'sha256': D.sha256(pb) if os.path.exists(pb) else None, 'equal_to_window_8b_bytes': eq8b,
                     'tree_diag': st['tree_diag'], 'fail': why}
        log(f'fixture {name:7s} pose {st["pose_hash"]} known {KNOWN.get(name)} == 8b bytes {eq8b} '
            f'{"OK" if not why else "FAIL " + "; ".join(why)}')
    json.dump(out, open(os.path.join(FIX, 'fixtures.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


# ---------------------------------------------------------------------------------------------------- rows
def part_rows(only):
    recs, est = [], {}
    for rid, row in ROWS.items():
        if row['kind'] not in ('runner', 'jolt') or (only and rid not in only):
            continue
        for key in row['binaries']:
            for w in row['workers']:
                cwd = os.path.join(GATE, 'cells', f'{rid}_{key}_W{w}')
                if row['kind'] == 'runner':
                    fixture = os.path.join(FIX, f'{row["pose_ref"]}.pose')
                    cmd = runner_cmd(key, row, w, cwd, f'gate:{rid}#{key}@W{w}', steps=row['steps'], expect=fixture)
                else:
                    cmd = [BINS[key]] + R.jolt_args(row, w)
                rc, so, se, wall = run(cmd, cwd, timeout=1800)
                rec = record(key, row, w, rc, so, cwd)
                why = list(rec['invalid'])
                extra = {}
                if row['kind'] == 'runner':
                    st = structural(rec['summary'])
                    extra = st
                    if row.get('tree_diag_expect') is None and row['broadphase'] == 'Tree':
                        why.append('Tree row without a tree_diag_expect')
                    if st['kd_order_builds']:
                        why.append(f'kd_order_builds {st["kd_order_builds"]} on a default-kernel row')
                else:
                    j = rec.get('jolt') or {}
                    extra = {'stat_lines': j.get('stat_lines'), 'patch_line': j.get('patch_line'),
                             'frames': rec.get('n_frames'), 'files_n': rec.get('files_n')}
                r = {'row': rid, 'binary': key, 'W': w, 'exit': rc, **extra, 'cwd': cwd, 'wall_s_for_schedule': wall, 'fail': why}
                recs.append(r)
                est[f'{rid}#{key}@W{w}'] = wall
                if row['kind'] == 'runner':
                    log(f'row {rid:14s} {key:6s} W{w:<2d} exit {rc} pose {extra["pose_hash"]} expect {extra["expect_pose"]} void '
                        f'{extra["void_steps"]} bp {extra["broadphase"]} brute {extra["tree_brute_max_rows"]} '
                        f'setup {extra["setup_steps"]}/{extra["setup_tasks"]} canary {extra["canary_ns"]} '
                        f'{"PASS" if not why else "FAIL " + "; ".join(why)}')
                else:
                    sl = (extra['stat_lines'] or [{}])[0]
                    log(f'row {rid:14s} {key:6s} W{w:<2d} exit {rc} jolt hash {sl.get("hash")} threads {sl.get("threads")} '
                        f'frames {extra["frames"]} patch "{(extra["patch_line"] or "")[:60]}" {"PASS" if not why else "FAIL " + "; ".join(why)}')
    p = os.path.join(GATE, 'gate_rows.json')
    old = json.load(open(p, encoding='utf-8')) if os.path.exists(p) and only else []
    keep = [r for r in old if r['row'] not in {x['row'] for x in recs}]
    json.dump(keep + recs, open(p, 'w', encoding='utf-8', newline='\n'), indent=1)
    est_update(est)
    log(f'rows gate: {len(recs)} processes, {sum(1 for r in recs if r["fail"])} failing')


# ---------------------------------------------------------------------------------------------------- pre-flight
def part_preflight():
    """cut.md §5 "Untimed pre-flight" on the exes as built: TIP flagless -> Tree, tree_brute_max_rows 128, the JT500 hash and bytes;
    PARENT flagless -> AllPairs, 144, the same hash; TIP --broadphase allpairs == PARENT flagless byte for byte (all five W);
    TIP flagless == PARENT flagless byte for byte (C4 moves no pose); --arm-profiler TIP flagless at W1/W8: void steps 0."""
    out, allok = [], True
    fx = os.path.join(FIX, 'JT500.pose')
    want = R.FIXTURES['JT500']['hash']
    args = FIX_SRC['JT500']
    for w in WS:
        poses = {}
        for tag, key, extra in (('tip-flagless', 'tip', []), ('tip-allpairs', 'tip', ['--broadphase', 'allpairs']),
                                ('parent-flagless', 'parent', [])):
            cwd = os.path.join(GATE, 'preflight', f'{tag}_W{w}')
            rc, so, se, wall = run(runner_cmd(key, None, w, cwd, f'gate-pre:{tag}@W{w}', args=args + extra, armed=False,
                                              expect=fx), cwd)
            st = structural(D.parse_summary(so))
            why = []
            if rc != 0:
                why.append(f'exit {rc}')
            if st['pose_hash'] != want or st['expect_pose'] != 'match':
                why.append(f'pose {st["pose_hash"]} expect_pose {st["expect_pose"]}')
            exp_bp, exp_brute = (('Tree', 128) if tag == 'tip-flagless' else ('AllPairs', 128) if tag == 'tip-allpairs' else ('AllPairs', 144))
            if st['broadphase'] != exp_bp or st['tree_brute_max_rows'] != exp_brute:
                why.append(f'broadphase {st["broadphase"]} brute {st["tree_brute_max_rows"]} != {exp_bp} {exp_brute}')
            if st['void_steps'] != 0:
                why.append(f'void_steps {st["void_steps"]}')
            poses[tag] = open(os.path.join(cwd, 'pose.bin'), 'rb').read() if os.path.exists(os.path.join(cwd, 'pose.bin')) else None
            out.append({'tag': tag, 'W': w, 'exit': rc, **st, 'fail': why})
            allok &= not why
            log(f'preflight {tag:16s} W{w:<2d} exit {rc} bp {st["broadphase"]} brute {st["tree_brute_max_rows"]} pose {st["pose_hash"]} '
                f'{"PASS" if not why else "FAIL " + "; ".join(why)}')
        eq1 = poses['tip-allpairs'] is not None and poses['tip-allpairs'] == poses['parent-flagless']
        eq2 = poses['tip-flagless'] is not None and poses['tip-flagless'] == poses['parent-flagless']
        out.append({'tag': 'bytes', 'W': w, 'tip_allpairs_eq_parent_flagless': eq1, 'tip_flagless_eq_parent_flagless': eq2})
        allok &= eq1 and eq2
        log(f'preflight bytes W{w}: TIP --broadphase allpairs == PARENT flagless {eq1}; TIP flagless == PARENT flagless {eq2}')
    for w in (1, 8):
        cwd = os.path.join(GATE, 'preflight', f'tip-armed_W{w}')
        rc, so, se, wall = run(runner_cmd('tip', None, w, cwd, f'gate-pre:armed@W{w}', args=args, armed=True, expect=fx), cwd)
        st = structural(D.parse_summary(so))
        why = [] if (rc == 0 and st['void_steps'] == 0 and st['expect_pose'] == 'match') else [f'exit {rc} void {st["void_steps"]}']
        out.append({'tag': 'tip-armed', 'W': w, 'exit': rc, **st, 'fail': why})
        allok &= not why
        log(f'preflight tip-armed W{w}: exit {rc} void_steps {st["void_steps"]} setup {st["setup_steps"]}/{st["setup_tasks"]} '
            f'{"PASS" if not why else "FAIL " + "; ".join(why)}')
    json.dump(out, open(os.path.join(GATE, 'gate_preflight.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    log(f'preflight: {"PASS" if allok else "FAIL"}')


# ---------------------------------------------------------------------------------------------------- red controls
def part_red():
    recs = []
    fx = os.path.join(FIX, 'JT500.pose')
    base = FIX_SRC['JT500']
    for key in ('parent', 'tip'):
        for tag, args, steps in (('501-steps-vs-JT500', base, 501), ('contact-reuse-off-vs-JT500', base + ['--contact-reuse', 'off'], 500)):
            cwd = os.path.join(GATE, 'red', f'{tag}_{key}')
            rc, so, se, wall = run(runner_cmd(key, None, 1, cwd, f'gate-red:{tag}', steps=steps, expect=fx, args=args, armed=False), cwd)
            st = structural(D.parse_summary(so))
            ok = rc == 4 and st['expect_pose'] not in (None, 'match')
            recs.append({'tag': tag, 'binary': key, 'exit': rc, 'expect_pose': st['expect_pose'], 'pass': ok})
            log(f'red {tag:28s} {key:6s} exit {rc} expect_pose {st["expect_pose"]} {"PASS (red as required)" if ok else "FAIL"}')
    json.dump(recs, open(os.path.join(GATE, 'gate_red.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def part_r4():
    out = []
    for rid, row in ROWS.items():
        twin = row.get('r4_twin')
        if not twin:
            continue
        for w in row['workers']:
            for key in row['binaries']:
                pa = os.path.join(GATE, 'cells', f'{rid}_{key}_W{w}', 'pose.bin')
                pb = os.path.join(GATE, 'cells', f'{twin}_{key}_W{w}', 'pose.bin')
                eq = os.path.exists(pa) and os.path.exists(pb) and open(pa, 'rb').read() == open(pb, 'rb').read()
                out.append({'row': rid, 'twin': twin, 'W': w, 'equal': eq})
                log(f'R4 {rid} vs {twin} W{w}: {"equal" if eq else "DIFFER/MISSING"}')
    json.dump(out, open(os.path.join(GATE, 'gate_r4.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


# ---------------------------------------------------------------------------------------------------- criterion
def list_ids(exe, regex):
    """`exe --bench --list <regex>`: criterion prints one 'id: benchmark' line per matching benchmark."""
    p = subprocess.run([exe, '--bench', '--list', regex], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, timeout=1800)
    ids = sorted(l.rsplit(':', 1)[0].strip() for l in D.decode(p.stdout).splitlines() if l.strip().endswith(': benchmark'))
    return p.returncode, ids


def part_list(only=None):
    out = []
    for rid, row in ROWS.items():
        if row['kind'] != 'criterion' or (only and rid not in only):
            continue
        regex = row['args'][-1]
        for key in row['binaries']:
            rc, ids = list_ids(BINS[key], regex)
            need = sorted(row['expect'])
            miss, extra = sorted(set(need) - set(ids)), sorted(set(ids) - set(need))
            ok = rc == 0 and not miss and not extra and len(ids) == len(need)
            out.append({'row': rid, 'binary': key, 'exit': rc, 'listed': len(ids), 'expected': len(need), 'missing': miss,
                        'unexpected': extra, 'pass': ok})
            log(f'list {rid:10s} {key:6s} exit {rc} listed {len(ids)} expected {len(need)} missing {miss[:3]} unexpected {extra[:3]} '
                f'{"PASS" if ok else "FAIL"}')
            p = os.path.join(GATE, 'gate_list.json')     # written after every process: a later timeout does not lose earlier rows
            old = json.load(open(p, encoding='utf-8')) if os.path.exists(p) else []
            keep = [r for r in old if (r['row'], r['binary']) != (rid, key)]
            json.dump(keep + [out[-1]], open(p, 'w', encoding='utf-8', newline='\n'), indent=1)


def part_criterion(only):
    out, est = [], {}
    for rid, row in ROWS.items():
        if row['kind'] != 'criterion' or (only and rid not in only):
            continue
        for key in row['binaries']:
            cwd = os.path.join(GATE, 'criterion_sample', f'{rid}_{key}')
            rc, so, se, wall = run([BINS[key]] + row['args'], cwd, timeout=7200, extra_env={'CRITERION_HOME': os.path.join(cwd, 'criterion')})
            rec = {'exit': rc}
            why = R.criterion_check(rec, row, so, se, cwd)
            out.append({'row': rid, 'binary': key, 'exit': rc, 'ids_on_disk': len(rec['criterion']['ids_on_disk']),
                        'ids_on_stdout': len(rec['criterion']['est_ms']), 'expected': len(row['expect']),
                        'wall_s_for_schedule': wall, 'fail': why})
            est[f'{rid}#{key}@W0'] = wall
            log(f'criterion {rid:10s} {key:6s} exit {rc} ids on disk {len(rec["criterion"]["ids_on_disk"])} stdout '
                f'{len(rec["criterion"]["est_ms"])} of {len(row["expect"])} {"PASS" if not why else "FAIL " + "; ".join(why)}')
    p = os.path.join(GATE, 'gate_criterion.json')
    old = json.load(open(p, encoding='utf-8')) if os.path.exists(p) and only else []
    keep = [r for r in old if (r['row'], r['binary']) not in {(x['row'], x['binary']) for x in out}]
    json.dump(keep + out, open(p, 'w', encoding='utf-8', newline='\n'), indent=1)


CRIT_STARTUP_S = 265.0     # a criterion process builds every group's scenes before it filters (run mode, quiet machine): see part_estimates
CRIT_PER_ID_S = 6.6


def part_estimates():
    """Criterion schedule estimates from a MODEL, not from the dry samples. Facts: window 8b's QUIET window ran a 60-id G4 process in
    653-668 s (8b's own loaded dry sample: 739 s). This prep's LOADED measurements (other lanes' rustc + this prep's gates, CPU 60-65 %):
    a 1-id --test process of g4rT took 732 s (so the per-process startup dominates), 8-id samples took 448 s (g4r7, the older exe) and
    877 s (g4r8b). Solving startup S and per-id r with ONE common load factor f: S_loaded + 8 r_loaded = 877, S_loaded = 732 gives
    r_loaded = 18 s; S + 60 r loaded = 1834 s against the quiet 660 s, f = 2.8, so quiet S = 265 s and r = 6.6 s (60-id check: 265 + 396 = 661).
    est = CRIT_STARTUP_S + n_ids x CRIT_PER_ID_S. This is an ESTIMATE resting on an assumed common factor (+-30 %); PREP.md says so. The raw
    loaded sample walls stay in gate_criterion.json (wall_s_for_schedule). Runner and Jolt cells keep their dry-sample walls (like 8b)."""
    est = {}
    for rid, row in ROWS.items():
        if row['kind'] != 'criterion':
            continue
        for key in row['binaries']:
            est[f'{rid}#{key}@W0'] = round(CRIT_STARTUP_S + len(row['expect']) * CRIT_PER_ID_S, 1)
    est_update(est)
    for k, v in sorted(est.items()):
        log(f'estimate {k}: {v} s')


# ---------------------------------------------------------------------------------------------------- rapier (reserved)
def part_rapier(only):
    recs, est = [], {}
    for rid, row in ROWS.items():
        if row['kind'] != 'rapier' or (only and rid not in only):
            continue
        for key in row['binaries']:
            for w in row['workers']:
                cwd = os.path.join(GATE, 'rapier', f'{rid}_{key}_W{w}')
                fixture = os.path.join(FIX, f'{row["pose_ref"]}.pose') if row.get('pose_ref') else None
                cmd = runner_cmd(key, row, w, cwd, f'gate:{rid}#{key}@W{w}', steps=row['steps'], expect=fixture)
                rc, so, se, wall = run(cmd, cwd, timeout=1800)
                rec = record(key, row, w, rc, so, cwd)
                recs.append({'row': rid, 'binary': key, 'W': w, 'exit': rc, 'wall_s_for_schedule': wall, 'fail': rec['invalid']})
                est[f'{rid}#{key}@W{w}'] = wall
                log(f'rapier {rid:20s} {key:8s} W{w:<2d} exit {rc} {"PASS" if not rec["invalid"] else "FAIL " + "; ".join(rec["invalid"])}')
    json.dump(recs, open(os.path.join(GATE, 'gate_rapier.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    est_update(est)


if __name__ == '__main__':
    part = sys.argv[1]
    log(f'=== gate9a {part} start')
    only = set(sys.argv[2:])
    if part == 'rows':
        part_rows(only)
    elif part == 'criterion':
        part_criterion(only)
    elif part == 'rapier':
        part_rapier(only)
    elif part == 'list':
        part_list(only)
    elif part == 'estimates':
        part_estimates()
    else:
        {'fixtures': part_fixtures, 'preflight': part_preflight, 'red': part_red, 'r4': part_r4}[part]()
    log(f'=== gate9a {part} end')
