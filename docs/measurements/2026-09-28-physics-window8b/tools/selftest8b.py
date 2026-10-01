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
# ---- 2026-09-29: the reserved blocks S7-AB and omega-v2 (rows8b.extra.json) --------------------------------------
if 's7p' in R.BINS and os.path.exists(os.path.join(GATE, 's7', 'preflight.json')):
    import s7pre8b as S7  # noqa: E402  (import only: its main is guarded)
    import micro8b as M  # noqa: E402
    # 9. S7 engagement receipt (tools/s7pre8b.py) on the REAL armed pre-flight records: each passes under its own
    #    key's label; relabelled as the other key (P/T swapped) it must fail - at W16, where the cap can engage.
    for cfg in ('JT', 'JA'):
        for role, key in (('T', 's7t'), ('P', 's7p')):
            so = open(os.path.join(GATE, 's7', f'{role}-{cfg}-W16-armed', 'stdout.txt'), 'rb').read()
            w8s = D.parse_summary(so)['w8s']
            lab = S7.LABEL[key]
            other = 'uncapped' if lab == 'capped' else 'capped'
            case(f'S7 engagement {role} {cfg} W16 as {key} ({lab})', S7.engagement_why(lab, 16, w8s), False)
            case(f'S7 engagement {role} {cfg} W16 relabelled {other} (P/T swapped)', S7.engagement_why(other, 16, w8s), True)
    # 10. The labelled binary check (verify_binaries + sha256_pin): swap the two S7 keys' exes exactly as the overlay
    #     loader would (exe, path, SHA256SUMS lookup) - the pin reds it; without the pins the same swap is GREEN,
    #     which is why the pin exists (SHA256SUMS is keyed by path).
    saved = {k: dict(R.BINS[k]) for k in ('s7p', 's7t')}
    case('verify_binaries as written (pins included)', R.verify_binaries(), False)
    for k, o in (('s7p', 's7t'), ('s7t', 's7p')):
        R.BINS[k].update({'exe': saved[o]['exe'], 'path': saved[o]['path'], 'sha256': R.SUMS.get(saved[o]['exe'])})
    case('verify_binaries with the s7p/s7t exes swapped (pins kept)', R.verify_binaries(), True)
    for k in ('s7p', 's7t'):
        R.BINS[k].pop('sha256_pin', None)
    case('the same swap without the pins (SHA256SUMS alone cannot see it)', R.verify_binaries(), False)
    for k in ('s7p', 's7t'):
        R.BINS[k].clear()
        R.BINS[k].update(saved[k])
    case('verify_binaries restored', R.verify_binaries(), False)
    # 11. omega_b v2 per-process rules (driver validate_micro -> micro8b.process_rules) on the REAL gate record.
    mdir = os.path.join(GATE, 'micro', 'omega2-worker')
    so = open(os.path.join(mdir, 'stdout.txt'), 'rb').read()
    row = R.ROWS['omega2-worker']
    case('omega2-worker gate sample as recorded', R.validate_micro({'exit': 0}, row, so), False)
    txt = D.decode(so)

    def mutate(fn):
        out = []
        for line in txt.splitlines():
            for tag in ('SUMMARY ', 'CALIBRATION '):
                if line.startswith(tag):
                    d = json.loads(line[len(tag):])
                    d2 = fn(tag.strip(), d)
                    line = None if d2 is None else tag + json.dumps(d2)
            if line is not None:
                out.append(line)
        return '\n'.join(out).encode()

    first = {'done': False}

    def once(f):
        def g(tag, d):
            if tag == 'SUMMARY' and not first['done']:
                first['done'] = True
                return f(dict(d))
            return d
        first['done'] = False
        return g
    case('omega2 one SUMMARY with version 1', R.validate_micro({'exit': 0}, row, mutate(once(lambda d: {**d, 'version': 1}))), True)
    case('omega2 one SUMMARY with exactly_once false',
         R.validate_micro({'exit': 0}, row, mutate(once(lambda d: {**d, 'exactly_once': False}))), True)
    case('omega2 one SUMMARY with lost_wakeups 1',
         R.validate_micro({'exit': 0}, row, mutate(once(lambda d: {**d, 'lost_wakeups': 1}))), True)
    case('omega2 one SUMMARY with work_ns_calibrated 450',
         R.validate_micro({'exit': 0}, row, mutate(once(lambda d: {**d, 'work_ns_calibrated': 450.0}))), True)
    case('omega2 CALIBRATION work_ns_calibrated 1200',
         R.validate_micro({'exit': 0}, row, mutate(lambda t, d: {**d, 'work_ns_calibrated': 1200.0} if t == 'CALIBRATION' else d)), True)
    case('omega2 CALIBRATION line missing',
         R.validate_micro({'exit': 0}, row, mutate(lambda t, d: None if t == 'CALIBRATION' else d)), True)
    case('omega2 CALIBRATION version 1',
         R.validate_micro({'exit': 0}, row, mutate(lambda t, d: {**d, 'version': 1} if t == 'CALIBRATION' else d)), True)
    so1 = open(os.path.join(GATE, 'micro', 'omega1-s36-worker', 'stdout.txt'), 'rb').read()
    case('omega1-s36-worker (v1, window 8 checks only) as recorded', R.validate_micro({'exit': 0}, R.ROWS['omega1-s36-worker'], so1), False)
    case('omega1-s36-worker validated as omega2-worker (v1 output under v2 rules)', R.validate_micro({'exit': 0}, row, so1), True)
    # 12. The park rule per cell (micro8b.cell_park_rule, used by counts8b and the gate), on the REAL gap record:
    #     3 reps as recorded -> no void; the gap > 0 park rows at 0 on every rep -> VOID; at 0 on 2 of 3 reps -> not
    #     void ("every rep"); a gap-0 park row at 0 on every rep -> reported "no park taken", not void.
    gso = open(os.path.join(GATE, 'micro', 'omega2-gap-worker', 'stdout.txt'), 'rb').read()
    grow = R.ROWS['omega2-gap-worker']

    def rep(so_bytes, rid='omega2-gap-worker', rw=grow):
        r = {'exit': 0, 'row': rid, 'kind': 'micro', 'attempt': 'original', 'block': 'omega-v2', 'binary': 'omega2', 'W': 0}
        r['invalid'] = R.validate_micro(r, rw, so_bytes)
        r['valid'] = not r['invalid']
        return r
    gtxt = D.decode(gso)
    zero = '\n'.join(('SUMMARY ' + json.dumps({**json.loads(l[8:]), 'parks_per_region_median': 0})
                      if l.startswith('SUMMARY ') and json.loads(l[8:]).get('helper') == 'park' else l)
                     for l in gtxt.splitlines()).encode()
    real3 = [rep(gso) for _ in range(3)]
    v, n, _ = M.cell_park_rule(real3)
    case('park rule: gap rows as recorded, 3 reps', [x[1] for x in v], False)
    z3 = [rep(zero) for _ in range(3)]
    case('park rule: every rep of the gap rows valid per process (the rule is per cell)', [r['invalid'] for r in z3 if r['invalid']], False)
    v, n, _ = M.cell_park_rule(z3)
    case('park rule: gap > 0 park rows at 0 parks on every rep -> VOID', [x[1] for x in v], True)
    v, n, _ = M.cell_park_rule(z3[:2] + real3[:1])
    case('park rule: 0 parks on 2 of 3 reps -> not void', [x[1] for x in v], False)
    wso = open(os.path.join(GATE, 'micro', 'omega2-worker', 'stdout.txt'), 'rb').read()
    wtxt = D.decode(wso)
    wzero = '\n'.join(('SUMMARY ' + json.dumps({**json.loads(l[8:]), 'parks_per_region_median': 0})
                       if l.startswith('SUMMARY ') and json.loads(l[8:]).get('helper') == 'park' else l)
                      for l in wtxt.splitlines()).encode()
    w3 = [rep(wzero, 'omega2-worker', row) for _ in range(3)]
    v, n, _ = M.cell_park_rule(w3)
    case('park rule: gap-0 park rows at 0 on every rep -> not void', [x[1] for x in v], False)
    case('park rule: gap-0 park rows at 0 on every rep -> reported "no park taken"', [x[1] for x in n], True)
    # 13. counts8b.py end to end on synthetic runs.jsonl files (the path the window's counts.txt takes).
    import subprocess  # noqa: E402
    for name, recs3, want in (('void', z3, True), ('asrecorded', real3, False)):
        d = os.path.join(GATE, 'selftest_tmp', f'park_{name}')
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, 'runs.jsonl'), 'w', encoding='utf-8', newline='\n') as f:
            for r in recs3:
                f.write(json.dumps(r) + '\n')
        out = subprocess.run([sys.executable, '-B', os.path.join(HERE, 'counts8b.py'), d], capture_output=True, text=True)
        hit = [l for l in out.stdout.splitlines() if l.startswith('VOID omega_b v2')]
        case(f'counts8b.py on {name} reps (exit {out.returncode}): VOID lines {len(hit)}',
             hit if out.returncode == 0 else [f'exit {out.returncode}'], want)
else:
    print('SKIP S7/omega cases: rows8b.extra.json or gate/s7/preflight.json absent')
# 14. 2026-09-29 (later): omega(W, gap) with the N4 receipt (rows omega-wgap-*, micro8b expect_present), on the REAL
#     gate samples. A red must be red for the named reason ONLY (only_for), so a mutation that trips some other rule
#     cannot pass as this rule's control: as recorded -> green; one receipt key dropped from ONE SUMMARY line -> red
#     (each key); one SUMMARY line dropped (7) or duplicated (9) -> red; helped_reps 0 with first_helper_ns_median null
#     (the exe's own "no helper took part" form) -> green (presence, not value); v1 omega-b output -> red.
WGAP = ('omega-wgap-worker', 'omega-wgap-external')
if all(r in R.ROWS for r in WGAP) and all(os.path.exists(os.path.join(GATE, 'micro', r, 'stdout.txt')) for r in WGAP):
    def only_for(why, sub):
        return why if why and all(sub in w for w in why) else []

    for rid in WGAP:
        wrow = R.ROWS[rid]
        wso = open(os.path.join(GATE, 'micro', rid, 'stdout.txt'), 'rb').read()
        wl = D.decode(wso).splitlines()
        si = [i for i, l in enumerate(wl) if l.startswith('SUMMARY ')]

        def with_lines(ls):
            return '\n'.join(ls).encode()

        def first_summary(fn):
            ls = list(wl)
            ls[si[0]] = 'SUMMARY ' + json.dumps(fn(json.loads(ls[si[0]][8:])))
            return with_lines(ls)
        case(f'{rid} gate sample as recorded', R.validate_micro({'exit': 0}, wrow, wso), False)
        for k in ('helped_reps', 'first_helper_ns_median'):
            mso = first_summary(lambda d, k=k: {x: v for x, v in d.items() if x != k})
            case(f'{rid} one SUMMARY without {k}',
                 only_for(R.validate_micro({'exit': 0}, wrow, mso), 'missing receipt field'), True)
        case(f'{rid} 7 SUMMARY lines (one dropped)',
             only_for(R.validate_micro({'exit': 0}, wrow, with_lines([l for i, l in enumerate(wl) if i != si[-1]])),
                      'SUMMARY lines, expected 8'), True)
        case(f'{rid} 9 SUMMARY lines (one duplicated)',
             only_for(R.validate_micro({'exit': 0}, wrow, with_lines(wl + [wl[si[0]]])), 'SUMMARY lines, expected 8'),
             True)
        case(f'{rid} helped_reps 0 / first_helper_ns_median null on one line (no helper: a receipt, not a miss)',
             R.validate_micro({'exit': 0}, wrow, first_summary(lambda d: {**d, 'helped_reps': 0,
                                                                          'first_helper_ns_median': None})), False)
    case('omega1-s36-worker (v1 omega-b) output validated as omega-wgap-worker',
         R.validate_micro({'exit': 0}, R.ROWS['omega-wgap-worker'],
                          open(os.path.join(GATE, 'micro', 'omega1-s36-worker', 'stdout.txt'), 'rb').read()), True)
else:
    print('SKIP omega-wgap cases: the rows or gate/micro/omega-wgap-* are absent')
print(f'selftest8b: {fails} failing case(s)')
sys.exit(1 if fails else 0)
