"""P0b TIMED WINDOW (MEASUREMENT-QUEUE section 10 rows) under the orchestrator's ruled protocol.

Protocol (ruling, binding):
  * The statistic of a (row, W) cell is the MEDIAN over K = 6 separate processes of the process's window
    mean. Its resolution is the process-to-process spread of the same binary (min-max and IQR reported).
    A comparison is claimed only if its effect exceeds twice the combined spread (the reduction's job).
  * Load receipts gate each process: > 5 % busy (or a build process using CPU) before or after =>
    that process is re-run ONCE at the end of its pass. A re-run that is contaminated too is excluded.
  * No band-based whole-window void.
  * Placement: P-none (the P0b diagnostic's recommendation): no affinity call at all, for both engines.
    The launch path is the diagnostic's own (CreateProcess suspended -> read the mask back -> resume),
    with no SetProcessAffinityMask.

Order: two passes; each pass runs the full cell list three times (rounds), so every cell's processes are
interleaved with every other cell's. The cell list is grouped by W (ascending in pass 0) and, inside a W
group, Jolt and boyko cells alternate (Jolt first, Jolt cells spread evenly over the boyko cells). Pass 1
runs the whole list reversed. One untimed warm-up process (boyko J-A disarmed, W=8) opens each pass.

Receipts: a 10-s receipt opens each pass (after the idle rule); a 5-s receipt is taken between every two
processes and serves as the AFTER of one and the BEFORE of the next.

Witnesses (never gates; recorded per process): the process's own CPU seconds, the machine-wide busy
fraction of each logical CPU over the process lifetime, the CPU seconds every OTHER process used during
it (Toolhelp walk before launch and after exit), and each logical CPU's '% Processor Performance'
sampled at ~1 Hz by a parent-side thread (PDH).

Rows: the driver's own row table and argument builders, from a byte-identical copy of
D:/wt/joltab/tools/physics_parity/driver.py in lib/ (bytecode writing off, so nothing lands in the tree).
Nothing is compiled. Binaries are verified against the manifest before each pass.

P0B_WINDOW_TEST=1 is an UNTIMED rehearsal: dry step counts, a few cells, one round, no idle wait,
written under test/window. Its numbers are not measurements.
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
sys.path.insert(0, os.path.join(HERE, 'window_lib'))
import driver as D  # noqa: E402  (a byte-identical copy, checked in main)
from pdhperf import PerfSampler  # noqa: E402

P0 = r'(session scratch)/p0'
TREE_DRIVER = r'D:/wt/joltab/tools/physics_parity/driver.py'
WAIT_PS1 = os.path.join(P0, 'wait_idle.ps1')
TEST = os.environ.get('P0B_WINDOW_TEST') == '1'
RAW = os.path.join(HERE, 'test', 'window') if TEST else os.path.join(HERE, 'raw', 'window')
WAIT_LOG = os.path.join(RAW, 'wait_log.txt')
BUSY_PCT = 5.0
RECEIPT_S = 0.5 if TEST else 5.0
PASS_RECEIPT_S = 0.5 if TEST else 10.0
QUIET_BUDGET_S = 420.0
IDLE_BUDGET_S = 3 * 3600.0
PASSES = 1 if TEST else 2
ROUNDS = 1 if TEST else 3

# (row id, W) cells per W group, in the in-group order. 'L1-A1' is the driver's id of the pre-L1 runner on
# J-S0 disarmed (L1's A); 'L1-B' is the tip on the same flags (L1's B). J-A disarmed is J-A-d1.
BOYKO_CELLS = {
    1: ['J-A-d1', 'J-A-a', 'J-C', 'J-B', 'J-P1', 'J-S0', 'L1-A1', 'L1-B', 'J-Son', 'R', 'R-ref', 'R-S', 'S16',
        'AA2-pre', 'SHIP'],
    2: ['J-A-d1'],
    4: ['J-A-d1'],
    8: ['J-A-d1', 'J-A-a', 'J-C', 'J-B', 'J-Son', 'R', 'R-S', 'S16', 'AA2-pre', 'SHIP'],
    16: ['J-A-d1'],
}
JOLT_CELLS = {
    1: ['JOLT-T', 'JOLT56-T', 'JOLT-SLP', 'JOLT-P'],
    2: ['JOLT-T', 'JOLT56-T'],
    4: ['JOLT-T', 'JOLT56-T'],
    8: ['JOLT-T', 'JOLT56-T', 'JOLT-NPC', 'JOLT-SLP', 'JOLT-P'],
    16: ['JOLT-T', 'JOLT56-T'],
}
TEST_CELLS = {('J-A-d1', 1), ('J-A-a', 8), ('J-C', 8), ('J-B', 8), ('JOLT-T', 8), ('JOLT-P', 8), ('R', 8),
              ('L1-A1', 1), ('JOLT-SLP', 1)}
J_POSE = '0x32d5e235342b4143'
EXPECT_POSE = {'J-A-d1': J_POSE, 'J-A-a': J_POSE, 'J-C': J_POSE, 'J-B': J_POSE, 'J-P1': J_POSE, 'J-S0': J_POSE,
               'L1-A1': J_POSE, 'L1-B': J_POSE, 'AA2-pre': J_POSE, 'SHIP': J_POSE,
               'J-Son': '0xcc2a5400c66eecce', 'R': '0x87e561d20589d4a5', 'R-ref': '0x67463b7ea88686d2',
               'R-S': '0x2a2b7926a48aab00', 'S16': '0x8877dbb1192e9b92'}
EXPECT_JOLT_HASH = {'JOLT-T': '0xee15b89965ec747', 'JOLT-P': '0xee15b89965ec747', 'JOLT56-T': '0xb8522b4e3fc62cfe',
                    'JOLT-NPC': '0xf7fd640e276d8281', 'JOLT-SLP': '0x6b2368f4c588aeae'}

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
    """CPU seconds each OTHER process used between two Toolhelp tables (a process that started in between
    counts from zero; one that ended in between is invisible)."""
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
    """P-none launch, identical to the diagnostic's: CreateProcess suspended, read the (inherited) mask
    back, resume, wait. No SetProcessAffinityMask call is made."""
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
    keys = [f'0,{i}' for i in range(NCPU)]
    info['perf'] = [{'t': t, 'total': v.get('_Total'), 'cpu': [v.get(k) for k in keys]} for t, v in perf]
    return info, so, se


def window_mean(rec, row, cwd, stdout):
    """(mean_ms, median_ms, n) of the per-step wall time over the row's window: the statistic the window
    quotes. boyko: run.csv wall_ns over the row window; Jolt: per_frame 'Time (ms)' over all frames;
    JOLT-P (no per-frame file): 1000 / its printed steps per second."""
    if row['engine'] == 'boyko':
        win = row['dry_window'] if TEST else row['window']
        c = D.load_csv(os.path.join(cwd, 'run.csv'))
        wall = c['wall_ns'][win[0]:win[1]] if c and 'wall_ns' in c else None
        expect_n = win[1] - win[0]
    else:
        pf = [f for f in os.listdir(cwd) if f.startswith('per_frame_')]
        if pf:
            c = D.load_csv(os.path.join(cwd, pf[0]))
            wall = [x * 1e6 for x in c['Time (ms)']] if c and 'Time (ms)' in c else None
            expect_n = row['dry_steps'] if TEST else row['steps']
        else:
            st = rec.get('jolt', {}).get('stat_lines') or []
            if st and st[0]['steps_per_s']:
                return 1000.0 / st[0]['steps_per_s'], None, None, None
            return None, None, None, None
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
    if row['engine'] == 'boyko':
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
            if not TEST and s.get('pose_hash') != EXPECT_POSE.get(row['id']):
                why.append(f'pose {s.get("pose_hash")} != {EXPECT_POSE.get(row["id"])}')
            if row['id'] == 'J-B' and s.get('expect_pose') not in ('match', 'none'):
                why.append(f'H7 expect_pose {s.get("expect_pose")}')
    else:
        st = rec.get('jolt', {}).get('stat_lines') or []
        if not st:
            why.append('no Jolt stat line')
        else:
            if st[0]['threads'] != w:
                why.append(f'threads {st[0]["threads"]} != {w}')
            if not TEST and st[0]['hash'] != EXPECT_JOLT_HASH.get(row['id']):
                why.append(f'hash {st[0]["hash"]} != {EXPECT_JOLT_HASH.get(row["id"])}')
        if row['id'] == 'JOLT-P':
            n_html = len([f for f in rec.get('files', []) if f.endswith('.html')])
            steps = row['dry_steps'] if TEST else row['steps']
            if n_html != (steps + 99) // 100:
                why.append(f'{n_html} profile charts')
    return why


def run_one(m, rid, w, pass_no, rnd, seq, tag, pdir, ctx, receipt, env, sampler, deadline):
    row = D.ROWS[rid]
    entry = (m['boyko'] if row['engine'] == 'boyko' else m['jolt'])[row['role']]
    suffix = '' if tag == 'original' else f'_{tag}'
    cwd = os.path.join(pdir, f'{seq:03d}_r{rnd}_{rid}_W{w}{suffix}')
    os.makedirs(cwd, exist_ok=True)
    rec = {'pass': pass_no, 'round': rnd, 'g': pass_no * ROUNDS + rnd, 'seq': seq, 'row': rid, 'engine': row['engine'],
           'role': row['role'], 'W': w, 'dry': TEST, 'attempt': tag, 'policy': 'P-none'}
    if row['engine'] == 'boyko':
        args, why = D.boyko_args(row, w, TEST, cwd, ctx)
        if row['canary_of']:
            ref = ctx.get((row['canary_of'], w))
            rec['canary_ref'] = {'pass': ref['pass'], 'round': ref['round'], 'seq': ref['seq'],
                                 'window_mean_ns': ref['summary']['window_mean_ns']} if ref else None
        if row['expect_pose_of']:
            ref = ctx.get((row['expect_pose_of'], w))
            rec['expect_pose_ref'] = {'pass': ref['pass'], 'round': ref['round'], 'seq': ref['seq']} if ref else None
    else:
        args, why = D.jolt_args(row, w, TEST)
    if args is None:
        rec['skipped'] = why
        return rec, receipt, 'ok'
    # The quiet rule before a timed region: wait up to the quiet budget, then fall back to the idle rule.
    t0 = time.time()
    while busy(receipt) and not TEST:
        if time.time() - t0 >= QUIET_BUDGET_S:
            log(f'  quiet budget exhausted before {rid} W={w}; running the idle rule')
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
    rec.update({'exe': entry['exe'], 'exe_sha256': entry['sha256'], 'args': args, 'cwd': cwd, 'receipt_before': receipt})
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
    rec['contaminated'] = rec['contaminated_before'] or rec['contaminated_after']
    if row['engine'] == 'boyko':
        rec['summary'] = D.parse_summary(so)
        rec['csv'] = os.path.join(cwd, 'run.csv')
        rec['pose'] = os.path.join(cwd, 'pose.bin')
    else:
        rec['jolt'] = D.parse_jolt_stdout(so)
        files = sorted(os.listdir(cwd))
        rec['files'] = files
        pf = [f for f in files if f.startswith('per_frame_')]
        rec['per_frame'] = os.path.join(cwd, pf[0]) if pf else None
        rec['receipt_csv'] = None
    mean_ms, median_ms, n, expect_n = window_mean(rec, row, cwd, so)
    rec.update({'mean_ms': mean_ms, 'median_ms': median_ms, 'n_steps': n, 'expect_steps': expect_n})
    rec['invalid'] = validate(rec, row, w)
    rec['valid'] = not rec['invalid']
    # The canary reference and H7's pose come from the most recent VALID run of their row at this W,
    # preferring an uncontaminated one.
    prev = ctx.get((rid, w))
    if rec['valid'] and (not rec['contaminated'] or prev is None or prev.get('contaminated')):
        ctx[(rid, w)] = rec
    return rec, receipt, 'ok'


def tag_of(rec):
    if 'aborted' in rec:
        return f'ABORTED ({rec["aborted"]})'
    if 'skipped' in rec:
        return f'SKIPPED ({rec["skipped"]})'
    m = rec.get('mean_ms')
    t = f'mean {m:.4f} ms' if m is not None else 'mean NONE'
    extra = ''
    if rec['engine'] == 'boyko' and rec.get('summary'):
        extra = f' pose {rec["summary"]["pose_hash"]}'
        if rec['summary'].get('canary_ns'):
            extra += f' canary_ns {rec["summary"]["canary_ns"]}'
    elif rec.get('jolt', {}).get('stat_lines'):
        st = rec['jolt']['stat_lines'][0]
        extra = f' hash {st["hash"]} threads {st["threads"]}'
    perf = [p['total'] for p in rec.get('perf', []) if p.get('total') is not None]
    pt = f' perf {statistics.mean(perf):.1f}' if perf else ''
    flags = []
    if rec.get('contaminated_before'):
        flags.append(f'CONTAMINATED-before({rec["receipt_before"]["cpu_avg"]}%)')
    if rec.get('contaminated_after'):
        flags.append(f'CONTAMINATED-after({rec["receipt_after"]["cpu_avg"]}%)')
    if rec.get('invalid'):
        flags.append(f'INVALID({"; ".join(rec["invalid"])})')
    return (f'{t}{extra} rb {rec["receipt_before"]["cpu_avg"]}% ra {rec["receipt_after"]["cpu_avg"]}% '
            f'others {rec.get("others_busy_pct")}% cpu_s {rec.get("proc_cpu_s")} wall {rec.get("wall_s")}s{pt} '
            f'waited {rec.get("waited_s")}s {" ".join(flags)}').rstrip()


def cell_order():
    """Pass-0 order of one round: W ascending; inside a W group Jolt first, Jolt cells spread evenly."""
    out = []
    for w in (1, 2, 4, 8, 16):
        b = [(r, w) for r in BOYKO_CELLS[w]]
        j = [(r, w) for r in JOLT_CELLS[w]]
        if TEST:
            b = [c for c in b if c in TEST_CELLS]
            j = [c for c in j if c in TEST_CELLS]
        if not b and not j:
            continue
        seq = []
        if len(j) >= len(b):
            for i in range(max(len(b), len(j))):
                if i < len(j):
                    seq.append(j[i])
                if i < len(b):
                    seq.append(b[i])
        else:
            before = {}
            for i in range(len(j)):
                before.setdefault((i * len(b)) // len(j), []).append(j[i])
            for i, c in enumerate(b):
                seq.extend(before.get(i, []))
                seq.append(c)
        out.extend(seq)
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
    for role, e in list(m['boyko'].items()) + list(m['jolt'].items()):
        got = D.sha256(e['exe'])
        if got != e['sha256']:
            bad.append(f'{role} {e["exe"]} {got} != {e["sha256"]}')
    bb = os.path.join(P0, 'bin', 'broadphase_bench.exe')
    for line in open(os.path.join(P0, 'bin', 'SHA256SUMS'), encoding='utf-8'):
        sha, name = line.split()
        path = os.path.join(P0, 'bin', name.lstrip('*'))
        if D.sha256(path) != sha:
            bad.append(f'{path} != SHA256SUMS')
    return bad


def append(path, rec):
    with open(path, 'a', encoding='utf-8') as f:
        f.write(json.dumps(rec) + '\n')


def run_broadphase(env, deadline):
    """The broadphase criterion bench, once, with its own receipts (and the same witnesses)."""
    bdir = os.path.join(RAW, 'broadphase')
    os.makedirs(bdir, exist_ok=True)
    exe = os.path.join(P0, 'bin', 'broadphase_bench.exe')
    rec = {'what': 'broadphase_bench --bench --noplot', 'exe': exe, 'exe_sha256': D.sha256(exe)}
    log('broadphase: idle rule')
    rc = wait_idle(deadline)
    if rc != 0:
        rec['aborted'] = f'idle rule not met (wait_idle exit {rc})'
        return rec
    rec['receipt_before'] = D.load_receipt(PASS_RECEIPT_S)
    benv = dict(env)
    benv['CRITERION_HOME'] = os.path.join(bdir, 'criterion')  # keeps criterion off `cargo metadata` and the tree
    args = ['--bench', '--noplot'] + (['broadphase/all_pairs/100', '--warm-up-time', '0.2', '--measurement-time', '0.5']
                                      if TEST else [])
    rec['args'] = args
    rec['start'] = D.now()
    sampler = PerfSampler(1.0)
    info, so, se = launch([exe] + args, bdir, benv, sampler)
    sampler.close()
    rec.update(info)
    rec['end'] = D.now()
    with open(os.path.join(bdir, 'stdout.txt'), 'wb') as f:
        f.write(so)
    with open(os.path.join(bdir, 'stderr.txt'), 'wb') as f:
        f.write(se)
    rec['receipt_after'] = D.load_receipt(RECEIPT_S)
    rec['contaminated'] = busy(rec['receipt_before']) or busy(rec['receipt_after'])
    return rec


def main():
    os.makedirs(RAW, exist_ok=True)
    if D.sha256(os.path.join(HERE, 'lib', 'driver.py')) != D.sha256(TREE_DRIVER):
        sys.exit('lib/driver.py differs from the tree driver')
    m = json.load(open(os.path.join(P0, 'manifest.json'), encoding='utf-8'))
    with open(os.path.join(RAW, 'manifest.json'), 'w', encoding='utf-8') as f:
        json.dump(m, f, indent=1)
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    order0 = cell_order()
    state = {'start': D.now(), 'test': TEST, 'placement': 'P-none (no affinity call)', 'cells_per_round': len(order0),
             'order_pass0': [f'{r}@W{w}' for r, w in order0], 'machine_before': machine_state()}
    log(f'WINDOW START {json.dumps(state)}')
    deadline = time.time() + IDLE_BUDGET_S
    status = 'complete'
    runs_path = os.path.join(RAW, 'runs.jsonl')
    ctx = {}
    sampler = PerfSampler(1.0)
    for p in range(PASSES):
        log(f'pass {p}: idle rule (wait_idle.ps1)')
        rc = wait_idle(deadline)
        if rc != 0:
            status = f'STOP before pass {p}: idle rule not met (wait_idle exit {rc})'
            break
        log(f'pass {p}: idle reached; {machine_state()}')
        bad = verify_binaries(m)
        if bad:
            status = f'STOP before pass {p}: binaries changed {bad}'
            break
        pdir = os.path.join(RAW, f'pass-{p:02d}')
        os.makedirs(pdir, exist_ok=True)
        order = order0 if p % 2 == 0 else order0[::-1]
        receipt = D.load_receipt(PASS_RECEIPT_S)
        log(f'pass {p}: opening receipt {receipt["cpu_avg"]}% (build procs {receipt["build_procs_busy"]})')
        # Untimed warm-up: no cell inherits the post-idle boost state.
        wrec, receipt, st = run_one(m, 'J-A-d1', 8, p, -1, 0, 'warmup', pdir, {}, receipt, env, sampler, deadline)
        append(runs_path, wrec)
        log(f'  p{p} warm-up J-A-d1 W=8: {tag_of(wrec) if st == "ok" else wrec.get("aborted")}')
        if st == 'abort':
            status = f'ABORT in pass {p}: {wrec.get("aborted")}'
            break
        seq = 1
        bad_runs = []
        for rnd in range(ROUNDS):
            for rid, w in order:
                rec, receipt, st = run_one(m, rid, w, p, rnd, seq, 'original', pdir, ctx, receipt, env, sampler, deadline)
                append(runs_path, rec)
                log(f'  p{p} r{rnd} [{seq:3d}] {rid} W={w}: {tag_of(rec)}')
                if st == 'abort':
                    status = f'ABORT in pass {p}: {rec.get("aborted")}'
                    break
                if 'skipped' in rec:
                    bad_runs.append((seq, rnd, rid, w, 'skipped'))
                elif rec['contaminated'] or not rec['valid']:
                    bad_runs.append((seq, rnd, rid, w, 'contaminated' if rec['contaminated'] else 'invalid'))
                seq += 1
            if status != 'complete':
                break
        if status != 'complete':
            break
        if bad_runs:
            log(f'pass {p}: re-running {len(bad_runs)} process(es) at the end of the pass: {bad_runs}')
        for oseq, rnd, rid, w, why in bad_runs:
            rec, receipt, st = run_one(m, rid, w, p, rnd, oseq, 'rerun', pdir, ctx, receipt, env, sampler, deadline)
            rec['rerun_reason'] = why
            append(runs_path, rec)
            log(f'  p{p} r{rnd} [{oseq:3d}] {rid} W={w} RERUN ({why}): {tag_of(rec)}')
            if st == 'abort':
                status = f'ABORT in pass {p} (re-runs): {rec.get("aborted")}'
                break
        if status != 'complete':
            break
        log(f'pass {p}: done')
    sampler.close()
    if status == 'complete':
        brec = run_broadphase(env, deadline)
        with open(os.path.join(RAW, 'broadphase', 'broadphase_run.json'), 'w', encoding='utf-8') as f:
            json.dump(brec, f, indent=1)
        log(f'broadphase: exit {brec.get("exit")} wall {brec.get("wall_s")} s rb '
            f'{(brec.get("receipt_before") or {}).get("cpu_avg")}% ra {(brec.get("receipt_after") or {}).get("cpu_avg")}% '
            f'others {brec.get("others_busy_pct")}% contaminated {brec.get("contaminated")} {brec.get("aborted", "")}')
    state['end'] = D.now()
    state['machine_after'] = machine_state()
    state['binaries_after'] = verify_binaries(m) or 'all match'
    state['status'] = status
    log(f'WINDOW END {json.dumps({k: state[k] for k in ("end", "status", "binaries_after", "machine_after")})}')
    with open(os.path.join(RAW, 'window_state.json'), 'w', encoding='utf-8') as f:
        json.dump(state, f, indent=1)
    with open(os.path.join(RAW, 'WINDOW_DONE'), 'w', encoding='utf-8') as f:
        f.write(status + '\n')


if __name__ == '__main__':
    main()
