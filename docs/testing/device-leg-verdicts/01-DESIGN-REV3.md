# Architecture: `boyko_leg`, so a test that skips gets its own verdict instead of `ok` (rev 3)

## Changes from rev 2

For each review item this table gives the resolution and quotes the rev-2 text that was removed, so nothing is dropped silently. Sections not listed here carry over from rev 2 with renumbered commits.

| Review item | Resolution (where) | Rev-2 text removed, verbatim |
|---|---|---|
| **W1** Triaged reds turn UG-12 red for every later rung | Adopted deqp's baseline-Fail + UnexpectedImprovement model. A new machine-file row `known <test> "<panic substring>" -- <FINDING-ID>: …` turns a matching libtest failure into KNOWN-RED. KNOWN-RED is not counted in F and not counted as measured. If the test passes it is FAIL unexpected-pass; if it skips, FAIL unexpected-skip; if it fails with a different message, FAIL; a row with no test is FAIL stale-known. UG-12's pass criterion is stated in §8. Only two routes may add a known row: a migration sub-rung, for reds reproduced on its parent commit, or a finding-filing document step (§3 step 2, judge rows 7/15, §8). **Why not `deferred:`:** it removes the test from the run, so the passing siblings of a masked assertion (R-4 masks seven) and any change in how it fails become invisible. | "It does not repair the triaged reds R-1..R-6. They show as FAIL." (§11; now: they show as KNOWN-RED against a finding, or FAIL) |
| **W2** Orphan-worker makes F1 = no un-greenable | orphan-worker fires only when a driver named in the worker's reason ended **PASS** and the worker has no BEGIN. A skipped, failed or unselected driver exempts its workers. The row now also works in `--only` runs (§3 post-run rows). | "FAIL orphan-worker: a `worker:` site with no `BEGIN` in any ledger. Full runs only; `--only` prints "n/a (partial)"." |
| **W3(i)** `LegCommand::run()` vs rule (i) | The method is renamed to `LegCommand::status()`, mirroring `std::process::Command::status`. Rule (i) now bans zero-argument `.run()` per binary. Checked: every non-comment `.run()` in `boyko_app/tests` is `App::run` (§1, §6). | "`pub fn run(&mut self) -> ChildRun;                                  // inherited stdio`" |
| **W3(h)** Nine CPU gates name `EnginePlugins::window` | Rule (h) no longer keys on `EnginePlugins::window`. The App entry point is `.run()` (rule (i)). The `log_host_*`, `particle_host_reachable` and `profiling_host_*` gates never call it, so they need no Leg and no waiver (verified in `log_host_reachable.rs:49`, `profiling_host_arm_flag.rs:37`). | "(h1) Every `#[test]` in a file naming `VulkanContext::boot`, `VulkanContext::boot_singleton`, `EnginePlugins::window`, `VulkanProbe::open`, `boyko_leg::vk::boot` or `boyko_leg::tool(` contains `boyko_leg::Leg::begin()` or `boyko_leg::worker()`, or is listed in the file's `leg-cpu-only` directive. The scan **verifies** that listed tests reach no entry point through the same-file call closure." Also the directive "`//! leg-cpu-only: <fn>, … -- <why>`". |
| **W3(f)** Stale-declaration clause vs App members | A `leg-validation: off` declaration is stale only if its file contains neither a validation-control form **nor a leg member**. The declaration reason now carries a class from `{timing, escape-hatch, raw-instance}` (§6 (f)). | "A declaration with none of these forms is stale." |
| **W4** Rule (h) is file-scoped | Replaced by type-forcing. (h1) bans raw `VulkanContext::boot(` / `boot_singleton(` in test code outside `boyko_leg` (waiver: g7). Every boot then goes through `vk::boot(&Leg, …)`, and (d2) requires `boyko_leg::Leg::begin()` to appear only in `#[test]` bodies. A helper therefore cannot boot without being handed a Leg, and a Leg cannot outlive an early return. Every rule is evaluated per test binary: the root file plus its resolved `mod` files. The H1 re-introduction is G5's named mutation. | as W3(h) |
| **W5** Multi-test delta is not deterministic; negative controls unmigrated | D10 rewritten. The oracle **mode** is a pure function of argv/env. **Exclusive** mode (a runner member or child, or `--test-threads=1`) gates on the process-ledger window. **Parallel** mode gates only per context: `vk::boot` registers each context's `Arc<DebugMessengerState>` with the Leg. `done()` panics if a registered context is still alive, so the device-destroy window is always inside the verdict. `app::run` requires exclusive mode. Negative controls use `vk::expect_validation(&leg, &ctx, Expect::Any)`; migrations are named for `sync_validation` B and `compute::negative_chained_barrier_hazard` (§5). | "In a multi-test process (plain `cargo test`) it reads the delta since `Leg::begin()`." / "The run's red/green outcome is deterministic: any message anywhere makes it red. Only which tests are red depends on timing." / "The DONE record is annotated with `teardown=covered` when `messengers_destroyed == messengers_created`, else `teardown=open`." |
| **W6** §5/§7 contradiction; unvalidated boots undecided | §5 now decides every `enable_validation: false` / `InstanceConfig::default()` site (17 sites, grep 2026-09-18). The cost harnesses stay off with `timing:`; the data-path and correctness sites are switched on. The rule for a new validation red at a switched-on site is stated (§5 end). | "Multi-line forms, `InstanceConfig::default()` boots and the vkval sites are enumerated by rule (f)'s report mode at C10." |
| **W7(a)** Locks park C1 and D-E23 | B5 is split into seven sub-rungs. **Only B5-0** (size S) touches shared files: root `Cargo.toml`, `runner.rs`, `debug.rs`/`device.rs`, `ci.yml`. Every §4.3 edge B5 adds is first-cut-wins (`·`), never `→`. The prefix sweep's collisions (e.g. `miri_fixed_loop.rs` with D-E23) are handled by a shrinking `PREFIX_PENDING` waiver instead of a lock (§6 (a), §8). | "root `Cargo.toml`: `A8 → B3 → B5(i) → C1 · RP-2 · …`" and "No rung waits on B5." and the B5(i)–(iii) table |
| **W7(b)** vkval unregistered | Added a 04 §1 row and a place in 04 §2's A5 order: `fix/boot-validation-errors` @ `e3cebe2d` plus uncommitted work, with gates UG-01, UG-12 and UG-18. Its `device.rs` hunk joins A8's hand-resolved set. There is a fallback rung V0 if it misses A5. DOC-B5 now edits 04 (§8). | "Its natural slot is A5's batch … If it misses A5, it merges in the trunk worktree after A8, before B5(i) is cut." |
| O1 First child SKIP hides later ones; allow laundering | Row 12 uses the **set** of skip classes (own plus every child's); all must be admissible. `allow` rows are limited to device-absence classes. `cap`, `surface-lost` and `engine-degraded` need per-test `expect` rows. | "effective skip class c (the member's own SKIP, else the first child SKIP)" |
| O2 Instances double-counted | Incremented at exactly one site, inside `create_instance` after `vkCreateInstance` succeeds. Probe (i) and G17 assert instances == messengers == 1. | "It is incremented in `boot` and `boot_singleton` right after `vkCreateInstance` succeeds." |
| O3 Wrong error type | `HostBootStage::VulkanDevice(VulkanError)`. `Boot(e)` maps through `class_of(e)`; every other `VulkanError`, including `SingletonAlreadyBooted`, maps to `None` (FAIL). | "`VulkanDevice(BootError)`" |
| O4 `NoRecord` too coarse | Split into `Panicked` / `Crashed(ExitStatus)` / `NeverBegan`. vb_bench maps two `Crashed` (layer load fault) children to `validation-unavailable`; `Panicked` panics. | "`pub enum ChildOutcome { Done, Skipped(SkipClass, String), Unmarked, NoRecord }`" |
| O5 Job Object ungated, spawn race | A **shepherd** (the runner re-executed) assigns *itself* to the job before spawning, so no descendant can escape. New gate G16 checks that a grandchild is killed. | "A Windows Job Object (`KILL_ON_JOB_CLOSE`) lets the timeout kill the whole process tree." (the mechanism, not the goal) |
| O6 `teardown=open` for about 150 tests | Closed by W5's contexts-dead-at-`done()` rule. | see W5 |
| O7 G14 at C2 cites `leg list` | G14 is split: a scan-level fixture at C2, `leg list` at C5. | "G14 membership \| C2, C4, C11" |
| O8 02 rules 7 and UG-15 | B5-0 files MQ-24 (record-only). UG-15 mode is named at the cut: strict, identical by construction, because `boyko_demo` links neither `boyko_app` nor `boyko_rhi_vulkan` (`crates/boyko_demo/Cargo.toml`: ecs, macros, diag, threadpool, log). | "If strict mode flags `frame_loop`, B5(i) declares attributed mode with that one named body." |
| O9 Census trips itself | Rule tables and fixtures live in `boyko_leg_core` (excluded from the scan). `tests/ignore_reasons_census.rs` becomes a thin driver. | — |
| O10 Feature unification | New std-only crate `boyko_leg_core` (vocabulary, records, scan, rules). The root package dev-depends on it alone, keeping `Cargo.toml:137-142`'s "adds nothing to any graph". | "the root package (core, for the census)" as a dependant of `boyko_leg` |
| Review OQ1: sizes | Per-sub-rung estimates are in §8. All are ≤ L; a cut that estimates above 5000 splits by file group. | — |
| Review OQ2: single vs multi | D10's mode rule. | — |
| Review OQ3: probe `set_var` | The probe runs in a shepherded child with the posture passed through `Command::env`. The runner never calls `set_var`. | — |
| Own changes | `boot_unvalidated` / `validation_off` must carry a declaration; `BOOT_LOCK` (compute.rs's measured loader race) moves into `vk::boot`; `BOYKO_LEG_ID` / `BOYKO_LEG_PIN` added to the reserved set; UG-12 is migrated per sub-rung; D-E21's future self-spawn harness is re-spelled on `worker_command` (DOC-B5). | — |

## Goal

- **Today:** on a GPU-less box, 108 of 141 device-leg tests report `ok` having measured nothing, and so do at least 108 default-leg device tests (inventory 1-E).
- **After:** in a leg run, every test whose outcome depends on the machine ends in exactly one of four verdicts:
  - **PASS:** it reached `done()` and its validation verdict is clean or the degrade is declared;
  - **SKIPPED(classes):** every skip class it recorded is admissible on this machine;
  - **KNOWN-RED(finding):** a failure the machine file names exactly;
  - **FAIL:** anything else.
- A run that measured nothing exits non-zero.

| Metric | Target |
|---|---|
| Engine, per frame | 0 bytes. +1 integer add in `frame_loop` (MQ-24, record-only) |
| Engine, per boot | +1 `fetch_add(Release)` (`instances_created`); `DebugMessengerState` is held by `Arc` instead of `Box` (the same one allocation) |
| Engine, per validation message | unchanged count of atomic RMWs per class (the per-context state records the ledger's class instead of severity only) |
| Leg members ending libtest `ok` with no terminal record | 0 (FAIL by construction) |
| Validation messages in a leg run not attributed to a test | 0 (exclusive mode) |
| Device contexts alive at `done()` | 0 (runtime panic) |
| Child records the judge never sees | 0 |
| Tracked files modified by a leg run | 0 (judge row 6) |
| Runner overhead per member | spawn + shepherd + `--list` + `git status`: measured in the report footer, not gated |

## Where the inputs are wrong (unchanged, still true)

- The census enforces no vocabulary (`ignore_reasons_census.rs:139-148`).
- The two exit-101 workers are already guarded (`vg_density_census.rs:201-208`, `vg_r0d_census.rs:165-178`).
- 24 boyko_app scene tests pass silently: `run_windowed` returns `AppExit(true)` on boot failure (`runner.rs:233-296`, `:1012-1014`).
- vkval already reached the core rule, "exit 0 = no verdict" (`boot_validation_clean.rs:641`).
- The claim that the layer crashes on load is a windows-gnu measurement, unverified under msvc (vkval `device.rs:820-838`).

## Key decisions

**D1. A skip is a typed record, judged by a runner against a committed per-machine baseline.**
- libtest has no runtime skip (rust-lang/rust#68007).
- The model is deqp-runner and the CTS: a status per test, a baseline keyed test × machine, `Missing` fails the run, and UnexpectedImprovement fails the run (W1).

**D2. Every leg test reports how it ended (piglit: no marker means NOTRUN).**
- `Leg` is a state machine: `Open → Skipped`, or `Open → consumed by done()`.
- These panic in-process: `done()` after `skip()`, a second `skip()`, and `degrade()` after `skip()`.
- An `Open` Leg dropped outside a panic writes UNMARKED and panics.

**D3. Identity is the libtest thread name, module path included (rust-lang/rust#103681).**
- `boyko_leg_core::scan` resolves inline `mod x {}` and `mod x;` files per test binary.

**D4. Channel.**
- Records go as one line to stderr, `$BOYKO_LEG_LEDGER` (per member) and `$BOYKO_LEG_SPAWN` (per child).
- Each record carries its parent chain and pid. `BEGIN` goes to the files only.

**D5. Skip and degrade are different types.** `validation-off` is a `DegradeClass`. No machine-wide row can admit `engine-degraded`.

**D6. One process per test; each (package, feature set) is built once.**
- The runner runs `exe --exact <name> --include-ignored --test-threads=1 --nocapture` under a shepherd (D13).
- cwd is the package dir. The environment is the parent's minus `BOYKO_*`, plus the posture.
- `BOYKO_PROFILE` stays a build input.

**D7. Membership is derived from source.** A test is a member if both hold:
- its body contains the fully qualified `boyko_leg::Leg::begin()`;
- it is not ignored, or is ignored with class ∈ {`gpu`, `gpu-windowed`, `gpu-cap`}.

Further:
- Pinned tests expand per pin.
- Features come from `cfg`; the `feature` class is removed.
- (d2) makes `Leg::begin()` legal only in `#[test]` bodies. Every device boot takes `&Leg` (W4), so every device test is a member by construction.

**D8. Workers.** The first statement is `let Some(leg) = boyko_leg::worker() else { return };`.
- If `BOYKO_LEG_WORKER` is absent, the worker records SKIP `worker-unspawned`.
- If it holds the worker's own name, the worker runs.
- If it holds another value, the worker panics.
- A member spawned as a worker panics in `Leg::begin()`.

**D9. Children go through `LegCommand` only.**
- `worker_command` / `child_command` set `BOYKO_LEG_PARENT`, `BOYKO_LEG_ID`, a fresh `BOYKO_LEG_SPAWN` and the posture, and keep `BOYKO_LEG_LEDGER`.
- `LegCommand` has no `env_clear`; `env`/`env_remove` panic on reserved keys; `scrub_boyko()` keeps `BOYKO_LEG_*`.
- `status()` / `output()` / `spawn()` return a `ChildRun` whose `outcome ∈ {Done, Skipped(class, detail), Unmarked, Panicked, Crashed(ExitStatus), NeverBegan}`, parsed from the spawn file and the exit status (O4):
  - `Panicked`: BEGIN seen, no terminal record, exit 101;
  - `Crashed`: exit ∉ {0, 101};
  - `NeverBegan`: no BEGIN, exit 0/101.

**D10. The validation oracle runs inside `Leg::done()`, in one of two modes fixed per process by a pure function of argv/env (W5).**

| Mode | Chosen when | What `done()` gates |
|---|---|---|
| **Exclusive** | `BOYKO_LEG_ID` equals this test's id (runner member or `LegCommand` child), **or** libtest runs sequentially: argv `--test-threads=1` / `--test-threads 1`, else env `RUST_TEST_THREADS=1` | The process-ledger delta over [begin, done]. Tests run one at a time and libtest joins each test thread before starting the next, so every message in the window belongs to this test. |
| **Parallel** | otherwise (plain `cargo test`, UG-01) | Only the per-context states of contexts this Leg booted. The process ledger is not read. |

Rules common to both modes, applied in order:
1. **Contexts dead.** Every context registered by `vk::boot` has `Arc::strong_count == 1`, else panic "context booted by this leg is alive at done(); drop it first". This puts the device-destroy window, heard by the persistent messenger (vkval `debug.rs:149-152`), inside the verdict. It also means that after `done()` no device work is possible: the Leg is consumed and every context is dead.
2. **Exempt contexts.** A context marked with `vk::expect_validation(&leg, &ctx, Expect::Any | Expect::AtLeast(n))` is exempt from the clean gate. With `AtLeast(n)`, fewer than n gated messages is a panic (a deaf oracle).
3. **Exclusive mode:**
   - `Δinstances_created == 0` → no oracle (`validation=none`).
   - `Δmessengers_created < Δinstances_created` → DEGRADE `validation-off` (`posture:` or `declared:`).
   - Gated = Δ(errors + general_errors + validation_warnings) − Σ exempt contexts' final per-class counts. Gated > 0 → panic with the counts.
4. **Parallel mode:**
   - Per registered, non-exempt context: gated per-class counts > 0 → panic.
   - `!validation_enabled()` → DEGRADE.
   - Instance-window messages (NULL user data) are not gated in this mode. This blind spot is disclosed in §11 and covered by leg runs.
5. **Mode-only calls.** `app::run` and `done_expecting_validation(min)` (the process-level canary) panic in parallel mode with the text "needs exclusive mode: --test-threads=1 or the leg runner". This is deterministic because the mode depends only on argv/env.
6. **DONE record:** `validation=clean|off(posture|declared)|none mode=exclusive|parallel heard=<n>`.

**Rejected alternatives:**
- A process delta in parallel mode: attribution would depend on timing.
- A closure-scoped context API: the closure can return the context, and Rust has no negative bound to prevent it.
- Keeping `teardown=open`: it discloses the gap instead of closing it.

**D11. One posture definition, in `boyko_leg::posture`, shared by the runner and `LegCommand`.**
- **Strip:** `VK_KHRONOS_VALIDATION_*`; `VK_LAYER_*` except `VK_LAYER_PATH` and `VK_ADD_LAYER_PATH`; `VK_INSTANCE_LAYERS`; `VK_LOADER_LAYERS_{ENABLE,DISABLE,ALLOW}`. Names are compared case-insensitively.
- **Set:**
  - `VK_LOADER_LAYERS_DISABLE=~implicit~`;
  - `VK_KHRONOS_VALIDATION_ENABLE_MESSAGE_LIMIT=false`;
  - `BOYKO_WIN_HIDDEN=1` unless the machine file says `window visible`;
  - `BOYKO_LEG_VALIDATION`, `BOYKO_LEG_WINDOW`;
  - posture `on` → `BOYKO_ENABLE_VALIDATION=1`; posture `off` → `BOYKO_DISABLE_VALIDATION=1`.
- **Current posture:** `Posture::current()` reads `BOYKO_LEG_VALIDATION`; when it is absent (outside the runner) the posture is `On`. Children inherit the current posture. `LegCommand::validation_off(why)` forces `off` for a child and requires the driver file's declaration (rule (f)).
- **Reserved keys:** all of the above plus `BOYKO_LEG_*`.

**D12. Engine witnesses, written once each, never per frame.**
- `HostBootFailed { stage }` is inserted on each boot-failure return.
- `HostFramesPresented(n)` is inserted once after `frame_loop` returns, before `teardown`.
- `frame_loop` counts `presented_ok`.
- Rejected, as in rev 2: a per-frame World write (G-LOOP), and a runner return-value change.

**D13. The runner never runs a member directly; a shepherd does (O5, OQ3).**
- The shepherd is `leg __shepherd <timeout_s> -- <exe> <args>`, i.e. the runner binary re-executed.
- It creates a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assigns **itself** to it, and only then spawns the member. Every descendant is therefore in the job from creation, with no spawn/assign race.
- On timeout it calls `TerminateJobObject` and exits 124. If the shepherd itself dies, the last job handle closes and the job is killed.
- The probe runs the same way, so the runner process never calls `std::env::set_var` (which is `unsafe` in edition 2024).
- On non-Windows the shepherd kills only its direct child (disclosed).

## 1. The crates

- **`crates/boyko_leg_core`** (std-only, new member):
  - `IGNORE_CLASSES`, `SkipClass`, `DegradeClass`;
  - the record grammar (`format_record`, `parse_record`);
  - `scan`: lexer, fn bodies, module paths per binary, directives, the cfg grammar;
  - `rules`: census rules (a)–(j), their token tables and fixture tables.

  The root package dev-depends on this crate **only** (O10).
- **`crates/boyko_leg`** (new member):
  - runtime: `Leg`, `LegCommand`, `posture`, `pins`, `tool`;
  - feature `vk` (→ `boyko-rhi-vulkan`) and feature `app` (implies `vk`, → `boyko-app`);
  - bin `leg` (`required-features = ["vk"]`): runner, judge, machine file, shepherd.

  It is a dev-dependency only, of boyko-app (`app`), boyko-render and boyko_rhi_vulkan (`vk`). Lib unit tests do not use it (§11).
- **Scan scope:** `tests/**`, `crates/*/tests/**`, `src/**/tests.rs`. `crates/boyko_leg{,_core}/**` is excluded.

**Skip classes.**

| class | meaning | admissible by |
|---|---|---|
| `device` | no loader, ICD or physical device | `allow` or `expect` |
| `validation-unavailable` | validation requested; the layer is absent or crashed | `allow` or `expect` |
| `wsi` | window, surface, swapchain or WSI extension | `allow` or `expect` |
| `payload-absent` | `assets/vg_corpus` missing | `allow` or `expect` |
| `artifact-absent` | a sibling binary is not built | `allow` or `expect` |
| `tool-absent` | dxc, spirv-dis or git missing | `allow` or `expect` |
| `platform` | E3004 | `allow` or `expect` |
| `cap` | the device lacks what the test queried | **`expect` only** (O1) |
| `surface-lost` | surface lost after a good boot | **`expect` only** (O1) |
| `engine-degraded` | the engine chose a degraded path | **`expect` only** |
| `worker-unspawned` | worker run without its driver | never |

- **Degrade class:** `validation-off`.
- **`IGNORE_CLASSES`:** `gpu`, `gpu-windowed`, `gpu-cap`, `worker`, `solo`, `slow`, `miri-slow`, `miri-unsupported`, `generator`, `deferred`, `flaky`.

```rust
// ---- boyko_leg (std) ----
#[must_use = "an open Leg dropped without done()/skip() panics"]
pub struct Leg {                                   // !Send + !Sync: all transitions on the test thread
    id: Box<str>,                                  // libtest thread name, module path included
    parent: Option<Box<str>>,                      // BOYKO_LEG_PARENT when a child
    mode: Mode,                                    // Exclusive | Parallel, fixed at begin (D10)
    state: Cell<State>,                            // Open | Skipped
    #[cfg(feature = "vk")] at_begin: ValidationLedgerSnapshot,
    #[cfg(feature = "vk")] contexts: [Cell<Option<Arc<DebugMessengerState>>>; 8], // registered by vk::boot; 9th boot panics
    #[cfg(feature = "vk")] expect: [Cell<Option<Expect>>; 8],                     // exempt contexts (negative controls)
}
impl Leg {
    pub fn begin() -> Leg;                         // panics: no/"main" thread name; BOYKO_LEG_WORKER set
    pub fn id(&self) -> &str;
    pub fn skip(&self, c: SkipClass, detail: impl Display);          // Open -> Skipped, else panic
    pub fn degrade(&self, c: DegradeClass, detail: impl Display);    // Open only
    pub fn done(self);                                               // D10 oracle, then DONE
    pub fn done_expecting_validation(self, min: u32);                // exclusive only: process canary
    pub fn propagate(&self, run: &ChildRun) -> bool;                 // Done→true; Skipped(c)→skip(c),false; else panic
}
impl Drop for Leg {}                               // Open && !panicking -> UNMARKED, then panic!
pub fn worker() -> Option<Leg>;
pub fn worker_command(parent: &Leg, worker: &str) -> LegCommand;
pub fn child_command(parent: &Leg, t: Target, test: &str) -> Option<LegCommand>; // None: sibling exe absent
pub enum Target { ThisBinary, Sibling(&'static str) }             // via $BOYKO_LEG_BINS, else newest-mtime + note
pub struct LegCommand { /* std::process::Command */ }
impl LegCommand {
    pub fn env(&mut self, k: &str, v: impl AsRef<OsStr>) -> &mut Self;   // panics on reserved keys
    pub fn env_remove(&mut self, k: &str) -> &mut Self;                  // panics on reserved keys
    pub fn scrub_boyko(&mut self) -> &mut Self;                          // removes non-reserved BOYKO_*
    pub fn validation_off(&mut self, why: &'static str) -> &mut Self;    // rule (f): file declares off
    pub fn current_dir/stdin/stdout/stderr(..) -> &mut Self;
    pub fn status(&mut self) -> io::Result<ChildRun>;                    // inherited stdio (renamed from run, W3)
    pub fn output(&mut self) -> io::Result<ChildRun>;                    // captured
    pub fn spawn(&mut self) -> io::Result<LegChild>;                     // try_wait / kill / wait -> ChildRun
}
pub struct ChildRun { pub status: ExitStatus, pub outcome: ChildOutcome, pub stdout: Vec<u8>, pub stderr: Vec<u8> }
pub enum ChildOutcome { Done, Skipped(SkipClass, String), Unmarked, Panicked, Crashed(ExitStatus), NeverBegan }
pub fn tool(leg: &Leg, t: Tool) -> Option<PathBuf>;                     // Tool::{Dxc, SpirvDis, Git}; None -> skip(tool-absent)
pub mod posture;  // Posture::{current, apply}, is_reserved, scrubbed
pub mod pins;     // strict reader of goldens/PINS.toml's flat subset; reserved keys and RUSTUP_TOOLCHAIN dropped
pub(crate) fn mode_from(args: &[OsString], env: impl Fn(&str) -> Option<OsString>, id: &str) -> Mode; // pure (G10)
// ---- feature "vk" ----
pub mod vk {
    pub fn boot(leg: &Leg, windowed: bool) -> Option<VulkanContext>;   // validation requested; BOOT_LOCK; registers the Arc; Err -> skip(class_of)
    pub fn boot_unvalidated(leg: &Leg, windowed: bool, why: &'static str) -> Option<VulkanContext>; // rule (f); DEGRADE declared
    pub fn expect_validation(leg: &Leg, ctx: &VulkanContext, e: Expect); // exempts that context
    pub enum Expect { Any, AtLeast(u32) }
    pub fn class_of(e: &BootError) -> SkipClass;                        // exhaustive, no `_` arm
    pub fn require_validation(leg: &Leg) -> bool;                       // posture off -> skip(validation-unavailable)
    pub(crate) fn oracle(inp: OracleInput) -> Oracle;                   // pure over snapshots and per-context counts (G10)
}
// ---- feature "app" ----
pub mod app {
    pub fn run(leg: &Leg, app: &mut App) -> Option<Ran>;                // exclusive only; app.run(), then judge_run
    pub fn judge_run(leg: &Leg, w: &World) -> Option<Ran>;              // pure over the World
    pub fn class_of_stage(s: &HostBootStage) -> Option<SkipClass>;      // None = not a machine fact -> panic
    pub struct Ran { pub frames_presented: u64, pub exit: AppExit }
}
```

**`judge_run`:**

| World after `app.run()` | Result |
|---|---|
| `HostBootFailed(s)` with `class_of_stage(s) = Some(c)` | `skip(c)`, returns `None` |
| `HostBootFailed(s)` with `class_of_stage(s) = None` | panic |
| `HostFramesPresented(0)` | panic: "BOOTED, ZERO FRAMES PRESENTED" |
| neither resource | panic: "not the windowed runner" |
| both resources | panic (runner invariant broken) |

**Records.** CR/LF in a detail becomes ` | `. Each line is one `write_all` to a file opened with `append(true)`.
```
BOYKO-BEGIN    <id>[ <- <parent>] #<pid>                       (files only)
BOYKO-SKIP     <id>[ <- <parent>] #<pid>: <class>: <detail>
BOYKO-DEGRADE  <id>[ <- <parent>] #<pid>: validation-off: <posture|declared>: <detail>
BOYKO-DONE     <id>[ <- <parent>] #<pid>[: validation=clean|off(posture|declared)|none mode=exclusive|parallel heard=<n>]
BOYKO-UNMARKED <id>[ <- <parent>] #<pid>: returned without done()/skip(); nothing it measured is a verdict
```

**Threading and memory ordering.**
- `Leg` is `!Send`, so every transition and every `contexts` slot access happens on one thread.
- `Arc<DebugMessengerState>` clones are read with `Acquire` loads that pair with the callback's `Release` `fetch_add`s (the existing vkval pairing). The `strong_count` check at `done()` runs after the context drop on the same thread, so no callback is in flight.
- `BOOT_LOCK` is a `std::sync::Mutex<()>` held only across `VulkanContext::boot`. It moves the measured loader/driver creation race from `compute.rs:62-70` into one place. It is test infrastructure, never the hot path, and carries `#[allow(clippy::disallowed_types)]` plus a rationale. Poison is tolerated.
- The spawn-file counter is an `AtomicU32` with `Relaxed` ordering; only uniqueness is needed.
- A torn ledger line is FAIL corrupt-ledger.

**unsafe.**
- Runner/shepherd: `CreateJobObjectW`, `SetInformationJobObject`, `GetCurrentProcess`, `AssignProcessToJobObject`, `TerminateJobObject`, `CloseHandle`, each with its own `// SAFETY:`.
- Engine: the callback's existing `unsafe` re-states its invariant for `Arc::as_ptr`: the context owns one `Arc` and destroys the messenger before dropping it; other holders only load atomics.

## 2. Engine additions (B5-0; all cold except one add)

- **`boyko_app/src/boot_outcome.rs`** (new, re-exported from the prelude):
  - `#[derive(Resource)] pub struct HostBootFailed { pub stage: HostBootStage, pub detail: Box<str> }`.
  - `pub enum HostBootStage { VulkanDevice(VulkanError), Window, Surface, Swapchain, Renderer, SwapchainFormat(i32), BindlessTextureTable, Platform }`. It has exactly one variant per `HostBootError` variant (`host.rs:31-46`) plus the three runner stages.
  - `#[derive(Resource, Clone, Copy)] pub struct HostFramesPresented(pub u64)`.
  - `class_of_stage` is an exhaustive match with no `_`:

    | Stage | Class |
    |---|---|
    | `VulkanDevice(VulkanError::Boot(e))` | `class_of(e)` |
    | `VulkanDevice(any other VulkanError)`, including `SingletonAlreadyBooted` | `None` |
    | `Window`, `Surface`, `Swapchain` | `wsi` |
    | `SwapchainFormat` | `cap` |
    | `Platform` | `platform` |
    | `Renderer`, `BindlessTextureTable` | `None` |

- **`runner.rs`** (lines are joltab's; re-derived at the cut):
  - Insert `HostBootFailed` before each `return AppExit(true)` at `:237`, `:255`, `:296`; the World holds nothing yet (`:250-253`).
  - In the non-Windows arm (`:1013`), insert `Platform`.
  - `frame_loop` (`:1087`) returns `u64`, adding `frames_presented += u64::from(presented_ok)` at `:2776`.
  - `:943-945`: a startup exit yields 0. `HostFramesPresented` is inserted once, before `teardown` (`:947`).
- **`boyko_rhi_vulkan`** (vkval's `debug.rs` / `device.rs`):
  - `ValidationLedger` gains `instances_created`, with a `fetch_add(Release)` at **one** site: `create_instance`, right after `vkCreateInstance` succeeds. Both `boot` and `boot_singleton` (`device.rs:952` calls `Self::boot`) reach it once (O2).
  - `DebugMessengerState` records the ledger's four classes (`errors`, `general_errors`, `validation_warnings`, `general_warnings`) from the same `classify_message` result, so per-context and process gates agree on what is gated.
  - `VulkanContext` holds `Arc<DebugMessengerState>` (was `Box`) and gains `pub fn debug_state_handle(&self) -> Option<Arc<DebugMessengerState>>`.

## 3. The runner (`boyko_leg` bin `leg`)

```
cargo run -p boyko_leg --features vk --bin leg -- list  --machine owner-rtx3060
cargo run -p boyko_leg --features vk --bin leg -- probe --machine owner-rtx3060 [--write-machine]
cargo run -p boyko_leg --features vk --bin leg -- run   --machine owner-rtx3060 [--only <pkg|substr,…>] [--no-probe]
```

1. **Scan** (per binary, `mod` files resolved). For each member derive:
   - package, feature set (cfg grammar: `windows`, `unix`, `miri`, `loom`, `feature=""`, `target_os=""`, `all`, `any`, `not`; anything else → exit 2 naming the site), libtest path and workers;
   - directives: `leg-env[(<test>)]`, `leg-timeout` (default 900), `leg-validation: off -- <class>: <why>`.

   **Pins:** one member per `goldens/PINS.toml` pin, identity `test[pin]`.
   - The environment is the pin's `[pin.env]` minus reserved keys and `RUSTUP_TOOLCHAIN`, with `BOYKO_HOST_DUMP` → `{out}/<pin>.bmp` and `BOYKO_LEG_PIN=<pin>`.
   - The runner checks that the toolchain it built with is the host's, and exits 2 otherwise.
2. **Machine file** `crates/boyko_leg/machines/<name>.leg`. It is strict: an unknown key or a row without `-- why` exits 2.
   ```
   device      NVIDIA GeForce RTX 3060 Laptop GPU
   validation  on|off  -- <measured: date, host, probe result>
   window      hidden|visible -- <why>
   allow       <class> -- <why>          # class ∈ {device, validation-unavailable, wsi, payload-absent, artifact-absent, tool-absent, platform}
   expect      <pkg>/<target>::<test>[<pin>] <class> -- <why>       # any class except worker-unspawned
   known       <pkg>/<target>::<test>[<pin>] "<panic-text substring>" -- <FINDING-ID>: <why>   # ID = [A-Z][A-Z0-9]*-[0-9]+
   ```
   After B5-6 merges, the file is edited only by the owner or a document step, never by a code rung.
3. **Probe** (OQ2: measured, never inherited). Required before each run; `--no-probe` exists only for G6(b). Every step runs in a shepherded child under the posture environment:
   - (i) a validated headless boot gives Δinstances == Δmessengers == 1 (O2);
   - (ii) a deliberately leaked shader module is heard at teardown (errors ≥ 1);
   - (iii) boot → drop 3× without a crash;
   - (iv) member `boyko-app/windowed_smoke` presents ≥ 1 frame with Δinstances == 1.

   A mismatch prints one line and exits 2. `--write-machine` writes the file; it is a separate command, never part of `run`.
4. **Build** per (package, feature set) with the invoker's environment. The strict in-house JSON reader maps `src_path` → `executable` and `manifest_path` → cwd. It writes `{run}/bins.txt` and exports `BOYKO_LEG_BINS`.
5. **Enumerate** with `<exe> --list --include-ignored`. An unlisted member is MISSING.
6. **Execute** one at a time through the shepherd (D13), with the posture, `BOYKO_LEG_LEDGER`, `BOYKO_LEG_ID`, `BOYKO_LEG_OUT={out}` and `BOYKO_LEG_BINS`.
   - A file with `leg-validation: off` gets posture `off`.
   - After each member, `git status --porcelain --untracked-files=no` is compared with the run start.
7. **Judge.** `judge()` is a pure function; the first matching row wins.

| # | condition | verdict |
|---|---|---|
| 1 | build failed | FAIL build |
| 2 | shepherd exit 124 | FAIL timeout |
| 3 | not listed, or no `running 1 test` | FAIL missing |
| 4 | a `BOYKO-` line that does not parse | FAIL corrupt-ledger |
| 5 | a record whose `<-` chain does not reach the member | FAIL foreign-record |
| 6 | a tracked file changed during this member | FAIL dirtied-tree(paths) |
| 7 | libtest FAILED (UNMARKED, state machine, oracle, contexts alive, zero frames, asserts) | KNOWN-RED(id) if a `known` row for this member matches the panic text; else FAIL libtest |
| 8 | the member's own terminal records ≠ 1 | FAIL no-verdict |
| 9 | the member is DONE and a child `BEGIN` lacks exactly one terminal record | FAIL child-no-verdict |
| 10 | a child UNMARKED | FAIL child-unmarked |
| 11 | a child SKIP `worker-unspawned` | FAIL driver-bypassed |
| 12 | skip-class set S = own SKIP ∪ every child SKIP is non-empty and every c ∈ S is admissible for this member | SKIPPED(S); FAIL unexpected-skip(id) if a `known` row exists |
| 13 | S non-empty, some c inadmissible | FAIL forbidden-skip(c[, child id#pid]) |
| 14 | DONE with a DEGRADE (own or child) that is neither posture `off` nor `declared:` by the member's or its parent's file | FAIL forbidden-degrade |
| 15 | DONE | PASS (annotated DEGRADED); FAIL unexpected-pass(id) if a `known` row exists |

   **Post-run rows:**
   - FAIL stale-expectation: an `expect` row whose selected member did not end SKIPPED with that class.
   - FAIL stale-allow: an `allow` row no SKIP used (full runs only).
   - FAIL stale-known: a `known` row naming a test absent from `leg list` (in `--only` runs, only rows matching the filter).
   - FAIL orphan-worker: a driver named in a worker's reason ended PASS, and the worker has no `BEGIN` in any ledger (W2).
8. **Report.**
   ```
   leg run @ owner-rtx3060 (<device>) | validation on (probe: armed 1/1, canary heard, 3/3, frames ≥1) | window hidden
     host x86_64-pc-windows-msvc, rustc <v> | BOYKO_PROFILE=<v|unset> | tree <root> @ <HEAD sha>, <clean|N modified at start>
     expected <N>  MEASURED <P> (<d> degraded)  skipped <S> (by class) -- NOT measured
     KNOWN-RED <k> (by finding)  FAILED <F> (each reason counted)  footer: runner overhead <s>
     VERDICT: RED|GREEN
   ```
   - Exit 0 iff F = 0 and P ≥ 1; exit 1 otherwise; exit 2 for infrastructure failures.
   - Output goes to `$CARGO_TARGET_DIR/leg/<run>/`. Maps are `BTreeMap`.

## 4. Children

- **Drivers** use `worker_command` (23 workers) or `child_command`, then `leg.propagate(&run)` or a match on `run.outcome`.
  - vb_bench: both children `Crashed(0xC0000005)` under posture on → `leg.skip(validation-unavailable, …)`. `Panicked` → panic. INCONCLUSIVE stays a panic.
- **`vb_mesh_screenshot_dump`** stays a member (`gpu-windowed`, 7 pins → 7 members): `Leg::begin()` + `app::run` + `done()`.
  - golden.ps1 runs it standalone as today. It passes `--test-threads=1`, so the mode is exclusive; with no `BOYKO_LEG_*`, records go to stderr only.
  - `vb_mesh_occ_pins_actually_split` re-runs it 5× through `child_command(&leg, ThisBinary, …)` with the environment from `boyko_leg::pins`. Its records are attributed to the parent.
- **`vb_p1d_cull_shade_bench`** is a member (`leg-validation: off -- timing: …`, `leg-env: BOYKO_WINDOW_FRAMES=<n>`) and the child of `vg_decidability_floor_measure`, which spawns it with `child_command(&leg, Sibling(..), ..)` plus `.validation_off("timing: the layer perturbs the timed window")`.
- **vkval workers** call `leg.done()` (the Gate A canary: `done_expecting_validation(1)`) before `process::exit(90|91|92)`. The App is torn down inside `app::run`, so teardown is inside the window.

## 5. Validation: every site and what it becomes

Sites are on joltab @ a1849541 unless marked K/ (vkval). The enumeration of `enable_validation: false` / `InstanceConfig::default()` is a grep of `crates/**/tests/**` on 2026-09-18.

| Site | Today | Becomes |
|---|---|---|
| every leg run | a shell-wide `BOYKO_DISABLE_VALIDATION=1` | the runner strips `BOYKO_*`; posture from the machine file |
| worker spawns forcing off: `hzb_engine_pyramid_gate :743`, `vb_cull_hzb_pairing :164`, `vb_inst_cull_scene/mod.rs :709`, `vb_occ_mixed :376`, `vb_occ_split_gate :549`, `vg_thresholds/mod.rs :296` | off; the only reason is the MinGW recipe | `worker_command`, posture (on) |
| timing drivers `vb_sv0_produce_run_timing :483`, `vg_occ_split_timing :1042`, `vg_decidability_floor :269`; `vb_p1d_cull_shade_bench` | off | `//! leg-validation: off -- timing: the layer perturbs the timed window`; drivers add `.validation_off(..)`; vb_p1d by directive |
| cost harnesses `ddgi_probe_gi_cost :131`, `software_ray_baseline_cost :120` (W6) | off, "measures cost" | `leg-validation: off -- timing: …` + `vk::boot_unvalidated`. §7's "V/ local boots → `vk::boot`" no longer covers these two. |
| `calibrated_timestamp_probe :38` | off, no reason | `timing:` + `boot_unvalidated`. The layer intercepts `vkGetCalibratedTimestampsEXT`, which is the deviation being measured. |
| query/zone data paths `gpu_zone_deadlines :36`, `gpu_zone_label_control :68`, `gpu_command_census :68`, `gpu_query_availability_truth :59` (W6) | off, "a data path" | **on:** `vk::boot`. Query-pool use is what the layer checks, and none of these assert wall-clock time. |
| correctness smokes `ddgi_probe_gi_arm :58`, `ddgi_probe_gi_resolve :72`, `roundtrip :32/:139/:378`, `vg_block_pool_growth :51`, `present_mode_probe :40` (W6) | off, no reason or the default config | **on:** `vk::boot` |
| `hzb_build_oracle_gate :420`, `hzb_verdict_oracle_gate :572` (OQ3) | off; the only reason is the run line (`:45`, `:83`) | **on:** `vk::boot(&leg, false)` |
| `g7_validation_reporting :69/:83/:94` | tests the escape hatch | `leg-validation: off -- escape-hatch: exercises E2101`; (h1) waiver (raw boots whose result is discarded by design); no Leg |
| `particle_sim_occupancy` `VulkanProbe` (own FFI instance) | no layer | `leg-validation: off -- raw-instance: reads register statistics`; its routes go through the Leg |
| K/`unwritten_shadow_map_gate :487` | off, no reason | posture (on) |
| `vb_bench_query_validation :368`; K/`boot_validation_clean :664-717` | local arming and scrub | `worker_command` (+`scrub_boyko`); `vk::require_validation`; the predicate moves to `posture` |
| PINS.toml `[*.env]` (32 blocks) | children forced off | `pins` drops reserved keys; the file is unchanged |
| ~55 local `var_os("BOYKO_DISABLE_VALIDATION")` oracle helpers | NOTE, or the oracle silently dropped | deleted; `done()` is the oracle |
| `m2 :60-70`, `orbit :610-617`, `p7b :1091-1098`, `ui_hud :1084-1091/:1160-1167`, `camera_drives :350` | the whole test skipped | deleted; the test runs |
| NOTE lines (H3n, H6, hwrt, spec_constant); silent checks (m3, m5, `window_present_gbuffer :10334/:8910`) | NOTE, or silent | removed; `done()` |
| **negative controls (W5):** `sync_validation::omitting_the_barrier_manifests_a_hazard`; `compute::negative_chained_barrier_hazard` | per-context oracle on its own context; "skips the clean assertion" | the hazard context gets `vk::expect_validation(&leg, &ctx2, Expect::Any)`; the control context and test A stay default-gated per context; the data-oracle fallback is unchanged |
| f6, f7_grow, texture_retire, the 24 scene/dump tests | nothing checked | posture; `app::run` + `done()` |
| `teardown_probe/mod.rs:69` `ENV_ALLOWED` | 4 `BOYKO_*` literals | `posture::is_reserved(k) \|\| k == "BOYKO_LOG"` |
| run lines "with BOYKO_DISABLE_VALIDATION=1" | prose | removed (C11) |

**A new validation red at a site this lane switched on (W6).** B5-4 and B5-5 change test code and `boyko_leg` only.
- A message caused by the test harness's own API use is fixed in the same commit.
- A message caused by engine code gets a finding ID in the session record and a `known` row keyed on the oracle's panic text, and the sub-rung merges.
- A red at a site that was already validated on the parent is a regression, and the rung fixes it.
- A new `leg-validation: off` cannot be the answer: its reason class vocabulary (`timing`, `escape-hatch`, `raw-instance`) has no entry for "the layer reports something".

## 6. Census rules (`boyko_leg_core::rules`; `tests/ignore_reasons_census.rs` is a thin driver)

All rules:
- are evaluated **per test binary** (root file plus resolved `mod` files; a shared `mod` file is evaluated once per including binary) (W4);
- have token tables and fixture tables in `boyko_leg_core` (O9);
- have a waiver table with reasons and the stale-row rule, and an anti-vacuity floor.

| Rule | Checks | On at | Floor, and why it is reachable there |
|---|---|---|---|
| (a) | Every plain reason starts `<class>: `, class ∈ `IGNORE_CLASSES`; `cfg_attr` is exempt. Sites in files held by an open rung at C11's cut go into `PREFIX_PENDING` (reasons required, count only shrinks, stale → red). A rung holding such a file prefixes it in passing and deletes the row (W7: e.g. D-E23's `miri_fixed_loop.rs:72`). | C11 | Prefixed sites ≥ the count at C11 (≥ 178 minus pending). C11 prefixes every unheld site, including the non-device ones below. |
| (b) | A `worker:` body's first statement is `let Some(<id>) = boyko_leg::worker() else { return };`. The reason is `worker: spawned by <fn>[, <fn>]; …`, and each named driver exists in the binary and contains `boyko_leg::worker_command(` with the worker's name. | C12 | ≥ 23 (16 joltab + 7 vkval after C9–C10) |
| (c) | No whole-word `SKIP`/`SKIPPED`/`SKIPPING` (case-sensitive), no `skip`/`skipped`/`skipping` (case-insensitive), and no "validation disabled", in the first literal of the print/format macros. Word characters are `[A-Za-z0-9_]`. | C12 | literals scanned ≥ the count at C12, rounded down to 100 |
| (d) | (d1) every `gpu*` body contains `boyko_leg::Leg::begin()` and is not `#[should_panic]`; every `worker:` body contains `boyko_leg::worker()`. (d2) `boyko_leg::Leg::begin()` appears only inside `#[test]` bodies. | C12 | ≥ 120 |
| (e) | `leg-env` parses and sets no reserved key; a pinned test has no `leg-env`. | C12 | ≥ 1 directive, plus fixtures |
| (f) | No string-literal token equal to `BOYKO_DISABLE_VALIDATION`, `BOYKO_ENABLE_VALIDATION`, `VK_INSTANCE_LAYERS` or `VK_LOADER_LAYERS_{ENABLE,DISABLE,ALLOW}`, or starting `VK_KHRONOS_VALIDATION_` / `VK_LAYER_`. No `InstanceConfig` or `enable_validation` token. `.validation_off(` / `vk::boot_unvalidated(` only in a file declaring `leg-validation: off -- <timing\|escape-hatch\|raw-instance>: <prose>`. A declaration is stale iff its file has neither a form nor a leg member (W3). | C12 | files ≥ `MIN_FILES`; declarations ≥ the count at C12 |
| (g) | No literal token `--ignored`, `--include-ignored` or `--exact`, and no `current_exe()`. | C12 | 4 waivers: `alloc_frame_census.rs`, `alloc_frame_attribution.rs`, `tb_neg_m2w_arm_present.rs`, `a6_panic_propagation.rs` |
| (h) | (h1) No `VulkanContext::boot(` or `VulkanContext::boot_singleton(` (waiver: `g7_validation_reporting` ×3). (h2) No local `fn find_dxc\|find_spirv_dis\|find_tool\|dxc_path\|spirv_dis_path`, and no literal token `dxc`, `dxc.exe`, `spirv-dis` or `spirv-dis.exe`. Every boot and every tool lookup then takes `&Leg`. | C12 | `vk::boot(` sites ≥ the count at C12 (≥ 100) |
| (i) | In a binary whose files name `EnginePlugins::window`, no zero-argument `.run()` call. `LegCommand::status()` does not match; the CPU gates call no `.run()` (W3). | C12 | window binaries ≥ 60 |
| (j) | A package that dev-depends on `boyko_leg` and depends on boyko_rhi_vulkan, boyko-render or boyko-app enables `vk` (or `app`). | C12 | 3 packages |

(b)–(j) switch on together at C12, because their floors are reachable only after C7–C10. C11 re-prefixes these non-device sites:

| Site | Class |
|---|---|
| `boyko_threadpool/tests/stress.rs:281` | `flaky:` |
| `boyko_sdf_math/src/brick/tests.rs:140`, `:461` | `deferred:` |
| `boyko_log/tests/l14_sink_policy.rs:102` | `solo:` |
| `boyko_ecs/tests/miri_fixed_loop.rs:72` | `miri-slow:` (or `PREFIX_PENDING` if D-E23 holds the file) |
| the two unprefixed generators | `generator:` |

## 7. Migration (joltab @ a1849541 unless K/; lines re-derived at each cut)

| Sites | New code |
|---|---|
| H1 (`R/common/mod.rs:30-41`, deleted); bindless_smoke, texture_upload_smoke, s35, p7b, p6b, `_msdf`; every 1-E render file | `boyko_leg::Leg::begin()` + `vk::boot` + drop `rhi`/`ecs` + `leg.done()` |
| V/ local boots (compute, ddgi_arm, ddgi_resolve, hwrt_blas, m2, m3, m5, H4, H6, spec_constant, and the §5 "on" sites); `compute.rs`'s `BOOT_LOCK` is deleted (it moves into `vk::boot`) | `vk::boot` |
| `ddgi_probe_gi_cost`, `software_ray_baseline_cost`, `calibrated_timestamp_probe` | `vk::boot_unvalidated` + declaration |
| cap routes (`ddgi_arm :156-159`, `ddgi_cost :253-259`, `hwrt` ×3, `software_ray :221-235`, `particle_sim_occupancy :738-745`, the 1-E timestamp routes) | `skip(cap)` + an `expect` row where the owner's box lacks the cap |
| `particle_sim_occupancy :609-611/:616-618/:652-654/:781-785` | `skip(device)` |
| H3 `window_present_gbuffer :1618-1707` (label → `&Leg`) | window/surface/swapchain → `skip(wsi)`; boot → `vk::boot`; extent/UNORM/Format → `skip(cap)` |
| 1-D runtime routes | `skip(surface-lost)`; the "capture failed" arms (`:6567-6575`, `:6690-6751`) → panic |
| H10 inline ×13; teardown_probe H7 `:241` | `app::run`; the `remaining == BUDGET` branch is deleted |
| `slot_fill :237-242`, `retire_churn :224-229`, H7 `:248` | `skip(engine-degraded)` |
| H8 ×3, H9 ×4 | `vk::boot(&leg, false)`, validated |
| the 24 fall-through tests | `Leg` + `app::run` + `done`; environment from the pin if pinned, else `leg-env: BOYKO_WINDOW_FRAMES=<n>` |
| `vg_decidability_floor` `:234-247`, `:267-311`, `:327-334`, `:113`/`:738` | `child_command(Sibling)`; `None` → `skip(artifact-absent)`; a child DONE with no artifact → panic; `FLOOR_DOC` goes to `{out}` unless `BOYKO_VG_FLOOR_WRITE_DOC=1` |
| `vb_sv0 :472-512`, `:549`/`:598`; `vg_occ_split_timing :1035-1128`, `:1200-1208` | the worker records `skip(cap)`; the driver calls `propagate`; text parsing deleted |
| `vb_bench_query_validation :573-586` | `require_validation`; `skip(cap)`; two `Crashed` → `skip(validation-unavailable)` |
| payload routes | `skip(payload-absent)` |
| tool routes (~13 `*_spv_sync`/`*_edsl_sync`, `particle_edsl_sync` ×13, `vb_geo_preprocess_sync`) | `boyko_leg::tool`; the locator is the union of the local copies' search roots |
| 16 joltab workers + K/ 6 Gate A workers + K/`shadow_poison_worker` | `worker()`; reason `worker: spawned by <driver>; …`; spawns → `worker_command` |
| `vb_mesh_screenshot_dump`, `vb_p1d_cull_shade_bench`, `vb_mesh_occ_pins_actually_split` | §4 |
| `sync_validation` A and B; `compute::negative_chained_barrier_hazard` | §5's negative-control row |
| tests red today (particle_lab, particle_counters, vb_cull_offscreen, vb_inst_cull_*, sv0_adequacy, vb_occ_dense_defers, hzb drivers ×3, vb_occ_split_gate, vb_occ_mixed ×2, vb_cull_hzb_pairing, vg_density_census_gate); K/ Gate A driver; K/ shadow drivers ×2 | `Leg` + `done` (+ `app::run`). If still red on the parent: a `known` row with a finding ID (W1) |
| `grand_showcase_mvpm :164` | `#![cfg(feature = "hwrt")]`; `!mesh_mvpm_active()` → `skip(cap)` |

## 8. Placement: rung B5 in Phase B (UNIFIED-SYSTEM-PLAN-02)

B5 absorbs B3's "ignore-reason prefix migration (closed vocabulary)", so the migration runs once, over the merged tree (feat/reflection via A6, feat/ui-advanced via A7). Rule 5 forbids XL, so B5 is seven sub-rungs. Only **B5-0** (S) touches files other rungs share.

| Sub-rung | Scope (commits) | Prereq | Size (est. changed lines) | Lock set | Gates |
|---|---|---|---|---|---|
| **B5-0** seams | root `Cargo.toml` (members, default-members, root dev-dep `boyko_leg_core`); crate skeletons with features `vk`/`app`; §2 engine additions; `ci.yml` feature-legs refusal row `boyko_leg/app` (platform; the same measurement as `boyko-app/*`); MQ-24 filed (C0) | A8; B3 merged (root `Cargo.toml`); vkval on the trunk | S (~580) | root `Cargo.toml`; `boyko_app/src/{runner.rs, boot_outcome.rs (new), lib.rs, prelude.rs}`; `boyko_rhi_vulkan/src/{debug.rs, device.rs}`; `.github/workflows/ci.yml`; `crates/boyko_leg{,_core}/{Cargo.toml, src/lib.rs}` (new) | UG-01; UG-15 strict (identical by construction: `boyko_demo` links neither crate); UG-16 record; UG-12 owner, legacy recipe (windowed_smoke, K/ Gate A); UG-14 if B2 has merged |
| **B5-1** core | vocabulary, records, G1 (C1); scan move to `boyko_leg_core`, per-binary modules, census as thin driver (C2) | B5-0 | L (~2400, incl. moved lines) | `crates/boyko_leg_core/**`; `tests/ignore_reasons_census.rs` | UG-01; UG-11 (identical counts, G15); UG-15 strict |
| **B5-2** runtime | Leg, LegCommand, posture, pins, tool (C3); `vk`/`app` (C4) | B5-1 | L (~3500) | `crates/boyko_leg/{Cargo.toml, src/**, tests/**}` except `src/bin/**` | UG-01; UG-15 strict; UG-12 owner (legacy recipe on `boyko_leg`'s device tests, G17) |
| **B5-3** runner | runner, shepherd, judge, report (C5); owner `probe --write-machine` (C6) | B5-2 | L (~3700) | `crates/boyko_leg/{src/bin/**, machines/**, tests/runner_*.rs}` | UG-01; UG-15 strict; UG-12 owner (probe + `leg run --only boyko_leg`) |
| **B5-4** rhi + render | C7, C8 | B5-3 | L (~2800) | files under `crates/{boyko_rhi_vulkan,boyko_render}/tests/**` that the scan lists at the cut; their `Cargo.toml`; `known` rows | UG-01; UG-10 scoped; UG-12 = `leg run --only boyko_rhi_vulkan,boyko-render` + G8; UG-15 strict |
| **B5-5** app | C9, C10 | B5-3 (parallel with B5-4; disjoint files) | L (~2900) | files under `crates/boyko_app/tests/**` listed at the cut; `boyko_app/Cargo.toml`; `known` rows | UG-01; UG-10 scoped; UG-12 = `leg run --only boyko-app` + golden.ps1's 7 `vb_mesh` pins; UG-18 |
| **B5-6** prefixes, rules, docs | C11, C12, C13 | B5-4, B5-5 | L (~2700) | files with plain `#[ignore]` not held by an open rung (the rest go to `PREFIX_PENDING`); `crates/boyko_leg_core/src/rules/**`; `tests/ignore_reasons_census.rs`; `CLAUDE.md`; `.claude/agents/tester.md`; the machine file | UG-01; UG-11; UG-12 full `leg run` (G6, G10 live, G14 real) |

A cut whose re-estimate exceeds 5000 lines splits by file group.

- **§4.3 rows.** Every new edge is `·` (first cut wins), not `→`, because no order among these rungs changes behaviour.

  | File | Rungs |
  |---|---|
  | root `Cargo.toml` | `A8 → B3 → { B5-0 · C1 · RP-2 }` |
  | `boyko_app/src/runner.rs` | `A2 → A5 (ddgi) → { B5-0 · D-E23 } → HO* → RF-A` |
  | `.github/workflows/ci.yml` | `AH → { B5-0 · RP-3 }` |
  | `boyko_rhi_vulkan/src/{debug,device}.rs` (new row) | `AH (device.rs) → A5 (vkval) → A8 (owner edit, device.rs) → B5-0 → (F4) RF-V` |
  | `tests/ignore_reasons_census.rs` (new row) | `B5-1 → B5-6` |
  | `crates/boyko_leg{,_core}/**` (new row) | `B5-0 → B5-1 → B5-2 → B5-3 → B5-6` |
  | `boyko_ecs/tests/miri_fixed_loop.rs` | `D-E23 · B5-6` via `PREFIX_PENDING` |

- **Why nothing is parked.**
  - C1 needs D-M0 (M), and B5-0 is S and cut right after B3, so C1 is ready only after B5-0 has merged.
  - D-E23 and B5-0 are both S and first-cut-wins on `runner.rs`.
  - No rung has any B5 sub-rung as a prerequisite.
  - Pool cost: at most one B5 sub-rung open at a time, two during B5-4 ∥ B5-5.
- **Pool order at A8:** B3, B1, B2; then B4; then B5-0, which waits on B3 for the root `Cargo.toml`.
- **UG-12 transition:**
  - rhi/render rungs run `leg run --only` from B5-4's merge;
  - boyko-app rungs from B5-5's merge;
  - every UG-12 from B5-6's merge.
- **UG-12's pass criterion (W1):** `leg run --machine owner-rtx3060 [--only <packages the rung touches>]` exits 0. That means F = 0 and P ≥ 1, where F includes unexpected-pass, unexpected-skip, stale-known and orphan-worker, and KNOWN-RED is not in F.
  - A rung may add no `known` row. Rows come only from B5-4 and B5-5, for reds that the owner's legacy-recipe run reproduces on their parent commit (listed in the cut entry, e.g. R-4 at `sdf_gbuffer_hybrid.rs:6967`), or from a finding-filing document step.
  - Which R-1..R-7 are still red is re-read at those cuts. R-1, R-2, R-3, R-5 and R-6 look repaired on joltab; R-4 does not.
- **vkval (W7b).**
  - **04 §1 new row:** `D:/wt/vkval` | `fix/boot-validation-errors` @ `e3cebe2d` plus uncommitted work (`debug.rs` ledger, `device.rs`, `boot_validation_clean.rs`, `unwritten_shadow_map_gate.rs`, `shadow_poison.rs`) | 0 committed | → committed on request, then A5.
  - **04 §2 step 5:** vkval is appended after `fix/ddgi-host-hook`. Its gates are UG-01, UG-12 (owner: Gate A and the shadow gate) and UG-18. Its `device.rs` hunk meets `chore/msvc-host` at the A5 merge and the owner's edit at A8 (§2 step 10's hand-resolved set).
  - **Fallback:** if it misses A5, it becomes rung **V0** after A8, in a pool worktree, with the same gates and lock set `{its five files}`, and B5-0 waits for it.
- **What B3 keeps:** UG-15 pin capture on msvc (probe build (i)–(v), frozen pin list), `boyko_symcensus`, the three profiles, leg (1)'s macro half, and the UG-08 and UG-09 recipes. B3's lock set **drops** `tests/ignore_reasons_census.rs` and the `#[ignore]` sites, so B1–B4 stay disjoint.
- **DOC-B5** (in `D:/wt/k-docs`, after approval):
  - **02:** the B3 row, the B5-0…B5-6 rows, lock sets, pool order, §3's DAG (`A8 → B3 → B5-0 → B5-1 → B5-2 → B5-3 → {B5-4, B5-5} → B5-6`, and `vkval → B5-0`), §4.3, MQ-24 in §3's MQ line, and D-E21's harness re-spelled as `worker:` + `boyko_leg::worker_command` (rule (g)).
  - **03:** UG-11 becomes "(B5-6)"; UG-12's mechanism becomes `leg run`, its anti-vacuity becomes the judge, and its §2 row adds B5-0, B5-2…B5-6.
  - **04:** §1 and §2 as above.
  - **00 §862:** the disjointness sentence.

## 9. Commit order (each commit green on its own)

**B5-0**
- **C0:** seams (§2). Root `Cargo.toml` members plus the root dev-dep on `boyko_leg_core`. Skeleton crates (`lib.rs` with module declarations; features `vk`/`app` declared). The `ci.yml` refusal row. MQ-24 filed. Red-first tests, device-free:
  - `HostBootStage` covers `HostBootError` exhaustively (unit test, no `_`);
  - calling `debug_callback` with fabricated severity/type bits and a state pointer moves the per-context class counters exactly as it moves the ledger's (red if per-context still counts by severity only).

**B5-1**
- **C1:** vocabulary, `IGNORE_CLASSES`, record grammar. G1.
- **C2:** scan move with per-binary module resolution, bodies, directives, cfg grammar. Census as a thin driver. G15, G2 (module), G14 (scan fixture).

**B5-2**
- **C3:** `Leg`, `LegCommand` (`status`/`output`/`spawn`), `ChildOutcome`, posture, pins, tool, `mode_from`. G2 (thread), G3, G11 (builder), G13.
- **C4:** `vk` and `app`: `boot` (+`BOOT_LOCK`, registration), `boot_unvalidated`, `expect_validation`, the D10 oracle, `class_of`, `class_of_stage`, `judge_run`, `require_validation`. G7, G9, G10 (tables), G17 (device members).

**B5-3**
- **C5:** runner `list`/`probe`/`run`, shepherd, judge, report, JSON reader, dirty check, `bins.txt`. G4, G11 (parity), G12, G14 (fixture tree), G16.
- **C6:** owner step on the msvc host: `leg probe --write-machine owner-rtx3060`; the measured file is committed.

**B5-4**
- **C7:** boyko_rhi_vulkan (§5, §7), tool locator, `known` rows for reds on the parent.
- **C8:** boyko_render the same. Owner: `leg run --only boyko_rhi_vulkan,boyko-render` and G8.

**B5-5**
- **C9:** boyko_app, joltab-origin: workers, drivers, children, pins, O6, `ENV_ALLOWED`, validation sites.
- **C10:** boyko_app, vkval-origin: Gate A (canary → `done_expecting_validation(1)`) and the shadow gate.

**B5-6**
- **C11:** prefix sweep over unheld plain sites; rule (a) on with `PREFIX_PENDING`.
- **C12:** rules (b)–(j) on, with floors pinned from this commit's counts.
- **C13:** the owner's full `leg run` (G6, G10 live, G14 real). `CLAUDE.md` (ignored suite: vocabulary, `leg run`, counts from `leg list`) and `tester.md` rewritten. `expect … payload-absent` rows are added only if F1 = no.

## 10. Gates

| Gate | At | Checks | Red-first run or named mutation |
|---|---|---|---|
| G1 record grammar | C1 | format→parse round-trip over details with `\n`, `\r`, `:`, `<-`, `#`, non-ASCII; malformed lines rejected | drop the CR/LF replacement → red; parse `<-` into the id → child fixture red |
| G2 identity | C2, C3 | `Leg::begin().id()` equals the test name at the default thread count and at `--test-threads=1`; a `mod gpu` fixture gets `gpu::name`, and the scan resolves the same | constant id → red; strip the module path → red |
| G3 mechanics | C3 | self-spawned fixtures: DONE; SKIP; early return → FAILED + UNMARKED; misspawn; unspawned; done-after-skip and double skip panic; `scrub_boyko` keeps `BOYKO_LEG_*`; reserved `env` panics; outcomes `Skipped(cap)`, `Panicked` (panic after BEGIN), `Crashed` (`std::process::abort`), `NeverBegan` (a filter matching nothing); `process::exit` after `done` keeps DONE | remove the Drop panic → red; `scrub_boyko` strips `BOYKO_LEG_*` → red; merge `Panicked` into `NeverBegan` → red |
| G4 judge | C5 | every row; the set fold (an allowed child class plus a forbidden one → FAIL); the attribution chain; dirtied tree; declared degrade; `allow cap` → exit 2; known: match → KNOWN-RED exit 0, pass → unexpected-pass, skip → unexpected-skip, other message → FAIL, missing test → stale-known; orphan: driver PASS without worker BEGIN → FAIL, driver SKIPPED → no row (W2); zero measured under an allow-everything file → exit 1 | count SKIPPED or KNOWN-RED as measured → red; "first child SKIP" semantics → the fold row red; orphan ignores driver verdict → the F1 = no fixture red |
| G5 census | C11, C12 | rules (a)–(j), floors and fixtures | (a) `wroker:`. (b) guard below `.expect(ENV_RUNG)`. (c) `eprintln!("SKIP x")` → red; controls `CTR_WAVES_SKIPPED` and "W3' SKIP semantic" green. (d) Leg removed from the vb_occ_split_gate driver; `use boyko_leg::Leg; Leg::begin()`; `boyko_leg::Leg::begin()` in a helper fn (d2). (e) `leg-env: BOYKO_DISABLE_VALIDATION=1`. (f) `InstanceConfig` token back in hzb_build_oracle_gate; `leg-validation: off -- findings: …` → red; vb_p1d's declaration stays green (W3). (g) `"--exact"` back in vb_occ_mixed. (h) **H1 re-introduced:** `VulkanContext::boot` in `R/common/mod.rs` called via `common::boot()` → red naming `common/mod.rs` (W4); a local `find_dxc`. (i) `app.run();` back in forward_mesh → red; `log_host_reachable.rs` and a `LegCommand::status()` driver in `vb_mesh.rs` stay green (W3). (j) `features = ["vk"]` dropped from boyko-render. |
| G6 bogus ICD (owner) | C13 | (a) `$env:VK_ICD_FILENAMES='C:\boyko-no-such-icd.json'` → exit 2, PROBE line names `device`. (b) same with `--no-probe` → exit 1; every device member FAIL forbidden-skip(`device`); the 24 scene tests report stage `VulkanDevice`. (c) variable unset → the probe passes | (b) vs (c) is the device difference |
| G7 class tables | C4 | table over every `BootError`, `VulkanError` and `HostBootStage` variant; a self-census asserts no `_ =>` in `class_of`/`class_of_stage` | add a `_ =>` → red; map `SingletonAlreadyBooted` to `device` → red |
| G8 m2 (owner) | B5-4 | posture off → `[m2-smoke] atlas_linear_filter_ok=…` and DEGRADE; posture on → no DEGRADE, clean | re-adding the early return does not compile; a bare `return` panics UNMARKED |
| G9 `judge_run` | C4 | fabricated worlds: `VulkanDevice(Boot(LoaderUnavailable))` → skip(`device`); `VulkanDevice(SingletonAlreadyBooted)` → panic; `Renderer` → panic; frames 0 → panic; neither → panic; both → panic; `(5)` → `Some` | drop the zero-frames check → red |
| G10 oracle | C4, C13 | `mode_from` table (runner id, `--test-threads=1`, `--test-threads 1`, `RUST_TEST_THREADS=1`, argv over env, default → Parallel). `oracle()` table: exclusive delta gating per class (errors, validation_warnings, general_errors each); exempt subtraction; `Expect::AtLeast` deaf → FAIL; parallel per-context only; process messages ignored in parallel; context alive → panic; `app::run` and `done_expecting_validation` in parallel → panic. Live arm (C13): Gate A canary → PASS `heard ≥ 1`; `cargo test -p boyko-render --test sync_validation` at default threads → test A PASS whether or not B's hazard fires | gate only `errors` → warnings row red; read the process delta in parallel mode → the A-with-B fixture red; drop the alive check → teardown-leak fixture green → red |
| G11 posture | C3, C5 | `apply` on a fabricated environment (strip, keep `VK_LAYER_PATH`, case-insensitive, set keys, keep `BOYKO_LEG_*`); runner and `worker_command` environments come from one function and have equal key sets | case-sensitive compare → red; a second builder → parity red |
| G12 no dirt | C5, C9 | dirty check in a temp repo (tracked modified → reported; untracked ignored); `vg_decidability_floor` resolver (no flag → `{out}`) | unconditional `FLOOR_DOC` write → red |
| G13 pins | C3 | parses the committed PINS.toml; every pin has crate/binary/test; reserved keys dropped; ≥ 30 env blocks | stop dropping → `[vb_mesh]` env holds `BOYKO_DISABLE_VALIDATION` → red |
| G14 membership | C2 (scan), C5 (`leg list`, fixture tree), C13 (real) | `#![cfg(feature="hwrt")]` gpu fixture listed `[hwrt]`; unparsed cfg → exit 2; real tree: `hwrt_blas_smoke` ×3, `spec_constant_smoke`, `f7_rt_cap` present | drop cfg-feature derivation → MISSING → red |
| G15 scan parity | C2 | census site counts identical before and after the move | drop the raw-string lexer state → counts differ |
| G16 shepherd (Windows) | C5 | a fixture member spawns a grandchild that writes its pid and sleeps 600 s; `leg-timeout: 3` → FAIL timeout, and `tasklist /FI "PID eq <pid>"` shows it gone | skip the self-assignment to the job → grandchild alive → red |
| G17 instance count | C4 (device members, owner) | `vk::boot` in an exclusive process: Δinstances == Δmessengers == 1; `app::run(windowed_smoke-shaped App)`: Δinstances == 1 | also increment in `boot_singleton` → App case reads 2 → red (O2) |

**`debug_assert!` invariants:**
- `judge_run`: `HostBootFailed` and `HostFramesPresented` never both present.
- `note_messenger_destroyed`'s existing assert.
- `Leg`: state `Open` on every `degrade`.
- `vk::boot`: registration slot count ≤ 8.
- Oracle: per-context class sums ≤ the matching exclusive-mode delta.

## 11. What it cannot claim

- **DONE means the body reached its end,** not that its assertions mean anything (m5's parity check, m3's empty-field case).
- **CPU tests without a Leg** keep libtest's "early return is ok".
- **Lib unit tests in `src/`** that boot a device (e.g. `device.rs`'s `mod tests`) are outside the leg; the scan prints their count at C12.
- **The raw entry-point list** is closed over today's engine (`VulkanContext::{boot, boot_singleton}`, App `.run()`). A new public boot path joins `boyko_leg_core`'s table in the same commit, by review.
- **Parallel mode does not gate the instance window** (`vkCreateInstance` / `vkDestroyInstance` messages, NULL user data). Leg runs are exclusive and do gate it.
- **Teardown after a `skip()`** is not judged.
- **Other oracle blind spots:**
  - `VulkanProbe` instances (no layer, declared);
  - sync validation, if dead on the box (G10's live arm records which);
  - image correctness (golden.ps1's job).
- **Machine identity** is the device-name string.
- **Linux:** 107 files are `#![cfg(windows)]`, and the shepherd kills only its direct child there.
- **Thread-name identity and the sequential join** under `--test-threads=1` are libtest implementation properties (#103681), pinned by G2.
- **Ledger appends** are assumed atomic for short writes; a torn line is red.
- **`cfg_attr` ignore prefixes** stay outside the vocabulary.
- **KNOWN-RED rows** are an honest record of an open finding, not a repair; the report lists them by finding ID.

## Owner decision

- **F1:** Is `assets/vg_corpus` fetched on owner-rtx3060 (with `scripts/fetch_corpus.ps1`) and kept there?
  - **Yes:** the corpus members must PASS; a missing corpus is FAIL.
  - **No:** C13 adds one `expect … payload-absent -- corpus not fetched on this box` row per corpus driver. Their workers are exempt from orphan-worker (W2), and the run can be green.
- **The validation posture is not an owner decision.** C6's probe measures it on the msvc host.

**Relevant paths:**
- joltab:
  - `D:/wt/joltab/tests/ignore_reasons_census.rs`
  - `D:/wt/joltab/Cargo.toml`
  - `D:/wt/joltab/crates/boyko_app/src/runner.rs`
  - `D:/wt/joltab/crates/boyko_app/src/host.rs`
  - `D:/wt/joltab/crates/boyko_render/tests/common/mod.rs`
  - `D:/wt/joltab/crates/boyko_render/tests/sync_validation.rs`
  - `D:/wt/joltab/crates/boyko_rhi_vulkan/tests/compute.rs`
  - `D:/wt/joltab/crates/boyko_rhi_vulkan/tests/ddgi_probe_gi_cost.rs`
  - `D:/wt/joltab/crates/boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs`
  - `D:/wt/joltab/crates/boyko_rhi_vulkan/tests/sdf_gbuffer_hybrid.rs`
  - `D:/wt/joltab/crates/boyko_app/tests/log_host_reachable.rs`
  - `D:/wt/joltab/.github/workflows/ci.yml`
  - `D:/wt/joltab/goldens/PINS.toml`
- vkval:
  - `D:/wt/vkval/crates/boyko_rhi_vulkan/src/debug.rs`
  - `D:/wt/vkval/crates/boyko_rhi_vulkan/src/device.rs`
  - `D:/wt/vkval/crates/boyko_rhi_vulkan/src/error.rs`
- unification plan:
  - `D:/claude/BoykoEngine/docs/unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md`
  - `D:/claude/BoykoEngine/docs/unification/UNIFIED-SYSTEM-PLAN-03-GATES.md`
  - `D:/claude/BoykoEngine/docs/unification/UNIFIED-SYSTEM-PLAN-04-INTEGRATION.md`

Sources:
- [rust-lang/rust#103681: libtest runs every test on its own named thread](https://github.com/rust-lang/rust/pull/103681)
- [rust-lang/rust#68007: runtime ignore; an early return reads as ok](https://github.com/rust-lang/rust/issues/68007)
- [deqp-runner runner_results.rs: Missing, UnexpectedImprovement, baseline](https://docs.rs/crate/deqp-runner/latest/source/src/runner_results.rs)
- [Piglit results.py: default NOTRUN](https://raw.githubusercontent.com/Igalia/piglit/main/framework/results.py)
- [Kernel DRM CI: per-hardware fails/skips/flakes expectation files](https://docs.kernel.org/gpu/automated_testing.html)