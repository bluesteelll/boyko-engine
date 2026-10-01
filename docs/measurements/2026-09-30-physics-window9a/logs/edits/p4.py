import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
P = 'tools/window9a_run.py'
apply(P, [
 ("""    if not TEST:
        args += ['--expect-pose', pose_path(row)]
    return args
""",
  """    if not TEST and row.get('pose_ref'):     # a 'runner' row without a pose_ref is refused at load; 'rapier' rows may omit it
        args += ['--expect-pose', pose_path(row)]
    return args
"""),
 ("""SUMS = {}
for line in open(os.path.join(BIN, 'SHA256SUMS'), encoding='utf-8'):""",
  """for _r in ROWS.values():
    if _r['kind'] == 'runner' and not _r.get('pose_ref'):
        sys.exit(f'row {_r["id"]}: a runner row needs a pose_ref (the pose gate must not be optional)')
SUMS = {}
for line in open(os.path.join(BIN, 'SHA256SUMS'), encoding='utf-8'):"""),
 # verify fixtures helper next to verify_binaries
 ("def append(path, rec):\n",
  '''def verify_fixtures():
    """Every pose_ref used by a row of a selected block names a recorded fixture: the .pose file exists and fixtures.json has
    its hash (a missing hash would make the driver's pose check compare against None)."""
    bad = []
    used = sorted({ROWS[rid]['pose_ref'] for b in BLOCKS for rid, _, _ in cell_order(b['name']) if ROWS[rid].get('pose_ref')})
    for ref in used:
        if not os.path.exists(os.path.join(FIX, f'{ref}.pose')):
            bad.append(f'{ref}.pose missing')
        if not (FIXTURES.get(ref) or {}).get('hash'):
            bad.append(f'{ref} has no hash in fixtures.json')
    return bad


def append(path, rec):
'''),
 ("""    bad = verify_binaries()
    if bad:
        return finish(3, f'STOP at start: binaries do not match bin/SHA256SUMS {bad}', state, None, {'timed': 0})
""",
  """    bad = verify_binaries()
    if bad:
        return finish(3, f'STOP at start: binaries do not match bin/SHA256SUMS {bad}', state, None, {'timed': 0})
    bad = verify_fixtures()
    if bad and not TEST:
        return finish(3, f'STOP at start: fixtures {bad}', state, None, {'timed': 0})
"""),
 # schedule header lines
 ("""    bad = verify_binaries()
    print(f'binaries: {"all match bin/SHA256SUMS" if not bad else bad}')
""",
  """    print(f're-run rule (ruling 8): a slot whose latest attempt is contaminated or invalid is re-run in waves at the end of the pass '
          f'until its pass-cell has every slot clean; at most {RERUN_MAX_WAVES} waves and {RERUN_STAGE_FRAC} x the pass wall; '
          f'a pass-cell with k_clean < {MIN_K_GATE} does not gate (passcell records in runs.jsonl)')
    bad = verify_binaries()
    print(f'binaries: {"all match bin/SHA256SUMS (+ sha256_pin)" if not bad else bad}')
    bad = verify_fixtures()
    print(f'fixture hashes: {"every pose_ref recorded" if not bad else bad}')
"""),
])
