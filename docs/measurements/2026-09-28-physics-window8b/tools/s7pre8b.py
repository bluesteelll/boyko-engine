"""Window 8b (2026-09-29): the S7-AB untimed pre-flight - s7-omega2/cut.md §5 item 2 as amended by "Window 8b: the S7
commands as amended" (receipt model: s7-omega2/bin/rows_s7.sh) - on the S7-AB keys of rows8b.extra.json (s7p = P,
s7t = T). Launches only those two runner exes. Reads no timing field: exits, pose hashes and bytes, void steps, the
S4 setup counters and the armed SUMMARY's w8s lane / scope / task COUNTS.
  python -B tools/s7pre8b.py fixture   # JA500 recorded by s7p (= the tip exe) at W1, 500 steps, byte-compared with
                                       # window 8's committed JA500.pose (from the a3adc827 export), -> gate/fixtures/
  python -B tools/s7pre8b.py run       # -> gate/s7/<cell>/{stdout,stderr,run.csv,pose.bin}, gate/s7/preflight.json,
                                       #    gate/s7/engagement.txt, gate/s7/preflight.log
engagement_why() is imported by selftest8b.py for its red controls."""
import json
import os
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
import driver as D  # noqa: E402

GATE = os.path.join(W8, 'gate')
S7 = os.path.join(GATE, 's7')
FIX = os.path.join(GATE, 'fixtures')
EX = json.load(open(os.path.join(W8, 'rows8b.extra.json'), encoding='utf-8'))
BINS = {k: os.path.join(W8, EX['binaries'][k]['exe']) for k in ('s7p', 's7t')}
LABEL = {k: EX['binaries'][k]['s7'] for k in ('s7p', 's7t')}
NAME = {'s7p': 'P', 's7t': 'T'}
# The engine's own physical-core read on this host (the runner has no such field: cut.md Q5 was not built). Receipt:
# s7-omega2/tr1/t_s7_debug.txt:14, T-S7-4 on a3adc827: "S7 host receipt: physical 8, logical 16, source Os".
PHYSICAL = 8
JT = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', 'tree', '--sleeping', 'off']
JA = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a']
CFG = {'JT': (JT, 'JT500'), 'JA': (JA, 'JA500')}
POSE = '0x30c5438bc6ad9ffa'  # JT500 and JA500 (mkrows8.py POSE_JT; window 8's fixtures.json)
W8_JA500 = 'D:/wt/_targets/w8b-trees/s7tip/docs/measurements/2026-09-27-physics-window8/gate/fixtures/JA500.pose'
LOG = os.path.join(S7, 'preflight.log')
env = dict(os.environ)
for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
    env.pop(k, None)


def log(msg):
    os.makedirs(S7, exist_ok=True)
    line = f'{D.now()} {msg}'
    print(line, flush=True)
    with open(LOG, 'a', encoding='utf-8', newline='\n') as f:
        f.write(line + '\n')


def lanes_bound(w8s, w, physical=PHYSICAL):
    """(lanes sum, bound): lanes sum = lanes_mean x waves; bound = physical x color_scopes + (W + 1) x setup_steps
    (cut.md amended: a capped colour wave runs on at most `physical` lanes, the setup wave on at most W + 1)."""
    lanes = w8s['lanes_mean'] * w8s['waves']
    bound = physical * w8s['color_scopes'] + (w + 1) * w8s['setup_steps']
    return lanes, bound


def engagement_why(label, w, w8s, physical=PHYSICAL):
    """The engagement receipt of one armed process, by its key's label: at W > physical the capped exe (T) must be
    within the bound and the uncapped one (P) ABOVE it - so the receipt can fail, and a P/T swap reds both. At
    W <= physical the cap cannot engage: recorded, not gated."""
    if label not in ('capped', 'uncapped'):
        return [f'unknown S7 label {label!r}']
    lanes, bound = lanes_bound(w8s, w, physical)
    within = lanes <= bound + 0.5
    if w <= physical:
        return []
    if label == 'capped' and not within:
        return [f'capped exe at W{w}: lanes sum {lanes:.0f} above the bound {bound}: the cap did not engage']
    if label == 'uncapped' and within:
        return [f'uncapped exe at W{w}: lanes sum {lanes:.0f} within the bound {bound}: not the uncapped path '
                f'(or the receipt cannot fail)']
    return []


def run_cell(key, cfg, w, armed, tag):
    args, fx = CFG[cfg]
    cwd = os.path.join(S7, f'{NAME[key]}-{cfg}-W{w}{"-armed" if armed else ""}')
    if os.path.isdir(cwd):
        shutil.rmtree(cwd)
    os.makedirs(cwd)
    cmd = [BINS[key]] + args + ['--workers', str(w), '--steps', '500', '--window', '0..500',
                                '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
                                '--label', f's7pre:{tag}', '--expect-pose', os.path.join(FIX, f'{fx}.pose')]
    if armed:
        cmd.append('--arm-profiler')
    p = subprocess.run(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=900)
    open(os.path.join(cwd, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(cwd, 'stderr.txt'), 'wb').write(p.stderr)
    s = D.parse_summary(p.stdout) or {}
    why = []
    if p.returncode != 0:
        why.append(f'exit {p.returncode}')
    if s.get('pose_hash') != POSE:
        why.append(f'pose {s.get("pose_hash")} != {POSE}')
    if s.get('expect_pose') != 'match':
        why.append(f'expect_pose {s.get("expect_pose")}')
    if s.get('void_steps') != 0:
        why.append(f'void_steps {s.get("void_steps")}')
    if s.get('workers') != w:
        why.append(f'workers {s.get("workers")}')
    if bool(s.get('armed')) != armed:
        why.append(f'armed {s.get("armed")}')
    w8s = s.get('w8s') if isinstance(s.get('w8s'), dict) else None
    if armed and w8s is None:
        why.append('armed but no w8s object')
    rec = {'binary': key, 'role': NAME[key], 'cfg': cfg, 'W': w, 'armed': armed, 'exit': p.returncode,
           'pose_hash': s.get('pose_hash'), 'expect_pose': s.get('expect_pose'), 'void_steps': s.get('void_steps'),
           'cwd': cwd, 'fail': why}
    if w8s is not None:
        rec['w8s_counts'] = {k: w8s.get(k) for k in ('waves', 'lanes_mean', 'color_scopes', 'color_tasks',
                                                      'setup_steps', 'setup_tasks')}
        if w >= 2 and not (w8s.get('setup_tasks') or 0) > 0:
            why.append(f'S4 on but setup_tasks {w8s.get("setup_tasks")} at W{w}')
    log(f'{NAME[key]} {cfg} W{w:<2d} {"armed" if armed else "disarmed"}: exit {p.returncode} pose {s.get("pose_hash")} '
        f'expect {s.get("expect_pose")} void {s.get("void_steps")} {"PASS" if not why else "FAIL " + "; ".join(why)}')
    return rec


def part_fixture():
    cwd = os.path.join(GATE, 'fixture_runs', 'JA500')
    if os.path.isdir(cwd):
        shutil.rmtree(cwd)
    os.makedirs(cwd)
    cmd = [BINS['s7p']] + JA + ['--workers', '1', '--steps', '500', '--window', '0..500', '--csv',
                                os.path.join(cwd, 'run.csv'), '--label', 'gate-fixture:JA500', '--pose-out',
                                os.path.join(cwd, 'pose.bin')]
    p = subprocess.run(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=900)
    open(os.path.join(cwd, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(cwd, 'stderr.txt'), 'wb').write(p.stderr)
    s = D.parse_summary(p.stdout) or {}
    pb = os.path.join(cwd, 'pose.bin')
    ours = open(pb, 'rb').read() if os.path.exists(pb) else b''
    theirs = open(W8_JA500, 'rb').read()
    ok = p.returncode == 0 and s.get('pose_hash') == POSE and s.get('void_steps') == 0 and ours == theirs
    log(f'fixture JA500: s7p (= tip exe) W1 exit {p.returncode} pose {s.get("pose_hash")} void {s.get("void_steps")}; '
        f'bytes equal to window 8\'s committed JA500.pose: {ours == theirs} -> {"OK" if ok else "FAIL"}')
    if not ok:
        return 1
    shutil.copyfile(pb, os.path.join(FIX, 'JA500.pose'))
    fp = os.path.join(FIX, 'fixtures.json')
    fx = json.load(open(fp, encoding='utf-8'))
    fx['JA500'] = {'hash': s.get('pose_hash'), 'known': POSE, 'recorded_by': 'tip@W1 500 steps (s7p = tip exe)',
                   'args': JA, 'sha256': D.sha256(pb), 'first_frozen_step': s.get('first_frozen_step'),
                   'tree_diag': s.get('broadphase_tree'), 'fail': [],
                   'window8_fixture': 'docs/measurements/2026-09-27-physics-window8/gate/fixtures/JA500.pose '
                                      '(export of a3adc827): byte-equal', 'added': '2026-09-29 (tools/s7pre8b.py fixture)'}
    json.dump(fx, open(fp, 'w', encoding='utf-8', newline='\n'), indent=1)
    return 0


def part_run():
    recs = []
    for key in ('s7p', 's7t'):
        for w in (1, 2, 4, 8, 16):
            for cfg in ('JT', 'JA'):
                recs.append(run_cell(key, cfg, w, False, f'{NAME[key]}-{cfg}-W{w}'))
    cmp = []
    for w in (1, 2, 4, 8, 16):
        for cfg in ('JT', 'JA'):
            a = os.path.join(S7, f'P-{cfg}-W{w}', 'pose.bin')
            b = os.path.join(S7, f'T-{cfg}-W{w}', 'pose.bin')
            eq = os.path.exists(a) and os.path.exists(b) and open(a, 'rb').read() == open(b, 'rb').read()
            cmp.append({'cfg': cfg, 'W': w, 'equal': eq})
            log(f'cmp P vs T {cfg} W{w}: {"equal (silent)" if eq else "DIFFER/MISSING"}')
    armed = {}
    for key in ('s7p', 's7t'):
        for w in (8, 16):
            for cfg in ('JT', 'JA'):
                r = run_cell(key, cfg, w, True, f'{NAME[key]}-{cfg}-W{w}-armed')
                recs.append(r)
                armed[(key, cfg, w)] = r
    lines, eng = [], []
    for cfg in ('JT', 'JA'):
        for w in (8, 16):
            for key in ('s7p', 's7t'):
                c = armed[(key, cfg, w)].get('w8s_counts')
                if not c:
                    eng.append({'cfg': cfg, 'W': w, 'binary': key, 'fail': ['no w8s counts']})
                    continue
                lanes, bound = lanes_bound(c, w)
                why = engagement_why(LABEL[key], w, c)
                eng.append({'cfg': cfg, 'W': w, 'binary': key, 'label': LABEL[key], 'lanes_sum': round(lanes),
                            'bound': bound, 'within': lanes <= bound + 0.5, 'fail': why, **c})
                lines.append(f'{NAME[key]} ({LABEL[key]}) {cfg} W{w}: lanes_sum {lanes:.0f}, bound {PHYSICAL} x scopes + '
                             f'(W+1) x setup_steps = {bound}, within {lanes <= bound + 0.5}; color_scopes '
                             f'{c["color_scopes"]}, color_tasks {c["color_tasks"]}, setup_steps {c["setup_steps"]}, '
                             f'setup_tasks {c["setup_tasks"]} {"(gated)" if w > PHYSICAL else "(W <= physical: recorded)"} '
                             f'{"PASS" if not why else "FAIL " + "; ".join(why)}')
            p, t = (armed[(k, cfg, w)].get('w8s_counts') or {} for k in ('s7p', 's7t'))
            same = all(p.get(k) == t.get(k) and p.get(k) is not None
                       for k in ('color_scopes', 'color_tasks', 'setup_steps', 'setup_tasks'))
            eng.append({'cfg': cfg, 'W': w, 'P_T_scopes_tasks_setup_equal': same, 'fail': [] if same else ['P/T differ']})
            lines.append(f'  {cfg} W{w}: P and T color_scopes / color_tasks / setup_steps / setup_tasks equal: {same}')
    open(os.path.join(S7, 'engagement.txt'), 'w', encoding='utf-8', newline='\n').write('\n'.join(lines) + '\n')
    for l in lines:
        log(l)
    nfail = sum(1 for r in recs if r['fail']) + sum(1 for c in cmp if not c['equal']) + sum(1 for e in eng if e['fail'])
    out = {'physical_cores': PHYSICAL, 'physical_source': 's7-omega2/tr1/t_s7_debug.txt:14 (T-S7-4 on a3adc827)',
           'processes': recs, 'cmp': cmp, 'engagement': eng, 'failing': nfail}
    json.dump(out, open(os.path.join(S7, 'preflight.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    log(f'S7 pre-flight: {len(recs)} processes ({sum(1 for r in recs if r["fail"])} failing), cmp {len(cmp)} '
        f'({sum(1 for c in cmp if not c["equal"])} differ), engagement {len(eng)} checks '
        f'({sum(1 for e in eng if e["fail"])} failing) -> {"PASS" if not nfail else "FAIL"}')
    return 1 if nfail else 0


if __name__ == '__main__':
    sys.exit({'fixture': part_fixture, 'run': part_run}[sys.argv[1]]())
