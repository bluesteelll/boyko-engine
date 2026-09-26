//! L10 D9b, the step record ([`StepInputs`], `levers/L10-sleeping/10-DESIGN-D9B.md`): the inputs
//! one physics step runs with, latched by its broadphase.
//!
//! **Invariant D9b: one latch per input per step.** Each input that carries a value has exactly
//! one latch point per step, and every stage after that point reads the latched value. For
//! [`PhysicsConfig`], [`SdfField`] and the `IslandSleep::wake_all` request that point is the
//! first statement of the step's broadphase, which copies them into this record. Every later
//! stage of the step — the narrowphase, the SDF stage, the solve and the soft steps — reads the
//! record, never the resources. So a write made after the broadphase takes effect at the next
//! broadphase, in every sleep-skip mode, whatever the schedule's topology, and a write undone
//! before the next broadphase is never seen.
//!
//! # The contract (design 10, §1.5)
//!
//! | class | members | contract |
//! |---|---|---|
//! | Value inputs | [`PhysicsConfig`], [`SdfField`] | Any write lands at the next broadphase: a field write through `ResMut`, assigning a whole new value, `insert_resource` over the existing one, or a `Commands` closure doing any of these. A value must respect the pipeline's wiring (`broadphase == Grid` on the coupling path). Removing either resource after setup is outside the contract. |
//! | Request input | `IslandSleep::wake_all` | The requests are counted; the broadphase latches the count and the next solve serves exactly that count. |
//! | Broadphase knobs | `BroadphaseTree::set_brute_max_rows`, `set_query_kernel` | Read only by the broadphase. Inside the contract. |
//! | Body components | `RigidBody`, `RigidBodyMass`, `Collider`, `Sensor`, `Simulated`, `Kinematic` | Latched by the gather. A value write to a row the step writes back (every awake, simulated, dynamic row), made between the gather and the write-back, is overwritten or combined with the step's write; the same write at a step boundary takes effect. Structural changes between the gather and the apply are outside the contract. |
//! | Pipeline state | `SolverScratch`, `ContactPairs`, `Manifolds`, `ConstraintGraph`, the tree's `step_direct`, `SoftRigidReaction`, the solvers' `solve_*` entries, `SleepSets`, this record | Outside the contract at any time after setup. |
//! | Stateful resources, replaced or removed | `IslandSleep`, `ColoredSoftStepSolver`, `SoftStepSolver`, `Manifolds`, `SleepSets`, `SolverScratch`, `ContactPairs`, `ConstraintGraph`, `BroadphaseTree`, `BroadphaseGrid`, `SoftRigidReaction`, `SoftColorScratch`, this record | Outside the contract after the first step; before it, it is setup. |
//!
//! A value input has no state beyond its value, so the latch copies whatever value is present and
//! no reader can tell how it arrived. A stateful resource replaced mid-step has no exact answer,
//! even as a latch, because one sleep-skip mode keeps state in it that the other rebuilds.
//!
//! # Cost
//!
//! One copy per step: the configuration (~100 B) and the wake count, plus the field (1 568 B) on
//! a world that has one. No heap, no atomics, no new read in any hot loop.

use boyko_macros::Resource;

use crate::resources::PhysicsConfig;
use crate::row_identity::RowIdentity;
use crate::sdf_query::SdfField;

/// The inputs this step runs with, written only by the broadphase (L10 D9b).
///
/// The broadphase latches [`PhysicsConfig`], the [`SdfField`] (when the world has one) and the
/// wake request count into this record before anything else, and every later stage of the step
/// reads them here. A write to either resource after the broadphase takes effect at the next
/// broadphase. See the module docs for the contract.
///
/// Pipeline state: replacing or removing it is outside the contract, and it has no public
/// constructor or mutator.
#[derive(Resource, Debug)]
#[repr(C, align(64))]
pub struct StepInputs {
    /// The field this step runs with, valid iff `has_field`. First, so its edit array starts on a
    /// cache line: the SDF kernels stream it from here.
    field: SdfField,
    /// The configuration this step runs with.
    cfg: PhysicsConfig,
    /// The gather sequence of the latch (`RowIdentity::gather_seq`); `0` = never latched, which no
    /// gather ever stamps.
    seq: u64,
    /// `IslandSleep::wake_all` requests counted at the latch; `0` in a world without
    /// `IslandSleep`.
    wake_requests: u64,
    /// Whether the latching world had an [`SdfField`].
    has_field: bool,
}

const _: () = assert!(align_of::<StepInputs>() == 64, "StepInputs starts on a cache line (D9b)");

impl StepInputs {
    /// A record no broadphase has latched: the default configuration, no field.
    pub(crate) fn new() -> Self {
        Self {
            field: SdfField::default(),
            cfg: PhysicsConfig::default(),
            seq: 0,
            wake_requests: 0,
            has_field: false,
        }
    }

    /// The configuration this step runs with.
    #[inline]
    pub fn config(&self) -> &PhysicsConfig {
        &self.cfg
    }

    /// The field this step runs with, or `None` when the world has no [`SdfField`].
    #[inline]
    pub fn sdf_field(&self) -> Option<&SdfField> {
        self.has_field.then_some(&self.field)
    }

    /// Latches the step's inputs: `cfg`, `field`, the gather sequence `seq` and the request count
    /// `wake_requests`. The first statement of every broadphase.
    #[inline]
    pub(crate) fn latch(
        &mut self,
        cfg: &PhysicsConfig,
        field: Option<&SdfField>,
        seq: u64,
        wake_requests: u64,
    ) {
        self.cfg = *cfg;
        if let Some(field) = field {
            self.field = *field;
        }
        self.has_field = field.is_some();
        self.seq = seq;
        self.wake_requests = wake_requests;
    }

    /// The `IslandSleep::wake_all` requests counted at the latch: the count this step's solve
    /// serves.
    #[inline]
    pub(crate) fn wake_requests(&self) -> u64 {
        self.wake_requests
    }

    /// This record, checked in debug builds to be the one `rows`' gather latched: every reader
    /// that holds the gather's `SolverScratch` reads the record through it.
    #[inline]
    pub(crate) fn for_gather(&self, rows: &RowIdentity) -> &Self {
        debug_assert_eq!(
            self.seq,
            rows.gather_seq(),
            "invariant: a stage after the broadphase reads the record its gather's broadphase latched"
        );
        self
    }
}

#[cfg(test)]
mod tests {
    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
    use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
    use boyko_ecs::ecs::core::system::{Access, IntoSystem, ResMut, System};
    use boyko_sdf_math::{SdfEdit, sdf_op};
    use boyko_threadpool::ThreadPoolBuilder;

    use super::StepInputs;
    use crate::math::Vec3;
    use crate::plugin::{
        PhysicsStageKeys, SceneSyncKeys, add_physics_sdf, add_physics_soft,
        add_physics_soft_colored, add_physics_systems_with_scene_sync,
    };
    use crate::resources::{
        BroadphaseKind, BroadphaseSelectMode, PhysicsConfig, SdfNarrowphaseKernel, SleepSkip,
    };
    use crate::row_identity::{RowIdentity, RowKey};
    use crate::scene_sync::{
        debug_assert_dynamic_bodies_are_roots, sync_body_to_transform, sync_transform_to_body,
    };
    use crate::sdf_query::SdfField;
    use crate::soft::{
        physics_soft_rigid_apply, physics_soft_step, physics_soft_step_colored,
        physics_soft_step_colored_latched, physics_soft_step_coupled,
        physics_soft_step_coupled_latched, physics_soft_step_latched,
    };
    use crate::solver::{DefaultRigidSolver, SoftStepSolver};
    use crate::systems::{
        physics_apply, physics_broadphase, physics_broadphase_colored,
        physics_broadphase_colored_sdf, physics_build_graph, physics_gather, physics_integrate,
        physics_narrowphase, physics_narrowphase_colored, physics_narrowphase_sdf,
        physics_solve_colored, physics_solve_step,
    };
    use crate::broadphase_policy::select_broadphase;

    /// A configuration with every field moved off its default. The struct literal names every
    /// field, so a field added to `PhysicsConfig` fails to compile here until it is given one.
    fn perturbed() -> PhysicsConfig {
        PhysicsConfig {
            gravity: Vec3::new(0.25, -3.0, 0.5),
            dt: 1.0 / 90.0,
            substeps: 7,
            relax_iterations: 5,
            contact_hertz: 41.0,
            contact_damping: 3.5,
            broadphase: BroadphaseKind::Tree,
            broadphase_select: BroadphaseSelectMode::Auto,
            simd: false,
            simd_solve: false,
            sdf_narrowphase: SdfNarrowphaseKernel::Avx2,
            parallel_broadphase: true,
            colored: true,
            parallel_solve: false,
            parallel_narrowphase: false,
            contact_reuse: false,
            contact_reuse_distance: 0.0025,
            sleeping: true,
            sleep_skip: SleepSkip::Off,
            sleep_threshold: 0.125,
            sleep_frames: 9,
            soft_body: true,
            soft_damping: 0.25,
            soft_rest_clamp: true,
            soft_rigid_coupling: true,
            self_collision_iters: 3,
            soft_body_colored: true,
            soft_self_collision_colored: true,
        }
    }

    /// Every field of `a` equals `b`'s, floats by their bits. The destructures are exhaustive, so
    /// a new field fails to compile here until it is compared.
    fn assert_config_eq(a: &PhysicsConfig, b: &PhysicsConfig) {
        let PhysicsConfig {
            gravity,
            dt,
            substeps,
            relax_iterations,
            contact_hertz,
            contact_damping,
            broadphase,
            broadphase_select,
            simd,
            simd_solve,
            sdf_narrowphase,
            parallel_broadphase,
            colored,
            parallel_solve,
            parallel_narrowphase,
            contact_reuse,
            contact_reuse_distance,
            sleeping,
            sleep_skip,
            sleep_threshold,
            sleep_frames,
            soft_body,
            soft_damping,
            soft_rest_clamp,
            soft_rigid_coupling,
            self_collision_iters,
            soft_body_colored,
            soft_self_collision_colored,
        } = *a;
        let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        assert_eq!(bits(gravity), bits(b.gravity), "gravity");
        assert_eq!(dt.to_bits(), b.dt.to_bits(), "dt");
        assert_eq!(substeps, b.substeps, "substeps");
        assert_eq!(relax_iterations, b.relax_iterations, "relax_iterations");
        assert_eq!(contact_hertz.to_bits(), b.contact_hertz.to_bits(), "contact_hertz");
        assert_eq!(contact_damping.to_bits(), b.contact_damping.to_bits(), "contact_damping");
        assert_eq!(broadphase, b.broadphase, "broadphase");
        assert_eq!(broadphase_select, b.broadphase_select, "broadphase_select");
        assert_eq!(simd, b.simd, "simd");
        assert_eq!(simd_solve, b.simd_solve, "simd_solve");
        assert_eq!(sdf_narrowphase, b.sdf_narrowphase, "sdf_narrowphase");
        assert_eq!(parallel_broadphase, b.parallel_broadphase, "parallel_broadphase");
        assert_eq!(colored, b.colored, "colored");
        assert_eq!(parallel_solve, b.parallel_solve, "parallel_solve");
        assert_eq!(parallel_narrowphase, b.parallel_narrowphase, "parallel_narrowphase");
        assert_eq!(contact_reuse, b.contact_reuse, "contact_reuse");
        assert_eq!(
            contact_reuse_distance.to_bits(),
            b.contact_reuse_distance.to_bits(),
            "contact_reuse_distance"
        );
        assert_eq!(sleeping, b.sleeping, "sleeping");
        assert_eq!(sleep_skip, b.sleep_skip, "sleep_skip");
        assert_eq!(sleep_threshold.to_bits(), b.sleep_threshold.to_bits(), "sleep_threshold");
        assert_eq!(sleep_frames, b.sleep_frames, "sleep_frames");
        assert_eq!(soft_body, b.soft_body, "soft_body");
        assert_eq!(soft_damping.to_bits(), b.soft_damping.to_bits(), "soft_damping");
        assert_eq!(soft_rest_clamp, b.soft_rest_clamp, "soft_rest_clamp");
        assert_eq!(soft_rigid_coupling, b.soft_rigid_coupling, "soft_rigid_coupling");
        assert_eq!(self_collision_iters, b.self_collision_iters, "self_collision_iters");
        assert_eq!(soft_body_colored, b.soft_body_colored, "soft_body_colored");
        assert_eq!(
            soft_self_collision_colored, b.soft_self_collision_colored,
            "soft_self_collision_colored"
        );
    }

    /// The edit bits of a field, for a value comparison.
    fn edit_bits(field: &SdfField) -> Vec<[u32; 4]> {
        field
            .edits()
            .iter()
            .map(|e| [e.center[0].to_bits(), e.params[0].to_bits(), e.kind, e.op])
            .collect()
    }

    fn floor_field(top: f32) -> SdfField {
        SdfField::from_edits(&[SdfEdit::box_shape(
            [0.0, top - 50.0, 0.0],
            [50.0, 50.0, 50.0],
            sdf_op::UNION,
            0.0,
        )])
    }

    /// One scripted gather into `rows`, `n` rows at generation 0.
    fn gather(rows: &mut RowIdentity, n: usize) {
        rows.begin_gather();
        {
            let (mut cur, _) = rows.gather_views();
            for id in 0..n {
                cur.push(RowKey::new(id, 0));
            }
        }
        rows.finish_gather();
    }

    #[test]
    fn latch_round_trips_every_config_field_and_the_wake_count() {
        let mut inputs = StepInputs::new();
        let cfg = perturbed();
        inputs.latch(&cfg, None, 7, 42);
        assert_config_eq(inputs.config(), &cfg);
        assert_eq!(inputs.wake_requests(), 42, "the latched wake count");
        assert_eq!(inputs.seq, 7, "the latched gather sequence");
        // Anti-vacuity: the perturbed configuration differs from the default in every field the
        // round trip compares, so a latch that kept `new()`'s value would fail above.
        let default = PhysicsConfig::default();
        let caught = std::panic::catch_unwind(|| assert_config_eq(&default, &cfg));
        assert!(caught.is_err(), "anti-vacuity: the perturbed config equals the default");
    }

    #[test]
    fn latch_without_a_field_has_none_and_with_one_copies_it() {
        let mut inputs = StepInputs::new();
        assert!(inputs.sdf_field().is_none(), "a never-latched record has no field");
        let field = floor_field(0.25);
        inputs.latch(&PhysicsConfig::default(), Some(&field), 1, 0);
        let latched = inputs.sdf_field().expect("a latch with a field keeps it");
        assert_eq!(edit_bits(latched), edit_bits(&field), "the latched field is the value copied");
        assert!(!latched.is_empty(), "anti-vacuity: the field has an edit");
        inputs.latch(&PhysicsConfig::default(), None, 2, 0);
        assert!(inputs.sdf_field().is_none(), "latch(None) gives sdf_field() == None");
    }

    #[test]
    fn for_gather_accepts_the_latching_gather() {
        let mut rows = RowIdentity::with_capacity(0);
        gather(&mut rows, 3);
        let mut inputs = StepInputs::new();
        inputs.latch(&PhysicsConfig::default(), None, rows.gather_seq(), 0);
        assert!(std::ptr::eq(inputs.for_gather(&rows), &inputs), "for_gather returns the record");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "reads the record its gather's broadphase latched")]
    fn for_gather_panics_on_a_stale_record() {
        let mut rows = RowIdentity::with_capacity(0);
        gather(&mut rows, 3);
        let mut inputs = StepInputs::new();
        inputs.latch(&PhysicsConfig::default(), None, rows.gather_seq(), 0);
        gather(&mut rows, 3);
        let _ = inputs.for_gather(&rows);
    }

    // ── G-ACCESS (design 10, Tests C) ──────────────────────────────────────────────────────

    /// The probe: a writer of both value inputs.
    #[allow(clippy::needless_pass_by_value)]
    fn writer(_: ResMut<PhysicsConfig>, _: ResMut<SdfField>) {}

    /// `system`'s name, and whether its access surface, initialised on `world`, conflicts with
    /// `probe`'s.
    fn conflicts<S: System>(world: &mut EcsMaster, mut system: S, probe: &Access) -> (&'static str, bool) {
        system.initialize(world);
        (system.name(), system.access().conflicts_with(probe))
    }

    /// The primary completeness check for the scene-sync stages: every field, classified.
    fn classify_scene_sync(keys: SceneSyncKeys) {
        let SceneSyncKeys {
            transform_to_body: _,      // before the latch: the S5 head (Transform in)
            body_to_transform: _,      // after: sync_body_to_transform
            parented_dynamic_guard: _, // after: debug_assert_dynamic_bodies_are_roots
        } = keys;
    }

    /// Every system after the broadphase: none may conflict with a writer of `PhysicsConfig` or
    /// `SdfField` (they read the record), and the broadphase variants and the public standalone
    /// soft forms must (the probe is not vacuous). Named systems are initialised on a world wired
    /// with the coupled soft pipeline plus an `SdfField`.
    ///
    /// The primary completeness check is the exhaustive destructure of [`PhysicsStageKeys`] and
    /// [`SceneSyncKeys`] below: a new keyed stage fails to compile until it is classified. The
    /// secondary check runs where system zones are compiled: every system of four pipeline
    /// shapes must be in the classified union.
    #[test]
    fn g_access_no_stage_after_the_broadphase_reads_a_value_input_live() {
        let mut world = EcsMaster::new();
        let mut builder = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
        let keys = add_physics_soft::<DefaultRigidSolver>(&mut builder, &mut world, true);
        world.insert_resource(floor_field(0.0));
        let mut probe = IntoSystem::into_system(writer);
        probe.initialize(&mut world);
        let probe = probe.access();

        // The primary completeness check: every keyed stage, classified.
        let PhysicsStageKeys {
            integrate: _,         // before the latch: integrate (reads the live gravity)
            gather: _,            // before the latch: the gather (stamps `dt`)
            select_broadphase: _, // before the latch: the density policy (writes `broadphase`)
            broadphase: _,        // the latch: the three broadphase variants (must conflict)
            narrowphase: _,       // after: physics_narrowphase, physics_narrowphase_colored
            narrowphase_sdf: _,   // after: physics_narrowphase_sdf
            build_graph: _,       // after: physics_build_graph
            solve: _,             // after: physics_solve_colored, physics_solve_step::<S>
            soft_step: _,         // after: the three *_latched soft forms
            apply: _,             // after: physics_apply
            scene_sync,           // classify_scene_sync
        } = keys;
        if let Some(scene_sync) = scene_sync {
            classify_scene_sync(scene_sync);
        }
        // Unkeyed: `physics_soft_rigid_apply`, registered `.after(apply)` on the coupling path.

        let after = [
            conflicts(&mut world, IntoSystem::into_system(physics_narrowphase), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_narrowphase_colored), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_narrowphase_sdf), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_build_graph), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_solve_colored), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_solve_step::<SoftStepSolver>), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_apply), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_soft_step_latched), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_soft_step_coupled_latched), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_soft_step_colored_latched), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_soft_rigid_apply), probe),
            conflicts(&mut world, IntoSystem::into_system(sync_body_to_transform), probe),
            conflicts(&mut world, IntoSystem::into_system(debug_assert_dynamic_bodies_are_roots), probe),
        ];
        for (name, conflict) in after {
            assert!(
                !conflict,
                "G-ACCESS: {name} runs after the broadphase and conflicts with a writer of                  PhysicsConfig or SdfField: it must read the step record (L10 D9b)"
            );
        }
        let latch_and_standalone = [
            conflicts(&mut world, IntoSystem::into_system(physics_broadphase), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_broadphase_colored), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_broadphase_colored_sdf), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_soft_step), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_soft_step_coupled), probe),
            conflicts(&mut world, IntoSystem::into_system(physics_soft_step_colored), probe),
        ];
        for (name, conflict) in latch_and_standalone {
            assert!(
                conflict,
                "G-ACCESS anti-vacuity: {name} reads the value inputs live, so it must conflict                  with the probe"
            );
        }

        // The secondary check: every system of four pipeline shapes is classified.
        let before = [
            IntoSystem::into_system(sync_transform_to_body).name(),
            IntoSystem::into_system(physics_integrate).name(),
            IntoSystem::into_system(physics_gather).name(),
            IntoSystem::into_system(select_broadphase).name(),
        ];
        let classified: Vec<&str> = before
            .into_iter()
            .chain(after.iter().map(|&(n, _)| n))
            .chain(latch_and_standalone.iter().map(|&(n, _)| n))
            .collect();
        if boyko_ecs::ecs::core::profiling::SYSTEM_ZONES_COMPILED {
            type Wire = fn(&mut ScheduleBuilder, &mut EcsMaster) -> PhysicsStageKeys;
            let shapes: [(&str, Wire); 4] = [
                ("add_physics_sdf", add_physics_sdf::<DefaultRigidSolver>),
                ("add_physics_soft(.., true)", |b, w| add_physics_soft::<DefaultRigidSolver>(b, w, true)),
                ("add_physics_soft_colored", add_physics_soft_colored::<DefaultRigidSolver>),
                ("add_physics_systems_with_scene_sync", add_physics_systems_with_scene_sync::<DefaultRigidSolver>),
            ];
            let mut listed = 0usize;
            for (shape, wire) in shapes {
                let mut world = EcsMaster::new();
                let mut builder =
                    ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(1).build());
                let _ = wire(&mut builder, &mut world);
                let schedule = builder.build(&mut world);
                for (name, _) in schedule.system_zones() {
                    assert!(
                        classified.contains(&name),
                        "G-ACCESS completeness: {shape} registers {name}, which no G-ACCESS row \
                         classifies"
                    );
                    listed += 1;
                }
            }
            assert!(listed > 0, "G-ACCESS anti-vacuity: the zone listing named no system");
            println!("G-ACCESS: checks ran: access sets, key destructure, system zones ({listed} names)");
        } else {
            println!("G-ACCESS: checks ran: access sets, key destructure; system zones NOT compiled");
        }
    }
}
