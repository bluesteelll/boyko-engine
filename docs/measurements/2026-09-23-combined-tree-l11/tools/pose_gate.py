"""Pose gate (untimed): every row's pose on the parent at its own step count (W=1 and W=8), the tip's
--expect-pose against the parent's pose file (exit 0, summary expect_pose 'match'), and a 501-step red
control on the tip (exit 4). Writes gate/<row>_W<w>_parent.pose and gate/gate.json."""
import json, os, subprocess, sys, hashlib
SP = r'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/d9f1cf70-4eed-44f3-9e33-2efbf030fc1a/scratchpad/win4b'
BIN = {'parent': SP + '/bin/runner_parent.exe', 'tip': SP + '/bin/runner_tip.exe'}
GATE = SP + '/gate'
os.makedirs(GATE, exist_ok=True)
ROWS = json.load(open(SP + '/rows.json', encoding='utf-8'))['rows']
EXPECT = {r['id']: r['expected_pose'] for r in ROWS}

def summary(out):
    for line in out.decode('utf-8', 'replace').splitlines():
        if line.startswith('SUMMARY '):
            return json.loads(line[8:])
    return None

def run(binary, row, w, steps, extra, tag):
    cwd = os.path.join(GATE, f'{row["id"]}_W{w}_{binary}{tag}')
    os.makedirs(cwd, exist_ok=True)
    args = list(row['args']) + row.get('per_w', {}).get(str(w), [])
    args += ['--workers', str(w), '--steps', str(steps), '--window', f'{row["window"][0]}..{min(row["window"][1], steps)}',
             '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'), '--label', f'gate-{row["id"]}@W{w}-{binary}{tag}']
    if row.get('armed'):
        args.append('--arm-profiler')
    if row.get('frozen_by'):
        args += ['--frozen-by', str(row['frozen_by'])]
    args += extra
    p = subprocess.run([BIN[binary]] + args, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    open(os.path.join(cwd, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(cwd, 'stderr.txt'), 'wb').write(p.stderr)
    s = summary(p.stdout) or {}
    pose = os.path.join(cwd, 'pose.bin')
    sha = hashlib.sha256(open(pose, 'rb').read()).hexdigest() if os.path.isfile(pose) else None
    rec = {'binary': binary, 'row': row['id'], 'W': w, 'steps': steps, 'args': args, 'exit': p.returncode,
           'pose_hash': s.get('pose_hash'), 'expect_pose': s.get('expect_pose'), 'pose_sha256': sha,
           'config': s.get('config'), 'void_steps': s.get('void_steps'), 'first_frozen_step': s.get('first_frozen_step'),
           'final_manifolds': s.get('final_manifolds'), 'window_mean_ns': s.get('window_mean_ns'),
           'stderr_tail': p.stderr.decode('utf-8', 'replace')[-300:]}
    return rec, pose

results = []
ok = True
for row in ROWS:
    for w in (1, 8):
        pr, ppose = run('parent', row, w, row['steps'], [], '')
        results.append(pr)
        gate_pose = os.path.join(GATE, f'{row["id"]}_W{w}_parent.pose')
        if os.path.isfile(ppose):
            open(gate_pose, 'wb').write(open(ppose, 'rb').read())
        tr, _ = run('tip', row, w, row['steps'], ['--expect-pose', gate_pose], '')
        results.append(tr)
        verdict = (pr['exit'] == 0 and tr['exit'] == 0 and tr['expect_pose'] == 'match' and pr['pose_hash'] == tr['pose_hash']
                   and pr['pose_hash'] == EXPECT[row['id']] and pr['config'] == tr['config'])
        ok &= verdict
        print(f'{row["id"]:8s} W={w:2d} steps={row["steps"]:5d} parent exit {pr["exit"]} pose {pr["pose_hash"]} | tip exit {tr["exit"]} pose {tr["pose_hash"]} expect_pose {tr["expect_pose"]} | task-expected {EXPECT[row["id"]]} | config-equal {pr["config"] == tr["config"]} | {"OK" if verdict else "RED"}', flush=True)
        if row['id'] == 'J-As' and w == 1:
            # red control: 501 steps on the tip against the parent's 500-step pose -> exit 4
            rr, _ = run('tip', row, w, row['steps'] + 1, ['--expect-pose', gate_pose], '_red501')
            rr['red_control'] = True
            results.append(rr)
            red_ok = rr['exit'] == 4 and rr['expect_pose'] != 'match'
            ok &= red_ok
            print(f'  red control: tip 501 steps vs parent 500-step pose: exit {rr["exit"]} expect_pose {rr["expect_pose"]} pose {rr["pose_hash"]} -> {"OK (red as required)" if red_ok else "NOT RED - gate cannot fail"}', flush=True)
json.dump(results, open(os.path.join(GATE, 'gate.json'), 'w', encoding='utf-8'), indent=1)
print('POSE GATE', 'OK' if ok else 'RED')
sys.exit(0 if ok else 1)
