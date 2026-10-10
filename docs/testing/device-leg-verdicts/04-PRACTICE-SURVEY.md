# Research: making a runtime test skip visible instead of a pass, and running GPU/device tests on machines that may lack the device

Scope note: the brief said "WEB ONLY for this part", so I did not read `D:/wt/joltab`, `D:/wt/vkval` or the devleg report. Every claim below comes from a fetched web page or source file listed under Sources.

## TL;DR
- **Rust's libtest cannot report a skip at runtime.** An early return shows up as `ok`. The request, rust-lang/rust#68007, has been open since 2020-01-08. Rust's testing team (T-testing-devex) wants custom test harnesses to carry this, not libtest. Two harness crates already have it: libtest-mimic 0.8.2 (2026-03-16) with `Trial::ignorable_test` / `Completion::Ignored{reason}`, and the experimental libtest2-harness 0.0.3 with `TestContext::ignore_for(reason)`. cargo's own test suite instead decides at **macro-expansion time**, and turns a missing tool into a **panic on CI**. [1][2][3][4][5][6][7]
- **Three ways to report a skip:**
  1. **Its own status in the harness protocol.** Examples: JUnit `ABORTED`, GoogleTest `SKIPPED`, Go `skip`, the Vulkan conformance suite's `NotSupported`, TAP `# SKIP`, kselftest exit code 4, Automake exit code 77.
  2. **A marker line on stdout that the runner parses.** Piglit prints `PIGLIT: {"result": "skip"}` and exits 0, and a test with no marker line stays `NOTRUN`. Dawn logs `Test unsupported:` / `Test suppressed:`. wgpu logs `TEST RESULT: SKIPPED`.
  3. **Deciding at list time.** The skip is decided when tests are enumerated, so it shows in the test list or the test name. Examples: the cargo test macro, wgpu's per-adapter test names, Dawn's per-adapter instantiation.
- **No "nothing ran" guard I examined catches "everything skipped at runtime".** nextest `--no-tests`, the JUnit `--fail-if-no-tests` flag, pytest exit code 5 and libtest-mimic's exit code all count tests *discovered or selected*. GoogleTest's documentation says outright that a test skipped via `GTEST_SKIP` still counts as "selected". What does catch missing results is a **known expected set**: deqp-runner turns every case-list entry with no result into `Missing`, which fails the run. Piglit's "no marker ⇒ NOTRUN" works the same way. [8][9][10][11][12][13]
- **Per-machine expectation files key on test identity × machine, not on skip class.** Examples: deqp-runner's `--baseline` / `--skips` / `--flakes`, the kernel's DRM CI `{driver}-{hw}-fails/skips/flakes.txt`, the Vulkan conformance suite's waiver file keyed by vendor/device, and wgpu's `FailureCase` keyed by backend/vendor/adapter/driver. An allow-list keyed on *skip class* per binary did not appear in any source I examined. [13][14][15][16]
- **The most common failure is losing the status at a translation boundary.** Examples:
  - JUnit's `aborted` becomes a Gradle `<skipped>`, then a reporting action shows it as a success.
  - A GoogleTest skip in environment `SetUp` prints `[ PASSED ]` and writes XML `result="completed"`.
  - Go's test2json `skip` also means "package contained no tests".
  - nextest's JUnit output leaves skipped tests out by default.

  [17][18][19][20][21]

## Approaches in the engines and tools examined

### Rust libtest (std)
- **Approach**: none at runtime. `#[ignore = "reason"]` is fixed at compile time. #68007 (open, labels A-libtest / T-dev-tools / T-lang, on the testing-devex backlog) says an early return "misleadingly reports tests as 'ok' rather than 'ignored'". [1]
- **Direction**: the 2024-07-26 "Skippable tests" pre-RFC debated a "magic return code" and how to carry the skip reason, then auto-closed with no resolution. Responders, per the thread summary, lean toward freezing libtest and moving work to custom harnesses. epage (2023-06): "libtest is static. If you `#[ignore]` a test, that is it." The libtest-JSON eRFC 3558 (2024-01-18) lists "Static and dynamic test skipping" as an evaluation criterion. [2][3][22]
- **Output capture**: "Usually the output is captured, and only displayed if the test fails." `--show-output` prints the output of passing tests. So a marker printed by a passing test is invisible by default. [23]

### libtest-mimic 0.8.2 / libtest2-harness 0.0.3
- **libtest-mimic**:
  - `Trial::ignorable_test` returns `Result<Completion, Failed>`, with `Completion::Ignored { reason: Option<String> }`. Added in 0.8.2 (2026-03-16, PR #58).
  - `Conclusion` has `num_ignored`, but `exit()` is "0 if all tests have passed, 101 if there have been failures". The ignored count does not affect the exit code.
  - `with_ignored_flag` sets ignore at construction time. [4][5]
- **libtest2-harness**: `TestContext::ignore()` / `ignore_for(reason)` return `Err(RunError::ignore…)` **unless `run_ignored`**. So under `--ignored` the call returns `Ok` and the test goes on. This is a runtime-evaluated `#[ignore]` ("skip unless explicitly asked"), not a capability probe. [6]

### cargo's own test suite (`cargo-test-macro`)
- **Approach**: the probe runs at macro expansion. For `requires = "cmd"` the macro calls `has_command` and emits `#[ignore = "<cmd> not installed"]`.
- **Escalation**: `has_command` panics when the build had `CARGO_TEST_REQUIRE_EXTERNAL_TOOLS` set. `check_command` panics when `is_ci()` is true (`CI` or `TF_BUILD` via `option_env!`).
- **Known cost**: their own comment says `option_env!` needs a macro rebuild to pick up changes, and prefers `tracked_env` once it is stable.
- **Trade-off**: the skip shows as a real `ignored` in libtest, but the probe result is frozen at build time. [7]

### cargo-nextest
- **Harness protocol**: `--list --format terse`, `--list … --ignored`, `<name> --nocapture --exact`. The documented protocol has no channel for a test to report a skip. I found no reliable information on whether nextest shows libtest-mimic's runtime `Completion::Ignored`. [24]
- **What "skipped" means**: tests that were not selected (filters or ignored status), e.g. "14 tests run: 14 passed, 177 skipped". [8]
- **`--no-tests`**: `auto` (default, resolves to fail) / `fail` (exit code 4, `NO_TESTS_RUN`) / `warn` / `pass`. It fires on zero *selected* tests. [8][25]
- **JUnit**: `junit.report-skipped` = `none` (default, "keeps machine-readable output stable") / `ignored` / `all`, added in 0.9.143 (2026-08-04). `store-success-output` defaults to false, so a marker from a passing test is not in the XML by default. [9][26]
- **Per-test config**: `[[profile.*.overrides]]` selected by `filter` (a filterset) and/or `platform`. They can override `retries`, `flaky-result` (pass|fail), `slow-timeout` (`on-timeout` fail|pass), `test-group`, `threads-required`, `run-extra-args`, and `junit.*`. Setup scripts and wrapper scripts exist (0.9.98, 0.9.131). [9][26]
- **Runtime skip request**: #1532 (2024-05-31) asked for skip-if from a setup script. It is closed, and the fetched page showed no maintainer response. [27]

### Go `testing`
- `t.Skip` = `Log` + `SkipNow`. `SkipNow` "marks the test as having been skipped and stops its execution by calling runtime.Goexit()". `t.Skipped()` exists. `-v` prints `--- SKIP`. [28]
- test2json action `"skip"`: "the test was skipped **or the package contained no tests**". Two events share one status. [21]
- Default (non-`-v`) output hides skips. #34306, a flag to show them, was closed without being implemented. #25951 (`-unskip`: run skipped tests and fail if they pass) is on Proposal-Hold. [29][30]
- **CI escalation precedent**: `internal/testenv.MustHaveSource` returns without skipping when `Builder()` (`GO_BUILDER_NAME`) is non-empty. Its comment: "The builders have the source tree available, and if they don't the tests should error out." [31]

### pytest
- `pytest.skip(reason)` works imperatively at runtime. `importorskip` exists. `-rxXs` lists skip reasons in the summary.
- `xfail(strict=True)` / `xfail_strict` turns an unexpected pass (XPASS) into a failure.
- Exit code 5 = "No tests were collected". The docs give no separate code for "all skipped". [10][32]

### JUnit 5/6 (Jupiter / Platform)
- A failed assumption throws `TestAbortedException`, and the test is **aborted**, not failed. `TestExecutionResult.Status` = `SUCCESSFUL` / `ABORTED` ("started but not finished") / `FAILED`. Disabled or conditional tests are **skipped**, i.e. never started. [33][34]
- The Console Launcher reports separate `tests skipped` and `tests aborted` counters. Exit code 1 on failures, 2 with `--fail-if-no-tests` when **no tests are found**. No option to fail on aborted tests is documented. [11]
- **Lossy consumers**:
  - Gradle maps aborted to `<skipped>`, while Ant writes `<aborted>`. publish-unit-test-result-action #673 (2025-06-09, open) shows aborted tests as successful.
  - Gradle #26198 (2023-08-28, closed as not planned): an assumption in `@BeforeAll` gives "no tests were run" and a successful build. [17][18]

### GoogleTest
- `GTEST_SKIP()` works in a test or in `SetUp()` of `::testing::Test` or `::testing::Environment`. XML: `<testcase status="run" result="skipped">` with a `<skipped message=…>` child. `<testsuite skipped="N">` exists, but the expected root `<testsuites>` in gtest's own XML test has no `skipped` attribute. [12][35]
- `--gtest_fail_if_no_test_selected`: "A test is considered selected if it begins to run, even if it is later skipped via `GTEST_SKIP`." So all-skipped passes this guard. `--gtest_fail_if_no_test_linked` checks only that test cases are linked into the binary. [12]
- #4653 (2024-11-05, open): a `GTEST_SKIP` in `Environment::SetUp` produces `[ PASSED ] 2 tests` and XML `result="completed"`. [19]

### TAP / kselftest / Automake (distinct status through protocol or exit code)
- TAP14: `ok N … # SKIP <why>` per test, and `1..0 # skip <why>` for a whole plan. Harnesses must not count a skipped point as a failure. [36]
- kselftest: `KSFT_PASS 0, FAIL 1, XFAIL 2, XPASS 3, SKIP 4`. `ksft_exit_skip` prints `1..0 # SKIP` or `ok N # SKIP` and then calls `exit(KSFT_SKIP)`. [37]
- Automake: exit code 77 = skip, 99 = hard error. `XFAIL_TESTS` inverts results, "with the provision that skips and hard errors remain untouched". [38]

### Piglit (the marker-line precedent)
- `piglit_report_result` prints `PIGLIT: {"result": "<status>" }`. It exits 0 for PASS, SKIP and WARN, and 1 for FAIL. [39]
- The runner parses lines that begin with `PIGLIT:` as JSON. **The default result is `status.NOTRUN`**, so a clean exit 0 with no marker line is not a pass. A non-zero return code turns PASS into WARN, and anything else into FAIL. Crash = return code < 0 (on Windows also == 3). [40][41][42]

### wgpu (Rust, GPU)
- **Pipeline**: `cargo xtask test` first builds `.gpuconfig` by running `wgpu-info`'s `generate_gpuconfig_report` test, then runs nextest. The test binary's `main` fails hard if `.gpuconfig` is missing: "Failed to read .gpuconfig, did you run the tests via `cargo xtask test`?". `WGPU_GPU_TESTS_USE_NOOP_BACKEND=1` switches to the noop backend. [43][44]
- **Per-adapter expansion, decided at list time**: each test becomes one libtest-mimic `Trial::test` per adapter, named `"[{running_msg}] [{backend}/{device}/{idx}] {name}"`. `running_msg` is one of `Executed`, `Executed Failure: …`, `Skipped Failure: …`, or `Unsupported: <missing features/limits/flags>`. With zero adapters, the `flat_map` produces zero GPU trials. xtask counts GPUs but does not check a minimum. [43][44]
- **Skip path**: `log::info!("TEST RESULT: SKIPPED"); return;`, so the harness reports it as **passed**. The skip is visible only through the name prefix. [45]
- **Expectations**: `.skip(FailureCase)` vs `.expect_fail(FailureCase)`. `FailureCase{backends, vendor, adapter, driver, reasons, behavior}` uses substring matching. `FailureBehavior::AssertFailure`: "If the test passes, the test harness will panic". `Ignore` is for flakes. [46][47]
- **CI**: Windows installs WARP and Mesa; Linux installs Mesa. The job prints `cat .gpuconfig`. `LVP_POISON_MEMORY=true` in Linux Vulkan CI. [48][49]

### Dawn (C++, GPU, GoogleTest)
- `DAWN_TEST_UNSUPPORTED_IF(cond)`: "requires a feature or a toggle". `DAWN_SUPPRESS_TEST_IF(cond)`: "failing on a specific HW / backend / OS combination", and can be turned off with `--run-suppressed-tests`. Both log `"Test " type ": " #cond` and then call `GTEST_SKIP()`. So the skip class goes into the log line and the real status is `SKIPPED`. [50]
- `DAWN_INSTANTIATE_TEST` creates tests only for available adapters and adds `GTEST_ALLOW_UNINSTANTIATED_PARAMETERIZED_TEST`. A missing adapter means the test does not exist; it is not recorded as a pass. [50]

### Vulkan CTS (the Khronos conformance suite)
- `qpTestResult` includes `NOT_SUPPORTED` ("Implementation does not support functionality needed by this test case"), separate from `FAIL`, `RESOURCE_ERROR`, `INTERNAL_ERROR`, `CRASH`, `TIMEOUT`, `WAIVER`, `DEVICE_LOST`, `CAPABILITY_WARNING`, and others. [51]
- Conformance allows `Pass, NotSupported, QualityWarning, CompatibilityWarning, Waiver`. Results are the `StatusCode` attribute in `TestResults.qpa`. The `mustpass/main/vk-default.txt` list defines the expected set. `--deqp-waiver-file` is keyed by vendor/device. [14]
- A framework function `checkMandatoryFeatures` logs "Mandatory feature X not supported" and returns false. So a *mandatory* capability is checked separately from each test's own NotSupported. I did not verify which test case calls it. [52]

### Mesa deqp-runner 0.23.3
- **Mapping**: `NotSupported→Skip`. `QualityWarning/CompatibilityWarning/Waiver→Warn`. `Pending/ResourceError/InternalError→Fail`. `DeviceLost→Crash`. gtest `SKIPPED→Skip`. [53][54]
- `RunnerStatus`: `Pass, Fail, Skip, Crash, Flake, KnownFlake, Warn, Missing, ExpectedFail, UnexpectedImprovement(TestStatus), Timeout`.
- `is_success`: true for Pass/**Skip**/Warn/Flake/KnownFlake/ExpectedFail. False for Fail/Crash/**Missing**/**UnexpectedImprovement**/Timeout. Any false exits with code 1. [13][55]
- **Baseline** = expected non-pass statuses per test:
  - Pass against a baseline of Fail/Crash/Missing/Timeout becomes `UnexpectedImprovement(Pass)`, which fails the run.
  - **Skip against a baseline of Fail/Crash/Missing/Timeout becomes `UnexpectedImprovement(Skip)`**, which also fails the run.
  - Skip with no baseline entry stays Skip, which counts as a success. [13]
- `Missing` is produced for every case-list test when results cannot be parsed. `--skips` excludes tests from the run. `--flakes` are regexes. [55][56]
- **DRM CI (kernel docs)**: `-fails.txt` "Lists the known failures for a given driver on a specific hardware revision". `-skips.txt` "Lists the tests that won't be run…" (hangs, OOM, too slow). Flake entries must carry a bug-report link, board, kernel version, IGT version and failure rate. [15]

### Bevy (rendering CI)
- `example-run.yml`: macOS-14 uses Metal. Ubuntu uses `mesa-vulkan-drivers` from the kisak PPA plus `xvfb-run`. Windows uses `WGPU_BACKEND=dx12`. Examples run under `CI_TESTING_CONFIG` `.ron` files, and screenshots go to PixelEagle. I did not verify which adapter the Windows runner actually gets. [57]
- `example-showcase` has **three** outcome classes: `"total / passed / failed / no screenshot"`, with `successes` / `failures` / `no_screenshots` report files. A run that exits cleanly but produces no screenshot gets its own status. [58]

## Comparison table

| System | How a skip reaches the runner | Separate status? | Reason carried | "Nothing ran" guard | Catches all-skipped-at-runtime? | Per-machine expectations |
|---|---|---|---|---|---|---|
| libtest (std) | none (early return = `ok`) | no (compile-time `#[ignore]` only) | `#[ignore = ".."]` | none | no | no |
| libtest-mimic 0.8.2 | `Completion::Ignored{reason}` | yes (`num_ignored`) | yes | exit code ignores ignored count | no | no |
| cargo test macro | decided at macro expansion → `#[ignore]` | yes (ignored) | yes | panics on CI or with REQUIRE_EXTERNAL_TOOLS | n/a (fails instead) | CI env var |
| nextest | no skip channel in protocol | "skipped" = not selected | — | `--no-tests` (zero selected) | no | overrides by `platform` / filter |
| Go | `t.Skip` (Goexit) | yes (`skip`, conflated in test2json) | log text | none | no | testenv `Builder()` |
| pytest | `pytest.skip` | yes | `-rs` | exit code 5 (none collected) | no | xfail strict |
| JUnit 5/6 | assumption → exception | yes (ABORTED ≠ skipped) | exception message | `--fail-if-no-tests` (none found) | no | — |
| GoogleTest | `GTEST_SKIP` | yes (`result="skipped"`) | message | `fail_if_no_test_selected` counts skips as selected | no (stated) | — |
| TAP / kselftest / Automake | `# SKIP` / exit 4 / exit 77 | yes | text after SKIP | `1..0 # SKIP` plan | — | Automake XFAIL_TESTS |
| Piglit | `PIGLIT:` JSON line, exit 0 | yes | JSON | default NOTRUN | missing marker ≠ pass | — |
| wgpu | log line + name prefix at list time | no (reported pass) | `Unsupported: …` in name | `.gpuconfig` must exist | no minimum-adapter check found | `FailureCase` per backend/vendor/driver |
| Dawn | `GTEST_SKIP` + class log | yes | `unsupported` / `suppressed` | per-adapter instantiation | via gtest only | suppress + `--run-suppressed-tests` |
| Vulkan CTS | `NotSupported` result | yes | log | mustpass list | separate mandatory-feature check | waiver file (vendor/device) |
| deqp-runner | parses CTS / gtest / piglit | Skip = success | CSV | `Missing` fails | only if baseline says Fail etc. | baseline / skips / flakes files |

## Key design patterns
1. **Separate status in the harness protocol** (JUnit, gtest, Go, pytest, CTS, TAP, exit-code conventions). The skip survives only if every consumer keeps it: the harness, the runner, the JUnit writer and the dashboard. The cases in [17][18][19][21][26] are each one hop that drops it.
2. **Marker line parsed by the runner** (Piglit, TAP; Dawn and wgpu as diagnostics). This works over a harness that has no skip status. Piglit's version holds up because **the marker is the result of record and its absence is `NOTRUN`**. In wgpu and Dawn the log line is diagnostic only; the status of record is a pass (wgpu) or `SKIPPED` (Dawn).
3. **Decide at list time** (cargo macro, wgpu name prefix, Dawn instantiation). The decision shows in `--list` and in the counts. The costs: the probe runs at build or list time (cargo's own `option_env!` staleness note), and in wgpu's case it depends on a pre-generated device report.
4. **Stricter rule where the capability is guaranteed** ("skip locally, fail where it must exist"): cargo (`CI`, `TF_BUILD`, `CARGO_TEST_REQUIRE_EXTERNAL_TOOLS`), Go testenv on builders, Dawn `--run-suppressed-tests`, and libtest2 `ignore()` returning `Ok` under `--ignored`.
5. **Expectation files keyed by test × machine**: deqp-runner baseline, DRM xfails, CTS waiver, wgpu `FailureCase`. All of them fail the run on an **unexpected improvement** (deqp `UnexpectedImprovement`, wgpu `AssertFailure`, pytest strict xfail, Go #25951 proposal), so stale expectations cannot build up silently.
6. **Stopping "all skipped" from reading as green**:
   - Count guards (nextest, JUnit, gtest, pytest) count tests discovered or selected, not executed.
   - What works in the sources: an **expected set with per-entry results** (deqp `Missing`, CTS mustpass), a **default of not-run** (Piglit), a **separate category for "ran but produced nothing observable"** (Bevy `no screenshot`), a **mandatory-capability check separate from the per-test NotSupported** (CTS), and a **device preflight that is a hard error** (wgpu `.gpuconfig`).

## Pitfalls and mistakes
- An early return is indistinguishable from a pass in libtest (#68007).
- A skip in global setup gets reported as a pass: GoogleTest #4653, Gradle #26198.
- The status is lost at format boundaries: JUnit aborted→skipped→"successful" (#673); test2json `skip` also means "no tests"; nextest JUnit omits skipped tests by default; gtest's root `<testsuites>` has no `skipped` count.
- "Fail if nothing ran" flags measure selection, not execution. GoogleTest documents this explicitly.
- A runner that counts Skip as success (deqp-runner `is_success`) lets a mass NotSupported pass unless the expectation file names those tests.
- By default a marker printed by a passing test is swallowed by libtest's capture and by nextest's JUnit output (`--show-output`, `store-success-output`).
- Compile-time probes go stale without a rebuild (cargo's comment).
- Skips outlive their cause. This is the motivation for Go #25951, pytest strict xfail, deqp `UnexpectedImprovement`, and DRM's "remove lines from fails.txt".

## Relevant academic works
None searched or found for runtime-skip accounting. All sources are tool documentation, source code and issue trackers.

## How this maps to boyko-engine
- **Proposal (a), a `BOYKO-SKIP` marker line**, matches pattern 2. Piglit is the precedent where the marker is the result of record and has a not-run default. TAP `# SKIP` is a standardised text form. Dawn shows a class name inside the log line. Under libtest the marker is captured unless the run uses `--no-capture` or `--show-output`.
- **Proposal (b), a class allow-list per binary, fail otherwise**: the closest precedents key on test identity × machine (deqp baseline, DRM xfails, CTS waiver, wgpu `FailureCase`). I found none keyed on skip class. deqp-runner's `Missing` and `UnexpectedImprovement(Skip)` are the nearest mechanisms that turn an unexpected skip or missing result into a failure.
- **Proposal (c), the reason vocabulary with a `worker:` class**: Dawn separates `unsupported` from `suppressed`, with a flag to override the latter. The CTS separates NotSupported from Waiver and ResourceError. libtest2's `ignore()` ("skip unless explicitly asked; proceed when asked") has the same shape as the worker driver-env-var case.
- **Constraints**:
  - std libtest has no runtime skip status. Inside it, the only options are list-time (`#[ignore]`, a cargo-style probe at compile time) or out-of-band (a marker).
  - libtest-mimic 0.8.2 and libtest2 have an in-harness runtime ignore but need custom-harness targets. How nextest handles a runtime-ignored libtest-mimic test: no reliable information found.

## Open questions for the architect
- Is the marker the **result of record** (Piglit: no marker and no pass proof ⇒ not-run), or a diagnostic next to libtest's `ok` (wgpu, Dawn)?
- Should an absent device count as a skip or as a failure on the device leg? Precedents for failure: cargo on CI, Go builders, wgpu's missing `.gpuconfig`.
- Does the allow-list key on class, on test identity, or on both? Should it also fail on a skip that is allowed but did not occur (the `UnexpectedImprovement` analogue)?
- How does the runner get stdout from passing tests, given `--show-output` / `--no-capture` in libtest and `store-success-output` in nextest?
- Does the leg need an expected-set check (deqp `Missing`) so that a binary producing zero markers and zero results cannot pass?

## Sources
[1] https://github.com/rust-lang/rust/issues/68007 — runtime-ignore request; early return reads as ok
[2] https://internals.rust-lang.org/t/skippable-tests/21260 — 2024 pre-RFC, direction toward custom harnesses
[3] https://epage.github.io/blog/2023/06/iterating-on-test/ — "libtest is static"; cargo's compile-time workaround
[4] https://docs.rs/libtest-mimic/latest/libtest_mimic/struct.Trial.html , https://docs.rs/libtest-mimic/latest/libtest_mimic/enum.Completion.html — `ignorable_test`, `Completion::Ignored`
[5] https://docs.rs/libtest-mimic/latest/libtest_mimic/struct.Conclusion.html , https://github.com/LukasKalbertodt/libtest-mimic/blob/master/CHANGELOG.md — exit code ignores ignored count; 0.8.2 date
[6] https://docs.rs/libtest2-harness/latest/src/libtest2_harness/context.rs.html — `ignore` / `ignore_for` semantics
[7] https://raw.githubusercontent.com/rust-lang/cargo/master/crates/cargo-test-macro/src/lib.rs — expansion-time probe, CI panic
[8] https://nexte.st/docs/running/ — skipped = not selected; `--no-tests`; exit code 4
[9] https://nexte.st/docs/configuration/reference/ — report-skipped, overrides, flaky-result
[10] https://docs.pytest.org/en/stable/reference/exit-codes.html — exit codes
[11] https://docs.junit.org/6.1.2/running-tests/console-launcher.html — skipped/aborted counters, `--fail-if-no-tests`
[12] https://google.github.io/googletest/advanced.html — GTEST_SKIP, fail_if_no_test_selected counts skipped as selected
[13] https://docs.rs/crate/deqp-runner/latest/source/src/runner_results.rs — RunnerStatus, with_baseline, is_success
[14] https://github.com/KhronosGroup/VK-GL-CTS/blob/main/external/vulkancts/README.md — allowed results, mustpass, waiver file
[15] https://docs.kernel.org/gpu/automated_testing.html — DRM CI fails/skips/flakes files
[16] https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/tests/src/expectations.rs — FailureCase / FailureBehavior
[17] https://github.com/EnricoMi/publish-unit-test-result-action/issues/673 — aborted tests displayed as successful
[18] https://github.com/gradle/gradle/issues/26198 — assumption in @BeforeAll → build successful
[19] https://github.com/google/googletest/issues/4653 — Environment SetUp skip → PASSED
[20] https://github.com/nextest-rs/nextest/issues/885 — skipped tests missing from JUnit
[21] https://pkg.go.dev/cmd/test2json — `skip` action conflation
[22] https://rust-lang.github.io/rfcs/3558-libtest-json.html — eRFC, "dynamic test skipping" criterion
[23] https://doc.rust-lang.org/rustc/tests/index.html — output capture, `--show-output`
[24] https://nexte.st/docs/design/custom-test-harnesses/ — harness protocol
[25] https://nexte.st/changelog/ — report-skipped 0.9.143, wrapper/setup scripts
[26] https://nexte.st/docs/machine-readable/junit/ — JUnit representation
[27] https://github.com/nextest-rs/nextest/issues/1532 — conditional skip request
[28] https://pkg.go.dev/testing — Skip / SkipNow
[29] https://github.com/golang/go/issues/34306 — skips hidden without -v
[30] https://github.com/golang/go/issues/25951 — `-unskip` proposal
[31] https://raw.githubusercontent.com/golang/go/master/src/internal/testenv/testenv.go — builders do not skip MustHaveSource
[32] https://docs.pytest.org/en/stable/how-to/skipping.html — skip / xfail / strict
[33] https://docs.junit.org/6.0.3/writing-tests/assumptions.html — assumptions abort
[34] https://docs.junit.org/current/api/org.junit.platform.engine/org/junit/platform/engine/TestExecutionResult.Status.html — SUCCESSFUL / ABORTED / FAILED
[35] https://raw.githubusercontent.com/google/googletest/main/googletest/test/gtest_xml_output_unittest.py — skipped XML form
[36] https://testanything.org/tap-version-14-specification.html — `# SKIP`, `1..0 # skip`
[37] https://raw.githubusercontent.com/torvalds/linux/master/tools/testing/selftests/kselftest.h — KSFT_SKIP 4
[38] https://www.gnu.org/software/automake/manual/html_node/Scripts_002dbased-Testsuites.html — exit codes 77 / 99
[39] https://raw.githubusercontent.com/Igalia/piglit/main/tests/util/piglit-util.c — PIGLIT marker line
[40] https://raw.githubusercontent.com/Igalia/piglit/main/framework/test/piglit_test.py — marker parsing
[41] https://raw.githubusercontent.com/Igalia/piglit/main/framework/test/base.py — return-code mapping
[42] https://raw.githubusercontent.com/Igalia/piglit/main/framework/results.py — default NOTRUN
[43] https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/tests/src/native.rs — per-adapter names, `.gpuconfig` hard error
[44] https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/xtask/src/test.rs — gpuconfig generation
[45] https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/tests/src/run.rs — "TEST RESULT: SKIPPED" then return
[46] https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/tests/src/params.rs — skip vs expect_fail, running_msg
[47] https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/tests/src/config.rs — GpuTestConfiguration
[48] https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/.github/workflows/ci.yml — WARP / Mesa install, `cat .gpuconfig`
[49] https://github.com/gfx-rs/wgpu/blob/trunk/docs/testing.md — testing overview, LVP_POISON_MEMORY
[50] https://raw.githubusercontent.com/google/dawn/main/src/dawn/tests/DawnTest.h — UNSUPPORTED / SUPPRESS macros, per-adapter instantiation
[51] https://raw.githubusercontent.com/KhronosGroup/VK-GL-CTS/main/framework/qphelper/qpTestLog.h — qpTestResult
[52] https://github.com/KhronosGroup/VK-GL-CTS/blob/3d0edd827ae15d134c12e0de11239d7b669a6547/external/vulkancts/framework/vulkan/vkMandatoryFeatures.inl — mandatory-feature check
[53] https://docs.rs/crate/deqp-runner/latest/source/src/parse_deqp.rs — status mapping
[54] https://docs.rs/crate/deqp-runner/latest/source/src/gtest_command.rs — gtest SKIPPED → Skip
[55] https://docs.rs/crate/deqp-runner/latest/source/src/lib.rs — Missing, exit(1)
[56] https://docs.rs/crate/deqp-runner/latest — README, baseline / skips / flakes flags
[57] https://raw.githubusercontent.com/bevyengine/bevy/main/.github/workflows/example-run.yml — Bevy rendering CI
[58] https://raw.githubusercontent.com/bevyengine/bevy/main/tools/example-showcase/src/main.rs — passed / failed / no-screenshot categories