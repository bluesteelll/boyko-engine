"""Writes rows7.json (window 7's run list). Blocks in priority / script order."""
import json, os
W7 = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
J = ['--scene', 'jolt', '--gap', '0.5']
WS = [1, 2, 4, 8, 16]
JM = [[0, 100], [100, 500]]
rows = []
def R(rid, blocks, bins, args, workers, steps=500, window=(0, 500), metric=None, armed=False, pose=None, bp='AllPairs',
      kind='runner', **kw):
    d = {'id': rid, 'blocks': blocks, 'kind': kind, 'binaries': bins, 'args': args, 'workers': workers, 'steps': steps,
         'window': list(window), 'metric_windows': metric or [], 'armed': armed, 'pose_ref': pose, 'broadphase': bp}
    d.update(kw)
    rows.append(d)
# P1 - the Jolt headline, same window, two separated blocks (A first, B last), K=6 each
HB = ['P1A-jolt', 'P1B-jolt']
R('H-trk-tree', HB, ['trk'], J + ['--cfg', 'default', '--broadphase', 'tree'], WS, metric=JM, pose='J500', bp='Tree',
  note='OUR default row on trunk integ/unified 93b2615b (Phase B + thin-box + D-M0; contact reuse off by default on this tree)')
R('H-jolt56', HB, ['j56'], ['-s=Pyramid', '-q=Discrete', '-f'], WS, metric=JM, kind='jolt', jolt_hash='0xb8522b4e3fc62cfe',
  note='Jolt v5.6.0 Distribution, P0 build 918fd2b7, the repository patch; the same exe and spelling as window 3 JOLT56-T')
R('H-trk-ap', HB, ['trk'], J + ['--cfg', 'default', '--broadphase', 'allpairs'], WS, metric=JM, pose='J500',
  note='the AllPairs reference row, same binary')
# P2 - L9 C4 G-TW on 989ca0f0, same binary, reuse off vs on
GB = ['P2-L9GTW']
for scene, a, steps, win, met, poff, pon in (
        ('JA', J + ['--cfg', 'a'], 500, (0, 500), JM, 'J500', 'JAon500c4'),
        ('JD', J + ['--cfg', 'default'], 500, (0, 500), JM, 'J500', 'JDon500c4'),
        ('R', ['--scene', 'rest', '--solver', 'colored'], 1100, (600, 1100), [], 'R1100', 'Ron1100c4')):
    R(f'L9-{scene}-off', GB, ['c4'], a + ['--contact-reuse', 'off'], WS, steps=steps, window=win, metric=met, pose=poff)
    R(f'L9-{scene}-on', GB, ['c4'], a + ['--contact-reuse', 'on'], WS, steps=steps, window=win, metric=met, pose=pon)
R('L9-JA-a-off', GB, ['c4'], J + ['--cfg', 'a', '--contact-reuse', 'off'], [1, 8], metric=[[100, 500]], armed=True, pose='J500',
  note='the armed table (per-stage spans bp/np/setup/colours, per manifold and per pair), W1 and W8')
R('L9-JA-a-on', GB, ['c4'], J + ['--cfg', 'a', '--contact-reuse', 'on'], [1, 8], metric=[[100, 500]], armed=True, pose='JAon500c4')
R('L9-JC', GB, ['c4'], J + ['--cfg', 'a', '--contact-reuse', 'on'], [1], metric=[[100, 500]], armed=True, pose='JAon500c4',
  canary_of='L9-JA-a-on', canary_frac=0.05,
  note='the canary on the L9 binary: --canary-frac 0.05 --canary-ref-ns = the latest valid L9-JA-a-on window mean (same binary, W1); must be SEEN')
# P3 - L11 G9 remainder: J-A at W 2/4/16, parent vs tip (window 5's binaries)
R('G9-JA-mid', ['P3-G9JA'], ['g9p', 'g9t'], J + ['--cfg', 'a'], [2, 4, 16], pose='J500')
# P4 - our per-stage armed spans on the trunk default row, W 1/8
R('S-trk-tree-a', ['P4-trkspans'], ['trk'], J + ['--cfg', 'default', '--broadphase', 'tree'], [1, 8], metric=[[100, 500]],
  armed=True, pose='J500', bp='Tree')
blocks = [
    {'name': 'P1A-jolt', 'item': 'P1', 'warmup': ['H-trk-tree', 'trk', 8]},
    {'name': 'P2-L9GTW', 'item': 'P2', 'warmup': ['L9-JA-off', 'c4', 8]},
    {'name': 'P3-G9JA', 'item': 'P3', 'warmup': ['G9-JA-mid', 'g9t', 4]},
    {'name': 'P4-trkspans', 'item': 'P4', 'warmup': ['S-trk-tree-a', 'trk', 8]},
]
bins = {
    'trk': {'exe': 'bin/runner_93b2615b.exe', 'commit': '93b2615bcea4873e2d3af6b560c6e7997740f67f', 'kind': 'runner'},
    'c4': {'exe': 'bin/runner_989ca0f0.exe', 'commit': '989ca0f0575e054b89bb9c288ad95569ce493734', 'kind': 'runner'},
    'g9p': {'exe': 'bin/runner_0ca312bd.exe', 'commit': '0ca312bd8d696cec739f27d13c98343f4e1d1a9b', 'kind': 'runner'},
    'g9t': {'exe': 'bin/runner_cbd86a65.exe', 'commit': 'cbd86a6525175fc18500b9733997d78c2cbc3e44', 'kind': 'runner'},
    'j56': {'exe': 'D:/tmp/jolt/build-v5.6.0-dist/PerformanceTest.exe', 'commit': 'Jolt v5.6.0 (wt-v5.6.0-parity, v5.6.0-dirty, repository patch)', 'kind': 'jolt'},
}
if os.path.exists(os.path.join(W7, 'bin', 'jolt56prof', 'PerformanceTest.exe')):
    bins['j56p'] = {'exe': 'bin/jolt56prof/PerformanceTest.exe',
                    'commit': 'Jolt v5.6.0 same source, Distribution + PROFILER_IN_DISTRIBUTION=ON', 'kind': 'joltprof'}
    R('P-jolt56-prof', ['P5-joltprof'], ['j56p'], ['-s=Pyramid', '-q=Discrete', '-f', '-p'], [1, 8], metric=JM, kind='joltprof',
      jolt_hash='0xb8522b4e3fc62cfe',
      note='profiled Jolt: per-stage times from the single-frame profile dumps at frames 100/200/300/400 (the harness dumps every 100th frame)')
    blocks.append({'name': 'P5-joltprof', 'item': 'P5', 'warmup': ['P-jolt56-prof', 'j56p', 8]})
blocks.append({'name': 'P1B-jolt', 'item': 'P1', 'warmup': ['H-trk-tree', 'trk', 8]})
proto = {'k_per_cell': 6, 'passes': 2, 'rounds_per_pass': 3, 'receipt_s_between_processes': 5.0, 'receipt_s_before_pass': 10.0,
         'busy_pct_limit': 5.0, 'rerun_contaminated_once_at_end_of_pass': True, 'void_pass_on_build_process': True,
         'placement': 'P-none',
         'idle_rule': 'tools/wait_idle7.ps1: 0 build procs (cargo/rustc/link/lld-link/dxc/clippy-driver/cl/msbuild/miri/cargo-miri/gcc toolchain/cmake/the bench exe names) AND 0 procs whose image is under D:/wt/_targets or D:/wt/mq-* on 3 consecutive 60-s polls, 10-s cpu < 5 %; up to 30 polls (30 min) per wait; a timeout STOPS the window',
         'hard_stop_local': '04:15'}
json.dump({'protocol': proto, 'binaries': bins, 'blocks': blocks, 'rows': rows},
          open(os.path.join(W7, 'rows7.json'), 'w', encoding='utf-8', newline='\n'), indent=1)
print(len(rows), 'rows;', [b['name'] for b in blocks])
