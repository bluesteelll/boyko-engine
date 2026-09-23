# L10 sleeping: the lane-base fixtures (2026-09-23)

Lane `u/phys-l10`, base `integ/unified` at `4db26681` (Phase A, B1/B2/B4, the tree broadphase's C3b
leaf-list query, L9 C0–C3 with contact reuse dormant). The design's brief is
`docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md` + `06-DESIGN-REV2.2.md` +
`08-DESIGN-REV2.3.md` + the block "L10 rev 2.2 … rev 2.3 CLOSED by ruling (2026-09-21)" of
`docs/physics/perf-campaign/levers/00-RULINGS.md`. This directory holds the fixtures the design
records before C0 (rev 2.2 §6, "Before C0"). Nothing here is a timing result: the machine was shared
when it was recorded. Pose bytes are untimed, so they are results.

## Pose fixtures (`fixtures/`)

Recorded on the base with the parity runner (`benches/jolt_parity_pyramid.rs`):

```text
cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid
```

The build used the msvc host (`stable-x86_64-pc-windows-msvc`) and no `RUSTFLAGS`. The exe sha256 is
`760d27f55af1138ba6bba3871f4dafa7089cb3a2712f6d6b24cbbc7e45229edd`. It was run by path, with
`--steps 600 --pose-out`. Each later commit of the lane must reproduce every file with
`--expect-pose fixtures/<row>_W<w>.pose`; the runner exits 4 on a mismatch.

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
