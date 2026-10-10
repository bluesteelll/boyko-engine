"""WINDOW 4b: L11 C2 (tip, f8873aae) against its parent C0 (146a1125) under the window-3 protocol block
(P0b's ruling), G9 of 02-DESIGN-REV1.md.

Adapted from window 3's window3_run.py (same launch path, receipts, witnesses, record layout). Differences:
  * two BINARIES of one runner interface ('parent', 'tip') instead of two engines; every row runs on both,
    interleaved inside a W group: row by row, parent then tip; pass 1 is the whole list reversed;
  * no Jolt (owner ruling: v5.6.0 only, and its window-3 cells are reused, never re-run);
  * three blocks, each with its own passes: 'main' (J-As, J-A at W 1/2/4/8/16; R, R-S at W 1/8; S16 at W 1),
    'armed' (J-As-a at W 1/8, --arm-profiler), and 'canary' (J-C once per binary at W 1, armed, with
    --canary-frac 0.05 --canary-ref-ns <this binary's latest valid J-As-a W1 window mean>);
  * every process gets --expect-pose gate/<row>_W1_parent.pose (the parent's pose from the untimed pose gate;
    the pose is W-independent, shown at the gate for W 1 and 8) and its summary pose_hash is asserted
    against rows.json; a differing pose is INVALID (re-run once, and reported).

Protocol (binding):
  * a (row, binary, W) cell = the MEDIAN over K separate processes of the process's window mean;
    spread = min-max, IQR; the median's SE 1.2533*SD/sqrt(K) beside it;
  * a 5-s load receipt between every two processes (the AFTER of one, the BEFORE of the next); a 10-s
    receipt opens each pass; > 5 % busy (or a build process using CPU) => that process is re-run ONCE at
    the end of its pass; a build process seen DURING a process (others_top5 names one) is reported;
  * no band-based void; placement P-none (CreateProcess suspended -> read the mask back -> resume);
  * idle rule (P0) before each pass: tools/wait_idle4b.ps1 -- 0 build/miri/lane-test processes on 3
    consecutive 60-s polls with the 10-s CPU average < 5 %.

WIN4B_TEST=1 is an UNTIMED rehearsal (tiny step counts, one round, no idle wait) under test/.
"""
import ctypes
import datetime
import json
import os
import statistics
import subprocess
import sys
import time
from ctypes import wintypes

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W4 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, os.path.join(HERE, 'window_lib'))
import driver as D  # noqa: E402  (window 3's byte-identical copy of the tree driver; only helpers are used)
from pdhperf import PerfSampler  # noqa: E402

WAIT_PS1 = os.path.join(HERE, 'wait_idle4b.ps1')
TEST = os.environ.get('WIN4B_TEST') == '1'
RAW = os.path.join(W4, 'test', 'window') if TEST else os.path.join(W4, 'raw')
WAIT_LOG = os.path.join(RAW, 'wait_log.txt')
BIN = os.path.join(W4, 'bin')
GATE = os.path.join(W4, 'gate')
BUSY_PCT = 5.0
RECEIPT_S = 0.5 if TEST else 5.0
PASS_RECEIPT_S = 0.5 if TEST else 10.0
QUIET_BUDGET_S = 420.0
IDLE_BUDGET_S = 2 * 3600.0
CANARY_FRAC = 0.05
BINARIES = ('parent', 'tip')

ROWS_JSON = json.load(open(os.path.join(W4, 'rows.json'), encoding='utf-8'))
PROTO = ROWS_JSON['protocol']
ARMED_ROUNDS = int(os.environ.get('WIN4B_ARMED_ROUNDS', PROTO['armed_block']['rounds_per_pass']))
BLOCKS = [
    {'name': 'main', 'passes': 1 if TEST else PROTO['passes'], 'rounds': 1 if TEST else PROTO['rounds_per_pass']},
    {'name': 'armed', 'passes': 1 if TEST else PROTO['armed_block']['passes'], 'rounds': 1 if TEST else ARMED_ROUNDS},
    {'name': 'canary', 'passes': 1, 'rounds': 1},
]
ROWS = {}
for rd in ROWS_JSON['rows']:
    r = dict(rd)
    r['ws'] = tuple(rd['workers'])
    r['per_w'] = {int(k): v for k, v in rd.get('per_w', {}).items()}
    r['dry_steps'] = 12 if rd['steps'] == 500 else (30 if rd['steps'] >= 800 else 10)
    r['dry_window'] = (0, r['dry_steps']) if rd['window'][0] == 0 else (r['dry_steps'] - 10, r['dry_steps'])
    r.setdefault('canary_of', None)
    r.setdefault('frozen_by', None)
    r.setdefault('armed', False)
    ROWS[rd['id']] = r
MANIFEST_ENTRIES = {}
for role, e in ROWS_JSON['binaries'].items():
    MANIFEST_ENTRIES[role] = {'exe': os.path.join(W4, e['exe'].replace('bin/', 'bin' + os.sep, 1)), 'sha256': e['sha256'],
                              'commit': e['commit']}
TEST_CELLS = {('J-As', 'parent', 1), ('J-As', 'tip', 1), ('R', 'tip', 8), ('R-S', 'parent', 1), ('S16', 'tip', 1),
              ('J-As-a', 'parent', 1), ('J-As-a', 'tip', 1), ('J-C', 'parent', 1), ('J-C', 'tip', 1), ('J-A', 'tip', 16)}

CREATE_SUSPENDED = 0x4
k32 = ctypes.WinDLL('kernel32', use_last_error=True)
k32.GetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)]
k32.GetProcessAffinityMask.restype = wintypes.BOOL
k32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
k32.GetProcessTimes.restype = wintypes.BOOL
k32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
ntdll = ctypes.WinDLL('ntdll')
ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]
ntdll.NtResumeProcess.restype = ctypes.c_long
ntdll.NtQuerySystemInformation.argtypes = [ctypes.c_int, ctypes.c_void_p, wintypes.ULONG, ctypes.POINTER(wintypes.ULONG)]
ntdll.NtQuerySystemInformation.restype = ctypes.c_long
NCPU = os.cpu_count()
MY_PID = os.getpid()


class SPPI(ctypes.Structure):  # SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION
    _fields_ = [('IdleTime', ctypes.c_longlong), ('KernelTime', ctypes.c_longlong), ('UserTime', ctypes.c_longlong),
                ('DpcTime', ctypes.c_longlong), ('InterruptTime', ctypes.c_longlong), ('InterruptCount', ctypes.c_ulong)]


def percpu():
    arr = (SPPI * NCPU)()
    ret = wintypes.ULONG(0)
    if ntdll.NtQuerySystemInformation(8, ctypes.byref(arr), ctypes.sizeof(arr), ctypes.byref(ret)) != 0:
        return None
    return [(a.IdleTime, a.KernelTime + a.UserTime) for a in arr]


def percpu_busy(a, b):
    if a is None or b is None:
        return None
    out = []
    for (i0, t0), (i1, t1) in zip(a, b):
        dt = t1 - t0
        out.append(round(100.0 * (1.0 - (i1 - i0) / dt), 1) if dt > 0 else None)
    return out


def ft(f):
    return ((f.dwHighDateTime << 32) | f.dwLowDateTime) / 1e7


def others_cpu(p0, p1, exclude):
    out = []
    for pid, (name, cpu) in p1.items():
        if pid in exclude:
            continue
        prev = p0.get(pid)
        d = cpu - (prev[1] if prev and prev[0] == name else 0.0)
        if d > 0.0005:
            out.append((round(d, 3), name, pid))
    out.sort(reverse=True)
    return out


def log(msg):
    line = f'{D.now()} {msg}'
    print(line, flush=True)
    with open(os.path.join(RAW, 'window_log.txt'), 'a', encoding='utf-8') as f:
        f.write(line + '\n')


def busy(receipt):
    return (receipt['cpu_avg'] or 0) > BUSY_PCT or bool(receipt['build_procs_busy'])


def launch(cmd, cwd, env, sampler):
    """P-none launch, identical to window 3's: CreateProcess suspended, read the (inherited) mask back,
    resume, wait. No SetProcessAffinityMask call is made."""
    t_tab0 = D.process_cpu_table()
    p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         creationflags=CREATE_SUSPENDED)
    h = int(p._handle)
    info = {'pid': p.pid, 'mask_requested': None}
    pm, sm = ctypes.c_size_t(), ctypes.c_size_t()
    k32.GetProcessAffinityMask(h, ctypes.byref(pm), ctypes.byref(sm))
    info['mask_readback'] = hex(pm.value)
    info['system_mask'] = hex(sm.value)
    c0 = percpu()
    if sampler is not None:
        sampler.start()
    t0 = time.perf_counter()
    st = ntdll.NtResumeProcess(h)
    if st != 0:
        k32.TerminateProcess(h, 97)
        p.communicate()
        raise OSError(f'NtResumeProcess failed: {st:#x}')
    so, se = p.communicate()
    info['wall_s'] = round(time.perf_counter() - t0, 3)
    c1 = percpu()
    perf = sampler.stop() if sampler is not None else []
    info['exit'] = p.returncode
    info['percpu_busy'] = percpu_busy(c0, c1)
    c, x, kk, u = (wintypes.FILETIME() for _ in range(4))
    if k32.GetProcessTimes(h, ctypes.byref(c), ctypes.byref(x), ctypes.byref(kk), ctypes.byref(u)):
        info['proc_cpu_s'] = round(ft(kk) + ft(u), 3)
    t_tab1 = D.process_cpu_table()
    oth = others_cpu(t_tab0, t_tab1, {p.pid, MY_PID})
    info['others_cpu_s'] = round(sum(d for d, _, _ in oth), 3)
    info['others_busy_pct'] = round(100 * info['others_cpu_s'] / (info['wall_s'] * NCPU), 2) if info['wall_s'] else None
    info['others_top5'] = [{'name': n, 'pid': pid, 'cpu_s': d} for d, n, pid in oth[:5]]
    info['build_proc_during'] = sorted({n for d, n, pid in oth if n.lower() in D.BUILD_PROCS})
    keys = [f'0,{i}' for i in range(NCPU)]
    info['perf'] = [{'t': t, 'total': v.get('_Total'), 'cpu': [v.get(k) for k in keys]} for t, v in perf]
    return info, so, se


def boyko_args(row, w, cwd, ctx, role):
    steps = row['dry_steps'] if TEST else row['steps']
    window = row['dry_window'] if TEST else row['window']
    args = list(row['args']) + row['per_w'].get(w, [])
    args += ['--workers', str(w), '--steps', str(steps), '--window', f'{window[0]}..{window[1]}',
             '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
             '--label', f'{row["id"]}#{role}@W{w}']
    if row['armed']:
        args.append('--arm-profiler')
    if row['frozen_by'] and not TEST:
        args += ['--frozen-by', str(row['frozen_by'])]
    if row['canary_of']:
        ref = ctx.get((row['canary_of'], role, w))
        if ref is None or not ref.get('summary'):
            return None, f'no {row["canary_of"]} on {role} at W={w} for the canary reference'
        t = ref['summary']['window_mean_ns']
        args += ['--canary-frac', str(CANARY_FRAC), '--canary-ref-ns', str(int(round(t)))]
    gate_pose = os.path.join(GATE, f'{row["id"]}_W1_parent.pose')
    if not TEST and os.path.isfile(gate_pose):
        args += ['--expect-pose', gate_pose]
    return args, None


def window_mean(row, cwd):
    win = row['dry_window'] if TEST else row['window']
    c = D.load_csv(os.path.join(cwd, 'run.csv'))
    wall = c['wall_ns'][win[0]:win[1]] if c and 'wall_ns' in c else None
    expect_n = win[1] - win[0]
    if not wall or any(x is None for x in wall):
        return None, None, None, expect_n
    return sum(wall) / len(wall) / 1e6, statistics.median(wall) / 1e6, len(wall), expect_n


def validate(rec, row, w):
    why = []
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    if rec.get('mean_ms') is None:
        why.append('no window mean')
    elif rec.get('n_steps') is not None and rec.get('expect_steps') is not None and rec['n_steps'] != rec['expect_steps']:
        why.append(f'{rec["n_steps"]} steps in the window, expected {rec["expect_steps"]}')
    s = rec.get('summary')
    if not s:
        why.append('no SUMMARY')
    else:
        if s.get('void_steps') != 0:
            why.append(f'void_steps {s.get("void_steps")}')
        if s.get('workers') != w:
            why.append(f'workers {s.get("workers")} != {w}')
        if s.get('target_env') != 'msvc':
            why.append(f'target_env {s.get("target_env")}')
        if not TEST and s.get('pose_hash') != row['expected_pose']:
            why.append(f'pose {s.get("pose_hash")} != {row["expected_pose"]}')
        if not TEST and s.get('expect_pose') != 'match':
            why.append(f'expect_pose {s.get("expect_pose")}')
        if s.get('disarmed_ring_traffic'):
            why.append(f'disarmed_ring_traffic {s.get("disarmed_ring_traffic")}')
        if row['armed'] and s.get('drops_total'):
            why.append(f'drops_total {s.get("drops_total")}')
        if row['frozen_by'] and not TEST and (s.get('first_frozen_step') is None or s['first_frozen_step'] > row['frozen_by']):
            why.append(f'first_frozen_step {s.get("first_frozen_step")} not by {row["frozen_by"]}')
        if row['canary_of'] and not s.get('canary_ns'):
            why.append('canary_ns missing')
    return why


def run_one(m, rid, role, w, block, pass_no, rnd, seq, tag, pdir, ctx, receipt, env, sampler, deadline):
    row = ROWS[rid]
    entry = m['binaries'][role]
    suffix = '' if tag == 'original' else f'_{tag}'
    cwd = os.path.join(pdir, f'{seq:03d}_r{rnd}_{rid}_{role}_W{w}{suffix}')
    os.makedirs(cwd, exist_ok=True)
    rec = {'block': block, 'pass': pass_no, 'round': rnd, 'seq': seq, 'row': rid, 'engine': 'boyko', 'role': role,
           'binary': role, 'W': w, 'dry': TEST, 'attempt': tag, 'policy': 'P-none', 'timed': tag != 'warmup'}
    args, why = boyko_args(row, w, cwd, ctx, role)
    if row['canary_of']:
        ref = ctx.get((row['canary_of'], role, w))
        rec['canary_ref'] = {'pass': ref['pass'], 'round': ref['round'], 'seq': ref['seq'], 'block': ref['block'],
                             'window_mean_ns': ref['summary']['window_mean_ns']} if ref else None
    if args is None:
        rec['skipped'] = why
        return rec, receipt, 'ok'
    t0 = time.time()
    while busy(receipt) and not TEST:
        if time.time() - t0 >= QUIET_BUDGET_S:
            log(f'  quiet budget exhausted before {rid}#{role} W={w}; running the idle rule')
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
    rec.update({'exe': entry['exe'], 'exe_sha256': entry['sha256'], 'commit': entry['commit'], 'args': args, 'cwd': cwd,
                'receipt_before': receipt})
    rec['start'] = D.now()
    info, so, se = launch([entry['exe']] + args, cwd, env, sampler)
    rec.update(info)
    rec['end'] = D.now()
    with open(os.path.join(cwd, 'stdout.txt'), 'wb') as f:
        f.write(so)
    with open(os.path.join(cwd, 'stderr.txt'), 'wb') as f:
        f.write(se)
    receipt = D.load_receipt(RECEIPT_S)
    rec['receipt_after'] = receipt
    rec['contaminated_before'] = busy(rec['receipt_before'])
    rec['contaminated_after'] = busy(receipt)
    rec['contaminated_during'] = bool(rec.get('build_proc_during'))
    rec['contaminated'] = rec['contaminated_before'] or rec['contaminated_after'] or rec['contaminated_during']
    rec['summary'] = D.parse_summary(so)
    rec['csv'] = os.path.join(cwd, 'run.csv')
    rec['pose'] = os.path.join(cwd, 'pose.bin')
    mean_ms, median_ms, n, expect_n = window_mean(row, cwd)
    rec.update({'mean_ms': mean_ms, 'median_ms': median_ms, 'n_steps': n, 'expect_steps': expect_n})
    rec['invalid'] = validate(rec, row, w)
    rec['valid'] = not rec['invalid']
    prev = ctx.get((rid, role, w))
    if rec['valid'] and (not rec['contaminated'] or prev is None or prev.get('contaminated')):
        ctx[(rid, role, w)] = rec
    return rec, receipt, 'ok'


def tag_of(rec):
    if 'aborted' in rec:
        return f'ABORTED ({rec["aborted"]})'
    if 'skipped' in rec:
        return f'SKIPPED ({rec["skipped"]})'
    m = rec.get('mean_ms')
    t = f'mean {m:.4f} ms' if m is not None else 'mean NONE'
    extra = ''
    if rec.get('summary'):
        extra = f' pose {rec["summary"]["pose_hash"]} expect {rec["summary"].get("expect_pose")}'
        if rec['summary'].get('canary_ns'):
            extra += f' canary_ns {rec["summary"]["canary_ns"]}'
    perf = [p['total'] for p in rec.get('perf', []) if p.get('total') is not None]
    pt = f' perf {statistics.mean(perf):.1f}' if perf else ''
    flags = []
    if rec.get('contaminated_before'):
        flags.append(f'CONTAMINATED-before({rec["receipt_before"]["cpu_avg"]}%)')
    if rec.get('contaminated_after'):
        flags.append(f'CONTAMINATED-after({rec["receipt_after"]["cpu_avg"]}%)')
    if rec.get('contaminated_during'):
        flags.append(f'BUILD-PROC-DURING({rec["build_proc_during"]})')
    if rec.get('invalid'):
        flags.append(f'INVALID({"; ".join(rec["invalid"])})')
    return (f'{t}{extra} rb {rec["receipt_before"]["cpu_avg"]}% ra {rec["receipt_after"]["cpu_avg"]}% '
            f'others {rec.get("others_busy_pct")}% cpu_s {rec.get("proc_cpu_s")} wall {rec.get("wall_s")}s{pt} '
            f'waited {rec.get("waited_s")}s {" ".join(flags)}').rstrip()


def cell_order(block):
    """Pass-0 order of one round: W ascending; inside a W group row by row (rows.json order), parent then tip."""
    out = []
    ids = [rid for rid, r in ROWS.items() if r['block'] == block]
    for w in (1, 2, 4, 8, 16):
        for rid in ids:
            if w not in ROWS[rid]['ws']:
                continue
            for role in BINARIES:
                c = (rid, role, w)
                if TEST and c not in TEST_CELLS:
                    continue
                out.append(c)
    return out


def wait_idle(deadline):
    remaining_polls = max(3, int((deadline - time.time()) // 60))
    if TEST:
        return 0
    return subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', WAIT_PS1,
                           '-Log', WAIT_LOG, '-MaxPolls', str(remaining_polls)]).returncode


def machine_state():
    rc, so, se = D.run_bytes(['powercfg', '/getactivescheme'])
    rc2, so2, se2 = D.run_bytes(['powershell.exe', '-NoProfile', '-Command',
                                 'Get-CimInstance Win32_Battery | Select-Object BatteryStatus,EstimatedChargeRemaining '
                                 '| ConvertTo-Json -Compress; (Get-Process).Count'])
    return {'time': D.now(), 'powercfg': D.decode(so).strip(), 'battery_and_proc_count': D.decode(so2).strip()}


def verify_binaries(m):
    bad = []
    for role, e in m['binaries'].items():
        got = D.sha256(e['exe'])
        if got != e['sha256']:
            bad.append(f'{role} {e["exe"]} {got} != {e["sha256"]}')
    for line in open(os.path.join(BIN, 'SHA256SUMS'), encoding='utf-8'):
        sha, name = line.split()
        name = name.lstrip('*')
        path = name if os.path.isabs(name) else os.path.join(BIN, name)
        if D.sha256(path) != sha:
            bad.append(f'{path} != SHA256SUMS')
    return bad


def append(path, rec):
    with open(path, 'a', encoding='utf-8') as f:
        f.write(json.dumps(rec) + '\n')


def main():
    os.makedirs(RAW, exist_ok=True)
    m = {'generated': D.now(), 'rows_json': os.path.join(W4, 'rows.json'), 'toolchain': 'stable-x86_64-pc-windows-msvc',
         'profile': 'parity', 'logical_cpus': NCPU, 'binaries': MANIFEST_ENTRIES,
         'rows': {rid: {k: v for k, v in r.items() if k != 'per_w'} for rid, r in ROWS.items()},
         'protocol': PROTO, 'blocks': BLOCKS, 'gate_dir': GATE}
    with open(os.path.join(RAW, 'manifest.json'), 'w', encoding='utf-8') as f:
        json.dump(m, f, indent=1, default=str)
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    state = {'start': D.now(), 'test': TEST, 'placement': 'P-none (no affinity call)', 'blocks': [], 'machine_before': machine_state()}
    log(f'WINDOW START {json.dumps(state)}')
    deadline = time.time() + IDLE_BUDGET_S
    status = 'complete'
    runs_path = os.path.join(RAW, 'runs.jsonl')
    ctx = {}
    sampler = PerfSampler(1.0)
    n_timed = 0
    n_rerun = 0
    n_warm = 0
    for blk in BLOCKS:
        bname = blk['name']
        order0 = cell_order(bname)
        if not order0:
            continue
        state['blocks'].append({'name': bname, 'passes': blk['passes'], 'rounds': blk['rounds'], 'cells_per_round': len(order0),
                                'order_pass0': [f'{r}#{b}@W{w}' for r, b, w in order0]})
        log(f'block {bname}: {len(order0)} cells per round, {blk["passes"]} passes x {blk["rounds"]} rounds')
        for p in range(blk['passes']):
            ptag = f'{bname}-p{p}'
            log(f'{ptag}: idle rule (wait_idle4b.ps1)')
            rc = wait_idle(deadline)
            if rc != 0:
                status = f'STOP before {ptag}: idle rule not met (wait_idle exit {rc})'
                break
            log(f'{ptag}: idle reached; {machine_state()}')
            bad = verify_binaries(m)
            if bad:
                status = f'STOP before {ptag}: binaries changed {bad}'
                break
            log(f'{ptag}: binaries verified against the manifest and bin/SHA256SUMS')
            pdir = os.path.join(RAW, f'{ptag}')
            os.makedirs(pdir, exist_ok=True)
            order = order0 if p % 2 == 0 else order0[::-1]
            receipt = D.load_receipt(PASS_RECEIPT_S)
            log(f'{ptag}: opening receipt {receipt["cpu_avg"]}% (build procs {receipt["build_procs_busy"]})')
            wrec, receipt, st = run_one(m, 'J-As', 'tip', 8, bname, p, -1, 0, 'warmup', pdir, {}, receipt, env, sampler, deadline)
            append(runs_path, wrec)
            n_warm += 1
            log(f'  {ptag} warm-up J-As#tip W=8 (untimed): {tag_of(wrec) if st == "ok" else wrec.get("aborted")}')
            if st == 'abort':
                status = f'ABORT in {ptag}: {wrec.get("aborted")}'
                break
            seq = 1
            bad_runs = []
            for rnd in range(blk['rounds']):
                for rid, role, w in order:
                    rec, receipt, st = run_one(m, rid, role, w, bname, p, rnd, seq, 'original', pdir, ctx, receipt, env, sampler, deadline)
                    append(runs_path, rec)
                    if 'skipped' not in rec and 'aborted' not in rec:
                        n_timed += 1
                    log(f'  {ptag} r{rnd} [{seq:3d}] {rid}#{role} W={w}: {tag_of(rec)}')
                    if st == 'abort':
                        status = f'ABORT in {ptag}: {rec.get("aborted")}'
                        break
                    if 'skipped' in rec:
                        bad_runs.append((seq, rnd, rid, role, w, 'skipped'))
                    elif rec['contaminated'] or not rec['valid']:
                        bad_runs.append((seq, rnd, rid, role, w, 'contaminated' if rec['contaminated'] else 'invalid'))
                    seq += 1
                if status != 'complete':
                    break
            if status != 'complete':
                break
            if bad_runs:
                log(f'{ptag}: re-running {len(bad_runs)} process(es) at the end of the pass: {bad_runs}')
            for oseq, rnd, rid, role, w, why in bad_runs:
                rec, receipt, st = run_one(m, rid, role, w, bname, p, rnd, oseq, 'rerun', pdir, ctx, receipt, env, sampler, deadline)
                rec['rerun_reason'] = why
                append(runs_path, rec)
                if 'skipped' not in rec and 'aborted' not in rec:
                    n_timed += 1
                    n_rerun += 1
                log(f'  {ptag} r{rnd} [{oseq:3d}] {rid}#{role} W={w} RERUN ({why}): {tag_of(rec)}')
                if st == 'abort':
                    status = f'ABORT in {ptag} (re-runs): {rec.get("aborted")}'
                    break
            if status != 'complete':
                break
            log(f'{ptag}: done')
        if status != 'complete':
            break
    sampler.close()
    state['end'] = D.now()
    state['machine_after'] = machine_state()
    state['binaries_after'] = verify_binaries(m) or 'all match'
    state['status'] = status
    state['timed_processes'] = n_timed
    state['reruns'] = n_rerun
    state['warmups'] = n_warm
    log(f'WINDOW END {json.dumps({k: state[k] for k in ("end", "status", "binaries_after", "machine_after", "timed_processes", "reruns", "warmups")})}')
    with open(os.path.join(RAW, 'window_state.json'), 'w', encoding='utf-8') as f:
        json.dump(state, f, indent=1)
    with open(os.path.join(RAW, 'WINDOW_DONE'), 'w', encoding='utf-8') as f:
        f.write(status + '\n')


if __name__ == '__main__':
    main()
