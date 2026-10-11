    //! W1 acceptance gate (plan §MAJOR W1): the `from_columns` /
    //! `local_inv_inertia` inertia DERIVATION. The `math.rs` suite covers the
    //! `Mat3` ops in isolation; these tests pin the per-shape local-tensor
    //! VALUES and the world-tensor `R₀ · I⁻¹_local · R₀ᵀ` construction that the
    //! gather builds — the values the solver's effective mass depends on.

    // Test-oracle model: the std `HashSet` is the REFERENCE "no color reuses a dynamic
    // body" checker the constraint-graph coloring is verified against. Compiled out of
    // every shipping build (`#[cfg(test)] #[path] mod tests` of `resources.rs`).
    #![allow(clippy::disallowed_types)]

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
        assert!(
            !cfg.colored,
            "O4: the struct default of `colored` is false; the plugin sets it from the solver type"
        );
        assert!(!cfg.soft_body, "SP1: soft_body is OFF by default (the 0%-gate)");
        assert!(
            cfg.simd,
            "O1: simd is ON by default since 2026-09-03 — the AVX2 integrate / inertia kernels \
             are bit-identical to their scalar oracles and their gates are non-vacuous"
        );
        assert!(
            cfg.simd_solve,
            "O7: simd_solve is ON by default since 2026-09-18 (owner decision) — the AVX2 cohort \
             kernel is bit-identical to the scalar colored oracle"
        );
        assert_eq!(
            cfg.sdf_narrowphase,
            SdfNarrowphaseKernel::Scalar,
            "O9: the box-vs-SDF kernel defaults to the SCALAR oracle. The AVX2 arm has a known \
             signed-zero divergence from it (owner-deferred fix), so reaching it must take an \
             explicit opt-in — this is the containment, not a preference"
        );
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
    //   * run under `cargo +nightly-x86_64-pc-windows-msvc miri test` (the pool spin is Miri-intractable;
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
            let mut out = ContactPairs::with_capacity(0);
            grid.build(bodies, &mut out);
            out.pairs().to_vec()
        }

        /// The shaped-path output at a forced `n_chunks` (single-threaded, no pool).
        fn shaped_build(bodies: &[BodyState], n_chunks: usize) -> Vec<(BodyIndex, BodyIndex)> {
            let mut grid = BroadphaseGrid::with_capacity(bodies.len());
            let mut out = ContactPairs::with_capacity(0);
            grid.build_emit_shaped_forced(bodies, &mut out, n_chunks);
            out.pairs().to_vec()
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
            let mut out = ContactPairs::with_capacity(0);
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
            let mut out = ContactPairs::with_capacity(0);
            // Warm on scene A at a DIFFERENT chunk count, then rebuild scene B.
            reused.build_emit_shaped_forced(&scene_a, &mut out, 8);
            reused.build_emit_shaped_forced(&scene_b, &mut out, 4);
            let reused_b = out.pairs().to_vec();

            let fresh_b = shaped_build(&scene_b, 4);
            assert_eq!(
                reused_b, fresh_b,
                "a reused grid matches a fresh shaped build (no stale pair_count/pair_offset)"
            );
            assert_eq!(reused_b, all_pairs(&scene_b), "and equals all-pairs");
        }

        // ── Gate 5 (Miri): the curated small-scene shaped sweep at every chunk
        //    count. `cargo +nightly-x86_64-pc-windows-msvc miri test` runs THIS (no pool needed — the
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
            let mut out = ContactPairs::with_capacity(0);
            grid.build_parallel(&bodies, &mut out);
            assert_eq!(
                out.pairs().to_vec(),
                serial,
                "build_parallel's no-pool route (the Miri-reachable branch) == serial build"
            );
        }
    }

    // ─── Defect A interim fix: `IslandSleep::rekey_rows` (T7) ──────────────────────
    //
    // Every arm of `rekey_rows` ends keyed by the current gather, so every arm must leave the
    // cursor stamped with that gather (invariant P). Red under M4 (the `Reset` arm does not
    // stamp) and M5 (the `Identity` arm does not stamp).
    mod rekey_rows {
        use crate::resources::IslandSleep;
        use crate::row_identity::{RemapCursor, RowIdentity, RowKey, RowRemap};

        /// Feeds one scripted gather into `rows`: `ids` are slots at generation 0.
        fn gather(rows: &mut RowIdentity, ids: &[usize], added: &[u32]) {
            rows.begin_gather();
            {
                let (mut cur, mut add) = rows.gather_views();
                for &id in ids {
                    cur.push(RowKey::new(id, 0));
                }
                for &r in added {
                    add.push(r);
                }
            }
            rows.finish_gather();
        }

        /// The gather sequence `rows` is at, read through a probe stamp (`RowIdentity` has no
        /// getter; `stamp` writes exactly this value).
        fn gather_seq(rows: &RowIdentity) -> u64 {
            let mut probe = RemapCursor::default();
            probe.stamp(rows);
            probe.synced_seq()
        }

        /// How the latch's cursor classifies against `rows`, without touching it.
        fn classify(sleep: &IslandSleep, rows: &RowIdentity) -> &'static str {
            let mut probe = sleep.cursor;
            match probe.remap(rows) {
                RowRemap::Identity => "Identity",
                RowRemap::Rows(_) => "Rows",
                RowRemap::Reset => "Reset",
            }
        }

        fn latch(sleep: &IslandSleep) -> Vec<(bool, u16)> {
            sleep
                .latch
                .as_read_slice()
                .iter()
                .map(|l| (l.asleep, l.below_count))
                .collect()
        }

        fn set_latch(sleep: &mut IslandSleep, values: &[(bool, u16)]) {
            assert_eq!(sleep.latch.len(), values.len(), "test setup: latch length");
            let mut view = sleep.latch.build_view();
            for (l, &(asleep, below)) in view.as_mut_slice().iter_mut().zip(values) {
                l.asleep = asleep;
                l.below_count = below;
            }
        }

        /// The per-row island contact key (defect A4), row order.
        fn keys(sleep: &IslandSleep) -> Vec<u32> {
            sleep.latch.as_read_slice().iter().map(|l| l.island_key).collect()
        }

        fn set_keys(sleep: &mut IslandSleep, values: &[u32]) {
            assert_eq!(sleep.latch.len(), values.len(), "test setup: key length");
            let mut view = sleep.latch.build_view();
            for (l, &key) in view.as_mut_slice().iter_mut().zip(values) {
                l.island_key = key;
            }
        }

        /// U8 (defect A4): the `Rows` arm permutes the island contact key with the latch.
        /// A new body reads `NO_ISLAND_KEY` (the value a fresh row must carry, so it can
        /// never compare equal to a live count), and the key stays sized with the latch.
        #[test]
        fn rekey_rows_rows_permutes_the_island_key_with_the_latch() {
            use crate::row_identity::NO_ISLAND_KEY;

            let mut rows = RowIdentity::with_capacity(0);
            let mut sleep = IslandSleep::with_capacity(0, 0);

            gather(&mut rows, &[1, 2, 3], &[0, 1, 2]);
            sleep.rekey_rows(&rows);
            assert_eq!(
                (keys(&sleep), keys(&sleep).len() == rows.rows_len()),
                (vec![NO_ISLAND_KEY; 3], true),
                "U8 first gather: every new row carries NO_ISLAND_KEY: (keys, key len == gather rows)"
            );
            set_keys(&mut sleep, &[11, 12, 13]);

            // Growth: ids 3 and 1 move, id 9 is new in row 2, id 2 moves to the end.
            gather(&mut rows, &[3, 1, 9, 2], &[2]);
            sleep.rekey_rows(&rows);
            assert_eq!(
                (keys(&sleep), keys(&sleep).len() == rows.rows_len()),
                (vec![13, 11, NO_ISLAND_KEY, 12], true),
                "U8 growth: each body carries its own key and the new body reads NO_ISLAND_KEY: \
                 (keys, key len == gather rows)"
            );

            // Shrink: ids 1 and 9 are gone, 2 and 3 swap ends.
            gather(&mut rows, &[2, 3], &[]);
            sleep.rekey_rows(&rows);
            assert_eq!(
                (keys(&sleep), keys(&sleep).len() == rows.rows_len()),
                (vec![12, 13], true),
                "U8 shrink: each survivor carries its own key: (keys, key len == gather rows)"
            );
        }

        /// T7 `Identity`: unchanged rows one gather on leave the latch in place, and the
        /// cursor is stamped with the current gather.
        #[test]
        fn rekey_rows_identity_keeps_the_latch_and_stamps() {
            let mut rows = RowIdentity::with_capacity(0);
            let mut sleep = IslandSleep::with_capacity(0, 0);
            gather(&mut rows, &[1, 2, 3], &[0, 1, 2]);
            sleep.rekey_rows(&rows);
            set_latch(&mut sleep, &[(true, 8), (false, 2), (true, 8)]);

            gather(&mut rows, &[1, 2, 3], &[]);
            assert_eq!(classify(&sleep, &rows), "Identity", "construction: rows unchanged one gather on");
            sleep.rekey_rows(&rows);
            assert_eq!(
                (latch(&sleep), sleep.cursor.synced_seq(), sleep.remap_resets(), sleep.reset_wake),
                (vec![(true, 8), (false, 2), (true, 8)], gather_seq(&rows), 0, false),
                "T7 Identity: (latch, synced_seq, remap_resets, reset_wake)"
            );
        }

        /// T7 `Rows`: the first gather of a fresh latch, a growth step with a new body, and a
        /// shrink step each permute the latch through `prev_row` and stamp.
        #[test]
        fn rekey_rows_rows_permutes_through_new_bodies_growth_and_shrink_and_stamps() {
            let mut rows = RowIdentity::with_capacity(0);
            let mut sleep = IslandSleep::with_capacity(0, 0);

            gather(&mut rows, &[1, 2, 3], &[0, 1, 2]);
            assert_eq!(classify(&sleep, &rows), "Rows", "construction: a fresh latch's first gather is Rows (P17)");
            sleep.rekey_rows(&rows);
            assert_eq!(
                (latch(&sleep), sleep.cursor.synced_seq(), sleep.remap_resets()),
                (vec![(false, 0); 3], gather_seq(&rows), 0),
                "T7 Rows, first gather: every row new and awake, cursor stamped"
            );
            set_latch(&mut sleep, &[(true, 8), (false, 2), (true, 8)]);

            // Growth: ids 3 and 1 move, id 9 is new in row 2, id 2 moves to the end.
            gather(&mut rows, &[3, 1, 9, 2], &[2]);
            assert_eq!(classify(&sleep, &rows), "Rows", "construction: growth step");
            sleep.rekey_rows(&rows);
            assert_eq!(
                (latch(&sleep), sleep.cursor.synced_seq(), sleep.remap_resets()),
                (vec![(true, 8), (true, 8), (false, 0), (false, 2)], gather_seq(&rows), 0),
                "T7 Rows, growth with a new body (NO_ROW): each body carries its own latch"
            );

            // Shrink: ids 1 and 9 are gone, 2 and 3 swap ends.
            gather(&mut rows, &[2, 3], &[]);
            assert_eq!(classify(&sleep, &rows), "Rows", "construction: shrink step");
            sleep.rekey_rows(&rows);
            assert_eq!(
                (latch(&sleep), sleep.cursor.synced_seq(), sleep.remap_resets()),
                (vec![(false, 2), (true, 8)], gather_seq(&rows), 0),
                "T7 Rows, shrink: each survivor carries its own latch"
            );
        }

        /// T7 `Reset`: a latch that missed a gather counts one reset, sizes to the gather,
        /// leaves a global wake pending and stamps; the next gather is `Identity` again.
        #[test]
        fn rekey_rows_reset_counts_once_leaves_a_wake_pending_and_stamps() {
            let mut rows = RowIdentity::with_capacity(0);
            let mut sleep = IslandSleep::with_capacity(0, 0);
            gather(&mut rows, &[1, 2, 3], &[0, 1, 2]);
            sleep.rekey_rows(&rows);
            set_latch(&mut sleep, &[(true, 8), (true, 8), (true, 8)]);

            gather(&mut rows, &[1, 2, 3], &[]); // the latch misses this gather
            gather(&mut rows, &[1, 2, 3, 4], &[3]);
            assert_eq!(classify(&sleep, &rows), "Reset", "construction: one gather was missed");
            sleep.rekey_rows(&rows);
            assert_eq!(
                (sleep.remap_resets(), sleep.reset_wake, sleep.latch.len(), sleep.cursor.synced_seq()),
                (1, true, 4, gather_seq(&rows)),
                "T7 Reset: (remap_resets, reset_wake, latch length, synced_seq)"
            );

            gather(&mut rows, &[1, 2, 3, 4], &[]);
            let next = classify(&sleep, &rows);
            sleep.rekey_rows(&rows);
            assert_eq!(
                (next, sleep.remap_resets(), sleep.cursor.synced_seq()),
                ("Identity", 1, gather_seq(&rows)),
                "T7 Reset liveness: the gather after a Reset is Identity and counts nothing"
            );
        }

        /// T7 `Rows` over a latch last sized by a direct drive: a previous row at or past the
        /// latch's length reads `(false, 0)`, and a row inside it carries its latch.
        #[test]
        fn rekey_rows_rows_reads_a_row_past_a_direct_drive_latch_as_awake() {
            let mut rows = RowIdentity::with_capacity(0);
            let mut sleep = IslandSleep::with_capacity(0, 0);
            gather(&mut rows, &[1, 2, 3, 4], &[0, 1, 2, 3]);
            sleep.rekey_rows(&rows);
            // A direct drive's `begin_step` sizes the latch to its own 2-row snapshot.
            sleep.sync_rows(2);
            set_latch(&mut sleep, &[(true, 8), (true, 8)]);

            gather(&mut rows, &[4, 3, 2, 1], &[]);
            assert_eq!(classify(&sleep, &rows), "Rows", "construction: a permutation one gather on");
            sleep.rekey_rows(&rows);
            assert_eq!(
                (latch(&sleep), sleep.cursor.synced_seq()),
                (vec![(false, 0), (false, 0), (true, 8), (true, 8)], gather_seq(&rows)),
                "T7 Rows past a direct-drive latch (old_len 2, m 4): rows 3 and 2 read awake"
            );
        }

        /// T7 direct drive: a scratch that never gathered is `Identity`; the latch stays as
        /// the direct drive left it and the stamp writes the instance's base.
        #[test]
        fn rekey_rows_on_a_scratch_that_never_gathered_is_identity() {
            let rows = RowIdentity::with_capacity(0);
            let mut sleep = IslandSleep::with_capacity(0, 0);
            sleep.sync_rows(2);
            set_latch(&mut sleep, &[(true, 8), (false, 1)]);
            assert_eq!(classify(&sleep, &rows), "Identity", "construction: never gathered");
            sleep.rekey_rows(&rows);
            assert_eq!(
                (latch(&sleep), sleep.cursor.synced_seq(), sleep.remap_resets(), sleep.reset_wake),
                (vec![(true, 8), (false, 1)], gather_seq(&rows), 0, false),
                "T7 direct drive: (latch, synced_seq, remap_resets, reset_wake)"
            );
        }
    }

    // L10 D9b, Decision 5: the explicit wake is a request count, latched by the broadphase and
    // served by the solve up to the latched count; the internal `Reset` wake is served by
    // whichever `begin_step` runs next.
    mod wake_requests {
        use crate::resources::{ConstraintGraph, IslandSleep};

        /// A sleep state over `rows` rows whose row 0 is latched asleep, with no island (an empty
        /// graph wakes no row by contact change).
        fn slept(graph: &ConstraintGraph, rows: usize) -> IslandSleep {
            let mut sleep = IslandSleep::with_capacity(0, 0);
            sleep.begin_step(graph, rows);
            sleep.force_sleep_row(0);
            sleep
        }

        #[test]
        fn wake_request_after_the_latch_stays_pending() {
            let graph = ConstraintGraph::with_capacity(0);
            let mut sleep = slept(&graph, 2);
            sleep.wake_all();
            // The broadphase latches the count; a second request lands after the latch.
            let upto = sleep.wake_requests();
            sleep.wake_all();
            sleep.begin_step_upto(&graph, 2, upto);
            assert!(!sleep.is_row_asleep(0), "the latched request is served: row 0 wakes");
            assert_eq!(sleep.wake_served, upto, "the step served exactly the latched count");
            let count = sleep.wake_requests();
            assert!(
                sleep.wake_pending(count),
                "the request raised after the latch is still pending for the next step"
            );
            sleep.force_sleep_row(0);
            sleep.begin_step_upto(&graph, 2, count);
            assert!(!sleep.is_row_asleep(0), "the next step serves the late request");
            assert!(!sleep.wake_pending(count), "nothing is pending once every request is served");
            // Anti-vacuity: with nothing pending the latch survives a step.
            sleep.force_sleep_row(0);
            sleep.begin_step_upto(&graph, 2, count);
            assert!(sleep.is_row_asleep(0), "anti-vacuity: no pending request, no wake");
        }

        #[test]
        fn reset_wake_is_served_whatever_upto() {
            let graph = ConstraintGraph::with_capacity(0);
            let mut sleep = slept(&graph, 2);
            sleep.reset_wake = true;
            let upto = sleep.wake_served;
            assert!(sleep.wake_pending(upto), "a Reset wake is pending at the served count");
            sleep.begin_step_upto(&graph, 2, upto);
            assert!(!sleep.is_row_asleep(0), "the Reset wake is served with no request pending");
            assert!(!sleep.reset_wake && !sleep.wake_pending(upto), "the Reset wake is spent");
        }

        #[test]
        fn direct_drive_begin_step_serves_every_request() {
            let graph = ConstraintGraph::with_capacity(0);
            let mut sleep = slept(&graph, 2);
            sleep.wake_all();
            sleep.wake_all();
            sleep.begin_step(&graph, 2);
            assert!(!sleep.is_row_asleep(0), "the live count is served");
            assert_eq!(sleep.wake_served, 2, "both requests are served");
        }
    }

    // Lane SL (design `sl/design_r2.md` §3, cut `sl/cut.md` C-1..C-6): the rest state, the settle
    // and the seal, driven directly through `begin_step` / `end_step` / `settle`. The pipeline's
    // behaviour is gated by `tests/sleep_velocity_zero.rs` and `tests/sleep_seal.rs`; these units
    // pin the state machine. Miri filter: `resources::tests::sl::`.
    mod sl {
        use crate::components::ColliderShape;
        use crate::manifold::{BodyIndex, Manifold};
        use crate::math::Vec3;
        use crate::resources::{
            BodyState, ConstraintGraph, DEFAULT_SLEEP_THRESHOLD, IslandSleep, PhysicsConfig,
            SdfNarrowphaseKernel, SealArm, SealEpoch, SleepSkip, TouchedMask,
        };
        use crate::row_identity::{REST_FROZEN, REST_NONE, REST_SETTLED};
        use crate::scratch_ids::touched_solver_id;
        use crate::sdf_query::SdfField;
        use boyko_sdf_math::{SdfEdit, sdf_op};

        /// The debounce every unit runs with: short, so a latch takes three steps.
        const FRAMES: u16 = 3;

        /// A unit dynamic box at `x`, carrying a sub-threshold velocity (W4c: speed²
        /// `1.25e-6 < 1e-4`), so a stash taken before the zero differs from the zeroed body.
        fn dynamic(x: f32) -> BodyState {
            BodyState {
                position: Vec3::new(x, 0.5, 0.0),
                linear_velocity: Vec3::new(1e-3, 0.0, 0.0),
                angular_velocity: Vec3::new(0.0, 5e-4, 0.0),
                inv_mass: 1.0,
                simulated: true,
                shape: ColliderShape::Box { half_extents: Vec3::new(0.5, 0.5, 0.5) },
                ..BodyState::default()
            }
        }

        /// The static floor box.
        fn floor() -> BodyState {
            BodyState {
                position: Vec3::new(0.0, -1.0, 0.0),
                shape: ColliderShape::Box { half_extents: Vec3::new(50.0, 1.0, 50.0) },
                ..BodyState::default()
            }
        }

        /// Three dynamic boxes (rows 0 to 2), then the floor (row 3).
        fn scene() -> Vec<BodyState> {
            vec![dynamic(0.0), dynamic(1.0), dynamic(2.0), floor()]
        }

        /// The graph of `pairs` over `bodies` (the partition reads only the two rows).
        fn graph(pairs: &[(u32, u32)], bodies: &[BodyState]) -> ConstraintGraph {
            let manifolds: Vec<Manifold> = pairs
                .iter()
                .map(|&(a, b)| Manifold::new(BodyIndex(a), BodyIndex(b)))
                .collect();
            let mut g = ConstraintGraph::with_capacity(0);
            g.build(&manifolds, bodies.len(), |r| {
                bodies.get(r as usize).is_some_and(|b| b.inv_mass != 0.0)
            });
            g
        }

        /// A touched mask with every one of `n` rows set: every unit's settling rows were solved
        /// (C-4: the settle's debug assert is its contract).
        fn touched_all(n: usize) -> TouchedMask {
            let mut t = TouchedMask::with_capacity(touched_solver_id(), n);
            t.reset(n);
            for r in 0..n {
                t.set(r);
            }
            t
        }

        fn arm(cfg: &PhysicsConfig) -> Option<SealArm<'_>> {
            Some(SealArm { cfg, field: None })
        }

        /// One direct step: `begin_step` then `end_step` over `g`.
        fn step(sleep: &mut IslandSleep, g: &ConstraintGraph, bodies: &[BodyState]) {
            sleep.begin_step(g, bodies.len());
            sleep.end_step(bodies, g, DEFAULT_SLEEP_THRESHOLD, FRAMES);
        }

        fn zero(b: &BodyState) -> bool {
            [b.linear_velocity, b.angular_velocity]
                .iter()
                .all(|v| v.x.to_bits() == 0 && v.y.to_bits() == 0 && v.z.to_bits() == 0)
        }

        /// The island `{0, 1, 2}` on the floor (three manifolds), latched through `end_step`:
        /// on return its rows are `REST_SETTLED` and the settle is pending.
        fn settled_island(bodies: &[BodyState]) -> (IslandSleep, ConstraintGraph) {
            let g = graph(&[(0, 3), (0, 1), (1, 2)], bodies);
            let mut sleep = IslandSleep::with_capacity(4, 4);
            for _ in 0..FRAMES {
                step(&mut sleep, &g, bodies);
            }
            assert!(
                (0..3).all(|r| sleep.rest_of(r) == REST_SETTLED) && sleep.settle_pending,
                "construction: the island went to sleep at the last end_step (rest {:?})",
                (0..3).map(|r| sleep.rest_of(r)).collect::<Vec<_>>()
            );
            (sleep, g)
        }

        /// T-SL2 (M-SL3): rows that latched while their island still had an unlatched member are
        /// not frozen when a later graph files them alone under a count equal to their key. They
        /// are solved one more step, settle (zeroed) at its `end_step`, and freeze at the next
        /// `begin_step`. The trunk froze them at once, with their solved velocity.
        #[test]
        fn an_island_that_never_went_to_sleep_does_not_freeze() {
            let mut bodies = scene();
            let n = bodies.len();
            // {0, 1} rest on the floor, row 2 moves on its own.
            bodies[2].linear_velocity = Vec3::new(1.0, 0.0, 0.0);
            let g_apart = graph(&[(0, 3), (0, 1), (2, 3)], &bodies);
            let mut sleep = IslandSleep::with_capacity(4, 4);
            for _ in 0..FRAMES - 1 {
                step(&mut sleep, &g_apart, &bodies);
            }
            // Row 2 stops and joins: {0, 1, 2}, three manifolds. Rows 0 and 1 latch, row 2 does
            // not, so the island does not go to sleep.
            bodies[2].linear_velocity = Vec3::ZERO;
            let g_merged = graph(&[(0, 3), (0, 1), (1, 2)], &bodies);
            step(&mut sleep, &g_merged, &bodies);
            assert_eq!(
                (
                    [0, 1, 2].map(|r| sleep.is_row_latched(r)),
                    [0, 1, 2].map(|r| sleep.rest_of(r)),
                    sleep.rows_put_to_sleep(),
                ),
                ([true, true, false], [REST_NONE; 3], 0),
                "construction: rows 0 and 1 latched inside an island with an unlatched member, and \
                 nothing went to sleep"
            );
            // {0, 1} alone, under three manifolds: the count their key holds.
            let g_alone = graph(&[(0, 3), (0, 1), (1, 3)], &bodies);
            sleep.begin_step(&g_alone, n);
            let isl = g_alone.island_of(0);
            assert_eq!(g_alone.island_of(1), isl, "construction: rows 0 and 1 share an island");
            assert!(
                !sleep.is_island_frozen(isl) && sleep.is_row_awake(0) && sleep.is_row_awake(1),
                "T-SL2: the island froze although it never went to sleep (its rows latched while \
                 row 2 was unlatched): it would freeze with its solved velocity"
            );
            assert_eq!(sleep.contact_wakes(), 0, "anti-vacuity: no contact wake kept it active");
            // It is solved this step, goes to sleep, and the settle zeroes it.
            sleep.end_step(&bodies, &g_alone, DEFAULT_SLEEP_THRESHOLD, FRAMES);
            sleep.settle(&mut bodies, &touched_all(n), None);
            assert_eq!(
                ([0, 1].map(|r| sleep.rest_of(r)), sleep.rows_put_to_sleep()),
                ([REST_SETTLED; 2], 2),
                "T-SL2: the island goes to sleep at the end of the step it was solved"
            );
            assert!(
                zero(&bodies[0]) && zero(&bodies[1]) && !zero(&bodies[2]),
                "T-SL2: the settle zeroes the sleeping island's rows and only them: {:?}",
                &bodies[..3]
            );
            sleep.begin_step(&g_alone, n);
            assert!(
                sleep.is_island_frozen(isl)
                    && !sleep.is_row_awake(0)
                    && [0, 1].map(|r| sleep.rest_of(r)) == [REST_FROZEN; 2],
                "T-SL2: the settled island freezes at the next begin_step"
            );
        }

        /// T-SL10 (M-SL9, M-SL11): on the sealed first step after a settle, a count change
        /// re-keys every settled row and the island freezes; with one watched row's position one
        /// ulp off, the same change wakes it by the trunk's rule.
        #[test]
        fn a_count_change_on_the_first_step_after_a_settle_is_rekeyed_when_sealed() {
            let cfg = PhysicsConfig::default();
            for nudge in [false, true] {
                let mut bodies = scene();
                let n = bodies.len();
                let (mut sleep, _) = settled_island(&bodies);
                sleep.settle(&mut bodies, &touched_all(n), arm(&cfg));
                assert!(
                    bodies[..3].iter().all(zero) && sleep.settles() == 1,
                    "construction: the armed settle zeroed the island and stashed a seal"
                );
                sleep.arm_seal_for_test();
                if nudge {
                    let x = &mut bodies[2].position.x;
                    *x = f32::from_bits(x.to_bits() + 1);
                }
                // Row 0 loses its floor manifold: the count falls from 3 to 2.
                let g_less = graph(&[(0, 1), (1, 2)], &bodies);
                let upto = sleep.wake_requests();
                sleep.begin_step_armed(&g_less, n, upto, &bodies, arm(&cfg));
                let isl = g_less.island_of(0);
                let read = (
                    sleep.is_island_frozen(isl),
                    [0, 1, 2].map(|r| sleep.is_row_latched(r)),
                    sleep.settle_rekeys(),
                    sleep.contact_wakes(),
                    sleep.seal_held(),
                    sleep.seal_breaks(),
                );
                if nudge {
                    assert_eq!(
                        read,
                        (false, [false; 3], 0, 3, 0, 1),
                        "T-SL10, a watched row moved by one ulp: the seal breaks and the count \
                         change wakes every row (frozen, latched, settle_rekeys, contact_wakes, \
                         seal_held, seal_breaks)"
                    );
                } else {
                    assert_eq!(
                        read,
                        (true, [true; 3], 3, 0, 1, 0),
                        "T-SL10, sealed: the count change re-keys the three settled rows and the \
                         island freezes (frozen, latched, settle_rekeys, contact_wakes, seal_held, \
                         seal_breaks)"
                    );
                    assert!(
                        [0, 1, 2].map(|r| sleep.rest_of(r)) == [REST_FROZEN; 3],
                        "T-SL10: the re-keyed rows are frozen"
                    );
                }
            }
        }

        /// T-SL17 (M-SL13): a seal serves only the first step after its settle. Island A
        /// (`{0, 1}`) settles armed and its seal holds on the next step; island B (`{2}`) settles
        /// at that step's end through a disarmed settle, so no new seal exists; B's count then
        /// changes on the step after, which must wake B by the trunk's rule, not re-key it with
        /// A's spent seal.
        #[test]
        fn a_seal_is_used_only_on_the_step_right_after_its_settle() {
            let cfg = PhysicsConfig::default();
            let mut bodies = scene();
            let n = bodies.len();
            let touched = touched_all(n);
            let g = graph(&[(0, 3), (0, 1), (2, 3)], &bodies);
            let mut sleep = IslandSleep::with_capacity(4, 4);
            // B moves on the first step, so it latches one step after A.
            bodies[2].linear_velocity = Vec3::new(1.0, 0.0, 0.0);
            step(&mut sleep, &g, &bodies);
            bodies[2].linear_velocity = Vec3::new(1e-3, 0.0, 0.0);
            for _ in 1..FRAMES {
                sleep.begin_step(&g, n);
                sleep.end_step(&bodies, &g, DEFAULT_SLEEP_THRESHOLD, FRAMES);
            }
            assert_eq!(
                [0, 1, 2].map(|r| sleep.rest_of(r)),
                [REST_SETTLED, REST_SETTLED, REST_NONE],
                "construction: A went to sleep, B did not"
            );
            sleep.settle(&mut bodies, &touched, arm(&cfg));
            sleep.arm_seal_for_test();
            let upto = sleep.wake_requests();
            sleep.begin_step_armed(&g, n, upto, &bodies, arm(&cfg));
            sleep.end_step(&bodies, &g, DEFAULT_SLEEP_THRESHOLD, FRAMES);
            sleep.settle(&mut bodies, &touched, None);
            assert_eq!(
                (
                    sleep.is_island_frozen(g.island_of(0)),
                    sleep.rest_of(2),
                    sleep.seal_held(),
                    sleep.seal_breaks(),
                ),
                (true, REST_SETTLED, 1, 0),
                "construction: A's seal held and A froze; B went to sleep through a disarmed \
                 settle (A frozen, B's rest, seal_held, seal_breaks)"
            );
            // B gains a manifold: its count changes on the first step after its settle.
            let g_b = graph(&[(0, 3), (0, 1), (2, 3), (2, 3)], &bodies);
            let upto = sleep.wake_requests();
            sleep.begin_step_armed(&g_b, n, upto, &bodies, arm(&cfg));
            assert_eq!(
                (
                    sleep.is_row_awake(2),
                    sleep.is_row_latched(2),
                    sleep.contact_wakes(),
                    sleep.settle_rekeys(),
                    sleep.seal_held(),
                ),
                (true, false, 1, 0, 1),
                "T-SL17: B's count change on an unsealed step wakes it (awake, latched, \
                 contact_wakes, settle_rekeys, seal_held): a spent seal must not serve a later step"
            );
        }

        /// T-SL18 (M-SL14, cut C-1): `end_step` unlatching a frozen row clears its rest state, so
        /// the fold, which reads `rest`, makes its island active on the next step: the one-step-late
        /// energy wake of a velocity written into a frozen body.
        #[test]
        fn an_end_step_unlatch_of_a_frozen_row_leaves_its_rest_state() {
            let mut bodies = scene();
            let n = bodies.len();
            let g = graph(&[(0, 3), (0, 1), (1, 2)], &bodies);
            let mut sleep = IslandSleep::with_capacity(4, 4);
            sleep.begin_step(&g, n);
            for r in 0..3 {
                sleep.force_sleep_row(r);
            }
            sleep.begin_step(&g, n);
            let isl = g.island_of(0);
            assert!(sleep.is_island_frozen(isl), "construction: the island is frozen");
            // A velocity above the threshold reaches the frozen island's energy.
            bodies[1].linear_velocity = Vec3::new(1.0, 0.0, 0.0);
            sleep.end_step(&bodies, &g, DEFAULT_SLEEP_THRESHOLD, FRAMES);
            assert_eq!(
                ([0, 1, 2].map(|r| sleep.is_row_latched(r)), [0, 1, 2].map(|r| sleep.rest_of(r))),
                ([false; 3], [REST_NONE; 3]),
                "T-SL18: an unlatched row keeps no rest state (latched, rest)"
            );
            sleep.begin_step(&g, n);
            assert!(
                !sleep.is_island_frozen(isl) && sleep.is_row_awake(1),
                "T-SL18: the island must be solved on the step after its energy unlatched it"
            );
        }

        /// T-SL19a (M-SL15a): a disarmed settle zeroes but stashes nothing, so even an armed
        /// `begin_step` with the test hook finds no seal: the count change wakes by the trunk's
        /// rule and no seal is evaluated.
        #[test]
        fn a_disarmed_settle_arms_no_seal() {
            let cfg = PhysicsConfig::default();
            let mut bodies = scene();
            let n = bodies.len();
            let (mut sleep, _) = settled_island(&bodies);
            sleep.settle(&mut bodies, &touched_all(n), None);
            assert!(bodies[..3].iter().all(zero), "T-SL19a: a disarmed settle still zeroes");
            sleep.arm_seal_for_test();
            let g_less = graph(&[(0, 1), (1, 2)], &bodies);
            let upto = sleep.wake_requests();
            sleep.begin_step_armed(&g_less, n, upto, &bodies, arm(&cfg));
            assert_eq!(
                (
                    sleep.contact_wakes(),
                    sleep.settle_rekeys(),
                    sleep.settles(),
                    sleep.seal_held(),
                    sleep.seal_breaks(),
                ),
                (3, 0, 0, 0, 0),
                "T-SL19a: (contact_wakes, settle_rekeys, settles, seal_held, seal_breaks): no seal \
                 exists, so none is evaluated, and the count change wakes every row"
            );
        }

        /// T-SL19b (M-SL15b): a disarmed `begin_step` never seals; a live seal reaching it is
        /// spent as a break, and the count change wakes by the trunk's rule.
        #[test]
        fn a_disarmed_begin_step_never_seals() {
            let cfg = PhysicsConfig::default();
            let mut bodies = scene();
            let n = bodies.len();
            let (mut sleep, _) = settled_island(&bodies);
            sleep.settle(&mut bodies, &touched_all(n), arm(&cfg));
            sleep.arm_seal_for_test();
            let g_less = graph(&[(0, 1), (1, 2)], &bodies);
            sleep.begin_step(&g_less, n);
            assert_eq!(
                (sleep.contact_wakes(), sleep.settle_rekeys(), sleep.seal_held(), sleep.seal_breaks()),
                (3, 0, 0, 1),
                "T-SL19b: (contact_wakes, settle_rekeys, seal_held, seal_breaks)"
            );
        }

        /// T-SL20 (cut C-5, O5): the seal's epoch is L10's epoch without its mode and warm
        /// fields. It differs for every narrowphase configuration field and for the SDF edits,
        /// and is equal across the sleep-skip modes and the warm start.
        #[test]
        fn the_seal_epoch_reads_every_narrowphase_input_and_no_mode() {
            let base = PhysicsConfig::default();
            let of = |cfg: &PhysicsConfig| SealEpoch::of(cfg, None);
            let edits = |w: f32| {
                SdfField::from_edits(&[SdfEdit::box_shape(
                    [0.0, -1.0, 0.0],
                    [w, 1.0, w],
                    sdf_op::UNION,
                    0.0,
                )])
            };
            let changed: [(&str, PhysicsConfig); 6] = [
                ("speculative_distance", PhysicsConfig { speculative_distance: 0.01, ..base }),
                (
                    "speculative_velocity_cap",
                    PhysicsConfig { speculative_velocity_cap: 0.25, ..base },
                ),
                ("contact_reuse", PhysicsConfig { contact_reuse: !base.contact_reuse, ..base }),
                ("contact_reuse_distance", PhysicsConfig { contact_reuse_distance: 0.002, ..base }),
                ("dt", PhysicsConfig { dt: base.dt + 1.0 / 60.0, ..base }),
                (
                    "sdf_narrowphase",
                    PhysicsConfig {
                        sdf_narrowphase: match base.sdf_narrowphase {
                            SdfNarrowphaseKernel::Scalar => SdfNarrowphaseKernel::Avx2,
                            SdfNarrowphaseKernel::Avx2 => SdfNarrowphaseKernel::Scalar,
                        },
                        ..base
                    },
                ),
            ];
            for (field, cfg) in &changed {
                assert_ne!(of(cfg), of(&base), "T-SL20: the seal epoch ignores `{field}`");
            }
            let (a, b) = (edits(50.0), edits(49.0));
            assert_ne!(SealEpoch::of(&base, Some(&a)), of(&base), "T-SL20: a field and none");
            assert_ne!(
                SealEpoch::of(&base, Some(&a)),
                SealEpoch::of(&base, Some(&b)),
                "T-SL20: an SDF edit's bits"
            );
            assert_eq!(SealEpoch::of(&base, Some(&a)), SealEpoch::of(&base, Some(&edits(50.0))));
            for cfg in [
                PhysicsConfig { sleep_skip: SleepSkip::Off, ..base },
                PhysicsConfig { sleep_skip: SleepSkip::Sets, ..base },
                PhysicsConfig { warm_start: !base.warm_start, ..base },
                PhysicsConfig { gravity: Vec3::ZERO, ..base },
            ] {
                assert_eq!(
                    of(&cfg),
                    of(&base),
                    "T-SL20: the seal epoch must not read the mode, the warm start or the gravity"
                );
            }
        }
    }
