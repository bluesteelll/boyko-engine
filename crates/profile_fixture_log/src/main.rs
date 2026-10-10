//! **G16(a)/(b)'s subject**: one binary, one `debug!` site, and nothing else that could emit.
//!
//! Built from this one source by the `dev` and `shipping` CI legs. `debug!`'s second gate is
//! `GLOBAL_CEILING as u8 >= Level::Debug as u8`; `shipping`'s ceiling is `Info` (3) and `Debug` is
//! 4, so the arm and its argument expressions are deleted and no `emit_impl` monomorphisation
//! reachable from this site survives. `dev`'s ceiling is `Trace` (5), so it must.
//!
//! # What this fixture CANNOT claim, and why the corpus asked for more than exists
//!
//! `SEAM.md`'s S9 and logging's G16 both specify that the fixture include a **dynamic** site
//! (`dyn_debug!`), because a dynamic site has no static-target gate (a) and `GLOBAL_CEILING` is the
//! only thing that deletes it — so a fixture without one leaves the dynamic path uncovered.
//! **MEASURED at this rung: `dyn_debug!` does not exist.** Dynamic targets are logging rung **L10**,
//! and this workspace's logger has landed through roughly L5 — `src/sample.rs`, `sink/binary.rs`,
//! `sink/request.rs`, `sink/crash.rs` and `bin/logdec.rs` are all absent. The clause is therefore
//! landed in its static form and its stated limit is real rather than rhetorical: **it says nothing
//! about a dynamic site**, and L10 owes the second site in this file.
//!
//! Two further pieces of G16 are not delivered here for the same reason, and they are named so the
//! gate is not read as covering them: clause (d) — the sink header carrying `build_profile`,
//! `runtime_preset` and `ceiling` as three independent fields, with a fixture proving the first and
//! second can differ in one binary — needs `LogRuntimePreset`, which is L17's own content and rests
//! on the sink lifecycle L13b–L16 build.

use boyko_log::{Level, Log, LogTarget};

fn main() {
    // MEASURED, and the fixture did not work without it. `debug!`'s THIRD gate is
    // `runtime_ceiling(<Log as LogTarget>::ID) >= Level::Debug`, and `CONTROL` is `.bss`-zero
    // (`Off`). Under the LTO link the census requires, LLVM sees that nothing in this whole program
    // ever writes that cell, folds the runtime gate to `false`, and deletes the site — in BOTH
    // profiles. The first version of this file printed `emit_impl = 0` under `dev` as well as under
    // `shipping`, which is a census with no subject and would have been read as a pass.
    //
    // Raising the target's runtime ceiling is what makes the dev leg a positive control. It is the
    // exact counterpart of `arm_scope` in the profiling fixture, and for the exact same reason: a
    // compile-ceiling gate can only be observed in a binary whose runtime gate is open.
    boyko_log::set_target_level(<Log as LogTarget>::ID, Level::Trace);

    // A `debug!` on an engine target whose STATIC_CEILING is `Trace`, so the only gate that can
    // delete this site is the per-profile `GLOBAL_CEILING` — which is the one under test. Choosing
    // a target with a lower static ceiling would make the site vanish in every profile and the
    // census inert in both legs.
    boyko_log::debug!(Log, "profile fixture debug site");

    // The ceiling, not the profile NAME, and the fixture depends on `boyko-log` alone so it could
    // not print the name if it wanted to. That is the right way round: across the five rows the
    // ceiling is 5 / 4 / 3 / 2 / 0, one per profile, so it identifies the build as precisely as a
    // name does — and unlike a name it is the thing the site's gate actually reads.
    println!("ceiling={}", boyko_log::GLOBAL_CEILING as u8);
}
