"""Mutation proof for the Rapier validator and block (2026-09-30). Each entry is ONE textual mutation of tools/window9a_run.py (the text must
occur exactly once). For every mutation a copy tools/_mut_rapier.py is written and BOTH gates are run against it:
  * tools/selftest9a.py --no-e2e --driver tools/_mut_rapier.py   (every case, the pure ones too, sees the mutated copy)
  * tools/xcheck_rapier9a.py --driver tools/_mut_rapier.py       (the driver's validator vs the harness's reference validator)
A mutation is CAUGHT when either gate goes red. The unmutated driver must be green in both (the control line). The copy is removed after.
  python -B logs/mutate_rapier.py > logs/mutate_rapier.out"""
import os
import re
import subprocess
import sys

W9 = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(W9, 'tools', 'window9a_run.py')
MUT = os.path.join(W9, 'tools', '_mut_rapier.py')
src = open(SRC, encoding='utf-8').read()

M = [
    ('V1 exit code ignored', "    if rc != 0:\n        flag('V1', f'exit code {rc}')", "    if False:\n        flag('V1', f'exit code {rc}')"),
    ('V1 hang ignored', "    if timed_out:\n        flag('V1', 'hang: terminated by the driver')", "    if False:\n        flag('V1', 'hang: terminated by the driver')"),
    ('V2 needs >= 1 SUMMARY line (two lines pass)', "    if len(lines) != 1:\n        flag('V2'", "    if len(lines) < 1:\n        flag('V2'"),
    ('V3 CSV pool_threads clause dropped', "any(int(c[2]) != workers for c in cells)", "False"),
    ('V3 install_receipt dropped', "    if th.get('install_receipt') != [workers, workers]:", "    if False:"),
    ('V3 loop_on_pool_worker dropped', "    if th.get('loop_on_pool_worker') is not True:", "    if False:"),
    ('V3 pool_threads_max not compared', "_is_int(th.get('pool_threads_min'), workers) and _is_int(th.get('pool_threads_max'), workers)",
     "_is_int(th.get('pool_threads_min'), workers)"),
    ('V4 features not compared', "    if s.get('features') != ap['features']:", "    if False:"),
    ('V4 exe sha256 not compared', "    if exe_sha256 != pins['exe_sha256'][arm]:", "    if False:"),
    ('V4 arm label / lanes not compared', "    if s.get('arm') != arm or not _is_int(s.get('simd_lanes'), ap['lanes']):", "    if False:"),
    ('V4 source hashes not compared', "    if s.get('source_fnv1a64') != pins['source_fnv1a64']:", "    if False:"),
    ('V4 rapier version not compared', "    if s.get('engine') != 'rapier3d' or s.get('rapier_version') != pins['rapier_version']:", "    if False:"),
    ('V5 config object not compared', "    if conf != cp['config_timed']:", "    if False:"),
    ('V5 counters_enabled ignored', "    if conf != cp['config_timed']:",
     "    if {k: v for k, v in conf.items() if k != 'counters_enabled'} != {k: v for k, v in cp['config_timed'].items() if k != 'counters_enabled'}:"),
    ('V5 install/cfg labels not compared', "    if s.get('cfg') != cfg or s.get('install') != 'loop':", "    if False:"),
    ('V6 spawn hash not compared', "    if si.get('spawn_hash') != pins['spawn_hash']:", "    if False:"),
    ('V6 scene identity not compared', "    if not ident:", "    if False:"),
    ('V6 void flag not read', "    if s.get('void') is not False:", "    if False:"),
    ('V7 pose bytes not compared', "        elif pose_raw != fixture_raw:", "        elif False:"),
    ('V7 harness verdict / pose_hash not read', "    if not test and (s.get('expect_pose') != 'match' or s.get('pose_hash') != cp['pose_hash']):", "    if False:"),
    ('V7 test flag skips everything (V1 included)', "    if timed_out:\n        flag('V1', 'hang: terminated by the driver')",
     "    if test:\n        return {}\n    if timed_out:\n        flag('V1', 'hang: terminated by the driver')"),
    ('V8 wall_ns >= 0 accepted', "any(int(c[1]) <= 0 for c in cells)", "any(int(c[1]) < 0 for c in cells)"),
    ('V8 row count not checked', "    if len(cells) != steps:\n        flag('V8'", "    if False:\n        flag('V8'"),
    ('V8 header not checked', "    if head != RAPIER_CSV_HEADER:", "    if False:"),
    ('V9 --receipt not read', "    if '--receipt' in sargs or '--receipt' in (launch_args or []) or rg.get('awake_min') is not None:", "    if False:"),
    ('V9 twin verdict not read', "    if cp['twin_ok'].get(str(workers)) is not True:", "    if False:"),
    ('validate_rapier judges against a fixed fixture (RD8)', "    ref = pose_ref_of(row, rec.get('binary'))\n    fixture", "    ref = 'RD8'\n    fixture"),
    ('validate_rapier ignores rec.hang', "bool(rec.get('hang')), rec.get('args'), steps_of(row), TEST)", "False, rec.get('args'), steps_of(row), TEST)"),
    ('validate_rapier ignores rec.exit', "exe_sha, rec.get('exit'),", "exe_sha, 0,"),
    ('validate_rapier trusts a cached exe sha (never hashes the file)', "    exe_sha = D.sha256(path) if os.path.exists(path) else 'MISSING'",
     "    exe_sha = RAPIER_PINS['exe_sha256'][arm]"),
    ('validate_rapier reads the arm from the row, not the binary', "    arm = (BINS.get(key) or {}).get('arm')", "    arm = 'simd8'"),
    ('verify_rapier_pins accepts any pins sha', "    if got != RAPIER_PINS_SHA:", "    if False:"),
    ('verify_rapier_pins accepts any fixture sha', "        if not os.path.exists(fpath) or D.sha256(fpath) != cp['fixture_sha256']:", "        if not os.path.exists(fpath):"),
    ('verify_rapier_pins ignores the fixtures.json hash', "        if (FIXTURES.get(pose_ref_of(row, key)) or {}).get('hash') != cp['pose_hash']:", "        if False:"),
    ('warmups_of keeps only the first warm-up', "        return [tuple(x) for x in block['warmups']]", "        return [tuple(block['warmups'][0])]"),
    ('block_estimate prices no warm-up', "    warm = sum(est_s(wr, wk, ww) + RECEIPT_S for wr, wk, ww in warmups_of(block))", "    warm = 0.0"),
    ('rapier hang limit is 600+', "HANG_S_RAPIER = 300.0", "HANG_S_RAPIER = 900.0"),
    ('VOID_NAMES lacks the harness exes', "                                   'rapier-parity-simd8.exe', 'rapier-parity-simd4.exe'}", "                                   }"),
    ('runner_args passes the pose of the first binary for every binary', "        args += ['--expect-pose', pose_path(row, key)]", "        args += ['--expect-pose', pose_path(row, row['binaries'][0])]"),
]


# Mutations only the END-TO-END selftest cases can see (the control flow of run_pass / run_one): run with the full selftest.
M_E2E = [
    ('run_pass runs only the first warm-up of a block', "    for wr, wk, ww in warmups_of(block):\n        ensure_time(est_s(wr, wk, ww) + RECEIPT_S, f'{ptag} warm-up {wr}#{wk}')",
     "    for wr, wk, ww in warmups_of(block)[:1]:\n        ensure_time(est_s(wr, wk, ww) + RECEIPT_S, f'{ptag} warm-up {wr}#{wk}')"),
    ('run_one never applies validate_rapier to a real process', "        rec['invalid'] = validate_rapier(rec, row, w)", "        rec['invalid'] = []"),
    ('run_one applies the RUNNER validator to a rapier record', "        rec['invalid'] = validate_rapier(rec, row, w)", "        rec['invalid'] = validate_runner(rec, row, w)"),
]


def run(driver, e2e=False):
    a = subprocess.run([sys.executable, '-B', os.path.join(W9, 'tools', 'selftest9a.py'), *([] if e2e else ['--no-e2e']), '--driver', driver],
                       capture_output=True, text=True, cwd=W9, timeout=1800)
    fails = [l for l in a.stdout.splitlines() if l.startswith('FAIL')]
    b = subprocess.run([sys.executable, '-B', os.path.join(W9, 'tools', 'xcheck_rapier9a.py'), '--driver', driver], capture_output=True, text=True, cwd=W9, timeout=900)
    m = re.search(r'(\d+) comparisons, (\d+) disagreeing', b.stdout)
    dis = int(m.group(2)) if m else -1
    return a.returncode, len(fails), fails, b.returncode, dis, (a.stderr[-300:] + b.stderr[-300:])


rc = run(SRC)
print(f'CONTROL (unmutated driver): selftest exit {rc[0]} ({rc[1]} FAIL), xcheck exit {rc[3]} ({rc[4]} disagreeing)')
assert rc[0] == 0 and rc[3] == 0, 'the unmutated driver must be green in both gates'
caught = 0
for name, old, new in M:
    n = src.count(old)
    if n != 1:
        print(f'SKIP {name}: the target text occurs {n} times')
        continue
    open(MUT, 'w', encoding='utf-8', newline='\n').write(src.replace(old, new))
    try:
        r = run(MUT)
    finally:
        pass
    hit = r[0] != 0 or r[3] != 0
    caught += hit
    first = (r[2][0][:150] if r[2] else '')
    print(f'{"CAUGHT" if hit else "NOT CAUGHT":10s} {name}: selftest exit {r[0]} ({r[1]} FAIL), xcheck exit {r[3]} ({r[4]} disagreeing) {first} {r[5] if (r[0] not in (0, 1) or r[3] not in (0, 1)) else ""}')
if os.path.exists(MUT):
    os.remove(MUT)
for name, old, new in M_E2E:
    n = src.count(old)
    if n != 1:
        print(f'SKIP {name}: the target text occurs {n} times')
        continue
    open(MUT, 'w', encoding='utf-8', newline='\n').write(src.replace(old, new))
    r = run(MUT, e2e=True)
    hit = r[0] != 0 or r[3] != 0
    caught += hit
    print(f'{"CAUGHT" if hit else "NOT CAUGHT":10s} [e2e] {name}: selftest exit {r[0]} ({r[1]} FAIL), xcheck exit {r[3]} ({r[4]} disagreeing) {(r[2][0][:150] if r[2] else "")}')
if os.path.exists(MUT):
    os.remove(MUT)
print(f'mutation proof: {caught} of {len(M) + len(M_E2E)} mutations caught by at least one gate')
