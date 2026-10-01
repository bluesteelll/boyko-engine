import sys
sys.path.insert(0, 'logs/edits')
from patchlib import apply
P = 'tools/window9a_run.py'
apply(P, [
 # process_dir: rerun_no
 ("""def process_dir(pdir, seq, rnd, rid, key, w, tag):
    leaf = f'{seq:03d}_r{rnd}_{rid}_{key}_W{w}'
    if tag == 'rerun':
        leaf += '_R'
""",
  """def process_dir(pdir, seq, rnd, rid, key, w, tag, rerun_no=0):
    leaf = f'{seq:03d}_r{rnd}_{rid}_{key}_W{w}'
    if tag == 'rerun':
        leaf += '_R' if rerun_no <= 1 else f'_R{rerun_no}'   # ruling 8: a slot can be re-run more than once
"""),
 # validate_runner: expect_config
 ("""    w8 = s.get('w8s') if isinstance(s.get('w8s'), dict) else None
    s4 = BINS[rec['binary']].get('s4')
""",
  """    # Window 9a: the binary's own config receipt (an exe/key swap shows here on EVERY row: tip 128, parent 144).
    cfg = s.get('config') if isinstance(s.get('config'), dict) else {}
    for k, v in (BINS[rec['binary']].get('expect_config') or {}).items():
        if cfg.get(k) != v:
            why.append(f'config.{k} {cfg.get(k)} != {v} expected of binary {rec["binary"]}')
    w8 = s.get('w8s') if isinstance(s.get('w8s'), dict) else None
    s4 = BINS[rec['binary']].get('s4')
"""),
 # run_one signature + rerun_no
 ("def run_one(rid, key, w, block, pass_no, attempt_no, rnd, seq, tag, pdir, receipt, env, sampler):\n    row = ROWS[rid]\n    b = BINS[key]\n    cwd = process_dir(pdir, seq, rnd, rid, key, w, tag)\n",
  "def run_one(rid, key, w, block, pass_no, attempt_no, rnd, seq, tag, pdir, receipt, env, sampler, rerun_no=0):\n    row = ROWS[rid]\n    b = BINS[key]\n    cwd = process_dir(pdir, seq, rnd, rid, key, w, tag, rerun_no)\n"),
 ("           'attempt': tag, 'policy': 'P-none', 'timed': tag != 'warmup'}\n",
  "           'attempt': tag, 'rerun_no': rerun_no, 'policy': 'P-none', 'timed': tag != 'warmup'}\n"),
 # runner-shaped kinds: args
 ("    if row['kind'] == 'runner':\n        args = runner_args(row, key, w, cwd)\n",
  "    if row['kind'] in ('runner', 'rapier'):\n        args = runner_args(row, key, w, cwd)\n"),
 # validation dispatch
 ("""    elif row['kind'] in ('jolt', 'joltprof'):
        rec.update(jolt_stats(row, cwd, so))
        rec['invalid'] = validate_jolt(rec, row, w)
    elif row['kind'] == 'micro':""",
  """    elif row['kind'] == 'rapier':
        rec['summary'] = D.parse_summary(so)
        rec.update(runner_stats(row, cwd))
        rec['invalid'] = validate_rapier(rec, row, w)
    elif row['kind'] in ('jolt', 'joltprof'):
        rec.update(jolt_stats(row, cwd, so))
        rec['invalid'] = validate_jolt(rec, row, w)
    elif row['kind'] == 'micro':"""),
 # the rapier validator, placed before jolt_args
 ("def jolt_args(row, w):\n",
  '''def dotted(d, path):
    """d['a']['b'] for 'a.b'; None when any step is missing (a missing key never equals an expectation)."""
    for part in path.split('.'):
        if not isinstance(d, dict) or part not in d:
            return None
        d = d[part]
    return d


def validate_rapier(rec, row, w):
    """RESERVED (window 9a, PREP.md "Adding the Rapier rows"): a runner-shaped external harness (D:/tmp/rapier-parity: --workers
    --steps --window --csv --pose-out --expect-pose --label, one SUMMARY line, exit 0 / 2 bad flag / 3 void / 4 pose). Data-driven:
    the row names what it gates - `expect_summary` {dotted path: value; the string "$W" means the row's W}, `expect_present`
    [dotted paths that must exist], `pose_ref` (a fixture under gate/fixtures, passed as --expect-pose, pose_hash equal to
    the fixture's hash). Nothing is assumed about Rapier's summary beyond `pose_hash`."""
    why = []
    if rec.get('hang'):
        why.append('hang (terminated)')
    if rec.get('exit') != 0:
        why.append(f'exit {rec.get("exit")}')
    if rec.get('mean_ms') is None:
        why.append('no window mean')
    elif rec.get('n_steps') != rec.get('expect_steps'):
        why.append(f'{rec.get("n_steps")} steps in the window, expected {rec.get("expect_steps")}')
    s = rec.get('summary')
    if not s:
        return why + ['no SUMMARY']
    if row.get('pose_ref') and not TEST:
        want = FIXTURES.get(row['pose_ref'], {}).get('hash')
        if want is None or s.get('pose_hash') != want:
            why.append(f'pose {s.get("pose_hash")} != {want}')
    for path, want in (row.get('expect_summary') or {}).items():
        want = w if want == '$W' else want
        got = dotted(s, path)
        if got != want:
            why.append(f'summary.{path} {got!r} != {want!r}')
    for path in row.get('expect_present') or []:
        if dotted(s, path) is None:
            why.append(f'summary.{path} missing')
    return why


def jolt_args(row, w):
'''),
])
