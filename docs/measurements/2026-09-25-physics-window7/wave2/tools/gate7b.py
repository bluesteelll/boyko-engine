"""Window 7 wave 2 POSE GATE (untimed; run before the window while this agent works). Adapted from win7/tools/gate7.py.

1. cells: one untimed process per timed runner cell of rows7b.json, with the row's own --expect-pose file (the earlier
   window's fixture path), --csv, --pose-out, --label, --arm-profiler on armed rows, and the canary flags on the
   canary rows (reference 16,683,800 ns = window 7's L9-JA-on W1 median): exit 0, SUMMARY expect_pose = match,
   pose hash = the fixture's, void_steps 0, target_env msvc, config.broadphase = the row's, TreeDiag (Tree rows:
   static_rebuilds 1, members 1, evictions 0; AllPairs rows: all 0), armed flag, drops_total 0 when armed, canary_ns
   present on canary rows; the wall is kept as the per-process estimate (gate/estimates_runner.json);
2. jolt: Jolt v5.6.0 (918fd2b7) at every timed W: exit 0, one stat line, threads = W, hash 0xb8522b4e3fc62cfe, the
   patch banner, 500 frames in the per_frame csv;
3. criterion: the Q3 instrument, one untimed process with the timed filter (the full criterion settings): exit 0,
   every expected id has an estimate and no other id, and every size of both families printed its kernel receipt
   (the bench asserts the tree's steady-state pair set equals AllPairs' under both kernels before timing);
4. red: a 501-step twin against the 500-step fixture must exit 4 on every runner binary (the gate can fail).
A pose mismatch is re-run ONCE on a fresh process before it is called (the machine's memory-fault signature).
Usage: python gate7b.py [--only cells,jolt,criterion,red]
"""
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
W7B = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, 'lib'))
import driver as D  # noqa: E402

GATE = os.path.join(W7B, 'gate')
ROWS = json.load(open(os.path.join(W7B, 'rows7b.json'), encoding='utf-8'))
FIXTURES = json.load(open(os.path.join(GATE, 'fixtures', 'fixtures.json'), encoding='utf-8'))
BIN = {k: (v['exe'].replace('/', os.sep) if ':' in v['exe'] else os.path.join(W7B, v['exe'].replace('/', os.sep)))
       for k, v in ROWS['binaries'].items()}
JOLT56_HASH = '0xb8522b4e3fc62cfe'
CANARY_REF_NS = 16683800
TREE_KEYS = ('static_rebuilds', 'sleeper_rebuilds', 'evictions', 'translations', 'patches', 'hint_candidates',
             'wide_rows', 'excluded_rows', 'locator_resets', 'members')
RESULTS = []
ENV = dict(os.environ)
for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
    ENV.pop(k, None)
EST_PATH = os.path.join(GATE, 'estimates_runner.json')


def load_est():
    return json.load(open(EST_PATH, encoding='utf-8')) if os.path.exists(EST_PATH) else {}


def save_est(est):
    json.dump(est, open(EST_PATH, 'w', encoding='utf-8', newline='\n'), indent=1)


def summary(so):
    for line in so.splitlines():
        if line.startswith('SUMMARY '):
            try:
                return json.loads(line[len('SUMMARY '):])
            except ValueError:
                return None
    return None


def run(tag, key, args, env=None):
    d = os.path.join(GATE, tag)
    if os.path.isdir(d):
        shutil.rmtree(d)
    os.makedirs(d, exist_ok=True)
    t0 = time.perf_counter()
    p = subprocess.run([BIN[key]] + args, cwd=d, env=env or ENV, capture_output=True)
    wall = time.perf_counter() - t0
    open(os.path.join(d, 'stdout.txt'), 'wb').write(p.stdout)
    open(os.path.join(d, 'stderr.txt'), 'wb').write(p.stderr)
    return d, p.returncode, p.stdout, p.stderr, round(wall, 3)


def runner_rec(tag, key, args, expect_exit=0):
    d, rc, so, se, wall = run(tag, key, args)
    s = summary(so.decode('utf-8', 'replace'))
    return {'tag': tag, 'bin': key, 'args': args, 'exit': rc, 'expect_exit': expect_exit, 'wall_s': wall,
            'pose': s.get('pose_hash') if s else None, 'expect_pose': s.get('expect_pose') if s else None,
            'void_steps': s.get('void_steps') if s else None, 'workers': s.get('workers') if s else None,
            'window_mean_ms': round(s['window_mean_ns'] / 1e6, 4) if s and s.get('window_mean_ns') else None,
            'broadphase': (s.get('config') or {}).get('broadphase') if s else None,
            'contact_reuse': (s.get('config') or {}).get('contact_reuse') if s else None,
            'target_env': s.get('target_env') if s else None, 'armed': s.get('armed') if s else None,
            'drops_total': s.get('drops_total') if s else None, 'canary_ns': s.get('canary_ns') if s else None,
            'tree': s.get('broadphase_tree') if s else None, 'dir': d}


def check_runner(rec, row, w):
    why = []
    if rec['exit'] != rec['expect_exit']:
        why.append(f"exit {rec['exit']} != {rec['expect_exit']}")
    if rec['expect_exit'] == 0:
        want = FIXTURES[row['pose_ref']]['hash']
        if rec['pose'] != want:
            why.append(f"pose {rec['pose']} != {want}")
        if rec['expect_pose'] != 'match':
            why.append(f"expect_pose {rec['expect_pose']}")
        if rec['void_steps'] != 0:
            why.append(f"void_steps {rec['void_steps']}")
        if rec['workers'] != w:
            why.append(f"workers {rec['workers']}")
        if rec['target_env'] != 'msvc':
            why.append(f"target_env {rec['target_env']}")
        if rec['broadphase'] != row['broadphase']:
            why.append(f"broadphase {rec['broadphase']} != {row['broadphase']}")
        if bool(rec['armed']) != bool(row['armed']):
            why.append(f"armed {rec['armed']}")
        if row['armed'] and rec['drops_total']:
            why.append(f"drops_total {rec['drops_total']}")
        bt = rec['tree']
        if isinstance(bt, dict):
            if row['broadphase'] == 'Tree':
                if not (bt.get('static_rebuilds') == 1 and bt.get('members') == 1 and bt.get('evictions') == 0):
                    why.append(f'TreeDiag {bt}')
            elif any(bt.get(k) for k in TREE_KEYS):
                why.append(f'TreeDiag nonzero {bt}')
        if row.get('canary_of') and not rec['canary_ns']:
            why.append(f"canary_ns {rec['canary_ns']}")
    return why


def report(rec, why):
    rec['why'] = why
    rec['ok'] = not why
    RESULTS.append(rec)
    print(f"{'OK ' if rec['ok'] else 'FAIL'} {rec['tag']:<44} exit {rec['exit']} pose {rec.get('pose')} "
          f"expect {rec.get('expect_pose')} bp {rec.get('broadphase')} reuse {rec.get('contact_reuse')} "
          f"canary_ns {rec.get('canary_ns')} wall {rec['wall_s']}s win {rec.get('window_mean_ms')} {'; '.join(why)}",
          flush=True)


def cell_args(row, key, w):
    a = list(row['args']) + ['--workers', str(w), '--steps', str(row['steps']), '--window',
                             f"{row['window'][0]}..{row['window'][1]}", '--csv', 'run.csv', '--pose-out', 'pose.bin',
                             '--label', f"gate:{row['id']}#{key}@W{w}"]
    if row['armed']:
        a.append('--arm-profiler')
    if row.get('canary_of'):
        a += ['--canary-frac', str(row['canary_frac']), '--canary-ref-ns', str(CANARY_REF_NS)]
    return a + ['--expect-pose', row['pose_path'].replace('/', os.sep)]


def cells():
    est = load_est()
    for row in ROWS['rows']:
        if row['kind'] != 'runner':
            continue
        for key in row['binaries']:
            for w in row['workers']:
                tag = f"cells/{row['id']}_{key}_W{w}"
                args = cell_args(row, key, w)
                rec = runner_rec(tag, key, args)
                why = check_runner(rec, row, w)
                if why:
                    rec2 = runner_rec(tag + '_rerun', key, args)
                    why2 = check_runner(rec2, row, w)
                    rec['rerun'] = dict(rec2, why=why2)
                    if not why2:
                        why = []
                        rec['ok_after_rerun'] = True
                report(rec, why)
                est[f"{row['id']}#{key}@W{w}"] = rec['wall_s']
                save_est(est)


def jolt():
    est = load_est()
    for row in ROWS['rows']:
        if row['kind'] != 'jolt':
            continue
        for key in row['binaries']:
            for w in row['workers']:
                tag = f"cells/{row['id']}_{key}_W{w}"
                args = list(row['args']) + [f'-t={w}', f"-i={row['steps']}"]
                d, rc, so, se, wall = run(tag, key, args)
                j = D.parse_jolt_stdout(so)
                st = j['stat_lines'][0] if j['stat_lines'] else {}
                rec = {'tag': tag, 'bin': key, 'args': args, 'exit': rc, 'wall_s': wall, 'hash': st.get('hash'),
                       'threads': st.get('threads'), 'patch_line': j['patch_line'], 'dir': d}
                why = []
                if rc != 0:
                    why.append(f'exit {rc}')
                if len(j['stat_lines']) != 1:
                    why.append(f"{len(j['stat_lines'])} stat lines")
                if st.get('hash') != JOLT56_HASH:
                    why.append(f"hash {st.get('hash')}")
                if st.get('threads') != w:
                    why.append(f"threads {st.get('threads')}")
                pl = j['patch_line'] or ''
                if not pl.startswith('boyko-parity-patch v1') or 'receipt=0' not in pl or 'allow_sleep=0' not in pl:
                    why.append(f'patch banner {pl!r}')
                pf = [f for f in os.listdir(d) if f.startswith('per_frame_')]
                if pf:
                    t = D.load_csv(os.path.join(d, pf[0]))['Time (ms)']
                    rec['frames'] = len(t)
                    rec['window_mean_ms'] = round(statistics.mean(t[0:500]), 4)
                    if len(t) != 500:
                        why.append(f'{len(t)} frames')
                else:
                    why.append('no per_frame csv')
                report(rec, why)
                est[f"{row['id']}#{key}@W{w}"] = wall
                save_est(est)


CRIT_TIME = re.compile(r'time:\s+\[([\d.]+) (\S+) ([\d.]+) (\S+) ([\d.]+) (\S+)\]')
RECEIPT = re.compile(r'^bp_g4_(uniform|disparity)/(\d+): kernel (LeafList|RowWalk) rows (\d+) pairs (\d+) ')


def criterion():
    est = load_est()
    row = next(r for r in ROWS['rows'] if r['kind'] == 'criterion')
    key = row['binaries'][0]
    tag = f"cells/{row['id']}_{key}_W1"
    d = os.path.join(GATE, tag)
    prev = os.path.join(d, 'gate_rec.json')
    if os.path.exists(prev) and os.path.exists(os.path.join(d, 'stdout.txt')):
        pr = json.load(open(prev, encoding='utf-8'))
        rc, wall = pr['exit'], pr['wall_s']
        so = open(os.path.join(d, 'stdout.txt'), 'rb').read()
        se = open(os.path.join(d, 'stderr.txt'), 'rb').read()
        assert pr['filter'] == row['args'][-1], (pr['filter'], row['args'][-1])
        src = 'the untimed run already in gate/cells (same filter, full criterion settings)'
    else:
        env = dict(ENV, CRITERION_HOME=os.path.join(d, 'criterion'))
        os.makedirs(d, exist_ok=True)
        d, rc, so, se, wall = run(tag, key, list(row['args']), env=env)
        src = 'run now'
    names, last = {}, None
    for line in D.decode(so).splitlines():
        s = line.strip()
        if s.startswith('bp_g4_') and ':' not in s:
            last = s.split()[0]
        m = CRIT_TIME.search(line)
        if m:
            names[s.split()[0] if s.startswith('bp_g4_') else last] = m.group(3) + ' ' + m.group(4)
    rcpt = {}
    for line in D.decode(se).splitlines():
        m = RECEIPT.match(line)
        if m:
            rcpt.setdefault((m.group(1), int(m.group(2))), set()).add(m.group(3))
    why = []
    if rc != 0:
        why.append(f'exit {rc}')
    miss = [n for n in row['expect'] if n not in names]
    extra = sorted(set(names) - set(row['expect']))
    if miss:
        why.append(f'missing {miss[:4]} ({len(miss)})')
    if extra:
        why.append(f'unexpected {extra[:4]} ({len(extra)})')
    sizes = sorted({int(n.rsplit('/', 1)[1]) for n in row['expect']})
    for fam in ('uniform', 'disparity'):
        for n in sizes:
            if rcpt.get((fam, n)) != {'LeafList', 'RowWalk'}:
                why.append(f'no kernel receipt for {fam}/{n}')
    rec = {'tag': tag, 'bin': key, 'args': row['args'], 'exit': rc, 'wall_s': wall, 'ids': len(names),
           'receipts': len(rcpt), 'source': src, 'dir': d}
    report(rec, why)
    est[f"{row['id']}#{key}@W1"] = wall
    save_est(est)


def red():
    seen = set()
    for row in ROWS['rows']:
        if row['kind'] != 'runner' or row.get('canary_of'):
            continue
        for key in row['binaries']:
            if key in seen:
                continue
            seen.add(key)
            args = list(row['args']) + ['--workers', '1', '--steps', '501', '--expect-pose',
                                        row['pose_path'].replace('/', os.sep)]
            rec = runner_rec(f'red/{key}_{row["id"]}_501', key, args, expect_exit=4)
            report(rec, [] if rec['exit'] == 4 else [f"exit {rec['exit']} != 4 (the gate cannot fail)"])


def main():
    only = None
    if '--only' in sys.argv:
        only = sys.argv[sys.argv.index('--only') + 1].split(',')
    for name, fn in (('cells', cells), ('jolt', jolt), ('criterion', criterion), ('red', red)):
        if only and name not in only:
            continue
        print(f'== {name} {time.strftime("%H:%M:%S")}', flush=True)
        fn()
    out = os.path.join(GATE, f'gate_{"_".join(only) if only else "all"}.json')
    json.dump(RESULTS, open(out, 'w', encoding='utf-8', newline='\n'), indent=1, default=str)
    bad = [r['tag'] for r in RESULTS if not r['ok']]
    print(f'== done {time.strftime("%H:%M:%S")}: {len(RESULTS)} processes, {len(bad)} failing: {bad}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
