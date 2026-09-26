# L10 sleeping: the lane-base fixtures (2026-09-23, re-recorded 2026-09-25 after L9 C4)

Lane `u/phys-l10`, base `integ/unified` at `4db26681` (Phase A, B1/B2/B4, the tree broadphase's C3b
leaf-list query, L9 C0–C3 with contact reuse dormant). The design's brief is
`docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md` + `06-DESIGN-REV2.2.md` +
`08-DESIGN-REV2.3.md` + the block "L10 rev 2.2 … rev 2.3 CLOSED by ruling (2026-09-21)" of
`docs/physics/perf-campaign/levers/00-RULINGS.md`. This directory holds the fixtures the design
records before C0 (rev 2.2 §6, "Before C0"). Nothing here is a timing result: the machine was shared
when it was recorded. Pose bytes are untimed, so they are results.

## Re-recorded after L9 C4 (2026-09-25)

The lane's pre-merge sync merged `integ/unified` at `6394bc5e` (merge commit `5248eaca`). That
carries L9 C4, which turns `PhysicsConfig::contact_reuse` on by default. Every row below was
recorded without a reuse flag, so every row now runs contact reuse and moves. Rev 2.2 expected
this move (§6: "P0's hashes no longer apply, because L9 C4 moved values"). The rows were
re-recorded by this directory's own rule, and the original bytes are kept as a
`--contact-reuse off` arm:

- `fixtures/` holds the rows as the table below spells them (no reuse flag, so reuse is on),
  recorded on the synced tree.
- `fixtures-reuse-off/` holds the lane-base recording of 2026-09-23 byte for byte. The files
  were moved, not re-recorded. The same rows with `--contact-reuse off` must reproduce them.

The build command and `--steps 600 --pose-out` are the same as below. The toolchain was msvc
(`stable-x86_64-pc-windows-msvc`), with no `RUSTFLAGS`, built at `5248eaca`. The exe sha256 is
`4ab79914218274e1ab01bb0cdd2fdab094441f56126d7339aede52e2edd3c336`. Every row gives the same hash
at W = 1 and W = 8. The reason on every row is "L9 C4: contact reuse on by default".

| row | W | old, now in `fixtures-reuse-off/` | new, in `fixtures/` |
|---|---|---|---|
| R-S | 1, 8 | `0x2a2b7926a48aab00` | `0xb7f1e9e8f91f75ab` |
| J-Son | 1, 8 | `0xcc2a5400c66eecce` | `0x3db47fae414b655c` |
| R | 1, 8 | `0xa945b866513aab84` | `0x2a5c44cb0c823cc9` |
| J-A-on | 1, 8 | `0xcc2a5400c66eecce` | `0x3db47fae414b655c` |
| R-on | 1, 8 | `0x2a2b7926a48aab00` | `0xb7f1e9e8f91f75ab` |
| J-D-on | 1, 8 | `0xcc2a5400c66eecce` | `0x3db47fae414b655c` |

Receipts on the synced tree (untimed):

- **The default is the only thing that moved.** Each new file is byte-equal to the same row run
  with an explicit `--contact-reuse on` (12 of 12, compared with `cmp`).
- **C4 moved only the default.** Each row run with `--contact-reuse off` matches its
  `fixtures-reuse-off/` file (12 of 12, exit 0, `match`).
- **Same bytes as L9's fixtures.** The twins still share bytes with the rows whose flags they
  share, so there are three distinct byte strings. Each is byte-equal to L9 C4's reuse-on fixture
  for that row, in `docs/measurements/2026-09-23-l9-contact-reuse/fixtures-c4/`.
  `fixtures-reuse-off/` stays byte-equal to L9's C0 fixtures.
- **Off === Sets holds in both arms.** Every sleeping-on row reproduces its file under every
  combination of `--sleep-skip` (unset, `off`, `sets`), `--broadphase` (unset, `tree`, `grid`) and
  W (1, 8). That holds flagless against `fixtures/`, and with `--contact-reuse off` against
  `fixtures-reuse-off/`. Unset runs `AllPairs` and `Sets`, as each run's summary records.
  That is 92 runs per arm, each exit 0 and `match`: the five rows times nine combinations times
  two W, plus R at W = 1 and 8. They ran on the exe built at `9b5bf588` (sha256
  `612dbcee58dec28d80bf6bafbc722c6814a55b7c62843e8188ad00316523d95c`); the commits after
  `5248eaca` change only comments. The same pass re-ran, in both arms, the 500-step J pose (P0's
  `0x32d5e235342b4143` with reuse off, C4's `0x30c5438bc6ad9ffa` flagless), L9's fixtures (C0's
  with reuse off, C4's flagless) and window 6's 1000- and 800-step sleeping rows. It also re-ran
  window 6's `--contact-reuse on` rows, which did not move. In all, 264 runs, 264 passing.
- **The comparison can fail.** Flagless R-S at W=1 checked against `fixtures-reuse-off/R-S_W1.pose`
  exits 4. So does R-S with `--contact-reuse off` checked against `fixtures/R-S_W1.pose`.
- `SHA256SUMS` in each directory checks OK. After a fresh checkout, strip the CR first, because
  autocrlf adds one to each file name: `tr -d '\r' < SHA256SUMS | sha256sum -c`.

The sections below record the lane-base recording of 2026-09-23 as it was made. Its files are the
ones now in `fixtures-reuse-off/`.

## Pose fixtures (`fixtures/` until 2026-09-25, now `fixtures-reuse-off/`)

Recorded on the base with the parity runner (`benches/jolt_parity_pyramid.rs`):

```text
cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid
```

The build used the msvc host (`stable-x86_64-pc-windows-msvc`) and no `RUSTFLAGS`. The exe sha256 is
`760d27f55af1138ba6bba3871f4dafa7089cb3a2712f6d6b24cbbc7e45229edd`. It was run by path, with
`--steps 600 --pose-out`. Each later commit of the lane must reproduce every file with
`--expect-pose fixtures/<row>_W<w>.pose`; the runner exits 4 on a mismatch. Since the re-record
above, that means flagless rows against `fixtures/`, and the same rows with `--contact-reuse off`
against `fixtures-reuse-off/`.

| row | runner flags | W | pose hash (FNV-1a 64) |
|---|---|---|---|
| R-S | `--scene rest --sleeping` | 1, 8 | `0x2a2b7926a48aab00` |
| J-Son | `--scene jolt --gap 0.5 --cfg a --sleeping` | 1, 8 | `0xcc2a5400c66eecce` |
| R | `--scene rest --solver colored` (W=8 adds `--parallel-solve`, a no-op since L4) | 1, 8 | `0xa945b866513aab84` |
| J-A-on | `--scene jolt --gap 0.5 --cfg a --sleeping` (J-A's sleeping-on twin) | 1, 8 | `0xcc2a5400c66eecce` |
| R-on | `--scene rest --solver colored --sleeping` (R's sleeping-on twin; W=8 adds `--parallel-solve`) | 1, 8 | `0x2a2b7926a48aab00` |
| J-D-on | `--scene jolt --gap 0.5 --cfg default --sleeping` (J-D's sleeping-on twin) | 1, 8 | `0xcc2a5400c66eecce` |

- Every row passes `--workers W --steps 600`. W=1 and W=8 give one hash per row.
- `SHA256SUMS` holds the sha256 of each file. Three distinct byte strings appear, and each is
  byte-equal to the corresponding L9 C0 fixture (`docs/measurements/2026-09-23-l9-contact-reuse/fixtures/`).
  R-S, J-Son and R are therefore also P0's hashes for those rows.
- Rev 2.2 expected new values here ("P0's hashes no longer apply, because L9 C4 moved values"). On
  this tree L9 C4 has not landed and `contact_reuse` defaults to off, so none of the earlier levers
  moved a value.
- The twins record the same bytes as the rows whose flags they share. `--cfg a` sets
  `sleeping = --sleeping`. `--cfg default --sleeping` forces sleeping on over the shipped default. The
  colored solve is bit-identical across cfg-A and the default (L9 C0 README), so every sleeping-on
  jolt row ends in J-Son's bytes, and every sleeping-on rest row ends in R-S's bytes.
- The twins exist because of the orchestrator's deviation (2). C1b (sleeping on by default, the
  design's only value mover) is not built in this lane. Every gate the design phrases against
  "C1b's hash" therefore runs its row with sleeping forced on, and compares against the `-on` twin
  here.

## Base receipts (untimed, same exe)

- The J pose of P0, `0x32d5e235342b4143` (500 steps), matched on eight of eight runs:
  `--cfg default` and `--cfg as` × `--broadphase allpairs` and `tree` × W = 1 and 8.
- L9's C0 fixtures matched ten of ten (exit 0): J-A, J-D, R, R-S and J-Son at W = 1 and 8, 600 steps,
  with `--expect-pose` against `docs/measurements/2026-09-23-l9-contact-reuse/fixtures/`.
- Sleeping on is invariant under the broadphase kind, eight of eight (exit 0). R-S and J-Son ran
  with `--broadphase tree` and `--broadphase grid` at W = 1 and 8, each checked with `--expect-pose`
  against this directory's R-S and J-Son files.
- Negative control: R-S at W=1 run with `--expect-pose fixtures/R_W1.pose` exits 4. So the
  comparison can fail.
- D1′'s L5 probe grep: `PROBE_MASK|eprintln!` over `crates/boyko_physics/src` hits only
  `solver/colored_tests.rs`, at `:2123`, `:2199`, `:2321`, `:2486`, `:3650` and `:4674`. These are
  test-module non-vacuity prints. Nothing is in `narrowphase/dispatch.rs`, and no `PROBE_MASK`
  remains.

## Not recorded here (deviation (1))

The design also asks for timed measurements before C0:

- the armed Off′ spans for J-Son and R-S at W ∈ {1, 8}, K=6;
- the pre-C0 refutation (armed Off′ J-Son at W=1, K=6: stop if it is below 0.584 ms).

Both are wall-clock readings. They move to the next quiet window under the P0 protocol, together
with every other timed gate of the lane.
