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

## C4 (contact reuse on by default, 2026-09-24): the reuse-on fixtures (`fixtures-c4/`) and H8

Lane `u/phys-l9-c4`, base `integ/unified` at `6c40b3e6` (L9 C0-C3, the tree C3b query and the thin-box fix),
commit `989ca0f0` (`PhysicsConfig::default().contact_reuse = true`, τ 1 mm). Parity runner exe sha256
`c6908070a72fdbfa162ee58b70aa825472e37b355060db98cb13b584af45ba45` (msvc, `--profile parity`, no `RUSTFLAGS`); the
base exe (`6c40b3e6`) sha256 `3048b7564a29c2e7e8b54ecf06288fe5cae5d0a8c3db7bb8e7e0632e6241c2c5`. Untimed, on a shared
machine: pose bytes and counters only.

From C4 on, a runner row without `--contact-reuse` runs contact reuse, and `--contact-reuse off` is the exact
narrowphase, the pre-C4 trajectory. So `fixtures/` above stays the reuse-off reference and is never re-recorded, and
`fixtures-c4/` holds the same ten rows recorded as C4 ships them: the flags of the table above with no reuse flag,
`--workers W --steps 600 --pose-out`. `SHA256SUMS` holds the files' sha256.

| row | W | C4 pose hash (reuse on) | `fixtures/` (reuse off) |
|---|---|---|---|
| J-A | 1, 8 | `0xcb74d44c7ddd1c69` | `0x49a5ed10875958d7` |
| J-D | 1, 8 | `0xcb74d44c7ddd1c69` | `0x49a5ed10875958d7` |
| R | 1, 8 | `0x2a5c44cb0c823cc9` | `0xa945b866513aab84` |
| R-S | 1, 8 | `0xb7f1e9e8f91f75ab` | `0x2a2b7926a48aab00` |
| J-Son | 1, 8 | `0x3db47fae414b655c` | `0xcc2a5400c66eecce` |

- **The flip moved only the default.** Every `fixtures-c4/` file is byte-identical to the base exe's run of the same
  row with `--contact-reuse on`, and the C4 exe with `--contact-reuse on` reproduces them too (ten of ten each).
- **Reuse off did not move.** The C4 exe with `--contact-reuse off` reproduces `fixtures/` (ten of ten `match`).
- **The window-6 families**, the same exe, W 1 and 8, one hash per row across W:

| row | reuse on (the C4 default, no flag) | `--contact-reuse off` (unchanged) |
|---|---|---|
| J500 (`--cfg default`, `as`, `a` × `--broadphase allpairs`, `tree`) | `0x30c5438bc6ad9ffa`, 12 of 12 | `0x32d5e235342b4143`, 12 of 12 |
| R1100 (`--scene rest --solver colored`, 1100 steps) | `0xc8bbe34cf6a8afc6` | `0x87e561d20589d4a5` |
| RS800 (`--scene rest --sleeping`, 800 steps) | `0xb7f1e9e8f91f75ab` | `0x2a2b7926a48aab00` |
| Son-J1000 (`--scene jolt --gap 0.5 --cfg a --sleeping`, 1000 steps) | `0x3db47fae414b655c` | `0xcc2a5400c66eecce` |
| S16-300 (`--scene s16 --cfg a`, 300 steps) | `0x4dccd02c5709844d` | `0x8877dbb1192e9b92` |
| rest-500 (`--scene rest --cfg default`, 500 steps) | `0x6cbe24bf8fafda26` | `0xee2a67a98434919a` |

  Every reuse-on row equals the base exe's `--contact-reuse on` run. J500 and R1100 equal window 6's JAon500 and
  Ron1100 files (`docs/measurements/2026-09-24-physics-window6/gate/fixtures/`) byte for byte, although those were
  recorded on `4db26681`, before the thin-box fix, so the fix moved neither row at τ = 1 mm. The
  explicit `--contact-reuse on` rows Offp-J1000 and RSOffp800 are byte-identical on the base and C4 exes. Every row
  kind (J-A, J-D, J-A armed, R, R-S, J-Son, S16, the self-check) exits 0 with no void step on the C4 exe; the
  self-check (no flags, 3 steps of s16) now runs reuse on and reads `0x9f678e72645e0633` (15 pairs reused), where it
  read `0x2d9782c2598c835a` before. A cross-window bridge from C4 on runs its rows with `--contact-reuse off`.
- **H8** (W=1, the runner's `--csv`, mean over the window; the same exe for both arms):

| row | window | manifolds, reuse on | manifolds, reuse off | pairs, reuse on | pairs, reuse off |
|---|---|---|---|---|---|
| J-A | [100, 500) | 4,467.665 | 4,519.2575 | 9,545.12 | 9,559.215 |
| R | [600, 1100) | 6,913.51 | 6,662.254 | 9,570 | 9,570 |

  The per-manifold denominators move with the manifold count: from C4 on, a per-manifold figure of a default row
  divides by the reuse-on column.
