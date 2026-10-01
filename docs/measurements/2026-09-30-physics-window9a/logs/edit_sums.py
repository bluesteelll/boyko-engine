sums = open('bin/SHA256SUMS', encoding='utf-8').read()
assert 'rapier_parity' not in sums
if not sums.endswith('\n'):
    sums += '\n'
sums += ('736c2a069f14a2ad4846cb787cb14f080ca2f1401896130dc7188f4410c9b307 *bin/rapier_parity_simd8_736c2a06.exe\n'
         '1e5136cada9777ab1bdf5eb1abb463bde6e395caaa5a58282779eb13ae1e0b77 *bin/rapier_parity_simd4_1e5136ca.exe\n')
open('bin/SHA256SUMS', 'w', encoding='utf-8', newline='\n').write(sums)
c = open('bin/COMMIT.txt', encoding='utf-8').read()
if not c.endswith('\n'):
    c += '\n'
c += """
2026-09-30, the Rapier block (C4-RAPIER, rows9a.extra.json; keys rs8 / rs4, kind rapier). Both exes are BYTE COPIES (sha256 checked against the
harness's own SHA256SUMS, gate/rapier/pins.json and the orchestrator's ruling) of D:/tmp/rapier-parity/gate/bin/rapier-parity-simd{8,4}.exe, the
harness rev 2 (reviewed GO); the harness dir was only read. Built by the harness's build.sh: rustc 1.98.1 msvc, `cargo build --release --locked`, fat LTO,
codegen-units 1, -C target-cpu=x86-64-v3 (the harness's .cargo/config.toml), rapier3d 0.36.0. Source state = the harness's gate/bin/SOURCES.sha256
(copied to gate/rapier/SOURCES.sha256): src/main.rs a27f8b55..., Cargo.toml c0368ed7..., Cargo.lock 36dc81e0..., build.sh ff3cfeaf..., .cargo/config.toml 2fab97fc...
(re-checked equal to the harness's current files at copy time); the exe also reports source_fnv1a64 in every SUMMARY (pinned in gate/rapier/pins.json).
rapier_parity_simd8_736c2a06.exe   (key rs8, arm simd8, 8 SIMD lanes: rapier3d default + parallel + simd8; parry3d parallel + simd8; features_simd8.txt)
rapier_parity_simd4_1e5136ca.exe   (key rs4, arm simd4, 4 SIMD lanes: rapier3d default + parallel, no simd8; the harness's simd4 feature is a marker; features_simd4.txt)
The det build (rapier-parity-det.exe f34291c5...) is a receipt build and is NOT a window arm: it is not copied.
Names use underscores on purpose: the idle-rule / witness lists name the harness's own exes (rapier-parity-simd8.exe, rapier-parity-simd4.exe), so a copy
under that name would void its own window. Pins: gate/rapier/pins.json (sha256 in rows9a.extra.json `rapier.pins_sha256`), fixtures RD8 RD4 RM8 RM4 in
gate/fixtures/ (sha256 = the pins' fixture_sha256, checked at every start by verify_rapier_pins).
"""
open('bin/COMMIT.txt', 'w', encoding='utf-8', newline='\n').write(c)
print('ok')
