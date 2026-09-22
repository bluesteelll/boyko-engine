"""Physics perf campaign P0: the parity-window driver.

Runs the rows of `docs/MEASUREMENT-QUEUE.md` §10 (the run sheet of
`docs/physics/perf-campaign/01-PLAN-REV1.md` §1-§2 with `00-RULINGS.md`) against PREBUILT
binaries, and reduces them. Python 3 standard library only; Windows (ctypes for the load
receipt).

Subcommands
-----------

    driver.py manifest --out M.json --repo <tree> --boyko tip=<exe> [--boyko pre_instrument=<exe>]
                       [--boyko pre_l1=<exe>] [--boyko shipping=<exe>] --jolt v530=<exe>
                       [--jolt v530_release=<exe>] [--jolt v560=<exe>] [--ancestor NAME=SHA ...]
                       [--toolchain stable-x86_64-pc-windows-msvc] [--profile parity]

        Hashes every binary (and the runtime DLLs beside each Jolt exe), reads each Jolt build's
        compiler and CMake options from the CMakeCache.txt beside it, and takes the H6 receipts
        of the boyko tree: HEAD, whether the tree is dirty, `rustc -vV`'s host line (must read
        msvc), and `git merge-base --is-ancestor` for S5 (8d656ad8), KE16 (67563d3b) and every
        `--ancestor`. Nothing is built: the binaries exist before the window.

    driver.py pass --manifest M.json --out DIR --pass N [--dry] [--only ROW,...] [--ws 1,2,...]
                   [--receipt-window S] [--busy-pct P] [--wait-budget S]

        One pass over every row, interleaved by W. On even passes W ascends and each W group runs
        Jolt then boyko; on odd passes W descends and boyko runs first (H12). A load receipt is
        taken between consecutive runs and serves as the AFTER of one and the BEFORE of the next,
        so every timed region has both; before a run the driver waits (up to --wait-budget) while
        the machine is busier than --busy-pct and marks the run contaminated if it never quiets.
        Every run gets a fresh working directory; stdout and stderr are captured as BYTES and
        written as bytes (never `text=True`), and decoded explicitly as UTF-8 only to parse.
        Each run appends one JSON record to DIR/runs.jsonl.

        `--dry` is the rehearsal: every row with tiny step counts (windows scaled), receipts
        taken but never waited on, `--frozen-by` dropped (a 30-step pile cannot freeze). It
        proves the pipeline end to end; its numbers are NOT measurements.

    driver.py reduce --out DIR [--dry]

        Reads DIR/runs.jsonl and the per-run files and writes DIR/reduction.json and
        DIR/reduction.txt: the headline per W, A/A0 (the band B(W)), A/A1, A/A2, the canary,
        anti-vacuity, determinism, closure, the plan §2 identity terms, the L1 gate, H8 and O5.
        Under --dry the text prints verdict names and counts only, no times.

        Pairing: the parity T(W) is J-A-d1's window mean; J-A-d2 exists to pair with it for the
        band. A/A1 pairs J-A-a with J-A-d1, A/A2 J-A-d1 with the pre-instrument row, the canary
        J-C with J-A-a (both armed, so the canary is the only difference), and L1's gate L1-B
        with the per-step mean of L1-A1 and L1-A2, whose own pairing is the spread. Every
        statistic is a median over the steps of the row's window, then a median over passes.

        Not reduced: Jolt's `-p` stage shares (the HTML charts are listed, not parsed), and the
        per-chunk imbalance share of W2 (no zone may sit inside a worker's chunk task, O1).

The rows
--------
`ROWS` below is §10's table, row for row: J-A (disarmed twice, armed once), J-B, J-P1, J-C,
J-Son, J-S0 with L1's gate (pre-L1 twice around the tip), R, R-ref, R-S, S16, A/A2's
pre-instrument row, the disarmed `shipping` row (O6), and the Jolt rows (timed v5.3.0, v5.6.0,
-no_pair_cache, -allow_sleep, the Release -p shares and the untimed -receipt). Edit the table,
not the loop, to change what a pass runs.

Thread counts (rulings, open question 2): boyko at W runs W pool workers plus the dispatcher,
which is parked while a system runs on a worker; every armed boyko summary carries a witness
(`threads.solve_on_dispatcher_steps`). Jolt at `-t=W` runs W - 1 job threads plus the calling
thread. The reduction prints both per W.
"""

import argparse
import csv
import ctypes
import datetime
import hashlib
import json
import os
import statistics
import subprocess
import sys
import time
from ctypes import wintypes

WS = (1, 2, 4, 8, 16)
S5_COMMIT = '8d656ad8'
KE16_COMMIT = '67563d3b'
CANARY_FRAC = 0.05
AA0_CEILING = 0.02
AA1_FLOOR = 0.005
CANARY_SPAN_TOL = 0.05
U_CEILING = 0.03
H8_TOLERANCE = 0.10
SUBWINDOW_FRAC = 0.2  # [0, 100) of 500
BUILD_PROCS = {'cargo.exe', 'rustc.exe', 'link.exe', 'lld.exe', 'rust-lld.exe', 'cc1plus.exe', 'cc1.exe',
               'lto1.exe', 'lto-wrapper.exe', 'ld.exe', 'collect2.exe', 'mingw32-make.exe', 'cmake.exe',
               'dxc.exe', 'clippy-driver.exe', 'cargo-miri.exe', 'miri.exe'}

# ── The row table (docs/MEASUREMENT-QUEUE.md §10) ────────────────────────────

JA = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a']
JS0 = JA + ['--sleeping', '--threshold', '0']


def B(rid, role, ws, args, steps=500, window=(0, 500), armed=False, dry_steps=12, dry_window=None, per_w=None,
      canary_of=None, frozen_by=None, expect_pose_of=None):
    return {'id': rid, 'engine': 'boyko', 'role': role, 'ws': ws, 'args': args, 'steps': steps,
            'window': window, 'armed': armed, 'dry_steps': dry_steps,
            'dry_window': dry_window or (0, dry_steps), 'per_w': per_w or {}, 'canary_of': canary_of,
            'frozen_by': frozen_by, 'expect_pose_of': expect_pose_of, 'once': False, 'timed': True}


def J(rid, role, ws, args, steps=500, dry_steps=12, once=False, timed=True, per_frame=True):
    return {'id': rid, 'engine': 'jolt', 'role': role, 'ws': ws, 'args': args, 'steps': steps,
            'window': (0, steps), 'dry_steps': dry_steps, 'dry_window': (0, dry_steps), 'once': once,
            'timed': timed, 'per_frame': per_frame, 'armed': False}


# Order inside a W group is the order here. J-C needs this pass's J-A-a at the same W (its canary
# reference), J-B asserts pose equality with this pass's J-A-d1 at the same W (H7).
BOYKO_ROWS = [
    B('J-A-d1', 'tip', WS, JA),
    B('J-A-a', 'tip', WS, JA, armed=True),
    B('J-B', 'tip', (1, 8), ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'b'], armed=True, expect_pose_of='J-A-d1'),
    B('J-C', 'tip', (1, 8), JA, armed=True, canary_of='J-A-a'),
    B('J-P1', 'tip', (1,), JA + ['--parallel-solve'], armed=True),
    B('J-Son', 'tip', (1, 8), JA + ['--sleeping'], steps=1000, window=(0, 1000), armed=True, dry_steps=20),
    B('J-S0', 'tip', (1,), JS0, armed=True),
    B('L1-A1', 'pre_l1', (1,), JS0),
    B('L1-B', 'tip', (1,), JS0),
    B('L1-A2', 'pre_l1', (1,), JS0),
    B('R', 'tip', (1, 8), ['--scene', 'rest', '--solver', 'colored'], steps=1100, window=(600, 1100), armed=True,
      dry_steps=30, dry_window=(20, 30), per_w={8: ['--parallel-solve']}),
    B('R-ref', 'tip', (1,), ['--scene', 'rest', '--solver', 'reference'], steps=1100, window=(600, 1100),
      armed=True, dry_steps=30, dry_window=(20, 30)),
    B('R-S', 'tip', (1, 8), ['--scene', 'rest', '--sleeping'], steps=800, window=(300, 800), armed=True,
      dry_steps=30, dry_window=(20, 30), frozen_by=300),
    B('S16', 'tip', (1, 8), ['--scene', 's16', '--cfg', 'a'], steps=300, window=(0, 300), armed=True, dry_steps=10),
    B('AA2-pre', 'pre_instrument', (1, 8), JA),
    B('SHIP', 'shipping', (1, 8), JA),
    B('J-A-d2', 'tip', WS, JA),
]

JOLT_T = ['-s=Pyramid', '-q=Discrete', '-f']
JOLT_ROWS = [
    J('JOLT-T', 'v530', WS, JOLT_T),
    J('JOLT56-T', 'v560', WS, JOLT_T),
    J('JOLT-NPC', 'v530', (8,), JOLT_T + ['-no_pair_cache']),
    J('JOLT-SLP', 'v530', (1, 8), JOLT_T + ['-allow_sleep'], steps=1000, dry_steps=20),
    J('JOLT-P', 'v530_release', (1, 8), ['-s=Pyramid', '-q=Discrete', '-p'], once=True, per_frame=False),
    J('JOLT-RCPT', 'v530', (1,), ['-s=Pyramid', '-q=Discrete', '-receipt'], once=True, timed=False,
      per_frame=False),
]
ROWS = {r['id']: r for r in BOYKO_ROWS + JOLT_ROWS}

# ── Small helpers ─────────────────────────────────────────────────────────────


def now():
    return datetime.datetime.now().astimezone().isoformat(timespec='milliseconds')


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def run_bytes(cmd, cwd=None, env=None):
    """Runs `cmd`, returning (exit code, stdout bytes, stderr bytes). Never text mode."""
    p = subprocess.run(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return p.returncode, p.stdout, p.stderr


def decode(b):
    return b.decode('utf-8', errors='replace')


def median(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def paired_median_ratio(num, den):
    """median_k(num_k / den_k) over the steps both have."""
    rs = [a / b for a, b in zip(num, den) if b]
    return statistics.median(rs) if rs else None


# ── The load receipt (ctypes: GetSystemTimes + a Toolhelp process walk) ──────

class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [('dwSize', wintypes.DWORD), ('cntUsage', wintypes.DWORD), ('th32ProcessID', wintypes.DWORD),
                ('th32DefaultHeapID', ctypes.c_size_t), ('th32ModuleID', wintypes.DWORD),
                ('cntThreads', wintypes.DWORD), ('th32ParentProcessID', wintypes.DWORD),
                ('pcPriClassBase', ctypes.c_long), ('dwFlags', wintypes.DWORD),
                ('szExeFile', ctypes.c_wchar * 260)]


def _kernel32():
    """kernel32 with the signatures this module calls: HANDLE results must not be truncated to the
    default 32-bit `int` restype."""
    k = ctypes.WinDLL('kernel32', use_last_error=True)
    ft = ctypes.POINTER(wintypes.FILETIME)
    k.GetSystemTimes.argtypes = [ft, ft, ft]
    k.GetSystemTimes.restype = wintypes.BOOL
    k.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    k.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    k.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
    k.Process32FirstW.restype = wintypes.BOOL
    k.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
    k.Process32NextW.restype = wintypes.BOOL
    k.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    k.OpenProcess.restype = wintypes.HANDLE
    k.GetProcessTimes.argtypes = [wintypes.HANDLE, ft, ft, ft, ft]
    k.GetProcessTimes.restype = wintypes.BOOL
    k.CloseHandle.argtypes = [wintypes.HANDLE]
    k.CloseHandle.restype = wintypes.BOOL
    return k


_k32 = _kernel32() if os.name == 'nt' else None
INVALID_HANDLE = wintypes.HANDLE(-1).value


def _ft(f):
    return ((f.dwHighDateTime << 32) | f.dwLowDateTime) / 1e7


def system_times():
    i, k, u = wintypes.FILETIME(), wintypes.FILETIME(), wintypes.FILETIME()
    if not _k32.GetSystemTimes(ctypes.byref(i), ctypes.byref(k), ctypes.byref(u)):
        raise OSError('GetSystemTimes failed')
    return _ft(i), _ft(k) + _ft(u)  # (idle, total incl. idle)


def process_cpu_table():
    """{pid: (exe name, cpu seconds)} for every process whose times this user may read."""
    out = {}
    snap = _k32.CreateToolhelp32Snapshot(0x2, 0)  # TH32CS_SNAPPROCESS
    if not snap or snap == INVALID_HANDLE:
        return out
    try:
        e = PROCESSENTRY32W()
        e.dwSize = ctypes.sizeof(PROCESSENTRY32W)
        ok = _k32.Process32FirstW(snap, ctypes.byref(e))
        while ok:
            pid = e.th32ProcessID
            h = _k32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
            if h:
                c, x, k, u = (wintypes.FILETIME() for _ in range(4))
                if _k32.GetProcessTimes(h, ctypes.byref(c), ctypes.byref(x), ctypes.byref(k), ctypes.byref(u)):
                    out[pid] = (e.szExeFile, _ft(k) + _ft(u))
                _k32.CloseHandle(h)
            ok = _k32.Process32NextW(snap, ctypes.byref(e))
    finally:
        _k32.CloseHandle(snap)
    return out


def load_receipt(window_s):
    """Busy % of the whole machine over `window_s` (sampled each 0.5 s), plus the top processes by
    CPU time in that window and any build process that used CPU in it."""
    p0 = process_cpu_table()
    samples = []
    t_idle, t_total = system_times()
    end = time.time() + window_s
    while time.time() < end:
        time.sleep(0.5)
        idle, total = system_times()
        d_total = total - t_total
        if d_total > 0:
            samples.append(100.0 * (1.0 - (idle - t_idle) / d_total))
        t_idle, t_total = idle, total
    p1 = process_cpu_table()
    deltas = []
    for pid, (name, cpu) in p1.items():
        prev = p0.get(pid)
        d = cpu - (prev[1] if prev and prev[0] == name else 0.0)
        deltas.append((d, name, pid))
    deltas.sort(reverse=True)
    ncpu = os.cpu_count() or 1
    return {'time': now(), 'window_s': window_s, 'cpu_samples': [round(s, 2) for s in samples],
            'cpu_avg': round(sum(samples) / len(samples), 2) if samples else None,
            'logical_cpus': ncpu,
            'top5': [{'name': n, 'pid': p, 'cpu_s': round(d, 3), 'pct_of_machine': round(100 * d / (window_s * ncpu), 2)}
                     for d, n, p in deltas[:5]],
            'build_procs_busy': sorted({n for d, n, p in deltas if d > 0 and n.lower() in BUILD_PROCS})}


# ── manifest ──────────────────────────────────────────────────────────────────


def cmake_cache(exe):
    path = os.path.join(os.path.dirname(exe), 'CMakeCache.txt')
    keys = {}
    if not os.path.isfile(path):
        return None
    for line in open(path, 'rb').read().decode('utf-8', errors='replace').splitlines():
        if ':' in line and '=' in line and not line.startswith(('//', '#')):
            k, v = line.split('=', 1)
            keys[k.split(':', 1)[0]] = v
    return keys


def jolt_entry(exe):
    cache = cmake_cache(exe) or {}
    compiler = cache.get('CMAKE_CXX_COMPILER')
    version = None
    if compiler and os.path.isfile(compiler):
        _, out, _ = run_bytes([compiler, '--version'])
        version = decode(out).splitlines()[0] if out else None
    dlls = {f: sha256(os.path.join(os.path.dirname(exe), f)) for f in sorted(os.listdir(os.path.dirname(exe)))
            if f.lower().endswith('.dll')}
    wanted = ('CMAKE_BUILD_TYPE', 'CMAKE_GENERATOR', 'CMAKE_HOME_DIRECTORY', 'INTERPROCEDURAL_OPTIMIZATION',
              'USE_AVX2', 'USE_AVX512', 'USE_FMADD', 'USE_F16C', 'USE_LZCNT', 'USE_TZCNT', 'GENERATE_DEBUG_SYMBOLS',
              'FLOATING_POINT_EXCEPTIONS_ENABLED', 'CROSS_PLATFORM_DETERMINISTIC', 'PROFILER_IN_DISTRIBUTION',
              'PROFILER_IN_DEBUG_AND_RELEASE', 'USE_ASSERTS', 'DOUBLE_PRECISION', 'JPH_USE_DX12', 'JPH_USE_VK',
              'JPH_USE_MTL', 'JPH_USE_CPU_COMPUTE', 'JPH_BUILD_SHARED_LIBS', 'BUILD_SHARED_LIBS')
    src = cache.get('CMAKE_HOME_DIRECTORY')
    head, diff_sha = None, None
    if src:
        root = os.path.dirname(src)
        rc, out, _ = run_bytes(['git', '-C', root, 'describe', '--tags', '--always', '--dirty'])
        head = decode(out).strip() if rc == 0 else None
        rc, out, _ = run_bytes(['git', '-C', root, 'diff'])
        diff_sha = diff_signature(out) if rc == 0 else None
    return {'exe': exe, 'sha256': sha256(exe), 'compiler': compiler, 'compiler_version': version,
            'cmake': {k: cache.get(k) for k in wanted if k in cache}, 'source_describe': head,
            'source_diff_sha256': diff_sha, 'dlls': dlls}


def repo_patch_sha(repo):
    """sha256 of the diff part of the repository's Jolt patch (its prose header excluded)."""
    path = os.path.join(repo, 'crates', 'boyko_physics', 'benches', 'jolt_parity', 'pyramid_scene.patch')
    if not os.path.isfile(path):
        return None
    data = open(path, 'rb').read()
    at = data.find(b'diff --git')
    return diff_signature(data[at:]) if at >= 0 else None


def diff_signature(diff):
    """sha256 over a unified diff's file headers and changed lines only, so the same change applied
    at other line offsets or over other blob ids (v5.3.0 against v5.6.0) has the same signature."""
    keep = []
    for line in diff.replace(b'\r\n', b'\n').split(b'\n'):
        if line.startswith(b'diff --git'):
            keep.append(line)
        elif line.startswith((b'+', b'-')) and not line.startswith((b'+++', b'---')):
            keep.append(line)
    return hashlib.sha256(b'\n'.join(keep)).hexdigest()


def cmd_manifest(a):
    repo = a.repo
    env = dict(os.environ)
    rc, out, _ = run_bytes(['git', '-C', repo, 'rev-parse', 'HEAD'])
    head = decode(out).strip()
    _, out, _ = run_bytes(['git', '-C', repo, 'status', '--porcelain'])
    dirty = [l for l in decode(out).splitlines() if l.strip()]
    rc, out, err = run_bytes(['rustc', f'+{a.toolchain}', '-vV'], env=env)
    rustc = decode(out)
    host = next((l.split(':', 1)[1].strip() for l in rustc.splitlines() if l.startswith('host:')), None)
    ancestors = {'S5': S5_COMMIT, 'KE16': KE16_COMMIT}
    for item in a.ancestor or []:
        k, v = item.split('=', 1)
        ancestors[k] = v
    anc = {}
    for k, sha in ancestors.items():
        rc, _, _ = run_bytes(['git', '-C', repo, 'merge-base', '--is-ancestor', sha, 'HEAD'])
        anc[k] = {'commit': sha, 'is_ancestor': rc == 0, 'git_exit': rc}
    boyko = {}
    for item in a.boyko or []:
        k, v = item.split('=', 1)
        boyko[k] = {'exe': v, 'sha256': sha256(v)}
    jolt = {}
    patch_sha = repo_patch_sha(repo)
    for item in a.jolt or []:
        k, v = item.split('=', 1)
        jolt[k] = jolt_entry(v)
        # The build's source tree carries exactly the repository's patch, or the receipt says not.
        jolt[k]['source_is_repo_patch'] = patch_sha is not None and jolt[k]['source_diff_sha256'] == patch_sha
    m = {'generated': now(), 'repo': repo, 'head': head, 'dirty_paths': dirty, 'toolchain': a.toolchain,
         'rustc_vV': rustc, 'host': host, 'host_is_msvc': bool(host and host.endswith('windows-msvc')),
         'profile': a.profile, 'ancestors': anc, 'boyko': boyko, 'jolt': jolt, 'logical_cpus': os.cpu_count()}
    with open(a.out, 'w', encoding='utf-8') as f:
        json.dump(m, f, indent=1)
    print(f'manifest: {a.out}: head {head[:12]} dirty {len(dirty)} host {host} '
          f'ancestors {{{", ".join(f"{k}: {v["is_ancestor"]}" for k, v in anc.items())}}} '
          f'boyko {sorted(boyko)} jolt {sorted(jolt)}')
    if not m['host_is_msvc']:
        print('manifest: WARNING the host line is not msvc; H6 forbids quoting a ratio from this tree')


# ── pass ──────────────────────────────────────────────────────────────────────


def plan_pass(pass_no, ws_filter, only, dry):
    """The ordered run list of one pass: [(row, W)]."""
    ws = [w for w in WS if w in ws_filter]
    if pass_no % 2 == 1:
        ws = ws[::-1]
    order = []
    for w in ws:
        jolt = [(r, w) for r in JOLT_ROWS if w in r['ws'] and not (r['once'] and pass_no != 1)]
        boyko = [(r, w) for r in BOYKO_ROWS if w in r['ws']]
        group = jolt + boyko if pass_no % 2 == 0 else boyko + jolt
        order.extend((r, w) for r, w in group if not only or r['id'] in only)
    return order


def boyko_args(row, w, dry, cwd, ctx):
    steps = row['dry_steps'] if dry else row['steps']
    window = row['dry_window'] if dry else row['window']
    args = list(row['args']) + row['per_w'].get(w, [])
    args += ['--workers', str(w), '--steps', str(steps), '--window', f'{window[0]}..{window[1]}',
             '--csv', os.path.join(cwd, 'run.csv'), '--pose-out', os.path.join(cwd, 'pose.bin'),
             '--label', f'{row["id"]}@W{w}']
    if row['armed']:
        args.append('--arm-profiler')
    if row['frozen_by'] and not dry:
        args += ['--frozen-by', str(row['frozen_by'])]
    if row['canary_of']:
        ref = ctx.get((row['canary_of'], w))
        if ref is None or not ref.get('summary'):
            return None, f'no {row["canary_of"]} at W={w} in this pass for the canary reference'
        t = ref['summary']['window_mean_ns']
        args += ['--canary-frac', str(CANARY_FRAC), '--canary-ref-ns', str(int(round(t)))]
    if row['expect_pose_of']:
        ref = ctx.get((row['expect_pose_of'], w))
        if ref is not None and os.path.isfile(os.path.join(ref['cwd'], 'pose.bin')):
            args += ['--expect-pose', os.path.join(ref['cwd'], 'pose.bin')]
    return args, None


def jolt_args(row, w, dry):
    steps = row['dry_steps'] if dry else row['steps']
    return list(row['args']) + [f'-t={w}', f'-i={steps}'], None


def parse_summary(stdout):
    for line in decode(stdout).splitlines():
        if line.startswith('SUMMARY '):
            try:
                return json.loads(line[8:])
            except json.JSONDecodeError:
                return None
    return None


def parse_jolt_stdout(stdout):
    res = {'patch_line': None, 'stat_lines': []}
    for line in decode(stdout).splitlines():
        if line.startswith('boyko-parity-patch'):
            res['patch_line'] = line
        parts = [p.strip() for p in line.split(',')]
        if len(parts) == 4 and parts[0] in ('Discrete', 'LinearCast'):
            try:
                res['stat_lines'].append({'quality': parts[0], 'threads': int(parts[1]),
                                          'steps_per_s': float(parts[2]), 'hash': parts[3]})
            except ValueError:
                pass
    return res


def cmd_pass(a):
    m = json.load(open(a.manifest, encoding='utf-8'))
    out = os.path.abspath(a.out)
    pdir = os.path.join(out, f'pass-{a.pass_no:02d}')
    os.makedirs(pdir, exist_ok=True)
    with open(os.path.join(out, 'manifest.json'), 'w', encoding='utf-8') as f:
        json.dump(m, f, indent=1)
    ws_filter = {int(x) for x in a.ws.split(',')} if a.ws else set(WS)
    only = set(a.only.split(',')) if a.only else None
    order = plan_pass(a.pass_no, ws_filter, only, a.dry)
    # Every binary is checked against the manifest before the first run: a rebuilt exe is a
    # different instrument.
    for role, e in list(m['boyko'].items()) + list(m['jolt'].items()):
        got = sha256(e['exe'])
        if got != e['sha256']:
            sys.exit(f'pass: {role} {e["exe"]} sha256 {got} != manifest {e["sha256"]}')
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    ctx = {}
    runs_path = os.path.join(out, 'runs.jsonl')
    receipt_window = a.receipt_window if not a.dry else min(a.receipt_window, 0.5)
    receipt = load_receipt(receipt_window)
    print(f'pass {a.pass_no}: {len(order)} runs{" (DRY)" if a.dry else ""}')
    for seq, (row, w) in enumerate(order):
        role = row['role']
        entry = (m['boyko'] if row['engine'] == 'boyko' else m['jolt']).get(role)
        rec = {'pass': a.pass_no, 'seq': seq, 'row': row['id'], 'engine': row['engine'], 'role': role, 'W': w,
               'dry': a.dry}
        if entry is None:
            rec['skipped'] = f'no {row["engine"]} binary for role {role!r} in the manifest'
            print(f'  [{seq:3d}] {row["id"]} W={w}: SKIPPED ({rec["skipped"]})')
            with open(runs_path, 'a', encoding='utf-8') as f:
                f.write(json.dumps(rec) + '\n')
            continue
        cwd = os.path.join(pdir, f'{seq:03d}_{row["id"]}_W{w}')
        os.makedirs(cwd, exist_ok=True)
        if row['engine'] == 'boyko':
            args, why = boyko_args(row, w, a.dry, cwd, ctx)
        else:
            args, why = jolt_args(row, w, a.dry)
        if args is None:
            rec['skipped'] = why
            print(f'  [{seq:3d}] {row["id"]} W={w}: SKIPPED ({why})')
            with open(runs_path, 'a', encoding='utf-8') as f:
                f.write(json.dumps(rec) + '\n')
            continue
        # The quiet rule, before a timed region.
        waited = 0.0
        if row['timed'] and not a.dry:
            t0 = time.time()
            while (receipt['cpu_avg'] or 0) > a.busy_pct and time.time() - t0 < a.wait_budget:
                time.sleep(20)
                receipt = load_receipt(receipt_window)
            waited = time.time() - t0
        rec.update({'exe': entry['exe'], 'exe_sha256': entry['sha256'], 'args': args, 'cwd': cwd,
                    'receipt_before': receipt, 'waited_s': round(waited, 1),
                    'contaminated_before': (receipt['cpu_avg'] or 0) > a.busy_pct or bool(receipt['build_procs_busy'])})
        rec['start'] = now()
        t_start = time.perf_counter()
        code, so, se = run_bytes([entry['exe']] + args, cwd=cwd, env=env)
        rec['wall_s'] = round(time.perf_counter() - t_start, 3)
        rec['end'] = now()
        rec['exit'] = code
        with open(os.path.join(cwd, 'stdout.txt'), 'wb') as f:
            f.write(so)
        with open(os.path.join(cwd, 'stderr.txt'), 'wb') as f:
            f.write(se)
        receipt = load_receipt(receipt_window)
        rec['receipt_after'] = receipt
        rec['contaminated_after'] = (receipt['cpu_avg'] or 0) > a.busy_pct or bool(receipt['build_procs_busy'])
        if row['engine'] == 'boyko':
            rec['summary'] = parse_summary(so)
            rec['csv'] = os.path.join(cwd, 'run.csv')
            rec['pose'] = os.path.join(cwd, 'pose.bin')
            tag = (f'void {rec["summary"]["void_steps"]} hash {rec["summary"]["pose_hash"]} '
                   f'pose {rec["summary"]["expect_pose"]}' if rec['summary'] else 'NO SUMMARY')
        else:
            rec['jolt'] = parse_jolt_stdout(so)
            files = sorted(os.listdir(cwd))
            rec['files'] = files
            pf = [f for f in files if f.startswith('per_frame_')]
            rc = [f for f in files if f.startswith('receipt_')]
            rec['per_frame'] = os.path.join(cwd, pf[0]) if pf else None
            rec['receipt_csv'] = os.path.join(cwd, rc[0]) if rc else None
            st = rec['jolt']['stat_lines']
            tag = f'hash {st[0]["hash"]} threads {st[0]["threads"]}' if st else 'NO STAT LINE'
        ctx[(row['id'], w)] = rec
        with open(runs_path, 'a', encoding='utf-8') as f:
            f.write(json.dumps(rec) + '\n')
        flag = '' if code == 0 else f' EXIT {code}'
        print(f'  [{seq:3d}] {row["id"]} W={w}: {tag}{flag}')


# ── reduce ────────────────────────────────────────────────────────────────────


def load_csv(path):
    if not path or not os.path.isfile(path):
        return None
    rows = list(csv.reader(open(path, encoding='utf-8', newline='')))
    if not rows:
        return None
    hdr = [h.strip() for h in rows[0]]
    cols = {h: [] for h in hdr}
    for r in rows[1:]:
        for h, v in zip(hdr, r):
            v = v.strip()
            try:
                cols[h].append(float(v) if v != '' else None)
            except ValueError:
                cols[h].append(None)
    return cols


def window_of(rec):
    row = ROWS[rec['row']]
    return row['dry_window'] if rec['dry'] else row['window']


def wmean(col, window):
    if col is None:
        return None
    return mean(col[window[0]:window[1]])


class Data:
    def __init__(self, out):
        self.recs = [json.loads(l) for l in open(os.path.join(out, 'runs.jsonl'), encoding='utf-8')]
        self.recs = [r for r in self.recs if 'skipped' not in r]
        self.cache = {}

    def runs(self, row, w=None):
        return [r for r in self.recs if r['row'] == row and (w is None or r['W'] == w)]

    def passes(self):
        return sorted({r['pass'] for r in self.recs})

    def one(self, row, w, p):
        rs = [r for r in self.recs if r['row'] == row and r['W'] == w and r['pass'] == p]
        return rs[0] if rs else None

    def cols(self, rec):
        key = (rec['pass'], rec['seq'])
        if key not in self.cache:
            if rec['engine'] == 'boyko':
                self.cache[key] = load_csv(rec.get('csv'))
            else:
                c = load_csv(rec.get('per_frame'))
                if c is not None:
                    c['wall_ns'] = [x * 1e6 if x is not None else None for x in c.get('Time (ms)', [])]
                    c['awake'] = c.get('Active Bodies')
                self.cache[key] = c
        return self.cache[key]

    def wall(self, rec):
        c = self.cols(rec)
        if c is None:
            return None
        w = window_of(rec)
        return c['wall_ns'][w[0]:w[1]]

    def t(self, rec, col='wall_ns', window=None):
        c = self.cols(rec)
        if c is None or col not in c:
            return None
        return wmean(c[col], window or window_of(rec))


def pct(x):
    return None if x is None else round(100 * x, 3)


def cmd_reduce(a):
    out = os.path.abspath(a.out)
    d = Data(out)
    P = d.passes()
    R = {'generated': now(), 'dry': a.dry, 'passes': P}

    # Receipts and exits.
    R['runs'] = len(d.recs)
    R['nonzero_exits'] = [(r['row'], r['W'], r['pass'], r['exit']) for r in d.recs if r.get('exit') not in (0, None)]
    R['contaminated'] = [(r['row'], r['W'], r['pass']) for r in d.recs
                         if r.get('contaminated_before') or r.get('contaminated_after')]
    tip_checks = []
    for r in d.recs:
        s = r.get('summary')
        if r['engine'] == 'boyko' and s:
            if s.get('target_env') != 'msvc':
                tip_checks.append(f'{r["row"]}@W{r["W"]}: target_env {s.get("target_env")}')
            if s.get('debug_assertions') and not a.dry:
                tip_checks.append(f'{r["row"]}@W{r["W"]}: a debug build')
            if r['row'] == 'SHIP' and s.get('profile_name') != 'shipping' and not a.dry:
                tip_checks.append(f'SHIP@W{r["W"]}: profile {s.get("profile_name")}, not shipping (O6)')
            if s.get('armed') and not s.get('zones_compiled'):
                tip_checks.append(f'{r["row"]}@W{r["W"]}: armed on a build that folds the zones')
    R['build_checks'] = tip_checks
    # H6: the receipts every quoted number carries.
    man_path = os.path.join(out, 'manifest.json')
    if os.path.isfile(man_path):
        m = json.load(open(man_path, encoding='utf-8'))
        R['h6'] = {'head': m.get('head'), 'dirty_paths': len(m.get('dirty_paths', [])), 'host': m.get('host'),
                   'host_is_msvc': m.get('host_is_msvc'), 'ancestors': m.get('ancestors'),
                   'profile': m.get('profile'),
                   'boyko': {k: v['sha256'] for k, v in m.get('boyko', {}).items()},
                   'jolt': {k: {'sha256': v['sha256'], 'compiler': v.get('compiler_version'),
                                'source': v.get('source_describe'),
                                'source_is_repo_patch': v.get('source_is_repo_patch')}
                            for k, v in m.get('jolt', {}).items()}}
    else:
        R['h6'] = None

    # Anti-vacuity (W4): every boyko run's own verdict.
    R['anti_vacuity_void'] = [(r['row'], r['W'], r['pass'], (r.get('summary') or {}).get('first_void'))
                              for r in d.recs if r['engine'] == 'boyko'
                              and (not r.get('summary') or r['summary'].get('void_steps', 1) != 0)]

    # Determinism: runs of one simulation carry one pose.
    groups = {}
    for r in d.recs:
        if r['engine'] != 'boyko' or not r.get('summary'):
            continue
        s = r['summary']
        c = s['config']
        key = (s['scene'], s['gap'], s['solver'], c['sleeping'], c['sleep_threshold'], s['steps'])
        pose = r.get('pose')
        digest = sha256(pose) if pose and os.path.isfile(pose) else s['pose_hash']
        groups.setdefault(json.dumps(key), set()).add(digest)
    R['determinism_boyko'] = {k: {'distinct_poses': len(v), 'ok': len(v) == 1} for k, v in groups.items()}
    R['h7_expect_pose'] = [(r['row'], r['W'], r['pass'], r['summary']['expect_pose']) for r in d.runs('J-B')
                           if r.get('summary')]
    jgroups = {}
    for r in d.recs:
        if r['engine'] == 'jolt' and r.get('jolt', {}).get('stat_lines'):
            key = json.dumps([r['role'], r['args']])
            jgroups.setdefault(key, set()).add(r['jolt']['stat_lines'][0]['hash'])
    R['determinism_jolt_per_W'] = {k: {'distinct_hashes': len(v), 'ok': len(v) == 1} for k, v in jgroups.items()}

    # The headline and A/A0.
    head = {}
    for w in WS:
        tb = [d.t(r) for r in d.runs('J-A-d1', w)]
        tj = [d.t(r) for r in d.runs('JOLT-T', w)]
        tj56 = [d.t(r) for r in d.runs('JOLT56-T', w)]
        if not tb and not tj:
            continue
        e = {'boyko_T_ns': median(tb), 'jolt_T_ns': median(tj), 'jolt56_T_ns': median(tj56)}
        if e['boyko_T_ns'] and e['jolt_T_ns']:
            e['boyko_over_jolt'] = e['boyko_T_ns'] / e['jolt_T_ns']
        if e['boyko_T_ns'] and e['jolt56_T_ns']:
            e['boyko_over_jolt56'] = e['boyko_T_ns'] / e['jolt56_T_ns']
        # Sub-windows (H1): [0, 0.2 N) and [0.2 N, N).
        for name, part in (('sub_early', 0), ('sub_late', 1)):
            vals_b, vals_j = [], []
            for r in d.runs('J-A-d1', w):
                n = window_of(r)[1]
                cut = int(round(n * SUBWINDOW_FRAC))
                vals_b.append(d.t(r, window=(0, cut) if part == 0 else (cut, n)))
            for r in d.runs('JOLT-T', w):
                n = window_of(r)[1]
                cut = int(round(n * SUBWINDOW_FRAC))
                vals_j.append(d.t(r, window=(0, cut) if part == 0 else (cut, n)))
            e[name] = {'boyko_T_ns': median(vals_b), 'jolt_T_ns': median(vals_j)}
        # H9: time per manifold-sweep (12 sweeps each).
        mb = median([d.t(r, 'manifolds') for r in d.runs('J-A-d1', w)])
        if mb and e['boyko_T_ns']:
            e['boyko_ns_per_manifold_sweep'] = e['boyko_T_ns'] / (mb * 12)
        # A/A0.
        per_pass = []
        for p in P:
            r1, r2 = d.one('J-A-d1', w, p), d.one('J-A-d2', w, p)
            if r1 and r2 and d.wall(r1) and d.wall(r2):
                per_pass.append(paired_median_ratio(d.wall(r2), d.wall(r1)))
        e['aa0_per_pass'] = per_pass
        e['B'] = max(abs(x - 1) for x in per_pass) if per_pass else None
        e['aa0_void'] = e['B'] is not None and e['B'] > AA0_CEILING
        head[w] = e
    ts = {w: head[w]['boyko_T_ns'] for w in head}
    tj = {w: head[w]['jolt_T_ns'] for w in head}
    for w in head:
        head[w]['boyko_scaling'] = ts[1] / ts[w] if ts.get(1) and ts.get(w) else None
        head[w]['jolt_scaling'] = tj[1] / tj[w] if tj.get(1) and tj.get(w) else None
    R['headline'] = head

    def band(w):
        return max(head.get(w, {}).get('B') or 0.0, AA1_FLOOR)

    def aa_stat(num_row, den_row, w):
        vals = []
        for p in P:
            rn, rd = d.one(num_row, w, p), d.one(den_row, w, p)
            if rn and rd and d.wall(rn) and d.wall(rd):
                vals.append(paired_median_ratio(d.wall(rn), d.wall(rd)))
        return vals

    # A/A1: armed against disarmed.
    R['aa1'] = {}
    for w in head:
        v = aa_stat('J-A-a', 'J-A-d1', w)
        s = median(v)
        R['aa1'][w] = {'per_pass': v, 'median': s, 'bound': band(w),
                       'pass': s is not None and abs(s - 1) <= band(w)}
    # A/A2: the disarmed tip against the pre-instrument runner.
    R['aa2'] = {}
    for w in (1, 8):
        v = aa_stat('J-A-d1', 'AA2-pre', w)
        s = median(v)
        R['aa2'][w] = {'per_pass': v, 'median': s, 'bound': band(w),
                       'pass': s is not None and abs(s - 1) <= band(w)}
    # The shipping row (O6): reported beside the parity row.
    R['shipping'] = {w: {'T_ns': median([d.t(r) for r in d.runs('SHIP', w)]),
                         'vs_tip_dev': median(aa_stat('SHIP', 'J-A-d1', w))} for w in (1, 8)}

    # The canary.
    R['canary'] = {}
    for w in (1, 8):
        spans, rises, stats, canary_ns = [], [], [], []
        for p in P:
            rc, ra = d.one('J-C', w, p), d.one('J-A-a', w, p)
            if not (rc and ra and rc.get('summary')):
                continue
            cn = rc['summary'].get('canary_ns')
            canary_ns.append(cn)
            sp = d.t(rc, 'sys_parity_canary_ns')
            if sp is not None and cn:
                spans.append(sp / cn - 1)
            tc, ta = d.t(rc), d.t(ra)
            if tc is not None and ta is not None and cn:
                rises.append((tc - ta - cn) / ta)
            if d.wall(rc) and d.wall(ra):
                stats.append(paired_median_ratio(d.wall(rc), d.wall(ra)))
        s = median(stats)
        R['canary'][w] = {
            'canary_ns': canary_ns,
            'span_rel_err': median(spans),
            'span_ok': median(spans) is not None and abs(median(spans)) <= CANARY_SPAN_TOL,
            'rise_rel_err_of_T': median(rises),
            'rise_ok': median(rises) is not None and abs(median(rises)) <= (head.get(w, {}).get('B') or 0.0),
            'aa1_stat': s,
            'aa1_gate_fails_as_required': s is not None and abs(s - 1) > band(w)}
        R['canary'][w]['pass'] = R['canary'][w]['span_ok'] and R['canary'][w]['rise_ok'] and \
            R['canary'][w]['aa1_gate_fails_as_required']

    # The profile: per-span medians, closure and the identity (plan §2, O2).
    serial_sys = ['physics_integrate', 'physics_gather', 'select_broadphase', 'physics_broadphase',
                  'physics_narrowphase', 'physics_build_graph', 'physics_apply']
    in_solve_serial = ['phys_solve_build', 'phys_gravity', 'phys_warm_apply', 'phys_integrate', 'phys_restitution',
                       'phys_store', 'phys_write_back', 'phys_sleep_begin', 'phys_sleep_freeze', 'phys_sleep_end']

    def span_medians(row, w):
        res = {}
        runs = d.runs(row, w)
        if not runs or d.cols(runs[0]) is None:
            return None
        cols = [c for c in d.cols(runs[0]) if c.endswith('_ns') or c in (
            'waves', 'colors', 'wide_colors', 'manifolds', 'pairs', 'phys_slots_wide', 'phys_slots_narrow',
            'phys_np_points', 'disp_lane', 'worker_lane_max')]
        for c in cols:
            res[c] = median([d.t(r, c) for r in runs])
        return res

    prof = {}
    for w in WS:
        sm = span_medians('J-A-a', w)
        if sm:
            prof[w] = sm
    R['profile_J-A'] = prof
    closure = {}
    for w, sm in prof.items():
        solve = sm.get('sys_physics_solve_colored_ns')
        u, g = sm.get('u_ns'), sm.get('g_ns')
        drops = [r['summary'].get('drops_total') for r in d.runs('J-A-a', w) if r.get('summary')]
        closure[w] = {'u_ns': u, 'u_share_of_solve': u / solve if u is not None and solve else None,
                      'u_bad': u is not None and solve is not None and (u < 0 or u > U_CEILING * solve),
                      'g_ns': g, 'g_bad': g is not None and g < 0,
                      'sys_sum_over_wall': (sm.get('sys_sum_ns') or 0) > (sm.get('wall_ns') or float('inf')),
                      'drops': drops, 'drops_bad': any(x for x in drops if x),
                      'r_ns': sm.get('r_ns')}
    # Waves: per step identical across W within each pass (the class does not depend on W).
    wave_mismatch = []
    for p in P:
        seqs = {}
        for w in WS:
            r = d.one('J-A-a', w, p)
            c = d.cols(r) if r else None
            if c and 'waves' in c:
                seqs[w] = c['waves']
        base = next(iter(seqs.values()), None)
        for w, s in seqs.items():
            if s != base:
                wave_mismatch.append((p, w))
    R['closure'] = {'per_W': closure, 'waves_differ_across_W': wave_mismatch}
    R['closure']['stop'] = bool(wave_mismatch) or any(
        v['u_bad'] or v['g_bad'] or v['sys_sum_over_wall'] or v['drops_bad'] for v in closure.values())

    ident = {}
    if 1 in prof:
        s1 = prof[1]

        def S(sm):
            return sum(sm.get(f'sys_{n}_ns') or 0 for n in serial_sys) + \
                sum(sm.get(f'{n}_ns') or 0 for n in in_solve_serial) + (sm.get('phys_color_narrow_ns') or 0)
        S1 = S(s1)
        P1 = s1.get('phys_color_wide_ns') or 0
        T1 = s1.get('wall_ns')
        T8 = prof.get(8, {}).get('wall_ns')
        ident['S1_ns'] = S1
        ident['P1_ns'] = P1
        ident['f'] = S1 / T1 if T1 else None
        ident['f_fit'] = (T8 / T1 - 1 / 8) / (1 - 1 / 8) if T1 and T8 else None
        p1 = span_medians('J-P1', 1)
        if p1 and p1.get('waves'):
            ident['omega1_ns'] = ((p1.get('phys_color_wide_ns') or 0) - P1) / p1['waves']
        for w, sm in prof.items():
            wide = sm.get('phys_color_wide_ns') or 0
            waves = sm.get('waves') or 0
            L = wide - P1 / w
            terms = {'T': sm.get('wall_ns'), 'S': S(sm), 'I': S(sm) - S1, 'P1_over_W': P1 / w, 'L': L,
                     'g': sm.get('g_ns'), 'u': sm.get('u_ns'), 'r': sm.get('r_ns'), 'waves': waves,
                     'E': P1 / (w * wide) if wide else None, 'omega': L / waves if waves else None}
            parts = [S1, terms['I'], terms['P1_over_W'], L, terms['g'] or 0, terms['u'] or 0, terms['r'] or 0]
            terms['identity_residual'] = (terms['T'] - sum(parts)) if terms['T'] is not None else None
            ident[w] = terms
        if 8 in ident and ident.get('omega1_ns') is not None and ident[8]['L']:
            ident['L6_fork_fixed_wave_cost_over_L8'] = ident['omega1_ns'] * ident[8]['waves'] / ident[8]['L']
        ident['imbalance_share'] = 'not measured: no zone may sit inside a worker chunk task (O1)'
    R['identity'] = ident

    # R rows, S16, J-B and the Jolt stage shares.
    R['rows'] = {}
    for row, ws in (('R', (1, 8)), ('R-ref', (1,)), ('R-S', (1, 8)), ('S16', (1, 8)), ('J-B', (1, 8)),
                    ('J-Son', (1, 8)), ('J-S0', (1,))):
        R['rows'][row] = {w: span_medians(row, w) for w in ws}
    R['rs_void'] = [(r['W'], r['pass'], (r.get('summary') or {}).get('first_void')) for r in d.runs('R-S')
                    if r.get('summary') and r['summary']['void_steps']]
    R['rs_first_frozen_step'] = [(r['W'], r['pass'], r['summary'].get('first_frozen_step')) for r in d.runs('R-S')
                                 if r.get('summary')]
    R['jolt_stage_share_profiles'] = [(r['W'], r['pass'], [f for f in r.get('files', []) if f.endswith('.html')])
                                      for r in d.runs('JOLT-P')]
    npc = median([d.t(r) for r in d.runs('JOLT-NPC', 8)])
    R['delta_J_pair_cache_ns'] = (npc - head[8]['jolt_T_ns']) if npc and head.get(8, {}).get('jolt_T_ns') else None

    # L1's gate (O7): pre-L1 twice around the tip, J-S0 at W=1, disarmed.
    l1 = []
    for p in P:
        a1, b, a2 = d.one('L1-A1', 1, p), d.one('L1-B', 1, p), d.one('L1-A2', 1, p)
        if a1 and b and a2 and d.wall(a1) and d.wall(b) and d.wall(a2):
            aa = abs(paired_median_ratio(d.wall(a2), d.wall(a1)) - 1)
            avg = [(x + y) / 2 for x, y in zip(d.wall(a1), d.wall(a2))]
            l1.append({'pass': p, 'aa_spread': aa, 'b_over_a': paired_median_ratio(d.wall(b), avg)})
    if l1:
        ba = median([x['b_over_a'] for x in l1])
        spread = max(x['aa_spread'] for x in l1)
        R['l1_gate'] = {'per_pass': l1, 'b_over_a': ba, 'aa_spread': spread, 'pass': ba <= 1 + spread}
    else:
        R['l1_gate'] = None

    # H8: manifolds over the resting window, both sides.
    rc = d.runs('JOLT-RCPT', 1)
    jcols = load_csv(rc[0].get('receipt_csv')) if rc else None
    bj = d.runs('J-A-d1', 1)
    bcols = d.cols(bj[0]) if bj else None
    if jcols and bcols:
        n = min(len(jcols['Manifolds']), len(bcols['manifolds']))
        lo = int(round(n * SUBWINDOW_FRAC))
        mj, mb = mean(jcols['Manifolds'][lo:n]), mean(bcols['manifolds'][lo:n])
        R['h8'] = {'window': [lo, n], 'jolt_manifolds': mj, 'boyko_manifolds': mb,
                   'rel_diff': (mb - mj) / mj if mj else None,
                   'per_manifold_needed': bool(mj and abs(mb - mj) / mj > H8_TOLERANCE),
                   'jolt_top_y_last': jcols['Top Y'][n - 1], 'boyko_top_y_last': bcols['top_y'][n - 1]}
    else:
        R['h8'] = None

    # O5: the sleeping floors, compared only on the tails after everything is asleep.
    o5 = {}
    for w in (1, 8):
        bs, js = d.runs('J-Son', w), d.runs('JOLT-SLP', w)
        if not bs or not js:
            continue
        bc, jc = d.cols(bs[0]), d.cols(js[0])
        fb = next((i for i, x in enumerate(bc.get('awake') or []) if x == 0), None)
        fj = next((i for i, x in enumerate(jc.get('awake') or []) if x == 0), None)
        e = {'boyko_all_asleep_from': fb, 'jolt_all_asleep_from': fj}
        if fb is not None and fj is not None:
            start = max(fb, fj)
            e['tail'] = [start, len(bc['wall_ns'])]
            e['boyko_tail_T_ns'] = median([wmean(d.cols(r)['wall_ns'], (start, len(bc['wall_ns']))) for r in bs])
            e['jolt_tail_T_ns'] = median([wmean(d.cols(r)['wall_ns'], (start, len(jc['wall_ns']))) for r in js])
        o5[w] = e
    R['o5'] = o5

    # Thread counts (rulings, open question 2).
    R['threads'] = {w: {'boyko_pool_workers': w, 'boyko_dispatcher': 1,
                        'boyko_solve_on_dispatcher_steps': [r['summary']['threads']['solve_on_dispatcher_steps']
                                                            for r in d.runs('J-A-a', w) if r.get('summary')],
                        'jolt_threads_printed': [r['jolt']['stat_lines'][0]['threads'] for r in d.runs('JOLT-T', w)
                                                 if r.get('jolt', {}).get('stat_lines')],
                        'jolt_job_threads': w - 1, 'jolt_calling_thread': 1} for w in WS}

    with open(os.path.join(out, 'reduction.json'), 'w', encoding='utf-8') as f:
        json.dump(R, f, indent=1, default=str)
    text = render(R, a.dry)
    with open(os.path.join(out, 'reduction.txt'), 'wb') as f:
        f.write(text.encode('utf-8'))
    sys.stdout.buffer.write(text.encode('utf-8'))


def render(R, dry):
    L = []
    P = L.append
    banner = 'DRY RUN: every number below is a rehearsal artefact, NOT a measurement' if dry else ''
    if banner:
        P(banner)
    h6 = R.get('h6')
    if h6:
        P(f'H6: head {h6["head"]} (dirty paths {h6["dirty_paths"]}), host {h6["host"]} (msvc {h6["host_is_msvc"]}), '
          f'profile {h6["profile"]}, ancestors '
          f'{ {k: v["is_ancestor"] for k, v in (h6["ancestors"] or {}).items()} }')
        for k, v in h6['jolt'].items():
            P(f'  jolt {k}: {v["sha256"][:16]} {v["source"]} repo patch {v["source_is_repo_patch"]} ({v["compiler"]})')
        for k, v in h6['boyko'].items():
            P(f'  boyko {k}: {v[:16]}')
    P(f'passes {R["passes"]}; runs {R["runs"]}; nonzero exits {len(R["nonzero_exits"])}; '
      f'contaminated {len(R["contaminated"])}; build checks {len(R["build_checks"])}')
    for x in R['nonzero_exits']:
        P(f'  exit: {x}')
    for x in R['build_checks']:
        P(f'  build: {x}')
    P(f'anti-vacuity: {len(R["anti_vacuity_void"])} void boyko runs')
    for x in R['anti_vacuity_void']:
        P(f'  void: {x}')
    bad = [k for k, v in R['determinism_boyko'].items() if not v['ok']]
    P(f'determinism (boyko): {len(R["determinism_boyko"])} simulation groups, {len(bad)} with more than one pose')
    for k in bad:
        P(f'  differs: {k}')
    P(f'H7 (cfg-B asserted against cfg-A by the runner): {R["h7_expect_pose"]}')
    badj = [k for k, v in R['determinism_jolt_per_W'].items() if not v['ok']]
    P(f'determinism (Jolt, per binary+flags+W across passes): {len(R["determinism_jolt_per_W"])} groups, '
      f'{len(badj)} differ')
    P(f'closure: stop={R["closure"]["stop"]}; waves differ across W in {R["closure"]["waves_differ_across_W"]}')
    for w, v in R['closure']['per_W'].items():
        P(f'  W={w}: u_bad {v["u_bad"]} g_bad {v["g_bad"]} sys>wall {v["sys_sum_over_wall"]} drops {v["drops"]}')
    for w, v in R['aa1'].items():
        P(f'A/A1 W={w}: pass {v["pass"]}' + ('' if dry else f' (median {v["median"]}, bound {v["bound"]})'))
    for w, v in R['aa2'].items():
        P(f'A/A2 W={w}: pass {v["pass"]}' + ('' if dry else f' (median {v["median"]}, bound {v["bound"]})'))
    for w, v in R['canary'].items():
        P(f'canary W={w}: span_ok {v["span_ok"]} rise_ok {v["rise_ok"]} gate_fails {v["aa1_gate_fails_as_required"]}'
          f' -> pass {v["pass"]}')
    P(f'L1 gate: {"not run" if R["l1_gate"] is None else ("pass " + str(R["l1_gate"]["pass"]))}')
    P(f'R-S void: {R["rs_void"]}; first frozen step: {R["rs_first_frozen_step"]}')
    if R['h8']:
        P(f'H8: window {R["h8"]["window"]}; per-manifold figure needed: {R["h8"]["per_manifold_needed"]}'
          + ('' if dry else f' (jolt {R["h8"]["jolt_manifolds"]}, boyko {R["h8"]["boyko_manifolds"]})'))
    for w, v in R['o5'].items():
        P(f'O5 W={w}: all asleep from boyko {v["boyko_all_asleep_from"]}, jolt {v["jolt_all_asleep_from"]}')
    for w, v in R['threads'].items():
        P(f'threads W={w}: boyko {v["boyko_pool_workers"]} workers + 1 dispatcher (solve on dispatcher, steps per '
          f'pass: {v["boyko_solve_on_dispatcher_steps"]}); Jolt {v["jolt_job_threads"]} job threads + calling thread '
          f'(printed {v["jolt_threads_printed"]})')
    if not dry:
        for w, v in R['headline'].items():
            P(f'headline W={w}: boyko {v["boyko_T_ns"]} ns, Jolt {v["jolt_T_ns"]} ns, ratio '
              f'{v.get("boyko_over_jolt")}, B(W) {v["B"]} (void {v["aa0_void"]}), v5.6 ratio {v.get("boyko_over_jolt56")}')
        ident = R['identity']
        P(f'identity: f {ident.get("f")} f_fit {ident.get("f_fit")} omega1 {ident.get("omega1_ns")} '
          f'L6 fork {ident.get("L6_fork_fixed_wave_cost_over_L8")}')
    else:
        P(f'headline rows present at W: {sorted(R["headline"])}; A/A0 void flags computed: '
          f'{sorted(w for w, v in R["headline"].items() if v["B"] is not None)}')
        P(f'identity terms computed at W: {sorted(k for k in R["identity"] if isinstance(k, int))}')
    return '\n'.join(L) + '\n'


# ── main ──────────────────────────────────────────────────────────────────────


def main():
    # Progress lines reach a redirected log as they happen, not at exit.
    sys.stdout.reconfigure(line_buffering=True)
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    sub = ap.add_subparsers(dest='cmd', required=True)
    m = sub.add_parser('manifest')
    m.add_argument('--out', required=True)
    m.add_argument('--repo', required=True)
    m.add_argument('--boyko', action='append')
    m.add_argument('--jolt', action='append')
    m.add_argument('--ancestor', action='append')
    m.add_argument('--toolchain', default='stable-x86_64-pc-windows-msvc')
    m.add_argument('--profile', default='parity')
    p = sub.add_parser('pass')
    p.add_argument('--manifest', required=True)
    p.add_argument('--out', required=True)
    p.add_argument('--pass', dest='pass_no', type=int, required=True)
    p.add_argument('--dry', action='store_true')
    p.add_argument('--only')
    p.add_argument('--ws')
    p.add_argument('--receipt-window', type=float, default=2.0)
    p.add_argument('--busy-pct', type=float, default=5.0)
    p.add_argument('--wait-budget', type=float, default=420.0)
    r = sub.add_parser('reduce')
    r.add_argument('--out', required=True)
    r.add_argument('--dry', action='store_true')
    a = ap.parse_args()
    {'manifest': cmd_manifest, 'pass': cmd_pass, 'reduce': cmd_reduce}[a.cmd](a)


if __name__ == '__main__':
    main()
