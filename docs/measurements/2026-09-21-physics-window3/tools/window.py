"""P0 timed window: MEASUREMENT-QUEUE section 10, five passes.

A thin wrapper over the UNMODIFIED tools/physics_parity/driver.py (imported from the tree, never
copied or edited). It reuses the driver's row table (ROWS), run order (plan_pass), argument
builders (boyko_args, jolt_args), byte-mode process runner (run_bytes), load receipt
(load_receipt), quiet rule and contamination flags, and adds only what the driver lacks:

1. A run whose receipts show load (the driver's own contaminated_before / contaminated_after) is
   RE-RUN ONCE at the end of its pass. A re-run of J-A-a at W also re-runs J-C at W, whose canary
   spin was derived from J-A-a's mean. If the re-run is contaminated too, the run is excluded from
   the timed reduction (kept in runs_all.jsonl, marked 'skipped' in runs.jsonl).
2. The stop rules, as we go: after every pass the driver's own `reduce` runs over the clean record
   set. A/A0 band > 2 % at any W voids the window (B is a max over passes, so it cannot recover),
   and a closure defect stops the analysis. Either one stops the window.
3. The machine is never timed busy: if the quiet rule's wait budget runs out before a timed run,
   the window aborts instead of timing (the driver would time it marked contaminated).

Pass numbers are 0..4 so that the first pass has H12's base order (W ascending, Jolt then boyko;
the driver's even-pass order), reversed on alternate passes. JOLT-P and JOLT-RCPT run in pass 1
only (the driver's `once` rule).
"""

import datetime
import json
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, r'D:/wt/joltab/tools/physics_parity')
import driver as D  # noqa: E402

P0 = r'(session scratch)/p0'
MANIFEST = os.path.join(P0, 'manifest.json')
BUSY_PCT = 5.0
# P0_WINDOW_TEST=1 is an UNTIMED rehearsal of this wrapper: the driver's dry step counts, a few rows,
# no quiet wait, written to p0/wtest. Its numbers are not measurements.
TEST = os.environ.get('P0_WINDOW_TEST') == '1'
if TEST:
    RAW = os.path.join(P0, 'wtest')
    PASSES = [0, 1]
    RECEIPT_WINDOW = 0.5
    WAIT_BUDGET = 0.0
    ONLY = {'J-A-d1', 'J-A-a', 'J-C', 'J-B', 'JOLT-T', 'J-A-d2', 'JOLT-RCPT'}
    WS_FILTER = {1, 8}
else:
    RAW = os.path.join(P0, 'raw')
    PASSES = [0, 1, 2, 3, 4]
    RECEIPT_WINDOW = 2.0
    WAIT_BUDGET = 420.0
    ONLY = None
    WS_FILTER = set(D.WS)
DRIVER = r'D:/wt/joltab/tools/physics_parity/driver.py'


def log(msg):
    line = f'{D.now()} {msg}'
    print(line, flush=True)
    with open(os.path.join(RAW, 'window_log.txt'), 'a', encoding='utf-8') as f:
        f.write(line + '\n')


def disk_free_gb(root):
    return shutil.disk_usage(root).free / 2**30


def machine_state():
    """Power source and scheme (a laptop part: boost depends on AC and on the plan)."""
    out = {}
    rc, so, se = D.run_bytes(['powercfg', '/getactivescheme'])
    out['powercfg_active'] = D.decode(so).strip()
    rc, so, se = D.run_bytes(['powershell.exe', '-NoProfile', '-Command',
                              'Get-CimInstance Win32_Battery | Select-Object BatteryStatus,EstimatedChargeRemaining '
                              '| ConvertTo-Json -Compress; (Get-Process).Count'])
    out['battery_and_proc_count'] = D.decode(so).strip()
    return out


def do_run(m, row, w, seq, tag, pass_no, pdir, ctx, receipt, env):
    """One run, exactly as driver.cmd_pass's loop body does it. Returns (rec, receipt, abort)."""
    role = row['role']
    entry = (m['boyko'] if row['engine'] == 'boyko' else m['jolt']).get(role)
    rec = {'pass': pass_no, 'seq': seq, 'row': row['id'], 'engine': row['engine'], 'role': role, 'W': w,
           'dry': TEST, 'attempt': tag}
    if entry is None:
        rec['skipped'] = f'no {row["engine"]} binary for role {role!r} in the manifest'
        return rec, receipt, False
    suffix = '' if tag == 'original' else '_rerun'
    cwd = os.path.join(pdir, f'{seq:03d}_{row["id"]}_W{w}{suffix}')
    os.makedirs(cwd, exist_ok=True)
    if row['engine'] == 'boyko':
        args, why = D.boyko_args(row, w, TEST, cwd, ctx)
    else:
        args, why = D.jolt_args(row, w, TEST)
    if args is None:
        rec['skipped'] = why
        return rec, receipt, False
    waited = 0.0
    if row['timed'] and not TEST:
        t0 = time.time()
        while (receipt['cpu_avg'] or 0) > BUSY_PCT and time.time() - t0 < WAIT_BUDGET:
            time.sleep(20)
            receipt = D.load_receipt(RECEIPT_WINDOW)
        waited = time.time() - t0
        if (receipt['cpu_avg'] or 0) > BUSY_PCT:
            rec.update({'receipt_before': receipt, 'waited_s': round(waited, 1),
                        'aborted': f'machine busy after the {WAIT_BUDGET:.0f} s wait budget; not timed'})
            return rec, receipt, True
    rec.update({'exe': entry['exe'], 'exe_sha256': entry['sha256'], 'args': args, 'cwd': cwd,
                'receipt_before': receipt, 'waited_s': round(waited, 1),
                'contaminated_before': (receipt['cpu_avg'] or 0) > BUSY_PCT or bool(receipt['build_procs_busy'])})
    rec['start'] = D.now()
    t_start = time.perf_counter()
    code, so, se = D.run_bytes([entry['exe']] + args, cwd=cwd, env=env)
    rec['wall_s'] = round(time.perf_counter() - t_start, 3)
    rec['end'] = D.now()
    rec['exit'] = code
    with open(os.path.join(cwd, 'stdout.txt'), 'wb') as f:
        f.write(so)
    with open(os.path.join(cwd, 'stderr.txt'), 'wb') as f:
        f.write(se)
    receipt = D.load_receipt(RECEIPT_WINDOW)
    rec['receipt_after'] = receipt
    rec['contaminated_after'] = (receipt['cpu_avg'] or 0) > BUSY_PCT or bool(receipt['build_procs_busy'])
    if row['engine'] == 'boyko':
        rec['summary'] = D.parse_summary(so)
        rec['csv'] = os.path.join(cwd, 'run.csv')
        rec['pose'] = os.path.join(cwd, 'pose.bin')
    else:
        rec['jolt'] = D.parse_jolt_stdout(so)
        files = sorted(os.listdir(cwd))
        rec['files'] = files
        pf = [f for f in files if f.startswith('per_frame_')]
        rc = [f for f in files if f.startswith('receipt_')]
        rec['per_frame'] = os.path.join(cwd, pf[0]) if pf else None
        rec['receipt_csv'] = os.path.join(cwd, rc[0]) if rc else None
    ctx[(row['id'], w)] = rec
    return rec, receipt, False


def tag_of(rec):
    if 'aborted' in rec:
        return 'ABORTED'
    if 'skipped' in rec:
        return f'SKIPPED ({rec["skipped"]})'
    if rec['engine'] == 'boyko':
        s = rec.get('summary')
        t = (f'void {s["void_steps"]} hash {s["pose_hash"]} pose {s["expect_pose"]} mean_ns {s["window_mean_ns"]:.0f}'
             if s else 'NO SUMMARY')
    else:
        st = rec['jolt']['stat_lines']
        t = f'hash {st[0]["hash"]} threads {st[0]["threads"]} steps/s {st[0]["steps_per_s"]}' if st else 'NO STAT LINE'
    c = []
    if rec.get('contaminated_before'):
        c.append(f'CONTAMINATED-before({rec["receipt_before"]["cpu_avg"]}%)')
    if rec.get('contaminated_after'):
        c.append(f'CONTAMINATED-after({rec["receipt_after"]["cpu_avg"]}%)')
    flag = '' if rec.get('exit') == 0 else f' EXIT {rec.get("exit")}'
    return f'{t}{flag} rb {rec["receipt_before"]["cpu_avg"]}% ra {rec["receipt_after"]["cpu_avg"]}% ' \
           f'waited {rec["waited_s"]}s wall {rec["wall_s"]}s {" ".join(c)}'


def append(path, rec):
    with open(path, 'a', encoding='utf-8') as f:
        f.write(json.dumps(rec) + '\n')


def run_pass(m, pass_no, env):
    pdir = os.path.join(RAW, f'pass-{pass_no:02d}')
    os.makedirs(pdir, exist_ok=True)
    order = D.plan_pass(pass_no, WS_FILTER, ONLY, TEST)
    for role, e in list(m['boyko'].items()) + list(m['jolt'].items()):
        got = D.sha256(e['exe'])
        if got != e['sha256']:
            log(f'STOP: {role} {e["exe"]} sha256 {got} != manifest {e["sha256"]}')
            return 'abort', []
    ctx = {}
    finals = {}
    all_path = os.path.join(RAW, 'runs_all.jsonl')
    receipt = D.load_receipt(RECEIPT_WINDOW)
    log(f'pass {pass_no}: {len(order)} runs; order {"W ascending, Jolt then boyko" if pass_no % 2 == 0 else "W descending, boyko then Jolt"}')
    t_pass = D.now()
    for seq, (row, w) in enumerate(order):
        rec, receipt, abort = do_run(m, row, w, seq, 'original', pass_no, pdir, ctx, receipt, env)
        append(all_path, rec)
        log(f'  p{pass_no} [{seq:3d}] {row["id"]} W={w}: {tag_of(rec)}')
        if abort:
            return 'abort', list(finals.values())
        finals[(row['id'], w)] = (seq, row, rec)
    # Re-run once, at the end of the pass, every timed run whose receipts show load.
    rerun = [(seq, row, w) for (rid, w), (seq, row, rec) in finals.items()
             if row['timed'] and 'skipped' not in rec and (rec.get('contaminated_before') or rec.get('contaminated_after'))]
    ids = {(row['id'], w) for _, row, w in rerun}
    for seq, row, w in list(rerun):
        if row['id'] == 'J-A-a' and ('J-C', w) in finals and ('J-C', w) not in ids:
            cseq, crow, _ = finals[('J-C', w)]
            rerun.append((cseq, crow, w))
            ids.add(('J-C', w))
    rerun.sort(key=lambda x: x[0])
    if rerun:
        log(f'pass {pass_no}: re-running {len(rerun)} contaminated run(s): {[(row["id"], w) for _, row, w in rerun]}')
    for seq, row, w in rerun:
        rec, receipt, abort = do_run(m, row, w, seq, 'rerun', pass_no, pdir, ctx, receipt, env)
        rec['rerun_reason'] = 'dependency: J-A-a re-run' if row['id'] == 'J-C' and not (
            finals[(row['id'], w)][2].get('contaminated_before') or finals[(row['id'], w)][2].get('contaminated_after')) \
            else 'contaminated'
        append(all_path, rec)
        log(f'  p{pass_no} [{seq:3d}] {row["id"]} W={w} RERUN: {tag_of(rec)}')
        if abort:
            return 'abort', [r for _, _, r in finals.values()]
        finals[(row['id'], w)] = (seq, row, rec)
    clean = []
    for (rid, w), (seq, row, rec) in sorted(finals.items(), key=lambda kv: kv[1][0]):
        r = dict(rec)
        if row['timed'] and 'skipped' not in r and r.get('attempt') == 'rerun' and \
                (r.get('contaminated_before') or r.get('contaminated_after')):
            r['skipped'] = 'contaminated on the original run and on its re-run'
            log(f'  p{pass_no} {rid} W={w}: EXCLUDED from the timed reduction (contaminated twice)')
        clean.append(r)
    for r in clean:
        append(os.path.join(RAW, 'runs.jsonl'), r)
    log(f'pass {pass_no}: done ({t_pass} .. {D.now()})')
    return 'ok', clean


def reduce_and_check(pass_no):
    rc, so, se = D.run_bytes([sys.executable, DRIVER, 'reduce', '--out', RAW] + (['--dry'] if TEST else []))
    with open(os.path.join(RAW, f'reduce_after_pass{pass_no}.txt'), 'wb') as f:
        f.write(so + b'\n--- stderr ---\n' + se)
    if rc != 0:
        log(f'reduce after pass {pass_no}: EXIT {rc}; stderr {D.decode(se)[-600:]}')
        return 'reduce-failed'
    shutil.copyfile(os.path.join(RAW, 'reduction.json'), os.path.join(RAW, f'reduction_after_pass{pass_no}.json'))
    R = json.load(open(os.path.join(RAW, 'reduction.json'), encoding='utf-8'))
    bands = {w: v.get('B') for w, v in R['headline'].items()}
    void = [w for w, v in R['headline'].items() if v.get('aa0_void')]
    log(f'after pass {pass_no}: B(W) {bands}; A/A0 void at {void}; closure stop {R["closure"]["stop"]}; '
        f'anti-vacuity void {len(R["anti_vacuity_void"])}; nonzero exits {R["nonzero_exits"]}; '
        f'determinism groups >1 pose {[k for k, v in R["determinism_boyko"].items() if not v["ok"]]}')
    if void:
        return f'VOID: A/A0 band > 2 % at W {void} (B {bands})'
    if R['closure']['stop']:
        return f'STOP: closure defect {R["closure"]}'
    return 'ok'


def main():
    os.makedirs(RAW, exist_ok=True)
    m = json.load(open(MANIFEST, encoding='utf-8'))
    with open(os.path.join(RAW, 'manifest.json'), 'w', encoding='utf-8') as f:
        json.dump(m, f, indent=1)
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    state = {'window_start': D.now(), 'machine_before': machine_state(),
             'disk_free_gb_before': {'C': disk_free_gb('C:/'), 'D': disk_free_gb('D:/')}}
    log(f'WINDOW START {state}')
    status = 'complete'
    for p in PASSES:
        if disk_free_gb('D:/') < 2.0 or disk_free_gb('C:/') < 2.0:
            status = f'STOP before pass {p}: disk below 2 GB'
            break
        st, _ = run_pass(m, p, env)
        if st == 'abort':
            status = f'ABORTED in pass {p}: the machine was busy (or a binary changed); see window_log.txt'
            break
        verdict = reduce_and_check(p)
        if verdict != 'ok':
            status = f'{verdict} (after pass {p})'
            break
    state['window_end'] = D.now()
    state['machine_after'] = machine_state()
    state['disk_free_gb_after'] = {'C': disk_free_gb('C:/'), 'D': disk_free_gb('D:/')}
    state['status'] = status
    log(f'WINDOW END {state}')
    with open(os.path.join(RAW, 'window_state.json'), 'w', encoding='utf-8') as f:
        json.dump(state, f, indent=1)
    with open(os.path.join(RAW, 'WINDOW_DONE'), 'w', encoding='utf-8') as f:
        f.write(status + '\n')


if __name__ == '__main__':
    main()
