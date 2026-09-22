"""WINDOW 3 (L4+L2 default against the P0b bridge and Jolt) under P0b's ruled protocol.

Adapted from P0b's window_run.py (same launch path, receipts, witnesses, ordering and record layout);
the row list comes from win3/rows.json (+ rows_l5.json if the C4 runner exists), not from the driver's
P0b table. Nothing is compiled here. Binaries are verified against bin/SHA256SUMS before each pass.

Protocol (P0b ruling, binding):
  * a (row, W) cell's statistic is the MEDIAN over K = 6 separate processes of the process's window mean;
    resolution = the process-to-process spread (min-max, IQR; the median's SE supplementary);
  * a 5-s load receipt between every two processes (the AFTER of one, the BEFORE of the next); a 10-s
    receipt opens each pass; > 5 % busy (or a build process using CPU) => that process is re-run ONCE
    at the end of its pass;
  * no band-based void; placement P-none (CreateProcess suspended -> read the mask back -> resume; no
    SetProcessAffinityMask call).

Order: two passes; each pass runs the full cell list three times (rounds). Pass 0: W ascending, and
inside a W group Jolt first, Jolt and boyko alternating. Pass 1: the whole list reversed. One untimed
warm-up process (runner_l4l2 --cfg default, W=8) opens each pass.

Idle rule (the task's step C): tools/wait_idle3.ps1 before each pass -- 0 build-named processes AND 0
processes running from D:/wt/_targets or D:/wt/mq-*, on 3 consecutive 60-s polls, with the 10-s CPU
average < 5 %. Up to 3 h; never idle => STOP and report.

After the passes: two UNTIMED Jolt `-receipt` runs (v5.3.0, v5.6.0; W=1, 500 steps) into raw/receipts/,
for Jolt's own manifold counts (not in runs.jsonl; not measurements).

WIN3_TEST=1 is an UNTIMED rehearsal (dry step counts, one round, no idle wait) under win3/test/.
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
W3 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
sys.path.insert(0, os.path.join(HERE, 'window_lib'))
import driver as D  # noqa: E402  (a byte-identical copy of the tree driver, checked in main)
from pdhperf import PerfSampler  # noqa: E402

TREE_DRIVER = r'D:/wt/joltab/tools/physics_parity/driver.py'
WAIT_PS1 = os.path.join(HERE, 'wait_idle3.ps1')
TEST = os.environ.get('WIN3_TEST') == '1'
RAW = os.path.join(W3, 'test', 'window') if TEST else os.path.join(W3, 'raw')
WAIT_LOG = os.path.join(RAW, 'wait_log.txt')
BIN = os.path.join(W3, 'bin')
BUSY_PCT = 5.0
RECEIPT_S = 0.5 if TEST else 5.0
PASS_RECEIPT_S = 0.5 if TEST else 10.0
QUIET_BUDGET_S = 420.0
IDLE_BUDGET_S = 3 * 3600.0
PASSES = 1 if TEST else 2
ROUNDS = 1 if TEST else 3
J_POSE = '0x32d5e235342b4143'

# -- rows from rows.json -------------------------------------------------------

ROWS_JSON = json.load(open(os.path.join(W3, 'rows.json'), encoding='utf-8'))
L5_JSON = os.path.join(W3, 'rows_l5.json')
ROW_DEFS = list(ROWS_JSON['rows'])
if os.path.isfile(L5_JSON):
    ROW_DEFS += json.load(open(L5_JSON, encoding='utf-8'))['rows']
ROLE_OF = {'D-L4L2': 'l4l2', 'J-A': 'tip', 'JOLT-T': 'v530', 'JOLT56-T': 'v560', 'D-L5': 'l5', 'D-L5-npoff': 'l5',
           'D-L5-npon': 'l5'}
ROWS, EXPECT_POSE, EXPECT_JOLT_HASH, MANIFEST_ENTRIES = {}, {}, {}, {'boyko': {}, 'jolt': {}}
for rd in ROW_DEFS:
    role = rd.get('role') or ROLE_OF[rd['id']]
    exe = rd['exe'] if os.path.isabs(rd['exe']) else os.path.join(W3, rd['exe'].replace('bin/', 'bin' + os.sep, 1))
    if rd['engine'] == 'boyko':
        r = D.B(rd['id'], role, tuple(rd['workers']), list(rd['args']), steps=rd['steps'], window=tuple(rd['window']),
                armed=rd.get('armed', False), expect_pose_of=rd.get('expect_pose_of'))
        EXPECT_POSE[rd['id']] = rd['expected_pose']
        MANIFEST_ENTRIES['boyko'][role] = {'exe': exe, 'sha256': rd['sha256'], 'commit': rd.get('commit')}
    else:
        r = D.J(rd['id'], role, tuple(rd['workers']), list(rd['args']), steps=rd['steps'])
        EXPECT_JOLT_HASH[rd['id']] = rd['expected_hash_500']
        MANIFEST_ENTRIES['jolt'][role] = {'exe': exe, 'sha256': rd['sha256']}
    r['what'] = rd.get('what')
    ROWS[rd['id']] = r
D.ROWS.update(ROWS)  # so D.window_of / D.ROWS lookups in downstream tools resolve these ids

BOYKO_IDS = [r['id'] for r in ROW_DEFS if r['engine'] == 'boyko']
JOLT_IDS = [r['id'] for r in ROW_DEFS if r['engine'] == 'jolt']
BOYKO_CELLS = {w: [rid for rid in BOYKO_IDS if w in ROWS[rid]['ws']] for w in (1, 2, 4, 8, 16)}
JOLT_CELLS = {w: [rid for rid in JOLT_IDS if w in ROWS[rid]['ws']] for w in (1, 2, 4, 8, 16)}
TEST_CELLS = {('D-L4L2', 1), ('J-A', 1), ('JOLT-T', 1), ('JOLT56-T', 8), ('D-L4L2', 8)}

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
    """P-none launch, identical to the P0b diagnostic's: CreateProcess suspended, read the (inherited) mask
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
    """(mean_ms, median_ms, n, expect_n) of the per-step wall time over the row's window.
    boyko: run.csv wall_ns over the row window; Jolt: per_frame 'Time (ms)' over all frames."""
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
            if row.get('expect_pose_of') and s.get('expect_pose') not in ('match', 'none'):
                why.append(f'expect_pose {s.get("expect_pose")}')
            if s.get('disarmed_ring_traffic'):
                why.append(f'disarmed_ring_traffic {s.get("disarmed_ring_traffic")}')
    else:
        st = rec.get('jolt', {}).get('stat_lines') or []
        if not st:
            why.append('no Jolt stat line')
        else:
            if st[0]['threads'] != w:
                why.append(f'threads {st[0]["threads"]} != {w}')
            if not TEST and st[0]['hash'] != EXPECT_JOLT_HASH.get(row['id']):
                why.append(f'hash {st[0]["hash"]} != {EXPECT_JOLT_HASH.get(row["id"])}')
    return why


def run_one(m, rid, w, pass_no, rnd, seq, tag, pdir, ctx, receipt, env, sampler, deadline):
    row = ROWS[rid]
    entry = (m['boyko'] if row['engine'] == 'boyko' else m['jolt'])[row['role']]
    suffix = '' if tag == 'original' else f'_{tag}'
    cwd = os.path.join(pdir, f'{seq:03d}_r{rnd}_{rid}_W{w}{suffix}')
    os.makedirs(cwd, exist_ok=True)
    rec = {'pass': pass_no, 'round': rnd, 'g': pass_no * ROUNDS + rnd, 'seq': seq, 'row': rid, 'engine': row['engine'],
           'role': row['role'], 'W': w, 'dry': TEST, 'attempt': tag, 'policy': 'P-none'}
    if row['engine'] == 'boyko':
        args, why = D.boyko_args(row, w, TEST, cwd, ctx)
        if row['expect_pose_of']:
            ref = ctx.get((row['expect_pose_of'], w))
            rec['expect_pose_ref'] = {'pass': ref['pass'], 'round': ref['round'], 'seq': ref['seq']} if ref else None
    else:
        args, why = D.jolt_args(row, w, TEST)
    if args is None:
        rec['skipped'] = why
        return rec, receipt, 'ok'
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
    elif rec.get('jolt', {}).get('stat_lines'):
        st = rec['jolt']['stat_lines'][0]
        extra = f' hash {st["hash"]} threads {st["threads"]} sps {st["steps_per_s"]}'
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
    """Pass-0 order of one round: W ascending; inside a W group Jolt first, Jolt and boyko alternating."""
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


def run_receipts(m, env):
    """UNTIMED Jolt -receipt runs (both exes, W=1, 500 steps): Jolt's own manifold/point counts per frame."""
    rdir = os.path.join(RAW, 'receipts')
    os.makedirs(rdir, exist_ok=True)
    out = []
    steps = 12 if TEST else 500
    for role, rid in (('v530', 'JOLT-T'), ('v560', 'JOLT56-T')):
        e = m['jolt'].get(role)
        if not e:
            continue
        cwd = os.path.join(rdir, f'{rid}_receipt_W1')
        os.makedirs(cwd, exist_ok=True)
        args = ['-s=Pyramid', '-q=Discrete', '-receipt', '-t=1', f'-i={steps}']
        rec = {'row': rid, 'role': role, 'engine': 'jolt', 'W': 1, 'timed': False, 'attempt': 'receipt',
               'exe': e['exe'], 'exe_sha256': e['sha256'], 'args': args, 'cwd': cwd, 'start': D.now()}
        info, so, se = launch([e['exe']] + args, cwd, env, None)
        rec.update(info)
        rec['end'] = D.now()
        with open(os.path.join(cwd, 'stdout.txt'), 'wb') as f:
            f.write(so)
        with open(os.path.join(cwd, 'stderr.txt'), 'wb') as f:
            f.write(se)
        rec['jolt'] = D.parse_jolt_stdout(so)
        rec['files'] = sorted(os.listdir(cwd))
        rc = [f for f in rec['files'] if f.startswith('receipt_')]
        rec['receipt_csv'] = os.path.join(cwd, rc[0]) if rc else None
        c = D.load_csv(rec['receipt_csv']) if rec['receipt_csv'] else None
        if c and 'Manifolds' in c:
            man = c['Manifolds']
            rec['manifolds'] = {'n_frames': len(man), 'mean_0_500': D.mean(man[0:500]), 'mean_0_100': D.mean(man[0:100]),
                                'mean_100_500': D.mean(man[100:500]), 'final': man[-1],
                                'points_mean_100_500': D.mean(c['Points'][100:500]),
                                'active_bodies_min': min(x for x in c['Active Bodies'] if x is not None)}
        out.append(rec)
        st = rec['jolt']['stat_lines']
        log(f'  receipt {rid}: exit {rec["exit"]} hash {st[0]["hash"] if st else None} manifolds {rec.get("manifolds")}')
    with open(os.path.join(rdir, 'receipts.json'), 'w', encoding='utf-8') as f:
        json.dump(out, f, indent=1)
    return out


def main():
    os.makedirs(RAW, exist_ok=True)
    if D.sha256(os.path.join(HERE, 'lib', 'driver.py')) != D.sha256(TREE_DRIVER):
        sys.exit('lib/driver.py differs from the tree driver')
    m = {'generated': D.now(), 'rows_json': os.path.join(W3, 'rows.json'),
         'rows_l5_json': L5_JSON if os.path.isfile(L5_JSON) else None,
         'toolchain': 'stable-x86_64-pc-windows-msvc', 'profile': 'parity', 'logical_cpus': NCPU,
         'boyko': MANIFEST_ENTRIES['boyko'], 'jolt': MANIFEST_ENTRIES['jolt'],
         'rows': {rid: {k: v for k, v in r.items() if k != 'per_w'} for rid, r in ROWS.items()},
         'expect_pose': EXPECT_POSE, 'expect_jolt_hash': EXPECT_JOLT_HASH, 'protocol': ROWS_JSON['protocol']}
    with open(os.path.join(RAW, 'manifest.json'), 'w', encoding='utf-8') as f:
        json.dump(m, f, indent=1, default=str)
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(k, None)
    order0 = cell_order()
    state = {'start': D.now(), 'test': TEST, 'placement': 'P-none (no affinity call)', 'cells_per_round': len(order0),
             'order_pass0': [f'{r}@W{w}' for r, w in order0], 'passes': PASSES, 'rounds': ROUNDS,
             'machine_before': machine_state()}
    log(f'WINDOW START {json.dumps(state)}')
    deadline = time.time() + IDLE_BUDGET_S
    status = 'complete'
    runs_path = os.path.join(RAW, 'runs.jsonl')
    ctx = {}
    sampler = PerfSampler(1.0)
    for p in range(PASSES):
        log(f'pass {p}: idle rule (wait_idle3.ps1)')
        rc = wait_idle(deadline)
        if rc != 0:
            status = f'STOP before pass {p}: idle rule not met (wait_idle exit {rc})'
            break
        log(f'pass {p}: idle reached; {machine_state()}')
        bad = verify_binaries(m)
        if bad:
            status = f'STOP before pass {p}: binaries changed {bad}'
            break
        log(f'pass {p}: binaries verified against the manifest and bin/SHA256SUMS')
        pdir = os.path.join(RAW, f'pass-{p:02d}')
        os.makedirs(pdir, exist_ok=True)
        order = order0 if p % 2 == 0 else order0[::-1]
        receipt = D.load_receipt(PASS_RECEIPT_S)
        log(f'pass {p}: opening receipt {receipt["cpu_avg"]}% (build procs {receipt["build_procs_busy"]})')
        wrec, receipt, st = run_one(m, 'D-L4L2', 8, p, -1, 0, 'warmup', pdir, {}, receipt, env, sampler, deadline)
        append(runs_path, wrec)
        log(f'  p{p} warm-up D-L4L2 W=8 (untimed): {tag_of(wrec) if st == "ok" else wrec.get("aborted")}')
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
        log('receipts: untimed Jolt -receipt runs')
        try:
            run_receipts(m, env)
        except Exception as e:  # noqa: BLE001 -- the receipts are not a gate
            log(f'receipts FAILED: {e!r}')
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
