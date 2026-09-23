# L9 contact reuse: the C0 instruments (2026-09-23)

Lane `u/phys-l9`, base `integ/unified` at `6dd1f916`. Design
`docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`, commit C0 ("C0 instruments").
Nothing in this directory is a timing result: the machine was shared when it was recorded. The pose
bytes and the class counts are untimed and are results.

## Pose fixtures (`fixtures/`)

Recorded on the base with the parity runner (`benches/jolt_parity_pyramid.rs`, `--profile parity`,
msvc, no `RUSTFLAGS`; exe sha256 `ebcb16d858748eb21284001ace234e18a326f0eddd0c924e5e91198edff039e6`),
600 steps, `--pose-out`. They are the gate G-L9a-2 of commits C1 and C2 and the "moved pose is a
defect" gate of every commit C0-C3: each later commit's runner must reproduce them with
`--expect-pose fixtures/<row>_W<w>.pose` (exit 4 otherwise).

| row | runner flags | W | pose hash (FNV-1a 64) |
|---|---|---|---|
| J-A | `--scene jolt --gap 0.5 --cfg a` | 1, 8 | `0x49a5ed10875958d7` |
| J-D | `--scene jolt --gap 0.5 --cfg default` | 1, 8 | `0x49a5ed10875958d7` |
| R | `--scene rest --solver colored` (W=8 adds `--parallel-solve`, a no-op since L4) | 1, 8 | `0xa945b866513aab84` |
| R-S | `--scene rest --sleeping` | 1, 8 | `0x2a2b7926a48aab00` |
| J-Son | `--scene jolt --gap 0.5 --cfg a --sleeping` | 1, 8 | `0xcc2a5400c66eecce` |

Every row is `--workers W --steps 600`. W=1 and W=8 give one hash per row. R-S and J-Son equal P0's
hashes for those rows (`docs/measurements/2026-09-19-physics-p0/p0/window_report.md`, "Determinism"):
both piles are frozen before step 600, so the step count past the freeze does not move them.
`SHA256SUMS` holds the files' sha256.

The J pose of P0, `0x32d5e235342b4143` (500 steps), was re-read on the same exe for `--cfg default`
and `--cfg as`, `--broadphase allpairs` and `tree`, W = 1 and 8: eight of eight match.

## The class bench (`benches/narrowphase_classes.rs`), counts

The `--bench` run of the parity build on the base. Snapshots are what the narrowphase read on the
snapshot step (a probe system after the narrowphase); classes per the bench's module docs.

| scene | step | pairs | separated (slow / fast) | touching (slow / fast) | first separating axis A-face / B-face / edge | axes an early exit evaluates per separated pair |
|---|---|---|---|---|---|---|
| jolt | 300 | 9,559 | 5,044 (5,044 / 0) | 4,515 (4,515 / 0) | 4,982 / 61 / 1 | 1.79 of 15 |
| rest | 300 | 9,570 | 2,889 (2,889 / 0) | 6,681 (6,681 / 0) | 2,625 / 239 / 25 | 2.33 of 15 |
| rain | 40 | 5,934 | 4,560 (0 / 4,560) | 1,374 (0 / 1,374) | 3,580 / 744 / 236 | 2.91 of 15 |

- J's counts sit within 0.2 % of the design's 5,037 / 4,524 (P0b); the bench asserts a 5 % band.
- The touching count equals the step's solver manifold count on every scene (asserted).
- On J the canonical-order early exit alone (L9a (i)) takes a separated pair from 15 axis
  evaluations to 1.79: the cached separating axis (L9a (ii)) can remove at most the remaining 0.79,
  which is the split the review's OQ3 asked C0 to report.
- The ns per pair and the refutation reading ("measured costs predict Δt_np(1) < 1.0 ms -> stop")
  are taken in the next quiet window, before C4, under the P0 protocol.

## C1 receipts (L9a (i) and the frame column; bit-identical)

- **G-L9a-2:** the C1 parity runner reproduces every fixture above with `--expect-pose` (ten of ten `match`, exit 0),
  and the J pose `0x32d5e235342b4143` on cfg default/as × AllPairs/Tree × W 1/8 (eight of eight). Mutation M-a3 (the
  frame column refilled only on Rows steps, at both fill sites) is red on all ten rows (exit 4; J-A/J-D/J-Son
  `0x7ca5b1f25e9fd5f0`, R `0x819bdbc068b84057`, R-S `0xa779b5f9e52fd373`).
- **G-L9a-1** (`narrowphase::box_box::tests::l9a_classify_equals_the_pre_l9_kernel`, 2048 cases): green, with 464
  contacts at depth exactly zero among its cases; M-a1 (the early exit on `depth <= 0`) and M-a2 (`RowFrame::of`
  storing rows) are red.
- Unchanged on C1: A7-R1 D_max 0.0007180063 m (release), the bodytype golden, the pyramid determinism test, the
  release frame allocation census, the physics `Vec` census (pin 34) and UG-02.
