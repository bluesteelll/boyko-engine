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
}
