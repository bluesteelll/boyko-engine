"""G4 of the tree-broadphase recipe (scratchpad/treebp/g4_g5_recipe.md section 1): the criterion benches under the
window-3 protocol block. Launch path, receipts and the during-process witness are window4_run.py's (imported);
nothing is compiled here.

Block A: the four bp_g4_* groups, K=3 whole-group processes, interleaved (u d s m, u d s m, u d s m).
Block B: the 24 row_identity_churn cells, K=6, one cell per process, allpairs/tree alternating, the
order reversed on odd k.

Per process: a 5-s load receipt before and after (> 5 % busy, or a build process using CPU => contaminated),
the during-process witness (others' CPU seconds, top-5 names; a build-process name seen during, or others
> 5 % of the machine => voided), stdout/stderr kept under raw/g4/logs/, and criterion's estimates under
raw/g4/<group>/<arm>/<param>/k<k>/ (CRITERION_HOME = raw/g4). Contaminated/voided processes are re-run ONCE
at the end of their block under the baseline name k<k>r after the idle rule; the original is kept. Before
each block: the idle rule (tools/wait_idle3.ps1, 3 consecutive quiet 60-s polls, 10-s cpu < 5 %), then a
10-s receipt.

G4_TEST=1 is an untimed rehearsal: one/two processes per block, no idle wait, 0.5-s receipts, short
criterion times, under test/g4/.
"""
import json
import os
import subprocess
import sys
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W4 = os.path.dirname(HERE)
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, os.path.join(HERE, 'window_lib'))
import driver as D  # noqa: E402
import window4_run as W  # noqa: E402  (launch, percpu witness, wait_idle3.ps1 path; its main() is not run)
from pdhperf import PerfSampler  # noqa: E402

TEST = os.environ.get('G4_TEST') == '1'
RAW = os.path.join(W4, 'test', 'g4') if TEST else os.path.join(W4, 'raw', 'g4')
LOGS = os.path.join(RAW, 'logs')
WAIT_LOG = os.path.join(RAW, 'wait_log.txt')
RECEIPT_S = 0.5 if TEST else 5.0
BLOCK_RECEIPT_S = 0.5 if TEST else 10.0
BUSY_PCT = 5.0
IDLE_BUDGET_S = 90 * 60.0
DEPS = 'D:/wt/_targets/mq-de06b6c9/release/deps'
EXES = {'broadphase': os.path.join(DEPS, 'broadphase-595045019352a67f.exe'),
        'churn': os.path.join(DEPS, 'row_identity_churn-ec1df0bd330016d8.exe')}
GROUPS = ['bp_g4_uniform', 'bp_g4_disparity', 'bp_g4_scene', 'bp_g4_maintenance']
CHURN_ARMS = ['stable', 'swap_churn', 'archetype_shift', 'first_archetype_spawn', 'burst_despawn', 'burst_migrate']
K_GROUPS = 1 if TEST else 3
K_CHURN = 1 if TEST else 6


def churn_cells():
    out = []
    for sl in ('sleeping_off', 'sleeping_on'):
        for arm in CHURN_ARMS:
            for bp in ('allpairs', 'tree'):
                out.append(f'row_identity_churn/{arm}/{sl}_{bp}')
    return out


def log(msg):
    line = f'{D.now()} {msg}'
    print(line, flush=True)
    with open(os.path.join(RAW, 'g4_log.txt'), 'a', encoding='utf-8') as f:
        f.write(line + '\n')


def busy(receipt):
    return (receipt['cpu_avg'] or 0) > BUSY_PCT or bool(receipt['build_procs_busy'])


def wait_idle(deadline):
    if TEST:
        return 0
    # Resume 03:40: a fresh 90-min budget per wait (the one-shot deadline set at start would leave only
    # the 3-poll minimum for every wait after the first ~90 min of a multi-hour run).
    remaining_polls = max(3, int(IDLE_BUDGET_S // 60))
    return subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', W.WAIT_PS1,
                           '-Log', WAIT_LOG, '-MaxPolls', str(remaining_polls)]).returncode


def sha_of(exe):
    for line in open(os.path.join(W4, 'bin', 'SHA256SUMS'), encoding='utf-8'):
        sha, name = line.split()
        if name.lstrip('*').replace('\\', '/') == exe.replace('\\', '/'):
            return sha
    return None


def estimates_for(filter_root, baseline):
    """Every <RAW>/<group>/<arm>/<param>/<baseline>/estimates.json under the group (or the one cell)."""
    root = os.path.join(RAW, *filter_root.split('/'))
    found = []
    for dp, _dn, fn in os.walk(root):
        if os.path.basename(dp) == baseline and 'estimates.json' in fn:
            e = json.load(open(os.path.join(dp, 'estimates.json'), encoding='utf-8'))
            rel = os.path.relpath(dp, RAW).replace('\\', '/')
            found.append({'cell': rel[: -len('/' + baseline)], 'baseline': baseline,
                          'median_ns': e['median']['point_estimate'], 'median_se_ns': e['median']['standard_error'],
                          'mean_ns': e['mean']['point_estimate']})
    found.sort(key=lambda x: x['cell'])
    return found


def run_one(block, k, exe_key, bench_filter, filter_root, baseline, tag, receipt, env, sampler, deadline, seq):
    exe = EXES[exe_key]
    name = f'{seq:03d}_{block}_k{k}_{filter_root.replace("/", "_")}{"" if tag == "original" else "_" + tag}'
    args = ['--bench', bench_filter, '--save-baseline', baseline, '--noplot']
    if TEST:
        args += ['--warm-up-time', '0.2', '--measurement-time', '0.5']
    rec = {'block': block, 'k': k, 'seq': seq, 'attempt': tag, 'exe': exe, 'exe_sha256': sha_of(exe), 'args': args,
           'filter_root': filter_root, 'baseline': baseline, 'policy': 'P-none', 'dry': TEST}
    t0 = time.time()
    while busy(receipt) and not TEST:
        if time.time() - t0 >= W.QUIET_BUDGET_S:
            log(f'  quiet budget exhausted before {name}; running the idle rule')
            rc = wait_idle(deadline)
            if rc != 0:
                rec['aborted'] = f'machine never idle (wait_idle exit {rc})'
                rec['receipt_before'] = receipt
                return rec, receipt, 'abort'
            receipt = D.load_receipt(RECEIPT_S)
            t0 = time.time()
            continue
        time.sleep(20)
        receipt = D.load_receipt(RECEIPT_S)
    rec['waited_s'] = round(time.time() - t0, 1)
    rec['receipt_before'] = receipt
    rec['start'] = D.now()
    info, so, se = W.launch([exe] + args, LOGS, env, sampler)
    rec.update(info)
    rec['end'] = D.now()
    with open(os.path.join(LOGS, name + '.stdout.txt'), 'wb') as f:
        f.write(so)
    with open(os.path.join(LOGS, name + '.stderr.txt'), 'wb') as f:
        f.write(se)
    receipt = D.load_receipt(RECEIPT_S)
    rec['receipt_after'] = receipt
    rec['contaminated_before'] = busy(rec['receipt_before'])
    rec['contaminated_after'] = busy(receipt)
    seen = sorted({t['name'] for t in rec.get('others_top5', []) if t['name'].lower() in D.BUILD_PROCS})
    rec['build_procs_during'] = seen
    rec['voided_during'] = bool(seen) or (rec.get('others_busy_pct') or 0) > BUSY_PCT
    rec['contaminated'] = rec['contaminated_before'] or rec['contaminated_after'] or rec['voided_during']
    rec['receipts'] = [l for l in D.decode(se).splitlines() if l.startswith(('bp_g4_', 'row_identity_churn/'))]
    rec['estimates'] = estimates_for(filter_root, baseline)
    rec['valid'] = rec['exit'] == 0 and bool(rec['estimates'])
    return rec, receipt, 'ok'


def tag_of(rec):
    if 'aborted' in rec:
        return f'ABORTED ({rec["aborted"]})'
    flags = []
    if rec.get('contaminated_before'):
        flags.append(f'CONTAMINATED-before({rec["receipt_before"]["cpu_avg"]}%)')
    if rec.get('contaminated_after'):
        flags.append(f'CONTAMINATED-after({rec["receipt_after"]["cpu_avg"]}%)')
    if rec.get('voided_during'):
        flags.append(f'VOIDED-during(build {rec["build_procs_during"]}, others {rec.get("others_busy_pct")}%)')
    if not rec.get('valid'):
        flags.append(f'INVALID(exit {rec.get("exit")}, {len(rec.get("estimates", []))} estimates)')
    return (f'exit {rec.get("exit")} wall {rec.get("wall_s")}s cpu_s {rec.get("proc_cpu_s")} '
            f'rb {rec["receipt_before"]["cpu_avg"]}% ra {rec["receipt_after"]["cpu_avg"]}% others {rec.get("others_busy_pct")}% '
            f'estimates {len(rec.get("estimates", []))} {" ".join(flags)}').rstrip()


def append(rec):
    with open(os.path.join(RAW, 'runs.jsonl'), 'a', encoding='utf-8') as f:
        f.write(json.dumps(rec) + '\n')


def main():
    os.makedirs(LOGS, exist_ok=True)
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    env['CRITERION_HOME'] = RAW
    for exe in EXES.values():
        got = D.sha256(exe)
        if got != sha_of(exe):
            sys.exit(f'{exe}: sha256 {got} != bin/SHA256SUMS')
    plan_a = [(k, g) for k in range(1, K_GROUPS + 1) for g in GROUPS]
    cells = churn_cells()
    plan_b = []
    for k in range(1, K_CHURN + 1):
        order = cells[::-1] if k % 2 == 1 else cells
        plan_b += [(k, c) for c in order]
    if TEST:
        plan_a = [(1, 'bp_g4_maintenance')]
        plan_b = plan_b[:2]
    log(f'G4 START test={TEST} block A {len(plan_a)} processes, block B {len(plan_b)} processes; '
        f'exes {json.dumps({k: (v, sha_of(v)) for k, v in EXES.items()})}; state {W.machine_state()}')
    deadline = time.time() + IDLE_BUDGET_S
    sampler = PerfSampler(1.0)
    status = 'complete'
    seq = 1
    for block, plan in (('A', plan_a), ('B', plan_b)):
        log(f'block {block}: idle rule (wait_idle3.ps1)')
        rc = wait_idle(deadline)
        if rc != 0:
            status = f'STOP before block {block}: idle rule not met (wait_idle exit {rc})'
            break
        receipt = D.load_receipt(BLOCK_RECEIPT_S)
        log(f'block {block}: idle reached; opening receipt {receipt["cpu_avg"]}% (build procs {receipt["build_procs_busy"]}); {W.machine_state()}')
        bad = []
        for k, target in plan:
            if block == 'A':
                exe_key, flt, root = 'broadphase', f'^{target}/', target
            else:
                exe_key, flt, root = 'churn', f'^{target}$', target
            rec, receipt, st = run_one(block, k, exe_key, flt, root, f'k{k}', 'original', receipt, env, sampler, deadline, seq)
            append(rec)
            log(f'  {block} k{k} [{seq:3d}] {root}: {tag_of(rec)}')
            if st == 'abort':
                status = f'ABORT in block {block}: {rec.get("aborted")}'
                break
            if rec['contaminated'] or not rec['valid']:
                bad.append((seq, k, target, exe_key, flt, root, 'contaminated' if rec['contaminated'] else 'invalid'))
            if rec['voided_during']:
                log(f'  build process during {root}: waiting for idle before the next process')
                rc = wait_idle(deadline)
                if rc != 0:
                    status = f'ABORT in block {block}: idle rule not met after a voided process (exit {rc})'
                    break
                receipt = D.load_receipt(RECEIPT_S)
            seq += 1
        if status != 'complete':
            break
        if bad:
            log(f'block {block}: re-running {len(bad)} process(es) at the end of the block: {[(s, k, t, w) for s, k, t, _, _, _, w in bad]}')
            rc = wait_idle(deadline)
            if rc != 0:
                status = f'ABORT in block {block} (re-runs): idle rule not met (exit {rc})'
                break
            receipt = D.load_receipt(RECEIPT_S)
        for oseq, k, target, exe_key, flt, root, why in bad:
            rec, receipt, st = run_one(block, k, exe_key, flt, root, f'k{k}r', 'rerun', receipt, env, sampler, deadline, oseq)
            rec['rerun_reason'] = why
            append(rec)
            log(f'  {block} k{k} [{oseq:3d}] {root} RERUN ({why}): {tag_of(rec)}')
            if st == 'abort':
                status = f'ABORT in block {block} (re-runs): {rec.get("aborted")}'
                break
        if status != 'complete':
            break
        log(f'block {block}: done')
    sampler.close()
    log(f'G4 END status {status}; {W.machine_state()}')
    with open(os.path.join(RAW, 'G4_DONE'), 'w', encoding='utf-8') as f:
        f.write(status + '\n')


if __name__ == '__main__':
    main()
