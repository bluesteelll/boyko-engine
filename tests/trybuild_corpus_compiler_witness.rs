//! **The compiler this repository's `.stderr` corpus was blessed against, pinned like the shaders'
//! `dxc`.**
//!
//! # Why this file exists
//!
//! `crates/boyko_ecs/tests/*_compile_fail/` and its siblings hold **byte-exact rustc output** for
//! **90 fixtures** (MEASURED by the walk below) across ten `trybuild` harnesses. That output is
//! compiler prose, and it changes when the compiler changes — for reasons that have nothing to do
//! with this engine.
//!
//! MEASURED at profiling rung 11: **23 fixtures across 7 suites were red at `3163078f`** under the
//! toolchain `CLAUDE.md` mandates, proved by stashing the entire working tree and re-running them.
//! They had drifted, the drift was invisible, and the rung-10 certification that reported
//! *"543 targets ok, 0 failed"* did not cover them. Nothing in the repository said which compiler
//! the corpus was true of, so nothing could notice.
//!
//! **The precedent for the fix is already here.** Every committed `.spv` in this tree is byte-gated
//! against a **frozen `dxc` recipe** written into the shader's own header, precisely so a compiler
//! change cannot silently redefine what the artifact means. A `.stderr` corpus is the same shape
//! with a different compiler, and it had no such freeze. This is that freeze.
//!
//! # A second thing this catches, which is why it reads the compiler rather than a config file
//!
//! On the machine this was written on, a **chocolatey `cargo`/`rustc` 1.95.0** at
//! `C:\ProgramData\chocolatey\bin` shadowed `~/.cargo/bin` on `PATH`. It produced a phantom
//! `E0133` in `boyko_diag::clock`, a wall of `link.exe` failures against an MSVC sysroot, and — the
//! dangerous part — it can bless `.stderr` files that the mandated toolchain then rejects. A
//! `rust-toolchain.toml` would NOT have caught that: a standalone (non-rustup) `cargo.exe` ignores
//! it entirely. Reading the compiler's own version string does.
//!
//! # What this gate cannot claim
//!
//! It witnesses the `rustc` on `PATH`, which is the one cargo drives **when cargo is the rustup
//! proxy** — the normal case, and the case every `TRYBUILD=overwrite` bless runs under. It cannot
//! prove that the compiler which built *this test binary* is the same one, because cargo exports no
//! `RUSTC` to a test process and this workspace has **no `build.rs` anywhere** to capture it at
//! build time (rung 14's `BOYKO_PROFILE` axis is scheduled to add the first). If the two ever
//! diverge, this gate reports the one that would do the blessing, which is the one that matters for
//! the corpus.
//!
//! It also cannot claim the fixtures are *correct* — only that the compiler whose prose they pin is
//! the compiler that is running. `trybuild` itself makes the byte-exactness claim.
//!
//! # When this fires
//!
//! The corpus is no longer certified. Re-bless and commit the diff **deliberately**, reviewing it
//! for content changes rather than rendering changes:
//!
//! ```text
//! $env:TRYBUILD = "overwrite"
//! cargo test --workspace --no-fail-fast -- compile_fail ui
//! ```
//!
//! then update [`BLESSED_RUSTC`] in the same commit.

use std::process::Command;

/// The exact `rustc --version` string the committed `.stderr` corpus was blessed against.
///
/// Updated **only** together with a re-bless, in the same commit, so the pair cannot diverge.
/// History:
///
/// | Blessed | Rung | Why |
/// |---|---|---|
/// | `rustc 1.97.1 (8bab26f4f 2026-07-14)` | 11 | first freeze; 23 inherited-drift fixtures plus the impl-count re-render this rung caused |
const BLESSED_RUSTC: &str = "rustc 1.97.1 (8bab26f4f 2026-07-14)";

/// Fixtures whose bytes this freeze speaks for — a lower bound, MEASURED at rung 11.
///
/// Carried as a number rather than a list because the claim is about *scale*: this is not one file
/// somebody will notice, it is **ninety** spread over ten harnesses, which is exactly why 23 of
/// them could drift unseen. The assertion below is `>=`, so adding fixtures never reds it.
///
/// The first draft of this constant said 24 — the number of files that rung 11's own diff
/// happened to touch. The walk measured 90. A count taken from a diff is a count of what changed,
/// not of what exists, and the two are only equal by accident.
const BLESSED_FIXTURES_AT_RUNG_11: usize = 90;

/// **The freeze.** The running compiler is the one the corpus was blessed against.
#[test]
fn the_stderr_corpus_names_the_compiler_it_was_blessed_against() {
    let out = Command::new("rustc")
        .arg("--version")
        .output()
        .expect("invariant: rustc is on PATH for any tree that just compiled this test");
    let live = String::from_utf8_lossy(&out.stdout).trim().to_string();

    assert_eq!(
        live, BLESSED_RUSTC,
        "\n\
         The `.stderr` corpus was blessed against a DIFFERENT compiler than the one running.\n\
         \n  blessed: {BLESSED_RUSTC}\n  running: {live}\n\n\
         Roughly {BLESSED_FIXTURES_AT_RUNG_11} fixtures across ten `trybuild` harnesses pin rustc's \
         prose byte for byte. A compiler change re-renders them for reasons unrelated to this \
         engine -- MEASURED at profiling rung 11, 23 of them had drifted unnoticed and a green \
         workspace report did not cover them.\n\n\
         This is not necessarily a defect. It is the corpus losing its certification. Re-bless with \
         `TRYBUILD=overwrite`, REVIEW the diff for content rather than rendering, and update \
         `BLESSED_RUSTC` in the SAME commit.\n\n\
         If `running` is a compiler you did not expect, check `PATH` before anything else: a \
         standalone cargo/rustc ahead of `~/.cargo/bin` shadows the rustup proxy, and \
         `rust-toolchain.toml` cannot stop it."
    );
}

/// Non-vacuity: the corpus this freeze speaks for actually exists and is the size claimed.
///
/// Without this, deleting every fixture would leave a green gate certifying nothing — the shape
/// this campaign has now met often enough to write down: a check whose subject can vanish while the
/// check stays green is not a check.
#[test]
fn the_corpus_this_freeze_speaks_for_is_not_empty() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("crates");
    let mut found = 0usize;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                // `target` never appears under `crates/`, and skipping nothing keeps the walk
                // honest about what it counted.
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "stderr") {
                found += 1;
            }
        }
    }
    assert!(
        found >= BLESSED_FIXTURES_AT_RUNG_11,
        "the freeze claims to speak for at least {BLESSED_FIXTURES_AT_RUNG_11} `.stderr` fixtures \
         and found {found}. Either the corpus shrank -- in which case the gate above is certifying \
         less than it says -- or this walk stopped seeing it."
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// The second freeze: a glob that matches nothing is a corpus that was never shown to rustc
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// `trybuild` glob call sites this scan speaks for — a lower bound, MEASURED 2026-08-26.
///
/// Carried as a number for the same reason [`BLESSED_FIXTURES_AT_RUNG_11`] is: the claim is
/// about *scale*. **63 call sites across 30 harnesses**, MEASURED by the scan below on the run
/// that landed it, is not something anyone audits by eye — which is exactly why one of them
/// could point at a directory that does not exist and report success.
///
/// `>=`, so adding a harness or a fixture never reds it. Removing one does, and should be
/// deliberate: this constant and the corpus it counts move in the same commit.
const TRYBUILD_GLOB_SITES: usize = 63;

/// Harnesses this scan speaks for — MEASURED 2026-08-26, on the same run.
const TRYBUILD_HARNESSES: usize = 30;

/// Everything in this repository that drives `trybuild` — the harnesses, by content rather
/// than by name, so a harness that does not follow the `*_compile_fail.rs` convention is still
/// covered.
fn trybuild_harnesses() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("tests")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs")
                // This census names the two call needles as literals, so it selects ITSELF and
                // then reports its own parser as two unresolvable globs. MEASURED on the first
                // run. Excluded by name rather than by a cuter content test, because a cuter
                // one is a thing that silently stops matching.
                && path.file_name().is_some_and(|f| f != "trybuild_corpus_compiler_witness.rs")
                && std::fs::read_to_string(&path)
                    .is_ok_and(|src| src.contains("trybuild::TestCases::new()"))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The package directory a harness belongs to — the working directory `trybuild` resolves its
/// glob against.
fn manifest_dir_of(file: &std::path::Path) -> std::path::PathBuf {
    file.ancestors()
        .skip(1)
        .find(|d| d.join("Cargo.toml").is_file())
        .unwrap_or_else(|| panic!("no Cargo.toml above {}", file.display()))
        .to_path_buf()
}

/// The first `"…"` span in `s`. The corpus has no escapes inside a glob; if one ever appears
/// this truncates, the pattern then fails to resolve, and the gate reds — which is the right
/// direction for a parser that stops understanding its input.
fn first_string_literal(s: &str) -> Option<String> {
    let open = s.find('"')? + 1;
    let close = s[open..].find('"')? + open;
    Some(s[open..close].to_string())
}

/// `const NAME: &str = "value";` declared in the same file, so a glob written as
/// `format!("tests/{CORPUS}/*.rs")` — the shape that keeps a directory to ONE spelling —
/// resolves here too.
fn string_consts(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let Some(rest) = line.trim_start().strip_prefix("const ") else { continue };
        let Some((name, tail)) = rest.split_once(':') else { continue };
        let Some((_ty, value)) = tail.split_once('=') else { continue };
        let Some(lit) = first_string_literal(value) else { continue };
        out.push((name.trim().to_string(), lit));
    }
    out
}

/// **The class gate.** Every `trybuild` glob in this repository resolves to a fixture that
/// exists.
///
/// # Why a second gate, and why here
///
/// The freeze above says *which compiler* the `.stderr` corpus was blessed against. It cannot
/// say the corpus was ever **handed to** that compiler, and the two are independent: a
/// `trybuild` harness whose glob matches nothing prints *"There are no trybuild tests enabled
/// yet"*, reports `running 1 test … ok`, and exits **0**. `running N` is blind to it, because
/// the harness function does run — over zero fixtures.
///
/// **MEASURED 2026-08-26**, on `crates/boyko_reflect/tests/seam_census.rs`: changing **one
/// character** of the glob — `seam_compile_fail` → `seam_compile_fai1`, with the directory and
/// all five fixtures untouched — left the target at exit **0**, `3 passed`. The harness's own
/// `>=` floor did not fire, because the floor counted a `CORPUS` constant that the glob did not
/// read: the directory was spelled twice, and only one spelling was load-bearing. That file and
/// `crates/reflect_fixture/tests/reflect_compile_fail.rs`, which it inherited the shape from,
/// now build their globs from their constants — but a per-harness repair only ever covers the
/// harnesses somebody looked at, and there are twenty-five of them.
///
/// This scan is deliberately coarse: it does not know what a fixture asserts, only that the
/// pattern a harness hands `trybuild` names something on disk. That is the whole of the
/// property, and it is the half `running N` and the compiler freeze both miss.
///
/// # What it refuses to guess
///
/// A glob it cannot resolve — a shape other than a literal or a `format!` over same-file
/// string constants, or one with a placeholder left unsubstituted — is a **failure**, not a
/// skip. A census that silently drops what it does not understand is how the count above stops
/// covering the corpus without anyone noticing.
#[test]
fn every_trybuild_glob_resolves_to_a_fixture_that_exists() {
    let harnesses = trybuild_harnesses();
    assert!(
        harnesses.len() >= TRYBUILD_HARNESSES,
        "found {} trybuild harness(es); {TRYBUILD_HARNESSES} were MEASURED. A scan that stops \
         seeing its \
         subject reports the same green as a scan that checked it.",
        harnesses.len()
    );

    let mut sites = 0usize;
    let mut violations: Vec<String> = Vec::new();

    for harness in &harnesses {
        let src = std::fs::read_to_string(harness).expect("invariant: just read this file");
        let manifest_dir = manifest_dir_of(harness);
        let consts = string_consts(&src);
        let shown = harness.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(harness);

        for (n, line) in src.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in [".compile_fail(", ".pass("] {
                let mut from = 0usize;
                while let Some(at) = line[from..].find(needle) {
                    let arg_start = from + at + needle.len();
                    from = arg_start;
                    sites += 1;
                    let site = format!("{}:{}", shown.display(), n + 1);

                    let Some(raw) = first_string_literal(&line[arg_start..]) else {
                        violations.push(format!(
                            "{site}: the glob is not a string literal and not a `format!` over \
                             one, so this census cannot resolve it. Teach it the shape or write \
                             the glob as `format!(\"tests/{{CONST}}/*.rs\")`."
                        ));
                        continue;
                    };
                    let mut pattern = raw;
                    for (name, value) in &consts {
                        pattern = pattern.replace(&format!("{{{name}}}"), value);
                    }
                    if pattern.contains('{') {
                        violations.push(format!(
                            "{site}: `{pattern}` still holds an unsubstituted placeholder -- the \
                             constant it names is not a `const … : &str = \"…\";` in the same file."
                        ));
                        continue;
                    }

                    if let Some(prefix) = pattern.strip_suffix("/*.rs") {
                        let dir = manifest_dir.join(prefix);
                        let found = std::fs::read_dir(&dir)
                            .map(|it| {
                                it.flatten()
                                    .filter(|e| {
                                        e.path().extension().is_some_and(|x| x == "rs")
                                    })
                                    .count()
                            })
                            .unwrap_or(0);
                        if found == 0 {
                            violations.push(format!(
                                "{site}: `{pattern}` matches NOTHING under {}. `trybuild` reports \
                                 \"There are no trybuild tests enabled yet\" and exits 0, so this \
                                 harness certifies an empty set. Either the directory moved, or \
                                 the glob and the floor name different places.",
                                dir.display()
                            ));
                        }
                    } else if pattern.contains('*') {
                        violations.push(format!(
                            "{site}: `{pattern}` is a wildcard shape this census does not expand \
                             (it knows `<dir>/*.rs`). Teach it before shipping the pattern."
                        ));
                    } else if !manifest_dir.join(&pattern).is_file() {
                        violations.push(format!(
                            "{site}: `{pattern}` names no file under {}.",
                            manifest_dir.display()
                        ));
                    }
                }
            }
        }
    }

    println!(
        "trybuild glob census: {} harness(es), {sites} glob call site(s), {} violation(s)",
        harnesses.len(),
        violations.len()
    );
    assert!(
        sites >= TRYBUILD_GLOB_SITES,
        "the scan found {sites} trybuild glob call site(s) and this tree has at least \
         {TRYBUILD_GLOB_SITES}. A shrunken scan passes over what it stopped reading, which is \
         the failure mode this whole file exists to close."
    );
    assert!(
        violations.is_empty(),
        "\n{} trybuild glob(s) do not resolve to a fixture. Each one is a harness that reports \
         success over an EMPTY corpus -- exit 0, `running 1 test … ok`, and the `.stderr` files \
         beside it never shown to a compiler.\n\n{}\n",
        violations.len(),
        violations.join("\n")
    );
}
