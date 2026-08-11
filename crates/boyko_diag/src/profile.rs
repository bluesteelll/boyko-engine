//! The build-profile axis — the ONE place `BOYKO_PROFILE` lands.
//!
//! # Why this is in the bottom crate at all
//!
//! [`crate`]'s growth rule is deliberately hard to satisfy: a thing enters here only if **both**
//! subsystems write it and a disagreement between two copies would be observable in a joined
//! artifact. These constants pass a *different* test, and the distinction is worth stating rather
//! than blurring: nobody writes them. They are emitted by a build script, and the whole point of
//! the seam decision is that there is **exactly one** build script in the workspace reading
//! `BOYKO_PROFILE`. A build script can only set `cargo::rustc-env` for the crate it belongs to, so
//! "one script, read by two subsystems" forces that script to the bottom of the graph — here. Two
//! scripts is the failure being avoided: a binary that prints a ceiling its profile does not name.
//!
//! # This module holds INTEGERS, not the subsystems' types
//!
//! `boyko_diag` explicitly does not own the level model or the tier model — those are logging's and
//! the profiler's taxonomies, and importing either here would make the bottom crate name a
//! consumer's type. So the constants are plain `u8`, and each consumer maps its own const on top:
//!
//! ```ignore
//! // in boyko_log
//! pub const GLOBAL_CEILING: Level = Level::from_raw(boyko_diag::profile::LOG_CEILING);
//! ```
//!
//! That mapping is a `const fn` over a `const`, so it folds at every call site and the emission
//! macros' gate (b) is still a compile-time constant — the property the whole two-axis design
//! rests on.
//!
//! # Generated since J1
//!
//! Every value below comes from [`crates/boyko_diag/build.rs`](../../../build.rs), the **first and
//! only build script in this workspace**, which reads `BOYKO_PROFILE` and writes one table into
//! `OUT_DIR`. The `pub const`s here re-state that table so the documentation stays in the source
//! rather than in generated text, and so a reader of this file sees what each constant *does*
//! rather than what it happens to equal in one profile.
//!
//! Until J1 the values were hand-written at the `dev` row, and this module was the seam J1 replaces
//! the **body** of — not a symbol J1 had to move between crates. That is why it was landed in its
//! final home at L0 rather than in `boyko_log` and relocated later, and it is why this rung changed
//! no consumer's import path.
//!
//! **Only the constants a landed rung actually reads are declared** — a constant nothing reads is a
//! value nothing can prove wrong. The rule was tested twice and held both times. Before J1 it kept
//! `GLOBAL_TIER` out until profiling rung 1 read it: adding it earlier would not even have made
//! that rung smaller, since its gate
//! (`const { $h::TIER as u8 <= GLOBAL_TIER as u8 } && ARM_MASK…`), the `ZoneTier` enum, `ARM_MASK`
//! and the `ZoneHandle`/`mod`-companion pair `declare_zone!` emits all arrive together or none of
//! them compiles. At J1 it keeps `BOYKO_BUILD_HASH` out, which `SEAM.md` §S9 lists and which
//! **nothing in this workspace has ever read** — rung 7 measured its absence and rung 13
//! re-measured it, shipping `HEADER_FLAG_INVARIANT_TSC` in its place.
//!
//! `LANE_COUNT` is **deliberately not here and never will be**: it lives in [`crate::lane`] with no
//! profile axis at all, because it indexes `boyko_threadpool::MAX_WORKERS`, which is
//! unconditional. Putting it on this axis is the exact unsoundness that was removed.
//!
//! # What this axis can enforce but cannot set
//!
//! `boyko_ecs`'s `profiling-analysis` is a **cargo feature**, and cargo resolves features before
//! any build script runs — so no value emitted here can turn it on or off. The axis therefore
//! publishes [`ANALYSIS_ADMITTED`] and the crate that owns the feature refuses a disagreement at
//! compile time. Three things were measured while landing that, and together they are why the
//! feature had to change shape rather than merely gain a check:
//!
//! - `boyko_ecs` was the **only** workspace member with a non-empty `default` feature list, and it
//!   contained exactly `profiling-analysis`.
//! - `cargo tree --workspace -e features --no-default-features` **still reported the feature
//!   enabled**. **Nine** sibling manifests depend on `boyko-ecs`, not one of them says
//!   `default-features = false`, and feature unification puts the default back — so with the
//!   feature default-on there was no command line at all that could turn it off, and the axis's
//!   `shipping` row would have been unenforceable.
//! - An *explicit* `features = […]` on a dependency edge survives `--no-default-features` too, so
//!   moving the request from the default list onto an edge does not help either.
//!
//! The feature is therefore **opt-in**: `boyko_ecs` declares `default = []`, and a build that wants
//! the concurrency report passes `--features boyko-ecs/profiling-analysis`, exactly as it already
//! passes `hwrt` or `bench-alloc`. That is the only shape in which the flag means what it says.
//!
//! (The count nine is itself a small lesson worth keeping: a first pass anchored on
//! `^boyko-ecs = { path` found **eight**, because `boyko_ui` aligns its table with extra spaces.
//! The filter's shape decided the count, which is the same defect class as a test-target filter
//! deciding what a sweep covered.)

/// The generated table, one row of the axis, written by `build.rs` into `OUT_DIR`.
///
/// It is a private module rather than a set of `pub use`s so that every constant this crate exports
/// is declared *below* with its documentation. A generated `pub const` would carry a generated doc
/// comment, and the paragraphs explaining what these values DO are the reason this file is worth
/// reading at all.
mod generated {
    include!(concat!(env!("OUT_DIR"), "/profile_axis.rs"));
}

/// The profile this build was configured with: one of `dev`, `editor`, `shipping`, `shipping-min`,
/// `off` or `custom`.
///
/// # This is not what an artifact prints
///
/// A telemetry or log header carries the profile as the **integers below**, never as this name.
/// Profiling rung 13 measured why and the reasoning has not changed: `LOG_CEILING`,
/// `PROFILING_TIER` and `REGION_CAPACITY` are what a profile materially IS, a name is a label
/// somebody chose for a row, and `custom` is a name that describes no particular row at all. The
/// name exists for the one question the integers cannot answer — *did the build script produce the
/// row that was asked for?* — which is J1's own gate and the only reader this constant has.
pub const PROFILE_NAME: &str = generated::PROFILE_NAME;

/// The logging severity ceiling for this build, as a raw [`u8`].
///
/// `0 = Off, 1 = Error, 2 = Warn, 3 = Info, 4 = Debug, 5 = Trace` — the numbering is
/// `boyko_log::Level`'s, which this crate does not name. The rows are
/// `dev` 5 · `editor` 4 · `shipping` 3 · `shipping-min` 2 · `off` 0.
///
/// A site whose level exceeds this value is deleted along with its argument expressions: the
/// emission macro's second gate is `GLOBAL_CEILING as u8 >= LVL as u8`, and a `const false` in an
/// `&&` chain removes the arm entirely. That is the only mechanism that reaches *zero* per-site
/// cost — a runtime flag has to be read in order to be a flag.
pub const LOG_CEILING: u8 = generated::LOG_CEILING;

/// The profiling tier ceiling for this build, as a raw [`u8`].
///
/// `0 = Always, 1 = Dev, 2 = Deep` — the numbering is [`crate::profiling_abi::ZoneTier`]'s, and it
/// is defined one layer up for the same reason [`LOG_CEILING`] is a `u8`: this module holds
/// integers, not the consumers' taxonomies. The rows are
/// `dev` 2 · `editor` 1 · `shipping` / `shipping-min` / `off` 0.
///
/// **There is no `Off` position**, and that is a property of `ZoneTier`, not an oversight of the
/// axis: its three values are `Always`, `Dev` and `Deep`, so even the lowest ceiling admits every
/// `Always` site. `BOYKO_PROFILE=off` therefore turns the **logger** off — [`LOG_CEILING`] is 0 and
/// `boyko_log::LANE_ARRAY_LEN` becomes zero-length — and leaves the profiler at its floor. Removing
/// the profiler's sites at compile time is the FEATURE axis's job (`G1`), and no `profiling`
/// feature exists in this workspace; removing its *cost* at run time is `ARM_MASK`'s (`GJ1`).
///
/// A zone whose declared tier exceeds this value is deleted **codegen-wise**: the gate is
/// `const { $h::TIER as u8 <= GLOBAL_TIER as u8 } && …`, and a `const false` in an `&&` chain
/// removes the arm. Note the asymmetry with the logging ceiling, which matters to anyone reasoning
/// about typos: the tier fold deletes *codegen*, not *tokens* — the expansion still names its
/// handle, twice — so a mistyped zone identifier is a hard `E0425` in **every** profile, retail
/// included, not only in the one whose tier admits it.
pub const PROFILING_TIER: u8 = generated::PROFILING_TIER;

/// Samples one lane region holds before it starts refusing, for this build.
///
/// `dev` / `editor` 1024, `shipping` / `shipping-min` / `off` 128. Read by the sample transport,
/// which is why it appears at all — a constant nothing reads is a value nothing can prove wrong.
///
/// The figure is burst headroom, not throughput: at ~400 engine samples per frame against a fold
/// that runs every frame, 1024 is **2.5 frames**. It is down from an earlier 2048 because the slab
/// is `LANE_COUNT × 2 regions × REGION_CAPACITY × 24 B`, and 2048 would put the `dev` slab at
/// 7.5 MiB against a 7 MiB budget. A shortfall is visible rather than silent: every refused sample
/// increments a per-region `overflow` the fold reports.
pub const REGION_CAPACITY: u32 = generated::REGION_CAPACITY;

/// Engine zone id slots — also the profiling store's zone stride.
///
/// `dev` / `editor` 4096, `shipping` / `shipping-min` / `off` 256. **MEASURED at this rung: the
/// engine declares 43 static zones**, so the shipping row leaves room for roughly five times what
/// exists today while cutting the store's per-frame column stride sixteenfold. The mint refuses
/// rather than aliases when it runs out (`ZONE_ID_EXHAUSTED`), so an overrun is counted, not silent.
///
/// It arrives on the axis at J1 and not before, and the constant it replaced said so in as many
/// words: *"sized here rather than per profile at this rung: nothing reads a profile value for it
/// yet"*. `profiling_abi::ENGINE_ZONE_SLOTS` re-exports this and stays the name every consumer
/// imports, so nothing downstream changed shape.
pub const ENGINE_ZONE_SLOTS: usize = generated::ENGINE_ZONE_SLOTS;

/// The ceiling on ids a `User`-partition crate may mint.
///
/// `dev` / `editor` 3072, `shipping` / `shipping-min` / `off` 512. A **cap, not a reservation**:
/// what a session actually spends is `ProfilerConfig::user_zone_budget` (default `0`), which sizes
/// the store's columns; this sizes the two `.bss` arenas and the upper half of the registry, and it
/// is what a game cannot exceed however it is configured.
pub const MAX_USER_BUDGET: usize = generated::MAX_USER_BUDGET;

/// The dynamic-zone name arena, in bytes.
///
/// `dev` / `editor` 64 KiB, `shipping` / `shipping-min` / `off` 8 KiB. Deliberately **not**
/// `MAX_USER_BUDGET × some_max_name_len`: names are wildly uneven, so a per-name ceiling either
/// truncates the long ones or reserves for a worst case no game reaches. One shared arena spends
/// the bytes where the names are.
pub const DYN_NAME_BYTES: usize = generated::DYN_NAME_BYTES;

/// Whether this profile admits `boyko_ecs`'s `profiling-analysis` cargo feature.
///
/// `true` for `dev` and `editor`, `false` for the three others. **The axis cannot switch the
/// feature** — cargo resolves features before any build script runs — so this constant exists for
/// exactly one purpose: `boyko_ecs` asserts at compile time that the feature and the profile agree,
/// and a `shipping` build with the analysis half still compiled in fails to build instead of
/// shipping symbols its own profile says are absent. See the module docs for the three measurements
/// that made the feature opt-in rather than default-on.
pub const ANALYSIS_ADMITTED: bool = generated::ANALYSIS_ADMITTED;

// NO assertion that this value is in range, and NO pin on the number itself. The two are refused
// for different reasons, and the difference is the one worth keeping:
//
// - *"the value names a real level"* **cannot fail**. It is enforced where the constant is
//   consumed: `boyko_log`'s `GLOBAL_CEILING` is `Level::from_raw(LOG_CEILING)`, a `const fn` that
//   `panic!`s outside `0..=5`, and a `panic!` in a `const` initialiser is a **compile error**. An
//   assertion here would fire strictly later than a build that already failed — clippy's
//   `assertions_on_constants` says the same thing about `assert!(5 <= 5)`, and it is right. Same
//   call `lane.rs` records for its deleted sentinel assert.
// - *"the value is 5"* **can** fail — an edit to this line compiles fine and silently deletes
//   every `debug!` and `trace!` in the engine. That one is worth pinning, but it is pinned **once,
//   at the consumer, as the consequence rather than the encoding**: `boyko_log`'s
//   `the_dev_profile_admits_every_level`. A second copy here would pin the number instead of what
//   the number does, and two statements of one fact is how this corpus has gone wrong before.
//
// J1 replaced this module's body with generated output, and the property that became checkable is
// **not** the one the paragraph above predicted. It said *"the generated value matches the profile
// that was requested"* would be J1's gate. Written here it would be a comparison of the build
// script's output against `option_env!("BOYKO_PROFILE")` — and this crate's test binary carries no
// `rerun-if-env-changed` directive of its own, so that `option_env!` can be **stale relative to the
// very table it is checking**: change the variable, `boyko_diag` rebuilds and the test binary's
// captured literal does not. A check whose two sides can disagree for a reason unrelated to the
// property is not a check.
//
// What IS honest here is the half a single build can see: the row is INTERNALLY CONSISTENT — every
// constant belongs to the same profile, not five constants from five rows. The other half — that
// the row is the one an operator asked for — belongs to whoever set the variable, and at J1 that is
// the census harness, which spawns the build itself and therefore knows its own input.

#[cfg(test)]
mod tests {
    use super::{
        ANALYSIS_ADMITTED, DYN_NAME_BYTES, ENGINE_ZONE_SLOTS, LOG_CEILING, MAX_USER_BUDGET,
        PROFILE_NAME, PROFILING_TIER, REGION_CAPACITY,
    };

    /// One row of the axis, spelled out rather than positional.
    ///
    /// A bare 8-tuple would compile and would be unreadable: `(3, 0, 128, 256, 512, …)` has two
    /// pairs of adjacent same-typed fields, and transposing either pair produces a table that still
    /// type-checks. Naming them is what makes a transposition visible to a reader — and clippy's
    /// `type_complexity` refuses the tuple anyway, which is the lint being right.
    #[derive(PartialEq, Eq, Debug)]
    struct Row {
        name: &'static str,
        log_ceiling: u8,
        profiling_tier: u8,
        region_capacity: u32,
        engine_zone_slots: usize,
        max_user_budget: usize,
        dyn_name_bytes: usize,
        analysis_admitted: bool,
    }

    /// The axis as `SEAM.md` §S9 tables it, restated here so the two can disagree.
    ///
    /// `custom` is deliberately absent — it is the one profile that does *not* have a fixed row,
    /// which is exactly what makes it the escape hatch.
    const TABLE: &[Row] = &[
        Row { name: "dev", log_ceiling: 5, profiling_tier: 2, region_capacity: 1024, engine_zone_slots: 4096, max_user_budget: 3072, dyn_name_bytes: 64 * 1024, analysis_admitted: true },
        Row { name: "editor", log_ceiling: 4, profiling_tier: 1, region_capacity: 1024, engine_zone_slots: 4096, max_user_budget: 3072, dyn_name_bytes: 64 * 1024, analysis_admitted: true },
        Row { name: "shipping", log_ceiling: 3, profiling_tier: 0, region_capacity: 128, engine_zone_slots: 256, max_user_budget: 512, dyn_name_bytes: 8 * 1024, analysis_admitted: false },
        Row { name: "shipping-min", log_ceiling: 2, profiling_tier: 0, region_capacity: 128, engine_zone_slots: 256, max_user_budget: 512, dyn_name_bytes: 8 * 1024, analysis_admitted: false },
        Row { name: "off", log_ceiling: 0, profiling_tier: 0, region_capacity: 128, engine_zone_slots: 256, max_user_budget: 512, dyn_name_bytes: 8 * 1024, analysis_admitted: false },
    ];

    /// The one thing about this module that is NOT decided by the compiler: that the constant is
    /// reachable through the crate's public path at all. A `pub mod` that stops being declared in
    /// `lib.rs` fails every consumer's build, but a `pub const` renamed inside a module that still
    /// exists fails only the consumers — and this crate's own test suite would stay green.
    #[test]
    fn the_ceiling_is_reachable_on_the_public_path() {
        let _: u8 = crate::profile::LOG_CEILING;
    }

    /// **J1's own gate**: whatever profile this build selected, all eight constants come from that
    /// profile's row and no other.
    ///
    /// The failure this catches is specific and is the reason a generated table is more dangerous
    /// than a hand-written one: a build script that mixes rows — a `shipping` ceiling beside a `dev`
    /// stride, because one `match` arm was edited and its neighbour was not — produces a binary that
    /// compiles, runs, and reports a profile it is not. Nothing else in the workspace would notice,
    /// because every consumer reads exactly one of these constants and each one is individually
    /// plausible.
    ///
    /// RED: change any single field of any row in `build.rs`'s table without changing this one.
    #[test]
    fn every_constant_comes_from_the_same_row_of_the_axis() {
        if PROFILE_NAME == "custom" {
            // `custom` selects no row by construction: it starts from `dev` and applies whichever
            // knobs are set, so "which row is this?" has no answer. The property that survives is
            // the one below.
            assert!(
                (LOG_CEILING as usize) < 6 && (PROFILING_TIER as usize) < 3,
                "custom produced LOG_CEILING={LOG_CEILING} PROFILING_TIER={PROFILING_TIER}, and \
                 one of them names no level or no tier"
            );
            return;
        }

        let row = TABLE.iter().find(|r| r.name == PROFILE_NAME).unwrap_or_else(|| {
            panic!(
                "the build script produced PROFILE_NAME={PROFILE_NAME:?}, which is not one of the \
                 five named profiles nor `custom`"
            )
        });

        let got = Row {
            name: PROFILE_NAME,
            log_ceiling: LOG_CEILING,
            profiling_tier: PROFILING_TIER,
            region_capacity: REGION_CAPACITY,
            engine_zone_slots: ENGINE_ZONE_SLOTS,
            max_user_budget: MAX_USER_BUDGET,
            dyn_name_bytes: DYN_NAME_BYTES,
            analysis_admitted: ANALYSIS_ADMITTED,
        };
        assert_eq!(
            got, *row,
            "the generated table does not match the {PROFILE_NAME} row: the constants this build \
             compiled against come from more than one profile, so nothing downstream can be said \
             to be built `shipping` or built `dev`"
        );
    }
}
