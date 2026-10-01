"""Window 9a, 2026-09-30: build rows9a.extra.json (the Rapier block C4-RAPIER) and record the four Rapier pose fixtures into
gate/fixtures/fixtures.json. Deterministic and idempotent: run it again after editing, never edit rows9a.extra.json by hand.
  python -B tools/mkextra9a.py
Inputs (all COPIES inside the window dir, so the harness dir is never read at launch): gate/rapier/pins.json, gate/rapier/window9a_rows.md
(the two reviewed rows RP-D / RP-M are PARSED out of its json block, not retyped), bin/rapier_parity_simd{8,4}_<sha8>.exe and
gate/fixtures/R{D,M}{8,4}.pose. Orchestrator rulings for this block: (a) own block C4-RAPIER, priority 1.5 (after C4-AB, before C4-G5), K 9
= 3 passes x 3 rounds, the same pass order (p0 reversed, p1 forward, p2 reversed); the four configurations RP-D/RP-M x simd8/simd4 are
adjacent inside every W group of every round (the row order RP-D, RP-M and each row's binaries [rs8, rs4] make it so: cell_order walks
rows inside a W group and binaries inside a row); adjacency to our C4-JD / C4-JT rows is NOT required (a different block).
One untimed warm-up per exe per pass (window9a_rows.md, Protocol): RP-D rs8 W8 and RP-D rs4 W8.
2026-10-01: the same file also generates the block C4-CGU (priority 0.5: it runs FIRST on --resume) - the tip's default row C4-JD on TWO
binaries, `tip` (the existing exe, [profile.parity] = fat LTO + 16 codegen units) and `tipcgu1` (the SAME 50e31f1a export rebuilt with
CARGO_PROFILE_PARITY_CODEGEN_UNITS=1, tools/build_tipcgu1.sh; nothing else differs). Owner ruling 2026-10-01 #3: codegen-units = 1 is
acceptable for the shipped profile if it measures faster. The row is DERIVED from C4-JD of rows9a.json (same args, workers, steps,
pose_ref, TreeDiag expectation), never retyped; only its id, block, binaries and metric windows differ."""
import copy
import hashlib
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
W9 = os.path.dirname(HERE)

# ---- C4-CGU (2026-10-01). The pin is a CONSTANT here, not read from the file: re-running this generator after the exe was replaced fails.
CGU1_EXE = 'bin/runner_tipcgu1_50e31f1a.exe'
CGU1_SHA256 = '3e1f72babf76987260988481f12ddbe409ecdc308ce141326529ef89b020d771'
CGU_CLAIM = ('PRE-REGISTERED 2026-10-01, before any C4-CGU process was timed (owner ruling 2026-10-01 #3: codegen-units = 1 is acceptable for the '
             'shipped profile if it measures faster): "cgu1 claimed faster than cgu16 at W8 AND not claimed slower at any W -> the shipped/parity '
             'profile moves to cgu 1", under ruling 1 (claim rule: IQR AND SE in every clean block and pooled, min-max beside, STRONG when it '
             'also passes); anything else leaves the profile at cgu 16. The row records the metric windows [100,500) and [0,500); the claim '
             'sentence names no primary one, so the analysis reports both.')


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


pins_path = os.path.join(W9, 'gate', 'rapier', 'pins.json')
pins = json.load(open(pins_path, encoding='utf-8'))
doc = open(os.path.join(W9, 'gate', 'rapier', 'window9a_rows.md'), encoding='utf-8').read()
block = re.search(r'```json\n(.*?)```', doc, re.S).group(1)
doc_rows = [json.loads(l) for l in block.splitlines() if l.strip()]
assert [r['id'] for r in doc_rows] == ['RP-D', 'RP-M'], [r['id'] for r in doc_rows]

EXE = {'rs8': ('simd8', 'bin/rapier_parity_simd8_736c2a06.exe'), 'rs4': ('simd4', 'bin/rapier_parity_simd4_1e5136ca.exe')}
sources = open(os.path.join(W9, 'gate', 'rapier', 'SOURCES.sha256'), encoding='utf-8').read().split()
src_line = '; '.join(f'{sources[i + 1].lstrip("*")} {sources[i][:8]}' for i in range(0, len(sources), 2))
binaries = {}
for key, (arm, exe) in EXE.items():
    sha = sha256(os.path.join(W9, exe))
    assert sha == pins['exe_sha256'][arm], f'{key}: {exe} sha256 {sha} != the pins\' {pins["exe_sha256"][arm]}'
    binaries[key] = {
        'exe': exe, 'kind': 'rapier', 'arm': arm, 'sha256_pin': sha,
        'commit': f'rapier-parity rev 2 (2026-09-30), rapier3d {pins["rapier_version"]} --locked, arm {arm}; sources sha256 (gate/rapier/SOURCES.sha256): {src_line}',
        'what': (f'Rapier {pins["rapier_version"]} parity harness, arm {arm} ({pins["arms"][arm]["lanes"]} SIMD lanes), a byte copy (sha256 checked) of '
                 f'D:/tmp/rapier-parity/gate/bin/rapier-parity-{arm}.exe; run by path, timed with counters off'),
    }

rows = []
for r in doc_rows:
    r = json.loads(json.dumps(r))
    assert r['kind'] == 'rapier' and r['binaries'] == ['rs8', 'rs4'] and r['workers'] == [1, 2, 4, 8, 16] and r['steps'] == 500
    r['blocks'] = ['C4-RAPIER']            # ruling (a): the doc's shared block PAR becomes the window's own block
    r['note'] = ('window 9a Rapier row, rows and void rules V1-V9 of gate/rapier/window9a_rows.md; the four configurations (RP-D/RP-M x rs8/rs4) '
                 'are adjacent at every W of every round (cell_order: rows in list order, binaries in order); validated by window9a_run.'
                 'rapier_check (V1-V9); rows_pin = the per-step mean row-iteration count of the harness\'s W1 receipt twin over each metric '
                 'window, recorded for the analysis (R-ROW-D / R-ROW-M), not gated by the driver')
    rows.append(r)

# ---- the C4-CGU block: binary `tipcgu1`, row C4-JDcgu (derived from C4-JD), block C4-CGU at priority 0.5
base = json.load(open(os.path.join(W9, 'rows9a.json'), encoding='utf-8'))
tip_bin = base['binaries']['tip']
jd = next(r for r in base['rows'] if r['id'] == 'C4-JD')
assert jd['binaries'] == ['tip'] and jd['kind'] == 'runner' and jd['workers'] == [1, 2, 4, 8, 16] and jd['steps'] == 500 and jd['window'] == [0, 500]
assert jd['args'][-2:] == ['--sleeping', 'off'] and '--broadphase' not in jd['args'], jd['args']      # the tip's DEFAULT (Tree), flagless but for sleeping
cgu_path = os.path.join(W9, CGU1_EXE)
assert sha256(cgu_path) == CGU1_SHA256, f'{CGU1_EXE}: sha256 {sha256(cgu_path)} != the constant pin {CGU1_SHA256} (a rebuilt or replaced exe)'
sums_line = f'{CGU1_SHA256} *{CGU1_EXE}'
assert sums_line in open(os.path.join(W9, 'bin', 'SHA256SUMS'), encoding='utf-8').read().splitlines(), 'bin/SHA256SUMS lacks the tipcgu1 line'
binaries['tipcgu1'] = {
    'exe': CGU1_EXE, 'commit': tip_bin['commit'] + ' + CARGO_PROFILE_PARITY_CODEGEN_UNITS=1', 'kind': 'runner', 's4': tip_bin['s4'],
    'sha256_pin': CGU1_SHA256, 'expect_config': dict(tip_bin['expect_config']),
    'what': ('TIP (u/phys-tree-c4 @ 50e31f1a) jolt_parity_pyramid --profile parity rebuilt from a second git-archive export with codegen-units = 1 '
             '(tools/build_tipcgu1.sh: the recipe of tools/build_runner.sh plus the environment override CARGO_PROFILE_PARITY_CODEGEN_UNITS=1); '
             'the exe `tip` is the same tree at the profile default (16). The two report the same config receipt and the same pose, so ONLY the sha256 pin '
             'tells them apart'),
}
cgu_row = copy.deepcopy(jd)
cgu_row.update({
    'id': 'C4-JDcgu', 'blocks': ['C4-CGU'], 'binaries': ['tip', 'tipcgu1'], 'metric_windows': [[100, 500], [0, 500]],
    'note': ('codegen-units A/B on the tip default (same args as C4-JD: flagless but for --sleeping off, Tree): `tip` (cgu 16, the existing exe) and '
             '`tipcgu1` (cgu 1) are adjacent at every W of every round (cell_order: binaries inside a row). Pose gate: JT500 for both. ' + CGU_CLAIM),
})
assert cgu_row['args'] == jd['args'] and cgu_row['pose_ref'] == 'JT500' and cgu_row['broadphase'] == 'Tree'
cgu_block = {
    'name': 'C4-CGU', 'item': 'codegen-units 1 vs 16 (tip default, C4-JD)', 'priority': 0.5, 'passes': 3, 'rounds': 3,
    'warmups': [['C4-JDcgu', 'tip', 8], ['C4-JDcgu', 'tipcgu1', 8]],
    'note': ('K 9 = 3 passes x 3 rounds, pass 0 REVERSED / pass 1 forward / pass 2 REVERSED (the window\'s pass order), one untimed warm-up PER EXE per pass '
             '(C4-JDcgu tip W8, C4-JDcgu tipcgu1 W8). Priority 0.5: runs before every other pending block on --resume. ' + CGU_CLAIM),
}

overlay = {
    '_comment': ('Window 9a extra blocks, GENERATED by tools/mkextra9a.py (never edit by hand): the Rapier block C4-RAPIER and (2026-10-01) the codegen-units '
                 'block C4-CGU. Merged over rows9a.json by the driver at start (binaries by key, rows by id, blocks by name; blocks ordered by priority). '
                 'Orchestrator ruling (a): the block C4-RAPIER has priority 1.5 (after C4-AB, before C4-G5), K 9 = 3 passes x 3 rounds, the same pass '
                 'order. C4-CGU has priority 0.5 (before everything still pending on --resume).'),
    'rapier': {'pins': 'gate/rapier/pins.json', 'pins_sha256': sha256(pins_path),
               'rules': 'V1-V9 of gate/rapier/window9a_rows.md, implemented by window9a_run.rapier_check / validate_rapier'},
    'binaries': binaries,
    'blocks': [{
        'name': 'C4-RAPIER', 'item': 'Rapier 0.36 vs boyko J-T (window 9a Rapier rows)', 'priority': 1.5, 'passes': 3, 'rounds': 3,
        'warmups': [['RP-D', 'rs8', 8], ['RP-D', 'rs4', 8]],
        'note': ('K 9 = 3 passes x 3 rounds. One untimed warm-up PER EXE per pass (RP-D rs8 W8, RP-D rs4 W8: window9a_rows.md, Protocol). '
                 'Every Rapier cell is adjacent to its three sibling configurations at each W of each round; ours J-T is a different block '
                 '(C4-AB) and is NOT required to be adjacent. Which arm is faster is decided by the analysis from the rows at every W.')}, cgu_block],
    'rows': rows + [cgu_row],
}
with open(os.path.join(W9, 'rows9a.extra.json'), 'w', encoding='utf-8', newline='\n') as f:
    json.dump(overlay, f, indent=1)

# The four pose fixtures: files under gate/fixtures/ (byte copies of the harness's, sha256 checked against the pins), their hashes in
# fixtures.json (the driver's verify_fixtures reads both; verify_rapier_pins re-checks the sha256 at every start).
fx_path = os.path.join(W9, 'gate', 'fixtures', 'fixtures.json')
fx = json.load(open(fx_path, encoding='utf-8'))
for arm, a in pins['arms'].items():
    for cfg, c in a['cfgs'].items():
        name = os.path.basename(c['fixture']).split('.')[0]
        path = os.path.join(W9, 'gate', 'fixtures', name + '.pose')
        assert sha256(path) == c['fixture_sha256'], f'{name}: sha256 differs from the pins'
        fx[name] = {'hash': c['pose_hash'], 'sha256': c['fixture_sha256'], 'arm': arm, 'cfg': cfg,
                    'recorded_by': 'rapier-parity gate g2 (the W1 --receipt run of the arm), a byte copy of D:/tmp/rapier-parity/gate/fixtures/',
                    'kind': 'rapier'}
with open(fx_path, 'w', encoding='utf-8', newline='\n') as f:
    json.dump(fx, f, indent=1)
print('rows9a.extra.json: 3 rows (RP-D, RP-M, C4-JDcgu), 3 binaries (rs8, rs4, tipcgu1), 2 blocks (C4-RAPIER, C4-CGU); fixtures.json:', sorted(k for k in fx if fx[k].get('kind') == 'rapier'))
