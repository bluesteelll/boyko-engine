# IB P0r: the lane record (the parity runner and a game-shaped probe as normal-dependency binaries, and ug15's V11 instrument)

IB P0r is the first IB commit set. It is value-neutral and carries no IB content: no presets, no
`ib-pin-*` features, no budget. It merges ALONE, ahead of the rest of IB (LEVERS-R2-PLAN P14; IB
Rev 4 item 13 step 2b, patched by Rev 5 and Rev 6, closed by ruling 15 W13/W14/O17). The branch is
`u/phys-ib-p0r`, cut from `d64c0d5c` (`integ/unified`: 68ec9312 + the release + SL + FM C1 +
sr-policy).

- The cut is `D:/tmp/phys-orch/ib-p0r/cut.md` (with two developer addenda), and its critique is
  `ib-p0r/critique.md`.
- The implementation report is `ib-p0r/impl.md`. It keeps the two STOP records (RF-5a, and C7's first
  run) verbatim.
- Orchestrator rulings: 55 (the cut's questions and the critique), 57 (RF-5a's red (c2)), 61 (C7's
  UNEXPECTED controls), all in `phys-next/RULINGS-2026-10-15-AM.md`.

Code is cited by symbol, never by line. This file is not in `tests/internal_docs_anchors.rs`'s
`GATED_DOCS`, so its anchors are not machine-checked.

---

## 1. What landed

| commit | subject | what |
|---|---|---|
| C1 `b0a96a83` | `refactor(physics): IB P0r - the parity runner and a game-shaped probe as normal-dependency binaries` | the runner and its two child modules `git mv`'d (3 renames at 100 %, blobs equal); the packages `boyko-physics-parity` and `boyko-physics-probe`; `boyko-physics` loses `[[bench]] jolt_parity_pyramid`, `bench-alloc` and `mimalloc` (`src/` byte-identical); root `members`/`default-members` and the root recipe line; `Cargo.lock` (offline, no third-party version moved); the three root censuses and `ug02-crates.tsv`; `dyn_scenes_spec`/`dyn_scenes_hook_order`; `rapier_gates.py`'s `SPEC_BLOB`; the forced `OPEN-QUESTIONS.md` re-point (ruling 55 Q1) |
| C2 `dbf3e392` | `test(physics): IB P0r - the runner's self-check is a #[test] of its binary; its docs name its new home` | `mod tests { #[test] fn self_check }`; the runner's module docs in place, line count unchanged |
| C3 `ff2d0b14` | `docs(physics): IB P0r - live references to the runner and to boyko-physics/bench-alloc follow the move` | the test doc lines, the `boyko-physics` consumer note, `pyramid_scene.patch`'s runner-naming prose line (by content, ruling 55 Q5), `dyn_spec_ref.py`, README (16 benches), the dated WORKFLOW.md census line, the dated OPEN-QUESTIONS note |
| C4 `7e251da4` | `feat(physics): IB P0r - ug15 retention, closure tables, the plain-build bridge and the instrument key` | `objbuild` (`IB_SUBJECTS`, request features/env/locked, the built artifacts, every `Running` line, stderr); `closure`, `retain`, `plain`, `key`, `unit`; `tests/ug15_unit_pure.rs` |
| C5 `840ea347` | `feat(physics): IB P0r - ug15 unit and unit-compare (V11 (a)-(f), certified)` | `ug15 unit`, `ug15 unit-compare`, the certification rule, the control-receipt format |
| sync `813ad2b5` | `merge: integ/unified into u/phys-ib-p0r (trunk sync)` | rd2-rd6 (`b888ab76`); clean, no conflict |
| `ad62ed24` | `fix(physics): IB P0r - IB builds pass the profile's effective codegen-units, so V11 (c2) reads the shipped release (ruling 57)` | `Request::codegen_units` (IB-only, `None` from `Request::new`); `retain::profile_codegen_units`; `objbuild::check_final_codegen_units` |
| C6 `eece2797` | `feat(physics): IB P0r - ug15 unit-controls: RC-11 (a)-(f), RC-13, RC-14 and both green controls` | `controls.rs`, `ug15 unit-controls`, `ug15 unit-arm`; critique W3 carried |
| `e9978bb0` | `fix(physics): IB P0r - scratch paths keep --scratch's separator form, and a mixed target dir or root is RED (ruling 61)` | `controls::scratch_paths`; `unit::check_single_form` in `UnitRun::start` and `unit_compare` |
| C7 `d866c0ab` | `test(physics): IB P0r - the ug15 control receipt for instrument 5995ccee` | `crates/boyko_symcensus/ug15/receipts/ib/controls-5995ccee.txt` |
| C8 | `docs(physics): IB P0r - the lane record` | this file |

Every commit subject matches IB's bracket pattern `^[a-z]+\(physics\): IB\b` (Rev 4 item 10).

## 2. What moved, and the new build command (statement (a))

| from | to |
|---|---|
| `crates/boyko_physics/benches/jolt_parity_pyramid.rs` | `crates/boyko_physics_parity/src/bin/jolt_parity_pyramid.rs` |
| `crates/boyko_physics/benches/jolt_parity_pyramid/dyn_scenes.rs` | `crates/boyko_physics_parity/src/bin/jolt_parity_pyramid/dyn_scenes.rs` |
| `crates/boyko_physics/benches/jolt_parity_pyramid/dyn_spec.rs` | `crates/boyko_physics_parity/src/bin/jolt_parity_pyramid/dyn_spec.rs` (byte-identical forever: the Rapier pin) |

- `boyko-physics-parity` holds the runner as a normal-dependency `[[bin]]` (`bench = false`,
  `doc = false`, `autobins = false`). It forwards `bench-alloc` (mimalloc moved with its one
  consumer) and `narrowphase-counts`.
- `boyko-physics-probe` holds `physics_probe`, the game-shaped probe: a `[[bin]]` that links
  `boyko-physics` and, on Windows, `boyko-app`, the way a game does. Its `scope-wide` feature takes
  the optional `ENGINE_PACKAGES` crates for V11 (e).
- The runner is now a libtest binary, and its self-check is `tests::self_check`.
  `cargo test --workspace --all-targets` lists it as `running 1 test`.

**Statement (a), for the lane template's pin header.** It replaces `cargo bench --no-run --profile
parity -p boyko-physics --bench jolt_parity_pyramid`.

> On a tree that contains IB P0r (`git merge-base --is-ancestor <P0r merge sha> HEAD`), build the runner
> with `cargo build --locked --profile parity -p boyko-physics-parity --bin jolt_parity_pyramid` (with
> the lane's build prefix), then run `$CARGO_TARGET_DIR/parity/jolt_parity_pyramid.exe` by path.
> On msvc a bin's file name carries no hash, so the next build of the same bin with other features
> (`narrowphase-counts`, `bench-alloc`) overwrites that path. **Copy the exe out with its sha256 right
> after the build, and run the copy.** On a tree without P0r, keep `cargo bench --no-run --profile
> parity -p boyko-physics --bench jolt_parity_pyramid` and run the printed exe by path. IB's own
> images come from `ug15 unit` / `ug15 ib` under `--profile release` only (D14 (c)).

The kit README's runner rows take the same command, `cargo build --profile parity -p
boyko-physics-parity --bin jolt_parity_pyramid --locked`, budget 60 (PC-P0R-2).

## 3. Statement (b): an open branch's trunk sync over P0r (P14)

This is the recipe for `u/phys-s5-c3b` (S5 C4) after its window, and for any branch that touches the
runner, its child modules or `crates/boyko_physics/Cargo.toml`.

1. `git merge --no-ff integ/unified`. Resolve conflicts by editing and keeping both sides. The expected
   touch points are in `crates/boyko_physics/Cargo.toml` (the deleted bench-alloc block and the
   deleted `[[bench]] jolt_parity_pyramid`): keep the branch's own lines and P0r's deletions. A runner
   hunk follows the rename.
2. The rename-follow proof, with O = the branch tip before the sync, T = `integ/unified`, M = the
   merge, and B = `git merge-base O T`:
   - (i) `test ! -e crates/boyko_physics/benches/jolt_parity_pyramid.rs && test ! -e
     crates/boyko_physics/benches/jolt_parity_pyramid` (no resurrection by a modify/delete conflict);
   - (ii) for each moved file, the changed-lines signature of `git diff B O -- <old path>` equals that
     of `git diff T M -- <new path>`, computed with `tools/physics_parity/driver.py`'s
     `diff_signature` imported in place (critique W1; shown green on a correct merge and red on a lost
     hunk, `ib-p0r/red/w1/proof.txt`). The one-liner is in `ib-p0r/impl.md`, statement (b). A recorded
     hand resolution replaces (ii) for that file;
   - (iii) any file the branch added under the old child directory now sits under
     `crates/boyko_physics_parity/src/bin/jolt_parity_pyramid/`.
3. A `#[cfg(feature = "X")]` the branch added to the runner needs `X = ["boyko-physics/X"]` in
   `crates/boyko_physics_parity/Cargo.toml`. A missing forward is a loud `unexpected_cfgs` under
   `-D warnings`, never silent.
4. Union `Cargo.lock`, re-derive it with `cargo update --workspace --offline`, and require that no
   third-party version changed.
5. Rebuild with statement (a). Run the branch's own runner gates (for S5, `s5_runner_gate.sh`), plus
   P4 `runner_pins.sh` and P11 `ours_gates.sh`, all in place.
6. From then on, the branch's lane header uses statement (a).
7. A timed A/B never straddles P0r (PC-P0R-1).

**Branches that edit no runner file** (PX0, every render lane): union the root `Cargo.toml` members
lines and `Cargo.lock`, then `cargo check --workspace --all-targets --locked`, which compiles the moved
runner (ruling 55 Q11).

## 4. V11 and the control receipts

The instrument key K is sha256 over `crates/boyko_symcensus/Cargo.toml` and `src/**` (`\r\n`
normalised, the bytes compiled into the executable, checked equal to the tree's), `rustc -vV` of the
compiler cargo invokes, and `cargo -V` (W13; critique W4 and open question 3). P0r is K's
calibration point.

| K8 | at | what it certified |
|---|---|---|
| `ee37c282` | C5 | nothing: RF-5a was red at (c2) |
| `a32aa3f5` | `ad62ed24` | nothing: RF-5a re-run green, uncertified (no controls yet) |
| `efc6e585` | C6 `eece2797` | nothing: C7's first run read 3 controls UNEXPECTED (the receipt was not committed; its copy is `ib-p0r/k/c7_controls/controls-efc6e585.txt`) |
| **`5995ccee`** | `e9978bb0` | **12 of 12 controls as expected**: `crates/boyko_symcensus/ug15/receipts/ib/controls-5995ccee.txt` (C7, `partial no`, tree `HEAD e9978bb0…, 0 uncommitted path(s)`) |

### 4.1 RF-5a: V11 (c2) was red at its calibration (ruling 57)

At C5, `ug15 unit` read (c2) red: the ug15 image's `.text` was `cee3314c…` against the plain release's
`cadc580d…`. Measured one flag at a time: `--emit=obj` with codegen-units unset makes rustc codegen
ONE unit, and `-C codegen-units=1` alone reproduces ug15's bytes. So the ug15 image was a fat-LTO +
codegen-units = 1 build, not the shipped release.

The fix (`ad62ed24`) is IB-only. Every IB build passes `-C codegen-units=<n>`, where n is the
profile's own `codegen-units`, else the nearest one up `inherits`, else rustc's non-incremental
default of 16. After the build, the final line must carry exactly that token (twice when the profile
sets it), and no `-C incremental`. Every UG-15 leg and `SUBJECTS` are byte-unchanged.

The re-run on `ad62ed24` (`ib-p0r/k/rf5r_receipt.txt`) read (a), (b), (c), (f) GREEN, with **(c2) by
bytes**: `.text` `cadc580d2db0c2e4c371e3349b027edc5442615b4aa2085957b30a8a93d409d9` in both retained
images. RC-14 (a) was vacuous before the fix, because its baseline (c2) was already red; it reads its
own argument now.

### 4.2 C7's first run, and ruling 61

C7's first run (`ib-p0r/k/c7_controls/`, K `efc6e585`) read 9 of 12 controls as expected. RC-11 (e),
RC-11 (f) and G-d read UNEXPECTED, all from one harness defect. The scratch target dir was formed with
`Path::join` on a forward-slash `--scratch`, which gives `D:/…/rc\<id>\target`. (d2) substitutes the
target dir in its two single-separator forms only, so the mixed string never became `<T>`. Rev 6 R21
forbids a wider normaliser, so the lane stopped.

The fix (`e9978bb0`) leaves the normaliser unchanged:

- `controls::scratch_paths` forms every scratch path in `--scratch`'s own separator form;
- `unit::check_single_form` states (d2)'s premise and checks it. `UnitRun::start` is RED [Usage] on a
  mixed `Ctx.target` or `Ctx.root` before anything else, and `unit_compare` refuses, by name, a
  receipt that carries one.

### 4.3 C7: the control receipt for `5995ccee`

`ug15 unit-controls --retain D:/tmp/phys-orch/ib-p0r/retained/controls-r61 --scratch
D:/tmp/phys-orch/ib-p0r/rc/r61`, on `e9978bb0`, under the heavy lock, through the kit
(`ib-p0r/k/c7r61_controls/log`, VERDICT GREEN exit_code 0). The first run's directories hold its
evidence, so this run used new ones.

| control | expected | observed |
|---|---|---|
| G-b | (b) green | 36 units identical; floor missing [] |
| RC-11 (c) | (b) red at boyko_diag: same path, different sha256 | sha256 moved; 0 path changes; 0 stray sha256 changes; 13 dependent sha256 changes (recorded) |
| RC-13 | the first build's `check_intact` RED [pair guard]; its retained copies intact | as expected |
| RC-14 (a) | (c1) green, (c2) red | as expected |
| RC-14 (b) | (c1) red; the non-fresh set equals boyko_input, its dependents and the final unit | 6 = 6; the final unit rebuilt |
| RC-11 (a) | (b) red: path differs at boyko_log and every target unit depending on it | 13 units |
| RC-11 (b) | (b) red at boyko_physics (path) only | as expected |
| RC-11 (d) | (e) red at windows_sys, boyko_input, boyko_scene, boyko_physics (paths) | all four by path; no second mechanism |
| RC-11 (e) | (d1) red at every target closure unit, host units unchanged; (d2) red at every target workspace line, the proc-macro's unchanged; (d3) red; (d4) green | (d1) 20 target red / 5 host unchanged; (d2) 12 target red / boyko_macros unchanged; (d3) red; (d4) green |
| RC-11 (f) | (d1) red at the four only; (d2) red at the three workspace lines; (d3) red at the boyko_physics/boyko_app `--extern` names; (d4) green; `Cargo.lock` unchanged | as expected |
| RC-14 (c) | (f) red: no boyko_app diagnostic | as expected |
| G-d | (d1)-(d4) all green | as expected |

### 4.4 G9: `ug15 unit`, certified

`ug15 unit --retain D:/tmp/phys-orch/ib-p0r/retained/p0r` on C7 `d866c0ab`, under the heavy lock,
through the kit (`ib-p0r/k/g9/log`, VERDICT GREEN exit_code 0; receipt `ib-p0r/k/g9_receipt.txt`,
sha256 `1e694bb7…`). It prints `ug15 unit: GREEN, certified`; the receipt's key is `5995ccee…` and its
tree line `HEAD d866c0ab…, 0 uncommitted path(s)`.

| leg | verdict | measured |
|---|---|---|
| (a) | GREEN | both manifests have the game shape; the runner's one registry entry is mimalloc ^0.1 behind bench-alloc; the probe's optional set is the 18 `ENGINE_PACKAGES` crates `scope-wide` activates; no diagnostic feature in either final line |
| (b) | GREEN | 36 library units identical in path and sha256; both builds cover the 25 closure crates |
| (c) | GREEN | (c1) 42 non-final artifacts fresh, the library set equals ug15's 36 and re-hashes equal, the final unit rebuilt; **(c2) `.text` `cadc580d…` in both retained images** |
| (e) | RED, the scope note (§4.5) | path differs at boyko_shaderdsl, boyko_sdf_math, boyko_physics |
| (f) | GREEN | `warning: extern crate `boyko_app` is unused in crate `jolt_parity_pyramid`` |
| cert | **certified** | `crates/boyko_symcensus/ug15/receipts/ib/controls-5995ccee.txt` |

The cut's G9 row lists (e) among the green legs. (e) is red here as ruling 55 Q13 and ruling 57 (3)
pre-ruled, and `unit`'s verdict excludes it (D14: "(e) aborts nothing").

### 4.5 V11 (e): the scope note narrows (ruling 55 Q13, ruling 57 (3))

(e) is red at boyko_shaderdsl, boyko_sdf_math and boyko_physics. D14 says "(e) aborts nothing", so it
is never `unit`'s verdict. `boyko_sdf_math` takes `boyko_shaderdsl` with `default-features = false`,
while the probe's `scope-wide` entry takes it at default features. Cargo then passes `--cfg
feature="default"` for an empty `default = []`, which makes `boyko_shaderdsl` a different unit, and
`boyko_sdf_math` and `boyko_physics` above it other files too. The code is identical (the feature
enables nothing), but V11's identity is the rlib file. **The scope note reads: "a game linking only
boyko-app and boyko-physics"**; the first divergence is at boyko_shaderdsl's empty default feature.

## 5. A finding outside this lane: the UG-15 legs census codegen-units = 1 objects (ruling 57 (2))

Every UG-15 leg builds through `objbuild`'s `--emit=obj` with codegen-units unset, so each one
censuses a codegen-units = 1 object, not the shipped codegen. `ug15 textcheck` compared two such
builds, so it could not see this. Their past receipts stay valid as identity claims between two
identically built trees. **Owed decision (orchestrator):** re-pin the legs on the shipped codegen, or
state their scope. It is also recorded in `objbuild.rs`'s module docs ("`--emit=obj` and codegen
units") and in `ug15/RECIPES.md` ("IB V11 commands").

⚠ *2026-10-19, decided: ruling 71 (4), narrowed by ruling 74 (A), and closed by ruling 77 (1) at
PHYS-TOOL-1. The legs keep their codegen-units = 1 recipe, and `ug15/RECIPES.md` states that scope
("The UG-15 legs' scope") with the sampled check's findings F1 and F2. The re-pin on the shipped
codegen is deferred to the unified plan's F2 end-state gates as PC-PT1-10
(`docs/physics/perf-campaign/levers/tooling/01-PHYS-TOOL-1-RECORD.md` section 4). F2 bears on this
lane's premise, ruling 57 (1)'s, that the explicit effective value reproduces the shipped bytes. It
held on the parity runner (RF-5a, and V11 (c2) on every run; (c2) builds no plain probe, so the
probe's `.text` is unchecked, PC-PT1-13), and it does not hold on `boyko_demo` or `clear`. V11 (c2)
stays the runner's proof: the `ib` lane re-runs it at its base and STOPs if it is red (ruling 77 (2), PC-PT1-11).*

## 6. Red-first tallies (outputs in each commit message; files under `D:/tmp/phys-orch/ib-p0r/red/`)

- **C1-C3.** RF-1a..h held their exact predictions (`red/rf1*.txt`). RF-2a: `running 0 tests` before
  C2, `running 1 test` after; RF-2b, a mutation fails the self-check. RF-3a: the patch signature is
  equal, and a mutated copy differs; RF-3b: the patch applies to clean Jolt v5.6.0 and fails on a broken
  `PyramidScene.h`. W2: a windowed-only top-level const reds the Linux clippy and leaves the native
  one green.
- **C4/C5.** M1-M7 and RF-4 each made their named test red (`red/c4/`). RF-5b: `unit-compare` of a
  receipt against itself is green, and with two `/MAP:` tokens it is red. W4: a stale ug15 refuses
  before any build.
- **Ruling 57.** RF-5a's red (c2) is the red the fix answers. M57a (ignore `inherits`), M57b (drop the
  token count) and M57c (drop the incremental refusal) each red their test.
- **C6.** M6a (the package key) and M6b (RC-11 (d)'s path count) each red their test.
- **Ruling 61.** C7's first run is the red the fix answers. Live: `ug15 unit-compare` on the as-built
  G-d receipts read `(d2) 13 red` silently before, and `REFUSED … mixes` after
  (`red/r61/before_`/`after_unit_compare_gd.txt`). A mixed `--scratch` is RED before any build
  (`red/r61/live_mixed_scratch.txt`). M61a (scratch paths through `Path::join`, which reproduces C7's
  mixed form), M61b (the check never REDs), M61c/M61d (`UnitRun::start` without its target / root
  check) and M61e (`unit_compare` without its refusal) each red their test.
- **The controls are the instrument's own red-first:** every red control red as specified, and both
  green controls green (§4.3).
- Every mutation was reverted by content, with md5 equal (`ib-p0r/scratch_ledger.tsv`).

## 7. Gates

**Developer, per commit** (kit logs in `ib-p0r/k/`): `cargo check` and `cargo clippy --workspace
--all-targets --locked -- -D warnings`; `cargo test -p boyko-symcensus --no-fail-fast`, every target by
name (`ug15_unit_pure` 16 at `e9978bb0`); boyko-log `print_census` 4/4; UG-10 `internal_docs_anchors`
24/24; `ignore_reasons_census` 12/12 (479 sites, 1963 `.rs` files walked, 0 waivers); at C1, G17 (the
Linux clippy leg) and `cargo test -p boyko-engine --tests` (23 targets, `runtime_data_ledger_gate` 49);
G3, G4, G5, G12a, G14, G15 as the cut lists.

**The tester's** (the cut's lane-level table, at the final HEAD, one test process at a time): G4, G5,
G6 P0R-POSE, P1-P12 (no pin moves: P0r moves no value), G9' (`ug15 unit`), G10' (`unit-controls --only
rc11c,rc13,rc14a,rc14b,g-b` against the committed receipt, ruling 55 Q10), G12, and G13 (the workspace
run; the runner's target `src\bin\jolt_parity_pyramid.rs` shows `running 1 test` by name, and the
probe's `src\bin\physics_probe.rs` `running 0 tests`).

## 8. PC lines

- **PC-P0R-1 (every timing lane and window).** P0r changes the runner image's closure on purpose (D14).
  A timed A/B or a BENCHMARKS row compares images from the same side of P0r only, and `BENCHMARKS.md`'s
  next update labels its runner recipe.
- **PC-P0R-2 (orchestrator tooling).** Replace the runner command in the lane template's pin header and
  in the kit README's runner rows with statement (a). The kit README's P10 row still shows the pre-SL
  anchors. The window idle lists and the driver's void names gain `jolt_parity_pyramid` and
  `physics_probe`. SCREEN-L/AM harnesses that patch or export the runner at its old path re-point
  before building from a tree with P0r.
- **PC-P0R-3 (record).** `dyn_spec.rs`'s header prose still names the old path. Its bytes are pinned by
  the Rapier harness, so the prose waits for the next deliberate spec change, which re-pins all three
  derivations.
- **PC-P0R-4 (IB P2).** P2 adds `ib-pin-*` to both packages, the root pin census and `ug15 ib`. `ib`
  changes K, so P2 re-runs `unit-controls`, and its receipt commit carries an IB-pattern subject.
- **PC-P0R-5 (doc-writer).** `book/src/reference/benchmarks.md` (the bench count of 17, and the link to
  the runner's old path, a 404 once master has P0r) and `book/src/simulation/physics.md`.
- **PC-P0R-6 (retention).** `D:/tmp/phys-orch/ib-p0r/retained/**` and `rc/**` stay until the lane
  merges, for the reviewer; the orchestrator then deletes them. The committed receipts are the record.
- **PC-P0R-7 (CI blind spot, unchanged class).** The probe's windowed branch and both `boyko-app`
  entries are `cfg(windows)`, and CI's clippy runs on ubuntu, so the local msvc clippy is their only
  clippy.
- **PC-P0R-8 (ruling 57 (2), owed).** §5: the UG-15 legs' codegen-units = 1 scope. ⚠ *2026-10-19,
  closed by ruling 77 (1) at PHYS-TOOL-1 (§5's marker). Three follow-ons remain: the deferred re-pin,
  PC-PT1-10 (unified plan F2); PC-PT1-11 for the `ib` lane (IB P1-P4, the physics roadmap's P6):
  re-run V11 (c2), which byte-checks the parity runner only, at its base, and STOP if it is red; and
  PC-PT1-13 for the orchestrator: the probe's `.text` at the explicit value is unchecked.*
- **PC-P0R-9 (continuity, ruling 55 Q12).** The optional P0R-CONT rows (PARENT vs TIP image, W ∈ {1, 8,
  16}) join the owed closed-app bench window; they need no window of their own (`ib-p0r/cut.md` §5).
