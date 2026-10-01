"""Window 9a: every validator and rule the 9a driver relies on must be able to go RED. Imports the driver without running it
(--dry-run parse only; main() is not called), feeds each validator one REAL gate record (must pass) and mutated copies (must
fail), and prints PASS/FAIL per case. Launches only tiny untimed processes (the end-to-end re-run cases: 12-step runner cells);
reads no timing value.
  python -B tools/selftest9a.py > gate/selftest9a.txt          # all cases
  python -B tools/selftest9a.py --no-e2e                       # skip the end-to-end re-run rehearsals (~1-2 min)
  python -B tools/selftest9a.py --driver tools/_mut_window9a_run.py   # EVERY case (pure ones too) against a MUTATED copy of the driver (must FAIL)
A red counts only for its NAMED reason (only_for) where a mutation could trip an unrelated rule.
2026-09-30 (the Rapier block C4-RAPIER, section 6): its structure and pins, the V1-V9 validator on the 20 real gate cells and on mutated copies,
an end-to-end --test rehearsal of the block (section 7), and the overlay placement cases over a GENERATED overlay (section 8)."""
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
case('blocks run in priority order AB, RAPIER, G5, BR, G4, G4-kd (merge gate first; Rapier after C4-AB and before C4-G5; BR precedes G4 as ruling 5 reads)',
     [] if [b['name'] for b in R.BLOCKS] == ['C4-AB', 'C4-RAPIER', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-G4-kd'] else [str([b['name'] for b in R.BLOCKS])], False)
# K per block: the letter
ks = {b['name']: R.block_passes(b) * R.block_rounds(b) for b in R.BLOCKS}
case('K per cell: C4-BR 3 (diagnostic, ruling (b)), C4-G4 9, C4-G4-kd 3, C4-AB 9, C4-G5 9, C4-RAPIER 9',
     [] if ks == {'C4-BR': 3, 'C4-G4': 9, 'C4-G4-kd': 3, 'C4-AB': 9, 'C4-G5': 9, 'C4-RAPIER': 9} else [str(ks)], False)
br_blk = next(b for b in R.BLOCKS if b['name'] == 'C4-BR')
case('C4-BR is marked DIAGNOSTIC ONLY / not claim-bearing in its block note, its row note and the protocol (ruling (b))',
     [w for w, t in (('block', br_blk.get('note', '')), ('row', R.ROWS['C4-BR'].get('note', '')), ('protocol', R.PROTO.get('own_k', '')))
      if 'claim-bearing' not in t or ('DIAGNOSTIC' not in t.upper())], False)
case('C4-BR runs ONE pass of three rounds (passes 1: K 3, one reversed pass)', [] if (R.block_passes(br_blk), R.block_rounds(br_blk)) == (1, 3) else [str(br_blk)], False)

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

# ------------------------------------------------------------------------------------------------ 6. the Rapier block (2026-09-30)
# The block C4-RAPIER (rows9a.extra.json, generated by tools/mkextra9a.py): its structure, its pins, and the validator V1-V9 (window9a_run.
# rapier_check / validate_rapier) on the REAL gate cells (gate/rapier/cells, the window's own exes run through `gate9a.py rapier`) and on
# mutated copies of them. A case names the rule it must turn red; `exactly` demands the WHOLE set of violated rules, `only_for` that every
# reason belongs to the named rule, so a mutation cannot pass by tripping some other rule. The driver's validator is also cross-checked
# against the harness's reference validator on 1,279 comparisons (tools/xcheck_rapier9a.py -> gate/xcheck_rapier9a.txt).
import re  # noqa: E402

RAPD = os.path.join(GATE, 'rapier')
TMPR = os.path.join(GATE, 'selftest_tmp', 'rapier9a')
shutil.rmtree(TMPR, ignore_errors=True)
os.makedirs(TMPR)
have_rap = 'RP-D' in R.ROWS and 'RP-M' in R.ROWS and 'C4-RAPIER' in [b['name'] for b in R.BLOCKS]
case('the Rapier overlay is merged (rows RP-D, RP-M, block C4-RAPIER): without it every Rapier case below would be vacuous',
     [] if have_rap else ['rows9a.extra.json did not merge'], False)


def rules_of(why):
    return sorted({w.split(':')[0] for w in why})


def exactly(why, rules):
    """[] when the reasons name EXACTLY these rules; else the reasons (or a note when there are none)."""
    return [] if rules_of(why) == sorted(rules) else (why or ['no reason: the process was judged valid'])


if have_rap:
    rb = next(b for b in R.BLOCKS if b['name'] == 'C4-RAPIER')
    names = [b['name'] for b in R.BLOCKS]
    case('C4-RAPIER: priority 1.5, K 9 = 3 passes x 3 rounds (orchestrator ruling (a))',
         [] if (rb['priority'], R.block_passes(rb), R.block_rounds(rb)) == (1.5, 3, 3) else [str(rb)], False)
    case('blocks run in priority order C4-AB, C4-RAPIER, C4-G5, C4-BR, C4-G4, C4-G4-kd (Rapier after C4-AB, before C4-G5)',
         [] if names == ['C4-AB', 'C4-RAPIER', 'C4-G5', 'C4-BR', 'C4-G4', 'C4-G4-kd'] else [str(names)], False)
    rcells = R.cell_order('C4-RAPIER')
    want = {(r, k, w) for r in ('RP-D', 'RP-M') for k in ('rs8', 'rs4') for w in (1, 2, 4, 8, 16)}
    case('C4-RAPIER has 20 cells: RP-D / RP-M x rs8 / rs4 x W 1/2/4/8/16', [] if len(rcells) == 20 and set(rcells) == want else [f'{len(rcells)} cells'], False)
    fwd = [(r, k) for r, k, w in rcells if w == 8]
    case('forward order inside a W group: RP-D rs8, RP-D rs4, RP-M rs8, RP-M rs4 (window9a_rows.md, Protocol)',
         [] if fwd == [('RP-D', 'rs8'), ('RP-D', 'rs4'), ('RP-M', 'rs8'), ('RP-M', 'rs4')] else [str(fwd)], False)

    def four_adjacent(cells):
        """[] when, at every W, the four Rapier configurations occupy four CONSECUTIVE positions of the flat cell list (ruling (a))."""
        bad = []
        for w in (1, 2, 4, 8, 16):
            idx = sorted(i for i, c in enumerate(cells) if c[2] == w)
            if len(idx) != 4 or idx[-1] - idx[0] != 3:
                bad.append(f'W{w}: positions {idx}')
        return bad
    for p in range(3):
        case(f'C4-RAPIER pass {p}: RP-D/RP-M x rs8/rs4 are adjacent within every W group (pass order p0 reversed, p1 forward, p2 reversed)',
             four_adjacent(R.pass_order(rcells, p, rb)), False)
    case('C4-RAPIER pass order: p0 reversed, p1 forward, p2 reversed',
         [] if [R.pass_order(rcells, p, rb) for p in range(3)] == [rcells[::-1], rcells, rcells[::-1]] else ['wrong'], False)
    moved = [c for c in rcells if c != ('RP-D', 'rs8', 8)] + [('RP-D', 'rs8', 8)]
    case('adjacency check on a permuted order (one W8 cell moved to the end)', four_adjacent(moved), True)
    case('K per cell of C4-RAPIER: 9', [] if R.block_passes(rb) * R.block_rounds(rb) == 9 else ['not 9'], False)
    case('C4-RAPIER: one untimed warm-up PER EXE per pass (RP-D rs8 W8, RP-D rs4 W8: window9a_rows.md, Protocol)',
         [] if R.warmups_of(rb) == [('RP-D', 'rs8', 8), ('RP-D', 'rs4', 8)] else [str(R.warmups_of(rb))], False)
    one = {k: v for k, v in rb.items() if k != 'warmups'}
    one['warmup'] = ['RP-D', 'rs8', 8]
    case('warmups_of keeps the single `warmup` form of every other block', [] if R.warmups_of(one) == [('RP-D', 'rs8', 8)] else [str(R.warmups_of(one))], False)
    case('block_estimate prices BOTH warm-ups (the two-warm-up block costs more per pass than the same block with one)',
         [] if R.block_estimate(rb)['per_pass_s'] > R.block_estimate(one)['per_pass_s'] else ['same'], False)
    case('C4-AB still carries its single warm-up', [] if R.warmups_of(next(b for b in R.BLOCKS if b['name'] == 'C4-AB')) == [('C4-JD', 'tip', 8)] else ['changed'], False)
    for rid, stem in (('RP-D', 'RD'), ('RP-M', 'RM')):
        row = R.ROWS[rid]
        case(f'{rid}: pose_ref per binary (rs8 -> {stem}8, rs4 -> {stem}4), config_pin {stem}, --install loop, K-shape of window9a_rows.md',
             [] if (row['pose_ref'] == {'rs8': stem + '8', 'rs4': stem + '4'} and row['config_pin'] == stem and row['args'][-2:] == ['--install', 'loop']
                    and row['steps'] == 500 and row['window'] == [0, 500] and row['metric_windows'] == [[0, 100], [100, 500]]
                    and row['armed'] is False and row['workers'] == [1, 2, 4, 8, 16] and row['binaries'] == ['rs8', 'rs4']) else [str(row)[:200]], False)
    rows_doc = {'RP-D': {'rs8': {'0..100': 303003, '100..500': 354674}, 'rs4': {'0..100': 303003, '100..500': 354674}},
                'RP-M': {'rs8': {'0..100': 935729, '100..500': 1071107}, 'rs4': {'0..100': 935729, '100..500': 1071107}}}
    case('rows_pin of RP-D / RP-M are the values window9a_rows.md gives', [r for r in rows_doc if R.ROWS[r]['rows_pin'] != rows_doc[r]], False)
    case('the four Rapier fixtures are recorded (RD8 RD4 RM8 RM4) and their hashes equal the pins\' pose hashes',
         [k for k in ('RD8', 'RD4', 'RM8', 'RM4')
          if (R.FIXTURES.get(k) or {}).get('hash') != R.RAPIER_PINS['arms']['simd8' if k.endswith('8') else 'simd4']['cfgs']['rapier-default' if k[1] == 'D' else 'matched']['pose_hash']], False)
    case('verify_fixtures with the Rapier block selected (pins file sha256, each fixture sha256 and hash)', R.verify_fixtures(), False)
    saved_sha = R.RAPIER_PINS_SHA
    R.RAPIER_PINS_SHA = '0' * 64
    case('verify_fixtures with the pins file sha256 no longer the overlay pin', only_for(R.verify_fixtures(), 'rapier pins'), True)
    R.RAPIER_PINS_SHA = saved_sha
    cpin = R.RAPIER_PINS['arms']['simd8']['cfgs']['matched']
    saved_fx = cpin['fixture_sha256']
    cpin['fixture_sha256'] = '0' * 64
    case('verify_fixtures with a fixture that is not the one the pins name (sha256)', only_for(R.verify_fixtures(), 'fixture'), True)
    cpin['fixture_sha256'] = saved_fx
    saved_h = R.FIXTURES['RM8']['hash']
    R.FIXTURES['RM8']['hash'] = '0x0'
    case('verify_fixtures with fixtures.json carrying another hash than the pins for RM8', only_for(R.verify_fixtures(), 'fixtures.json'), True)
    R.FIXTURES['RM8']['hash'] = saved_h
    case('verify_fixtures restored', R.verify_fixtures(), False)
    case('rs8 / rs4 are the simd8 / simd4 arms and carry sha256_pin = the pins\' exe_sha256 of their arm (what V4 compares)',
         [k for k, a in (('rs8', 'simd8'), ('rs4', 'simd4')) if R.BINS[k].get('arm') != a or R.BINS[k].get('sha256_pin') != R.RAPIER_PINS['exe_sha256'][a]], False)
    case('rs8 / rs4 are in bin/SHA256SUMS', [k for k in ('rs8', 'rs4') if R.SUMS.get(R.BINS[k]['exe']) != R.BINS[k]['sha256_pin']], False)
    sv = {k: dict(R.BINS[k]) for k in ('rs8', 'rs4')}
    for k, o2 in (('rs8', 'rs4'), ('rs4', 'rs8')):
        R.BINS[k].update({'exe': sv[o2]['exe'], 'path': sv[o2]['path'], 'sha256': R.SUMS.get(sv[o2]['exe'])})
    case('verify_binaries with the rs8 / rs4 exes swapped (pins kept): a swapped Rapier arm is red', R.verify_binaries(), True)
    for k in sv:
        R.BINS[k].clear()
        R.BINS[k].update(sv[k])
    case('verify_binaries restored (Rapier exes included)', R.verify_binaries(), False)
    case('the window\'s Rapier exes are named with underscores; VOID_NAMES holds the harness\'s hyphenated names (a copy cannot void its own window)',
         [] if ({'rapier-parity-simd8.exe', 'rapier-parity-simd4.exe'} <= R.VOID_NAMES
                and not any(os.path.basename(R.BINS[k]['path']).lower() in R.VOID_NAMES for k in ('rs8', 'rs4'))) else ['names'], False)
    ps1 = open(os.path.join(HERE, 'wait_idle9a.ps1'), encoding='utf-8').read()
    case('wait_idle9a.ps1 names both Rapier harness exes (window9a_rows.md, preconditions)', [] if "'rapier-parity-simd8'" in ps1 and "'rapier-parity-simd4'" in ps1 else ['missing'], False)

    # ---- the validator on REAL gate cells (the window's own exes, run through gate9a.py rapier) and mutated copies
    def rcell(rid, key, w):
        return os.path.join(GATE, 'rapier', 'cells', f'{rid}_{key}_W{w}')

    def rsummary(cwd):
        so = open(os.path.join(cwd, 'stdout.txt'), encoding='utf-8').read()
        return json.loads(next(l for l in so.splitlines() if l.startswith('SUMMARY '))[8:])

    N_RC = [0]

    def rrec(rid, key, w, stdout=None, csv=None, pose=None, drop=(), exit_code=0, hang=False, args=None, binary=None):
        """A driver-shaped record of one real gate cell. Any of stdout / csv (fn: text -> text) and pose (fn: bytes -> bytes) makes a
        COPY of the cell dir under selftest_tmp with that file rewritten; `drop` deletes files of the copy."""
        src = rcell(rid, key, w)
        cwd = src
        if stdout or csv or pose or drop:
            N_RC[0] += 1
            cwd = os.path.join(TMPR, f'c{N_RC[0]}')
            shutil.copytree(src, cwd)
            for fn, tf, text in (('stdout.txt', stdout, True), ('run.csv', csv, True), ('pose.bin', pose, False)):
                if tf:
                    raw = open(os.path.join(cwd, fn), 'rb').read()
                    out = tf(raw.decode('utf-8')) if text else tf(raw)
                    open(os.path.join(cwd, fn), 'wb').write(out.encode('utf-8') if text else out)
            for fn in drop:
                os.remove(os.path.join(cwd, fn))
        return {'binary': binary or key, 'exit': exit_code, 'hang': hang, 'cwd': cwd, 'args': args if args is not None else rsummary(src)['args']}

    def judge(rec, rid, w, row_id=None):
        return R.validate_rapier(rec, R.ROWS[row_id or rid], w)

    def sedit(fn):
        """A stdout transform: edit the JSON of the SUMMARY line in place (fn mutates the dict)."""
        def go(text):
            lines = text.splitlines()
            i = next(k for k, l in enumerate(lines) if l.startswith('SUMMARY '))
            d = json.loads(lines[i][8:])
            fn(d)
            lines[i] = 'SUMMARY ' + json.dumps(d, separators=(',', ':'))
            return '\n'.join(lines) + '\n'
        return go

    def csv_rows(fn):
        def go(text):
            return '\n'.join(fn(text.splitlines())) + '\n'
        return go

    def flip(raw):
        b = bytearray(raw)
        b[52 * 617] ^= 0x01
        return bytes(b)

    for rid in ('RP-D', 'RP-M'):
        for key in ('rs8', 'rs4'):
            for w in (1, 2, 4, 8, 16):
                case(f'{rid} {key} W{w} as recorded by the gate (the real cell of the window\'s own exe: V1-V9 all green)', judge(rrec(rid, key, w), rid, w), False)
    A = ('RP-D', 'rs8', 8)     # the mutation baseline
    case('V1 exit code 4 (--expect-pose mismatch)', exactly(judge(rrec(*A, exit_code=4), A[0], A[2]), ['V1']), False)
    case('V1 exit code 3 (the harness voided itself)', exactly(judge(rrec(*A, exit_code=3), A[0], A[2]), ['V1']), False)
    case('V1 exit code 2 (usage)', exactly(judge(rrec(*A, exit_code=2), A[0], A[2]), ['V1']), False)
    case('V1 a hang (terminated by the driver: rec.hang)', exactly(judge(rrec(*A, hang=True), A[0], A[2]), ['V1']), False)
    case('V2 no SUMMARY line in the output', exactly(judge(rrec(*A, stdout=lambda t: '\n'.join(l for l in t.splitlines() if not l.startswith('SUMMARY '))), A[0], A[2]), ['V2']), False)
    case('V2 stdout missing altogether (the process wrote nothing)', exactly(judge(rrec(*A, drop=('stdout.txt',)), A[0], A[2]), ['V2']), False)
    case('V2 two SUMMARY lines', exactly(judge(rrec(*A, stdout=lambda t: t + next(l for l in t.splitlines() if l.startswith('SUMMARY ')) + '\n'), A[0], A[2]), ['V2']), False)
    case('V2 a SUMMARY line that is not JSON', exactly(judge(rrec(*A, stdout=lambda t: t.replace('SUMMARY {', 'SUMMARY {{')), A[0], A[2]), ['V2']), False)
    case('V3 threads.pool_threads_max = W+1 (wrong pool size)', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['threads'].update(pool_threads_max=9))), A[0], A[2]), ['V3']), False)
    case('V3 threads.install_receipt = [W, W+1]', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['threads'].update(install_receipt=[8, 9]))), A[0], A[2]), ['V3']), False)
    case('V3 threads.loop_on_pool_worker false', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['threads'].update(loop_on_pool_worker=False))), A[0], A[2]), ['V3']), False)
    case('V3 the CSV pool_threads column is W+1 in row 250 (wrong pool size, timed CSV)',
         exactly(judge(rrec(*A, csv=csv_rows(lambda ls: ls[:251] + [ls[251].rsplit(',', 1)[0] + ',9'] + ls[252:])), A[0], A[2]), ['V3']), False)
    case('V3 the W8 record judged at W4 (workers, pool threads and CSV all say 8)', exactly(judge(rrec(*A), A[0], 4), ['V3']), False)
    case('V4 features.simd8 flipped (wrong arm features)', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['features'].update(simd8=False))), A[0], A[2]), ['V4']), False)
    case('V4 SUMMARY arm label swapped (simd4 on a simd8 exe)', exactly(judge(rrec(*A, stdout=sedit(lambda d: d.update(arm='simd4'))), A[0], A[2]), ['V4']), False)
    case('V4 rapier_version 0.35.0', exactly(judge(rrec(*A, stdout=sedit(lambda d: d.update(rapier_version='0.35.0'))), A[0], A[2]), ['V4']), False)
    case('V4 target_features.avx2 false', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['target_features'].update(avx2=False))), A[0], A[2]), ['V4']), False)
    case('V4 source_fnv1a64 edited (an exe built from other sources)', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['source_fnv1a64'].update(main_rs='0x0'))), A[0], A[2]), ['V4']), False)
    case('V4 the record of the rs8 files claims binary rs4 (arm simd4 pins against a simd8 SUMMARY)', exactly(judge(rrec(*A, binary='rs4'), A[0], A[2]), ['V4']), False)
    sv_path = R.BINS['rs8']['path']
    R.BINS['rs8']['path'] = R.BINS['rs4']['path']
    case('V4 wrong exe sha256 (the process ran the simd4 exe under the rs8 key)', exactly(judge(rrec(*A), A[0], A[2]), ['V4']), False)
    R.BINS['rs8']['path'] = sv_path
    R.BINS['rs8']['path'] = os.path.join(TMPR, 'no_such.exe')
    case('V4 the exe is gone (sha256 MISSING)', exactly(judge(rrec(*A), A[0], A[2]), ['V4']), False)
    R.BINS['rs8']['path'] = sv_path
    case('V5 config.counters_enabled true in a timed row (Rapier\'s statistics walk inside the timed pair)',
         exactly(judge(rrec(*A, stdout=sedit(lambda d: d['config'].update(counters_enabled=True))), A[0], A[2]), ['V5']), False)
    case('V5 config.num_solver_iterations 4 -> 3', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['config'].update(num_solver_iterations=3))), A[0], A[2]), ['V5']), False)
    case('V5 install step', exactly(judge(rrec(*A, stdout=sedit(lambda d: d.update(install='step'))), A[0], A[2]), ['V5']), False)
    case('V5+V7 the RP-D files judged as the RP-M row (another config: config differs, pose is not RM8)',
         exactly(judge(rrec(*A), A[0], A[2], row_id='RP-M'), ['V5', 'V7']), False)
    case('V6 spawn hash edited', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['scene_identity'].update(spawn_hash='0x1'))), A[0], A[2]), ['V6']), False)
    case('V6 perturb not null', exactly(judge(rrec(*A, stdout=sedit(lambda d: d.update(perturb={'box': 0, 'axis': 0, 'dir': 1}))), A[0], A[2]), ['V6']), False)
    case('V6 scene_identity.first_equals_ours false', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['scene_identity'].update(first_equals_ours=False))), A[0], A[2]), ['V6']), False)
    case('V6 the run reports itself void', exactly(judge(rrec(*A, stdout=sedit(lambda d: d.update(void=True, voids=['x']))), A[0], A[2]), ['V6']), False)
    case('V7 the final pose differs by one bit (pose mismatch)', exactly(judge(rrec(*A, pose=flip), A[0], A[2]), ['V7']), False)
    case('V7 the SUMMARY expect_pose says mismatch', exactly(judge(rrec(*A, stdout=sedit(lambda d: d.update(expect_pose='mismatch: 1 vs 2 bytes'))), A[0], A[2]), ['V7']), False)
    case('V7 the final pose file is missing', exactly(judge(rrec(*A, drop=('pose.bin',)), A[0], A[2]), ['V7']), False)
    case('V7 the SUMMARY pose_hash is not the pin\'s', exactly(judge(rrec(*A, stdout=sedit(lambda d: d.update(pose_hash='0x1'))), A[0], A[2]), ['V7']), False)
    case('V8 the CSV has 499 rows (V3 sees the short pool_threads column too)',
         exactly(judge(rrec(*A, csv=csv_rows(lambda ls: ls[:-1])), A[0], A[2]), ['V3', 'V8']), False)
    case('V8 wall_ns 0 in row 10', exactly(judge(rrec(*A, csv=csv_rows(lambda ls: ls[:11] + [ls[11].split(',')[0] + ',0,' + ls[11].split(',')[2]] + ls[12:])), A[0], A[2]), ['V8']), False)
    case('V8 the CSV header carries an extra column', exactly(judge(rrec(*A, csv=csv_rows(lambda ls: [ls[0] + ',bp_pairs'] + [l + ',1' for l in ls[1:]])), A[0], A[2]), ['V8']), False)
    case('V8 run.csv is missing (V3 loses its column too)', exactly(judge(rrec(*A, drop=('run.csv',)), A[0], A[2]), ['V3', 'V8']), False)
    case('V9 --receipt among the launch arguments (the SUMMARY args are clean)',
         exactly(judge(rrec(*A, args=rsummary(rcell(*A))['args'] + ['--receipt']), A[0], A[2]), ['V9']), False)
    case('V9 receipt_gates.awake_min filled (a --receipt run)', exactly(judge(rrec(*A, stdout=sedit(lambda d: d['receipt_gates'].update(awake_min=1240))), A[0], A[2]), ['V9']), False)
    tw = R.RAPIER_PINS['arms']['simd8']['cfgs']['rapier-default']['twin_ok']
    saved_tw = tw['8']
    tw['8'] = False
    case('V9 the prep twin of (simd8, rapier-default, W8) did not pass', exactly(judge(rrec(*A), A[0], A[2]), ['V9']), False)
    tw['8'] = saved_tw
    gone = rules_of(judge(rrec(*A, drop=('stdout.txt', 'run.csv', 'pose.bin')), A[0], A[2]))
    case('nothing passes because files are absent: stdout, run.csv and pose.bin all missing turn V2, V3, V7 and V8 red',
         [] if {'V2', 'V3', 'V7', 'V8'} <= set(gone) else [f'only {gone}'], False)
    case('a binary with no pins (unknown arm) is red, never a crash', only_for(R.validate_rapier(dict(rrec(*A), binary='tip'), R.ROWS['RP-D'], 8), 'V4'), True)
    case('the pose reference is per binary: the rs4 cell is judged against RD4 (equal bytes today), rs8 against RD8',
         [] if (R.pose_ref_of(R.ROWS['RP-D'], 'rs4'), R.pose_ref_of(R.ROWS['RP-D'], 'rs8'), R.pose_ref_of(R.ROWS['C4-JD'], 'tip')) == ('RD4', 'RD8', 'JT500') else ['wrong'], False)
    ra = R.runner_args(R.ROWS['RP-D'], 'rs4', 8, TMPR)
    case('runner_args passes --expect-pose <RD4 fixture> for rs4 and no --arm-profiler / --receipt / --counters',
         [] if (ra[ra.index('--expect-pose') + 1].endswith('RD4.pose') and not {'--arm-profiler', '--receipt', '--counters'} & set(ra)
                and ra[:4] == ['--cfg', 'rapier-default', '--install', 'loop'] and '--workers' in ra and ra[ra.index('--steps') + 1] == '500') else [str(ra)], False)
    case('a rapier process alive past HANG_S_RAPIER (300 s) is a hang (V1)', [] if R.HANG_S_RAPIER == 300.0 else [str(R.HANG_S_RAPIER)], False)
    # the --test rehearsal shape: 12 steps, a pose that is not the 500-step fixture; `test` skips V7 and NOTHING else
    tlog, tcsv, tpose, tfix = R.rapier_process_files(rrec(*A), R.ROWS['RP-D'])
    tcsv12 = '\n'.join(tcsv.splitlines()[:13]) + '\n'

    def tcheck(**kw):
        a = dict(pins=R.RAPIER_PINS, arm='simd8', cfg='rapier-default', workers=8, log_text=tlog, csv_text=tcsv12, pose_raw=b'a 12-step pose',
                 fixture_raw=tfix, exe_sha256=R.RAPIER_PINS['exe_sha256']['simd8'], rc=0, steps=12, test=True)
        a.update(kw)
        return R.rapier_check(**a)
    case('--test shape (12 CSV rows, steps 12, a pose that is not the fixture, test=True) is valid: V7 is the only rule a rehearsal skips',
         sorted(tcheck()), False)
    case('--test shape with test=False: the pose that is not the fixture is V7', [] if 'V7' in tcheck(test=False) else ['not flagged'], False)
    case('--test shape still flags V1 (exit 4): test skips V7 only', [] if 'V1' in tcheck(rc=4) else ['not flagged'], False)
    case('--test shape still flags V2 (no SUMMARY): test skips V7 only', [] if 'V2' in tcheck(log_text='') else ['not flagged'], False)
    case('--test shape judged at steps 500 (12 CSV rows): V8', [] if 'V8' in tcheck(steps=500) else ['not flagged'], False)
    case('--test shape: counters_enabled true is still V5', [] if 'V5' in tcheck(log_text=sedit(lambda d: d['config'].update(counters_enabled=True))(tlog)) else ['not flagged'], False)
    case('rerun_allowance of C4-RAPIER: the floor lets a single slot reach the full wave depth', [] if R.rerun_capacity(rb, 1.25) >= R.RERUN_MAX_WAVES else [str(R.rerun_capacity(rb, 1.25))], False)

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
    # ---- the Rapier block end to end (2026-09-30): --test rehearses control flow with real 12-step Rapier processes on the REAL overlay
    if have_rap:
        E2E_N[0] += 1
        rraw = os.path.join(GATE, 'selftest_tmp', f'e2e_rapier_{E2E_N[0]}')
        shutil.rmtree(rraw, ignore_errors=True)
        os.makedirs(rraw)
        e = dict(os.environ, W9A_TEST_RAW=rraw, W9A_TEST_HOT='RP-D#rs8@W4:0:1')
        e.pop('W9A_EXTRA', None)
        for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            e.pop(k, None)
        pr = subprocess.run([sys.executable, '-B', DRIVER, '--test', '--blocks', 'C4-RAPIER', '--test-rounds', '1'], env=e,
                            capture_output=True, text=True, cwd=W9, timeout=900)
        rrecs = [json.loads(l) for l in open(os.path.join(rraw, 'runs.jsonl'), encoding='utf-8')] if os.path.exists(os.path.join(rraw, 'runs.jsonl')) else []
        rprocs = [r for r in rrecs if 'row' in r and r.get('attempt') != 'warmup']
        rwarm = [r for r in rrecs if 'row' in r and r.get('attempt') == 'warmup']
        orig = sorted((r for r in rprocs if r['attempt'] == 'original'), key=lambda r: r['seq'])
        case('e2e C4-RAPIER --test (1 round, W9A_TEST_HOT: RP-D#rs8@W4 hot once): exit 0, 20 originals + 1 re-run, 0 invalid, every record kind rapier',
             [] if (pr.returncode == 0 and len(orig) == 20 and len(rprocs) == 21 and not any(r.get('invalid') for r in rprocs + rwarm)
                    and all(r['kind'] == 'rapier' for r in rprocs + rwarm)) else [f'exit {pr.returncode} originals {len(orig)} procs {len(rprocs)} invalid {[r.get("invalid") for r in rprocs if r.get("invalid")][:2]} {pr.stderr[-200:]}'], False)
        case('e2e C4-RAPIER: two warm-ups per pass in order RP-D rs8 W8 then RP-D rs4 W8 (one per exe)',
             [] if [(r['row'], r['binary'], r['W']) for r in rwarm] == [('RP-D', 'rs8', 8), ('RP-D', 'rs4', 8)] else [str([(r['row'], r['binary'], r['W']) for r in rwarm])], False)
        case('e2e C4-RAPIER: the EXECUTED order of pass 0 is the reversed forward order and the four configurations are adjacent at every W',
             [] if [(r['row'], r['binary'], r['W']) for r in orig] == R.pass_order(R.cell_order('C4-RAPIER'), 0, rb) else ['executed order differs'], False)
        case('e2e C4-RAPIER: the injected hot slot was re-run once (round 0, rerun_no 1) and left the stage clean',
             [] if sorted((r['round'], r['rerun_no'], r['row'], r['binary'], r['W']) for r in rprocs if r['attempt'] == 'rerun') == [(0, 1, 'RP-D', 'rs8', 4)] else ['re-run set differs'], False)
        case('e2e C4-RAPIER: the process records carry the harness command line (--cfg, --install loop, --workers W, no --receipt) and the per-binary exe',
             [] if all(('--install' in r['args'] and 'loop' in r['args'] and '--receipt' not in r['args'] and r['args'][r['args'].index('--workers') + 1] == str(r['W'])
                        and r['exe'].endswith({'rs8': 'rapier_parity_simd8_736c2a06.exe', 'rs4': 'rapier_parity_simd4_1e5136ca.exe'}[r['binary']]))
                       for r in rprocs + rwarm) else ['args or exe differ'], False)
        # WIRING: the same rehearsal with the simd4 exe under the simd8 key (arm simd8): every real process must come back INVALID with V4
        # (features, arm, lanes and exe sha256 of the simd8 pins against a simd4 exe) - proof run_one applies the validator to a real record.
        ov_real = json.load(open(os.path.join(W9, 'rows9a.extra.json'), encoding='utf-8'))
        ov_bad = {'_comment': 'REHEARSAL ONLY (selftest9a.py wiring case): the simd4 exe under the simd8 key', 'rapier': ov_real['rapier'],
                  'binaries': {'rs8': dict(ov_real['binaries']['rs4'], arm='simd8')},
                  'blocks': [{'name': 'C4-RAPIER', 'item': 'Rapier (wiring rehearsal)', 'priority': 1.5, 'warmup': ['RAP-w', 'rs8', 8]}],
                  'rows': [dict(next(r for r in ov_real['rows'] if r['id'] == 'RP-M'), id='RAP-w', blocks=['C4-RAPIER'], binaries=['rs8'], pose_ref={'rs8': 'RM8'})]}
        bad_path = os.path.join(GATE, 'selftest_tmp', 'rapier_wiring_overlay.json')
        with open(bad_path, 'w', encoding='utf-8', newline='\n') as f:
            json.dump(ov_bad, f)
        E2E_N[0] += 1
        braw = os.path.join(GATE, 'selftest_tmp', f'e2e_rapier_wiring_{E2E_N[0]}')
        shutil.rmtree(braw, ignore_errors=True)
        os.makedirs(braw)
        e2 = dict(os.environ, W9A_EXTRA=bad_path, W9A_TEST_RAW=braw, W9A_TEST_HOT='none#none@W0:0:0')
        for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            e2.pop(k, None)
        pb = subprocess.run([sys.executable, '-B', DRIVER, '--test', '--blocks', 'C4-RAPIER', '--test-rounds', '1'], env=e2,
                            capture_output=True, text=True, cwd=W9, timeout=900)
        brecs = [json.loads(l) for l in open(os.path.join(braw, 'runs.jsonl'), encoding='utf-8')] if os.path.exists(os.path.join(braw, 'runs.jsonl')) else []
        bprocs = [r for r in brecs if 'row' in r and r.get('attempt') != 'warmup']
        b_orig = [r for r in bprocs if r['attempt'] == 'original']
        b_pcs = [r for r in brecs if r.get('passcell')]
        case('e2e wiring: the simd4 exe under the simd8 key -> every real process (5 originals, then ruling 8 re-runs the INVALID slots for its 4 waves = 20) '
             'is INVALID with V4 (run_one applies the validator), and no pass-cell gates',
             [] if (pb.returncode == 0 and len(b_orig) == 5 and len(bprocs) == 25 and all(any(x.startswith('V4:') for x in (r.get('invalid') or [])) for r in bprocs)
                    and all(r.get('valid') is False for r in bprocs) and len(b_pcs) == 5 and not any(c['gates'] for c in b_pcs)
                    and all(c['k_clean'] == 0 for c in b_pcs))
             else [f'exit {pb.returncode} originals {len(b_orig)} procs {len(bprocs)} passcells {[(c["k_clean"], c["gates"]) for c in b_pcs]} '
                   f'invalid {[r.get("invalid") for r in bprocs][:1]} {pb.stderr[-200:]}'], False)
        cn = subprocess.run([sys.executable, '-B', os.path.join(HERE, 'counts9a.py'), braw], capture_output=True, text=True).stdout.splitlines()
        case('e2e wiring: counts9a.py prints an INVALID line (with the rule V4) for a rapier process the validator rejected',
             [] if any(l.startswith('INVALID') and 'V4:' in l for l in cn) else ['no INVALID V4 line'], False)
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


real_ov = json.load(open(os.path.join(W9, 'rows9a.extra.json'), encoding='utf-8'))
rap_row = json.loads(json.dumps(next(r for r in real_ov['rows'] if r['id'] == 'RP-M')))
rap_row.update({'id': 'RAP-matched', 'blocks': ['C4-RAPIER'], 'binaries': ['rs8'], 'pose_ref': {'rs8': 'RM8'}})
base_ov = {'_comment': 'REHEARSAL ONLY, GENERATED by selftest9a.py from the real rows9a.extra.json: one Rapier row on the rs8 exe, for the placement cases below',
           'rapier': real_ov['rapier'], 'binaries': {'rs8': real_ov['binaries']['rs8']},
           'blocks': [{'name': 'C4-RAPIER', 'item': 'Rapier (placement rehearsal)', 'priority': 4.5, 'warmup': ['RAP-matched', 'rs8', 8]}],
           'rows': [rap_row]}
os.makedirs(os.path.join(W9, 'test', 'rapier_overlay'), exist_ok=True)
with open(os.path.join(W9, 'test', 'rapier_overlay', 'rows9a.extra.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(base_ov, f, indent=1)


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
