"""Window 9a: every validator and rule the 9a driver relies on must be able to go RED. Imports the driver without running it
(--dry-run parse only; main() is not called), feeds each validator one REAL gate record (must pass) and mutated copies (must
fail), and prints PASS/FAIL per case. Launches only tiny untimed processes (the end-to-end re-run cases: 12-step runner cells);
reads no timing value.
  python -B tools/selftest9a.py > gate/selftest9a.txt          # all cases
  python -B tools/selftest9a.py --no-e2e                       # skip the end-to-end re-run rehearsals (~1-2 min)
  python -B tools/selftest9a.py --driver tools/_mut_window9a_run.py   # EVERY case (pure ones too) against a MUTATED copy of the driver (must FAIL)
A red counts only for its NAMED reason (only_for) where a mutation could trip an unrelated rule."""
import copy
import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
W9 = os.path.dirname(HERE)
ARGV = sys.argv[1:]
E2E = '--no-e2e' not in ARGV
DRIVER = os.path.join(HERE, 'window9a_run.py')
if '--driver' in ARGV:
    DRIVER = os.path.abspath(ARGV[ARGV.index('--driver') + 1])
sys.argv = [sys.argv[0], '--dry-run']
sys.path.insert(0, HERE)
if '--driver' in ARGV:      # audit-1: the pure cases too must see the mutated copy, or a mutation of a pure rule proves nothing
    import importlib.util
    _spec = importlib.util.spec_from_file_location('window9a_run', DRIVER)
    R = importlib.util.module_from_spec(_spec)
    sys.modules['window9a_run'] = R
    _spec.loader.exec_module(R)
else:
    import window9a_run as R  # noqa: E402

GATE = os.path.join(W9, 'gate')
D = R.D
fails = 0
ncases = 0


def case(name, why, want_red):
    global fails, ncases
    ncases += 1
    ok = bool(why) == want_red
    fails += not ok
    print(f'{"PASS" if ok else "FAIL"} {name}: {"red" if why else "green"} (want {"red" if want_red else "green"}) {why[:2]}')


def only_for(why, sub):
    """The reasons, but only if EVERY one contains `sub` (a str, or any of a tuple of strs), so a mutation cannot pass by
    tripping some other rule."""
    subs = (sub,) if isinstance(sub, str) else sub
    return why if why and all(any(s in w for s in subs) for w in why) else []


def cell_dir(rid, key, w):
    return os.path.join(GATE, 'cells', f'{rid}_{key}_W{w}')


def runner_rec(rid, key, w, summary_edit=None):
    cwd = cell_dir(rid, key, w)
    so = open(os.path.join(cwd, 'stdout.txt'), 'rb').read()
    rec = {'binary': key, 'exit': 0, 'summary': D.parse_summary(so)}
    rec.update(R.runner_stats(R.ROWS[rid], cwd))
    if summary_edit:
        rec['summary'] = summary_edit(copy.deepcopy(rec['summary']))
    return rec


def judged(rid, key, w, row_id=None, binary=None, edit=None):
    rec = runner_rec(rid, key, w, edit)
    if binary:
        rec['binary'] = binary
    return R.validate_runner(rec, R.ROWS[row_id or rid], w)


# ------------------------------------------------------------------------------------------------ 1. protocol shape
o = [('a', 'k', 1), ('b', 'k', 1)]
po = [R.pass_order(o, p) for p in range(3)]
case('pass order p0 reversed / p1 forward / p2 reversed', [] if po == [o[::-1], o, o[::-1]] else [f'got {po}'], False)
ab = R.cell_order('C4-AB')


def adjacency(cells, a, b):
    """True when the two (row, binary) cells are neighbours at every W where both exist."""
    bad = []
    for w in (1, 2, 4, 8, 16):
        idx = {(r, k): i for i, (r, k, ww) in enumerate(cells) if ww == w}
        if a in idx and b in idx and abs(idx[a] - idx[b]) != 1:
            bad.append(f'W{w}: {a} at {idx[a]}, {b} at {idx[b]}')
        if (a in idx) != (b in idx) and (a[0] != 'C4-JT' and b[0] != 'C4-JT'):
            bad.append(f'W{w}: only one of {a}, {b} exists')
    return bad


for p in range(3):
    cells = R.pass_order(ab, p)
    case(f'C4-AB pass {p}: the Jolt cell is adjacent to C4-JD#tip at every W', adjacency(cells, ('C4-jolt56', 'j56'), ('C4-JD', 'tip')), False)
    case(f'C4-AB pass {p}: the Jolt cell is adjacent to C4-JT#tip at W 1/8/16', adjacency(cells, ('C4-jolt56', 'j56'), ('C4-JT', 'tip')), False)
    case(f'C4-AB pass {p}: C4-JD#tip is adjacent to C4-JDap#tip (the same-binary merge-gate pair)',
         adjacency(cells, ('C4-JD', 'tip'), ('C4-JDap', 'tip')), False)
moved = [c for c in ab if c[0] != 'C4-jolt56'] + [c for c in ab if c[0] == 'C4-jolt56']       # Jolt cells at the end
case('adjacency check on a permuted order (Jolt cells moved to the end)', adjacency(moved, ('C4-jolt56', 'j56'), ('C4-JD', 'tip')), True)
case('C4-AB has the Jolt cell at each of W 1/2/4/8/16', [f'W{w}' for w in (1, 2, 4, 8, 16) if ('C4-jolt56', 'j56', w) not in ab], False)
case('C4-AB has no cell for a block it should not (no C4-JOLT block)', [] if 'C4-JOLT' not in [b['name'] for b in R.BLOCKS] else ['C4-JOLT'], False)
br = R.cell_order('C4-BR')
case('C4-BR round = g4r7, g4r8b, g4rT adjacent (forward order)', [] if [c[1] for c in br] == ['g4r7', 'g4r8b', 'g4rT'] else [str(br)], False)
case('blocks run in priority order AB, G5, BR, G4, G4-kd (merge gate first; BR precedes G4 as ruling 5 reads)',
     [] if [b['name'] for b in R.BLOCKS] == ['C4-AB', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-G4-kd'] else [str([b['name'] for b in R.BLOCKS])], False)
# K per block: the letter
ks = {b['name']: R.block_passes(b) * R.block_rounds(b) for b in R.BLOCKS}
case('K per cell: C4-BR 9, C4-G4 9, C4-G4-kd 3, C4-AB 9, C4-G5 9',
     [] if ks == {'C4-BR': 9, 'C4-G4': 9, 'C4-G4-kd': 3, 'C4-AB': 9, 'C4-G5': 9} else [str(ks)], False)

# ------------------------------------------------------------------------------------------------ 2. ruling 8, the pure part
def rec_of(bad):
    return {'contaminated': bad == 'contaminated', 'valid': bad != 'invalid'}


def slots_of(spec):
    """spec: list of (round, bad or None, n_rerun) -> the slots dict of run_pass."""
    return {(rnd, 'X', 'tip', 1): {'seq': rnd + 1, 'rec': rec_of(bad), 'n_rerun': n} for rnd, bad, n in spec}


s0 = slots_of([(0, None, 0), (1, None, 0), (2, None, 0)])
case('all slots clean: no re-run wave', ['pending'] if R.pending_reruns(s0) else [], False)
s1 = slots_of([(0, None, 0), (1, 'contaminated', 0), (2, None, 0)])
case('one contaminated slot: exactly that slot is pending', [] if [k[0] for k, _ in R.pending_reruns(s1)] == [1] else ['wrong'], False)
s2 = slots_of([(0, 'invalid', 0), (1, None, 0), (2, 'contaminated', 1)])
case('invalid and contaminated slots are both pending, in original order',
     [] if [k[0] for k, _ in R.pending_reruns(s2)] == [0, 2] else ['wrong'], False)
c = R.passcell_table(s1)[('X', 'tip', 1)]
case('pass-cell with one unclean slot: k_clean 2 of 3', [] if (c['clean'], c['target']) == (2, 3) else [str(c)], False)
case('pass-cell K < 3 does not gate (MIN_K_GATE)', [] if not (c['clean'] >= R.MIN_K_GATE) else ['gates'], False)
c0 = R.passcell_table(s0)[('X', 'tip', 1)]
case('pass-cell with every slot clean gates (k_clean 3 >= MIN_K_GATE)', [] if c0['clean'] >= R.MIN_K_GATE else ['does not gate'], False)


def pending_broken(slots):     # the 8b behaviour: only contaminated originals, invalid never re-run
    return [(k, s) for k, s in slots.items() if s['rec'].get('contaminated')]


case('negative control: a pending rule that ignores invalid slots differs from the real one on s2',
     [] if len(pending_broken(s2)) != len(R.pending_reruns(s2)) else ['same'], False)
case('constants: 4 waves, allowance 0.5, K gate 3', [] if (R.RERUN_MAX_WAVES, R.RERUN_STAGE_FRAC, R.MIN_K_GATE) == (4, 0.5, 3) else ['moved'], False)
case('constants: the allowance floor slack is 1.3 (the criterion estimates are good to +-30 %)', [] if R.RERUN_EST_SLACK == 1.3 else ['moved'], False)

# ---- 2b. audit-1 W1: the per-block re-run ALLOWANCE (real blocks, real estimates). "A single slot can reach the full wave depth".
def capacity_of(block, allowance, overrun=1.0):
    """The stage loop of run_pass for one always-hot slot of the block's longest cell, against a GIVEN allowance (uses the driver's own
    admission predicate, so a mutated predicate shows here)."""
    longest = max(R.est_s(rid, key, w) for rid, key, w in R.cell_order(block['name']))
    n, elapsed = 0, 0.0
    while n < 99 and R.rerun_admitted(elapsed, longest, allowance):
        elapsed += overrun * (longest + R.RECEIPT_S)
        n += 1
    return n


for blk in R.BLOCKS:
    al, fr, fl = R.rerun_allowance(blk)
    case(f'allowance {blk["name"]}: a single always-hot slot reaches the full wave depth ({R.RERUN_MAX_WAVES}) even at 25 % over its estimate',
         [] if capacity_of(blk, al, 1.25) >= R.RERUN_MAX_WAVES else [f'capacity {capacity_of(blk, al, 1.25)} at allowance {al:.0f} s'], False)
    case(f'allowance {blk["name"]}: rerun_capacity() agrees with the loop replay',
         [] if R.rerun_capacity(blk, 1.25) == capacity_of(blk, al, 1.25) else [f'{R.rerun_capacity(blk, 1.25)} vs {capacity_of(blk, al, 1.25)}'], False)
    case(f'allowance {blk["name"]} = max(fraction, floor), never below the fraction (8b/9a first cut)',
         [] if al == max(fr, fl) and al >= fr and fr == R.RERUN_STAGE_FRAC * R.block_rounds(blk) * R.block_estimate(blk)['per_round_s'] else [f'{al} {fr} {fl}'], False)
    case(f'allowance {blk["name"]}: floor = {R.RERUN_MAX_WAVES} x {R.RERUN_EST_SLACK} x the longest process + receipt',
         [] if abs(fl - R.RERUN_MAX_WAVES * R.RERUN_EST_SLACK * max(R.est_s(r, k, w) + R.RECEIPT_S for r, k, w in R.cell_order(blk['name']))) < 1e-6 else [str(fl)], False)
# the pre-fix allowance (fraction only) FAILS the same rule on the one-cell criterion blocks: the finding, and proof the rule can go red
for name in ('C4-G4', 'C4-G4-kd'):
    blk = next(x for x in R.BLOCKS if x['name'] == name)
    fr_only = R.RERUN_STAGE_FRAC * R.block_rounds(blk) * R.block_estimate(blk)['per_round_s']
    case(f'negative control: the pre-fix allowance (fraction only) on {name} admits a single re-run at 25 % over its estimate',
         [] if capacity_of(blk, fr_only, 1.25) >= R.RERUN_MAX_WAVES else [f'fraction-only capacity {capacity_of(blk, fr_only, 1.25)}'], True)
case('admission: a re-run that fits the allowance exactly is admitted', [] if R.rerun_admitted(0.0, 100.0, 100.0 + R.RECEIPT_S) else ['refused'], False)
case('admission: a re-run that overshoots the allowance by 0.1 s is refused', [] if not R.rerun_admitted(0.0, 100.0, 100.0 + R.RECEIPT_S - 0.1) else ['admitted'], False)
case('admission: elapsed time counts against the allowance', [] if not R.rerun_admitted(50.0, 100.0, 100.0 + R.RECEIPT_S) else ['admitted'], False)

# ------------------------------------------------------------------------------------------------ 3. runner validators on REAL records
have_cells = os.path.exists(cell_dir('C4-JD', 'tip', 8))
if have_cells:
    case('C4-JD tip W8 as recorded', judged('C4-JD', 'tip', 8), False)
    case('C4-JDpar parent W8 as recorded', judged('C4-JDpar', 'parent', 8), False)
    case('C4-JDap tip W8 as recorded', judged('C4-JDap', 'tip', 8), False)
    case('C4-JD tip record relabelled parent (swapped exes): the config receipt tree_brute_max_rows',
         only_for(judged('C4-JD', 'tip', 8, binary='parent'), 'tree_brute_max_rows'), True)
    case('C4-JDpar parent record relabelled tip (swapped exes): the config receipt',
         only_for(judged('C4-JDpar', 'parent', 8, binary='tip'), 'tree_brute_max_rows'), True)
    case('C4-JA tip W8 record relabelled parent (AllPairs in both: only the receipt can see the swap)',
         only_for(judged('C4-JA', 'tip', 8, binary='parent'), 'tree_brute_max_rows'), True)
    case('C4-JD tip record judged as the AllPairs row C4-JDap (the flip is visible)',
         only_for(judged('C4-JD', 'tip', 8, row_id='C4-JDap'), ('roadphase', 'TreeDiag')), True)
    case('C4-JDap tip record judged as the Tree row C4-JD', only_for(judged('C4-JDap', 'tip', 8, row_id='C4-JD'), ('roadphase', 'TreeDiag')), True)
    case('C4-JDpar (parent flagless = AllPairs) judged as the Tree row C4-JD', only_for(judged('C4-JDpar', 'parent', 8, row_id='C4-JD'), ('roadphase', 'TreeDiag', 'tree_brute_max_rows')), True)
    case('C4-JT tip W8 as recorded', judged('C4-JT', 'tip', 8), False)
    case('C4-JT parent W8 as recorded (Tree pinned by --broadphase tree in the parent)', judged('C4-JT', 'parent', 8), False)
    case('C4-JT tip record judged as C4-JA (Tree under an AllPairs row)', only_for(judged('C4-JT', 'tip', 8, row_id='C4-JA'), ('roadphase', 'TreeDiag')), True)
    case('pose hash mutated', only_for(judged('C4-JD', 'tip', 8, edit=lambda s: {**s, 'pose_hash': '0x0'}), 'pose'), True)
    case('expect_pose mismatch', only_for(judged('C4-JD', 'tip', 8, edit=lambda s: {**s, 'expect_pose': 'mismatch'}), 'expect_pose'), True)
    case('void_steps 1', only_for(judged('C4-JD', 'tip', 8, edit=lambda s: {**s, 'void_steps': 1}), 'void_steps'), True)
    case('workers != W (record of W8 judged at W4)', only_for(R.validate_runner(runner_rec('C4-JD', 'tip', 8), R.ROWS['C4-JD'], 4), 'workers'), True)
    case('TreeDiag static_rebuilds 2 on C4-JD',
         only_for(judged('C4-JD', 'tip', 8, edit=lambda s: {**s, 'broadphase_tree': {**s['broadphase_tree'], 'static_rebuilds': 2}}), 'TreeDiag'), True)
    case('kd_order_builds 1 on a default-kernel row',
         only_for(judged('C4-JD', 'tip', 8, edit=lambda s: {**s, 'broadphase_tree': {**s['broadphase_tree'], 'kd_order_builds': 1}}), 'kd_order_builds'), True)
    case('TreeDiag nonzero on the AllPairs row C4-JDap',
         only_for(judged('C4-JDap', 'tip', 8, edit=lambda s: {**s, 'broadphase_tree': {**s['broadphase_tree'], 'members': 1}}), 'TreeDiag'), True)
    case('C4-rung as recorded (canary_ns 60000)', judged('C4-rung', 'tip', 8), False)
    case('C4-rung with canary_ns null (the canary did not run)', only_for(judged('C4-rung', 'tip', 8, edit=lambda s: {**s, 'canary_ns': None}), 'canary'), True)
    case('C4-JD-armed tip W8 as recorded', judged('C4-JD-armed', 'tip', 8), False)
    case('C4-JD-armed tip W1 as recorded', judged('C4-JD-armed', 'tip', 1), False)
    case('C4-JD-armed W8 with setup_tasks 0 (S4 never dispatched)',
         only_for(judged('C4-JD-armed', 'tip', 8, edit=lambda s: {**s, 'w8s': {**s['w8s'], 'setup_tasks': 0}}), 'S4'), True)
    case('C4-JD-armed W1 with setup_tasks 5 (S4 must be inline at W1)',
         only_for(judged('C4-JD-armed', 'tip', 1, edit=lambda s: {**s, 'w8s': {**s['w8s'], 'setup_steps': 5, 'setup_tasks': 5}}), 'S4'), True)
    case('C4-JD-armed record judged as the disarmed row C4-JD', only_for(judged('C4-JD-armed', 'tip', 8, row_id='C4-JD'), 'armed'), True)
    case('C4-S16 (16 rows, brute path) as recorded', judged('C4-S16', 'tip', 1), False)
    case('C4-S16 under the default TreeDiag expectation {1,1,0} (tree_diag_expect dropped: the brute-path proof can fail)',
         only_for(R.validate_runner(runner_rec('C4-S16', 'tip', 1), dict(R.ROWS['C4-S16'], tree_diag_expect=None), 1), 'TreeDiag'), True)
    case('C4-S16 with a tree built (static_rebuilds 1)',
         only_for(judged('C4-S16', 'tip', 1, edit=lambda s: {**s, 'broadphase_tree': {**s['broadphase_tree'], 'static_rebuilds': 1, 'members': 1}}), 'TreeDiag'), True)
    # jolt
    jd = os.path.join(GATE, 'cells', 'C4-jolt56_j56_W8')
    jso = open(os.path.join(jd, 'stdout.txt'), 'rb').read()
    jrow = R.ROWS['C4-jolt56']

    def jjudge(so=jso, w=8, row=jrow):
        rec = {'exit': 0}
        rec.update(R.jolt_stats(row, jd, so))
        return R.validate_jolt(rec, row, w)
    case('C4-jolt56 j56 W8 as recorded (hash, threads, patch banner, 500 frames)', jjudge(), False)
    case('Jolt hash mutated', only_for(jjudge(jso.replace(b'0xb8522b4e3fc62cfe', b'0xb8522b4e3fc62cff')), 'hash'), True)
    case('Jolt threads != W (the W8 record judged at W4)', only_for(jjudge(w=4), 'threads'), True)
    case('Jolt patch banner allow_sleep=1 (sleeping on)', only_for(jjudge(jso.replace(b'allow_sleep=0', b'allow_sleep=1')), 'patch banner'), True)
    case('Jolt patch banner receipt=1', only_for(jjudge(jso.replace(b'receipt=0', b'receipt=1')), 'patch banner'), True)
    case('Jolt second stat line', only_for(jjudge(jso + b'\nDiscrete, 8, 1.0, 0xb8522b4e3fc62cfe\n'), 'stat lines'), True)
    case('Jolt row expecting another hash', only_for(jjudge(row=dict(jrow, jolt_hash='0x1')), 'hash'), True)
else:
    print('SKIP runner/jolt cases: gate/cells has not run')

# ------------------------------------------------------------------------------------------------ 4. binaries and fixtures
case('verify_binaries as written (SHA256SUMS + sha256_pin)', R.verify_binaries(), False)
saved = {k: dict(R.BINS[k]) for k in ('tip', 'parent')}
for k, o2 in (('tip', 'parent'), ('parent', 'tip')):
    R.BINS[k].update({'exe': saved[o2]['exe'], 'path': saved[o2]['path'], 'sha256': R.SUMS.get(saved[o2]['exe'])})
case('verify_binaries with the tip/parent exes swapped (pins kept)', R.verify_binaries(), True)
for k in ('tip', 'parent'):
    R.BINS[k].pop('sha256_pin', None)
case('the same swap without the pins (SHA256SUMS alone cannot see it: the pin is load-bearing)', R.verify_binaries(), False)
for k in ('tip', 'parent'):
    R.BINS[k].clear()
    R.BINS[k].update(saved[k])
case('verify_binaries restored', R.verify_binaries(), False)
case('every j56 / g4r7 / g4r8b / g4rT / tip / parent carries a sha256_pin',
     [k for k, e in R.BINS.items() if not e.get('sha256_pin')], False)
case('verify_fixtures as written (JT500 JA500 RT500 S16500)', R.verify_fixtures(), False)
sf = R.FIXTURES.pop('S16500', None)
case('verify_fixtures with S16500 unrecorded', R.verify_fixtures(), True)
R.FIXTURES['S16500'] = sf
# every runner row has a pose gate whose fixture hash is one of the four known
case('every runner row names a recorded pose_ref', [r for r, row in R.ROWS.items() if row['kind'] == 'runner' and row.get('pose_ref') not in R.FIXTURES], False)

# ------------------------------------------------------------------------------------------------ 5. criterion
cs_root = os.path.join(GATE, 'criterion_sample')
sample_dirs = {(r, k): os.path.join(cs_root, f'{r}_{k}') for r in ('C4-BR', 'C4-G4', 'C4-G4-kd') for k in R.ROWS[r]['binaries']}
if all(os.path.exists(os.path.join(d, 'stdout.txt')) for d in sample_dirs.values()):
    for (rid, key), d in sample_dirs.items():
        so = open(os.path.join(d, 'stdout.txt'), 'rb').read()
        se = open(os.path.join(d, 'stderr.txt'), 'rb').read()
        row = R.ROWS[rid]
        case(f'{rid} {key} full sample as recorded', R.criterion_check({'exit': 0}, row, so, se, d), False)
        r2 = dict(row, expect=row['expect'] + [row['expect'][0].rsplit('/', 1)[0] + '/9999'])
        case(f'{rid} {key} with one id the run lacks', only_for(R.criterion_check({'exit': 0}, r2, so, se, d), 'no result'), True)
        r3 = dict(row, expect=row['expect'][1:])
        case(f'{rid} {key} with one unexpected id', only_for(R.criterion_check({'exit': 0}, r3, so, se, d), 'unexpected'), True)
    d0 = sample_dirs[('C4-G4', 'g4rT')]
    so = open(os.path.join(d0, 'stdout.txt'), 'rb').read()
    se = open(os.path.join(d0, 'stderr.txt'), 'rb').read()
    case('C4-G4 sample judged as the kd row (36 ids under a 6-id expectation)',
         only_for(R.criterion_check({'exit': 0}, R.ROWS['C4-G4-kd'], so, se, d0), ('unexpected', 'no result')), True)
    case('C4-G4 sample with exit 4', only_for(R.criterion_check({'exit': 4}, R.ROWS['C4-G4'], so, se, d0), 'exit'), True)
    case('C4-G4: 36 = 2 families x 2 kernels x 9 sizes (96..160 step 8), all in the 28-size patch',
         [] if len(R.ROWS['C4-G4']['expect']) == 36 and {int(i.rsplit('/', 1)[1]) for i in R.ROWS['C4-G4']['expect']} == set(range(96, 161, 8)) else ['wrong'], False)
else:
    print('SKIP criterion cases: gate/criterion_sample has not run for every (row, exe)')

# ------------------------------------------------------------------------------------------------ 6. the reserved Rapier slot
RAP = 'D:/tmp/rapier-parity/gate/bin/rapier-parity-simd8.exe'
if os.path.exists(RAP):
    tmp = os.path.join(GATE, 'selftest_tmp', 'rapier')
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp)
    p = subprocess.run([RAP, '--cfg', 'matched', '--workers', '2', '--steps', '20', '--window', '5..20', '--csv', os.path.join(tmp, 'run.csv'),
                        '--pose-out', os.path.join(tmp, 'pose.bin'), '--label', 'selftest'], cwd=tmp, capture_output=True)
    rrow = {'id': 'RAP-t', 'kind': 'rapier', 'steps': 20, 'window': [5, 20], 'metric_windows': [], 'armed': False, 'pose_ref': None,
            'expect_summary': {'engine': 'rapier3d', 'cfg': 'matched', 'workers': '$W', 'target_env': 'msvc', 'void': False, 'voids': [],
                               'threads.pool_threads_min': '$W', 'threads.pool_threads_max': '$W'},
            'expect_present': ['pose_hash', 'receipt_gates']}
    rrec = {'exit': p.returncode, 'summary': D.parse_summary(p.stdout)}
    rrec.update(R.runner_stats(rrow, tmp))
    case('rapier harness sample (exit 0, 15-step window) as its row', R.validate_rapier(rrec, rrow, 2), False)
    case('rapier sample judged at W4 (workers and pool-thread receipts are "$W")', only_for(R.validate_rapier(rrec, rrow, 4), 'summary.'), True)
    case('rapier row expecting cfg rapier-default', only_for(R.validate_rapier(rrec, dict(rrow, expect_summary={**rrow['expect_summary'], 'cfg': 'rapier-default'}), 2), 'cfg'), True)
    case('rapier row expecting an unproduced pose_hash (the placeholder cannot pass)',
         only_for(R.validate_rapier(rrec, dict(rrow, expect_summary={**rrow['expect_summary'], 'pose_hash': '<FILL>'}), 2), 'pose_hash'), True)
    case('rapier row requiring an absent summary key', only_for(R.validate_rapier(rrec, dict(rrow, expect_present=['no.such.key']), 2), 'missing'), True)
    case('rapier record with exit 3 (void)', only_for(R.validate_rapier(dict(rrec, exit=3), rrow, 2), 'exit'), True)
    case('rapier dotted lookup: a missing step is None, never equal to a value', [] if R.dotted({'a': {'b': 1}}, 'a.c') is None else ['x'], False)
else:
    print('SKIP rapier cases: the harness binary is absent')

# ------------------------------------------------------------------------------------------------ 7. end-to-end re-run rule
E2E_N = [0]


def e2e(name, hot, want, allowance=None, rounds=3, virtual=None):
    """Run the driver in --test on the RERUN-T overlay (one runner cell, 3 rounds, real 12-step processes) with the injection
    `hot` and check the records: total processes, re-run numbers, the pass-cell record and the log line."""
    E2E_N[0] += 1
    raw = os.path.join(GATE, 'selftest_tmp', f'e2e_{E2E_N[0]}')
    shutil.rmtree(raw, ignore_errors=True)
    os.makedirs(raw)
    e = dict(os.environ, W9A_EXTRA=os.path.join(W9, 'test', 'rerun_overlay', 'rows9a.extra.json'), W9A_TEST_RAW=raw,
             W9A_TEST_HOT=hot)
    if allowance is not None:
        e['W9A_TEST_ALLOWANCE_S'] = str(allowance)
    if virtual is not None:      # the REAL per-block allowance stays; every re-run is charged `virtual` seconds (driver: stage_elapsed_s)
        e['W9A_TEST_RERUN_S'] = str(virtual)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'W9A_TEST_HOT_UNUSED'):
        e.pop(k, None)
    p = subprocess.run([sys.executable, '-B', DRIVER, '--test', '--blocks', 'RERUN-T', '--test-rounds', str(rounds)], env=e,
                       capture_output=True, text=True, cwd=W9, timeout=600)
    rp = os.path.join(raw, 'runs.jsonl')
    recs = [json.loads(l) for l in open(rp, encoding='utf-8')] if os.path.exists(rp) else []
    procs = [r for r in recs if 'row' in r and r.get('attempt') != 'warmup']
    pcs = [r for r in recs if r.get('passcell')]
    got = {'exit': p.returncode, 'processes': len(procs), 'reruns': sorted((r['round'], r['rerun_no']) for r in procs if r['attempt'] == 'rerun'),
           'invalid': sum(1 for r in procs if r.get('invalid')),
           'passcell': [(c['k_clean'], c['k_target'], c['reruns'], c['gates'], c['short']) for c in pcs]}
    wlog = open(os.path.join(raw, 'window_log.txt'), encoding='utf-8').read() if os.path.exists(os.path.join(raw, 'window_log.txt')) else ''
    if 'wave_log' in want:
        got['wave_log'] = want['wave_log'] in wlog
    diff = [f'{k}: got {got.get(k)} want {v}' for k, v in want.items() if k != 'wave_log' and got.get(k) != v]
    if 'wave_log' in want and not got['wave_log']:
        diff.append(f'log lacks {want["wave_log"]!r}')
    case(f'e2e re-run rule: {name}', diff, False)
    return raw


if E2E:
    e2e('no injection: 3 processes, no re-run, K 3 of 3',
        'RR-a#tip@W1:0:0', {'exit': 0, 'processes': 3, 'reruns': [], 'invalid': 0, 'passcell': [(3, 3, 0, True, False)]})
    e2e('round 1 hot once: one re-run clean -> K 3 of 3',
        'RR-a#tip@W1:1:1', {'exit': 0, 'processes': 4, 'reruns': [(1, 1)], 'passcell': [(3, 3, 1, True, False)]})
    e2e('round 1 hot twice: the SECOND re-run (ruling 8: not the single re-run of 8b) makes it clean',
        'RR-a#tip@W1:1:2', {'exit': 0, 'processes': 5, 'reruns': [(1, 1), (1, 2)], 'passcell': [(3, 3, 2, True, False)]})
    raw4 = e2e('round 1 never clean: 4 waves then the stage ends, K 2 of 3 does not gate',
               'RR-a#tip@W1:1:99', {'exit': 0, 'processes': 7, 'reruns': [(1, 1), (1, 2), (1, 3), (1, 4)],
                                    'passcell': [(2, 3, 4, False, True)], 'wave_log': '4 waves spent'})
    e2e('two slots hot (round 0 once, round 2 three times): a clean slot leaves the stage (r0 is not re-run in wave 2)',
        'RR-a#tip@W1:0:1;RR-a#tip@W1:2:3', {'exit': 0, 'processes': 7, 'reruns': [(0, 1), (2, 1), (2, 2), (2, 3)],
                                             'passcell': [(3, 3, 4, True, False)]})
    e2e('allowance spent: 0 s allowance -> no re-run at all, K 2 of 3',
        'RR-a#tip@W1:1:99', {'exit': 0, 'processes': 3, 'reruns': [], 'passcell': [(2, 3, 0, False, True)], 'wave_log': 'allowance spent'},
        allowance=0.0)
    # audit-1 W1, end to end under the REAL allowance of the one-cell block RERUN-T (est 5.4 s -> fraction 1.5 processes, floor 5.2):
    # every re-run is charged exactly its own estimate. Fraction-only (the pre-fix loop) admits ONE re-run and drops the slot.
    rr_steps = json.load(open(os.path.join(W9, 'test', 'rerun_overlay', 'rows9a.extra.json'), encoding='utf-8'))['rows'][0]['steps']
    xr = 0.01 * rr_steps + R.LAUNCH_OVERHEAD_S + 0.5      # RR-a's est_s (no estimates.json entry: 0.01 x steps + launch) + the --test receipt (0.5 s)
    e2e('REAL allowance, one-cell block: round 1 hot 3 times -> 3 re-runs (the pre-fix fraction-only allowance stops after 1), K 3 of 3',
        'RR-a#tip@W1:1:3', {'exit': 0, 'processes': 6, 'reruns': [(1, 1), (1, 2), (1, 3)], 'passcell': [(3, 3, 3, True, False)]},
        virtual=xr)
    e2e('REAL allowance, one-cell block: never clean -> the WAVE cap (4) ends the stage, not the allowance (floor admits 5)',
        'RR-a#tip@W1:1:99', {'exit': 0, 'processes': 7, 'reruns': [(1, 1), (1, 2), (1, 3), (1, 4)], 'passcell': [(2, 3, 4, False, True)],
                             'wave_log': '4 waves spent'}, virtual=xr)
    e2e('REAL allowance, three slots hot once each: wave 1 re-runs all three (3 x its estimate fits the floor), K 3 of 3',
        'RR-a#tip@W1:0:1;RR-a#tip@W1:1:1;RR-a#tip@W1:2:1',
        {'exit': 0, 'processes': 6, 'reruns': [(0, 1), (1, 1), (2, 1)], 'passcell': [(3, 3, 3, True, False)]}, virtual=xr)
    # counts9a.py end to end on the never-clean scenario's records: it must print the SHORT line and "does not gate"
    out = subprocess.run([sys.executable, '-B', os.path.join(HERE, 'counts9a.py'), raw4], capture_output=True, text=True)
    hit = [l for l in out.stdout.splitlines() if l.startswith('SHORT') and 'does not gate' in l]
    case(f'counts9a.py on the never-clean records (exit {out.returncode}): a SHORT ... does not gate line', [] if hit else ['no SHORT line'], False)
    pcl = [l for l in out.stdout.splitlines() if l.startswith('pass-cells (ruling 8)')]
    case('counts9a.py pass-cell census line: 1 cell, short 1, NOT gating 1, 4 re-runs, deepest 4',
         [] if pcl and 'short 1' in pcl[0] and 'NOT gating 1' in pcl[0] and 're-runs spent 4' in pcl[0] and 'deepest re-run 4' in pcl[0] else (pcl or ['no pass-cells line at all']), False)
    inc = [l for l in out.stdout.splitlines() if l.startswith('INCOMPLETE')]
    case('counts9a.py on a COMPLETE pass (pass_done written): no INCOMPLETE line', [] if not inc else inc, False)
    # a pass with process records and no pass_done / voided_pass record (a cut): counts9a must say INCOMPLETE; with pass_done, not
    rr = os.path.join(GATE, 'selftest_tmp', 'counts_incomplete')
    shutil.rmtree(rr, ignore_errors=True)
    os.makedirs(rr)
    base = json.loads([l for l in open(os.path.join(raw4, 'runs.jsonl'), encoding='utf-8') if '"row"' in l and '"passcell"' not in l][0])
    with open(os.path.join(rr, 'runs.jsonl'), 'w', encoding='utf-8') as f:
        f.write(json.dumps(dict(base, block='C4-CUT', run_tag='111111', **{'pass': 2, 'pass_attempt': 0})) + '\n')
        f.write(json.dumps(dict(base, block='C4-OK', run_tag='111111', **{'pass': 0, 'pass_attempt': 0})) + '\n')
        f.write(json.dumps({'pass_done': True, 'run_tag': '111111', 'block': 'C4-OK', 'pass': 0, 'pass_attempt': 0}) + '\n')
    o2 = subprocess.run([sys.executable, '-B', os.path.join(HERE, 'counts9a.py'), rr], capture_output=True, text=True).stdout.splitlines()
    inc2 = [l for l in o2 if l.startswith('INCOMPLETE')]
    case('counts9a.py: a pass without pass_done is reported INCOMPLETE (C4-CUT-p2), the closed pass (C4-OK-p0) is not',
         [] if len(inc2) == 1 and inc2[0].startswith('INCOMPLETE C4-CUT-p2') else (inc2 or ['no INCOMPLETE line at all']), False)
    case('counts9a.py: the INCOMPLETE line says a cut re-run stage leaves no passcell records and --resume re-runs the pass whole',
         [] if inc2 and 'NO passcell records' in inc2[0] and '--resume re-runs the pass whole' in inc2[0] else (inc2 or ['no INCOMPLETE line at all']), False)
else:
    print('SKIP end-to-end re-run cases (--no-e2e)')

# ------------------------------------------------------------------------------------------------ 8. overlay placement (audit-1 W2)
# Where does an overlay row land? Real merged output of the driver's own --dry-run (W9A_EXTRA overlay), never a claim about it.
import re  # noqa: E402

CELL_RE = re.compile(r'([A-Za-z0-9_\-]+)#([A-Za-z0-9_]+)@W(\d+)\(')


def dry(overlay_obj, name):
    """The driver's --dry-run over rows9a.json + a generated overlay -> (returncode, stdout, stderr)."""
    tmp = os.path.join(GATE, 'selftest_tmp', 'placement')
    os.makedirs(tmp, exist_ok=True)
    path = os.path.join(tmp, name + '.json')
    with open(path, 'w', encoding='utf-8') as f:
        json.dump(overlay_obj, f)
    p = subprocess.run([sys.executable, '-B', DRIVER, '--dry-run', '--start', '20:00', '--cutoff', '02:32'],
                       env=dict(os.environ, W9A_EXTRA=path), capture_output=True, text=True, cwd=W9, timeout=120)
    return p.returncode, p.stdout, p.stderr


def dry_pass(stdout, block, pass_no):
    """The cell list (rid, key, W) of one pass of one block, from the '  pass N: ... each round: ...' line."""
    lines = stdout.splitlines()
    i = next(k for k, l in enumerate(lines) if l.startswith(f'[{block} prio'))
    j = next(k for k in range(i + 1, len(lines)) if lines[k].startswith(f'  pass {pass_no}'))
    return [(r, k, int(w)) for r, k, w in CELL_RE.findall(lines[j])]


def dry_blocks(stdout):
    return re.findall(r'^\[(\S+) prio', stdout, flags=re.M)


base_ov = json.load(open(os.path.join(W9, 'test', 'rapier_overlay', 'rows9a.extra.json'), encoding='utf-8'))


def in_ab(insert_after=None):
    ov = json.loads(json.dumps(base_ov))
    ov['blocks'] = []
    ov['rows'][0]['blocks'] = ['C4-AB']
    if insert_after:
        ov['rows'][0]['insert_after'] = insert_after
    return ov


def neighbours(cells, rid, w):
    """(left, right) of the row's cell at W in a flat cell list, as (row, key) or None at the ends."""
    i = [k for k, c in enumerate(cells) if c[0] == rid and c[2] == w][0]
    return ((cells[i - 1][0], cells[i - 1][1]) if i > 0 else None,
            (cells[i + 1][0], cells[i + 1][1]) if i + 1 < len(cells) else None)


def index_of(cells, rid, key, w):
    return [k for k, c in enumerate(cells) if c == (rid, key, w)][0]


rc, so, se = dry(in_ab(), 'ab_tail')
tail = dry_pass(so, 'C4-AB', 1)
case('overlay row in C4-AB with no insert_after: dry-run rc 0 and the row has a cell at each of W 1/2/4/8/16',
     [] if rc == 0 and sorted(c[2] for c in tail if c[0] == 'RAP-matched') == [1, 2, 4, 8, 16] else [f'rc {rc} {se[-200:]}'], False)
case('overlay row appended (no insert_after): it is the LAST cell of its W group, after C4-JDpar#parent at W 1/2/4',
     [f'W{w}: {neighbours(tail, "RAP-matched", w)}' for w in (1, 2, 4) if neighbours(tail, 'RAP-matched', w)[0] != ('C4-JDpar', 'parent')], False)
case('overlay row appended: after C4-rung#tip at W 8/16',
     [f'W{w}: {neighbours(tail, "RAP-matched", w)}' for w in (8, 16) if neighbours(tail, 'RAP-matched', w)[0] != ('C4-rung', 'tip')], False)
case('overlay row appended is NOT adjacent to C4-JD#tip at any W (the claim the first cut of PREP.md made; audit-1 W2)',
     [f'W{w}' for w in (1, 2, 4, 8, 16) if ('C4-JD', 'tip') in neighbours(tail, 'RAP-matched', w)], False)
rc, so, se = dry(in_ab('C4-jolt56'), 'ab_after_jolt')
aj = dry_pass(so, 'C4-AB', 1)
case('insert_after C4-jolt56: the row sits between C4-jolt56#j56 and C4-JD#tip at EVERY W (adjacent to J-D each round)',
     [f'W{w}: {neighbours(aj, "RAP-matched", w)}' for w in (1, 2, 4, 8, 16)
      if neighbours(aj, 'RAP-matched', w) != (('C4-jolt56', 'j56'), ('C4-JD', 'tip'))], False)
aj0 = dry_pass(so, 'C4-AB', 0)      # pass 0 is reversed: the same neighbours, mirrored
case('insert_after C4-jolt56, reversed pass 0: still between C4-JD#tip and C4-jolt56#j56 at every W',
     [f'W{w}: {neighbours(aj0, "RAP-matched", w)}' for w in (1, 2, 4, 8, 16)
      if neighbours(aj0, 'RAP-matched', w) != (('C4-JD', 'tip'), ('C4-jolt56', 'j56'))], False)
case('insert_after C4-jolt56 costs the Jolt / J-D adjacency (stated in PREP.md): Jolt and J-D are no longer neighbours',
     [f'W{w}' for w in (1, 2, 4, 8, 16)
      if abs(index_of(aj, 'C4-jolt56', 'j56', w) - index_of(aj, 'C4-JD', 'tip', w)) == 1], False)
case('insert_after C4-jolt56: Jolt keeps its C4-JT#tip neighbour at W 1/8/16 (the example note says so)',
     [f'W{w}' for w in (1, 8, 16) if abs(index_of(aj, 'C4-jolt56', 'j56', w) - index_of(aj, 'C4-JT', 'tip', w)) != 1], False)
case('insert_after C4-jolt56: at W 2/4 Jolt is next to no C4-JD / C4-JT cell (the example note says so)',
     [f'W{w}' for w in (2, 4) if {neighbours(aj, 'C4-jolt56', w)[0][0] if neighbours(aj, 'C4-jolt56', w)[0] else None,
                                  neighbours(aj, 'C4-jolt56', w)[1][0] if neighbours(aj, 'C4-jolt56', w)[1] else None} & {'C4-JD', 'C4-JT'}], False)
rc, so, se = dry(in_ab('C4-JD'), 'ab_after_jd')
ad = dry_pass(so, 'C4-AB', 1)
case('insert_after C4-JD: between C4-JD#tip and C4-JDap#tip at every W (it costs the merge-gate pair adjacency)',
     [f'W{w}: {neighbours(ad, "RAP-matched", w)}' for w in (1, 2, 4, 8, 16)
      if neighbours(ad, 'RAP-matched', w) != (('C4-JD', 'tip'), ('C4-JDap', 'tip'))], False)
rc, so, se = dry(in_ab('C4-NO-SUCH-ROW'), 'ab_bad_anchor')
case('insert_after naming a row that does not exist is refused (exit != 0, the anchor named)',
     [] if rc != 0 and 'C4-NO-SUCH-ROW' in se else [f'rc {rc} {se[-200:]}'], False)
ov_replace = in_ab()
ov_replace['rows'][0]['id'] = 'C4-JD'
ov_replace['rows'][0]['insert_after'] = 'C4-JA'
rc, so, se = dry(ov_replace, 'ab_replace_with_anchor')
case('insert_after on a row that REPLACES an existing id is refused (it only positions a new row)',
     [] if rc != 0 and 'insert_after' in se else [f'rc {rc} {se[-200:]}'], False)
ov45 = json.loads(json.dumps(base_ov))
case('an own block at priority 4.5 runs AFTER C4-G4 (audit-1 W2: the first cut of PREP.md said "before C4-G4")',
     [] if dry_blocks(dry(ov45, 'own_45')[1]) == ['C4-AB', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-RAPIER', 'C4-G4-kd'] else ['order'], False)
ov35 = json.loads(json.dumps(base_ov))
ov35['blocks'][0]['priority'] = 3.5
case('an own block at priority 3.5 runs after C4-BR and BEFORE C4-G4',
     [] if dry_blocks(dry(ov35, 'own_35')[1]) == ['C4-AB', 'C4-G5', 'C4-BR', 'C4-RAPIER', 'C4-G4', 'C4-G4-kd'] else ['order'], False)
ov_ex = json.load(open(os.path.join(W9, 'rows9a.extra.example.json'), encoding='utf-8'))
case('rows9a.extra.example.json: its own block sits at priority 3.5 (after C4-BR, before C4-G4), as PREP.md says',
     [] if [b for b in ov_ex['blocks'] if b['name'] == 'C4-RAPIER'][0]['priority'] == 3.5 else [str(ov_ex['blocks'])], False)

print(f'selftest9a: {ncases} cases, {fails} failing')
sys.exit(1 if fails else 0)
