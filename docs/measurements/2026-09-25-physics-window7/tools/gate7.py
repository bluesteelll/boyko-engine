"""Window 7 POSE GATE (untimed; run before the window, machine shared by this agent only). Adapted from gate6.py.

1. base, per runner binary: J --cfg default --broadphase tree|allpairs W 1/8 (500 steps) = 0x32d5e235342b4143 and
   rest --cfg default W 1/8 (500 steps) = 0xee2a67a98434919a; on the C4 binary (989ca0f0, reuse on by default) with
   --contact-reuse off, and once without the flag (the shipped C4 default, recorded);
2. L9's C0 fixtures (600 steps, docs/measurements/2026-09-23-l9-contact-reuse/fixtures) on the C4 binary with
   --contact-reuse off (the design's pose rule: every C4 reuse-off row equals C0's hash);
3. fixtures for the timed rows (gate/fixtures/<pose_ref>.pose): J500 asserted; the others recorded from the first
   process and required of every other process (other W, other binary); R1100 is compared with window 4b's hash;
4. one untimed process per timed cell with --expect-pose (exit 0, match, void_steps 0, TreeDiag, broadphase), its
   wall kept as the per-process estimate (gate/estimates_runner.json);
5. Jolt v5.6.0 (P0's exe 918fd2b7): every timed W with -f, hash = window 3's 0xb8522b4e3fc62cfe; -receipt at W 1 and 8
   (manifold counts per frame, gate/jolt_receipt.json); the profiled build (P5) at W 1/8 with -f -p;
6. a 501-step twin per runner binary against the 500-step J fixture: must exit 4 (the gate can fail).
A mismatch is re-run ONCE on a fresh process before it is called (memory-fault signature on this machine).
Usage: python gate7.py [--only base,l9fix,cells,jolt,red]
"""
import json
import os
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
W7 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, HERE)
import driver as D  # noqa: E402
import joltprof  # noqa: E402

GATE = os.path.join(W7, 'gate')
FIX = os.path.join(GATE, 'fixtures')
ROWS = json.load(open(os.path.join(W7, 'rows7.json'), encoding='utf-8'))
BIN = {k: (v['exe'] if ':' in v['exe'] else os.path.join(W7, v['exe'].replace('/', os.sep))) for k, v in ROWS['binaries'].items()}
L9FIX = os.path.join(W7, 'trees', '989ca0f0575e054b89bb9c288ad95569ce493734', 'docs', 'measurements',
                     '2026-09-23-l9-contact-reuse', 'fixtures')
KNOWN = {'J500': '0x32d5e235342b4143'}
WIN4B = {'R1100': '0x87e561d20589d4a5'}   # window 4b / window 6 (pre-thin-box binaries): compared, not assumed
WIN6 = {'JAon500c4': '0x30c5438bc6ad9ffa', 'JDon500c4': '0x30c5438bc6ad9ffa', 'Ron1100c4': '0xc8bbe34cf6a8afc6'}
REST500 = '0xee2a67a98434919a'
JOLT56_HASH = '0xb8522b4e3fc62cfe'
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
    if os.path.isdir(d):
        shutil.rmtree(d)
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
           'contact_reuse': (s.get('config') or {}).get('contact_reuse') if s else None,
           'target_env': s.get('target_env') if s else None,
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
        if rec['target_env'] not in ('msvc', None):
            why.append(f"target_env {rec['target_env']}")
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
    print(f"{flag} {rec['tag']:<46} exit {rec['exit']} pose {rec['pose']} expect {rec['expect_pose']} "
          f"void {rec['void_steps']} bp {rec['broadphase']} reuse {rec['contact_reuse']} wall {rec['wall_s']}s "
          f"win {rec['window_mean_ms']} {'; '.join(why)}", flush=True)
    return rec


def jb(bp):
    return ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', bp]


def base_gate():
    for key, b in ROWS['binaries'].items():
        if b['kind'] != 'runner':
            continue
        off = ['--contact-reuse', 'off'] if key == 'c4' else []
        for w in (1, 8):
            for bp in ('tree', 'allpairs'):
                args = jb(bp) + off + ['--workers', str(w), '--steps', '500', '--pose-out', 'pose.bin']
                tag = f'base/{key}_J-default-{bp}{"-reuseoff" if off else ""}_W{w}'
                rec, so, se = run(tag, key, args)
                check(rec, KNOWN['J500'], 'Tree' if bp == 'tree' else 'AllPairs', bp == 'tree',
                      rerun_fn=lambda t=tag, k=key, a=args: run(t + '_rerun', k, a)[0])
            args = ['--scene', 'rest', '--cfg', 'default'] + off + ['--workers', str(w), '--steps', '500', '--pose-out', 'pose.bin']
            tag = f'base/{key}_R-default{"-reuseoff" if off else ""}_W{w}'
            rec, so, se = run(tag, key, args)
            check(rec, REST500, rerun_fn=lambda t=tag, k=key, a=args: run(t + '_rerun', k, a)[0])
            if key == 'c4':
                # the shipped C4 default (reuse on, no flag): recorded, not asserted
                args = jb('tree') + ['--workers', str(w), '--steps', '500', '--pose-out', 'pose.bin']
                rec, so, se = run(f'base/c4_J-default-tree-noflag_W{w}', key, args)
                check(rec, None, 'Tree', True, allow_rerun=False)


def l9_fixtures():
    rows = {'J-A': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a'],
            'J-D': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default'],
            'R': ['--scene', 'rest', '--solver', 'colored'],
            'R-S': ['--scene', 'rest', '--sleeping'],
            'J-Son': ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a', '--sleeping']}
    for key in ('c4', 'trk'):
        for rid, a in rows.items():
            for w in (1, 8):
                extra = ['--parallel-solve'] if rid == 'R' and w == 8 else []
                off = ['--contact-reuse', 'off'] if key == 'c4' else []
                fx = os.path.join(L9FIX, f'{rid}_W{w}.pose')
                args = a + extra + off + ['--workers', str(w), '--steps', '600', '--expect-pose', fx]
                tag = f'l9fix/{key}_{rid}_W{w}'
                rec, so, se = run(tag, key, args)
                rec2 = check(rec, None, rerun_fn=lambda t=tag, k=key, a=args: run(t + '_rerun', k, a)[0])
                em = rec2['rerun']['expect_pose'] if rec2.get('rerun') else rec2['expect_pose']
                if em != 'match':
                    rec2['ok'] = False
                    rec2['ok_after_rerun'] = False
                    print(f'FAIL {tag}: expect_pose {rec2["expect_pose"]} (re-run: {em})', flush=True)


def ref_path(ref):
    return os.path.join(FIX, f'{ref}.pose')


def cells():
    os.makedirs(FIX, exist_ok=True)
    est_path = os.path.join(GATE, 'estimates_runner.json')
    est = json.load(open(est_path, encoding='utf-8')) if os.path.exists(est_path) else {}
    fx_json = os.path.join(FIX, 'fixtures.json')
    fixtures = json.load(open(fx_json, encoding='utf-8')) if os.path.exists(fx_json) else {}
    for row in ROWS['rows']:
        if row.get('kind', 'runner') != 'runner':
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
                    base += ['--canary-frac', str(row.get('canary_frac', 0.05)), '--canary-ref-ns', '9000000']
                tag = f"cells/{row['id']}_{key}_W{w}"
                recording = not os.path.exists(ref_path(ref))
                args = base if recording else base + ['--expect-pose', ref_path(ref)]
                rec, so, se = run(tag, key, args)
                want = KNOWN.get(ref) or fixtures.get(ref, {}).get('hash')
                rec = check(rec, want, row['broadphase'], row['broadphase'] == 'Tree',
                            rerun_fn=lambda t=tag, k=key, a=args: run(t + '_rerun', k, a)[0])
                good = rec['ok'] or rec.get('ok_after_rerun')
                if recording and good:
                    shutil.copyfile(os.path.join(rec['dir'], 'pose.bin'), ref_path(ref))
                    fixtures[ref] = {'hash': rec['pose'], 'recorded_by': tag, 'binary': key,
                                     'commit': ROWS['binaries'][key]['commit'], 'args': args,
                                     'known': KNOWN.get(ref), 'window4b_or_6': WIN4B.get(ref) or WIN6.get(ref),
                                     'equals_earlier_window': (rec['pose'] == (WIN4B.get(ref) or WIN6.get(ref)))
                                     if (WIN4B.get(ref) or WIN6.get(ref)) else None}
                    json.dump(fixtures, open(fx_json, 'w', encoding='utf-8', newline='\n'), indent=1)
                    print(f'   fixture {ref} = {rec["pose"]} recorded from {tag} '
                          f'(earlier window: {WIN4B.get(ref) or WIN6.get(ref)})', flush=True)
                elif not recording:
                    em = rec['rerun']['expect_pose'] if rec.get('rerun') else rec.get('expect_pose')
                    if em != 'match':
                        rec['ok'] = False
                        rec['ok_after_rerun'] = False
                        print(f'FAIL {tag}: expect_pose {rec.get("expect_pose")} (re-run {em})', flush=True)
                if row.get('canary_of') and rec['exit'] == 0:
                    s = summary(open(os.path.join(rec['dir'], 'stdout.txt'), encoding='utf-8').read())
                    print(f"   canary_ns {s.get('canary_ns') if s else None}", flush=True)
                est[f"{row['id']}#{key}@W{w}"] = rec['wall_s']
                json.dump(est, open(est_path, 'w', encoding='utf-8', newline='\n'), indent=1)


def jolt_run(tag, key, args):
    d = os.path.join(GATE, tag)
    if os.path.isdir(d):
        shutil.rmtree(d)
    os.makedirs(d, exist_ok=True)
    t0 = time.perf_counter()
    p = subprocess.run([BIN[key]] + args, cwd=d, env=ENV, capture_output=True)
    wall = time.perf_counter() - t0
    open(os.path.join(d, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(d, 'stderr.txt'), 'wb').write(p.stderr)
    j = D.parse_jolt_stdout(p.stdout)
    st = j['stat_lines'][0] if j['stat_lines'] else {}
    return {'tag': tag, 'bin': key, 'args': args, 'exit': p.returncode, 'wall_s': round(wall, 3), 'hash': st.get('hash'),
            'threads': st.get('threads'), 'patch_line': j['patch_line'], 'dir': d, 'files': sorted(os.listdir(d))}


def jolt():
    est_path = os.path.join(GATE, 'estimates_runner.json')
    est = json.load(open(est_path, encoding='utf-8')) if os.path.exists(est_path) else {}
    for row in ROWS['rows']:
        if row.get('kind') not in ('jolt', 'joltprof'):
            continue
        for key in row['binaries']:
            for w in row['workers']:
                args = list(row['args']) + [f'-t={w}', f"-i={row['steps']}"]
                tag = f"cells/{row['id']}_{key}_W{w}"
                rec = jolt_run(tag, key, args)
                why = []
                if rec['exit'] != 0:
                    why.append(f"exit {rec['exit']}")
                if rec['hash'] != JOLT56_HASH:
                    rec2 = jolt_run(tag + '_rerun', key, args)
                    rec['rerun'] = rec2
                    if rec2['hash'] != JOLT56_HASH:
                        why.append(f"hash {rec['hash']} / re-run {rec2['hash']} != {JOLT56_HASH}")
                if rec['threads'] != w:
                    why.append(f"threads {rec['threads']}")
                pf = [f for f in rec['files'] if f.startswith('per_frame_')]
                if pf:
                    c = D.load_csv(os.path.join(rec['dir'], pf[0]))
                    t = c['Time (ms)']
                    rec['frames'] = len(t)
                    rec['mean_0_500_ms'] = round(statistics.mean(t[0:500]), 4)
                    rec['mean_100_500_ms'] = round(statistics.mean(t[100:500]), 4)
                else:
                    why.append('no per_frame csv')
                if row['kind'] == 'joltprof':
                    dumps = {}
                    for f in rec['files']:
                        if f.startswith('profile_chart_') and f.endswith('.html'):
                            p = joltprof.parse(os.path.join(rec['dir'], f))
                            upd = [v for n, v in p['scopes'].items() if n.startswith('JPH::EPhysicsUpdateError JPH::PhysicsSystem::Update')]
                            dumps[f] = {'threads': p['threads'], 'update_wall_ms': upd[0]['wall_ms'] if upd else None}
                    rec['dumps'] = dumps
                    if len(dumps) < 5:
                        why.append(f'{len(dumps)} profile dumps')
                rec['why'] = why
                rec['ok'] = not why
                RESULTS.append(rec)
                print(f"{'OK ' if not why else 'FAIL'} {tag:<46} exit {rec['exit']} hash {rec['hash']} threads {rec['threads']} "
                      f"wall {rec['wall_s']}s mean[0,500) {rec.get('mean_0_500_ms')} [100,500) {rec.get('mean_100_500_ms')} "
                      f"{'dumps ' + str(len(rec.get('dumps', {}))) if 'dumps' in rec else ''} {'; '.join(why)}", flush=True)
                est[f"{row['id']}#{key}@W{w}"] = rec['wall_s']
                json.dump(est, open(est_path, 'w', encoding='utf-8', newline='\n'), indent=1)
    # Jolt's own manifold counts per frame (window 3's -receipt method), untimed, W 1 and 8
    rc = {}
    for w in (1, 8):
        tag = f'jolt_receipt/j56_W{w}'
        rec = jolt_run(tag, 'j56', ['-s=Pyramid', '-q=Discrete', '-receipt', f'-t={w}', '-i=500'])
        c = D.load_csv(os.path.join(rec['dir'], f'receipt_discrete_th{w}.csv'))
        m = c['Manifolds']
        rec.update({'manifolds_mean_100_500': statistics.mean(m[100:500]), 'manifolds_mean_0_100': statistics.mean(m[0:100]),
                    'manifolds_mean_0_500': statistics.mean(m[0:500]), 'manifolds_final': m[499],
                    'points_mean_100_500': statistics.mean(c['Points'][100:500]),
                    'active_bodies_min': min(c['Active Bodies']), 'frames': len(m)})
        rec['ok'] = rec['exit'] == 0 and rec['hash'] == JOLT56_HASH and rec['frames'] == 500
        rec['why'] = [] if rec['ok'] else [f"exit {rec['exit']} hash {rec['hash']} frames {rec['frames']}"]
        RESULTS.append(rec)
        rc[f'W{w}'] = {k: rec[k] for k in ('hash', 'threads', 'manifolds_mean_100_500', 'manifolds_mean_0_100',
                                            'manifolds_mean_0_500', 'manifolds_final', 'points_mean_100_500',
                                            'active_bodies_min', 'frames', 'args')}
        print(f"{'OK ' if rec['ok'] else 'FAIL'} {tag:<46} hash {rec['hash']} manifolds [100,500) {rec['manifolds_mean_100_500']} "
              f"[0,100) {rec['manifolds_mean_0_100']} final {rec['manifolds_final']} (window 3 v5.6.0: 8489.0 / 7042.23 / 8489)", flush=True)
    json.dump(rc, open(os.path.join(GATE, 'jolt_receipt.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def red():
    for key, b in ROWS['binaries'].items():
        if b['kind'] != 'runner':
            continue
        off = ['--contact-reuse', 'off'] if key == 'c4' else []
        args = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a'] + off + ['--workers', '1', '--steps', '501',
                                                                          '--expect-pose', ref_path('J500')]
        rec, so, se = run(f'red/{key}_J-A_501', key, args, expect_exit=4)
        check(rec, None, allow_rerun=False)


def main():
    only = None
    if '--only' in sys.argv:
        only = sys.argv[sys.argv.index('--only') + 1].split(',')
    steps = [('base', base_gate), ('l9fix', l9_fixtures), ('cells', cells), ('jolt', jolt), ('red', red)]
    for name, fn in steps:
        if only and name not in only:
            continue
        print(f'== {name} {time.strftime("%H:%M:%S")}', flush=True)
        fn()
    out = os.path.join(GATE, f'gate_{"_".join(only) if only else "all"}.json')
    json.dump(RESULTS, open(out, 'w', encoding='utf-8', newline='\n'), indent=1, default=str)
    bad = [r['tag'] for r in RESULTS if not (r['ok'] or r.get('ok_after_rerun'))]
    print(f'== done {time.strftime("%H:%M:%S")}: {len(RESULTS)} processes, {len(bad)} failing: {bad}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
