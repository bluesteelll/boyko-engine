"""Window 8 untimed gate (runner, micro, Jolt). Reads NO timing statistic: it checks exits, poses, hashes, voids,
armed/canary receipts and structural counts, and records each process's wall ONLY as the driver's schedule estimate.
  python -B tools/gate8.py fixtures   # record the pose fixtures (J-T, J-T-off, J-A at W1; J-Son at W8)
  python -B tools/gate8.py runner     # every runner row at W in {1,2,4,8,16}, --steps 500, --expect-pose
  python -B tools/gate8.py micro      # omega exe: --help, the smallest configuration, one full-size dry sample per row
  python -B tools/gate8.py jolt       # j56 at W 1/8/16 (-t=1 is the hash gate) and j56p at W 1/8/16
Writes gate/fixtures/, gate/gate_<part>.json, gate/gate_run.log, gate/estimates.json (merged)."""
import json
import os
import re
import shutil
import subprocess
import sys
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
import driver as D  # noqa: E402
sys.path.insert(0, HERE)
import joltprof  # noqa: E402

GATE = os.path.join(W8, 'gate')
FIX = os.path.join(GATE, 'fixtures')
RJ = json.load(open(os.path.join(W8, 'rows8.json'), encoding='utf-8'))
ROWS = {r['id']: r for r in RJ['rows']}
BINS = {k: (e['exe'] if ':' in e['exe'] else os.path.join(W8, e['exe'])) for k, e in RJ['binaries'].items()}
KNOWN = {'JT500': '0x30c5438bc6ad9ffa', 'JToff500': '0x32d5e235342b4143'}
FIX_SRC = {'JT500': ('J-T', 1), 'JToff500': ('J-T-off', 1), 'JA500': ('J-A', 1), 'JSon500': ('J-Son-a', 8)}
WS = [1, 2, 4, 8, 16]
LOG = os.path.join(GATE, 'gate_run.log')
EST = os.path.join(GATE, 'estimates.json')
env = dict(os.environ)
for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
    env.pop(k, None)


def log(msg):
    line = f'{D.now()} {msg}'
    print(line, flush=True)
    with open(LOG, 'a', encoding='utf-8', newline='\n') as f:
        f.write(line + '\n')


def run(cmd, cwd, timeout=900):
    os.makedirs(cwd, exist_ok=True)
    t0 = time.perf_counter()
    p = subprocess.run(cmd, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    wall = round(time.perf_counter() - t0, 3)
    open(os.path.join(cwd, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(cwd, 'stderr.txt'), 'wb').write(p.stderr)
    return p.returncode, p.stdout, p.stderr, wall


def est_update(d):
    cur = json.load(open(EST, encoding='utf-8')) if os.path.exists(EST) else {}
    cur.update(d)
    json.dump(cur, open(EST, 'w', encoding='utf-8', newline='\n'), indent=1, sort_keys=True)


def runner_cmd(row, w, cwd, label, expect=None, pose_out=True):
    a = list(row['args']) + ['--workers', str(w), '--steps', '500', '--window', '0..500',
                             '--csv', os.path.join(cwd, 'run.csv'), '--label', label]
    if pose_out:
        a += ['--pose-out', os.path.join(cwd, 'pose.bin')]
    if row['armed']:
        a.append('--arm-profiler')
    if expect:
        a += ['--expect-pose', expect]
    return [BINS['instr']] + a


def check_runner(row, w, rc, s, cwd, want_hash):
    why = []
    if rc != 0:
        why.append(f'exit {rc}')
    if not s:
        return why + ['no SUMMARY']
    if want_hash and s.get('pose_hash') != want_hash:
        why.append(f'pose {s.get("pose_hash")} != {want_hash}')
    if s.get('workers') != w:
        why.append(f'workers {s.get("workers")}')
    if s.get('target_env') != 'msvc':
        why.append(f'target_env {s.get("target_env")}')
    if s.get('void_steps') != 0:
        why.append(f'void_steps {s.get("void_steps")} first {s.get("first_void")}')
    if bool(s.get('armed')) != bool(row['armed']):
        why.append(f'armed {s.get("armed")}')
    if not row['armed'] and s.get('disarmed_ring_traffic'):
        why.append(f'disarmed_ring_traffic {s.get("disarmed_ring_traffic")}')
    if row['armed'] and s.get('drops_total'):
        why.append(f'drops_total {s.get("drops_total")}')
    if row['armed'] and not s.get('w8s'):
        why.append('armed but no w8s object')
    if not row['armed'] and s.get('w8s') not in (None, 'null'):
        why.append('disarmed but a w8s object')
    cfg = s.get('config') or {}
    if cfg.get('broadphase') != row['broadphase']:
        why.append(f'broadphase {cfg.get("broadphase")} != {row["broadphase"]}')
    bt = s.get('broadphase_tree') or {}
    if row['broadphase'] == 'Tree' and not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
        why.append(f'TreeDiag {bt}')
    if row.get('canary') and not s.get('canary_ns'):
        why.append(f'canary_ns {s.get("canary_ns")}')
    zc = row.get('zone_canary')
    if zc and (s.get('canary_zone') != zc[0] or s.get('canary_zone_ns') != zc[1]):
        why.append(f'canary_zone {s.get("canary_zone")} {s.get("canary_zone_ns")} != {zc}')
    if not zc and s.get('canary_zone') not in (None,):
        why.append(f'unexpected canary_zone {s.get("canary_zone")}')
    return why


def csv_header(cwd):
    p = os.path.join(cwd, 'run.csv')
    if not os.path.exists(p):
        return []
    with open(p, encoding='utf-8') as f:
        return [h.strip() for h in f.readline().split(',')]


def part_fixtures():
    os.makedirs(FIX, exist_ok=True)
    out = {}
    for name, (rid, w) in FIX_SRC.items():
        row = ROWS[rid]
        cwd = os.path.join(GATE, 'fixture_runs', f'{name}_{rid}_W{w}')
        rc, so, se, wall = run(runner_cmd(row, w, cwd, f'gate-fixture:{name}'), cwd)
        s = D.parse_summary(so)
        why = check_runner(row, w, rc, s, cwd, KNOWN.get(name))
        pb = os.path.join(cwd, 'pose.bin')
        if not why and os.path.exists(pb):
            shutil.copyfile(pb, os.path.join(FIX, f'{name}.pose'))
        out[name] = {'hash': (s or {}).get('pose_hash'), 'known': KNOWN.get(name), 'recorded_by': f'{rid}@W{w}',
                     'args': runner_cmd(row, w, 'CWD', 'L')[1:], 'sha256': D.sha256(pb) if os.path.exists(pb) else None,
                     'fail': why}
        log(f'fixture {name} ({rid} W{w}): pose {out[name]["hash"]} known {KNOWN.get(name)} '
            f'{"OK" if not why else "FAIL " + "; ".join(why)}')
    json.dump(out, open(os.path.join(FIX, 'fixtures.json'), 'w', encoding='utf-8', newline='\n'), indent=1)


def part_runner():
    fx = json.load(open(os.path.join(FIX, 'fixtures.json'), encoding='utf-8'))
    recs, est = [], {}
    seen_args = {}
    for rid, row in ROWS.items():
        if row['kind'] != 'runner':
            continue
        for w in WS:
            cwd = os.path.join(GATE, 'cells', f'{rid}_W{w}')
            fixture = os.path.join(FIX, f'{row["pose_ref"]}.pose')
            rc, so, se, wall = run(runner_cmd(row, w, cwd, f'gate:{rid}@W{w}', expect=fixture), cwd)
            s = D.parse_summary(so)
            want = fx[row['pose_ref']]['hash']
            why = check_runner(row, w, rc, s, cwd, want)
            if s and s.get('expect_pose') != 'match':
                why.append(f'expect_pose {s.get("expect_pose")}')
            hdr = csv_header(cwd)
            setup = [h for h in hdr if 'setup_chunks' in h]
            rec = {'row': rid, 'W': w, 'timed_cell': w in row['workers'], 'exit': rc, 'pose_hash': (s or {}).get('pose_hash'),
                   'want': want, 'expect_pose': (s or {}).get('expect_pose'), 'void_steps': (s or {}).get('void_steps'),
                   'armed': (s or {}).get('armed'), 'canary_ns': (s or {}).get('canary_ns'),
                   'canary_zone': (s or {}).get('canary_zone'), 'canary_zone_ns': (s or {}).get('canary_zone_ns'),
                   'w8s_keys': sorted((s or {}).get('w8s') or {}) if row['armed'] else None,
                   'csv_cols': len(hdr), 'setup_chunks_cols': setup, 'wall_s_for_schedule': wall, 'fail': why}
            recs.append(rec)
            est[f'{rid}#instr@W{w}'] = wall
            log(f'runner {rid:14s} W{w:<2d} exit {rc} pose {rec["pose_hash"]} expect {rec["expect_pose"]} void {rec["void_steps"]} '
                f'armed {rec["armed"]} setup_chunks_cols {setup or "ABSENT"} {"PASS" if not why else "FAIL " + "; ".join(why)}')
    json.dump(recs, open(os.path.join(GATE, 'gate_runner.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    est_update(est)
    bad = [r for r in recs if r['fail']]
    log(f'runner gate: {len(recs)} processes, {len(bad)} failing ({sum(1 for r in recs if r["timed_cell"])} are timed cells)')


def summaries(so):
    out = []
    for line in D.decode(so).splitlines():
        if line.startswith('SUMMARY '):
            try:
                out.append(json.loads(line[8:]))
            except ValueError:
                out.append({'unparsed': True})
    return out


def part_micro():
    recs, est = [], {}
    exe = BINS['omega']
    cwd = os.path.join(GATE, 'micro', 'help')
    rc, so, se, wall = run([exe, '--help'], cwd)
    txt = D.decode(so) + D.decode(se)
    recs.append({'what': '--help (the exe has no help text: without --bench --mode it runs the untimed self-check)',
                 'exit': rc, 'self_check_ok': 'omega_b_region self-check: ok' in txt,
                 'modes_in_source': ['omega-b', 'omega'], 'routes_in_source': ['worker', 'external']})
    log(f'micro --help: exit {rc}, self-check ok {"omega_b_region self-check: ok" in txt}')
    small = [('small-omega-b-worker', ['--bench', '--mode', 'omega-b', '--participants', '2', '--regions', '10', '--route', 'worker'], 1, 'omega_b', 'worker'),
             ('small-omega-b-external', ['--bench', '--mode', 'omega-b', '--participants', '2', '--regions', '10', '--route', 'external'], 1, 'omega_b', 'external'),
             ('small-omega-worker', ['--bench', '--mode', 'omega', '--workers', '8', '--gap-us', '0', '--reps', '10', '--route', 'worker'], 1, 'omega', 'worker'),
             ('small-omega-external', ['--bench', '--mode', 'omega', '--workers', '8', '--gap-us', '0', '--reps', '10', '--route', 'external'], 1, 'omega', 'external')]
    small = [x + (None,) for x in small]
    full = [(rid, r['args'], r['expect_summaries'], r['expect_bench'], r['expect_route'], r.get('expect_stages'))
            for rid, r in ROWS.items() if r['kind'] == 'micro']
    for tag, args, n, bench, route, stages in small + full:
        cwd = os.path.join(GATE, 'micro', tag)
        rc, so, se, wall = run([exe] + args, cwd)
        ss = summaries(so)
        why = []
        if rc != 0:
            why.append(f'exit {rc}')
        if len(ss) != n:
            why.append(f'{len(ss)} SUMMARY lines, expected {n}')
        if any(x.get('bench') != bench or x.get('route') != route for x in ss):
            why.append('a SUMMARY with the wrong bench/route')
        if stages is not None and any(x.get('stages') != stages for x in ss):
            why.append(f'a SUMMARY with stages != {stages}')
        recs.append({'what': tag, 'args': args, 'exit': rc, 'summaries': len(ss), 'wall_s_for_schedule': wall, 'fail': why})
        if not tag.startswith('small-'):
            est[f'{tag}#omega@W0'] = wall
        log(f'micro {tag:22s} exit {rc} summaries {len(ss)}/{n} {"PASS" if not why else "FAIL " + "; ".join(why)}')
    json.dump(recs, open(os.path.join(GATE, 'gate_micro.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    est_update(est)


def part_jolt():
    recs, est = [], {}
    for key, rid in (('j56', 'H-jolt56'), ('j56p', 'P-jolt56-prof')):
        row = ROWS[rid]
        for w in (1, 8, 16):
            cwd = os.path.join(GATE, 'jolt_cells', f'{rid}_W{w}')
            if os.path.isdir(cwd):
                shutil.rmtree(cwd)
            rc, so, se, wall = run([BINS[key]] + list(row['args']) + [f'-t={w}', '-i=500'], cwd)
            j = D.parse_jolt_stdout(so)
            why = []
            if rc != 0:
                why.append(f'exit {rc}')
            st = j['stat_lines']
            if len(st) != 1 or st[0]['threads'] != w or st[0]['hash'] != row['jolt_hash']:
                why.append(f'stat lines {[(x["threads"], x["hash"]) for x in st]}')
            pl = j.get('patch_line') or ''
            if not pl.startswith('boyko-parity-patch v1') or 'receipt=0' not in pl or 'allow_sleep=0' not in pl:
                why.append(f'patch banner {pl!r}')
            files = sorted(os.listdir(cwd))
            extra = {}
            if key == 'j56p':
                if 'boyko-w8s-patch v1' not in D.decode(so):
                    why.append('no boyko-w8s-patch banner')
                dumps = sorted(int(m.group(1)) for f in files for m in [re.match(r'profile_chart_.*_it(\d+)\.html$', f)] if m)
                if [i for i in row['profile_frames'] if i not in dumps]:
                    why.append(f'profile dumps {dumps}')
                names = joltprof.parse(os.path.join(cwd, f'profile_chart_discrete_th{w}_it100.html'))['scopes']
                gone = [n for n in names if 'Add Constraint From Cached Manifold' in n or 'sSolveVelocityConstraints' in n
                        or 'ContactConstraintManager::SolveVelocityConstraints' in n]
                if gone:
                    why.append(f'lighter profile: scopes still present {gone}')
                wf = [f for f in files if f.startswith('wfb_')]
                jobs = None
                if not wf:
                    why.append('no wfb csv')
                else:
                    rows_ = [l.split(',') for l in open(os.path.join(cwd, wf[0]), encoding='utf-8').read().splitlines()[1:]]
                    per = {}
                    for fr, sl, t, e, jb in rows_:
                        per[int(fr)] = per.get(int(fr), 0) + int(jb)
                    jobs = sorted(set(per.values()))
                    if len(per) != 500 or jobs != [w]:
                        why.append(f'wfb: {len(per)} frames, jobs per frame {jobs} (expected {w})')
                extra = {'dumps': dumps, 'wfb_jobs_per_frame': jobs}
            recs.append({'row': rid, 'binary': key, 'W': w, 'exit': rc, 'hash': st[0]['hash'] if st else None,
                         'files': len(files), 'wall_s_for_schedule': wall, 'fail': why, **extra})
            est[f'{rid}#{key}@W{w}'] = wall
            log(f'jolt {rid:14s} {key:5s} W{w:<2d} exit {rc} hash {st[0]["hash"] if st else None} {extra} '
                f'{"PASS" if not why else "FAIL " + "; ".join(why)}')
    json.dump(recs, open(os.path.join(GATE, 'gate_jolt.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
    est_update(est)


if __name__ == '__main__':
    part = sys.argv[1]
    log(f'=== gate8 {part} start')
    {'fixtures': part_fixtures, 'runner': part_runner, 'micro': part_micro, 'jolt': part_jolt}[part]()
    log(f'=== gate8 {part} end')
