"""Window 9a: build rows9a.json from the tree-c4 lane's AMENDED window commands (tree-c4/window9_c4_rows.json; the cut's original
window9_c4_rows.cut.json is NOT used) and window 8b's protocol block. Deterministic: run it again after editing, never edit
rows9a.json by hand.
  python -B tools/mkrows9a.py            # writes rows9a.json (C4-BR at the letter, K 9)
  python -B tools/mkrows9a.py --br-k3    # the lane's Q5 recommendation: C4-BR one pass x three rounds (K 3)
  python -B tools/mkrows9a.py --lane-order   # the cut's block order BR, G4, G4-kd, AB, G5 (default: AB, G5, BR, G4, G4-kd)
Every deviation from the lane's file is listed in DEVIATIONS below and printed into rows9a.json's `_derived`."""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
W9 = os.path.dirname(HERE)
SP = 'C:/Users/flint/AppData/Local/Temp/claude/D--claude-BoykoEngine/238150a2-5c92-4147-8309-b6428586e8a7/scratchpad'
LANE = json.load(open(f'{SP}/tree-c4/window9_c4_rows.json', encoding='utf-8'))
R8B = json.load(open(f'{SP}/win8b/rows8b.json', encoding='utf-8'))

TIP_SHA = '50e31f1a78bd53adf31628a3fa03ff28c2647275'
PARENT_SHA = '3d9433aed2d2251d3bc4cc2ca59692027ddb2494'
PINS = {
    'tip': '8d6e7d4173857ca83b47cd350890b8e178451fb48f030ac3d25aaef33cd268c3',
    'parent': 'a09fca085613f966e09b3e8cd78ed711aa1c51f069cfab649834d93f88988478',
    'g4rT': '19c9eb1f67cc728505525b5d669adfd2db133a1ae461217fcab7f386bc7c0bcf',
    'g4r7': 'f96a9c11ebd72d4d581784cf37728fd6da15bc63b28414d7e895667753033fd4',
    'g4r8b': 'b887850f38633d62419344c41080d1bada92bc931c3307989b6f6fb314a33b6d',
    'j56': '918fd2b70ecc634cc0574cb676c56a3c929a46a51173cf9aa76fdd628e2ef0ad',
}
DEVIATIONS = []
BR_K3 = '--br-k3' in sys.argv
LANE_ORDER = '--lane-order' in sys.argv


def dev(s):
    DEVIATIONS.append(s)


# ------------------------------------------------------------------ protocol: window 8b's, with ruling 8 and the 9a names
proto = json.loads(json.dumps(R8B['protocol']))
proto['ruling'] = ('RULINGS-2026-09-27-W8.md ruling 1 (K 9 = 3 passes x 3 rounds; IQR AND SE claim rule; min-max reported) + '
                   'RULINGS-2026-09-29-W8B.md ruling 8 (a pass-cell with K < 3 does not gate; a dropped slot is re-run until the '
                   'cell has K = 3 or the pass ends)')
proto.pop('rerun_once_at_end_of_pass', None)
proto['rerun_rule'] = ('RULING 8, implemented in tools/window9a_run.py (run_pass, pending_reruns, passcell_table): the re-run STAGE at '
                       'the end of each pass. A slot is unclean when its latest attempt has receipt before/after > 5 %, OR the '
                       'during-process witness others_busy_pct > 2 %, OR a build process during the run, OR is invalid. Wave a '
                       're-runs every unclean slot (original order); a slot leaves the stage at its first clean valid attempt. '
                       '"The pass ends" = RERUN_MAX_WAVES (4) waves, or the wall allowance rerun_allowance(block) = max(RERUN_STAGE_FRAC '
                       '(0.5) x the pass\'s own timed wall, RERUN_MAX_WAVES x RERUN_EST_SLACK (1.3) x the block\'s longest process) spent '
                       'on re-runs (audit-1 W1: the fraction alone gave a one-cell criterion block ONE re-run), or the cutoff / STOP flag; '
                       'the cutoff / STOP flag RAISE out of run_pass, so a pass cut inside its re-run stage writes no passcell / pass_done '
                       'records and --resume re-runs it whole (counts9a.py prints INCOMPLETE). Records: attempt "rerun" + rerun_no 1.., rerun_reason; one '
                       '{"passcell": true, pc_row, pc_binary, pc_W, k_target, k_clean, gates (k_clean >= 3), short} record per '
                       '(pass, cell). The reduction\'s slot rule: the original if clean and valid, else the FIRST clean valid '
                       're-run, else drop the slot; a pass-cell with fewer than 3 clean slots does not gate.')
proto['idle_rule'] = proto['idle_rule'].replace('wait_idle8b.ps1 (window 8 wait_idle8.ps1 + the 8b exe names)',
                                                'wait_idle9a.ps1 (= wait_idle8b.ps1; the 9a cargo exe names are 8b\'s)')
proto['hard_stop'] = proto['hard_stop'].replace('win8b/STOP', 'win9a/STOP')
proto['order'] = proto['order'].replace('(S4-AB: parent then tip, so the two exes are adjacent in every round)',
                                        '(C4-AB: the binaries of a row are adjacent; the Jolt row sits between the JT (Tree) pair '
                                        'and C4-JD, so it is adjacent to the tip J-D and J-T rows in every round)')
proto['own_k'] = ('C4-G4-kd (F3 keep/freeze, not gating) keeps the Q3 recipe\'s own K = 3 (one pass x three rounds). C4-BR and C4-G4 run '
                  'the letter (K = 9); C4-BR may be cut to K = 3 by setting its block "passes" to 1 (the lane\'s Q5 recommendation).')
proto['criterion'] = ('CRITERION_HOME per process; every expected id and no other, on stdout AND on disk (criterion_check); '
                      'criterion warms each benchmark, so no separate warm-up process')

# ------------------------------------------------------------------ binaries
binaries = {
    'tip': {'exe': 'bin/runner_tip_50e31f1a.exe', 'commit': TIP_SHA, 'kind': 'runner', 's4': 'on', 'sha256_pin': PINS['tip'],
            'expect_config': {'tree_brute_max_rows': 128, 'broadphase_select': 'Manual'},
            'what': 'TIP = u/phys-tree-c4 @ 50e31f1a (the Tree is the default), jolt_parity_pyramid --profile parity from a git-archive export'},
    'parent': {'exe': 'bin/runner_parent_3d9433ae.exe', 'commit': PARENT_SHA, 'kind': 'runner', 's4': 'on',
               'sha256_pin': PINS['parent'],
               'expect_config': {'tree_brute_max_rows': 144, 'broadphase_select': 'Manual'},
               'what': 'PARENT = git merge-base u/phys-tree-c4 integ/unified = trunk 3d9433ae (AllPairs is the default), the same build from its own export'},
    'g4rT': {'exe': 'bin/broadphase_g4rT_50e31f1a.exe', 'commit': TIP_SHA + ' + bin/g4ref.patch', 'kind': 'criterion',
             'sha256_pin': PINS['g4rT'],
             'what': 'benches/broadphase.rs of the TIP export with the 28-size G4_SIZES line (bin/g4ref.patch, by content), cargo bench --no-run --locked (bench profile)'},
    'g4r7': {'exe': 'bin/bpbench_g4ref_93b2615b.exe', 'commit': '93b2615b + window 7 variant/g4ref.diff (27 sizes, 17..256)',
             'kind': 'criterion', 'sha256_pin': PINS['g4r7'],
             'what': "window 7 wave 2's Q3 instrument, a byte copy (sha256 checked) of 7dc37fc3.../win7b/bin/bpbench_g4ref_93b2615b.exe"},
    'g4r8b': {'exe': 'bin/broadphase_g4ref_16191fda.exe', 'commit': '16191fda + win8b/bin/g4ref.patch', 'kind': 'criterion',
              'sha256_pin': PINS['g4r8b'],
              'what': "window 8b's G4 instrument, a byte copy (sha256 checked) of win8b/bin/broadphase_g4ref_16191fda.exe"},
    'j56': {'exe': 'D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe', 'commit': 'Jolt v5.6.0 (wt-v5.6.0-parity, repository patch)',
            'kind': 'jolt', 'sha256_pin': PINS['j56'], 'what': "windows 7/8's j56 (P0 Distribution build), run in place"},
}
# The lane's file named j56 with the same sha; the copied criterion exes replace the absolute scratchpad paths of the lane's file.
assert LANE['binaries']['j56']['sha256'] == PINS['j56'] and LANE['binaries']['g4r7']['sha256'] == PINS['g4r7']
assert LANE['binaries']['g4r8b']['sha256'] == PINS['g4r8b']
dev('g4r7 and g4r8b are byte copies under bin/ (the lane file pointed into two other sessions\' scratchpads); sha256 equal')
dev('tip/parent/g4rT carry sha256_pin and (runner) expect_config: tree_brute_max_rows 128 (tip) / 144 (parent) is checked on every process')

# ------------------------------------------------------------------ blocks
blocks = [
    {'name': 'C4-BR', 'item': 'tree C4 / M9', 'priority': 1 if LANE_ORDER else 3, 'passes': 1 if BR_K3 else 3, 'rounds': 3,
     'note': 'the all_pairs bracket (M9 verdict b: placement only): g4r7, g4r8b, g4rT adjacent in every round, one process per exe per '
             'round, CRITERION_HOME per process. K 9 (the letter); passes 1 = the lane\'s Q5 recommendation (K 3, saves ~30 min)'},
    {'name': 'C4-G4', 'item': 'tree C4 thresholds', 'priority': 2 if LANE_ORDER else 4, 'passes': 3, 'rounds': 3,
     'note': 'sizes 96..160 step 8, all_pairs and tree, uniform and disparity: 36 ids; K 9'},
    {'name': 'C4-G4-kd', 'item': 'tree C4 / F3 keep-freeze', 'priority': 3 if LANE_ORDER else 5, 'passes': 1, 'rounds': 3,
     'note': 'tree_kd at 96/112/128 (6 ids), the Q3 recipe\'s own K = 3, recorded for F3\'s keep/freeze, not gating'},
    {'name': 'C4-AB', 'item': 'tree C4 merge gate + Jolt 5.6', 'priority': 4 if LANE_ORDER else 1, 'warmup': ['C4-JD', 'tip', 8]},
    {'name': 'C4-G5', 'item': 'tree C4 G5', 'priority': 5 if LANE_ORDER else 2, 'warmup': ['C4-JD-armed', 'tip', 8]},
]
if not LANE_ORDER:
    dev('BLOCK ORDER differs from the cut (BR, G4, G4-kd, AB, G5): AB, G5 first, then BR, G4, G4-kd. Reason: the merge gate (AB + G5, ~70 min, '
        'deterministic cost) must not be the block a cutoff skips when the criterion blocks run long (a criterion process costs a per-process '
        'startup of minutes and its wall swings 2x with machine load); a cut criterion block is re-runnable (--resume). Ruling 5 ("check the '
        'all-pairs slowdown against window 7 first") is about reading G4, and BR still precedes G4. --lane-order restores the order of the cut.')
dev('AUDIT-1 (window 9a pre-launch audit, round 1): W1 the re-run allowance is max(0.5 x the pass wall, 4 waves x 1.3 x the longest process), so a one-cell criterion block (C4-G4, C4-G4-kd) can re-run a slot to the full wave depth (the fraction alone gave ONE re-run); W2 an overlay row may carry insert_after (its row-list position decides its neighbours; an appended row is the LAST cell of every W group)')
dev('C4-JOLT is not a block: its row is folded into C4-AB (W 1/2/4/8/16, K 9, adjacent to C4-JD each round) by the orchestrator\'s instruction')
dev('C4-G4-kd is its own block (own K 3): the driver takes K from the block, and the lane\'s file put it in the K-9 block')
dev('C4-G5 is included although the task text names only C4-AB, C4-G4 and C4-BR: the lane\'s merge gate reads "C4-JD not slower ... AND G5 recorded" (cut.md §5)')

# ------------------------------------------------------------------ rows
lane_rows = {r['id']: r for r in LANE['rows']}


def clone(rid, **over):
    r = json.loads(json.dumps(lane_rows[rid]))
    r.update(over)
    return r


def g4_ids(kernels, sizes, fams=('uniform', 'disparity')):
    return sorted(f'bp_g4_{f}/{k}/{n}' for f in fams for k in kernels for n in sizes)


rows = []
# -- criterion
br = clone('C4-BR', binaries=['g4r7', 'g4r8b', 'g4rT'], test_filter='^bp_g4_uniform/all_pairs/144$',
           test_expect=['bp_g4_uniform/all_pairs/144'])
assert br['expect'] == g4_ids(('all_pairs', 'tree'), (144, 256)), 'lane BR expect list differs from the regex'
rows.append(br)
sizes = list(range(96, 161, 8))
g4 = clone('C4-G4', expect=g4_ids(('all_pairs', 'tree'), sizes), test_filter='^bp_g4_uniform/all_pairs/128$',
           test_expect=['bp_g4_uniform/all_pairs/128'])
assert len(g4['expect']) == g4['expect_count'] == 36
rows.append(g4)
kd = clone('C4-G4-kd', blocks=['C4-G4-kd'], expect=g4_ids(('tree_kd',), (96, 112, 128)), test_filter='^bp_g4_uniform/tree_kd/128$',
           test_expect=['bp_g4_uniform/tree_kd/128'])
assert len(kd['expect']) == kd['expect_count'] == 6
rows.append(kd)
# -- C4-AB, in the order that puts the Jolt row next to the tip J-T and J-D rows (a reversed pass keeps the adjacency)
rows.append(clone('C4-JA'))
rows.append(clone('C4-JT'))
jolt = clone('C4-jolt56', blocks=['C4-AB'], workers=[1, 2, 4, 8, 16])
jolt['note'] = ('Jolt 5.6 in-block (orchestrator instruction): -s=Pyramid -q=Discrete -f, -t=W -i=500; the driver checks the parity '
                'patch banner (receipt=0, allow_sleep=0 = sleeping off), one stat line, threads = W and the hash '
                + jolt['jolt_hash'] + ' on every process')
rows.append(jolt)
rows.append(clone('C4-JD'))
rows.append(clone('C4-JDap'))
rows.append(clone('C4-JDpar'))
rows.append(clone('C4-rung'))
# -- C4-G5 (S16 needs a recorded fixture: the lane says "hash recorded at pre-flight")
for rid in ('C4-JD-armed', 'C4-JDap-armed', 'C4-R', 'C4-Rap'):
    rows.append(clone(rid))
rows.append(clone('C4-S16', pose_ref='S16500', tree_diag_expect={'static_rebuilds': 0, 'members': 0, 'evictions': 0}))
rows.append(clone('C4-S16ap', pose_ref='S16500'))
dev('C4-S16 / C4-S16ap get pose_ref S16500 (a fixture recorded at pre-flight on TIP --broadphase allpairs; the lane\'s file had null, '
    'which would make the driver pass --expect-pose None.pose: a runner row must have a pose gate)')

dev('C4-S16 expects TreeDiag static_rebuilds 0 / members 0 (16 rows <= tree_brute_max_rows 128: the Tree never builds - that IS the brute-path proof); the lane file had null, which the driver reads as the default {1,1,0} and reds')
for r in rows:
    if r.get('armed') and '--arm-profiler' in r['args']:
        r['args'] = [a for a in r['args'] if a != '--arm-profiler']      # the driver appends it once for every armed row
        if 'armed_flag_deduped' not in DEVIATIONS:
            DEVIATIONS.append('armed_flag_deduped')
DEVIATIONS[:] = [('armed rows: --arm-profiler is removed from the lane args (the driver appends it once for row["armed"]; the lane file '
                  'had it twice on the command line)') if d == 'armed_flag_deduped' else d for d in DEVIATIONS]
for r in rows:
    r.pop('expect_count', None)
    r.pop('rule_ref', None)

out = {
    'window': 'physics window 9a (2026-09-30): PARENT trunk 3d9433ae vs TIP u/phys-tree-c4 50e31f1a; Jolt 5.6 in C4-AB',
    'protocol': proto,
    'binaries': binaries,
    'blocks': blocks,
    'rows': rows,
    'reserved': [{'block': 'rapier', 'how': 'rows9a.extra.json (PREP.md, "Adding the Rapier rows"); example: rows9a.extra.example.json'}],
    'lane_rules': {'C4-G4': LANE['rows'][1]['rule'], 'C4-BR': LANE['rows'][0]['read']},
    '_derived': {'from': 'tree-c4/window9_c4_rows.json (amended)', 'deviations': DEVIATIONS},
}
with open(os.path.join(W9, 'rows9a.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(out, f, indent=1)
print(f'rows9a.json: {len(rows)} rows, {len(blocks)} blocks, {len(binaries)} binaries')
for d in DEVIATIONS:
    print(' -', d)
