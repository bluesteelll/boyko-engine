//! **Reflection GATES G3 — the artifact census: three legs, two needles, and the link
//! configuration MEASURED (P-C).**
//!
//! Built on `crates/profile_fixture/tests/profile_axis_census.rs` — the template, cited
//! and copied, not rediscovered. The tool resolution is **copied, not shared**, for the
//! reason that file states: sharing would mean a dev-dependency edge that unifies
//! features into the very images under census (GATES D6).
//!
//! # The three legs (GATES G3; the L3 correction of 2026-08-21 is already in force)
//!
//! | leg | bin | `reflect` feature | opt-in in the source? | role |
//! |---|---|---|---|---|
//! | L1 | `reflect_off_twin` | off | present, `#[cfg]`-stripped | the SHIP cell — needle A must read 0 |
//! | L2 | `reflect_on` | on | present, live | the PRESENT CONTROL — needle A must read > 0 |
//! | L3 | `reflect_never` | **on** | **absent** | the DISCRIMINATOR — resolved, compiled, linked, and named by nothing |
//!
//! ~~Until CORE C7 lands the `#[component(reflect)]` key, "present" means G0's landed
//! deviation: the direct `#[cfg(feature = "reflect")]` fn-pointer reference to
//! `boyko_reflect::install_type_info` (`reflect_linkage()`), and "absent" means
//! `reflect_never.rs` carries no such linkage.~~ ~~→ **corrected at C7's landing
//! (2026-08-21, CORE D26).** C7 landed the key and `reflect_on.rs` now carries **both** …
//! `OPT_IN_TOKENS` below and its assertion's failure text still say "C7" and are
//! **deliberately left**: C8's `Lands` carries them.~~
//!
//! → **BOTH SUPERSEDED at CORE C8's landing (2026-08-21).** "Present" now means the
//! annotation and nothing else: `reflect_linkage()` is deleted from `reflect_on.rs` (and
//! from `reflect_off_twin_plus.rs`), and needle B's subject in the L2 image is the
//! derive's own emitted `install_type_info` call. `OPT_IN_TOKENS` is the annotation form,
//! and the crate-path token it used to carry moved to `L3_FORBIDDEN_TOKENS` — it could not
//! stay, because the same list is asserted **present in L2**, and post-C8 `reflect_on.rs`
//! does not spell that path anywhere.
//!
//! **The retirement is sound only because `main()` now touches `component_id()`** (CORE
//! D27). The install slot lives inside that method; measured before C8, `component_id`
//! occurred **zero** times in every link configuration of this bin, so deleting the
//! linkage without the touch would have reddened `l2_a > 0` *and* `l2_b > 0` together. All
//! three bins carry the touch, because L3 and G7a's control must not differ from L2 in a
//! second way.
//!
//! # THE MEASURED LINK-CONFIGURATION TABLE (this box, `x86_64-pc-windows-gnu`)
//!
//! Filled by running `measure_link_configuration_table` (`--ignored --nocapture`);
//! pasted into `docs/REFLECTION-PLAN-GATES.md` §G3 as that rung requires.
//!
//! **At G0 (2026-08-21, the hollow stub — one fn in the crate):**
//!
//! | link configuration | L1 A | L2 A | L3 A | L1 B | L3 B |
//! |---|---|---|---|---|---|
//! | default release | 0 | 1 | **0** | 0 | **0** |
//! | `-C link-arg=-Wl,--gc-sections` | 0 | 1 | **0** | 0 | **0** |
//! | `lto = "fat"`, `codegen-units = 1` | 0 | 1 | **0** | 0 | **0** |
//!
//! **RE-CALIBRATED at CORE C2 (2026-08-21, the real registry surface), as the G0
//! measured note required:**
//!
//! | link configuration | L1 A | L2 A | L3 A | L1 B | L3 B |
//! |---|---|---|---|---|---|
//! | default release | 0 | **6** | **0** | 0 | **0** |
//! | `-C link-arg=-Wl,--gc-sections` | 0 | **6** | **0** | 0 | **0** |
//! | `lto = "fat"`, `codegen-units = 1` | 0 | **5** | **0** | 0 | **0** |
//!
//! The gated cells did not move: **L1 and L3 read 0 in every row, needle B included**,
//! so the census still distinguishes *reachable* from *linked* and P-C stays an
//! independent property. What moved is L2's magnitude — the pulled-object rule made
//! visible: the one referenced symbol (`install_type_info`) pulls its object, which now
//! carries the `REFLECT` static and three std `OnceLock`/`Once` instantiations whose
//! v0-mangled names embed `boyko_reflect` as the type parameter's defining crate (the
//! instantiations-in-a-downstream-CGU class needle A was designed to count; fat LTO
//! strips the non-LTO rows' extra `__imp_` import thunk for the `REFLECT` data symbol,
//! 6 → 5). `type_info_of` appears in NO row — `#[inline]`, uncalled by the fixture, so
//! it has no instantiation to carry. The C2 re-run of G3's first RED (drop fat LTO from
//! `build()`) left the gate GREEN again: both gated zeros are protected upstream of the
//! link configuration — L1 by the resolver (the crate is not in the graph), L3 by the
//! pulled-object rule (nothing references the crate, so no object is pulled) — and L2 is
//! asserted only `> 0`. B.6's "every cell reads 1" still does not reproduce on this
//! subject. The leg stays fat-LTO for the reason that now has a date on it: at CORE C7
//! the derive's expansion starts referencing SOME of the crate's symbols, and per-symbol
//! decidability inside a pulled object — exactly what the non-LTO rows lack — becomes
//! the census's whole question.
//!
//! **At G0, L3 read 0 under fat LTO — and, unlike B.6's `mint_cold`, 0 in every row**
//! (confirmed unchanged by the C2 re-calibration above). The plan deliberately did not
//! predict which way this would go; the measurement says: the census genuinely
//! distinguishes *reachable* from *linked* on this subject, so P-C is an independent
//! property rather than the resolver's zero wearing the census's name.
//! Why this subject decides differently from `mint_cold` (which read 1 without LTO and
//! `--gc-sections` could not fix it): a PE linker pulls an rlib's object only when some
//! undefined symbol resolves into it. `profile_fixture` CALLS into `boyko_diag`, so that
//! object is pulled and every symbol in it — `mint_cold` included — rides along.
//! `reflect_never` references NOTHING in `boyko_reflect`, so its object is never pulled
//! at all. The distinction that survives both measurements: **"one referenced symbol
//! pulls the whole object"** — at C2 the crate gained its real surface and the L2 rows
//! above show the rule in action; per-symbol sharpness without LTO is what C7's
//! derive-emission census will need, which is why the gate leg stays fat-LTO.
//! (Verified at G0: L2's single needle-A hit was needle B's subject —
//! `_RNvCsd7WGKwjPoHP_13boyko_reflect17install_type_info`, then the crate's only fn; v0
//! mangling encodes the defining crate, so needle A counts it. At C2 the same symbol
//! survives at its new module path, `…13boyko_reflect8registry17install_type_info`.)
//!
//! **RE-CALIBRATED at CORE C7 (2026-08-21) — `reflect_on.rs`'s `FixturePod` now carries
//! `#[component(reflect)]` BESIDE the linkage (CORE D26), and the table gains its
//! missing `L2 B` column (C7 gate 9a):**
//!
//! | link configuration | L1 A | L2 A | L3 A | L1 B | **L2 B** | L3 B |
//! |---|---|---|---|---|---|---|
//! | default release | 0 | **41** | **0** | 0 | **1** | **0** |
//! | `-C link-arg=-Wl,--gc-sections` | 0 | **41** | **0** | 0 | **1** | **0** |
//! | `lto = "fat"`, `codegen-units = 1` | 0 | **5** | **0** | 0 | **1** | **0** |
//!
//! The gated cells did not move: L1 and L3 read 0 in every row, both needles.
//!
//! **`L2 B` is the column this table did not have, and it is why D26 exists.** Needle B
//! is the literal `install_type_info`, and the property every scheduled re-run of this
//! calibration checks is *per-symbol* decidability — a statement about needle B in the
//! one leg where needle B has a subject, which is L2. With no such cell, and with the
//! gate asserting only `l2_a > 0`, retiring the linkage at C7 would have taken L2's
//! `install_type_info` reference to zero and moved nothing this instrument reports. It
//! now reads **1** in every row.
//!
//! **And `L2 B` is now ASSERTED, not merely printed.** As landed, the column lived only
//! in `measure_link_configuration_table`, which is `#[ignore]`d and asserts nothing —
//! the instrument existed and the gate still could not see it, which is this campaign's
//! dead-datum class (five instances found so far). `l2_b > 0` is a clause of
//! `reflect_absence_census_three_legs_under_fat_lto` as of the C7 follow-up; the
//! reasoning for `> 0` rather than `== 1`, and the honest statement that the clause is a
//! **constant until C8**, are at the assertion itself.
//!
//! **The 6 → 41 growth in L2 A is C3–C6's, NOT the derive's — measured by A/B rather
//! than attributed.** Rebuilding the identical leg with the annotation commented out
//! gives **41 as well** (default release, same `CARGO_TARGET_DIR`; the source was
//! restored byte-identically). The crate's own surface is what grew since the C2 run:
//! `prim`'s 24 accessors, `array_get`/`array_set`, `validate` with
//! `walk_nested`/`exhaust`, and the `Violation`/`Problem` `Debug`/`Display` impls with
//! their `Vec<Problem>` `RawVec` instantiations. One referenced symbol still pulls the
//! whole object; the object got bigger.
//!
//! **RE-CALIBRATED at CORE C8 (2026-08-21) — the linkage is RETIRED, `main()` touches
//! `component_id()`, and needle B's subject is now the derive's own emitted install:**
//!
//! | link configuration | L1 A | L2 A | L3 A | L1 B | L2 B | L3 B |
//! |---|---|---|---|---|---|---|
//! | default release | 0 | **42** | **0** | 0 | **1** | **0** |
//! | `-C link-arg=-Wl,--gc-sections` | 0 | **42** | **0** | 0 | **1** | **0** |
//! | `lto = "fat"`, `codegen-units = 1` | 0 | **7** | **0** | 0 | **1** | **0** |
//!
//! The gated cells did not move: L1 and L3 read 0 in every row, both needles. **The rung's
//! whole claim is in the two cells that DID move**, and both moved in the direction that
//! says the emission reached an artifact — measured, not attributed:
//!
//! * **L2 A under fat LTO: 5 → 7.** At C7 the surviving five were the pulled object's
//!   floor. The two additions, read off `llvm-nm` rather than guessed, are
//!   `boyko_reflect::prim::get_f32` and `prim::set_f32` — the accessor pair
//!   `FixturePod::value`'s baked `FieldInfo` names. They survive LTO now because the
//!   descriptor is REACHED; at C7 nothing referenced it, so they were dropped. The whole
//!   fat-LTO row is: those two accessors, `install_type_info`, the `REFLECT` data symbol,
//!   and the three std `OnceLock<&TypeInfo>` instantiations.
//! * **The consumer-side descriptor, 0 → 2.** `__REFLECT_TYPE_INFO` / `__REFLECT_FIELDS`
//!   read **zero in every row at C7** — the artifact-level witness of *"the static exists
//!   and is inert"*. They now read **2** in the L2 fat-LTO image and **0** in L1 and L3.
//!   Needle A still cannot count them (they are mangled with `reflect_on` as the defining
//!   crate), which is why this is recorded here in prose rather than left to a cell.
//!
//! **And the number that makes L1's zero mean something, first measured at this rung:**
//! `component_id` occurs **4** times in the L1 image and **4** in L3, where it occurred
//! **0** times in every configuration before C8. The funnel is now compiled into *both*
//! images and only the emitted `#[cfg(feature = "reflect")]` separates them. Until the
//! D27 touch landed, the ship cell's zero was earned by the funnel being absent from
//! everything — a true statement about a program that called nothing, not about the
//! feature gate.
//!
//! **And needle A structurally cannot see the derive's emission at all.**
//! `__REFLECT_TYPE_INFO` / `__REFLECT_FIELDS` / `__reflect_type_id_of::<T>` are defined
//! in the CONSUMER crate, so v0 mangling names *`reflect_on`* as their defining crate — a
//! needle keyed on `boyko_reflect` counts none of them by construction. The image in fact
//! contains **zero** symbols matching those names in any row: at C7 the descriptor is
//! referenced by nothing and is dropped before the linker sees it, which is C7's own
//! *"the static exists and is inert"* with an artifact-level witness, and why the fat-LTO
//! row is unchanged at 5. Whether the emission reached the image is **G6b's** question,
//! not this gate's — and from CORE C8 it becomes needle B's, because the install slot is
//! the first thing that references `boyko_reflect` FROM the emission. ~~Re-run this
//! calibration at C8.~~ → **done, 2026-08-21; the C8 table is above.** The prediction held
//! and the counter-reading is recorded there: the descriptor symbols went 0 → 2 while
//! needle A stayed structurally blind to them, so the derive's emission reaching the image
//! is visible in this file only through needle B and through that prose row.
//!
//! And "absent from the image" still does not mean "absent from the build": L3's build
//! DID resolve, compile and link the crate. That is G1/G2's question and not this
//! gate's (see "what this gate cannot claim").
//!
//! # Cost, stated rather than hidden
//!
//! The calibration run (9 builds, 3 link configurations, cold per-leg target dirs) took
//! **2 m 31 s on this box**; the gate itself builds only the three fat-LTO legs
//! (~17 s per cold leg, seconds warm — the wall time is written into the ledger from the
//! gate's own run). Per-leg target dirs under the system temp dir keep the legs from
//! rebuilding each other, and keep a nested cargo out of the outer sweep's dir — the
//! linker `permission denied` this campaign has already paid for once.
//!
//! # What this gate cannot claim (GATES G3, verbatim scope)
//!
//! Nothing about a member it does not build (it censuses a fixture — D2 — not
//! `boyko_demo`). Nothing about build time or dependency surface: a crate resolved,
//! compiled, linked and then stripped reads 0 here and is not absent in the sense the
//! owner means — that is G1/G2's question. Nothing about metadata that lives in
//! `boyko_ecs` under B.1 Horn 1 (D16). Nothing about a profile a CI leg does not build.

// Miri cannot spawn processes, and this file exists to spawn cargo/llvm-nm. The guard is
// load-bearing for G4's Miri allowlist row (`-p reflect-fixture --features
// reflect-fixture/reflect` runs `--all-targets`): without it that row reds on a process
// spawn, for a reason that has nothing to do with reflection — and the likeliest "fix"
// (dropping the package from the sweep) would silently revert B.9. The template this
// file copies carries NO such guard and does not need one, because `profile_fixture` is
// not on the allowlist; copying it verbatim onto the allowlist imports the failure.
#![cfg(not(miri))]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Needle A — the crate-name fragment (GATES D5). Both Rust mangling schemes encode the
/// defining crate, so this counts every symbol `boyko_reflect` contributes, including
/// instantiations landing in a downstream CGU. It is a count, not a boolean, and the
/// count is the measurand.
const NEEDLE_A: &str = "boyko_reflect";

/// Needle B — the plain-`pub fn` LTO-sensitivity probe (GATES D5): the same symbol kind
/// as `mint_cold`, the one B.6 measured as undecidable without `lto = "fat"`. CORE C2
/// replaces the fn's BODY and keeps this name — which is why the needle is this name.
const NEEDLE_B: &str = "install_type_info";

/// One link configuration the census can build under. The gate runs `FatLto`; the other
/// two exist for the measured table and the first RED (drop fat LTO ⇒ re-read all
/// cells).
#[derive(Clone, Copy)]
enum LinkCfg {
    DefaultRelease,
    GcSections,
    FatLto,
}

impl LinkCfg {
    fn tag(self) -> &'static str {
        match self {
            LinkCfg::DefaultRelease => "default",
            LinkCfg::GcSections => "gc-sections",
            LinkCfg::FatLto => "fat-lto",
        }
    }
}

/// Builds one fixture bin in one feature state under one link configuration and returns
/// the image path. Panics rather than returns on failure: a census whose artifact could
/// not be produced has not measured anything (RED-not-SKIP, applied to the build step).
///
/// Per-leg `CARGO_TARGET_DIR` under the system temp dir; `RUSTFLAGS` removed because an
/// inherited `-C embed-bitcode=no` is incompatible with `-C lto`.
///
/// **Log-scraper caveat, permanent (G0):** every build of this package prints Cargo's
/// *"found to be present in multiple build targets"* notice for the shared twin source.
/// It is a Cargo notice, not a rustc lint; nothing here treats build-log `warning:`
/// presence as failure — failure is the exit status.
fn build(bin: &str, feature_on: bool, link: LinkCfg) -> PathBuf {
    let target = std::env::temp_dir().join(format!(
        "boyko-reflect-census-{bin}-{}-{}",
        if feature_on { "on" } else { "off" },
        link.tag()
    ));
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args(["build", "-p", "reflect-fixture", "--bin", bin, "--release"]);
    if feature_on {
        cmd.args(["--features", "reflect"]);
    }
    match link {
        LinkCfg::DefaultRelease => {}
        LinkCfg::GcSections => {
            cmd.env("RUSTFLAGS", "-C link-arg=-Wl,--gc-sections");
        }
        LinkCfg::FatLto => {
            cmd.args(["--config", "profile.release.lto=\"fat\""]);
            cmd.args(["--config", "profile.release.codegen-units=1"]);
        }
    }
    if !matches!(link, LinkCfg::GcSections) {
        cmd.env_remove("RUSTFLAGS");
    }
    let status = cmd
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .unwrap_or_else(|e| panic!("could not spawn cargo to build {bin}: {e}"));
    assert!(
        status.success(),
        "building {bin} (feature_on={feature_on}, {}) failed, so the census has no artifact",
        link.tag()
    );
    let exe = target.join("release").join(format!("{bin}{}", std::env::consts::EXE_SUFFIX));
    assert!(exe.is_file(), "{} was not produced", exe.display());
    exe
}

/// Counts symbols in `image` whose name contains `needle`. **Tool absence is a RED,
/// never a SKIP** (GATES D6): a gate that passes on every machine lacking its tool is a
/// gate that passes.
fn symbols_matching(image: &Path, needle: &str) -> usize {
    let tool = resolve_tool("llvm-nm").unwrap_or_else(|| {
        panic!(
            "llvm-nm is on neither PATH nor any rustup toolchain's rustlib bin. That is a \
             RED, not a skip: without it this gate cannot distinguish an absent crate from \
             a present one. Install it with `rustup component add llvm-tools`."
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

/// Runs a built fixture and returns its one stdout line (GATES G3 gate 5).
fn run(image: &Path) -> String {
    let out = Command::new(image)
        .output()
        .unwrap_or_else(|e| panic!("{} could not be run: {e}", image.display()));
    assert!(out.status.success(), "{} exited non-zero", image.display());
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Locates an LLVM binutil: `PATH` first, then the rustup toolchains' `rustlib` bins.
/// Copied from `crates/profile_fixture/tests/profile_axis_census.rs:165` — copied, not
/// shared (GATES D6).
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

/// Maps a `[[bin]]` target name to its source path via this package's own manifest —
/// the single source of truth for targets (`autobins = false` is load-bearing, G0). The
/// L3 discriminator assertion keys on the source the leg ACTUALLY builds, so pointing
/// L3 at another bin cannot dodge it.
fn bin_source(bin: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .expect("reflect-fixture's own manifest is readable");
    let mut current_name: Option<String> = None;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current_name = None;
            continue;
        }
        if let Some(value) = line.strip_prefix("name = ") {
            current_name = Some(value.trim_matches('"').to_owned());
        }
        if let Some(value) = line.strip_prefix("path = ")
            && current_name.as_deref() == Some(bin)
        {
            return manifest_dir.join(value.trim_matches('"'));
        }
    }
    panic!(
        "bin target `{bin}` has no [[bin]] row in reflect_fixture/Cargo.toml -- the \
         target table is the single source of truth (autobins = false) and this leg's \
         subject does not exist"
    );
}

/// Returns `source` with every Rust comment removed, so a token clause reads **code** and
/// never **prose**.
///
/// Both token clauses of the gate below run over this text, and the reason they must move
/// together is at [`OPT_IN_TOKENS`]. This function is the whole mechanism: with it, an
/// annotation quoted in a doc comment is not an annotation, in either direction.
///
/// # What it models, and what it REFUSES rather than mis-reads
///
/// Line comments (`//`, `///`, `//!`) and plain string literals with backslash escapes —
/// which is every construct the two scanned sources (`reflect_on.rs`, `reflect_never.rs`)
/// contain, a fact the preconditions below re-check on every run rather than assume it
/// stays true. It deliberately does **not**
/// model block comments, raw strings, or a `'"'` character literal; each of those is
/// detected *in the normal state* (so a mention inside a comment is invisible to the check,
/// which is why `for"` in a prose sentence does not trip the raw-string guard) and panics
/// with a message saying what to do. A silent mis-scan of a construct the scanner does not
/// understand is the failure this refusal exists to prevent: it would drop code text, and a
/// dropped token is a GREEN on the L2 half.
fn code_only(source: &str) -> String {
    let src: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut in_string = false;
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(&next) = src.get(i + 1) {
                    out.push(next);
                    i += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        // ── normal state ─────────────────────────────────────────────────────────────
        if c == '/' && src.get(i + 1) == Some(&'/') {
            // Drop to end of line; the newline itself is pushed by the next iteration, so
            // line structure survives and a stripped line cannot join its neighbour.
            while i < src.len() && src[i] != '\n' {
                i += 1;
            }
            continue;
        }
        assert!(
            !(c == '/' && src.get(i + 1) == Some(&'*')),
            "a fixture source opens a BLOCK comment, which `code_only` does not model -- it \
             would be copied into the scanned text and could satisfy the L2 token clause \
             exactly like the prose this function exists to exclude. Use `//` comments in \
             the census's legs, or teach this scanner block comments in the same change."
        );
        if c == '"' {
            let quoted_char = i > 0 && src[i - 1] == '\'' && src.get(i + 1) == Some(&'\'');
            assert!(
                !quoted_char,
                "a fixture source contains a `'\\\"'` character literal, which `code_only` \
                 would mistake for the start of a string and then scan the rest of the file \
                 in string state -- every comment after it would be copied into the scanned \
                 text. Rewrite it, or teach this scanner character literals."
            );
            let mut start = i;
            while start > 0 && src[start - 1] == '#' {
                start -= 1;
            }
            assert!(
                !(start > 0 && src[start - 1] == 'r'),
                "a fixture source contains a RAW string literal, which `code_only` does not \
                 model -- its `\\` is not an escape, so the scanner would leave string state \
                 at the wrong quote and mis-classify everything after it. Rewrite it, or \
                 teach this scanner raw strings."
            );
            in_string = true;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// [`code_only`] is the gate's instrument, so it is pinned rather than trusted.
///
/// An instrument that only the gate exercises is checked exactly as far as today's fixture
/// sources happen to reach — and this campaign's most-repeated defect is a datum computed
/// and asserted nowhere. The four shapes below are the ones the two token clauses turn on:
/// a whole-line comment (the C8 header's two mentions), a **trailing** comment (the shape a
/// line-oriented stripper would miss), real code, and a `//` inside a string literal (which
/// must NOT be treated as a comment, or the rest of a code line vanishes).
#[test]
fn code_only_keeps_code_and_drops_prose() {
    const TOKEN: &str = "#[component(reflect)]";

    let whole_line = format!("// the target table says `{TOKEN}` and it now is\nstruct A;\n");
    assert!(!code_only(&whole_line).contains(TOKEN), "a whole-line comment survived");
    assert!(code_only(&whole_line).contains("struct A;"), "the code line did not survive");

    let trailing = format!("struct A; // beside {TOKEN}\n");
    assert!(!code_only(&trailing).contains(TOKEN), "a TRAILING comment survived");

    let code = format!("{TOKEN}\nstruct A;\n");
    assert!(code_only(&code).contains(TOKEN), "a real annotation was stripped");

    let in_string = "let s = \"a // b\"; let t = 1;\n";
    assert!(
        code_only(in_string).contains("let t = 1;"),
        "a `//` inside a string literal was taken for a comment, so the rest of the line \
         was dropped -- on the L3 half that direction is a silent GREEN"
    );
}

/// The opt-in whose presence defines `reflect_on` and whose absence defines
/// `reflect_never` — **the annotation form since CORE C8** (2026-08-21), which retired
/// G0's `reflect_linkage()` deviation.
///
/// Deliberately NOT `feature = "reflect"`: the gate-5 self-report line legitimately probes
/// `cfg!(feature = "reflect")` in every bin, and a `cfg!` probe can put no symbol in the
/// image. The opt-in is the one thing that makes the derive emit.
///
/// # Why this list SHRANK to one entry at C8, rather than gaining the annotation
///
/// It read `["reflect_linkage", "boyko_reflect::"]`, and both clauses below use it: L3
/// must contain **none** of these, L2 must contain **all** of them. Post-C8 the second
/// requirement is what decides the list. `reflect_on.rs` no longer spells
/// `boyko_reflect::` anywhere — that path now exists only inside the derive's expansion —
/// so keeping the entry here (the literal reading of "swap in the annotation form") reds
/// the twin-source clause on the very change that landed the rung. The crate-path token
/// did not simply vanish; it moved to [`L3_FORBIDDEN_TOKENS`], which is the half of its
/// old job that survives.
///
/// # These strings are matched against CODE, never against prose — and BOTH halves must be
///
/// The entries are ordinary Rust that any file may also mention **in a comment**, and the
/// list is read by two clauses pointing in **opposite** directions: L3 must contain
/// **none** of them, L2 must contain **all** of them. Prose satisfies a `contains`-all just
/// as readily as it trips a `contains`-any, so a rule applied to one half only leaves the
/// other half's defect intact — and the C8 landing did exactly that, twice in one change:
///
/// * The **L3** half fired on prose first: `reflect_never.rs`'s `FixturePod` doc comment
///   spelled the attribute out inside a sentence explaining what the file lacks, and the
///   census failed on its own documentation. The rule written then — *"L3's source may not
///   quote these tokens"* — fixed the file and left the mechanism untouched.
/// * The **L2** half then failed silently, which is the worse direction. C8 changed this
///   list from `["reflect_linkage", "boyko_reflect::"]` — tokens the pre-C8 `reflect_on.rs`
///   carried **only in code** — to an attribute the same change's rewritten header spells
///   **twice in prose**. MEASURED at the C8 audit: deleting the real
///   `#[component(reflect)]` from `reflect_on.rs` left the header's two mentions, left this
///   clause GREEN, and the census only reported three assertions and ~16 s of fat-LTO builds
///   later, on `l2_a`, with a message about the linker rather than about the retired
///   annotation.
///
/// So the scan runs over [`code_only`], for **both** halves, from one derived text. That is
/// the mechanism rather than a rule authors must remember, and it is deliberately shared: a
/// future entry cannot be prose-proof on one side and prose-satisfiable on the other,
/// because there is only one scan. Neither leg's rustdoc has to avoid the tokens any more —
/// `reflect_never.rs`'s warning paragraph says so at its own site.
const OPT_IN_TOKENS: &[&str] = &["#[component(reflect)]"];

/// Tokens L3 must not contain, but which L2 is no longer required to spell.
///
/// `boyko_reflect::` was in [`OPT_IN_TOKENS`] until CORE C8 and did two jobs there. The
/// job that ends at C8 is *"the present control carries a hand-written reference"* — G0's
/// deviation, now retired. The job that does **not** end is L3's: `reflect_never` is the
/// LINKED-UNUSED leg, and its whole meaning is that the crate is resolved, compiled and
/// linked while the source **references nothing in it**. A source-text reference would end
/// that without touching the derive key, and the check for it must survive the constant it
/// used to live in — dropping it silently would be the same class of loss the `l2_b`
/// clause exists to prevent.
const L3_FORBIDDEN_TOKENS: &[&str] = &["boyko_reflect::"];

/// **The gate** (GATES G3, all five clauses, under the one link configuration the
/// measured table shows is decidable: `lto = "fat"`, `codegen-units = 1`).
#[test]
fn reflect_absence_census_three_legs_under_fat_lto() {
    // ── L3's non-collision assertion runs FIRST (fourth RED's subject): the
    //    discriminator must not silently be the present control under another name. ──
    let l3_bin = "reflect_never";
    let l3_source = bin_source(l3_bin);
    let l3_text = std::fs::read_to_string(&l3_source)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", l3_source.display()));
    let l3_code = code_only(&l3_text);
    let colliding: Vec<&&str> = OPT_IN_TOKENS
        .iter()
        .chain(L3_FORBIDDEN_TOKENS)
        .filter(|t| l3_code.contains(*(*t)))
        .collect();
    assert!(
        colliding.is_empty(),
        "L3's fixture ({}) contains the reflect opt-in tokens {colliding:?} IN CODE -- the \
         linked-unused discriminator has collapsed into the present control under a \
         different name, and any number it reported would be a false statement about the \
         instrument's reach (the exact wrong conclusion the corrected G3 exists to \
         prevent). Comments are stripped before this scan ([`code_only`]), so this is a \
         real reference and not that file's rustdoc describing what it lacks.",
        l3_source.display()
    );
    // ...and the twin source MUST carry the opt-in, or the annotation was retired without
    // updating this census. `L3_FORBIDDEN_TOKENS` is deliberately NOT required here: since
    // CORE C8 the crate path lives only in the derive's expansion, and requiring the
    // source to spell it would demand the very hand-written reference C8 deleted.
    //
    // SAME `code_only` TEXT AS THE CLAUSE ABOVE, and that is the point (see OPT_IN_TOKENS):
    // read against raw source this clause is satisfied by `reflect_on.rs`'s own header,
    // which spells the attribute twice, and deleting the real annotation leaves it green.
    let l2_text = std::fs::read_to_string(bin_source("reflect_on")).expect("reflect_on source");
    let l2_code = code_only(&l2_text);
    let missing: Vec<&&str> = OPT_IN_TOKENS.iter().filter(|t| !l2_code.contains(*(*t))).collect();
    assert!(
        missing.is_empty(),
        "reflect_on.rs no longer carries the reflect opt-in {missing:?} IN CODE -- the \
         present control has stopped being annotated, so `l2_a`/`l2_b` below would report \
         a build that opted into nothing. Comments are stripped before this scan \
         ([`code_only`]), so a mention of {missing:?} in that file's header does NOT \
         satisfy this clause: the attribute must be on a type. If the opt-in's spelling \
         changed, update OPT_IN_TOKENS in the same change."
    );

    // ── The three legs, fat-LTO linked. ──────────────────────────────────────────────
    let l1 = build("reflect_off_twin", false, LinkCfg::FatLto);
    let l2 = build("reflect_on", true, LinkCfg::FatLto);
    let l3 = build(l3_bin, true, LinkCfg::FatLto);

    // Gate 5: each artifact reports the configuration this test asked for.
    let l1_line = run(&l1);
    assert!(
        l1_line.contains("bin=reflect_off_twin")
            && l1_line.contains("reflect_feature=off")
            && l1_line.contains("linkage=absent"),
        "L1's artifact reports {l1_line:?} -- the build did not use the leg this test \
         asked for (ship cell: reflect_off_twin, feature off)"
    );
    let l2_line = run(&l2);
    assert!(
        l2_line.contains("bin=reflect_on")
            && l2_line.contains("reflect_feature=on")
            && l2_line.contains("linkage=present"),
        "L2's artifact reports {l2_line:?} -- the build did not use the leg this test \
         asked for (present control: reflect_on, feature on)"
    );
    let l3_line = run(&l3);
    assert!(
        l3_line.contains("bin=reflect_never")
            && l3_line.contains("reflect_feature=on")
            && l3_line.contains("linkage=never"),
        "L3's artifact reports {l3_line:?} -- the build did not use the leg this test \
         asked for (linked-unused: reflect_never, feature ON, no opt-in)"
    );

    // ── The counts. ──────────────────────────────────────────────────────────────────
    let l1_a = symbols_matching(&l1, NEEDLE_A);
    let l1_b = symbols_matching(&l1, NEEDLE_B);
    let l2_a = symbols_matching(&l2, NEEDLE_A);
    let l2_b = symbols_matching(&l2, NEEDLE_B);
    let l3_a = symbols_matching(&l3, NEEDLE_A);
    let l3_b = symbols_matching(&l3, NEEDLE_B);

    // Gate 2 runs before gate 1: an absent control makes L1's zero mean nothing, so its
    // failure must not be reported as the ship cell's success.
    assert!(
        l2_a > 0,
        "NOT RESOLVED (census inert): the present control (reflect_on, feature on) \
         carries NO `{NEEDLE_A}` symbol, so the ship cell's zero below is \
         indistinguishable from `no fixture` -- suspect the link configuration or the \
         needle before believing anything this census says (L2 A = {l2_a})"
    );

    // Gate 2's needle-B half (CORE D26). The clause above is the present control for
    // needle A and says NOTHING about needle B, whose zeros are asserted twice below
    // (L1 B, L3 B). L2 is the one leg where needle B has a subject at all, so this is
    // the only place its presence can be controlled for.
    //
    // `> 0`, not `== 1`, and the difference is the property: the measurand is "needle B
    // HAS a subject in the present control", exactly as `l2_a > 0` is for needle A. The
    // count itself is not stable across link configurations -- the header's own table
    // shows needle A moving 6 -> 5 when fat LTO strips an `__imp_` import thunk -- so an
    // equality here would red on the linker's bookkeeping rather than on the property.
    // Measured 1 in all three rows at C7.
    //
    // **THE CLAUSE WENT LIVE AT CORE C8** (2026-08-21). It was written at the C7 follow-up
    // and stated plainly there that it was a CONSTANT until C8: what put `install_type_info`
    // in the L2 image at C7 was `reflect_on.rs`'s temporary `reflect_linkage()`, whose
    // presence the OPT_IN_TOKENS clause already required, so the two clauses could not
    // disagree. C8 deleted that reference. Since C8 the ONLY thing that can put needle B in
    // this image is the derive's own emitted install call -- and, because that call sits
    // inside `component_id()`, only if `main()` still CALLS `component_id()` (CORE D27,
    // measured: with the funnel untouched the whole slot is dropped before the linker sees
    // it, and this clause reds together with `l2_a > 0`).
    //
    // So this is now the clause that answers "did C8's emission reach an artifact?", which
    // no other check in the campaign asks.
    //
    // **AND IT HAS NO OBSERVED RED OF ITS OWN THAT DOES NOT ALSO RED `l2_a`.** Written at
    // the C8 landing as *"its two live REDs are the deletion of the `#[cfg]` wrapper and
    // the deletion of the `black_box(component_id())` touch, which reds exactly here"* --
    // both halves of that sentence are refuted by measurement. The `cfg` deletion reds gate
    // 2 (`E0433` on `reflect_off_twin`, in the BUILD, before any needle is counted). The
    // touch deletion takes needle A and needle B to zero TOGETHER (D27 measured it: with
    // the funnel untouched the whole slot is dropped), and `l2_a > 0` is asserted ~35 lines
    // above, so THAT is the clause that reports -- observed at `l2_a`, not here. The same
    // holds for deleting the annotation itself. Every mutation that removes the install
    // removes the pulled object, and the pulled object is what needle A counts.
    //
    // Its ONE distinguishing case is *needle B losing its subject while the crate is still
    // in the image* -- the "probe loses its subject in silence" shape D26 exists to prevent.
    // That case IS constructible at C8, and the red was OBSERVED (2026-08-21, this box) by
    // flipping `boyko_reflect::registry::install_type_info` from `#[inline(never)]` to
    // `#[inline(always)]`: under fat LTO the call is inlined into the emitted slot, the
    // symbol NAME leaves the image, and this clause reds ALONE with `L2 B = 0` while
    // `l2_a > 0` stays green on the six remaining `boyko_reflect` symbols. The sibling case
    // -- renaming the installer without re-pointing NEEDLE_B -- is the same shape by a
    // different route and needs no separate run. `#[inline(never)]` on the installer is
    // therefore load-bearing for this census, not a codegen preference.
    assert!(
        l2_b > 0,
        "NOT RESOLVED (needle B inert): the present control (reflect_on, feature on) \
         carries NO `{NEEDLE_B}` symbol, so L1 B = {l1_b} and L3 B = {l3_b} below are \
         indistinguishable from `the needle matches nothing anywhere` and neither zero \
         is evidence of anything. Since CORE C8 this means the derive's emitted install \
         call did not reach the image: either the seventh slot in `component_id()` is not \
         being emitted, or nothing calls `component_id()` in `reflect_on.rs`'s `main()` -- \
         an uncalled `#[inline]` method is dropped before the linker sees it, so the slot \
         can be present, correct, and in no artifact (L2 B = {l2_b}; the C8 re-calibration \
         in this file's header records what it reads now)"
    );

    // Gate 1 — the ship cell.
    assert!(
        l1_a == 0 && l1_b == 0,
        "THE SHIP CELL IS NOT ZERO: reflect_off_twin (feature off, fat LTO) carries \
         needle A = {l1_a}, needle B = {l1_b}. With the feature off the crate must not \
         be in the closure at all (G1/G2), so any symbol here is a leak the resolver did \
         not cause -- a derive residue, a stray edge, or an inert link configuration."
    );

    // Gate 3 — L3, asserted against the MEASURED table value (0/0 on this target under
    // fat LTO), with the interpretation the table forces.
    assert!(
        l3_a == 0 && l3_b == 0,
        "L3 (linked-unused) reads needle A = {l3_a}, needle B = {l3_b}, but the measured \
         table for this target says 0/0 under fat LTO. If this fires, the census has \
         STOPPED distinguishing `reachable` from `linked` (or the linkage crept into \
         reflect_never): L1's zero would then be earned by the resolver, not by this \
         instrument, and the claim P-C certifies must be re-narrowed per GATES G3 -- \
         re-measure the table before trusting any cell."
    );
}

/// **The measurement protocol** — fills the link-configuration table in this file's
/// header and in `docs/REFLECTION-PLAN-GATES.md` §G3. Ignored by default: it is the
/// instrument's calibration run (~9 builds), not the gate; run it with
/// `cargo test -p reflect-fixture --test reflect_absence_census -- --ignored --nocapture`.
///
/// # The **L2 B** column, added at CORE C7 (D23 gate 9a)
///
/// The table printed six aggregate columns and carried **no needle-B column for L2** —
/// the present control. Needle B is the literal name `install_type_info` (GATES D5's
/// LTO-sensitivity probe), and every scheduled re-run of this calibration exists to
/// re-check *per-symbol* decidability inside a pulled object; that property is about
/// needle B in the leg where needle B has a subject, and L2 is that leg. Without the
/// column, a change that removed L2's `install_type_info` reference would move nothing
/// the table reports and nothing the gate asserts (`l2_a > 0` only) — which is exactly
/// why D26 refused to retire the linkage at C7: **a probe losing its subject in
/// silence.** The column is the instrument half of that refusal.
///
/// The **gate** half arrived in the C7 follow-up. An instrument that only ever prints,
/// from inside a test that is `#[ignore]`d and asserts nothing, is not a check — it is
/// the same silence with a table in it, and `l2_b` was computed here and asserted
/// nowhere. `reflect_absence_census_three_legs_under_fat_lto` now carries `l2_b > 0`
/// beside `l2_a > 0`; see that clause for why `> 0` and for what it is worth before C8.
#[test]
#[ignore = "calibration: builds 3 legs x 3 link configurations and prints the table"]
fn measure_link_configuration_table() {
    println!("| link configuration | L1 A | L2 A | L3 A | L1 B | L2 B | L3 B |");
    println!("|---|---|---|---|---|---|---|");
    for link in [LinkCfg::DefaultRelease, LinkCfg::GcSections, LinkCfg::FatLto] {
        let l1 = build("reflect_off_twin", false, link);
        let l2 = build("reflect_on", true, link);
        let l3 = build("reflect_never", true, link);
        println!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            link.tag(),
            symbols_matching(&l1, NEEDLE_A),
            symbols_matching(&l2, NEEDLE_A),
            symbols_matching(&l3, NEEDLE_A),
            symbols_matching(&l1, NEEDLE_B),
            symbols_matching(&l2, NEEDLE_B),
            symbols_matching(&l3, NEEDLE_B),
        );
    }
}
