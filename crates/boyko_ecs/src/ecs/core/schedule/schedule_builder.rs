//! [`ScheduleBuilder`] — user-facing schedule constructor.
//!
//! See Phase 9 plan §5.5 (with the Round 3 W-NEW-3 pre-destructure
//! pattern) and §14 Step 9 acceptance criteria. The builder collects
//! systems + ordering hints, then on [`ScheduleBuilder::build`]:
//!
//! 1. Initialises every system once (capturing its `Access` surface);
//! 2. Validates that the ordering DAG is acyclic via Tarjan SCC;
//! 3. Linearises systems via Kahn's topological sort;
//! 4. Hands the topologically-ordered descriptors to
//!    [`ConflictGraph::build`] (Wave 4 Step 10) for the per-system
//!    conflict bitsets + predecessor counts;
//! 5. Constructs a [`Schedule`] that the (Wave 5) executor can run.
//!
//! # Round 3 W-NEW-3 pre-destructure
//!
//! `build` immediately destructures `self` into its four fields. This
//! avoids the borrow conflicts of the Round 1 sketch (which had
//! `&mut self` mutating `descriptors` while the same `self` was being
//! used to read `order_edges`). The destructure also lets us move the
//! descriptor vec into `insert_sync_points` in the eventual Wave 5
//! Step 14 path without an extra clone.
//!
//! [`ScheduleBuilder::build`]: ScheduleBuilder::build
//! [`ConflictGraph::build`]: super::conflict_graph::ConflictGraph::build

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

use boyko_threadpool::ThreadPool;
use fixedbitset::FixedBitSet;

use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::schedule::conflict_graph::{ConflictGraph, SystemIndex};
use crate::ecs::core::schedule::ordering::{OrderingEdge, SetOrderEdge, SystemKey};
use crate::ecs::core::schedule::executor_scratch::ExecutorScratch;
use crate::ecs::core::schedule::schedule::{Schedule, SetConditionEntry};
use crate::ecs::core::schedule::system_box::{BoolSystem, SystemBox};
use crate::ecs::core::schedule::system_config::SystemConfig;
use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;
use crate::ecs::core::schedule::system_set::{SystemSet, SystemSetId};
use crate::ecs::core::state::states::States;
use crate::ecs::core::state::{StateEntry, apply_state_transition};
use crate::ecs::core::system::into_system::IntoSystem;
use crate::ecs::core::system::system::System;
use crate::ecs::core::system::system_kind::SystemKind;

/// Build-time soft cap on the expanded edge count (§2.5 / §13-P4). This is
/// a `debug_assert!`-only early-warning rail — it vanishes in release; the
/// real release bound is [`MAX_SYSTEMS_PER_SCHEDULE`] (which caps `N`, and
/// therefore the worst-case `N²` cartesian expansion). `1 << 20` (1M) is
/// the absolute ceiling at `N = 1024`.
const MAX_EXPANDED_EDGES: usize = 1 << 20;

/// Maximum number of systems a single [`Schedule`] can hold (plan §3 Q4 /
/// §13.6). The cap fits comfortably into the `u16` used by `SystemIndex`
/// and `pred_count`, and corresponds to ~2 KB of `pred_remaining` data
/// plus ~128 KB of conflict bitsets — both well within L2.
pub const MAX_SYSTEMS_PER_SCHEDULE: usize = 1024;

/// Phase 17 — one recorded state registration, drained in
/// [`ScheduleBuilder::try_build`].
///
/// The builder cannot insert resources at `insert_state` call time (it holds
/// no `&mut EcsMaster`), so it records the request here and realises it at
/// `build`: `insert` performs `world.insert_state::<S>(initial)` (the captured
/// `initial` is moved into the boxed thunk), and `apply` is the monomorphised
/// [`apply_state_transition::<S>`] fn-pointer that becomes the built
/// [`StateEntry::apply`]. `type_id` keys the M2 idempotency dedup (first
/// registration of an `S` wins; later ones are no-ops); `type_name` is carried
/// for diagnostics. Mirrors the Phase-16 record-then-realise set-condition path.
struct StateRegistration {
    /// `TypeId::of::<S>()` — the M2 dedup key (one entry per `S`).
    type_id: TypeId,
    /// `type_name::<S>()` — diagnostics, copied into the built [`StateEntry`].
    type_name: &'static str,
    /// Inserts `State<S>`/`NextState<S>`/`StateTransitionRecord<S>` into the
    /// world, carrying the captured `initial`. Called once at `try_build`
    /// (cold), consuming the boxed closure.
    insert: Box<dyn FnOnce(&mut EcsMaster)>,
    /// Monomorphised `apply_state_transition::<S>` coerced to a plain fn
    /// pointer (a safe reified-fn coercion — no `unsafe`).
    apply: fn(&mut EcsMaster, u32, bool),
}

/// Builder for [`Schedule`]. Construct via [`ScheduleBuilder::new`];
/// chain `add_system(...).before(...).after(...)` calls; finalise with
/// [`build`](Self::build).
pub struct ScheduleBuilder {
    /// Pool reference. Cloned into the resulting [`Schedule`] on build.
    pub(crate) pool: Arc<ThreadPool>,

    /// Staging slot per system. Index in this vec == `SystemKey.0`.
    pub(crate) descriptors: Vec<SystemDescriptor>,

    /// `(TypeId(SystemSet), discriminant)` → `SystemSetId` interning
    /// (Phase 15 §13.1 R3-A). The first reference to a set (`in_set` /
    /// `before_set` / `configure_set`) allocates a fresh sequential id;
    /// subsequent references to the same type+discriminant return it.
    pub(crate) sets: HashMap<(TypeId, u32), SystemSetId>,

    /// `SystemSetId` → list of **direct** member [`SystemKey`]s (systems
    /// joined via `in_set`). Flattened transitively in `build` (D3).
    pub(crate) set_members: HashMap<SystemSetId, Vec<SystemKey>>,

    /// Set-level ordering edges collected by `before_set` / `after_set` /
    /// `ConfigureSet::before` / `ConfigureSet::after`, in declaration order
    /// (determinism — §13-P3). Expanded into `(SystemKey, SystemKey)` pairs
    /// at build by `expand_set_edges` (D1).
    pub(crate) set_ordering: Vec<SetOrderEdge>,

    /// Set hierarchy: child `SystemSetId` → its direct parent ids
    /// (`ConfigureSet::in_set`). A `Vec` of parents permits multi-set
    /// nesting (D3 / §4.1). Flattened transitively in `build`.
    pub(crate) set_parents: HashMap<SystemSetId, Vec<SystemSetId>>,

    /// `SystemSetId` → human-readable name (`set_name()`), for cycle /
    /// empty-set diagnostics (§6.3 / §13-P5). Populated on first intern.
    pub(crate) set_names: HashMap<SystemSetId, &'static str>,

    /// Phase 16 — set-level run conditions, keyed by [`SystemSetId`]. A set
    /// may accumulate multiple conditions (eager AND). Built into
    /// `Schedule::set_conditions` (the flat `SetConditionEntry` table) at
    /// build. The same `SystemSetId` keys both `set_members` and this map
    /// (the `set_id_of_value` intern path guarantees config-id == member-id).
    /// See `PHASE-16-PLAN.md` §2.3 / §8.2.
    pub(crate) set_conditions: HashMap<SystemSetId, Vec<BoolSystem>>,

    /// Phase 17 — recorded state-type registrations (`insert_state` /
    /// `init_state`), drained in `try_build`. Deduped by `TypeId` so each `S`
    /// yields exactly one `Schedule::state_entries` slot (M2 idempotency).
    state_registrations: Vec<StateRegistration>,
}

impl ScheduleBuilder {
    /// Constructs an empty builder bound to the given pool.
    #[inline]
    pub fn new(pool: Arc<ThreadPool>) -> Self {
        Self {
            pool,
            descriptors: Vec::new(),
            sets: HashMap::new(),
            set_members: HashMap::new(),
            set_ordering: Vec::new(),
            set_parents: HashMap::new(),
            set_names: HashMap::new(),
            set_conditions: HashMap::new(),
            state_registrations: Vec::new(),
        }
    }

    /// Registers a system. Returns a [`SystemConfig`] handle for fluent
    /// `.before(...)` / `.after(...)` / `.chain(...)` / `.in_set(...)`
    /// chaining.
    ///
    /// Systems are stored in insertion order; `SystemKey.0` equals the
    /// index in the descriptor vec at insertion time. Topological
    /// re-ordering happens in [`build`](Self::build).
    ///
    /// # Output bound
    ///
    /// Plan SCH10 / Q1 — only `Out = ()` systems flow through the
    /// scheduler. Non-unit-output systems use `EcsMaster::run_system`
    /// outside the schedule.
    pub fn add_system<F, M>(&mut self, system: F) -> SystemConfig<'_>
    where
        F: IntoSystem<(), (), M>,
        F::System: System<Out = ()> + 'static,
    {
        let sys = F::into_system(system);
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let system_box = SystemBox::new(boxed);
        let key = SystemKey(self.descriptors.len());
        self.descriptors.push(SystemDescriptor::new(system_box));
        SystemConfig {
            builder: self,
            key,
        }
    }

    /// Interns a system set value, returning a stable [`SystemSetId`].
    ///
    /// This is the **sole** intern entry point (Phase 15 §13-P1). The key
    /// is `(TypeId::of::<S>(), set.set_discriminant())`, so distinct enum
    /// variants of one type get distinct ids while a unit struct (or the
    /// same variant) always resolves to the same id. The set's
    /// `set_name()` is recorded alongside the id for diagnostics. First
    /// call allocates a fresh sequential id; subsequent calls return it.
    pub(crate) fn set_id_of_value<S: SystemSet>(&mut self, set: S) -> SystemSetId {
        let key = (TypeId::of::<S>(), set.set_discriminant());
        let next = self.sets.len();
        let id = *self.sets.entry(key).or_insert_with(|| SystemSetId(next));
        self.set_names.entry(id).or_insert_with(|| set.set_name());
        id
    }

    /// Begins configuring set `set` — order it relative to other sets, or
    /// nest it inside a parent set. Mirrors Bevy's `configure_sets`.
    ///
    /// Interns `set` (so even a memberless set gets an id, letting
    /// `before_set(set)` resolve), then returns a [`ConfigureSet`] handle
    /// for fluent `.before(other)` / `.after(other)` / `.in_set(parent)`
    /// chaining. All chained targets/parents are taken **by value** so
    /// enum-variant sets work uniformly (§13-P1).
    pub fn configure_set<S: SystemSet>(&mut self, set: S) -> ConfigureSet<'_> {
        let set_id = self.set_id_of_value(set);
        ConfigureSet {
            builder: self,
            set_id,
        }
    }

    /// Registers state type `S` with initial value `initial` (Phase 17 D7).
    ///
    /// Records the request; the resources (`State<S>`, `NextState<S>`,
    /// `StateTransitionRecord<S>`) are inserted into the world at
    /// [`build`](Self::build)/[`try_build`](Self::try_build) (the builder holds
    /// no `&mut EcsMaster` until then), which also adds the schedule-side
    /// [`StateEntry`] that fires the initial `OnEnter` and drains transitions
    /// each frame.
    ///
    /// # Idempotency
    /// Registering the **same** `S` more than once on the **same** builder is a
    /// no-op: the first registration wins (a later `initial` value is ignored),
    /// guaranteeing exactly one transition pass per `S` per frame. Dedup is by
    /// `TypeId::of::<S>()`.
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    pub fn insert_state<S: States>(&mut self, initial: S) -> &mut Self {
        let type_id = TypeId::of::<S>();
        // M2 idempotency: first registration of `S` wins. A duplicate would
        // push a second `StateEntry`, so the transition pass would synthesize
        // the initial `OnEnter` twice and drain `NextState<S>` twice per frame.
        if self
            .state_registrations
            .iter()
            .any(|r| r.type_id == type_id)
        {
            return self;
        }
        self.state_registrations.push(StateRegistration {
            type_id,
            type_name: std::any::type_name::<S>(),
            insert: Box::new(move |world: &mut EcsMaster| world.insert_state::<S>(initial)),
            apply: apply_state_transition::<S>,
        });
        self
    }

    /// Registers state type `S` using `S::default()` as the initial value
    /// (Phase 17 D7). Shorthand for `insert_state(S::default())`.
    ///
    /// # Idempotency
    /// Registering the **same** `S` more than once on the **same** builder is a
    /// no-op (the first registration wins) — see [`insert_state`](Self::insert_state).
    ///
    /// # Initial-transition interaction
    /// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
    /// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
    /// the synthesized `none → initial` transition is overwritten in the same
    /// first pass by the real `initial → requested` transition, so
    /// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
    /// does. Queue the first transition from *inside* a system (it lands on the
    /// next frame's pass) if you need the initial `OnEnter` to fire first.
    pub fn init_state<S: States + Default>(&mut self) -> &mut Self {
        self.insert_state(S::default())
    }

    /// Number of systems registered so far (pre-build).
    #[inline]
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// `true` iff no systems have been registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// Finalises the schedule, panicking on any build error.
    ///
    /// Thin wrapper over [`try_build`](Self::try_build): formats the
    /// [`ScheduleBuildError`] into a `boyko-B900x` panic (preserving the
    /// existing message scheme — `cycle_in_before_after_panics` matches
    /// `boyko-B9001`). Library/tool callers that want to surface schedule
    /// errors gracefully should call [`try_build`](Self::try_build) instead.
    ///
    /// The returned [`Schedule`] is **bound to `world`** (Phase 21): it
    /// records the world's `WorldId`, and `Schedule::run` panics
    /// (`boyko-B9101`) when handed a different world. Build a separate
    /// schedule per world.
    ///
    /// # Panics
    ///
    /// * Ordering cycle among systems (`boyko-B9001`) — the DAG built from
    ///   `.before/.after/.chain` **and the expanded set edges** contains an
    ///   SCC with > 1 node. The message lists the system names in the cycle.
    /// * Set-hierarchy cycle (`boyko-B9002`).
    /// * Two ordered sets share a member (`boyko-B9004`).
    /// * A `before/after`/`before_set` target indexes outside this builder
    ///   (`boyko-B9005`).
    /// * `descriptors.len() > MAX_SYSTEMS_PER_SCHEDULE` (`debug_assert!`,
    ///   plan §13.6 O-NEW-3).
    pub fn build(self, world: &mut EcsMaster) -> Schedule {
        match self.try_build(world) {
            Ok(schedule) => schedule,
            Err(e) => panic!("{}", e.formatted()),
        }
    }

    /// Finalises the schedule, returning a [`ScheduleBuildError`] on failure
    /// instead of panicking.
    ///
    /// The returned [`Schedule`] is **bound to `world`** (Phase 21) — see
    /// [`build`](Self::build).
    ///
    /// Round 3 W-NEW-3 mandates this body pre-destructure `self` so that the
    /// descriptor vec can be moved into successive phases without aliasing
    /// the builder.
    ///
    /// # Build pipeline (Phase 15)
    ///
    /// `Step 1` init → `Step 2` names → `Step 2.5` flatten set hierarchy
    /// (D3) → `Step 2.6` expand set edges (D1) → `Step 3` collect direct
    /// edges + endpoint validation → `Step 4` Tarjan SCC → `Step 5` Kahn →
    /// `Step 6-10` permute + ConflictGraph + scratch. The expanded set
    /// edges enter the **same** `dag_edges_keys` vec that Tarjan/Kahn
    /// consume — sets never become graph nodes (§2.4), so the executor and
    /// `ConflictGraph::build` are untouched.
    pub fn try_build(self, world: &mut EcsMaster) -> Result<Schedule, ScheduleBuildError> {
        // Round 3 W-NEW-3 pre-destructure: this gives independent
        // mutable handles into each field, lets us move `descriptors`
        // by value into downstream phases, and keeps the (now long)
        // build body readable. Phase 15 consumes the formerly-discarded
        // `sets` / `set_members` (plus the new `set_*` maps).
        let Self {
            pool,
            mut descriptors,
            sets,
            set_members,
            set_ordering,
            set_parents,
            set_names,
            mut set_conditions,
            state_registrations,
        } = self;

        // Step 1 — initialise every system. Allocation is allowed here
        // because the builder runs on the dispatcher with no workers
        // active (ALLOC2).
        for d in &mut descriptors {
            d.system_box.system.initialize(world);
            // SCH15 / Round 2 C9 / Phase 4 D5 + CR-B: refresh the cached
            // `kind` now that `Access` is filled in. `SystemBox::new`
            // recorded the post-construction value (often `CpuConcurrent`
            // before init); `initialize` may have mutated `meta.access`.
            // We re-resolve once here to freeze the truth at the same point
            // the executor will observe it.
            //
            // Resolution (CR-B): GpuCompute-marker FIRST → else
            // `is_universal()` ⇒ `CpuExclusive` → else `CpuConcurrent`.
            // Byte-identical to the previous `is_exclusive` derivation for
            // every non-GPU system (`is_gpu` defaults false). `GpuCompute`
            // carries NO access constraint — it is set by the explicit
            // marker, not derived from access.
            d.system_box.kind = if d.is_gpu {
                SystemKind::GpuCompute
            } else if d.system_box.system.access().is_universal() {
                SystemKind::CpuExclusive
            } else {
                SystemKind::CpuConcurrent
            };

            // Phase 16 (§2.5) — initialize this system's own conditions in the
            // SAME world pass, so each condition's `Access` + `Local` state are
            // live before the first frame. The per-frame `run_condition` call's
            // `initialize` is then the FS1 no-op.
            for cond in d.conditions.iter_mut() {
                cond.initialize(world);
                debug_assert_condition_read_only(cond.as_ref());
            }
        }

        // Phase 16 (§2.5) — initialize every set-level condition against the
        // same build-time world (a disjoint `&mut set_conditions` borrow,
        // separate from the `&mut descriptors` loop above). Done here, not in
        // `ConfigureSet::run_if` (which has no `world`), so set conditions are
        // ready before assembly.
        for conds in set_conditions.values_mut() {
            for cond in conds.iter_mut() {
                cond.initialize(world);
                debug_assert_condition_read_only(cond.as_ref());
            }
        }

        // Phase 17 — drain recorded state registrations (record-then-realise,
        // mirroring the set-condition path). For each registered `S`: insert its
        // backing resources into the world (the `insert` thunk carries the
        // captured `initial`), then build a `StateEntry { apply, pending_initial:
        // true, type_name }`. The list was already deduped by `TypeId` at
        // `insert_state`, so there is exactly one entry per `S`. States are
        // inserted now, before the schedule runs, so frame 1's transition pass
        // can fire each state's initial `OnEnter` (D7). Empty for a no-state
        // schedule ⇒ an empty `Vec` and the §6 pass early-outs.
        let mut state_entries: Vec<StateEntry> = Vec::with_capacity(state_registrations.len());
        for reg in state_registrations {
            let StateRegistration {
                type_name,
                insert,
                apply,
                ..
            } = reg;
            insert(world);
            state_entries.push(StateEntry {
                apply,
                pending_initial: true,
                type_name,
            });
        }

        // Step 2 — capture names BEFORE the descriptors move into later
        // phases. Diagnostics in `cycle_in_before_after_panics` rely on
        // this snapshot.
        let names: Vec<&'static str> = descriptors
            .iter()
            .map(|d| d.system_box.name)
            .collect();

        // Plan §13.6 O-NEW-3 (and §3 Q4): hard cap fits u16.
        debug_assert!(
            descriptors.len() <= MAX_SYSTEMS_PER_SCHEDULE,
            "Schedule cap MAX_SYSTEMS_PER_SCHEDULE = {} exceeded ({})",
            MAX_SYSTEMS_PER_SCHEDULE,
            descriptors.len()
        );
        debug_assert!(
            descriptors.len() <= u16::MAX as usize,
            "ScheduleBuilder::build: descriptors.len() must fit u16"
        );

        let n = descriptors.len();

        // Step 2.5 (D3) — flatten the set hierarchy into transitive leaf
        // membership. A set-hierarchy cycle is caught HERE (before any
        // system-edge Tarjan) because a membership cycle produces no
        // *system* edge and so would be invisible to the later SCC pass.
        let transitive_members =
            flatten_set_membership(&set_members, &set_parents, &sets, &set_names)?;

        // Step 2.6 (D1) — expand set-level ordering into system→system
        // pairs over the flattened membership. Empty sets warn (never
        // error); two ordered sets that share a member are a contradiction
        // and error early with a precise message (§13-P2).
        let expanded_set_edges =
            expand_set_edges(&set_ordering, &transitive_members, &set_names, &names)?;

        // Step 3 — collect raw DAG edges from each descriptor's ordering
        // hints, then append the expanded set edges into the SAME vec.
        let direct_edges = descriptors
            .iter()
            .flat_map(|d| d.ordering_hints.iter().filter_map(OrderingEdge::as_dag_edge));
        let mut dag_edges_keys: Vec<(SystemKey, SystemKey)> = direct_edges.collect();
        let n_direct = dag_edges_keys.len();
        dag_edges_keys.extend_from_slice(&expanded_set_edges);

        // §2.5 / §13-P4 build-time soft rail (debug-only). The real
        // release bound is `MAX_SYSTEMS_PER_SCHEDULE` capping N.
        debug_assert!(
            dag_edges_keys.len() <= MAX_EXPANDED_EDGES,
            "expanded edge count {} exceeds MAX_EXPANDED_EDGES {} (direct {} + set {})",
            dag_edges_keys.len(),
            MAX_EXPANDED_EDGES,
            n_direct,
            expanded_set_edges.len(),
        );

        // §6.2 — validate every endpoint is in range BEFORE Tarjan. A
        // foreign / stale `SystemKey` (from `before(SystemKey(9999))` or a
        // different builder with an in-range `.0`) would otherwise silently
        // mis-index `reorder[..]` in release. This upgrades that to a
        // precise build error in BOTH debug and release.
        for &(a, b) in &dag_edges_keys {
            if a.0 >= n {
                return Err(ScheduleBuildError::UnknownSystemKey { key: a, n });
            }
            if b.0 >= n {
                return Err(ScheduleBuildError::UnknownSystemKey { key: b, n });
            }
        }

        // Step 4 — Tarjan SCC for cycle detection on the COMBINED ordering
        // DAG (direct ++ expanded). A cycle through sets surfaces here as a
        // system-level SCC (§8.1 C1). `tarjan_scc` returns one Vec per SCC;
        // any SCC with > 1 node is a cycle.
        let sccs = tarjan_scc(n, &dag_edges_keys);
        for scc in &sccs {
            if scc.len() > 1 {
                // §6.3 — enrich the cycle with each system's set
                // memberships so a set-induced cycle names the sets.
                let systems = scc
                    .iter()
                    .map(|k| enrich_system_name(*k, &names, &set_members, &set_names))
                    .collect();
                return Err(ScheduleBuildError::OrderingCycle { systems });
            }
        }

        // Step 5 — topological sort via Kahn's algorithm.
        // `topo_order[new_index] = old_key.0` (mapping from post-sort
        // index back into the original `descriptors` vec).
        let topo_order = kahn_topological_sort(n, &dag_edges_keys);
        debug_assert_eq!(topo_order.len(), n, "Kahn's must produce a full ordering");

        // Step 6 — permute descriptors into topological order. We build
        // a `SystemIndex`-keyed edge list along the way: each old
        // `SystemKey.0` is mapped to its new index via `reorder`.
        let mut reorder = vec![0u16; n];
        for (new_idx, &old_key) in topo_order.iter().enumerate() {
            reorder[old_key.0] = new_idx as u16;
        }

        // Step 6.5 (Phase 16, §7.2) — build `system_gating_sets`, the
        // post-topo `SystemIndex → conditioned-set-ids` map. Reuses the
        // Phase-15 transitive membership (already cycle-checked, sorted,
        // deduped) inverted for the conditioned subset only: a set is
        // "gating" iff it carries at least one `.run_if` condition. Indexed
        // by post-topo index via `reorder`.
        let conditioned_sets: std::collections::HashSet<SystemSetId> =
            set_conditions.keys().copied().collect();
        let mut gating_by_new_idx: Vec<Vec<SystemSetId>> = vec![Vec::new(); n];
        for (&set_id, members) in &transitive_members {
            if !conditioned_sets.contains(&set_id) {
                continue;
            }
            for &member_key in members {
                let new_idx = reorder[member_key.0] as usize;
                gating_by_new_idx[new_idx].push(set_id);
            }
        }
        let system_gating_sets: Vec<Box<[SystemSetId]>> = gating_by_new_idx
            .into_iter()
            .map(|mut v| {
                v.sort_unstable_by_key(|s| s.0);
                v.dedup();
                v.into_boxed_slice()
            })
            .collect();

        // Permute the descriptors. We pop from a `Vec<Option<...>>` to
        // avoid double-moves while we iterate.
        let mut taking: Vec<Option<SystemDescriptor>> =
            descriptors.into_iter().map(Some).collect();
        let mut ordered: Vec<SystemDescriptor> = Vec::with_capacity(n);
        for &old_key in &topo_order {
            ordered.push(
                taking[old_key.0]
                    .take()
                    .expect("invariant: each descriptor consumed exactly once by topo sort"),
            );
        }
        debug_assert!(
            taking.iter().all(|opt| opt.is_none()),
            "invariant: every descriptor must be moved by the topo permutation"
        );

        // Step 7 — translate raw `SystemKey` edges to post-permutation
        // `SystemIndex` edges. Dedupe along the way — multiple
        // `.before(other).after(other)` chains AND set expansion (the
        // canonical duplicate source) can emit duplicates that would
        // otherwise inflate `pred_count` and trip the executor's underflow
        // `debug_assert!`.
        let mut dedup: std::collections::HashSet<(u16, u16)> = std::collections::HashSet::new();
        let mut dag_edges_idx: Vec<(SystemIndex, SystemIndex)> =
            Vec::with_capacity(dag_edges_keys.len());
        for &(from_key, to_key) in &dag_edges_keys {
            let from_idx = reorder[from_key.0];
            let to_idx = reorder[to_key.0];
            if dedup.insert((from_idx, to_idx)) {
                dag_edges_idx.push((SystemIndex(from_idx), SystemIndex(to_idx)));
            }
        }

        // Step 8 — sync-point insertion is the Wave 5 Step 14 deliverable.
        // For now this is a no-op pass-through; the descriptors and edges
        // flow straight into the ConflictGraph.
        let (descriptors_with_sync, dag_edges_with_sync) =
            insert_sync_points(ordered, dag_edges_idx);

        // Step 9 — ConflictGraph build (Wave 4 Step 10). The expanded set
        // edges arrive as ordinary `(SystemIndex, SystemIndex)` pairs — the
        // conflict graph is byte-identical to a hand-written equivalent set
        // of `.before` edges, so the executor hot path is untouched.
        let conflict_graph = ConflictGraph::build(&descriptors_with_sync, &dag_edges_with_sync);

        // Plan §13.6 O-NEW-3 secondary check: every pred_count fits u16.
        // The `pred_count: Box<[u16]>` type itself enforces the per-element
        // bound; `ConflictGraph::build` uses `checked_add` on each increment
        // so any overflow would have already panicked before reaching here.
        // The bound assertion is therefore implicit — no runtime check is
        // necessary, and the previous `c <= u16::MAX` form is tautological
        // at the `u16` type level (clippy::absurd_extreme_comparisons).

        // Step 10 — drop the descriptor envelope, keep the `SystemBox`es and
        // (Phase 16) move each descriptor's `conditions` Vec into
        // `system_conditions`, aligned by post-topo index since
        // `descriptors_with_sync` is already in topological order.
        let n_final = descriptors_with_sync.len();

        // §0-P3 sizing guard: `system_conditions` / `system_gating_sets` /
        // `has_condition` are indexed by post-topo `SystemIndex`, which today
        // equals the pre-sync index because `insert_sync_points` is the
        // IDENTITY stub (`n == n_final`). When Phase-9.1 sync-insertion injects
        // nodes, these arrays must be sized off `n_final` and indexed off the
        // post-sync descriptor order — revisit this assert then.
        debug_assert_eq!(
            n, n_final,
            "Phase 16 condition-array indexing assumes identity sync-insertion; \
             revisit when insert_sync_points injects nodes"
        );

        let mut systems: Vec<SystemBox> = Vec::with_capacity(n_final);
        let mut system_conditions: Vec<Vec<BoolSystem>> = Vec::with_capacity(n_final);
        for d in descriptors_with_sync {
            system_conditions.push(d.conditions); // move (no clone)
            systems.push(d.system_box);
        }

        // Phase 16 — flatten the set-condition map into the dense
        // `SetConditionEntry` table. Each row gets a dense `slot` (its index
        // in the table) used by the per-frame memo bitsets in
        // `ExecutorScratch`. Conditions were already `initialize`d in Step 1.
        let mut set_conditions_table: Vec<SetConditionEntry> = Vec::new();
        for (set_id, conds) in set_conditions {
            for condition in conds {
                let slot = set_conditions_table.len() as u16;
                set_conditions_table.push(SetConditionEntry {
                    set_id,
                    condition,
                    slot,
                });
            }
        }

        // Phase 16 (§2.5) — `has_condition[i]` set iff system `i` has any own
        // condition OR belongs to any conditioned (gating) set. THE 0%-GATE.
        let mut has_condition = FixedBitSet::with_capacity(n_final);
        for i in 0..n_final {
            if !system_conditions[i].is_empty() || !system_gating_sets[i].is_empty() {
                has_condition.insert(i);
            }
        }

        // Build the scratch *after* the conflict graph so we can seed
        // `pred_remaining` from `pred_count` in one pass. `set_conditions_table.len()`
        // sizes the set-condition memo bitsets (§7.1).
        let executor_scratch =
            ExecutorScratch::new(systems.len(), set_conditions_table.len(), &conflict_graph);

        Ok(Schedule {
            pool,
            systems,
            conflict_graph,
            executor_scratch,
            has_condition,
            system_conditions,
            system_gating_sets,
            set_conditions: set_conditions_table,
            state_entries,
            // Phase 16.1 (W2): overwritten at the top of every `Schedule::run`
            // (the frame-start `this_run`). The `ZERO` here is the pre-first-run
            // placeholder; no system/condition reads it before `run` sets it.
            frame_this_run: Tick::ZERO,
            // Phase 21 (H2): bind the schedule to its build world. `run`
            // release-panics (`boyko-B9101`) when handed any other world.
            world_id: world.world_id(),
        })
    }
}

/// Fluent handle for set-level ordering + hierarchy, returned by
/// [`ScheduleBuilder::configure_set`]. Mirrors Bevy's `configure_sets`.
///
/// All targets/parents are taken **by value** (Phase 15 §13-P1) so
/// enum-variant sets work uniformly with unit-struct sets. The handle
/// stores only the resolved [`SystemSetId`] of the set being configured
/// plus a borrow of the builder; each chained call interns its own fresh
/// argument value.
pub struct ConfigureSet<'a> {
    builder: &'a mut ScheduleBuilder,
    set_id: SystemSetId,
}

impl ConfigureSet<'_> {
    /// Returns the [`SystemSetId`] of the set being configured. Useful for
    /// tests that assert config and membership resolve to the same id.
    #[inline]
    pub fn id(&self) -> SystemSetId {
        self.set_id
    }

    /// Orders this set **before** `set`: every member of this set runs
    /// before every member of `set`.
    #[inline]
    pub fn before<T: SystemSet>(self, set: T) -> Self {
        let target = self.builder.set_id_of_value(set);
        self.builder
            .set_ordering
            .push(SetOrderEdge::SetBeforeSet(self.set_id, target));
        self
    }

    /// Orders this set **after** `set`: every member of this set runs after
    /// every member of `set` (recorded as `set`-before-this).
    #[inline]
    pub fn after<T: SystemSet>(self, set: T) -> Self {
        let target = self.builder.set_id_of_value(set);
        self.builder
            .set_ordering
            .push(SetOrderEdge::SetBeforeSet(target, self.set_id));
        self
    }

    /// Nests this set inside `parent`: every member of this set
    /// transitively joins `parent` (flattened in D3). Enables enum-variant
    /// set hierarchy (`configure_set(E::Child).in_set(E::Parent)`).
    #[inline]
    pub fn in_set<P: SystemSet>(self, parent: P) -> Self {
        let parent_id = self.builder.set_id_of_value(parent);
        self.builder
            .set_parents
            .entry(self.set_id)
            .or_default()
            .push(parent_id);
        self
    }

    /// Attaches a **run condition** to this set (Phase 16). Every transitive
    /// member of the set runs in a frame only if every set condition returns
    /// `true`.
    ///
    /// The set condition is evaluated exactly ONCE per frame (memoized), not
    /// once per member — so a stateful set condition advances its `Local`
    /// once per frame regardless of member count (`PHASE-16-PLAN.md` §7.1).
    /// Multiple `.run_if(a).run_if(b)` accumulate into an eager AND.
    ///
    /// The same read-only requirement as
    /// [`SystemConfig::run_if`](super::system_config::SystemConfig::run_if)
    /// applies. Change-detection conditions (`Changed<T>` / `Added<T>` /
    /// `Ref<T>`) work correctly here too: a set condition's tick snapshot is
    /// bumped once per frame by [`Schedule::run`] (Phase 16.1, B-1). Note the
    /// memoization interaction — the set condition's body runs at most once per
    /// frame, so its observation window advances per frame just like a system's.
    ///
    /// [`Schedule::run`]: super::schedule::Schedule::run
    #[inline]
    pub fn run_if<C, M>(self, condition: C) -> Self
    where
        C: IntoSystem<(), bool, M>,
        C::System: System<Out = bool> + 'static,
    {
        let sys = C::into_system(condition);
        let boxed: BoolSystem = Box::new(sys);
        self.builder
            .set_conditions
            .entry(self.set_id)
            .or_default()
            .push(boxed);
        self
    }
}

/// Sync-point insertion — Wave 5 Step 14 deliverable.
///
/// # Phase 9 (this revision): conservative pass-through
///
/// Plan §8 describes the Bevy-style auto-insertion algorithm:
///
/// 1. Detect every system that owns a `CommandQueue` `SystemParam`
///    (`has_deferred == true`).
/// 2. For each `(A, B)` DAG edge where `A` is deferred and `B` performs
///    structural reads, insert an `ApplyDeferred` exclusive system between
///    them and rewire the edge through it.
/// 3. Coalesce shared upstream cones to minimise the number of inserted
///    syncs.
///
/// The full implementation is **deferred to Phase 9.1**. Two prerequisites
/// are missing today:
///
/// * `SystemMeta`/`SystemBox` does not yet expose a `has_deferred()` query
///   — the flag must thread through `SystemParam::init_access` into a new
///   bit on `Access` or a sibling cache.
/// * `ApplyDeferred` is not yet a registered system type (its body is a
///   no-op; the dispatcher special-cases it by walking an `upstream`
///   `Vec<SystemIndex>` and calling `apply` on each). The infrastructure
///   exists in `ExclusiveFunctionSystem` + universal `Access` but the
///   marker has not been wired.
///
/// # Why the pass-through is correct (SCH7)
///
/// The apply window barrier (plan §2.2 SCH7 / §5.4.5.1) already
/// serialises every system's `apply` against every concurrent worker.
/// `Commands::add` enqueues into the system's own `CommandQueue` (which
/// is `!Sync`, per CQ-SEND2); the queue is flushed by
/// `SystemParam::apply` from the dispatcher inside the apply window.
/// Downstream systems run only after their predecessors' `apply` calls
/// have returned (the executor sets `completed[i]` AFTER `apply`).
///
/// Therefore: without explicit `ApplyDeferred` insertion, every
/// `Commands`-enqueued mutation is visible to every downstream system —
/// just at the cost of one extra dispatcher round per system that has
/// deferred work. The trade is "slightly less parallelism vs the full
/// Bevy algorithm", not "correctness".
///
/// # Phase 9.1 follow-up
///
/// The full algorithm is enumerated in plan §8.2; it will be wired
/// alongside change-detection ticks (Phase 10) when `has_deferred`
/// becomes a first-class SystemParam predicate.
///
/// # Phase 15 re-confirmation (§7)
///
/// Phase 15 only ADDS ordering edges (set membership / set-level ordering).
/// Each expanded edge gets a `pred_remaining` dependency + conflict bit
/// through the **unchanged** `ConflictGraph::build`, and the apply-window
/// barrier already serialises `apply` against every edge. So every new
/// edge inherits the same correct deferred-command-visibility semantics —
/// the no-op pass-through remains sound under the expanded edge set, and no
/// `ApplyDeferred` node is needed (nor permitted by the Phase 15 task).
#[inline]
fn insert_sync_points(
    descriptors: Vec<SystemDescriptor>,
    dag_edges: Vec<(SystemIndex, SystemIndex)>,
) -> (Vec<SystemDescriptor>, Vec<(SystemIndex, SystemIndex)>) {
    (descriptors, dag_edges)
}

/// Phase 16 CR1 (§8.5) — debug-only read-only contract check for a run
/// condition. A condition MUST declare no component / resource writes.
///
/// The check is build-time and elided in release (the access bitmask scan
/// vanishes). A write-declaring condition in release runs and mutates the
/// world, which is SOUND (it holds the exclusive `&mut` at the single-threaded
/// apply barrier) but is an API misuse; the assert turns it into a debug
/// panic — the right severity (Bevy forbids it at compile time via a
/// `ReadOnlySystem` bound; we forbid it via this assert + docs, deferring the
/// marker trait).
#[cfg(debug_assertions)]
#[inline]
fn debug_assert_condition_read_only(condition: &dyn System<Out = bool>) {
    let access = condition.access();
    debug_assert!(
        access.component_writes.is_empty() && access.resource_writes.is_empty(),
        "Phase 16 CR1: run condition '{}' declares writes; conditions must be read-only",
        condition.name(),
    );
}

/// Release no-op counterpart — the read-only check is debug-only, so release
/// builds skip the `Access` scan entirely.
#[cfg(not(debug_assertions))]
#[inline(always)]
fn debug_assert_condition_read_only(_condition: &dyn System<Out = bool>) {}

/// Standard Tarjan strongly-connected-components.
///
/// Returns one `Vec<SystemKey>` per SCC. Trivial (single-node) SCCs are
/// returned alongside non-trivial ones; callers filter for `len() > 1`
/// when detecting cycles.
///
/// # Implementation notes
///
/// Iterative — recursion would blow the stack on a 1024-system schedule.
/// The control stack stores `(node, child_iter_index)` so we can resume
/// after recursing into a child. Lowlink + on-stack state lives in
/// parallel arrays keyed by `SystemKey.0`.
fn tarjan_scc(n: usize, edges: &[(SystemKey, SystemKey)]) -> Vec<Vec<SystemKey>> {
    // Build an adjacency list (index = source key).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(from, to) in edges {
        adj[from.0].push(to.0);
    }

    // Per-node Tarjan state.
    const UNVISITED: u32 = u32::MAX;
    let mut index_of: Vec<u32> = vec![UNVISITED; n];
    let mut lowlink: Vec<u32> = vec![0; n];
    let mut on_stack: Vec<bool> = vec![false; n];

    let mut next_index: u32 = 0;
    let mut scc_stack: Vec<usize> = Vec::with_capacity(n);
    let mut sccs: Vec<Vec<SystemKey>> = Vec::new();

    // Iterative DFS: control stack frames are `(node, next_child_idx)`.
    // When we visit `node` for the first time we push the frame; each
    // step advances `next_child_idx` and either recurses (push child)
    // or pops (back-edge / unwind).
    let mut ctrl: Vec<(usize, usize)> = Vec::with_capacity(n);

    for start in 0..n {
        if index_of[start] != UNVISITED {
            continue;
        }
        // Begin visit at `start`.
        index_of[start] = next_index;
        lowlink[start] = next_index;
        next_index += 1;
        scc_stack.push(start);
        on_stack[start] = true;
        ctrl.push((start, 0));

        while let Some(&(node, child_pos)) = ctrl.last() {
            if child_pos < adj[node].len() {
                let child = adj[node][child_pos];
                // Advance the parent's child iterator first so the
                // back-edge unwind sees the right position.
                ctrl.last_mut().unwrap().1 += 1;
                if index_of[child] == UNVISITED {
                    // Tree edge — recurse.
                    index_of[child] = next_index;
                    lowlink[child] = next_index;
                    next_index += 1;
                    scc_stack.push(child);
                    on_stack[child] = true;
                    ctrl.push((child, 0));
                } else if on_stack[child] {
                    // Back edge — propagate lowlink.
                    lowlink[node] = lowlink[node].min(index_of[child]);
                }
                // (Forward / cross edges to nodes that are visited but
                // not on the SCC stack do not update lowlink — they
                // belong to already-emitted SCCs.)
            } else {
                // Finished `node`. If it is an SCC root, pop the SCC.
                let node_low = lowlink[node];
                let node_idx = index_of[node];
                ctrl.pop();
                if node_low == node_idx {
                    let mut component = Vec::new();
                    loop {
                        let top = scc_stack.pop().expect("SCC stack must contain root");
                        on_stack[top] = false;
                        component.push(SystemKey(top));
                        if top == node {
                            break;
                        }
                    }
                    sccs.push(component);
                }
                // Propagate lowlink back to the parent (if any).
                if let Some(&mut (parent, _)) = ctrl.last_mut() {
                    lowlink[parent] = lowlink[parent].min(node_low);
                }
            }
        }
    }

    sccs
}

/// Kahn's algorithm for topological sort.
///
/// Returns a permutation of `0..n` as a `Vec<SystemKey>`. The order is
/// stable for fixed input (the ready queue is a FIFO — ties break in
/// insertion order, which matches user expectation for "two unordered
/// systems appear in `add_system` order").
///
/// # Pre-condition
///
/// The caller has already validated that the DAG is acyclic (via
/// `tarjan_scc`). Kahn's will not detect a cycle directly; it would
/// simply produce a partial order shorter than `n`. The `debug_assert!`
/// in `build` catches that misuse.
fn kahn_topological_sort(n: usize, edges: &[(SystemKey, SystemKey)]) -> Vec<SystemKey> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_degree: Vec<u32> = vec![0; n];
    for &(from, to) in edges {
        adj[from.0].push(to.0);
        in_degree[to.0] += 1;
    }

    // FIFO ready queue; using `VecDeque` to preserve insertion order.
    let mut ready: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (idx, &d) in in_degree.iter().enumerate() {
        if d == 0 {
            ready.push_back(idx);
        }
    }

    let mut out: Vec<SystemKey> = Vec::with_capacity(n);
    while let Some(node) = ready.pop_front() {
        out.push(SystemKey(node));
        for &child in &adj[node] {
            in_degree[child] -= 1;
            if in_degree[child] == 0 {
                ready.push_back(child);
            }
        }
    }
    out
}

/// Errors produced by [`ScheduleBuilder::try_build`]. Phase 15 §6.1 / §13.1.
///
/// `build` formats these into a `boyko-B900x` panic; `try_build` returns
/// them. An interned-but-memberless set is **not** an error — it produces a
/// build warning and zero edges (§13.1 R3-C).
#[derive(Debug)]
#[non_exhaustive]
pub enum ScheduleBuildError {
    /// `before/after/chain` (or an expanded set relation) formed a cycle
    /// among systems. `systems` lists the names in the cycle, enriched with
    /// each system's set memberships (§6.3). Maps to `boyko-B9001`.
    OrderingCycle {
        /// System names (with set memberships appended) forming the cycle.
        systems: Vec<String>,
    },

    /// A set-hierarchy cycle (`configure_set(S).in_set(T)` +
    /// `configure_set(T).in_set(S)`, possibly multi-hop). `sets` lists the
    /// involved set names. Maps to `boyko-B9002`.
    SetHierarchyCycle {
        /// Set names forming the hierarchy cycle.
        sets: Vec<&'static str>,
    },

    /// Two sets ordered relative to each other share a (transitive) member,
    /// which would expand to a `sys → sys` self-edge (a trivial cycle).
    /// Caught with precise names before Tarjan. Maps to `boyko-B9004`.
    SetsOrderedButIntersect {
        /// The earlier-ordered set's name.
        a: &'static str,
        /// The later-ordered set's name.
        b: &'static str,
        /// A system present in both sets.
        shared: &'static str,
    },

    /// A `before(key)` / `after(key)` / `before_set` endpoint indexes
    /// outside this builder (foreign or stale `SystemKey`). Maps to
    /// `boyko-B9005`. This is the §6.2 silent-misindex fix.
    UnknownSystemKey {
        /// The out-of-range key.
        key: SystemKey,
        /// The number of systems registered in this builder.
        n: usize,
    },
}

impl ScheduleBuildError {
    /// Renders the error as a `boyko-B900x: …` string — the message body of
    /// the panic raised by [`ScheduleBuilder::build`].
    pub(crate) fn formatted(&self) -> String {
        match self {
            ScheduleBuildError::OrderingCycle { systems } => format!(
                "boyko-B9001: schedule contains a cycle of {} systems: {:?}",
                systems.len(),
                systems
            ),
            ScheduleBuildError::SetHierarchyCycle { sets } => format!(
                "boyko-B9002: set hierarchy contains a cycle of {} sets: {:?}",
                sets.len(),
                sets
            ),
            ScheduleBuildError::SetsOrderedButIntersect { a, b, shared } => format!(
                "boyko-B9004: sets '{a}' and '{b}' are ordered relative to each \
                 other but share member '{shared}' (a system cannot run both \
                 before and after itself)"
            ),
            ScheduleBuildError::UnknownSystemKey { key, n } => format!(
                "boyko-B9005: ordering references SystemKey({}) which is not in \
                 this schedule (it has {} systems); the key is foreign or stale",
                key.0, n
            ),
        }
    }
}

impl std::fmt::Display for ScheduleBuildError {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.formatted())
    }
}

impl std::error::Error for ScheduleBuildError {}

/// D3 — flatten the set hierarchy into transitive leaf membership.
///
/// Produces `transitive_members[S] = every system in S OR in any set nested
/// (directly or transitively) under S`. Seeds an **empty** `Vec` for every
/// interned set id (so a referenced-but-memberless set has an entry — the
/// empty-set warning in [`expand_set_edges`] always has a name to print,
/// §13.1 R3-B). Each result vec is **sorted ascending by `SystemKey.0` and
/// deduped** (determinism — §13-P3).
///
/// A **set-hierarchy cycle** is detected here via iterative DFS colour
/// marking (WHITE/GRAY/BLACK) before any transitive computation — a
/// membership cycle produces no *system* edge and would be invisible to the
/// later system-level Tarjan (§4.2). GRAY-revisit ⇒
/// [`ScheduleBuildError::SetHierarchyCycle`].
fn flatten_set_membership(
    direct_members: &HashMap<SystemSetId, Vec<SystemKey>>,
    set_parents: &HashMap<SystemSetId, Vec<SystemSetId>>,
    sets: &HashMap<(TypeId, u32), SystemSetId>,
    set_names: &HashMap<SystemSetId, &'static str>,
) -> Result<HashMap<SystemSetId, Vec<SystemKey>>, ScheduleBuildError> {
    let n_sets = sets.len();

    // Child graph: parent_id → its direct children. Built by inverting
    // `set_parents` (child → parents).
    let mut children: HashMap<SystemSetId, Vec<SystemSetId>> = HashMap::new();
    for (&child, parents) in set_parents {
        for &parent in parents {
            children.entry(parent).or_default().push(child);
        }
    }

    // Iterative post-order DFS with colour marking over the child graph.
    // `Color::Gray` means "on the current DFS path" → revisiting a Gray
    // node is a hierarchy cycle. Post-order guarantees each set's children
    // are fully computed before the set itself (memoised, computed once).
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut color: HashMap<SystemSetId, Color> = HashMap::with_capacity(n_sets);
    let mut transitive: HashMap<SystemSetId, Vec<SystemKey>> =
        HashMap::with_capacity(n_sets);

    // Seed an empty membership for EVERY interned set (§13.1 R3-B) and mark
    // every set White so the DFS visits ids reachable only as parents too.
    for &id in sets.values() {
        transitive.entry(id).or_default();
        color.entry(id).or_insert(Color::White);
    }
    // A parent referenced via `set_parents` is always interned (intern
    // happens in `configure_set`/`in_set` before recording the edge), so
    // `sets.values()` already covers it — no extra seeding needed.

    // Visit every set in ascending id order for deterministic traversal.
    let mut start_ids: Vec<SystemSetId> = sets.values().copied().collect();
    start_ids.sort_unstable_by_key(|s| s.0);

    // Control stack frames: (set, next_child_index). On first push we mark
    // Gray; on pop we fold children into the set and mark Black.
    let mut ctrl: Vec<(SystemSetId, usize)> = Vec::with_capacity(n_sets);

    for start in start_ids {
        if color[&start] != Color::White {
            continue;
        }
        color.insert(start, Color::Gray);
        ctrl.push((start, 0));

        while let Some(&(node, child_pos)) = ctrl.last() {
            let node_children = children.get(&node).map(Vec::as_slice).unwrap_or(&[]);
            if child_pos < node_children.len() {
                let child = node_children[child_pos];
                ctrl.last_mut().unwrap().1 += 1;
                match color[&child] {
                    Color::White => {
                        color.insert(child, Color::Gray);
                        ctrl.push((child, 0));
                    }
                    Color::Gray => {
                        // Back edge in the hierarchy → cycle.
                        return Err(ScheduleBuildError::SetHierarchyCycle {
                            sets: collect_cycle_set_names(&ctrl, child, set_names),
                        });
                    }
                    Color::Black => {} // already fully computed
                }
            } else {
                // Finished `node`: fold its direct members + every child's
                // transitive membership, then sort+dedup.
                ctrl.pop();
                let mut acc: Vec<SystemKey> =
                    direct_members.get(&node).cloned().unwrap_or_default();
                for &child in node_children {
                    if let Some(child_members) = transitive.get(&child) {
                        acc.extend_from_slice(child_members);
                    }
                }
                acc.sort_unstable_by_key(|k| k.0);
                acc.dedup();
                debug_assert!(
                    acc.windows(2).all(|w| w[0].0 < w[1].0),
                    "transitive_members must be sorted ascending + deduped (§13-P3)"
                );
                transitive.insert(node, acc);
                color.insert(node, Color::Black);
            }
        }
    }

    Ok(transitive)
}

/// Reconstructs the set names on the current DFS path from `start` (the
/// re-entered Gray node) to the stack top — the hierarchy cycle.
fn collect_cycle_set_names(
    ctrl: &[(SystemSetId, usize)],
    start: SystemSetId,
    set_names: &HashMap<SystemSetId, &'static str>,
) -> Vec<&'static str> {
    let from = ctrl
        .iter()
        .position(|&(s, _)| s == start)
        .unwrap_or(0);
    let mut out: Vec<&'static str> = ctrl[from..]
        .iter()
        .map(|&(s, _)| set_name_or_default(s, set_names))
        .collect();
    // Close the loop back to the start for readability.
    out.push(set_name_or_default(start, set_names));
    out
}

/// D1 — expand set-level ordering edges into `(SystemKey, SystemKey)` pairs
/// over the **transitive** membership (D3 output).
///
/// Each `SetOrderEdge` is expanded per §2.1:
/// * `SystemBeforeSet(X, S)` → `{X → sᵢ}` for every `sᵢ ∈ members(S)`.
/// * `SystemAfterSet(X, S)` → `{sᵢ → X}`.
/// * `SetBeforeSet(S, T)` → `{sᵢ → tⱼ}` (cartesian product).
///
/// `members(S)` defaults to `&[]` for a memberless set (never errors —
/// §13.1 R3-C). When an edge references an empty set, a single
/// `boyko-W15xx` warning is emitted off the **edge** iteration (§13.1 R3-B)
/// so a never-`in_set`'d target is caught loudly instead of silently
/// producing zero edges. Two ordered sets sharing a member is the one
/// error path ([`ScheduleBuildError::SetsOrderedButIntersect`]).
fn expand_set_edges(
    set_ordering: &[SetOrderEdge],
    transitive_members: &HashMap<SystemSetId, Vec<SystemKey>>,
    set_names: &HashMap<SystemSetId, &'static str>,
    names: &[&'static str],
) -> Result<Vec<(SystemKey, SystemKey)>, ScheduleBuildError> {
    let members = |s: SystemSetId| -> &[SystemKey] {
        transitive_members.get(&s).map(Vec::as_slice).unwrap_or(&[])
    };

    let mut out: Vec<(SystemKey, SystemKey)> = Vec::new();
    for e in set_ordering {
        match *e {
            SetOrderEdge::SystemBeforeSet(x, s) => {
                let m = members(s);
                warn_if_empty(m, s, set_names);
                for &sys in m {
                    out.push((x, sys));
                }
            }
            SetOrderEdge::SystemAfterSet(x, s) => {
                let m = members(s);
                warn_if_empty(m, s, set_names);
                for &sys in m {
                    out.push((sys, x));
                }
            }
            SetOrderEdge::SetBeforeSet(s, t) => {
                let ms = members(s);
                let mt = members(t);
                warn_if_empty(ms, s, set_names);
                warn_if_empty(mt, t, set_names);
                // A system transitively in BOTH sides would expand to a
                // `sys → sys` self-edge (a trivial cycle). Detect early with
                // a precise message rather than letting Tarjan report an
                // opaque SCC (§2.3). Both lists are sorted (D3), so a linear
                // merge finds the intersection in O(k + m).
                if let Some(shared) = first_shared(ms, mt) {
                    return Err(ScheduleBuildError::SetsOrderedButIntersect {
                        a: set_name_or_default(s, set_names),
                        b: set_name_or_default(t, set_names),
                        shared: names.get(shared.0).copied().unwrap_or("<shared system>"),
                    });
                }
                for &a in ms {
                    for &b in mt {
                        out.push((a, b));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Returns the first `SystemKey` present in both sorted slices, if any.
/// Linear merge over two ascending-by-`.0` slices (D3 guarantees order).
fn first_shared(a: &[SystemKey], b: &[SystemKey]) -> Option<SystemKey> {
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => return Some(a[i]),
        }
    }
    None
}

/// Emits the empty-set build warning (§6.4 / §13.1 R3-B) when an ordering
/// edge references a set with no (transitive) members. Build-time and cold,
/// so `eprintln!` is acceptable (no logging dependency in the ECS crate).
#[cold]
fn warn_if_empty(
    members: &[SystemKey],
    set: SystemSetId,
    set_names: &HashMap<SystemSetId, &'static str>,
) {
    if members.is_empty() {
        eprintln!(
            "boyko-W1501: ordering references set '{}' which has no members \
             (no system joined it via in_set); the ordering has no effect",
            set_name_or_default(set, set_names)
        );
    }
}

/// Looks up a set's recorded name, falling back to a synthetic label if the
/// id was somehow never named (should not happen — every intern records a
/// name; defensive only).
#[inline]
fn set_name_or_default(
    set: SystemSetId,
    set_names: &HashMap<SystemSetId, &'static str>,
) -> &'static str {
    set_names.get(&set).copied().unwrap_or("<unknown set>")
}

/// Builds an enriched system label `"name [in: SetA, SetB]"` for cycle
/// diagnostics (§6.3). If the system joined no sets, returns just the name.
fn enrich_system_name(
    key: SystemKey,
    names: &[&'static str],
    set_members: &HashMap<SystemSetId, Vec<SystemKey>>,
    set_names: &HashMap<SystemSetId, &'static str>,
) -> String {
    let base = names.get(key.0).copied().unwrap_or("<unknown system>");
    // Reverse lookup: which sets list `key` as a direct member. Build-time,
    // cold (only on a cycle error path).
    let mut joined: Vec<&'static str> = set_members
        .iter()
        .filter(|(_, members)| members.contains(&key))
        .map(|(&id, _)| set_name_or_default(id, set_names))
        .collect();
    if joined.is_empty() {
        base.to_string()
    } else {
        joined.sort_unstable();
        format!("{base} [in: {}]", joined.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_threadpool::ThreadPoolBuilder;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::ecs::core::system::access::Access;
    use crate::ecs::core::system::system::System;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

    /// Test-only `System` that counts `initialize` calls. Used to assert
    /// `build_initializes_systems_once`.
    struct CountingSystem {
        meta: SystemMeta,
        init_count: Arc<AtomicUsize>,
    }

    // SAFETY (S1): `run_unsafe` is empty; the trait contract is vacuous.
    unsafe impl System for CountingSystem {
        type Out = ();
        fn name(&self) -> &'static str {
            self.meta.name()
        }
        fn access(&self) -> &Access {
            self.meta.access()
        }
        fn initialize(&mut self, _world: &mut EcsMaster) {
            self.init_count.fetch_add(1, Ordering::Relaxed);
        }
        unsafe fn run_unsafe(&mut self, _world: UnsafeEcsCell<'_>) -> Self::Out {}
        fn meta(&self) -> &SystemMeta {
            &self.meta
        }
        fn set_change_ticks(
            &mut self,
            last_run: crate::ecs::core::change_detection::Tick,
            this_run: crate::ecs::core::change_detection::Tick,
        ) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }
        fn check_change_tick(&mut self, current: crate::ecs::core::change_detection::Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
        }
    }

    /// Wrapper that turns a `CountingSystem` into an `IntoSystem<(), ()>`
    /// via the identity-style closure pattern — easier than dragging the
    /// full `SystemParamFunction` chain in for a unit test.
    fn add_counting(
        builder: &mut ScheduleBuilder,
        name: &'static str,
        init_count: Arc<AtomicUsize>,
    ) -> SystemKey {
        let sys = CountingSystem {
            meta: SystemMeta::for_testing(name),
            init_count,
        };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let system_box = SystemBox::new(boxed);
        let key = SystemKey(builder.descriptors.len());
        builder
            .descriptors
            .push(SystemDescriptor::new(system_box));
        key
    }

    fn fresh_pool() -> Arc<ThreadPool> {
        ThreadPoolBuilder::new().num_threads(1).build()
    }

    /// `add_system` returns a `SystemConfig` whose key matches the
    /// insertion order.
    #[test]
    fn add_system_assigns_key() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));
        let a = add_counting(&mut builder, "a", Arc::clone(&init));
        let b = add_counting(&mut builder, "b", Arc::clone(&init));
        assert_eq!(a.0, 0);
        assert_eq!(b.0, 1);
        assert_eq!(builder.len(), 2);
    }

    /// `build` runs `System::initialize` exactly once per registered
    /// system.
    #[test]
    fn build_initializes_systems_once() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));
        let _a = add_counting(&mut builder, "a", Arc::clone(&init));
        let _b = add_counting(&mut builder, "b", Arc::clone(&init));
        let _c = add_counting(&mut builder, "c", Arc::clone(&init));

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);
        assert_eq!(init.load(Ordering::Relaxed), 3);
        assert_eq!(schedule.len(), 3);
    }

    /// Phase 4 D5 + CR-B — `SystemKind` resolution at `build`:
    ///   * a non-universal CPU system resolves `CpuConcurrent`;
    ///   * a universal-access system resolves `CpuExclusive`;
    ///   * the `SystemDescriptor::is_gpu` marker resolves `GpuCompute`
    ///     regardless of access (marker wins first).
    ///
    /// The resulting `kind` is read directly off the built schedule's
    /// `systems` slice (post-topological-order). With no `.before/.after`
    /// edges the topo sort preserves insertion order, so the indices line up.
    #[test]
    fn build_resolves_system_kind() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));

        // [0] plain CPU system (empty access) → CpuConcurrent.
        let _concurrent = add_counting(&mut builder, "concurrent", Arc::clone(&init));

        // [1] universal-access system → CpuExclusive.
        {
            let mut meta = SystemMeta::for_testing("universal");
            meta.access = Access::universal();
            let sys = CountingSystem {
                meta,
                init_count: Arc::clone(&init),
            };
            let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
            builder
                .descriptors
                .push(SystemDescriptor::new(SystemBox::new(boxed)));
        }

        // [2] GPU-marked system with EMPTY access → GpuCompute (the marker
        // wins before the universal check; GpuCompute carries no access
        // constraint).
        {
            let sys = CountingSystem {
                meta: SystemMeta::for_testing("gpu"),
                init_count: Arc::clone(&init),
            };
            let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
            let mut d = SystemDescriptor::new(SystemBox::new(boxed));
            d.is_gpu = true;
            builder.descriptors.push(d);
        }

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);

        assert_eq!(schedule.systems.len(), 3);
        assert_eq!(
            schedule.systems[0].kind,
            SystemKind::CpuConcurrent,
            "non-universal CPU system must resolve CpuConcurrent"
        );
        assert!(!schedule.systems[0].kind.runs_on_dispatcher());
        assert_eq!(
            schedule.systems[1].kind,
            SystemKind::CpuExclusive,
            "universal-access system must resolve CpuExclusive"
        );
        assert!(schedule.systems[1].kind.runs_on_dispatcher());
        assert_eq!(
            schedule.systems[2].kind,
            SystemKind::GpuCompute,
            "is_gpu marker must resolve GpuCompute even with empty access"
        );
        assert!(schedule.systems[2].kind.runs_on_dispatcher());
    }

    /// `.before(other)` + `.after(other)` on the same pair forms a cycle
    /// that `build` rejects with the documented `boyko-B9001` message.
    #[test]
    #[should_panic(expected = "boyko-B9001")]
    fn cycle_in_before_after_panics() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));
        let a = add_counting(&mut builder, "a", Arc::clone(&init));
        let b = add_counting(&mut builder, "b", Arc::clone(&init));

        // a -> b
        builder.descriptors[a.0]
            .ordering_hints
            .push(OrderingEdge::Before(a, b));
        // b -> a
        builder.descriptors[b.0]
            .ordering_hints
            .push(OrderingEdge::Before(b, a));

        let mut world = EcsMaster::new();
        let _schedule = builder.build(&mut world);
    }

    /// Topological sort respects `before` ordering: if `a` declares
    /// `before(b)`, `a` precedes `b` in the resulting `systems` vec.
    #[test]
    fn topological_sort_respects_before() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));

        // Insertion order: c, a, b. With c.before(a) and a.before(b) the
        // post-build order must be c, a, b.
        let c = add_counting(&mut builder, "c", Arc::clone(&init));
        let a = add_counting(&mut builder, "a", Arc::clone(&init));
        let b = add_counting(&mut builder, "b", Arc::clone(&init));

        builder.descriptors[c.0]
            .ordering_hints
            .push(OrderingEdge::Before(c, a));
        builder.descriptors[a.0]
            .ordering_hints
            .push(OrderingEdge::Before(a, b));

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);
        let names: Vec<&'static str> =
            schedule.systems.iter().map(|sb| sb.name).collect();
        // The exact ordering must place c first, then a, then b.
        assert_eq!(names, vec!["c", "a", "b"]);
    }

    /// Sanity probe — Kahn's on a small DAG.
    #[test]
    fn kahn_sort_basic() {
        // a -> b -> c, a -> c
        let edges = vec![
            (SystemKey(0), SystemKey(1)),
            (SystemKey(1), SystemKey(2)),
            (SystemKey(0), SystemKey(2)),
        ];
        let order = kahn_topological_sort(3, &edges);
        let pos: HashMap<usize, usize> =
            order.iter().enumerate().map(|(i, k)| (k.0, i)).collect();
        assert!(pos[&0] < pos[&1]);
        assert!(pos[&1] < pos[&2]);
        assert!(pos[&0] < pos[&2]);
    }

    /// Sanity probe — Tarjan on a known cycle returns one SCC of size 3.
    #[test]
    fn tarjan_detects_three_cycle() {
        let edges = vec![
            (SystemKey(0), SystemKey(1)),
            (SystemKey(1), SystemKey(2)),
            (SystemKey(2), SystemKey(0)),
        ];
        let sccs = tarjan_scc(3, &edges);
        let big = sccs.iter().filter(|s| s.len() > 1).count();
        assert_eq!(big, 1);
    }

    /// Sanity probe — Tarjan returns trivial SCCs (one per node) on an
    /// acyclic graph.
    #[test]
    fn tarjan_acyclic_yields_only_singletons() {
        let edges = vec![(SystemKey(0), SystemKey(1)), (SystemKey(1), SystemKey(2))];
        let sccs = tarjan_scc(3, &edges);
        assert!(sccs.iter().all(|s| s.len() == 1));
        assert_eq!(sccs.len(), 3);
    }

    // ── Phase 16 — run-condition build wiring ────────────────────────────────

    use crate::ecs::core::schedule::system_set::SystemSet;

    /// A hand-written `SystemSet` (all methods defaulted) — avoids dragging the
    /// `#[derive(SystemSet)]` proc-macro into a lib unit test.
    struct CondSet;
    impl SystemSet for CondSet {}

    /// `configure_set(S).run_if(c)` stores the condition under the set's id in
    /// the builder's `set_conditions` map. (Plan §10 `configure_set_run_if_stores`.)
    #[test]
    fn configure_set_run_if_stores_in_set_conditions() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let set_id = builder.configure_set(CondSet).run_if(|| true).id();
        assert_eq!(
            builder
                .set_conditions
                .get(&set_id)
                .map(Vec::len)
                .unwrap_or(0),
            1,
            "configure_set(S).run_if stores one set condition under S's id"
        );
    }

    /// A schedule with NO `.run_if` anywhere builds with an all-zero
    /// `has_condition` bitset — THE 0%-gate precondition. (Plan §10
    /// `has_condition_clear_when_no_run_if`.)
    #[test]
    fn has_condition_clear_when_no_run_if() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let init = Arc::new(AtomicUsize::new(0));
        add_counting(&mut builder, "a", Arc::clone(&init));
        add_counting(&mut builder, "b", Arc::clone(&init));

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);
        assert!(
            schedule.has_condition.is_clear(),
            "a conditionless schedule must have an all-zero has_condition bitset (0%-gate)"
        );
        assert!(
            schedule.system_conditions.iter().all(Vec::is_empty),
            "no system carries conditions"
        );
        assert!(schedule.set_conditions.is_empty(), "no set conditions");
    }

    /// A schedule with a `.run_if` builds with the conditioned system's
    /// `has_condition` bit set and its `system_conditions` slot populated
    /// (the condition rode through the topo permutation, §2.5). Also proves the
    /// build initialised the condition (its `Access` is non-default after init —
    /// here a `Res`-reading condition declares a resource read).
    #[test]
    fn build_sets_has_condition_and_moves_conditions() {
        use crate::ecs::core::resources::resource::Resource;
        use crate::ecs::core::resources::resource_registry::register_new;
        use crate::ecs::identifiers::primitives::ResourceId;
        use std::sync::OnceLock;

        #[allow(dead_code)]
        struct Marker(u32);
        // Mint the id via `register_new` (populating the resource_registry
        // drop_fn slot) — a hardcoded `ResourceId` would bypass the registry
        // and trip `Resources::insert`'s populated-slot invariant.
        impl Resource for Marker {
            fn resource_id() -> ResourceId {
                static ID: OnceLock<ResourceId> = OnceLock::new();
                *ID.get_or_init(|| ResourceId(register_new::<Self>()))
            }
        }

        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        // One unconditioned system, one conditioned with a Res-reading cond.
        builder.add_system(|| {});
        builder
            .add_system(|| {})
            .run_if(|_m: crate::ecs::core::system::Res<Marker>| true);

        let mut world = EcsMaster::new();
        world.insert_resource(Marker(0));
        let schedule = builder.build(&mut world);

        // Exactly one system has a condition → exactly one has_condition bit.
        assert_eq!(
            schedule.has_condition.count_ones(..),
            1,
            "exactly one conditioned system ⇒ one has_condition bit"
        );
        // That system's system_conditions slot holds the moved BoolSystem.
        let conditioned: Vec<usize> = (0..schedule.systems.len())
            .filter(|&i| !schedule.system_conditions[i].is_empty())
            .collect();
        assert_eq!(conditioned.len(), 1, "one slot carries a condition");
        assert_eq!(
            schedule.system_conditions[conditioned[0]].len(),
            1,
            "the conditioned slot carries exactly one BoolSystem"
        );
    }

    /// `system_gating_sets` is built for members of a conditioned set, and stays
    /// empty for members of an UNCONDITIONED set (only sets carrying a `.run_if`
    /// are "gating", §7.2).
    #[test]
    fn gating_sets_populated_only_for_conditioned_sets() {
        struct GatingSet;
        impl SystemSet for GatingSet {}
        struct PlainSet;
        impl SystemSet for PlainSet {}

        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        builder.add_system(|| {}).in_set(GatingSet); // member of conditioned set
        builder.add_system(|| {}).in_set(PlainSet); // member of unconditioned set
        builder.configure_set(GatingSet).run_if(|| true);

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);

        let with_gating = (0..schedule.systems.len())
            .filter(|&i| !schedule.system_gating_sets[i].is_empty())
            .count();
        assert_eq!(
            with_gating, 1,
            "only the member of the conditioned set gets a non-empty gating-set list"
        );
        // The conditioned-set member also gets a has_condition bit (gated via set).
        assert_eq!(
            schedule.has_condition.count_ones(..),
            1,
            "the conditioned-set member's has_condition bit is set"
        );
    }
}
