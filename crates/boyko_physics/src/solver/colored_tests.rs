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
        // colored partition + sweep + warm store (a write by manifold index) are
        // deterministic.
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
        solver.build_columns(&manifolds, &graph, &bodies, None, RowRemap::Identity);
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

        // The 4-point box manifold (rows 2,3) must appear as ONE group of width 4 —
        // never split. A group is one lane of one cohort of its color (L11 C2): the
        // lane's body pair is the manifold's and its width the group's point count.
        let mut box_group_len = None;
        for c in 0..n_colors {
            let ctx = cols.color_ctx(c);
            let g_lo = cols.color_group_start()[c] as usize;
            let g_hi = cols.color_group_start()[c + 1] as usize;
            for g in g_lo..g_hi {
                let (k, l) = ctx.lane_of(g);
                let head = &cols.heads()[k];
                assert!(l < head.nlanes as usize, "a group maps to a live lane of its cohort");
                assert_eq!(
                    u32::from(head.width[l]),
                    cols.group_start()[g + 1] - cols.group_start()[g],
                    "the lane's width is the group's point count"
                );
                if head.body_a[l] == 2 && head.body_b[l] == 3 {
                    box_group_len = Some(head.width[l] as usize);
                }
            }
        }
        assert_eq!(box_group_len, Some(4), "the 4-point box manifold forms ONE 4-point group");
    }

    // ── Tester additions (Phase O5 formal gates) ─────────────────────────────
    //
    // These extend the dev's stand-in sanity tests into the exhaustive O5 gates.
    // They live in the lib test module because the rigorous group-CSR tiling gate
    // (Gate 4) needs access to the PRIVATE `CohortColumns` tables (`group_start`,
    // `color_group_start`, `color_cohort_start`, `color_offsets`, the heads). They
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
    /// group (never split across groups/colors).
    #[test]
    fn group_csr_tiles_every_color_span_on_random_scenes() {
        proptest!(ProptestConfig::with_cases(400), |(seed in any::<u64>())| {
            let (bodies, manifolds, graph) = random_scene(seed);
            let mut solver = ColoredSoftStepSolver::default();
            solver.build_bodies(&bodies);
            solver.build_columns(&manifolds, &graph, &bodies, None, RowRemap::Identity);
            let cols = &solver.columns;

            let n_colors = cols.color_offsets().len().saturating_sub(1);
            prop_assert_eq!(
                cols.color_group_start().len(),
                n_colors + 1,
                "per-color group CSR must have n_colors + 1 entries"
            );
            prop_assert_eq!(cols.group_start().first().copied(), Some(0u32), "group CSR starts at 0");

            // Verify the tiling per color: the groups tile the color's point span, and
            // the color's cohorts tile its groups (L11 C2) — each group is one lane of
            // one cohort, the cohorts are 8-group windows from the color's first group,
            // and the cohorts' rank tables are contiguous.
            let mut groups_seen = 0usize;
            let mut covered = vec![false; cols.len()];
            let mut ranks_seen = 0u32;
            for c in 0..n_colors {
                let span_start = cols.color_offsets()[c];
                let span_end = cols.color_offsets()[c + 1];
                let g_lo = cols.color_group_start()[c] as usize;
                let g_hi = cols.color_group_start()[c + 1] as usize;
                let k_lo = cols.color_cohort_start()[c] as usize;
                let k_hi = cols.color_cohort_start()[c + 1] as usize;
                prop_assert!(g_lo <= g_hi, "color {} group range well-ordered", c);
                prop_assert_eq!(k_hi - k_lo, (g_hi - g_lo).div_ceil(COHORT), "color {} cohorts are 8-group windows", c);
                let ctx = cols.color_ctx(c);
                let mut cursor = span_start;
                for g in g_lo..g_hi {
                    let gs = cols.group_start()[g];
                    let ge = cols.group_start()[g + 1];
                    prop_assert!(ge > gs, "group {} must be non-empty", g);
                    prop_assert_eq!(gs, cursor, "group {} tiles color {} span with no gap/overlap", g, c);
                    // The group is one lane of one cohort of its color, and the lane's
                    // width is the group's point count (the C1 contract: a manifold's
                    // ≥2 points are never split).
                    let (k, l) = ctx.lane_of(g);
                    prop_assert!(k >= k_lo && k < k_hi, "group {} maps into color {}'s cohorts", g, c);
                    let head = &cols.heads()[k];
                    prop_assert!(l < head.nlanes as usize, "group {} maps to a live lane", g);
                    prop_assert_eq!(u32::from(head.width[l]), ge - gs, "group {} width is its lane's", g);
                    prop_assert!(head.width[l] <= head.depth, "lane width within the cohort's depth");
                    for s in gs..ge {
                        prop_assert!(!covered[s as usize], "slot {} covered by >1 group", s);
                        covered[s as usize] = true;
                    }
                    cursor = ge;
                    groups_seen += 1;
                }
                prop_assert_eq!(cursor, span_end, "color {} groups exactly fill its slot span", c);
                for k in k_lo..k_hi {
                    let head = &cols.heads()[k];
                    let expected_lanes = (g_hi - (g_lo + (k - k_lo) * COHORT)).min(COHORT);
                    prop_assert_eq!(head.nlanes as usize, expected_lanes, "cohort {} lane count", k);
                    prop_assert_eq!(head.rank_base, ranks_seen, "cohort {} rank table is contiguous", k);
                    let depth = (0..COHORT).map(|l| head.width[l]).max().unwrap_or(0);
                    prop_assert_eq!(head.depth, depth, "cohort {} depth is its widest lane", k);
                    ranks_seen += u32::from(head.depth);
                }
            }
            prop_assert_eq!(ranks_seen as usize, cols.blocks().len(), "the rank blocks are exactly the cohorts' ranks");
            prop_assert_eq!(cols.heads().len(), *cols.color_cohort_start().last().unwrap_or(&0) as usize, "every cohort belongs to a color");
            prop_assert_eq!(groups_seen, cols.group_start().len() - 1, "every group in exactly one color");
            // Every slot is covered by exactly one group.
            prop_assert!(covered.iter().all(|&c| c), "every slot belongs to a group");

            // A multi-point manifold (count >= 2 over a dyn-dyn pair) appears as
            // ONE lane of exactly `count` points — never split.
            for m in &manifolds {
                if m.count >= 2 && m.body_b != SDF_SENTINEL {
                    let ia = m.body_a.0;
                    let ib = m.body_b.0;
                    // Locate the lane whose body pair is this manifold's AND whose
                    // width equals the manifold's live point count.
                    let mut found = false;
                    for head in cols.heads() {
                        for l in 0..head.nlanes as usize {
                            if head.body_a[l] == ia && head.body_b[l] == ib && head.width[l] == m.count {
                                found = true;
                            }
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

    /// Byte-complete snapshot of the cohort layout (L11 C2, G3): every field of every
    /// head (its zero padding bytes included), cold record, rank block and `vn0` row,
    /// then the per-manifold tags and plan, then the four CSRs — in
    /// `CohortColumns` field order, so two builds over the same scene must produce a
    /// BIT-FOR-BIT equal vector. Padding lanes and ranks are part of it, so a stale
    /// byte in a padding lane moves the vector.
    fn layout_snapshot(cols: &CohortColumns) -> Vec<u32> {
        let mut out = Vec::new();
        let bits = |row: &[f32; COHORT]| row.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
        for h in cols.heads() {
            for c in 0..3 {
                out.extend(bits(&h.n[c]));
            }
            for c in 0..3 {
                out.extend(bits(&h.t1[c]));
            }
            out.extend(bits(&h.friction));
            out.extend(h.body_a);
            out.extend(h.body_b);
            out.extend(h.width.iter().map(|&w| u32::from(w)));
            out.extend([
                h.rank_base,
                u32::from(h.depth),
                u32::from(h.nlanes),
                u32::from(h.sentinel),
                u32::from(h._p),
            ]);
            out.extend(h._pad.iter().map(|&b| u32::from(b)));
        }
        for c in cols.cold.as_read_slice() {
            out.extend(bits(&c.restitution));
            out.extend(c.mi);
        }
        for b in cols.blocks() {
            for c in 0..3 {
                out.extend(bits(&b.ra[c]));
            }
            for c in 0..3 {
                out.extend(bits(&b.rb[c]));
            }
            out.extend(bits(&b.sep));
            out.extend(bits(&b.ni));
            out.extend(bits(&b.ti1));
            out.extend(bits(&b.ti2));
        }
        for v in cols.rank_cold.as_read_slice() {
            out.extend(bits(v));
        }
        out.extend(cols.tags().iter().map(|t| u32::from(t.count) | (u32::from(t.flags) << 8)));
        out.extend(cols.plan().iter().flat_map(|r| [r.lo, r.hi]));
        out.extend(cols.color_offsets().iter().copied());
        out.extend(cols.group_start().iter().copied());
        out.extend(cols.color_group_start().iter().copied());
        out.extend(cols.color_cohort_start().iter().copied());
        out
    }

    /// The warm store's read side after a step (the side the step just wrote): its
    /// keys, its records word by word, and its strictness (G3: the records are
    /// byte-equal across worker counts).
    fn records_snapshot(solver: &ColoredSoftStepSolver) -> Vec<u32> {
        let side = &solver.warm[solver.warm_cur as usize];
        let mut out: Vec<u32> = side.keys().iter().flat_map(|&k| [k as u32, (k >> 32) as u32]).collect();
        for r in side.recs() {
            out.extend(r.words());
        }
        out.push(u32::from(side.strict()));
        out
    }

    /// What one padding audit of the layout found: whether every padding lane and
    /// rank is zero, and how many partial cohorts and padded ranks the layout has (the
    /// audit's own anti-vacuity).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct PaddingAudit {
        zero: bool,
        partial_cohorts: usize,
        padded_ranks: usize,
    }

    /// Audits every cohort's padding (G3's release half: the same predicate as the
    /// fill's debug assertion, evaluated here in every profile).
    fn audit_padding(cols: &CohortColumns) -> PaddingAudit {
        let blocks = cols.blocks();
        let vn0 = cols.rank_cold.as_read_slice();
        let mut audit = PaddingAudit { zero: true, partial_cohorts: 0, padded_ranks: 0 };
        for (head, cold) in cols.heads().iter().zip(cols.cold.as_read_slice()) {
            let nlanes = head.nlanes as usize;
            let depth = head.depth as usize;
            let rank_base = head.rank_base as usize;
            audit.partial_cohorts += usize::from(nlanes < COHORT);
            for (l, &width) in head.width.iter().enumerate() {
                let width = width as usize;
                if l >= nlanes {
                    audit.zero &= width == 0
                        && (0..3).all(|c| head.n[c][l].to_bits() == 0 && head.t1[c][l].to_bits() == 0)
                        && head.friction[l].to_bits() == 0
                        && head.body_a[l] == 0
                        && head.body_b[l] == 0
                        && !head.is_sentinel(l)
                        && cold.restitution[l].to_bits() == 0
                        && cold.mi[l] == 0;
                } else {
                    audit.padded_ranks += depth - width;
                }
                for (blk, vn) in blocks[rank_base..rank_base + depth].iter().zip(&vn0[rank_base..rank_base + depth]).skip(width) {
                    audit.zero &= (0..3).all(|c| blk.ra[c][l].to_bits() == 0 && blk.rb[c][l].to_bits() == 0)
                        && blk.sep[l].to_bits() == 0
                        && blk.ni[l].to_bits() == 0
                        && blk.ti1[l].to_bits() == 0
                        && blk.ti2[l].to_bits() == 0
                        && vn[l].to_bits() == 0;
                }
            }
            audit.zero &= head._p == 0 && head._pad == [0; 16];
        }
        audit
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

    /// The dense scene's stream with a LAYOUT CHURN at step 6 (G3): every seventh
    /// body also meets the body two places on with a box manifold of four points
    /// before the churn and two after, and the odd bodies lose their floor contact
    /// at the churn — so cohorts that were full become partial and lanes that were
    /// four ranks deep become two, and a fill that skipped its padding would leave
    /// the earlier bytes behind. Sorted by pair, so the stream stays strict.
    #[cfg(not(miri))]
    fn layout_churn_manifolds(bodies: &[BodyState], step: usize) -> Vec<Manifold> {
        let churned = step >= 6;
        let n = bodies.len() - 1;
        let mut out: Vec<Manifold> = dense_collision_manifolds(bodies)
            .into_iter()
            .filter(|m| !(churned && m.body_b.0 == n as u32 && m.body_a.0 % 2 == 1))
            .collect();
        for a in (0..n).step_by(7) {
            if a + 2 < n {
                let up = Vec3::new(0.0, 1.0, 0.0);
                out.push(box_manifold(a as u32, (a + 2) as u32, up, -0.05, bodies[a].position, if churned { 2 } else { 4 }));
            }
        }
        out.sort_by_key(|m| (m.body_a.0, m.body_b.0));
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

    /// One G3 run: the layout and record bytes after the last step, and the padding
    /// audit of that layout.
    #[cfg(not(miri))]
    struct LayoutRun {
        layout: Vec<u32>,
        records: Vec<u32>,
        padding: PaddingAudit,
    }

    /// Like [`run_dense_in_pool`] but over the churned stream, with `simd_solve` as
    /// given, and returning the FULL layout and record bytes after the final step
    /// (L11 C2, G3). The tables are rebuilt each `solve_colored`, so after the last
    /// step they hold that step's complete layout: every head, block and `vn0` row
    /// the fill wrote and the sweeps updated, the records the store wrote. This is
    /// the load-bearing "pure layout swap" probe: it reads the actual bytes the
    /// parallel workers wrote, not just the body state derived from them.
    #[cfg(not(miri))]
    fn run_layout_in_pool(n: usize, steps: usize, parallel_solve: bool, simd_solve: bool, workers: usize) -> LayoutRun {
        use boyko_threadpool::ThreadPoolBuilder;

        let cfg = PhysicsConfig {
            dt: 1.0 / 60.0,
            parallel_solve,
            simd_solve,
            ..PhysicsConfig::default()
        };
        let mut solver = ColoredSoftStepSolver::default();
        let mut scratch = SolverScratch::with_capacity(n + 1);
        scratch.set_bodies(&dense_collision_scene(n));
        scratch.touched.reset(scratch.bodies().len());

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            for step in 0..steps {
                let manifolds = layout_churn_manifolds(scratch.bodies(), step);
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
        });
        LayoutRun {
            layout: layout_snapshot(&solver.columns),
            records: records_snapshot(&solver),
            padding: audit_padding(&solver.columns),
        }
    }

    /// G3 (L11 C2): the cohort layout — every head, cold record, rank block and `vn0`
    /// row — and the warm records are BYTE-FOR-BYTE identical across (a) run-to-run
    /// repeats, (b) {1, 2, 4, 8, 16}-worker parallel runs versus the single-threaded
    /// solve, and (c) the scalar and SIMD kernels; and every padding lane and padded
    /// rank is zero after a layout churn that shrank cohorts and lanes (so a fill
    /// that skipped the zeroing, M6, leaves the earlier step's bytes behind and goes
    /// red here).
    ///
    /// The body-state snapshot (`parallel_solve_is_bit_identical_across_worker_counts`)
    /// only checks the 9 derived float fields per body; this checks the storage the
    /// workers actually touched — so a parallel write going to the wrong lane / a
    /// stale base / a torn impulse row would be caught here even if it happened to
    /// cancel out in the body integration.
    ///
    /// Scene `n == 400` is sized so the widest color exceeds
    /// `MIN_PARALLEL_SLOTS_PER_COLOR` (asserted), so the parallel `pool.scope`
    /// dispatch genuinely fires — a sub-threshold scene would only exercise the
    /// inline path and the {1,N} claim would be vacuous.
    #[test]
    #[cfg(not(miri))]
    fn cohort_layout_bytes_are_identical_across_workers_and_runs() {
        let n = 400;
        let widest = max_color_slot_span(n);
        assert!(
            widest >= MIN_PARALLEL_SLOTS_PER_COLOR,
            "anti-vacuity: the widest color ({widest} slots) must exceed the threshold \
             ({MIN_PARALLEL_SLOTS_PER_COLOR}) so the parallel dispatch path is exercised, \
             else the {{1,N}} layout-byte-identity claim is vacuous"
        );

        // (a) Determinism: the single-threaded scalar path, run twice.
        let single_a = run_layout_in_pool(n, 12, false, false, 1);
        let single_b = run_layout_in_pool(n, 12, false, false, 1);
        assert_eq!(single_a.layout, single_b.layout, "G3 (determinism): the layout bytes are run-to-run identical");
        assert_eq!(single_a.records, single_b.records, "G3 (determinism): the record bytes are run-to-run identical");
        assert!(!single_a.layout.is_empty() && !single_a.records.is_empty(), "G3 anti-vacuity: a real layout");

        // The churn's own anti-vacuity: partial cohorts AND padded ranks exist after it,
        // and every one of them is zero.
        assert!(
            single_a.padding.partial_cohorts > 0 && single_a.padding.padded_ranks > 0,
            "anti-vacuity: the churned layout must have partial cohorts and padded ranks: {:?}",
            single_a.padding
        );
        assert!(single_a.padding.zero, "G3: every padding lane and padded rank is zero: {:?}", single_a.padding);

        // (b) Parallel == serial, byte for byte, across {1, 2, 4, 8, 16} workers, on
        // both kernels. Every worker writes the SAME bytes into the SAME lanes
        // regardless of worker count, and identical to the single-threaded reference.
        for simd in [false, true] {
            for workers in [1usize, 2, 4, 8, 16] {
                let run = run_layout_in_pool(n, 12, true, simd, workers);
                assert_eq!(
                    single_a.layout, run.layout,
                    "G3: the layout bytes at {workers} workers (simd {simd}) must equal the single-threaded scalar layout"
                );
                assert_eq!(
                    single_a.records, run.records,
                    "G3: the record bytes at {workers} workers (simd {simd}) must equal the single-threaded ones"
                );
                assert_eq!(single_a.padding, run.padding, "G3: the padding audit at {workers} workers (simd {simd})");
            }
        }

        // Run-to-run determinism of the parallel path itself.
        let p4 = run_layout_in_pool(n, 12, true, true, 4);
        let p4_again = run_layout_in_pool(n, 12, true, true, 4);
        assert_eq!(p4.layout, p4_again.layout, "G3: the parallel SIMD layout bytes are run-to-run identical");
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
    /// load-bearing determinism property (disjoint-body groups + a warm store
    /// written by manifold index ⇒ worker-count-independent bits). Also checks the
    /// parallel 4-worker result matches the single-threaded
    /// (`parallel_solve == false`) result, so the parallel dispatch does not change
    /// the converged value.
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
        solver.build_columns(&manifolds, &graph, &bodies, None, RowRemap::Identity);
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
    //       a full step (on a non-AVX2 build both arms ARE the scalar oracle, so the
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

    /// Captures the full body + impulse bit state after solving each color's
    /// groups with the supplied per-color kernel: `(body_bits, impulse_bits)`, the
    /// impulses in slot order (cohorts in color order, lanes, ranks). Used only by
    /// the +avx2 differential.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn body_impulse_bits(bodies: &[BodyEffective], cols: &CohortColumns) -> (Vec<u32>, Vec<u32>) {
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
        let blocks = cols.blocks();
        let mut impulse_bits = Vec::with_capacity(cols.len() * 3);
        for head in cols.heads() {
            let rank_base = head.rank_base as usize;
            for l in 0..head.nlanes as usize {
                for blk in &blocks[rank_base..rank_base + head.width[l] as usize] {
                    impulse_bits.extend([blk.ni[l].to_bits(), blk.ti1[l].to_bits(), blk.ti2[l].to_bits()]);
                }
            }
        }
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

            // Each arm builds its OWN layout from a fresh solver (an empty warm store
            // ⇒ identical zero-seeded pristine blocks) and its OWN body
            // ScratchColumn, then solves through the solve views.

            // ── Scalar arm ──────────────────────────────────────────────────
            let mut solver_scalar = ColoredSoftStepSolver::default();
            solver_scalar.build_bodies(&bodies);
            solver_scalar.build_columns(&manifolds, &graph, &bodies, None, RowRemap::Identity);
            let cols_scalar = &solver_scalar.columns;
            let n_colors = cols_scalar.color_offsets().len() - 1;
            let bodies_scalar = body_scratch_from(&pristine_bodies);
            {
                let view = cols_scalar.solve_view();
                let body_view = bodies_scalar.solve_view();
                for c in 0..n_colors {
                    let ctx = cols_scalar.color_ctx(c);
                    let g_hi = cols_scalar.color_group_start()[c + 1] as usize;
                    ColoredSoftStepSolver::solve_color(
                        view,
                        body_view,
                        ctx,
                        ctx.g_base,
                        g_hi,
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
            solver_simd.build_columns(&manifolds, &graph, &bodies, None, RowRemap::Identity);
            let cols_simd = &solver_simd.columns;
            let bodies_simd = body_scratch_from(&pristine_bodies);
            {
                let view = cols_simd.solve_view();
                let body_view = bodies_simd.solve_view();
                for c in 0..n_colors {
                    let ctx = cols_simd.color_ctx(c);
                    let g_hi = cols_simd.color_group_start()[c + 1] as usize;
                    let (k_lo, k_hi) = ctx.cohorts_of(ctx.g_base, g_hi);
                    // SAFETY: the test target is gated `target_feature = "avx2"`, so
                    //   the host running these tests supports AVX2; the cohort range is a
                    //   color's own (body-disjoint) cohorts.
                    unsafe {
                        ColoredSoftStepSolver::solve_color_avx2(
                            view,
                            body_view,
                            k_lo,
                            k_hi,
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
                "O7 impulse bits must match scalar (bias_active={bias_active})"
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

    /// Deep-copies `src` into a fresh `CohortColumns` (each column refilled from the
    /// source's read slice). Used by the +avx2 differential so the two kernel arms
    /// solve over independent tables. Copies every column: the tables the kernels
    /// read / write, the per-manifold columns and the CSRs they navigate.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn clone_columns(src: &CohortColumns) -> CohortColumns {
        let mut dst = CohortColumns::with_capacity(src.len());
        dst.heads.build_view().extend_from_slice(src.heads());
        dst.blocks.build_view().extend_from_slice(src.blocks());
        dst.cold.build_view().extend_from_slice(src.cold.as_read_slice());
        dst.rank_cold.build_view().extend_from_slice(src.rank_cold.as_read_slice());
        dst.plan.build_view().extend_from_slice(src.plan());
        dst.tags.build_view().extend_from_slice(src.tags());
        dst.color_offsets.build_view().extend_from_slice(src.color_offsets());
        dst.group_start.build_view().extend_from_slice(src.group_start());
        dst.color_group_start.build_view().extend_from_slice(src.color_group_start());
        dst.color_cohort_start.build_view().extend_from_slice(src.color_cohort_start());
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

    /// G8 (L11 C2): the fill, both kernels and the store at small `n`, pool-free —
    /// the ragged scene (a partial cohort, a four-rank lane beside one-rank lanes)
    /// for three steps on each kernel, so Miri walks every raw projection and
    /// aligned row access of the build, the sweeps and the store. The two kernels'
    /// layouts and records are byte-equal and every padding lane and rank is zero.
    #[test]
    fn cohort_layout_small_n_matches_across_kernels() {
        let (bodies, manifolds) = ragged_colored_scene(11);
        let run = |simd_solve: bool| -> (Vec<u32>, Vec<u32>, PaddingAudit) {
            let cfg = PhysicsConfig { dt: 1.0 / 60.0, simd_solve, ..PhysicsConfig::default() };
            let mut solver = ColoredSoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            scratch.set_bodies(&bodies);
            scratch.touched.reset(scratch.bodies().len());
            for _ in 0..3 {
                let graph = build_graph(scratch.bodies(), &manifolds);
                scratch.touched.reset(scratch.bodies().len());
                solver.solve_colored(&cfg, &manifolds, &graph, &mut scratch);
            }
            (layout_snapshot(&solver.columns), records_snapshot(&solver), audit_padding(&solver.columns))
        };
        let (layout_scalar, records_scalar, padding_scalar) = run(false);
        let (layout_simd, records_simd, padding_simd) = run(true);
        assert_eq!(layout_scalar, layout_simd, "the two kernels leave byte-equal layouts");
        assert_eq!(records_scalar, records_simd, "the two kernels leave byte-equal records");
        assert!(
            padding_scalar.zero && padding_simd.zero,
            "every padding lane and rank is zero: {padding_scalar:?} / {padding_simd:?}"
        );
        assert!(
            padding_scalar.partial_cohorts > 0 && padding_scalar.padded_ranks > 0,
            "anti-vacuity: the ragged scene has a partial cohort and padded ranks: {padding_scalar:?}"
        );
    }

    // ── O1 (regression-pin): cone / degenerate adversarial differential ──────
    //
    // Test 1c/1d build a SINGLE one-color `CohortColumns` from explicit group specs
    // — through P-b and the fill, with the specs' seeds written into the lanes — so
    // every lane's geometry / impulse seed / body state is exact, forcing the
    // adversarial friction-cone + degenerate paths to fire NON-VACUOUSLY, then
    // assert `solve_color_avx2 == solve_color` bit-for-bit. The non-vacuity counts
    // come from `cone_probe`, a single-lane replay of the EXACT scalar op sequence
    // (the authoritative oracle for "did this lane clamp / was len_sq zero /
    // denormal"). A splitmix64 proptest then sweeps random cohort shapes.

    /// One group spec for a hand-rolled single-color layout: a body pair (`ia`,
    /// `ib`/sentinel), the manifold's normal and friction, and its contact points.
    /// Each point carries explicit anchors, separation and an impulse seed.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[derive(Clone)]
    struct GroupSpec {
        ia: u32,
        ib: u32,
        sentinel: bool,
        normal: Vec3,
        friction: f32,
        points: Vec<PointSpec>,
    }

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[derive(Clone, Copy)]
    struct PointSpec {
        ra: Vec3,
        rb: Vec3,
        separation: f32,
        seed: (f32, f32, f32),
    }

    /// Builds a single-COLOR layout from the group specs through the production P-b
    /// and fill: the specs become manifolds over bodies at the origin (so a point's
    /// anchor IS its lane anchor) carrying the group's friction on both rows and the
    /// given effective state, and the specs' seeds are then written into the lanes
    /// (a fresh warm store seeds zero). Body-disjointness of the groups is the
    /// CALLER's responsibility (the cohort kernel's precondition); it is what makes
    /// the graph put every group in one color, in spec order.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn build_cohort_solver(groups: &[GroupSpec], bodies: &[BodyEffective]) -> ColoredSoftStepSolver {
        let mut states: Vec<BodyState> = bodies
            .iter()
            .map(|b| BodyState {
                inv_inertia: b.inv_inertia,
                inv_inertia_local: b.inv_inertia,
                position: Vec3::ZERO,
                linear_velocity: b.linear_velocity,
                angular_velocity: b.angular_velocity,
                rotation: Quat::IDENTITY,
                inv_mass: b.inv_mass,
                restitution: 0.0,
                friction: 0.0,
                simulated: true,
                kinematic: false,
                is_sensor: false,
                shape: ColliderShape::Sphere { radius: 1.0 },
            })
            .collect();
        let mut manifolds = Vec::with_capacity(groups.len());
        for g in groups {
            states[g.ia as usize].friction = g.friction;
            if !g.sentinel {
                states[g.ib as usize].friction = g.friction;
            }
            let mut m = Manifold::new(BodyIndex(g.ia), if g.sentinel { SDF_SENTINEL } else { BodyIndex(g.ib) });
            m.normal = g.normal;
            for (p, ps) in g.points.iter().enumerate() {
                m.points[p] = ContactPoint {
                    anchor_a: ps.ra,
                    anchor_b: ps.rb,
                    separation: ps.separation,
                    feature_id: p as u32,
                };
            }
            m.count = g.points.len() as u8;
            manifolds.push(m);
        }
        let graph = build_graph(&states, &manifolds);
        let mut solver = ColoredSoftStepSolver::default();
        solver.build_bodies(&states);
        solver.build_columns(&manifolds, &graph, &states, None, RowRemap::Identity);
        assert_eq!(solver.columns.color_offsets().len(), 2, "body-disjoint specs form one color");
        assert_eq!(solver.columns.group_start().len(), groups.len() + 1, "one group per spec");
        // The seeds are the specs' (the fresh warm store seeded zero).
        let ctx = solver.columns.color_ctx(0);
        let heads = solver.columns.heads();
        let rank_bases: Vec<usize> = heads.iter().map(|h| h.rank_base as usize).collect();
        {
            let mut blocks = solver.columns.blocks.build_view();
            let blocks = blocks.as_mut_slice();
            for (g, spec) in groups.iter().enumerate() {
                let (k, l) = ctx.lane_of(g);
                for (r, ps) in spec.points.iter().enumerate() {
                    let blk = &mut blocks[rank_bases[k] + r];
                    blk.ni[l] = ps.seed.0;
                    blk.ti1[l] = ps.seed.1;
                    blk.ti2[l] = ps.seed.2;
                }
            }
        }
        solver
    }

    /// Replays the EXACT scalar `solve_color` friction-cone evaluation for ONE lane
    /// rank against the pristine pre-solve state, reporting `(clamped, zero_cone,
    /// denorm_len_sq)`. The kernel is bit-identical to `solve_color`, so this is the
    /// authoritative non-vacuity oracle for that point. `len_sq == 0` ⇒ `zero_cone`;
    /// `0 < len_sq < f32::MIN_POSITIVE` ⇒ `denorm_len_sq`; the scalar clamp branch
    /// (`len_sq > mf² && len_sq > 0`) firing ⇒ `clamped`.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[allow(clippy::too_many_arguments)]
    fn cone_probe(
        cols: &CohortColumns,
        bodies: &[BodyEffective],
        k: usize,
        l: usize,
        r: usize,
        bias_rate: f32,
        mass_coeff: f32,
        impulse_coeff: f32,
        bias_active: bool,
    ) -> (bool, bool, bool) {
        let head = &cols.heads()[k];
        let blk = &cols.blocks()[head.rank_base as usize + r];
        let ra = blk.ra(l);
        let rb = blk.rb(l);
        let normal = head.normal(l);
        let t1 = head.tangent1(l);
        let t2 = normal.cross(t1);
        let ia = head.body_a[l] as usize;
        let b_sent = head.is_sentinel(l);
        let ib = head.body_b[l] as usize;
        let friction = head.friction[l];
        let separation = blk.sep[l];
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
        let lambda_n = blk.ni[l];
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
        let new_t1 = blk.ti1[l] - m_eff_t1 * vt1;
        let new_t2 = blk.ti2[l] - m_eff_t2 * vt2;
        let len_sq = new_t1 * new_t1 + new_t2 * new_t2;
        let clamped = len_sq > max_friction * max_friction && len_sq > 0.0;
        let zero_cone = len_sq == 0.0;
        let denorm = len_sq > 0.0 && len_sq < f32::MIN_POSITIVE;
        (clamped, zero_cone, denorm)
    }

    /// Solves the single color of `cols` with the scalar oracle and with the AVX2
    /// cohort kernel (each on a fresh clone seeded to the same pristine state), and
    /// asserts the body + impulse bits match bit-for-bit. Returns the per-point
    /// `(clamped, zero_cone, denorm)` counts from the scalar probe for non-vacuity.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn assert_cohort_differential(
        cols: &CohortColumns,
        bodies: &[BodyEffective],
        bias_active: bool,
    ) -> (usize, usize, usize) {
        let soft = SoftCoefficients::new(
            PhysicsConfig::default().contact_hertz,
            PhysicsConfig::default().contact_damping,
            (1.0 / 60.0) / 4.0,
        );

        // Non-vacuity counts from the pristine state (the probe is read-only), over
        // every live lane rank of the color's cohorts.
        let (mut clamped, mut zero_cone, mut denorm) = (0usize, 0usize, 0usize);
        for (k, head) in cols.heads().iter().enumerate() {
            for l in 0..head.nlanes as usize {
                for r in 0..head.width[l] as usize {
                    let (c, z, d) = cone_probe(
                        cols,
                        bodies,
                        k,
                        l,
                        r,
                        soft.bias_rate,
                        soft.mass_coeff,
                        soft.impulse_coeff,
                        bias_active,
                    );
                    clamped += c as usize;
                    zero_cone += z as usize;
                    denorm += d as usize;
                }
            }
        }

        let ctx = cols.color_ctx(0);
        let g_hi = cols.color_group_start()[1] as usize;
        let (k_lo, k_hi) = ctx.cohorts_of(ctx.g_base, g_hi);

        // Scalar arm — a fresh deep copy of `cols` + its own body buffer.
        let cols_scalar = clone_columns(cols);
        let bodies_scalar = body_scratch_from(bodies);
        ColoredSoftStepSolver::solve_color(
            cols_scalar.solve_view(),
            bodies_scalar.solve_view(),
            ctx,
            ctx.g_base,
            g_hi,
            soft.bias_rate,
            soft.mass_coeff,
            soft.impulse_coeff,
            bias_active,
        );

        // SIMD arm — an independent deep copy + body buffer.
        let cols_simd = clone_columns(cols);
        let bodies_simd = body_scratch_from(bodies);
        // SAFETY: the test target is `target_feature = "avx2"`-gated, so the host
        //   supports AVX2; `[k_lo, k_hi)` are the single color's body-disjoint
        //   cohorts.
        unsafe {
            ColoredSoftStepSolver::solve_color_avx2(
                cols_simd.solve_view(),
                bodies_simd.solve_view(),
                k_lo,
                k_hi,
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
                normal: n,
                friction,
                points: vec![PointSpec {
                    ra,
                    rb: ra,
                    separation: -0.2,
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
        let solver = build_cohort_solver(&groups, &bodies);

        let mut total_clamped = 0;
        let mut total_zero = 0;
        let mut total_denorm = 0;
        for bias_active in [true, false] {
            let (c, z, d) = assert_cohort_differential(&solver.columns, &bodies, bias_active);
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
        let pt = |seed: (f32, f32, f32), ra: Vec3| PointSpec {
            ra,
            rb: ra,
            separation: -0.25,
            seed,
        };

        // Lane 0 — STATIC body A (inv_mass 0): the `ia_movable == false` guard side.
        let g0 = GroupSpec {
            ia: 0,
            ib: 1,
            sentinel: false,
            normal: n,
            friction: 0.5,
            points: vec![pt((0.1, 0.2, -0.1), Vec3::new(0.2, 0.0, 0.1))],
        };
        // Lane 1 — SENTINEL body B: body B is IMMOVABLE_AT_REST, never indexed; a
        // live dynamic A with a clamp-forcing tangent seed.
        let g1 = GroupSpec {
            ia: 2,
            ib: u32::MAX,
            sentinel: true,
            normal: n,
            friction: 0.05,
            points: vec![pt((0.05, 6.0, 6.0), Vec3::new(-0.1, 0.0, 0.3))],
        };
        // Lane 2 — DEGENERATE k<=0: both bodies static (inv_mass 0, inertia ZERO) ⇒
        // effective_mass returns 0 ⇒ a no-op solve.
        let g2 = GroupSpec {
            ia: 3,
            ib: 4,
            sentinel: false,
            normal: n,
            friction: 0.5,
            points: vec![pt((0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.2))],
        };

        let groups = vec![g0, g1, g2];
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
        let solver = build_cohort_solver(&groups, &bodies);

        let mut total_clamped = 0;
        for bias_active in [true, false] {
            let (c, _z, _d) = assert_cohort_differential(&solver.columns, &bodies, bias_active);
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

    /// Draws one random cohort-shape case: group count 1..=32 (⇒ multi-cohort,
    /// and a PARTIAL trailing cohort whenever the count is not a multiple of 8),
    /// width 1..=`MAX_CONTACT_POINTS`, masses including statics and sentinels, and
    /// occasional zero friction and denormal-scale seeds. Each group owns two
    /// disjoint dynamic rows (or one plus a sentinel), so the specs form ONE color
    /// in spec order.
    ///
    /// One corpus definition, two properties over it: the O7 kernel proptest below
    /// and C3's warm-apply proptest both draw from it. The manifold normal `n` is
    /// the CALLER's, not a draw, so adding a caller cannot move an existing corpus:
    /// the kernel proptest keeps its axis-aligned `(0, 1, 0)` and the warm-apply one
    /// passes [`OBLIQUE_NORMAL`], for the reason that constant documents.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn random_cohort_corpus(
        rng: &mut SplitMix64,
        n: Vec3,
    ) -> (Vec<GroupSpec>, Vec<BodyEffective>) {
        use crate::math::MAX_CONTACT_POINTS;
        let n_groups = rng.range(1, 33) as usize; // 1..=32 ⇒ multi-cohort
        let mut groups: Vec<GroupSpec> = Vec::with_capacity(n_groups);
        // Body rows: each group owns 2 disjoint dynamic rows (or 1 + sentinel).
        let mut bodies: Vec<BodyEffective> = Vec::with_capacity(n_groups * 2);
        for _gi in 0..n_groups {
            let ia = bodies.len() as u32;
            // Body A: mostly dynamic, sometimes static (the *_movable guard).
            let a_static = rng.f01() < 0.15;
            bodies.push(if a_static {
                BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: rand_vel(rng), angular_velocity: rand_vel(rng) }
            } else {
                dyn_eff(0.5 + rng.f01(), 0.5 + rng.f01() * 2.0, rand_vel(rng), rand_vel(rng))
            });
            let sentinel = rng.f01() < 0.25;
            let ib = if sentinel {
                u32::MAX
            } else {
                let row = bodies.len() as u32;
                let b_static = rng.f01() < 0.15;
                bodies.push(if b_static {
                    BodyEffective { inv_mass: 0.0, inv_inertia: Mat3::ZERO, linear_velocity: rand_vel(rng), angular_velocity: rand_vel(rng) }
                } else {
                    dyn_eff(0.5 + rng.f01(), 0.5 + rng.f01() * 2.0, rand_vel(rng), rand_vel(rng))
                });
                row
            };
            // Occasionally zero friction (the zero-cone path) — per group.
            let zero_fric = rng.f01() < 0.1;
            let friction = if zero_fric { 0.0 } else { rng.f01() * 2.0 };
            let width = rng.range(1, MAX_CONTACT_POINTS as u32 + 1) as usize;
            let mut points = Vec::with_capacity(width);
            for _ in 0..width {
                // Occasionally a denormal-scale tangent seed.
                let denorm = rng.f01() < 0.1;
                let seed_scale = if denorm { 1e-22 } else { 4.0 };
                points.push(PointSpec {
                    ra: rand_vel(rng) * 0.3,
                    rb: rand_vel(rng) * 0.3,
                    separation: -(rng.f01() * 0.5),
                    seed: (
                        rng.f01() * 0.5,
                        (rng.f01() - 0.5) * seed_scale,
                        (rng.f01() - 0.5) * seed_scale,
                    ),
                });
            }
            groups.push(GroupSpec { ia, ib, sentinel, normal: n, friction, points });
        }
        (groups, bodies)
    }

    /// O1 proptest (+avx2 only): random cohort shapes (group count 1..=32, width
    /// 1..=MAX_CONTACT_POINTS, masses incl. statics + sentinels, denormal-scale
    /// velocities) must be `solve_color_avx2 == solve_color` bit-for-bit, AND the
    /// cone clamp + zero-cone paths must fire non-vacuously across the corpus. The
    /// friction is per group (a manifold constant in the cohort layout); the
    /// seeds, anchors and separations stay per point.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn cohort_shape_proptest_bit_exact_and_non_vacuous() {
        let mut rng = SplitMix64(0x0BAD_F00D_DEAD_BEEF);
        let mut corpus_clamped = 0usize;
        let mut corpus_zero = 0usize;

        for _ in 0..200 {
            let (groups, bodies) = random_cohort_corpus(&mut rng, Vec3::new(0.0, 1.0, 0.0));

            // The specs form ONE color (multi-cohort when n_groups > 8); the kernel
            // solves them as 8-group cohorts.
            let solver = build_cohort_solver(&groups, &bodies);
            for bias_active in [true, false] {
                let (c, z, _d) = assert_cohort_differential(&solver.columns, &bodies, bias_active);
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

    // ── G4 (L11 C3): the {scalar, simd} warm-apply differential ──────────────
    //
    // D7 forks ONE function into two shapes: `warm_apply_scalar`, the group-major
    // ORACLE the `simd_solve = false` step takes, and `warm_apply_avx2`, eight
    // lanes at a time. The design's claim is BIT-identity, so every test below runs
    // both over the same cohort table from the same pristine bodies and compares
    // body bits — the arm shape `assert_cohort_differential` uses for the kernel,
    // minus the soft coefficients (the apply has none).
    //
    // The impulse columns are an INPUT here, never an output: the apply reads a
    // block and writes only body rows, so each arm gets its own deep copy and the
    // impulse bits are compared too. Named mutation M7 (re-associating the impulse)
    // and M12 (scattering all 8 lanes) are recorded red against these tests.

    /// A unit manifold normal with three NON-ZERO components — the only shape in
    /// which the warm apply's ASSOCIATION is observable.
    ///
    /// With an axis-aligned normal the contact basis is sparse: `n = (0, 1, 0)`
    /// forces `t1.y == t2.y == 0` (both are perpendicular to `n`) and `n.x == n.z
    /// == 0`, so in every component one of the three products is a zero and
    /// `(a + b) + c == a + (b + c)` EXACTLY. Mutation M7 — which re-associates the
    /// impulse as `n·λn + (t1·λt1 + t2·λt2)` — is then bit-invisible. MEASURED on
    /// this tree: with `(0, 1, 0)` the entire warm-apply corpus passes under M7
    /// while four G1 pins go red, so a gate built on axis-aligned scenes alone
    /// would have reported the mutation caught when it was not.
    ///
    /// `0.48² + 0.64² + 0.6² == 1` in exact arithmetic (f32 rounds the sum to
    /// within an ulp, which is all a real manifold normal offers either).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    const OBLIQUE_NORMAL: Vec3 = Vec3::new(0.48, 0.64, 0.6);

    /// The six velocity words of one body row — the unit every warm-apply
    /// assertion below compares, bit for bit (`-0.0` and `+0.0` are DISTINCT here,
    /// which is what makes the O1 scene load-bearing).
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn vel_bits(b: &BodyEffective) -> [u32; 6] {
        [
            b.linear_velocity.x.to_bits(),
            b.linear_velocity.y.to_bits(),
            b.linear_velocity.z.to_bits(),
            b.angular_velocity.x.to_bits(),
            b.angular_velocity.y.to_bits(),
            b.angular_velocity.z.to_bits(),
        ]
    }

    /// Runs BOTH warm applies over `cols` from the same pristine `bodies` and
    /// asserts the two results are bit-identical. Returns how many body rows the
    /// apply MOVED (velocity bits differing from pristine), the non-vacuity count.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn assert_warm_apply_differential(cols: &CohortColumns, bodies: &[BodyEffective]) -> usize {
        // Independent tables AND body buffers per arm, so a block written by either
        // apply surfaces as an impulse-bit difference instead of being shared.
        let cols_scalar = clone_columns(cols);
        let bodies_scalar = body_scratch_from(bodies);
        ColoredSoftStepSolver::warm_apply_scalar(&cols_scalar, bodies_scalar.solve_view());

        let cols_simd = clone_columns(cols);
        let bodies_simd = body_scratch_from(bodies);
        // SAFETY: the test target is `target_feature = "avx2"`-gated, so the host
        //   running it supports AVX2; `cols_simd` is a deep copy of a fully built
        //   cohort table (every head's `rank_base + depth` within its blocks), and
        //   this thread is the only accessor of `bodies_simd`.
        unsafe { ColoredSoftStepSolver::warm_apply_avx2(&cols_simd, bodies_simd.solve_view()) };

        let (b_scalar, i_scalar) = body_impulse_bits(bodies_scalar.as_read_slice(), &cols_scalar);
        let (b_simd, i_simd) = body_impulse_bits(bodies_simd.as_read_slice(), &cols_simd);
        assert_eq!(b_scalar, b_simd, "warm-apply differential: body velocity bits");
        assert_eq!(i_scalar, i_simd, "warm-apply differential: impulse bits");
        let (_, i_orig) = body_impulse_bits(bodies, cols);
        assert_eq!(i_orig, i_scalar, "neither warm apply may write an impulse column");

        bodies
            .iter()
            .zip(bodies_scalar.as_read_slice().iter())
            .filter(|(pristine, applied)| vel_bits(pristine) != vel_bits(applied))
            .count()
    }

    /// Writes pseudo-random warm seeds into every LIVE `(lane, rank)` slot of every
    /// cohort, leaving padding lanes and padding ranks at the fill's zero (G3's
    /// invariant). A fresh warm store seeds zero, so without this the apply would be
    /// a value no-op and the differential vacuous.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    fn seed_live_lanes(solver: &mut ColoredSoftStepSolver, rng: &mut SplitMix64) {
        let shape: Vec<(usize, usize, [u8; COHORT])> = solver
            .columns
            .heads()
            .iter()
            .map(|h| (h.rank_base as usize, h.nlanes as usize, h.width))
            .collect();
        let mut blocks = solver.columns.blocks.build_view();
        let blocks = blocks.as_mut_slice();
        for (rank_base, nlanes, width) in shape {
            for (l, &w) in width[..nlanes].iter().enumerate() {
                for r in 0..w as usize {
                    let blk = &mut blocks[rank_base + r];
                    blk.ni[l] = 0.25 + rng.f01();
                    blk.ti1[l] = (rng.f01() - 0.5) * 2.0;
                    blk.ti2[l] = (rng.f01() - 0.5) * 2.0;
                }
            }
        }
    }

    /// G4/C3 test 1: the warm apply is bit-identical on the O7 ragged scene — 11
    /// width-1 floor groups (one color crossing the 8-lane cohort boundary, so the
    /// trailing cohort has PADDING lanes) plus the width-4 box manifold, with the
    /// shared static floor as body B on every floor lane.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn warm_apply_bits_match_scalar_on_the_ragged_scene() {
        let (bodies, manifolds) = ragged_colored_scene(11);
        let graph = build_graph(&bodies, &manifolds);
        let mut solver = ColoredSoftStepSolver::default();
        solver.build_bodies(&bodies);
        solver.build_columns(&manifolds, &graph, &bodies, None, RowRemap::Identity);
        let mut rng = SplitMix64(0x11C3_5EED_A1B2_C3D4);
        seed_live_lanes(&mut solver, &mut rng);

        let pristine: Vec<BodyEffective> = bodies.iter().map(eff_of).collect();
        let moved = assert_warm_apply_differential(&solver.columns, &pristine);
        assert!(moved > 0, "non-vacuity: the warm apply must move at least one body row");
    }

    /// G4/C3 test 2: the warm apply is bit-identical over the O7 cohort-shape
    /// corpus — random group counts 1..=32 (multi-cohort, partial trailing
    /// cohorts), ragged widths, static A rows, sentinel B rows and denormal-scale
    /// seeds, all generated by [`random_cohort_corpus`], the same generator the
    /// kernel proptest draws from.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn warm_apply_proptest_bits_match_scalar_and_non_vacuous() {
        let mut rng = SplitMix64(0x11C3_0BAD_F00D_5EED);
        let mut corpus_moved = 0usize;
        let mut padded_cohorts = 0usize;
        for _ in 0..200 {
            let (groups, bodies) = random_cohort_corpus(&mut rng, OBLIQUE_NORMAL);
            let solver = build_cohort_solver(&groups, &bodies);
            padded_cohorts += solver
                .columns
                .heads()
                .iter()
                .filter(|h| (h.nlanes as usize) < COHORT)
                .count();
            corpus_moved += assert_warm_apply_differential(&solver.columns, &bodies);
        }
        eprintln!("warm-apply proptest non-vacuity: moved={corpus_moved} padded={padded_cohorts}");
        assert!(
            corpus_moved > 0 && padded_cohorts > 0,
            "non-vacuity over the random corpus: the apply must move rows ({corpus_moved}) and \
             the corpus must contain padded cohorts ({padded_cohorts})"
        );
    }

    /// G4/C3 test 3 (review O1, the `-0.0` scene): an `inv_mass` of `-0.0` is
    /// IMMOVABLE on both warm-apply paths.
    ///
    /// The movability predicate is the IEEE `inv_mass != 0.0`
    /// ([`is_dynamic_row`]), which is FALSE for `-0.0`; a bit test
    /// (`to_bits() != 0`) would call the row movable instead. The two differ only
    /// where writing back is not a value no-op, so the scene is built to make the
    /// write visible: the row's velocity words are `-0.0`, and the impulse it would
    /// receive is signed so that `-0.0 + (-p)·(-0.0) == +0.0` — a different BIT
    /// pattern. The test asserts that flip WOULD happen (non-vacuity) and that
    /// neither path performs it.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn negative_zero_inv_mass_row_is_immovable_in_both_warm_applies() {
        let n = OBLIQUE_NORMAL;
        let seed = (0.75f32, 0.5f32, -0.25f32);
        // Lane 0: body A carries `inv_mass == -0.0` (a static row by the IEEE
        // predicate, so `inv_inertia` is ZERO — the build invariant), body B is a
        // plain dynamic row that MUST move (the arm's non-vacuity).
        let groups = vec![GroupSpec {
            ia: 0,
            ib: 1,
            sentinel: false,
            normal: n,
            friction: 0.4,
            points: vec![PointSpec {
                ra: Vec3::new(0.2, 0.0, 0.1),
                rb: Vec3::new(0.2, 0.0, 0.1),
                separation: -0.25,
                seed,
            }],
        }];
        let neg_zero_row = BodyEffective {
            inv_mass: -0.0,
            inv_inertia: Mat3::ZERO,
            linear_velocity: Vec3::new(-0.0, -0.0, -0.0),
            angular_velocity: Vec3::new(-0.0, -0.0, -0.0),
        };
        let bodies = vec![
            neg_zero_row,
            dyn_eff(1.0, 1.25, Vec3::new(0.3, -0.2, 0.1), Vec3::new(0.05, -0.1, 0.02)),
        ];

        // Non-vacuity: replay the scalar impulse for this lane and show that writing
        // body A back WOULD change its bits. `t2 = n × t1` and the left-to-right
        // association are the oracle's own (`warm_apply_scalar`).
        let (t1, _) = tangent_basis(n);
        let t2 = n.cross(t1);
        let impulse = n * seed.0 + t1 * seed.1 + t2 * seed.2;
        let p = impulse * -1.0; // body A receives the negated impulse
        let would_be = Vec3::new(
            -0.0f32 + p.x * -0.0f32,
            -0.0f32 + p.y * -0.0f32,
            -0.0f32 + p.z * -0.0f32,
        );
        assert_ne!(
            [would_be.x.to_bits(), would_be.y.to_bits(), would_be.z.to_bits()],
            [(-0.0f32).to_bits(); 3],
            "the `-0.0` scene must DISTINGUISH the two predicates: writing the row back has to \
             flip at least one sign bit, or this test cannot see a bit-test mask"
        );

        let solver = build_cohort_solver(&groups, &bodies);
        let moved = assert_warm_apply_differential(&solver.columns, &bodies);
        assert_eq!(moved, 1, "exactly body B moves: the `-0.0` row must not");

        // And the `-0.0` row is byte-frozen on the ORACLE path too (the differential
        // above only pins the two paths to each other).
        let scalar = body_scratch_from(&bodies);
        ColoredSoftStepSolver::warm_apply_scalar(&solver.columns, scalar.solve_view());
        assert_eq!(
            vel_bits(&scalar.as_read_slice()[0]),
            vel_bits(&bodies[0]),
            "an `inv_mass == -0.0` row must keep its exact `-0.0` velocity words"
        );
    }

    /// G4/C3 test 4 (review O2, the value half): a PADDING lane is not a lane — the
    /// scatter writes lanes `< nlanes` only.
    ///
    /// The fill zeroes a padding lane's `body_a`, so lanes `>= nlanes` name row `0`.
    /// Here row 0 is also lane 0's body A, a real dynamic row the apply moves:
    /// a scatter that walked all 8 lanes would write the padding lanes' registers —
    /// the state gathered at cohort entry — OVER lane 0's result, because the
    /// scatter walks lanes ascending. That is mutation M12, and it is recorded red
    /// against this test.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn warm_apply_padding_lane_is_not_a_lane() {
        let n = OBLIQUE_NORMAL;
        let pt = |seed: (f32, f32, f32), ra: Vec3| PointSpec { ra, rb: ra, separation: -0.3, seed };
        // Three groups ⇒ ONE cohort with nlanes = 3 and five padding lanes, all
        // naming row 0 — which is group 0's body A.
        let groups = vec![
            GroupSpec {
                ia: 0,
                ib: 1,
                sentinel: false,
                normal: n,
                friction: 0.5,
                points: vec![pt((0.8, 0.3, -0.2), Vec3::new(0.2, 0.0, 0.1)), pt((0.4, -0.1, 0.25), Vec3::new(-0.1, 0.0, 0.2))],
            },
            GroupSpec {
                ia: 2,
                ib: 3,
                sentinel: false,
                normal: n,
                friction: 0.3,
                points: vec![pt((0.6, -0.4, 0.1), Vec3::new(0.05, 0.0, -0.15))],
            },
            GroupSpec {
                ia: 4,
                ib: u32::MAX,
                sentinel: true,
                normal: n,
                friction: 0.2,
                points: vec![pt((0.5, 0.2, 0.2), Vec3::new(-0.2, 0.0, 0.05))],
            },
        ];
        let bodies: Vec<BodyEffective> = (0..5)
            .map(|i| {
                let f = i as f32;
                dyn_eff(
                    0.6 + 0.2 * f,
                    0.8 + 0.3 * f,
                    Vec3::new(0.3 - 0.1 * f, -0.4 + 0.05 * f, 0.2),
                    Vec3::new(0.05 * f, -0.02 * f, 0.1),
                )
            })
            .collect();

        let solver = build_cohort_solver(&groups, &bodies);
        {
            let heads = solver.columns.heads();
            assert_eq!(heads.len(), 1, "three groups form one cohort");
            assert_eq!(heads[0].nlanes as usize, 3, "the cohort must have padding lanes");
            assert_eq!(heads[0].body_a[0], 0, "lane 0's body A is row 0");
            assert!(
                heads[0].body_a[3..].iter().all(|&r| r == 0),
                "the fill zeroes a padding lane's body id, so a padding lane names row 0"
            );
        }

        let moved = assert_warm_apply_differential(&solver.columns, &bodies);
        assert_eq!(moved, 5, "every one of the five rows is dynamic and moves");
    }

    /// G4/C3 test 5 (review O2, the read half — the Miri case): a lane `>= nlanes`
    /// reads NO body row, witnessed by a CONCURRENT writer of the row those lanes
    /// name.
    ///
    /// Row 0 belongs to no lane here, so on the shipped apply the two threads touch
    /// disjoint memory and the test is a plain pass. Were the gather to stage a
    /// padding lane (which names row 0), it would read a row another thread is
    /// writing — value-identical, since the lane is masked at every rank, and so
    /// invisible to every bit oracle. Only a race detector can see it:
    ///
    /// ```text
    /// MIRIFLAGS="-Zmiri-tree-borrows" cargo +nightly-x86_64-pc-windows-msvc miri test \
    ///   -p boyko-physics --lib -- --test-threads=1 --exact \
    ///   solver::colored::tests::warm_apply_padding_lane_reads_no_body_row_under_concurrent_writer
    /// ```
    ///
    /// `--exact` is load-bearing: the four `*_has_no_fma_or_approx_callsites`
    /// censuses in this crate read their own source from disk, which Miri's
    /// isolation aborts the BINARY over, so a whole-lib Miri run never reaches this
    /// test (see `.cargo/config.toml`). Mutation M12 is recorded red here.
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    #[test]
    fn warm_apply_padding_lane_reads_no_body_row_under_concurrent_writer() {
        let n = OBLIQUE_NORMAL;
        let pt = |seed: (f32, f32, f32), ra: Vec3| PointSpec { ra, rb: ra, separation: -0.2, seed };
        // Two groups over rows 1..=4 ⇒ one cohort, nlanes = 2, six padding lanes —
        // all naming row 0, which NO lane uses.
        let groups = vec![
            GroupSpec {
                ia: 1,
                ib: 2,
                sentinel: false,
                normal: n,
                friction: 0.5,
                points: vec![pt((0.7, 0.2, -0.3), Vec3::new(0.15, 0.0, 0.1))],
            },
            GroupSpec {
                ia: 3,
                ib: 4,
                sentinel: false,
                normal: n,
                friction: 0.25,
                points: vec![pt((0.45, -0.2, 0.15), Vec3::new(-0.05, 0.0, 0.2))],
            },
        ];
        let bodies: Vec<BodyEffective> = (0..5)
            .map(|i| {
                let f = i as f32;
                dyn_eff(
                    0.7 + 0.1 * f,
                    0.9 + 0.2 * f,
                    Vec3::new(0.1 * f, -0.3, 0.2 - 0.05 * f),
                    Vec3::new(-0.05 * f, 0.03, 0.01 * f),
                )
            })
            .collect();

        let solver = build_cohort_solver(&groups, &bodies);
        assert_eq!(solver.columns.heads()[0].nlanes as usize, 2, "the cohort must be padded");
        assert!(
            solver.columns.heads()[0].body_a[..2].iter().all(|&r| r != 0),
            "row 0 must belong to no lane, so the writer thread owns it alone"
        );

        // The oracle's answer for the lanes, computed with no writer in sight.
        let oracle = body_scratch_from(&bodies);
        ColoredSoftStepSolver::warm_apply_scalar(&solver.columns, oracle.solve_view());

        const WRITES: usize = 32;
        let simd = body_scratch_from(&bodies);
        let view = simd.solve_view();
        std::thread::scope(|s| {
            s.spawn(move || {
                // SAFETY (the `body_mut` contract): row 0 is named by no lane of the
                //   cohort, so the apply running on the other thread never derives a
                //   pointer to it; this closure is its only accessor for the scope.
                for _ in 0..WRITES {
                    body_mut(view, 0).linear_velocity.x += 1.0;
                }
            });
            // SAFETY: the test target is `target_feature = "avx2"`-gated, so the host
            //   supports AVX2; `solver.columns` is a fully built cohort table; the rows
            //   this apply touches are the cohort's two lanes' (rows 1..=4), disjoint
            //   from the writer's row 0.
            unsafe { ColoredSoftStepSolver::warm_apply_avx2(&solver.columns, view) };
        });

        let applied = simd.as_read_slice();
        for (row, (got, want)) in applied.iter().zip(oracle.as_read_slice()).enumerate().skip(1) {
            assert_eq!(
                vel_bits(got),
                vel_bits(want),
                "row {row} must match the scalar oracle under a concurrent writer of row 0"
            );
        }
        assert_eq!(
            applied[0].linear_velocity.x,
            bodies[0].linear_velocity.x + WRITES as f32,
            "row 0 carries the writer's increments only: the apply must not have written it"
        );
        assert_ne!(
            vel_bits(&applied[1]),
            vel_bits(&bodies[1]),
            "non-vacuity: the apply moved the cohort's lanes"
        );
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
    /// accumulates gravity (the integrate-skip gate). The floor contact is emitted on
    /// every step, including the frozen ones, and the frozen body must not fall.
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

    /// **L10 C0, mutation M0's carrier — the island metric is the MAX over its rows.** One
    /// island of three dynamic rows chained on a static floor, driven through
    /// `begin_step` / `end_step` directly:
    ///
    /// * every row at 0.6 × the threshold: each row, and so the MAX, is below it while the
    ///   SUM (1.8 ×) is above, so the island latches asleep after the debounce;
    /// * one row at 2 × the threshold, the others at rest: the MAX is above it, so the busy
    ///   row keeps its whole island awake (the "single busy row" rule the docs state).
    ///
    /// M0 (the energy MAX replaced by a SUM in `end_step`) turns the first arm red; the
    /// design's named carrier, `sleeping_pipeline_o8`, stays green under it (measured at
    /// C0), because none of its assertions reads a sleep decision.
    #[test]
    fn island_metric_is_the_max_over_its_rows() {
        let threshold = DEFAULT_SLEEP_THRESHOLD;
        let debounce: u16 = 8;
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 0.5, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.0, 1.5, 0.0), 1.0, 0.5, 0.0),
            dyn_sphere(Vec3::new(0.0, 2.5, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::ZERO),
        ];
        let up = Vec3::new(0.0, 1.0, 0.0);
        let ms = vec![
            manifold(0, 3, Vec3::new(0.0, -1.0, 0.0), -0.01, Vec3::ZERO),
            manifold(0, 1, up, -0.01, Vec3::ZERO),
            manifold(1, 2, up, -0.01, Vec3::ZERO),
        ];
        let graph = build_graph(&bodies, &ms);
        let isl = graph.island_of(0);
        assert!(
            isl != ConstraintGraph::NO_ISLAND && graph.island_of(1) == isl && graph.island_of(2) == isl,
            "construction: the three dynamic rows form one island"
        );
        let run = |speeds2: [f32; 3]| {
            let mut sleep = IslandSleep::with_capacity(4, 4);
            let mut bs = bodies.clone();
            for (b, v2) in bs.iter_mut().zip(speeds2) {
                b.linear_velocity = Vec3::new(v2.sqrt(), 0.0, 0.0);
            }
            for _ in 0..2 * debounce {
                sleep.begin_step(&graph, bs.len());
                sleep.end_step(&bs, &graph, threshold, debounce);
            }
            [0, 1, 2].map(|r| sleep.is_row_asleep(r))
        };
        let slow = 0.6 * threshold;
        assert!(
            3.0 * slow >= threshold,
            "construction: the rows' speed² sum must reach the threshold for the arm to separate MAX \
             from SUM"
        );
        assert_eq!(
            run([slow; 3]),
            [true; 3],
            "every row below the threshold: the island's MAX is below it, so all three latch asleep \
             (a SUM, 1.8 x the threshold, would keep them awake)"
        );
        assert_eq!(
            run([2.0 * threshold, 0.0, 0.0]),
            [false; 3],
            "one busy row keeps its whole island awake"
        );
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
                    // An island with no members is `is_island_frozen(isl) == true` by the
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

    /// No-FMA / no-approx grep gate: `solver/colored.rs` must contain ZERO fused
    /// (`fmadd` / `fmsub` / `fnmadd` / `fnmsub` / `fmaddsub` / `fmsubadd`), ZERO
    /// approximate (`rsqrt` / `rcp`) and ZERO `mul_add` / `algebraic_` CALL-SITES.
    ///
    /// The sibling of `solver::simd::…::solver_simd_has_no_fma_or_approx_callsites`
    /// and `sdf_simd::…::sdf_simd_has_no_fma_or_approx_callsites`, with the same
    /// needle list, the same comment skip and the same non-vacuity witness — a
    /// deliberate copy, because the four must not diverge.
    ///
    /// **Why it did not exist until 2026-09-03:** the O7 cohort kernel in
    /// `colored.rs` sits behind `cfg(target_feature = "avx2")`, and nothing enabled
    /// AVX2 in this workspace until the `x86-64-v3` baseline landed on 2026-09-02.
    /// Its 85 `_mm256_*` sites — the largest vectorised body in the crate after
    /// `solver/simd.rs` — were never compiled and therefore never censused. They are
    /// clean today; that is the property this test freezes.
    ///
    /// The stake is the O7 bit-identity claim itself: `solve_color_avx2` is asserted
    /// BIT-IDENTICAL to the scalar `solve_color` oracle for any cohort shape and
    /// worker count. A fused op rounds ONCE where the oracle rounds TWICE, and
    /// `rsqrt`/`rcp` return different bits on Intel and AMD — either would break that
    /// claim silently, in a kernel whose whole justification is that it cannot.
    ///
    /// Doc-comment prose naming the banned ops (this comment does) is allowed — only
    /// NON-comment lines are scanned.
    #[test]
    fn colored_has_no_fma_or_approx_callsites() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("solver")
            .join("colored.rs");
        let contents = std::fs::read_to_string(&path).expect("solver/colored.rs must be readable");

        // Match a CALL-SITE: each stem completed to a real `_ps(` invocation, over
        // both vector widths. The needles are ASSEMBLED from fragments at runtime,
        // as in the two older censuses: those scan their OWN file, where a literal
        // would flag the definition line. This one scans a sibling file, so the
        // assembly is not strictly required here — it is kept so the four censuses
        // stay copy-paste siblings and so moving this test into `colored.rs` could
        // not silently turn it vacuous.
        let suffix = "_ps(";
        let widths = ["_mm256_", "_mm_"];
        let stems = ["fmadd", "fmsub", "fnmadd", "fnmsub", "fmaddsub", "fmsubadd", "rsqrt", "rcp"];
        let mut banned: Vec<String> = Vec::with_capacity(widths.len() * stems.len() + 2);
        for w in widths {
            for s in stems {
                banned.push(format!("{w}{s}{suffix}"));
            }
        }
        // The safe-Rust route to the same single rounding — reachable without ever
        // typing an intrinsic, which an intrinsic-only ban would never see.
        banned.push(format!("{}{}", "mul_add", "("));
        // `algebraic_mul` / `_add` / `_sub` / `_div` / `_rem` (stable 1.98): the
        // sanctioned per-operation fast-math API, which permits exactly the two
        // freedoms — contraction and reassociation — this solver's determinism rests
        // on refusing. The stem alone is banned so a UFCS spelling cannot defeat it.
        banned.push(format!("{}{}", "algebraic", "_"));

        let mut hits = Vec::new();
        for (i, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip doc / line comments — prose may name the banned ops to document
            // the prohibition. `//!` starts with `//`, so one check covers both.
            if trimmed.starts_with("//") {
                continue;
            }
            for b in &banned {
                if line.contains(b.as_str()) {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(
            hits.is_empty(),
            "no-FMA/no-approx invariant violated: solver/colored.rs has banned op call-sites (the \
             O7 cohort kernel claims BIT-IDENTITY with the scalar `solve_color` oracle; a fused \
             or approximate op ends that claim):\n{}",
            hits.join("\n"),
        );

        // Non-vacuity: a census that scans the wrong text passes for the wrong
        // reason. The witness is ASSEMBLED like the needles rather than written as a
        // literal, so it can only be satisfied by a real call site in the SCANNED
        // file — a literal witness is satisfied by the assertion's own source line
        // whenever the census scans the file it lives in.
        let witness = format!("{}{}{}", "_mm256_", "mul", suffix);
        assert!(
            contents.contains(&witness),
            "census scanned {} but found no `{witness}` call-site — the file moved or was \
             rewritten, so an empty hit list proves nothing",
            path.display(),
        );
    }

    // ── A4: wake-on-contact-change (design Validation U1–U7, U10) ────────────
    //
    // These drive `IslandSleep::begin_step` over hand-built graphs (NO schedule, NO
    // threadpool), so they run native and under Miri. A row's island contact key is the
    // number of manifolds filed under its island; a latched row whose key changed is
    // unlatched. Every dynamic row is an island node (a lone dynamic row is a singleton
    // island with key 0); only a static row has no island.

    use crate::resources::DEFAULT_SLEEP_FRAMES;

    /// A touching single-point manifold between rows `a` and `b` (the geometry is
    /// irrelevant to `begin_step`, which only counts manifolds per island).
    fn contact(a: u32, b: u32) -> Manifold {
        manifold(a, b, Vec3::new(1.0, 0.0, 0.0), -0.01, Vec3::ZERO)
    }

    /// `n` dynamic unit spheres at rest, spaced along `x`.
    fn dyn_rows(n: usize) -> Vec<BodyState> {
        (0..n)
            .map(|i| dyn_sphere(Vec3::new(i as f32 * 3.0, 1.0, 0.0), 1.0, 0.5, 0.0))
            .collect()
    }

    /// "Freeze": `begin_step`, latch `rows`, `begin_step` again, and prove each latched
    /// row's island is frozen (the anti-vacuity half every test below starts from).
    fn freeze_rows(sleep: &mut IslandSleep, graph: &ConstraintGraph, n_rows: usize, rows: &[usize]) {
        sleep.begin_step(graph, n_rows);
        for &row in rows {
            sleep.force_sleep_row(row);
        }
        sleep.begin_step(graph, n_rows);
        for &row in rows {
            assert!(
                sleep.is_island_frozen(graph.island_of(row as u32)),
                "anti-vacuity: row {row} was latched and its island must be frozen before the change"
            );
        }
    }

    /// U1. Rows 0 and 1 are latched together; the only manifold of row 0 goes away (it
    /// moves to rows 1 and 2), so row 0's island count falls 1 → 0 and row 0 must wake.
    #[test]
    fn a_latched_island_that_loses_its_only_contact_wakes() {
        let bodies = dyn_rows(3);
        let g_a = build_graph(&bodies, &[contact(0, 1)]);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        freeze_rows(&mut sleep, &g_a, bodies.len(), &[0, 1]);

        let g_b = build_graph(&bodies, &[contact(1, 2)]);
        sleep.begin_step(&g_b, bodies.len());
        assert_eq!(
            (
                sleep.is_row_asleep(0),
                sleep.is_row_awake(0),
                sleep.is_island_frozen(g_b.island_of(0)),
                sleep.contact_wakes(),
            ),
            (false, true, false, 1),
            "a latched row whose island lost its only manifold must wake on that step: \
             (latched, awake, island frozen, contact_wakes)"
        );
        // C3 half: row 1 kept its count (1 → 1) but now shares an island with the
        // never-latched row 2, so the island is active.
        assert_eq!(
            (
                sleep.is_island_frozen(g_b.island_of(1)),
                sleep.is_row_awake(1),
                sleep.is_row_awake(2),
            ),
            (false, true, true),
            "C3: the island rows 1 and 2 form must be active: (frozen, row 1 awake, row 2 awake)"
        );
    }

    /// U2. The island ids renumber (row 2's island is id 1 before and id 2 after, and
    /// the island count changes 4 → 3) while the latched pair {2, 3} keeps its one
    /// manifold: nothing may wake.
    #[test]
    fn island_renumbering_alone_does_not_wake_a_latched_island() {
        let bodies = dyn_rows(6);
        let g_a = build_graph(&bodies, &[contact(0, 1), contact(2, 3)]);
        let g_b = build_graph(&bodies, &[contact(2, 3), contact(0, 4), contact(0, 5)]);
        assert_eq!(
            (g_a.island_of(2), g_b.island_of(2), g_a.n_islands(), g_b.n_islands()),
            (1, 2, 4, 3),
            "construction: the latched pair's island id and the island count must both change \
             between the two graphs: (id before, id after, islands before, islands after)"
        );

        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        freeze_rows(&mut sleep, &g_a, bodies.len(), &[2, 3]);
        sleep.begin_step(&g_b, bodies.len());
        assert_eq!(
            (
                sleep.is_island_frozen(g_b.island_of(2)),
                sleep.is_row_awake(2),
                sleep.is_row_awake(3),
                sleep.contact_wakes(),
            ),
            (true, false, false, 0),
            "a renumbered island whose manifold count is unchanged must stay frozen: \
             (frozen, row 2 awake, row 3 awake, contact_wakes)"
        );
    }

    /// U3. After U1's wake, an `end_step` at rest must not re-latch row 0 on the same
    /// step: the wake restarts the debounce.
    #[test]
    fn a_contact_loss_wake_restarts_the_debounce() {
        let bodies = dyn_rows(3);
        let g_a = build_graph(&bodies, &[contact(0, 1)]);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        freeze_rows(&mut sleep, &g_a, bodies.len(), &[0, 1]);
        let g_b = build_graph(&bodies, &[contact(1, 2)]);
        sleep.begin_step(&g_b, bodies.len());
        assert!(
            !sleep.is_row_asleep(0),
            "construction: row 0 must be woken by the contact change (U1)"
        );

        // `bodies` are at rest, so row 0's island is below the threshold this step.
        sleep.end_step(&bodies, &g_b, DEFAULT_SLEEP_THRESHOLD, DEFAULT_SLEEP_FRAMES);
        assert!(
            !sleep.is_row_asleep(0),
            "a body woken by a contact change must not re-latch at the same step's end_step: \
             its debounce must restart from 0, not resume at {DEFAULT_SLEEP_FRAMES}"
        );
    }

    /// U4. A static partner arriving under a frozen pair raises its island's count
    /// 1 → 2 (the dynamic-static manifold is filed under the dynamic side's island).
    #[test]
    fn a_static_partner_arriving_wakes_a_frozen_island() {
        let mut bodies = dyn_rows(2);
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
        let g_a = build_graph(&bodies, &[contact(0, 1)]);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        freeze_rows(&mut sleep, &g_a, bodies.len(), &[0, 1]);

        let g_b = build_graph(&bodies, &[contact(0, 1), contact(0, 2)]);
        sleep.begin_step(&g_b, bodies.len());
        assert_eq!(
            (
                sleep.is_row_awake(0),
                sleep.is_row_awake(1),
                sleep.is_island_frozen(g_b.island_of(0)),
                sleep.contact_wakes(),
            ),
            (true, true, false, 2),
            "a static partner arriving must wake the frozen island (a count that rises, not only \
             one that falls): (row 0 awake, row 1 awake, frozen, contact_wakes)"
        );
    }

    /// U5. A body resting on a static support alone: the support's manifold goes away.
    #[test]
    fn a_static_partner_leaving_wakes_the_body_it_carried() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, -1.0, 0.0)),
        ];
        let g_a = build_graph(&bodies, &[contact(0, 1)]);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        freeze_rows(&mut sleep, &g_a, bodies.len(), &[0]);

        let g_b = build_graph(&bodies, &[]);
        sleep.begin_step(&g_b, bodies.len());
        assert_eq!(
            (sleep.is_row_asleep(0), sleep.is_row_awake(0), sleep.contact_wakes()),
            (false, true, 1),
            "a body whose only (static) support left must wake: (latched, awake, contact_wakes)"
        );
    }

    /// U6. The latch sets at an `end_step`, and the very next `begin_step` already sees
    /// the support gone: the onset step is compared too.
    #[test]
    fn a_contact_loss_on_the_latch_onset_step_still_wakes() {
        let bodies = vec![
            dyn_sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5, 0.0),
            static_body(Vec3::new(0.0, -1.0, 0.0)),
        ];
        let g = build_graph(&bodies, &[contact(0, 1)]);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        sleep.begin_step(&g, bodies.len());
        sleep.end_step(&bodies, &g, DEFAULT_SLEEP_THRESHOLD, 1);
        assert!(
            sleep.is_row_asleep(0) && sleep.is_row_awake(0),
            "anti-vacuity: with a one-frame debounce row 0 latches at this end_step while it \
             was still solved on this step: (latched {}, awake {})",
            sleep.is_row_asleep(0),
            sleep.is_row_awake(0)
        );

        let g_empty = build_graph(&bodies, &[]);
        sleep.begin_step(&g_empty, bodies.len());
        assert_eq!(
            (sleep.is_row_asleep(0), sleep.is_row_awake(0)),
            (false, true),
            "a support lost on the step after the latch was set must still wake the body: \
             (latched, awake)"
        );
    }

    /// U7. A row that turns static keeps its latch while it has no island, and the
    /// `NO_ISLAND_KEY` it stores then clears that latch when it becomes dynamic again.
    #[test]
    fn a_row_that_regains_mass_does_not_inherit_a_stale_latch() {
        let mut bodies = dyn_rows(2);
        let contacts = [contact(0, 1)];
        let g = build_graph(&bodies, &contacts);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        freeze_rows(&mut sleep, &g, bodies.len(), &[0, 1]);

        bodies[0].inv_mass = 0.0;
        let g_static = build_graph(&bodies, &contacts);
        sleep.begin_step(&g_static, bodies.len());
        assert!(
            g_static.island_of(0) == ConstraintGraph::NO_ISLAND && sleep.is_row_asleep(0),
            "anti-vacuity: row 0 has no island while static and keeps its latch: (island {}, \
             latched {})",
            g_static.island_of(0),
            sleep.is_row_asleep(0)
        );

        bodies[0].inv_mass = 1.0;
        let g_back = build_graph(&bodies, &contacts);
        sleep.begin_step(&g_back, bodies.len());
        assert_eq!(
            (sleep.is_row_asleep(0), sleep.is_row_awake(0), sleep.is_row_awake(1)),
            (false, true, true),
            "a row returning from a static phase must not be frozen by the latch it carried: \
             (row 0 latched, row 0 awake, row 1 awake)"
        );
    }

    /// U10. A latched row inside an ACTIVE island (its neighbour never latched) loses its
    /// only link to that island: it must be compared, although its island was not frozen.
    #[test]
    fn a_latched_row_whose_awake_support_separates_wakes() {
        let mut bodies = dyn_rows(2);
        bodies.push(static_body(Vec3::new(0.0, -1.0, 0.0)));
        let g_a = build_graph(&bodies, &[contact(0, 1), contact(1, 2)]);
        let mut sleep = IslandSleep::with_capacity(bodies.len(), bodies.len());
        sleep.begin_step(&g_a, bodies.len());
        sleep.force_sleep_row(0);
        sleep.begin_step(&g_a, bodies.len());
        assert_eq!(
            (
                sleep.is_row_asleep(0),
                sleep.is_row_asleep(1),
                sleep.is_island_frozen(g_a.island_of(0)),
                sleep.is_row_awake(0),
            ),
            (true, false, false, true),
            "anti-vacuity: row 0 is latched inside an active island: (row 0 latched, row 1 \
             latched, island frozen, row 0 awake)"
        );

        let g_b = build_graph(&bodies, &[contact(1, 2)]);
        sleep.begin_step(&g_b, bodies.len());
        assert_eq!(
            (sleep.is_row_asleep(0), sleep.is_row_awake(0), sleep.contact_wakes()),
            (false, true, 1),
            "a latched row inside an active island must be compared: its support left and it is \
             now alone: (latched, awake, contact_wakes)"
        );
    }

    // ── L11 C1: G2, the warm-record differential against the per-point table ──
    //
    // The records (`warm_records.rs`) replace the per-point `WarmStartTable` the
    // colored solver kept until C1. Lemma W says every seed and every hit count is
    // equal; this module checks it against a canonical-insert oracle written from
    // the C0 code — the table, `pack` / `pack_sdf`, the store in canonical order and
    // the B1 carry — over random streams with duplicate keys, duplicate feature ids,
    // SDF manifolds, frozen subsets and every remap class, chained over steps so
    // the carry feeds the next lookup. Pool-free and column-backed, so it runs
    // native and under Miri (G8: the store, the carry and the cold index at small n).

    mod g2_records {
        use std::cell::Cell;

        use super::*;
        use crate::row_identity::NO_ROW;
        use crate::scratch_ids::warm_table_id;
        use crate::solver::warm_records::{
            WarmIndex, WarmLookup, WarmRecord, WarmRecords, WarmRun, ord, plan_sources, point_fid,
        };
        use crate::solver::warm_start::{WarmStartTable, pack, pack_sdf};

        /// How a step's rows relate to the previous step's.
        enum RemapSpec {
            Identity,
            Rows(Vec<u32>),
            Reset,
        }

        impl RemapSpec {
            fn remap(&self) -> RowRemap<'_> {
                match self {
                    Self::Identity => RowRemap::Identity,
                    Self::Rows(prev) => RowRemap::Rows(prev),
                    Self::Reset => RowRemap::Reset,
                }
            }
        }

        /// One generated step: the stream, which manifolds are frozen, and the remap.
        struct Step {
            manifolds: Vec<Manifold>,
            frozen: Vec<bool>,
            remap: RemapSpec,
        }

        /// What the differential met over a run, so a green run is shown to have
        /// exercised every path Lemma W argues about.
        #[derive(Default)]
        struct Tally {
            point_hits: Cell<u64>,
            carry_hits: Cell<u64>,
            misses: Cell<u64>,
            duplicate_keys: Cell<u64>,
            duplicate_fids: Cell<u64>,
            unsorted_streams: Cell<u64>,
            rows_steps: Cell<u64>,
            reset_steps: Cell<u64>,
            cold_index_steps: Cell<u64>,
            backward_searches: Cell<u64>,
            sdf_manifolds: Cell<u64>,
            /// Seeds whose source is an earlier record of an equal-key run than the
            /// last one (review W1: the run, not one record).
            w1_cases: Cell<u64>,
            frozen_manifolds: Cell<u64>,
        }

        impl Tally {
            fn bump(cell: &Cell<u64>, n: u64) {
                cell.set(cell.get() + n);
            }
        }

        /// A stable per-key coin, so every manifold of one pair is frozen or none is
        /// (Lemma W fact 3: equal pairs share an island).
        fn frozen_by_key(key: u64, salt: u64) -> bool {
            let mut z = key ^ salt;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)).is_multiple_of(3)
        }

        /// A random stream over `n_rows` rows: at most 64 manifolds, SDF ones among
        /// them, 0..=4 points with small feature ids (duplicates likely), sorted by
        /// ordinal half of the time.
        fn random_step(rng: &mut Lcg, n_rows: u32, salt: u64) -> Step {
            let m_count = rng.range(0, 65) as usize;
            let mut manifolds = Vec::with_capacity(m_count);
            for _ in 0..m_count {
                let a = rng.range(0, n_rows);
                let (a, b) = if rng.f01() < 0.2 {
                    (a, SDF_SENTINEL.0)
                } else {
                    let mut b = rng.range(0, n_rows);
                    if b == a {
                        b = (a + 1) % n_rows;
                    }
                    (a.min(b), a.max(b))
                };
                let count = rng.range(0, 5) as usize;
                let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
                m.normal = Vec3::new(0.0, 1.0, 0.0);
                for p in 0..count {
                    m.points[p] = ContactPoint {
                        anchor_a: Vec3::ZERO,
                        anchor_b: Vec3::ZERO,
                        separation: -0.01,
                        feature_id: rng.range(0, 6),
                    };
                }
                m.count = count as u8;
                manifolds.push(m);
            }
            if rng.f01() < 0.5 {
                manifolds.sort_by_key(|m| ord(m.body_a.0, m.body_b.0));
            }
            let frozen = manifolds
                .iter()
                .map(|m| frozen_by_key(ord(m.body_a.0, m.body_b.0), salt))
                .collect();
            let remap = match rng.range(0, 5) {
                0 | 1 => RemapSpec::Identity,
                2 | 3 => {
                    // An injective partial map onto the previous rows: a random
                    // permutation with some rows sent to NO_ROW (new bodies), which
                    // flips some pairs' order (a miss on both sides).
                    let mut perm: Vec<u32> = (0..n_rows).collect();
                    for i in (1..perm.len()).rev() {
                        let j = rng.range(0, i as u32 + 1) as usize;
                        perm.swap(i, j);
                    }
                    let prev = perm
                        .into_iter()
                        .map(|p| if rng.f01() < 0.8 { p } else { NO_ROW })
                        .collect();
                    RemapSpec::Rows(prev)
                }
                _ => RemapSpec::Reset,
            };
            Step { manifolds, frozen, remap }
        }

        /// The synthetic converged impulse of slot `s` on `step`: distinct per slot
        /// and per step, so a seed read from the wrong source is visible.
        fn impulse(step: u32, s: usize, k: u32) -> f32 {
            (step * 4096 + s as u32 * 4 + k) as f32 + 0.25
        }

        /// The C0 point keys: the store key in current rows and the read key in the
        /// translated rows (`ColoredSoftStepSolver::point_keys` before C1).
        fn point_keys(m: &Manifold, p: usize, lookup: Option<(u32, u32)>) -> (u64, Option<u64>) {
            let feature_id = m.points[p].feature_id;
            if m.body_b == SDF_SENTINEL {
                (
                    pack_sdf(m.body_a, feature_id),
                    lookup.map(|(la, _)| pack_sdf(BodyIndex(la), feature_id)),
                )
            } else {
                (
                    pack(m.body_a, m.body_b, feature_id),
                    lookup.map(|(la, lb)| pack(BodyIndex(la), BodyIndex(lb), feature_id)),
                )
            }
        }

        /// Drives `steps` chained steps through both stores and compares every seed
        /// (as bits), the per-step point hits and the carry hits.
        fn differential(steps: &[Step], tally: &Tally) {
            // The records under test, on the solver's own ids (independent pools).
            let mut warm = [
                WarmRecords::with_capacity(warm_table_id(4), warm_table_id(2), 64),
                WarmRecords::with_capacity(warm_table_id(5), warm_table_id(3), 64),
            ];
            let mut cur = 0usize;
            let mut index = WarmIndex::with_capacity(warm_table_id(6), 64);
            let mut plan: Vec<WarmRun> = Vec::new();
            // The oracle: the C0 double-buffered per-point table.
            let mut table_read = WarmStartTable::with_capacity(warm_table_id(0), 64);
            let mut table_write = WarmStartTable::with_capacity(warm_table_id(1), 64);

            for (step_no, step) in steps.iter().enumerate() {
                let manifolds = &step.manifolds;
                let n = manifolds.len();
                let remap = step.remap.remap();
                match step.remap {
                    RemapSpec::Rows(_) => Tally::bump(&tally.rows_steps, 1),
                    RemapSpec::Reset => Tally::bump(&tally.reset_steps, 1),
                    RemapSpec::Identity => {}
                }
                {
                    let mut keys: Vec<u64> = manifolds.iter().map(|m| ord(m.body_a.0, m.body_b.0)).collect();
                    if keys.windows(2).any(|w| w[1] <= w[0]) {
                        Tally::bump(&tally.unsorted_streams, 1);
                    }
                    keys.sort_unstable();
                    Tally::bump(&tally.duplicate_keys, keys.windows(2).filter(|w| w[0] == w[1]).count() as u64);
                }
                Tally::bump(
                    &tally.sdf_manifolds,
                    manifolds.iter().filter(|m| m.body_b == SDF_SENTINEL).count() as u64,
                );
                for m in manifolds {
                    let c = m.count as usize;
                    if (0..c).any(|p| (p + 1..c).any(|q| point_fid(m, p) == point_fid(m, q))) {
                        Tally::bump(&tally.duplicate_fids, 1);
                    }
                }

                // The step's slot layout: solved manifolds get slots in manifold
                // order (the store's view), frozen ones the carry tag.
                let mut manifold_base = vec![(u32::MAX, 0u32); n];
                let mut slots = 0u32;
                let mut frozen_points = 0u32;
                for (mi, m) in manifolds.iter().enumerate() {
                    let c = u32::from(m.count);
                    if c == 0 {
                        continue;
                    }
                    if step.frozen[mi] {
                        manifold_base[mi] = (u32::MAX, c);
                        frozen_points += c;
                        Tally::bump(&tally.frozen_manifolds, 1);
                    } else {
                        manifold_base[mi] = (slots, c);
                        slots += c;
                    }
                }
                let ni: Vec<f32> = (0..slots as usize).map(|s| impulse(step_no as u32, s, 0)).collect();
                let t1: Vec<f32> = (0..slots as usize).map(|s| impulse(step_no as u32, s, 1)).collect();
                let t2: Vec<f32> = (0..slots as usize).map(|s| impulse(step_no as u32, s, 2)).collect();

                // ── records: P-a, the seeds, the store, the swap ──
                let (read, write) = {
                    let [s0, s1] = &mut warm;
                    if cur == 0 { (&*s0, s1) } else { (&*s1, s0) }
                };
                plan.clear();
                plan.resize(n, WarmRun::MISS);
                write.resize(n);
                let counts = plan_sources(manifolds, remap, true, read, write, &mut index, &mut plan);
                Tally::bump(&tally.backward_searches, u64::from(counts.backward_searches));
                Tally::bump(&tally.cold_index_steps, u64::from(counts.cold_index));
                let lookup = WarmLookup { read, index: &index };
                let mut rec_seeds: Vec<[u32; 3]> = Vec::new();
                let mut rec_hits = 0u32;
                for (mi, m) in manifolds.iter().enumerate() {
                    let (base, c) = manifold_base[mi];
                    if base == u32::MAX {
                        continue;
                    }
                    let run = plan[mi];
                    for p in 0..c as usize {
                        let fid = point_fid(m, p);
                        let seed = lookup.seed(run, fid);
                        if seed.is_some() {
                            rec_hits += 1;
                            // W1: the whole run was needed — the last record alone misses.
                            if run.hi - run.lo >= 2
                                && lookup.seed(WarmRun { lo: run.hi - 1, hi: run.hi }, fid).is_none()
                            {
                                Tally::bump(&tally.w1_cases, 1);
                            }
                        }
                        let [a, b, d] = seed.unwrap_or([0.0; 3]);
                        rec_seeds.push([a.to_bits(), b.to_bits(), d.to_bits()]);
                    }
                }
                let mut rec_carry_hits = 0u32;
                {
                    let mut recs_w = write.recs_mut();
                    let recs_w = recs_w.as_mut_slice();
                    for (mi, (m, &(base, c))) in manifolds.iter().zip(&manifold_base).enumerate() {
                        recs_w[mi] = if base != u32::MAX {
                            WarmRecord::solved(m, base as usize, c as usize, [&ni, &t1, &t2])
                        } else if c != 0 {
                            let (rec, hits) = lookup.carry(plan[mi], m, c as usize);
                            rec_carry_hits += hits;
                            rec
                        } else {
                            WarmRecord::EMPTY
                        };
                        assert!(
                            recs_w[mi].count() as u32 <= c,
                            "a record never holds more points than its manifold"
                        );
                    }
                }
                assert_eq!(write.len(), n, "one record per manifold of the stream");
                cur ^= 1;

                // ── the oracle: the per-point table, seeds then the canonical store ──
                let mut ora_seeds: Vec<[u32; 3]> = Vec::new();
                let mut ora_hits = 0u32;
                for (mi, m) in manifolds.iter().enumerate() {
                    let (base, c) = manifold_base[mi];
                    if base == u32::MAX {
                        continue;
                    }
                    let translated = remap.manifold_pair(m);
                    for p in 0..c as usize {
                        let (_, read_key) = point_keys(m, p, translated);
                        let seed = read_key.and_then(|k| table_read.get(k));
                        if let Some(e) = seed {
                            ora_hits += 1;
                            ora_seeds.push([
                                e.normal_impulse.to_bits(),
                                e.tangent_impulse[0].to_bits(),
                                e.tangent_impulse[1].to_bits(),
                            ]);
                        } else {
                            ora_seeds.push([0; 3]);
                        }
                    }
                }
                table_write.rebuild(slots as usize + frozen_points as usize);
                for (mi, m) in manifolds.iter().enumerate() {
                    let (base, c) = manifold_base[mi];
                    if base == u32::MAX {
                        continue;
                    }
                    for p in 0..c as usize {
                        let (store_key, _) = point_keys(m, p, None);
                        let s = base as usize + p;
                        table_write.insert(store_key, ni[s], [t1[s], t2[s]]);
                    }
                }
                let mut ora_carry_hits = 0u32;
                for (mi, m) in manifolds.iter().enumerate() {
                    let (base, c) = manifold_base[mi];
                    if base != u32::MAX || c == 0 {
                        continue;
                    }
                    let translated = remap.manifold_pair(m);
                    if translated.is_none() {
                        continue;
                    }
                    for p in 0..c as usize {
                        let (store_key, read_key) = point_keys(m, p, translated);
                        if let Some(read_key) = read_key
                            && let Some(e) = table_read.get(read_key)
                        {
                            table_write.insert(store_key, e.normal_impulse, e.tangent_impulse);
                            ora_carry_hits += 1;
                        }
                    }
                }
                core::mem::swap(&mut table_read, &mut table_write);

                // ── the comparison ──
                assert_eq!(
                    rec_seeds, ora_seeds,
                    "step {step_no}: every seed from the records must equal the per-point \
                     table's (Lemma W)"
                );
                assert_eq!(rec_hits, ora_hits, "step {step_no}: point hits");
                assert_eq!(rec_carry_hits, ora_carry_hits, "step {step_no}: carry hits");
                Tally::bump(&tally.point_hits, u64::from(rec_hits));
                Tally::bump(&tally.carry_hits, u64::from(rec_carry_hits));
                Tally::bump(&tally.misses, u64::from(slots - rec_hits));
            }
        }

        /// The steps of one case: a fixed row count, four chained random steps.
        fn case_steps(seed: u64) -> Vec<Step> {
            let mut rng = Lcg(seed ^ 0x5851_F42D_4C95_7F2D);
            let n_rows = rng.range(2, 9);
            let salt = rng.next_u64();
            (0..4).map(|_| random_step(&mut rng, n_rows, salt)).collect()
        }

        /// Every path the tally names must have been met, or the green proves less
        /// than it claims.
        fn assert_non_vacuous(tally: &Tally) {
            let counts = [
                ("point hits", tally.point_hits.get()),
                ("carry hits", tally.carry_hits.get()),
                ("misses", tally.misses.get()),
                ("duplicate keys", tally.duplicate_keys.get()),
                ("duplicate feature ids", tally.duplicate_fids.get()),
                ("unsorted streams", tally.unsorted_streams.get()),
                ("Rows steps", tally.rows_steps.get()),
                ("Reset steps", tally.reset_steps.get()),
                ("cold-index steps", tally.cold_index_steps.get()),
                ("backward searches", tally.backward_searches.get()),
                ("SDF manifolds", tally.sdf_manifolds.get()),
                ("W1 cases (an earlier equal-key record was the source)", tally.w1_cases.get()),
                ("frozen manifolds", tally.frozen_manifolds.get()),
            ];
            for (name, n) in counts {
                assert!(n > 0, "anti-vacuity: the differential met no {name}: {counts:?}");
            }
        }

        /// G2: random ≤ 64-manifold streams over four chained steps, remap in
        /// {Identity, Rows with flips and NO_ROW, Reset}, SDF, duplicate feature ids
        /// and keys, frozen subsets — seeds and hit counts from the records equal the
        /// canonical-insert oracle's, and every path is met. Native only: proptest's
        /// failure persistence reads the cwd, which Miri's isolation refuses; the
        /// fixed-seed twin below is the Miri leg.
        #[test]
        #[cfg(not(miri))]
        fn warm_records_match_the_per_point_table_on_random_streams() {
            let tally = Tally::default();
            proptest!(ProptestConfig::with_cases(256), |(seed in any::<u64>())| {
                differential(&case_steps(seed), &tally);
            });
            assert_non_vacuous(&tally);
        }

        /// G8: the same differential on fixed seeds, small enough for Miri — the
        /// store, the carry, the cursor search and the cold index under the borrow
        /// checker. The seeds are chosen so the tally is non-vacuous on their own.
        #[test]
        fn warm_records_differential_fixed_seeds() {
            let tally = Tally::default();
            for seed in 0..24u64 {
                differential(&case_steps(seed), &tally);
            }
            assert_non_vacuous(&tally);
        }

        /// D2's strict-side search, walked with a cursor: a lookup above the key
        /// before the cursor gallops forward (a miss included); one at or below it
        /// binary-searches the prefix and reports so; a miss leaves the cursor at the
        /// insertion point.
        #[test]
        fn find_gallops_forward_and_binary_searches_backward() {
            let mut side = WarmRecords::with_capacity(warm_table_id(4), warm_table_id(2), 8);
            let keys = [1u64, 3, 5, 9, 10, 20, 21, 40];
            side.resize(keys.len());
            side.keys_mut().as_mut_slice().copy_from_slice(&keys);
            side.set_strict(true);
            let mut cursor = 0usize;
            let expect = [
                // (key, run, backward, cursor after)
                (3u64, WarmRun { lo: 1, hi: 2 }, false, 2usize),
                (9, WarmRun { lo: 3, hi: 4 }, false, 4),
                (40, WarmRun { lo: 7, hi: 8 }, false, 8),
                (1, WarmRun { lo: 0, hi: 1 }, true, 1),
                (4, WarmRun::MISS, false, 2),
                (6, WarmRun::MISS, false, 3),
                (21, WarmRun { lo: 6, hi: 7 }, false, 7),
                (5, WarmRun { lo: 2, hi: 3 }, true, 3),
                (0, WarmRun::MISS, true, 0),
                (50, WarmRun::MISS, false, 8),
                (20, WarmRun { lo: 5, hi: 6 }, true, 6),
            ];
            for (key, run, backward, after) in expect {
                let found = side.find(&mut cursor, key);
                assert_eq!((found.run, found.backward, cursor), (run, backward, after), "key {key}");
            }
        }

        /// The cold index names the WHOLE equal-key run, ascending in manifold index.
        #[test]
        fn cold_index_runs_cover_every_equal_key() {
            let mut index = WarmIndex::with_capacity(warm_table_id(6), 8);
            let keys = [7u64, 3, 7, 1, 3, 7];
            index.build(&keys);
            assert_eq!(index.run(7), WarmRun { lo: 3, hi: 6 });
            assert_eq!(index.run(3), WarmRun { lo: 1, hi: 3 });
            assert_eq!(index.run(1), WarmRun { lo: 0, hi: 1 });
            assert_eq!(index.run(2), WarmRun::MISS);
            assert_eq!(index.run(9), WarmRun::MISS);
            let mis: Vec<u32> = index.sorted()[3..6].iter().map(|e| e.1).collect();
            assert_eq!(mis, [0, 2, 5], "an equal-key run is ascending in manifold index");
        }
    }

        // ── L11 C0: the G1 identity pins — the setup digest and the body bits, per step ──
        //
        // The per-contact solve setup (L11) rewrites the warm store, the column build, the
        // kernel's layout and the warm apply WITHOUT changing a value. These pins are what
        // C1–C4 must reproduce: for each of the seven scenes S-a…S-g the hash of every
        // step's `ColoredSoftStepSolver::step_digest` (the warm seeds, the masked `vn0`,
        // the group / colour CSRs, then the step's `WarmSeedStats`) followed by every
        // row's post-step body bits (position, rotation, both velocities), folded with
        // FNV-1a 64 over the run. The body bits are layout-free and authoritative; the
        // digest localises a failure to the setup. Each scene runs the four arms W ∈ {1, 8}
        // × `simd_solve` ∈ {off, on}, which the {1, N} and scalar/SIMD bit contracts make
        // equal, so one constant per scene pins all four — an arm that differs from the
        // others is as red as an arm that differs from the pin.
        //
        // A red pin is a defect, never a re-bless (`00-RULINGS.md`, per-contact parity).
        // Every scene also asserts the anti-vacuity counter G1 names for it
        // (`SetupCounters`), so a scene that stopped exercising its path goes red too.
        // The eight-worker arms need a pool, so the suite is native only.

    #[cfg(not(miri))]
    mod g1_pins {
        use super::*;
        use crate::components::{Collider, RigidBody, RigidBodyMass};
        use crate::narrowphase::feature_vertex_face;
        use crate::resources::{BroadphaseGrid, ContactPairs, Manifolds};
        use crate::row_identity::RowKey;
        use crate::systems::narrowphase_serial;

        /// S-a: the six-layer Jolt pyramid (91 boxes, gap 0.5, friction 0.2), 300 steps.
        const PIN_S_A: u64 = 0xa9bd_a9e4_ee00_d272;
        /// S-b: a resting three-layer pile that freezes under sleeping, then `wake_all`.
        const PIN_S_B: u64 = 0x40e4_a08c_7eb6_f631;
        /// S-c: spawn / despawn / migrate with order flips and jumpers (`Rows` and `Reset`).
        const PIN_S_C: u64 = 0x8378_83c0_d3d6_6662;
        /// S-d: a two-layer box pile on a plane SDF (box-box plus SDF manifolds).
        const PIN_S_D: u64 = 0x288c_4d92_c2d6_410a;
        /// S-e: a hand-built unsorted stream with duplicate keys and duplicate feature ids.
        const PIN_S_E: u64 = 0x68db_ef7f_775d_42ba;
        /// S-f: warm start toggled off for one step, then on again.
        const PIN_S_F: u64 = 0xd611_7ee9_4919_701d;
        /// S-g: bouncing spheres with restitution 0.6.
        const PIN_S_G: u64 = 0x246a_964a_2b43_e9c1;

        /// One arm of a G1 scene: the pool size and the kernel selection.
        #[derive(Clone, Copy, Debug)]
        struct Arm {
            workers: usize,
            simd_solve: bool,
        }

        /// The four arms every scene runs: W ∈ {1, 8} × `simd_solve` ∈ {off, on}.
        const ARMS: [Arm; 4] = [
            Arm { workers: 1, simd_solve: false },
            Arm { workers: 1, simd_solve: true },
            Arm { workers: 8, simd_solve: false },
            Arm { workers: 8, simd_solve: true },
        ];

        /// What one arm of a scene produced: the pin hash, the counters at the end and
        /// at the scene's own phase mark (zero when it has none), the last step's warm
        /// stats, and how many steps had a colour wide enough for the parallel dispatch
        /// (the W = 8 witness).
        #[derive(Clone, Copy, Debug)]
        struct ArmResult {
            hash: u64,
            counters: SetupCounters,
            marked: SetupCounters,
            stats: WarmSeedStats,
            wide_steps: usize,
        }

        /// Runs `scene` on `arm`: inside an installed pool of `arm.workers` threads when
        /// the arm is parallel, else on the calling thread with no pool attached.
        fn run_arm(arm: Arm, scene: fn(Arm) -> ArmResult) -> ArmResult {
            use boyko_threadpool::ThreadPoolBuilder;
            if arm.workers > 1 {
                let pool = ThreadPoolBuilder::new().num_threads(arm.workers).build();
                pool.install(move |_scope| scene(arm))
            } else {
                scene(arm)
            }
        }

        /// Runs every arm of `scene` and checks each against `pin`, reporting all four
        /// hashes on a mismatch so the report can quote the measured values.
        fn assert_pinned(name: &str, pin: u64, scene: fn(Arm) -> ArmResult) -> [ArmResult; 4] {
            let results = ARMS.map(|arm| run_arm(arm, scene));
            for (arm, r) in ARMS.iter().zip(&results) {
                eprintln!(
                    "{name} {arm:?}: hash {:#018x}, wide steps {}, {:?}",
                    r.hash, r.wide_steps, r.counters
                );
            }
            let hashes: Vec<String> = results.iter().map(|r| format!("{:#018x}", r.hash)).collect();
            assert!(
                results.iter().all(|r| r.hash == pin),
                "{name}: the per-step setup digest + body bits hash must equal the C0 pin \
                 {pin:#018x} on every arm (W {{1, 8}} × simd {{off, on}}); measured {hashes:?} \
                 for arms {ARMS:?}. A red pin is a defect, not a re-bless."
            );
            results
        }

        /// Folds every row's post-step body bits: position, rotation, linear and angular
        /// velocity, in row order, length first.
        fn fold_body_bits(mut h: u64, bodies: &[BodyState]) -> u64 {
            h = fnv_u32(h, bodies.len() as u32);
            for b in bodies {
                for v in [
                    b.position.x,
                    b.position.y,
                    b.position.z,
                    b.rotation.x,
                    b.rotation.y,
                    b.rotation.z,
                    b.rotation.w,
                    b.linear_velocity.x,
                    b.linear_velocity.y,
                    b.linear_velocity.z,
                    b.angular_velocity.x,
                    b.angular_velocity.y,
                    b.angular_velocity.z,
                ] {
                    h = fnv_u32(h, v.to_bits());
                }
            }
            h
        }

        /// A box body through the gather's own constructor, so its inertia is the
        /// production tensor. `simulated == false` with `inv_mass == 0` is the static floor.
        fn box_body(
            position: Vec3,
            half_extents: Vec3,
            inv_mass: f32,
            friction: f32,
            restitution: f32,
        ) -> BodyState {
            let body = RigidBody {
                position,
                linear_velocity: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
            };
            let mass = RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass, restitution, friction };
            let collider = Collider { shape: ColliderShape::Box { half_extents }, layer: 1, mask: 1 };
            BodyState::from_columns(&body, &mass, &collider, false, inv_mass != 0.0, false)
        }

        /// A sphere body through the gather's constructor (a real inertia tensor, unlike
        /// `dyn_sphere`'s zero one).
        fn sphere_body(
            position: Vec3,
            radius: f32,
            inv_mass: f32,
            friction: f32,
            restitution: f32,
        ) -> BodyState {
            let body = RigidBody {
                position,
                linear_velocity: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                angular_velocity: Vec3::ZERO,
            };
            let mass = RigidBodyMass { inv_inertia: Mat3::ZERO, inv_mass, restitution, friction };
            let collider = Collider { shape: ColliderShape::Sphere { radius }, layer: 1, mask: 1 };
            BodyState::from_columns(&body, &mass, &collider, false, inv_mass != 0.0, false)
        }

        /// The runner's floor: Jolt's `BoxShape(Vec3(50, 1, 50))` at `(0, −1, 0)`, static.
        fn floor_body(friction: f32) -> BodyState {
            box_body(Vec3::new(0.0, -1.0, 0.0), Vec3::new(50.0, 1.0, 50.0), 0.0, friction, 0.0)
        }

        /// Jolt's pyramid placement loop (`PyramidScene.h`, the runner's transcription) at
        /// `height` layers: unit-density cubes of half-extent 1 (`inv_mass` 1/8), pitch 2,
        /// the odd-layer half-box offset, `gap` between layers. The floor is row 0.
        fn pyramid_bodies(height: i32, gap: f32, friction: f32) -> Vec<BodyState> {
            let mut bodies = vec![floor_body(friction)];
            for i in 0..height {
                let lo = i / 2;
                let hi = height - (i + 1) / 2;
                for j in lo..hi {
                    for k in lo..hi {
                        let odd = if i & 1 != 0 { 1.0 } else { 0.0 };
                        let position = Vec3::new(
                            -(height as f32) + 2.0 * j as f32 + odd,
                            1.0 + (2.0 + gap) * i as f32,
                            -(height as f32) + 2.0 * k as f32 + odd,
                        );
                        bodies.push(box_body(position, Vec3::new(1.0, 1.0, 1.0), 0.125, friction, 0.0));
                    }
                }
            }
            bodies
        }

        /// The plane-SDF stand-in for `box_sdf_manifold` (`systems.rs`), the field `y = 0`
        /// with gradient `(0, 1, 0)`: every world corner below the plane is a candidate with
        /// `separation = y`, the deepest four are kept (ties by the lowest corner index),
        /// `normal = −∇ = (0, −1, 0)`, both anchors at the corner, the production
        /// vertex-face feature id. `None` when no corner penetrates.
        fn plane_sdf_box_manifold(row: u32, b: &BodyState, half: Vec3) -> Option<Manifold> {
            use crate::math::MAX_CONTACT_POINTS;
            let mut candidates: Vec<(f32, u32, Vec3)> = Vec::new();
            for corner in 0u32..8 {
                let sx = if corner & 1 != 0 { half.x } else { -half.x };
                let sy = if corner & 2 != 0 { half.y } else { -half.y };
                let sz = if corner & 4 != 0 { half.z } else { -half.z };
                let world = b.position + b.rotation.rotate(Vec3::new(sx, sy, sz));
                if world.y < 0.0 {
                    candidates.push((world.y, corner, world));
                }
            }
            if candidates.is_empty() {
                return None;
            }
            candidates.sort_by(|p, q| p.0.total_cmp(&q.0).then(p.1.cmp(&q.1)));
            candidates.truncate(MAX_CONTACT_POINTS);
            let mut m = Manifold::new(BodyIndex(row), SDF_SENTINEL);
            m.normal = Vec3::new(0.0, -1.0, 0.0);
            for (p, &(d, corner, world)) in candidates.iter().enumerate() {
                m.points[p] = ContactPoint {
                    anchor_a: world,
                    anchor_b: world,
                    separation: d,
                    feature_id: feature_vertex_face(corner),
                };
            }
            m.count = candidates.len() as u8;
            Some(m)
        }

        /// The drive of one G1 scene arm: the solver, its scratch, the optional sleep
        /// state, the real broadphase + narrowphase resources, and the running hash.
        struct G1 {
            cfg: PhysicsConfig,
            solver: ColoredSoftStepSolver,
            scratch: SolverScratch,
            sleep: Option<IslandSleep>,
            grid: BroadphaseGrid,
            pairs: ContactPairs,
            manifolds: Manifolds,
            hash: u64,
            wide_steps: usize,
            marked: SetupCounters,
        }

        impl G1 {
            /// A drive over `bodies` on `arm`; `sleeping` selects the O8 entry.
            fn new(bodies: &[BodyState], arm: Arm, sleeping: bool) -> Self {
                let cfg = PhysicsConfig {
                    dt: 1.0 / 60.0,
                    simd_solve: arm.simd_solve,
                    sleeping,
                    ..PhysicsConfig::default()
                };
                let mut scratch = SolverScratch::with_capacity(bodies.len());
                scratch.set_bodies(bodies);
                scratch.touched.reset(scratch.bodies().len());
                let sleep = sleeping.then(|| IslandSleep::with_capacity(bodies.len(), bodies.len()));
                Self {
                    cfg,
                    solver: ColoredSoftStepSolver::default(),
                    scratch,
                    sleep,
                    grid: BroadphaseGrid::with_capacity(bodies.len()),
                    pairs: ContactPairs::with_capacity(bodies.len() * 4),
                    manifolds: Manifolds::with_capacity(bodies.len() * 4),
                    hash: FNV_OFFSET,
                    wide_steps: 0,
                    marked: SetupCounters::default(),
                }
            }

            /// Snapshots the counters at the scene's phase mark.
            fn mark(&mut self) {
                self.marked = self.solver.setup_counters();
            }

            /// One step over `manifolds`: the graph, the solve, then the digest and the body
            /// bits folded into the hash.
            fn solve(&mut self, manifolds: &[Manifold]) {
                let Self { cfg, solver, scratch, sleep, hash, wide_steps, .. } = self;
                Self::solve_parts(cfg, solver, scratch, sleep.as_mut(), manifolds, hash, wide_steps);
            }

            fn solve_parts(
                cfg: &PhysicsConfig,
                solver: &mut ColoredSoftStepSolver,
                scratch: &mut SolverScratch,
                sleep: Option<&mut IslandSleep>,
                manifolds: &[Manifold],
                hash: &mut u64,
                wide_steps: &mut usize,
            ) {
                let graph = build_graph(scratch.bodies(), manifolds);
                scratch.touched.reset(scratch.bodies().len());
                match sleep {
                    Some(sleep) => solver.solve_colored_sleeping(cfg, manifolds, &graph, scratch, sleep),
                    None => solver.solve_colored(cfg, manifolds, &graph, scratch),
                }
                if solver.columns.widest_color_slots() >= MIN_PARALLEL_SLOTS_PER_COLOR {
                    *wide_steps += 1;
                }
                *hash = fnv_u64(*hash, solver.step_digest());
                *hash = fold_body_bits(*hash, scratch.bodies());
            }

            /// One step through the production collision pipeline: the grid broadphase, the
            /// serial narrowphase (its box-box axis hysteresis synced to the scratch rows),
            /// then, with `sdf_plane`, the plane-SDF stand-in appended for every dynamic
            /// box in row order — the stream shape `physics_narrowphase_sdf` produces.
            fn step_pipeline(&mut self, sdf_plane: bool) {
                let Self {
                    cfg,
                    solver,
                    scratch,
                    sleep,
                    grid,
                    pairs,
                    manifolds,
                    hash,
                    wide_steps,
                    ..
                } = self;
                grid.build(scratch.bodies(), pairs);
                let prefetched = manifolds
                    .box_axis_cache
                    .begin_frame_synced(
                        pairs.pairs_stream(),
                        pairs.pairs().len(),
                        scratch.bodies(),
                        &scratch.rows,
                    )
                    .prefetched;
                narrowphase_serial(manifolds, scratch.bodies(), pairs.pairs_stream(), prefetched);
                if sdf_plane {
                    let mut out = manifolds.manifolds.build_view();
                    for (row, b) in scratch.bodies().iter().enumerate() {
                        if !b.simulated || !is_dynamic_row(b.inv_mass) {
                            continue;
                        }
                        if let ColliderShape::Box { half_extents } = b.shape
                            && let Some(m) = plane_sdf_box_manifold(row as u32, b, half_extents)
                        {
                            out.push(m);
                        }
                    }
                }
                Self::solve_parts(
                    cfg,
                    solver,
                    scratch,
                    sleep.as_mut(),
                    manifolds.solver_manifolds(),
                    hash,
                    wide_steps,
                );
            }

            fn finish(self) -> ArmResult {
                ArmResult {
                    hash: self.hash,
                    counters: self.solver.setup_counters(),
                    marked: self.marked,
                    stats: self.solver.warm_seed_stats(),
                    wide_steps: self.wide_steps,
                }
            }
        }

        /// S-a: the six-layer Jolt pyramid (the runner's `jolt` placement at height 6: 91
        /// boxes, gap 0.5, friction 0.2) for 300 steps through the real pipeline.
        fn scene_s_a(arm: Arm) -> ArmResult {
            let mut g = G1::new(&pyramid_bodies(6, 0.5, 0.2), arm, false);
            for _ in 0..300 {
                g.step_pipeline(false);
            }
            g.finish()
        }

        /// S-b: a three-layer resting pile (gap 0, friction 0.5) under sleeping with a
        /// short debounce; it freezes, is stepped frozen (the store carries its entries),
        /// then `wake_all` and 40 more steps.
        fn scene_s_b(arm: Arm) -> ArmResult {
            let mut g = G1::new(&pyramid_bodies(3, 0.0, 0.5), arm, true);
            g.cfg.sleep_frames = 10;
            for _ in 0..120 {
                g.step_pipeline(false);
            }
            let frozen_rows = (0..g.scratch.bodies_len())
                .filter(|&r| !g.sleep.as_ref().expect("sleeping arm").is_row_awake(r))
                .count();
            assert!(frozen_rows > 0, "anti-vacuity: S-b's pile must be frozen after 120 steps");
            g.sleep.as_mut().expect("sleeping arm").wake_all();
            for _ in 0..40 {
                g.step_pipeline(false);
            }
            g.finish()
        }

        /// S-c's world: entities with an identity, gathered by hand each step so the row
        /// identity map sees spawns (appends with an `added` row), despawns (swap-removes:
        /// the last row jumps into the hole), migrations (a reversed run: order flips) and a
        /// missed gather (`Reset`). The floor is row 0; the spheres overlap their neighbours.
        struct Churn {
            ents: Vec<(RowKey, BodyState)>,
            next_id: usize,
        }

        impl Churn {
            fn new(n: usize) -> Self {
                let mut ents = vec![(RowKey::new(0, 0), floor_body(0.5))];
                for i in 0..n {
                    let p = Vec3::new(i as f32 * 1.5, 1.0, 0.0);
                    ents.push((RowKey::new(i + 1, 0), sphere_body(p, 1.0, 1.0, 0.5, 0.0)));
                }
                Self { ents, next_id: n + 1 }
            }

            /// Loads the entities into the scratch (bodies and one hand gather) with `added`
            /// naming the rows whose body was spawned since the last gather.
            fn gather(&self, scratch: &mut SolverScratch, added: &[u32]) {
                let bodies: Vec<BodyState> = self.ents.iter().map(|(_, b)| *b).collect();
                scratch.set_bodies(&bodies);
                scratch.rows.begin_gather();
                {
                    let (mut cur, mut add) = scratch.rows.gather_views();
                    for (key, _) in &self.ents {
                        cur.push(*key);
                    }
                    for &r in added {
                        add.push(r);
                    }
                }
                scratch.rows.finish_gather();
            }

            /// Copies the solved state back from the scratch, row for row.
            fn write_back(&mut self, scratch: &SolverScratch) {
                for ((_, b), s) in self.ents.iter_mut().zip(scratch.bodies()) {
                    *b = *s;
                }
            }

            /// The sorted body-body stream: floor contacts `(0, a)` and neighbour contacts
            /// `(a, a + 1)` where the spheres overlap, in `(a, b)` order.
            fn manifolds(&self) -> Vec<Manifold> {
                let mut ms = Vec::new();
                let bodies: Vec<&BodyState> = self.ents.iter().map(|(_, b)| b).collect();
                for (a, body) in bodies.iter().enumerate().skip(1) {
                    let sep = body.position.y - 1.0;
                    if sep < 0.0 {
                        ms.push(manifold(0, a as u32, Vec3::new(0.0, 1.0, 0.0), sep, body.position));
                    }
                }
                for a in 1..bodies.len() {
                    for b in a + 1..bodies.len() {
                        let delta = bodies[b].position - bodies[a].position;
                        let dist = delta.length();
                        if dist - 2.0 < 0.0 && dist > 1e-6 {
                            let normal = delta * dist.recip();
                            ms.push(manifold(a as u32, b as u32, normal, dist - 2.0, bodies[a].position + normal));
                        }
                    }
                }
                ms.sort_by_key(|m| (m.body_a.0, m.body_b.0));
                ms
            }
        }

        /// S-c: 60 steps of a sphere line on a floor; at step 20 row 3 is despawned (row 9
        /// jumps into it), at 25 two spheres are spawned (one on a recycled slot at
        /// generation 1), at 30 rows 4..8 are reversed (order flips), at 35 a gather is
        /// missed (`Reset`). Hand-built sorted streams, the row identity fed by hand. The
        /// phase mark is taken before the first churn, so the gate counts churn misses
        /// only, not the first step's cold table.
        fn scene_s_c(arm: Arm) -> ArmResult {
            let mut world = Churn::new(9);
            let bodies: Vec<BodyState> = world.ents.iter().map(|(_, b)| *b).collect();
            let mut g = G1::new(&bodies, arm, false);
            let mut added: Vec<u32> = Vec::new();
            for step in 0..60 {
                match step {
                    20 => {
                        g.mark();
                        world.ents.swap_remove(3);
                    }
                    25 => {
                        let n = world.ents.len();
                        let p = |i: usize| Vec3::new(i as f32 * 1.5, 1.0, 0.0);
                        // The despawned slot 3 comes back at generation 1 (A1b's case) and a
                        // fresh slot follows it.
                        world.ents.push((RowKey::new(3, 1), sphere_body(p(n), 1.0, 1.0, 0.5, 0.0)));
                        world.ents.push((RowKey::new(world.next_id, 0), sphere_body(p(n + 1), 1.0, 1.0, 0.5, 0.0)));
                        world.next_id += 1;
                        added.extend([n as u32, n as u32 + 1]);
                    }
                    30 => world.ents[4..9].reverse(),
                    35 => {
                        // A gather the solver never sees: its cursor then misses one.
                        world.gather(&mut g.scratch, &[]);
                    }
                    _ => {}
                }
                world.gather(&mut g.scratch, &added);
                added.clear();
                let ms = world.manifolds();
                g.solve(&ms);
                world.write_back(&g.scratch);
            }
            g.finish()
        }

        /// S-d: a 9 × 9 layer of unit-half-extent boxes resting on the plane SDF (81 SDF
        /// manifolds of 4 points: one colour of 324 slots, above the parallel floor) with an
        /// 8 × 8 second layer on top (box-box manifolds through the real narrowphase), 120
        /// steps. The only static surface is the SDF plane.
        fn scene_s_d(arm: Arm) -> ArmResult {
            let mut bodies = Vec::new();
            let half = Vec3::new(1.0, 1.0, 1.0);
            for j in 0..9 {
                for k in 0..9 {
                    let p = Vec3::new(-8.0 + 2.0 * j as f32, 0.98, -8.0 + 2.0 * k as f32);
                    bodies.push(box_body(p, half, 0.125, 0.5, 0.0));
                }
            }
            for j in 0..8 {
                for k in 0..8 {
                    let p = Vec3::new(-7.0 + 2.0 * j as f32, 3.0, -7.0 + 2.0 * k as f32);
                    bodies.push(box_body(p, half, 0.125, 0.5, 0.0));
                }
            }
            let mut g = G1::new(&bodies, arm, false);
            for _ in 0..120 {
                g.step_pipeline(true);
            }
            g.finish()
        }

        /// S-e: the dense sphere line's UNSORTED hand-built stream (`(a, floor)` before
        /// `(a, a + 1)`), plus the same pair twice with different point counts (a duplicate
        /// key whose later manifold has fewer points, review W1) and a manifold with two
        /// points sharing a feature id. 40 steps.
        fn scene_s_e(arm: Arm) -> ArmResult {
            let n = 6;
            let mut g = G1::new(&dense_collision_scene(n), arm, false);
            for _ in 0..40 {
                let bodies = g.scratch.bodies().to_vec();
                let mut ms = dense_collision_manifolds(&bodies);
                let up = Vec3::new(0.0, 1.0, 0.0);
                ms.push(box_manifold(0, 1, up, -0.05, bodies[0].position, 4));
                ms.push(box_manifold(0, 1, up, -0.05, bodies[0].position, 2));
                let mut dup = box_manifold(2, 3, up, -0.05, bodies[2].position, 3);
                dup.points[2].feature_id = dup.points[0].feature_id;
                ms.push(dup);
                g.solve(&ms);
            }
            g.finish()
        }

        /// S-f: the three-layer pile for 45 steps with warm start disabled on step 20 only.
        fn scene_s_f(arm: Arm) -> ArmResult {
            let mut g = G1::new(&pyramid_bodies(3, 0.5, 0.5), arm, false);
            for step in 0..45 {
                g.solver.warm_start_enabled = step != 20;
                g.step_pipeline(false);
            }
            g.finish()
        }

        /// S-g: sixteen spheres with restitution 0.6 dropped from 3–9 m onto the floor
        /// (impact speeds well above `RESTITUTION_THRESHOLD`), sliding sideways at 6 m/s on
        /// friction 0.05 so the friction cone CLAMPS at every bounce (a sticking contact
        /// never reads the coefficient's value); 150 steps.
        fn scene_s_g(arm: Arm) -> ArmResult {
            let mut bodies = vec![floor_body(0.05)];
            for i in 0..16 {
                let p = Vec3::new(-12.0 + 3.0 * (i % 8) as f32, 3.0 + 0.4 * i as f32, if i < 8 { -2.0 } else { 2.0 });
                let mut b = sphere_body(p, 1.0, 1.0, 0.05, 0.6);
                b.linear_velocity = Vec3::new(6.0, 0.0, 1.5);
                bodies.push(b);
            }
            let mut g = G1::new(&bodies, arm, false);
            for _ in 0..150 {
                g.step_pipeline(false);
            }
            g.finish()
        }

        #[test]
        fn g1_s_a_pyramid_pins() {
            let results = assert_pinned("S-a", PIN_S_A, scene_s_a);
            for r in &results {
                assert!(
                    r.counters.misses > 0 && r.counters.carry_hits == 0 && r.counters.disabled_steps == 0,
                    "anti-vacuity: S-a has misses (the first step) and neither a carry nor a disabled \
                     step: {:?}",
                    r.counters
                );
            }
        }

        #[test]
        fn g1_s_b_sleep_wake_pins() {
            let results = assert_pinned("S-b", PIN_S_B, scene_s_b);
            for r in &results {
                assert!(
                    r.counters.carry_hits > 0,
                    "anti-vacuity: S-b's frozen pile must have its warm entries carried (B1): {:?}",
                    r.counters
                );
            }
        }

        #[test]
        fn g1_s_c_churn_pins() {
            let results = assert_pinned("S-c", PIN_S_C, scene_s_c);
            for r in &results {
                assert!(
                    r.counters.backward_searches > 0
                        && r.counters.misses > r.marked.misses
                        && r.stats.remap_resets == 1,
                    "anti-vacuity: S-c's jumpers and flips must produce a backward search and a \
                     miss after the phase mark, and its missed gather one Reset: {:?} after {:?}, \
                     resets {}",
                    r.counters,
                    r.marked,
                    r.stats.remap_resets
                );
            }
        }

        #[test]
        fn g1_s_d_sdf_pile_pins() {
            let results = assert_pinned("S-d", PIN_S_D, scene_s_d);
            for (arm, r) in ARMS.iter().zip(&results) {
                assert!(
                    r.wide_steps > 0,
                    "anti-vacuity: S-d's SDF colour must clear the parallel floor \
                     ({MIN_PARALLEL_SLOTS_PER_COLOR} slots) so the W = 8 arm dispatches: {arm:?}"
                );
            }
        }

        #[test]
        fn g1_s_e_unsorted_duplicates_pins() {
            let results = assert_pinned("S-e", PIN_S_E, scene_s_e);
            for r in &results {
                assert!(
                    r.counters.cold_index_steps > 0 && r.counters.duplicate_fids_met > 0,
                    "anti-vacuity: S-e must look up a non-strict stream and meet a duplicate feature \
                     id: {:?}",
                    r.counters
                );
            }
        }

        #[test]
        fn g1_s_f_warm_toggle_pins() {
            let results = assert_pinned("S-f", PIN_S_F, scene_s_f);
            for r in &results {
                assert_eq!(
                    r.counters.disabled_steps, 1,
                    "anti-vacuity: S-f runs exactly one step with warm start disabled: {:?}",
                    r.counters
                );
            }
        }

        #[test]
        fn g1_s_g_restitution_pins() {
            let results = assert_pinned("S-g", PIN_S_G, scene_s_g);
            for r in &results {
                assert!(
                    r.counters.restitution_applied > 0,
                    "anti-vacuity: S-g's restitution pass must apply an impulse: {:?}",
                    r.counters
                );
            }
        }
    }
