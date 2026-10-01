"""Window 8b: every validator the 8b driver ADDED must be able to go red. Imports the driver without running it
(--dry-run parse only; main() is not called), feeds each validator one real gate record (must pass) and one mutated
copy (must fail), and prints PASS/FAIL per case. Launches nothing, reads no timing value.
  python -B tools/selftest8b.py > gate/selftest8b.txt"""
import copy
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)
sys.argv = [sys.argv[0], '--dry-run']
sys.path.insert(0, HERE)
import window8b_run as R  # noqa: E402

GATE = os.path.join(W8, 'gate')
D = R.D
fails = 0


def case(name, why, want_red):
    global fails
    ok = bool(why) == want_red
    fails += not ok
    print(f'{"PASS" if ok else "FAIL"} {name}: {"red" if why else "green"} (want {"red" if want_red else "green"}) {why[:2]}')


def runner_rec(row_id, key, w, cwd):
    so = open(os.path.join(cwd, 'stdout.txt'), 'rb').read()
    rec = {'binary': key, 'exit': 0, 'summary': D.parse_summary(so)}
    rec.update(R.runner_stats(R.ROWS[row_id], cwd))
    return rec


# 1. S4 counters by binary: a real TIP armed W8 record passes; relabelled as PARENT it must fail (swapped exes).
cwd = os.path.join(GATE, 'cells', 'S4-JT-a_tip_W8')
rec = runner_rec('S4-JT-a', 'tip', 8, cwd)
case('S4-JT-a tip W8 as recorded', R.validate_runner(rec, R.ROWS['S4-JT-a'], 8), False)
m = copy.deepcopy(rec)
m['binary'] = 'parent'
case('S4-JT-a tip W8 relabelled parent (swapped binaries)', R.validate_runner(m, R.ROWS['S4-JT-a'], 8), True)
cwdp = os.path.join(GATE, 'cells', 'S4-JT-a_parent_W8')
recp = runner_rec('S4-JT-a', 'parent', 8, cwdp)
case('S4-JT-a parent W8 as recorded', R.validate_runner(recp, R.ROWS['S4-JT-a'], 8), False)
m = copy.deepcopy(recp)
m['binary'] = 'tip'
case('S4-JT-a parent W8 relabelled tip (S4 never dispatched)', R.validate_runner(m, R.ROWS['S4-JT-a'], 8), True)
# 2. kd_order_builds: the kd record passes; the same record checked as the leaflist row must fail, and vice versa.
reck = runner_rec('F3-JT-kd', 'tip', 8, os.path.join(GATE, 'cells', 'F3-JT-kd_tip_W8'))
case('F3-JT-kd W8 as recorded', R.validate_runner(reck, R.ROWS['F3-JT-kd'], 8), False)
case('F3-JT-kd W8 validated as F3-JT-leaflist', R.validate_runner(reck, R.ROWS['F3-JT-leaflist'], 8), True)
recl = runner_rec('F3-JT-leaflist', 'tip', 8, os.path.join(GATE, 'cells', 'F3-JT-leaflist_tip_W8'))
case('F3-JT-leaflist W8 validated as F3-JT-kd', R.validate_runner(recl, R.ROWS['F3-JT-kd'], 8), True)
# 3. TreeDiag per row: J-Son-T passes under its own expectation; under the default Tree rule it must fail.
recj = runner_rec('JSonT', 'tip', 8, os.path.join(GATE, 'cells', 'JSonT_tip_W8'))
case('JSonT W8 as recorded', R.validate_runner(recj, R.ROWS['JSonT'], 8), False)
row = dict(R.ROWS['JSonT'])
row['tree_diag_expect'] = None
case('JSonT W8 under the default Tree rule', R.validate_runner(recj, row, 8), True)
# 4. the freeze step: a wrong expectation must fail.
row = dict(R.ROWS['JSonT'])
row['expect_first_frozen'] = (row['expect_first_frozen'] or 0) + 1
case('JSonT W8 with the freeze step off by one', R.validate_runner(recj, row, 8), True)
# 5. R4: equal poses -> equal; a different pose file -> DIFFER.
tmp = os.path.join(GATE, 'selftest_tmp')
os.makedirs(tmp, exist_ok=True)
R.RAW = tmp
blk = {'name': 'selftest'}
recs = {('F3-JT-kd', 'tip', 8): {'cwd': os.path.join(GATE, 'cells', 'F3-JT-kd_tip_W8'), 'attempt': 'original'},
        ('F3-JT-leaflist', 'tip', 8): {'cwd': os.path.join(GATE, 'cells', 'F3-JT-leaflist_tip_W8'), 'attempt': 'original'}}
out = R.r4_compare(blk, 0, 0, 0, recs, os.path.join(tmp, 'runs.jsonl'))
case('R4 on the gate pair (equal poses)', [] if out[0]['verdict'] == 'equal' else [out[0]['verdict']], False)
recs[('F3-JT-leaflist', 'tip', 8)]['cwd'] = os.path.join(GATE, 's4', 'J-T-off_tip_W8')  # the JToff500 pose
out = R.r4_compare(blk, 0, 0, 0, recs, os.path.join(tmp, 'runs.jsonl'))
case('R4 against a J-T-off pose', [] if out[0]['verdict'] == 'equal' else [out[0]['verdict']], True)
# 6. criterion: the full dry sample passes; one missing id and one extra id each fail.
cs_cwd = os.path.join(GATE, 'criterion_sample')
if os.path.exists(os.path.join(cs_cwd, 'stdout.txt')):
    so = open(os.path.join(cs_cwd, 'stdout.txt'), 'rb').read()
    se = open(os.path.join(cs_cwd, 'stderr.txt'), 'rb').read()
    rec = {'exit': 0}
    case('G4 dry sample as recorded', R.criterion_check(rec, R.ROWS['G4'], so, se, cs_cwd), False)
    row = dict(R.ROWS['G4'])
    row['expect'] = row['expect'] + ['bp_g4_uniform/tree_kd/9999']
    case('G4 with one id the run lacks', R.criterion_check({'exit': 0}, row, so, se, cs_cwd), True)
    row = dict(R.ROWS['G4'])
    row['expect'] = row['expect'][1:]
    case('G4 with one unexpected id', R.criterion_check({'exit': 0}, row, so, se, cs_cwd), True)
else:
    print('SKIP criterion cases: gate/criterion_sample has not run')
# 7. DM1: the gate sample passes as its row; the present mode and a missing zone each fail.
dcwd = os.path.join(GATE, 'dm1', 'sample_A_vb_idle')
so = open(os.path.join(dcwd, 'stdout.txt'), 'rb').read()
se = open(os.path.join(dcwd, 'stderr.txt'), 'rb').read()
row = R.ROWS['dm-vb-1920x1080-idle']
case('DM1 vb idle sample as recorded', R.dm1_check({'exit': 0}, row, so, se, dcwd), False)
r2 = dict(row)
r2['present_mode'] = 'mailbox'
case('DM1 vb idle expecting mailbox', R.dm1_check({'exit': 0}, r2, so, se, dcwd), True)
r2 = dict(row)
r2['need_zones'] = dict(row['need_zones'], GBUF_DEFERRED_RESOLVE=17)
case('DM1 vb idle requiring zone 17 (deferred only)', R.dm1_check({'exit': 0}, r2, so, se, dcwd), True)
# 8. pass orientation (ruling 1): p0 and p2 reversed, p1 forward.
o = [('a', 'k', 1), ('b', 'k', 1)]
po = [R.pass_order(o, p) for p in range(3)]
case('pass order p0 reversed / p1 forward / p2 reversed',
     [] if po == [o[::-1], o, o[::-1]] else [f'got {po}'], False)
dm = next(b for b in R.ROWS_JSON['blocks'] if b['name'] == 'DM1')
case('DM1 explicit ABBA order is not reversed in pass 0',
     [] if R.pass_order(R.cell_order('DM1'), 0, dm)[0] == ('dm-vb-1920x1080-idle', 'dmA', 0) else ['reversed'], False)
print(f'selftest8b: {fails} failing case(s)')
sys.exit(1 if fails else 0)
