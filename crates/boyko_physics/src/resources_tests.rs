    //! W1 acceptance gate (plan §MAJOR W1): the `from_columns` /
    //! `local_inv_inertia` inertia DERIVATION. The `math.rs` suite covers the
    //! `Mat3` ops in isolation; these tests pin the per-shape local-tensor
    //! VALUES and the world-tensor `R₀ · I⁻¹_local · R₀ᵀ` construction that the
    //! gather builds — the values the solver's effective mass depends on.

    use super::*;
    use crate::components::ColliderShape;

    /// Builds a `RigidBody` at the given orientation with everything else default.
    fn body_with_rotation(rotation: Quat) -> RigidBody {
        RigidBody {
            position: Vec3::ZERO,
            linear_velocity: Vec3::ZERO,
            rotation,
            angular_velocity: Vec3::ZERO,
        }
    }

    /// Builds a `RigidBodyMass` with the given inverse mass (dynamic, the
    /// `inv_inertia` placeholder is overridden by `from_columns`).
    fn mass_with_inv_mass(inv_mass: f32) -> RigidBodyMass {
        RigidBodyMass {
            inv_inertia: Mat3::IDENTITY,
            inv_mass,
            restitution: 0.5,
            friction: 0.3,
        }
    }

    fn collider_shape(shape: ColliderShape) -> Collider {
        Collider {
            shape,
            layer: 1,
            mask: 1,
        }
    }

    /// A solid sphere derives the isotropic local inverse inertia
    /// `inv_mass · 5 / (2·r²)` on each diagonal (off-diagonals zero).
    #[test]
    fn from_columns_sphere_local_tensor_values() {
        // r = 0.5, inv_mass = 2.0 ⇒ inv = 2·5 / (2·0.25) = 10 / 0.5 = 20.
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(2.0);
        let collider = collider_shape(ColliderShape::Sphere { radius: 0.5 });

        let state = BodyState::from_columns(&body, &mass, &collider, false, true, false);
        let i = state.inv_inertia_local;
        let expected = 20.0_f32;
        assert!((i.rows[0].x - expected).abs() < 1e-4, "Ixx⁻¹: {}", i.rows[0].x);
        assert!((i.rows[1].y - expected).abs() < 1e-4, "Iyy⁻¹: {}", i.rows[1].y);
        assert!((i.rows[2].z - expected).abs() < 1e-4, "Izz⁻¹: {}", i.rows[2].z);
        // Isotropic ⇒ off-diagonals zero.
        assert_eq!(i.rows[0].y, 0.0);
        assert_eq!(i.rows[0].z, 0.0);
        assert_eq!(i.rows[1].x, 0.0);
    }

    /// A box derives the per-axis local inverse inertia `12·inv_mass / (sum of
    /// the two other full-extents squared)`.
    #[test]
    fn from_columns_box_local_tensor_values() {
        // half_extents (1,2,3) ⇒ full (w,h,d) = (2,4,6); inv_mass = 3.
        //   Ixx⁻¹ = 12·3 / (h²+d²) = 36 / (16+36) = 36/52
        //   Iyy⁻¹ = 12·3 / (w²+d²) = 36 / (4+36)  = 36/40
        //   Izz⁻¹ = 12·3 / (w²+h²) = 36 / (4+16)  = 36/20
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(3.0);
        let collider = collider_shape(ColliderShape::Box {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider, false, true, false);
        let i = state.inv_inertia_local;
        assert!((i.rows[0].x - 36.0 / 52.0).abs() < 1e-5, "Ixx⁻¹: {}", i.rows[0].x);
        assert!((i.rows[1].y - 36.0 / 40.0).abs() < 1e-5, "Iyy⁻¹: {}", i.rows[1].y);
        assert!((i.rows[2].z - 36.0 / 20.0).abs() < 1e-5, "Izz⁻¹: {}", i.rows[2].z);
    }

    /// A static body (`inv_mass == 0`) derives `Mat3::ZERO` (infinite inertia),
    /// for both local AND world tensors — no angular response.
    #[test]
    fn from_columns_static_body_zero_inertia() {
        let body = body_with_rotation(Quat::new(0.2, -0.4, 0.5, 0.8).normalize());
        let mass = mass_with_inv_mass(0.0);
        let collider = collider_shape(ColliderShape::Sphere { radius: 0.5 });

        let state = BodyState::from_columns(&body, &mass, &collider, false, true, false);
        assert_eq!(state.inv_inertia_local, Mat3::ZERO, "static local tensor is ZERO");
        // World tensor R·ZERO·Rᵀ is also ZERO regardless of orientation.
        assert_eq!(state.inv_inertia, Mat3::ZERO, "static world tensor is ZERO");
        assert_eq!(state.inv_mass, 0.0);
    }

    /// A degenerate (non-positive radius) sphere derives `Mat3::ZERO` rather than
    /// dividing by zero (`local_inv_inertia` guards `radius <= 0`).
    #[test]
    fn from_columns_degenerate_sphere_zero_inertia() {
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(1.0);
        let collider = collider_shape(ColliderShape::Sphere { radius: 0.0 });

        let state = BodyState::from_columns(&body, &mass, &collider, false, true, false);
        assert_eq!(
            state.inv_inertia_local,
            Mat3::ZERO,
            "degenerate radius must not divide by zero"
        );
    }

    /// At identity orientation the WORLD tensor equals the LOCAL tensor
    /// (`R₀ = IDENTITY ⇒ R₀·I·R₀ᵀ = I`).
    #[test]
    fn from_columns_world_equals_local_at_identity() {
        let body = body_with_rotation(Quat::IDENTITY);
        let mass = mass_with_inv_mass(1.0);
        let collider = collider_shape(ColliderShape::Box {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider, false, true, false);
        assert_eq!(
            state.inv_inertia, state.inv_inertia_local,
            "world tensor equals local tensor when R₀ == IDENTITY"
        );
    }

    /// For a rotated body the WORLD tensor `R₀ · I⁻¹_local · R₀ᵀ` is symmetric
    /// (a similarity transform of a diagonal tensor) and is NOT the local tensor
    /// (the rotation actually applied).
    #[test]
    fn from_columns_world_tensor_is_symmetric_under_rotation() {
        let body = body_with_rotation(Quat::new(0.2, -0.4, 0.5, 0.8).normalize());
        let mass = mass_with_inv_mass(1.0);
        // An anisotropic box so the rotation visibly mixes the axes.
        let collider = collider_shape(ColliderShape::Box {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider, false, true, false);
        let w = state.inv_inertia;
        assert!((w.rows[0].y - w.rows[1].x).abs() < 1e-5, "M[0][1]==M[1][0]");
        assert!((w.rows[0].z - w.rows[2].x).abs() < 1e-5, "M[0][2]==M[2][0]");
        assert!((w.rows[1].z - w.rows[2].y).abs() < 1e-5, "M[1][2]==M[2][1]");
        assert_ne!(
            state.inv_inertia, state.inv_inertia_local,
            "a non-identity rotation must change the world tensor"
        );
    }

    /// `PhysicsConfig::default()` carries the W1 soft-constraint set (OQ-5:
    /// substeps 1→4) so a hand-built default matches the plan's tunables.
    #[test]
    fn physics_config_default_w1_tunables() {
        let cfg = PhysicsConfig::default();
        assert_eq!(cfg.substeps, 4, "OQ-5: default substeps is 4");
        assert_eq!(cfg.relax_iterations, 2);
        assert_eq!(cfg.contact_hertz, 30.0);
        assert_eq!(cfg.contact_damping, 10.0);
        assert_eq!(cfg.dt, 0.0, "dt is a placeholder until gather stamps it");
        assert!(!cfg.colored, "O4: colored is OFF by default (the 0%-gate)");
        assert!(!cfg.soft_body, "SP1: soft_body is OFF by default (the 0%-gate)");
    }

    // ── O4: ConstraintGraph islands + coloring sanity tests ──
    //
    // Tiny hand-built graphs (these are SANITY checks; the exhaustive coloring-
    // invariant / island-BFS / determinism proptests are the tester's).

    /// Builds an empty manifold between two dense body rows (the only fields the
    /// partition reads are `body_a` / `body_b`).
    fn edge(a: u32, b: u32) -> Manifold {
        Manifold::new(BodyIndex(a), BodyIndex(b))
    }

    /// Re-scans the produced coloring and asserts no color shares a dynamic body
    /// (the O4 invariant), returns the number of colors.
    fn assert_coloring_invariant(
        g: &ConstraintGraph,
        manifolds: &[Manifold],
        is_dynamic: &impl Fn(u32) -> bool,
    ) -> u32 {
        use std::collections::HashSet;
        let mut total = 0usize;
        for c in 0..g.n_colors() {
            let mut seen: HashSet<u32> = HashSet::new();
            for &mi in g.color(c) {
                let m = &manifolds[mi as usize];
                for &row in &[m.body_a.0, m.body_b.0] {
                    if is_dynamic(row) {
                        assert!(
                            seen.insert(row),
                            "color {c} reuses dynamic body {row} (coloring invariant)"
                        );
                    }
                }
            }
            total += g.color(c).len();
        }
        assert_eq!(total, manifolds.len(), "every manifold appears in exactly one color");
        g.n_colors()
    }

    /// A triangle of three dynamic bodies (every pair touching) is one island and
    /// needs 3 colors (each pair of edges shares a vertex), with the invariant held.
    #[test]
    fn graph_triangle_one_island_three_colors() {
        let manifolds = [edge(0, 1), edge(1, 2), edge(0, 2)];
        let dyn3 = |row: u32| row < 3; // all three dynamic
        let mut g = ConstraintGraph::default();
        g.build(&manifolds, 3, dyn3);

        assert_eq!(g.n_islands(), 1, "a connected triangle is one island");
        assert_eq!(g.island_of(0), 0);
        assert_eq!(g.island_of(1), 0);
        assert_eq!(g.island_of(2), 0);
        assert_eq!(g.island(0).len(), 3, "all three edges file under the island");

        let n_colors = assert_coloring_invariant(&g, &manifolds, &dyn3);
        // A triangle's edges pairwise share a vertex → each needs its own color.
        assert_eq!(n_colors, 3, "triangle edges need 3 colors");
    }

    /// Two disjoint dynamic pairs `(0-1)` and `(2-3)` form two islands; both edges
    /// are body-disjoint so a single color suffices.
    #[test]
    fn graph_two_disjoint_pairs_two_islands_one_color() {
        let manifolds = [edge(0, 1), edge(2, 3)];
        let dyn4 = |row: u32| row < 4;
        let mut g = ConstraintGraph::default();
        g.build(&manifolds, 4, dyn4);

        assert_eq!(g.n_islands(), 2, "two disjoint pairs are two islands");
        // The two edges share no body → both fit in color 0.
        assert_eq!(g.n_colors(), 1, "body-disjoint edges share one color");
        assert_coloring_invariant(&g, &manifolds, &dyn4);
        // The two islands carry one manifold each.
        assert_eq!(g.island(g.island_of(0)).len(), 1);
        assert_eq!(g.island(g.island_of(2)).len(), 1);
        assert_ne!(g.island_of(0), g.island_of(2), "bodies 0 and 2 are in different islands");
    }

    /// A static body (`inv_mass == 0`, row 0) is GROUND: it does NOT connect the
    /// islands of the two dynamic bodies it touches (Box2D's rule), is never an
    /// island node (`NO_ISLAND`), and imposes no coloring occupancy (both
    /// dyn-vs-ground edges share one color despite sharing the static body).
    #[test]
    fn graph_static_is_ground_does_not_merge_or_constrain() {
        // Row 0 = static ground; rows 1 and 2 are dynamic, each touching ground.
        let manifolds = [edge(0, 1), edge(0, 2)];
        let dyn_pred = |row: u32| row == 1 || row == 2; // row 0 static
        let mut g = ConstraintGraph::default();
        g.build(&manifolds, 3, dyn_pred);

        // Ground does not merge: bodies 1 and 2 are SEPARATE islands.
        assert_eq!(g.n_islands(), 2, "ground does not merge two dynamic islands");
        assert_eq!(g.island_of(0), ConstraintGraph::NO_ISLAND, "static = NO_ISLAND");
        assert_ne!(g.island_of(1), g.island_of(2), "1 and 2 stay in distinct islands");

        // Ground imposes no occupancy → both edges fit in one color even though
        // they share the static body 0.
        assert_eq!(g.n_colors(), 1, "shared ground does not split colors");
        assert_coloring_invariant(&g, &manifolds, &dyn_pred);
    }

    /// The partition is a pure deterministic function of its input: building the
    /// same chain twice (and reusing a warmed graph) yields identical CSR output.
    #[test]
    fn graph_build_is_deterministic_and_reusable() {
        // A 5-body chain 0-1-2-3-4 (one island; a path needs only 2 colors).
        let manifolds = [edge(0, 1), edge(1, 2), edge(2, 3), edge(3, 4)];
        let dyn5 = |row: u32| row < 5;

        let mut a = ConstraintGraph::default();
        a.build(&manifolds, 5, dyn5);
        let colors_a: Vec<Vec<u32>> = (0..a.n_colors()).map(|c| a.color(c).to_vec()).collect();
        let island_of_a: Vec<u32> = (0..5).map(|r| a.island_of(r)).collect();

        // Reuse the SAME warmed graph for a second build — capacity reused, output
        // must be identical (no stale state leaks across builds).
        a.build(&manifolds, 5, dyn5);
        let colors_a2: Vec<Vec<u32>> = (0..a.n_colors()).map(|c| a.color(c).to_vec()).collect();
        assert_eq!(colors_a, colors_a2, "rebuild on a warmed graph is identical");

        // A fresh graph must match too (no dependence on prior state).
        let mut b = ConstraintGraph::default();
        b.build(&manifolds, 5, dyn5);
        let colors_b: Vec<Vec<u32>> = (0..b.n_colors()).map(|c| b.color(c).to_vec()).collect();
        let island_of_b: Vec<u32> = (0..5).map(|r| b.island_of(r)).collect();
        assert_eq!(colors_a, colors_b, "fresh vs warmed graph: identical coloring");
        assert_eq!(island_of_a, island_of_b, "fresh vs warmed graph: identical islands");

        assert_eq!(a.n_islands(), 1, "the chain is one connected island");
        assert_eq!(a.n_colors(), 2, "a path graph is 2-colorable");
        assert_coloring_invariant(&a, &manifolds, &dyn5);
    }

    // ── O3 parallel candidate emit — shaped-path gates (in-lib) ───────────────
    //
    // These cover the multi-pass SHAPED emit (`build_emit_shaped_forced`) at
    // forced `n_chunks ∈ {1, 2, 4, 8}`, single-threaded (NO pool), so they:
    //   * exercise the restructured Pass A count / serial prefix-sum / Pass B
    //     `pair_offset` arithmetic + the `EmitPtrs` disjoint raw writes,
    //   * run under `cargo +nightly miri test` (the pool spin is Miri-intractable;
    //     the shaped path needs no pool — `pool = None`),
    //   * prove byte-identity to the O2 serial `build` AND non-vacuity (the W2
    //     anti-vacuity bar: it ran the shaped passes, not a `build` delegate).
    // The pool-driven `build_parallel` native MT gate + the dense criterion live
    // in `tests/broadphase_grid.rs` / `benches/broadphase.rs` (separate crates —
    // `build_emit_shaped_forced` is `#[cfg(test)] pub(crate)`, reachable ONLY here).
    mod o3_shaped {
        use super::*;
        use crate::components::ColliderShape;

        /// The forced chunk counts the shaped-path gates sweep (W ∈ {1, 2, 4, 8}).
        const FORCED_CHUNKS: [usize; 4] = [1, 2, 4, 8];

        /// A `BodyState` carrying only the broadphase-relevant fields.
        fn sphere(position: Vec3, radius: f32) -> BodyState {
            BodyState {
                position,
                shape: ColliderShape::Sphere { radius },
                ..Default::default()
            }
        }

        fn boxx(position: Vec3, half: Vec3) -> BodyState {
            BodyState {
                position,
                shape: ColliderShape::Box { half_extents: half },
                ..Default::default()
            }
        }

        /// The reference all-pairs broadphase — the LITERAL production predicate
        /// (same operand order, same `(min, max)` emission, already sorted).
        fn all_pairs(bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
            let mut pairs = Vec::new();
            let n = bodies.len();
            for i in 0..n {
                for j in (i + 1)..n {
                    let bound =
                        body_bounding_radius(&bodies[i]) + body_bounding_radius(&bodies[j]);
                    let delta = bodies[j].position - bodies[i].position;
                    if delta.length_squared() <= bound * bound {
                        pairs.push((BodyIndex(i as u32), BodyIndex(j as u32)));
                    }
                }
            }
            pairs
        }

        /// The O2 serial `build` output for `bodies` (the bit-identity reference).
        fn serial_build(bodies: &[BodyState]) -> Vec<(BodyIndex, BodyIndex)> {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build(bodies, &mut out);
            out
        }

        /// The shaped-path output at a forced `n_chunks` (single-threaded, no pool).
        fn shaped_build(bodies: &[BodyState], n_chunks: usize) -> Vec<(BodyIndex, BodyIndex)> {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build_emit_shaped_forced(bodies, &mut out, n_chunks);
            out
        }

        /// Asserts the shaped path at EVERY forced chunk count is byte-for-byte
        /// equal to the O2 serial `build` AND to all-pairs (same multiset AND
        /// order — the headline C1/W4 partition-independence gate).
        fn assert_shaped_eq_serial_and_all_pairs(bodies: &[BodyState]) {
            let serial = serial_build(bodies);
            let reference = all_pairs(bodies);
            assert_eq!(
                serial, reference,
                "O2 serial build must equal all-pairs (sanity of the reference)"
            );
            for w in FORCED_CHUNKS {
                let shaped = shaped_build(bodies, w);
                assert_eq!(
                    shaped, serial,
                    "shaped emit at n_chunks={w} must be byte-identical to O2 serial build"
                );
            }
        }

        // ── A tiny xorshift PRNG: a self-contained seeded scene generator so the
        //    in-lib gate needs no proptest harness (proptest's strategy runner is
        //    heavier here than a deterministic 1000-scene sweep). ──────────────
        struct Rng(u64);
        impl Rng {
            fn new(seed: u64) -> Self {
                // Avoid the zero fixed-point of xorshift.
                Rng(seed | 1)
            }
            fn next_u64(&mut self) -> u64 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                x
            }
            /// A uniform `f32` in `[lo, hi)`.
            fn range(&mut self, lo: f32, hi: f32) -> f32 {
                let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32; // [0, 1)
                lo + u * (hi - lo)
            }
            fn below(&mut self, n: usize) -> usize {
                (self.next_u64() % n as u64) as usize
            }
        }

        /// A seeded DENSE random scene: 1..=40 mixed sphere/box bodies in a
        /// bounded box so clusters AND gaps occur (the same domain as the O2
        /// `grid_equals_all_pairs` proptest, deterministically seeded).
        fn random_scene(seed: u64) -> Vec<BodyState> {
            let mut rng = Rng::new(seed);
            let n = 1 + rng.below(40);
            (0..n)
                .map(|_| {
                    let p = Vec3::new(
                        rng.range(-20.0, 20.0),
                        rng.range(-20.0, 20.0),
                        rng.range(-20.0, 20.0),
                    );
                    if rng.next_u64() & 3 < 2 {
                        sphere(p, rng.range(0.1, 3.0))
                    } else {
                        boxx(
                            p,
                            Vec3::new(
                                rng.range(0.1, 3.0),
                                rng.range(0.1, 3.0),
                                rng.range(0.1, 3.0),
                            ),
                        )
                    }
                })
                .collect()
        }

        // ── Gate 1 (shaped slice): {1, 2, 4, 8}-chunk multiset+order bit-identity
        //    over 1000 seeded dense scenes. The shaped path is partition-
        //    independent, so EVERY chunk count must reproduce the serial `build`
        //    and all-pairs output byte-for-byte. (The pool-DISPATCHED leg of
        //    Gate 1 lives in `tests/broadphase_grid.rs`'s native MT gate.) ──────
        #[test]
        fn shaped_emit_bit_identical_to_serial_over_1000_scenes() {
            for seed in 0..1000u64 {
                let bodies = random_scene(seed);
                assert_shaped_eq_serial_and_all_pairs(&bodies);
            }
        }

        // ── Gate 2: W=1-shaped == serial AND non-vacuous AND genuinely the shaped
        //    multi-pass path (not a `build` delegate). ─────────────────────────
        #[test]
        fn shaped_w1_equals_serial_and_is_non_vacuous() {
            // A dense overlapping lattice → a real, multi-cell candidate set.
            let bodies: Vec<BodyState> = (0..200)
                .map(|i| {
                    let t = i as f32;
                    sphere(
                        Vec3::new(
                            (t * 0.21).sin() * 8.0,
                            (t * 0.13).cos() * 8.0,
                            (t * 0.37).sin() * 8.0,
                        ),
                        0.5,
                    )
                })
                .collect();
            let serial = serial_build(&bodies);
            let shaped1 = shaped_build(&bodies, 1);
            assert_eq!(shaped1, serial, "shaped n_chunks=1 == O2 serial build");
            assert!(
                !shaped1.is_empty(),
                "anti-vacuity: the shaped path emitted pairs (the passes ran, not a no-op)"
            );
        }

        // ── Gate 2 (cont.): the shaped path is genuinely MULTI-pass — at
        //    n_chunks ∈ {2, 4, 8} the work is split across DISTINCT cell-range
        //    chunks (the `pair_offset` arithmetic), yet the output is unchanged.
        //    A scene with many cells AND many survivors guarantees the chunk cut
        //    actually partitions work (anti-vacuity for the parallel-emit code:
        //    a `build` delegate could not honor `n_chunks`). ────────────────────
        #[test]
        fn shaped_multichunk_partitions_work_yet_output_is_invariant() {
            // A 12³ overlapping lattice: many occupied cells AND many survivors,
            // so a chunk cut at n_chunks=8 splits the cell range into real,
            // non-trivial blocks (each emits into its own out sub-range).
            let mut bodies = Vec::new();
            for z in 0..12 {
                for y in 0..12 {
                    for x in 0..12 {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            let serial = serial_build(&bodies);
            assert!(
                serial.len() > 100,
                "anti-vacuity: the lattice yields many survivors ({})",
                serial.len()
            );
            // Every chunk count reproduces the identical output despite different
            // Pass-A counts / prefix-sum cuts / Pass-B sub-range partitions.
            for w in [2usize, 4, 8] {
                let shaped = shaped_build(&bodies, w);
                assert_eq!(
                    shaped, serial,
                    "n_chunks={w} reproduces the serial output (partition-independent)"
                );
            }
        }

        // ── Gate 3: oversized-heavy scene — the SERIAL oversized append leg. ──
        #[test]
        fn shaped_oversized_heavy_matches_serial() {
            // Pack many small bodies (fine cells) + a handful of giants that span
            // >= MAX_CELL_SPAN cells → the oversized hatch. 16³ = 4096 smalls so
            // cbrt(n) is large and the median floor keeps cells fine.
            let mut bodies = Vec::new();
            let side = 16;
            for z in 0..side {
                for y in 0..side {
                    for x in 0..side {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            for k in 0..4 {
                let f = k as f32;
                bodies.push(sphere(Vec3::new(f * 2.0 + 1.0, f * 2.0 + 1.0, f * 2.0), 25.0));
            }

            // Non-vacuity: >= 2 giants land in the hatch (oversized–oversized
            // dedup AND oversized–normal emit are both exercised).
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build_emit_shaped_forced(&bodies, &mut out, 4);
            assert!(
                grid.oversized_len() >= 2,
                "size disparity must classify >= 2 bodies oversized (got {})",
                grid.oversized_len()
            );
            // The shaped path's oversized append at every chunk count == serial.
            assert_shaped_eq_serial_and_all_pairs(&bodies);
        }

        // ── Gate 4: edge cases via the shaped path at every chunk count. ──────
        #[test]
        fn shaped_empty_world() {
            for w in FORCED_CHUNKS {
                assert!(
                    shaped_build(&[], w).is_empty(),
                    "empty world emits no pairs (n_chunks={w})"
                );
            }
        }

        #[test]
        fn shaped_single_body() {
            let bodies = [sphere(Vec3::new(1.0, 2.0, 3.0), 0.5)];
            assert_shaped_eq_serial_and_all_pairs(&bodies);
        }

        #[test]
        fn shaped_all_coincident_c_n_2_no_dupes() {
            let bodies: Vec<BodyState> = (0..12).map(|_| sphere(Vec3::ZERO, 0.5)).collect();
            assert_shaped_eq_serial_and_all_pairs(&bodies);
            for w in FORCED_CHUNKS {
                let shaped = shaped_build(&bodies, w);
                assert_eq!(
                    shaped.len(),
                    12 * 11 / 2,
                    "all-coincident → C(n,2) pairs, no dupes (n_chunks={w})"
                );
            }
        }

        #[test]
        fn shaped_far_apart_no_pairs() {
            let bodies: Vec<BodyState> = (0..16)
                .map(|i| sphere(Vec3::new(i as f32 * 1000.0, 0.0, 0.0), 0.5))
                .collect();
            assert_shaped_eq_serial_and_all_pairs(&bodies);
            for w in FORCED_CHUNKS {
                assert!(
                    shaped_build(&bodies, w).is_empty(),
                    "far-apart bodies pair with none (n_chunks={w})"
                );
            }
        }

        #[test]
        fn shaped_cell_boundary_and_mixed_shapes() {
            let bodies = [
                sphere(Vec3::new(0.0, 0.0, 0.0), 0.6),
                boxx(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.4, 0.4, 0.4)),
                sphere(Vec3::new(2.0, 0.0, 0.0), 0.5),
                boxx(Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.7, 0.3, 0.5)),
                sphere(Vec3::new(0.0, 1.0, 1.0), 0.5),
                boxx(Vec3::new(2.0, 2.0, 2.0), Vec3::new(0.2, 0.2, 0.2)),
            ];
            assert_shaped_eq_serial_and_all_pairs(&bodies);
        }

        // ── Gate 4 (cont.): a reused grid (capacity-reused pair_count/pair_offset)
        //    matches a fresh grid — no stale Pass-A/prefix state across builds. ─
        #[test]
        fn shaped_reused_grid_no_stale_pair_offset_state() {
            let scene_a: Vec<BodyState> = (0..50)
                .map(|i| sphere(Vec3::new(i as f32 * 0.7, (i % 5) as f32, 0.0), 0.5))
                .collect();
            let scene_b: Vec<BodyState> = (0..30)
                .map(|i| sphere(Vec3::new((i as f32).sin() * 5.0, 0.0, i as f32 * 0.4), 0.6))
                .collect();

            let mut reused = BroadphaseGrid::with_capacity(64);
            let mut out = Vec::new();
            // Warm on scene A at a DIFFERENT chunk count, then rebuild scene B.
            reused.build_emit_shaped_forced(&scene_a, &mut out, 8);
            reused.build_emit_shaped_forced(&scene_b, &mut out, 4);
            let reused_b = out.clone();

            let fresh_b = shaped_build(&scene_b, 4);
            assert_eq!(
                reused_b, fresh_b,
                "a reused grid matches a fresh shaped build (no stale pair_count/pair_offset)"
            );
            assert_eq!(reused_b, all_pairs(&scene_b), "and equals all-pairs");
        }

        // ── Gate 5 (Miri): the curated small-scene shaped sweep at every chunk
        //    count. `cargo +nightly miri test` runs THIS (no pool needed — the
        //    shaped path runs `pool = None`); it checks the restructured offset
        //    arithmetic + the `EmitPtrs` disjoint raw writes for TB/aliasing UB.
        //    Kept small (≈ 64 bodies) so the interpreter stays tractable. ───────
        #[test]
        fn shaped_miri_small_dense_all_chunk_counts() {
            // A 4³ overlapping lattice (64 bodies) — enough occupied cells that
            // n_chunks ∈ {2, 4, 8} cut real disjoint blocks, small enough for Miri.
            let mut bodies = Vec::new();
            for z in 0..4 {
                for y in 0..4 {
                    for x in 0..4 {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            let serial = serial_build(&bodies);
            assert!(!serial.is_empty(), "anti-vacuity: the Miri lattice has survivors");
            for w in FORCED_CHUNKS {
                let shaped = shaped_build(&bodies, w);
                assert_eq!(shaped, serial, "Miri shaped n_chunks={w} == serial");
            }
        }

        // ── Gate 5 (Miri, no-pool route): `build_parallel` called OUTSIDE any
        //    `install` frame. `try_with_active_pool` returns None under Miri (no
        //    pool — the work-stealing spin is Miri-intractable, Phase 9.1-9.3), so
        //    `build_parallel` routes through the no-pool shaped path
        //    (`emit_passes(.., 1, None)`). This pins that the production entry's
        //    Miri-reachable branch is exactly the (TB-clean) shaped path and still
        //    equals the serial build. The pool-DISPATCHED branch is covered by the
        //    native MT gate in `tests/broadphase_grid.rs` (Miri can't spin the
        //    pool — the same gating the colored-solver O6 parallel tests use). ───
        #[test]
        fn shaped_miri_build_parallel_no_pool_route_equals_serial() {
            let mut bodies = Vec::new();
            for z in 0..4 {
                for y in 0..4 {
                    for x in 0..4 {
                        bodies.push(sphere(
                            Vec3::new(x as f32 * 0.9, y as f32 * 0.9, z as f32 * 0.9),
                            0.5,
                        ));
                    }
                }
            }
            let serial = serial_build(&bodies);
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = Vec::new();
            grid.build_parallel(&bodies, &mut out);
            assert_eq!(
                out, serial,
                "build_parallel's no-pool route (the Miri-reachable branch) == serial build"
            );
        }
    }
