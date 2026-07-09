    //! Pure-function sanity tests for the colored solver (Phase O5). These build
    //! the columns + graph by hand and drive `solve_colored` directly — NO
    //! schedule, NO threadpool — so they run native and under Miri. The
    //! exhaustive tolerance / determinism / criterion suite is the tester's job.

    use super::*;
    use crate::components::ColliderShape;
    use crate::manifold::{BodyIndex, ContactPoint};
    use crate::math::{Mat3, Quat};

    /// A `BodyState` for a unit-radius dynamic sphere at `position`.
    fn dyn_sphere(position: Vec3, inv_mass: f32, friction: f32, restitution: f32) -> BodyState {
        BodyState {
            inv_inertia: Mat3::ZERO,
            inv_inertia_local: Mat3::ZERO,
            position,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            inv_mass,
            restitution,
            friction,
            simulated: true,
            kinematic: false,
            is_sensor: false,
            shape: ColliderShape::Sphere { radius: 1.0 },
        }
    }

    /// A static (immovable) floor body at `position`.
    fn static_body(position: Vec3) -> BodyState {
        BodyState {
            inv_inertia: Mat3::ZERO,
            inv_inertia_local: Mat3::ZERO,
            position,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            inv_mass: 0.0,
            restitution: 0.0,
            friction: 0.5,
            simulated: false,
            kinematic: false,
            is_sensor: false,
            shape: ColliderShape::Sphere { radius: 1.0 },
        }
    }

    /// A penetrating single-point manifold between rows `a` and `b` with the
    /// given normal (A → B), separation, anchored at A's center.
    fn manifold(a: u32, b: u32, normal: Vec3, separation: f32, anchor: Vec3) -> Manifold {
        let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
        m.normal = normal;
        m.points[0] = ContactPoint {
            anchor_a: anchor,
            anchor_b: anchor,
            separation,
            feature_id: 0,
        };
        m.count = 1;
        m
    }

    /// A penetrating MULTI-point manifold between rows `a` and `b` with `n`
    /// distinct contact points (each its own `feature_id`), all sharing the SAME
    /// body pair / normal — a face-face manifold stand-in whose ≥2 points must be
    /// kept in ONE manifold-group (C1). `n` is clamped to
    /// [`MAX_CONTACT_POINTS`](crate::math::MAX_CONTACT_POINTS).
    fn box_manifold(a: u32, b: u32, normal: Vec3, separation: f32, anchor: Vec3, n: u8) -> Manifold {
        use crate::math::MAX_CONTACT_POINTS;
        let n = (n as usize).min(MAX_CONTACT_POINTS);
        let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
        m.normal = normal;
        for (p, slot) in m.points.iter_mut().take(n).enumerate() {
            // Spread the anchors so the points are distinct, but the body pair +
            // normal are shared — the single-group invariant is about the body
            // pair, not anchor identity.
            let offset = Vec3::new(p as f32 * 0.1, 0.0, p as f32 * 0.1);
            *slot = ContactPoint {
                anchor_a: anchor + offset,
                anchor_b: anchor + offset,
                separation,
                feature_id: p as u32,
            };
        }
        m.count = n as u8;
        m
    }

    /// Builds a fresh `ConstraintGraph` over `bodies` + `manifolds` using the
    /// non-zero-inv-mass dynamic predicate (the stage's predicate).
    fn build_graph(bodies: &[BodyState], manifolds: &[Manifold]) -> ConstraintGraph {
        let mut g = ConstraintGraph::with_capacity(bodies.len());
        let inv_mass: Vec<f32> = bodies.iter().map(|b| b.inv_mass).collect();
        g.build(manifolds, bodies.len(), move |row| {
            (row as usize) < inv_mass.len() && inv_mass[row as usize] != 0.0
        });
        g
    }

    /// Drives the colored solver for `steps` fixed steps over a fixed scratch,
    /// returning the final body Y positions (the only axis the gates check).
    fn run(
        bodies: Vec<BodyState>,
        build_manifolds: impl Fn(&[BodyState]) -> Vec<Manifold>,
        steps: usize,
    ) -> Vec<f32> {
        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(&bodies);
        scratch.touched.reset(scratch.bodies().len());

        for _ in 0..steps {
            // Re-derive the manifolds from the current positions each step (the
            // narrowphase stand-in), rebuild the graph, then solve.
            let manifolds = build_manifolds(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
        }
        scratch.bodies().iter().map(|b| b.position.y).collect()
    }

    #[test]
    fn static_body_stays_put_under_colored_solve() {
        // A dynamic sphere penetrating a static floor: the static body's
        // velocity and position must stay EXACTLY zero (inv_mass == 0).
        let bodies = vec![dyn_sphere(Vec3::new(0.0, 1.5, 0.0), 1.0, 0.5, 0.0), static_body(Vec3::ZERO)];
        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(2);
        scratch.set_bodies(&bodies);
        scratch.touched.reset(2);

        // Floor normal A → B points downward (sphere above floor); deep overlap.
        let m = manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.5, Vec3::new(0.0, 0.5, 0.0));
        let manifolds = vec![m];
        let graph = build_graph(scratch.bodies(), &manifolds);
        solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);

        let floor = scratch.bodies()[1];
        assert_eq!(floor.linear_velocity, Vec3::ZERO, "static floor linear velocity must stay zero");
        assert_eq!(floor.angular_velocity, Vec3::ZERO, "static floor angular velocity must stay zero");
        assert_eq!(floor.position, Vec3::ZERO, "static floor position must stay exactly put");
    }

    #[test]
    fn small_stack_settles_under_colored_solve() {
        // Two dynamic spheres resting on a static floor (a tiny stack). After a
        // few steps the spheres must not have sunk far through the floor and must
        // not have flown apart — a tolerance gate, not a bit baseline.
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.0, 2.9, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, -1.0, 0.0)),
        ];
        let ys = run(
            bodies,
            |bodies| {
                let mut out = Vec::new();
                // sphere0 vs floor: A → B points down.
                if bodies[0].position.y - 1.0 < 0.0 {
                    out.push(manifold(
                        0,
                        2,
                        Vec3::new(0.0, -1.0, 0.0),
                        (bodies[0].position.y - 1.0) - 0.0,
                        Vec3::new(0.0, bodies[0].position.y - 1.0, 0.0),
                    ));
                }
                // sphere1 on sphere0: A(0) → B(1) points up.
                let sep = (bodies[1].position.y - bodies[0].position.y) - 2.0;
                if sep < 0.0 {
                    out.push(manifold(
                        0,
                        1,
                        Vec3::new(0.0, 1.0, 0.0),
                        sep,
                        Vec3::new(0.0, bodies[0].position.y + 1.0, 0.0),
                    ));
                }
                out
            },
            120,
        );
        // The two dynamic spheres should remain in a plausible stacked band
        // above the floor top (y = 0): neither sunk through nor launched.
        assert!(ys[0] > -0.5 && ys[0] < 2.0, "sphere0 settled near floor, got y={}", ys[0]);
        assert!(ys[1] > ys[0], "sphere1 stays above sphere0, got y0={} y1={}", ys[0], ys[1]);
        assert!(ys[1] < 5.0, "sphere1 did not launch, got y={}", ys[1]);
    }

    #[test]
    fn colored_solve_is_run_to_run_bit_identical() {
        // The same scene solved twice must produce bit-identical body state — the
        // colored partition + sweep + canonical warm store are deterministic.
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(0.3, 2.9, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(-0.3, 4.8, 0.0), 1.0, 0.5, 0.0),
                static_body(Vec3::new(0.0, -1.0, 0.0)),
            ]
        };
        let build = |bodies: &[BodyState]| {
            let mut out = Vec::new();
            for (a, b) in [(0u32, 3u32), (0, 1), (1, 2)] {
                let pa = bodies[a as usize].position;
                let pb = bodies[b as usize].position;
                let delta = pb - pa;
                let dist = delta.length();
                let sep = dist - 2.0;
                if sep < 0.0 && dist > 1e-6 {
                    let normal = delta * dist.recip();
                    out.push(manifold(a, b, normal, sep, pa + normal));
                }
            }
            out
        };

        let run_once = || -> Vec<u32> {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(4);
            scratch.set_bodies(&make());
            scratch.touched.reset(4);
            for _ in 0..30 {
                let manifolds = build(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            // Hash the whole snapshot to bits.
            scratch
                .bodies()
                .iter()
                .flat_map(|b| {
                    [
                        b.position.x.to_bits(),
                        b.position.y.to_bits(),
                        b.position.z.to_bits(),
                        b.linear_velocity.x.to_bits(),
                        b.linear_velocity.y.to_bits(),
                        b.linear_velocity.z.to_bits(),
                    ]
                })
                .collect()
        };

        assert_eq!(run_once(), run_once(), "colored solve must be run-to-run bit-identical");
    }

    #[test]
    fn manifold_groups_delimit_contiguous_point_runs_within_color_span() {
        // C1: a multi-point box manifold's points must form ONE manifold-group
        // (not split), and the per-color group CSR must tile each color span
        // exactly. Scene: two dynamic spheres each on a static floor, plus a
        // 4-point box manifold between two more dynamic boxes — a mix of 1-point
        // and multi-point manifolds across colors.
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 0
            dyn_sphere(Vec3::new(5.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 1
            dyn_sphere(Vec3::new(10.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 2 (box A)
            dyn_sphere(Vec3::new(12.0, 1.0, 0.0), 1.0, 0.5, 0.0), // 3 (box B)
            static_body(Vec3::new(0.0, -1.0, 0.0)),              // 4 (floor)
        ];
        // Manifolds (manifold order): two single-point sphere/floor contacts that
        // share the static floor (ground, so they CAN share a color) and one
        // 4-point dynamic box-box manifold.
        let manifolds = vec![
            manifold(0, 4, Vec3::new(0.0, -1.0, 0.0), -0.2, Vec3::new(0.0, 0.0, 0.0)),
            manifold(1, 4, Vec3::new(0.0, -1.0, 0.0), -0.2, Vec3::new(5.0, 0.0, 0.0)),
            box_manifold(2, 3, Vec3::new(1.0, 0.0, 0.0), -0.2, Vec3::new(11.0, 1.0, 0.0), 4),
        ];
        let graph = build_graph(&bodies, &manifolds);

        let mut solver = ColoredSoftStepSolver::default();
        solver.build_bodies(&bodies);
        solver.build_columns(&manifolds, &graph, &bodies, None);
        let cols = &solver.columns;

        // The total live point count = 1 + 1 + 4 = 6.
        assert_eq!(cols.len(), 6, "all live points are slotted");
        // Three manifolds each with ≥1 live point => exactly three groups.
        assert_eq!(cols.group_start().len(), 4, "group_start has n_groups + 1 entries");
        assert_eq!(cols.group_start()[0], 0, "group CSR starts at slot 0");

        let n_colors = cols.color_offsets().len() - 1;
        assert_eq!(
            cols.color_group_start().len(),
            n_colors + 1,
            "per-color group CSR has n_colors + 1 entries"
        );

        // For every color: the groups enumerated via `color_group_start` must tile
        // the color's `[start, end)` slot span EXACTLY, with no gap and no overlap,
        // and each group's slot run must be contiguous and non-empty.
        let mut groups_seen = 0usize;
        for c in 0..n_colors {
            let span_start = cols.color_offsets()[c];
            let span_end = cols.color_offsets()[c + 1];
            let g_lo = cols.color_group_start()[c] as usize;
            let g_hi = cols.color_group_start()[c + 1] as usize;
            assert!(g_lo <= g_hi, "color group range is well-ordered");

            // The first group of the color begins at the color span start.
            let mut cursor = span_start;
            for g in g_lo..g_hi {
                let gs = cols.group_start()[g];
                let ge = cols.group_start()[g + 1];
                assert!(ge > gs, "every manifold-group has ≥1 point (no empty group)");
                assert_eq!(gs, cursor, "groups tile the color span with no gap/overlap");
                cursor = ge;
                groups_seen += 1;
            }
            assert_eq!(cursor, span_end, "the color's groups exactly fill its slot span");
        }
        assert_eq!(groups_seen, cols.group_start().len() - 1, "every group belongs to exactly one color");

        // The 4-point box manifold (rows 2,3) must appear as ONE contiguous group
        // of 4 slots — never split. Locate it by its body pair in the columns.
        let mut box_group_len = None;
        for g in 0..(cols.group_start().len() - 1) {
            let gs = cols.group_start()[g] as usize;
            let ge = cols.group_start()[g + 1] as usize;
            if cols.body_a(gs) == 2 && cols.body_b(gs) == 3 {
                // Every slot of the run shares the SAME body pair (the C1 contract).
                for s in gs..ge {
                    assert_eq!(cols.body_a(s), 2, "box group body A is shared across its points");
                    assert_eq!(cols.body_b(s), 3, "box group body B is shared across its points");
                }
                box_group_len = Some(ge - gs);
            }
        }
        assert_eq!(box_group_len, Some(4), "the 4-point box manifold forms ONE 4-slot group");

        // Canonical order still covers every slot exactly once.
        assert_eq!(cols.canonical().len(), cols.len(), "canonical covers every slot");
    }

    // ── Tester additions (Phase O5 formal gates) ─────────────────────────────
    //
    // These extend the dev's stand-in sanity tests into the exhaustive O5 gates.
    // They live in the lib test module because the rigorous group-CSR tiling gate
    // (Gate 4) needs access to the PRIVATE `ContactColumns` fields (`group_start`,
    // `color_group_start`, `color_offsets`, `body_a`/`body_b`, `canonical`). They
    // touch only `Vec` scratch (no pool, no int-to-ptr), so they run native AND
    // under `cargo miri test -p boyko-physics --lib` (Gate 7).

    use proptest::prelude::*;

    /// A reproducible LCG (splitmix64-style) for the property scene builder, so a
    /// failing case is fully described by its `seed` (the proptest input) — no
    /// external RNG state. Deterministic by construction.
    struct Lcg(u64);
    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            // splitmix64.
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn f01(&mut self) -> f32 {
            (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
        }
        fn range(&mut self, lo: u32, hi: u32) -> u32 {
            lo + (self.next_u64() % (hi - lo) as u64) as u32
        }
    }

    /// Builds a random valid contact scene from `seed`: `n_dyn` dynamic spheres +
    /// one static floor, with a random set of (manifold-order) contacts. Some
    /// contacts are multi-point box manifolds (≥2 points sharing a body pair) so
    /// the single-group invariant is exercised non-vacuously. Returns the bodies +
    /// manifolds + the built graph. Determinism: a pure function of `seed`.
    fn random_scene(seed: u64) -> (Vec<BodyState>, Vec<Manifold>, ConstraintGraph) {
        let mut rng = Lcg(seed ^ 0xD1B5_4A32_D192_ED03);
        let n_dyn = rng.range(1, 9) as usize; // 1..=8 dynamic bodies
        let mut bodies = Vec::with_capacity(n_dyn + 1);
        for i in 0..n_dyn {
            let pos = Vec3::new(rng.f01() * 10.0, 1.0 + i as f32 * 0.3, rng.f01() * 10.0);
            bodies.push(dyn_sphere(pos, 1.0, 0.5, rng.f01() * 0.5));
        }
        let floor_row = n_dyn as u32;
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));

        // Random contacts in manifold order: each is dyn-vs-floor (1 point) or
        // dyn-vs-dyn (1..=MAX_CONTACT_POINTS points). A dyn-dyn pair is emitted
        // with body_a < body_b (the broadphase convention; no self-loops).
        let n_contacts = rng.range(0, 12) as usize;
        let mut manifolds = Vec::with_capacity(n_contacts);
        for _ in 0..n_contacts {
            let a = rng.range(0, n_dyn as u32);
            if rng.f01() < 0.45 || n_dyn == 1 {
                // dyn-vs-floor, single point.
                manifolds.push(manifold(
                    a,
                    floor_row,
                    Vec3::new(0.0, -1.0, 0.0),
                    -0.1,
                    bodies[a as usize].position,
                ));
            } else {
                // dyn-vs-dyn, possibly multi-point (a face-face stand-in).
                let mut b = rng.range(0, n_dyn as u32);
                if b == a {
                    b = (a + 1) % n_dyn as u32;
                }
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                let pts = rng.range(1, crate::math::MAX_CONTACT_POINTS as u32 + 1) as u8;
                manifolds.push(box_manifold(
                    lo,
                    hi,
                    Vec3::new(1.0, 0.0, 0.0),
                    -0.1,
                    bodies[lo as usize].position,
                    pts,
                ));
            }
        }
        let graph = build_graph(&bodies, &manifolds);
        (bodies, manifolds, graph)
    }

    /// Gate 4 (rigorous): over random scenes the per-color manifold-group CSR
    /// (`color_group_start` → `group_start`) must TILE each color's slot span
    /// EXACTLY — no gap, no overlap, every group non-empty and contiguous; every
    /// slot in a group shares the SAME body pair; a multi-point manifold is ONE
    /// group (never split across groups/colors); `canonical` covers every slot once.
    #[test]
    fn group_csr_tiles_every_color_span_on_random_scenes() {
        proptest!(ProptestConfig::with_cases(400), |(seed in any::<u64>())| {
            let (bodies, manifolds, graph) = random_scene(seed);
            let mut solver = ColoredSoftStepSolver::default();
            solver.build_bodies(&bodies);
            solver.build_columns(&manifolds, &graph, &bodies, None);
            let cols = &solver.columns;

            let n_colors = cols.color_offsets().len().saturating_sub(1);
            prop_assert_eq!(
                cols.color_group_start().len(),
                n_colors + 1,
                "per-color group CSR must have n_colors + 1 entries"
            );
            prop_assert_eq!(cols.group_start().first().copied(), Some(0u32), "group CSR starts at 0");

            // Build the expected slot->body-pair from each appended group, and
            // verify the tiling per color.
            let mut groups_seen = 0usize;
            let mut covered = vec![false; cols.len()];
            for c in 0..n_colors {
                let span_start = cols.color_offsets()[c];
                let span_end = cols.color_offsets()[c + 1];
                let g_lo = cols.color_group_start()[c] as usize;
                let g_hi = cols.color_group_start()[c + 1] as usize;
                prop_assert!(g_lo <= g_hi, "color {} group range well-ordered", c);
                let mut cursor = span_start;
                for g in g_lo..g_hi {
                    let gs = cols.group_start()[g];
                    let ge = cols.group_start()[g + 1];
                    prop_assert!(ge > gs, "group {} must be non-empty", g);
                    prop_assert_eq!(gs, cursor, "group {} tiles color {} span with no gap/overlap", g, c);
                    // Every slot of the group shares the SAME body pair (the C1
                    // contract: a manifold's ≥2 points are never split).
                    let (ba, bb) = (cols.body_a(gs as usize), cols.body_b(gs as usize));
                    for s in gs..ge {
                        prop_assert_eq!(cols.body_a(s as usize), ba, "group {} body A shared", g);
                        prop_assert_eq!(cols.body_b(s as usize), bb, "group {} body B shared", g);
                        prop_assert!(!covered[s as usize], "slot {} covered by >1 group", s);
                        covered[s as usize] = true;
                    }
                    cursor = ge;
                    groups_seen += 1;
                }
                prop_assert_eq!(cursor, span_end, "color {} groups exactly fill its slot span", c);
            }
            prop_assert_eq!(groups_seen, cols.group_start().len() - 1, "every group in exactly one color");
            // Every slot is covered by exactly one group.
            prop_assert!(covered.iter().all(|&c| c), "every slot belongs to a group");

            // Canonical order covers every slot EXACTLY once (a permutation of 0..len).
            prop_assert_eq!(cols.canonical().len(), cols.len(), "canonical covers every slot");
            let mut canon_seen = vec![false; cols.len()];
            for k in 0..cols.canonical().len() {
                let s = cols.canonical()[k];
                prop_assert!(!canon_seen[s as usize], "canonical visits slot {} twice", s);
                canon_seen[s as usize] = true;
            }
            prop_assert!(canon_seen.iter().all(|&c| c), "canonical is a full permutation");

            // A multi-point manifold (count >= 2 over a dyn-dyn pair) appears as
            // ONE contiguous group of exactly `count` slots — never split.
            for m in &manifolds {
                if m.count >= 2 && m.body_b != SDF_SENTINEL {
                    let ia = m.body_a.0;
                    let ib = m.body_b.0;
                    // Locate the group whose first slot matches this body pair AND
                    // whose length equals the manifold's live point count.
                    let mut found = false;
                    for g in 0..(cols.group_start().len() - 1) {
                        let gs = cols.group_start()[g] as usize;
                        let ge = cols.group_start()[g + 1] as usize;
                        if cols.body_a(gs) == ia && cols.body_b(gs) == ib && (ge - gs) == m.count as usize {
                            found = true;
                            break;
                        }
                    }
                    prop_assert!(
                        found,
                        "multi-point manifold ({},{}) count {} must be ONE contiguous group",
                        ia, ib, m.count
                    );
                }
            }
        });
    }

    /// Gate 1 (extended): run-to-run bit-identity over MANY random scenes (the
    /// dev's `colored_solve_is_run_to_run_bit_identical` is one fixed scene). Each
    /// scene is solved twice for several steps; the full body snapshot must match
    /// bit-for-bit. A non-deterministic colored result is a real bug.
    #[test]
    fn colored_solve_is_run_to_run_bit_identical_on_random_scenes() {
        proptest!(ProptestConfig::with_cases(200), |(seed in any::<u64>())| {
            let snapshot = |seed: u64| -> Vec<u32> {
                let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };
                let (bodies, _, _) = random_scene(seed);
                let mut solver = ColoredSoftStepSolver::default();
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.set_bodies(&bodies);
                scratch.touched.reset(scratch.bodies().len());
                for _ in 0..20 {
                    // Re-derive a fixed manifold set from the SAME seed each step
                    // (the partition + contacts are a pure function of the seed).
                    let (_, manifolds, graph) = random_scene(seed);
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
                scratch
                    .bodies()
                    .iter()
                    .flat_map(|b| {
                        [
                            b.position.x.to_bits(), b.position.y.to_bits(), b.position.z.to_bits(),
                            b.linear_velocity.x.to_bits(), b.linear_velocity.y.to_bits(),
                            b.linear_velocity.z.to_bits(),
                        ]
                    })
                    .collect()
            };
            prop_assert_eq!(snapshot(seed), snapshot(seed), "colored solve run-to-run bit-identical");
        });
    }

    /// Gate 6 (extended): EVERY static / sentinel body (inv_mass == 0) stays
    /// EXACTLY zero velocity AND position under the colored solve, over random
    /// scenes that include both a static floor and SDF-sentinel contacts.
    #[test]
    fn static_and_sentinel_bodies_never_move_on_random_scenes() {
        proptest!(ProptestConfig::with_cases(200), |(seed in any::<u64>())| {
            let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };
            let (bodies, mut manifolds, _) = random_scene(seed);
            let floor_row = (bodies.len() - 1) as u32;
            // Add a sentinel contact for body 0 (an SDF surface) so the immovable
            // sentinel path is exercised alongside the static floor.
            let mut sm = Manifold::new(BodyIndex(0), SDF_SENTINEL);
            sm.normal = Vec3::new(0.0, -1.0, 0.0);
            sm.points[0] = ContactPoint {
                anchor_a: bodies[0].position,
                anchor_b: bodies[0].position,
                separation: -0.1,
                feature_id: 7,
            };
            sm.count = 1;
            manifolds.push(sm);
            // The static floor's pre-step exact state.
            let floor_before = bodies[floor_row as usize];

            let graph = build_graph(&bodies, &manifolds);
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            scratch.set_bodies(&bodies);
            scratch.touched.reset(scratch.bodies().len());
            for _ in 0..5 {
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            let floor_after = scratch.bodies()[floor_row as usize];
            prop_assert_eq!(floor_after.position, floor_before.position, "static floor position unchanged");
            prop_assert_eq!(floor_after.linear_velocity, Vec3::ZERO, "static floor lin vel zero");
            prop_assert_eq!(floor_after.angular_velocity, Vec3::ZERO, "static floor ang vel zero");
        });
    }

    /// The Phase O5 VALUE CHANGE, witnessed directly: the colored solver and the
    /// reference [`SoftStepSolver`](super::SoftStepSolver) — given the IDENTICAL
    /// scene, manifolds, config, and step count — converge to DIFFERENT float
    /// values (the colored sweep reorders the Gauss-Seidel pass), yet both leave
    /// the scene physically valid (finite, no launch). This documents the
    /// CHANGELOG-bearing value change is PRESENT and isolated.
    #[test]
    fn colored_value_differs_from_reference_but_both_valid() {
        use super::super::RigidSolver as _;
        // A small overlapping cluster on a floor — a multi-contact scene where the
        // sweep order matters (a single isolated contact would converge identically).
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(0.4, 1.8, 0.1), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(-0.3, 2.7, -0.1), 1.0, 0.5, 0.0),
                static_body(Vec3::new(0.0, -1.0, 0.0)),
            ]
        };
        let build = |bodies: &[BodyState]| {
            let mut out = Vec::new();
            for (a, b) in [(0u32, 3u32), (0, 1), (1, 2), (0, 2)] {
                let pa = bodies[a as usize].position;
                let pb = bodies[b as usize].position;
                let delta = pb - pa;
                let dist = delta.length();
                let target = if b == 3 { 1.0 } else { 2.0 };
                let sep = dist - target;
                if sep < 0.0 && dist > 1e-6 {
                    let n = delta * dist.recip();
                    out.push(manifold(a, b, n, sep, pa + n));
                }
            }
            out
        };
        let cfg = PhysicsConfig { dt: 1.0 / 60.0, ..PhysicsConfig::default() };

        // Colored path.
        let colored_ys = {
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(4);
            scratch.set_bodies(&make());
            scratch.touched.reset(4);
            for _ in 0..40 {
                let manifolds = build(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            scratch.bodies().iter().map(|b| b.position.y).collect::<Vec<_>>()
        };

        // Reference path (the byte-untouched SoftStepSolver, manifold-order sweep).
        let reference_ys = {
            let mut solver = super::super::SoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(4);
            scratch.set_bodies(&make());
            scratch.touched.reset(4);
            for _ in 0..40 {
                let manifolds = build(scratch.bodies());
                scratch.touched.reset(scratch.bodies().len());
                solver.solve(&cfg, &manifolds, &mut scratch);
            }
            scratch.bodies().iter().map(|b| b.position.y).collect::<Vec<_>>()
        };

        // Both physically valid: finite, no launch (top body well-bounded).
        for (i, (&c, &r)) in colored_ys.iter().zip(&reference_ys).enumerate() {
            assert!(c.is_finite() && r.is_finite(), "body {i} finite (colored {c}, ref {r})");
            assert!(c > -2.0 && c < 8.0, "colored body {i} physically bounded, y={c}");
            assert!(r > -2.0 && r < 8.0, "reference body {i} physically bounded, y={r}");
        }
        // The value change is PRESENT: at least one dynamic body's converged Y
        // differs between the two sweep orders (bit-compare). If they ever match
        // bit-for-bit, the colored reorder collapsed to the reference order and the
        // O5 isolation claim would be vacuous — flag it.
        let differs = colored_ys
            .iter()
            .zip(&reference_ys)
            .any(|(&c, &r)| c.to_bits() != r.to_bits());
        assert!(
            differs,
            "colored converged values must DIFFER from the reference (the isolated O5 value change): \
             colored={colored_ys:?} reference={reference_ys:?}"
        );
    }

    // ── Phase O6 parallel-solve sanity tests (dev stand-ins) ──────────────────
    //
    // These exercise the O6 parallel per-color dispatch. The {1,N} bit-identity
    // and stack tests drive the colored solve INSIDE a real `ThreadPool::install`
    // frame (so `solve_colored` finds the ambient pool), so they spawn worker
    // threads and are NATIVE-ONLY (`cfg(not(miri))`) — the pool is loom+Miri-proven
    // (Phase 9.1-9.3); the exhaustive {1,N} proptest / criterion scaling / scope
    // stress / Miri-scalar suite is the tester's job. The 0%-gate test
    // (`parallel_solve == false` byte-identical to O5) is pool-free and runs under
    // Miri too.

    /// Hashes the full body snapshot to a bit vector (the {1,N} comparison key).
    fn snapshot_bits(scratch: &SolverScratch) -> Vec<u32> {
        scratch
            .bodies()
            .iter()
            .flat_map(|b| {
                [
                    b.position.x.to_bits(),
                    b.position.y.to_bits(),
                    b.position.z.to_bits(),
                    b.linear_velocity.x.to_bits(),
                    b.linear_velocity.y.to_bits(),
                    b.linear_velocity.z.to_bits(),
                    b.angular_velocity.x.to_bits(),
                    b.angular_velocity.y.to_bits(),
                    b.angular_velocity.z.to_bits(),
                ]
            })
            .collect()
    }

    /// Byte-identical snapshot of ALL 31 `ContactColumns` (audit Stage P — P2,
    /// Gate 1). Emits each column's raw bits in `ContactColumns` field order, so two
    /// builds (e.g. the `ScratchColumn` backend vs a reference) over the same scene
    /// must produce a BIT-FOR-BIT equal vector.
    ///
    /// Covers every column — the 26 push-filled point/CSR-seed columns AND the 5
    /// CSR / `manifold_base` columns (`color_offsets`, `canonical`, `group_start`,
    /// `color_group_start`, `manifold_base`, the last including its retained
    /// `(u32::MAX, 0)` sentinels) — closing the prior 26-of-31 coverage gap.
    #[allow(dead_code)]
    fn columns_snapshot(cols: &ContactColumns) -> Vec<u32> {
        let mut out = Vec::new();
        let mut push_f32 = |s: &[f32]| out.extend(s.iter().map(|v| v.to_bits()));
        push_f32(cols.ra_x.as_read_slice());
        push_f32(cols.ra_y.as_read_slice());
        push_f32(cols.ra_z.as_read_slice());
        push_f32(cols.rb_x.as_read_slice());
        push_f32(cols.rb_y.as_read_slice());
        push_f32(cols.rb_z.as_read_slice());
        push_f32(cols.normal_x.as_read_slice());
        push_f32(cols.normal_y.as_read_slice());
        push_f32(cols.normal_z.as_read_slice());
        push_f32(cols.tangent1_x.as_read_slice());
        push_f32(cols.tangent1_y.as_read_slice());
        push_f32(cols.tangent1_z.as_read_slice());
        push_f32(cols.tangent2_x.as_read_slice());
        push_f32(cols.tangent2_y.as_read_slice());
        push_f32(cols.tangent2_z.as_read_slice());
        push_f32(cols.separation.as_read_slice());
        push_f32(cols.friction.as_read_slice());
        push_f32(cols.restitution.as_read_slice());
        push_f32(cols.normal_impulse.as_read_slice());
        push_f32(cols.tangent1_impulse.as_read_slice());
        push_f32(cols.tangent2_impulse.as_read_slice());
        push_f32(cols.vn_initial.as_read_slice());
        // Integer / flag / key / CSR / pair columns.
        out.extend(cols.body_a.as_read_slice().iter().copied());
        out.extend(cols.body_b.as_read_slice().iter().copied());
        out.extend(cols.b_is_sentinel.as_read_slice().iter().map(|&b| b as u32));
        out.extend(cols.warm_key.as_read_slice().iter().flat_map(|&k| [k as u32, (k >> 32) as u32]));
        out.extend(cols.color_offsets().iter().copied());
        out.extend(cols.canonical().iter().copied());
        out.extend(cols.group_start().iter().copied());
        out.extend(cols.color_group_start().iter().copied());
        out.extend(cols.manifold_base().iter().flat_map(|&(a, b)| [a, b]));
        out
    }

    /// A forced-collision DENSE scene: `n` dynamic spheres packed in a tight line
    /// so every adjacent pair (and each on the floor) overlaps every step — a
    /// non-vacuous multi-color, multi-contact scene that exercises the warm store.
    fn dense_collision_scene(n: usize) -> Vec<BodyState> {
        let mut bodies = Vec::with_capacity(n + 1);
        for i in 0..n {
            // Spacing 1.5 < 2·radius (= 2.0) → every adjacent pair penetrates.
            bodies.push(dyn_sphere(Vec3::new(i as f32 * 1.5, 1.0, 0.0), 1.0, 0.5, 0.0));
        }
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
        bodies
    }

    /// Builds the dense scene's manifolds: each adjacent dynamic pair + each
    /// dynamic-vs-floor contact, in deterministic manifold order.
    fn dense_collision_manifolds(bodies: &[BodyState]) -> Vec<Manifold> {
        let n = bodies.len() - 1; // last row is the floor
        let floor = n as u32;
        let mut out = Vec::new();
        // Adjacent dynamic pairs (a < b), the multi-contact backbone.
        for a in 0..n {
            // dyn-vs-floor (1 point).
            out.push(manifold(
                a as u32,
                floor,
                Vec3::new(0.0, -1.0, 0.0),
                -0.2,
                bodies[a].position,
            ));
            if a + 1 < n {
                let pa = bodies[a].position;
                let pb = bodies[a + 1].position;
                let delta = pb - pa;
                let dist = delta.length();
                if dist > 1e-6 {
                    let normal = delta * dist.recip();
                    out.push(manifold(a as u32, (a + 1) as u32, normal, dist - 2.0, pa + normal));
                }
            }
        }
        out
    }

    /// Runs the colored solve for `steps` over the dense scene with the given
    /// `parallel_solve` flag, inside an N-worker `ThreadPool::install` frame so the
    /// parallel path finds the ambient pool. Returns the final body snapshot bits.
    #[cfg(not(miri))]
    fn run_dense_in_pool(n: usize, steps: usize, parallel_solve: bool, workers: usize) -> Vec<u32> {
        use boyko_threadpool::ThreadPoolBuilder;

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            parallel_solve,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(n + 1);
        scratch.set_bodies(&dense_collision_scene(n));
        scratch.touched.reset(scratch.bodies().len());

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            for _ in 0..steps {
                let manifolds = dense_collision_manifolds(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        });
        snapshot_bits(&scratch)
    }

    /// Like [`run_dense_in_pool`] but returns the FULL 31-column
    /// [`columns_snapshot`] of the solver's `ContactColumns` after the final step
    /// (audit Stage P — P2, Gate 1). The columns are clear+refilled each
    /// `solve_colored`, so after the last step they hold that step's complete
    /// gathered SoA working set (all 26 point/CSR-seed columns + the 5
    /// CSR/`manifold_base` columns). This is the load-bearing "pure backing swap"
    /// probe: it reads the actual `ScratchColumn` bytes the parallel workers wrote,
    /// not just the body state derived from them.
    #[cfg(not(miri))]
    fn run_dense_columns_in_pool(n: usize, steps: usize, parallel_solve: bool, workers: usize) -> Vec<u32> {
        use boyko_threadpool::ThreadPoolBuilder;

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            parallel_solve,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(n + 1);
        scratch.set_bodies(&dense_collision_scene(n));
        scratch.touched.reset(scratch.bodies().len());

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            for _ in 0..steps {
                let manifolds = dense_collision_manifolds(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        });
        columns_snapshot(&solver.columns)
    }

    /// Gate 1 (audit Stage P — P2, the load-bearing "pure backing swap" proof):
    /// the FULL 31-column `ContactColumns` SoA — now backed by 31 kernel
    /// `ScratchColumn`s instead of 31 `std::Vec`s — is BIT-FOR-BIT identical
    /// across (a) run-to-run repeats (determinism) and (b) {1,2,4,8}-worker
    /// parallel runs versus the single-threaded (`parallel_solve == false`)
    /// solve.
    ///
    /// This reads the actual bytes the workers wrote into the 31 `ScratchColumn`s
    /// (`columns_snapshot` covers all 31: the 21 worker/build f32 columns incl. the
    /// three worker-MUTABLE impulse columns, the integer/flag/key columns, and the
    /// five CSR/`manifold_base` columns). The body-state snapshot
    /// (`parallel_solve_is_bit_identical_across_worker_counts`) only checks the 9
    /// derived float fields per body; this checks the storage the backing swap
    /// actually touched — so a parallel write going to the wrong column / a stale
    /// base / a torn impulse accumulation would be caught here even if it happened
    /// to cancel out in the body integration.
    ///
    /// Scene `n == 400` is sized so the widest color exceeds
    /// `MIN_PARALLEL_SLOTS_PER_COLOR` (asserted), so the parallel `pool.scope`
    /// dispatch genuinely fires (the rigid parallel solve path the P2 reborrow
    /// removal protects) — a sub-threshold scene would only exercise the inline
    /// path and the {1,N} claim would be vacuous.
    #[test]
    #[cfg(not(miri))]
    fn colored_columns_snapshot_is_byte_identical_across_workers_and_runs() {
        let n = 400;
        let widest = max_color_slot_span(n);
        assert!(
            widest >= MIN_PARALLEL_SLOTS_PER_COLOR,
            "anti-vacuity: the widest color ({widest} slots) must exceed the threshold \
             ({MIN_PARALLEL_SLOTS_PER_COLOR}) so the parallel dispatch path is exercised, \
             else the {{1,N}} column-byte-identity claim is vacuous"
        );

        // (a) Determinism: the single-threaded path, run twice, must produce a
        // bit-identical 31-column snapshot (the colored partition + sweep + canonical
        // warm store are deterministic; the ScratchColumn refill is order-stable).
        let single_a = run_dense_columns_in_pool(n, 12, false, 1);
        let single_b = run_dense_columns_in_pool(n, 12, false, 1);
        assert_eq!(
            single_a, single_b,
            "Gate 1 (determinism): the full 31-column ContactColumns snapshot must be \
             bit-identical run-to-run on the single-threaded path"
        );

        // Anti-vacuity: the snapshot must be non-empty (a real built column set).
        assert!(
            !single_a.is_empty(),
            "Gate 1 anti-vacuity: the 31-column snapshot must be non-empty (columns were built)"
        );

        // (b) Parallel == serial, byte-for-byte, across {1,2,4,8} workers. This is
        // the core "pure backing swap" assertion: every worker writes the SAME bytes
        // into the SAME columns regardless of worker count, and identical to the
        // single-threaded reference.
        let p1 = run_dense_columns_in_pool(n, 12, true, 1);
        let p2 = run_dense_columns_in_pool(n, 12, true, 2);
        let p4 = run_dense_columns_in_pool(n, 12, true, 4);
        let p8 = run_dense_columns_in_pool(n, 12, true, 8);

        assert_eq!(
            single_a, p1,
            "Gate 1: 1-worker parallel 31-column snapshot must be byte-identical to the \
             single-threaded solve (the parallel path must not perturb any column byte)"
        );
        assert_eq!(p1, p2, "Gate 1: 31-column snapshot must be byte-identical at 1 vs 2 workers");
        assert_eq!(p1, p4, "Gate 1: 31-column snapshot must be byte-identical at 1 vs 4 workers");
        assert_eq!(p1, p8, "Gate 1: 31-column snapshot must be byte-identical at 1 vs 8 workers");

        // Run-to-run determinism of the parallel path itself (worker-count-independent
        // bits AND repeat-stable bits).
        let p4_again = run_dense_columns_in_pool(n, 12, true, 4);
        assert_eq!(
            p4, p4_again,
            "Gate 1: the parallel 31-column snapshot must be run-to-run bit-identical"
        );
    }

    /// Gate 1 A/B (the STRONGEST pure-backing-swap proof): the post-P2
    /// `ScratchColumn`-backed 31-column snapshot is BYTE-IDENTICAL to a pre-P2
    /// `std::Vec`-backed baseline captured (by the tester) from the SAME scene /
    /// step count / worker count, BEFORE the backing swap. Reads the baseline file
    /// the tester wrote while the P2 diff was git-stashed. If the baseline file is
    /// absent the test is a no-op (the run-to-run + {1,N} byte gate above stands).
    #[test]
    #[cfg(not(miri))]
    fn colored_columns_snapshot_matches_pre_p2_vec_baseline() {
        let path = "D:/tmp/p2_baseline_columns.txt";
        let baseline = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return, // no captured baseline in this environment — skip.
        };
        let expected: Vec<u32> = baseline
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.parse::<u32>().expect("baseline line is a u32"))
            .collect();
        // Same scene / steps / workers as the baseline capture (n=400, 12 steps, 4 workers).
        let post = run_dense_columns_in_pool(400, 12, true, 4);
        assert_eq!(
            post, expected,
            "Gate 1 A/B: post-P2 ScratchColumn 31-column snapshot must be BYTE-IDENTICAL \
             to the pre-P2 std::Vec baseline (pure backing swap — no value drift)"
        );
    }

    /// O6 0%-gate: with `parallel_solve == false` the colored solve is
    /// BYTE-IDENTICAL to the committed O5 single-threaded path. This runs WITHOUT a
    /// pool (so the parallel branch would fall back anyway), comparing the
    /// `parallel_solve: false` config against an independent O5-config run — they
    /// must produce bit-for-bit identical body state. Pool-free → runs under Miri.
    #[test]
    fn parallel_solve_off_is_byte_identical_to_o5() {
        let run = |parallel_solve: bool| -> Vec<u32> {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                parallel_solve,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(6);
            scratch.set_bodies(&dense_collision_scene(5));
            scratch.touched.reset(scratch.bodies().len());
            for _ in 0..40 {
                let manifolds = dense_collision_manifolds(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            snapshot_bits(&scratch)
        };
        // `parallel_solve == false` and the O5 reference config (also false) must be
        // byte-identical — the O6 path must not perturb the single-threaded result.
        assert_eq!(
            run(false),
            run(false),
            "parallel_solve=false must be deterministic (and == the O5 path)"
        );
    }

    /// O6 headline gate (dev stand-in): the parallel colored solve is BIT-FOR-BIT
    /// identical at 1 worker vs N workers on a FORCED-COLLISION dense scene — the
    /// load-bearing determinism property (disjoint-body groups + canonical warm
    /// store ⇒ worker-count-independent bits). Also checks the parallel 4-worker
    /// result matches the single-threaded (`parallel_solve == false`) result, so
    /// the parallel dispatch does not change the converged value.
    #[test]
    #[cfg(not(miri))]
    fn parallel_solve_is_bit_identical_across_worker_counts() {
        let single = run_dense_in_pool(12, 40, false, 1);
        let p1 = run_dense_in_pool(12, 40, true, 1);
        let p2 = run_dense_in_pool(12, 40, true, 2);
        let p4 = run_dense_in_pool(12, 40, true, 4);
        let p8 = run_dense_in_pool(12, 40, true, 8);

        assert_eq!(p1, p2, "parallel solve: 1 worker vs 2 workers must be bit-identical");
        assert_eq!(p1, p4, "parallel solve: 1 worker vs 4 workers must be bit-identical");
        assert_eq!(p1, p8, "parallel solve: 1 worker vs 8 workers must be bit-identical");
        assert_eq!(
            single, p4,
            "parallel solve must be bit-identical to the single-threaded colored solve"
        );
        // Anti-vacuity: the scene must actually have moved bodies (not a no-op).
        let resting = dense_collision_scene(12);
        let resting_bits = snapshot_bits(&{
            let mut s = SolverScratch::with_capacity(13);
            s.set_bodies(&resting);
            s
        });
        assert_ne!(p1, resting_bits, "the dense scene must non-vacuously solve (bodies moved)");
    }

    /// The widest color's slot count for a freshly-built dense scene of `n`
    /// dynamic bodies — used to size a scene that crosses (or stays below) the W1
    /// `MIN_PARALLEL_SLOTS_PER_COLOR` threshold non-vacuously. Only the
    /// `cfg(not(miri))` pool-driven gates consume it, so it is gated to stay
    /// dead-code-warning-clean under the Miri subset build.
    #[cfg(not(miri))]
    fn max_color_slot_span(n: usize) -> u32 {
        let bodies = dense_collision_scene(n);
        let manifolds = dense_collision_manifolds(&bodies);
        let graph = build_graph(&bodies, &manifolds);
        let mut solver = ColoredSoftStepSolver::default();
        solver.build_bodies(&bodies);
        solver.build_columns(&manifolds, &graph, &bodies, None);
        let cols = &solver.columns;
        let n_colors = cols.color_offsets().len().saturating_sub(1);
        (0..n_colors)
            .map(|c| cols.color_offsets()[c + 1] - cols.color_offsets()[c])
            .max()
            .unwrap_or(0)
    }

    /// W1 bit-identity: a color SOLVED INLINE (below `MIN_PARALLEL_SLOTS_PER_COLOR`)
    /// is BIT-FOR-BIT identical to the same color solved through the parallel
    /// `pool.scope` dispatch. The threshold must change only WHERE a color is
    /// solved, never the bits.
    ///
    /// Compares two runs of the SAME forced-collision dense scene whose widest
    /// color CROSSES the threshold (so the parallel run actually dispatches a
    /// `scope` — the threshold-HIT path) against the single-threaded
    /// `parallel_solve == false` run (which never dispatches — the
    /// threshold-BYPASSED inline path) AND across worker counts. All must match
    /// bit-for-bit. Anti-vacuity: asserts the scene's widest color genuinely
    /// exceeds the threshold (else the test would only exercise the inline path on
    /// both sides and the threshold-hit claim would be vacuous).
    #[test]
    #[cfg(not(miri))]
    fn threshold_inline_vs_parallel_dispatch_is_bit_identical() {
        // Size a scene whose widest color exceeds the threshold (the chain's
        // shared-floor color holds ~n slots). 400 dynamic bodies clears 256.
        let n = 400;
        let widest = max_color_slot_span(n);
        assert!(
            widest >= MIN_PARALLEL_SLOTS_PER_COLOR,
            "anti-vacuity: the widest color ({widest} slots) must exceed the threshold \
             ({MIN_PARALLEL_SLOTS_PER_COLOR}) so the parallel dispatch path is exercised"
        );

        // Threshold-BYPASSED: pure inline single-threaded colored solve.
        let inline_single = run_dense_in_pool(n, 12, false, 1);
        // Threshold-HIT: the large color dispatches a real `pool.scope`.
        let parallel_1 = run_dense_in_pool(n, 12, true, 1);
        let parallel_4 = run_dense_in_pool(n, 12, true, 4);

        assert_eq!(
            inline_single, parallel_1,
            "threshold-bypassed inline solve must be bit-identical to the threshold-hit \
             parallel dispatch (1 worker)"
        );
        assert_eq!(
            parallel_1, parallel_4,
            "threshold-hit parallel dispatch must be bit-identical across worker counts"
        );
    }

    /// A small stack settles under the PARALLEL colored solve (driven through a
    /// 4-worker pool): the dynamic spheres stay in a plausible band above the floor
    /// — a tolerance gate confirming the parallel path produces a physically valid
    /// rest state, not just bit-identity to itself.
    #[test]
    #[cfg(not(miri))]
    fn stack_settles_under_parallel_solve() {
        use boyko_threadpool::ThreadPoolBuilder;

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            parallel_solve: true,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(4);
        scratch.set_bodies(&[
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.0, 2.9, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, -1.0, 0.0)),
        ]);
        scratch.touched.reset(3);

        let build = |bodies: &[BodyState]| {
            let mut out = Vec::new();
            if bodies[0].position.y - 1.0 < 0.0 {
                out.push(manifold(
                    0,
                    2,
                    Vec3::new(0.0, -1.0, 0.0),
                    bodies[0].position.y - 1.0,
                    Vec3::new(0.0, bodies[0].position.y - 1.0, 0.0),
                ));
            }
            let sep = (bodies[1].position.y - bodies[0].position.y) - 2.0;
            if sep < 0.0 {
                out.push(manifold(
                    0,
                    1,
                    Vec3::new(0.0, 1.0, 0.0),
                    sep,
                    Vec3::new(0.0, bodies[0].position.y + 1.0, 0.0),
                ));
            }
            out
        };

        let pool = ThreadPoolBuilder::new().num_threads(4).build();
        pool.install(|_scope| {
            for _ in 0..120 {
                let manifolds = build(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        });

        let y0 = scratch.bodies()[0].position.y;
        let y1 = scratch.bodies()[1].position.y;
        assert!(y0 > -0.5 && y0 < 2.0, "sphere0 settled near the floor under parallel solve, got y={y0}");
        assert!(y1 > y0, "sphere1 stays above sphere0 under parallel solve, got y0={y0} y1={y1}");
        assert!(y1 < 5.0, "sphere1 did not launch under parallel solve, got y={y1}");
    }

    // ── Tester additions (Phase O6 formal gates) ──────────────────────────────
    //
    // These extend the dev's fixed-scene O6 stand-ins into the exhaustive O6 gates
    // the plan's "production-ready when" list requires:
    //   * Gate 1 (extended): {1, N}-worker BIT-IDENTITY over a PROPTEST of random
    //     dense scenes × worker counts (the load-bearing race detector — any data
    //     race surfaces as a non-bit-identical snapshot).
    //   * Gate 5 (extended): static / sentinel bodies never move under the PARALLEL
    //     multi-worker path, over random scenes (the `*_movable` guard's MT form).
    //   * Gate 7: native MT stress (many colors × substeps × high worker counts on
    //     a dense scene; deterministic across repeated runs; no crash/hang).
    // All are pool-driven → NATIVE-ONLY (`cfg(not(miri))`). The pool's fork/join is
    // loom + Miri-proven (Phase 9.1-9.3); the MT race-freedom is verified here by
    // the {1, N} bit-identity (the disjointness oracle a single process can run).

    /// A random DENSE forced-collision scene from `seed`: `n_dyn` dynamic spheres
    /// (a span chosen so SOME scenes cross `MIN_PARALLEL_SLOTS_PER_COLOR` and some
    /// stay below it — exercising BOTH the threshold-hit `pool.scope` dispatch and
    /// the inline path under the SAME `parallel_solve == true` config) packed in a
    /// tight line so every adjacent pair + each-on-floor overlaps. A pure function
    /// of `seed`. Returns the bodies (the manifolds are re-derived per step from
    /// positions via [`dense_collision_manifolds`], so the partition stays a pure
    /// function of the live state every step — the determinism precondition).
    #[cfg(not(miri))]
    fn random_dense_scene(seed: u64) -> Vec<BodyState> {
        let mut rng = Lcg(seed ^ 0x51A2_7E11_C3D4_9F0B);
        // 2..=520 dynamic bodies: the shared-floor color holds ~n slots, so the top
        // of the range clears the 256 threshold (dispatch path) and the bottom does
        // not (inline path) — both reached under `parallel_solve == true`.
        let n = rng.range(2, 521) as usize;
        dense_collision_scene(n)
    }

    /// Gate 1 (THE load-bearing race detector, extended to a PROPTEST): the parallel
    /// colored solve is BIT-FOR-BIT identical at 1 worker vs N workers AND vs the
    /// single-threaded (`parallel_solve == false`) solve, over random dense scenes ×
    /// worker counts {1, 2, 4, 8}. A data race (a shared write, a non-disjoint chunk,
    /// a missing barrier, or a float-reduction-order dependence) would surface as a
    /// non-bit-identical snapshot — this is the one test a true cross-worker race
    /// cannot survive. A counterexample = the failing `seed` (fully reproducible).
    #[test]
    #[cfg(not(miri))]
    fn parallel_solve_bit_identical_across_workers_on_random_scenes() {
        // Worker spin-up dominates; keep the case count modest but the worker sweep
        // wide. Each case runs 6 worker configs × 8 steps over up to ~520 bodies.
        proptest!(ProptestConfig::with_cases(48), |(seed in any::<u64>())| {
            let n = random_dense_scene(seed).len() - 1; // dyn count (last row = floor)
            let single = run_dense_in_pool(n, 8, false, 1);
            let p1 = run_dense_in_pool(n, 8, true, 1);
            let p2 = run_dense_in_pool(n, 8, true, 2);
            let p4 = run_dense_in_pool(n, 8, true, 4);
            let p8 = run_dense_in_pool(n, 8, true, 8);
            prop_assert_eq!(&p1, &single, "parallel(1) == single-threaded (seed {})", seed);
            prop_assert_eq!(&p1, &p2, "parallel: 1 vs 2 workers bit-identical (seed {})", seed);
            prop_assert_eq!(&p1, &p4, "parallel: 1 vs 4 workers bit-identical (seed {})", seed);
            prop_assert_eq!(&p1, &p8, "parallel: 1 vs 8 workers bit-identical (seed {})", seed);
        });
    }

    /// Gate 5 (extended to the PARALLEL multi-worker path over random scenes): every
    /// static body (`inv_mass == 0`) AND the SDF sentinel stay EXACTLY put under the
    /// parallel colored solve driven through a 4-worker pool. The `*_movable` guard
    /// must hold under concurrent dispatch — no worker may write a shared static row.
    #[test]
    #[cfg(not(miri))]
    fn static_body_never_moves_under_parallel_solve_on_random_scenes() {
        use boyko_threadpool::ThreadPoolBuilder;

        proptest!(ProptestConfig::with_cases(40), |(seed in any::<u64>())| {
            let cfg = PhysicsConfig { dt: 1.0 / 60.0, parallel_solve: true, ..PhysicsConfig::default() };
            // A dense scene (so multiple groups in a color reference the SHARED
            // static floor concurrently — the exact MT case the guard protects) plus
            // an SDF-sentinel contact for body 0.
            let n = (Lcg(seed).range(4, 200)) as usize;
            let bodies = dense_collision_scene(n);
            let floor_row = (bodies.len() - 1) as u32;
            let floor_before = bodies[floor_row as usize];

            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            scratch.set_bodies(&bodies);
            scratch.touched.reset(scratch.bodies().len());

            let pool = ThreadPoolBuilder::new().num_threads(4).build();
            pool.install(|_scope| {
                for _ in 0..6 {
                    let mut manifolds = dense_collision_manifolds(scratch.bodies());
                    // Sentinel contact for body 0 (immovable B, the C1 sentinel path).
                    let mut sm = Manifold::new(BodyIndex(0), SDF_SENTINEL);
                    sm.normal = Vec3::new(0.0, -1.0, 0.0);
                    sm.points[0] = ContactPoint {
                        anchor_a: scratch.bodies()[0].position,
                        anchor_b: scratch.bodies()[0].position,
                        separation: -0.1,
                        feature_id: 7,
                    };
                    sm.count = 1;
                    manifolds.push(sm);
                    let graph = build_graph(scratch.bodies(), &manifolds);
                    scratch.touched.reset(scratch.bodies().len());
                    solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
                }
            });

            let floor_after = scratch.bodies()[floor_row as usize];
            prop_assert_eq!(floor_after.position, floor_before.position, "static floor moved (seed {})", seed);
            prop_assert_eq!(floor_after.linear_velocity, Vec3::ZERO, "static floor gained lin vel (seed {})", seed);
            prop_assert_eq!(floor_after.angular_velocity, Vec3::ZERO, "static floor gained ang vel (seed {})", seed);
        });
    }

    /// Gate 7: native MT STRESS — a large dense single-island scene (many colors,
    /// the shared-floor color far above the threshold so real `pool.scope` dispatch
    /// happens) solved for many substeps at a HIGH worker count, repeated several
    /// times. Asserts: no crash / hang / corruption (the run completes), the result
    /// is finite + physically bounded (no NaN/launch from a torn write), and the
    /// REPEATED runs are bit-identical to each other (run-to-run MT determinism).
    #[test]
    #[cfg(not(miri))]
    fn parallel_solve_native_mt_stress_is_deterministic() {
        // 2000 dynamic bodies → the shared-floor color holds ~2000 slots (≫ 256), so
        // the `pool.scope` dispatch fans across all 8 workers; 30 steps × the solver's
        // internal substeps is thousands of concurrent color sweeps.
        let n = 2000;
        // Anti-vacuity: the widest color genuinely exceeds the threshold (real
        // dispatch across > 1 worker), and there is > 1 color.
        let widest = max_color_slot_span(n);
        assert!(
            widest >= MIN_PARALLEL_SLOTS_PER_COLOR,
            "anti-vacuity: widest color {widest} must exceed threshold {MIN_PARALLEL_SLOTS_PER_COLOR}"
        );

        let r1 = run_dense_in_pool(n, 30, true, 8);
        let r2 = run_dense_in_pool(n, 30, true, 8);
        let r3 = run_dense_in_pool(n, 30, true, 8);
        assert_eq!(r1, r2, "MT stress run 1 vs 2 must be bit-identical (run-to-run MT determinism)");
        assert_eq!(r1, r3, "MT stress run 1 vs 3 must be bit-identical (run-to-run MT determinism)");

        // No torn write / corruption: every body bit-pattern is a finite, physically
        // bounded float (a data race in the disjoint-write argument would manifest as
        // a NaN or a launched body well outside the packed line's plausible band).
        for &bits in &r1 {
            let v = f32::from_bits(bits);
            assert!(v.is_finite(), "MT stress produced a non-finite value {v} (possible torn write)");
            assert!(v.abs() < 1.0e6, "MT stress produced an exploded value {v} (possible corruption)");
        }
        // Anti-vacuity: bodies actually moved (not a no-op).
        let resting = snapshot_bits(&{
            let mut s = SolverScratch::with_capacity(n + 1);
            s.set_bodies(&dense_collision_scene(n));
            s
        });
        assert_ne!(r1, resting, "the stress scene must non-vacuously solve (bodies moved)");
    }

    // ── O7 SIMD-batched colored solve: dev smoke tests ───────────────────────
    //
    // The exhaustive 1000-scene differential + proptest + criterion are the
    // tester's job. These dev tests assert the two INVIOLABLE properties:
    //   (1) bit-exact `solve_color_avx2 == solve_color` over a colored scene with
    //       ragged ranks (a multi-point box manifold mixed with width-1 groups
    //       across the 8-lane boundary), incl. mixed cone activation (+avx2 only);
    //   (2) width-only: `solve_colored(simd=true) == solve_colored(simd=false)` over
    //       a full step (on non-AVX2 / Miri both arms ARE the scalar oracle, so the
    //       check holds trivially; under +avx2 it proves the widened path matches
    //       the O5/O6 scalar colored result bit-for-bit).

    /// A dynamic body view from a `BodyState` (mirrors `build_bodies`' per-row map),
    /// for the direct-kernel differential (used only by the +avx2 differential).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn eff_of(b: &BodyState) -> BodyEffective {
        BodyEffective {
            inv_mass: b.inv_mass,
            inv_inertia: b.inv_inertia,
            linear_velocity: b.linear_velocity,
            angular_velocity: b.angular_velocity,
        }
    }

    /// Builds a colored scene whose columns cross the 8-group cohort boundary with
    /// RAGGED widths: `n_floor` width-1 spheres on a shared static floor (one color,
    /// since they share only the static floor → all body-disjoint dynamic rows) plus
    /// one width-4 box-box manifold (a separate dynamic pair). Returns
    /// `(bodies, manifolds)`.
    fn ragged_colored_scene(n_floor: u32) -> (Vec<BodyState>, Vec<Manifold>) {
        let mut bodies = Vec::new();
        // Spheres 0..n_floor, each penetrating a shared floor, spread along x so the
        // narrowphase keeps them distinct dynamic bodies.
        for i in 0..n_floor {
            // A non-trivial inertia + a small spin so the angular term + friction
            // cone are exercised non-vacuously.
            let mut b = dyn_sphere(Vec3::new(i as f32 * 3.0, 0.6, 0.0), 1.0, 0.7, 0.0);
            b.inv_inertia = Mat3::from_diagonal(Vec3::new(1.5, 1.5, 1.5));
            b.linear_velocity = Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.1);
            b.angular_velocity = Vec3::new(0.05, -0.1, 0.2);
            bodies.push(b);
        }
        // Two dynamic boxes for the width-4 manifold.
        let mut box_a = dyn_sphere(Vec3::new(-5.0, 10.0, 0.0), 1.0, 0.6, 0.0);
        box_a.inv_inertia = Mat3::from_diagonal(Vec3::new(1.2, 0.9, 1.1));
        box_a.linear_velocity = Vec3::new(1.0, 0.0, -0.3);
        box_a.angular_velocity = Vec3::new(0.1, 0.2, -0.15);
        let mut box_b = dyn_sphere(Vec3::new(-3.0, 10.0, 0.0), 1.0, 0.6, 0.0);
        box_b.inv_inertia = Mat3::from_diagonal(Vec3::new(0.8, 1.3, 1.0));
        box_b.linear_velocity = Vec3::new(-1.0, 0.0, 0.3);
        box_b.angular_velocity = Vec3::new(-0.2, 0.05, 0.1);
        let box_a_row = bodies.len() as u32;
        bodies.push(box_a);
        let box_b_row = bodies.len() as u32;
        bodies.push(box_b);
        // The shared static floor (last row).
        let floor_row = bodies.len() as u32;
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));

        let mut manifolds = Vec::new();
        for i in 0..n_floor {
            manifolds.push(manifold(
                i,
                floor_row,
                Vec3::new(0.0, -1.0, 0.0),
                -0.2,
                Vec3::new(i as f32 * 3.0, 0.0, 0.0),
            ));
        }
        // Width-4 box-box manifold: A → B along +x, deep overlap.
        manifolds.push(box_manifold(
            box_a_row,
            box_b_row,
            Vec3::new(1.0, 0.0, 0.0),
            -0.3,
            Vec3::new(-4.0, 10.0, 0.0),
            4,
        ));
        (bodies, manifolds)
    }

    /// Captures the full body + impulse-column bit state after solving each color's
    /// groups with the supplied per-color kernel. Returns `(body_bits,
    /// impulse_bits)`. Used only by the +avx2 differential.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn body_impulse_bits(bodies: &[BodyEffective], cols: &ContactColumns) -> (Vec<u32>, Vec<u32>) {
        let body_bits = bodies
            .iter()
            .flat_map(|b| {
                [
                    b.linear_velocity.x.to_bits(),
                    b.linear_velocity.y.to_bits(),
                    b.linear_velocity.z.to_bits(),
                    b.angular_velocity.x.to_bits(),
                    b.angular_velocity.y.to_bits(),
                    b.angular_velocity.z.to_bits(),
                ]
            })
            .collect();
        let impulse_bits = (0..cols.len())
            .flat_map(|i| {
                [
                    cols.normal_impulse(i).to_bits(),
                    cols.tangent1_impulse(i).to_bits(),
                    cols.tangent2_impulse(i).to_bits(),
                ]
            })
            .collect();
        (body_bits, impulse_bits)
    }

    /// Test 1 (INVIOLABLE-1): `solve_color_avx2 == solve_color` bit-exact over a
    /// ragged colored scene (width-1 floor groups crossing the 8-lane boundary + a
    /// width-4 box manifold ⇒ exhausted lanes at high ranks), for both
    /// `bias_active ∈ {true, false}`. AVX2-only (the kernel is `cfg`-gated); on a
    /// non-AVX2 build the dispatch IS the scalar oracle so the property is vacuous —
    /// the always-compiled width-only test below covers that build.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn simd_solve_bits_match_scalar() {
        // 11 floor spheres + 1 box pair ⇒ 12 width-1 groups + 1 width-4 group; the
        // floor spheres land in one color (≥ 9 groups ⇒ crosses the 8-cohort
        // boundary with a partial trailing cohort), the box pair in its own
        // group(s). Ragged widths in one cohort ⇒ exhausted-lane coverage.
        let (bodies, manifolds) = ragged_colored_scene(11);
        let graph = build_graph(&bodies, &manifolds);

        let soft = SoftCoefficients::new(
            PhysicsConfig::default().contact_hertz,
            PhysicsConfig::default().contact_damping,
            (1.0 / 60.0) / 4.0,
        );

        for bias_active in [true, false] {
            // The pristine pre-solve body state shared by both arms.
            let pristine_bodies: Vec<BodyEffective> = bodies.iter().map(eff_of).collect();

            // Each arm builds its OWN columns from a fresh solver (an empty
            // warm-start table ⇒ identical zero-seeded pristine columns) and its OWN
            // body ScratchColumn, then solves through the per-element solve views.

            // ── Scalar arm ──────────────────────────────────────────────────
            let mut solver_scalar = ColoredSoftStepSolver::default();
            solver_scalar.build_bodies(&bodies);
            solver_scalar.build_columns(&manifolds, &graph, &bodies, None);
            let cols_scalar = &solver_scalar.columns;
            let n_colors = cols_scalar.color_offsets().len() - 1;
            let bodies_scalar = body_scratch_from(&pristine_bodies);
            {
                let view = cols_scalar.solve_view();
                let body_view = bodies_scalar.solve_view();
                for c in 0..n_colors {
                    let start = cols_scalar.color_offsets()[c] as usize;
                    let end = cols_scalar.color_offsets()[c + 1] as usize;
                    ColoredSoftStepSolver::solve_color(
                        view,
                        body_view,
                        (start, end),
                        soft.bias_rate,
                        soft.mass_coeff,
                        soft.impulse_coeff,
                        bias_active,
                    );
                }
            }

            // ── SIMD arm ─────────────────────────────────────────────────────
            let mut solver_simd = ColoredSoftStepSolver::default();
            solver_simd.build_bodies(&bodies);
            solver_simd.build_columns(&manifolds, &graph, &bodies, None);
            let cols_simd = &solver_simd.columns;
            let bodies_simd = body_scratch_from(&pristine_bodies);
            {
                let view = cols_simd.solve_view();
                let body_view = bodies_simd.solve_view();
                for c in 0..n_colors {
                    let g_lo = cols_simd.color_group_start()[c] as usize;
                    let g_hi = cols_simd.color_group_start()[c + 1] as usize;
                    let span = (
                        cols_simd.color_offsets()[c] as usize,
                        cols_simd.color_offsets()[c + 1] as usize,
                    );
                    // SAFETY: the test target is gated `target_feature = "avx2"`, so
                    //   the host running these tests supports AVX2; the group range is a
                    //   color's own (body-disjoint) groups, and `span` is exactly that
                    //   range's slot run (the kernel's own-span contract).
                    unsafe {
                        ColoredSoftStepSolver::solve_color_avx2(
                            view,
                            body_view,
                            span,
                            g_lo,
                            g_hi,
                            soft.bias_rate,
                            soft.mass_coeff,
                            soft.impulse_coeff,
                            bias_active,
                        );
                    }
                }
            }

            let (b_scalar, i_scalar) =
                body_impulse_bits(bodies_scalar.as_read_slice(), &solver_scalar.columns);
            let (b_simd, i_simd) =
                body_impulse_bits(bodies_simd.as_read_slice(), &solver_simd.columns);
            assert_eq!(
                b_scalar, b_simd,
                "O7 body velocity bits must match scalar (bias_active={bias_active})"
            );
            assert_eq!(
                i_scalar, i_simd,
                "O7 impulse column bits must match scalar (bias_active={bias_active})"
            );
        }
    }

    /// Builds a fresh `BodyEffective` [`ScratchColumn`] seeded from `bodies` (the
    /// +avx2 differential's per-arm body buffer — each arm needs its own buffer so
    /// the two kernels do not share mutable body rows). The synthetic
    /// `SCRATCH_ID_BODY_EFF_COLORED` id backs it (the same id the colored solver
    /// owns; the columns are independent pools keyed by id).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn body_scratch_from(bodies: &[BodyEffective]) -> ScratchColumn<BodyEffective> {
        register_scratch_layouts();
        let mut col = ScratchColumn::<BodyEffective>::new(
            body_eff_colored_id(),
            bodies.len().max(scratch_reserve_rows(size_of::<BodyEffective>())),
        );
        {
            let mut view = col.build_view();
            view.clear();
            view.extend_from_slice(bodies);
        }
        col
    }

    /// Deep-copies `src` into a fresh `ContactColumns` (each column refilled from the
    /// source's read slice). Used by the +avx2 differential so the two kernel arms
    /// solve over independent column buffers. Copies the columns the kernels read /
    /// write plus the CSR columns they navigate.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn clone_columns(src: &ContactColumns) -> ContactColumns {
        let mut dst = ContactColumns::with_capacity(src.len());
        {
            let mut v = dst.build_view();
            v.clear();
            v.ra_x.extend_from_slice(src.ra_x.as_read_slice());
            v.ra_y.extend_from_slice(src.ra_y.as_read_slice());
            v.ra_z.extend_from_slice(src.ra_z.as_read_slice());
            v.rb_x.extend_from_slice(src.rb_x.as_read_slice());
            v.rb_y.extend_from_slice(src.rb_y.as_read_slice());
            v.rb_z.extend_from_slice(src.rb_z.as_read_slice());
            v.normal_x.extend_from_slice(src.normal_x.as_read_slice());
            v.normal_y.extend_from_slice(src.normal_y.as_read_slice());
            v.normal_z.extend_from_slice(src.normal_z.as_read_slice());
            v.tangent1_x.extend_from_slice(src.tangent1_x.as_read_slice());
            v.tangent1_y.extend_from_slice(src.tangent1_y.as_read_slice());
            v.tangent1_z.extend_from_slice(src.tangent1_z.as_read_slice());
            v.tangent2_x.extend_from_slice(src.tangent2_x.as_read_slice());
            v.tangent2_y.extend_from_slice(src.tangent2_y.as_read_slice());
            v.tangent2_z.extend_from_slice(src.tangent2_z.as_read_slice());
            v.separation.extend_from_slice(src.separation.as_read_slice());
            v.friction.extend_from_slice(src.friction.as_read_slice());
            v.restitution.extend_from_slice(src.restitution.as_read_slice());
            v.normal_impulse.extend_from_slice(src.normal_impulse.as_read_slice());
            v.tangent1_impulse.extend_from_slice(src.tangent1_impulse.as_read_slice());
            v.tangent2_impulse.extend_from_slice(src.tangent2_impulse.as_read_slice());
            v.body_a.extend_from_slice(src.body_a.as_read_slice());
            v.body_b.extend_from_slice(src.body_b.as_read_slice());
            v.b_is_sentinel.extend_from_slice(src.b_is_sentinel.as_read_slice());
            v.warm_key.extend_from_slice(src.warm_key.as_read_slice());
            v.vn_initial.extend_from_slice(src.vn_initial.as_read_slice());
        }
        // The CSR columns the kernels navigate (group_start) + the dispatcher CSRs.
        dst.color_offsets.build_view().extend_from_slice(src.color_offsets());
        dst.group_start.build_view().extend_from_slice(src.group_start());
        dst.color_group_start
            .build_view()
            .extend_from_slice(src.color_group_start());
        dst.canonical.build_view().extend_from_slice(src.canonical());
        dst
    }

    /// Test 2 (width-only / 0%-gate proxy): `solve_colored(simd=true)` produces a
    /// full-step body snapshot bit-identical to `solve_colored(simd=false)`. On a
    /// non-AVX2 build both arms run the scalar oracle (so the equality is the
    /// structural 0%-gate); under +avx2 it proves the widened cohort kernel
    /// reproduces the O5/O6 scalar colored result bit-for-bit over a multi-substep
    /// step incl. the multi-point box manifold.
    #[test]
    fn simd_solve_width_only_matches_scalar_step() {
        let (bodies, manifolds) = ragged_colored_scene(11);

        let run_step = |simd_solve: bool| -> Vec<u32> {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                simd_solve,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            scratch.set_bodies(&bodies);
            scratch.touched.reset(scratch.bodies().len());
            let graph = build_graph(scratch.bodies(), &manifolds);
            solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            scratch
                .bodies()
                .iter()
                .flat_map(|b| {
                    [
                        b.position.x.to_bits(),
                        b.position.y.to_bits(),
                        b.position.z.to_bits(),
                        b.linear_velocity.x.to_bits(),
                        b.linear_velocity.y.to_bits(),
                        b.linear_velocity.z.to_bits(),
                        b.angular_velocity.x.to_bits(),
                        b.angular_velocity.y.to_bits(),
                        b.angular_velocity.z.to_bits(),
                    ]
                })
                .collect()
        };

        assert_eq!(
            run_step(false),
            run_step(true),
            "width-only: simd_solve=true must be bit-identical to the scalar colored result"
        );
    }

    // ── O1 (regression-pin): cone / degenerate adversarial differential ──────
    //
    // Test 1c/1d build a SINGLE one-color, one-cohort `ContactColumns` BY HAND so
    // every lane's geometry / impulse seed / body state is exact, forcing the
    // adversarial friction-cone + degenerate paths to fire NON-VACUOUSLY, then
    // assert `solve_color_avx2 == solve_color` bit-for-bit. The non-vacuity counts
    // come from `cone_probe`, a single-slot replay of the EXACT scalar op sequence
    // (the authoritative oracle for "did this lane clamp / was len_sq zero /
    // denormal"). A splitmix64 proptest then sweeps random cohort shapes.

    /// One built group spec for a hand-rolled single-color cohort: a body pair
    /// (`ia`, `ib`/sentinel) and its contact points. Each point carries explicit
    /// geometry, friction, separation, and an impulse seed.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[derive(Clone)]
    struct GroupSpec {
        ia: u32,
        ib: u32,
        sentinel: bool,
        /// `(ra, rb, normal, t1, t2, separation, friction, seed_ni, seed_t1, seed_t2)`.
        points: Vec<PointSpec>,
    }

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[derive(Clone, Copy)]
    struct PointSpec {
        ra: Vec3,
        rb: Vec3,
        normal: Vec3,
        t1: Vec3,
        t2: Vec3,
        separation: f32,
        friction: f32,
        seed: (f32, f32, f32),
    }

    /// Builds a single-COLOR, single-cohort (`groups.len() <= 8`) `ContactColumns`
    /// from the group specs, appending groups in order with the C1 CSR
    /// (`group_start` / `color_group_start` / `color_offsets`). Body-disjointness of
    /// the groups is the CALLER's responsibility (the cohort kernel's precondition).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn build_cohort_columns(groups: &[GroupSpec]) -> ContactColumns {
        let mut cols = ContactColumns::with_capacity(0);
        cols.begin_build();
        let mut warm_key = 0u64;
        for g in groups {
            {
                let mut view = cols.build_view();
                for ps in g.points.iter() {
                    view.push_point(
                        ps.ra,
                        ps.rb,
                        ps.normal,
                        ps.t1,
                        ps.t2,
                        ps.separation,
                        ps.friction,
                        0.0, // restitution (the kernels do not read it)
                        ps.seed,
                        g.ia,
                        g.ib,
                        g.sentinel,
                        warm_key,
                        0.0,
                    );
                    warm_key = warm_key.wrapping_add(1);
                }
            }
            // `canonical` covers the slots appended for this group, in order.
            let len = cols.len() as u32;
            for s in (len - g.points.len() as u32)..len {
                cols.push_canonical(s);
            }
            cols.push_group_start(len);
        }
        let len = cols.len() as u32;
        cols.push_color_offset(len);
        cols.push_color_group_start((cols.group_start().len() - 1) as u32);
        cols
    }

    /// Replays the EXACT scalar `solve_color` friction-cone evaluation for ONE slot
    /// against the pristine pre-solve state, reporting `(clamped, zero_cone,
    /// denorm_len_sq)`. The kernel is bit-identical to `solve_color`, so this is the
    /// authoritative non-vacuity oracle for that slot. `len_sq == 0` ⇒ `zero_cone`;
    /// `0 < len_sq < f32::MIN_POSITIVE` ⇒ `denorm_len_sq`; the scalar clamp branch
    /// (`len_sq > mf² && len_sq > 0`) firing ⇒ `clamped`.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn cone_probe(
        cols: &ContactColumns,
        bodies: &[BodyEffective],
        slot: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) -> (bool, bool, bool) {
        let ra = cols.ra(slot);
        let rb = cols.rb(slot);
        let normal = cols.normal(slot);
        let t1 = cols.tangent1(slot);
        let t2 = cols.tangent2(slot);
        let ia = cols.body_a(slot) as usize;
        let b_sent = cols.b_is_sentinel(slot);
        let ib = cols.body_b(slot) as usize;
        let friction = cols.friction.as_read_slice()[slot];
        let separation = cols.separation.as_read_slice()[slot];
        let bb = if b_sent { IMMOVABLE_AT_REST } else { bodies[ib] };
        let ba = bodies[ia];

        // Normal solve (to obtain the new normal impulse the cone uses).
        let m_eff = effective_mass(normal, ra, rb, &ba, &bb);
        let vn = (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal);
        let bias = if bias_active {
            (bias_rate * separation).max(-MAX_BIAS_VELOCITY)
        } else {
            0.0
        };
        let lambda_n = cols.normal_impulse(slot);
        let d_lambda = if bias_active {
            -mass_coeff * m_eff * (vn + bias) - impulse_coeff * lambda_n
        } else {
            -m_eff * vn
        };
        let new_lambda = (lambda_n + d_lambda).max(0.0);

        // Friction solve (no body mutation needed — single-point group, the cone
        // reads only the post-normal velocity; a single-point group's normal apply
        // does change velocity, so re-derive from a local copy).
        let mut ba_m = ba;
        let mut bb_m = bb;
        let applied_n = new_lambda - lambda_n;
        let imp = normal * applied_n;
        if is_dynamic_row(ba_m.inv_mass) {
            ba_m.apply_impulse(ra, imp * -1.0);
        }
        if !b_sent && is_dynamic_row(bb_m.inv_mass) {
            bb_m.apply_impulse(rb, imp);
        }
        let max_friction = friction * new_lambda;
        let m_eff_t1 = effective_mass(t1, ra, rb, &ba_m, &bb_m);
        let m_eff_t2 = effective_mass(t2, ra, rb, &ba_m, &bb_m);
        let dv = bb_m.point_velocity(rb) - ba_m.point_velocity(ra);
        let (vt1, vt2) = (dv.dot(t1), dv.dot(t2));
        let new_t1 = cols.tangent1_impulse(slot) - m_eff_t1 * vt1;
        let new_t2 = cols.tangent2_impulse(slot) - m_eff_t2 * vt2;
        let len_sq = new_t1 * new_t1 + new_t2 * new_t2;
        let clamped = len_sq > max_friction * max_friction && len_sq > 0.0;
        let zero_cone = len_sq == 0.0;
        let denorm = len_sq > 0.0 && len_sq < f32::MIN_POSITIVE;
        (clamped, zero_cone, denorm)
    }

    /// Solves the single color of `cols` with the scalar oracle and with the AVX2
    /// cohort kernel (each on a fresh clone seeded to the same pristine state), and
    /// asserts the body + impulse bits match bit-for-bit. Returns the per-slot
    /// `(clamped, zero_cone, denorm)` counts from the scalar probe for non-vacuity.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn assert_cohort_differential(
        cols: &ContactColumns,
        bodies: &[BodyEffective],
        bias_active: bool,
    ) -> (usize, usize, usize) {
        let soft = SoftCoefficients::new(
            PhysicsConfig::default().contact_hertz,
            PhysicsConfig::default().contact_damping,
            (1.0 / 60.0) / 4.0,
        );

        // Non-vacuity counts from the pristine state (the probe is read-only).
        let (mut clamped, mut zero_cone, mut denorm) = (0usize, 0usize, 0usize);
        for s in 0..cols.len() {
            let (c, z, d) = cone_probe(
                cols,
                bodies,
                s,
                soft.bias_rate,
                soft.mass_coeff,
                soft.impulse_coeff,
                bias_active,
            );
            clamped += c as usize;
            zero_cone += z as usize;
            denorm += d as usize;
        }

        let g_lo = cols.color_group_start()[0] as usize;
        let g_hi = cols.color_group_start()[1] as usize;
        let span = (cols.color_offsets()[0] as usize, cols.color_offsets()[1] as usize);

        // Scalar arm — a fresh deep copy of `cols` + its own body buffer.
        let cols_scalar = clone_columns(cols);
        let bodies_scalar = body_scratch_from(bodies);
        ColoredSoftStepSolver::solve_color(
            cols_scalar.solve_view(),
            bodies_scalar.solve_view(),
            span,
            soft.bias_rate,
            soft.mass_coeff,
            soft.impulse_coeff,
            bias_active,
        );

        // SIMD arm — an independent deep copy + body buffer.
        let cols_simd = clone_columns(cols);
        let bodies_simd = body_scratch_from(bodies);
        // SAFETY: the test target is `target_feature = "avx2"`-gated, so the host
        //   supports AVX2; `[g_lo, g_hi)` is the single color's body-disjoint groups
        //   and `span` is exactly that group range's slot run (the own-span contract).
        unsafe {
            ColoredSoftStepSolver::solve_color_avx2(
                cols_simd.solve_view(),
                bodies_simd.solve_view(),
                span,
                g_lo,
                g_hi,
                soft.bias_rate,
                soft.mass_coeff,
                soft.impulse_coeff,
                bias_active,
            );
        }

        let (b_scalar, i_scalar) =
            body_impulse_bits(bodies_scalar.as_read_slice(), &cols_scalar);
        let (b_simd, i_simd) = body_impulse_bits(bodies_simd.as_read_slice(), &cols_simd);
        assert_eq!(b_scalar, b_simd, "cohort differential: body bits (bias_active={bias_active})");
        assert_eq!(i_scalar, i_simd, "cohort differential: impulse bits (bias_active={bias_active})");
        (clamped, zero_cone, denorm)
    }

    /// A dynamic `BodyEffective` with a diagonal inertia and the given velocity.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn dyn_eff(inv_mass: f32, inertia_diag: f32, lin: Vec3, ang: Vec3) -> BodyEffective {
        BodyEffective {
            inv_mass,
            inv_inertia: Mat3::from_diagonal(Vec3::new(inertia_diag, inertia_diag, inertia_diag)),
            linear_velocity: lin,
            angular_velocity: ang,
        }
    }

    /// Test 1c (+avx2 only): a single cohort with a cone-CLAMPED lane, an unclamped
    /// lane, a `len_sq == 0` zero-tangent lane, and a denormal-`len_sq` lane, all
    /// body-disjoint. Asserts `solve_color_avx2 == solve_color` bit-for-bit AND that
    /// the clamp / zero-cone / denormal paths each fire (non-vacuity).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn cone_adversarial_differential_test_1c() {
        // Build 5 body-disjoint single-point groups (one cohort), bodies 0..10.
        let n = Vec3::new(0.0, 1.0, 0.0);
        let (t1, t2) = tangent_basis(n);
        let mk = |ia: u32,
                  ib: u32,
                  friction: f32,
                  seed_ni: f32,
                  seed_t1: f32,
                  seed_t2: f32,
                  ra: Vec3|
         -> GroupSpec {
            GroupSpec {
                ia,
                ib,
                sentinel: false,
                points: vec![PointSpec {
                    ra,
                    rb: ra,
                    normal: n,
                    t1,
                    t2,
                    separation: -0.2,
                    friction,
                    seed: (seed_ni, seed_t1, seed_t2),
                }],
            }
        };

        // Lane 0 — cone-CLAMPED: large pre-seeded tangent impulse + a small normal
        // cap (friction·λn small) ⇒ len_sq ≫ mf² ⇒ clamp fires.
        let g0 = mk(0, 1, 0.1, 0.05, 5.0, 5.0, Vec3::new(0.3, 0.0, 0.1));
        // Lane 1 — UNCLAMPED: tiny tangent seed, generous friction cap ⇒ inside cone.
        let g1 = mk(2, 3, 2.0, 2.0, 1e-4, 1e-4, Vec3::new(-0.2, 0.0, 0.2));
        // Lane 2 — ZERO-tangent (`len_sq == 0`): zero friction AND zero tangent seed
        // with zero tangential velocity ⇒ new_t1 == new_t2 == 0 ⇒ len_sq == 0.
        let g2 = mk(4, 5, 0.0, 1.0, 0.0, 0.0, Vec3::ZERO);
        // Lane 3 — DENORMAL len_sq: a tiny tangent seed (subnormal-squared) with zero
        // tangential velocity ⇒ new_t stays the seed ⇒ len_sq ≈ seed² is subnormal.
        let tiny = 1e-22f32; // tiny² ≈ 1e-44 < f32::MIN_POSITIVE (≈ 1.18e-38)
        let g3 = mk(6, 7, 5.0, 0.0, tiny, 0.0, Vec3::ZERO);
        // Lane 4 — a second clamped lane on a sentinel body B (static surface).
        let mut g4 = mk(8, 9, 0.2, 0.1, 4.0, -3.0, Vec3::new(0.1, 0.0, -0.3));
        g4.sentinel = true;
        g4.ib = u32::MAX;

        let groups = vec![g0, g1, g2, g3, g4];
        let cols = build_cohort_columns(&groups);
        // 10 real bodies; spins so the angular term is non-vacuous. Lane-2 (g2)
        // bodies are zero-velocity so its tangent stays exactly zero.
        let bodies: Vec<BodyEffective> = (0..10)
            .map(|i| {
                if (4..=5).contains(&i) {
                    dyn_eff(1.0, 1.5, Vec3::ZERO, Vec3::ZERO)
                } else if (6..=7).contains(&i) {
                    // Lane-3 bodies zero-velocity too so its tangent stays the seed.
                    dyn_eff(1.0, 1.5, Vec3::ZERO, Vec3::ZERO)
                } else {
                    dyn_eff(
                        1.0,
                        1.5,
                        Vec3::new(0.2 * (i as f32 + 1.0), -1.0, 0.15),
                        Vec3::new(0.05, -0.1, 0.2),
                    )
                }
            })
            .collect();

        let mut total_clamped = 0;
        let mut total_zero = 0;
        let mut total_denorm = 0;
        for bias_active in [true, false] {
            let (c, z, d) = assert_cohort_differential(&cols, &bodies, bias_active);
            total_clamped += c;
            total_zero += z;
            total_denorm += d;
        }
        eprintln!(
            "test_1c non-vacuity: clamped={total_clamped} zero_cone={total_zero} denorm={total_denorm}"
        );
        assert!(
            total_clamped > 0 && total_zero > 0 && total_denorm > 0,
            "non-vacuity: cone clamp ({total_clamped}), zero-cone ({total_zero}), and denormal \
             len_sq ({total_denorm}) lanes must each fire across the two bias modes"
        );
    }

    /// Test 1d (+avx2 only): a single cohort mixing a static-A lane (`inv_mass == 0`
    /// on body A — the `*_movable` guard side), a sentinel-B lane, and a `k <= 0`
    /// degenerate lane (both bodies static ⇒ `effective_mass == 0`). Asserts
    /// `solve_color_avx2 == solve_color` bit-for-bit; non-vacuity asserts the cone
    /// fires on the live lane.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn degenerate_lane_differential_test_1d() {
        let n = Vec3::new(0.0, 1.0, 0.0);
        let (t1, t2) = tangent_basis(n);
        let pt = |friction: f32, seed: (f32, f32, f32), ra: Vec3| PointSpec {
            ra,
            rb: ra,
            normal: n,
            t1,
            t2,
            separation: -0.25,
            friction,
            seed,
        };

        // Lane 0 — STATIC body A (inv_mass 0): the `ia_movable == false` guard side.
        let g0 = GroupSpec {
            ia: 0,
            ib: 1,
            sentinel: false,
            points: vec![pt(0.5, (0.1, 0.2, -0.1), Vec3::new(0.2, 0.0, 0.1))],
        };
        // Lane 1 — SENTINEL body B: body B is IMMOVABLE_AT_REST, never indexed; a
        // live dynamic A with a clamp-forcing tangent seed.
        let g1 = GroupSpec {
            ia: 2,
            ib: u32::MAX,
            sentinel: true,
            points: vec![pt(0.05, (0.05, 6.0, 6.0), Vec3::new(-0.1, 0.0, 0.3))],
        };
        // Lane 2 — DEGENERATE k<=0: both bodies static (inv_mass 0, inertia ZERO) ⇒
        // effective_mass returns 0 ⇒ a no-op solve.
        let g2 = GroupSpec {
            ia: 3,
            ib: 4,
            sentinel: false,
            points: vec![pt(0.5, (0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.2))],
        };

        let groups = vec![g0, g1, g2];
        let cols = build_cohort_columns(&groups);
        // Bodies: 0 static-A, 1 dynamic, 2 dynamic (sentinel lane's A), 3+4 static.
        let bodies = vec![
            // 0: static A (inv_mass 0 ⇒ inertia ZERO to match the build invariant).
            BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: Vec3::new(1.0, -0.5, 0.2), angular_velocity: Vec3::new(0.1, 0.0, -0.1) },
            // 1: dynamic B.
            dyn_eff(1.0, 1.2, Vec3::new(-0.3, 0.4, 0.1), Vec3::new(-0.05, 0.1, 0.0)),
            // 2: dynamic A (sentinel lane) with a fast tangential slide ⇒ cone fires.
            dyn_eff(1.0, 1.0, Vec3::new(2.0, -1.0, -1.5), Vec3::new(0.2, -0.1, 0.3)),
            // 3, 4: both static (degenerate k<=0 lane).
            BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: Vec3::ZERO, angular_velocity: Vec3::ZERO },
            BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: Vec3::ZERO, angular_velocity: Vec3::ZERO },
        ];

        let mut total_clamped = 0;
        for bias_active in [true, false] {
            let (c, _z, _d) = assert_cohort_differential(&cols, &bodies, bias_active);
            total_clamped += c;
        }
        eprintln!("test_1d non-vacuity: clamped={total_clamped}");
        assert!(
            total_clamped > 0,
            "non-vacuity: the sentinel-B live lane's friction cone must clamp at least once"
        );
    }

    /// A splitmix64 PRNG (deterministic, no deps) for the cohort-shape proptest.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    struct SplitMix64(u64);

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    impl SplitMix64 {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn f01(&mut self) -> f32 {
            (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
        }
        fn range(&mut self, lo: u32, hi: u32) -> u32 {
            lo + (self.next_u64() % (hi - lo) as u64) as u32
        }
    }

    /// O1 proptest (+avx2 only): random cohort shapes (group count 1..=32, width
    /// 1..=MAX_CONTACT_POINTS, masses incl. statics + sentinels, denormal-scale
    /// velocities) must be `solve_color_avx2 == solve_color` bit-for-bit, AND the
    /// cone clamp + zero-cone paths must fire non-vacuously across the corpus.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn cohort_shape_proptest_bit_exact_and_non_vacuous() {
        use crate::math::MAX_CONTACT_POINTS;
        let n = Vec3::new(0.0, 1.0, 0.0);
        let (t1, t2) = tangent_basis(n);

        let mut rng = SplitMix64(0x0BAD_F00D_DEAD_BEEF);
        let mut corpus_clamped = 0usize;
        let mut corpus_zero = 0usize;

        for _ in 0..200 {
            let n_groups = rng.range(1, 33) as usize; // 1..=32 ⇒ multi-cohort
            let mut groups: Vec<GroupSpec> = Vec::with_capacity(n_groups);
            // Body rows: each group owns 2 disjoint dynamic rows (or 1 + sentinel).
            let mut bodies: Vec<BodyEffective> = Vec::with_capacity(n_groups * 2);
            for _gi in 0..n_groups {
                let ia = bodies.len() as u32;
                // Body A: mostly dynamic, sometimes static (the *_movable guard).
                let a_static = rng.f01() < 0.15;
                bodies.push(if a_static {
                    BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: rand_vel(&mut rng), angular_velocity: rand_vel(&mut rng) }
                } else {
                    dyn_eff(0.5 + rng.f01(), 0.5 + rng.f01() * 2.0, rand_vel(&mut rng), rand_vel(&mut rng))
                });
                let sentinel = rng.f01() < 0.25;
                let ib = if sentinel {
                    u32::MAX
                } else {
                    let row = bodies.len() as u32;
                    let b_static = rng.f01() < 0.15;
                    bodies.push(if b_static {
                        BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: rand_vel(&mut rng), angular_velocity: rand_vel(&mut rng) }
                    } else {
                        dyn_eff(0.5 + rng.f01(), 0.5 + rng.f01() * 2.0, rand_vel(&mut rng), rand_vel(&mut rng))
                    });
                    row
                };
                let width = rng.range(1, MAX_CONTACT_POINTS as u32 + 1) as usize;
                let mut points = Vec::with_capacity(width);
                for _ in 0..width {
                    // Occasionally a denormal-scale tangent seed + zero friction.
                    let denorm = rng.f01() < 0.1;
                    let zero_fric = rng.f01() < 0.1;
                    let seed_scale = if denorm { 1e-22 } else { 4.0 };
                    points.push(PointSpec {
                        ra: rand_vel(&mut rng) * 0.3,
                        rb: rand_vel(&mut rng) * 0.3,
                        normal: n,
                        t1,
                        t2,
                        separation: -(rng.f01() * 0.5),
                        friction: if zero_fric { 0.0 } else { rng.f01() * 2.0 },
                        seed: (
                            rng.f01() * 0.5,
                            (rng.f01() - 0.5) * seed_scale,
                            (rng.f01() - 0.5) * seed_scale,
                        ),
                    });
                }
                groups.push(GroupSpec { ia, ib, sentinel, points });
            }

            // build_cohort_columns packs ALL groups into ONE color (multi-cohort
            // when n_groups > 8); the kernel solves them as 8-group cohorts.
            let cols = build_cohort_columns(&groups);
            for bias_active in [true, false] {
                let (c, z, _d) = assert_cohort_differential(&cols, &bodies, bias_active);
                corpus_clamped += c;
                corpus_zero += z;
            }
        }
        eprintln!("proptest non-vacuity: clamped={corpus_clamped} zero_cone={corpus_zero}");
        assert!(
            corpus_clamped > 0 && corpus_zero > 0,
            "non-vacuity over the random corpus: clamp ({corpus_clamped}) and zero-cone \
             ({corpus_zero}) paths must both fire"
        );
    }

    /// A bounded random velocity for the proptest.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn rand_vel(rng: &mut SplitMix64) -> Vec3 {
        Vec3::new(
            (rng.f01() - 0.5) * 4.0,
            (rng.f01() - 0.5) * 4.0,
            (rng.f01() - 0.5) * 4.0,
        )
    }

    // ── O8 sleeping sanity tests ─────────────────────────────────────────────
    //
    // These build the bodies + per-step manifolds by hand and drive
    // `solve_colored_sleeping` directly (NO schedule / threadpool), so they run
    // native and under Miri. The exhaustive determinism / oscillation / criterion
    // suite is the tester's job.

    /// Drives the colored solver with O8 sleeping for `steps` fixed steps, returning
    /// `(final Y positions, the IslandSleep state)`. `cfg_mut` tweaks the config (e.g.
    /// the sleep threshold / frame count). The manifolds are re-derived from the
    /// current positions each step (the narrowphase stand-in), so a settled stack
    /// keeps producing its resting-floor contacts.
    fn run_sleeping(
        bodies: Vec<BodyState>,
        build_manifolds: impl Fn(&[BodyState]) -> Vec<Manifold>,
        steps: usize,
        cfg_mut: impl Fn(&mut PhysicsConfig),
    ) -> (Vec<f32>, IslandSleep) {
        let mut cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            ..PhysicsConfig::default()
        };
        cfg_mut(&mut cfg);
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(&bodies);
        scratch.touched.reset(scratch.bodies().len());
        let mut sleep = IslandSleep::with_capacity(scratch.bodies().len(), scratch.bodies().len());

        for _ in 0..steps {
            let manifolds = build_manifolds(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        let ys = scratch.bodies().iter().map(|b| b.position.y).collect();
        (ys, sleep)
    }

    /// A dynamic sphere resting on a static floor settles, then the island sleeps
    /// after `sleep_frames` consecutive low-energy frames (the headline O8 gate).
    #[test]
    fn dropped_body_settles_then_sleeps() {
        // Sphere just above a static floor; a short debounce so the test is brisk.
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.05, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, 0.0, 0.0)),
        ];
        // The narrowphase stand-in: emit a floor contact whenever the sphere dips into
        // the floor (separation < 0), keyed (sphere, floor).
        let build = |bs: &[BodyState]| {
            let y = bs[0].position.y;
            let sep = y - 1.0; // sphere radius 1, floor surface at y = 1.0.
            if sep < 0.0 {
                vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), sep, Vec3::new(0.0, 1.0, 0.0))]
            } else {
                vec![]
            }
        };
        // Settle for many frames with a short 8-frame debounce, then keep stepping so
        // the debounce elapses.
        let (ys, sleep) = run_sleeping(bodies, build, 200, |c| c.sleep_frames = 8);

        // The sphere rests on the floor (did not sink far through it, did not fly off).
        assert!(
            (ys[0] - 1.0).abs() < 0.1,
            "sphere should rest near the floor surface (y ≈ 1.0), got {}",
            ys[0]
        );
        // The sphere's row is latched asleep (it settled, the debounce elapsed).
        assert!(
            sleep.is_row_asleep(0),
            "the settled body row must be latched asleep after the debounce"
        );
    }

    /// A slept island stays frozen across steps — its body neither drifts nor
    /// accumulates gravity (the integrate-skip gate). The floor contact is dropped
    /// while asleep, but the body must not fall.
    #[test]
    fn slept_body_is_frozen_no_drift() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, 0.0, 0.0)),
        ];
        // Resting-floor contact every step (sphere exactly on the surface).
        let build = |_bs: &[BodyState]| {
            vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.001, Vec3::new(0.0, 1.0, 0.0))]
        };
        let (ys, sleep) = run_sleeping(bodies, build, 100, |c| c.sleep_frames = 4);
        assert!(sleep.is_row_asleep(0), "the resting body row must be latched asleep");
        // A frozen body neither drifts down (gravity skipped) nor pops up.
        assert!(
            (ys[0] - 1.0).abs() < 1.0e-3,
            "a slept body must stay frozen at its rest Y, got {}",
            ys[0]
        );
    }

    /// **The real-pipeline wake-on-merge gate (the rewritten C1/C2 test).** A faller
    /// (a new awake body bringing a NEW contact) wakes a slept pile the SAME frame the
    /// contact appears — validated through the REAL solve, NOT a stale-graph artifact.
    ///
    /// Both arms drive `solve_colored_sleeping` step-by-step, re-deriving the manifolds
    /// AND the graph from the SAME current positions every frame (so `begin_step` sees
    /// exactly the graph the solve uses — the bug the old test cheated around). A pile
    /// of two spheres on a floor settles + latches asleep; then a faller is dropped onto
    /// it. The asserted behaviour: on the frame the faller's contact first appears, the
    /// pile's rows are ACTIVE (awake, not frozen) and the contact is resolved — no
    /// mid-air freeze, no penetration-stick.
    #[test]
    fn faller_wakes_slept_pile_same_frame_no_penetration() {
        // Pile: two stacked dynamic spheres (radius 1) resting on a static floor.
        // floor top surface at y = 1; sphere 0 centre at y ≈ 1; sphere 1 at y ≈ 3.
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0), // row 0 (bottom)
                dyn_sphere(Vec3::new(0.0, 3.0, 0.0), 1.0, 0.5, 0.0), // row 1 (top)
                static_body(Vec3::new(0.0, 0.0, 0.0)),               // row 2 (floor)
                dyn_sphere(Vec3::new(0.0, 30.0, 0.0), 1.0, 0.5, 0.0), // row 3 (faller, far above)
            ]
        };
        // Narrowphase stand-in: floor↔bottom, bottom↔top, top↔faller — each emitted
        // only while penetrating (separation < 0). Sphere radius 1 ⇒ centres touch at
        // distance 2; floor surface at y = 1.
        let build = |bs: &[BodyState]| {
            let mut ms = Vec::new();
            // floor contact for the bottom sphere.
            let sep_floor = bs[0].position.y - 1.0;
            if sep_floor < 0.0 {
                ms.push(manifold(0, 2, Vec3::new(0.0, -1.0, 0.0), sep_floor, bs[0].position));
            }
            // bottom↔top sphere-sphere.
            let d01 = bs[1].position.y - bs[0].position.y;
            if d01 - 2.0 < 0.0 {
                ms.push(manifold(0, 1, Vec3::new(0.0, 1.0, 0.0), d01 - 2.0, bs[0].position));
            }
            // top↔faller sphere-sphere (the NEW contact that must wake the pile).
            let d13 = bs[3].position.y - bs[1].position.y;
            if d13 - 2.0 < 0.0 {
                ms.push(manifold(1, 3, Vec3::new(0.0, 1.0, 0.0), d13 - 2.0, bs[1].position));
            }
            ms
        };

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            sleep_frames: 6,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(4);
        scratch.set_bodies(&make());
        // Park the faller out of the simulation (no gravity reaches it until we drop
        // it) by zeroing its inv_mass for the settle phase: an inv_mass==0 row is not
        // an island node, so it cannot perturb the pile's sleep.
        {
            let mut __bv = scratch.bodies_mut();
            __bv.as_mut_slice()[3].inv_mass = 0.0;
        }
        let mut sleep = IslandSleep::with_capacity(4, 4);

        // Settle phase: step until the pile latches asleep (bottom + top rows).
        for _ in 0..120 {
            let manifolds = build(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        assert!(
            sleep.is_row_asleep(0) && sleep.is_row_asleep(1),
            "the pile rows must latch asleep before the faller arrives"
        );
        let pile_top_y_before = scratch.bodies()[1].position.y;

        // Drop the faller: give it mass + place it just above the top sphere so its
        // contact appears within a couple of steps.
        {
            let mut __bv = scratch.bodies_mut();
            __bv.as_mut_slice()[3].inv_mass = 1.0;
        }
        {
            let mut __bv = scratch.bodies_mut();
            __bv.as_mut_slice()[3].position.y = 5.0;
        } // touches the top sphere (centre y≈3) soon.

        // Step until the faller's contact first appears, then assert the pile woke that
        // SAME frame: its rows are awake (active), the contact was solved, and the pile
        // is not penetrated through.
        let mut woke_frame = None;
        for frame in 0..30 {
            let manifolds = build(scratch.bodies());
            let faller_contact = manifolds.iter().any(|m| {
                (m.body_a.0 == 1 && m.body_b.0 == 3) || (m.body_a.0 == 3 && m.body_b.0 == 1)
            });
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);

            if faller_contact {
                // The frame the new contact appears: the pile's rows MUST be active
                // (awake) this same frame — wake-on-merge. They share an island with
                // the awake faller (row 3), so none of {0,1,3} may be frozen.
                assert!(
                    sleep.is_row_awake(0) && sleep.is_row_awake(1) && sleep.is_row_awake(3),
                    "the pile + faller rows must be ACTIVE the frame the new contact appears \
                     (wake-on-merge), not frozen"
                );
                woke_frame = Some(frame);
                break;
            }
        }
        assert!(
            woke_frame.is_some(),
            "the faller must produce a contact with the pile within the step budget"
        );

        // No penetration-stick: keep stepping; the faller must come to rest ABOVE the
        // top sphere (it cannot pass through a now-active pile).
        for _ in 0..60 {
            let manifolds = build(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        assert!(
            scratch.bodies()[3].position.y > scratch.bodies()[1].position.y,
            "the faller must rest ABOVE the top sphere, not sink through it: faller y={}, top y={}",
            scratch.bodies()[3].position.y,
            scratch.bodies()[1].position.y
        );
        // The pile did not get shoved through the floor by the impact.
        assert!(
            scratch.bodies()[1].position.y < pile_top_y_before + 0.5,
            "the pile must absorb the faller near its rest height, not be launched: \
             top y={}, was {}",
            scratch.bodies()[1].position.y,
            pile_top_y_before
        );
    }

    /// `wake_all` clears every row's latch on the next `begin_step` (wake condition
    /// (i)/(iii) — explicit / config-change wake), so no island can be frozen.
    #[test]
    fn wake_all_wakes_every_row() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.5, 1.0, 0.0), 1.0, 0.5, 0.0),
        ];
        let ms = vec![manifold(0, 1, Vec3::new(1.0, 0.0, 0.0), -0.01, Vec3::new(0.25, 1.0, 0.0))];
        let graph = build_graph(&bodies, &ms);
        let isl = graph.island_of(0);

        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        sleep.begin_step(&graph, bodies.len());
        // Latch both rows of the island asleep, then confirm the island is frozen.
        sleep.force_sleep_row(0);
        sleep.force_sleep_row(1);
        sleep.begin_step(&graph, bodies.len());
        assert!(
            sleep.is_island_frozen(isl),
            "an island whose every row is latched must be frozen"
        );

        // wake_all clears the latch, so the island is active again next frame.
        sleep.wake_all();
        sleep.begin_step(&graph, bodies.len());
        assert!(
            !sleep.is_island_frozen(isl) && !sleep.is_row_asleep(0) && !sleep.is_row_asleep(1),
            "wake_all must wake every row (no island frozen)"
        );
    }

    /// Topology change is row-keyed and cannot spuriously freeze a moving island (C3).
    /// A two-row island latches asleep; then the manifold set splits it into two
    /// singleton islands AND a brand-new awake row joins one of them. The row latch
    /// follows the BODY, not the volatile island id, so: the unperturbed singleton
    /// stays frozen (its row is still latched), and the singleton that gained the new
    /// awake row is ACTIVE (no spurious freeze of a now-moving partition).
    #[test]
    fn topology_split_is_row_keyed_no_spurious_freeze() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0), // row 0
            dyn_sphere(Vec3::new(0.5, 1.0, 0.0), 1.0, 0.5, 0.0), // row 1
            dyn_sphere(Vec3::new(0.6, 1.0, 0.0), 1.0, 0.5, 0.0), // row 2 (new awake)
        ];
        // Frame A: rows 0+1 form one island; row 2 is its own singleton.
        let ms_a = vec![manifold(0, 1, Vec3::new(1.0, 0.0, 0.0), -0.01, Vec3::new(0.25, 1.0, 0.0))];
        let graph_a = build_graph(&bodies, &ms_a);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        sleep.begin_step(&graph_a, bodies.len());
        // Latch rows 0 and 1 asleep (the resting pair); leave row 2 awake.
        sleep.force_sleep_row(0);
        sleep.force_sleep_row(1);

        // Frame B: the manifold set SPLITS — 0 alone, and 1+2 now coupled (a new awake
        // contact). Island ids are re-derived; the row latch is what carries.
        let ms_b = vec![manifold(1, 2, Vec3::new(1.0, 0.0, 0.0), -0.01, Vec3::new(0.55, 1.0, 0.0))];
        let graph_b = build_graph(&bodies, &ms_b);
        sleep.begin_step(&graph_b, bodies.len());

        let isl0 = graph_b.island_of(0);
        let isl12 = graph_b.island_of(1);
        // Row 0 alone: still latched ⇒ its singleton island is frozen.
        assert!(
            sleep.is_island_frozen(isl0) && !sleep.is_row_awake(0),
            "the undisturbed latched row must stay frozen across the split"
        );
        // Rows 1+2: row 2 is awake (never latched), so their merged island is ACTIVE —
        // no spurious freeze of a partition that gained a moving row.
        assert!(
            isl0 != isl12,
            "the split must put row 0 in a different island from rows 1+2"
        );
        assert!(
            !sleep.is_island_frozen(isl12) && sleep.is_row_awake(1) && sleep.is_row_awake(2),
            "an island that gained an awake row must be ACTIVE (no C3 spurious freeze)"
        );
    }

    /// Sleeping with a threshold of `0` (an island can NEVER drop below it) is
    /// byte-identical to the sleeping-OFF colored solve — nothing ever sleeps, so the
    /// solve + integrate are never skipped (the 0%-gate at the value level).
    #[test]
    fn sleeping_that_never_sleeps_matches_sleeping_off() {
        let make = || {
            vec![
                dyn_sphere(Vec3::new(0.0, 2.0, 0.0), 1.0, 0.5, 0.0),
                dyn_sphere(Vec3::new(0.0, 4.0, 0.0), 1.0, 0.5, 0.0),
                static_body(Vec3::new(0.0, 0.0, 0.0)),
            ]
        };
        // A simple stacking narrowphase: floor contact + sphere-sphere contact.
        let build = |bs: &[BodyState]| {
            let mut ms = Vec::new();
            let y0 = bs[0].position.y;
            if y0 - 1.0 < 0.0 {
                ms.push(manifold(0, 2, Vec3::new(0.0, -1.0, 0.0), y0 - 1.0, Vec3::new(0.0, 1.0, 0.0)));
            }
            let d = bs[1].position.y - bs[0].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(0, 1, Vec3::new(0.0, 1.0, 0.0), d - 2.0, bs[0].position));
            }
            ms
        };

        // Sleeping OFF reference.
        let ys_off = run(make(), build, 30);
        // Sleeping ON but threshold 0 → nothing ever sleeps.
        let (ys_on, sleep) = run_sleeping(make(), build, 30, |c| c.sleep_threshold = 0.0);

        assert!(
            !sleep.is_row_asleep(0) && !sleep.is_row_asleep(1),
            "with threshold 0 no row may latch asleep"
        );
        // Bit-identical (threshold-0 sleeping never skips solve/integrate).
        assert_eq!(
            ys_off.len(),
            ys_on.len(),
            "the two runs must produce the same body count"
        );
        for (off, on) in ys_off.iter().zip(ys_on.iter()) {
            assert_eq!(
                off.to_bits(),
                on.to_bits(),
                "sleeping-ON-but-never-sleeps must be BIT-identical to sleeping-OFF"
            );
        }
    }

    // ── O8 TESTER GATES (the re-review's deferred formal-gate list) ───────────
    //
    // These extend the dev's in-module sanity tests to the FORMAL gates: a larger
    // settled+slept stack hit by a faller (gate 1), rest==rest to ε (gate 2),
    // run-to-run bit-determinism on a sleep+WAKE scene (gate 3), the 0%-gate at
    // SCALE over the O6/O7 random corpus (gate 4), the no-oscillation debounce
    // proptest (gate 5), and topology-churn no-spurious-freeze (gate 7). They reuse
    // the in-module helpers (`dyn_sphere`/`static_body`/`manifold`/`build_graph`/
    // `run`/`run_sleeping`/`random_scene`) and the `#[cfg(test)]` `is_row_asleep`
    // hook, so they run native AND under Miri (no schedule / threadpool).

    use crate::resources::DEFAULT_SLEEP_THRESHOLD;

    /// A full body snapshot (position + rotation + velocities, bit-exact) of every
    /// row — the load-bearing comparand for the determinism / rest-to-ε gates.
    #[derive(Clone, PartialEq)]
    struct Snap {
        position: Vec3,
        rotation: Quat,
        linear_velocity: Vec3,
        angular_velocity: Vec3,
    }

    fn snap(bodies: &[BodyState]) -> Vec<Snap> {
        bodies
            .iter()
            .map(|b| Snap {
                position: b.position,
                rotation: b.rotation,
                linear_velocity: b.linear_velocity,
                angular_velocity: b.angular_velocity,
            })
            .collect()
    }

    /// Bit-exact equality of two snapshots (every f32 component compared by `to_bits`).
    fn snaps_bit_equal(a: &[Snap], b: &[Snap]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b.iter()).all(|(x, y)| {
            let v = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
            let q = |r: Quat| [r.x.to_bits(), r.y.to_bits(), r.z.to_bits(), r.w.to_bits()];
            v(x.position) == v(y.position)
                && q(x.rotation) == q(y.rotation)
                && v(x.linear_velocity) == v(y.linear_velocity)
                && v(x.angular_velocity) == v(y.angular_velocity)
        })
    }

    /// Drives the colored solver with O8 sleeping, returning the FULL final body
    /// snapshot (not just Y). `sleeping` toggles the O8 path; `cfg_mut` tweaks the
    /// rest of the config. Mirrors `run_sleeping` but exposes the whole state so the
    /// determinism / rest-to-ε gates can compare every field, and lets the caller
    /// drop the sleeping flag (for the rest==rest reference arm).
    fn run_snap(
        bodies: Vec<BodyState>,
        build_manifolds: impl Fn(&[BodyState]) -> Vec<Manifold>,
        steps: usize,
        sleeping: bool,
        cfg_mut: impl Fn(&mut PhysicsConfig),
    ) -> Vec<Snap> {
        let mut cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping,
            ..PhysicsConfig::default()
        };
        cfg_mut(&mut cfg);
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(&bodies);
        scratch.touched.reset(scratch.bodies().len());
        let mut sleep = IslandSleep::with_capacity(scratch.bodies().len(), scratch.bodies().len());

        for _ in 0..steps {
            let manifolds = build_manifolds(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            if sleeping {
                solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
            } else {
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        }
        snap(scratch.bodies())
    }

    /// A vertical stack of `n` dynamic spheres (radius 1) resting on a static floor:
    /// centres at y = 1, 3, 5, …; the floor is the last row. Returns the bodies.
    fn vertical_stack(n: usize) -> Vec<BodyState> {
        let mut bodies = Vec::with_capacity(n + 1);
        for i in 0..n {
            bodies.push(dyn_sphere(Vec3::new(0.0, 1.0 + 2.0 * i as f32, 0.0), 1.0, 0.5, 0.0));
        }
        bodies.push(static_body(Vec3::new(0.0, 0.0, 0.0)));
        bodies
    }

    /// The per-step narrowphase stand-in for a vertical stack of `n` dynamic spheres
    /// on a floor (floor is row `n`): floor↔bottom + each adjacent sphere pair, each
    /// emitted only while penetrating (separation < 0).
    fn stack_manifolds(bs: &[BodyState], n: usize) -> Vec<Manifold> {
        let floor = n as u32;
        let mut ms = Vec::new();
        let sep_floor = bs[0].position.y - 1.0;
        if sep_floor < 0.0 {
            ms.push(manifold(0, floor, Vec3::new(0.0, -1.0, 0.0), sep_floor, bs[0].position));
        }
        for i in 0..n.saturating_sub(1) {
            let d = bs[i + 1].position.y - bs[i].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(
                    i as u32,
                    (i + 1) as u32,
                    Vec3::new(0.0, 1.0, 0.0),
                    d - 2.0,
                    bs[i].position,
                ));
            }
        }
        ms
    }

    /// **Gate 1 — a larger settled+slept stack hit by a faller wakes the SAME frame,
    /// the faller does not freeze mid-air, and no body penetrates beyond one
    /// narrowphase margin.** Beyond the dev's 2-sphere test: a 6-sphere stack on a
    /// floor (rows 0..=5, floor row 6, faller row 7). Built from the SAME per-frame
    /// manifolds the solve sees (the honest pipeline). The stack settles + latches,
    /// then a faller is dropped onto the top; the frame its contact appears the whole
    /// merged island is ACTIVE (no frozen row), and no resting body sinks through.
    #[test]
    fn larger_slept_stack_wakes_same_frame_no_penetration() {
        const N: usize = 6;
        let faller = (N + 1) as u32;
        let make = || {
            let mut b = vertical_stack(N); // rows 0..N dyn, row N floor
            b.push(dyn_sphere(Vec3::new(0.0, 60.0, 0.0), 1.0, 0.5, 0.0)); // row N+1 faller
            b
        };
        // Narrowphase: the stack contacts (rows 0..N + floor) plus a top↔faller
        // contact when the faller penetrates the top sphere (row N-1).
        let build = |bs: &[BodyState]| {
            let mut ms = stack_manifolds(bs, N);
            let top = (N - 1) as u32;
            let d = bs[faller as usize].position.y - bs[top as usize].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(top, faller, Vec3::new(0.0, 1.0, 0.0), d - 2.0, bs[top as usize].position));
            }
            ms
        };

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            sleep_frames: 6,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(N + 2);
        scratch.set_bodies(&make());
        // Park the faller (inv_mass 0 = not an island node) so it cannot perturb the
        // pile's settle; un-park it once the pile is asleep.
        {
            let mut __bv = scratch.bodies_mut();
            __bv.as_mut_slice()[faller as usize].inv_mass = 0.0;
        }
        let mut sleep = IslandSleep::with_capacity(N + 2, N + 2);

        for _ in 0..400 {
            let manifolds = build(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        // The whole stack must be latched asleep before the faller arrives.
        for r in 0..N {
            assert!(
                sleep.is_row_asleep(r),
                "stack row {r} must latch asleep before the faller (settle failed)"
            );
        }
        // Resting heights of the slept stack — used to bound penetration after impact.
        let rest_y: Vec<f32> = (0..N).map(|r| scratch.bodies()[r].position.y).collect();

        // Drop the faller onto the top sphere.
        {
            let mut __bv = scratch.bodies_mut();
            __bv.as_mut_slice()[faller as usize].inv_mass = 1.0;
        }
        {
            let mut __bv = scratch.bodies_mut();
            __bv.as_mut_slice()[faller as usize].position.y = 5.0;
        } // just above the top sphere (centre ~11)?
        // Place it a touch above the actual top so the contact appears within a few steps.
        {
            let top_y = scratch.bodies()[N - 1].position.y + 2.5;
            scratch.bodies_mut().as_mut_slice()[faller as usize].position.y = top_y;
        }

        let mut woke = false;
        for _ in 0..40 {
            let manifolds = build(scratch.bodies());
            let faller_contact = manifolds.iter().any(|m| {
                (m.body_a.0 == (N - 1) as u32 && m.body_b.0 == faller)
                    || (m.body_a.0 == faller && m.body_b.0 == (N - 1) as u32)
            });
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);

            if faller_contact {
                // Wake-on-merge: the merged island (stack rows + faller) is ACTIVE the
                // SAME frame — every member row awake, none frozen.
                for r in 0..N {
                    assert!(
                        sleep.is_row_awake(r),
                        "stack row {r} must be ACTIVE the frame the faller's contact appears (wake-on-merge)"
                    );
                }
                assert!(
                    sleep.is_row_awake(faller as usize),
                    "the faller must be awake (it never slept; it must not freeze mid-air)"
                );
                woke = true;
                break;
            }
            // While the faller is still falling (no contact yet) it must NOT be frozen.
            assert!(
                sleep.is_row_awake(faller as usize),
                "the faller must not freeze mid-air before it touches the pile"
            );
        }
        assert!(woke, "the faller must reach the pile within the step budget");

        // Settle the impact and assert no penetration-stick: no resting body sank
        // more than one narrowphase margin (1.0) below its pre-impact rest height, and
        // adjacent spheres keep their ~2.0 centre spacing (no inter-penetration > 1).
        for _ in 0..120 {
            let manifolds = build(scratch.bodies());
            let graph = build_graph(scratch.bodies(), &manifolds);
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
        }
        for (r, &rest) in rest_y.iter().enumerate() {
            assert!(
                scratch.bodies()[r].position.y > rest - 1.0,
                "stack row {r} sank through the pile under impact: y={}, rest was {rest}",
                scratch.bodies()[r].position.y,
            );
        }
        for i in 0..N - 1 {
            let gap = scratch.bodies()[i + 1].position.y - scratch.bodies()[i].position.y;
            assert!(
                gap > 1.0,
                "adjacent stack spheres {i}/{} penetrated > one margin: gap={gap}",
                i + 1
            );
        }
        // The faller came to rest ABOVE the top sphere (did not tunnel through).
        assert!(
            scratch.bodies()[faller as usize].position.y > scratch.bodies()[N - 1].position.y,
            "the faller tunnelled through the pile: faller y={}, top y={}",
            scratch.bodies()[faller as usize].position.y,
            scratch.bodies()[N - 1].position.y
        );
    }

    /// **Gate 2 — a settled stack's resting state with sleeping ON == with sleeping
    /// OFF, to a small ε.** Sleeping must not change the settled configuration, only
    /// stop integrating it. A 4-sphere stack settles for many frames under both
    /// configs; the final positions must match within ε (the slept arm freezes the
    /// converged rest pose; the awake arm keeps micro-integrating it — they agree to ε).
    #[test]
    fn rest_state_with_sleeping_equals_without_to_epsilon() {
        const N: usize = 4;
        let make = || vertical_stack(N);
        let build = move |bs: &[BodyState]| stack_manifolds(bs, N);
        // Use the default debounce-friendly threshold so the slept arm actually sleeps.
        let off = run_snap(make(), build, 600, false, |c| c.sleep_frames = 30);
        let on = run_snap(make(), build, 600, true, |c| c.sleep_frames = 30);

        const EPS: f32 = 1.0e-2;
        for r in 0..N {
            let dy = (on[r].position.y - off[r].position.y).abs();
            assert!(
                dy < EPS,
                "row {r} rest Y differs sleeping ON vs OFF beyond ε: on={}, off={}, |Δ|={dy}",
                on[r].position.y,
                off[r].position.y
            );
        }
    }

    /// **Gate 3 — run-to-run BIT-determinism on a sleep+WAKE scene.** A scene that
    /// settles → sleeps → is woken by a faller → re-settles, run N independent times,
    /// must produce bit-identical final body snapshots. The in-module tests do not
    /// loop runs — this is the load-bearing determinism gate (every f32 by `to_bits`).
    #[test]
    fn sleep_then_wake_scene_is_run_to_run_bit_deterministic() {
        const N: usize = 4;
        let floor = N as u32;
        let faller = (N + 1) as u32;
        let make = || {
            let mut b = vertical_stack(N);
            b.push(dyn_sphere(Vec3::new(0.0, 40.0, 0.0), 1.0, 0.5, 0.0)); // faller
            b
        };
        let build = move |bs: &[BodyState]| {
            let mut ms = stack_manifolds(bs, N);
            let top = (N - 1) as u32;
            let d = bs[faller as usize].position.y - bs[top as usize].position.y;
            if d - 2.0 < 0.0 {
                ms.push(manifold(top, faller, Vec3::new(0.0, 1.0, 0.0), d - 2.0, bs[top as usize].position));
            }
            let _ = floor;
            ms
        };

        // One full sleep+wake trajectory: settle (faller parked) → drop faller →
        // re-settle. Returns the final snapshot.
        let trajectory = || {
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                sleeping: true,
                sleep_frames: 6,
                ..PhysicsConfig::default()
            };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(N + 2);
            scratch.set_bodies(&make());
            {
                let mut __bv = scratch.bodies_mut();
                __bv.as_mut_slice()[faller as usize].inv_mass = 0.0;
            }
            let mut sleep = IslandSleep::with_capacity(N + 2, N + 2);
            // settle phase
            for _ in 0..200 {
                let manifolds = build(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
            }
            // wake phase: drop the faller
            {
                let mut __bv = scratch.bodies_mut();
                __bv.as_mut_slice()[faller as usize].inv_mass = 1.0;
            }
            {
            let top_y = scratch.bodies()[N - 1].position.y + 2.5;
            scratch.bodies_mut().as_mut_slice()[faller as usize].position.y = top_y;
        }
            for _ in 0..200 {
                let manifolds = build(scratch.bodies());
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored_sleeping(&cfg, &manifolds, &graph, &mut scratch, &mut sleep);
            }
            snap(scratch.bodies())
        };

        let baseline = trajectory();
        for run_idx in 1..8 {
            let again = trajectory();
            assert!(
                snaps_bit_equal(&baseline, &again),
                "sleep+wake scene was NOT run-to-run bit-deterministic on run {run_idx}"
            );
        }
    }

    /// **Gate 4 — the 0%-gate at SCALE.** With sleeping=false the colored solve must
    /// be BYTE-identical to the pre-O8 colored path (`solve_colored` / `build_columns(None)`)
    /// across the O6/O7 random-scene corpus, not just a 3-body scene. Here: drive each
    /// random scene through `solve_colored_inner(.., None)` (the live path) vs the
    /// explicit `solve_colored` entry; both must produce bit-identical body state. The
    /// stronger claim — sleeping=ON-but-never-sleeps == sleeping=OFF — is also checked
    /// per scene (threshold 0 ⇒ no freeze, so the O8 path must be byte-identical).
    #[test]
    fn zero_gate_at_scale_sleeping_off_byte_identical_on_random_corpus() {
        proptest!(ProptestConfig::with_cases(300), |(seed in any::<u64>())| {
            let (bodies, manifolds, graph) = random_scene(seed);
            let cfg = PhysicsConfig {
                dt: 1.0 / 60.0,
                ..PhysicsConfig::default()
            };

            // Arm A: the byte-untouched O6/O7 path (sleep == None).
            let mut solver_a = ColoredSoftStepSolver::default();
            let mut scratch_a = SolverScratch::with_capacity(bodies.len());
            scratch_a.set_bodies(&bodies);
            scratch_a.touched.reset(scratch_a.bodies().len());
            solver_a.solve_colored(&cfg, &manifolds, &graph, &mut scratch_a);
            let after_off = snap(scratch_a.bodies());

            // Arm B: the O8 path with threshold 0 (nothing can sleep ⇒ no freeze) —
            // must be byte-identical to arm A (sleeping bookkeeping changes nothing).
            let cfg_on = PhysicsConfig {
                sleeping: true,
                sleep_threshold: 0.0,
                ..cfg
            };
            let mut solver_b = ColoredSoftStepSolver::default();
            let mut scratch_b = SolverScratch::with_capacity(bodies.len());
            scratch_b.set_bodies(&bodies);
            scratch_b.touched.reset(scratch_b.bodies().len());
            let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
            solver_b.solve_colored_sleeping(&cfg_on, &manifolds, &graph, &mut scratch_b, &mut sleep);
            let after_on = snap(scratch_b.bodies());

            prop_assert!(
                snaps_bit_equal(&after_off, &after_on),
                "0%-gate at scale FAILED for seed {}: sleeping-off result != sleeping-on-but-never-sleeps",
                seed
            );
        });
    }

    /// **Gate 5 — no wake/sleep oscillation.** A body hovering near the threshold must
    /// not flap asleep/awake every frame; the integer debounce must hold. A proptest
    /// over near-threshold per-island energies: drive `begin_step`/`end_step` directly
    /// with a synthetic body velocity sampled around `sleep_threshold` and count latch
    /// TRANSITIONS over many frames — the count must be bounded (no per-frame flapping).
    #[test]
    fn no_sleep_wake_oscillation_near_threshold() {
        proptest!(ProptestConfig::with_cases(200), |(
            speed_bits in 0u32..=40u32,    // index into a near-threshold speed table
            frames_seed in any::<u64>(),
        )| {
            // A single dynamic body, no contacts, in its own singleton island so its
            // island energy is exactly its own |v|².
            let threshold = DEFAULT_SLEEP_THRESHOLD; // 1e-4
            let debounce: u16 = 8;
            // A speed² straddling the threshold: below for `speed_bits` even, above for
            // odd — deterministically alternating around the boundary to bait flapping.
            let base = threshold * 0.5; // safely below
            let above = threshold * 2.0; // safely above
            let mut rng = Lcg(frames_seed ^ (speed_bits as u64));

            let body = dyn_sphere(Vec3::new(0.0, 5.0, 0.0), 1.0, 0.5, 0.0);
            // Single-row graph: a manifold to a (added) static floor so the dyn body
            // forms an island. Use a 2-body world (dyn + static) and one contact.
            let bodies = vec![body, static_body(Vec3::ZERO)];
            let ms = vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.01, Vec3::ZERO)];
            let graph = build_graph(&bodies, &ms);
            let mut sleep = IslandSleep::with_capacity(2, 2);

            // Manually feed end_step a body whose speed² we control, then begin_step,
            // and count how many times the row's latch CHANGES state across frames.
            let mut bs = bodies.clone();
            let mut transitions = 0usize;
            let mut prev_asleep = false;
            for f in 0..200 {
                sleep.begin_step(&graph, bs.len());
                // Choose this frame's speed: a low-bias random walk that mostly stays
                // below threshold but occasionally pops above (the near-threshold case).
                let pop = rng.f01() < 0.15; // 15% of frames spike above threshold
                let v2 = if pop { above } else { base * rng.f01().max(0.01) };
                let speed = v2.sqrt();
                bs[0].linear_velocity = Vec3::new(speed, 0.0, 0.0);
                bs[0].angular_velocity = Vec3::ZERO;
                sleep.end_step(&bs, &graph, threshold, debounce);
                let now = sleep.is_row_asleep(0);
                if f > 0 && now != prev_asleep {
                    transitions += 1;
                }
                prev_asleep = now;
            }
            // With a debounce of 8 frames a body cannot flap each frame: every
            // sleep→wake costs 1 frame (an above-threshold spike) and every wake→sleep
            // costs ≥ debounce frames. Over 200 frames with ~15% spikes the transition
            // count must be far below the no-debounce worst case (~200). A debounce that
            // works keeps it bounded by roughly 2× the number of spike clusters.
            prop_assert!(
                transitions <= 60,
                "near-threshold latch oscillated {} times over 200 frames (debounce broken)",
                transitions
            );
        });
    }

    /// **Gate 5b — a body steadily AT rest (just below threshold every frame) latches
    /// exactly once and never flaps.** The clean no-oscillation case: 0 transitions
    /// after the single sleep latch.
    #[test]
    fn steady_below_threshold_latches_once_no_flap() {
        let bodies = vec![dyn_sphere(Vec3::new(0.0, 5.0, 0.0), 1.0, 0.5, 0.0), static_body(Vec3::ZERO)];
        let ms = vec![manifold(0, 1, Vec3::new(0.0, -1.0, 0.0), -0.01, Vec3::ZERO)];
        let graph = build_graph(&bodies, &ms);
        let mut sleep = IslandSleep::with_capacity(2, 2);
        let threshold = DEFAULT_SLEEP_THRESHOLD;
        let debounce: u16 = 8;

        let mut bs = bodies.clone();
        bs[0].linear_velocity = Vec3::ZERO; // exactly at rest, always below threshold
        let mut transitions = 0usize;
        let mut prev = false;
        for f in 0..100 {
            sleep.begin_step(&graph, bs.len());
            sleep.end_step(&bs, &graph, threshold, debounce);
            let now = sleep.is_row_asleep(0);
            if f > 0 && now != prev {
                transitions += 1;
            }
            prev = now;
        }
        assert_eq!(transitions, 1, "a steadily-resting body must latch asleep exactly ONCE (no flap)");
        assert!(sleep.is_row_asleep(0), "the resting body must end latched asleep");
    }

    /// **Gate 9 probe — does a dense resting pile actually latch asleep, and after
    /// how many frames?** This mirrors the criterion `sleeping` bench's `pile_scene`
    /// (a grid of sphere columns on a floor with vertical + lateral contacts) and
    /// reports the slept-row fraction over time. If a dense, lateral-contact pile
    /// does NOT sleep, the bench's `mostly_settled_sleeping_on` arm measures the
    /// AWAKE path (no skip) and the headline-win claim is vacuous — so this is the
    /// load-bearing diagnostic behind the criterion result.
    #[test]
    fn dense_resting_pile_sleeps_diagnostic() {
        // A small pile (4 columns × 3 high) — the chromatic shape of the bench scene
        // at a Miri/native-cheap size.
        let n_columns = 4u32;
        let height = 3u32;
        let n_dyn = (n_columns * height) as usize;
        let mut bodies: Vec<BodyState> = Vec::with_capacity(n_dyn + 1);
        for col in 0..n_columns {
            for h in 0..height {
                let x = col as f32 * 1.05;
                let y = 0.5 + h as f32 * 0.99;
                bodies.push(dyn_sphere(Vec3::new(x, y, 0.0), 1.0, 0.5, 0.0));
            }
        }
        let floor_row = n_dyn as u32;
        bodies.push(static_body(Vec3::new(0.0, -50.0, 0.0)));

        // The bench's FIXED-anchor manifolds: contacts are re-emitted every step at the
        // ORIGINAL rest anchors regardless of how the bodies move (the bench reuses one
        // prebuilt manifold set + graph — it does NOT re-run narrowphase).
        let row_of = |col: u32, h: u32| col * height + h;
        let mut fixed = Vec::new();
        for col in 0..n_columns {
            for h in 0..height {
                let r = row_of(col, h);
                if h == 0 {
                    fixed.push(manifold(r, floor_row, Vec3::new(0.0, -1.0, 0.0), -0.001, bodies[r as usize].position));
                } else {
                    let below = row_of(col, h - 1);
                    fixed.push(manifold(below, r, Vec3::new(0.0, 1.0, 0.0), -0.001, bodies[r as usize].position));
                }
                if col + 1 < n_columns {
                    let right = row_of(col + 1, h);
                    fixed.push(manifold(r, right, Vec3::new(1.0, 0.0, 0.0), -0.001, bodies[r as usize].position));
                }
            }
        }
        let graph = build_graph(&bodies, &fixed);

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            sleeping: true,
            sleep_frames: 4,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(bodies.len());
        scratch.set_bodies(&bodies);
        let mut sleep = IslandSleep::with_capacity(scratch.bodies().len(), scratch.bodies().len());

        let mut first_all_asleep = None;
        for frame in 0..400 {
            scratch.touched.reset(scratch.bodies().len());
            solver.solve_colored_sleeping(&cfg, &fixed, &graph, &mut scratch, &mut sleep);
            let asleep = (0..n_dyn).filter(|&r| sleep.is_row_asleep(r)).count();
            if asleep == n_dyn && first_all_asleep.is_none() {
                first_all_asleep = Some(frame);
            }
        }
        let asleep_final = (0..n_dyn).filter(|&r| sleep.is_row_asleep(r)).count();
        eprintln!(
            "dense_pile_diagnostic: {asleep_final}/{n_dyn} rows asleep after 400 frames; \
             first all-asleep frame = {first_all_asleep:?}"
        );
        // The diagnostic gate: a dense resting pile MUST eventually sleep, else the
        // bench measures the awake path. (If this fails, the criterion headline-win
        // arm is vacuous — report the slept-row count.)
        assert_eq!(
            asleep_final, n_dyn,
            "a dense resting pile did not fully sleep ({asleep_final}/{n_dyn}); the criterion \
             mostly-settled arm would measure the AWAKE path"
        );
    }

    /// **Gate 7 — topology-change no-spurious-freeze (the C3 regression gate, at
    /// scale).** Random merge/split sequences: a body that should be active is never
    /// frozen because of a stale latch. The row-keyed latch must survive island
    /// renumbering. Drives `begin_step` over a sequence of random manifold sets over a
    /// fixed body set, latching/waking rows, and asserts the freeze decision is ALWAYS
    /// a pure function of the per-row latch — an island is frozen IFF every member row
    /// is latched, never otherwise.
    #[test]
    fn topology_churn_freeze_is_pure_function_of_row_latch() {
        proptest!(ProptestConfig::with_cases(300), |(seed in any::<u64>())| {
            let mut rng = Lcg(seed ^ 0x5DEE_CE66_D1CE_4B27);
            // A fixed set of dynamic bodies that we re-island with random manifolds.
            let n_dyn = rng.range(2, 9) as usize; // 2..=8 dynamic bodies
            let mut bodies: Vec<BodyState> = (0..n_dyn)
                .map(|i| dyn_sphere(Vec3::new(i as f32 * 0.3, 1.0, 0.0), 1.0, 0.5, 0.0))
                .collect();
            bodies.push(static_body(Vec3::ZERO));
            let n_rows = bodies.len();

            let mut sleep = IslandSleep::with_capacity(n_rows, n_rows);

            // Run several frames; each frame re-derive a random manifold set (random
            // merges/splits), randomly latch/wake rows via the energy path, then assert
            // the per-island freeze decision matches the pure predicate over the rows.
            for _frame in 0..20 {
                // Random contact set over the dynamic bodies (random merges/splits).
                let n_contacts = rng.range(0, (n_dyn * 2) as u32) as usize;
                let mut ms = Vec::with_capacity(n_contacts);
                for _ in 0..n_contacts {
                    let a = rng.range(0, n_dyn as u32);
                    let mut b = rng.range(0, n_dyn as u32);
                    if b == a {
                        b = (a + 1) % n_dyn as u32;
                    }
                    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                    ms.push(manifold(lo, hi, Vec3::new(1.0, 0.0, 0.0), -0.01, bodies[lo as usize].position));
                }
                let graph = build_graph(&bodies, &ms);
                sleep.begin_step(&graph, n_rows);

                // The per-island freeze decision must be EXACTLY: island frozen iff
                // every member dynamic row is latched asleep (and the island is non-empty).
                let n_islands = graph.n_islands() as usize;
                let mut member_count = vec![0usize; n_islands];
                let mut all_asleep = vec![true; n_islands];
                for row in 0..n_dyn {
                    let isl = graph.island_of(row as u32);
                    if isl == ConstraintGraph::NO_ISLAND {
                        continue;
                    }
                    member_count[isl as usize] += 1;
                    if !sleep.is_row_asleep(row) {
                        all_asleep[isl as usize] = false;
                    }
                }
                for isl in 0..n_islands {
                    let expect_frozen = member_count[isl] > 0 && all_asleep[isl];
                    // An island with no members is `frozen_islands[isl] == true` by the
                    // resize default but has no rows, so no row reports awake/frozen via it.
                    if member_count[isl] > 0 {
                        prop_assert_eq!(
                            sleep.is_island_frozen(isl as u32),
                            expect_frozen,
                            "spurious/missing freeze for island {} on frame {} (seed {}): \
                             members={}, all_asleep={}",
                            isl, _frame, seed, member_count[isl], all_asleep[isl]
                        );
                    }
                    // Every awake row's island must NOT be frozen (C3): no member of an
                    // active partition is frozen because of a stale latch.
                    for row in 0..n_dyn {
                        if graph.island_of(row as u32) == isl as u32 && !sleep.is_row_asleep(row) {
                            prop_assert!(
                                sleep.is_row_awake(row),
                                "C3: awake row {} was frozen by a stale latch (seed {}, frame {})",
                                row, seed, _frame
                            );
                        }
                    }
                }

                // Randomly latch / wake some rows for the next frame (drive churn).
                for row in 0..n_dyn {
                    if rng.f01() < 0.5 {
                        sleep.force_sleep_row(row);
                    }
                }
            }
        });
    }
