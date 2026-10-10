"""Writes win7b/rows7b.json, bin/SHA256SUMS and gate/fixtures/fixtures.json for window 7 wave 2 (untimed; run once).

Blocks in run order: Q1C, Q2C, Q3, Q4, Q2D, Q1D (priorities Q1 1, Q2 2, Q3 3, Q4 4).
Sources of every pinned number are in plan.md; the ones computed here:
  * Q1 cwd_len / cmdline_ref: window 7's P1A/P1B original processes (win7/raw/runs.jsonl), per row and W;
  * Q2 cwd_len / cmdline_ref: window 6's P3 (C3b-TA-armed, 163 / 648) and P5g (G5-TA-tree-a, 166 / 654) processes;
  * Q4 canary fractions: k x R / 1.0609 with R = 7.542 % (window 7's G-TW J-A W1 [0,500) two-spread bar,
    2 x hypot(3.352, 1.728) %) and 1.0609 = window 7's canary step rise / injected (0.8961 / 0.8447 ms).
"""
import hashlib
import json
import math
import os

HERE = os.path.dirname(os.path.abspath(__file__))
W7B = os.path.dirname(HERE)
SP = os.path.dirname(W7B)
WIN7 = SP + '/win7'
WIN6 = SP + '/win6'


def fwd(p):
    return p.replace('\\', '/')


def sha(p):
    h = hashlib.sha256()
    with open(p, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


BINARIES = {
    'trk': {'exe': fwd(WIN7) + '/bin/runner_93b2615b.exe', 'commit': '93b2615bcea4873e2d3af6b560c6e7997740f67f',
            'kind': 'runner', 'note': "window 7's trunk runner, run in place (same exe path as blocks A and B)"},
    'j56': {'exe': 'D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe', 'commit': 'Jolt v5.6.0 (P0 build 918fd2b7)',
            'kind': 'jolt', 'note': "P0's Distribution build, run in place as windows 3 and 7"},
    'par': {'exe': fwd(WIN6) + '/bin/runner_6dd1f916.exe', 'commit': '6dd1f916d5399b7bbe80429bdfde4e5c0bd63848',
            'kind': 'runner', 'note': "window 6's C3b parent (RowWalk), run in place (same exe path as window 6)"},
    'tip': {'exe': fwd(WIN6) + '/bin/runner_983480a9.exe', 'commit': '983480a9a1aaf1d28e9c17b08c0c06821176cacf',
            'kind': 'runner', 'note': "window 6's C3b tip (LeafList), run in place (same exe path as window 6)"},
    'c4': {'exe': fwd(WIN7) + '/bin/runner_989ca0f0.exe', 'commit': '989ca0f0575e054b89bb9c288ad95569ce493734',
           'kind': 'runner', 'note': "window 7's L9 C4 runner, run in place"},
    'g4r': {'exe': 'bin/bpbench_g4ref_93b2615b.exe', 'commit': '93b2615bcea4873e2d3af6b560c6e7997740f67f + G4_SIZES',
            'kind': 'criterion', 'note': 'INSTRUMENT: benches/broadphase.rs of 93b2615b with only G4_SIZES changed '
                                         '(variant/g4ref.diff), bench profile; never committed'},
}
# the sha256 each reused binary must have: the earlier window's own SHA256SUMS (checked here, not assumed)
EARLIER = {'trk': (WIN7 + '/bin/SHA256SUMS', 'bin/runner_93b2615b.exe'),
           'c4': (WIN7 + '/bin/SHA256SUMS', 'bin/runner_989ca0f0.exe'),
           'j56': (WIN7 + '/bin/SHA256SUMS', 'D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe'),
           'par': (WIN6 + '/bin/SHA256SUMS', 'runner_6dd1f916.exe'),
           'tip': (WIN6 + '/bin/SHA256SUMS', 'runner_983480a9.exe')}


def earlier_sha(key):
    path, name = EARLIER[key]
    for line in open(path, encoding='utf-8'):
        if line.strip():
            h, n = line.split()
            if n.lstrip('*') == name:
                return h
    raise SystemExit(f'{key}: {name} not in {path}')


def w7_lengths():
    out = {}
    for line in open(WIN7 + '/raw/runs.jsonl', encoding='utf-8'):
        r = json.loads(line)
        if r.get('block') in ('P1A-jolt', 'P1B-jolt') and r.get('attempt') == 'original' and r.get('cwd'):
            out.setdefault((r['row'], r['W']), set()).add((len(r['cwd']), sum(len(x) + 1 for x in r['args'])))
    res = {}
    for k, v in out.items():
        assert len(v) == 1, (k, v)
        res[k] = next(iter(v))
    return res


def w6_lengths():
    out = {}
    for line in open(WIN6 + '/raw/runs.jsonl', encoding='utf-8'):
        r = json.loads(line)
        if r.get('row') in ('C3b-TA-armed', 'G5-TA-tree-a') and r.get('attempt') == 'original' and r.get('cwd'):
            out.setdefault((r['row'], r['W']), set()).add((len(r['cwd']), sum(len(x) + 1 for x in r['args'])))
    res = {}
    for k, v in out.items():
        assert len(v) == 1, (k, v)
        res[k] = next(iter(v))
    return res


L7 = w7_lengths()
L6 = w6_lengths()
Q1W = [4, 8, 16]
J500_W7 = fwd(WIN7) + '/gate/fixtures/J500.pose'
J500_W6 = fwd(WIN6) + '/gate/fixtures/J500.pose'
JAON_W7 = fwd(WIN7) + '/gate/fixtures/JAon500c4.pose'
JA = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a']
R_GTW = 2 * math.hypot(3.352, 1.728) / 100          # 0.07542
RISE = 0.8961 / 0.8447                              # 1.0609
LADDER = {'L9-JC-r050': 0.5, 'L9-JC-r100': 1.0, 'L9-JC-r150': 1.5, 'L9-JC-r200': 2.0}
G4_TIMED = [64, 96, 112, 128, 136, 144, 152, 160, 168, 176, 192, 256]
G4_BRIDGE = [64, 128, 256]  # window 4's grid points: the same-binary RowWalk arm (C1's kernel) bridges to window 4
G4_FILTER = ('^bp_g4_(uniform|disparity)/((all_pairs|tree)/(' + '|'.join(str(n) for n in G4_TIMED) + ')|tree_rowwalk/('
             + '|'.join(str(n) for n in G4_BRIDGE) + '))$')
G4_EXPECT = ([f'bp_g4_{fam}/{arm}/{n}' for fam in ('uniform', 'disparity') for n in G4_TIMED for arm in ('all_pairs', 'tree')]
             + [f'bp_g4_{fam}/tree_rowwalk/{n}' for fam in ('uniform', 'disparity') for n in G4_BRIDGE])

rows = []
for rid, key, args, bp in (('H-trk-tree', 'trk', JA[:4] + ['--cfg', 'default', '--broadphase', 'tree'], 'Tree'),
                           ('H-jolt56', 'j56', ['-s=Pyramid', '-q=Discrete', '-f'], 'AllPairs'),
                           ('H-trk-ap', 'trk', JA[:4] + ['--cfg', 'default', '--broadphase', 'allpairs'], 'AllPairs')):
    r = {'id': rid, 'item': 'Q1', 'blocks': ['Q1C', 'Q1D'], 'kind': 'jolt' if key == 'j56' else 'runner',
         'binaries': [key], 'args': args, 'workers': Q1W, 'steps': 500, 'window': [0, 500],
         'metric_windows': [[0, 100], [100, 500]], 'armed': False, 'broadphase': bp,
         'cwd_len': {str(w): L7[(rid, w)][0] for w in Q1W}, 'cmdline_ref': {str(w): L7[(rid, w)][1] for w in Q1W},
         'cwd_len_source': "window 7's P1A/P1B original processes (win7/raw/runs.jsonl)",
         'note': "window 7 P1's row, verbatim; blocks C (first) and D (last) join window 7's A and B"}
    if key == 'j56':
        r.update({'pose_ref': None, 'jolt_hash': '0xb8522b4e3fc62cfe'})
    else:
        r.update({'pose_ref': 'J500', 'pose_path': J500_W7})
    rows.append(r)

C3B_A = JA + ['--broadphase', 'tree']
for rid, args, ws, cl, src, note in (
        ('C3b-TA-armed', C3B_A, [1, 8], ('C3b-TA-armed', 163), "window 6's P3 layout (C3b-TA-armed: cwd 163, 648 chars)",
         "window 6's C3b-TA-armed = T-A-tree-armed (c3b/design.md:222), plain layout = window 6's P3 block"),
        ('C3b-TA-pad06', C3B_A, [1], ('G5-TA-tree-a', 166), "window 6's P5g layout (G5-TA-tree-a: cwd 166, 654 chars)",
         'the PADDED-PATH VARIANT: the same process with every path argument 3 characters longer (+6 on the '
         "command line), = window 6's P5g launch context, where t_q(W1) read 0.2354 against P3's 0.2102"),
        ('C3b-TD-armed', JA[:4] + ['--cfg', 'default', '--broadphase', 'tree'], [1], None, None,
         "the DEFAULT row armed: the C2 rule's letter reads 't_q(J, W=1) >= 0.235 ms on the default row' "
         '(c3b/design.md:231, :372); window 6 read t_q off the cfg-A armed row')):
    r = {'id': rid, 'item': 'Q2', 'blocks': ['Q2C', 'Q2D'], 'kind': 'runner', 'binaries': ['par', 'tip'], 'args': args,
         'workers': ws, 'steps': 500, 'window': [0, 500], 'metric_windows': [[100, 500]], 'armed': True,
         'pose_ref': 'J500', 'pose_path': J500_W6, 'broadphase': 'Tree', 'note': note}
    if cl:
        r['cwd_len'] = {str(w): cl[1] for w in ws}
        r['cmdline_ref'] = {str(w): L6[(cl[0], w)][1] for w in ws}
        assert all(L6[(cl[0], w)][0] == cl[1] for w in ws), (rid, L6)
        r['cwd_len_source'] = src
    rows.append(r)

rows.append({'id': 'G4-refine', 'item': 'Q3', 'blocks': ['Q3'], 'kind': 'criterion', 'binaries': ['g4r'],
             'args': ['--bench', '--noplot', G4_FILTER], 'workers': [1], 'expect': G4_EXPECT,
             'test_filter': '^bp_g4_uniform/tree/64$', 'test_expect': ['bp_g4_uniform/tree/64'],
             'note': "the recipe's refinement run (g4_g5_recipe.md:131-132; window 4 analysis 6.1/6.2): all_pairs vs "
                     'tree on the uniform and disparity families at 12 sizes between 64 and 256, K = 3; plus the '
                     "same-binary tree_rowwalk arm (C1's kernel) at window 4's grid points 64/128/256 as the bridge"})

rows.append({'id': 'L9-JA-on', 'item': 'Q4', 'blocks': ['Q4'], 'kind': 'runner', 'binaries': ['c4'],
             'args': JA + ['--contact-reuse', 'on'], 'workers': [1], 'steps': 500, 'window': [0, 500],
             'metric_windows': [[0, 100], [100, 500]], 'armed': False, 'pose_ref': 'JAon500c4', 'pose_path': JAON_W7,
             'broadphase': 'AllPairs', 'note': "G-TW's own 'on' row (window 7 P2), unarmed: the canary's reference"})
for rid, k in LADDER.items():
    frac = round(k * R_GTW / RISE, 3)
    rows.append({'id': rid, 'item': 'Q4', 'blocks': ['Q4'], 'kind': 'runner', 'binaries': ['c4'],
                 'args': JA + ['--contact-reuse', 'on'], 'workers': [1], 'steps': 500, 'window': [0, 500],
                 'metric_windows': [[0, 100], [100, 500]], 'armed': False, 'pose_ref': 'JAon500c4',
                 'pose_path': JAON_W7, 'broadphase': 'AllPairs', 'canary_of': 'L9-JA-on', 'canary_frac': frac,
                 'canary_k': k, 'note': f'canary rung {k} x R: frac {frac} = {k} x {R_GTW:.5f} / {RISE:.4f}; '
                                        'expected step rise ' + f'{100 * k * R_GTW:.2f} % of the reference'})

blocks = [
    {'name': 'Q1C', 'item': 'Q1', 'priority': 1, 'warmup': ['H-trk-tree', 'trk', 8]},
    {'name': 'Q2C', 'item': 'Q2', 'priority': 2, 'warmup': ['C3b-TA-armed', 'tip', 1]},
    {'name': 'Q3', 'item': 'Q3', 'priority': 3, 'warmup': None, 'passes': 1, 'rounds': 3},
    {'name': 'Q4', 'item': 'Q4', 'priority': 4, 'warmup': ['L9-JA-on', 'c4', 1]},
    {'name': 'Q2D', 'item': 'Q2', 'priority': 2, 'warmup': ['C3b-TA-armed', 'tip', 1]},
    {'name': 'Q1D', 'item': 'Q1', 'priority': 1, 'warmup': ['H-trk-tree', 'trk', 8]},
]
protocol = {'k_per_cell': 6, 'passes': 2, 'rounds_per_pass': 3, 'receipt_s_between_processes': 5.0,
            'receipt_s_before_pass': 10.0, 'busy_pct_limit': 5.0, 'rerun_contaminated_once_at_end_of_pass': True,
            'void_pass_on_build_process': True, 'placement': 'P-none',
            'idle_rule': "tools/wait_idle7.ps1 (window 7's, verbatim): 0 build procs AND 0 procs under D:/wt/_targets "
                         'or D:/wt/mq-* on 3 consecutive 60-s polls, 10-s cpu < 5 %; up to 30 polls per wait; a timeout '
                         'STOPS the window',
            'hard_stop_local': '11:45', 'stop_flag': 'win7b/STOP',
            'q3_k': 3, 'canary': {'R_gtw': R_GTW, 'rise_over_injected': RISE}}

bins = {}
sums = []
for key, b in BINARIES.items():
    path = b['exe'] if ':' in b['exe'] else fwd(W7B) + '/' + b['exe']
    h = sha(path)
    if key in EARLIER:
        want = earlier_sha(key)
        assert h == want, (key, h, want)
        b = dict(b, sha256_earlier_window=want)
    bins[key] = b
    sums.append(f'{h} *{b["exe"]}')
open(W7B + '/bin/SHA256SUMS', 'w', encoding='utf-8', newline='\n').write('\n'.join(sums) + '\n')
json.dump({'protocol': protocol, 'binaries': bins, 'blocks': blocks, 'rows': rows},
          open(W7B + '/rows7b.json', 'w', encoding='utf-8', newline='\n'), indent=1)
fx7 = json.load(open(WIN7 + '/gate/fixtures/fixtures.json', encoding='utf-8'))
os.makedirs(W7B + '/gate/fixtures', exist_ok=True)
fx = {'J500': {'hash': '0x32d5e235342b4143', 'file': J500_W7, 'also': J500_W6, 'sha256': sha(J500_W7),
               'sha256_win6': sha(J500_W6)},
      'JAon500c4': {'hash': fx7['JAon500c4']['hash'], 'file': JAON_W7, 'sha256': sha(JAON_W7),
                    'recorded_by': "window 7's gate7.py (win7/gate/fixtures/fixtures.json)"}}
assert fx['J500']['sha256'] == fx['J500']['sha256_win6']
json.dump(fx, open(W7B + '/gate/fixtures/fixtures.json', 'w', encoding='utf-8', newline='\n'), indent=1)
print('rows', len(rows), 'blocks', len(blocks), 'R_gtw', R_GTW, 'rise', RISE,
      'ladder', {k: round(v * R_GTW / RISE, 3) for k, v in LADDER.items()})
print('\n'.join(sums))
