# Lane SL's pose-gate fixtures (SL-1, 2026-10-11)

Ruling 39 (Q1 + Q2): the sleeping-on pose-gate fixtures that lane SL-1 moves are re-recorded HERE,
and no record directory is edited (V2's precedent: the originals are read, never edited). This
directory mirrors both pose-gate fixture sets:

| here | mirrors | read by |
|---|---|---|
| `v2/l9-reuse-on`, `v2/l9-reuse-off`, `v2/l10-reuse-on`, `v2/l10-reuse-off`, `v2/w6`, `v2/w8` | `docs/measurements/2026-09-30-v2-speculative/<same>` | the V2 set (`gate/pose_gates_v2.sh`, through the `*_v2.sh` that `gate/make_v2_scripts.py` writes) |
| `d0/l9-fixtures`, `d0/l9-fixtures-c4` | `docs/measurements/2026-09-23-l9-contact-reuse/fixtures{,-c4}` | the overlap-only set (`gate/pose_gates_d0.sh`, through `gate/l10/*.sh`) |
| `d0/l10-fixtures`, `d0/l10-fixtures-reuse-off` | `docs/measurements/2026-09-23-l10-sleeping/fixtures{,-reuse-off}` | the same |
| `d0/w6-fixtures` | `docs/measurements/2026-09-24-physics-window6/gate/fixtures` (the four files the gate reads) | the same |

`gate/l10/*.sh` point their five fixture directories here and pin this directory's d0 hashes;
`make_v2_scripts.py` maps them onto `v2/` and reads `logs/record.tsv` for the V2 hashes. The gate
commands and their tallies (264/26/17, both negatives exit 4) are unchanged.

## How it was recorded

`bash record_sl_fixtures.sh <exe> <this directory> <logdir>` with the lane's parity runner
(`cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid`), msvc. Every row
both sets read is recorded at W1 — sleeping off as well as on; a W8 file is a copy of its W1 file, so a
W8 gate row checks worker-count identity against the W1 bytes. The d0 rows run through
`gate/runner_d0.sh`, which appends `--speculative-distance 0 --speculative-velocity-cap 0` (the args
below are the recorder's, without them). `logs/record.tsv` is every row's directory, name, exit code,
pose hash and args; `SHA256SUMS` covers every `.pose` file here.

The same recorder was run with the parent's runner (SL-2a, `f1f733f6`) into a scratch directory: it
reproduces all 105 files of the record directories byte for byte (both sets, W8 copies included), so the
recorder's rows are the ones the originals were recorded with.

## What moved

- **Sleeping off (39 files): nothing.** Every sleeping-off file is byte-equal to the record directory's
  original and to the parent's re-recording. They are here so each set reads one directory.
- **The V2 set's 34 sleeping-on files: the velocity bytes only.** Positions and rotations are bit-equal
  to the parent's (the masked hash: FNV-1a 64 over fields 0-2 and 6-9 of every 13-float record), and
  every row freezes on the parent's step: under V2 none of these rows rose at an attempt, so SL only
  zeroes the stored velocity a frozen body kept.
- **The overlap-only set's 32 sleeping-on files: poses moved.** Under `d = 0` every one of these rows
  rose at its first latch attempt on the parent (1240 or 3720 contact wakes) and froze one or two
  attempts later; under SL each first attempt's count change is re-keyed by the seal (1240 rows), so the
  pile freezes at its first attempt, 60 steps or more earlier, from a pose its later attempts would have
  moved. That is the cut's prediction (§4.1, "a pre-V2 attempt-step rise is re-keyed and moves
  positions: reported").
- **The negatives still see a difference:** the reuse-on and reuse-off `R-S` files differ in both sets
  (V2 `0x9a30f553cdd9d7eb` / `0x413825ded25d21ed`, d0 `0xa42b663994f7b36d` / `0xd70e51f6ae4dd2b8`).

Not mirrored: window 6's `J500`, `JAon500`, `R1100`, `Ron1100`, `S16-300` and `fixtures.json`, which no
pose gate reads.

## Old / new, per sleeping-on row (W1; the W8 file is its copy)

| set/dir | file | verdict | parent pose hash | SL-1 pose hash | positions+rotations hash (parent = SL-1 when velocity-only) | first frozen step parent → SL-1 | SL-1 settle re-keys / seal breaks / contact wakes | args |
|---|---|---|---|---|---|---|---|---|
| v2/l9-reuse-on | R-S_W1 (W8: copy) | VELOCITY-ONLY | `0xfcf3d49356b45a08` (contact wakes 0) | `0x9a30f553cdd9d7eb` | `0x35be10df71a4ec7b` | 86 → 86 | rk0/br0/cw0 | `--scene rest --sleeping on --steps 600` |
| v2/l9-reuse-on | J-Son_W1 (W8: copy) | VELOCITY-ONLY | `0x130c76cb6b463ab8` (contact wakes 0) | `0xc637a0517adabab7` | `0x832c403cd7b63347` | 147 → 147 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600` |
| v2/l10-reuse-on | R-S_W1 (W8: copy) | VELOCITY-ONLY | `0xfcf3d49356b45a08` (contact wakes 0) | `0x9a30f553cdd9d7eb` | `0x35be10df71a4ec7b` | 86 → 86 | rk0/br0/cw0 | `--scene rest --sleeping on --steps 600` |
| v2/l10-reuse-on | J-Son_W1 (W8: copy) | VELOCITY-ONLY | `0x130c76cb6b463ab8` (contact wakes 0) | `0xc637a0517adabab7` | `0x832c403cd7b63347` | 147 → 147 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600` |
| v2/l10-reuse-on | J-A-on_W1 (W8: copy) | VELOCITY-ONLY | `0x130c76cb6b463ab8` (contact wakes 0) | `0xc637a0517adabab7` | `0x832c403cd7b63347` | 147 → 147 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600` |
| v2/l10-reuse-on | R-on_W1 (W8: copy) | VELOCITY-ONLY | `0xfcf3d49356b45a08` (contact wakes 0) | `0x9a30f553cdd9d7eb` | `0x35be10df71a4ec7b` | 86 → 86 | rk0/br0/cw0 | `--scene rest --solver colored --sleeping on --steps 600` |
| v2/l10-reuse-on | J-D-on_W1 (W8: copy) | VELOCITY-ONLY | `0x130c76cb6b463ab8` (contact wakes 0) | `0xc637a0517adabab7` | `0x832c403cd7b63347` | 147 → 147 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg default --sleeping on --steps 600` |
| d0/l9-fixtures-c4 | R-S_W1 (W8: copy) | POSES-MOVED | `0xb7f1e9e8f91f75ab` (contact wakes 3720) | `0xa42b663994f7b36d` | `0x0763860bfa825944 → 0xc54cfc6a31fd65ed` | 250 → 70 | rk1240/br0/cw0 | `--scene rest --sleeping on --steps 600` |
| d0/l9-fixtures-c4 | J-Son_W1 (W8: copy) | POSES-MOVED | `0x3db47fae414b655c` (contact wakes 1240) | `0x74c14f3a08e24421` | `0xe64f927f27605db8 → 0x0924f2418b612a51` | 274 → 214 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600` |
| d0/l10-fixtures | R-S_W1 (W8: copy) | POSES-MOVED | `0xb7f1e9e8f91f75ab` (contact wakes 3720) | `0xa42b663994f7b36d` | `0x0763860bfa825944 → 0xc54cfc6a31fd65ed` | 250 → 70 | rk1240/br0/cw0 | `--scene rest --sleeping on --steps 600` |
| d0/l10-fixtures | J-Son_W1 (W8: copy) | POSES-MOVED | `0x3db47fae414b655c` (contact wakes 1240) | `0x74c14f3a08e24421` | `0xe64f927f27605db8 → 0x0924f2418b612a51` | 274 → 214 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600` |
| d0/l10-fixtures | J-A-on_W1 (W8: copy) | POSES-MOVED | `0x3db47fae414b655c` (contact wakes 1240) | `0x74c14f3a08e24421` | `0xe64f927f27605db8 → 0x0924f2418b612a51` | 274 → 214 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600` |
| d0/l10-fixtures | R-on_W1 (W8: copy) | POSES-MOVED | `0xb7f1e9e8f91f75ab` (contact wakes 3720) | `0xa42b663994f7b36d` | `0x0763860bfa825944 → 0xc54cfc6a31fd65ed` | 250 → 70 | rk1240/br0/cw0 | `--scene rest --solver colored --sleeping on --steps 600` |
| d0/l10-fixtures | J-D-on_W1 (W8: copy) | POSES-MOVED | `0x3db47fae414b655c` (contact wakes 1240) | `0x74c14f3a08e24421` | `0xe64f927f27605db8 → 0x0924f2418b612a51` | 274 → 214 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg default --sleeping on --steps 600` |
| v2/l9-reuse-off | R-S_W1 (W8: copy) | VELOCITY-ONLY | `0xe99ed04348f5d898` (contact wakes 0) | `0x413825ded25d21ed` | `0x392834e09920966d` | 82 → 82 | rk0/br0/cw0 | `--scene rest --sleeping on --steps 600 --contact-reuse off` |
| v2/l9-reuse-off | J-Son_W1 (W8: copy) | VELOCITY-ONLY | `0xc671831894394df6` (contact wakes 0) | `0x4aec2a5f78f488d0` | `0x6d926dea060f48e0` | 142 → 142 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600 --contact-reuse off` |
| v2/l10-reuse-off | R-S_W1 (W8: copy) | VELOCITY-ONLY | `0xe99ed04348f5d898` (contact wakes 0) | `0x413825ded25d21ed` | `0x392834e09920966d` | 82 → 82 | rk0/br0/cw0 | `--scene rest --sleeping on --steps 600 --contact-reuse off` |
| v2/l10-reuse-off | J-Son_W1 (W8: copy) | VELOCITY-ONLY | `0xc671831894394df6` (contact wakes 0) | `0x4aec2a5f78f488d0` | `0x6d926dea060f48e0` | 142 → 142 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600 --contact-reuse off` |
| v2/l10-reuse-off | J-A-on_W1 (W8: copy) | VELOCITY-ONLY | `0xc671831894394df6` (contact wakes 0) | `0x4aec2a5f78f488d0` | `0x6d926dea060f48e0` | 142 → 142 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600 --contact-reuse off` |
| v2/l10-reuse-off | R-on_W1 (W8: copy) | VELOCITY-ONLY | `0xe99ed04348f5d898` (contact wakes 0) | `0x413825ded25d21ed` | `0x392834e09920966d` | 82 → 82 | rk0/br0/cw0 | `--scene rest --solver colored --sleeping on --steps 600 --contact-reuse off` |
| v2/l10-reuse-off | J-D-on_W1 (W8: copy) | VELOCITY-ONLY | `0xc671831894394df6` (contact wakes 0) | `0x4aec2a5f78f488d0` | `0x6d926dea060f48e0` | 142 → 142 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg default --sleeping on --steps 600 --contact-reuse off` |
| d0/l9-fixtures | R-S_W1 (W8: copy) | POSES-MOVED | `0x2a2b7926a48aab00` (contact wakes 3720) | `0xd70e51f6ae4dd2b8` | `0xaa8d8e0f6118df32 → 0x3c6a731c7784a2c8` | 248 → 68 | rk1240/br0/cw0 | `--scene rest --sleeping on --steps 600 --contact-reuse off` |
| d0/l9-fixtures | J-Son_W1 (W8: copy) | POSES-MOVED | `0xcc2a5400c66eecce` (contact wakes 1240) | `0x13d95ddbd8f6907e` | `0xcfe7596eeecf89a7 → 0x156318f8c69bc63e` | 265 → 205 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600 --contact-reuse off` |
| d0/l10-fixtures-reuse-off | R-S_W1 (W8: copy) | POSES-MOVED | `0x2a2b7926a48aab00` (contact wakes 3720) | `0xd70e51f6ae4dd2b8` | `0xaa8d8e0f6118df32 → 0x3c6a731c7784a2c8` | 248 → 68 | rk1240/br0/cw0 | `--scene rest --sleeping on --steps 600 --contact-reuse off` |
| d0/l10-fixtures-reuse-off | J-Son_W1 (W8: copy) | POSES-MOVED | `0xcc2a5400c66eecce` (contact wakes 1240) | `0x13d95ddbd8f6907e` | `0xcfe7596eeecf89a7 → 0x156318f8c69bc63e` | 265 → 205 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600 --contact-reuse off` |
| d0/l10-fixtures-reuse-off | J-A-on_W1 (W8: copy) | POSES-MOVED | `0xcc2a5400c66eecce` (contact wakes 1240) | `0x13d95ddbd8f6907e` | `0xcfe7596eeecf89a7 → 0x156318f8c69bc63e` | 265 → 205 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping on --steps 600 --contact-reuse off` |
| d0/l10-fixtures-reuse-off | R-on_W1 (W8: copy) | POSES-MOVED | `0x2a2b7926a48aab00` (contact wakes 3720) | `0xd70e51f6ae4dd2b8` | `0xaa8d8e0f6118df32 → 0x3c6a731c7784a2c8` | 248 → 68 | rk1240/br0/cw0 | `--scene rest --solver colored --sleeping on --steps 600 --contact-reuse off` |
| d0/l10-fixtures-reuse-off | J-D-on_W1 (W8: copy) | POSES-MOVED | `0xcc2a5400c66eecce` (contact wakes 1240) | `0x13d95ddbd8f6907e` | `0xcfe7596eeecf89a7 → 0x156318f8c69bc63e` | 265 → 205 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg default --sleeping on --steps 600 --contact-reuse off` |
| v2/w6 | Offp-J1000 (W8: copy) | VELOCITY-ONLY | `0x130c76cb6b463ab8` (contact wakes 0) | `0xc637a0517adabab7` | `0x832c403cd7b63347` | 147 → 147 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on --steps 1000` |
| v2/w6 | Son-J1000 (W8: copy) | VELOCITY-ONLY | `0xc671831894394df6` (contact wakes 0) | `0x4aec2a5f78f488d0` | `0x6d926dea060f48e0` | 142 → 142 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping --contact-reuse off --steps 1000` |
| v2/w6 | RSOffp800 (W8: copy) | VELOCITY-ONLY | `0xfcf3d49356b45a08` (contact wakes 0) | `0x9a30f553cdd9d7eb` | `0x35be10df71a4ec7b` | 86 → 86 | rk0/br0/cw0 | `--scene rest --sleeping --broadphase tree --contact-reuse on --steps 800` |
| v2/w6 | RS800 (W8: copy) | VELOCITY-ONLY | `0xe99ed04348f5d898` (contact wakes 0) | `0x413825ded25d21ed` | `0x392834e09920966d` | 82 → 82 | rk0/br0/cw0 | `--scene rest --sleeping --contact-reuse off --steps 800` |
| d0/w6-fixtures | Offp-J1000 (W8: copy) | POSES-MOVED | `0x3db47fae414b655c` (contact wakes 1240) | `0x74c14f3a08e24421` | `0xe64f927f27605db8 → 0x0924f2418b612a51` | 274 → 214 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on --steps 1000` |
| d0/w6-fixtures | Son-J1000 (W8: copy) | POSES-MOVED | `0xcc2a5400c66eecce` (contact wakes 1240) | `0x13d95ddbd8f6907e` | `0xcfe7596eeecf89a7 → 0x156318f8c69bc63e` | 265 → 205 | rk1240/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --sleeping --contact-reuse off --steps 1000` |
| d0/w6-fixtures | RSOffp800 (W8: copy) | POSES-MOVED | `0xb7f1e9e8f91f75ab` (contact wakes 3720) | `0xa42b663994f7b36d` | `0x0763860bfa825944 → 0xc54cfc6a31fd65ed` | 250 → 70 | rk1240/br0/cw0 | `--scene rest --sleeping --broadphase tree --contact-reuse on --steps 800` |
| d0/w6-fixtures | RS800 (W8: copy) | POSES-MOVED | `0x2a2b7926a48aab00` (contact wakes 3720) | `0xd70e51f6ae4dd2b8` | `0xaa8d8e0f6118df32 → 0x3c6a731c7784a2c8` | 248 → 68 | rk1240/br0/cw0 | `--scene rest --sleeping --contact-reuse off --steps 800` |
| v2/w8 | JSonT500 (W8: copy) | VELOCITY-ONLY | `0x130c76cb6b463ab8` (contact wakes 0) | `0xc637a0517adabab7` | `0x832c403cd7b63347` | 147 → 147 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --broadphase tree --sleeping on --steps 500` |
| v2/w8 | JSonToff500 (W8: copy) | VELOCITY-ONLY | `0xc671831894394df6` (contact wakes 0) | `0x4aec2a5f78f488d0` | `0x6d926dea060f48e0` | 142 → 142 | rk0/br0/cw0 | `--scene jolt --gap 0.5 --cfg a --broadphase tree --sleeping on --contact-reuse off --steps 500` |

