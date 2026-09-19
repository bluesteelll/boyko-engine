"""P0b DIAGNOSTIC (not a measurement window): does CPU placement explain the per-process spread?

Three placement policies, both engines, W in {2, 8}, 6 processes per (policy, engine, W) cell:
  P-none  no affinity call at all (today's launch);
  P-phys  process affinity = one logical CPU per physical core, first W cores
          (SMT map from GetLogicalProcessorInformationEx, see topology.json: core k = CPUs {2k, 2k+1},
          so W=2 -> CPUs {0,2} = 0x5, W=8 -> CPUs {0,2,..,14} = 0x5555);
  P-full  process affinity = all 16 logical CPUs, set explicitly (0xFFFF): control, must equal P-none.

Launch path (identical for all three policies): CreateProcess SUSPENDED (subprocess.Popen with
CREATE_SUSPENDED), then SetProcessAffinityMask (P-phys / P-full only), GetProcessAffinityMask read
back, then NtResumeProcess. The child executes no instruction before its mask is set, so every thread
it ever creates (boyko's pool, Jolt's job threads) inherits the mask. Neither binary sets thread
affinity itself (boyko_threadpool's `affinity` is a no-op stub; Jolt's JobSystemThreadPool has no
affinity call).

Rows reuse the driver's own builders (a byte-identical copy of D:/wt/joltab/tools/physics_parity/driver.py
at HEAD, in lib/, so nothing is written into the tree): boyko = J-A-d1 (disarmed, --scene jolt --gap 0.5
--cfg a, 500 steps, window 0..500), Jolt = JOLT-T (v5.3.0 Distribution, -s=Pyramid -q=Discrete -f -t=W
-i=500). Statistic per process = the window mean of the per-step wall time (boyko run.csv wall_ns over
[0,500); Jolt per_frame 'Time (ms)' over its 500 frames), exactly the quantity the window quotes.

Gates: idle rule (p0/wait_idle.ps1: 3 consecutive 60 s polls, 0 build processes, 10-s CPU < 5 %, process
count in the hundreds) before each of the 6 passes, within an overall 3 h budget; the driver's 2-s load
receipt before and after each process; a busy receipt (> 5 % or a build process using CPU) before a
process -> wait for quiet (420 s budget, else abort); after -> re-run that process once, immediately.
One untimed warm-up process (boyko W=8, P-none) opens every pass so that no cell inherits the
post-idle boost state. Witness per process: per-logical-CPU busy fraction over the process lifetime
(NtQuerySystemInformation SystemProcessorPerformanceInformation, machine-wide, idle machine) and the
process's own CPU seconds.
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
sys.path.insert(0, os.path.join(HERE, 'lib'))
import driver as D  # noqa: E402  (a copy; see the docstring)

P0 = r'(session scratch)/p0'
TREE_DRIVER = r'D:/wt/joltab/tools/physics_parity/driver.py'
WAIT_PS1 = os.path.join(P0, 'wait_idle.ps1')
TEST = os.environ.get('P0B_TEST') == '1'  # rehearsal: 1 pass, 12 steps, no idle wait; not a measurement
RAW = os.path.join(HERE, 'test' if TEST else 'raw')
WAIT_LOG = os.path.join(HERE, 'wait_log.txt')
BUSY_PCT = 5.0
RECEIPT_S = 2.0
QUIET_BUDGET_S = 420.0
IDLE_BUDGET_S = 3 * 3600.0
PASSES = 6
WS = (2, 8)
POLICIES = ('P-none', 'P-phys', 'P-full')
MASKS = {'P-none': {2: None, 8: None}, 'P-phys': {2: 0x5, 8: 0x5555}, 'P-full': {2: 0xFFFF, 8: 0xFFFF}}
EXPECT_BOYKO_POSE = '0x32d5e235342b4143'
EXPECT_JOLT_HASH = '0xee15b89965ec747'

CREATE_SUSPENDED = 0x4
k32 = ctypes.WinDLL('kernel32', use_last_error=True)
k32.SetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.c_size_t]
k32.SetProcessAffinityMask.restype = wintypes.BOOL
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


class SPPI(ctypes.Structure):  # SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION
    _fields_ = [('IdleTime', ctypes.c_longlong), ('KernelTime', ctypes.c_longlong), ('UserTime', ctypes.c_longlong),
                ('DpcTime', ctypes.c_longlong), ('InterruptTime', ctypes.c_longlong), ('InterruptCount', ctypes.c_ulong)]


NCPU = os.cpu_count()


def percpu():
    arr = (SPPI * NCPU)()
    ret = wintypes.ULONG(0)
    st = ntdll.NtQuerySystemInformation(8, ctypes.byref(arr), ctypes.sizeof(arr), ctypes.byref(ret))
    if st != 0:
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


def log(msg):
    line = f'{D.now()} {msg}'
    print(line, flush=True)
    with open(os.path.join(RAW, 'diag_log.txt'), 'a', encoding='utf-8') as f:
        f.write(line + '\n')


def launch(cmd, cwd, env, mask):
    """CreateProcess suspended -> affinity (if any) -> read back -> resume -> wait. Returns a dict."""
    p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         creationflags=CREATE_SUSPENDED)
    h = int(p._handle)
    info = {'pid': p.pid, 'mask_requested': hex(mask) if mask is not None else None}
    if mask is not None:
        if not k32.SetProcessAffinityMask(h, mask):
            err = ctypes.get_last_error()
            k32.TerminateProcess(h, 99)
            p.communicate()
            raise OSError(f'SetProcessAffinityMask({hex(mask)}) failed: {err}')
    pm, sm = ctypes.c_size_t(), ctypes.c_size_t()
    k32.GetProcessAffinityMask(h, ctypes.byref(pm), ctypes.byref(sm))
    info['mask_readback'] = hex(pm.value)
    info['system_mask'] = hex(sm.value)
    if mask is not None and pm.value != mask:
        k32.TerminateProcess(h, 98)
        p.communicate()
        raise OSError(f'affinity read back {hex(pm.value)} != requested {hex(mask)}')
    c0 = percpu()
    t0 = time.perf_counter()
    st = ntdll.NtResumeProcess(h)
    if st != 0:
        k32.TerminateProcess(h, 97)
        p.communicate()
        raise OSError(f'NtResumeProcess failed: {st:#x}')
    so, se = p.communicate()
    info['wall_s'] = round(time.perf_counter() - t0, 3)
    c1 = percpu()
    info['exit'] = p.returncode
    info['percpu_busy'] = percpu_busy(c0, c1)
    c, x, kk, u = (wintypes.FILETIME() for _ in range(4))
    if k32.GetProcessTimes(h, ctypes.byref(c), ctypes.byref(x), ctypes.byref(kk), ctypes.byref(u)):
        info['proc_cpu_s'] = round(ft(kk) + ft(u), 3)
    return info, so, se


def busy(receipt):
    return (receipt['cpu_avg'] or 0) > BUSY_PCT or bool(receipt['build_procs_busy'])


def run_one(exe, sha, engine, w, policy, pdir, seq, tag, env, receipt):
    cwd = os.path.join(pdir, f'{seq:03d}_{engine}_W{w}_{policy}{"" if tag == "original" else "_" + tag}')
    os.makedirs(cwd, exist_ok=True)
    if engine == 'boyko':
        args, why = D.boyko_args(D.ROWS['J-A-d1'], w, TEST, cwd, {})
    else:
        args, why = D.jolt_args(D.ROWS['JOLT-T'], w, TEST)
    rec = {'seq': seq, 'engine': engine, 'W': w, 'policy': policy, 'attempt': tag, 'exe': exe, 'exe_sha256': sha,
           'args': args, 'cwd': cwd}
    t0 = time.time()
    while busy(receipt) and time.time() - t0 < QUIET_BUDGET_S and not TEST:
        time.sleep(20)
        receipt = D.load_receipt(RECEIPT_S)
    rec['waited_s'] = round(time.time() - t0, 1)
    rec['receipt_before'] = receipt
    if busy(receipt) and not TEST:
        rec['aborted'] = 'machine busy after the quiet budget'
        return rec, receipt, True
    rec['start'] = D.now()
    info, so, se = launch([exe] + args, cwd, env, MASKS[policy][w])
    rec.update(info)
    rec['end'] = D.now()
    for name, data in (('stdout.txt', so), ('stderr.txt', se)):
        with open(os.path.join(cwd, name), 'wb') as f:
            f.write(data)
    receipt = D.load_receipt(RECEIPT_S)
    rec['receipt_after'] = receipt
    rec['contaminated'] = busy(rec['receipt_before']) or busy(receipt)
    steps = D.ROWS['J-A-d1']['dry_steps'] if TEST else 500
    if engine == 'boyko':
        s = D.parse_summary(so)
        rec['summary'] = {k: s.get(k) for k in ('window_mean_ns', 'pose_hash', 'void_steps', 'workers', 'profile_name',
                                                  'armed', 'disarmed_ring_traffic')} if s else None
        c = D.load_csv(os.path.join(cwd, 'run.csv'))
        wall = c['wall_ns'][0:steps] if c else None
    else:
        j = D.parse_jolt_stdout(so)
        rec['jolt'] = j['stat_lines'][0] if j['stat_lines'] else None
        pf = [f for f in os.listdir(cwd) if f.startswith('per_frame_')]
        c = D.load_csv(os.path.join(cwd, pf[0])) if pf else None
        wall = [x * 1e6 for x in c['Time (ms)']] if c else None
    if wall:
        rec['n_steps'] = len(wall)
        rec['mean_ms'] = sum(wall) / len(wall) / 1e6
        rec['median_ms'] = statistics.median(wall) / 1e6
        rec['mean_skip20_ms'] = sum(wall[20:]) / len(wall[20:]) / 1e6 if len(wall) > 20 else None
    ok = rec['exit'] == 0 and wall is not None and rec.get('n_steps') == steps
    if engine == 'boyko':
        ok = ok and rec['summary'] is not None and rec['summary']['pose_hash'] == EXPECT_BOYKO_POSE and \
            rec['summary']['void_steps'] == 0 and rec['summary']['workers'] == w
    else:
        ok = ok and rec['jolt'] is not None and rec['jolt']['hash'] == EXPECT_JOLT_HASH and rec['jolt']['threads'] == w
    rec['valid'] = bool(ok) if not TEST else rec['exit'] == 0
    return rec, receipt, False


def order(r):
    ws = list(WS) if r % 2 == 0 else list(WS)[::-1]
    out = []
    for wi, w in enumerate(ws):
        for i in range(3):
            pol = POLICIES[(i + r) % 3]
            sides = ('boyko', 'jolt') if (i + r + wi) % 2 == 0 else ('jolt', 'boyko')
            out.extend((w, pol, s) for s in sides)
    return out


def wait_idle(deadline):
    remaining_polls = max(3, int((deadline - time.time()) // 60))
    rc = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', WAIT_PS1,
                         '-Log', WAIT_LOG, '-MaxPolls', str(remaining_polls)]).returncode
    return rc


def machine_state():
    rc, so, se = D.run_bytes(['powercfg', '/getactivescheme'])
    rc2, so2, se2 = D.run_bytes(['powershell.exe', '-NoProfile', '-Command',
                                 'Get-CimInstance Win32_Battery | Select-Object BatteryStatus,EstimatedChargeRemaining '
                                 '| ConvertTo-Json -Compress; (Get-Process).Count'])
    return {'powercfg': D.decode(so).strip(), 'battery_and_proc_count': D.decode(so2).strip()}


def main():
    os.makedirs(RAW, exist_ok=True)
    if D.sha256(os.path.join(HERE, 'lib', 'driver.py')) != D.sha256(TREE_DRIVER):
        sys.exit('lib/driver.py differs from the tree driver')
    m = json.load(open(os.path.join(P0, 'manifest.json'), encoding='utf-8'))
    bexe, bsha = m['boyko']['tip']['exe'], m['boyko']['tip']['sha256']
    jexe, jsha = m['jolt']['v530']['exe'], m['jolt']['v530']['sha256']
    exes = {'boyko': (bexe, bsha), 'jolt': (jexe, jsha)}
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    state = {'start': D.now(), 'machine_before': machine_state(), 'test': TEST}
    log(f'DIAG START {json.dumps(state)}')
    deadline = time.time() + IDLE_BUDGET_S
    status = 'complete'
    runs_path = os.path.join(RAW, 'runs.jsonl')
    passes = 1 if TEST else PASSES
    for r in range(passes):
        if not TEST:
            log(f'pass {r}: idle rule (wait_idle.ps1)')
            rc = wait_idle(deadline)
            if rc != 0:
                status = f'STOP before pass {r}: idle rule not met (wait_idle exit {rc})'
                break
            log(f'pass {r}: idle reached')
        changed = [exe for exe, sha in exes.values() if D.sha256(exe) != sha]
        if changed:
            status = f'STOP before pass {r}: {changed} changed'
            break
        pdir = os.path.join(RAW, f'pass-{r:02d}')
        os.makedirs(pdir, exist_ok=True)
        receipt = D.load_receipt(RECEIPT_S)
        wrec, receipt, abort = run_one(bexe, bsha, 'boyko', 8, 'P-none', pdir, 0, 'warmup', env, receipt)
        wrec['pass'] = r
        with open(runs_path, 'a', encoding='utf-8') as f:
            f.write(json.dumps(wrec) + '\n')
        log(f'  p{r} warm-up boyko W=8: mean {wrec.get("mean_ms")} ms')
        if abort:
            status = f'ABORT in pass {r}: busy'
            break
        for seq, (w, pol, eng) in enumerate(order(r), start=1):
            exe, sha = exes[eng]
            rec, receipt, abort = run_one(exe, sha, eng, w, pol, pdir, seq, 'original', env, receipt)
            rec['pass'] = r
            with open(runs_path, 'a', encoding='utf-8') as f:
                f.write(json.dumps(rec) + '\n')
            if abort:
                status = f'ABORT in pass {r}: busy'
                break
            log(f'  p{r} [{seq:2d}] {eng:5s} W={w} {pol}: mean {rec.get("mean_ms", float("nan")):.4f} ms '
                f'median {rec.get("median_ms", float("nan")):.4f} valid {rec["valid"]} mask {rec["mask_readback"]} '
                f'rb {rec["receipt_before"]["cpu_avg"]}% ra {rec["receipt_after"]["cpu_avg"]}% '
                f'cpu_s {rec.get("proc_cpu_s")} wall {rec["wall_s"]}s{" CONTAMINATED" if rec["contaminated"] else ""}')
            if rec['contaminated'] or not rec['valid']:
                why = 'contaminated' if rec['contaminated'] else 'invalid'
                rec2, receipt, abort = run_one(exe, sha, eng, w, pol, pdir, seq, 'rerun', env, receipt)
                rec2['pass'] = r
                rec2['rerun_reason'] = why
                with open(runs_path, 'a', encoding='utf-8') as f:
                    f.write(json.dumps(rec2) + '\n')
                if abort:
                    status = f'ABORT in pass {r}: busy'
                    break
                log(f'  p{r} [{seq:2d}] RERUN ({why}): mean {rec2.get("mean_ms", float("nan")):.4f} ms valid '
                    f'{rec2["valid"]} rb {rec2["receipt_before"]["cpu_avg"]}% ra {rec2["receipt_after"]["cpu_avg"]}%'
                    f'{" CONTAMINATED" if rec2.get("contaminated") else ""}')
        if status != 'complete':
            break
    state['end'] = D.now()
    state['machine_after'] = machine_state()
    state['status'] = status
    log(f'DIAG END {json.dumps(state)}')
    with open(os.path.join(RAW, 'diag_state.json'), 'w', encoding='utf-8') as f:
        json.dump(state, f, indent=1)
    with open(os.path.join(RAW, 'DIAG_DONE'), 'w', encoding='utf-8') as f:
        f.write(status + '\n')


if __name__ == '__main__':
    main()
