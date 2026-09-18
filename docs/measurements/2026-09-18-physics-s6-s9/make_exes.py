import json, hashlib, sys
MQ = sys.argv[1]
full = {'d5782d43': 'd5782d43e58d01a966e43be21daa791c8360e7d9', 'b74f7ee8': 'b74f7ee89609acd4a54e52a86f13c02cb30067ec',
        'd552be05': 'd552be05be4b4f063b6cb39ddfd63eb4688f83fd', 'a56007ab': 'a56007ab6fd31c7aa0abbe144ad2c7655ff04915',
        '08fe7b9f': '08fe7b9fd88302791db1105992f3218c0b9deed5', '8d656ad8': '8d656ad825a8d3ee4eb3ea90ff2c4796b3630f3e'}
arms = {}
for sha in full:
    exes = {}
    for line in open(f'{MQ}/logs/{sha}.proof.json', encoding='utf-8'):
        if not line.startswith('{'): continue
        m = json.loads(line)
        if m.get('reason') == 'compiler-artifact' and m['target']['kind'] == ['bench']:
            p = m['executable'].replace(chr(92), '/')
            exes[m['target']['name']] = {'exe': p, 'sha256': hashlib.sha256(open(p, 'rb').read()).hexdigest(), 'fresh_on_rebuild': m['fresh']}
    arms[sha] = {'commit': full[sha], 'worktree': f'D:/wt/mq-{sha}', 'target_dir': f'D:/wt/_targets/mq-{sha}',
                 'tree_state': 'clean at commit' if sha != 'd552be05' else
                 'commit + s8 arm-A port (untracked benches/sleeping_pipeline.rs, modified crates/boyko_physics/Cargo.toml; see s8_armA_port.diff). jolt_parity_pyramid/row_identity_churn/soft_step_sp2 were built BEFORE the port and are byte-identical after it (sha256 unchanged).',
                 'benches': dict(sorted(exes.items()))}
doc = {
  'generated': '2026-09-18, tester prepare step; nothing timed',
  'toolchain': 'stable-x86_64-pc-windows-msvc, rustc 1.98.1 (48a229cea 2026-09-01); profile bench (codegen-units=1, lto=false); RUSTFLAGS unset; target-cpu=x86-64-v3 from .cargo/config.toml',
  'invoke': 'cd <worktree>; CARGO_TARGET_DIR=<target_dir> <exe> --bench [criterion filter]. WITHOUT --bench criterion runs TEST mode (one iteration, no timing).',
  'entries': {
    's6': {'A': 'd5782d43', 'B': 'b74f7ee8', 'benches': {'jolt_parity_pyramid': 'A,B: full_step/1, full_step/4', 'row_identity_churn': 'B only'}},
    's7': {'A': 'd552be05', 'B': 'a56007ab', 'benches': {'jolt_parity_pyramid': 'full_step/1, full_step/4', 'row_identity_churn': 'stable', 'soft_step_sp2': 'soft_step_sp2/coupled'}},
    's8': {'A': 'd552be05 (ported sleeping_pipeline, contact_wakes body = 0)', 'B': 'a56007ab', 'benches': {'sleeping_pipeline': 'all three arms', 'jolt_parity_pyramid': 'full_step/1'}},
    's9': {'A': '08fe7b9f', 'B': '8d656ad8', 'benches': {'sleeping_pipeline': 'pyramid_sleeping_off, pyramid_awake_sleeping_on', 'jolt_parity_pyramid': 'full_step/1, full_step/4'}},
  },
  'arms': arms,
  'do_not_use': {
    'D:/wt/_targets/mq-d552be05/release/deps/sleeping_pipeline-892ed66c006bb282.exe': 'release-profile (fat LTO) TEST build from the structural receipt, not the bench-profile binary',
    'D:/wt/_targets/mq-a56007ab/release/deps/sleeping_pipeline-892ed66c006bb282.exe': 'same',
    'D:/wt/_targets/mq-probe-*': 'probe builds (s9_probe.diff); never time these',
  },
}
json.dump(doc, open(f'{MQ}/exes.json', 'w'), indent=2)
print(json.dumps(arms, indent=1)[:200])
