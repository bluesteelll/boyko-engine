//! Dynamic parity scenes (lane DYN-SCENES): the runner's glue between the shared program
//! specification ([`crate::dyn_spec`]) and this engine.
//!
//! Everything here runs OUTSIDE the timed pair: [`configure_world`] after the scene is built,
//! [`DynRun::before_step`] between the previous step's readout and the next `Instant::now()`, and
//! [`DynRun::after_step`] / [`DynRun::finish`] after the pair closed. A row without `--dyn`,
//! `--sanity` or `--scene-dump` never builds a [`DynRun`], so its run is the runner's unchanged.
//!
//! * Kicks are written to `RigidBody::linear_velocity` at a step boundary, which the step-input
//!   contract admits (`boyko_physics::step_inputs`, "Body components"), and read back.
//! * Launches spawn a J-T box with a velocity, appended to the run's bodies in spawn order.
//! * `--sanity` reads, per state, the energies and bounds of the sanity CSV, and per step the
//!   static-pair receipts (ruling 22): candidate pairs with a static endpoint, static-static pairs,
//!   and their manifolds — the work the arena's large statics add, which S-SHOOT and S-SLIDE
//!   measure by declaration.
//! * `--scene-dump` writes the canonical scene dump from values READ BACK from the world.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_physics::components::{
    Collider, ColliderShape, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{ContactPairs, Manifolds, PhysicsConfig, SolverScratch};

use crate::dyn_spec::{self, BodyLine, KickEvent, Launch, Program, SanityRow, StaticLine};
use crate::{BOX_INV_INERTIA, BOX_INV_MASS, HALF_BOX, JOLT_FRICTION, Rig, as_bytes, fnv1a64, json_f64};

/// Spawns one box into the `RigidBodyBundle` archetype: a static one (not `Simulated`, zero
/// inverse mass) of `half` extents, or a dynamic J-T box moving at `velocity`. The components are
/// the runner's `spawn_box`'s, so a launched box is a J-T box; the dump's read-back proves it.
fn spawn(world: &mut EcsMaster, position: Vec3, velocity: Vec3, half: Option<Vec3>) -> Entity {
    let body = RigidBody { position, linear_velocity: velocity, rotation: Quat::IDENTITY, angular_velocity: Vec3::ZERO };
    let (mass, half_extents) = match half {
        Some(h) => (
            RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass: 0.0, restitution: 0.0, friction: JOLT_FRICTION },
            h,
        ),
        None => (
            RigidBodyMass {
                inv_inertia: Mat3::from_diagonal(Vec3::new(BOX_INV_INERTIA, BOX_INV_INERTIA, BOX_INV_INERTIA)),
                inv_mass: BOX_INV_MASS,
                restitution: 0.0,
                friction: JOLT_FRICTION,
            },
            Vec3::new(HALF_BOX, HALF_BOX, HALF_BOX),
        ),
    };
    let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let e = world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("construction: the RigidBodyBundle archetype accepts the three columns");
    if half.is_none() {
        world.enable::<Simulated>(e);
    }
    e
}

/// Builds `program`'s additions to the J-T world `build` spawned: the statics after the floor
/// (created after the J-T boxes, as in every harness) and the gravity override. Called after
/// `configure`, so the override is the value the world runs with.
pub fn configure_world(world: &mut EcsMaster, statics: &mut Vec<Entity>, program: Program) {
    for s in &program.statics()[1..] {
        let v = |a: [i32; 3]| Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
        statics.push(spawn(world, v(s.center), Vec3::ZERO, Some(v(s.half))));
    }
    let g = program.gravity();
    world.resource_mut::<PhysicsConfig>().gravity = Vec3::new(g[0], g[1], g[2]);
}

fn v3(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn diag(m: &Mat3) -> [f32; 3] {
    [m.rows[0].x, m.rows[1].y, m.rows[2].z]
}

/// The box's half-extents, or NaN for a collider that is not a box (the dump then cannot match).
fn half_extents(c: &Collider) -> [f32; 3] {
    match c.shape {
        ColliderShape::Box { half_extents } => v3(half_extents),
        _ => [f32::NAN; 3],
    }
}

/// The static-pair receipts of one step (ruling 22), from the narrowphase's input and output.
#[derive(Clone, Copy, Default, Debug)]
pub struct StaticCounts {
    /// Candidate pairs (the broadphase's stream).
    pub pairs: u64,
    /// Candidate pairs with exactly one static endpoint.
    pub pairs_static: u64,
    /// Candidate pairs with two static endpoints.
    pub pairs_static_static: u64,
    /// Manifolds.
    pub manifolds: u64,
    /// Manifolds with exactly one static endpoint.
    pub manifolds_static: u64,
    /// Manifolds with two static endpoints.
    pub manifolds_static_static: u64,
    /// Contact points in manifolds with a static endpoint.
    pub points_static: u64,
}

impl StaticCounts {
    fn read(world: &EcsMaster) -> Self {
        let rows = world.resource::<SolverScratch>().bodies();
        let fixed = |r: u32| rows.get(r as usize).is_none_or(|b| b.inv_mass == 0.0);
        let mut c = Self::default();
        for &(a, b) in world.resource::<ContactPairs>().pairs().iter() {
            c.pairs += 1;
            match (fixed(a.0), fixed(b.0)) {
                (true, true) => c.pairs_static_static += 1,
                (true, false) | (false, true) => c.pairs_static += 1,
                (false, false) => {}
            }
        }
        for m in world.resource::<Manifolds>().manifolds().iter() {
            c.manifolds += 1;
            match (fixed(m.body_a.0), fixed(m.body_b.0)) {
                (true, true) => {
                    c.manifolds_static_static += 1;
                    c.points_static += u64::from(m.count);
                }
                (true, false) | (false, true) => {
                    c.manifolds_static += 1;
                    c.points_static += u64::from(m.count);
                }
                (false, false) => {}
            }
        }
        c
    }

    fn add(&mut self, o: &Self) {
        self.pairs += o.pairs;
        self.pairs_static += o.pairs_static;
        self.pairs_static_static += o.pairs_static_static;
        self.manifolds += o.manifolds;
        self.manifolds_static += o.manifolds_static;
        self.manifolds_static_static += o.manifolds_static_static;
        self.points_static += o.points_static;
    }

    fn json(&self) -> String {
        format!(
            "{{\"pairs\":{},\"pairs_static\":{},\"pairs_static_static\":{},\"manifolds\":{},\
             \"manifolds_static\":{},\"manifolds_static_static\":{},\"points_static\":{}}}",
            self.pairs,
            self.pairs_static,
            self.pairs_static_static,
            self.manifolds,
            self.manifolds_static,
            self.manifolds_static_static,
            self.points_static
        )
    }
}

/// One run's dynamic program and its untimed readouts.
pub struct DynRun {
    program: Program,
    /// `--dyn` was given: the program's events are applied (otherwise the run is J-T, read only).
    events_on: bool,
    kicks: Vec<KickEvent>,
    launches: Vec<Launch>,
    next_kick: usize,
    next_launch: usize,
    /// Kick events and launches applied.
    events_applied: usize,
    gravity: [f32; 3],
    /// Σ injections of the events applied so far.
    e_inj: f64,
    kicks_applied: usize,
    launches_applied: usize,
    readback_mismatches: usize,
    clearance_min: f64,
    /// `--sanity`: one row per state, row s = the state after s steps.
    sanity: Option<Vec<SanityRow>>,
    /// `--sanity`: the static-pair receipts over the metric window, and their per-step maxima.
    statics_window: StaticCounts,
    statics_max: StaticCounts,
    /// `--scene-dump`: the read-back lines, in dump order after the header.
    dump: Option<String>,
    /// The first J-T box's inverse mass and inertia diagonal: the ratios' denominators.
    box0: (f32, [f32; 3]),
    can_sleep: bool,
    /// A static that is not static (non-zero inverse mass, or `Simulated`): voids the run.
    bad_statics: usize,
}

impl DynRun {
    /// Reads the built world (before step 0): the statics' and boxes' read-back lines and the
    /// sanity row of the spawn state.
    pub fn new(rig: &mut Rig, program: Program, events_on: bool, steps: usize, sanity: bool, dump: bool) -> Self {
        let world = &rig.world;
        let mass0 = world
            .get_component::<RigidBodyMass>(rig.boxes[0])
            .expect("invariant: the first J-T box is live");
        let box0 = (mass0.inv_mass, diag(&mass0.inv_inertia));
        let cfg = world.resource::<PhysicsConfig>();
        let can_sleep = cfg.sleeping;
        let g = v3(cfg.gravity);
        let mut bad_statics = 0;
        let dump = dump.then(|| {
            let mut s = String::with_capacity(400 * 1024);
            for &e in &rig.statics {
                let b = world.get_component::<RigidBody>(e).expect("invariant: a static is live");
                let c = world.get_component::<Collider>(e).expect("invariant: a static has a collider");
                let m = world.get_component::<RigidBodyMass>(e).expect("invariant: a static has a mass");
                let line = StaticLine {
                    half: half_extents(c),
                    p: v3(b.position),
                    q: [b.rotation.x, b.rotation.y, b.rotation.z, b.rotation.w],
                    friction: m.friction,
                    restitution: m.restitution,
                    radius: 0.0,
                };
                line.write(&mut s).expect("invariant: writing to a String cannot fail");
            }
            s
        });
        for &e in &rig.statics {
            let m = world.get_component::<RigidBodyMass>(e).expect("invariant: a static has a mass");
            bad_statics += usize::from(m.inv_mass != 0.0 || world.is_enabled::<Simulated>(e));
        }
        let mut run = Self {
            program,
            events_on,
            kicks: if events_on && program == Program::Kick { dyn_spec::kick_program().to_vec() } else { Vec::new() },
            launches: if events_on && program == Program::Shoot {
                dyn_spec::shoot_program().to_vec()
            } else {
                Vec::new()
            },
            next_kick: 0,
            next_launch: 0,
            events_applied: 0,
            gravity: g,
            e_inj: 0.0,
            kicks_applied: 0,
            launches_applied: 0,
            readback_mismatches: 0,
            clearance_min: f64::INFINITY,
            sanity: sanity.then(|| Vec::with_capacity(steps + 1)),
            statics_window: StaticCounts::default(),
            statics_max: StaticCounts::default(),
            dump,
            box0,
            can_sleep,
            bad_statics,
        };
        if let Some(d) = &mut run.dump {
            for (n, &e) in rig.boxes.iter().enumerate() {
                let line = run_body_line(world, e, n as u32, box0, can_sleep);
                line.write_body(d).expect("invariant: writing to a String cannot fail");
            }
        }
        rig.boxes.reserve(run.launches.len());
        run.push_row(rig, 0);
        run
    }

    fn push_row(&mut self, rig: &Rig, state: usize) {
        if let Some(rows) = &mut self.sanity {
            let mut row = SanityRow::new(state, self.gravity, self.e_inj);
            for &e in &rig.boxes {
                let b = rig.world.get_component::<RigidBody>(e).expect("invariant: a spawned body is live");
                row.add_body(
                    v3(b.position),
                    [b.rotation.x, b.rotation.y, b.rotation.z, b.rotation.w],
                    v3(b.linear_velocity),
                    v3(b.angular_velocity),
                );
            }
            rows.push(row);
        }
    }

    /// Applies the events of `step` (after the state after `step` steps was read, before the step
    /// of loop index `step` runs).
    pub fn before_step(&mut self, step: usize, rig: &mut Rig) {
        if !self.events_on {
            return;
        }
        while let Some(ev) = self.kicks.get(self.next_kick).filter(|e| e.step as usize == step) {
            let ev = *ev;
            for x in &ev.entries {
                let e = rig.boxes[x.index as usize];
                let dv = x.dv();
                let (e0, want) = {
                    let mut body =
                        rig.world.get_component_mut::<RigidBody>(e).expect("invariant: a kicked box is live");
                    let before = body.linear_velocity;
                    let e0 = dyn_spec::energy(v3(body.position), v3(before), v3(body.angular_velocity), self.gravity);
                    let want = Vec3::new(before.x + dv[0], before.y + dv[1], before.z + dv[2]);
                    body.linear_velocity = want;
                    (e0, want)
                };
                let b = rig.world.get_component::<RigidBody>(e).expect("invariant: a kicked box is live");
                let ok = v3(b.linear_velocity).map(f32::to_bits) == v3(want).map(f32::to_bits);
                self.e_inj +=
                    dyn_spec::energy(v3(b.position), v3(b.linear_velocity), v3(b.angular_velocity), self.gravity) - e0;
                self.readback_mismatches += usize::from(!ok);
                if let Some(d) = &mut self.dump {
                    let r = if ok {
                        dyn_spec::write_kick(ev.step, x.index, dv, d)
                    } else {
                        dyn_spec::write_kick_mismatch(ev.step, x.index, d)
                    };
                    r.expect("invariant: writing to a String cannot fail");
                }
                self.kicks_applied += 1;
            }
            self.next_kick += 1;
            self.events_applied += 1;
        }
        while let Some(l) = self.launches.get(self.next_launch).filter(|l| l.step as usize == step) {
            let l = *l;
            let p = l.position();
            let clearance = dyn_spec::clearance(
                p,
                rig.boxes.iter().map(|&e| {
                    v3(rig.world.get_component::<RigidBody>(e).expect("invariant: a spawned body is live").position)
                }),
            );
            self.clearance_min = self.clearance_min.min(clearance);
            if let Some(row) = self.sanity.as_mut().and_then(|rows| rows.last_mut()) {
                row.set_clearance(clearance);
            }
            let v = l.velocity();
            let e = spawn(&mut rig.world, Vec3::new(p[0], p[1], p[2]), Vec3::new(v[0], v[1], v[2]), None);
            rig.boxes.push(e);
            let index = (rig.boxes.len() - 1) as u32;
            debug_assert_eq!(index, l.index, "invariant: launches append in program order");
            let line = run_body_line(&rig.world, e, index, self.box0, self.can_sleep);
            self.e_inj += dyn_spec::energy(line.p, line.v, line.w, self.gravity);
            if let Some(d) = &mut self.dump {
                line.write_launch(l.step, d).expect("invariant: writing to a String cannot fail");
            }
            self.launches_applied += 1;
            self.next_launch += 1;
            self.events_applied += 1;
        }
    }

    /// Reads the state after `step + 1` steps (`--sanity` only) and the step's static-pair
    /// receipts.
    pub fn after_step(&mut self, step: usize, rig: &Rig) {
        if self.sanity.is_none() {
            return;
        }
        self.push_row(rig, step + 1);
        let c = StaticCounts::read(&rig.world);
        let (a, b) = self.program.metric_window();
        if (a..b).contains(&step) {
            self.statics_window.add(&c);
        }
        let m = &mut self.statics_max;
        m.pairs = m.pairs.max(c.pairs);
        m.pairs_static = m.pairs_static.max(c.pairs_static);
        m.pairs_static_static = m.pairs_static_static.max(c.pairs_static_static);
        m.manifolds = m.manifolds.max(c.manifolds);
        m.manifolds_static = m.manifolds_static.max(c.manifolds_static);
        m.manifolds_static_static = m.manifolds_static_static.max(c.manifolds_static_static);
        m.points_static = m.points_static.max(c.points_static);
    }

    /// Whether the run must be void: a static that is not static, or a kick whose read-back
    /// velocity is not `v + Δv`.
    pub fn void_reason(&self) -> Option<String> {
        if self.bad_statics > 0 {
            return Some(format!("dyn: {} static bodies are not static", self.bad_statics));
        }
        (self.readback_mismatches > 0)
            .then(|| format!("dyn: {} kicks read back a velocity other than v + dv", self.readback_mismatches))
    }

    /// Writes `--sanity` and `--scene-dump` and returns the summary's `"dyn"` object. `steps` is the
    /// run's length; `write` reports an unwritable file.
    pub fn finish(
        &mut self,
        rig: &Rig,
        sanity_path: Option<&std::path::Path>,
        dump_path: Option<&std::path::Path>,
    ) -> Result<String, String> {
        let program = self.program;
        let mut canonical = String::with_capacity(400 * 1024);
        dyn_spec::canonical_dump(program, &mut canonical).expect("invariant: writing to a String cannot fail");
        let canonical_fnv = fnv1a64(canonical.as_bytes());
        let mut dump_fnv = None;
        if let (Some(body), Some(path)) = (&self.dump, dump_path) {
            let cfg = rig.world.resource::<PhysicsConfig>();
            let mut text = String::with_capacity(body.len() + 256);
            dyn_spec::write_header(program, cfg.dt, v3(cfg.gravity), &mut text)
                .expect("invariant: writing to a String cannot fail");
            text.push_str(body);
            dyn_spec::write_end(self.events_applied, rig.boxes.len(), &mut text)
                .expect("invariant: writing to a String cannot fail");
            dump_fnv = Some(fnv1a64(text.as_bytes()));
            std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
        }
        // The final state's receipts (bars B1-B3 on the pose; the scorer is the verdict).
        let mut last = SanityRow::new(0, self.gravity, 0.0);
        for &e in &rig.boxes {
            let b = rig.world.get_component::<RigidBody>(e).expect("invariant: a spawned body is live");
            last.add_body(
                v3(b.position),
                [b.rotation.x, b.rotation.y, b.rotation.z, b.rotation.w],
                v3(b.linear_velocity),
                v3(b.angular_velocity),
            );
        }
        let final_json = format!(
            "{{\"nonfinite\":{},\"min_y\":{},\"max_speed\":{},\"escaped\":{}}}",
            last.nonfinite(),
            json_f64(last.min_y()),
            json_f64(last.max_speed()),
            last.escaped(program)
        );
        let sanity_json = match &self.sanity {
            None => "null".to_owned(),
            Some(rows) => {
                if let Some(path) = sanity_path {
                    let mut text = String::with_capacity(rows.len() * 320 + 256);
                    text.push_str(dyn_spec::SANITY_HEADER);
                    text.push('\n');
                    for r in rows {
                        r.write_csv(&mut text).expect("invariant: writing to a String cannot fail");
                    }
                    std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
                }
                let (a, b) = program.metric_window();
                format!(
                    "{{\"rows\":{},\"e0\":{},\"e_inj\":{},\"nonfinite_rows\":{},\"escaped_rows\":{},\
                     \"max_speed\":{},\"min_y\":{},\"clearance_min\":{},\"statics_window\":[{a},{b}],\
                     \"statics\":{},\"statics_step_max\":{}}}",
                    rows.len(),
                    json_f64(rows.first().map_or(f64::NAN, SanityRow::e_total)),
                    json_f64(self.e_inj),
                    rows.iter().filter(|r| r.nonfinite() > 0).count(),
                    rows.iter().filter(|r| r.escaped(program)).count(),
                    json_f64(rows.iter().map(SanityRow::max_speed).fold(0.0, f64::max)),
                    json_f64(rows.iter().map(SanityRow::min_y).fold(f64::INFINITY, f64::min)),
                    json_f64(self.clearance_min),
                    self.statics_window.json(),
                    self.statics_max.json(),
                )
            }
        };
        let (a, b) = program.metric_window();
        Ok(format!(
            ",\"dyn\":{{\"program\":\"{}\",\"events_on\":{},\"program_steps\":{},\"metric_window\":[{a},{b}],\
             \"statics\":{},\"canonical_dump_fnv\":\"{canonical_fnv:#018x}\",\"scene_dump_fnv\":{},\
             \"kicks\":{},\"launches\":{},\"readback_mismatches\":{},\"bad_statics\":{},\"bodies\":{},\
             \"final\":{final_json},\"sanity\":{sanity_json},\"reuse_probe\":\"{}\"}}",
            program.name(),
            self.events_on,
            program.steps(),
            rig.statics.len(),
            dump_fnv.map_or_else(|| "null".to_owned(), |h| format!("\"{h:#018x}\"")),
            self.kicks_applied,
            self.launches_applied,
            self.readback_mismatches,
            self.bad_statics,
            rig.boxes.len(),
            if self.events_on { "whole-run" } else { "J-T" },
        ))
    }
}

/// A dynamic body's dump line, read back from its components.
fn run_body_line(world: &EcsMaster, e: Entity, index: u32, box0: (f32, [f32; 3]), can_sleep: bool) -> BodyLine {
    let b = world.get_component::<RigidBody>(e).expect("invariant: a spawned body is live");
    let c = world.get_component::<Collider>(e).expect("invariant: a spawned body has a collider");
    let m = world.get_component::<RigidBodyMass>(e).expect("invariant: a spawned body has a mass");
    let i = diag(&m.inv_inertia);
    BodyLine {
        index,
        p: v3(b.position),
        q: [b.rotation.x, b.rotation.y, b.rotation.z, b.rotation.w],
        v: v3(b.linear_velocity),
        w: v3(b.angular_velocity),
        half: half_extents(c),
        friction: m.friction,
        restitution: m.restitution,
        // boyko's boxes have no convex radius and its bodies no damping and no CCD.
        radius: 0.0,
        inv_mass_ratio: m.inv_mass / box0.0,
        inv_inertia_ratio: [i[0] / box0.1[0], i[1] / box0.1[1], i[2] / box0.1[2]],
        linear_damping: 0.0,
        angular_damping: 0.0,
        can_sleep,
        ccd: false,
    }
}
