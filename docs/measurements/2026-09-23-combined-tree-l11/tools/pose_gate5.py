"""Window 5 pose gate (UNTIMED). Adapted from window 4b's pose_gate.py and window 4c's pose_gate.sh.

On BOTH binaries (parent first, then tip), every row of rows.json at W 1 and W 8 (HL-D-* and HL-R-* are
tip-only in the timed block but gated on both binaries here), 500 steps, window 0..500:
  * the reference pose files come from the PARENT's first run of each pose family, with no --expect-pose:
      gate/J_ref.pose = parent HL-D-allpairs W1,  gate/R_ref.pose = parent HL-R-allpairs W1;
    every other run passes --expect-pose <ref> and must print expect_pose "match" and exit 0;
  * the summary pose_hash must equal the row's expected_pose (task value);
  * TreeDiag (summary broadphase_tree): tree rows static_rebuilds 1, members 1, evictions 0; allpairs rows
    every field 0;
  * config JSON compared field by field between parent and tip for each (row, W);
  * red controls on each binary: HL-D-tree, J-As and HL-R-tree at 501 steps against the 500-step ref pose
    must exit 4 with expect_pose != "match".
Writes gate/<row>_W<w>_<binary>[_red501]/{stdout,stderr,run.csv,pose.bin}, gate/gate.json; prints one line
per process. Exit 0 iff every check holds."""
import hashlib
import json
import os
import subprocess
import sys

SP = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = {'parent': os.path.join(SP, 'bin', 'runner_parent.exe'), 'tip': os.path.join(SP, 'bin', 'runner_tip.exe')}
GATE = os.path.join(SP, 'gate')
os.makedirs(GATE, exist_ok=True)
ROWS = json.load(open(os.path.join(SP, 'rows.json'), encoding='utf-8'))['rows']
BYID = {r['id']: r for r in ROWS}
TREE_KEYS = ('static_rebuilds', 'sleeper_rebuilds', 'evictions', 'translations', 'patches', 'hint_candidates',
             'wide_rows', 'excluded_rows', 'locator_resets', 'members')


def summary(out):
    for line in out.decode('utf-8', 'replace').splitlines():
        if line.startswith('SUMMARY '):
            return json.loads(line[8:])
    return None


def tree_ok(row, bt):
    if not isinstance(bt, dict):
        return False, 'no broadphase_tree'
    if row['tree']:
        ok = bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0
        return ok, 'tree: static_rebuilds 1 / members 1 / evictions 0'
    ok = all(bt.get(k) == 0 for k in TREE_KEYS)
    return ok, 'allpairs: every field 0'


def run(binary, row, w, steps, expect, tag):
    cwd = os.path.join(GATE, f'{row["id"]}_W{w}_{binary}{tag}')
    os.makedirs(cwd, exist_ok=True)
    args = list(row['args']) + ['--workers', str(w), '--steps', str(steps), '--window', f'0..{min(500, steps)}',
                                '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
                                '--label', f'gate-{row["id"]}@W{w}-{binary}{tag}']
    if row['armed']:
        args.append('--arm-profiler')
    if expect:
        args += ['--expect-pose', expect]
    p = subprocess.run([BIN[binary]] + args, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    open(os.path.join(cwd, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(cwd, 'stderr.txt'), 'wb').write(p.stderr)
    s = summary(p.stdout) or {}
    pose = os.path.join(cwd, 'pose.bin')
    sha = hashlib.sha256(open(pose, 'rb').read()).hexdigest() if os.path.isfile(pose) else None
    cfg = s.get('config') or {}
    return {'binary': binary, 'row': row['id'], 'W': w, 'steps': steps, 'tag': tag, 'args': args, 'exit': p.returncode,
            'pose_hash': s.get('pose_hash'), 'expect_pose': s.get('expect_pose'), 'pose_sha256': sha,
            'config': s.get('config'), 'broadphase': cfg.get('broadphase') if isinstance(cfg, dict) else None,
            'broadphase_tree': s.get('broadphase_tree'), 'void_steps': s.get('void_steps'),
            'drops_total': s.get('drops_total'), 'waves_total': s.get('waves_total'), 'workers': s.get('workers'),
            'target_env': s.get('target_env'), 'profile_name': s.get('profile_name'), 'armed': s.get('armed'),
            'final_manifolds': s.get('final_manifolds'), 'window_mean_ns': s.get('window_mean_ns'),
            'stderr_tail': p.stderr.decode('utf-8', 'replace')[-300:]}, pose


results = []
ok_all = True
refs = {}
# Reference-producing rows first on the parent, so every other run has a pose to be checked against.
ORDER = ['HL-D-allpairs', 'HL-D-tree', 'J-As', 'J-As-a', 'HL-R-allpairs', 'HL-R-tree']
for binary in ('parent', 'tip'):
    for rid in ORDER:
        row = BYID[rid]
        for w in (1, 8):
            ref_path = os.path.join(GATE, f'{row["pose_ref"]}_ref.pose')
            produce = binary == 'parent' and w == 1 and rid in ('HL-D-allpairs', 'HL-R-allpairs')
            rec, pose = run(binary, row, w, row['steps'], None if produce else ref_path, '')
            if produce and os.path.isfile(pose):
                open(ref_path, 'wb').write(open(pose, 'rb').read())
                refs[row['pose_ref']] = hashlib.sha256(open(ref_path, 'rb').read()).hexdigest()
            t_ok, t_rule = tree_ok(row, rec['broadphase_tree'])
            checks = {
                'exit0': rec['exit'] == 0,
                'expect_pose_match': produce or rec['expect_pose'] == 'match',
                'pose_hash': rec['pose_hash'] == row['expected_pose'],
                'tree_diag': t_ok,
                'void0': rec['void_steps'] == 0,
                'workers': rec['workers'] == w,
                'msvc': rec['target_env'] == 'msvc',
            }
            if row['armed']:
                checks['drops0'] = rec['drops_total'] == 0
            rec['checks'] = checks
            rec['tree_rule'] = t_rule
            ok = all(checks.values())
            ok_all &= ok
            results.append(rec)
            print(f'{binary:6s} {rid:14s} W={w} exit {rec["exit"]} pose {rec["pose_hash"]} expect {rec["expect_pose"]} '
                  f'bp {rec["broadphase"]} tree {json.dumps(rec["broadphase_tree"], separators=(",", ":"))} '
                  f'void {rec["void_steps"]} drops {rec["drops_total"]} waves {rec["waves_total"]} '
                  f'pose_sha {str(rec["pose_sha256"])[:8]} -> {"OK" if ok else "RED " + str([k for k, v in checks.items() if not v])}',
                  flush=True)
    # red controls on this binary
    for rid in ('HL-D-tree', 'J-As', 'HL-R-tree'):
        row = BYID[rid]
        ref_path = os.path.join(GATE, f'{row["pose_ref"]}_ref.pose')
        rec, _ = run(binary, row, 1, row['steps'] + 1, ref_path, '_red501')
        rec['red_control'] = True
        red_ok = rec['exit'] == 4 and rec['expect_pose'] != 'match'
        rec['checks'] = {'exit4_and_mismatch': red_ok}
        ok_all &= red_ok
        results.append(rec)
        print(f'{binary:6s} {rid:14s} W=1 RED CONTROL 501 steps vs 500-step {row["pose_ref"]}_ref.pose: exit {rec["exit"]} '
              f'expect_pose {rec["expect_pose"]} pose {rec["pose_hash"]} -> {"OK (red as required)" if red_ok else "NOT RED - gate cannot fail"}',
              flush=True)

# config parity parent vs tip
cfg_pairs = []
for rid in ORDER:
    for w in (1, 8):
        a = [r for r in results if r['row'] == rid and r['W'] == w and r['binary'] == 'parent' and not r.get('red_control')][0]
        b = [r for r in results if r['row'] == rid and r['W'] == w and r['binary'] == 'tip' and not r.get('red_control')][0]
        diff = sorted(k for k in set((a['config'] or {})) | set((b['config'] or {}))
                      if (a['config'] or {}).get(k) != (b['config'] or {}).get(k))
        cfg_pairs.append({'row': rid, 'W': w, 'config_equal': not diff, 'differing_keys': diff})
        print(f'config parent vs tip {rid:14s} W={w}: {"equal" if not diff else "DIFFER " + str(diff)}', flush=True)
shas = sorted({(r['row'], r['pose_sha256']) for r in results if not r.get('red_control')})
json.dump({'results': results, 'config_pairs': cfg_pairs, 'ref_sha256': refs, 'pose_sha256_by_row': shas},
          open(os.path.join(GATE, 'gate.json'), 'w', encoding='utf-8'), indent=1)
print('ref pose sha256:', json.dumps(refs))
print('pose.bin sha256 by row (non-red):', json.dumps(shas))
print('POSE GATE', 'OK' if ok_all else 'RED')
sys.exit(0 if ok_all else 1)
