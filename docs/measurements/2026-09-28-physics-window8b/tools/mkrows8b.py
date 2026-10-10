"""Writes rows8b.json: PHYSICS WINDOW 8b (2026-09-28) on the trunk 16191fda, under RULINGS-2026-09-27-W8 ruling 1.
Blocks in priority order: S4-AB -> SPLIT -> F3 (runner rows) -> F3-G4 (criterion) -> J-Son-T -> DM1.
Reserved (ruling 5 / the orchestrator): S7-AB and omega_b v2 are NOT here; they go into rows8b.extra.json (PREP.md).
Sources: w8s-s4/cut.md §5 "WINDOW 8b" (S4-AB, SPLIT), tree-f3/window_cmds.md (F3, F3-G4), ruling 8 (J-Son-T),
ruling 9 (DM1 re-read). Model: win8/tools/mkrows8.py.
Run: python -B tools/mkrows8b.py   (reads gate/jsont_freeze.json, when the gate has written it, for J-Son-T's
post-freeze metric window, and gate/tree_diag_expect.json for the per-row TreeDiag expectations)."""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
W8B = os.path.dirname(HERE)
SP = os.path.dirname(W8B)
GATE = os.path.join(W8B, 'gate')

TRUNK = '16191fda6de68ab377391c3e781d4709b7a3ecd7'
DM_A = 'cad5439b61ba6f07739523901aab90edf7079433'
DM_B = '97ee830f754e008032992fcd8a928cd29c04936f'
WIN8_BIN = (SP + '/win8/bin').replace('\\', '/')

protocol = {
    'ruling': 'RULINGS-2026-09-27-W8.md ruling 1 (pre-registered before any 8b data)',
    'k_per_cell': 9, 'passes': 3, 'rounds_per_pass': 3,
    'order': 'pass 0 REVERSED, pass 1 forward, pass 2 REVERSED (ruling 1: "p0 reversed, p1, p2 reversed"). The '
             'forward list: W ascending, inside a W group row by row (this file\'s order), each row\'s binaries in '
             'order (S4-AB: parent then tip, so the two exes are adjacent in every round). One untimed warm-up per pass.',
    'receipt_s_between_processes': 5.0, 'receipt_s_before_pass': 10.0, 'busy_pct_limit': 5.0,
    'others_busy_pct_limit': 2.0,
    'rerun_once_at_end_of_pass': 'receipt before/after > 5 %, OR the during-process witness others_busy_pct > 2 %, '
                                 'OR invalid; a slot whose re-run is also hot is dropped by the reduction',
    'void_pass_on_build_process': True, 'placement': 'P-none',
    'placement_receipt': 'per process, RECORDED, NEVER used to drop a process: the per-logical-CPU busy share over '
                         'the process lifetime (window 8 analysis\'s reading: the top CPU and its share) plus the main '
                         'thread\'s own CPU time (GetThreadTimes on the thread the process was created with); '
                         'main_share_top_est = top CPU busy seconds / main-thread CPU seconds, capped at 1. Exact at '
                         'W1 on an idle machine; an upper bound at W >= 2 (workers share the CPUs).',
    'claim_rule': 'the analysis: IQR (i) AND SE (s) in every clean block and pooled; min-max (r) reported beside, '
                  '"STRONG" when it also passes',
    'idle_rule': 'tools/wait_idle8b.ps1 (window 8 wait_idle8.ps1 + the 8b exe names): 0 build procs AND 0 procs '
                 'under D:/wt/_targets or D:/wt/mq-* on 3 consecutive 60-s polls, 10-s cpu < 5 %; up to 30 polls per '
                 'wait; a timeout STOPS the window (exit 3)',
    'hard_stop': '--cutoff HH:MM (no process starts whose estimated end passes it); the STOP flag file win8b/STOP; '
                 'a block that does not fit before the cutoff is skipped WHOLE',
    'windowed_hang_s': 300,
    'own_k': 'F3-G4 keeps the Q3 recipe\'s own K = 3 (one pass x three rounds, no separate warm-up: criterion warms '
             'every benchmark); DM1 keeps ruling 9\'s ABBA x 2 per row (one pass, 8 processes per row)',
}

binaries = {
    'tip': {'exe': 'bin/runner_tip_16191fda.exe', 'commit': TRUNK, 'kind': 'runner', 's4': 'on',
            'what': 'TIP: jolt_parity_pyramid --profile parity from an export of 16191fda, unmodified'},
    'parent': {'exe': 'bin/runner_parent_16191fda_s4off.exe', 'commit': TRUNK + ' + bin/parent.patch', 'kind': 'runner',
               's4': 'off',
               'what': 'PARENT: the same export with ONE edit, SETUP_MAX_TASKS 32 -> 1 (S4 needs >= 2 ranges, so it '
                       'never dispatches); bin/parent.patch'},
    'g4ref': {'exe': 'bin/broadphase_g4ref_16191fda.exe', 'commit': TRUNK + ' + bin/g4ref.patch', 'kind': 'criterion',
              'what': 'the G4 instrument: benches/broadphase.rs with G4_SIZES replaced (tree-f3/window_cmds.md §1), '
                      'cargo bench --no-run --locked (bench profile)'},
    'dmA': {'exe': WIN8_BIN + '/dm1_A.exe', 'commit': DM_A, 'kind': 'dm1',
            'what': "window 8's DM1 A (host-visible table), reused by sha256 (win8/bin/SHA256SUMS)"},
    'dmB': {'exe': WIN8_BIN + '/dm1_B.exe', 'commit': DM_B, 'kind': 'dm1',
            'what': "window 8's DM1 B = C6 (device-local table), reused by sha256"},
}

JT = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', 'tree', '--sleeping', 'off']
F3J = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'default', '--broadphase', 'tree']   # window_cmds.md's rows, verbatim
F3A = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a', '--broadphase', 'tree']
F3R = ['--scene', 'rest', '--cfg', 'default', '--broadphase', 'tree']
JSONT = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a', '--broadphase', 'tree', '--sleeping', 'on']
JSOFFT = ['--scene', 'jolt', '--gap', '0.5', '--cfg', 'a', '--broadphase', 'tree', '--sleeping', 'off']
MW = [[0, 100], [100, 500]]
ALLW = [1, 2, 4, 8, 16]

freeze = {}
fp = os.path.join(GATE, 'jsont_freeze.json')
if os.path.exists(fp):
    freeze = json.load(open(fp, encoding='utf-8'))
tde = {}
tp = os.path.join(GATE, 'tree_diag_expect.json')
if os.path.exists(tp):
    tde = json.load(open(tp, encoding='utf-8'))
TD_DEFAULT = {'static_rebuilds': 1, 'members': 1, 'evictions': 0}   # window 8's Tree-row rule


def runner(rid, blocks, bins, args, ws, armed, pose_ref, **kw):
    r = {'id': rid, 'blocks': blocks, 'kind': 'runner', 'binaries': bins, 'args': args, 'workers': ws,
         'steps': 500, 'window': [0, 500], 'metric_windows': MW, 'armed': armed, 'pose_ref': pose_ref,
         'broadphase': 'Tree', 'tree_diag_expect': tde.get(rid, TD_DEFAULT)}
    r.update(kw)
    if 'bp_kernel' not in r:
        r['bp_kernel'] = None
    return r


rows = [
    # ---- 1. S4-AB (cut.md §5 WINDOW 8b): both exes interleaved per round; the rung and the zone canary on TIP
    runner('S4-JT', ['S4-AB'], ['parent', 'tip'], JT, ALLW, False, 'JT500'),
    runner('S4-JT-a', ['S4-AB'], ['parent', 'tip'], JT, [8, 16], True, 'JT500'),
    runner('S4-rung', ['S4-AB'], ['tip'], JT + ['--canary-frac', '1', '--canary-ref-ns', '60000'], [8], False, 'JT500',
           canary=True, note="S4's own resolution evidence: its reference is S4-JT#tip@W8, same block and round"),
]
for n in ('30000', '60000'):
    rows.append(runner(f'S4-zone-N{n}', ['S4-AB'], ['tip'], JT + ['--canary-zone', 'phys_solve_build', '--canary-ns', n],
                       [8], True, 'JT500', zone_canary=['phys_solve_build', int(n)],
                       note='the zone canary; its reference is S4-JT-a#tip@W8, same block and round'))
rows += [
    # ---- 2. SPLIT: the per-wave split (B1/B2/N3/N6) on PARENT; the disarmed twin in the same block and W
    runner('SPLIT-J-T', ['SPLIT'], ['parent'], JT, [1, 8, 16], False, 'JT500'),
    runner('SPLIT-J-T-a', ['SPLIT'], ['parent'], JT, [1, 8, 16], True, 'JT500'),
]
# ---- 3. F3 (tree-f3/window_cmds.md §3), TIP; each kernel pair adjacent; R4 = --expect-pose + cmp of the pair's poses
for base, args, ws, armed, pose in (('F3-TD-armed', F3J, [1, 8], True, 'JT500'),
                                    ('F3-TA-armed', F3A, [1], True, 'JTA500')):
    rows.append(runner(f'{base}-leaflist', ['F3'], ['tip'], args + ['--bp-kernel', 'leaflist'], ws, armed, pose,
                       bp_kernel='leaflist'))
    rows.append(runner(f'{base}-kd', ['F3'], ['tip'], args + ['--bp-kernel', 'leaflist-kd'], ws, armed, pose,
                       bp_kernel='leaflist-kd', r4_twin=f'{base}-leaflist'))
    if base == 'F3-TD-armed':
        pass
rows.append(runner('F3-TR-armed', ['F3'], ['tip'], F3J + ['--bp-kernel', 'rowwalk'], [1], True, 'JT500',
                   bp_kernel='rowwalk', note='optional in window_cmds.md: the bridge to windows 4 and 6'))
for base, args, pose in (('F3-JT', F3J, 'JT500'), ('F3-RT', F3R, 'RT500')):
    rows.append(runner(f'{base}-leaflist', ['F3'], ['tip'], args + ['--bp-kernel', 'leaflist'], ALLW, False, pose,
                       bp_kernel='leaflist'))
    rows.append(runner(f'{base}-kd', ['F3'], ['tip'], args + ['--bp-kernel', 'leaflist-kd'], ALLW, False, pose,
                       bp_kernel='leaflist-kd', r4_twin=f'{base}-leaflist'))
# ---- 4. F3-G4: the criterion block (window_cmds.md §4, the Q3 recipe)
G4_RE = ('^bp_g4_(uniform|disparity)/((all_pairs|tree|tree_kd)/(128|136|144|152|160|176|256|1000)|tree_rowwalk/(144|256))$'
         '|^bp_g4_scene/(tree|tree_kd)/(j100|1240|10000|100000)$')
g4_expect = []
for fam in ('uniform', 'disparity'):
    for arm in ('all_pairs', 'tree', 'tree_kd'):
        for n in (128, 136, 144, 152, 160, 176, 256, 1000):
            g4_expect.append(f'bp_g4_{fam}/{arm}/{n}')
    for n in (144, 256):
        g4_expect.append(f'bp_g4_{fam}/tree_rowwalk/{n}')
for arm in ('tree', 'tree_kd'):
    for p in ('j100', '1240', '10000', '100000'):
        g4_expect.append(f'bp_g4_scene/{arm}/{p}')
rows.append({'id': 'G4', 'blocks': ['F3-G4'], 'kind': 'criterion', 'binaries': ['g4ref'], 'workers': [0],
             'args': ['--bench', '--noplot', G4_RE], 'armed': False, 'expect': sorted(g4_expect),
             'test_filter': '^bp_g4_uniform/tree_kd/128$', 'test_expect': ['bp_g4_uniform/tree_kd/128'],
             'note': 'CRITERION_HOME per process; the rehearsal (--test) runs one id at 0.1 s + 0.2 s'})
# ---- 5. J-Son-T (ruling 8, L10 C1b): sleeping on the TREE broadphase and its sleeping-off twin, disarmed and armed
fz = freeze.get('post_freeze_window')
jmw = MW + ([fz] if fz else [])
rows += [
    runner('JSonT', ['J-Son-T'], ['tip'], JSONT, [1, 8], False, 'JSonT500', metric_windows=jmw,
           expect_first_frozen=freeze.get('first_frozen_step')),
    runner('JSonT-a', ['J-Son-T'], ['tip'], JSONT, [1, 8], True, 'JSonT500', metric_windows=jmw,
           expect_first_frozen=freeze.get('first_frozen_step')),
    runner('JSoffT', ['J-Son-T'], ['tip'], JSOFFT, [1, 8], False, 'JSoffT500', metric_windows=jmw),
    runner('JSoffT-a', ['J-Son-T'], ['tip'], JSOFFT, [1, 8], True, 'JSoffT500', metric_windows=jmw),
]
# ---- 6. DM1 re-read (ruling 9): ABBA x 2 per row, 1920x1080; the gated zones by id (gpu_zone.rs at cad5439b/97ee830f)
DM_ENV = {'BOYKO_DISABLE_VALIDATION': '1', 'BOYKO_VB_ZONE': '1', 'BOYKO_VB_BENCH_FRAMES': '220'}
DM_ARGS = ['dm1_material_table_timing', '--exact', '--ignored', '--test-threads=1', '--nocapture']
ZONES = {'vb': {'VB_SHADE': 2, 'VB_EARLY_CULL': 4, 'VB_RUN': 9, 'VB_PRODUCE_NET': 14},
         'deferred': {'GBUF_DEFERRED_RESOLVE': 17}}
dm_cells = []
for rid, path, edit in (('dm-vb-1920x1080-idle', 'vb', 0), ('dm-deferred-1920x1080-idle', 'deferred', 0),
                        ('dm-vb-1920x1080-edit100', 'vb', 100)):
    env = dict(DM_ENV)
    env.update({'BOYKO_DM1_PATH': path, 'BOYKO_DM1_RES': '1920x1080', 'BOYKO_DM1_EDIT_ROWS': str(edit)})
    rows.append({'id': rid, 'blocks': ['DM1'], 'kind': 'dm1', 'binaries': ['dmA', 'dmB'], 'args': DM_ARGS, 'env': env,
                 'workers': [0], 'armed': False, 'res': '1920x1080', 'path': path, 'edit_rows': edit, 'grow_at': None,
                 'need_zones': ZONES[path], 'present_mode': 'fifo'})
    dm_cells += [[rid, k, 0] for k in ('dmA', 'dmB', 'dmB', 'dmA', 'dmA', 'dmB', 'dmB', 'dmA')]

blocks = [
    {'name': 'S4-AB', 'item': 'S4', 'priority': 1, 'warmup': ['S4-JT', 'tip', 8]},
    {'name': 'SPLIT', 'item': 'W8S', 'priority': 2, 'warmup': ['SPLIT-J-T', 'parent', 8]},
    {'name': 'F3', 'item': 'F3', 'priority': 3, 'warmup': ['F3-JT-leaflist', 'tip', 8]},
    {'name': 'F3-G4', 'item': 'F3', 'priority': 4, 'passes': 1, 'rounds': 3,
     'note': 'the Q3 recipe: K 3, one pass x three rounds, no separate warm-up (criterion warms each benchmark)'},
    {'name': 'J-Son-T', 'item': 'L10', 'priority': 5, 'warmup': ['JSonT', 'tip', 8]},
    {'name': 'DM1', 'item': 'DM1', 'priority': 6, 'warmup': ['dm-vb-1920x1080-idle', 'dmA', 0], 'passes': 1,
     'rounds': 1, 'order': dm_cells},
]
out = {'window': 'physics window 8b (2026-09-28), trunk 16191fda', 'protocol': protocol, 'binaries': binaries,
       'blocks': blocks, 'rows': rows, 'jsont_freeze': freeze or None,
       'reserved': [{'block': 'S7-AB', 'how': 'rows8b.extra.json (PREP.md)'},
                    {'block': 'omega_b v2', 'how': 'rows8b.extra.json (PREP.md)'}]}
json.dump(out, open(os.path.join(W8B, 'rows8b.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
print(f'rows8b.json: {len(rows)} rows, {len(blocks)} blocks, DM1 cells {len(dm_cells)}, G4 ids {len(g4_expect)}, '
      f'J-Son-T post-freeze window {fz}, tree_diag overrides {sorted(tde)}')
