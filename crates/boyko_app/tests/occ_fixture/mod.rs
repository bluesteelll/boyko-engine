//! **VG R3 piece 4 rung P4-4 — THE single insert site for the occlusion axis.**
//!
//! Every fixture in `crates/boyko_app/tests` that boots an `App` and wants the occlusion decision
//! arms it through THIS module: `vb_mesh.rs` (the binary all five occlusion PINS render through),
//! `vb_occ_mixed.rs` (G-P3-A/B/C), `vb_occ_split_gate.rs` (G2 and the disjunct's only executable
//! red), `hzb_engine_pyramid_gate.rs` (G5 / G-P3-E) and `vg_occ_split_timing.rs` (the channel-G
//! worker).
//!
//! # Why one module, and why not `vb_occ_mixed_scene`
//!
//! One edit here reaches the PINNED binary **and** every gate binary, which is what makes the
//! vacuity control a true sentence rather than a hope: deleting the `OcclusionConfig` insert from
//! [`arm_occlusion_with`] is ONE edit that leaves all five occlusion pins — and the cross-pin
//! equality guard — GREEN, while `vb_mesh_occ_pins_actually_split`, G2, the no-HZB leg and G-P3-B
//! all red. A hash of an image cannot see a split that stopped happening; those gates can.
//!
//! `vb_occ_mixed_scene` was the obvious home and is the wrong one, because two of its consumers do
//! not line up with the occlusion axis: `vb_occ_split_gate.rs` arms occlusion on the FIVE-SPHERE
//! scene and does not declare that module at all, while `vg_occ_verdict_census.rs` declares it and
//! boots no `App`. Folding the two axes into one module would force every consumer of one to
//! compile the other, and would leave `vb_occ_split_gate.rs` — the binary that owns G2 and the
//! disjunct's red — outside the single edit's reach, which is exactly the property the control
//! needs.
//!
//! # What moved here from shipping code
//!
//! The `BOYKO_VG_OCC_FORCE` decode, and its panic. Until rung P4-4 it was an `env::var` plus a
//! boot `panic!` inside `GpuSceneBundles::boot` — production code implementing a measurement
//! instrument, beside an arming that was an ECS-derived per-frame predicate. The decode belongs
//! where the instrument is used, and the panic belongs with the decode: a typo'd regime that
//! renders the DEFAULT while the operator believes it forced one is how a control gets reported
//! as green.
//!
//! # ⚠️ What arming here does NOT do
//!
//! It does not mark anything. The per-instance `OcclusionCulling` capability is a component in a
//! spawn flush, owned by each fixture's own setup system, and `OcclusionMode::Off` means *do not
//! test*, never *do not gather*. Arming the config on a scene that marks nothing produces a
//! frame with no candidates, not a frame with no config.

#![allow(dead_code)]

use boyko_app::OcclusionForce;
use boyko_app::prelude::App;
use boyko_render::{OcclusionConfig, OcclusionMode};

/// The env knob that arms the occlusion CONSUMER — `[vb_occ_split.env]`'s and every
/// `[vb_occ_mixed*.env]`'s own, and the same knob whose `== "1"` predicate puts
/// `OcclusionCulling` in the spawn bundle.
pub const ENV_OCC: &str = "BOYKO_VG_OCC";

/// The env knob that selects the diagnostic verdict override — `keep` / `late`, exactly the two
/// values `[vb_occ_mixed_keep.env]` and `[vb_occ_mixed_late.env]` carry.
pub const ENV_OCC_FORCE: &str = "BOYKO_VG_OCC_FORCE";

/// The ONE value [`ENV_OCC`] recognises.
///
/// ⚠️ The predicate is `== "1"` and nothing wider — the shipped contract, unchanged by rung P4-4.
/// Any other value, including a plausible-looking `"true"`, is FALSE and arms nothing. That used
/// to be silent; since P4-4 it is covered, because `vb_mesh_occ_pins_actually_split` re-runs each
/// pin's `[*.env]` block VERBATIM and asserts the frame split, so a pin whose knob spelled
/// anything else reds there instead of rendering a default scene under a confident name.
pub const OCC_ARMED_VALUE: &str = "1";

/// `true` iff this process was asked for the occlusion capability — the MARKER predicate.
///
/// Spelled once, here, for its two kinds of reader: the fixtures' setup systems, which put
/// `OcclusionCulling` in the spawn flush, and [`occlusion_from_env`] below, which turns the same
/// answer into the config. A second spelling would be a second text that can disagree about
/// whether a run is a marked run.
#[must_use]
pub fn occ_marked() -> bool {
    std::env::var(ENV_OCC).is_ok_and(|v| v == OCC_ARMED_VALUE)
}

/// Decodes [`ENV_OCC`] / [`ENV_OCC_FORCE`] into the two Resources' values.
///
/// * `BOYKO_VG_OCC == "1"` ⇒ `Some(OcclusionConfig { mode: TwoPhase })`; anything else ⇒ `None`,
///   which is the same answer as the default `Off` (a host that inserts nothing).
/// * `BOYKO_VG_OCC_FORCE` unset ⇒ [`OcclusionForce::None`]; `"keep"` / `"late"` ⇒ the two
///   overrides.
///
/// # Panics
///
/// On an UNKNOWN regime word — the panic the deleted shipping decode used to own, moved with it.
/// A silent `None` here would render the real decision while the operator believed a control was
/// forced, and the run would be reported green.
#[must_use]
pub fn occlusion_from_env() -> (Option<OcclusionConfig>, OcclusionForce) {
    let config = occ_marked().then_some(OcclusionConfig { mode: OcclusionMode::TwoPhase });
    let force = match std::env::var(ENV_OCC_FORCE) {
        Err(_) => OcclusionForce::None,
        Ok(word) => OcclusionForce::from_word(&word).unwrap_or_else(|| {
            panic!(
                "{ENV_OCC_FORCE}={word:?} is not a regime. Valid values are `{}` (defer nothing -- \
                 the zero control) and `{}` (defer every marked instance); unset selects the real \
                 occlusion decision (`{}`). A typo'd regime that rendered the default while the \
                 operator believed it forced one is how a control gets reported as green.",
                OcclusionForce::KeepAll.as_str(),
                OcclusionForce::DeferAll.as_str(),
                OcclusionForce::None.as_str(),
            )
        }),
    };
    (config, force)
}

/// **THE single insert site** for the occlusion axis across every fixture that boots an `App`.
///
/// ⚠️ This is the function the vacuity control edits. Deleting the `OcclusionConfig` insert below
/// is ONE edit; it leaves all five occlusion pins and the cross-pin equality guard GREEN, and reds
/// `vb_mesh_occ_pins_actually_split`, G2, `vb_occ_probe_dump_marked_no_hzb` and G-P3-B. Both
/// halves are the point: the green half is the measured statement of what the pinned corpus cannot
/// see.
///
/// Inserted AFTER `add_plugins`, so it overrides `OcclusionPlugin`'s `Off` default — the
/// post-plugins owner-override discipline every fixture in this directory follows for
/// `RenderPathConfig`.
///
/// [`OcclusionForce`] is inserted UNCONDITIONALLY, including its `None` variant, so that the
/// world's Resource set does not itself depend on the regime: the host reads it with
/// `try_resource` and an absent Resource IS `None`, so this changes no frame — it just means one
/// fixture cannot accidentally differ from another in *which Resources exist*.
pub fn arm_occlusion_with(app: &mut App, mode: OcclusionMode, force: OcclusionForce) {
    app.insert_resource(OcclusionConfig { mode });
    app.insert_resource(force);
}

/// The env-driven entry point: [`occlusion_from_env`] then [`arm_occlusion_with`].
///
/// An unmarked run arms `Off` explicitly rather than inserting nothing, which is the same frame
/// (`Off` and absent are one answer) and a better fixture: the disarmed leg then differs from the
/// armed one in the config's VALUE alone.
///
/// Gates that need a FIXED configuration — a worker whose regime is a property of the test rather
/// than of the environment — call [`arm_occlusion_with`] directly. Both routes pass through ONE
/// insert, which is what the vacuity control depends on.
pub fn arm_occlusion(app: &mut App) {
    let (config, force) = occlusion_from_env();
    let mode = config.map_or(OcclusionMode::Off, |c| c.mode);
    arm_occlusion_with(app, mode, force);
}
