"""PHYSICS WINDOW 9a (2026-09-30) driver = window 8b's window8b_run.py (protocol verbatim) with window 9a's changes:
  * rows9a.json (+ the optional overlay rows9a.extra.json, merged by key as in 8b): the C4 blocks (C4-BR, C4-G4, C4-G4-kd,
    C4-AB with the in-block Jolt 5.6 row, C4-G5) and the reserved slot for the Rapier rows (kind 'rapier', PREP.md);
  * RULING 8 of RULINGS-2026-09-29-W8B (a K = 2 pass-cell does not gate; a dropped slot is re-run until its pass-cell has K =
    3 or the pass ends): the single end-of-pass re-run of 8b becomes a re-run STAGE of up to RERUN_MAX_WAVES waves inside a
    wall allowance (the "pass ends" bound; the cutoff and the STOP flag still end everything) of
    max(RERUN_STAGE_FRAC x the pass's own timed wall, RERUN_MAX_WAVES x RERUN_EST_SLACK x the block's longest process) - the
    second term is the audit-1 fix W1: a one-cell criterion block (C4-G4: 3 slots of 508 s per pass) got 1.5 processes out of
    the fraction alone, i.e. ONE re-run, so a slot whose first re-run was also hot was dropped (8b's behaviour, exactly what
    ruling 8 removes); now any single slot can reach the full wave depth. rerun_allowance / rerun_capacity are pure and are
    selftested against the real blocks. A wave re-runs every slot whose latest attempt is contaminated or invalid; a slot leaves the stage
    at its first clean valid attempt. Records: attempt 'rerun' (as in 8b) + rerun_no 1..; one {'passcell': ...} record per
    (pass, cell) with k_clean / k_target (no 'row' key, like the R4 records). The reduction's slot rule: the original if
    clean and valid, else the FIRST clean valid re-run, else the slot is dropped; a pass-cell with fewer than 3 clean
    slots does not gate;
  * binaries may carry expect_config {key: value} (checked against the runner SUMMARY's config on every process: an exe/key
    swap shows as tree_brute_max_rows 128 vs 144) besides sha256_pin;
  * kind 'rapier' (reserved): a runner-shaped external harness, data-driven validation (expect_summary dotted paths, "$W");
  * --test-rounds N (rehearsals of the re-run rule) and the test-only injection W9A_TEST_HOT.
Window 8b's own description follows.
PHYSICS WINDOW 8b (2026-09-28) driver = window 8's window8_run.py (the protocol verbatim) with window 8b's changes:
  * rows9a.json (+ the optional overlay rows9a.extra.json, merged by key: binaries, rows, blocks, fixtures) - the
    block list lives in the rows file, so a reserved block (S7-AB, omega_b v2) is added without a code change;
  * ruling 1 (RULINGS-2026-09-27-W8): K = 9 = three passes x three rounds, pass 0 REVERSED, pass 1 forward, pass 2
    REVERSED; a per-process PLACEMENT RECEIPT (recorded, never used to drop a process);
  * the 'criterion' kind (window 7b's Q3: CRITERION_HOME per process, every expected id and no other);
  * runner validation adds: the row's TreeDiag expectation, kd_order_builds > 0 exactly on leaflist-kd, the S4 setup
    counters by binary (armed rows: parent 0 at every W, tip 0 at W1 and > 0 at W >= 2), the J-Son-T freeze step;
  * R4 (tree-f3/window_cmds.md): after every round (and every re-run) the leaflist / leaflist-kd pose files of each
    (row pair, W, round) are compared byte for byte; a record {'r4': ...} goes to runs.jsonl;
  * DM1: the zones a row gates come from the row (ruling 9 adds VB_EARLY_CULL and VB_RUN), present_mode is checked.
  * 2026-09-29 (S7-AB / omega-v2): micro rows are validated by micro9a.py (window 8's expect_* checks + the row's
    void_rules: one CALIBRATION line, ranges, the park receipt kept as micro_notes for counts9a's per-cell rule); a
    binary key may carry sha256_pin (verify_binaries: a key/exe swap is red); --test gives omega-b2 --regions 10.
Window 8's own description follows.
PHYSICS WINDOW 8 (2026-09-27) driver: window 7 wave 2's window7b_run.py (verbatim protocol: P-none launch, 10-s
and 5-s receipts, presence snapshots, the VOID rule, the idle rule before every pass, re-run once at the end of the
pass, checkpoints, --resume, --cutoff hard stop, the STOP flag, exit 3 when the idle rule keeps failing) with:
  * rows8.json: blocks W8S-A -> W8S-R -> micro -> DM1 -> P-jolt56-prof (w8s-s4/cut.md §5 without S4-AB; dm1/cut.md §5
    as amended by §11.5);
  * the during-process witness GATED (window 7 FOLLOW-UP 8, design 01-DESIGN.md §7): others_busy_pct > 2 % counts as
    contaminated -> re-run once at the end of the pass (the reduction drops a slot whose re-run is also hot);
  * static canary arguments (the ladder, the rung and the zone canary carry fixed --canary-ref-ns / --canary-ns);
  * two new kinds: 'micro' (omega_b_region --bench: SUMMARY lines counted and route-checked) and 'dm1' (the windowed
    DM1 test binary under BOYKO_VB_ZONE=1 + BOYKO_VB_BENCH_FRAMES=220, validation off, BOYKO_PROFILE_ARTIFACT per
    process; ONE windowed process at a time; a process alive past 300 s is a hang and is terminated by ITS OWN handle);
  * a block may give an explicit cell 'order' (DM1: A B B A per (path, RES), then the 100-row edit, then 3x grow on B);
  * joltprof validates the lighter-profile banner, the dumps at frames 100..400 and the wfb csv (one job per thread
    per frame).
Every process: raw/<block>-p<n>_<runtag>/<seq>_r<k>_<row>_<bin>_W<w>[_R|_warmup]/ {stdout.txt, stderr.txt, run.csv |
per_frame csv + dumps + wfb csv | artifact.toml}; raw/runs.jsonl one record per process; progress.txt per pass;
raw/driver_done.txt -> WINDOW_DONE (run_window8.sh).
Reads no statistic to decide anything except the receipts; never builds; never sets RUSTFLAGS.
"""

import argparse
import ctypes
import datetime as dt
import json
import math
import os
import re
import statistics
import subprocess
import sys
import time
from ctypes import wintypes

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, os.path.join(HERE, 'window_lib'))
import driver as D  # noqa: E402  (window 3's tree driver; only helpers are used)

AP = argparse.ArgumentParser()
AP.add_argument('--dry-run', action='store_true')
AP.add_argument('--test', action='store_true', help='untimed control-flow rehearsal under test/: 12 steps, one round, one pass, no idle wait')
AP.add_argument('--resume', action='store_true')
AP.add_argument('--blocks', default='')
AP.add_argument('--cutoff', default='23:59')
AP.add_argument('--start', default='', help='dry-run only: the assumed start time HH:MM (default: now)')
AP.add_argument('--test-rounds', type=int, default=1, help='--test only: rounds per pass (default 1; 3 rehearses the re-run rule)')
ARGS = AP.parse_args()
TEST = ARGS.test
DRY = ARGS.dry_run

WAIT_PS1 = os.path.join(HERE, 'wait_idle9a.ps1')
RAW = os.path.join(W8, 'test', 'window') if TEST else os.path.join(W8, 'raw')
if TEST and os.environ.get('W9A_TEST_RAW'):    # rehearsals only (selftest9a's end-to-end re-run cases): a scratch raw dir
    RAW = os.environ['W9A_TEST_RAW']
STOP_FLAG = os.path.join(RAW, 'STOP') if TEST else os.path.join(W8, 'STOP')
WAIT_LOG = os.path.join(RAW, 'wait_log.txt') if TEST else os.path.join(W8, 'wait_log.txt')
PROGRESS = os.path.join(RAW, 'progress.txt') if TEST else os.path.join(W8, 'progress.txt')
DONE_FILE = os.path.join(RAW, 'driver_done.txt')
PASSES_DONE = os.path.join(RAW, 'passes_done.txt')
BIN = os.path.join(W8, 'bin')
GATE = os.path.join(W8, 'gate')
FIX = os.path.join(GATE, 'fixtures')
BUSY_PCT = 5.0
OTHERS_PCT = 2.0
RECEIPT_S = 0.5 if TEST else 5.0
PASS_RECEIPT_S = 0.5 if TEST else 10.0
QUIET_BUDGET_S = 420.0
IDLE_MAX_POLLS = 30
IDLE_MIN_S = 135.0          # three polls 60 s apart, each with a 10-s counter sample
LAUNCH_OVERHEAD_S = 0.4
MAX_VOIDS_PER_PASS = 20
# Ruling 8 (RULINGS-2026-09-29-W8B): the re-run stage of a pass. "The pass ends" = one of these bounds, whichever comes first:
# every pass-cell has all its slots clean; RERUN_MAX_WAVES waves have run; the stage has spent RERUN_STAGE_FRAC x the pass's
# own timed wall (rounds x per-round estimate; window 8b's 108 re-runs were 14 % of its 786 slots, so 0.5 is generous).
RERUN_MAX_WAVES = 4
RERUN_STAGE_FRAC = 0.5
# Audit-1 fix W1: the fraction alone quantises badly on a block with few slots per pass (C4-G4: 3 slots, 0.5 x 3 x 508 s = 1.5
# processes = ONE re-run). The allowance is max(fraction, floor) with floor = RERUN_MAX_WAVES x RERUN_EST_SLACK x the longest
# process of the block, so ANY single slot can reach the full wave depth. RERUN_EST_SLACK = 1.3 because the criterion cells'
# wall estimates are a model good to +-30 % (PREP.md, "The criterion cost"): a slower-than-estimated process must not take a
# wave away. Runner blocks keep the fraction (it is the larger term there).
RERUN_EST_SLACK = 1.3
# --dry-run prints the expected re-run time under window 8b's measured slot rate (108 re-runs of 786 slots); a MODEL, printed as such.
RERUN_MODEL_RATE = 108 / 786
MIN_K_GATE = 3            # a pass-cell with fewer clean slots than this does not gate (ruling 8, window 7's clause)
HANG_S_WINDOWED = 300.0
RUN_TAG = time.strftime('%H%M%S')

ROWS_PATH = os.path.join(W8, 'rows9a.json')
EXTRA_PATH = os.path.join(W8, 'rows9a.extra.json')
if (TEST or DRY) and os.environ.get('W9A_EXTRA'):   # rehearsals only: a scratch overlay, never honoured in the window
    EXTRA_PATH = os.environ['W9A_EXTRA']
INJECT = {}                 # --test only: W9A_TEST_HOT='row#bin@W:round:n;...' forces the first n attempts of a slot hot
if TEST and os.environ.get('W9A_TEST_HOT'):
    for _spec in os.environ['W9A_TEST_HOT'].split(';'):
        _cell, _rnd, _n = _spec.rsplit(':', 2)
        _rid, _rest = _cell.split('#')
        _key, _w = _rest.split('@W')
        INJECT[(_rid, _key, int(_w), int(_rnd))] = int(_n)


def load_rows():
    """rows9a.json, then the overlay rows9a.extra.json if it exists: binaries merge by key, rows replace by id or
    append, blocks replace by name or append (the merged block list is ordered by 'priority'). Pose fixtures are not
    part of the overlay: tools/gate9a.py records them into gate/fixtures/ (PREP.md, "Adding a block").
    A NEW overlay row may carry "insert_after": "<row id>" (audit-1 fix W2): it is inserted right after that row in the row
    list instead of appended. cell_order walks the rows in list order inside every W group, so this is how an overlay row
    chooses its neighbour in a shared block (an appended row always lands at the END of each W group)."""
    rj = json.load(open(ROWS_PATH, encoding='utf-8'))
    if os.path.exists(EXTRA_PATH):
        ex = json.load(open(EXTRA_PATH, encoding='utf-8'))
        rj['binaries'].update(ex.get('binaries', {}))
        for r in ex.get('rows', []):
            ids = {x['id']: i for i, x in enumerate(rj['rows'])}
            anchor = r.get('insert_after')
            if r['id'] in ids:
                if anchor:
                    sys.exit(f'overlay row {r["id"]}: insert_after only positions a NEW row (it replaces an existing row in place)')
                rj['rows'][ids[r['id']]] = r
            elif anchor:
                if anchor not in ids:
                    sys.exit(f'overlay row {r["id"]}: insert_after {anchor!r} is not a row id of rows9a.json / the overlay so far')
                rj['rows'].insert(ids[anchor] + 1, r)
            else:
                rj['rows'].append(r)
        names = {b['name']: i for i, b in enumerate(rj['blocks'])}
        for b in ex.get('blocks', []):
            if b['name'] in names:
                rj['blocks'][names[b['name']]] = b
            else:
                rj['blocks'].append(b)
        rj['extra_overlay'] = EXTRA_PATH
    rj['blocks'].sort(key=lambda b: b['priority'])   # window 9a: always by priority (8b sorted only with an overlay)
    return rj


ROWS_JSON = load_rows()
PROTO = ROWS_JSON['protocol']
ONLY = [b for b in ARGS.blocks.split(',') if b]
BLOCKS = [b for b in ROWS_JSON['blocks'] if not ONLY or b['name'] in ONLY]
ROWS = {r['id']: dict(r) for r in ROWS_JSON['rows']}
BINS = {}
for key, e in ROWS_JSON['binaries'].items():
    BINS[key] = dict(e)
    BINS[key]['path'] = (e['exe'].replace('/', os.sep) if ':' in e['exe'] else os.path.join(W8, e['exe'].replace('/', os.sep)))
for _r in ROWS.values():
    if _r['kind'] == 'runner' and not _r.get('pose_ref'):
        sys.exit(f'row {_r["id"]}: a runner row needs a pose_ref (the pose gate must not be optional)')
SUMS = {}
for line in open(os.path.join(BIN, 'SHA256SUMS'), encoding='utf-8'):
    if line.strip():
        sha, name = line.split(maxsplit=1)
        SUMS[name.strip().lstrip('*')] = sha
for key, e in BINS.items():
    e['sha256'] = SUMS.get(e['exe'])
    if e['sha256'] is None and (TEST or DRY):     # a rehearsal overlay may bring an exe that is not in bin/SHA256SUMS: its own pin
        e['sha256'] = e.get('sha256_pin')
EST = json.load(open(os.path.join(GATE, 'estimates.json'), encoding='utf-8')) if os.path.exists(os.path.join(GATE, 'estimates.json')) else {}
FIXTURES = json.load(open(os.path.join(FIX, 'fixtures.json'), encoding='utf-8')) if os.path.exists(os.path.join(FIX, 'fixtures.json')) else {}
TREE_KEYS = ('static_rebuilds', 'sleeper_rebuilds', 'evictions', 'translations', 'patches', 'hint_candidates',
             'wide_rows', 'excluded_rows', 'locator_resets', 'members')
COUNT_COLS = ('manifolds', 'pairs', 'awake', 'waves', 'colors', 'wide_colors', 'phys_np_pairs', 'phys_np_manifolds',
              'phys_np_points', 'phys_bp_pairs', 'phys_np_reused', 'phys_np_sep_hits', 'phys_np_full', 'phys_bp_queried',
              'phys_slots_wide', 'phys_slots_narrow', 'phys_np_chunks')
sys.path.insert(0, HERE)
import joltprof  # noqa: E402
import micro9a as M  # noqa: E402  (2026-09-29: the micro rows' validity rules, shared with the gate and counts9a)


def cutoff_dt():
    hh, mm = (int(x) for x in ARGS.cutoff.split(':'))
    now = dt.datetime.now()
    c = now.replace(hour=hh, minute=mm, second=0, microsecond=0)
    if c < now - dt.timedelta(hours=1):  # a cutoff after midnight
        c += dt.timedelta(days=1)
    return c


CUTOFF = cutoff_dt()


def est_s(rid, key, w):
    """Estimated wall of one process of the cell (the gate's untimed dry sample), plus the launch overhead."""
    v = EST.get(f'{rid}#{key}@W{w}')
    if v is None:
        row = ROWS[rid]
        v = {'dm1': 40.0, 'micro': 30.0, 'criterion': 900.0}.get(row['kind'], 0.01 * row.get('steps', 500))
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
class THREADENTRY32(ctypes.Structure):
    _fields_ = [('dwSize', wintypes.DWORD), ('cntUsage', wintypes.DWORD), ('th32ThreadID', wintypes.DWORD),
                ('th32OwnerProcessID', wintypes.DWORD), ('tpBasePri', ctypes.c_long), ('tpDeltaPri', ctypes.c_long),
                ('dwFlags', wintypes.DWORD)]


k32.Thread32First.argtypes = [wintypes.HANDLE, ctypes.POINTER(THREADENTRY32)]
k32.Thread32First.restype = wintypes.BOOL
k32.Thread32Next.argtypes = [wintypes.HANDLE, ctypes.POINTER(THREADENTRY32)]
k32.Thread32Next.restype = wintypes.BOOL
k32.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
k32.OpenThread.restype = wintypes.HANDLE
k32.GetThreadTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
k32.GetThreadTimes.restype = wintypes.BOOL
k32.QueryThreadCycleTime.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_ulonglong)]
k32.QueryThreadCycleTime.restype = wintypes.BOOL
k32.QueryProcessCycleTime.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_ulonglong)]
k32.QueryProcessCycleTime.restype = wintypes.BOOL
THREAD_QUERY_LIMITED_INFORMATION = 0x0800


def main_thread_handle(pid):
    """The thread a CREATE_SUSPENDED process was created with (it has no other yet): its id and a query handle kept
    open across the run, so GetThreadTimes still answers after the process exits."""
    snap = k32.CreateToolhelp32Snapshot(0x4, 0)  # TH32CS_SNAPTHREAD
    if not snap or snap == D.INVALID_HANDLE:
        return None, None
    tids = []
    try:
        e = THREADENTRY32()
        e.dwSize = ctypes.sizeof(THREADENTRY32)
        ok = k32.Thread32First(snap, ctypes.byref(e))
        while ok:
            if e.th32OwnerProcessID == pid:
                tids.append(e.th32ThreadID)
            ok = k32.Thread32Next(snap, ctypes.byref(e))
    finally:
        k32.CloseHandle(snap)
    if len(tids) != 1:
        return (tids or None), None
    h = k32.OpenThread(THREAD_QUERY_LIMITED_INFORMATION, False, tids[0])
    return tids[0], (h or None)


def placement_receipt(info, th, ph):
    """RECORDED, NEVER used to drop a process (ruling 1). Window 8's analysis read placement off the per-logical-CPU
    busy shares over the process lifetime (a fast W1 process sat 64-88 % on one CPU, a slow one <= 44 % spread over
    three): `top3` keeps that reading ([cpu, busy %] of the three busiest logical CPUs; the full vector stays in
    percpu_busy). Added, from the process's thread CPU times:
      main_cpu_s          GetThreadTimes of the thread the process was created with (the runner's dispatcher);
                          Windows charges it per clock tick (15.6 ms), so it can read 0 on a sub-second process;
      main_cycle_frac     QueryThreadCycleTime(main) / QueryProcessCycleTime - exact, the main thread's share of
                          the process's CPU;
      main_share_top_est  top CPU busy seconds / main_cpu_s, capped at 1 - the main thread's share on its top
                          logical CPU; exact at W1 on an idle machine, an upper bound at W >= 2 (workers share CPUs);
      top_share_of_proc   top CPU busy seconds / the process's CPU seconds, capped at 1."""
    pc = info.get('percpu_busy') or []
    ranked = sorted(((v, i) for i, v in enumerate(pc) if v is not None), reverse=True)
    out = {'method': 'percpu busy over the process lifetime + main-thread CPU (GetThreadTimes, QueryThreadCycleTime)',
           'top3': [[i, v] for v, i in ranked[:3]], 'main_tid': info.get('main_tid'), 'main_cpu_s': None,
           'main_cycles': None, 'proc_cycles': None, 'main_cycle_frac': None, 'main_share_top_est': None,
           'top_share_of_proc': None}
    if th:
        c, x, kk, u = (wintypes.FILETIME() for _ in range(4))
        if k32.GetThreadTimes(th, ctypes.byref(c), ctypes.byref(x), ctypes.byref(kk), ctypes.byref(u)):
            out['main_cpu_s'] = round(ft(kk) + ft(u), 4)
        cy = ctypes.c_ulonglong(0)
        if k32.QueryThreadCycleTime(th, ctypes.byref(cy)):
            out['main_cycles'] = cy.value
    cy = ctypes.c_ulonglong(0)
    if ph and k32.QueryProcessCycleTime(ph, ctypes.byref(cy)):
        out['proc_cycles'] = cy.value
    if out['main_cycles'] is not None and out['proc_cycles']:
        out['main_cycle_frac'] = round(out['main_cycles'] / out['proc_cycles'], 4)
    if ranked and info.get('wall_s'):
        top_s = ranked[0][0] / 100.0 * info['wall_s']
        out['top_cpu'] = ranked[0][1]
        out['top_busy_s'] = round(top_s, 4)
        if out['main_cpu_s']:
            out['main_share_top_est'] = round(min(1.0, top_s / out['main_cpu_s']), 3)
        if info.get('proc_cpu_s'):
            out['top_share_of_proc'] = round(min(1.0, top_s / info['proc_cpu_s']), 3)
    return out


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


def launch(cmd, cwd, env, sampler, hang_s):
    """P-none launch, identical to windows 3-7: CreateProcess suspended, read the (inherited) mask back, resume,
    wait. No SetProcessAffinityMask call is made. A process alive past hang_s is terminated through ITS OWN handle
    (never by image name) and recorded as a hang."""
    t_tab0 = D.process_cpu_table()
    p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         creationflags=CREATE_SUSPENDED)
    h = int(p._handle)
    info = {'pid': p.pid, 'mask_requested': None, 'hang': False}
    pm, sm = ctypes.c_size_t(), ctypes.c_size_t()
    k32.GetProcessAffinityMask(h, ctypes.byref(pm), ctypes.byref(sm))
    info['mask_readback'] = hex(pm.value)
    info['system_mask'] = hex(sm.value)
    info['main_tid'], th = main_thread_handle(p.pid)
    c0 = percpu()
    if sampler is not None:
        sampler.start()
    t0 = time.perf_counter()
    st = ntdll.NtResumeProcess(h)
    if st != 0:
        k32.TerminateProcess(h, 97)
        p.communicate()
        raise OSError(f'NtResumeProcess failed: {st:#x}')
    try:
        so, se = p.communicate(timeout=hang_s)
    except subprocess.TimeoutExpired:
        info['hang'] = True
        k32.TerminateProcess(h, 98)  # this process's own handle: never taskkill /IM
        so, se = p.communicate()
    info['wall_s'] = round(time.perf_counter() - t0, 3)
    c1 = percpu()
    perf = sampler.stop() if sampler is not None else []
    info['exit'] = p.returncode
    info['percpu_busy'] = percpu_busy(c0, c1)
    c, x, kk, u = (wintypes.FILETIME() for _ in range(4))
    if k32.GetProcessTimes(h, ctypes.byref(c), ctypes.byref(x), ctypes.byref(kk), ctypes.byref(u)):
        info['proc_cpu_s'] = round(ft(kk) + ft(u), 3)
    info['placement'] = placement_receipt(info, th, h)
    if th:
        k32.CloseHandle(th)
    t_tab1 = D.process_cpu_table()
    oth = others_cpu(t_tab0, t_tab1, {p.pid, MY_PID})
    info['others_cpu_s'] = round(sum(d for d, _, _ in oth), 3)
    info['others_busy_pct'] = round(100 * info['others_cpu_s'] / (info['wall_s'] * NCPU), 2) if info['wall_s'] else None
    info['others_top5'] = [{'name': n, 'pid': pid, 'cpu_s': d} for d, n, pid in oth[:5]]
    info['build_proc_during'] = sorted({n for d, n, pid in oth if n.lower() in VOID_NAMES})
    info['void_names_new_during'] = sorted({n for pid, (n, cpu) in t_tab1.items() if pid not in t_tab0 and n.lower() in VOID_NAMES})
    info['perf'] = [{'t': t, 'total': v.get('_Total')} for t, v in perf]
    return info, so, se


# ------------------------------------------------------------------ per-kind arguments, statistics, validation
def steps_of(row):
    return 12 if TEST else row['steps']


def window_of(row):
    return (0, 12) if TEST else tuple(row['window'])


def pose_path(row):
    return os.path.join(FIX, f'{row["pose_ref"]}.pose')


def runner_args(row, key, w, cwd):
    win = window_of(row)
    args = list(row['args'])
    args += ['--workers', str(w), '--steps', str(steps_of(row)), '--window', f'{win[0]}..{win[1]}',
             '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
             '--label', f'{row["id"]}#{key}@W{w}']
    if row['armed']:
        args.append('--arm-profiler')
    if not TEST and row.get('pose_ref'):     # a 'runner' row without a pose_ref is refused at load; 'rapier' rows may omit it
        args += ['--expect-pose', pose_path(row)]
    return args


def process_dir(pdir, seq, rnd, rid, key, w, tag, rerun_no=0):
    leaf = f'{seq:03d}_r{rnd}_{rid}_{key}_W{w}'
    if tag == 'rerun':
        leaf += '_R' if rerun_no <= 1 else f'_R{rerun_no}'   # ruling 8: a slot can be re-run more than once
    elif tag != 'original':
        leaf += f'_{tag}'
    return os.path.join(pdir, leaf)


def col_stats(cols, lo, hi):
    out = {}
    for name, col in cols.items():
        if not (name == 'wall_ns' or name.endswith('_ns') or name in COUNT_COLS):
            continue
        v = [x for x in col[lo:hi] if x is not None]
        if v:
            out[name] = {'mean': statistics.mean(v), 'median': statistics.median(v)}
    return out


def runner_stats(row, cwd):
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
    wins = [list(win)] + ([] if TEST else [list(m) for m in row.get('metric_windows', [])])
    res['cols'] = {f'{a}..{b}': col_stats(c, a, b) for a, b in wins}
    return res


def validate_runner(rec, row, w):
    why = validate_runner_w8(rec, row, w)
    s = rec.get('summary') or {}
    bt = s.get('broadphase_tree') if isinstance(s.get('broadphase_tree'), dict) else {}
    if row.get('bp_kernel') is not None or row['broadphase'] == 'Tree':
        kd = bt.get('kd_order_builds')
        if row.get('bp_kernel') == 'leaflist-kd':
            if not kd:
                why.append(f'kd_order_builds {kd} on leaflist-kd')
        elif kd:
            why.append(f'kd_order_builds {kd} on {row.get("bp_kernel") or "the default kernel"}')
    # Window 9a: the binary's own config receipt (an exe/key swap shows here on EVERY row: tip 128, parent 144).
    cfg = s.get('config') if isinstance(s.get('config'), dict) else {}
    for k, v in (BINS[rec['binary']].get('expect_config') or {}).items():
        if cfg.get(k) != v:
            why.append(f'config.{k} {cfg.get(k)} != {v} expected of binary {rec["binary"]}')
    w8 = s.get('w8s') if isinstance(s.get('w8s'), dict) else None
    s4 = BINS[rec['binary']].get('s4')
    if row['armed'] and w8 is not None and s4:
        st, sk = w8.get('setup_steps'), w8.get('setup_tasks')
        if s4 == 'off' and (st, sk) != (0, 0):
            why.append(f'S4 off but setup_steps/tasks {st}/{sk}')
        if s4 == 'on' and w == 1 and (st, sk) != (0, 0):
            why.append(f'S4 at W1 must be inline, setup_steps/tasks {st}/{sk}')
        if s4 == 'on' and w >= 2 and not (sk or 0) > 0 and not TEST:
            why.append(f'S4 on at W{w} but setup_tasks {sk}')
    ef = row.get('expect_first_frozen')
    if ef is not None and not TEST and s.get('first_frozen_step') != ef:
        why.append(f'first_frozen_step {s.get("first_frozen_step")} != the gate\'s {ef}')
    return why


def validate_runner_w8(rec, row, w):
    """Window 8's runner validation; the TreeDiag rule reads the row's own expectation (tree_diag_expect)."""
    why = []
    if rec.get('hang'):
        why.append('hang (terminated)')
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
    if not TEST:
        want = FIXTURES.get(row['pose_ref'], {}).get('hash')
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
    if row['armed'] and not s.get('w8s'):
        why.append('armed but no w8s object')
    cfg = s.get('config') or {}
    if isinstance(cfg, dict) and cfg.get('broadphase') != row['broadphase']:
        why.append(f'broadphase {cfg.get("broadphase")} != {row["broadphase"]}')
    bt = s.get('broadphase_tree')
    if isinstance(bt, dict) and not TEST:
        if row['broadphase'] == 'Tree':
            exp = row.get('tree_diag_expect') or {'static_rebuilds': 1, 'members': 1, 'evictions': 0}
            if any(bt.get(k) != v for k, v in exp.items()):
                why.append(f'TreeDiag {bt} != {exp}')
        elif any(bt.get(k) != 0 for k in TREE_KEYS):
            why.append(f'TreeDiag nonzero on {row["broadphase"]} {bt}')
    if row.get('canary') and not s.get('canary_ns'):
        why.append(f'canary_ns {s.get("canary_ns")}: the canary did not run')
    zc = row.get('zone_canary')
    if zc and (s.get('canary_zone') != zc[0] or s.get('canary_zone_ns') != zc[1]):
        why.append(f'canary_zone {s.get("canary_zone")}/{s.get("canary_zone_ns")} != {zc}')
    return why


def dotted(d, path):
    """d['a']['b'] for 'a.b'; None when any step is missing (a missing key never equals an expectation)."""
    for part in path.split('.'):
        if not isinstance(d, dict) or part not in d:
            return None
        d = d[part]
    return d


def validate_rapier(rec, row, w):
    """RESERVED (window 9a, PREP.md "Adding the Rapier rows"): a runner-shaped external harness (D:/tmp/rapier-parity: --workers
    --steps --window --csv --pose-out --expect-pose --label, one SUMMARY line, exit 0 / 2 bad flag / 3 void / 4 pose). Data-driven:
    the row names what it gates - `expect_summary` {dotted path: value; the string "$W" means the row's W}, `expect_present`
    [dotted paths that must exist], `pose_ref` (a fixture under gate/fixtures, passed as --expect-pose, pose_hash equal to
    the fixture's hash). Nothing is assumed about Rapier's summary beyond `pose_hash`."""
    why = []
    if rec.get('hang'):
        why.append('hang (terminated)')
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    if rec.get('mean_ms') is None:
        why.append('no window mean')
    elif rec.get('n_steps') != rec.get('expect_steps'):
        why.append(f'{rec.get("n_steps")} steps in the window, expected {rec.get("expect_steps")}')
    s = rec.get('summary')
    if not s:
        return why + ['no SUMMARY']
    if row.get('pose_ref') and not TEST:
        want = FIXTURES.get(row['pose_ref'], {}).get('hash')
        if want is None or s.get('pose_hash') != want:
            why.append(f'pose {s.get("pose_hash")} != {want}')
    for path, want in (row.get('expect_summary') or {}).items():
        want = w if want == '$W' else want
        got = dotted(s, path)
        if got != want:
            why.append(f'summary.{path} {got!r} != {want!r}')
    for path in row.get('expect_present') or []:
        if dotted(s, path) is None:
            why.append(f'summary.{path} missing')
    return why


def jolt_args(row, w):
    return list(row['args']) + [f'-t={w}', f'-i={steps_of(row)}']


def jolt_stats(row, cwd, so):
    """Window 3's Jolt statistic (per_frame csv 'Time (ms)', the harness's own clock) over the window and the metric
    windows; for the profiled build, every profile dump's per-scope numbers, and the wfb csv's per-frame sums."""
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
    wins = [list(win)] + ([] if TEST else [list(m) for m in row.get('metric_windows', [])])
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
            prof[str(it)] = {'threads': len(p['threads']), 'cycles_per_second': p['cycles_per_second'],
                             'frame_ms': (wall[it] / 1e6 if it < len(wall) and wall[it] else None),
                             'scopes': {nm: [v['calls'], round(v['cpu_ms'], 6), round(v['wall_ms'], 6), v['depth_min']]
                                        for nm, v in p['scopes'].items()}}
        res['profile'] = prof
        wf = [f for f in files if f.startswith('wfb_')]
        if wf:
            per = {}
            for line in open(os.path.join(cwd, wf[0]), encoding='utf-8').read().splitlines()[1:]:
                fr, sl, t, e, j = (int(x) for x in line.split(','))
                d = per.setdefault(fr, {'ticks': 0, 'episodes': 0, 'jobs': 0, 'slots': 0})
                d['ticks'] += t
                d['episodes'] += e
                d['jobs'] += j
                d['slots'] += 1
            res['wfb'] = {'frames': len(per), 'jobs_per_frame': sorted({d['jobs'] for d in per.values()}),
                          'per_frame': {str(k): v for k, v in sorted(per.items())}}
        m = re.search(r'tsc ticks per second ([\d.]+)', D.decode(so))
        res['tsc_ticks_per_s'] = float(m.group(1)) if m else None
    return res


def validate_jolt(rec, row, w):
    why = []
    if rec.get('hang'):
        why.append('hang (terminated)')
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
        if not TEST and st[0]['hash'] != row['jolt_hash']:
            why.append(f'hash {st[0]["hash"]} != {row["jolt_hash"]}')
    pl = j.get('patch_line') or ''
    if not pl.startswith('boyko-parity-patch v1') or 'receipt=0' not in pl or 'allow_sleep=0' not in pl:
        why.append(f'patch banner {pl!r}')
    if row['kind'] == 'joltprof' and not TEST:
        got = sorted(int(k) for k, v in (rec.get('profile') or {}).items() if 'scopes' in v)
        need = [i for i in row.get('profile_frames', []) if i < steps_of(row)]
        if [i for i in need if i not in got]:
            why.append(f'profile dumps {got}, need {need}')
        if '-wfb' in row['args']:
            wfb = rec.get('wfb') or {}
            if wfb.get('frames') != steps_of(row) or wfb.get('jobs_per_frame') != [w]:
                why.append(f'wfb frames {wfb.get("frames")} jobs/frame {wfb.get("jobs_per_frame")}')
    return why


CRIT_TIME = re.compile(r'time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]')
CRIT_UNIT = {'ps': 1e-9, 'ns': 1e-6, 'us': 1e-3, 'ms': 1.0, 's': 1e3}


def crit_unit_ms(u):
    # criterion prints U+00B5 (micro sign) for microseconds; accept U+03BC, a mangled byte pair or 'u' too
    if u in CRIT_UNIT:
        return CRIT_UNIT[u]
    if u.endswith('s') and len(u) >= 2 and u[-2] in ('µ', 'μ', '�', 'u'):
        return 1e-3
    return float('nan')


def criterion_stats(so, se, cwd):
    """Window 7b's reader (the 'time:' triple per id, recorded for the analyst; the driver decides nothing on it) plus
    a structural census: the ids criterion wrote a new/estimates.json for under this process's CRITERION_HOME."""
    res = {'est_ms': {}, 'ids_on_disk': []}
    last = None
    for line in (D.decode(so) + '\n' + D.decode(se)).splitlines():
        s = line.strip()
        if s.startswith('bp_g4_') and ':' not in s:
            last = s.split()[0]
        m = CRIT_TIME.search(line)
        if m:
            name = s.split()[0] if s.startswith('bp_g4_') else last
            if name:
                res['est_ms'][name] = [float(m.group(i)) * crit_unit_ms(m.group(i + 1)) for i in (1, 3, 5)]
    home = os.path.join(cwd, 'criterion')
    for root, dirs, files in os.walk(home):
        if 'benchmark.json' in files and os.path.basename(root) == 'new':
            try:
                bj = json.load(open(os.path.join(root, 'benchmark.json'), encoding='utf-8'))
                res['ids_on_disk'].append(bj.get('full_id') or bj.get('title'))
            except (OSError, ValueError):
                res['ids_on_disk'].append(f'UNREADABLE {root}')
    res['ids_on_disk'].sort()
    return res


def criterion_args(row):
    """The row's arguments; the --test rehearsal runs one id for 0.1 s + 0.2 s (window 7b's short form)."""
    if TEST:
        return ['--bench', '--noplot', '--warm-up-time', '0.1', '--measurement-time', '0.2', row['test_filter']]
    return list(row['args'])


def criterion_check(rec, row, so, se, cwd):
    cs = criterion_stats(so, se, cwd)
    need = list(row.get('test_expect') or []) if TEST else list(row.get('expect') or [])
    rec['criterion'] = cs
    rec['mean_ms'] = None
    why = []
    if rec.get('hang'):
        why.append('hang (terminated)')
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    for label, got in (('stdout', set(cs['est_ms'])), ('CRITERION_HOME', set(cs['ids_on_disk']))):
        miss = [n for n in need if n not in got]
        if miss:
            why.append(f'{label}: no result for {len(miss)} of {len(need)} ids: {miss[:4]}')
        extra = sorted(got - set(need))
        if extra:
            why.append(f'{label}: {len(extra)} unexpected ids (the filter matched more): {extra[:4]}')
    return why


def micro_summaries(so):
    out = []
    for line in D.decode(so).splitlines():
        if line.startswith('SUMMARY '):
            try:
                out.append(json.loads(line[8:]))
            except ValueError:
                out.append({'unparsed': line})
    return out


def validate_micro(rec, row, so):
    """A micro process's invalid reasons (window 8's checks + the row's void_rules, micro9a.process_rules); stores
    the SUMMARY / CALIBRATION lines and the park receipt (micro_notes) on the record for counts9a's per-cell rule."""
    ss, cal = M.parse(D.decode(so))
    rec['summaries'] = ss
    rec['calibration'] = cal
    rec['mean_ms'] = None
    why = []
    if rec.get('hang'):
        why.append('hang (terminated)')
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    rules, notes = M.process_rules(row, ss, cal, TEST)
    if notes:
        rec['micro_notes'] = notes
    return why + rules


DM1_LINE = re.compile(r'DM1 timing: path=(\w+) res=(\d+)x(\d+) edit_rows=(\d+) grow_at=(\S+) frames=(\d+)')


def dm1_check(rec, row, so, se, cwd):
    txt = D.decode(so) + '\n' + D.decode(se)
    why = []
    if rec.get('hang'):
        why.append(f'hang: alive past {HANG_S_WINDOWED:.0f} s, terminated by its own handle')
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    if 'NOT MEASURED' in txt:
        rec['not_measured'] = True
        why.append('NOT MEASURED: the window came up at another size (a clamped display)')
    m = DM1_LINE.search(txt)
    rec['dm1_line'] = m.group(0) if m else None
    if not m:
        why.append('no "DM1 timing:" line')
    else:
        path = {'vb': 'VisibilityBuffer', 'deferred': 'Deferred'}[row['path']]
        w, h = row['res'].split('x')
        if m.group(1) != path or m.group(2) != w or m.group(3) != h or int(m.group(4)) != row['edit_rows']:
            why.append(f'DM1 line {m.group(0)!r} does not match the row')
        rec['frames'] = int(m.group(6))
    if not re.search(r'test result: ok\. 1 passed', txt):
        why.append('libtest did not report 1 passed')
    art = os.path.join(cwd, 'artifact.toml')
    rec['artifact_bytes'] = os.path.getsize(art) if os.path.exists(art) else None
    if not rec['artifact_bytes']:
        why.append('no BOYKO_PROFILE_ARTIFACT file')
    else:
        # The zones the DM1 A/B reads, by id (gpu_zone.rs at cad5439b): ZONE_VB_SHADE 2 and ZONE_VB_PRODUCE_NET 14
        # (vb_resolve), ZONE_GBUF_DEFERRED_RESOLVE 17 (deferred_pbr); each must carry one sample per bench frame.
        txt_a = open(art, encoding='utf-8', errors='replace').read()
        zones = {int(i): int(n) for i, n in re.findall(r'\[\[zone\]\]\s*\nid = (\d+)\s*\nlabel = "[^"]*"\s*\nn = (\d+)', txt_a)}
        rec['artifact_zone_n'] = zones
        # Window 8b: the row names its gated zones (ruling 9 adds VB_EARLY_CULL 4 and VB_RUN 9 on the vb path).
        need = sorted((row.get('need_zones') or {'vb': {'VB_SHADE': 2, 'VB_PRODUCE_NET': 14},
                                                 'deferred': {'GBUF_DEFERRED_RESOLVE': 17}}[row['path']]).values())
        frames = 10 if TEST and row.get('grow_at') is None else int(row['env']['BOYKO_VB_BENCH_FRAMES'])
        miss = [z for z in need if zones.get(z) != frames]
        if miss:
            why.append(f'artifact zones {miss} missing or n != {frames} (zones {zones})')
        pm = re.search(r'^present_mode = "([^"]*)"', txt_a, re.M)
        rec['present_mode'] = pm.group(1) if pm else None
        if row.get('present_mode') and rec['present_mode'] != row['present_mode']:
            why.append(f'present_mode {rec["present_mode"]} != {row["present_mode"]}')
    return why


def run_one(rid, key, w, block, pass_no, attempt_no, rnd, seq, tag, pdir, receipt, env, sampler, rerun_no=0):
    row = ROWS[rid]
    b = BINS[key]
    cwd = process_dir(pdir, seq, rnd, rid, key, w, tag, rerun_no)
    os.makedirs(cwd, exist_ok=True)
    rec = {'run_tag': RUN_TAG, 'block': block['name'], 'item': block['item'], 'pass': pass_no, 'pass_attempt': attempt_no,
           'round': rnd, 'seq': seq, 'row': rid, 'kind': row['kind'], 'role': key, 'binary': key, 'W': w, 'dry': TEST,
           'attempt': tag, 'rerun_no': rerun_no, 'policy': 'P-none', 'timed': tag != 'warmup'}
    penv = dict(env)
    if row['kind'] in ('runner', 'rapier'):
        args = runner_args(row, key, w, cwd)
    elif row['kind'] in ('jolt', 'joltprof'):
        args = jolt_args(row, w)
    elif row['kind'] == 'dm1':
        args = list(row['args'])
        penv.update(row['env'])
        penv['BOYKO_PROFILE_ARTIFACT'] = os.path.join(cwd, 'artifact.toml')
        if TEST and row.get('grow_at') is None:  # a grow row keeps its frames: the run must reach the grow frame
            penv['BOYKO_VB_BENCH_FRAMES'] = '10'
    elif row['kind'] == 'criterion':
        args = criterion_args(row)
        penv['CRITERION_HOME'] = os.path.join(cwd, 'criterion')
    else:
        args = list(row['args'])
        if TEST:  # omega-b2 (v2) takes --regions like omega-b; only v1's omega mode takes --reps
            args += ['--regions', '10'] if ('omega-b' in args or 'omega-b2' in args) else ['--reps', '10']
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
    extra_keys = sorted(row.get('env', {})) + {'dm1': ['BOYKO_PROFILE_ARTIFACT'], 'criterion': ['CRITERION_HOME']}.get(row['kind'], [])
    rec.update({'exe': b['path'], 'exe_sha256': b['sha256'], 'commit': b['commit'], 'args': args, 'cwd': cwd,
                'env_extra': {k: penv[k] for k in extra_keys}, 'receipt_before': receipt})
    rec['start'] = D.now()
    hang_s = HANG_S_WINDOWED if row['kind'] == 'dm1' else max(600.0, 10 * est_s(rid, key, w))
    info, so, se = launch([b['path']] + args, cwd, penv, sampler, hang_s)
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
    rec['contaminated_witness'] = (rec.get('others_busy_pct') or 0) > OTHERS_PCT
    rec['contaminated'] = (rec['contaminated_before'] or rec['contaminated_after'] or rec['contaminated_during']
                           or rec['contaminated_witness'])
    if row['kind'] == 'runner':
        rec['summary'] = D.parse_summary(so)
        rec.update(runner_stats(row, cwd))
        rec['invalid'] = validate_runner(rec, row, w)
    elif row['kind'] == 'rapier':
        rec['summary'] = D.parse_summary(so)
        rec.update(runner_stats(row, cwd))
        rec['invalid'] = validate_rapier(rec, row, w)
    elif row['kind'] in ('jolt', 'joltprof'):
        rec.update(jolt_stats(row, cwd, so))
        rec['invalid'] = validate_jolt(rec, row, w)
    elif row['kind'] == 'micro':
        # Data-driven for the reserved omega_b rows: every check applies only when the row names it (micro9a.py).
        rec['invalid'] = validate_micro(rec, row, so)
    elif row['kind'] == 'criterion':
        rec['invalid'] = criterion_check(rec, row, so, se, cwd)
    else:  # dm1
        rec['mean_ms'] = None
        rec['invalid'] = dm1_check(rec, row, so, se, cwd)
    rec['valid'] = not rec['invalid']
    if not TEST and (void_seen(receipt) or rec.get('build_proc_during') or rec.get('void_names_new_during')):
        rec['void'] = (f'build/lane process during/after the process: presence {receipt.get("presence")} '
                       f'busy {receipt.get("build_procs_busy")} during {rec.get("build_proc_during")} '
                       f'new {rec.get("void_names_new_during")}')
        return rec, receipt, 'void'
    return rec, receipt, 'ok'


def tag_of(rec):
    """The one-line log of a process. It names validity and receipts; the statistic itself is not printed."""
    if 'aborted' in rec:
        return f'ABORTED ({rec["aborted"]})'
    extra = ''
    if rec.get('summary'):
        extra = f'pose {rec["summary"].get("pose_hash")} expect {rec["summary"].get("expect_pose")}'
    elif (rec.get('jolt') or {}).get('stat_lines'):
        st = rec['jolt']['stat_lines'][0]
        extra = f'jolt hash {st["hash"]} threads {st["threads"]}' + (f' dumps {len(rec.get("profile") or {})}' if 'profile' in rec else '')
    elif rec.get('kind') == 'micro':
        extra = f'summaries {len(rec.get("summaries") or [])}'
    elif rec.get('kind') == 'dm1':
        extra = f'{rec.get("dm1_line")} artifact {rec.get("artifact_bytes")} B present {rec.get("present_mode")}'
    elif rec.get('kind') == 'criterion':
        cs = rec.get('criterion') or {}
        extra = f'criterion ids stdout {len(cs.get("est_ms") or {})} on disk {len(cs.get("ids_on_disk") or [])}'
    flags = []
    if rec.get('contaminated_before'):
        flags.append(f'CONTAMINATED-before({rec["receipt_before"]["cpu_avg"]}%)')
    if rec.get('contaminated_after'):
        flags.append(f'CONTAMINATED-after({rec["receipt_after"]["cpu_avg"]}%)')
    if rec.get('contaminated_during'):
        flags.append(f'BUILD-PROC-DURING({rec["build_proc_during"]})')
    if rec.get('contaminated_witness'):
        flags.append(f'OTHERS-BUSY({rec.get("others_busy_pct")}%)')
    if rec.get('invalid'):
        flags.append(f'INVALID({"; ".join(rec["invalid"])})')
    if rec.get('void'):
        flags.append(f'VOID({rec["void"]})')
    rb = rec.get('receipt_before', {}).get('cpu_avg')
    ra = rec.get('receipt_after', {}).get('cpu_avg') if rec.get('receipt_after') else None
    return (f'{extra} rb {rb}% ra {ra}% others {rec.get("others_busy_pct")}% exit {rec.get("exit")} '
            f'waited {rec.get("waited_s")}s {" ".join(flags)}').rstrip()


def cell_order(bname):
    block = next(b for b in ROWS_JSON['blocks'] if b['name'] == bname)
    if block.get('order'):
        return [tuple(x) for x in block['order']]
    out = []
    ids = [rid for rid, r in ROWS.items() if bname in r['blocks']]
    for w in (0, 1, 2, 4, 8, 16):
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
        # 2026-09-29: a key labelled with its exe's sha256 (rows9a.extra.json sha256_pin). SHA256SUMS is keyed by
        # path, so it cannot see two keys' exe paths swapped; the pin can.
        if e.get('sha256_pin') and got != e['sha256_pin']:
            bad.append(f'{key} {e["path"]} {got} != its pinned sha256 {e["sha256_pin"]} (a key/exe swap)')
    return bad


def verify_fixtures():
    """Every pose_ref used by a row of a selected block names a recorded fixture: the .pose file exists and fixtures.json has
    its hash (a missing hash would make the driver's pose check compare against None)."""
    bad = []
    used = sorted({ROWS[rid]['pose_ref'] for b in BLOCKS for rid, _, _ in cell_order(b['name']) if ROWS[rid].get('pose_ref')})
    for ref in used:
        if not os.path.exists(os.path.join(FIX, f'{ref}.pose')):
            bad.append(f'{ref}.pose missing')
        if not (FIXTURES.get(ref) or {}).get('hash'):
            bad.append(f'{ref} has no hash in fixtures.json')
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
    if TEST:
        return
    if dt.datetime.now() + dt.timedelta(seconds=need_s) > CUTOFF:
        raise Cut(what)


def block_rounds(block):
    return block.get('rounds', PROTO['rounds_per_pass'])


def pass_order(order0, p, block=None):
    """Ruling 1: pass 0 REVERSED, pass 1 forward, pass 2 REVERSED (window 8 ran pass 0 forward). A block with an
    explicit 'order' (DM1's ABBA x 2, one pass) runs as written."""
    if block is not None and block.get('order'):
        return order0
    return order0[::-1] if p % 2 == 0 else order0


def r4_compare(block, p, attempt, rnd, recs, runs_path):
    """tree-f3/window_cmds.md R4: per (row pair, W, round), the leaflist-kd pose file equals the leaflist one. `recs`
    maps (rid, key, w) -> the latest record of that cell in this round. Recorded; a mismatch is logged as a defect."""
    out = []
    for (rid, key, w), rec in sorted(recs.items()):
        twin = ROWS[rid].get('r4_twin')
        if not twin:
            continue
        other = recs.get((twin, key, w))
        pa = os.path.join(rec.get('cwd') or '', 'pose.bin')
        pb = os.path.join((other or {}).get('cwd') or '', 'pose.bin')
        if other is None or not os.path.exists(pa) or not os.path.exists(pb):
            verdict = 'missing'
        else:
            verdict = 'equal' if open(pa, 'rb').read() == open(pb, 'rb').read() else 'DIFFER'
        # Keys are r4_-prefixed on purpose: every process selector in this campaign's tools is `'row' in r`.
        r = {'r4': True, 'run_tag': RUN_TAG, 'block': block['name'], 'pass': p, 'pass_attempt': attempt, 'round': rnd,
             'r4_row': rid, 'r4_twin': twin, 'r4_binary': key, 'r4_W': w, 'verdict': verdict,
             'r4_attempts': [rec.get('attempt'), (other or {}).get('attempt')], 'time': D.now()}
        append(runs_path, r)
        out.append(r)
        if verdict != 'equal':
            log(f'  R4 {block["name"]}-p{p} r{rnd} {rid} vs {twin} W={w}: {verdict} (a mismatch voids the F3 block in the analysis)')
    return out


def block_passes(block):
    return block.get('passes', PROTO['passes'])


def slot_bad(rec):
    """None when the attempt is clean and valid, else its reason ('contaminated' before 'invalid', as in window 8b)."""
    if rec.get('contaminated'):
        return 'contaminated'
    if not rec.get('valid'):
        return 'invalid'
    return None


def pending_reruns(slots):
    """The slots the next re-run wave takes: those whose LATEST attempt is not clean and valid, in original order (pure)."""
    return sorted(((k, s) for k, s in slots.items() if slot_bad(s['rec'])), key=lambda kv: kv[1]['seq'])


def passcell_table(slots):
    """(row, binary, W) -> the pass-cell's slot census: target = slots scheduled, clean = slots whose latest attempt is clean
    and valid (K), reruns = re-runs spent, unclean_rounds = the rounds still unclean (pure)."""
    cells = {}
    for (rnd, rid, key, w), s in slots.items():
        c = cells.setdefault((rid, key, w), {'target': 0, 'clean': 0, 'reruns': 0, 'unclean_rounds': []})
        c['target'] += 1
        c['reruns'] += s['n_rerun']
        if slot_bad(s['rec']) is None:
            c['clean'] += 1
        else:
            c['unclean_rounds'].append(rnd)
    return cells


def inject_hot(rec, rid, key, w, rnd, attempt_no):
    """--test only (W9A_TEST_HOT): the first n attempts of a slot are recorded as contaminated, so the re-run rule is rehearsed
    end to end with real processes. Never active outside --test."""
    if TEST and INJECT:
        # With an injection active the rehearsal is deterministic: contamination is EXACTLY the injected pattern (the real
        # receipts of a shared machine are kept as contaminated_real, not used).
        rec['contaminated_real'] = rec.get('contaminated')
        rec['contaminated'] = attempt_no < INJECT.get((rid, key, w, rnd), 0)
        rec['injected_hot'] = rec['contaminated']


def run_pass(block, p, attempt, runs_path, env, sampler, counters, state):
    bname = block['name']
    ptag = f'{bname}-p{p}' + (f'v{attempt}' if attempt else '')
    pdir = os.path.join(RAW, f'{ptag}_{RUN_TAG}')
    os.makedirs(pdir, exist_ok=True)
    order0 = cell_order(bname)
    order = pass_order(order0, p, block)
    rounds = ARGS.test_rounds if TEST else block_rounds(block)
    has_r4 = any(ROWS[rid].get('r4_twin') for rid, _, _ in order)
    round_recs = {}
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
    seq = 1
    slots = {}   # (round, row, binary, W) -> {'seq', 'rec' (the LATEST attempt), 'n_rerun'}
    state['rounds_done'] = 0
    for rnd in range(rounds):
        round_recs[rnd] = {}
        for rid, key, w in order:
            ensure_time(est_s(rid, key, w) + RECEIPT_S, f'{ptag} r{rnd} {rid}#{key}@W{w}')
            rec, receipt, st = run_one(rid, key, w, block, p, attempt, rnd, seq, 'original', pdir, receipt, env, sampler)
            inject_hot(rec, rid, key, w, rnd, 0)
            append(runs_path, rec)
            state['last'] = f'{ptag} r{rnd} {rid}#{key}@W{w}'
            if 'aborted' not in rec:
                counters['timed'] += 1
            log(f'  {ptag} r{rnd} [{seq:3d}] {rid}#{key} W={w}: {tag_of(rec)}')
            if st in ('abort', 'void'):
                return (st, rec.get('aborted') or rec.get('void'))
            round_recs[rnd][(rid, key, w)] = rec
            slots[(rnd, rid, key, w)] = {'seq': seq, 'rec': rec, 'n_rerun': 0}
            seq += 1
        if has_r4:
            counters['r4_differ'] = counters.get('r4_differ', 0) + sum(
                1 for r in r4_compare(block, p, attempt, rnd, round_recs[rnd], runs_path) if r['verdict'] == 'DIFFER')
        state['rounds_done'] = rnd + 1
    # Ruling 8: the re-run STAGE. Wave a re-runs every slot whose latest attempt is contaminated or invalid; a slot leaves the
    # stage at its first clean valid attempt. Bounds ("the pass ends"): RERUN_MAX_WAVES waves, or the wall allowance
    # rerun_allowance(block) spent on re-runs; the cutoff and the STOP flag (ensure_time) end everything as before: they RAISE out
    # of run_pass, so a pass cut inside its re-run stage writes NO passcell records and no pass_done (--resume re-runs it whole;
    # counts9a.py prints an INCOMPLETE line for it).
    stage_t0 = time.time()
    allowance_s = rerun_allowance(block, rounds)[0]
    if TEST and not os.environ.get('W9A_TEST_RERUN_S'):
        # a rehearsal's 12-step processes have no meaningful wall: unbounded unless W9A_TEST_ALLOWANCE_S names one (the allowance
        # case). With W9A_TEST_RERUN_S set, the REAL allowance above is kept and every re-run is charged that many seconds.
        allowance_s = float(os.environ['W9A_TEST_ALLOWANCE_S']) if os.environ.get('W9A_TEST_ALLOWANCE_S') else float('inf')
    n_stage_reruns = 0
    wave = 0
    ended = None
    while ended is None:
        todo = pending_reruns(slots)
        if not todo:
            break
        if wave >= RERUN_MAX_WAVES:
            ended = f'{RERUN_MAX_WAVES} waves spent'
            break
        wave += 1
        log(f'{ptag}: re-run wave {wave}: {len(todo)} slot(s) unclean: '
            f'{[(k[0], k[1], k[2], k[3], slot_bad(s["rec"])) for k, s in todo]}')
        for (rnd, rid, key, w), s in todo:
            if not rerun_admitted(stage_elapsed_s(stage_t0, n_stage_reruns), est_s(rid, key, w), allowance_s):
                ended = f'allowance spent ({allowance_s:.0f} s = max({RERUN_STAGE_FRAC} x pass wall, {RERUN_MAX_WAVES} waves x {RERUN_EST_SLACK} x the longest process))'
                break
            ensure_time(est_s(rid, key, w) + RECEIPT_S, f'{ptag} rerun {rid}#{key}@W{w}')
            why = slot_bad(s['rec'])
            n = s['n_rerun'] + 1
            rec, receipt, st = run_one(rid, key, w, block, p, attempt, rnd, s['seq'], 'rerun', pdir, receipt, env, sampler,
                                       rerun_no=n)
            inject_hot(rec, rid, key, w, rnd, n)
            rec['rerun_reason'] = why
            append(runs_path, rec)
            if 'aborted' not in rec:
                counters['timed'] += 1
                counters['rerun'] += 1
            log(f'  {ptag} r{rnd} [{s["seq"]:3d}] {rid}#{key} W={w} RERUN {n} ({why}): {tag_of(rec)}')
            if st in ('abort', 'void'):
                return (st, rec.get('aborted') or rec.get('void'))
            s['rec'] = rec
            s['n_rerun'] = n
            n_stage_reruns += 1
            if has_r4 and (ROWS[rid].get('r4_twin') or any(r.get('r4_twin') == rid for r in ROWS.values())):
                round_recs[rnd][(rid, key, w)] = rec  # the pair is compared again with the re-run in place
                pair = {k: v for k, v in round_recs[rnd].items() if k[2] == w and k[1] == key and
                        (k[0] == rid or ROWS[k[0]].get('r4_twin') == rid or ROWS[rid].get('r4_twin') == k[0])}
                counters['r4_differ'] = counters.get('r4_differ', 0) + sum(
                    1 for r in r4_compare(block, p, attempt, rnd, pair, runs_path) if r['verdict'] == 'DIFFER')
    cells = passcell_table(slots)
    for (rid, key, w), c in sorted(cells.items()):
        append(runs_path, {'passcell': True, 'run_tag': RUN_TAG, 'block': bname, 'pass': p, 'pass_attempt': attempt,
                           'pc_row': rid, 'pc_binary': key, 'pc_W': w, 'k_target': c['target'], 'k_clean': c['clean'],
                           'reruns': c['reruns'], 'unclean_rounds': c['unclean_rounds'],
                           'gates': c['clean'] >= MIN_K_GATE, 'short': c['clean'] < c['target'], 'time': D.now()})
    short = [(rid, key, w, c['clean'], c['target']) for (rid, key, w), c in sorted(cells.items()) if c['clean'] < c['target']]
    if wave or short:
        log(f'{ptag}: re-run stage ended after {wave} wave(s)' + (f' ({ended})' if ended else '') + f'; {len(short)} pass-cell(s) '
            f'short of their slots (k_clean < k_target; K < {MIN_K_GATE} does not gate): {short}')
    if short:
        progress(f'{block["item"]} {bname} p{p}: {len(short)} pass-cell(s) with unclean slots after the re-run stage: '
                 + ', '.join(f'{rid}#{key}@W{w} K={kc}/{kt}' for rid, key, w, kc, kt in short))
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


def stage_elapsed_s(stage_t0, n_done):
    """Seconds the re-run stage has spent. --test only: W9A_TEST_RERUN_S=<x> charges every re-run exactly x seconds (virtual), so the
    REAL per-block allowance can be exercised end to end with 12-step processes that finish in a blink (selftest9a's
    'real allowance' cases; the loop cannot silently fall back to another formula)."""
    if TEST and os.environ.get('W9A_TEST_RERUN_S'):
        return n_done * float(os.environ['W9A_TEST_RERUN_S'])
    return time.time() - stage_t0


def rerun_admitted(elapsed_s, process_est_s, allowance_s):
    """The stage's admission test for the next re-run (pure; ONE place for run_pass and the selftest): the stage has spent
    `elapsed_s`, the re-run is estimated at `process_est_s` (est_s: the dry-sample wall + launch overhead) plus its receipt."""
    return elapsed_s + process_est_s + RECEIPT_S <= allowance_s


def rerun_allowance(block, rounds=None):
    """(allowance_s, fraction_term_s, floor_term_s) of the block's per-pass re-run stage (pure). allowance = max of the two:
    the fraction is RERUN_STAGE_FRAC x the pass's own timed wall (rounds x per-round estimate); the floor is RERUN_MAX_WAVES x
    RERUN_EST_SLACK x the block's longest process (est + receipt), so a slot can always reach the full wave depth."""
    rounds = block_rounds(block) if rounds is None else rounds
    cells = cell_order(block['name'])
    frac_s = RERUN_STAGE_FRAC * rounds * block_estimate(block)['per_round_s']
    longest = max(est_s(rid, key, w) + RECEIPT_S for rid, key, w in cells)
    floor_s = RERUN_MAX_WAVES * RERUN_EST_SLACK * longest
    return max(frac_s, floor_s), frac_s, floor_s


def rerun_capacity(block, overrun=1.0, rounds=None):
    """How many consecutive re-runs of the block's LONGEST cell the stage admits when each takes overrun x its estimate (pure;
    the loop of run_pass with one always-hot slot: the depth a single slot can reach), capped at 99."""
    allowance = rerun_allowance(block, rounds)[0]
    longest = max(est_s(rid, key, w) for rid, key, w in cell_order(block['name']))
    n, elapsed = 0, 0.0
    while n < 99 and rerun_admitted(elapsed, longest, allowance):
        elapsed += overrun * (longest + RECEIPT_S)
        n += 1
    return n


def remaining_block_s(block, done):
    e = block_estimate(block)
    left = sum(1 for p in range(block_passes(block)) if f'{block["name"]}-p{p}' not in done)
    return left * e['per_pass_s']


def reserve_s(block, done):
    names = [b['name'] for b in BLOCKS]
    later = BLOCKS[names.index(block['name']) + 1:]
    return sum(remaining_block_s(b, done) for b in later if b['priority'] < block['priority'])


def print_schedule():
    t = dt.datetime.now()
    if ARGS.start:
        hh, mm = (int(x) for x in ARGS.start.split(':'))
        t = t.replace(hour=hh, minute=mm, second=0, microsecond=0)
    print(f'PHYSICS WINDOW 9a DRY RUN: rows9a.json{" + rows9a.extra.json" if "extra_overlay" in ROWS_JSON else ""}, {len(BLOCKS)} blocks, K per cell = passes x rounds (default '
          f'{PROTO["passes"]} x {PROTO["rounds_per_pass"]}; a block may set its own passes / rounds), cutoff {CUTOFF.strftime("%H:%M")}, '
          f'assumed start {t.strftime("%H:%M")}; STOP flag {STOP_FLAG} {"PRESENT" if os.path.exists(STOP_FLAG) else "absent"}')
    print(f'per pass: idle rule >= {IDLE_MIN_S:.0f} s, opening receipt {PASS_RECEIPT_S:.0f} s, one warm-up; per process: '
          f"the gate's untimed dry-sample wall + {LAUNCH_OVERHEAD_S} s launch + {RECEIPT_S:.0f} s receipt")
    print(f're-run rule (ruling 8): a slot whose latest attempt is contaminated or invalid is re-run in waves at the end of the pass '
          f'until its pass-cell has every slot clean; at most {RERUN_MAX_WAVES} waves and a per-pass wall allowance of '
          f'max({RERUN_STAGE_FRAC} x the pass wall, {RERUN_MAX_WAVES} waves x {RERUN_EST_SLACK} x the block\'s longest process); '
          f'a pass-cell with k_clean < {MIN_K_GATE} does not gate (passcell records in runs.jsonl)')
    bad = verify_binaries()
    print(f'binaries: {"all match bin/SHA256SUMS (+ sha256_pin)" if not bad else bad}')
    bad = verify_fixtures()
    print(f'fixture hashes: {"every pose_ref recorded" if not bad else bad}')
    missing = sorted({pose_path(r) for r in ROWS.values() if r['kind'] == 'runner' and not os.path.exists(pose_path(r))})
    print(f'fixtures: {"all present" if not missing else "MISSING " + ", ".join(missing)}')
    noest = sorted({f'{rid}#{k}@W{w}' for b in BLOCKS for rid, k, w in cell_order(b['name']) if f'{rid}#{k}@W{w}' not in EST})
    print(f'estimates: {"every cell has an untimed dry sample" if not noest else "MISSING (default assumed) " + ", ".join(noest)}')
    for d in ROWS_JSON.get('dm1_dropped', []):
        print(f'DM1 row dropped: {d["row"]}: {d["why"]}')
    done = set()
    if os.path.exists(PASSES_DONE) and ARGS.resume:
        done = {l.strip() for l in open(PASSES_DONE, encoding='utf-8') if l.strip()}
    t0 = t
    total = 0.0
    worst_rerun = 0.0
    model_rerun = 0.0
    for block in BLOCKS:
        e = block_estimate(block)
        need = remaining_block_s(block, done)
        res = reserve_s(block, done)
        start = t
        if need > 0 and start + dt.timedelta(seconds=need + res) > CUTOFF:
            print(f'\n[{block["name"]} prio {block["priority"]}]: WOULD BE SKIPPED at {start.strftime("%H:%M")} (needs '
                  f'{need / 60:.1f} min + {res / 60:.1f} min reserved; cutoff {CUTOFF.strftime("%H:%M")})')
            continue
        t = t + dt.timedelta(seconds=need)
        total += need
        left = sum(1 for p in range(block_passes(block)) if f'{block["name"]}-p{p}' not in done)
        allow, frac_s, floor_s = rerun_allowance(block)
        worst_rerun += left * allow
        model_rerun += left * RERUN_MODEL_RATE * block_rounds(block) * e['per_round_s']
        print(f'\n[{block["name"]} prio {block["priority"]}]: {e["cells"]} cells x K {e["k"]} = {e["processes"]} timed '
              f'processes + {block_passes(block) if block.get("warmup") else 0} warm-up(s); {block_passes(block)} pass(es) x {block_rounds(block)} rounds; '
              f'per round {e["per_round_s"]:.0f} s; per pass {e["per_pass_s"] / 60:.1f} min; block {need / 60:.1f} min; '
              f'{start.strftime("%H:%M")} -> {t.strftime("%H:%M")}')
        print(f'  re-run stage per pass: allowance {allow / 60:.1f} min = max(fraction {frac_s / 60:.1f}, floor {floor_s / 60:.1f}); admits '
              f'{rerun_capacity(block)} back-to-back re-runs of the longest cell ({rerun_capacity(block, 1.25)} if each runs 25 % over its '
              f'estimate); block worst case {left * allow / 60:.1f} min')
        order0 = cell_order(block['name'])
        for p in range(block_passes(block)):
            order = pass_order(order0, p, block)
            skip = ' (DONE, skipped by --resume)' if f'{block["name"]}-p{p}' in done else ''
            wu = f'warm-up {"#".join(str(x) for x in block["warmup"])}' if block.get('warmup') else 'no warm-up'
            print(f'  pass {p}{skip}: idle rule; {wu}; each round: '
                  + ', '.join(f'{rid}#{k}@W{w}({est_s(rid, k, w):.1f}s)' for rid, k, w in order))
    print(f'\nTOTAL {total / 60:.1f} min if every block runs; ends {t.strftime("%H:%M")}; the cutoff stops starting '
          f'processes at {CUTOFF.strftime("%H:%M")}')
    # The TOTAL above leaves the re-run stages out (as it always did); these lines price them (audit-1 W1: "re-price the cutoff").
    margin = 0.2 * total
    print(f'RE-RUN PRICE (not in TOTAL): model {model_rerun / 60:.0f} min (window 8b\'s slot rate {RERUN_MODEL_RATE * 100:.0f} % x the timed '
          f'wall; a MODEL, unmeasured for the 5-8 min criterion processes); worst case {worst_rerun / 60:.0f} min (every pass spends its '
          f'whole allowance: a bound, not an expectation)')
    print(f'RECOMMENDED --cutoff = start + TOTAL x 1.2 = {(t0 + dt.timedelta(minutes=math.ceil(1.2 * total / 60))).strftime("%H:%M")} (margin '
          f'{margin / 60:.0f} min over TOTAL: {"covers" if model_rerun <= margin else "DOES NOT cover"} the modelled re-run time '
          f'{model_rerun / 60:.0f} min; the worst case {"fits" if worst_rerun <= margin else "does NOT fit"} in it; a cut takes the last '
          f'blocks first and --resume recovers them)')


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
    manifest = {'generated': D.now(), 'rows_json': ROWS_PATH, 'rows_extra': ROWS_JSON.get('extra_overlay'), 'binaries': BINS,
                'fixtures': FIXTURES, 'protocol': PROTO, 'blocks': BLOCKS, 'cutoff': CUTOFF.isoformat(),
                'test': TEST, 'resume': ARGS.resume}
    with open(os.path.join(RAW, f'manifest_{int(time.time())}.json'), 'w', encoding='utf-8', newline='\n') as f:
        json.dump(manifest, f, indent=1, default=str)
    state = {'start': D.now(), 'test': TEST, 'placement': 'P-none (no affinity call)', 'voids': [],
             'machine_before': machine_state(), 'last': None}
    log(f'WINDOW START cutoff {CUTOFF.isoformat()} blocks {[b["name"] for b in BLOCKS]} test {TEST} resume {ARGS.resume}')
    bad = verify_binaries()
    if bad:
        return finish(3, f'STOP at start: binaries do not match bin/SHA256SUMS {bad}', state, None, {'timed': 0})
    bad = verify_fixtures()
    if bad and not TEST:
        return finish(3, f'STOP at start: fixtures {bad}', state, None, {'timed': 0})
    runs_path = os.path.join(RAW, 'runs.jsonl')
    sampler = PerfSampler(1.0)
    counters = {'timed': 0, 'rerun': 0, 'warm': 0, 'voided_processes': 0}
    done = set()
    if ARGS.resume and os.path.exists(PASSES_DONE):
        done = {l.strip() for l in open(PASSES_DONE, encoding='utf-8') if l.strip()}
    skipped = []
    for block in BLOCKS:
        bname = block['name']
        rows_here = sorted({c[0] for c in cell_order(bname)})
        passes = 1 if TEST else block_passes(block)
        if not TEST:
            need = remaining_block_s(block, done)
            res = reserve_s(block, done)
            if need > 0 and dt.datetime.now() + dt.timedelta(seconds=need + res) > CUTOFF:
                skipped.append(bname)
                msg = (f'{bname} SKIPPED: needs {need / 60:.1f} min + {res / 60:.1f} min reserved for later higher-priority '
                       f'blocks, cutoff {CUTOFF.strftime("%H:%M")}')
                log(msg)
                progress(f'{block["item"]} {bname} SKIPPED ({need / 60:.1f} + {res / 60:.1f} min > cutoff)')
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
                    return finish(2, f'cut at {CUTOFF.strftime("%H:%M:%S")} after {state["last"] or "nothing"} (next: {c})',
                                  state, sampler, counters)
                except StopFlag as c:
                    progress(f'{block["item"]} {bname} p{p} STOP flag before the idle rule')
                    return finish(2, f'cut by the STOP flag after {state["last"] or "nothing"} (next: {c})', state, sampler, counters)
                log(f'{ptag}: idle rule (wait_idle9a.ps1, up to {IDLE_MAX_POLLS} polls)')
                rc = wait_idle()
                if rc != 0:
                    progress(f'{block["item"]} {bname} p{p} STOP idle rule not met')
                    return finish(3, f'STOP before {ptag}: idle rule not met in {IDLE_MAX_POLLS} polls (wait_idle exit {rc}); '
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
                    return finish(2, f'cut at {CUTOFF.strftime("%H:%M:%S")} after {state["last"]} (next would have been {c})',
                                  state, sampler, counters)
                except StopFlag as c:
                    rd = state.get('rounds_done', 0)
                    for rid in rows_here:
                        progress(f'{block["item"]} {rid} p{p} STOP flag ({rd} of {block_rounds(block)} rounds complete)')
                    return finish(2, f'cut by the STOP flag after {state["last"]} (next would have been {c})', state, sampler, counters)
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
                append(runs_path, {'voided_pass': True, 'run_tag': RUN_TAG, 'block': bname, 'pass': p,
                                   'pass_attempt': attempt, 'why': why, 'time': D.now()})
                progress(f'{block["item"]} {bname} p{p} VOIDED ({why[:160]}); re-running after idle')
                log(f'{ptag}: PASS VOIDED ({why}); {n_void} processes discarded; waiting for idle and re-running')
                attempt += 1
                if attempt > MAX_VOIDS_PER_PASS:
                    return finish(3, f'STOP in {pkey}: voided {attempt} times', state, sampler, counters)
    if skipped:
        return finish(2, f'complete except the blocks skipped at the cutoff: {", ".join(skipped)}', state, sampler, counters)
    return finish(0, 'complete', state, sampler, counters)


if __name__ == '__main__':
    sys.exit(main())
