"""WINDOW 5 (2026-09-23): (1) the HEADLINE on the tip (cbd86a65) - J --cfg default tree/allpairs at W 1/2/4/8
and rest tree/allpairs at W 1/8; (2) G9-ON-C3 - parent 0ca312bd vs tip cbd86a65 on J-As at W 1/8, then the
armed twins J-As-a at W 1/8. Adapted from window 4b's window4b_run.py (itself window 3's window3_run.py):
same P-none launch, receipts, witnesses, record layout. Differences:
  * rows carry their own binary list (headline rows: tip only; G9 rows: parent then tip);
  * three blocks, each 2 passes x 3 rounds (K = 6): 'headline', 'g9', 'armed';
  * order per block, pass 0: W ascending, inside a W group row by row in rows.json order (tree then allpairs;
    parent then tip); pass 1 the whole list reversed; one untimed warm-up per pass;
  * every process gets --expect-pose gate/<pose_ref>_ref.pose; its summary pose_hash is asserted against
    rows.json, and its TreeDiag against the row's arm (tree: static_rebuilds 1, members 1, evictions 0;
    allpairs: every field 0);
  * VOID rule (task): a build process (cargo/rustc/link/miri/...) or any process running from
    D:/wt/_targets or D:/wt/mq-* seen at ANY point of a timed pass (presence snapshot at every receipt, plus the
    during-process CPU table) VOIDS the pass: it is abandoned at once, the idle rule is re-run, and the pass
    is re-run from its start in a new directory <block>-p<n>-v<k>; its records stay in runs.jsonl marked
    'voided_pass' and are excluded from every cell;
  * the idle rule before every pass: tools/wait_idle5.ps1, up to 120 polls (120 min) per wait; a timeout STOPs.
Protocol otherwise as window 3 / 4b: a cell = MEDIAN over K processes of the process window mean; 5-s receipt
between processes; 10-s receipt opening each pass; a process whose before/after receipt is > 5 % busy (or
invalid) is re-run ONCE at the end of its pass.

WIN5_TEST=1 is an UNTIMED rehearsal (12 steps, one round, one pass, no idle wait) under test/.
"""
import ctypes
import json
import os
import statistics
import subprocess
import sys
import time
from ctypes import wintypes

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W5 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, os.path.join(HERE, 'window_lib'))
import driver as D  # noqa: E402  (window 3's byte-identical copy of the tree driver; only helpers are used)
from pdhperf import PerfSampler  # noqa: E402

WAIT_PS1 = os.path.join(HERE, 'wait_idle5.ps1')
TEST = os.environ.get('WIN5_TEST') == '1'
RAW = os.path.join(W5, 'test', 'window') if TEST else os.path.join(W5, 'raw')
WAIT_LOG = os.path.join(RAW, 'wait_log.txt')
BIN = os.path.join(W5, 'bin')
GATE = os.path.join(W5, 'gate')
BUSY_PCT = 5.0
RECEIPT_S = 0.5 if TEST else 5.0
PASS_RECEIPT_S = 0.5 if TEST else 10.0
QUIET_BUDGET_S = 420.0
IDLE_MAX_POLLS = 120
MAX_VOIDS_PER_PASS = 20
VOID_CHECK = not TEST or os.environ.get('WIN5_TEST_VOID') == '1'
ONLY_BLOCKS = [b for b in os.environ.get('WIN5_BLOCKS', '').split(',') if b]

ROWS_JSON = json.load(open(os.path.join(W5, 'rows.json'), encoding='utf-8'))
PROTO = ROWS_JSON['protocol']
BLOCK_NAMES = [b for b in PROTO['blocks'] if not ONLY_BLOCKS or b in ONLY_BLOCKS]
BLOCKS = [{'name': b, 'passes': 1 if TEST else PROTO['passes'], 'rounds': 1 if TEST else PROTO['rounds_per_pass']}
          for b in BLOCK_NAMES]
WARMUP = {'headline': ('HL-D-tree', 'tip', 8), 'g9': ('J-As', 'tip', 8), 'armed': ('J-As', 'tip', 8)}
ROWS = {}
for rd in ROWS_JSON['rows']:
    r = dict(rd)
    r['ws'] = tuple(rd['workers'])
    r['dry_steps'] = 12
    r['dry_window'] = (0, 12)
    ROWS[rd['id']] = r
MANIFEST_ENTRIES = {}
for role, e in ROWS_JSON['binaries'].items():
    MANIFEST_ENTRIES[role] = {'exe': os.path.join(W5, e['exe'].replace('bin/', 'bin' + os.sep, 1)), 'sha256': e['sha256'],
                              'commit': e['commit']}
TREE_KEYS = ('static_rebuilds', 'sleeper_rebuilds', 'evictions', 'translations', 'patches', 'hint_candidates',
             'wide_rows', 'excluded_rows', 'locator_resets', 'members')

CREATE_SUSPENDED = 0x4
k32 = ctypes.WinDLL('kernel32', use_last_error=True)
k32.GetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_size_t), ctypes.POINTER(ctypes.c_size_t)]
k32.GetProcessAffinityMask.restype = wintypes.BOOL
k32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
k32.GetProcessTimes.restype = wintypes.BOOL
k32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
k32.OpenProcess.restype = wintypes.HANDLE
k32.CloseHandle.argtypes = [wintypes.HANDLE]
k32.CloseHandle.restype = wintypes.BOOL
k32.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
k32.QueryFullProcessImageNameW.restype = wintypes.BOOL
k32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
k32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
k32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(D.PROCESSENTRY32W)]
k32.Process32FirstW.restype = wintypes.BOOL
k32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(D.PROCESSENTRY32W)]
k32.Process32NextW.restype = wintypes.BOOL
ntdll = ctypes.WinDLL('ntdll')
ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]
ntdll.NtResumeProcess.restype = ctypes.c_long
ntdll.NtQuerySystemInformation.argtypes = [ctypes.c_int, ctypes.c_void_p, wintypes.ULONG, ctypes.POINTER(wintypes.ULONG)]
ntdll.NtQuerySystemInformation.restype = ctypes.c_long
NCPU = os.cpu_count()
MY_PID = os.getpid()
# The idle script's name list plus the driver's BUILD_PROCS (both, lower-case, with .exe).
VOID_NAMES = set(D.BUILD_PROCS) | {'lld-link.exe', 'cl.exe', 'msbuild.exe', 'runner_c3.exe'}
LANE_PREFIXES = ('d:\\wt\\_targets\\', 'd:\\wt\\mq-')


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


def presence():
    """Every running process that voids a pass: a build/miri name, or an image under D:/wt/_targets or D:/wt/mq-*.
    Existence, not CPU use - an idle cargo still counts."""
    names, lanes = [], []
    snap = k32.CreateToolhelp32Snapshot(0x2, 0)
    if not snap or snap == D.INVALID_HANDLE:
        return {'build': ['SNAPSHOT-FAILED'], 'lane': []}
    try:
        e = D.PROCESSENTRY32W()
        e.dwSize = ctypes.sizeof(D.PROCESSENTRY32W)
        ok = k32.Process32FirstW(snap, ctypes.byref(e))
        while ok:
            n = e.szExeFile.lower()
            pid = e.th32ProcessID
            if n in VOID_NAMES:
                names.append(f'{e.szExeFile}({pid})')
            h = k32.OpenProcess(0x1000, False, pid)
            if h:
                buf = ctypes.create_unicode_buffer(1024)
                sz = wintypes.DWORD(1024)
                if k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(sz)):
                    p = buf.value.lower().replace('/', '\\')
                    if p.startswith(LANE_PREFIXES):
                        lanes.append(f'{buf.value}({pid})')
                k32.CloseHandle(h)
            ok = k32.Process32NextW(snap, ctypes.byref(e))
    finally:
        k32.CloseHandle(snap)
    return {'build': names, 'lane': lanes}


def receipt_with_presence(window_s):
    r = D.load_receipt(window_s)
    r['presence'] = presence()
    return r


def void_seen(receipt):
    pr = receipt.get('presence') or {}
    return bool(pr.get('build') or pr.get('lane') or receipt.get('build_procs_busy'))


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
    """P-none launch, identical to window 3's / 4b's: CreateProcess suspended, read the (inherited) mask back,
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
    info['build_proc_during'] = sorted({n for d, n, pid in oth if n.lower() in VOID_NAMES})
    # Processes that STARTED and ENDED inside the process's run appear in t_tab1 only if still alive; new pids in
    # t_tab1 whose name voids are caught here, and the presence snapshot after the process catches survivors.
    new_void = sorted({n for pid, (n, cpu) in t_tab1.items() if pid not in t_tab0 and n.lower() in VOID_NAMES})
    info['void_names_new_during'] = new_void
    keys = [f'0,{i}' for i in range(NCPU)]
    info['perf'] = [{'t': t, 'total': v.get('_Total'), 'cpu': [v.get(k) for k in keys]} for t, v in perf]
    return info, so, se


def boyko_args(row, w, cwd, role):
    steps = row['dry_steps'] if TEST else row['steps']
    window = row['dry_window'] if TEST else row['window']
    args = list(row['args'])
    args += ['--workers', str(w), '--steps', str(steps), '--window', f'{window[0]}..{window[1]}',
             '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
             '--label', f'{row["id"]}#{role}@W{w}']
    if row['armed']:
        args.append('--arm-profiler')
    gate_pose = os.path.join(GATE, f'{row["pose_ref"]}_ref.pose')
    if not TEST:
        args += ['--expect-pose', gate_pose]
    return args


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
        if bool(s.get('armed')) != bool(row['armed']):
            why.append(f'armed {s.get("armed")} != {row["armed"]}')
        bt = s.get('broadphase_tree')
        if not TEST:
            if not isinstance(bt, dict):
                why.append('no broadphase_tree')
            elif row['tree']:
                if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
                    why.append(f'TreeDiag {bt}')
            elif any(bt.get(k) != 0 for k in TREE_KEYS):
                why.append(f'TreeDiag nonzero on allpairs {bt}')
            cfg = s.get('config') or {}
            want_bp = 'Tree' if row['tree'] else 'AllPairs'
            if isinstance(cfg, dict) and cfg.get('broadphase') != want_bp:
                why.append(f'broadphase {cfg.get("broadphase")} != {want_bp}')
    return why


def run_one(m, rid, role, w, block, pass_no, pass_attempt, rnd, seq, tag, pdir, receipt, env, sampler):
    row = ROWS[rid]
    entry = m['binaries'][role]
    suffix = '' if tag == 'original' else f'_{tag}'
    cwd = os.path.join(pdir, f'{seq:03d}_r{rnd}_{rid}_{role}_W{w}{suffix}')
    os.makedirs(cwd, exist_ok=True)
    rec = {'block': block, 'pass': pass_no, 'pass_attempt': pass_attempt, 'round': rnd, 'seq': seq, 'row': rid,
           'engine': 'boyko', 'role': role, 'binary': role, 'W': w, 'dry': TEST, 'attempt': tag, 'policy': 'P-none',
           'timed': tag != 'warmup'}
    args = boyko_args(row, w, cwd, role)
    t0 = time.time()
    while busy(receipt) and not TEST and not void_seen(receipt):
        if time.time() - t0 >= QUIET_BUDGET_S:
            log(f'  quiet budget exhausted before {rid}#{role} W={w}; running the idle rule')
            rc = wait_idle()
            if rc != 0:
                rec['aborted'] = f'machine never idle (wait_idle exit {rc})'
                rec['receipt_before'] = receipt
                return rec, receipt, 'abort'
            receipt = receipt_with_presence(RECEIPT_S)
            t0 = time.time()
            continue
        time.sleep(20)
        receipt = receipt_with_presence(RECEIPT_S)
    rec['waited_s'] = round(time.time() - t0, 1)
    if void_seen(receipt) and VOID_CHECK:
        rec['receipt_before'] = receipt
        rec['void'] = f'build/lane process present before the process: {receipt.get("presence")} busy {receipt.get("build_procs_busy")}'
        return rec, receipt, 'void'
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
    receipt = receipt_with_presence(RECEIPT_S)
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
    if VOID_CHECK and (void_seen(receipt) or rec.get('build_proc_during') or rec.get('void_names_new_during')):
        rec['void'] = (f'build/lane process during/after the process: presence {receipt.get("presence")} '
                       f'busy {receipt.get("build_procs_busy")} during {rec.get("build_proc_during")} '
                       f'new {rec.get("void_names_new_during")}')
        return rec, receipt, 'void'
    return rec, receipt, 'ok'


def tag_of(rec):
    if 'aborted' in rec:
        return f'ABORTED ({rec["aborted"]})'
    m = rec.get('mean_ms')
    t = f'mean {m:.4f} ms' if m is not None else 'mean NONE'
    extra = ''
    if rec.get('summary'):
        extra = f' pose {rec["summary"]["pose_hash"]} expect {rec["summary"].get("expect_pose")}'
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
    if rec.get('void'):
        flags.append(f'VOID({rec["void"]})')
    rb = rec.get('receipt_before', {}).get('cpu_avg')
    ra = rec.get('receipt_after', {}).get('cpu_avg') if rec.get('receipt_after') else None
    return (f'{t}{extra} rb {rb}% ra {ra}% '
            f'others {rec.get("others_busy_pct")}% cpu_s {rec.get("proc_cpu_s")} wall {rec.get("wall_s")}s{pt} '
            f'waited {rec.get("waited_s")}s {" ".join(flags)}').rstrip()


def cell_order(block):
    """Pass-0 order of one round: W ascending; inside a W group row by row (rows.json order), binaries in the
    row's order (parent then tip)."""
    out = []
    ids = [rid for rid, r in ROWS.items() if r['block'] == block]
    for w in (1, 2, 4, 8, 16):
        for rid in ids:
            if w not in ROWS[rid]['ws']:
                continue
            for role in ROWS[rid]['binaries']:
                out.append((rid, role, w))
    if TEST:
        out = [c for c in out if c[2] in (1, 8)]
    return out


def wait_idle():
    if TEST:
        return 0
    return subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', WAIT_PS1,
                           '-Log', WAIT_LOG, '-MaxPolls', str(IDLE_MAX_POLLS)]).returncode


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


def run_pass(m, blk, p, attempt, runs_path, env, sampler, counters):
    """One pass (warm-up, rounds, end-of-pass re-runs). Returns 'ok', ('void', why) or ('abort', why)."""
    bname = blk['name']
    ptag = f'{bname}-p{p}' + (f'-v{attempt}' if attempt else '')
    pdir = os.path.join(RAW, ptag)
    os.makedirs(pdir, exist_ok=True)
    order0 = cell_order(bname)
    order = order0 if p % 2 == 0 else order0[::-1]
    receipt = receipt_with_presence(PASS_RECEIPT_S)
    log(f'{ptag}: opening receipt {receipt["cpu_avg"]}% (build procs busy {receipt["build_procs_busy"]}, presence {receipt["presence"]})')
    if void_seen(receipt) and VOID_CHECK:
        return ('void', f'opening receipt presence {receipt["presence"]}')
    wr, ww, wwk = WARMUP[bname]
    wrec, receipt, st = run_one(m, wr, ww, wwk, bname, p, attempt, -1, 0, 'warmup', pdir, receipt, env, sampler)
    append(runs_path, wrec)
    counters['warm'] += 1
    log(f'  {ptag} warm-up {wr}#{ww} W={wwk} (untimed): {tag_of(wrec)}')
    if st == 'abort':
        return ('abort', wrec.get('aborted'))
    if st == 'void':
        return ('void', wrec.get('void'))
    seq = 1
    bad_runs = []
    for rnd in range(blk['rounds']):
        for rid, role, w in order:
            rec, receipt, st = run_one(m, rid, role, w, bname, p, attempt, rnd, seq, 'original', pdir, receipt, env, sampler)
            append(runs_path, rec)
            if 'aborted' not in rec and 'mean_ms' in rec:
                counters['timed'] += 1
            log(f'  {ptag} r{rnd} [{seq:3d}] {rid}#{role} W={w}: {tag_of(rec)}')
            if st == 'abort':
                return ('abort', rec.get('aborted'))
            if st == 'void':
                return ('void', rec.get('void'))
            if rec['contaminated'] or not rec['valid']:
                bad_runs.append((seq, rnd, rid, role, w, 'contaminated' if rec['contaminated'] else 'invalid'))
            seq += 1
    if bad_runs:
        log(f'{ptag}: re-running {len(bad_runs)} process(es) at the end of the pass: {bad_runs}')
    for oseq, rnd, rid, role, w, why in bad_runs:
        rec, receipt, st = run_one(m, rid, role, w, bname, p, attempt, rnd, oseq, 'rerun', pdir, receipt, env, sampler)
        rec['rerun_reason'] = why
        append(runs_path, rec)
        if 'aborted' not in rec and 'mean_ms' in rec:
            counters['timed'] += 1
            counters['rerun'] += 1
        log(f'  {ptag} r{rnd} [{oseq:3d}] {rid}#{role} W={w} RERUN ({why}): {tag_of(rec)}')
        if st == 'abort':
            return ('abort', rec.get('aborted'))
        if st == 'void':
            return ('void', rec.get('void'))
    log(f'{ptag}: done')
    return 'ok'


def main():
    os.makedirs(RAW, exist_ok=True)
    m = {'generated': D.now(), 'rows_json': os.path.join(W5, 'rows.json'), 'toolchain': 'stable-x86_64-pc-windows-msvc',
         'profile': 'parity', 'logical_cpus': NCPU, 'binaries': MANIFEST_ENTRIES,
         'rows': {rid: r for rid, r in ROWS.items()}, 'protocol': PROTO, 'blocks': BLOCKS, 'gate_dir': GATE}
    with open(os.path.join(RAW, 'manifest.json'), 'w', encoding='utf-8') as f:
        json.dump(m, f, indent=1, default=str)
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    state = {'start': D.now(), 'test': TEST, 'placement': 'P-none (no affinity call)', 'blocks': [], 'voids': [],
             'machine_before': machine_state()}
    log(f'WINDOW START {json.dumps(state)}')
    status = 'complete'
    runs_path = os.path.join(RAW, 'runs.jsonl')
    sampler = PerfSampler(1.0)
    counters = {'timed': 0, 'rerun': 0, 'warm': 0, 'voided_processes': 0}
    for blk in BLOCKS:
        bname = blk['name']
        order0 = cell_order(bname)
        state['blocks'].append({'name': bname, 'passes': blk['passes'], 'rounds': blk['rounds'], 'cells_per_round': len(order0),
                                'order_pass0': [f'{r}#{b}@W{w}' for r, b, w in order0]})
        log(f'block {bname}: {len(order0)} cells per round, {blk["passes"]} passes x {blk["rounds"]} rounds')
        for p in range(blk['passes']):
            attempt = 0
            while True:
                ptag = f'{bname}-p{p}' + (f'-v{attempt}' if attempt else '')
                log(f'{ptag}: idle rule (wait_idle5.ps1, up to {IDLE_MAX_POLLS} polls)')
                rc = wait_idle()
                if rc != 0:
                    status = f'STOP before {ptag}: idle rule not met in {IDLE_MAX_POLLS} min (wait_idle exit {rc})'
                    break
                log(f'{ptag}: idle reached; {machine_state()}')
                bad = verify_binaries(m)
                if bad:
                    status = f'STOP before {ptag}: binaries changed {bad}'
                    break
                log(f'{ptag}: binaries verified against the manifest and bin/SHA256SUMS')
                timed_before = counters['timed']
                res = run_pass(m, blk, p, attempt, runs_path, env, sampler, counters)
                if res == 'ok':
                    break
                kind, why = res
                if kind == 'abort':
                    status = f'ABORT in {ptag}: {why}'
                    break
                n_void = counters['timed'] - timed_before
                counters['voided_processes'] += n_void
                state['voids'].append({'block': bname, 'pass': p, 'attempt': attempt, 'time': D.now(), 'why': why,
                                       'timed_processes_voided': n_void})
                append(runs_path, {'voided_pass': True, 'block': bname, 'pass': p, 'pass_attempt': attempt, 'why': why,
                                   'time': D.now()})
                log(f'{ptag}: PASS VOIDED ({why}); {n_void} timed processes discarded; waiting for idle and re-running')
                attempt += 1
                if attempt > MAX_VOIDS_PER_PASS:
                    status = f'STOP in {bname}-p{p}: voided {attempt} times'
                    break
            if status != 'complete':
                break
        if status != 'complete':
            break
    sampler.close()
    state['end'] = D.now()
    state['machine_after'] = machine_state()
    state['binaries_after'] = verify_binaries(m) or 'all match'
    state['status'] = status
    state['timed_processes'] = counters['timed']
    state['reruns'] = counters['rerun']
    state['warmups'] = counters['warm']
    state['voided_processes'] = counters['voided_processes']
    log(f'WINDOW END {json.dumps({k: state[k] for k in ("end", "status", "binaries_after", "machine_after", "timed_processes", "reruns", "warmups", "voided_processes", "voids")})}')
    with open(os.path.join(RAW, 'window_state.json'), 'w', encoding='utf-8') as f:
        json.dump(state, f, indent=1)
    with open(os.path.join(RAW, 'WINDOW_DONE'), 'w', encoding='utf-8') as f:
        f.write(status + '\n')


if __name__ == '__main__':
    main()
