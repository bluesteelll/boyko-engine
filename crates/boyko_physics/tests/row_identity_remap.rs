//! Defect A interim fix (the row identity map) — schedule-level validation.
//!
//! The specification is Design v1 with Rev 2 and then Rev 3 applied, plus critique pass 3's
//! remarks O1-O4. The unit-level tests live beside the code: `row_identity.rs` (T6, T6c,
//! T12), `narrowphase/axis_cache.rs` (T5, T5b, T10), `resources_tests.rs` (T7) and
//! `scratch_ids.rs` (the row-identity cohort enumeration). This file drives the real physics
//! `Schedule`, and follows entities rather than rows exactly like
//! `row_keyed_state_defect_a.rs`: every body carries a `BodyId`, and the walk is checked
//! against the solver's own `SolverScratch::bodies()` snapshot.
//!
//! | test | what goes red |
//! |---|---|
//! | T1 `recycled_id_respawned_into_the_same_row_is_fresh` | a recycled id respawned into its dead body's own row inherits the dead body's latch (M3b) |
//! | T1b `recycled_id_spawned_in_the_same_run_is_fresh_within_one_step` | a spawn applied inside the run before the gather stays frozen longer than the one-step O1 bound |
//! | `gather_set_after_edge_sees_the_same_runs_gather` | the gather is not ordered by `PhysicsGatherSet`, the hook T1b orders its spawner through |
//! | T2 `sleeping_toggle_gap_resets_rather_than_carries` | a latch that missed gathers is read in place or carried by a one-step map |
//! | T3 `row_remap_builds_only_on_structural_change` | a map built on a step with no change, a stage-2 cost bound exceeded (M10, M10b), a consumer reset on a live schedule (M5, M9) |
//! | T4 `structural_churn_is_run_to_run_bit_deterministic` | two runs of one scripted churn differ in any bit |
//! | T8 `sleep_consumer_relatches_after_a_reset` | the latch cannot leave `Reset` (M4) |
//! | T8b `sleep_consumer_carries_through_a_solve_that_returns_early` | the re-key sits after the solver's early return (M8) |
//! | T9 `warm_start_consumer_carries_and_leaves_reset_{colored,serial}` | a warm table does not carry across a row move (M2), stays in `Reset` (M6), or is stamped at lookup (M7, serial only) |
//!
//! # Numbers recorded at the real row count of each phase (critique pass 3, O3)
//!
//! T3's scene starts at 62 rows (M0, the floor, S1..S60); (b) and (c) each despawn one body.
//! * (b) `m = 62, n = 61`: the last body swap-moves one row earlier; the walk realigns through
//!   one E4, so 0 rows reach stage 2 (bound: at most 1).
//! * (c) `m = 61, n = 60`: E4 for S3, E6 for S60, so 1 row reaches stage 2. With
//!   `REMAP_WINDOW = 0` the walk E4s every row down to S60 and E2s the 55 rows after it: 55.
//! * (d) `m = n = 60`, `n + m = 120`, budget `8 * 120 = 960`, the 16-body burst spends
//!   `136 + 256 = 392`: 16 rows reach stage 2. `REMAP_WINDOW = 15` gives 58 (43 E4 rows cost
//!   645, the burst 315 more, then the budget runs out in the second E4 run); `REMAP_WINDOW =
//!   0` gives 58 (E4 down to S58, then E2).
//! * (e) `m = 60, n = 63`, three flagged spawns ahead of the floor: 0 rows (E1 three times).
//!
//! # Deviations from the specification text, and why
//!
//! * **T1b's ordering edge.** The specification orders the spawning system
//!   `SystemConfig::before` the gather key. The gather's `SystemKey` is crate-private, so the
//!   spawner is ordered with `SystemConfig::before_set(PhysicsGatherSet)`, the set form of the
//!   same edge, through the set the physics plugin puts the gather in. That edge cannot fail
//!   T1b by itself: without it the spawner has no predecessor, is dispatched in the first
//!   round, and its commands are applied before the gather is dispatched anyway. So T1b
//!   proves the spawn reached step s's gather with a construction assertion
//!   (`bodies_len() == 5`, walk row 4 is E), and
//!   `gather_set_after_edge_sees_the_same_runs_gather` proves the set orders the gather from
//!   the `after_set` side, with a read-only recorder: a spawner there would put a structural
//!   change before `physics_apply`, whose row-count `debug_assert!` panics on a worker thread,
//!   and the run never completes.
//! * **T2's gap ends with a flagged spawn.** With the delete as the gap's only structural
//!   change, the last one-step map (the delete step's) equals the whole gap's map, so a gap
//!   misread as `Rows` would carry C's own clear latch and stay green. A far-away spawn on the
//!   re-enable step makes the last map send C's row to itself, where the latch is P2's.
//! * **T3 phase 1's `warm_seed_stats().manifolds >= 1`** is taken as the maximum over the
//!   200 steps. With sleeping on, a frozen island's manifolds are not pushed, so once every
//!   sphere has latched a single solve can legitimately seed 0 manifolds.
//! * **T8b and T9 phase 3** prove the solver's early return with `solved_steps()` (critique
//!   pass 3, O2): a latched or disabled body's pose is bit-identical across the step whether
//!   or not the early return ran, so a pose check cannot fail.
//! * **The whole file sits in `mod schedule` under `#[cfg(not(miri))]`** rather than a
//!   file-level `#![cfg(not(miri))]`: it spins up `boyko_threadpool`, which Miri cannot run,
//!   and the standing rules forbid adding a file-top attribute line.

#[cfg(not(miri))]
mod schedule {
    use std::sync::Arc;
    use std::time::Duration;

    use boyko_ecs::ecs::core::component::component::Component;
    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
    use boyko_ecs::ecs::core::entity::entity::Entity;
    use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
    use boyko_ecs::ecs::core::system::{Commands, Res, ResMut};
    use boyko_ecs::ecs::core::time::FixedTime;
    use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
    use boyko_macros::{Bundle, Component, Resource};
    use boyko_threadpool::{ThreadPool, ThreadPoolBuilder};

    use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass, Simulated};
    use boyko_physics::math::{Mat3, Quat, Vec3};
    use boyko_physics::plugin::{PhysicsGatherSet, add_physics_colored_solve, add_physics_systems};
    use boyko_physics::resources::{
        DEFAULT_SLEEP_THRESHOLD, IslandSleep, Manifolds, PhysicsConfig, SolverScratch,
    };
    use boyko_physics::solver::{ColoredSoftStepSolver, SoftStepSolver, WarmSeedStats};

    // ── Scene constants ──────────────────────────────────────────────────────

    /// The fixed step (60 Hz).
    const DT: f32 = 1.0 / 60.0;
    /// The red harness's debounce, so a pile latches within the step budget.
    const SLEEP_FRAMES: u16 = 8;
    /// A latch is looked for only after this many steps.
    const MIN_SETTLE_STEPS: usize = 120;
    /// Upper bound on a settle loop; exceeding it is a harness failure, not the defect.
    const MAX_SETTLE_STEPS: usize = 600;
    /// A static floor box whose top face is `y = 0`.
    const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(20.0, 0.5, 20.0);

    const FLOOR: u32 = 0;
    const P1: u32 = 1;
    const P2: u32 = 2;
    const P3: u32 = 3;
    const P4: u32 = 4;
    /// P1 under P2 at `x = 0`, P3 under P4 at `x = 2`.
    const PILE: [u32; 4] = [P1, P2, P3, P4];
    const FALLER: u32 = 5;
    const NEWCOMER: u32 = 6;
    const APEX: u32 = 7;
    const MIGRANT: u32 = 10;
    const CUBE_A: u32 = 20;
    const CUBE_B: u32 = 21;
    const CUBE_C: u32 = 22;

    // ── Test-only components, bundle and resource ────────────────────────────

    /// Stable identity of a body, so assertions follow entities rather than rows.
    #[derive(Component, Clone, Copy, Debug)]
    #[repr(C)]
    struct BodyId {
        id: u32,
    }

    /// A data component whose insert or remove migrates a body between the two archetypes.
    #[derive(Component, Clone, Copy, Debug)]
    #[repr(C)]
    struct Marker {
        _tag: u32,
    }

    /// The PLAIN archetype's components, for a spawn through `Commands`.
    #[derive(Bundle)]
    struct BodyBundle {
        body: RigidBody,
        mass: RigidBodyMass,
        collider: Collider,
        id: BodyId,
    }

    /// T1b: a body the spawning system spawns on its next run, and the entity it spawned.
    #[derive(Resource, Default)]
    struct SpawnRequest {
        spec: Option<Spec>,
        spawned: Option<Entity>,
    }

    /// T1b's spawning system: on the run after `SpawnRequest::spec` is set, spawns that body
    /// through `Commands` with `Simulated` enabled.
    //
    // `clippy::needless_pass_by_value`: `Commands` / `ResMut` are by-value `SystemParam`s,
    // the same false positive the physics systems carry.
    #[allow(clippy::needless_pass_by_value)]
    fn spawn_on_request(mut commands: Commands, mut request: ResMut<SpawnRequest>) {
        if let Some(spec) = request.spec.take() {
            let (body, mass, collider, id) = spec.columns();
            let mut spawned = commands.spawn(BodyBundle {
                body,
                mass,
                collider,
                id,
            });
            spawned.enable::<Simulated>();
            request.spawned = Some(spawned.id());
        }
    }

    /// The `SolverScratch` row count `record_gathered_len` saw on its last run.
    #[derive(Resource, Default)]
    struct GatheredLen {
        len: Option<usize>,
    }

    /// Records the gather's snapshot length. It is read-only on physics state on purpose: a
    /// structural change between the gather and `physics_apply` trips `physics_apply`'s
    /// row-count invariant on a worker thread, and the run never completes.
    //
    // `clippy::needless_pass_by_value`: `Res` / `ResMut` are by-value `SystemParam`s, the same
    // false positive the physics systems carry.
    #[allow(clippy::needless_pass_by_value)]
    fn record_gathered_len(scratch: Res<SolverScratch>, mut gathered: ResMut<GatheredLen>) {
        gathered.len = Some(scratch.bodies_len());
    }

    // ── Harness ──────────────────────────────────────────────────────────────

    /// Views a `#[repr(C)]` POD value as its bytes for the raw `create_entity` path.
    fn as_bytes<T>(value: &T) -> &[u8] {
        // SAFETY: `value` is a live, initialised `#[repr(C)]` `T` borrowed for the returned
        // slice's lifetime; the slice covers exactly its `size_of::<T>()` bytes read-only —
        // the layout the component pool stores (mirrors `row_keyed_state_defect_a::as_bytes`).
        unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
    }

    /// A single-threaded pool (deterministic).
    fn serial_pool() -> Arc<ThreadPool> {
        ThreadPoolBuilder::new().num_threads(1).build()
    }

    /// What to spawn.
    #[derive(Clone, Copy)]
    struct Spec {
        id: u32,
        position: Vec3,
        velocity: Vec3,
        shape: ColliderShape,
        inv_mass: f32,
    }

    impl Spec {
        fn columns(self) -> (RigidBody, RigidBodyMass, Collider, BodyId) {
            (
                RigidBody {
                    position: self.position,
                    linear_velocity: self.velocity,
                    rotation: Quat::IDENTITY,
                    angular_velocity: Vec3::ZERO,
                },
                RigidBodyMass {
                    inv_inertia: if self.inv_mass == 0.0 {
                        Mat3::ZERO
                    } else {
                        Mat3::IDENTITY
                    },
                    inv_mass: self.inv_mass,
                    restitution: 0.0,
                    friction: 0.5,
                },
                Collider {
                    shape: self.shape,
                    layer: 1,
                    mask: 1,
                },
                BodyId { id: self.id },
            )
        }
    }

    /// A static floor box with the given half extents, top face at `y = 0`.
    fn floor(half_extents: Vec3) -> Spec {
        Spec {
            id: FLOOR,
            position: Vec3::new(0.0, -0.5, 0.0),
            velocity: Vec3::ZERO,
            shape: ColliderShape::Box { half_extents },
            inv_mass: 0.0,
        }
    }

    /// A unit-mass sphere of radius 0.5 at rest.
    fn ball(id: u32, position: Vec3) -> Spec {
        Spec {
            id,
            position,
            velocity: Vec3::ZERO,
            shape: ColliderShape::Sphere { radius: 0.5 },
            inv_mass: 1.0,
        }
    }

    /// A unit-mass unit cube at rest (a four-point face-face manifold on the floor).
    fn cube(id: u32, position: Vec3) -> Spec {
        Spec {
            shape: ColliderShape::Box {
                half_extents: Vec3::new(0.5, 0.5, 0.5),
            },
            ..ball(id, position)
        }
    }

    fn bits_eq(a: Vec3, b: Vec3) -> bool {
        a.x.to_bits() == b.x.to_bits()
            && a.y.to_bits() == b.y.to_bits()
            && a.z.to_bits() == b.z.to_bits()
    }

    /// Which solve the schedule runs.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Solve {
        /// `add_physics_colored_solve` (`ColoredSoftStepSolver`, `IslandSleep` present).
        Colored,
        /// `add_physics_systems::<SoftStepSolver>` (no constraint graph, no sleeping).
        Serial,
    }

    /// A test system registered ahead of the physics block and ordered against the gather
    /// through `PhysicsGatherSet`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum GatherProbe {
        /// T1b: `spawn_on_request`, `before_set(PhysicsGatherSet)`.
        SpawnerBefore,
        /// `record_gathered_len`, `after_set(PhysicsGatherSet)`.
        RecorderAfter,
    }

    struct Harness {
        world: EcsMaster,
        physics: Schedule,
        /// `{RigidBody, RigidBodyMass, Collider, BodyId, Marker}` — created FIRST, so the
        /// gather walks it first.
        marked: ArchetypeId,
        /// `{RigidBody, RigidBodyMass, Collider, BodyId}`.
        plain: ArchetypeId,
        solve: Solve,
    }

    impl Harness {
        fn new(solve: Solve, sleeping: bool) -> Self {
            Self::build(solve, sleeping, None)
        }

        fn build(solve: Solve, sleeping: bool, probe: Option<GatherProbe>) -> Self {
            let mut world = EcsMaster::new();
            let marked = world.create_archetype(&[
                RigidBody::component_id(),
                RigidBodyMass::component_id(),
                Collider::component_id(),
                BodyId::component_id(),
                Marker::component_id(),
            ]);
            let plain = world.create_archetype(&[
                RigidBody::component_id(),
                RigidBodyMass::component_id(),
                Collider::component_id(),
                BodyId::component_id(),
            ]);

            let mut builder = ScheduleBuilder::new(serial_pool());
            // Ordered against the gather by the physics plugin's named set (module doc,
            // "T1b's ordering edge").
            match probe {
                None => {}
                Some(GatherProbe::SpawnerBefore) => {
                    world.insert_resource(SpawnRequest::default());
                    builder
                        .add_system(spawn_on_request)
                        .before_set(PhysicsGatherSet);
                }
                Some(GatherProbe::RecorderAfter) => {
                    world.insert_resource(GatheredLen::default());
                    builder
                        .add_system(record_gathered_len)
                        .after_set(PhysicsGatherSet);
                }
            }
            match solve {
                Solve::Colored => {
                    let _keys = add_physics_colored_solve(&mut builder, &mut world);
                }
                Solve::Serial => {
                    let _keys = add_physics_systems::<SoftStepSolver>(&mut builder, &mut world);
                }
            }
            world.insert_resource(FixedTime::new(Duration::from_secs_f32(DT)));
            let physics = builder.build(&mut world);
            {
                let cfg = world.resource_mut::<PhysicsConfig>();
                cfg.sleeping = sleeping;
                cfg.sleep_frames = SLEEP_FRAMES;
            }
            Self {
                world,
                physics,
                marked,
                plain,
                solve,
            }
        }

        fn spawn(&mut self, spec: Spec, marked: bool) -> Entity {
            let (body, mass, collider, id) = spec.columns();
            let marker = Marker { _tag: 0 };
            let created = if marked {
                self.world.create_entity(
                    self.marked,
                    &[
                        (RigidBody::component_id(), as_bytes(&body)),
                        (RigidBodyMass::component_id(), as_bytes(&mass)),
                        (Collider::component_id(), as_bytes(&collider)),
                        (BodyId::component_id(), as_bytes(&id)),
                        (Marker::component_id(), as_bytes(&marker)),
                    ],
                )
            } else {
                self.world.create_entity(
                    self.plain,
                    &[
                        (RigidBody::component_id(), as_bytes(&body)),
                        (RigidBodyMass::component_id(), as_bytes(&mass)),
                        (Collider::component_id(), as_bytes(&collider)),
                        (BodyId::component_id(), as_bytes(&id)),
                    ],
                )
            };
            let entity = created.expect("construction: the body archetype accepts every column");
            self.world.enable::<Simulated>(entity);
            entity
        }

        /// P1 under P2 at `x = 0`, P3 under P4 at `x = 2`, in `PILE` order.
        fn spawn_pile(&mut self) -> [Entity; 4] {
            [
                self.spawn(ball(P1, Vec3::new(0.0, 0.5, 0.0)), false),
                self.spawn(ball(P2, Vec3::new(0.0, 1.5, 0.0)), false),
                self.spawn(ball(P3, Vec3::new(2.0, 0.5, 0.0)), false),
                self.spawn(ball(P4, Vec3::new(2.0, 1.5, 0.0)), false),
            ]
        }

        fn step(&mut self) {
            self.physics.run(&mut self.world);
        }

        fn steps(&mut self, n: usize) {
            for _ in 0..n {
                self.step();
            }
        }

        /// Body ids in the gather's walk order (row `i` of the solver is element `i`).
        fn walk_ids(&mut self) -> Vec<u32> {
            let q = self
                .world
                .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
            q.iter().map(|(_, _, _, id)| id.id).collect()
        }

        fn row_of(&mut self, id: u32) -> usize {
            self.walk_ids()
                .iter()
                .position(|&x| x == id)
                .unwrap_or_else(|| panic!("harness: body {id} is not in the gather walk"))
        }

        /// Whether body `id`'s row was awake (solved and integrated) on the last step.
        fn awake(&mut self, id: u32) -> bool {
            let row = self.row_of(id);
            self.world.resource::<IslandSleep>().is_row_awake(row)
        }

        fn awake_of(&mut self, ids: &[u32]) -> Vec<u32> {
            ids.iter().copied().filter(|&id| self.awake(id)).collect()
        }

        fn body(&self, entity: Entity) -> RigidBody {
            *self
                .world
                .get_component::<RigidBody>(entity)
                .expect("harness: a tracked body is live")
        }

        fn scratch(&self) -> &SolverScratch {
            self.world.resource::<SolverScratch>()
        }

        fn sleep(&self) -> &IslandSleep {
            self.world.resource::<IslandSleep>()
        }

        fn warm_stats(&self) -> WarmSeedStats {
            match self.solve {
                Solve::Colored => self
                    .world
                    .resource::<ColoredSoftStepSolver>()
                    .warm_seed_stats(),
                Solve::Serial => self.world.resource::<SoftStepSolver>().warm_seed_stats(),
            }
        }

        fn solved_steps(&self) -> u64 {
            match self.solve {
                Solve::Colored => self
                    .world
                    .resource::<ColoredSoftStepSolver>()
                    .solved_steps(),
                Solve::Serial => self.world.resource::<SoftStepSolver>().solved_steps(),
            }
        }

        fn axis_resets(&self) -> u64 {
            self.world
                .resource::<Manifolds>()
                .box_axis_cache
                .remap_resets()
        }

        /// Inserts a fresh solver resource of this schedule's solver type.
        fn replace_solver(&mut self) {
            match self.solve {
                Solve::Colored => self.world.insert_resource(ColoredSoftStepSolver::default()),
                Solve::Serial => self.world.insert_resource(SoftStepSolver::default()),
            }
        }

        fn set_sleeping(&mut self, on: bool) {
            self.world.resource_mut::<PhysicsConfig>().sleeping = on;
        }

        /// After a step and before any structural change: gather row `i` holds exactly the
        /// pose of the body the walk puts at row `i`.
        fn assert_walk_matches_gather(&mut self) {
            let walked: Vec<(u32, Vec3)> = {
                let q = self
                    .world
                    .query::<(&RigidBody, &RigidBodyMass, &Collider, &BodyId), ()>();
                q.iter().map(|(b, _, _, id)| (id.id, b.position)).collect()
            };
            let gathered: Vec<Vec3> = self.scratch().bodies().iter().map(|b| b.position).collect();
            assert_eq!(
                walked.len(),
                gathered.len(),
                "harness: the test's walk and the gather disagree on the row count"
            );
            for (row, ((id, walked_pos), gathered_pos)) in walked.iter().zip(&gathered).enumerate()
            {
                assert!(
                    bits_eq(*walked_pos, *gathered_pos),
                    "harness: walk row {row} is body {id} at {walked_pos:?}, but gather row {row} is \
                     at {gathered_pos:?} - the test's row map is not the solver's"
                );
            }
        }

        /// Steps at least `MIN_SETTLE_STEPS`, then until every body in `ids` has a row that was
        /// not awake on the last step. Returns the step count.
        fn settle_until_latched(&mut self, ids: &[u32]) -> usize {
            for step in 1..=MAX_SETTLE_STEPS {
                self.step();
                if step >= MIN_SETTLE_STEPS && self.awake_of(ids).is_empty() {
                    return step;
                }
            }
            panic!("harness: bodies {ids:?} did not latch asleep within {MAX_SETTLE_STEPS} steps");
        }

        /// Removes `Marker` from `entity` through `Commands`, between physics runs.
        fn remove_marker(&mut self, entity: Entity) {
            let mut builder = ScheduleBuilder::new(serial_pool());
            builder.add_system(move |mut commands: Commands| {
                commands.entity(entity).remove::<Marker>();
            });
            let mut schedule = builder.build(&mut self.world);
            schedule.run(&mut self.world);
        }

        /// Inserts `Marker` on every entity of `entities`, in that order, through `Commands`,
        /// between physics runs.
        fn insert_markers(&mut self, entities: Vec<Entity>) {
            let mut builder = ScheduleBuilder::new(serial_pool());
            builder.add_system(move |mut commands: Commands| {
                for &entity in &entities {
                    commands.entity(entity).insert(Marker { _tag: 0 });
                }
            });
            let mut schedule = builder.build(&mut self.world);
            schedule.run(&mut self.world);
        }

        /// The upward speed that puts a body thrown at spawn at its apex after `steps` steps:
        /// the per-substep gravity increment as the solver forms it, times the substep count.
        fn apex_speed(&self, steps: usize) -> f32 {
            let cfg = self.world.resource::<PhysicsConfig>();
            let substeps = cfg.substeps.max(1) as usize;
            let dt = self.world.resource::<FixedTime>().delta_secs();
            -cfg.gravity.y * (dt / substeps as f32) * (steps * substeps) as f32
        }
    }

    // ── T1: a recycled id in its dead body's own row is a new body ───────────

    /// T1: the last row P4 of a latched pile is despawned (nothing swap-moves) and body E is
    /// spawned at rest in mid-air. E recycles P4's `EntityId` and lands in P4's row, so the id
    /// compare alone calls the rows unchanged; only the added row makes the gather rebuild the
    /// map and start E awake. Red under M3b (`added_rows.is_empty()` dropped from `stable`):
    /// E inherits P4's latch and hangs at `y = 4`.
    #[test]
    fn recycled_id_respawned_into_the_same_row_is_fresh() {
        let mut h = Harness::new(Solve::Colored, true);
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        let pile = h.spawn_pile();
        let settle_steps = h.settle_until_latched(&PILE);
        h.assert_walk_matches_gather();
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, P1, P2, P3, P4],
            "construction: walk before the change"
        );
        let p4_row = h.row_of(P4);
        let p4_id = pile[3].id();

        assert!(
            h.world.delete_entity(pile[3]),
            "construction: P4 must be despawnable"
        );
        let e = h.spawn(ball(NEWCOMER, Vec3::new(5.0, 4.0, 0.0)), false);
        assert_eq!(
            e.id(),
            p4_id,
            "construction: E must recycle P4's EntityId, or the test does not exercise a recycled id"
        );
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, P1, P2, P3, NEWCOMER],
            "construction: E must land in P4's old row {p4_row}"
        );

        let builds_before = h.scratch().row_remap_builds();
        h.step();
        let built_on_first_step = h.scratch().row_remap_builds() - builds_before;
        let awake_on_first_step = h.awake(NEWCOMER);
        h.steps(29);
        let y = h.body(e).position.y;
        assert!(
            y < 3.0,
            "T1: body E, spawned at rest at y = 4 into P4's row {p4_row} with P4's recycled EntityId, \
             must fall; after 30 steps E.y = {y} (expected < 3.0). E's row awake on its first step: \
             {awake_on_first_step}; previous-row maps built on that step: {built_on_first_step}; pile \
             latched after {settle_steps} steps"
        );
    }

    // ── T1b: the O1 bound ────────────────────────────────────────────────────

    /// T1b: P4, the last row of a latched pile, is despawned between runs; on step s a system
    /// that runs before the gather spawns E at rest at (5, 4, 0) through `Commands`, recycling
    /// P4's id into P4's row. That spawn is stamped `this_run + 1`, so step s's gather cannot
    /// see it as added and E may carry P4's latch for that one step (printed, not asserted).
    /// Step s + 1 flags it: E's row must be awake, and E must then fall.
    #[test]
    fn recycled_id_spawned_in_the_same_run_is_fresh_within_one_step() {
        let mut h = Harness::build(Solve::Colored, true, Some(GatherProbe::SpawnerBefore));
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        let pile = h.spawn_pile();
        let settle_steps = h.settle_until_latched(&PILE);
        h.assert_walk_matches_gather();
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, P1, P2, P3, P4],
            "construction: walk before the change"
        );
        let p4_id = pile[3].id();

        assert!(
            h.world.delete_entity(pile[3]),
            "construction: P4 must be despawnable"
        );
        h.world.resource_mut::<SpawnRequest>().spec =
            Some(ball(NEWCOMER, Vec3::new(5.0, 4.0, 0.0)));

        // Step s.
        h.step();
        let e = h
            .world
            .resource::<SpawnRequest>()
            .spawned
            .expect("construction: the spawning system must have run on step s");
        assert_eq!(
            h.scratch().bodies_len(),
            5,
            "construction: E's spawn command must land before step s's gather (otherwise this test \
             does not exercise O1)"
        );
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, P1, P2, P3, NEWCOMER],
            "construction: E must be walk row 4, P4's old row"
        );
        h.assert_walk_matches_gather();
        assert_eq!(e.id(), p4_id, "construction: E must recycle P4's EntityId");
        let awake_on_step_s = h.awake(NEWCOMER);
        eprintln!("T1b: E's row awake on step s (the O1 step, not asserted): {awake_on_step_s}");

        // Step s + 1.
        let y_s = h.body(e).position.y;
        h.step();
        assert!(
            h.awake(NEWCOMER),
            "T1b: E's row must be awake on step s + 1, one gather after its late-flagged spawn (the O1 \
             bound). Awake on step s: {awake_on_step_s}; y after step s = {y_s}; pile latched after \
             {settle_steps} steps"
        );
        h.steps(30);
        let y = h.body(e).position.y;
        assert!(
            y < 3.0,
            "T1b: E must fall once flagged; y after 30 more steps = {y} (expected < 3.0)"
        );
    }

    /// `PhysicsGatherSet`, the hook T1b orders its spawner through, checked from the side where
    /// a missing edge changes what is observed. T1b's `before_set` edge cannot fail T1b: without
    /// it the spawner has no predecessor, is dispatched in the first round, and its commands are
    /// applied before the gather anyway. Here `record_gathered_len`, ordered
    /// `after_set(PhysicsGatherSet)`, must see a body created between runs in the snapshot of
    /// the very next run. Red if the gather is not a member of the set: the edge expands to
    /// nothing, the recorder has no predecessor and runs ahead of the gather, and it records
    /// the previous run's snapshot.
    ///
    /// A recorder, not a spawner: a spawn ordered after the gather lands before
    /// `physics_apply`, whose row-count `debug_assert!` then panics on a worker thread, and the
    /// run never completes (a documented misuse, not a defect of the set).
    #[test]
    fn gather_set_after_edge_sees_the_same_runs_gather() {
        let mut h = Harness::build(Solve::Colored, true, Some(GatherProbe::RecorderAfter));
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        h.step();
        assert!(
            h.world.resource::<GatheredLen>().len.is_some(),
            "construction: the recorder must run on every step"
        );

        h.spawn(ball(NEWCOMER, Vec3::new(5.0, 4.0, 0.0)), false);
        h.step();
        assert_eq!(
            h.scratch().bodies_len(),
            2,
            "construction: a body created between runs must be in the next run's gather"
        );
        let recorded = h.world.resource::<GatheredLen>().len;
        assert_eq!(
            recorded,
            Some(2),
            "a recorder ordered after_set(PhysicsGatherSet) ran before this run's gather: the \
             gather is not ordered by its set (recorded {recorded:?}, the gather produced 2 rows)"
        );
    }

    // ── T2: a sleeping toggle resets the latch ───────────────────────────────

    /// Steps from spawn to the apex of the vertical throw.
    const APEX_STEPS: usize = 240;
    /// The step at which sleeping is switched off, well after the pile has latched.
    const GAP_START: usize = 200;
    /// T2's far-away body spawned on the step sleeping comes back.
    const SENTINEL: u32 = 8;

    /// T2: a latched pile and a body C thrown straight up. Sleeping is switched off, P2 is
    /// deleted so C swap-moves into P2's latched row, and the schedule runs with sleeping off
    /// until C's apex; then sleeping is switched back on. The latch missed every gather of the
    /// gap, so it must be reset, and C (slower than the sleep threshold at its apex) must
    /// fall. Red if the gap is treated as `Identity` (C reads P2's latch in place) or as
    /// `Rows` (a one-step map cannot describe a move several gathers old).
    ///
    /// A far-away body is spawned on the re-enable step. Without it the delete is the gap's
    /// only structural change, so the last one-step map (the delete step's) equals the whole
    /// gap's map, and a gap misread as `Rows` would carry C's own clear latch and stay green.
    /// With it, the last map sends C's row to itself, where the latch is P2's.
    #[test]
    fn sleeping_toggle_gap_resets_rather_than_carries() {
        let mut h = Harness::new(Solve::Colored, true);
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        let pile = h.spawn_pile();
        let v0 = h.apex_speed(APEX_STEPS);
        let c = h.spawn(
            Spec {
                velocity: Vec3::new(0.0, v0, 0.0),
                ..ball(APEX, Vec3::new(-5.0, 4.0, 0.0))
            },
            false,
        );
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, P1, P2, P3, P4, APEX],
            "construction: walk order"
        );

        h.steps(GAP_START);
        h.assert_walk_matches_gather();
        assert!(
            h.awake_of(&PILE).is_empty(),
            "construction: the pile must be latched by step {GAP_START}"
        );
        assert!(
            h.awake(APEX),
            "construction: C's own latch must be clear before the gap"
        );

        h.set_sleeping(false);
        assert!(
            h.world.delete_entity(pile[1]),
            "construction: P2 must be despawnable"
        );
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, P1, APEX, P3, P4],
            "construction: C must swap-move into P2's latched row"
        );
        h.steps(APEX_STEPS - GAP_START);
        let at_apex = h.body(c);
        let speed_sq = at_apex.linear_velocity.dot(at_apex.linear_velocity)
            + at_apex.angular_velocity.dot(at_apex.angular_velocity);
        assert!(
            speed_sq < DEFAULT_SLEEP_THRESHOLD,
            "construction: C must be slower than the sleep threshold when sleeping is re-enabled \
             (speed^2 = {speed_sq})"
        );

        h.spawn(ball(SENTINEL, Vec3::new(0.0, 4.0, -50.0)), false);
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, P1, APEX, P3, P4, SENTINEL],
            "construction: the sentinel is appended, so the re-enable step builds a one-step map"
        );
        h.set_sleeping(true);
        let apex_y = at_apex.position.y;
        let builds = h.scratch().row_remap_builds();
        h.step();
        assert_eq!(
            h.scratch().row_remap_builds(),
            builds + 1,
            "construction: the re-enable step must build a previous-row map"
        );
        let awake_on_first_step = h.awake(APEX);
        let resets = h.sleep().remap_resets();
        h.steps(29);
        let y = h.body(c).position.y;
        assert!(
            y < apex_y - 1.0,
            "T2: C, moved into P2's latched row during a {}-step sleeping-off gap and at its apex when \
             sleeping came back, must fall ~1.24 m in 30 steps; apex y = {apex_y}, y after 30 steps = \
             {y}. C's row awake on the first step: {awake_on_first_step}; IslandSleep remap_resets {resets}",
            APEX_STEPS - GAP_START
        );
    }

    // ── T3: maps are built only on change steps, at bounded cost ─────────────

    const T3_FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(40.0, 0.5, 40.0);
    const T3_M0: u32 = 100;
    /// Sphere `k` (1-based) has id `T3_S_BASE + k`.
    const T3_S_BASE: u32 = 1000;
    const T3_SPHERES: u32 = 60;
    /// The three spheres spawned in phase (e) have ids `T3_NEW_BASE + 1..=3`.
    const T3_NEW_BASE: u32 = 2000;

    fn s(k: u32) -> u32 {
        T3_S_BASE + k
    }

    /// One step, asserting the constant check: `row_remap_builds` rises by exactly 1 on a change
    /// step and never otherwise.
    fn t3_step(h: &mut Harness, change: bool, label: &str) {
        let before = h.scratch().row_remap_builds();
        h.step();
        let after = h.scratch().row_remap_builds();
        assert_eq!(
            after - before,
            u64::from(change),
            "T3 constant check ({label}): row_remap_builds must rise by exactly 1 on a change step and \
             never otherwise (before {before}, after {after})"
        );
    }

    /// The three consumers' reset counters, asserted in the order the specification names them.
    fn t3_assert_no_resets(h: &Harness, phase: &str) {
        let sleep = h.sleep().remap_resets();
        assert_eq!(
            sleep, 0,
            "T3 {phase}: IslandSleep remap_resets {sleep} (expected 0)"
        );
        let warm = h.warm_stats().remap_resets;
        assert_eq!(
            warm, 0,
            "T3 {phase}: warm start remap_resets {warm} (expected 0)"
        );
        let axis = h.axis_resets();
        assert_eq!(
            axis, 0,
            "T3 {phase}: box axis cache remap_resets {axis} (expected 0)"
        );
    }

    /// T3 (Rev 3 P22, numbers recomputed per critique pass 3 O3 in the module doc): M0 resting
    /// in MARKED (walked first); the static floor and 60 resting spheres in a 10 x 6 grid
    /// 1.2 m apart in PLAIN; sleeping on, warm start on. Phase 1 runs 200 steps with no churn;
    /// then (b) a second-to-last despawn, (c) a far-from-tail despawn, (d) a 16-body `Marker`
    /// burst into MARKED in reverse row order, (e) three flagged spawns into MARKED, with 50
    /// quiet steps after (b) and after (e).
    #[test]
    fn row_remap_builds_only_on_structural_change() {
        let mut h = Harness::new(Solve::Colored, true);
        h.spawn(ball(T3_M0, Vec3::new(-20.0, 0.5, 0.0)), true);
        h.spawn(floor(T3_FLOOR_HALF_EXTENTS), false);
        let mut entities: Vec<(u32, Entity)> = Vec::new();
        for k in 0..T3_SPHERES {
            let x = -5.4 + (k % 10) as f32 * 1.2;
            let z = -3.0 + (k / 10) as f32 * 1.2;
            let id = s(k + 1);
            entities.push((id, h.spawn(ball(id, Vec3::new(x, 0.5, z)), false)));
        }
        let entity_of = |id: u32| {
            entities
                .iter()
                .find(|(x, _)| *x == id)
                .map(|&(_, e)| e)
                .unwrap_or_else(|| panic!("harness: no entity for body {id}"))
        };
        let mut expected: Vec<u32> = [T3_M0, FLOOR]
            .into_iter()
            .chain((1..=T3_SPHERES).map(s))
            .collect();
        assert_eq!(h.walk_ids(), expected, "construction: T3's initial walk");

        // Phase 1: liveness over 200 steps with no churn.
        let mut max_manifolds = 0u32;
        for step in 1..=200 {
            t3_step(&mut h, step == 1, "phase 1");
            max_manifolds = max_manifolds.max(h.warm_stats().manifolds);
        }
        let (builds, searched) = (
            h.scratch().row_remap_builds(),
            h.scratch().row_remap_searched(),
        );
        assert_eq!(
            (builds, searched),
            (1, 0),
            "T3 phase 1: (row_remap_builds, row_remap_searched) after 200 steps with no churn; a red \
             here is review open question 3 (a body flagged added in more or fewer than exactly its \
             first gather), not a walk defect, until the kernel tick model says otherwise"
        );
        t3_assert_no_resets(&h, "phase 1 (liveness)");
        assert!(
            max_manifolds >= 1,
            "anti-vacuity: T3 phase 1 must seed at least one manifold (max over the phase {max_manifolds})"
        );
        let asleep = (1..=T3_SPHERES).filter(|&k| !h.awake(s(k))).count();
        assert!(
            asleep >= 1,
            "anti-vacuity: at least one sphere row must be latched after phase 1"
        );
        h.assert_walk_matches_gather();

        // (b) Despawn PLAIN's second-to-last body; the last swap-moves one row earlier.
        let before = h.scratch().row_remap_searched();
        assert!(
            h.world.delete_entity(entity_of(s(59))),
            "construction: S59 despawnable"
        );
        let at = expected
            .iter()
            .position(|&id| id == s(59))
            .expect("S59 walked");
        expected[at] = s(60);
        expected.pop();
        assert_eq!(
            h.walk_ids(),
            expected,
            "construction (b): S60 swap-moved into S59's row"
        );
        t3_step(&mut h, true, "(b)");
        let delta = h.scratch().row_remap_searched() - before;
        assert!(
            delta <= 1,
            "T3 (b): second-to-last despawn sent {delta} rows to stage 2 (bound 1)"
        );
        h.assert_walk_matches_gather();
        for _ in 0..50 {
            t3_step(&mut h, false, "50 steps after (b)");
        }
        t3_assert_no_resets(&h, "50 steps after (b)");

        // (c) Despawn S3, far from PLAIN's tail; the last body moves into its row.
        let row_s3 = h.row_of(s(3));
        let rows_after = expected.len() - row_s3 - 1;
        assert!(
            rows_after >= 40,
            "construction (c): {rows_after} rows follow S3 (need >= 40)"
        );
        let before = h.scratch().row_remap_searched();
        assert!(
            h.world.delete_entity(entity_of(s(3))),
            "construction: S3 despawnable"
        );
        let last = expected.pop().expect("walk non-empty");
        expected[row_s3] = last;
        assert_eq!(
            h.walk_ids(),
            expected,
            "construction (c): PLAIN's last body moved into S3's row"
        );
        assert_eq!(last, s(60), "construction (c): the moved body is S60");
        t3_step(&mut h, true, "(c)");
        let delta = h.scratch().row_remap_searched() - before;
        assert!(
            delta <= 1,
            "T3 (c): far-from-tail despawn sent {delta} rows to stage 2 (bound 1; REMAP_WINDOW = 0 gives 55)"
        );
        h.assert_walk_matches_gather();

        // (d) Insert Marker on PLAIN's last 16 bodies, in reverse row order.
        let walk = h.walk_ids();
        let burst: Vec<u32> = walk[walk.len() - 16..].to_vec(); // L1..L16 in walk order
        h.insert_markers(burst.iter().rev().map(|&id| entity_of(id)).collect());
        expected = std::iter::once(T3_M0)
            .chain(burst.iter().rev().copied())
            .chain(walk[1..walk.len() - 16].iter().copied())
            .collect();
        assert_eq!(
            h.walk_ids(),
            expected,
            "construction (d): the walk must be [M0, L16, .., L1, F, remaining PLAIN in order]"
        );
        let before = h.scratch().row_remap_searched();
        t3_step(&mut h, true, "(d)");
        let delta = h.scratch().row_remap_searched() - before;
        assert!(
            delta <= 16,
            "T3 (d): a 16-body burst into the first-walked archetype sent {delta} rows to stage 2 (bound \
             16 at n + m = 120, budget 960, burst cost 392; REMAP_WINDOW = 15 or 0 gives 58)"
        );
        h.assert_walk_matches_gather();

        // (e) Three flagged spawns into MARKED between runs.
        let before = h.scratch().row_remap_searched();
        for k in 1..=3u32 {
            h.spawn(
                ball(T3_NEW_BASE + k, Vec3::new(20.0, 0.5, -4.0 + 2.0 * k as f32)),
                true,
            );
        }
        expected.splice(17..17, (1..=3).map(|k| T3_NEW_BASE + k));
        assert_eq!(
            h.walk_ids(),
            expected,
            "construction (e): the spawns land at the end of MARKED"
        );
        t3_step(&mut h, true, "(e)");
        let delta = h.scratch().row_remap_searched() - before;
        assert_eq!(
            delta, 0,
            "T3 (e): three flagged spawns sent {delta} rows to stage 2 (expected 0)"
        );
        h.assert_walk_matches_gather();
        for _ in 0..50 {
            t3_step(&mut h, false, "50 steps after (e)");
        }
        t3_assert_no_resets(&h, "50 steps after (e)");
    }

    // ── T4: determinism under churn ──────────────────────────────────────────

    /// Every surviving body's state as bits (position, velocity, angular velocity, rotation),
    /// sorted by body id.
    type BodyBits = (u32, [u32; 13]);

    /// What one churn run produced.
    struct ChurnRun {
        state: Vec<BodyBits>,
        walks: Vec<Vec<u32>>,
        builds: u64,
        latched_before_churn: usize,
        newcomer_y: f32,
    }

    /// Sleeping on and warm start on: M in MARKED, the floor, the pile, a two-cube stack and a
    /// faller in PLAIN. After 150 steps: P2 is despawned, E spawned (recycling P2's id), M loses
    /// its marker (every PLAIN row shifts) and the faller is despawned, with steps between.
    fn churn_run() -> ChurnRun {
        let mut h = Harness::new(Solve::Colored, true);
        let migrant = h.spawn(ball(MIGRANT, Vec3::new(-4.0, 0.5, 0.0)), true);
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        let pile = h.spawn_pile();
        h.spawn(cube(CUBE_A, Vec3::new(6.0, 0.5, 0.0)), false);
        h.spawn(cube(CUBE_B, Vec3::new(6.0, 1.5, 0.0)), false);
        let faller = h.spawn(ball(FALLER, Vec3::new(50.0, 4.0, 0.0)), false);
        let mut walks = vec![h.walk_ids()];

        h.steps(150);
        let latched_before_churn = PILE.len() - h.awake_of(&PILE).len();
        assert!(
            h.world.delete_entity(pile[1]),
            "construction: P2 despawnable"
        );
        walks.push(h.walk_ids());
        h.steps(5);
        let newcomer = h.spawn(ball(NEWCOMER, Vec3::new(5.0, 4.0, 0.0)), false);
        walks.push(h.walk_ids());
        h.steps(5);
        h.remove_marker(migrant);
        walks.push(h.walk_ids());
        h.steps(5);
        assert!(
            h.world.delete_entity(faller),
            "construction: faller despawnable"
        );
        walks.push(h.walk_ids());
        h.steps(30);

        let mut state: Vec<BodyBits> = {
            let q = h.world.query::<(&RigidBody, &BodyId), ()>();
            q.iter()
                .map(|(b, id)| {
                    let v = [
                        b.position.x,
                        b.position.y,
                        b.position.z,
                        b.linear_velocity.x,
                        b.linear_velocity.y,
                        b.linear_velocity.z,
                        b.angular_velocity.x,
                        b.angular_velocity.y,
                        b.angular_velocity.z,
                        b.rotation.x,
                        b.rotation.y,
                        b.rotation.z,
                        b.rotation.w,
                    ];
                    (id.id, v.map(f32::to_bits))
                })
                .collect()
        };
        state.sort_unstable_by_key(|&(id, _)| id);
        ChurnRun {
            state,
            walks,
            builds: h.scratch().row_remap_builds(),
            latched_before_churn,
            newcomer_y: h.body(newcomer).position.y,
        }
    }

    /// T4: the scripted churn run twice in fresh worlds ends bit-identical per entity. The row
    /// identity map's process-wide epoch differs between the two runs, so this also shows that
    /// no sequence value leaks into a result.
    #[test]
    fn structural_churn_is_run_to_run_bit_deterministic() {
        let first = churn_run();
        let second = churn_run();
        assert_eq!(
            first.walks, second.walks,
            "construction: both runs walk the same rows"
        );
        assert!(
            first.builds >= 5 && first.latched_before_churn == PILE.len() && first.newcomer_y < 3.0,
            "anti-vacuity: the churn must rebuild maps (builds {} >= 5), run with a latched pile ({} of \
             4 latched) and move the newcomer (y {} < 3)",
            first.builds,
            first.latched_before_churn,
            first.newcomer_y
        );
        assert_eq!(
            first.state.len(),
            second.state.len(),
            "T4: surviving body counts differ"
        );
        for (a, b) in first.state.iter().zip(&second.state) {
            assert_eq!(
                a, b,
                "T4: body {} ended in different bits across two identical churn runs (position, \
                 velocity, angular velocity, rotation as f32 bits)",
                a.0
            );
        }
    }

    // ── T8 / T8b: the sleep latch's cursor ───────────────────────────────────

    /// T8: a latched pile; sleeping is switched off for one step and back on. The next step must
    /// count exactly one reset and wake the pile; the pile must re-latch within
    /// `SLEEP_FRAMES + 4` steps and stay latched for 30 more with no further reset. Red under M4
    /// (the `Reset` arm does not stamp: every step is a `Reset` and the pile never re-latches).
    #[test]
    fn sleep_consumer_relatches_after_a_reset() {
        let mut h = Harness::new(Solve::Colored, true);
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        h.spawn_pile();
        let settle_steps = h.settle_until_latched(&PILE);
        assert_eq!(
            h.sleep().remap_resets(),
            0,
            "T8 step 1: a fresh latch's first step is Rows, never a counted Reset (P17)"
        );
        let r0 = 0;

        h.set_sleeping(false);
        h.step();
        h.set_sleeping(true);
        h.step();
        let resets = h.sleep().remap_resets();
        let awake = h.awake_of(&PILE);
        assert!(
            resets == r0 + 1 && awake.len() == PILE.len(),
            "T8 step 3: re-enabling sleeping after a missed gather must Reset once and wake the pile; \
             remap_resets {resets} (expected {}), pile rows awake {awake:?} (pile latched after \
             {settle_steps} steps)",
            r0 + 1
        );

        let budget = SLEEP_FRAMES as usize + 4;
        let relatched = (1..=budget).find(|_| {
            h.step();
            h.awake_of(&PILE).is_empty()
        });
        let awake = h.awake_of(&PILE);
        let resets = h.sleep().remap_resets();
        assert!(
            relatched.is_some(),
            "T8 phase 4: pile must re-latch within {budget} steps after a Reset; awake {awake:?}; \
             remap_resets {resets} (expected {})",
            r0 + 1
        );
        for k in 1..=30 {
            h.step();
            let awake = h.awake_of(&PILE);
            let resets = h.sleep().remap_resets();
            assert!(
                awake.is_empty() && resets == r0 + 1,
                "T8 phase 4, step {k} after re-latching: pile rows awake {awake:?} (expected none), \
                 remap_resets {resets} (expected {})",
                r0 + 1
            );
        }
    }

    /// T8b: a latched pile; `Simulated` is disabled on every dynamic body, so the colored
    /// solve takes its no-dynamic-body early return; then it is re-enabled. The latch was
    /// re-keyed before that return, so the next step is not a reset and the pile stays
    /// latched. Red under M8 (the re-key placed after the early return: the next step is a
    /// counted Reset and the pile wakes).
    #[test]
    fn sleep_consumer_carries_through_a_solve_that_returns_early() {
        let mut h = Harness::new(Solve::Colored, true);
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        let pile = h.spawn_pile();
        let settle_steps = h.settle_until_latched(&PILE);
        assert_eq!(
            h.sleep().remap_resets(),
            0,
            "T8b step 1: a latched pile has counted no reset"
        );
        let r0 = 0;

        let solved_before = h.solved_steps();
        assert!(
            solved_before > 0,
            "anti-vacuity: the solve ran while the pile settled"
        );
        for &e in &pile {
            h.world.disable::<Simulated>(e);
        }
        h.step();
        assert_eq!(
            h.solved_steps(),
            solved_before,
            "construction: with no simulated dynamic body the colored solve must take its early \
             return, so solved_steps must not move across the step"
        );

        for &e in &pile {
            h.world.enable::<Simulated>(e);
        }
        h.step();
        assert_eq!(
            h.solved_steps(),
            solved_before + 1,
            "construction: the re-enabled step must run the solve"
        );
        let resets = h.sleep().remap_resets();
        let awake = h.awake_of(&PILE);
        assert!(
            resets == r0 && awake.is_empty(),
            "T8b: remap_resets {resets} (expected {r0}); pile rows awake {awake:?} after an early-return \
             step (expected none; pile latched after {settle_steps} steps)"
        );
    }

    // ── T9: the warm-start tables' cursors ───────────────────────────────────

    /// T9 on one solver: static box floor F; cubes A (0, 0.5, 0), B (5, 0.5, 5), C (-5, 0.5, -5);
    /// sleeping off.
    fn warm_start_consumer_scene(solve: Solve) {
        let mut h = Harness::new(solve, false);
        h.spawn(floor(FLOOR_HALF_EXTENTS), false);
        let a = h.spawn(cube(CUBE_A, Vec3::new(0.0, 0.5, 0.0)), false);
        let b = h.spawn(cube(CUBE_B, Vec3::new(5.0, 0.5, 5.0)), false);
        let c = h.spawn(cube(CUBE_C, Vec3::new(-5.0, 0.5, -5.0)), false);
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, CUBE_A, CUBE_B, CUBE_C],
            "construction: walk order"
        );

        // Phase 1.
        h.steps(120);
        let st = h.warm_stats();
        assert!(
            st.remap_resets == 0 && st.manifolds >= 3 && st.translated == st.manifolds,
            "T9 [{solve:?}] phase 1: after 120 quiet steps expected remap_resets 0, manifolds >= 3, \
             translated == manifolds; got {st:?}"
        );
        h.assert_walk_matches_gather();

        // Phase 2: carry across a row move.
        assert!(h.world.delete_entity(a), "construction: A despawnable");
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, CUBE_C, CUBE_B],
            "construction: C must swap-move into A's row 1"
        );
        let builds = h.scratch().row_remap_builds();
        h.step();
        assert_eq!(
            h.scratch().row_remap_builds(),
            builds + 1,
            "construction: phase 2 must be a change step"
        );
        let st = h.warm_stats();
        assert!(
            st.translated == st.manifolds && st.manifolds >= 2 && st.remap_resets == 0,
            "T9 [{solve:?}] phase 2: translated {} of {} on a row-move step (expected all; C's pair \
             (0, 1) maps to old (0, 3)), remap_resets {} (expected 0)",
            st.translated,
            st.manifolds,
            st.remap_resets
        );
        h.assert_walk_matches_gather();

        // Phase 3: a step whose solve returns early, so the table is not stored.
        h.world.disable::<Simulated>(b);
        h.world.disable::<Simulated>(c);
        assert!(h.world.delete_entity(c), "construction: C despawnable");
        assert_eq!(
            h.walk_ids(),
            vec![FLOOR, CUBE_B],
            "construction: B must swap-move into row 1"
        );
        let solved = h.solved_steps();
        h.step();
        assert_eq!(
            h.solved_steps(),
            solved,
            "construction [{solve:?}] phase 3: with no simulated dynamic body the solve must take its \
             early return, so solved_steps must not move"
        );

        // Phase 4: the table missed a store.
        h.world.enable::<Simulated>(b);
        h.step();
        assert_eq!(
            h.solved_steps(),
            solved + 1,
            "construction: phase 4 must run the solve"
        );
        let st = h.warm_stats();
        assert!(
            st.translated == 0 && st.manifolds >= 1 && st.remap_resets == 1,
            "T9 [{solve:?}] phase 4: one step after a missed store expected translated 0 of >= 1, \
             remap_resets 1; got translated {} of {}, remap_resets {}",
            st.translated,
            st.manifolds,
            st.remap_resets
        );

        // Phase 5: out of Reset after one consumption.
        h.step();
        let st = h.warm_stats();
        assert!(
            st.translated == st.manifolds && st.manifolds >= 1,
            "T9 [{solve:?}] phase 5: translated {} of {} one step after a Reset (expected all); \
             remap_resets {}",
            st.translated,
            st.manifolds,
            st.remap_resets
        );
        h.steps(50);
        let st = h.warm_stats();
        assert_eq!(
            st.remap_resets, 1,
            "T9 [{solve:?}] phase 5: remap_resets after 50 more quiet steps (expected 1); {st:?}"
        );

        // Phase 6: a replaced solver resource.
        h.replace_solver();
        h.step();
        let st = h.warm_stats();
        assert!(
            st.remap_resets == 0 && st.translated == 0 && st.manifolds >= 1,
            "T9 [{solve:?}] phase 6: a replaced solver's first step is an uncounted Reset; expected \
             remap_resets 0, translated 0 of >= 1; got {st:?}"
        );
        h.step();
        let st = h.warm_stats();
        assert!(
            st.translated == st.manifolds && st.manifolds >= 1,
            "T9 [{solve:?}] phase 6: the replaced solver's second step must carry; got {st:?}"
        );
        h.steps(20);
        let st = h.warm_stats();
        assert_eq!(
            st.remap_resets, 0,
            "T9 [{solve:?}] phase 6: remap_resets after 20 more quiet steps (expected 0); {st:?}"
        );
    }

    /// T9 on the colored solver. Red under M2 (phase 2: translated 1 of 2) and M6 (phase 5:
    /// translated 0 one step after a Reset). M7 stays green here by construction: the colored
    /// solve classifies after its early return.
    #[test]
    fn warm_start_consumer_carries_and_leaves_reset_colored() {
        warm_start_consumer_scene(Solve::Colored);
    }

    /// T9 on the serial solver. Red under M2, M6 and M7 (phase 4: the lookup at phase 3 stamped
    /// the table, so phase 4 is `Identity`, translated == manifolds and remap_resets 0).
    #[test]
    fn warm_start_consumer_carries_and_leaves_reset_serial() {
        warm_start_consumer_scene(Solve::Serial);
    }
}
