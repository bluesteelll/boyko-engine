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
