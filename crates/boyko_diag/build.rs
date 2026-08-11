//! **The one build script in this workspace**, and the only reader of `BOYKO_PROFILE`.
//!
//! # Why it is here and nowhere else
//!
//! A build script can set `cargo::rustc-env` and write files for **the crate it belongs to and no
//! other**. Both subsystems need the same profile decision, so "one script read by two subsystems"
//! forces that script to the bottom of the graph — this crate, which `boyko_log`, `boyko_ecs`,
//! `boyko_threadpool` and `boyko_rhi_vulkan` all sit above. Two scripts is the failure being
//! avoided: a binary that prints a ceiling its own profile does not name.
//!
//! It is the **first** build script this workspace has ever had. Two consequences were measured
//! before it landed rather than discovered after:
//!
//! - `crates/boyko_diag/tests/zero_dependency_census.rs` already lists `[build-dependencies]`
//!   among the tables that must stay empty, and it stays empty: everything below is `std`.
//! - `DG9`'s mute-leaf scan reads `crates/boyko_diag/src`, and this file is not under `src`. That
//!   is not a loophole being exploited, it is the rule applying where it means something. The
//!   mute-leaf rule is about what the **leaf contains at run time** — no print, no file, no
//!   process, no thread in the image a game links. A build script is not in that image at all: it
//!   is a host program whose entire output is text consumed by `cargo`, and `println!` is the only
//!   way it can speak. A `std::fs` write here reaches `OUT_DIR`, never a shipped binary.
//!
//! # What it does NOT emit, and why the absence is deliberate
//!
//! **`BOYKO_BUILD_HASH` is not emitted.** `SEAM.md`'s S9 lists it, and profiling rung 7 already
//! measured that no such value exists anywhere in the workspace; rung 13 re-measured it and shipped
//! `HEADER_FLAG_INVARIANT_TSC` in its place, on the rule that *a header field that is always absent
//! is indistinguishable from one that is broken*. Nothing reads a build hash today, and
//! `profile.rs`'s own rule — **only the constants a landed rung actually reads are declared** —
//! refuses a constant nothing reads. Producing one would also mean spawning `git` from a build
//! script, which makes every build depend on a repository state cargo cannot see and cannot
//! invalidate on.
//!
//! **`LANE_COUNT` is not emitted**, and never will be: Q1 deleted its profile axis because it
//! indexes `boyko_threadpool::MAX_WORKERS`, which has none. Returning it here re-opens the
//! unsoundness Q1 closed.
//!
//! **A cargo FEATURE cannot be set from here.** `profiling-analysis` is a feature of `boyko_ecs`;
//! cargo resolves features before any build script runs, and `cargo::rustc-cfg` applies only to the
//! crate that emitted it. The axis therefore *enforces* agreement instead of *setting* it: it emits
//! [`ANALYSIS_ADMITTED`] and the crate that owns the feature asserts the two agree. See
//! `crates/boyko_ecs/src/ecs/core/profiling/mod.rs`.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

/// One row of the axis: what a named profile materially IS.
///
/// Every field is an integer, never a subsystem's enum — this crate does not own the level model or
/// the tier model, and naming either here would put a consumer's taxonomy in the bottom crate.
struct Row {
    /// `0 = Off, 1 = Error, 2 = Warn, 3 = Info, 4 = Debug, 5 = Trace` (`boyko_log::Level`'s).
    log_ceiling: u8,
    /// `0 = Always, 1 = Dev, 2 = Deep` (`boyko_diag::profiling_abi::ZoneTier`'s).
    profiling_tier: u8,
    /// Samples one lane region holds before it refuses.
    region_capacity: u32,
    /// Engine zone id slots, which is also the profiling store's zone stride.
    engine_zone_slots: usize,
    /// The cap on ids a `User`-partition crate may mint.
    max_user_budget: usize,
    /// The dynamic-zone name arena, in bytes.
    dyn_name_bytes: usize,
    /// Whether this profile admits `boyko_ecs`'s `profiling-analysis` feature.
    analysis_admitted: bool,
}

/// The axis, exactly as `docs/diagnostics/SEAM.md` §S9 tables it.
///
/// # The `off` row is a LOGGING off switch, and the corpus's word for it does not exist
///
/// S9's table gives `off` the entry *"feature `profiling` off"* in the tier column. **MEASURED at
/// this rung: there is no `profiling` cargo feature anywhere in the workspace** — `boyko_diag`
/// declares `section-gate`, `boyko_ecs` declares `profiling-analysis`, `big_query_table` and
/// `bench-alloc`, and no crate gates `zone!` or `declare_zone!` on a feature at all. Nor could this
/// script set one if it existed (see the module docs). And `ZoneTier` has no `Off` position: its
/// three values are `Always`, `Dev` and `Deep`, so the *lowest* compile ceiling the profiler has
/// still admits every `Always` site.
///
/// So `off` is spelled honestly: `LOG_CEILING = 0`, which is live and load-bearing —
/// `boyko_log::LANE_ARRAY_LEN` is `if GLOBAL_CEILING == Off { 0 } else { LANE_COUNT }`, so the
/// logger's lane array really does become zero-length — beside the `shipping` row's profiling
/// numbers, because the profiler's compile axis has no off position to select. Turning the
/// profiler off is the RUNTIME axis's job (`ARM_MASK`), which is `GJ1`'s subject, and giving it a
/// compile-time off position is the FEATURE axis's, which is `G1`'s. Neither is this rung's, and
/// writing a zero here that no consumer could act on would be the shape rung 13 named: a value
/// indistinguishable from a broken one.
const ROWS: &[(&str, Row)] = &[
    (
        "dev",
        Row {
            log_ceiling: 5,
            profiling_tier: 2,
            region_capacity: 1024,
            engine_zone_slots: 4096,
            max_user_budget: 3072,
            dyn_name_bytes: 64 * 1024,
            analysis_admitted: true,
        },
    ),
    (
        "editor",
        Row {
            log_ceiling: 4,
            profiling_tier: 1,
            region_capacity: 1024,
            engine_zone_slots: 4096,
            max_user_budget: 3072,
            dyn_name_bytes: 64 * 1024,
            analysis_admitted: true,
        },
    ),
    (
        "shipping",
        Row {
            log_ceiling: 3,
            profiling_tier: 0,
            region_capacity: 128,
            engine_zone_slots: 256,
            max_user_budget: 512,
            dyn_name_bytes: 8 * 1024,
            analysis_admitted: false,
        },
    ),
    (
        "shipping-min",
        Row {
            log_ceiling: 2,
            profiling_tier: 0,
            region_capacity: 128,
            engine_zone_slots: 256,
            max_user_budget: 512,
            dyn_name_bytes: 8 * 1024,
            analysis_admitted: false,
        },
    ),
    (
        "off",
        Row {
            log_ceiling: 0,
            profiling_tier: 0,
            region_capacity: 128,
            engine_zone_slots: 256,
            max_user_budget: 512,
            dyn_name_bytes: 8 * 1024,
            analysis_admitted: false,
        },
    ),
];

/// The per-knob overrides, which survive **only** under `BOYKO_PROFILE=custom`.
///
/// Setting one beside a named profile is a `compile_error!` naming the conflict. That is the whole
/// point of the single axis: two axes is how a binary ends up printing a ceiling its profile does
/// not name.
const KNOBS: &[&str] = &[
    "BOYKO_PROFILING_TIER",
    "BOYKO_PROFILING_REGION_CAPACITY",
    "BOYKO_PROFILING_DYN_CAP",
    "BOYKO_LOG_MAX_LEVEL",
];

/// `boyko_log::Level`'s spelling, lowest to highest.
const LEVEL_NAMES: &[&str] = &["off", "error", "warn", "info", "debug", "trace"];

/// `ZoneTier`'s spelling, lowest to highest.
const TIER_NAMES: &[&str] = &["always", "dev", "deep"];

fn main() {
    // Emitting ANY `rerun-if-*` directive opts out of cargo's default "rescan the whole package"
    // behaviour, so this line is not tidiness: without it a change to THIS FILE would not re-run
    // it, and the workspace would keep compiling against the previous table.
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=BOYKO_PROFILE");
    for knob in KNOBS {
        println!("cargo::rerun-if-env-changed={knob}");
    }

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("invariant: cargo sets OUT_DIR"));
    let body = match resolve() {
        Ok(text) => text,
        // A refusal still emits the full `dev` table beside its `compile_error!`, so the build
        // fails with exactly ONE diagnostic — the named one — instead of that message plus an
        // `E0425` for every constant the rest of the workspace reads.
        Err(message) => format!("{}\ncompile_error!(\"{}\");\n", table(&ROWS[0].1, "dev"), escape(&message)),
    };
    fs::write(out.join("profile_axis.rs"), body).expect("invariant: OUT_DIR is writable");
}

/// The whole decision: which row, with which overrides, or why not.
fn resolve() -> Result<String, String> {
    let requested = match env::var_os("BOYKO_PROFILE") {
        None => return Ok(table(&ROWS[0].1, "dev")),
        Some(v) => match v.into_string() {
            Ok(s) => s,
            Err(_) => return Err("BOYKO_PROFILE is set to a value that is not valid Unicode".into()),
        },
    };
    let requested = requested.trim().to_owned();

    if requested == "custom" {
        return custom();
    }

    let Some((name, row)) = ROWS.iter().find(|(n, _)| *n == requested) else {
        return Err(format!(
            "BOYKO_PROFILE={requested} names no profile. The five are {}, plus `custom`, \
             which is the only value under which the per-knob overrides ({}) are honoured",
            names(),
            KNOBS.join(", "),
        ));
    };

    // The single-axis rule, enforced. A knob set beside a named profile is refused rather than
    // silently losing to it or silently beating it -- both of which produce a binary whose printed
    // ceiling and whose actual ceiling come from different places.
    for knob in KNOBS {
        if let Some(v) = env::var_os(knob) {
            return Err(format!(
                "BOYKO_PROFILE={name} selects LOG_CEILING={} and PROFILING_TIER={}, but {knob}={} \
                 is also set. One build axis, or a binary prints a ceiling its profile does not \
                 name. Either drop {knob}, or select BOYKO_PROFILE=custom, which is the one value \
                 that honours it",
                row.log_ceiling,
                row.profiling_tier,
                v.to_string_lossy(),
            ));
        }
    }

    Ok(table(row, name))
}

/// `custom` starts from the `dev` row and applies each knob that is set.
///
/// A base is needed because the knobs do not cover every constant on the axis — there is no knob
/// for `ENGINE_ZONE_SLOTS` — and a profile with unset sizing constants is not a profile. `dev` is
/// the base because it is the default profile, so `custom` with no knob set is exactly `dev` and
/// the difference between them is precisely what the operator asked for.
fn custom() -> Result<String, String> {
    let mut row = Row {
        log_ceiling: ROWS[0].1.log_ceiling,
        profiling_tier: ROWS[0].1.profiling_tier,
        region_capacity: ROWS[0].1.region_capacity,
        engine_zone_slots: ROWS[0].1.engine_zone_slots,
        max_user_budget: ROWS[0].1.max_user_budget,
        dyn_name_bytes: ROWS[0].1.dyn_name_bytes,
        analysis_admitted: ROWS[0].1.analysis_admitted,
    };

    if let Some(v) = knob_value("BOYKO_LOG_MAX_LEVEL")? {
        row.log_ceiling = named(&v, LEVEL_NAMES, "BOYKO_LOG_MAX_LEVEL")?;
    }
    if let Some(v) = knob_value("BOYKO_PROFILING_TIER")? {
        row.profiling_tier = named(&v, TIER_NAMES, "BOYKO_PROFILING_TIER")?;
    }
    if let Some(v) = knob_value("BOYKO_PROFILING_REGION_CAPACITY")? {
        let n = number(&v, "BOYKO_PROFILING_REGION_CAPACITY")?;
        // `as u32` would TRUNCATE, and the truncation is worse than it looks: 2^32 becomes 0, and
        // `number` has already accepted the value by then, so the zero the consumer's assertion
        // exists to refuse would arrive from a knob that named a large number.
        row.region_capacity = u32::try_from(n).map_err(|_| {
            format!("BOYKO_PROFILING_REGION_CAPACITY={v} does not fit in a u32")
        })?;
    }
    if let Some(v) = knob_value("BOYKO_PROFILING_DYN_CAP")? {
        row.max_user_budget = number(&v, "BOYKO_PROFILING_DYN_CAP")?;
    }

    Ok(table(&row, "custom"))
}

/// One knob's text, refusing a non-Unicode value rather than ignoring it.
fn knob_value(knob: &str) -> Result<Option<String>, String> {
    match env::var_os(knob) {
        None => Ok(None),
        Some(v) => v
            .into_string()
            .map(|s| Some(s.trim().to_owned()))
            .map_err(|_| format!("{knob} is set to a value that is not valid Unicode")),
    }
}

/// Maps a knob's spelling onto its raw discriminant.
fn named(value: &str, names: &[&str], knob: &str) -> Result<u8, String> {
    let lower = value.to_ascii_lowercase();
    names
        .iter()
        .position(|n| *n == lower)
        .map(|i| i as u8)
        .ok_or_else(|| format!("{knob}={value} names nothing; the values are {}", names.join(", ")))
}

/// Parses a sizing knob, refusing zero.
///
/// Zero is refused here rather than at the consumer because the consumer's refusal is a `const`
/// assertion whose message names the constant, not the knob that produced it — and this crate has
/// one such assertion already (`ENGINE_ZONE_SLOTS > 0`, "a zero stride would alias 'never armed'").
fn number(value: &str, knob: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(0) => Err(format!("{knob}=0 is not a size")),
        Ok(n) => Ok(n),
        Err(_) => Err(format!("{knob}={value} is not a non-negative integer")),
    }
}

/// The five named profiles, for a refusal's message.
fn names() -> String {
    ROWS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}

/// Renders one row as the generated module's body.
fn table(row: &Row, name: &str) -> String {
    let mut s = String::with_capacity(768);
    let _ = writeln!(s, "// GENERATED by crates/boyko_diag/build.rs from BOYKO_PROFILE. Do not edit.");
    let _ = writeln!(s, "pub const PROFILE_NAME: &str = \"{}\";", escape(name));
    let _ = writeln!(s, "pub const LOG_CEILING: u8 = {};", row.log_ceiling);
    let _ = writeln!(s, "pub const PROFILING_TIER: u8 = {};", row.profiling_tier);
    let _ = writeln!(s, "pub const REGION_CAPACITY: u32 = {};", row.region_capacity);
    let _ = writeln!(s, "pub const ENGINE_ZONE_SLOTS: usize = {};", row.engine_zone_slots);
    let _ = writeln!(s, "pub const MAX_USER_BUDGET: usize = {};", row.max_user_budget);
    let _ = writeln!(s, "pub const DYN_NAME_BYTES: usize = {};", row.dyn_name_bytes);
    let _ = writeln!(s, "pub const ANALYSIS_ADMITTED: bool = {};", row.analysis_admitted);
    s
}

/// Makes a message safe to sit inside the generated `compile_error!`'s string literal.
///
/// The text carries operator-supplied environment values, so it is untrusted input to a code
/// generator: an unescaped `"` would end the literal and turn a named refusal into a parse error
/// that names nothing.
fn escape(s: &str) -> String {
    s.chars().flat_map(char::escape_default).collect()
}
