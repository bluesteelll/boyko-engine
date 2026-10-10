//! The body set: the one row selection shared by every stage that pairs its rows with the
//! gathered snapshot by position (defect A5).
//!
//! # The rule
//!
//! [`physics_gather`](crate::systems::physics_gather) pushes one
//! [`BodyState`](crate::resources::BodyState) per walked row, so the dense row index is the
//! [`BodyIndex`](crate::manifold::BodyIndex).
//! [`physics_apply`](crate::systems::physics_apply) and
//! [`physics_soft_rigid_apply`](crate::soft::physics_soft_rigid_apply) count the rows of their
//! own walk by hand and write snapshot row `i` into the `i`-th entity they visit. That pairing is
//! correct only when all three stages walk the same entities in the same order, so **every
//! stage that pairs rows by position takes a
//! [`BodyQuery`](crate::body_set::BodyQuery)**: a `Query` whose filter is fixed to
//! [`BodySetFilter`](crate::body_set::BodySetFilter). The data terms still differ by stage
//! (`Ref` in the gather, which must stay a read, and `Mut` in the two write-backs), which is why
//! the data side has its own check.
//!
//! Before this module each stage's row set was a side effect of what it read: the gather
//! required `RigidBody`, `RigidBodyMass` and `Collider`, both write-backs required only
//! `RigidBody`, and an entity carrying `RigidBody` without one of the other two moved every later
//! row of a write-back walk onto another body's snapshot row.
//!
//! # Why equal selections walk in equal order
//!
//! A query's archetype list (`matched_ids`) is only ever appended in ascending archetype id
//! (`boyko_ecs`, `iters/query_state.rs:236-262`). Archetype ids are not reused, and the one reset
//! (a world `clear`) bumps the structural generation, which empties the list and re-sweeps it in
//! ascending order. The list's only reorderer, the swap-remove `remove_matched_at`
//! (`query_state.rs:496`), is reached only from `QueryDataState::post_filter_matched`
//! (`iters/query/state.rs:358-392`), which drops an archetype only when a term's own archetype
//! test rejects what the include/exclude masks admitted. A table `With<C>` filter term tests
//! exactly its include bit, and every data term here either requires its component or always
//! matches, so that removal never fires: each stage's walk is a function of its
//! `(include, exclude)` masks and the live archetype set alone. The compile-time block below
//! rejects the terms that would break this: any filter term other than a table `With<C>` (an
//! `Or`, an enable term, a change-detection term, a dense `With`), and any data term that trims
//! archetypes (`AnyOf`) or rows (a dense `&T`).
//!
//! # What the filter costs
//!
//! A table `With<C>` is archetypal (`IS_ARCHETYPAL = !C::STORAGE_IS_DENSE`,
//! `iters/query/filter.rs:535`), so the cursor never calls its per-row `filter_fetch`, and its
//! `set_table_*` bodies sit entirely inside `if const { C::STORAGE_IS_DENSE }`
//! (`filter.rs:620-682`). No per-row or per-archetype instruction is added to a stage. What is
//! left is the filter's fetch initialisation once per iterator (two null-pointer pairs, 32 B) and
//! the archetype list itself, which can only get shorter. The filter declares reads of
//! `RigidBodyMass` and `Collider`; no system in this crate writes either.
//!
//! # The three checks, and what they miss
//!
//! 1. **Signature pins** (compile time): each stage's function item is coerced to a fn-pointer
//!    type spelled with its own data alias, so a stage that stops taking a
//!    [`BodyQuery`](crate::body_set::BodyQuery), or takes one over a data type other than its
//!    alias, fails to build here.
//! 2. **Trim-free terms** (compile time): the `const _` block asserts the conditions above for
//!    [`BodySetFilter`](crate::body_set::BodySetFilter) and for each of the three data aliases.
//! 3. **Data-side mask equality** (wire-up): `assert_body_set_agrees` runs once per
//!    `add_physics_*` call and panics if the three stages' `(include, exclude)` masks differ, for
//!    example because the gather gained a required read that
//!    [`BodySetFilter`](crate::body_set::BodySetFilter) does not name.
//!
//! Not caught:
//!
//! - (a) a future kernel data term that drops rows without setting `REQUIRES_POST_FILTER_TRIM`
//!   or `HAS_DENSE_INCLUDE`, which would break the kernel's own contract for those flags;
//! - (b) a structural change between the gather and a write-back within one schedule run, which
//!   only the write-backs' `debug_assert!` row counts detect;
//! - (c) a new stage that pairs rows by position without taking a
//!   [`BodyQuery`](crate::body_set::BodyQuery). Such a stage must take one, and gains a pin and
//!   an entry in [`body_walker_selections`](crate::body_set::body_walker_selections).

use std::fmt;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_mask::ComponentMask;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::data::{Mut, QueryData, Ref};
use boyko_ecs::ecs::core::iters::query::data_is_enabled::IsEnabled;
use boyko_ecs::ecs::core::iters::query::filter::{QueryFilter, With};
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Entities, Res, ResMut};
use boyko_ecs::ecs::core::time::FixedTime;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

use crate::components::{Collider, Kinematic, RigidBody, RigidBodyMass, Sensor, Simulated};
use crate::resources::{PhysicsConfig, SolverScratch};
use crate::soft::SoftRigidReaction;

/// The rows every position-pairing physics stage walks: entities carrying `RigidBody`,
/// `RigidBodyMass` and `Collider`.
///
/// `RigidBody` is not named here because every stage's data already requires it. Only table
/// `With<C>` terms are accepted (a private bound checked at compile time), so the filter adds no
/// per-row work and cannot reorder a walk; see the module docs.
pub type BodySetFilter = (With<RigidBodyMass>, With<Collider>);

/// A `Query` over the body set: `D` is the stage's data, the filter is [`BodySetFilter`].
pub type BodyQuery<'w, 's, D> = Query<'w, 's, D, BodySetFilter>;

/// The gather's data: the three body columns plus the three non-filtering per-row terms.
///
/// `Option<&Sensor>`, `IsEnabled<Simulated>` and `IsEnabled<Kinematic>` add no include bit and
/// never reject a row, so a body with `Simulated` off is still gathered (Encoding A). `Ref` keeps
/// the gather a read while exposing the added tick the row identity map records.
pub type BodyGatherData = (
    Ref<'static, RigidBody>,
    &'static RigidBodyMass,
    &'static Collider,
    Option<&'static Sensor>,
    IsEnabled<Simulated>,
    IsEnabled<Kinematic>,
);

/// `physics_apply`'s data: a write guard on `RigidBody`.
pub type BodyApplyData = Mut<'static, RigidBody>;

/// `physics_soft_rigid_apply`'s data: a write guard on `RigidBody`.
///
/// The same type as [`BodyApplyData`], named separately so each stage has its own edit point
/// and its own signature pin.
pub type BodySoftApplyData = Mut<'static, RigidBody>;

/// One stage's archetype selection, computed the way `QueryDataState::new` builds it.
///
/// Wire-up diagnostics only; lives on the stack (224 B, align 32).
#[derive(Clone, Copy, Debug)]
pub struct StageSelection {
    /// The stage's function name.
    pub stage: &'static str,
    /// The include bits contributed by the stage's data terms alone.
    pub data_include: ComponentMask,
    /// `data_include` plus the filter's include bits: the mask a matched archetype must contain.
    pub include: ComponentMask,
    /// The filter's exclude bits: the mask a matched archetype must not intersect.
    pub exclude: ComponentMask,
}

/// The first stage whose selection differs from the reference (entry 0) in
/// [`check_selections`].
#[derive(Debug)]
pub struct SelectionMismatch {
    /// The reference stage's name (entry 0).
    pub reference: &'static str,
    /// The differing stage's name.
    pub stage: &'static str,
    /// Include bits the reference requires and the stage does not.
    pub include_only_in_reference: ComponentMask,
    /// Include bits the stage requires and the reference does not.
    pub include_only_in_stage: ComponentMask,
    /// Whether the two exclude masks differ.
    pub exclude_differs: bool,
}

impl fmt::Display for SelectionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "body set disagreement: `{stage}` does not select the rows `{reference}` gathers \
             (components required only by `{reference}`: [{only_reference}]; required only by \
             `{stage}`: [{only_stage}]; exclude masks differ: {exclude}). Every stage that pairs \
             rows with the gathered snapshot by position must select the same rows: add the \
             component to `BodySetFilter`, or read it through `Option<&T>`",
            stage = self.stage,
            reference = self.reference,
            only_reference = MaskIds(&self.include_only_in_reference),
            only_stage = MaskIds(&self.include_only_in_stage),
            exclude = self.exclude_differs,
        )
    }
}

impl std::error::Error for SelectionMismatch {}

/// Lists the [`ComponentId`]s set in a mask, without allocating.
struct MaskIds<'a>(&'a ComponentMask);

impl fmt::Display for MaskIds<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for block in 0..8 {
            for bit in self.0.block(block).iter_ones() {
                if !first {
                    f.write_str(", ")?;
                }
                first = false;
                write!(f, "{}", ComponentId(block * 64 + bit))?;
            }
        }
        Ok(())
    }
}

/// The filter terms [`BodySetFilter`] may contain: table `With<C>` leaves and AND-tuples of them.
///
/// Private, so no other filter can be admitted. An `Or` is excluded on purpose: the kernel's
/// `ArchetypalQueryFilter` would admit it, and an `Or` trims archetypes after the mask match.
trait BodySetTerm: QueryFilter {}

impl<C: Component> BodySetTerm for With<C> {}
impl<A: BodySetTerm> BodySetTerm for (A,) {}
impl<A: BodySetTerm, B: BodySetTerm> BodySetTerm for (A, B) {}
impl<A: BodySetTerm, B: BodySetTerm, E: BodySetTerm> BodySetTerm for (A, B, E) {}
impl<A: BodySetTerm, B: BodySetTerm, E: BodySetTerm, G: BodySetTerm> BodySetTerm for (A, B, E, G) {}

/// Fails the build unless `F` selects whole archetypes by its masks alone.
const fn assert_trim_free_filter<F: BodySetTerm>() {
    assert!(
        F::IS_ARCHETYPAL,
        "BodySetFilter must be archetypal: a per-row filter term drops rows one at a time"
    );
    assert!(
        !F::HAS_DENSE,
        "BodySetFilter must not contain a dense `With<C>`: its membership is tested per row"
    );
}

/// Fails the build unless the data `D` keeps every row its include mask admits.
const fn assert_trim_free_data<D: QueryData>() {
    assert!(
        !D::REQUIRES_POST_FILTER_TRIM,
        "a body-set stage's data must not trim archetypes after the mask match (e.g. `AnyOf`)"
    );
    assert!(
        !D::HAS_DENSE_INCLUDE,
        "a body-set stage's data must not require a dense component: rows are dropped one at a time"
    );
}

const _: () = {
    assert_trim_free_filter::<BodySetFilter>();
    assert_trim_free_data::<BodyGatherData>();
    assert_trim_free_data::<BodyApplyData>();
    assert_trim_free_data::<BodySoftApplyData>();
};

// Signature pins: each stage must take its `BodyQuery` with its own data alias, or this file
// stops compiling. The full parameter list is spelled, so a signature edit in either place is a
// one-line edit of the matching alias here.
type GatherSignature = for<'w, 's, 'e, 'a, 'b, 'c> fn(
    BodyQuery<'w, 's, BodyGatherData>,
    Entities<'e>,
    ResMut<'a, SolverScratch>,
    ResMut<'b, PhysicsConfig>,
    Res<'c, FixedTime>,
);
type ApplySignature = for<'w, 's, 'a> fn(BodyQuery<'w, 's, BodyApplyData>, Res<'a, SolverScratch>);
type SoftApplySignature =
    for<'w, 's, 'a> fn(BodyQuery<'w, 's, BodySoftApplyData>, ResMut<'a, SoftRigidReaction>);

const _: GatherSignature = crate::systems::physics_gather;
const _: ApplySignature = crate::systems::physics_apply;
const _: SoftApplySignature = crate::soft::physics_soft_rigid_apply;

/// Computes one stage's selection from its data alias and [`BodySetFilter`], in the order
/// `QueryDataState::new` folds them (data include, filter include, filter exclude).
fn selection_of<D: QueryData>(world: &mut EcsMaster, stage: &'static str) -> StageSelection {
    let data_state = D::init_state(world);
    let filter_state = <BodySetFilter as QueryFilter>::init_state(world);

    let mut data_include = ComponentMask::new();
    D::aggregate_include(&data_state, &mut data_include);
    let mut include = data_include;
    <BodySetFilter as QueryFilter>::aggregate_include(&filter_state, &mut include);
    let mut exclude = ComponentMask::new();
    <BodySetFilter as QueryFilter>::aggregate_exclude(&filter_state, &mut exclude);

    StageSelection {
        stage,
        data_include,
        include,
        exclude,
    }
}

/// The selections of `[physics_gather, physics_apply, physics_soft_rigid_apply]`, read from
/// their data aliases and [`BodySetFilter`].
///
/// Setup and diagnostics only. Resolves the component ids the aliases name (registering them if
/// no system has yet) and allocates nothing.
pub fn body_walker_selections(world: &mut EcsMaster) -> [StageSelection; 3] {
    [
        selection_of::<BodyGatherData>(world, "physics_gather"),
        selection_of::<BodyApplyData>(world, "physics_apply"),
        selection_of::<BodySoftApplyData>(world, "physics_soft_rigid_apply"),
    ]
}

/// `Ok` when every entry's `(include, exclude)` equals entry 0's; otherwise the first entry
/// that differs. An empty or single-entry slice is `Ok`.
//
// `clippy::result_large_err`: the error is five inline fields (192 B) returned once per wire-up,
// on a cold path. Boxing it would add a heap allocation to the failure path for no gain.
#[allow(clippy::result_large_err)]
pub fn check_selections(selections: &[StageSelection]) -> Result<(), SelectionMismatch> {
    let Some((reference, rest)) = selections.split_first() else {
        return Ok(());
    };
    for selection in rest {
        let exclude_differs = selection.exclude != reference.exclude;
        if selection.include != reference.include || exclude_differs {
            return Err(SelectionMismatch {
                reference: reference.stage,
                stage: selection.stage,
                include_only_in_reference: reference.include.difference(&selection.include),
                include_only_in_stage: selection.include.difference(&reference.include),
                exclude_differs,
            });
        }
    }
    Ok(())
}

/// Panics unless the three position-pairing stages select the same rows.
///
/// Called once per wire-up by `insert_physics_resources`, the world half that both the builder
/// form (`add_physics_*`) and `PhysicsPlugin` drive. A hard `assert` rather than a
/// `debug_assert!`: a disagreement would otherwise write solved state into the wrong entities in
/// a release build, silently.
pub(crate) fn assert_body_set_agrees(world: &mut EcsMaster) {
    let selections = body_walker_selections(world);
    if let Err(mismatch) = check_selections(&selections) {
        body_set_disagreement(&mismatch);
    }
}

#[cold]
#[inline(never)]
fn body_set_disagreement(mismatch: &SelectionMismatch) -> ! {
    panic!("{mismatch}")
}
