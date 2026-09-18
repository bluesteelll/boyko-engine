"""MEASUREMENT-QUEUE timed-run driver: ONE timed run of a PREBUILT criterion bench exe.

Order: verify exe sha256 against exes.json -> load receipt BEFORE (poll every 60 s while the
10 s average is above 5 %) -> run `<exe> --bench --noplot <filter>` with cwd = the arm's
crates/boyko_physics and CARGO_TARGET_DIR = the arm's target dir (never through cargo, so
nothing can compile) -> load receipt AFTER -> copy every criterion `new/` dir written by this
run into raw/<entry>/<arm><n>/<dir>/ and append one JSON record to runs.jsonl.

usage: run_one.py <entry> <tag> <A|B> <n> <sha8> <bench> [--waited S] -- <criterion filter args...>
"""
import datetime, glob, hashlib, json, os, re, shutil, subprocess, sys, time

MQ = os.path.dirname(os.path.abspath(__file__))
RECEIPT = os.path.join(MQ, 'receipt.ps1')
EXES = json.load(open(os.path.join(MQ, 'exes.json')))
WATCH = re.compile(r'^(cargo|rustc|link|lld|rust-lld|dxc|rust-analyzer|clippy-driver|cargo-miri|miri)$', re.I)
THRESH = 5.0
CALL_WAIT_BUDGET = 420  # seconds of waiting inside one call (the tool call has a 10 min cap)
TOTAL_WAIT_BUDGET = 1200  # 20 min across calls


def receipt():
    out = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', RECEIPT],
                         capture_output=True, text=True, encoding='utf-8', errors='replace')
    r = json.loads(out.stdout.strip().splitlines()[-1])
    # Toolchain / compile processes present at all (the top-5 alone can miss an idle-but-present one).
    tl = subprocess.run(['powershell.exe', '-NoProfile', '-Command',
                         'Get-Process | ForEach-Object { $_.ProcessName }'],
                        capture_output=True, text=True, encoding='utf-8', errors='replace').stdout.split()
    r['watched_present'] = sorted({n for n in tl if WATCH.match(n)})
    return r


SNAP = os.path.join(MQ, 'snap.ps1')


def snap():
    out = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', SNAP],
                         capture_output=True, text=True, encoding='utf-8', errors='replace')
    return json.loads(out.stdout.strip())


def during(s0, s1, wall, bench_pid):
    # CPU time of every OTHER process over the run window (the bench exe itself excluded).
    rows = []
    for pid, (name, cpu) in s1.items():
        if int(pid) == bench_pid:
            continue
        prev = s0.get(pid, [name, 0.0])[1] if s0.get(pid, [None])[0] == name else 0.0
        rows.append((cpu - prev, name, int(pid)))
    rows.sort(reverse=True)
    tot = sum(r[0] for r in rows if r[0] > 0)
    return {'other_cpu_s': round(tot, 3), 'other_pct_of_machine': round(100.0 * tot / (wall * 16), 2),
            'top5': [{'name': n, 'pid': p, 'cpu_s': round(c, 3)} for c, n, p in rows[:5]]}


import ctypes
from ctypes import wintypes
_k32 = ctypes.WinDLL('kernel32', use_last_error=True)


def _ft(f):
    return ((f.dwHighDateTime << 32) | f.dwLowDateTime) / 1e7  # 100 ns units -> s


def system_times():
    # (idle, kernel_incl_idle, user) seconds summed over all logical CPUs since boot.
    i, k, u = wintypes.FILETIME(), wintypes.FILETIME(), wintypes.FILETIME()
    assert _k32.GetSystemTimes(ctypes.byref(i), ctypes.byref(k), ctypes.byref(u))
    return _ft(i), _ft(k), _ft(u)


def process_cpu(handle):
    c, e, k, u = (wintypes.FILETIME() for _ in range(4))
    assert _k32.GetProcessTimes(wintypes.HANDLE(int(handle)), ctypes.byref(c), ctypes.byref(e),
                                ctypes.byref(k), ctypes.byref(u))
    return _ft(k) + _ft(u)


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    argv = sys.argv[1:]
    sep = argv.index('--')
    head, fargs = argv[:sep], argv[sep + 1:]
    waited_before = 0
    if '--waited' in head:
        i = head.index('--waited')
        waited_before = int(head[i + 1])
        del head[i:i + 2]
    entry, tag, arm, n, sha, bench = head
    info = EXES['arms'][sha]
    exe = info['benches'][bench]['exe']
    want = info['benches'][bench]['sha256']
    got = sha256(exe)
    assert got == want, f'exe sha256 mismatch: {got} != {want}'
    cwd = info['worktree'] + '/crates/boyko_physics'
    tdir = info['target_dir']
    label = f'{arm}{n}'
    rec = {'entry': entry, 'tag': tag, 'arm': arm, 'n': int(n), 'sha8': sha, 'commit': info['commit'],
           'bench': bench, 'exe': exe, 'exe_sha256': got, 'cwd': cwd, 'target_dir': tdir,
           'args': ['--bench', '--noplot'] + fargs, 'env': {'CARGO_TARGET_DIR': tdir, 'CRITERION_DEBUG': '1'}}

    # --- receipt BEFORE, with the wait rule ---
    polls = []
    waited = waited_before
    t_wait0 = time.time()
    r = receipt()
    polls.append(r)
    while r['cpu_avg'] > THRESH and waited < TOTAL_WAIT_BUDGET:
        if time.time() - t_wait0 > CALL_WAIT_BUDGET:
            print(json.dumps({'status': 'STILL_BUSY', 'waited_s': waited, 'polls': polls}))
            sys.exit(3)
        time.sleep(48)  # + ~12 s receipt = one poll per ~60 s
        waited = waited_before + int(time.time() - t_wait0)
        r = receipt()
        polls.append(r)
    rec['receipt_before'] = r
    rec['before_polls'] = polls[:-1]
    rec['waited_s'] = waited
    rec['contaminated'] = r['cpu_avg'] > THRESH

    # --- the timed run ---
    env = dict(os.environ)
    for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CRITERION_HOME', 'CARGO', 'CARGO_CRITERION_PORT'):
        env.pop(k, None)
    env.update(rec['env'])
    logbase = os.path.join(MQ, 'runlogs', f'{entry}_{tag}_{label}')
    s0 = snap()
    st0 = system_times()
    start_wall = time.time()
    rec['run_start'] = datetime.datetime.now().astimezone().isoformat(timespec='milliseconds')
    with open(logbase + '.log', 'wb') as fo:
        p = subprocess.Popen([exe, '--bench', '--noplot'] + fargs, cwd=cwd, env=env, stdout=fo,
                             stderr=subprocess.STDOUT)
        bench_pid = p.pid
        p.wait()
        bench_cpu = process_cpu(p._handle)  # handle stays valid until the Popen object is freed
    st1 = system_times()
    rec['run_end'] = datetime.datetime.now().astimezone().isoformat(timespec='milliseconds')
    rec['run_wall_s'] = round(time.time() - start_wall, 2)
    rec['exit_code'] = p.returncode
    s1 = snap()
    rec['during_run'] = during(s0, s1, rec['run_wall_s'], bench_pid)
    idle = st1[0] - st0[0]
    total = (st1[1] - st0[1]) + (st1[2] - st0[2])
    busy = total - idle
    rec['during_run']['system_busy_cpu_s'] = round(busy, 3)
    rec['during_run']['system_total_cpu_s'] = round(total, 3)
    rec['during_run']['bench_cpu_s'] = round(bench_cpu, 3)
    rec['during_run']['non_bench_busy_pct_of_machine'] = round(100.0 * (busy - bench_cpu) / total, 2)
    rec['during_run']['bench_pct_of_machine'] = round(100.0 * bench_cpu / total, 2)

    # --- receipt AFTER ---
    rec['receipt_after'] = receipt()

    # --- parse the log ---
    text = open(logbase + '.log', encoding='utf-8', errors='replace').read()
    postprocess(rec, text, start_wall, tdir, entry, label)
    summary(rec, waited)


def postprocess(rec, text, start_wall, tdir, entry, label):
    ran = []
    warm = {}
    cur = None
    for line in text.splitlines():
        m = re.match(r'Benchmarking (\S+): Warming up', line)
        if m:
            cur = m.group(1)
            if cur not in ran:
                ran.append(cur)
        m = re.match(r'Completed (\d+) iterations in (\d+) nanoseconds', line)
        if m and cur:
            warm[cur] = {'W': int(m.group(1)), 'warm_ns': int(m.group(2))}
    rec['ids_ran'] = ran
    rec['warmup'] = warm
    rec['warnings'] = [l for l in text.splitlines() if 'Warning' in l or 'panicked' in l]
    rec['log_tail'] = text.splitlines()[-12:]

    # --- copy criterion results written by THIS run ---
    rows = {}
    for bj in glob.glob(os.path.join(tdir, 'criterion', '**', 'new', 'benchmark.json'), recursive=True):
        b = json.load(open(bj))
        if b['full_id'] not in ran:
            continue
        newdir = os.path.dirname(bj)
        est = os.path.join(newdir, 'estimates.json')
        mt = os.path.getmtime(est)
        dst = os.path.join(MQ, 'raw', entry, label, b['directory_name'].replace('/', '__'))
        os.makedirs(dst, exist_ok=True)
        for f in os.listdir(newdir):
            shutil.copy2(os.path.join(newdir, f), os.path.join(dst, f))
        e = json.load(open(est))
        s = json.load(open(os.path.join(newdir, 'sample.json')))
        rows[b['full_id']] = {
            'estimates_path': est.replace('\\', '/'),
            'estimates_mtime': datetime.datetime.fromtimestamp(mt).astimezone().isoformat(timespec='milliseconds'),
            'written_by_this_run': mt >= start_wall,
            'copied_to': dst.replace('\\', '/'),
            'median_ns': e['median']['point_estimate'],
            'median_ci': [e['median']['confidence_interval']['lower_bound'],
                          e['median']['confidence_interval']['upper_bound']],
            'ci_level': e['median']['confidence_interval']['confidence_level'],
            'mean_ns': e['mean']['point_estimate'],
            'mean_ci': [e['mean']['confidence_interval']['lower_bound'],
                        e['mean']['confidence_interval']['upper_bound']],
            'std_dev_ns': e['std_dev']['point_estimate'],
            'slope_ns': e['slope']['point_estimate'] if e.get('slope') else None,
            'slope_ci': ([e['slope']['confidence_interval']['lower_bound'],
                          e['slope']['confidence_interval']['upper_bound']] if e.get('slope') else None),
            'sampling_mode': s.get('sampling_mode'),
            'iters': s['iters'],
            'N': int(sum(s['iters'])),
        }
    rec['rows'] = rows
    missing = [i for i in ran if i not in rows or not rows[i]['written_by_this_run']]
    rec['missing_or_stale'] = missing
    with open(os.path.join(MQ, 'runs.jsonl'), 'a', encoding='utf-8') as f:
        f.write(json.dumps(rec) + '\n')



def summary(rec, waited):
    rows, ran, warm, missing = rec['rows'], rec['ids_ran'], rec['warmup'], rec['missing_or_stale']
    entry, tag, label, sha = rec['entry'], rec['tag'], f"{rec['arm']}{rec['n']}", rec['sha8']
    rb, ra = rec['receipt_before'], rec['receipt_after']
    print(f"{entry} {tag} {label} {sha} rc={rec['exit_code']} wall={rec['run_wall_s']}s waited={waited}s "
          f"before={rb['cpu_avg']}% procs={rb['process_count']} watched={rb['watched_present']} | "
          f"after={ra['cpu_avg']}% procs={ra['process_count']} watched={ra['watched_present']}")
    if 'during_run' in rec:
        d = rec['during_run']
        print(f"  during run: system busy minus bench = {d['non_bench_busy_pct_of_machine']}% of machine "
              f"(bench itself {d['bench_pct_of_machine']}%); readable other processes {d['other_cpu_s']} cpu-s:",
              [(t['name'], t['cpu_s']) for t in d['top5']])
    print('  top5 before:', [(t['name'], t['cpu_s']) for t in rb['top5']])
    print('  top5 after :', [(t['name'], t['cpu_s']) for t in ra['top5']])
    for i in ran:
        r_ = rows.get(i)
        if r_:
            print(f"  {i}: median {r_['median_ns']/1e6:.4f} ms CI [{r_['median_ci'][0]/1e6:.4f}, {r_['median_ci'][1]/1e6:.4f}] "
                  f"slope {(r_['slope_ns'] or 0)/1e6:.4f} W={warm.get(i, {}).get('W')} N={r_['N']} mode={r_['sampling_mode']} fresh={r_['written_by_this_run']} mtime={r_['estimates_mtime']}")
        else:
            print(f'  {i}: NO RESULT FILE')
    if rec['warnings']:
        print('  warnings:', rec['warnings'])
    if missing:
        print('  MISSING/STALE:', missing)
    if rec['exit_code'] != 0:
        print('  log tail:', rec['log_tail'])


if __name__ == '__main__':
    main()
