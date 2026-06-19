//! Physics O11 SP3 SELF-COLLISION acceptance-gate suite — the per-body
//! open-addressed spatial-hash (counting-sort CSR) same-body particle-vs-particle
//! self-collision pass.
//!
//! These encode the proof obligations the code review named for SP3:
//!
//! 1. EXACTLY-2r — an isolated overlapping pair of equal inverse mass separates to
//!    exactly `2·radius` in ONE Gauss-Seidel iteration (the standard one-iteration
//!    rigid PBD result); a pinned-endpoint variant moves only the free particle.
//! 2. BIT-STABLE DETERMINISM — the identical self-collision world run twice (fresh
//!    build each time) yields BYTE-identical position columns (`to_bits()`); each of
//!    `self_collision_iters ∈ {1,2,3}` is independently reproducible.
//! 3. HASH-COLLISION DETERMINISM (the load-bearing C1/C2 gate) — an adversarial
//!    multi-cluster world where distinct neighbour cells DO hash-collide into shared
//!    buckets produces a result BYTE-identical to a brute-force O(n²) reference that
//!    applies the same push-to-2r constraint in the same pinned (`i` asc, 27-cell
//!    `dz→dy→dx`, `j>i` asc within a queried coord) order. Equality proves the
//!    coordinate filter de-duplicated (no double-apply) and dropped no contact (no
//!    hash-collision miss). The test PRINTS the collision count so the gate is
//!    non-vacuous.
//! 4. 0%-GATE — `self_collision_iters == 0` is byte-identical to a run that never
//!    calls the pass (toggle off == SP2-equivalent), on a soft world exercising both
//!    distance + volume constraints.
//! 5. EDGE CASES — `n==0`, `n==1`, `radius==0` (release-safe no-op), all-coincident
//!    particles (the `LEN_EPS` guard ⇒ no NaN/inf push), and a DEEP overlap that
//!    converges over iters without exploding.
//!
//! # How the pass is driven
//!
//! The pass lives inside `step_body`, reachable via the public `physics_soft_step`
//! `Query<&mut SoftBody>` system. Identically to the SP1/SP2 suites, every gate runs
//! ONLY `physics_soft_step` via [`EcsMaster::run_system_once`] on a hoisted
//! `FunctionSystem` — NO `Schedule::run` work-stealing deque (so the Miri gate
//! witnesses the soft kernel directly, free of the pre-existing crossbeam-deque
//! retag noise `Schedule::run → ThreadPool::install` surfaces for ANY system). To
//! isolate the SELF-COLLISION arithmetic from gravity / SDF / coupling, the gates use
//! `dt = 0`, an empty `SdfField`, and `gravity = 0`: predict is then the identity
//! (`prev = pos`, `pos += v·0`), the distance/volume sweeps idle at rest, and the
//! ONLY position change is the self-collision push — so the asserted geometry is the
//! pass output alone.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::physics_soft_step;
use boyko_physics::soft::{SoftBody, SoftBodyError};

// ── Test-only harness (mirrors soft_body_sp1.rs) ─────────────────────────────────

/// Drives [`physics_soft_step`] ONCE per call via `run_system_once` on a hoisted
/// `FunctionSystem` — no threadpool / `Schedule::run` deque (Miri-clean + fast).
fn soft_driver() -> impl FnMut(&mut EcsMaster) {
    let mut sys = IntoSystem::into_system(physics_soft_step);
    move |world: &mut EcsMaster| {
        world.run_system_once(&mut sys);
    }
}

/// Runs the soft step `n` times on `world`.
fn step_soft_n(world: &mut EcsMaster, n: usize) {
    let mut step = soft_driver();
    for _ in 0..n {
        step(world);
    }
}

/// Spawns one [`SoftBody`] into a fresh `{SoftBody}` archetype.
fn spawn_soft(world: &mut EcsMaster, body: SoftBody) {
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world
        .spawn_one(arch, body)
        .expect("invariant: {SoftBody} archetype accepts a SoftBody");
}

/// Inserts a [`PhysicsConfig`] with `soft_body = true`, the given
/// `self_collision_iters`, gravity = 0, and a SMALL FINITE `dt` (so the velocity
/// update `(pos - prev) · inv_h` is finite — `dt == 0` would make `inv_h = 1/0 = inf`
/// and a zero-displacement particle's velocity `0 · inf = NaN`, which a SECOND step
/// would then fold back into the position via `pos += vel · h`; that is a
/// harness artifact of `dt == 0`, NOT a self-collision behaviour). With gravity = 0
/// and `substeps = 1`, a single step's ONLY position change is the self-collision
/// push, and a particle the pass does not touch keeps `vel == 0` so repeated steps
/// leave it byte-stable. Plus an empty [`SdfField`].
fn install_self_collision_config(world: &mut EcsMaster, self_collision_iters: usize) {
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 1,
        gravity: Vec3::ZERO,
        soft_body: true,
        self_collision_iters,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
}

/// Reads back the single soft body as an owned clone (for snapshotting).
fn read_soft(world: &mut EcsMaster) -> SoftBody {
    let q = world.query::<&SoftBody, ()>();
    let mut it = q.iter();
    it.next().expect("one soft body spawned").clone()
}

/// `true` if every particle position component is finite.
fn all_pos_finite(body: &SoftBody) -> bool {
    (0..body.particle_count()).all(|i| {
        body.pos_x[i].is_finite() && body.pos_y[i].is_finite() && body.pos_z[i].is_finite()
    })
}

/// Builds a constraint-free soft body (just particles + radius) — the self-collision
/// fixture. No edges ⇒ the distance sweep is a no-op; no tets ⇒ the volume sweep is a
/// no-op. With `dt == 0` the ONLY position change is the self-collision push.
fn free_particles(positions: &[[f32; 3]], inv_masses: &[f32], radius: f32) -> SoftBody {
    SoftBody::from_mesh(positions, inv_masses, &[], None, 0.0, radius)
        .expect("a constraint-free particle cloud is well-formed")
}

/// The bit-pattern snapshot of a body's position columns (the determinism oracle —
/// `f32::to_bits` so `-0.0`/`NaN`/every ULP is compared exactly, never `==`).
fn pos_bits(body: &SoftBody) -> Vec<(u32, u32, u32)> {
    (0..body.particle_count())
        .map(|i| {
            (
                body.pos_x[i].to_bits(),
                body.pos_y[i].to_bits(),
                body.pos_z[i].to_bits(),
            )
        })
        .collect()
}

/// The Euclidean distance between particles `i` and `j` of a body.
fn particle_dist(body: &SoftBody, i: usize, j: usize) -> f32 {
    let d = Vec3::new(
        body.pos_x[i] - body.pos_x[j],
        body.pos_y[i] - body.pos_y[j],
        body.pos_z[i] - body.pos_z[j],
    );
    d.length()
}

// ── Brute-force O(n²) self-collision reference (Gate 3) ───────────────────────────

/// `LEN_EPS` — re-declared to match the kernel's `crate::soft::solver::LEN_EPS`.
const LEN_EPS: f32 = 1e-6;

/// Floors a world coordinate to its integer cell index — BYTE-IDENTICAL op sequence
/// to the kernel's `cell_coord` (`(x · inv_cell).floor() as i32`), so the reference's
/// coordinate filter agrees with the kernel's to the bit.
fn ref_cell_coord(x: f32, inv_cell: f32) -> i32 {
    (x * inv_cell).floor() as i32
}

/// One push-to-`2·radius` projection of pair `(i, j)` — BYTE-IDENTICAL arithmetic to
/// the kernel's `project_self_pair` (exact sqrt + explicit `d·(1/len)`, inverse-mass
/// split, both-pinned + `len >= cell` + `len < LEN_EPS` guards in the same order).
fn ref_project_pair(body: &mut SoftBody, i: usize, j: usize, cell: f32) {
    let wi = body.inv_mass[i];
    let wj = body.inv_mass[j];
    let wsum = wi + wj;
    if wsum == 0.0 {
        return;
    }
    let dx = body.pos_x[i] - body.pos_x[j];
    let dy = body.pos_y[i] - body.pos_y[j];
    let dz = body.pos_z[i] - body.pos_z[j];
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len >= cell {
        return;
    }
    if len < LEN_EPS {
        return;
    }
    let inv_len = 1.0 / len;
    let nx = dx * inv_len;
    let ny = dy * inv_len;
    let nz = dz * inv_len;
    let cc = len - cell;
    let s = -cc / wsum;
    let si = s * wi;
    let sj = -s * wj;
    body.pos_x[i] += nx * si;
    body.pos_y[i] += ny * si;
    body.pos_z[i] += nz * si;
    body.pos_x[j] += nx * sj;
    body.pos_y[j] += ny * sj;
    body.pos_z[j] += nz * sj;
}

/// A hash-INDEPENDENT brute-force reference for the SP3 pass: replicates the kernel's
/// VISIT ORDER exactly — for each `i` ascending, query the 27 neighbour cell
/// coordinates in `dz→dy→dx` nesting order, and within each queried coordinate apply
/// every `j` (ascending) with `j > i` and `cell_coord(j) == (cx,cy,cz)`. This is the
/// same unordered-pair-once, same-accumulation-order projection the spatial hash
/// performs, but with NO hashing — so a byte-match between this and the kernel proves
/// the kernel's hash + coordinate filter is a pure function of the geometry.
///
/// Returns the number of pairs actually projected (overlap accepted), so the
/// hash-collision gate can assert the scene is non-vacuous.
fn brute_force_self_collision(body: &mut SoftBody, iters: usize, radius: f32) -> usize {
    let n = body.particle_count();
    if iters == 0 || radius <= 0.0 || n < 2 {
        return 0;
    }
    let cell = 2.0 * radius;
    let inv_cell = 1.0 / cell;
    let mut projected = 0usize;
    for _ in 0..iters {
        for i in 0..n {
            let ix = ref_cell_coord(body.pos_x[i], inv_cell);
            let iy = ref_cell_coord(body.pos_y[i], inv_cell);
            let iz = ref_cell_coord(body.pos_z[i], inv_cell);
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let cx = ix + dx;
                        let cy = iy + dy;
                        let cz = iz + dz;
                        for j in (i + 1)..n {
                            let jx = ref_cell_coord(body.pos_x[j], inv_cell);
                            let jy = ref_cell_coord(body.pos_y[j], inv_cell);
                            let jz = ref_cell_coord(body.pos_z[j], inv_cell);
                            if jx == cx && jy == cy && jz == cz {
                                // Count only genuinely-overlapping pairs (the
                                // constraint is one-sided), matching the kernel's
                                // `len < cell` accept.
                                if particle_dist(body, i, j) < cell {
                                    projected += 1;
                                }
                                ref_project_pair(body, i, j, cell);
                            }
                        }
                    }
                }
            }
        }
    }
    projected
}

/// Counts how many distinct OCCUPIED cell coordinates hash-COLLIDE into a shared
/// bucket under the kernel's Teschner hash (the same `next_pow2(2n)` table the body
/// uses). A non-zero count makes the hash-collision gate non-vacuous: it proves the
/// scene actually drives ≥ 2 distinct cells through one bucket, exercising the
/// coordinate filter's de-dup / no-drop guarantee.
fn count_hash_collisions(body: &SoftBody, radius: f32) -> usize {
    const HASH_P1: i32 = 73_856_093;
    const HASH_P2: i32 = 19_349_663;
    const HASH_P3: i32 = 83_492_791;
    let n = body.particle_count();
    let table = body.self_table_size();
    let cell = 2.0 * radius;
    let inv_cell = 1.0 / cell;
    // Distinct occupied cell coordinates → their bucket.
    let mut cells: Vec<(i32, i32, i32)> = Vec::new();
    for i in 0..n {
        let c = (
            ref_cell_coord(body.pos_x[i], inv_cell),
            ref_cell_coord(body.pos_y[i], inv_cell),
            ref_cell_coord(body.pos_z[i], inv_cell),
        );
        if !cells.contains(&c) {
            cells.push(c);
        }
    }
    let bucket = |c: (i32, i32, i32)| -> usize {
        let h = (c.0.wrapping_mul(HASH_P1)) ^ (c.1.wrapping_mul(HASH_P2)) ^ (c.2.wrapping_mul(HASH_P3));
        (h as u32 as usize) & (table - 1)
    };
    // Count cells whose bucket is shared by another distinct cell.
    let mut collisions = 0usize;
    for a in 0..cells.len() {
        for b in (a + 1)..cells.len() {
            if bucket(cells[a]) == bucket(cells[b]) {
                collisions += 1;
            }
        }
    }
    collisions
}

// ══════════════════════════════════════════════════════════════════════════════
// Gate 1 — EXACTLY-2r
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn isolated_pair_separates_to_exactly_2r_one_iter() {
    // Two equal-inv_mass particles overlapping along +X by `gap < 2r`. ONE
    // self-collision iteration must push them to EXACTLY 2*radius apart (the
    // one-iteration rigid PBD result for an isolated symmetric pair): the constraint
    // value C = len - 2r is removed in full, split 50/50 by equal inverse mass.
    let radius = 0.5_f32;
    let cell = 2.0 * radius; // = 1.0
    let gap = 0.6_f32; // overlapping (gap < cell)
    let positions = [[0.0_f32, 0.0, 0.0], [gap, 0.0, 0.0]];
    let inv_masses = [1.0_f32, 1.0];
    let body = free_particles(&positions, &inv_masses, radius);

    let mut world = EcsMaster::new();
    spawn_soft(&mut world, body);
    install_self_collision_config(&mut world, 1);
    step_soft_n(&mut world, 1);

    let body = read_soft(&mut world);
    let d = particle_dist(&body, 0, 1);
    let residual = (d - cell).abs();
    println!(
        "[exactly-2r] post-iter separation = {d:.9}, target = {cell}, residual = {residual:.3e}"
    );
    // Tight ULP-scale bound: the exact-sqrt + single divide leaves a few-ULP residual
    // at this magnitude, well under 1e-6.
    assert!(
        residual < 1e-6,
        "isolated equal-mass pair must separate to EXACTLY 2*radius in one iter; \
         residual {residual:.3e} (separation {d}, target {cell})"
    );
    // Symmetric split: each moved by (cell - gap)/2 from its start, so they remain
    // centered on the original midpoint.
    let mid = (body.pos_x[0] + body.pos_x[1]) * 0.5;
    assert!(
        (mid - gap * 0.5).abs() < 1e-6,
        "equal-mass split must keep the pair centered on the original midpoint; \
         got midpoint {mid}, expected {}",
        gap * 0.5
    );
}

#[test]
fn pinned_endpoint_takes_full_correction_one_iter() {
    // One particle pinned (inv_mass == 0), the other free. ONE iteration must move
    // the FREE particle the full correction to land exactly 2*radius from the pinned
    // one, and the pinned particle must NOT move at all.
    let radius = 0.5_f32;
    let cell = 2.0 * radius;
    let gap = 0.7_f32;
    let positions = [[0.0_f32, 0.0, 0.0], [gap, 0.0, 0.0]];
    let inv_masses = [0.0_f32, 1.0]; // particle 0 pinned
    let body = free_particles(&positions, &inv_masses, radius);

    let pinned_start = positions[0];

    let mut world = EcsMaster::new();
    spawn_soft(&mut world, body);
    install_self_collision_config(&mut world, 1);
    step_soft_n(&mut world, 1);

    let body = read_soft(&mut world);
    assert_eq!(
        (body.pos_x[0].to_bits(), body.pos_y[0].to_bits(), body.pos_z[0].to_bits()),
        (pinned_start[0].to_bits(), pinned_start[1].to_bits(), pinned_start[2].to_bits()),
        "pinned endpoint (inv_mass == 0) must not move at all"
    );
    let d = particle_dist(&body, 0, 1);
    let residual = (d - cell).abs();
    println!("[pinned-2r] post-iter separation = {d:.9}, residual = {residual:.3e}");
    assert!(
        residual < 1e-6,
        "free particle must take the FULL correction to 2*radius; residual {residual:.3e}"
    );
    // The free particle moved the whole correction along +X.
    assert!(
        (body.pos_x[1] - cell).abs() < 1e-6,
        "free particle should land at x = 2*radius from the pinned origin; got {}",
        body.pos_x[1]
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gate 2 — BIT-STABLE DETERMINISM
// ══════════════════════════════════════════════════════════════════════════════

/// Builds + runs a small self-collision world FROM SCRATCH for `iters` and returns
/// the resulting position bit-pattern — the input to the run-twice byte-identity gate.
fn run_self_collision_scene(iters: usize) -> Vec<(u32, u32, u32)> {
    // A 3x3 grid of overlapping particles in the z=0 plane (spacing < 2r so neighbours
    // overlap) — enough constraints that a NON-deterministic accumulation order would
    // show.
    let radius = 0.5_f32;
    let spacing = 0.7_f32;
    let mut positions = Vec::new();
    for gy in 0..3 {
        for gx in 0..3 {
            positions.push([gx as f32 * spacing, gy as f32 * spacing, 0.0]);
        }
    }
    let inv_masses = vec![1.0_f32; positions.len()];
    let body = free_particles(&positions, &inv_masses, radius);

    let mut world = EcsMaster::new();
    spawn_soft(&mut world, body);
    install_self_collision_config(&mut world, iters);
    step_soft_n(&mut world, 1);
    pos_bits(&read_soft(&mut world))
}

#[test]
fn run_twice_byte_identical() {
    let a = run_self_collision_scene(1);
    let b = run_self_collision_scene(1);
    assert_eq!(
        a, b,
        "the self-collision pass must be a pure function of input: two fresh builds \
         of the identical world must produce byte-identical position columns"
    );
}

#[test]
fn each_iter_count_independently_reproducible() {
    for iters in [1usize, 2, 3] {
        let a = run_self_collision_scene(iters);
        let b = run_self_collision_scene(iters);
        assert_eq!(
            a, b,
            "self_collision_iters == {iters} must be byte-reproducible run-to-run"
        );
    }
    // Sanity: different iter counts generally produce different results (the GS sweeps
    // converge further) — anti-vacuity that `iters` is actually wired through.
    let one = run_self_collision_scene(1);
    let three = run_self_collision_scene(3);
    assert_ne!(
        one, three,
        "anti-vacuity: 1 vs 3 GS sweeps should differ on an overlapping cluster \
         (else self_collision_iters is not actually driving the loop)"
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gate 3 — HASH-COLLISION DETERMINISM (the load-bearing C1/C2 gate)
// ══════════════════════════════════════════════════════════════════════════════

/// A deterministic splitmix64 PRNG (seeded, reproducible) — for the adversarial
/// scatter. No external rand dependency on the hot test path.
struct SplitMix64(u64);
impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A cell-coordinate index in `[-span, span)`.
    fn cell_in(&mut self, span: i64) -> i64 {
        (self.next_u64() % (2 * span as u64)) as i64 - span
    }
}

#[test]
fn spatial_hash_matches_brute_force_under_collisions() {
    // An ADVERSARIAL world: ~60 OVERLAPPING PAIRS scattered (via a seeded PRNG) over
    // a wide cell-coordinate volume, so the body occupies ~60+ DISTINCT cells in a
    // next_pow2(2n)-bucket table — far below the birthday bound, so distinct
    // neighbour cells DO hash-collide into shared buckets. Each pair is placed
    // OVERLAPPING (< 2r apart) so the pass actually projects. The kernel result must
    // be BYTE-IDENTICAL to the hash-independent brute-force reference: equality proves
    // (a) the coordinate filter de-duplicated every double-scan a shared bucket
    // causes (no double-apply), and (b) no contact was dropped because two cells
    // collided into one bucket (no miss). The non-vacuity asserts below PROVE the
    // scene drives real collisions + real projections.
    let radius = 0.5_f32;
    let cell = 2.0 * radius; // = 1.0 ⇒ a cell coord c spans world [c, c+1)
    let overlap_off = 0.4_f32; // < cell ⇒ the second particle of each pair overlaps

    // 64 pairs = 128 particles. Each pair's first particle sits at the CENTER of a
    // randomly chosen cell (well inside it), the second is `overlap_off` away (same
    // cell). Distinct cells ≈ 64 in a next_pow2(256) = 256-bucket table.
    let mut rng = SplitMix64(0xDEAD_BEEF_C0FF_EE00);
    let span: i64 = 5_000; // a +-5000-cell volume ⇒ huge coord range, hashes spread
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut used: Vec<(i64, i64, i64)> = Vec::new();
    while positions.len() < 128 {
        let cxi = rng.cell_in(span);
        let cyi = rng.cell_in(span);
        let czi = rng.cell_in(span);
        // Keep cells distinct so each pair is isolated (the brute-force reference and
        // the kernel agree pair-by-pair; collisions are between DISTINCT cells).
        if used.contains(&(cxi, cyi, czi)) {
            continue;
        }
        used.push((cxi, cyi, czi));
        // Particle at the cell center (+0.5 of a cell) — safely inside [c, c+1)·cell.
        let bx = (cxi as f32 + 0.5) * cell;
        let by = (cyi as f32 + 0.5) * cell;
        let bz = (czi as f32 + 0.5) * cell;
        positions.push([bx, by, bz]);
        positions.push([bx + overlap_off, by, bz]);
    }
    let n = positions.len();
    let inv_masses = vec![1.0_f32; n];

    // Reference body (clone of identical input) run through the brute force.
    let mut ref_body = free_particles(&positions, &inv_masses, radius);
    let iters = 3usize;

    // Non-vacuity instrumentation: count distinct-cell hash collisions + projected
    // pairs BEFORE running (collisions are a property of the initial bucketing).
    let collisions = count_hash_collisions(&ref_body, radius);
    let projected = brute_force_self_collision(&mut ref_body, iters, radius);
    println!(
        "[hash-collision gate] n = {n} particles, distinct-cell hash collisions = {collisions}, \
         projected overlapping pairs (brute force, all iters) = {projected}"
    );
    assert!(
        collisions > 0,
        "GATE NON-VACUITY: the engineered scene must force >= 1 distinct-cell hash \
         collision (else it does not exercise the C1/C2 coordinate-filter fix); got 0"
    );
    assert!(
        projected > 0,
        "GATE NON-VACUITY: the scene must project >= 1 overlapping pair (else no \
         self-collision work happens); got 0"
    );

    // Kernel body (identical input) run through the real pass.
    let kernel_body = free_particles(&positions, &inv_masses, radius);
    let mut world = EcsMaster::new();
    spawn_soft(&mut world, kernel_body);
    install_self_collision_config(&mut world, iters);
    step_soft_n(&mut world, 1);
    let kernel_body = read_soft(&mut world);

    assert_eq!(
        pos_bits(&kernel_body),
        pos_bits(&ref_body),
        "the spatial-hash self-collision result must be BYTE-identical to the \
         hash-independent brute-force reference (proves: no double-apply from shared \
         buckets, no dropped contact from hash collisions)"
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gate 4 — 0%-GATE (self_collision_iters == 0 is byte-identical to no-pass)
// ══════════════════════════════════════════════════════════════════════════════

/// Builds a tet-meshed, edge-constrained soft body (exercises BOTH the distance and
/// volume sweeps) and runs it `steps` steps under gravity onto nothing, returning the
/// position bits. `self_collision_iters` selects whether the SP3 pass is active.
fn run_constrained_body(self_collision_iters: usize, steps: usize) -> Vec<(u32, u32, u32)> {
    // A single tet: 4 distinct, non-coplanar particles + its 6 edges + the one tet.
    let positions = [
        [0.0_f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ];
    let inv_masses = [1.0_f32, 1.0, 1.0, 1.0];
    let edges = [(0u32, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];
    let tets = [(0u32, 1, 2, 3)];
    let body = SoftBody::from_tet_mesh(
        &positions,
        &inv_masses,
        &edges,
        &tets,
        None,
        None,
        0.0,  // edge compliance (rigid)
        0.0,  // tet compliance (rigid)
        // radius LARGER than the smallest rest length (1.0) would violate the
        // cell<=L0 precondition (debug-warn) AND make the 1-unit-apart particles
        // self-collide — which is the POINT for the toggle-on arm, but it would also
        // change the constrained result. Keep radius small so the distance/volume
        // sweeps dominate and the only difference between the arms is whether the
        // (here inactive, particles are 1.0 apart > 2r) SP3 pass is entered.
        0.1,
    )
    .expect("a single non-degenerate tet is well-formed");

    let mut world = EcsMaster::new();
    spawn_soft(&mut world, body);
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 120.0,
        // substeps == 1 so ONE step performs exactly ONE CSR rebuild. The iters==0
        // arm never enters the pass at all, so its multi-step run is unaffected
        // regardless of substeps.
        substeps: 1,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        self_collision_iters,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
    step_soft_n(&mut world, steps);
    pos_bits(&read_soft(&mut world))
}

#[test]
fn iters_zero_byte_identical_to_no_pass() {
    // With particles 1.0 apart (radius 0.1 ⇒ cell 0.2), no pair ever overlaps, so the
    // SP3 pass — whether entered (iters>0) or skipped (iters==0) — must produce the
    // EXACT same constrained result. This is the SP3 0%-gate: the pass is a true
    // no-op when nothing self-collides, and the iters==0 early-return is byte-clean.
    //
    // Run at `steps == 1` for a focused single-rebuild equivalence; the repeated-
    // rebuild path (many substeps/steps) is covered by the dedicated
    // `multi_substep_rebuild_does_not_corrupt_csr` regression test.
    let off = run_constrained_body(0, 1);
    let on_but_inactive = run_constrained_body(1, 1);
    assert_eq!(
        off, on_but_inactive,
        "self_collision_iters == 0 (pass skipped) must be BYTE-identical to \
         self_collision_iters == 1 on a world where nothing overlaps (the pass enters \
         but projects nothing) — the SP3 0%-gate"
    );
    // Anti-vacuity: the constrained body actually moved (gravity + constraints did
    // work) AND a long iters==0 run (the pass NEVER entered) is reproducible — the
    // off path is byte-stable over a non-trivial trajectory.
    let off_long_a = run_constrained_body(0, 20);
    let off_long_b = run_constrained_body(0, 20);
    assert_eq!(off_long_a, off_long_b, "the iters==0 run must be reproducible over 20 steps");
    assert_ne!(
        off, off_long_a,
        "anti-vacuity: the constrained body must actually move between step 1 and step 20"
    );
}

#[test]
fn multi_substep_rebuild_does_not_corrupt_csr() {
    // REGRESSION for the trailing-CSR-slot defect (build_hash once zeroed only
    // sc_cell_start[..table], leaving slot[table] stale across rebuilds, which made
    // the prefix-sum total debug_assert panic on the 2nd+ rebuild). An overlapping
    // cluster stepped with iters>0 over MULTIPLE substeps/steps rebuilds the CSR
    // scratch repeatedly; each rebuild must re-establish `acc == n` from a clean
    // slate. The production build_hash now clears the full slot[..=table] every
    // rebuild, so this run must NOT panic and must stay finite.
    let radius = 0.5_f32;
    let spacing = 0.6_f32; // < 2r ⇒ overlapping neighbours
    let mut positions = Vec::new();
    for gy in 0..3 {
        for gx in 0..3 {
            positions.push([gx as f32 * spacing, gy as f32 * spacing, 0.0]);
        }
    }
    let inv_masses = vec![1.0_f32; positions.len()];
    let body = free_particles(&positions, &inv_masses, radius);
    let mut world = EcsMaster::new();
    spawn_soft(&mut world, body);
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 120.0,
        substeps: 4, // 4 rebuilds per step ⇒ rebuilds #2..4 hit the stale slot
        gravity: Vec3::new(0.0, -1.0, 0.0),
        soft_body: true,
        self_collision_iters: 2,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
    step_soft_n(&mut world, 5); // 5 steps × 4 substeps = 20 rebuilds
    let body = read_soft(&mut world);
    assert!(
        all_pos_finite(&body),
        "a multi-substep self-collision run must stay finite (no CSR corruption)"
    );
}

#[test]
fn iters_zero_runs_zero_constraint_world_unchanged() {
    // A pure positional sanity 0%-gate: with iters == 0, dt == 0 and gravity == 0, an
    // overlapping cluster must be returned BYTE-identical to its input (the pass is a
    // pre-hash early-return; predict is the identity).
    let radius = 0.5_f32;
    let positions = [
        [0.0_f32, 0.0, 0.0],
        [0.3, 0.0, 0.0],
        [0.0, 0.3, 0.0],
    ];
    let inv_masses = vec![1.0_f32; 3];
    let input = pos_bits(&free_particles(&positions, &inv_masses, radius));

    let mut world = EcsMaster::new();
    spawn_soft(&mut world, free_particles(&positions, &inv_masses, radius));
    install_self_collision_config(&mut world, 0); // OFF
    step_soft_n(&mut world, 1);
    let out = pos_bits(&read_soft(&mut world));

    assert_eq!(
        input, out,
        "with self_collision_iters == 0 (+ dt == 0, gravity == 0) overlapping \
         particles must be returned byte-identical (the pre-hash early-return no-op)"
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gate 5 — EDGE CASES
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn empty_body_self_collision_noop() {
    // n == 0: the pass must not panic (the table size is 0, the < 2 guard returns).
    let body = free_particles(&[], &[], 0.5);
    assert_eq!(body.particle_count(), 0, "empty body has 0 particles");
    let mut world = EcsMaster::new();
    spawn_soft(&mut world, body);
    install_self_collision_config(&mut world, 3);
    step_soft_n(&mut world, 3); // must not panic
    let body = read_soft(&mut world);
    assert_eq!(body.particle_count(), 0, "empty body stays empty");
}

#[test]
fn single_particle_self_collision_noop() {
    // n == 1: the < 2 guard returns; the lone particle must not move (no pair).
    let positions = [[1.0_f32, 2.0, 3.0]];
    let inv_masses = [1.0_f32];
    let input = pos_bits(&free_particles(&positions, &inv_masses, 0.5));
    let mut world = EcsMaster::new();
    spawn_soft(&mut world, free_particles(&positions, &inv_masses, 0.5));
    install_self_collision_config(&mut world, 3);
    step_soft_n(&mut world, 1);
    let out = pos_bits(&read_soft(&mut world));
    assert_eq!(input, out, "a single particle has no pair and must not move");
}

#[test]
fn zero_radius_is_release_safe_noop() {
    // radius == 0: cell = 0 would divide by zero / form a degenerate grid. The pass
    // MUST early-return (radius <= 0) — no panic, no div-by-zero, particles unchanged.
    // (In debug a radius_warn fires to stderr; behaviour is identical in release.)
    let positions = [[0.0_f32, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]];
    let inv_masses = vec![1.0_f32; 3];
    let input = pos_bits(&free_particles(&positions, &inv_masses, 0.0));
    let mut world = EcsMaster::new();
    spawn_soft(&mut world, free_particles(&positions, &inv_masses, 0.0));
    install_self_collision_config(&mut world, 3);
    step_soft_n(&mut world, 2); // must not panic / divide by zero
    let out = pos_bits(&read_soft(&mut world));
    assert_eq!(
        input, out,
        "radius == 0 must be a release-safe no-op (early-return before any hashing)"
    );
}

#[test]
fn coincident_particles_no_nan_push() {
    // All particles at the SAME point: every pair has len ~0 < LEN_EPS ⇒ the guard
    // skips the push (undefined direction). The result must be finite (no NaN/inf
    // from a 1/0 normalize) and the particles must stay coincident (no push applied).
    let radius = 0.5_f32;
    let positions = [[2.0_f32, 2.0, 2.0]; 5];
    let inv_masses = vec![1.0_f32; 5];
    let mut world = EcsMaster::new();
    spawn_soft(&mut world, free_particles(&positions, &inv_masses, radius));
    install_self_collision_config(&mut world, 4);
    step_soft_n(&mut world, 1);
    let body = read_soft(&mut world);
    assert!(
        all_pos_finite(&body),
        "coincident particles must NOT produce NaN/inf (the LEN_EPS guard suppresses \
         the undefined-direction push)"
    );
    for i in 0..body.particle_count() {
        assert_eq!(
            (body.pos_x[i].to_bits(), body.pos_y[i].to_bits(), body.pos_z[i].to_bits()),
            (2.0_f32.to_bits(), 2.0_f32.to_bits(), 2.0_f32.to_bits()),
            "a coincident particle (len < LEN_EPS) must not be pushed"
        );
    }
}

#[test]
fn deep_overlap_converges_without_explosion() {
    // A DEEP overlap (two particles nearly coincident but past LEN_EPS, target
    // separation 2r) driven by MANY Gauss-Seidel sweeps in a single step: the pair
    // must converge toward 2r and never blow up to NaN/inf. The one-sided rigid
    // constraint can only push apart; it must not overshoot into an oscillating
    // explosion. (Driven by `self_collision_iters`, not multiple steps, so the
    // post-push velocity is not re-integrated into the geometry being asserted — the
    // `iters` count is the documented deep-overlap limiter.)
    let radius = 0.5_f32;
    let cell = 2.0 * radius;
    // Start deeply overlapped (1e-3 apart, well above LEN_EPS = 1e-6).
    let positions = [[0.0_f32, 0.0, 0.0], [1.0e-3, 0.0, 0.0]];
    let inv_masses = [1.0_f32, 1.0];
    let mut world = EcsMaster::new();
    spawn_soft(&mut world, free_particles(&positions, &inv_masses, radius));
    // 16 GS sweeps in ONE step.
    install_self_collision_config(&mut world, 16);
    step_soft_n(&mut world, 1);

    let body = read_soft(&mut world);
    assert!(all_pos_finite(&body), "deep overlap must never produce NaN/inf");
    let sep = particle_dist(&body, 0, 1);
    println!("[deep-overlap] separation after 16 GS sweeps = {sep:.9}, target = {cell}");
    // An isolated pair reaches 2r in a SINGLE rigid iteration regardless of depth (the
    // full constraint value is removed each sweep); extra sweeps then idle (len >= cell
    // ⇒ the one-sided constraint is inactive). So it lands at ~2r, stable, no explosion.
    assert!(
        (sep - cell).abs() < 1e-4,
        "deep overlap must converge to ~2*radius without exploding; got {sep}"
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Construction sanity — the fixtures are well-formed (a non-gate guard)
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn fixture_constructors_reject_bad_radius() {
    // A negative radius is rejected at construction (NonFinite arm covers radius < 0).
    let err = SoftBody::from_mesh(&[[0.0, 0.0, 0.0]], &[1.0], &[], None, 0.0, -1.0)
        .expect_err("a negative radius must be rejected at construction");
    assert_eq!(
        err,
        SoftBodyError::NonFinite,
        "radius < 0 must construct-fail with NonFinite (not reach the solver)"
    );
}
