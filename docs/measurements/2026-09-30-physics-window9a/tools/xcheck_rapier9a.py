"""Window 9a, 2026-09-30 (orchestrator ruling (c)): the driver's Rapier validator against the harness's reference validator.
The driver's window9a_run.rapier_check (V1-V9 written from gate/rapier/window9a_rows.md) and the harness's own gate/rapier/void_check.py
(byte copy of D:/tmp/rapier-parity/gate/void_check.py, sha256 checked against it when the harness dir is present) must return the SAME SET
OF VIOLATED RULES on:
  A. the 20 timed-shape twins of the harness's gate g2t (2 arms x 2 configs x W 1/2/4/8/16), copied to gate/rapier/g2t/: both must say VALID;
  B. the 33 red-control mutations of the harness's analyze.py "gv" section (gate/rapier/mutations_ref.py, verbatim), on the same two baselines
     the harness used (simd8 rapier-default W8, simd4 matched W16) AND on every one of the 20 twins;
  C. the harness's own g5 red-control runs (real SUMMARY lines of exit 0 / 2 / 3 / 4 runs: mutated / truncated / padded / missing / other-cfg /
     other-arm fixtures, --steps 501, counters off under --receipt, counters forced on in the timed shape), each judged on its own log with the
     twin's CSV and pose of the same (arm, cfg, W);
  D. extra mutations written for this cross-check (rules the 33 do not reach: workers / requested, target_env, debug assertions, lanes,
     an extra target feature, an absent config field, install step, colliders, void, receipt_gates, an empty CSV, an extra CSV row, ...);
  E. combinations of two mutations (the rule sets must still agree).
Reasons are compared as RULE SETS, not as text. Cases only the driver can see (the launch arguments, an empty fixture, a JSON true where an
integer is required) are listed separately and must be red in the driver.
No timing value is printed: only the sign of wall_ns is ever read, by both validators. Exit 0 iff every comparison agrees.
  python -B tools/xcheck_rapier9a.py > gate/xcheck_rapier9a.txt
  python -B tools/xcheck_rapier9a.py --driver tools/_mut.py     # a MUTATED copy of the driver: the cross-check must then disagree (exit 1)"""
import copy
import hashlib
import importlib.util
import json
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W9 = os.path.dirname(HERE)
ARGV = sys.argv[1:]
DRIVER = os.path.join(HERE, 'window9a_run.py')
if '--driver' in ARGV:
    DRIVER = os.path.abspath(ARGV[ARGV.index('--driver') + 1])
sys.argv = [sys.argv[0], '--dry-run']
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, HERE)
_spec = importlib.util.spec_from_file_location('window9a_run', DRIVER)
R = importlib.util.module_from_spec(_spec)
sys.modules['window9a_run'] = R
_spec.loader.exec_module(R)
sys.argv = [sys.argv[0]] + ARGV

RAP = os.path.join(W9, 'gate', 'rapier')
FIX = os.path.join(W9, 'gate', 'fixtures')
HARNESS = 'D:/tmp/rapier-parity/gate'


def load(name, path):
    sp = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(sp)
    sp.loader.exec_module(m)
    return m


vc = load('void_check_ref', os.path.join(RAP, 'void_check.py'))
mut_ref = load('mutations_ref', os.path.join(RAP, 'mutations_ref.py'))
PINS = R.RAPIER_PINS
ARMS = ('simd8', 'simd4')
CFGS = ('rapier-default', 'matched')
WS = (1, 2, 4, 8, 16)
n_cases = 0
n_bad = 0
disagree = []


def sha(path):
    return hashlib.sha256(open(path, 'rb').read()).hexdigest()


def read(path, mode='r'):
    return open(path, mode, **({} if 'b' in mode else {'encoding': 'utf-8'})).read()


def rc_of(log_text):
    return next((int(l[3:]) for l in reversed(log_text.splitlines()) if l.startswith('rc=')), -1)


def cell(arm, cfg, w, log_path=None):
    base = os.path.join(RAP, 'g2t', arm, f'{cfg}_W{w}')
    log_text = read(log_path or base + '.log')
    name = os.path.basename(PINS['arms'][arm]['cfgs'][cfg]['fixture']).split('.')[0]
    return dict(pins=copy.deepcopy(PINS), arm=arm, cfg=cfg, workers=w, log_text=log_text, csv_text=read(base + '.csv'),
                pose_raw=read(base + '.pose', 'rb'), fixture_raw=read(os.path.join(FIX, name + '.pose'), 'rb'),
                exe_sha256=PINS['exe_sha256'][arm], rc=rc_of(log_text), timed_out=False)


def verdicts(ctx):
    """(reference rules, driver rules), each a sorted list."""
    a = sorted(vc.check(**copy.deepcopy(ctx)))
    b = sorted(R.rapier_check(**copy.deepcopy(ctx)))
    return a, b


def compare(label, ctx, must_include=None, expect_valid=False):
    """One cross-check case. `must_include`: rules the case is documented to turn red (both validators must flag them)."""
    global n_cases, n_bad
    n_cases += 1
    a, b = verdicts(ctx)
    ok = a == b
    if expect_valid:
        ok = ok and not a
    if must_include:
        ok = ok and set(must_include) <= set(a)
    if not ok:
        n_bad += 1
        disagree.append(label)
    print(f'{"AGREE" if ok else "DISAGREE"} {label}: reference {a or "VALID"} driver {b or "VALID"}'
          + (f' (must include {sorted(must_include)})' if must_include else ''))
    return a, b


print(f'reference validator: {os.path.join(RAP, "void_check.py")} sha256 {sha(os.path.join(RAP, "void_check.py"))[:16]}...')
if os.path.exists(os.path.join(HARNESS, 'void_check.py')):
    same = sha(os.path.join(HARNESS, 'void_check.py')) == sha(os.path.join(RAP, 'void_check.py'))
    print(f'  equals the harness dir\'s void_check.py: {same}')
    n_cases += 1
    if not same:
        n_bad += 1
        disagree.append('the window copy of void_check.py differs from the harness dir')
    an = os.path.join(HARNESS, 'analyze.py')
    if os.path.exists(an):
        src = open(an, encoding='utf-8').read()
        i, j = src.index('def edit_summary(log_text, fn):'), src.index('MUTATIONS = [')
        region = src[i:src.index('\n]\n', j) + 3]
        mine = open(os.path.join(RAP, 'mutations_ref.py'), encoding='utf-8').read()
        n_cases += 1
        if region not in mine:
            n_bad += 1
            disagree.append('mutations_ref.py is not a verbatim excerpt of the harness analyze.py')
        print(f'  mutations_ref.py is a verbatim excerpt of the harness analyze.py: {region in mine}')

print('\n== A. the 20 timed-shape twins of the harness gate g2t: both validators must say VALID ==')
for arm in ARMS:
    for cfg in CFGS:
        for w in WS:
            compare(f'A {arm} {cfg} W{w}', cell(arm, cfg, w), expect_valid=True)

print('\n== B. the harness\'s 33 gv mutations: on its two baselines and on all 20 twins ==')
MUT = mut_ref.MUTATIONS
for arm, cfg, w in (('simd8', 'rapier-default', 8), ('simd4', 'matched', 16)):
    for name, expect, allow, fn in MUT:
        ctx = cell(arm, cfg, w)
        fn(ctx)
        a, b = compare(f'B {arm} {cfg} W{w} [{name}]', ctx, must_include=expect)
        # the harness's own acceptance: it turns its named rules red and only those (plus the rules it allows)
        ok = set(expect) <= set(a) <= set(expect) | set(allow)
        n_cases += 1
        if not ok:
            n_bad += 1
            disagree.append(f'B reference {arm} {cfg} W{w} [{name}] outside its own contract')
            print(f'DISAGREE reference outside the harness\'s own contract for [{name}]: flagged {a}, expected {sorted(expect)} + {sorted(allow)}')
n_all = 0
for arm in ARMS:
    for cfg in CFGS:
        for w in WS:
            for name, expect, allow, fn in MUT:
                ctx = cell(arm, cfg, w)
                fn(ctx)
                a, b = verdicts(ctx)
                n_all += 1
                n_cases += 1
                if a != b or not set(expect) <= set(b):
                    n_bad += 1
                    disagree.append(f'B* {arm} {cfg} W{w} [{name}]')
                    print(f'DISAGREE B* {arm} {cfg} W{w} [{name}]: reference {a} driver {b}')
print(f'B*: {n_all} (twin x mutation) cases compared; disagreements so far {n_bad}')

print('\n== C. the harness\'s own g5 red-control runs (real logs), each with the twin\'s CSV and pose of its (arm, cfg, W) ==')
n_g5 = 0
for arm in ARMS:
    for cfg in CFGS:
        d = os.path.join(RAP, 'g5', arm)      # byte copies of the harness's gate/<arm>/g5/*.log
        if not os.path.isdir(d):
            continue
        for fn in sorted(os.listdir(d)):
            if not (fn.startswith(cfg + '_') and fn.endswith('.log')):
                continue
            text = read(os.path.join(d, fn))
            summ = next((l for l in text.splitlines() if l.startswith('SUMMARY ')), None)
            if summ is None:
                w = 1
            else:
                w = json.loads(summ[8:]).get('workers', 1)
            if w not in WS:
                w = 1
            ctx = cell(arm, cfg, w, log_path=os.path.join(d, fn))
            compare(f'C {arm} {cfg} {fn[len(cfg) + 1:-4]} (rc {ctx["rc"]}, judged at W{w})', ctx)
            n_g5 += 1
if n_g5 != 44:      # 2 arms x 2 configs x 11 logs per config (flip_0/617/1239, trunc, pad, other_cfg, other_arm, steps501, missing, counters off under --receipt, counters on timed)
    print(f'NOTE C: {n_g5} g5 runs compared')
print(f'C: {n_g5} g5 runs compared')


# ---- D. extra mutations
def summ_edit(fn):
    def go(c):
        lines = c['log_text'].splitlines()
        i = next(k for k, l in enumerate(lines) if l.startswith('SUMMARY '))
        d = json.loads(lines[i][8:])
        fn(d, c)
        lines[i] = 'SUMMARY ' + json.dumps(d, separators=(',', ':'))
        c['log_text'] = '\n'.join(lines) + '\n'
    return go


def csv_edit(fn):
    def go(c):
        lines = c['csv_text'].splitlines()
        c['csv_text'] = '\n'.join(fn(lines, c)) + '\n'
    return go


def drop_key(path):
    def f(d, c):
        o = d
        for p in path[:-1]:
            o = o[p]
        o.pop(path[-1], None)
    return f


EXTRA = [
    ('V1 exit code 2 (usage)', {'V1'}, lambda c: c.update(rc=2)),
    ('V1 exit code -1 (a crash)', {'V1'}, lambda c: c.update(rc=-1)),
    ('V1 exit None', {'V1'}, lambda c: c.update(rc=None)),
    ('V3 summary workers = W+1', {'V3'}, summ_edit(lambda d, c: d.update(workers=c['workers'] + 1))),
    ('V3 pool_threads_requested = W+1', {'V3'}, summ_edit(lambda d, c: d['threads'].update(pool_threads_requested=c['workers'] + 1))),
    ('V3 threads object absent', {'V3'}, summ_edit(drop_key(['threads']))),
    ('V3 install_receipt = [W]', {'V3'}, summ_edit(lambda d, c: d['threads'].update(install_receipt=[c['workers']]))),
    ('V3 CSV pool_threads row 0 = W+1', {'V3'}, csv_edit(lambda ls, c: [ls[0], ls[1].rsplit(',', 1)[0] + ',' + str(c['workers'] + 1)] + ls[2:])),
    ('V3 CSV pool_threads last row = W+1', {'V3'}, csv_edit(lambda ls, c: ls[:-1] + [ls[-1].rsplit(',', 1)[0] + ',' + str(c['workers'] + 1)])),
    ('V4 target_env gnu', {'V4'}, summ_edit(lambda d, c: d.update(target_env='gnu'))),
    ('V4 debug_assertions true', {'V4'}, summ_edit(lambda d, c: d.update(debug_assertions=True))),
    ('V4 simd_lanes swapped', {'V4'}, summ_edit(lambda d, c: d.update(simd_lanes=12 - d['simd_lanes']))),
    ('V4 features.parallel false', {'V4'}, summ_edit(lambda d, c: d['features'].update(parallel=False))),
    ('V4 target_features carries an extra key', {'V4'}, summ_edit(lambda d, c: d['target_features'].update(avx512f=True))),
    ('V4 target_features.fma false', {'V4'}, summ_edit(lambda d, c: d['target_features'].update(fma=False))),
    ('V4 bmi2 false', {'V4'}, summ_edit(lambda d, c: d['target_features'].update(bmi2=False))),
    ('V4 engine label', {'V4'}, summ_edit(lambda d, c: d.update(engine='rapier2d'))),
    ('V4 source_fnv1a64 cargo_lock edited', {'V4'}, summ_edit(lambda d, c: d['source_fnv1a64'].update(cargo_lock='0x0'))),
    ('V4 source_fnv1a64 absent', {'V4'}, summ_edit(drop_key(['source_fnv1a64']))),
    ('V4 exe sha256 one hex digit off', {'V4'}, lambda c: c.update(exe_sha256=c['exe_sha256'][:-1] + ('0' if c['exe_sha256'][-1] != '0' else '1'))),
    ('V4 exe sha256 MISSING (the exe is not there)', {'V4'}, lambda c: c.update(exe_sha256='MISSING')),
    ('V5 install step', {'V5'}, summ_edit(lambda d, c: d.update(install='step'))),
    ('V5 cfg label = the other config', {'V5'}, summ_edit(lambda d, c: d.update(cfg='matched' if c['cfg'] == 'rapier-default' else 'rapier-default'))),
    ('V5 counters_enabled field absent', {'V5'}, summ_edit(drop_key(['config', 'counters_enabled']))),
    ('V5 config object absent', {'V5'}, summ_edit(drop_key(['config']))),
    ('V5 dt edited', {'V5'}, summ_edit(lambda d, c: d['config'].update(dt=d['config']['dt'] * 2))),
    ('V5 can_sleep true', {'V5'}, summ_edit(lambda d, c: d['config'].update(can_sleep=True))),
    ('V6 colliders 1240', {'V6'}, summ_edit(lambda d, c: d['scene_identity'].update(colliders=1240))),
    ('V6 dynamic_bodies 1239', {'V6'}, summ_edit(lambda d, c: d['scene_identity'].update(dynamic_bodies=1239))),
    ('V6 uniform_boxes false', {'V6'}, summ_edit(lambda d, c: d['scene_identity'].update(uniform_boxes=False))),
    ('V6 inv_inertia_ulps_vs_ours [0,0,1]', {'V6'}, summ_edit(lambda d, c: d['scene_identity'].update(inv_inertia_ulps_vs_ours=[0, 0, 1]))),
    ('V6 inv_mass_ulps_vs_ours 1', {'V6'}, summ_edit(lambda d, c: d['scene_identity'].update(inv_mass_ulps_vs_ours=1))),
    ('V6 scene_identity absent', {'V6'}, summ_edit(drop_key(['scene_identity']))),
    ('V6 void true (with a reason)', {'V6'}, summ_edit(lambda d, c: d.update(void=True, voids=['pool size read != W']))),
    ('V6 void field absent', {'V6'}, summ_edit(drop_key(['void']))),
    ('V7 summary pose_hash edited', {'V7'}, summ_edit(lambda d, c: d.update(pose_hash='0x0000000000000001'))),
    ('V7 summary expect_pose absent', {'V7'}, summ_edit(drop_key(['expect_pose']))),
    ('V7 pose file one byte shorter', {'V7'}, lambda c: c.update(pose_raw=c['pose_raw'][:-1])),
    ('V7 pose file one byte longer', {'V7'}, lambda c: c.update(pose_raw=c['pose_raw'] + b'\0')),
    ('V7 pose file empty', {'V7'}, lambda c: c.update(pose_raw=b'')),
    ('V7 the OTHER config\'s fixture bytes', {'V7'}, lambda c: c.update(pose_raw=read(os.path.join(FIX, ('RM' if c['cfg'] == 'rapier-default' else 'RD')
                                                                                                 + ('8' if c['arm'] == 'simd8' else '4') + '.pose'), 'rb'))),
    ('V8 empty CSV', {'V8'}, lambda c: c.update(csv_text='')),
    ('V8 header columns reordered', {'V8'}, csv_edit(lambda ls, c: ['step,pool_threads,wall_ns'] + ls[1:])),
    ('V8 one CSV row too many', {'V8'}, csv_edit(lambda ls, c: ls + [ls[-1]])),
    ('V8 steps not 0..N-1 (rows 5 and 6 swapped)', {'V8'}, csv_edit(lambda ls, c: ls[:6] + [ls[7], ls[6]] + ls[8:])),
    ('V8 wall_ns negative in row 300', {'V8'}, csv_edit(lambda ls, c: ls[:301] + [','.join([ls[301].split(',')[0], '-5', ls[301].split(',')[2]])] + ls[302:])),
    ('V8 wall_ns not a number in row 3', {'V8'}, csv_edit(lambda ls, c: ls[:4] + [','.join([ls[4].split(',')[0], 'x', ls[4].split(',')[2]])] + ls[5:])),
    ('V9 receipt_gates.awake_min 1240 (a --receipt run)', {'V9'}, summ_edit(lambda d, c: d['receipt_gates'].update(awake_min=1240))),
    ('V9 the twin flag of this W absent', {'V9'}, lambda c: c['pins']['arms'][c['arm']]['cfgs'][c['cfg']]['twin_ok'].pop(str(c['workers']))),
    ('V2 SUMMARY line indented (not at the line start)', {'V2'}, lambda c: c.update(log_text=c['log_text'].replace('SUMMARY {', ' SUMMARY {', 1))),
    ('V2 truncated SUMMARY (cut in the middle)', {'V2'},
     lambda c: c.update(log_text='\n'.join((l[:len(l) // 2] if l.startswith('SUMMARY ') else l) for l in c['log_text'].splitlines()) + '\n')),
]
print('\n== D. extra mutations (rules the 33 do not reach), on four twins ==')
for arm, cfg, w in (('simd8', 'rapier-default', 8), ('simd4', 'matched', 16), ('simd8', 'matched', 1), ('simd4', 'rapier-default', 2)):
    for name, expect, fn in EXTRA:
        ctx = cell(arm, cfg, w)
        fn(ctx)
        compare(f'D {arm} {cfg} W{w} [{name}]', ctx, must_include=expect)

print('\n== E. two mutations at once (the rule sets must still agree) ==')
pairs = 0
import itertools  # noqa: E402
allm = [(n, e, f) for n, e, _a, f in MUT] + EXTRA
for (n1, e1, f1), (n2, e2, f2) in itertools.combinations(allm[::4], 2):
    ctx = cell('simd8', 'rapier-default', 4)
    f1(ctx)
    try:
        f2(ctx)
    except Exception:       # the second mutation needs a piece the first one removed: skip that pair, not the run
        continue
    compare(f'E [{n1}] + [{n2}]', ctx)
    pairs += 1
print(f'E: {pairs} pairs compared')

print('\n== F. cases only the driver can see (the reference has no such input); each must be RED in the driver ==')
F = []


def driver_only(label, ctx, want_rule, **kw):
    global n_cases, n_bad
    n_cases += 1
    args = dict(copy.deepcopy(ctx))
    args.update(kw)
    bad = R.rapier_check(**args)
    ok = want_rule in bad
    if not ok:
        n_bad += 1
        disagree.append(label)
    print(f'{"OK" if ok else "FAIL"} F {label}: driver {sorted(bad) or "VALID"} (must include {want_rule})')


base = cell('simd8', 'rapier-default', 8)
driver_only('launch arguments carry --receipt (the SUMMARY args do not)', base, 'V9', launch_args=['--cfg', 'rapier-default', '--receipt'])
driver_only('fixture bytes empty (two empty files must not compare equal)', dict(base, pose_raw=b'', fixture_raw=b''), 'V7')
driver_only('timed_out with exit 0 (a hang the driver terminated)', base, 'V1', timed_out=True)
driver_only('steps 12 expected but the CSV has 500 rows', base, 'V8', steps=12)
driver_only('W1: pool_threads_min a JSON true (bool is not the integer 1)',
            (lambda c: (summ_edit(lambda d, cc: d['threads'].update(pool_threads_min=True))(c), c)[1])(cell('simd8', 'rapier-default', 1)), 'V3')
# test=True skips V7 ONLY: a rehearsal's pose is not the 500-step fixture, everything else stays red
t = cell('simd8', 'rapier-default', 8)
t['pose_raw'] = b'x'
n_cases += 1
tb = R.rapier_check(**dict(t, test=True))
ok = 'V7' not in tb
n_bad += (not ok)
print(f'{"OK" if ok else "FAIL"} F test=True skips V7 (a rehearsal pose is not the fixture): driver {sorted(tb) or "VALID"}')
n_cases += 1
tb2 = R.rapier_check(**dict(t, test=True, rc=4))
ok = 'V1' in tb2
n_bad += (not ok)
print(f'{"OK" if ok else "FAIL"} F test=True still flags V1 (only V7 is skipped): driver {sorted(tb2)}')

print(f'\nxcheck_rapier9a: {n_cases} comparisons, {n_bad} disagreeing')
for d in disagree[:30]:
    print('  DISAGREE:', d)
sys.exit(1 if n_bad else 0)
