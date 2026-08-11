//! **G14 and G16 — the cross-profile symbol census**, run over artifacts this file builds itself.
//!
//! # What it measures
//!
//! One source, two `BOYKO_PROFILE` legs, one symbol question per fixture:
//!
//! | binary | site's tier / level | `dev` | `shipping` |
//! |---|---|---|---|
//! | `deep_zone` | `ZoneTier::Deep` | present | **absent** |
//! | `always_zone` | `ZoneTier::Always` | present | present |
//! | `profile-fixture-log` | `Level::Debug` | present | **absent** |
//!
//! The profiling half is a **2×2, not the 1×2 the corpus specifies**, and the extra row is what
//! makes the interesting cell mean anything. A `shipping` binary with no emission symbol is
//! ambiguous on its own: it is equally consistent with "the tier fold deleted this site" and with
//! "a shipping build contains no profiler at all". `always_zone` — the same fixture one tier down,
//! nothing else changed — answers that, and it must be **present** in `shipping`. Exactly one of
//! the four cells is zero, and it is the one under test.
//!
//! # THE FINDING: without LTO this census is INERT, and `--gc-sections` does not fix it
//!
//! MEASURED on this box, `x86_64-pc-windows-gnu`, release, over `deep_zone`:
//!
//! | link configuration | `dev` | `shipping` | decidable? |
//! |---|---|---|---|
//! | default release | `mint_cold` = 1 | `mint_cold` = **1** | no |
//! | `-C link-arg=-Wl,--gc-sections` | 1 | **1** | no — *no effect whatsoever* |
//! | `lto = "fat"`, `codegen-units = 1` | 1 | **0** | yes |
//!
//! The reason is visible in the same output: the default-release `deep_zone` image contains
//! `core::ptr::drop_glue::<boyko_diag::telemetry::Block>`, in a binary whose source never mentions
//! telemetry. The whole `boyko_diag` rlib is carried into the image and nothing collects it, so a
//! whole-image census answers *"was this symbol codegen'd into some rlib on the way here?"* rather
//! than *"can this program reach it?"*. Those are different questions and only the second is G14's.
//!
//! The corpus anticipated an instrument failure here and predicted the wrong side of it: it warned
//! that `open`/`record` might **inline away in the `dev` leg**, leaving the census with no subject,
//! and required the `dev` leg as the control that would catch it. The control was right and the
//! prediction was backwards — the subject survives in the `shipping` leg instead, and the `dev` leg
//! would have looked perfect while the gate could not fail. Two REDs of the same shape had to be
//! run before this file was believed: the `--gc-sections` leg (which changed nothing at all) and
//! the first draft of the logging fixture (below).
//!
//! # The two subsystems do not need the same instrument, and the reason is their symbol's KIND
//!
//! MEASURED by running the no-LTO RED through this file: `g14a` failed with all four cells at 1,
//! and **`g16ab` passed unchanged**. The logging census is decidable without LTO and the profiling
//! one is not, because the two symbols are different kinds of thing:
//!
//! - `emit_impl` is **generic** (`emit_impl<A: LogArgs>`), so a monomorphisation exists only if
//!   some site instantiated it. Delete the site and the symbol was never codegen'd anywhere.
//! - `mint_cold` is a **plain function in a dependency's rlib**. It is codegen'd when `boyko_diag`
//!   is compiled, whether or not anything reaches it, and on this target nothing collects it out of
//!   the final image.
//!
//! Both legs are still built with LTO here — one instrument, not two, so a future reader does not
//! have to remember which clause tolerates which link. But the asymmetry is recorded because it is
//! the thing that decides whether *any* new census clause needs LTO: ask what kind of symbol it
//! names, not which subsystem it belongs to.
//!
//! **What this does and does not license.** The tier gate is `const { … } && …`, so the *call* is
//! deleted by the compiler in every configuration; LTO is not what deletes it. LTO is what lets a
//! census SEE that nothing references the callee. The claim is therefore "the fold removes the
//! site's codegen", proved under a link that can observe it — not "a shipping game must use LTO".
//!
//! # Cost, stated rather than hidden
//!
//! Four small LTO builds (two profiles × two fixture packages, each a 2- or 3-crate graph) and four
//! `llvm-nm` runs, ~20 s total on this box. Each leg gets its **own `CARGO_TARGET_DIR`** under the
//! system temp dir: the profile changes `boyko_diag`'s generated table, so two legs sharing a
//! target dir would rebuild each other in a loop, and a nested cargo sharing the outer sweep's
//! target dir is the linker `permission denied` this campaign has already paid for once.
//!
//! # What it cannot claim
//!
//! Nothing about the **runtime** flag: a symbol present in `dev` says nothing about whether it
//! executes, which is `GJ1`'s question. Nothing about a profile CI does not build (`custom`).
//! Nothing about a **dynamic** logging site — `dyn_debug!` is logging rung L10 and does not exist
//! yet, so the `emit_impl` clause covers the static path only.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The mangled fragment naming the profiler's out-of-line emission path.
///
/// `ZoneGuard::open` and its `Drop` are both `#[inline]` and trivial, so neither survives a release
/// build as a symbol — the corpus names them and they cannot be the subject. `mint_cold` can:
/// it is `#[cold] #[inline(never)]`, it is on the path every zone site takes on its first use, and
/// it is reachable from a `zone!` and from nothing else in these fixtures.
const ZONE_EMIT_SYMBOL: &str = "mint_cold";

/// The mangled fragment naming the logger's out-of-line emission path.
const LOG_EMIT_SYMBOL: &str = "emit_impl";

/// Builds one fixture package under one profile, LTO-linked, and returns the binary's path.
///
/// Panics rather than returns on failure: a census whose artifact could not be produced has not
/// measured anything, and the RED-not-SKIP rule applies to the build step exactly as it applies to
/// the tool.
fn build(package: &str, bin: &str, profile: &str) -> PathBuf {
    let target = std::env::temp_dir().join(format!("boyko-axis-census-{profile}"));
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", package, "--release"])
        // `--config` rather than a `[profile.release]` edit in the workspace manifest: LTO here is
        // the INSTRUMENT's requirement, not the engine's, and writing it into the shared profile
        // would change every release build in the repository to satisfy one gate.
        .args(["--config", "profile.release.lto=\"fat\""])
        .args(["--config", "profile.release.codegen-units=1"])
        .env("BOYKO_PROFILE", profile)
        .env("CARGO_TARGET_DIR", &target)
        // Inherited incremental state is per-target-dir, but an inherited RUSTFLAGS is not, and
        // `-C embed-bitcode=no` from an outer invocation is incompatible with `-C lto`.
        .env_remove("RUSTFLAGS")
        .status()
        .unwrap_or_else(|e| panic!("could not spawn cargo to build {package} under {profile}: {e}"));
    assert!(
        status.success(),
        "building {package} under BOYKO_PROFILE={profile} failed, so the census has no artifact"
    );

    let exe = target.join("release").join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
    assert!(exe.is_file(), "{} was not produced", exe.display());
    exe
}

/// Counts symbols in `image` whose name contains `needle`.
///
/// **Tool absence is a RED, never a SKIP** — the rule G22a states and this campaign has caught
/// vacuity under more than once. A gate that passes on every machine without the tool is a gate
/// that passes.
fn symbols_matching(image: &Path, needle: &str) -> usize {
    let tool = resolve_tool("llvm-nm").unwrap_or_else(|| {
        panic!(
            "llvm-nm is on neither PATH nor any rustup toolchain's rustlib bin. That is a RED, not \
             a skip: without it this gate cannot distinguish a folded site from a present one. \
             Install it with `rustup component add llvm-tools`."
        )
    });
    let out = Command::new(&tool)
        .arg(image)
        .output()
        .unwrap_or_else(|e| panic!("{} could not be run: {e}", tool.display()));
    assert!(out.status.success(), "{} exited non-zero on {}", tool.display(), image.display());
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().filter(|l| l.contains(needle)).count()
}

/// Runs a built fixture and returns its one stdout line.
fn run(image: &Path) -> String {
    let out = Command::new(image)
        .output()
        .unwrap_or_else(|e| panic!("{} could not be run: {e}", image.display()));
    assert!(out.status.success(), "{} exited non-zero", image.display());
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Locates an LLVM binutil: `PATH` first, then the rustup toolchains' `rustlib` bins.
///
/// This is the same resolution order `boyko_diag::storage`'s probe uses, and it is deliberately
/// **not** shared with it. Sharing would mean a `[dev-dependencies] boyko-diag = { features =
/// ["section-gate"] }` in this package — and cargo unifies features across one build, so that entry
/// would switch `section-gate` on for the fixture BINARIES too, compiling `std::process` and
/// `std::fs` into the very images this file takes a census of. The duplication is ~30 lines; the
/// alternative perturbs the measurement.
fn resolve_tool(stem: &str) -> Option<PathBuf> {
    let mut exe = String::from(stem);
    if cfg!(windows) {
        exe.push_str(".exe");
    }

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join(&exe);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }

    let home = std::env::var_os("RUSTUP_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(|h| PathBuf::from(h).join(".rustup"))
    })?;

    let toolchains = home.join("toolchains");
    let named: Vec<PathBuf> = match std::env::var_os("RUSTUP_TOOLCHAIN") {
        Some(t) => vec![toolchains.join(t)],
        None => std::fs::read_dir(&toolchains).ok()?.filter_map(|e| e.ok().map(|e| e.path())).collect(),
    };

    let mut fallback = None;
    for tc in named {
        let Ok(targets) = std::fs::read_dir(tc.join("lib").join("rustlib")) else {
            continue;
        };
        for target in targets.filter_map(|e| e.ok()) {
            let cand = target.path().join("bin").join(&exe);
            if !cand.is_file() {
                continue;
            }
            let name = target.file_name();
            let name = name.to_string_lossy();
            if name.contains(std::env::consts::ARCH) && name.contains(std::env::consts::OS) {
                return Some(cand);
            }
            if fallback.is_none() {
                fallback = Some(cand);
            }
        }
    }
    fallback
}

/// **G14(a)** — the tier fold is per-site, shown across two profiles over single-site binaries.
///
/// RED: delete `const { $h::TIER as u8 <= GLOBAL_TIER as u8 }` from `zone_enabled!` ⇒ the `Deep`
/// site's emission path appears in the `shipping` leg ⇒ the one zero cell fills in.
///
/// SECOND RED, and the one that matters more: drop `lto = "fat"` from `build` ⇒ **every cell reads
/// 1** and the gate can no longer fail at all. That is the state this gate would have shipped in.
#[test]
fn g14a_the_deep_site_is_deleted_by_the_shipping_ceiling_and_only_there() {
    let cells = [
        ("deep_zone", "dev", true),
        ("deep_zone", "shipping", false),
        ("always_zone", "dev", true),
        ("always_zone", "shipping", true),
    ];

    let mut report = String::new();
    let mut wrong = Vec::new();
    for (bin, profile, expect_present) in cells {
        let image = build("profile-fixture", bin, profile);
        let n = symbols_matching(&image, ZONE_EMIT_SYMBOL);
        report.push_str(&format!("  {bin:<12} {profile:<9} {ZONE_EMIT_SYMBOL} = {n}\n"));
        if (n > 0) != expect_present {
            wrong.push(format!("{bin}/{profile}: expected {expect_present}, got {n}"));
        }
    }

    assert!(
        wrong.is_empty(),
        "the cross-profile census does not have the shape the tier fold requires:\n{report}\n\
         mismatched: {}\n\
         Exactly one cell may be zero -- `deep_zone` under `shipping` -- and the other three are \
         what make that zero mean the fold rather than an empty binary. If ALL FOUR are non-zero, \
         suspect the link configuration before the fold: without `lto = \"fat\"` the whole \
         `boyko_diag` rlib rides into the image and this census is inert.",
        wrong.join("; ")
    );
}

/// **G14(b)** — the shipping build is not vacuous: its `Always` tier still records.
///
/// The clause rev 3 of the corpus wanted, obtained from behaviour instead of from a symbol. A
/// ceiling that folded everything gives zero samples here, and a profiler that records nothing in a
/// shipping title is indistinguishable from one that was never compiled in.
///
/// RED: give `ZoneTier` an `Off` position below `Always` and select it ⇒ `calls=0` ⇒ red.
#[test]
fn g14b_the_shipping_build_still_runs_its_always_tier() {
    let image = build("profile-fixture", "always_zone", "shipping");
    let line = run(&image);
    assert!(
        line.contains("profile=shipping"),
        "the artifact reports {line:?}, so the build did not use the profile this test asked for -- \
         which is the half of `the generated value matches the profile that was requested` that only \
         a harness spawning its own build can check"
    );
    assert!(
        line.contains("tier=0"),
        "shipping must compile at ZoneTier::Always (0); the artifact says {line:?}"
    );
    assert!(
        line.contains("calls=10"),
        "the shipping build opened and closed its Always-tier zone ten times and recorded {line:?}"
    );
}

/// **G16(a)/(b)** — the per-profile logging ceiling deletes a `debug!` site, and only in the
/// profile whose ceiling is below it.
///
/// RED: drop `$crate::GLOBAL_CEILING as u8 >= $crate::Level::Debug as u8` from `debug!` ⇒
/// `emit_impl` appears in the `shipping` leg.
///
/// SECOND RED, run and recorded because it is the reason the fixture calls `set_target_level`:
/// remove that call ⇒ `emit_impl = 0` in **both** legs. `CONTROL` is `.bss`-zero, LTO proves the
/// runtime gate false for the whole program, and the site vanishes everywhere — a census with no
/// subject, which reads exactly like a pass.
#[test]
fn g16ab_the_debug_site_is_deleted_by_the_shipping_ceiling() {
    let dev = build("profile-fixture-log", "profile-fixture-log", "dev");
    let ship = build("profile-fixture-log", "profile-fixture-log", "shipping");

    let n_dev = symbols_matching(&dev, LOG_EMIT_SYMBOL);
    let n_ship = symbols_matching(&ship, LOG_EMIT_SYMBOL);

    assert!(
        n_dev > 0,
        "the `dev` leg carries no {LOG_EMIT_SYMBOL} at all, so the `shipping` leg's zero would \
         measure nothing. The dev leg is this gate's positive control and its absence is \
         NOT RESOLVED (census inert), never a pass."
    );
    assert_eq!(
        n_ship, 0,
        "a `shipping` build (GLOBAL_CEILING = Info) still references {LOG_EMIT_SYMBOL} from a \
         `debug!` site, so the per-profile compile ceiling did not delete it (dev leg: {n_dev})"
    );

    // The ceiling, read out of the artifacts, is the second half of "the build used the profile
    // that was asked for" -- and across the five rows it is a one-to-one label for the profile.
    assert!(run(&dev).contains("ceiling=5"), "the dev artifact does not report a Trace ceiling");
    assert!(run(&ship).contains("ceiling=3"), "the shipping artifact does not report an Info ceiling");
}

/// Runs `cargo check` on one package under one environment and returns (succeeded, stderr).
fn check(package: &str, envs: &[(&str, &str)], features: &[&str]) -> (bool, String) {
    let target = std::env::temp_dir().join("boyko-axis-refusal");
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["check", "-p", package]).env("CARGO_TARGET_DIR", &target).env_remove("RUSTFLAGS");
    // Every variable this axis reads is cleared first, so an outer invocation's environment cannot
    // decide the answer. A refusal gate inheriting the very variable it is testing would report
    // whatever the operator happened to be running under.
    for k in ["BOYKO_PROFILE", "BOYKO_PROFILING_TIER", "BOYKO_PROFILING_REGION_CAPACITY", "BOYKO_PROFILING_DYN_CAP", "BOYKO_LOG_MAX_LEVEL"] {
        cmd.env_remove(k);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if !features.is_empty() {
        cmd.args(["--features", &features.join(",")]);
    }
    let out = cmd.output().unwrap_or_else(|e| panic!("could not spawn cargo: {e}"));
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// **G14(c)** — a profile that does not admit the analysis half REFUSES a build that enables it.
///
/// This replaces the corpus's symbol census over `ConcurrencyReport` / `resolve` / the TOML writer,
/// and it is strictly stronger rather than a weakening. A census answers *"did this particular
/// artifact end up containing them?"* — and, per the LTO finding above, on this target it would
/// have answered that question wrongly. A build refusal answers *"can any shipping artifact contain
/// them?"*, which is what the row in `SEAM.md` actually claims.
///
/// Both directions are asserted. A one-sided version would pass on a workspace that failed to build
/// for any reason at all, which is the same vacuity the `dev` control exists to catch above.
#[test]
fn g14c_a_shipping_profile_refuses_the_analysis_half() {
    let (ok, stderr) = check("boyko-ecs", &[("BOYKO_PROFILE", "shipping")], &["profiling-analysis"]);
    assert!(
        !ok,
        "BOYKO_PROFILE=shipping accepted `--features profiling-analysis`, so a shipping build can \
         carry `ConcurrencyReport`, `resolve` and the TOML writer while its own profile says the \
         analysis half is absent"
    );
    assert!(
        stderr.contains("does not admit the analysis half"),
        "the refusal did not name the conflict; a build that fails for an unnamed reason teaches \
         the operator nothing. stderr was:\n{stderr}"
    );

    let (ok, stderr) = check("boyko-ecs", &[("BOYKO_PROFILE", "dev")], &["profiling-analysis"]);
    assert!(ok, "BOYKO_PROFILE=dev must ACCEPT the analysis half, or the refusal above proves \
                 nothing about the profile. stderr was:\n{stderr}");
}

/// **G16(c)** — a per-knob override beside a named profile is a `compile_error!`, not a silent
/// winner or a silent loser.
///
/// This is the single-axis rule made mechanical. With two axes a binary ends up printing a ceiling
/// its profile does not name, and no test downstream can tell which of the two produced the value.
///
/// RED: delete the knob loop from `crates/boyko_diag/build.rs` ⇒ the build succeeds and the ceiling
/// silently comes from whichever side the script checked last.
#[test]
fn g16c_a_stray_knob_beside_a_named_profile_refuses_to_build() {
    let (ok, stderr) = check(
        "boyko-diag",
        &[("BOYKO_PROFILE", "shipping"), ("BOYKO_LOG_MAX_LEVEL", "trace")],
        &[],
    );
    assert!(!ok, "BOYKO_PROFILE=shipping accepted BOYKO_LOG_MAX_LEVEL=trace beside it");
    assert!(
        stderr.contains("One build axis"),
        "the refusal did not name the rule it is enforcing. stderr was:\n{stderr}"
    );

    // The same knob under `custom`, which is the one value that honours it, must BUILD -- otherwise
    // the refusal above is indistinguishable from "this knob is simply broken".
    let (ok, stderr) = check(
        "boyko-diag",
        &[("BOYKO_PROFILE", "custom"), ("BOYKO_LOG_MAX_LEVEL", "trace")],
        &[],
    );
    assert!(ok, "BOYKO_PROFILE=custom must honour BOYKO_LOG_MAX_LEVEL. stderr was:\n{stderr}");

    // And a value that names no profile is refused by name rather than defaulted to `dev`, which
    // would ship a typo as a full-fat development build.
    let (ok, stderr) = check("boyko-diag", &[("BOYKO_PROFILE", "retail")], &[]);
    assert!(!ok, "BOYKO_PROFILE=retail was accepted; a misspelt profile must not fall back");
    assert!(
        stderr.contains("names no profile"),
        "the refusal did not name the typo. stderr was:\n{stderr}"
    );
}
