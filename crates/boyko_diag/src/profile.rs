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
//! # Hand-written until J1
//!
//! `crates/boyko_diag/build.rs` does not exist yet: it would be the **first build script in this
//! workspace**, it sits under every default member, and rung D0 refused to land a rebuild trigger
//! for all of them before anything read its output. Until the joint rung **J1** creates it, the
//! values below are hand-written at the `dev` profile's row and this module is the seam that J1
//! replaces the *body* of — not a symbol J1 has to move between crates, which is why it is landed
//! in its final home now rather than in `boyko_log` and relocated later.
//!
//! **Only the constants a landed rung actually reads are declared.** `GLOBAL_TIER`,
//! `REGION_CAPACITY`, `ENGINE_ZONE_SLOTS`, `MAX_USER_BUDGET` and `DYN_NAME_BYTES` belong to the
//! same axis and arrive with the rungs that read them. A constant nothing reads is a value nothing
//! can prove wrong.
//!
//! **This rule was tested and held.** Profiling rung 1 hosts `profiling_abi` here and wants
//! `GLOBAL_TIER` — the tier ceiling, `Deep` in `dev`, whose raw form would sit beside
//! [`LOG_CEILING`] as a second `u8`. Adding it *before* that rung would put a constant here with
//! no reader, which is the shape this module exists to refuse, and it would not make the rung any
//! smaller: `profiling_abi` cannot be landed in fragments. Its tier gate is
//! `const { $h::TIER as u8 <= GLOBAL_TIER as u8 } && ARM_MASK…`, so the const, the `ZoneTier`
//! enum, `ARM_MASK`, the `ZoneHandle`/`mod`-companion pair that `declare_zone!` emits and the
//! guard that reads them all arrive together or none of them compiles. The const is the *last*
//! piece of that rung, not the first.
//!
//! `LANE_COUNT` is **deliberately not here and never will be**: it lives in [`crate::lane`] with no
//! profile axis at all, because it indexes `boyko_threadpool::MAX_WORKERS`, which is
//! unconditional. Putting it on this axis is the exact unsoundness that was removed.

/// The logging severity ceiling for this build, as a raw [`u8`].
///
/// `0 = Off, 1 = Error, 2 = Warn, 3 = Info, 4 = Debug, 5 = Trace` — the numbering is
/// `boyko_log::Level`'s, which this crate does not name. **5 is the `dev` profile's row.**
///
/// A site whose level exceeds this value is deleted along with its argument expressions: the
/// emission macro's second gate is `GLOBAL_CEILING as u8 >= LVL as u8`, and a `const false` in an
/// `&&` chain removes the arm entirely. That is the only mechanism that reaches *zero* per-site
/// cost — a runtime flag has to be read in order to be a flag.
pub const LOG_CEILING: u8 = 5;

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
// When J1 replaces this module's body with generated output, a genuinely new property becomes
// checkable — *"the generated value matches the profile that was requested"*, a comparison between
// the build script's input and its output. That is J1's gate and cannot be written here.

#[cfg(test)]
mod tests {
    /// The one thing about this module that is NOT decided by the compiler: that the constant is
    /// reachable through the crate's public path at all. A `pub mod` that stops being declared in
    /// `lib.rs` fails every consumer's build, but a `pub const` renamed inside a module that still
    /// exists fails only the consumers — and this crate's own test suite would stay green.
    #[test]
    fn the_ceiling_is_reachable_on_the_public_path() {
        let _: u8 = crate::profile::LOG_CEILING;
    }
}
