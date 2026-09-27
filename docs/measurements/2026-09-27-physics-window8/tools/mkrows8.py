"""Writes rows8.json: physics window 8 = the W8S instrument block (w8s-s4/cut.md §5, rows W8S-A, W8S-R, micro; the
S4-AB block is EXCLUDED - S4 is not built) + the DM1 C6 A/B (dm1/cut.md §5 as amended by §11.5) + P-jolt56-prof.
Block order: W8S-A -> W8S-R -> micro -> DM1 -> P-jolt56-prof.
Run: python -B tools/mkrows8.py [--drop-res 2560x1440,...] (the DM1 resolutions the display cannot host)."""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
W8 = os.path.dirname(HERE)

DROP_RES = []
for i, a in enumerate(sys.argv):
    if a == '--drop-res' and i + 1 < len(sys.argv):
        DROP_RES = [x for x in sys.argv[i + 1].split(',') if x]

INSTR = '226bd99efba61c73886826eee4ab977af551bc23'
DM_A = 'cad5439b61ba6f07739523901aab90edf7079433'
DM_B = '97ee830f754e008032992fcd8a928cd29c04936f'

protocol = {
    'k_per_cell': 6, 'passes': 2, 'rounds_per_pass': 3,
    'order': 'pass 0: W ascending, inside a W group row by row (this file\'s order), each row\'s binaries in order; '
             'pass 1: the whole list reversed; one untimed warm-up per pass',
    'receipt_s_between_processes': 5.0, 'receipt_s_before_pass': 10.0, 'busy_pct_limit': 5.0,
    'others_busy_pct_limit': 2.0,
    'rerun_once_at_end_of_pass': 'receipt before/after > 5 %, OR the during-process witness others_busy_pct > 2 % '
                                 '(window 7 FOLLOW-UP 8; design 01-DESIGN.md §7 "the witness is gated"), OR invalid; '
                                 'a slot whose re-run is also hot is dropped by the reduction',
    'void_pass_on_build_process': True, 'placement': 'P-none',
    'idle_rule': 'tools/wait_idle8.ps1 (window 7 wait_idle7.ps1 + the window-8 exe names): 0 build procs AND 0 procs '
                 'under D:/wt/_targets or D:/wt/mq-* on 3 consecutive 60-s polls, 10-s cpu < 5 %; up to 30 polls per '
                 'wait; a timeout STOPS the window (exit 3)',
    'hard_stop': '--cutoff HH:MM (no process starts whose estimated end passes it); the STOP flag file win8/STOP',
    'windowed_hang_s': 300,
    'dm1': 'ABBA per (path, RES); noise band O3 = max(|A1-A2|, |B1-B2|); a clamped RES is "not measured" (the '
           'harness asserts the client area after the run)',
}

binaries = {
    'instr': {'exe': 'bin/runner_226bd99e.exe', 'commit': INSTR, 'kind': 'runner',
              'what': 'u/phys-w8s commit (1): the W8S instrument (armed-only) + omega_b bench; jolt_parity_pyramid, --profile parity'},
    'omega': {'exe': 'bin/omega_b_region_226bd99e.exe', 'commit': INSTR, 'kind': 'micro',
              'what': 'omega_b_region bench at the same commit, --profile parity'},
    'j56': {'exe': 'D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe', 'commit': 'Jolt v5.6.0 (wt-v5.6.0-parity, repository patch)',
            'kind': 'jolt', 'what': "window 7's j56 (P0 Distribution build), run in place"},
    'j56p': {'exe': 'bin/jolt56prof8/PerformanceTest.exe', 'commit': 'Jolt v5.6.0 build copy + tools/patch_jolt_w8.py',
             'kind': 'joltprof', 'what': "window 7's j56p recipe (PROFILER_IN_DISTRIBUTION=ON) on a build copy with the "
                                         'lighter profile + the WaitingForBatch per-thread counter (-wfb)'},
    'dmA': {'exe': 'bin/dm1_A.exe', 'commit': DM_A, 'kind': 'dm1', 'what': 'DM1 A = trunk-sync merge cad5439b (host-visible table)'},
    'dmB': {'exe': 'bin/dm1_B.exe', 'commit': DM_B, 'kind': 'dm1', 'what': 'DM1 B = C6 97ee830f (device-local table)'},
}

JT = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', 'tree', '--sleeping', 'off']
JTOFF = JT + ['--contact-reuse', 'off']
JA = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a']
JSON_ = JA + ['--sleeping', 'on']
MW = [[0, 100], [100, 500]]
POSE_JT = '0x30c5438bc6ad9ffa'
POSE_JTOFF = '0x32d5e235342b4143'
JOLT_HASH = '0xb8522b4e3fc62cfe'


def runner(rid, blocks, args, ws, armed, pose_ref, bp, **kw):
    r = {'id': rid, 'blocks': blocks, 'kind': 'runner', 'binaries': ['instr'], 'args': args, 'workers': ws,
         'steps': 500, 'window': [0, 500], 'metric_windows': MW, 'armed': armed, 'pose_ref': pose_ref, 'broadphase': bp}
    r.update(kw)
    return r


rows = [
    # ---- W8S-A (H-jolt56 listed right after J-T: adjacent to J-T at every W in every round)
    runner('J-T', ['W8S-A'], JT, [1, 8, 16], False, 'JT500', 'Tree'),
    {'id': 'H-jolt56', 'blocks': ['W8S-A'], 'kind': 'jolt', 'binaries': ['j56'], 'args': ['-s=Pyramid', '-q=Discrete', '-f'],
     'workers': [1, 8, 16], 'steps': 500, 'window': [0, 500], 'metric_windows': MW, 'armed': False, 'pose_ref': None,
     'broadphase': None, 'jolt_hash': JOLT_HASH},
    runner('J-T-a', ['W8S-A'], JT, [1, 8, 16], True, 'JT500', 'Tree'),
    runner('J-T-off', ['W8S-A'], JTOFF, [1, 8, 16], False, 'JToff500', 'Tree'),
    runner('J-T-off-a', ['W8S-A'], JTOFF, [1, 8, 16], True, 'JToff500', 'Tree'),
    runner('J-A', ['W8S-A'], JA, [1, 8, 16], False, 'JA500', 'AllPairs'),
    runner('J-A-a', ['W8S-A'], JA, [1, 8, 16], True, 'JA500', 'AllPairs'),
    # orchestrator amendment 4 (review N2): every armed row keeps its disarmed twin in the same block and W, and D(W)
    # needs the W1 pair, so J-Son (disarmed) is added at W8 and both J-Son rows get W1 (the spec had J-Son-a at W8 only)
    runner('J-Son', ['W8S-A'], JSON_, [1, 8], False, 'JSon500', 'AllPairs', note='amendment 4: the disarmed twin of J-Son-a'),
    runner('J-Son-a', ['W8S-A'], JSON_, [1, 8], True, 'JSon500', 'AllPairs', note='spec W8; W1 added by amendment 4 for D(W)'),
    # ---- W8S-R: the references J-T (every W a rung runs at) and J-T-a (W8) sit in the block, as window 7b's Q4 did
    runner('R-J-T', ['W8S-R'], JT, [1, 2, 4, 8, 16], False, 'JT500', 'Tree', note='the ladder/rung reference (J-T)'),
]
for f in ('0.5', '1', '1.5', '2'):
    rows.append(runner(f'ladder8-F{f}', ['W8S-R'], JT + ['--canary-frac', f, '--canary-ref-ns', '60000'], [8], False,
                       'JT500', 'Tree', canary=True))
for f in ('0.5', '1', '1.5', '2'):
    rows.append(runner(f'ladder16-F{f}', ['W8S-R'], JT + ['--canary-frac', f, '--canary-ref-ns', '105000'], [16], False,
                       'JT500', 'Tree', canary=True))
rows.append(runner('rung', ['W8S-R'], JT + ['--canary-frac', '1', '--canary-ref-ns', '60000'], [1, 2, 4], False, 'JT500',
                   'Tree', canary=True, note='Q8'))
rows.append(runner('R-J-T-a', ['W8S-R'], JT, [8], True, 'JT500', 'Tree', note='the zone canary reference (J-T-a)'))
for n in ('30000', '60000'):
    rows.append(runner(f'zone-N{n}', ['W8S-R'], JT + ['--canary-zone', 'phys_solve_build', '--canary-ns', n], [8], True,
                       'JT500', 'Tree', zone_canary=['phys_solve_build', int(n)]))
# ---- micro (the idle machine, separate block). Every mode the exe has at 226bd99e: omega-b and omega, both routes.
# orchestrator amendment 1 (review N5): omega_b at TWO --stages values, 36 and 72 (the per-stage cost is the slope);
# amendment 2: omega(W, gap) unchanged (read as a lower bound).
for tag, mode, extra, n in (('omega-b-s36', 'omega-b', ['--participants', '2,4,8,16', '--stages', '36'], 4),
                            ('omega-b-s72', 'omega-b', ['--participants', '2,4,8,16', '--stages', '72'], 4),
                            ('omega', 'omega', ['--workers', '8,16', '--gap-us', '0,5,20,80'], 8)):
    for route in ('worker', 'external'):
        rows.append({'id': f'{tag}-{route}', 'blocks': ['micro'], 'kind': 'micro', 'binaries': ['omega'],
                     'args': ['--bench', '--mode', mode] + extra + ['--route', route], 'workers': [0], 'armed': False,
                     'expect_summaries': n, 'expect_bench': mode.replace('-', '_'), 'expect_route': route,
                     'expect_stages': int(extra[-1]) if mode == 'omega-b' else None})
# ---- DM1 A/B
DM_ENV = {'BOYKO_DISABLE_VALIDATION': '1', 'BOYKO_VB_ZONE': '1', 'BOYKO_VB_BENCH_FRAMES': '220'}
DM_ARGS = ['dm1_material_table_timing', '--exact', '--ignored', '--test-threads=1', '--nocapture']
dm_cells = []  # explicit order: ABBA per group
dropped = []


def dm_row(rid, path, res, edit, grow=None, bins=('dmA', 'dmB')):
    env = dict(DM_ENV)
    env.update({'BOYKO_DM1_PATH': path, 'BOYKO_DM1_RES': res, 'BOYKO_DM1_EDIT_ROWS': str(edit)})
    if grow is not None:
        env['BOYKO_DM1_GROW_AT'] = str(grow)
    return {'id': rid, 'blocks': ['DM1'], 'kind': 'dm1', 'binaries': list(bins), 'args': DM_ARGS, 'env': env,
            'workers': [0], 'armed': False, 'res': res, 'path': path, 'edit_rows': edit, 'grow_at': grow}


for path in ('vb', 'deferred'):
    for res in ('1920x1080', '2560x1440', '3840x2160'):
        rid = f'dm-{path}-{res}-idle'
        if res in DROP_RES:
            dropped.append({'row': rid, 'why': f'the display cannot host a {res} client area (see gate/dm1_res.json)'})
            continue
        rows.append(dm_row(rid, path, res, 0))
        dm_cells += [[rid, 'dmA', 0], [rid, 'dmB', 0], [rid, 'dmB', 0], [rid, 'dmA', 0]]
for path in ('vb', 'deferred'):
    rid = f'dm-{path}-1920x1080-edit100'
    rows.append(dm_row(rid, path, '1920x1080', 100))
    dm_cells += [[rid, 'dmA', 0], [rid, 'dmB', 0], [rid, 'dmB', 0], [rid, 'dmA', 0]]
rid = 'dm-vb-1920x1080-grow150'
rows.append(dm_row(rid, 'vb', '1920x1080', 0, grow=150, bins=('dmB',)))
dm_cells += [[rid, 'dmB', 0]] * 3
# ---- P-jolt56-prof (last)
rows.append({'id': 'P-jolt56-prof', 'blocks': ['P-jolt56-prof'], 'kind': 'joltprof', 'binaries': ['j56p'],
             'args': ['-s=Pyramid', '-q=Discrete', '-f', '-p', '-wfb'], 'workers': [1, 8, 16], 'steps': 500,
             'window': [0, 500], 'metric_windows': MW, 'armed': False, 'pose_ref': None, 'broadphase': None,
             'jolt_hash': JOLT_HASH, 'profile_frames': [100, 200, 300, 400],
             'note': 'job walls per frame + the lighter profile + the WaitingForBatch counter (wfb_<tag>.csv), frames 100-400'})

blocks = [
    {'name': 'W8S-A', 'item': 'W8S', 'priority': 1, 'warmup': ['J-T', 'instr', 8]},
    {'name': 'W8S-R', 'item': 'W8S', 'priority': 2, 'warmup': ['R-J-T', 'instr', 8]},
    {'name': 'micro', 'item': 'W8S', 'priority': 3, 'warmup': ['omega-worker', 'omega', 0]},
    {'name': 'DM1', 'item': 'DM1', 'priority': 4, 'warmup': ['dm-vb-1920x1080-idle', 'dmA', 0], 'passes': 1, 'rounds': 1,
     'order': dm_cells},
    {'name': 'P-jolt56-prof', 'item': 'W8S', 'priority': 5, 'warmup': ['P-jolt56-prof', 'j56p', 8]},
]
out = {'window': 'physics window 8 (2026-09-27)', 'protocol': protocol, 'binaries': binaries, 'blocks': blocks,
       'rows': rows, 'dm1_dropped': dropped, 'excluded': [
           {'block': 'S4-AB', 'why': 'S4 (commit (4) of u/phys-w8s) is not built yet; it gets its own window'}]}
json.dump(out, open(os.path.join(W8, 'rows8.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
print(f'rows8.json: {len(rows)} rows, {len(blocks)} blocks, DM1 cells {len(dm_cells)}, dropped {dropped}')
