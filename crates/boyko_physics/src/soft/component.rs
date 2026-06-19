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
    /// A slice length disagrees with another it must match (e.g. `inv_masses.len()
    /// != positions.len()`, or `compliance_per_edge.len() != edges.len()`).
    LengthMismatch,
    /// `positions.len() > u32::MAX` — the endpoint indices are `u32`.
    TooManyParticles,
    /// An edge connects a particle to itself (`a == b`) — a degenerate constraint.
    SelfEdge,
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
    /// - [`SoftBodyError::NonFinite`] if any position component, inverse mass, or
    ///   supplied rest length is not finite, or `radius < 0` / non-finite.
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
    /// `compliance_per_edge.len() != edges.len()`.
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

        // All checks passed — build the SoA columns sized exactly once.
        let m = edges.len();
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
            particle_radius: radius,
        })
    }
}

/// How constraint compliance is supplied to the shared [`SoftBody::build`] path —
/// either one value broadcast to every edge, or a per-edge slice.
enum Compliance<'a> {
    /// One compliance broadcast to every constraint.
    Uniform(f32),
    /// A per-edge compliance slice (`len == edges.len()`, checked by the caller).
    PerEdge(&'a [f32]),
}
