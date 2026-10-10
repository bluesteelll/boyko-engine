"""WINDOW 7 WAVE 2 (2026-09-25) driver: window 7's window7_run.py (verbatim protocol: P-none launch, receipts, presence,
VOID rule, idle rule before every pass, re-run once at the end of the pass, checkpoints, --resume) with:
  * rows7b.json: six blocks Q1C, Q2C, Q3, Q4, Q2D, Q1D (Q1's and Q2's two separated blocks open and close the run);
    each block names its item and its PRIORITY (Q1 1, Q2 2, Q3 3, Q4 4); a block may set its own passes/rounds
    (Q3: 1 pass x 3 rounds = K 3, the recipe's letter) and may have no warm-up (Q3, a criterion process);
  * PRIORITY SKIP: before a block starts, the driver reserves the estimated time of every LATER block of a HIGHER
    priority (a smaller number); if the block would not fit before the cutoff with that reserve, it is skipped
    (logged, progress line 'SKIPPED for priority') so the closing Q2D/Q1D blocks are never crowded out;
  * a HARD STOP two ways: --cutoff HH:MM (default 11:45: no process starts whose estimated end passes it) and the
    flag file win7b/STOP (checked before every process and pass; creating it stops the driver at the next check,
    exit 2 'cut by the STOP flag');
  * LAUNCH-CONTEXT LENGTHS: a row may pin the length of its process directory per W ('cwd_len'); the driver pads the
    directory name with 'x' so the cwd string, and therefore every path argument (--csv, --pose-out), has exactly
    that length (Q1: window 7's P1A/P1B lengths; Q2: window 6's P3 layout 163 and the P5g layout 166 = the padded
    variant). The record keeps cwd_len_target / cwd_len_actual and cmdline_chars; a miss is flagged, not voided;
  * a per-row fixture path ('pose_path') so the --expect-pose argument is byte-identical to the earlier window's;
  * the criterion kind reads the 'time:' middle estimate of every id the row expects ('expect').
Everything else as window 7 (window7_run.py's docstring follows).

WINDOW 7 (2026-09-25) driver: window 6's window6_run.py (verbatim protocol) plus two process kinds, Jolt's
PerformanceTest (per_frame csv; hash, threads and patch banner checked) and the profiled Jolt build (per-stage times
from its single-frame profile dumps), rows that belong to several blocks (P1 A and B), the canary reference
restored from runs.jsonl on --resume, and a 04:15 default cutoff.
WINDOW 6 (2026-09-24) driver. Adapted from window 5's window5_run.py (itself window 4b's / window 3's): the same
P-none launch (CreateProcess suspended, mask read back, resume), the same receipts, presence snapshots and VOID
rule, the same record layout in raw/runs.jsonl. Differences:
  * one rows6.json with BLOCKS IN PRIORITY ORDER (P1 L10, P2 L9, P3 C3b, P4 G9, P5 extras); each block is
    2 passes x 3 rounds (K = 6); pass 0: W ascending, inside a W group row by row (rows6.json order), binaries in
    the row's order (arms alternate inside the W group); pass 1: the whole list reversed; one untimed warm-up per
    pass (the block's 'warmup' cell);
  * three process kinds: the parity runner (window mean of wall_ns over the row's window and every zone column's
    mean and median over the window and the row's metric_windows), the L9 class bench (--bench; its refutation
    band and per-class ns), a filtered criterion run (the 'time:' estimates);
  * the idle rule before EVERY pass (tools/wait_idle6.ps1: 3 consecutive quiet 60-s polls, up to 30 polls); a
    timeout STOPS the window;
  * a HARD STOP at the cutoff (default 12:45 local): no process is started whose estimated end passes the cutoff;
    the current process always finishes; WINDOW_DONE then reads 'cut at 12:45 after <block> ...';
  * checkpoints: every process appends to raw/runs.jsonl at once; every finished pass writes progress.txt lines
    '<time> <item> <row> p<n> done' and raw/passes_done.txt (--resume skips the passes listed there);
  * --dry-run prints the schedule and the estimated duration per block (gate/estimates_*.json: one untimed process
    per cell), runs nothing; --test runs an untimed rehearsal (tiny steps, one round, one pass, no idle wait) under
    test/.
Protocol otherwise as windows 3-5: a cell = MEDIAN over K processes of the process statistic; 5-s receipt between
processes (the receipt after process i is the receipt before process i+1); 10-s receipt opening each pass; a
process whose before/after receipt is > 5 % busy (or invalid) is re-run ONCE at the end of its pass; a build
process (cargo/rustc/link/miri/...) or any process whose image is under D:/wt/_targets or D:/wt/mq-* seen at any
point of a pass VOIDS the pass: it is abandoned, the idle rule re-run and the pass re-run from its start.
"""

import argparse
import ctypes
import datetime as dt
import json
import os
import re
import statistics
import subprocess
import sys
import time
from ctypes import wintypes

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W6 = os.path.dirname(HERE)  # the win7b directory (name kept from window 6's driver)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, os.path.join(HERE, 'window_lib'))
import driver as D  # noqa: E402  (window 3's byte-identical tree driver; only helpers are used)

AP = argparse.ArgumentParser()
AP.add_argument('--dry-run', action='store_true')
AP.add_argument('--test', action='store_true')
AP.add_argument('--test-real', action='store_true')
AP.add_argument('--resume', action='store_true')
AP.add_argument('--blocks', default='')
AP.add_argument('--cutoff', default='11:45')
AP.add_argument('--start', default='', help='dry-run only: the assumed start time HH:MM (default: now)')
AP.add_argument('--test-cutoff-s', type=float, default=None, help='--test only: exercise the hard stop N s after start')
ARGS = AP.parse_args()
FULL = ARGS.test_real  # untimed rehearsal of the control flow with REAL steps, windows and pose checks (test/real/)
TEST = ARGS.test or FULL
SHORT = ARGS.test and not FULL  # 12-step rehearsal: no metric windows, no pose/hash checks
DRY = ARGS.dry_run

WAIT_PS1 = os.path.join(HERE, 'wait_idle7.ps1')
RAW = os.path.join(W6, 'test', 'real' if FULL else 'window') if TEST else os.path.join(W6, 'raw')
# the hard-stop flag file: create it to stop at the next check (a rehearsal has its own, under its test directory)
STOP_FLAG = os.path.join(RAW, 'STOP') if TEST else os.path.join(W6, 'STOP')
WAIT_LOG = os.path.join(W6, 'wait_log.txt') if not TEST else os.path.join(RAW, 'wait_log.txt')
PROGRESS = os.path.join(W6, 'progress.txt') if not TEST else os.path.join(RAW, 'progress.txt')
DONE_FILE = os.path.join(RAW, 'driver_done.txt')  # run_window.sh turns it into WINDOW_DONE after the reduction
PASSES_DONE = os.path.join(RAW, 'passes_done.txt')
BIN = os.path.join(W6, 'bin')
GATE = os.path.join(W6, 'gate')
FIX = os.path.join(GATE, 'fixtures')
BUSY_PCT = 5.0
RECEIPT_S = 0.5 if TEST else 5.0
PASS_RECEIPT_S = 0.5 if TEST else 10.0
QUIET_BUDGET_S = 420.0
IDLE_MAX_POLLS = 30
IDLE_MIN_S = 135.0          # three polls 60 s apart, each with a 10-s counter sample
LAUNCH_OVERHEAD_S = 0.4
MAX_VOIDS_PER_PASS = 20
RUN_TAG = time.strftime('%H%M%S')  # one per driver invocation: a resumed pass never overwrites a cut pass's files
CTX = {}  # (row, binary, W) -> the latest valid window_mean_ns: the canary's reference (P0 recipe, driver.py:427)

ROWS_JSON = json.load(open(os.path.join(W6, 'rows7b.json'), encoding='utf-8'))
PROTO = ROWS_JSON['protocol']
ONLY = [b for b in ARGS.blocks.split(',') if b]
BLOCKS = [b for b in ROWS_JSON['blocks'] if not ONLY or b['name'] in ONLY]
ROWS = {}
for rd in ROWS_JSON['rows']:
    r = dict(rd)
    r.setdefault('kind', 'runner')
    ROWS[r['id']] = r
BINS = {}
for key, e in ROWS_JSON['binaries'].items():
    BINS[key] = dict(e)
    BINS[key]['path'] = (e['exe'].replace('/', os.sep) if ':' in e['exe'] else os.path.join(W6, e['exe'].replace('/', os.sep)))
SUMS = {}
for line in open(os.path.join(BIN, 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split()
        SUMS[name.lstrip('*')] = sha
for key, e in BINS.items():
    e['sha256'] = SUMS.get(e['exe'])
EST = {}
for f in ('estimates_runner.json', 'estimates_bench.json'):
    p = os.path.join(GATE, f)
    if os.path.exists(p):
        EST.update(json.load(open(p, encoding='utf-8')))
FIXTURES = json.load(open(os.path.join(FIX, 'fixtures.json'), encoding='utf-8')) if os.path.exists(
    os.path.join(FIX, 'fixtures.json')) else {}
KNOWN = {'J500': '0x32d5e235342b4143'}  # every other hash comes from gate/fixtures/fixtures.json (checked by gate7b.py)
TREE_KEYS = ('static_rebuilds', 'sleeper_rebuilds', 'evictions', 'translations', 'patches', 'hint_candidates',
             'wide_rows', 'excluded_rows', 'locator_resets', 'members')
# per-step count columns kept beside the *_ns spans (the per-manifold and per-pair denominators)
COUNT_COLS = ('manifolds', 'pairs', 'awake', 'waves', 'colors', 'wide_colors', 'phys_np_pairs', 'phys_np_manifolds',
              'phys_np_points', 'phys_bp_pairs', 'phys_np_reused', 'phys_np_sep_hits', 'phys_np_full', 'phys_bp_queried',
              'phys_slots_wide', 'phys_slots_narrow', 'phys_np_chunks')
sys.path.insert(0, HERE)
import joltprof  # noqa: E402  (the profile-dump parser, P5)


def restore_ctx():
    """The canary's reference survives a --resume: the latest valid, clean reference process in runs.jsonl."""
    p = os.path.join(RAW, 'runs.jsonl')
    if not os.path.exists(p):
        return
    for line in open(p, encoding='utf-8'):
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if (r.get('kind') == 'runner' and r.get('attempt') in ('original', 'rerun') and r.get('valid')
                and not r.get('contaminated') and not r.get('void') and (r.get('summary') or {}).get('window_mean_ns')):
            CTX[(r['row'], r['binary'], r['W'])] = r['summary']['window_mean_ns']


def cutoff_dt():
    hh, mm = (int(x) for x in ARGS.cutoff.split(':'))
    now = dt.datetime.now()
    return now.replace(hour=hh, minute=mm, second=0, microsecond=0)


CUTOFF = cutoff_dt()
if TEST and ARGS.test_cutoff_s is not None:
    CUTOFF = dt.datetime.now() + dt.timedelta(seconds=ARGS.test_cutoff_s)


def est_s(rid, key, w):
    """Estimated wall of one process of the cell (the gate's untimed run), plus the launch overhead."""
    v = EST.get(f'{rid}#{key}@W{w}')
    if v is None:
        row = ROWS[rid]
        v = 0.01 * row.get('steps', 500)
    return v + LAUNCH_OVERHEAD_S


# ------------------------------------------------------------------ process presence and CPU (window 5 verbatim)
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
VOID_NAMES = set(D.BUILD_PROCS) | {'lld-link.exe', 'cl.exe', 'msbuild.exe'}
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
        d = t1 - t0
        out.append(round(100.0 * (1.0 - (i1 - i0) / d), 1) if d > 0 else None)
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
    with open(os.path.join(RAW, 'window_log.txt'), 'a', encoding='utf-8', newline='\n') as f:
        f.write(line + '\n')


def progress(line):
    with open(PROGRESS, 'a', encoding='utf-8', newline='\n') as f:
        f.write(f'{dt.datetime.now().strftime("%H:%M:%S")} {line}\n')


def busy(receipt):
    return (receipt['cpu_avg'] or 0) > BUSY_PCT or bool(receipt['build_procs_busy'])


def launch(cmd, cwd, env, sampler):
    """P-none launch, identical to windows 3-5: CreateProcess suspended, read the (inherited) mask back, resume,
    wait. No SetProcessAffinityMask call is made."""
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
    new_void = sorted({n for pid, (n, cpu) in t_tab1.items() if pid not in t_tab0 and n.lower() in VOID_NAMES})
    info['void_names_new_during'] = new_void
    keys = [f'0,{i}' for i in range(NCPU)]
    info['perf'] = [{'t': t, 'total': v.get('_Total')} for t, v in perf]
    return info, so, se


# ------------------------------------------------------------------ per-kind arguments, statistics, validation
def steps_of(row):
    return 12 if SHORT else row['steps']


def window_of(row):
    if SHORT:
        return (0, 12)
    return tuple(row['window'])


def runner_args(row, key, w, cwd):
    win = window_of(row)
    args = list(row['args'])
    args += ['--workers', str(w), '--steps', str(steps_of(row)), '--window', f'{win[0]}..{win[1]}',
             '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
             '--label', f'{row["id"]}#{key}@W{w}']
    if row['armed']:
        args.append('--arm-profiler')
    if not SHORT:
        args += ['--expect-pose', pose_path(row)]
    return args


def pose_path(row):
    """The fixture file of the row: its own 'pose_path' (the earlier window's file, so the argument is byte-identical
    to that window's) or gate/fixtures/<pose_ref>.pose."""
    pp = row.get('pose_path')
    return pp.replace('/', os.sep) if pp else os.path.join(FIX, f'{row["pose_ref"]}.pose')


def process_dir(pdir, seq, rnd, rid, key, w, tag):
    """The process directory. A row with 'cwd_len' {W: n} gets its leaf padded with 'x' so the full cwd string has
    exactly n characters (the launch context of the earlier window it is compared with); a re-run is suffixed '_R'
    (2 characters, so it still fits), a warm-up '_warmup' (untimed, never padded). Returns (cwd, target, actual)."""
    row = ROWS[rid]
    leaf = f'{seq:03d}_r{rnd}_{rid}_{key}_W{w}'
    if tag == 'rerun':
        leaf += '_R'
    elif tag != 'original':
        leaf += f'_{tag}'
    base = os.path.join(pdir, leaf)
    target = (row.get('cwd_len') or {}).get(str(w)) if tag in ('original', 'rerun') else None
    if target and not SHORT and len(base) < target:
        base = base + 'x' * (target - len(base))
    return base, target, len(base)


def col_stats(cols, lo, hi):
    out = {}
    for name, col in cols.items():
        if not (name == 'wall_ns' or name.endswith('_ns') or name in COUNT_COLS):
            continue
        v = [x for x in col[lo:hi] if x is not None]
        if not v:
            continue
        out[name] = {'mean': statistics.mean(v), 'median': statistics.median(v)}
    return out


def runner_stats(row, cwd, summary=None):
    c = D.load_csv(os.path.join(cwd, 'run.csv'))
    win = window_of(row)
    res = {'mean_ms': None, 'median_ms': None, 'n_steps': None, 'expect_steps': win[1] - win[0], 'cols': {}}
    if not c or 'wall_ns' not in c:
        return res
    wall = c['wall_ns'][win[0]:win[1]]
    if not wall or any(x is None for x in wall):
        return res
    res['mean_ms'] = sum(wall) / len(wall) / 1e6
    res['median_ms'] = statistics.median(wall) / 1e6
    res['n_steps'] = len(wall)
    wins = [list(win)] + ([] if SHORT else [list(m) for m in row.get('metric_windows', [])])
    res['cols'] = {f'{a}..{b}': col_stats(c, a, b) for a, b in wins}
    ffs = (summary or {}).get('first_frozen_step')
    if isinstance(ffs, int) and ffs < steps_of(row):
        # the all-frozen tail (awake == 0 from this step on), beside the design's literal window
        res['cols']['frozen..end'] = col_stats(c, ffs, steps_of(row))
        res['frozen_from'] = ffs
    return res


def validate_runner(rec, row, w):
    why = []
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    if rec.get('mean_ms') is None:
        why.append('no window mean')
    elif rec.get('n_steps') != rec.get('expect_steps'):
        why.append(f'{rec.get("n_steps")} steps in the window, expected {rec.get("expect_steps")}')
    s = rec.get('summary')
    if not s:
        why.append('no SUMMARY')
        return why
    if s.get('void_steps') != 0:
        why.append(f'void_steps {s.get("void_steps")}')
    if s.get('workers') != w:
        why.append(f'workers {s.get("workers")} != {w}')
    if s.get('target_env') != 'msvc':
        why.append(f'target_env {s.get("target_env")}')
    if not SHORT:
        want = KNOWN.get(row['pose_ref']) or FIXTURES.get(row['pose_ref'], {}).get('hash')
        if s.get('pose_hash') != want:
            why.append(f'pose {s.get("pose_hash")} != {want}')
        if s.get('expect_pose') != 'match':
            why.append(f'expect_pose {s.get("expect_pose")}')
    if s.get('disarmed_ring_traffic'):
        why.append(f'disarmed_ring_traffic {s.get("disarmed_ring_traffic")}')
    if row['armed'] and s.get('drops_total'):
        why.append(f'drops_total {s.get("drops_total")}')
    if bool(s.get('armed')) != bool(row['armed']):
        why.append(f'armed {s.get("armed")} != {row["armed"]}')
    cfg = s.get('config') or {}
    if isinstance(cfg, dict) and cfg.get('broadphase') != row['broadphase']:
        why.append(f'broadphase {cfg.get("broadphase")} != {row["broadphase"]}')
    bt = s.get('broadphase_tree')
    if isinstance(bt, dict) and not SHORT:
        if row['broadphase'] == 'Tree':
            if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
                why.append(f'TreeDiag {bt}')
        elif any(bt.get(k) != 0 for k in TREE_KEYS):
            why.append(f'TreeDiag nonzero on {row["broadphase"]} {bt}')
    return why


CLASS_BAND = re.compile(r'^(\w+): design refutation reading dt_np\(1\) in \[([-\d.]+), ([-\d.]+)\] ms')
CLASS_NS = re.compile(r'^(\w+): (\w+) (\d+) pairs ([\d.]+) ns/pair')


def classes_stats(so):
    txt = D.decode(so)
    res = {'band': {}, 'ns': {}, 'summary': None}
    for line in txt.splitlines():
        m = CLASS_BAND.match(line)
        if m:
            res['band'][m.group(1)] = [float(m.group(2)), float(m.group(3))]
        m = CLASS_NS.match(line)
        if m:
            res['ns'][f'{m.group(1)}/{m.group(2)}'] = {'pairs': int(m.group(3)), 'ns_per_pair': float(m.group(4))}
        if line.startswith('SUMMARY '):
            try:
                res['summary'] = json.loads(line[8:])
            except ValueError:
                pass
    j = res['band'].get('jolt')
    res['mean_ms'] = (j[0] + j[1]) / 2 if j else None
    return res


CRIT_TIME = re.compile(r'time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]')
UNIT = {'ps': 1e-9, 'ns': 1e-6, 'us': 1e-3, 'ms': 1.0, 's': 1e3}


def unit_ms(u):
    # criterion prints U+00B5 (micro sign) for microseconds; accept U+03BC, a mangled byte pair or 'u' too
    if u in UNIT:
        return UNIT[u]
    if u.endswith('s') and len(u) >= 2 and u[-2] in ('µ', 'μ', '�', 'u'):
        return 1e-3
    return float('nan')


def criterion_stats(so, se):
    res = {'est_ms': {}}
    last = None
    for line in (D.decode(so) + '\n' + D.decode(se)).splitlines():
        s = line.strip()
        if s.startswith('bp_g4_') and ':' not in s:
            last = s.split()[0]
        m = CRIT_TIME.search(line)
        if m:
            name = s.split()[0] if s.startswith('bp_g4_') else last
            if name:
                res['est_ms'][name] = [float(m.group(i)) * unit_ms(m.group(i + 1)) for i in (1, 3, 5)]
    return res


def criterion_expect(row):
    return list(row.get('test_expect') or []) if SHORT else list(row.get('expect') or [])


def criterion_args(row):
    """The row's criterion arguments; the 12-step rehearsal (--test) runs one id for 0.3 s instead."""
    if SHORT:
        return ['--bench', '--noplot', '--warm-up-time', '0.1', '--measurement-time', '0.2', row['test_filter']]
    return list(row['args'])


def jolt_args(row, w):
    return list(row['args']) + [f'-t={w}', f'-i={steps_of(row)}']


def jolt_stats(row, cwd, so):
    """Window 3's Jolt statistic: the per_frame csv's 'Time (ms)' (PhysicsSystem::Update, the harness's own clock),
    the window mean; plus mean/median over the metric windows. For the profiled build, the per-stage numbers of
    every profile dump inside [100, steps) (the harness dumps frames 0, 100, 200, ...; one frame each)."""
    res = {'jolt': D.parse_jolt_stdout(so), 'mean_ms': None, 'median_ms': None, 'n_steps': None, 'cols': {}}
    win = window_of(row)
    res['expect_steps'] = win[1] - win[0]
    files = sorted(os.listdir(cwd))
    res['files_n'] = len(files)
    pf = [f for f in files if f.startswith('per_frame_')]
    if not pf:
        return res
    c = D.load_csv(os.path.join(cwd, pf[0]))
    if not c or 'Time (ms)' not in c:
        return res
    wall = [x * 1e6 if x is not None else None for x in c['Time (ms)']]
    res['n_frames'] = len(wall)
    sel = wall[win[0]:win[1]]
    if not sel or any(x is None for x in sel):
        return res
    res['mean_ms'] = sum(sel) / len(sel) / 1e6
    res['median_ms'] = statistics.median(sel) / 1e6
    res['n_steps'] = len(sel)
    wins = [list(win)] + ([] if SHORT else [list(m) for m in row.get('metric_windows', [])])
    for a, b in wins:
        v = wall[a:b]
        if v and all(x is not None for x in v):
            res['cols'][f'{a}..{b}'] = {'wall_ns': {'mean': statistics.mean(v), 'median': statistics.median(v)}}
    if row['kind'] == 'joltprof':
        prof = {}
        for f in files:
            m = re.match(r'profile_chart_.*_it(\d+)\.html$', f)
            if not m:
                continue
            it = int(m.group(1))
            try:
                p = joltprof.parse(os.path.join(cwd, f))
            except Exception as ex:  # noqa: BLE001 - a broken dump is recorded, not fatal
                prof[str(it)] = {'error': repr(ex)}
                continue
            prof[str(it)] = {'threads': len(p['threads']), 'frame_ms': (wall[it] / 1e6 if it < len(wall) and wall[it] else None),
                             'scopes': {nm: [v['calls'], round(v['cpu_ms'], 6), round(v['wall_ms'], 6), v['depth_min']]
                                        for nm, v in p['scopes'].items()}}
        res['profile'] = prof
    return res


def validate_jolt(rec, row, w):
    why = []
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    if rec.get('mean_ms') is None:
        why.append('no window mean (per_frame csv)')
    elif rec.get('n_steps') != rec.get('expect_steps'):
        why.append(f'{rec.get("n_steps")} frames in the window, expected {rec.get("expect_steps")}')
    j = rec.get('jolt') or {}
    st = j.get('stat_lines') or []
    if len(st) != 1:
        why.append(f'{len(st)} stat lines')
    else:
        if st[0]['threads'] != w:
            why.append(f'threads {st[0]["threads"]} != {w}')
        if not SHORT and st[0]['hash'] != row['jolt_hash']:
            why.append(f'hash {st[0]["hash"]} != {row["jolt_hash"]}')
    pl = j.get('patch_line') or ''
    if not pl.startswith('boyko-parity-patch v1') or 'receipt=0' not in pl or 'allow_sleep=0' not in pl:
        why.append(f'patch banner {pl!r}')
    if row['kind'] == 'joltprof' and not SHORT:
        got = sorted(int(k) for k, v in (rec.get('profile') or {}).items() if 'scopes' in v)
        need = [i for i in range(100, steps_of(row), 100)]
        if [i for i in need if i not in got]:
            why.append(f'profile dumps {got}, need {need}')
    return why


def run_one(rid, key, w, block, pass_no, attempt_no, rnd, seq, tag, pdir, receipt, env, sampler):
    row = ROWS[rid]
    b = BINS[key]
    cwd, cwd_target, cwd_actual = process_dir(pdir, seq, rnd, rid, key, w, tag)
    os.makedirs(cwd, exist_ok=True)
    rec = {'run_tag': RUN_TAG, 'block': block['name'], 'item': block['item'], 'pass': pass_no, 'pass_attempt': attempt_no, 'round': rnd,
           'seq': seq, 'row': rid, 'kind': row['kind'], 'role': key, 'binary': key, 'W': w, 'dry': TEST, 'attempt': tag,
           'policy': 'P-none', 'timed': tag != 'warmup', 'cwd_len_target': cwd_target, 'cwd_len_actual': cwd_actual,
           'cwd_len_miss': bool(cwd_target and cwd_actual != cwd_target)}
    if row['kind'] == 'runner':
        args = runner_args(row, key, w, cwd)
        if row.get('canary_of'):
            ref = CTX.get((row['canary_of'], key, w))
            if ref is None:
                ref = 9.0e6 if TEST else None
            if ref is None:
                rec.update({'receipt_before': receipt, 'receipt_after': receipt, 'contaminated': False, 'valid': False,
                            'invalid': [f'no {row["canary_of"]} of {key} at W={w} yet for the canary reference'],
                            'mean_ms': None, 'skipped': True})
                return rec, receipt, 'ok'
            args += ['--canary-frac', str(row.get('canary_frac', 0.05)), '--canary-ref-ns', str(int(round(ref)))]
            rec['canary_ref_ns'] = int(round(ref))
    elif row['kind'] in ('jolt', 'joltprof'):
        args = jolt_args(row, w)
    elif row['kind'] == 'criterion':
        args = criterion_args(row)
    else:
        args = list(row['args'])
    t0 = time.time()
    while busy(receipt) and not TEST and not void_seen(receipt):
        if time.time() - t0 >= QUIET_BUDGET_S:
            log(f'  quiet budget exhausted before {rid}#{key} W={w}; running the idle rule')
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
    if rec['waited_s'] > 0:
        ensure_time(est_s(rid, key, w), f'{block["name"]}-p{pass_no} {rid}#{key}@W{w} after a {rec["waited_s"]} s quiet wait')
    if void_seen(receipt) and not TEST:
        rec['receipt_before'] = receipt
        rec['void'] = f'build/lane process present before the process: {receipt.get("presence")} busy {receipt.get("build_procs_busy")}'
        return rec, receipt, 'void'
    penv = dict(env)
    if row['kind'] == 'criterion':
        penv['CRITERION_HOME'] = os.path.join(cwd, 'criterion')
    rec.update({'exe': b['path'], 'exe_sha256': b['sha256'], 'commit': b['commit'], 'args': args, 'cwd': cwd,
                'cmdline_chars': sum(len(x) + 1 for x in args), 'receipt_before': receipt})
    rec['start'] = D.now()
    info, so, se = launch([b['path']] + args, cwd, penv, sampler)
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
    if row['kind'] == 'runner':
        rec['summary'] = D.parse_summary(so)
        rec.update(runner_stats(row, cwd, rec['summary']))
        rec['invalid'] = validate_runner(rec, row, w)
        if row.get('canary_of'):
            cn = (rec['summary'] or {}).get('canary_ns')
            if not cn:
                rec['invalid'].append(f'canary_ns {cn}: the canary did not run')
        elif not rec['invalid'] and not rec['contaminated'] and rec.get('summary'):
            CTX[(rid, key, w)] = rec['summary'].get('window_mean_ns')
    elif row['kind'] in ('jolt', 'joltprof'):
        rec.update(jolt_stats(row, cwd, so))
        rec['invalid'] = validate_jolt(rec, row, w)
    elif row['kind'] == 'classes':
        cs = classes_stats(so)
        rec.update({'classes': cs, 'mean_ms': cs['mean_ms']})
        why = []
        if rec['exit'] != 0:
            why.append(f'exit {rec["exit"]}')
        if 'jolt' not in cs['band']:
            why.append('no jolt refutation band')
        if not cs['summary']:
            why.append('no SUMMARY')
        rec['invalid'] = why
    else:
        cs = criterion_stats(so, se)
        need = criterion_expect(row)
        first = cs['est_ms'].get(need[0]) if need else None
        rec.update({'criterion': cs, 'mean_ms': first[1] if first else None})
        why = []
        if rec['exit'] != 0:
            why.append(f'exit {rec["exit"]}')
        miss = [n for n in need if n not in cs['est_ms']]
        if miss:
            why.append(f'no estimate for {len(miss)} of {len(need)} ids: {miss[:4]}')
        extra = sorted(set(cs['est_ms']) - set(need))
        if extra:
            why.append(f'{len(extra)} unexpected ids (the filter matched more): {extra[:4]}')
        rec['invalid'] = why
    rec['valid'] = not rec['invalid']
    if not TEST and (void_seen(receipt) or rec.get('build_proc_during') or rec.get('void_names_new_during')):
        rec['void'] = (f'build/lane process during/after the process: presence {receipt.get("presence")} '
                       f'busy {receipt.get("build_procs_busy")} during {rec.get("build_proc_during")} '
                       f'new {rec.get("void_names_new_during")}')
        return rec, receipt, 'void'
    return rec, receipt, 'ok'


def tag_of(rec):
    if 'aborted' in rec:
        return f'ABORTED ({rec["aborted"]})'
    m = rec.get('mean_ms')
    t = f'stat {m:.4f} ms' if m is not None else 'stat NONE'
    extra = ''
    if rec.get('summary'):
        extra = f' pose {rec["summary"].get("pose_hash")} expect {rec["summary"].get("expect_pose")}'
    elif (rec.get('jolt') or {}).get('stat_lines'):
        st = rec['jolt']['stat_lines'][0]
        extra = f' jolt hash {st["hash"]} threads {st["threads"]}' + (f' dumps {len(rec.get("profile") or {})}' if 'profile' in rec else '')
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
    return (f'{t}{extra} rb {rb}% ra {ra}% others {rec.get("others_busy_pct")}% cpu_s {rec.get("proc_cpu_s")} '
            f'wall {rec.get("wall_s")}s waited {rec.get("waited_s")}s {" ".join(flags)}').rstrip()


def cell_order(bname):
    out = []
    ids = [rid for rid, r in ROWS.items() if bname in r['blocks']]
    for w in (1, 2, 4, 8, 16):
        for rid in ids:
            if w not in ROWS[rid]['workers']:
                continue
            for key in ROWS[rid]['binaries']:
                out.append((rid, key, w))
    return out


def wait_idle():
    if TEST:
        return 0
    return subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', WAIT_PS1,
                           '-Log', WAIT_LOG, '-MaxPolls', str(IDLE_MAX_POLLS)]).returncode


def machine_state():
    rc, so, se = D.run_bytes(['powercfg', '/getactivescheme'])
    return {'time': D.now(), 'powercfg': D.decode(so).strip()}


def verify_binaries():
    bad = []
    for key, e in BINS.items():
        got = D.sha256(e['path']) if os.path.exists(e['path']) else 'MISSING'
        if got != e['sha256']:
            bad.append(f'{key} {e["path"]} {got} != {e["sha256"]}')
    return bad


def append(path, rec):
    with open(path, 'a', encoding='utf-8', newline='\n') as f:
        f.write(json.dumps(rec) + '\n')


class Cut(Exception):
    pass


class StopFlag(Exception):
    pass


def ensure_time(need_s, what):
    if os.path.exists(STOP_FLAG):
        raise StopFlag(what)
    if TEST and ARGS.test_cutoff_s is None:
        return
    if dt.datetime.now() + dt.timedelta(seconds=need_s) > CUTOFF:
        raise Cut(what)


def block_rounds(block):
    return block.get('rounds', PROTO['rounds_per_pass'])


def block_passes(block):
    return block.get('passes', PROTO['passes'])


def run_pass(block, p, attempt, runs_path, env, sampler, counters, state):
    bname = block['name']
    ptag = f'{bname}-p{p}' + (f'v{attempt}' if attempt else '')  # 'v<n>' without a dash: the padded rows keep fitting
    pdir = os.path.join(RAW, f'{ptag}_{RUN_TAG}')
    os.makedirs(pdir, exist_ok=True)
    order0 = cell_order(bname)
    order = order0 if p % 2 == 0 else order0[::-1]
    rounds = 1 if TEST else block_rounds(block)
    receipt = receipt_with_presence(PASS_RECEIPT_S)
    log(f'{ptag}: opening receipt {receipt["cpu_avg"]}% (build procs busy {receipt["build_procs_busy"]}, presence {receipt["presence"]})')
    if void_seen(receipt) and not TEST:
        return ('void', f'opening receipt presence {receipt["presence"]}')
    if block.get('warmup'):
        wr, wk, ww = block['warmup']
        ensure_time(est_s(wr, wk, ww) + RECEIPT_S, f'{ptag} warm-up')
        wrec, receipt, st = run_one(wr, wk, ww, block, p, attempt, -1, 0, 'warmup', pdir, receipt, env, sampler)
        append(runs_path, wrec)
        counters['warm'] += 1
        log(f'  {ptag} warm-up {wr}#{wk} W={ww} (untimed): {tag_of(wrec)}')
        if st in ('abort', 'void'):
            return (st, wrec.get('aborted') or wrec.get('void'))
    else:
        log(f'  {ptag}: no warm-up (block has none)')
    seq = 1
    bad_runs = []
    state['rounds_done'] = 0
    for rnd in range(rounds):
        for rid, key, w in order:
            ensure_time(est_s(rid, key, w) + RECEIPT_S, f'{ptag} r{rnd} {rid}#{key}@W{w}')
            rec, receipt, st = run_one(rid, key, w, block, p, attempt, rnd, seq, 'original', pdir, receipt, env, sampler)
            append(runs_path, rec)
            state['last'] = f'{ptag} r{rnd} {rid}#{key}@W{w}'
            if 'aborted' not in rec and 'mean_ms' in rec:
                counters['timed'] += 1
            log(f'  {ptag} r{rnd} [{seq:3d}] {rid}#{key} W={w}: {tag_of(rec)}')
            if st in ('abort', 'void'):
                return (st, rec.get('aborted') or rec.get('void'))
            if rec['contaminated'] or not rec['valid']:
                bad_runs.append((seq, rnd, rid, key, w, 'contaminated' if rec['contaminated'] else 'invalid'))
            seq += 1
        state['rounds_done'] = rnd + 1
    if bad_runs:
        log(f'{ptag}: re-running {len(bad_runs)} process(es) at the end of the pass: {bad_runs}')
    for oseq, rnd, rid, key, w, why in bad_runs:
        ensure_time(est_s(rid, key, w) + RECEIPT_S, f'{ptag} rerun {rid}#{key}@W{w}')
        rec, receipt, st = run_one(rid, key, w, block, p, attempt, rnd, oseq, 'rerun', pdir, receipt, env, sampler)
        rec['rerun_reason'] = why
        append(runs_path, rec)
        if 'aborted' not in rec and 'mean_ms' in rec:
            counters['timed'] += 1
            counters['rerun'] += 1
        log(f'  {ptag} r{rnd} [{oseq:3d}] {rid}#{key} W={w} RERUN ({why}): {tag_of(rec)}')
        if st in ('abort', 'void'):
            return (st, rec.get('aborted') or rec.get('void'))
    log(f'{ptag}: done')
    return 'ok'


# ------------------------------------------------------------------ schedule / dry run
def block_estimate(block):
    cells = cell_order(block['name'])
    per_round = sum(est_s(rid, key, w) + RECEIPT_S for rid, key, w in cells)
    warm = 0.0
    if block.get('warmup'):
        wr, wk, ww = block['warmup']
        warm = est_s(wr, wk, ww) + RECEIPT_S
    per_pass = IDLE_MIN_S + PASS_RECEIPT_S + warm + block_rounds(block) * per_round
    k = block_rounds(block) * block_passes(block)
    return {'cells': len(cells), 'k': k, 'processes': len(cells) * k, 'per_round_s': per_round,
            'per_pass_s': per_pass, 'block_s': block_passes(block) * per_pass}


def remaining_block_s(block, done):
    """The estimated seconds of the block's passes not yet done."""
    e = block_estimate(block)
    left = sum(1 for p in range(block_passes(block)) if f'{block["name"]}-p{p}' not in done)
    return left * e['per_pass_s']


def reserve_s(block, done):
    """The estimated time of every LATER block of a strictly higher priority (a smaller number), not yet done."""
    names = [b['name'] for b in BLOCKS]
    later = BLOCKS[names.index(block['name']) + 1:]
    return sum(remaining_block_s(b, done) for b in later if b['priority'] < block['priority'])


def layout_lines():
    """Dry run: the launch-context lengths each padded row will get (pass 0, round 0, original and re-run), beside the
    earlier window's lengths the row pins (rows7b.json 'cwd_len_source')."""
    out = []
    for block in BLOCKS:
        pdir = os.path.join(RAW, f'{block["name"]}-p0_{RUN_TAG}')
        for rid, key, w in cell_order(block['name']):
            row = ROWS[rid]
            if not row.get('cwd_len') or row['kind'] == 'criterion':
                continue
            for tag in ('original', 'rerun'):
                cwd, tgt, act = process_dir(pdir, 1, 0, rid, key, w, tag)
                args = runner_args(row, key, w, cwd) if row['kind'] == 'runner' else jolt_args(row, w)
                if row['kind'] == 'runner' and row.get('canary_of'):
                    args += ['--canary-frac', str(row.get('canary_frac', 0.05)), '--canary-ref-ns', '16683800']
                ref = (row.get('cmdline_ref') or {}).get(str(w))
                out.append(f'  {block["name"]} {rid}#{key}@W{w} {tag}: cwd {act} (target {tgt}{"" if act == tgt else " MISS"}); '
                           f'cmdline {sum(len(x) + 1 for x in args)} chars (earlier window: {ref})')
    return out


def print_schedule():
    t = dt.datetime.now()
    if ARGS.start:
        hh, mm = (int(x) for x in ARGS.start.split(':'))
        t = t.replace(hour=hh, minute=mm, second=0, microsecond=0)
    print(f'WINDOW 7 WAVE 2 DRY RUN: rows7b.json, {len(BLOCKS)} blocks, K per block = passes x rounds (default '
          f'{PROTO["passes"]} x {PROTO["rounds_per_pass"]}), cutoff {CUTOFF.strftime("%H:%M")}, '
          f'assumed start {t.strftime("%H:%M")}; STOP flag {STOP_FLAG} {"PRESENT" if os.path.exists(STOP_FLAG) else "absent"}')
    print(f'per pass: idle rule >= {IDLE_MIN_S:.0f} s (3 polls 60 s apart), opening receipt {PASS_RECEIPT_S:.0f} s, '
          f"one warm-up (if the block has one); per process: the gate's untimed wall + {LAUNCH_OVERHEAD_S} s launch + "
          f'{RECEIPT_S:.0f} s receipt')
    bad = verify_binaries()
    print(f'binaries: {"all match bin/SHA256SUMS" if not bad else bad}')
    missing = sorted({pose_path(r) for r in ROWS.values() if r['kind'] == 'runner' and not os.path.exists(pose_path(r))})
    print(f'fixtures: {"all present" if not missing else "MISSING " + ", ".join(missing)}')
    noest = sorted({f'{rid}#{k}@W{w}' for b in BLOCKS for rid, k, w in cell_order(b['name'])
                    if f'{rid}#{k}@W{w}' not in EST})
    print(f'estimates: {"every cell has an untimed gate wall" if not noest else "MISSING (0.01 s/step assumed) " + ", ".join(noest)}')
    done = set()
    if os.path.exists(PASSES_DONE) and ARGS.resume:
        done = {l.strip() for l in open(PASSES_DONE, encoding='utf-8') if l.strip()}
    total = 0.0
    per_item = {}
    for block in BLOCKS:
        e = block_estimate(block)
        need = remaining_block_s(block, done)
        res = reserve_s(block, done)
        start = t
        fits_with_reserve = start + dt.timedelta(seconds=need + res) <= CUTOFF
        if not fits_with_reserve and need > 0:
            print(f'\n[{block["item"]} prio {block["priority"]}] {block["name"]}: WOULD BE SKIPPED FOR PRIORITY at '
                  f'{start.strftime("%H:%M")} (needs {need / 60:.1f} min + {res / 60:.1f} min reserved for later '
                  f'higher-priority blocks)')
            continue
        t = t + dt.timedelta(seconds=need)
        total += need
        per_item[block['item']] = per_item.get(block['item'], 0.0) + need
        fits = 'fits' if t <= CUTOFF else ('PARTIAL (cut inside)' if start < CUTOFF else 'LEFT OUT (after the cutoff)')
        wtxt = f'+ {block_passes(block)} warm-ups' if block.get('warmup') else 'no warm-up'
        print(f'\n[{block["item"]} prio {block["priority"]}] {block["name"]}: {e["cells"]} cells x K {e["k"]} = '
              f'{e["processes"]} timed processes {wtxt}; {block_passes(block)} pass(es) x {block_rounds(block)} rounds; '
              f'per round {e["per_round_s"]:.0f} s; per pass {e["per_pass_s"] / 60:.1f} min; block {need / 60:.1f} min '
              f'(reserve after it {res / 60:.1f} min); {start.strftime("%H:%M")} -> {t.strftime("%H:%M")} {fits}')
        order0 = cell_order(block['name'])
        for p in range(block_passes(block)):
            order = order0 if p % 2 == 0 else order0[::-1]
            key = f'{block["name"]}-p{p}'
            skip = ' (DONE, skipped by --resume)' if key in done else ''
            wu = f'warm-up {"#".join(str(x) for x in block["warmup"])}' if block.get('warmup') else 'no warm-up'
            print(f'  pass {p}{skip}: idle rule; {wu}; each of {block_rounds(block)} rounds: '
                  + ', '.join(f'{rid}#{k}@W{w}({est_s(rid, k, w):.1f}s)' for rid, k, w in order))
    print('\nper item (minutes): ' + ', '.join(f'{k} {v / 60:.1f}' for k, v in per_item.items()))
    print(f'TOTAL {total / 60:.1f} min if every block runs; ends {t.strftime("%H:%M")}; the cutoff stops starting '
          f'processes at {CUTOFF.strftime("%H:%M")}')
    print('\nlaunch-context lengths of the padded rows:')
    for line in layout_lines():
        print(line)


# ------------------------------------------------------------------ main
def finish(status_code, status, state, sampler, counters):
    if sampler is not None:
        sampler.close()
    state['end'] = D.now()
    state['status'] = status
    state['exit'] = status_code
    state.update({k: counters[k] for k in counters})
    state['binaries_after'] = verify_binaries() or 'all match'
    log(f'WINDOW END exit {status_code}: {status}; counters {counters}; binaries after {state["binaries_after"]}')
    with open(os.path.join(RAW, 'window_state.json'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(state, f, indent=1, default=str)
    with open(DONE_FILE, 'w', encoding='utf-8', newline='\n') as f:
        f.write(f'exit {status_code}\n{status}\n{D.now()}\n')
    return status_code


def main():
    if DRY:
        print_schedule()
        return 0
    os.makedirs(RAW, exist_ok=True)
    if os.path.exists(DONE_FILE):
        os.replace(DONE_FILE, DONE_FILE + f'.prev-{int(time.time())}')
    from pdhperf import PerfSampler
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    manifest = {'generated': D.now(), 'rows_json': os.path.join(W6, 'rows7b.json'), 'binaries': BINS,
                'fixtures': FIXTURES, 'protocol': PROTO, 'blocks': BLOCKS, 'cutoff': CUTOFF.isoformat(),
                'test': TEST, 'resume': ARGS.resume}
    with open(os.path.join(RAW, f'manifest_{int(time.time())}.json'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(manifest, f, indent=1, default=str)
    state = {'start': D.now(), 'test': TEST, 'placement': 'P-none (no affinity call)', 'voids': [],
             'machine_before': machine_state(), 'last': None}
    log(f'WINDOW START cutoff {CUTOFF.isoformat()} blocks {[b["name"] for b in BLOCKS]} test {TEST} resume {ARGS.resume}')
    runs_path = os.path.join(RAW, 'runs.jsonl')
    if ARGS.resume:
        restore_ctx()
        log(f'--resume: canary references restored {sorted(CTX)}')
    sampler = PerfSampler(1.0)
    counters = {'timed': 0, 'rerun': 0, 'warm': 0, 'voided_processes': 0}
    done = set()
    if ARGS.resume and os.path.exists(PASSES_DONE):
        done = {l.strip() for l in open(PASSES_DONE, encoding='utf-8') if l.strip()}
    skipped = []
    for block in BLOCKS:
        bname = block['name']
        rows_here = [rid for rid, r in ROWS.items() if bname in r['blocks']]
        passes = 1 if TEST else block_passes(block)
        if not TEST or ARGS.test_cutoff_s is not None:  # a rehearsal exercises the priority skip only with a cutoff
            need = remaining_block_s(block, done)
            res = reserve_s(block, done)
            if need > 0 and dt.datetime.now() + dt.timedelta(seconds=need + res) > CUTOFF:
                skipped.append(bname)
                msg = (f'{bname} SKIPPED for priority: needs {need / 60:.1f} min + {res / 60:.1f} min reserved for the later '
                       f'higher-priority blocks, cutoff {CUTOFF.strftime("%H:%M")}')
                log(msg)
                progress(f'{block["item"]} {bname} SKIPPED for priority ({need / 60:.1f} + {res / 60:.1f} min > cutoff)')
                append(runs_path, {'block_skipped': True, 'run_tag': RUN_TAG, 'block': bname, 'why': msg, 'time': D.now()})
                continue
        for p in range(passes):
            pkey = f'{bname}-p{p}'
            if pkey in done:
                log(f'{pkey}: already done (--resume), skipped')
                continue
            attempt = 0
            while True:
                ptag = pkey + (f'v{attempt}' if attempt else '')
                try:
                    ensure_time((0.0 if TEST else IDLE_MIN_S) + PASS_RECEIPT_S, f'{ptag} idle rule')
                except Cut as c:
                    progress(f'{block["item"]} {bname} p{p} cut before the idle rule')
                    return finish(2, f'cut at {CUTOFF.strftime("%H:%M:%S")} after {state["last"] or "nothing"} (next: {c})', state,
                                  sampler, counters)
                except StopFlag as c:
                    progress(f'{block["item"]} {bname} p{p} STOP flag before the idle rule')
                    return finish(2, f'cut by the STOP flag after {state["last"] or "nothing"} (next: {c})', state,
                                  sampler, counters)
                log(f'{ptag}: idle rule (wait_idle7.ps1, up to {IDLE_MAX_POLLS} polls)')
                rc = wait_idle()
                if rc != 0:
                    progress(f'{block["item"]} {bname} p{p} STOP idle rule not met')
                    return finish(3, f'STOP before {ptag}: idle rule not met in {IDLE_MAX_POLLS} min (wait_idle exit {rc}); '
                                     f'last finished: {state["last"]}', state, sampler, counters)
                log(f'{ptag}: idle reached; {machine_state()}')
                bad = verify_binaries()
                if bad:
                    return finish(3, f'STOP before {ptag}: binaries changed {bad}', state, sampler, counters)
                timed_before = counters['timed']
                try:
                    res = run_pass(block, p, attempt, runs_path, env, sampler, counters, state)
                except Cut as c:
                    rd = state.get('rounds_done', 0)
                    for rid in rows_here:
                        progress(f'{block["item"]} {rid} p{p} cut ({rd} of {block_rounds(block)} rounds complete)')
                    return finish(2, f'cut at {CUTOFF.strftime("%H:%M:%S")} after {state["last"]} (next would have been {c})', state,
                                  sampler, counters)
                except StopFlag as c:
                    rd = state.get('rounds_done', 0)
                    for rid in rows_here:
                        progress(f'{block["item"]} {rid} p{p} STOP flag ({rd} of {block_rounds(block)} rounds complete)')
                    return finish(2, f'cut by the STOP flag after {state["last"]} (next would have been {c})', state,
                                  sampler, counters)
                if res == 'ok':
                    append(runs_path, {'pass_done': True, 'run_tag': RUN_TAG, 'block': bname, 'pass': p,
                                       'pass_attempt': attempt, 'time': D.now()})
                    for rid in rows_here:
                        progress(f'{block["item"]} {rid} p{p} done')
                    with open(PASSES_DONE, 'a', encoding='utf-8', newline='\n') as f:
                        f.write(pkey + '\n')
                    done.add(pkey)
                    break
                kind, why = res
                if kind == 'abort':
                    return finish(3, f'ABORT in {ptag}: {why}', state, sampler, counters)
                n_void = counters['timed'] - timed_before
                counters['voided_processes'] += n_void
                state['voids'].append({'block': bname, 'pass': p, 'attempt': attempt, 'time': D.now(), 'why': why,
                                       'timed_processes_voided': n_void})
                append(runs_path, {'voided_pass': True, 'run_tag': RUN_TAG, 'block': bname, 'pass': p, 'pass_attempt': attempt, 'why': why,
                                   'time': D.now()})
                progress(f'{block["item"]} {bname} p{p} VOIDED ({why[:160]}); re-running after idle')
                log(f'{ptag}: PASS VOIDED ({why}); {n_void} timed processes discarded; waiting for idle and re-running')
                attempt += 1
                if attempt > MAX_VOIDS_PER_PASS:
                    return finish(3, f'STOP in {pkey}: voided {attempt} times', state, sampler, counters)
    if skipped:
        return finish(2, f'complete except the blocks skipped for priority: {", ".join(skipped)}', state, sampler, counters)
    return finish(0, 'complete', state, sampler, counters)


if __name__ == '__main__':
    sys.exit(main())
