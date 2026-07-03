//! [`FixedSet`] — THE fixed-schedule ordering seam (host plan D4).
//!
//! The engine's interpolation pipeline depends on one cross-crate ordering
//! guarantee inside `CoreSchedule::Fixed`: the per-substep snapshot systems
//! (`pack_gpu_transforms`, physics pose mirrors, …) must run AFTER the user's
//! Fixed gameplay wrote the substep's transforms — otherwise the render
//! interpolates one substep behind (a permanent one-substep lag). That edge is
//! pinned **by name**, not by topological accident:
//!
//! * user (and physics) Fixed gameplay systems join [`FixedSet::Gameplay`];
//! * engine per-substep snapshot systems join [`FixedSet::Snapshot`];
//! * `boyko_app::EnginePlugins` wires `Snapshot.after(Gameplay)` in
//!   `CoreSchedule::Fixed`.
//!
//! The set lives HERE (the scene crate — the bottom of the spatial dependency
//! DAG) so both the producers (user gameplay, `boyko_physics`) and the
//! consumers (`boyko_render`'s pack, `boyko_app`) can name it without a cycle.

/// The two named phases of `CoreSchedule::Fixed` (host plan D4).
///
/// `EnginePlugins` configures `Snapshot.after(Gameplay)`, so every member of
/// `Snapshot` observes the substep's FINAL gameplay-written state. Membership:
///
/// * [`Gameplay`](FixedSet::Gameplay) — user simulation systems (and composed
///   physics steps): everything that WRITES per-substep world state. Register
///   with `.in_set(FixedSet::Gameplay)`.
/// * [`Snapshot`](FixedSet::Snapshot) — engine systems that READ the substep's
///   final state into render-facing snapshots (`pack_gpu_transforms` joins in
///   host plan R5; this rung ships the seam + the wiring).
///
/// A Fixed system registered in NEITHER set is unordered relative to the seam
/// — put gameplay in `Gameplay` or the interpolation pack may read a stale
/// pose (the exact one-substep lag D4 exists to prevent).
#[derive(boyko_macros::SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FixedSet {
    /// User/physics Fixed gameplay — writes the substep's world state.
    Gameplay,
    /// Engine per-substep snapshots — read the substep's FINAL state
    /// (`pack_gpu_transforms` in R5). Wired `.after(Gameplay)`.
    Snapshot,
}

/// The two named phases of the `Main`-schedule camera pipeline (host plan R6).
///
/// The interactive-camera path has the same cross-crate ordering shape as the
/// `Fixed` interpolation seam: a camera *controller* (a fly / orbit driver) must
/// write its entity's [`Transform`](crate::transform::Transform) BEFORE
/// [`propagate_transforms`](crate::propagation::propagate_transforms) recomposes
/// its [`GlobalTransform`](crate::transform::GlobalTransform) and
/// [`resolve_active_camera`](crate::camera::resolve_active_camera) derives the
/// [`ViewUniform`](crate::camera::ViewUniform) — otherwise the view lags the
/// input by one frame. But the controller lives in a DIFFERENT crate/plugin than
/// the propagation+resolve pair (`fly_camera_system` is wired by
/// `boyko_app::FlyCameraPlugin`, while `propagate_transforms` +
/// `resolve_active_camera` are wired by [`CameraPlugin`](crate::camera_plugin::CameraPlugin)),
/// so their per-system `SystemKey`s are not co-visible: the edge is pinned **by
/// name**, set-to-set, so it holds REGARDLESS of plugin add-order.
///
/// * a camera controller (`fly_camera_system`) joins [`Control`](CameraSet::Control);
/// * `propagate_transforms` + `resolve_active_camera` join
///   [`Resolve`](CameraSet::Resolve);
/// * [`CameraPlugin`](crate::camera_plugin::CameraPlugin) configures
///   `Control.before(Resolve)`.
///
/// A controller that joins NEITHER set is unordered relative to the resolve and
/// may drive a one-frame-stale view — join `Control`.
#[derive(boyko_macros::SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraSet {
    /// Camera CONTROLLERS — systems that WRITE a camera entity's `Transform`
    /// from input/animation (`fly_camera_system`). Register with
    /// `.in_set(CameraSet::Control)`.
    Control,
    /// Camera RESOLUTION — `propagate_transforms` + `resolve_active_camera`,
    /// which READ the controller's written pose into the world transform + the
    /// derived `ViewUniform`. Wired `.after(Control)` via `Control.before(Resolve)`.
    Resolve,
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_ecs::ecs::core::schedule::system_set::SystemSet;

    /// The enum derive assigns distinct discriminants per variant (set identity
    /// is `(TypeId, discriminant)`), so `Gameplay` and `Snapshot` are DISTINCT
    /// sets — the D4 `after` edge is meaningful, not a self-edge.
    #[test]
    fn variants_are_distinct_sets() {
        assert_ne!(
            FixedSet::Gameplay.set_discriminant(),
            FixedSet::Snapshot.set_discriminant()
        );
    }

    /// Same identity property for the R6 camera seam: `Control` and `Resolve`
    /// are DISTINCT sets, so the `Control.before(Resolve)` edge is meaningful.
    #[test]
    fn camera_set_variants_are_distinct() {
        assert_ne!(
            CameraSet::Control.set_discriminant(),
            CameraSet::Resolve.set_discriminant()
        );
    }
}
