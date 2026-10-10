"""Window 6 POSE GATE (untimed; run before the window, machine shared by this agent only).

1. base gate per runner binary: J --cfg default --broadphase tree|allpairs W 1/8 (500 steps) = 0x32d5e235342b4143,
   rest --cfg default W 1/8 (500 steps) = 0xee2a67a98434919a;
2. L9's C0 fixtures (600 steps, docs/measurements/2026-09-23-l9-contact-reuse/fixtures of tree 4db26681) on b4db;
3. fixtures for the timed rows (gate/fixtures/<pose_ref>.pose): known hashes asserted, new ones recorded from
   the first process and then required of every other process (other W, other binary);
4. one untimed process per timed cell with --expect-pose (exit 0, match, void_steps 0, TreeDiag, broadphase),
   its wall time kept as the per-process estimate (gate/estimates.json);
5. a 501-step twin per runner binary against the 500-step J fixture: must exit 4 (the gate can fail);
6. one --bench run of the class bench and one filtered criterion run.
A mismatch is re-run ONCE on a fresh process before it is called (memory-fault signature on this machine).
Usage: python gate6.py [--only base,l9fix,cells,red,bench]
"""
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
W6 = os.path.dirname(HERE)
GATE = os.path.join(W6, 'gate')
FIX = os.path.join(GATE, 'fixtures')
ROWS = json.load(open(os.path.join(W6, 'rows6.json'), encoding='utf-8'))
BIN = {k: os.path.join(W6, v['exe'].replace('/', os.sep)) for k, v in ROWS['binaries'].items()}
L9FIX = os.path.join(W6, 'trees', '4db26681', 'docs', 'measurements', '2026-09-23-l9-contact-reuse', 'fixtures')
KNOWN = {'J500': '0x32d5e235342b4143', 'R1100': '0x87e561d20589d4a5', 'RS800': '0x2a2b7926a48aab00',
         'S16-300': '0x8877dbb1192e9b92'}
REST500 = '0xee2a67a98434919a'
TREE_KEYS = ('static_rebuilds', 'sleeper_rebuilds', 'evictions', 'translations', 'patches', 'hint_candidates',
             'wide_rows', 'excluded_rows', 'locator_resets', 'members')
RESULTS = []
ENV = dict(os.environ)
for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
    ENV.pop(k, None)


def summary(so):
    for line in so.splitlines():
        if line.startswith('SUMMARY '):
            try:
                return json.loads(line[len('SUMMARY '):])
            except ValueError:
                return None
    return None


def run(tag, exe_key, args, expect_exit=0):
    d = os.path.join(GATE, tag)
    os.makedirs(d, exist_ok=True)
    cmd = [BIN[exe_key]] + args
    t0 = time.perf_counter()
    p = subprocess.run(cmd, cwd=d, env=ENV, capture_output=True)
    wall = time.perf_counter() - t0
    so = p.stdout.decode('utf-8', 'replace')
    se = p.stderr.decode('utf-8', 'replace')
    open(os.path.join(d, 'stdout.txt'), 'w', encoding='utf-8', newline='\n').write(so)
    open(os.path.join(d, 'stderr.txt'), 'w', encoding='utf-8', newline='\n').write(se)
    s = summary(so)
    rec = {'tag': tag, 'bin': exe_key, 'args': args, 'exit': p.returncode, 'expect_exit': expect_exit,
           'wall_s': round(wall, 3), 'pose': s.get('pose_hash') if s else None,
           'expect_pose': s.get('expect_pose') if s else None, 'void_steps': s.get('void_steps') if s else None,
           'window_mean_ms': round(s['window_mean_ns'] / 1e6, 4) if s and s.get('window_mean_ns') else None,
           'broadphase': (s.get('config') or {}).get('broadphase') if s else None,
           'tree': s.get('broadphase_tree') if s else None, 'dir': d}
    return rec, so, se


def check(rec, want_pose=None, want_bp=None, tree=None, allow_rerun=True, rerun_fn=None):
    why = []
    if rec['exit'] != rec['expect_exit']:
        why.append(f"exit {rec['exit']} != {rec['expect_exit']}")
    if rec['expect_exit'] == 0:
        if want_pose and rec['pose'] != want_pose:
            why.append(f"pose {rec['pose']} != {want_pose}")
        if rec['void_steps'] not in (0, None):
            why.append(f"void_steps {rec['void_steps']}")
        if want_bp and rec['broadphase'] != want_bp:
            why.append(f"broadphase {rec['broadphase']} != {want_bp}")
        bt = rec['tree']
        if tree is True and isinstance(bt, dict):
            if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
                why.append(f'TreeDiag {bt}')
        if tree is False and isinstance(bt, dict) and any(bt.get(k) for k in TREE_KEYS):
            why.append(f'TreeDiag nonzero {bt}')
    rec['why'] = why
    rec['ok'] = not why
    if why and allow_rerun and rerun_fn is not None:
        rec2 = rerun_fn()
        rec2 = check(rec2, want_pose, want_bp, tree, allow_rerun=False)
        rec['rerun'] = rec2
        rec['ok_after_rerun'] = rec2['ok']
    RESULTS.append(rec)
    flag = 'OK ' if rec['ok'] else ('RERUN-OK' if rec.get('ok_after_rerun') else 'FAIL')
    print(f"{flag} {rec['tag']:<44} exit {rec['exit']} pose {rec['pose']} expect {rec['expect_pose']} "
          f"void {rec['void_steps']} bp {rec['broadphase']} wall {rec['wall_s']}s win {rec['window_mean_ms']} {'; '.join(why)}",
          flush=True)
    return rec


def base_gate():
    for key, b in ROWS['binaries'].items():
        if b['kind'] != 'runner':
            continue
        for w in (1, 8):
            for bp in ('tree', 'allpairs'):
                args = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', bp, '--workers', str(w),
                        '--steps', '500', '--pose-out', 'pose.bin']
                tag = f'base/{key}_J-default-{bp}_W{w}'
                rec, so, se = run(tag, key, args)
                if rec['exit'] == 2 and '--broadphase' in se + so:
                    # a runner that predates the flag: the shipped default (AllPairs) once, and say so
                    args = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--workers', str(w), '--steps', '500',
                            '--pose-out', 'pose.bin']
                    tag = f'base/{key}_J-default-noflag_W{w}'
                    rec, so, se = run(tag, key, args)
                check(rec, KNOWN['J500'], rerun_fn=lambda t=tag, k=key, a=args: run(t + '_rerun', k, a)[0])
            args = ['--scene', 'rest', '--cfg', 'default', '--workers', str(w), '--steps', '500', '--pose-out', 'pose.bin']
            tag = f'base/{key}_R-default_W{w}'
            rec, so, se = run(tag, key, args)
            check(rec, REST500, rerun_fn=lambda t=tag, k=key, a=args: run(t + '_rerun', k, a)[0])


def l9_fixtures():
    rows = {'J-A': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a'],
            'J-D': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default'],
            'R': ['--scene', 'rest', '--solver', 'colored'],
            'R-S': ['--scene', 'rest', '--sleeping'],
            'J-Son': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a', '--sleeping']}
    for rid, a in rows.items():
        for w in (1, 8):
            extra = ['--parallel-solve'] if rid == 'R' and w == 8 else []
            fx = os.path.join(L9FIX, f'{rid}_W{w}.pose')
            args = a + extra + ['--workers', str(w), '--steps', '600', '--expect-pose', fx]
            tag = f'l9fix/{rid}_W{w}'
            rec, so, se = run(tag, 'b4db', args)
            rec2 = check(rec, None, rerun_fn=lambda t=tag, a=args: run(t + '_rerun', 'b4db', a)[0])
            if rec2['expect_pose'] != 'match':
                rec2['ok'] = False
                print(f'FAIL {tag}: expect_pose {rec2["expect_pose"]}')


def ref_path(ref):
    return os.path.join(FIX, f'{ref}.pose')


def cells():
    os.makedirs(FIX, exist_ok=True)
    est = {}
    fixtures = {}
    fx_json = os.path.join(FIX, 'fixtures.json')
    if os.path.exists(fx_json):
        fixtures = json.load(open(fx_json, encoding='utf-8'))
    for row in ROWS['rows']:
        kind = row.get('kind', 'runner')
        if kind != 'runner':
            continue
        for key in row['binaries']:
            for w in row['workers']:
                ref = row['pose_ref']
                base = list(row['args']) + ['--workers', str(w), '--steps', str(row['steps']), '--window',
                                            f"{row['window'][0]}..{row['window'][1]}", '--csv', 'run.csv',
                                            '--pose-out', 'pose.bin', '--label', f"gate:{row['id']}#{key}@W{w}"]
                if row['armed']:
                    base.append('--arm-profiler')
                if row.get('canary_of'):
                    # a fixed reference for the untimed gate; the window uses the pass's own reference row
                    base += ['--canary-frac', str(row.get('canary_frac', 0.05)), '--canary-ref-ns', '9000000']
                tag = f"cells/{row['id']}_{key}_W{w}"
                recording = not os.path.exists(ref_path(ref))
                args = base if recording else base + ['--expect-pose', ref_path(ref)]
                rec, so, se = run(tag, key, args)
                want = KNOWN.get(ref) or fixtures.get(ref, {}).get('hash')
                tree = row['broadphase'] == 'Tree'
                rec = check(rec, want, row['broadphase'], tree,
                            rerun_fn=lambda t=tag, k=key, a=args: run(t + '_rerun', k, a)[0])
                good = rec['ok'] or rec.get('ok_after_rerun')
                if recording and good:
                    import shutil
                    shutil.copyfile(os.path.join(rec['dir'], 'pose.bin'), ref_path(ref))
                    fixtures[ref] = {'hash': rec['pose'], 'recorded_by': tag, 'binary': key,
                                     'commit': ROWS['binaries'][key]['commit'], 'args': args,
                                     'known': KNOWN.get(ref)}
                    json.dump(fixtures, open(fx_json, 'w', encoding='utf-8', newline='\n'), indent=1)
                    print(f'   fixture {ref} = {rec["pose"]} recorded from {tag}')
                elif not recording and rec.get('expect_pose') != 'match' and not rec.get('ok_after_rerun'):
                    rec['ok'] = False
                    print(f'FAIL {tag}: expect_pose {rec.get("expect_pose")}')
                est[f"{row['id']}#{key}@W{w}"] = rec['wall_s']
    json.dump(est, open(os.path.join(GATE, 'estimates_runner.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def red():
    for key, b in ROWS['binaries'].items():
        if b['kind'] != 'runner':
            continue
        args = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a', '--workers', '1', '--steps', '501',
                '--expect-pose', ref_path('J500')]
        rec, so, se = run(f'red/{key}_J-A_501', key, args, expect_exit=4)
        check(rec, None, allow_rerun=False)


def bench():
    est = {}
    rec, so, se = run('bench/L9-classes_c4db', 'c4db', ['--bench'])
    check(rec, None, allow_rerun=False)
    est['L9-classes#c4db@W1'] = rec['wall_s']
    for line in so.splitlines():
        if 'refutation' in line or 'ns/pair' in line:
            print('   ' + line)
    env_home = os.path.join(GATE, 'bench', 'criterion_home')
    os.makedirs(env_home, exist_ok=True)
    ENV['CRITERION_HOME'] = env_home
    row = [r for r in ROWS['rows'] if r['id'] == 'C3b-G4'][0]
    rec, so, se = run('bench/C3b-G4_bpb', 'bpb', row['args'])
    check(rec, None, allow_rerun=False)
    est['C3b-G4#bpb@W1'] = rec['wall_s']
    for line in (so + se).splitlines():
        if 'time:' in line or line.startswith('bp_g4_scene'):
            print('   ' + line.strip())
    json.dump(est, open(os.path.join(GATE, 'estimates_bench.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def cells_only_missing():
    """Gate only the cells that have no gate directory yet (rows added after the first gate run)."""
    global ROWS
    full = ROWS
    keep = []
    for row in full['rows']:
        if row.get('kind', 'runner') != 'runner':
            continue
        miss = [(k, w) for k in row['binaries'] for w in row['workers']
                if not os.path.isdir(os.path.join(GATE, 'cells', f"{row['id']}_{k}_W{w}"))]
        if miss:
            keep.append(row)
    ROWS = dict(full, rows=keep)
    est_path = os.path.join(GATE, 'estimates_runner.json')
    old = json.load(open(est_path, encoding='utf-8')) if os.path.exists(est_path) else {}
    cells()
    new = json.load(open(est_path, encoding='utf-8'))
    old.update(new)
    json.dump(old, open(est_path, 'w', encoding='utf-8', newline=chr(10)), indent=1)
    ROWS = full


def main():
    only = None
    if '--only' in sys.argv:
        only = sys.argv[sys.argv.index('--only') + 1].split(',')
    steps = [('base', base_gate), ('l9fix', l9_fixtures), ('cells', cells), ('red', red), ('bench', bench),
             ('newcells', cells_only_missing)]
    for name, fn in steps:
        if (only and name not in only) or (not only and name == 'newcells'):
            continue
        print(f'== {name} {time.strftime("%H:%M:%S")}', flush=True)
        fn()
    out = os.path.join(GATE, f'gate_{"_".join(only) if only else "all"}.json')
    json.dump(RESULTS, open(out, 'w', encoding='utf-8', newline='\n'), indent=1)
    bad = [r['tag'] for r in RESULTS if not (r['ok'] or r.get('ok_after_rerun'))]
    print(f'== done {time.strftime("%H:%M:%S")}: {len(RESULTS)} processes, {len(bad)} failing: {bad}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
