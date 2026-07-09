//! The [`SoftBody`] component and its validating constructors (Physics O11 SP1,
//! plan D5).
//!
//! [`SoftBody`] is an ordinary `#[derive(Component)]` column storing one soft body
//! as Structure-of-Arrays BY AXIS: position, previous position, and velocity each
//! live in three `Vec<f32>` (x / y / z), with a parallel `inv_mass` column and an
//! immutable distance-constraint topology (`c_a` / `c_b` endpoint indices, `c_rest`
//! rest lengths, `c_compliance` per-constraint α). Every `Vec` is sized ONCE at
//! construction and refilled in place each substep — the solver holds no scratch
//! and the step does ZERO heap allocation (principle 5).

use boyko_macros::Component;

use crate::math::Vec3;
use crate::soft::solver::DENOM_EPS;

/// Construction-time validation failure for a [`SoftBody`] (plan SP1).
///
/// All variants are caught at construction by [`SoftBody::from_mesh`] /
/// [`SoftBody::from_mesh_per_edge`], so a constructed `SoftBody` is always
/// well-formed: every edge endpoint is in range, the particle count fits a `u32`,
/// no edge is a self-loop, and all positions / rest lengths are finite. The solver
/// therefore never re-validates on the hot path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoftBodyError {
    /// An edge references a particle index `>= positions.len()`.
    IndexOutOfRange,
    /// A position component or a supplied rest length is `NaN` / `±Inf`.
    NonFinite,
    /// A compliance value is negative. XPBD compliance α (inverse stiffness) must be
    /// `>= 0`: a negative α makes the constraint denominator `wsum + α/dt²` cross
    /// zero, driving the Lagrange step (hence the corrected position) to `±Inf` /
    /// `NaN` in release — silently voiding the serial/colored bit-equality keystone.
    /// Rejected at construction so the solve never divides through a poisoned denom.
    NegativeCompliance,
    /// A slice length disagrees with another it must match (e.g. `inv_masses.len()
    /// != positions.len()`, or `compliance_per_edge.len() != edges.len()`).
    LengthMismatch,
    /// `positions.len() > u32::MAX` — the endpoint indices are `u32`.
    TooManyParticles,
    /// An edge connects a particle to itself (`a == b`) — a degenerate constraint.
    SelfEdge,
    /// A tetrahedron is degenerate at rest (SP2 D1): its four vertices are not
    /// distinct, or they are coplanar (`|V0| < DENOM_EPS`), so the signed-volume
    /// constraint has no usable gradient. Rejected at construction so the volume
    /// sweep never divides by a vanishing denominator on the hot path.
    DegenerateTet,
}

/// One XPBD soft body — a particle cloud tied by distance constraints (plan SP1,
/// D5).
///
/// **SoA by axis (preallocated, zero per-step alloc).** Particle state is split
/// across `x`/`y`/`z` `Vec<f32>` so each substep streams a tight, vectorizable
/// column. `pos_*` is the live position (written every substep); `prev_*` is the
/// previous position (carried across the substep for the velocity update); `vel_*`
/// is the velocity (frame-carried). `inv_mass` is the per-particle inverse mass —
/// `0.0` PINS the particle (the solver freezes it).
///
/// **Immutable topology.** `c_a` / `c_b` are the constraint endpoint indices,
/// `c_rest` the rest length `L0`, and `c_compliance` the PER-CONSTRAINT XPBD
/// compliance α (inverse stiffness; `0.0` = perfectly stiff). These are set once at
/// construction and never resized.
///
/// All ten particle columns have length `particle_count()`; all four constraint
/// columns have length `constraint_count()`. The constructors enforce this, and the
/// solver `debug_assert!`s it on entry.
#[derive(Component, Clone, Debug, Default)]
pub struct SoftBody {
    /// Current position X (written every substep).
    pub pos_x: Vec<f32>,
    /// Current position Y.
    pub pos_y: Vec<f32>,
    /// Current position Z.
    pub pos_z: Vec<f32>,
    /// Previous position X (frame-carried; rewritten at the start of each substep).
    pub prev_x: Vec<f32>,
    /// Previous position Y.
    pub prev_y: Vec<f32>,
    /// Previous position Z.
    pub prev_z: Vec<f32>,
    /// Velocity X (frame-carried).
    pub vel_x: Vec<f32>,
    /// Velocity Y.
    pub vel_y: Vec<f32>,
    /// Velocity Z.
    pub vel_z: Vec<f32>,
    /// Per-particle inverse mass; `0.0` = pinned (frozen by the solver).
    pub inv_mass: Vec<f32>,
    /// Distance-constraint endpoint A (particle index `< particle_count()`).
    pub c_a: Vec<u32>,
    /// Distance-constraint endpoint B (particle index `< particle_count()`).
    pub c_b: Vec<u32>,
    /// Per-constraint rest length `L0`.
    pub c_rest: Vec<f32>,
    /// Per-constraint XPBD compliance α (inverse stiffness; `0.0` = perfectly
    /// stiff).
    pub c_compliance: Vec<f32>,
    /// Tetrahedron vertex 0 (particle index `< particle_count()`) — SP2 D1.
    ///
    /// `t0`/`t1`/`t2`/`t3` are the four corners of each volume-constraint
    /// tetrahedron, all length [`tet_count()`](Self::tet_count). SP1-only bodies
    /// have empty tet columns, so the volume sweep `0..tet_count()` is a per-body
    /// no-op (the per-body 0%-gate).
    pub t0: Vec<u32>,
    /// Tetrahedron vertex 1 (SP2 D1).
    pub t1: Vec<u32>,
    /// Tetrahedron vertex 2 (SP2 D1).
    pub t2: Vec<u32>,
    /// Tetrahedron vertex 3 (SP2 D1).
    pub t3: Vec<u32>,
    /// Per-tet SIGNED rest volume `V0` (SP2 D1) — the target of the volume
    /// constraint `C = V - V0`. Computed at construction with the IDENTICAL op
    /// sequence as the solve so `C` is exactly `0` at rest.
    pub t_rest: Vec<f32>,
    /// Per-tet XPBD compliance α (inverse stiffness; `0.0` = perfectly stiff) —
    /// SP2 D1.
    pub t_compliance: Vec<f32>,
    /// Per-particle coupling velocity-baseline position X (SP2 D6/D4, W1) —
    /// scratch.
    ///
    /// Length `particle_count()`, preallocated, written ONLY on the soft↔rigid
    /// coupling path. It records each coupled particle's position BEFORE the coupling
    /// push so the velocity update can exclude that push (the momentum exchange is
    /// carried by the rigid reaction, not the position diff); the post-coupling
    /// SDF-collide displacement is then folded in (SP2 W1), so the baseline excludes
    /// ONLY the coupling push and the SDF push still reaches the coupled velocity.
    /// Untouched (and unread) on the SP1 / non-coupling path.
    pub coupling_prev_x: Vec<f32>,
    /// Per-particle pre-coupling-push position Y (SP2 D6/D4) — scratch.
    pub coupling_prev_y: Vec<f32>,
    /// Per-particle pre-coupling-push position Z (SP2 D6/D4) — scratch.
    pub coupling_prev_z: Vec<f32>,
    /// Per-particle accumulated coupling velocity delta X (SP2 D7) — scratch.
    ///
    /// Length `particle_count()`, preallocated, written ONLY on the coupling path.
    /// Holds the D7 particle impulse `p_imp · w_particle` for a particle the
    /// coupling pushed this substep, so the velocity update can apply the D4
    /// baseline `(coupling_prev − prev)·inv_h` AND add the D7 momentum exchange on
    /// top (the position diff alone excludes the push). Untouched / unread off the
    /// coupling path.
    pub coupling_dv_x: Vec<f32>,
    /// Per-particle accumulated coupling velocity delta Y (SP2 D7) — scratch.
    pub coupling_dv_y: Vec<f32>,
    /// Per-particle accumulated coupling velocity delta Z (SP2 D7) — scratch.
    pub coupling_dv_z: Vec<f32>,
    /// Per-particle "was pushed by coupling this substep" flag (SP2 D6) — scratch.
    ///
    /// Length `particle_count()`, preallocated, `1` iff the coupling pushed the
    /// particle this substep (reset to `0` each substep before the coupling pass).
    /// The velocity update reads it to choose the D4 `coupling_prev` baseline +
    /// D7 delta over the plain SP1 position diff. Untouched / unread off the
    /// coupling path.
    pub coupling_hit: Vec<u8>,
    /// SP3 self-collision spatial-hash CSR bucket offsets — scratch.
    ///
    /// Length `self_table_size() + 1`, preallocated, written ONLY on the SP3
    /// self-collision path (`PhysicsConfig::self_collision_iters > 0` and
    /// `particle_radius > 0`). `sc_cell_start[b]..sc_cell_start[b + 1]` is the slice
    /// of [`sc_cell_items`](Self::sc_cell_items) holding the particle indices hashed
    /// to bucket `b` (a counting-sort compressed-sparse-row table rebuilt each
    /// substep with zero allocation). Untouched / unread off the self-collision path.
    pub sc_cell_start: Vec<u32>,
    /// SP3 self-collision spatial-hash CSR particle indices — scratch.
    ///
    /// Length `particle_count()`, preallocated. The counting-sort scatter target:
    /// `sc_cell_items[sc_cell_start[b]..sc_cell_start[b + 1]]` are the particle
    /// indices in bucket `b`, ascending within a bucket (a stable scatter), so the
    /// candidate visit order is deterministic. Untouched / unread off the
    /// self-collision path.
    pub sc_cell_items: Vec<u32>,
    /// SP3 self-collision spatial-hash per-bucket scatter cursor — scratch.
    ///
    /// Length `self_table_size() + 1`, preallocated. A transient running cursor the
    /// counting-sort scatter advances per bucket; reset from `sc_cell_start` each
    /// rebuild. Untouched / unread off the self-collision path.
    pub sc_cursor: Vec<u32>,
    /// Collision radius of each particle against the SDF (world units; `>= 0`).
    pub particle_radius: f32,
}

impl SoftBody {
    /// Builds a soft body from a mesh, broadcasting a single `compliance` to every
    /// constraint (plan SP1 API).
    ///
    /// `positions` are the initial particle positions; `inv_masses` (same length)
    /// the per-particle inverse masses (`0.0` pins a particle); `edges` the distance
    /// constraints as `(a, b)` particle-index pairs. `rest` optionally supplies a
    /// rest length per edge — when `None`, each edge's rest length is the initial
    /// distance `|pos[a] - pos[b]|`. `compliance` (broadcast to every constraint)
    /// is the XPBD inverse stiffness; `radius` is the per-particle SDF collision
    /// radius.
    ///
    /// # Errors
    ///
    /// - [`SoftBodyError::TooManyParticles`] if `positions.len() > u32::MAX`.
    /// - [`SoftBodyError::LengthMismatch`] if `inv_masses.len() != positions.len()`,
    ///   or `rest` is `Some` with `rest.len() != edges.len()`.
    /// - [`SoftBodyError::NonFinite`] if any position component, inverse mass,
    ///   supplied rest length, or `compliance` is not finite, or `radius < 0` /
    ///   non-finite.
    /// - [`SoftBodyError::NegativeCompliance`] if `compliance < 0`.
    /// - [`SoftBodyError::IndexOutOfRange`] if an edge endpoint is `>=
    ///   positions.len()`.
    /// - [`SoftBodyError::SelfEdge`] if an edge has `a == b`.
    pub fn from_mesh(
        positions: &[[f32; 3]],
        inv_masses: &[f32],
        edges: &[(u32, u32)],
        rest: Option<&[f32]>,
        compliance: f32,
        radius: f32,
    ) -> Result<Self, SoftBodyError> {
        // Broadcast the single compliance to a per-edge view, then reuse the
        // per-edge constructor (one validation path, no duplication).
        Self::build(
            positions,
            inv_masses,
            edges,
            rest,
            Compliance::Uniform(compliance),
            radius,
        )
    }

    /// Builds a soft body with a PER-EDGE compliance (plan SP1 API).
    ///
    /// Identical to [`from_mesh`](Self::from_mesh) except `compliance_per_edge`
    /// supplies the XPBD compliance of each constraint individually; it must have
    /// the same length as `edges`.
    ///
    /// # Errors
    ///
    /// As [`from_mesh`](Self::from_mesh), plus [`SoftBodyError::LengthMismatch`] if
    /// `compliance_per_edge.len() != edges.len()`. A non-finite entry yields
    /// [`SoftBodyError::NonFinite`], a negative one [`SoftBodyError::NegativeCompliance`].
    pub fn from_mesh_per_edge(
        positions: &[[f32; 3]],
        inv_masses: &[f32],
        edges: &[(u32, u32)],
        rest: Option<&[f32]>,
        compliance_per_edge: &[f32],
        radius: f32,
    ) -> Result<Self, SoftBodyError> {
        if compliance_per_edge.len() != edges.len() {
            return Err(SoftBodyError::LengthMismatch);
        }
        Self::build(
            positions,
            inv_masses,
            edges,
            rest,
            Compliance::PerEdge(compliance_per_edge),
            radius,
        )
    }

    /// Builds a soft body with distance constraints AND volume-constraint
    /// tetrahedra (SP2 D1/D2).
    ///
    /// The particle / edge build is identical to [`from_mesh`](Self::from_mesh)
    /// (one `edge_compliance` broadcast to every edge; `rest_len` optionally
    /// supplies per-edge rest lengths, else the initial distance). `tets` are the
    /// volume constraints as `(v0, v1, v2, v3)` particle-index quads;
    /// `tet_compliance` is broadcast to every tet. `rest_vol` optionally supplies a
    /// per-tet signed rest volume `V0`; when `None`, each `V0` is computed from the
    /// initial positions with the IDENTICAL op sequence as the solve, so the
    /// constraint `C = V - V0` is exactly `0` at rest.
    ///
    /// # Errors
    ///
    /// As [`from_mesh`](Self::from_mesh), plus:
    /// - [`SoftBodyError::LengthMismatch`] if `rest_vol` is `Some` with
    ///   `rest_vol.len() != tets.len()`.
    /// - [`SoftBodyError::IndexOutOfRange`] if a tet vertex is `>= positions.len()`.
    /// - [`SoftBodyError::NonFinite`] if `tet_compliance` or a supplied `rest_vol`
    ///   is not finite.
    /// - [`SoftBodyError::NegativeCompliance`] if `edge_compliance < 0` or
    ///   `tet_compliance < 0`.
    /// - [`SoftBodyError::DegenerateTet`] if a tet's four vertices are not distinct,
    ///   or are coplanar at rest (`|V0| < DENOM_EPS` — no usable gradient).
    #[allow(clippy::too_many_arguments)]
    pub fn from_tet_mesh(
        positions: &[[f32; 3]],
        inv_masses: &[f32],
        edges: &[(u32, u32)],
        tets: &[(u32, u32, u32, u32)],
        rest_len: Option<&[f32]>,
        rest_vol: Option<&[f32]>,
        edge_compliance: f32,
        tet_compliance: f32,
        radius: f32,
    ) -> Result<Self, SoftBodyError> {
        if !tet_compliance.is_finite() {
            return Err(SoftBodyError::NonFinite);
        }
        // A negative tet compliance poisons the volume-constraint denominator the
        // same way a negative edge compliance does (see `NegativeCompliance`); reject
        // it up front so the volume solve never divides through a poisoned denom.
        if tet_compliance < 0.0 {
            return Err(SoftBodyError::NegativeCompliance);
        }
        if let Some(rv) = rest_vol
            && rv.len() != tets.len()
        {
            return Err(SoftBodyError::LengthMismatch);
        }
        if let Some(rv) = rest_vol
            && rv.iter().any(|v| !v.is_finite())
        {
            return Err(SoftBodyError::NonFinite);
        }

        // Reuse the SP1 validating build for particles + edges (byte-identical to
        // `from_mesh`); then validate + append the tet columns.
        let mut body = Self::build(
            positions,
            inv_masses,
            edges,
            rest_len,
            Compliance::Uniform(edge_compliance),
            radius,
        )?;

        let n_u32 = body.particle_count() as u32;
        let k = tets.len();
        let mut t0 = Vec::with_capacity(k);
        let mut t1 = Vec::with_capacity(k);
        let mut t2 = Vec::with_capacity(k);
        let mut t3 = Vec::with_capacity(k);
        let mut t_rest = Vec::with_capacity(k);
        for (ti, &(a, b, c, d)) in tets.iter().enumerate() {
            if a >= n_u32 || b >= n_u32 || c >= n_u32 || d >= n_u32 {
                return Err(SoftBodyError::IndexOutOfRange);
            }
            if a == b || a == c || a == d || b == c || b == d || c == d {
                return Err(SoftBodyError::DegenerateTet);
            }
            // Compute the signed rest volume with the IDENTICAL op sequence as the
            // solve's `project_volume` (edge-anchored at p0, pinned cross operand
            // order, dot left-to-right, the `1/6` factor kept) so the runtime
            // `C = V - V0` is exactly `0` at rest when `rest_vol` is `None`.
            let p0 = Self::particle_pos(&body, a as usize);
            let p1 = Self::particle_pos(&body, b as usize);
            let p2 = Self::particle_pos(&body, c as usize);
            let p3 = Self::particle_pos(&body, d as usize);
            let e1 = p1 - p0;
            let e2 = p2 - p0;
            let e3 = p3 - p0;
            let v0_computed = (1.0 / 6.0) * e1.cross(e2).dot(e3);
            let v0 = match rest_vol {
                Some(rv) => rv[ti],
                None => v0_computed,
            };
            // Reject a coplanar/degenerate tet (no usable signed-volume gradient).
            // The check is on the GEOMETRY (`v0_computed`), not the supplied target,
            // so a finite-but-tiny authored `rest_vol` cannot smuggle a collapsed
            // tet past the guard.
            if v0_computed.abs() < DENOM_EPS {
                return Err(SoftBodyError::DegenerateTet);
            }
            t0.push(a);
            t1.push(b);
            t2.push(c);
            t3.push(d);
            t_rest.push(v0);
        }

        body.t0 = t0;
        body.t1 = t1;
        body.t2 = t2;
        body.t3 = t3;
        body.t_rest = t_rest;
        body.t_compliance = vec![tet_compliance; k];
        Ok(body)
    }

    /// Reads particle `i`'s current position as a [`Vec3`] (construction helper for
    /// the rest-volume computation).
    #[inline]
    fn particle_pos(body: &Self, i: usize) -> Vec3 {
        Vec3::new(body.pos_x[i], body.pos_y[i], body.pos_z[i])
    }

    /// Number of particles (the length of every particle column).
    #[inline]
    pub fn particle_count(&self) -> usize {
        self.pos_x.len()
    }

    /// Number of distance constraints (the length of every constraint column).
    #[inline]
    pub fn constraint_count(&self) -> usize {
        self.c_a.len()
    }

    /// Number of volume-constraint tetrahedra (the length of every tet column) —
    /// SP2 D1. `0` for an SP1-only body, so the volume sweep is a per-body no-op.
    #[inline]
    pub fn tet_count(&self) -> usize {
        self.t0.len()
    }

    /// SP3 self-collision spatial-hash table size `T = next_pow2(2·n)` (a power of
    /// two so the cell hash masks with `T - 1`).
    ///
    /// `0` for an empty body (`n == 0`), so the self-collision pass is a per-body
    /// no-op. Otherwise `T >= 2` and `T.is_power_of_two()`. The CSR offset columns
    /// ([`sc_cell_start`](Self::sc_cell_start) / [`sc_cursor`](Self::sc_cursor)) have
    /// length `T + 1`.
    #[inline]
    pub fn self_table_size(&self) -> usize {
        self_table_size_for(self.particle_count())
    }

    /// The single validating construction path shared by both public constructors.
    ///
    /// Validates everything up front (so the hot path never re-checks), then builds
    /// the SoA columns sized exactly once.
    fn build(
        positions: &[[f32; 3]],
        inv_masses: &[f32],
        edges: &[(u32, u32)],
        rest: Option<&[f32]>,
        compliance: Compliance<'_>,
        radius: f32,
    ) -> Result<Self, SoftBodyError> {
        let n = positions.len();
        if n > u32::MAX as usize {
            return Err(SoftBodyError::TooManyParticles);
        }
        if inv_masses.len() != n {
            return Err(SoftBodyError::LengthMismatch);
        }
        if let Some(r) = rest
            && r.len() != edges.len()
        {
            return Err(SoftBodyError::LengthMismatch);
        }
        if !radius.is_finite() || radius < 0.0 {
            return Err(SoftBodyError::NonFinite);
        }

        // Validate particle data (finite positions + inverse masses).
        for (p, &w) in positions.iter().zip(inv_masses.iter()) {
            if !p[0].is_finite() || !p[1].is_finite() || !p[2].is_finite() || !w.is_finite() {
                return Err(SoftBodyError::NonFinite);
            }
        }

        // Validate the topology: every endpoint in range, no self-edge. (The cast
        // is safe: `n <= u32::MAX` was checked above.)
        let n_u32 = n as u32;
        for &(a, b) in edges {
            if a == b {
                return Err(SoftBodyError::SelfEdge);
            }
            if a >= n_u32 || b >= n_u32 {
                return Err(SoftBodyError::IndexOutOfRange);
            }
        }
        if let Some(r) = rest
            && r.iter().any(|v| !v.is_finite())
        {
            return Err(SoftBodyError::NonFinite);
        }

        // Validate edge compliance (finiteness AND non-negativity). A non-finite or
        // negative α poisons the constraint denominator on the hot path (see
        // `NegativeCompliance`), so it is rejected here in the shared funnel — the
        // solver never re-checks. `Uniform` broadcasts one α; `PerEdge` is a slice
        // whose length the caller already matched to `edges`.
        match &compliance {
            Compliance::Uniform(alpha) => {
                if !alpha.is_finite() {
                    return Err(SoftBodyError::NonFinite);
                }
                if *alpha < 0.0 {
                    return Err(SoftBodyError::NegativeCompliance);
                }
            }
            Compliance::PerEdge(slice) => {
                if slice.iter().any(|a| !a.is_finite()) {
                    return Err(SoftBodyError::NonFinite);
                }
                if slice.iter().any(|a| *a < 0.0) {
                    return Err(SoftBodyError::NegativeCompliance);
                }
            }
        }

        // All checks passed — build the SoA columns sized exactly once.
        let m = edges.len();
        // SP3 self-collision hash table size `next_pow2(2n)` (`0` for an empty body).
        // Routed through the SAME helper as `self_table_size()` so the build-time
        // scratch sizing and the runtime `debug_assert` can never desync (O1). The CSR
        // offset columns are `sc_table + 1`.
        let sc_table = self_table_size_for(n);
        let mut pos_x = Vec::with_capacity(n);
        let mut pos_y = Vec::with_capacity(n);
        let mut pos_z = Vec::with_capacity(n);
        for p in positions {
            pos_x.push(p[0]);
            pos_y.push(p[1]);
            pos_z.push(p[2]);
        }
        // `prev_*` seed = current position; `vel_*` seed = zero (rest start).
        let prev_x = pos_x.clone();
        let prev_y = pos_y.clone();
        let prev_z = pos_z.clone();
        let vel_x = vec![0.0; n];
        let vel_y = vec![0.0; n];
        let vel_z = vec![0.0; n];
        let inv_mass = inv_masses.to_vec();

        let mut c_a = Vec::with_capacity(m);
        let mut c_b = Vec::with_capacity(m);
        for &(a, b) in edges {
            c_a.push(a);
            c_b.push(b);
        }

        let c_rest = match rest {
            Some(r) => r.to_vec(),
            None => {
                let mut out = Vec::with_capacity(m);
                for &(a, b) in edges {
                    let (a, b) = (a as usize, b as usize);
                    let dx = pos_x[a] - pos_x[b];
                    let dy = pos_y[a] - pos_y[b];
                    let dz = pos_z[a] - pos_z[b];
                    // EXACT sqrt only (determinism boundary) — no `rsqrt`.
                    out.push((dx * dx + dy * dy + dz * dz).sqrt());
                }
                out
            }
        };

        let c_compliance = match compliance {
            Compliance::Uniform(alpha) => vec![alpha; m],
            Compliance::PerEdge(slice) => slice.to_vec(),
        };

        Ok(Self {
            pos_x,
            pos_y,
            pos_z,
            prev_x,
            prev_y,
            prev_z,
            vel_x,
            vel_y,
            vel_z,
            inv_mass,
            c_a,
            c_b,
            c_rest,
            c_compliance,
            // SP1-only body: empty tet columns ⇒ the volume sweep `0..0` is a
            // per-body no-op (the SP2 0%-gate). The coupling scratch is
            // preallocated to `n` (so the coupling path is zero per-step alloc)
            // but never read on the SP1 / non-coupling path.
            t0: Vec::new(),
            t1: Vec::new(),
            t2: Vec::new(),
            t3: Vec::new(),
            t_rest: Vec::new(),
            t_compliance: Vec::new(),
            coupling_prev_x: vec![0.0; n],
            coupling_prev_y: vec![0.0; n],
            coupling_prev_z: vec![0.0; n],
            coupling_dv_x: vec![0.0; n],
            coupling_dv_y: vec![0.0; n],
            coupling_dv_z: vec![0.0; n],
            coupling_hit: vec![0; n],
            // SP3 self-collision spatial-hash scratch, sized once. `table` is
            // `next_pow2(2n)` (`0` for an empty body); the CSR offset columns are
            // `table + 1`. Preallocated so the self-collision rebuild is zero
            // per-step alloc, but never read off the SP3 path (the SP3 0%-gate).
            sc_cell_start: vec![0; sc_table + 1],
            sc_cell_items: vec![0; n],
            sc_cursor: vec![0; sc_table + 1],
            particle_radius: radius,
        })
    }
}

/// The SP3 self-collision spatial-hash table size for a body of `n` particles:
/// `next_pow2(2n)` (a power of two so the cell hash masks with `T - 1`), or `0` for
/// `n == 0` (an empty body — the pass is a per-body no-op).
///
/// The SINGLE source of truth shared by [`SoftBody::self_table_size`] (the runtime
/// query + `debug_assert`) and [`SoftBody::build`] (the scratch sizing), so the two
/// can never desync. `const fn` — usable in const context and trivially inlinable.
#[inline]
const fn self_table_size_for(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    // `next_pow2(2n)`. `n <= u32::MAX` (construction invariant), so `2n` and the
    // rounded-up power of two fit a `usize` on every supported (64-bit) target.
    (2 * n).next_power_of_two()
}

/// How constraint compliance is supplied to the shared [`SoftBody::build`] path —
/// either one value broadcast to every edge, or a per-edge slice.
enum Compliance<'a> {
    /// One compliance broadcast to every constraint.
    Uniform(f32),
    /// A per-edge compliance slice (`len == edges.len()`, checked by the caller).
    PerEdge(&'a [f32]),
}
