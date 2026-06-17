//! Physics step resources — the preallocated, reused step buffers (plan D1/D4/
//! IM-1).
//!
//! Every `Vec` here is sized once and refilled each step (cleared, capacity
//! reused): the foundation does no per-step / per-manifold heap allocation
//! (principle 5). [`SolverScratch`] is the dense, row-indexed snapshot the
//! gather→solve→apply pipeline addresses by [`BodyIndex`](crate::manifold::BodyIndex)
//! — see [`crate::systems`].

use boyko_macros::Resource;
use boyko_utils::bit_mask::bit_set_256::BitSet256;

use crate::components::{BodyType, Collider, ColliderShape, RigidBody, RigidBodyMass};
use crate::manifold::{BodyIndex, Manifold};
use crate::math::{Mat3, Quat, Vec3};

/// Number of bits in one [`BitSet256`] chunk.
const BITS_PER_CHUNK: usize = 256;

/// Global physics tunables (plan D1; P2 W1 soft-constraint set).
///
/// `gravity`, `substeps`, `relax_iterations`, and the soft-constraint pair
/// (`contact_hertz` / `contact_damping`) are user-set; `dt` is NOT — it is
/// stamped by [`physics_gather`](crate::systems::physics_gather) from the
/// fixed clock each step (OQ-1), so a hand-set value is overwritten.
#[derive(Resource, Clone, Copy, Debug)]
pub struct PhysicsConfig {
    /// Constant acceleration applied to dynamic bodies each step (world
    /// units/s²).
    pub gravity: Vec3,
    /// The step delta in seconds, stamped each step by
    /// [`physics_gather`](crate::systems::physics_gather) from
    /// [`FixedTime::delta_secs`](boyko_ecs::ecs::core::time::FixedTime::delta_secs)
    /// (OQ-1). Not user-set — the TGS solver reads `h = dt / substeps`.
    pub dt: f32,
    /// Solver substep count (default `4`, OQ-5). The TGS solver loops this many
    /// times per step over the same contact set; the no-op solver ignores it.
    pub substeps: u32,
    /// Relaxation passes per substep (default `2`): post-solve iterations that
    /// re-solve the constraints bias-free to remove soft-bias energy.
    pub relax_iterations: u32,
    /// Soft-constraint stiffness, in hertz (default `30.0`): the natural
    /// frequency of the contact's penetration-recovery spring. Higher = stiffer
    /// (faster recovery, less squish).
    pub contact_hertz: f32,
    /// Soft-constraint damping ratio ζ (default `10.0`): the contact spring's
    /// damping. `1.0` is critically damped; the Box2D-v3 "Soft Step" default of
    /// `10.0` is heavily overdamped for stable resting contact.
    pub contact_damping: f32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            // Earth-like downward gravity by default (−Y is "down").
            gravity: Vec3::new(0.0, -9.81, 0.0),
            // Stamped by `physics_gather` from `FixedTime` every step before any
            // solver reads it; the integrate->gather->solve `.after` chain
            // (plugin.rs) guarantees gather precedes the solve, so this `0.0`
            // placeholder is never the value a solver actually sees.
            dt: 0.0,
            substeps: 4,
            relax_iterations: 2,
            contact_hertz: 30.0,
            contact_damping: 10.0,
        }
    }
}

/// Whether the pipeline's [`physics_integrate`](crate::systems::physics_integrate)
/// stage integrates, or the solver owns integration (C2).
///
/// Inserted by [`add_physics_systems`](crate::plugin::add_physics_systems) from
/// the chosen solver's
/// [`RigidSolver::owns_integration`](crate::solver::RigidSolver::owns_integration):
/// an owning TGS solver (the [`SoftStepSolver`](crate::solver::SoftStepSolver))
/// integrates DYNAMIC bodies inside its own substep loop, so the pipeline stage
/// must early-return to avoid double-integration. See the C2 contract block in
/// [`crate::systems`].
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IntegrationMode {
    /// The pipeline's [`physics_integrate`](crate::systems::physics_integrate)
    /// stage integrates every body (the foundation default, used by the no-op /
    /// non-owning solvers).
    #[default]
    Foundation,
    /// The solver owns the substep integration (a TGS solver); the pipeline
    /// stage early-returns so it does NOT also integrate.
    SolverOwned,
}

/// Candidate collision pairs emitted by broadphase (plan D4).
///
/// Each pair is `(BodyIndex, BodyIndex)` keyed by the dense scratch row index
/// (IM-1). The list is sorted deterministically by `(min, max)` (D4) so contact
/// iteration order is reproducible (float add is non-associative). The `Vec` is
/// cleared and refilled each step, capacity reused.
#[derive(Resource, Default)]
pub struct ContactPairs {
    /// Candidate pairs in deterministic `(min, max)` order.
    pub pairs: Vec<(BodyIndex, BodyIndex)>,
}

impl ContactPairs {
    /// Builds an empty pair buffer pre-sized for `capacity` pairs.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            pairs: Vec::with_capacity(capacity),
        }
    }
}

/// Dense manifold buffer produced by narrowphase, consumed by the solver
/// (plan D1/D4).
///
/// Sequential over the ordered pairs (matching [`ContactPairs`]), so the solve
/// iterates a packed array. Cleared and refilled each step, capacity reused.
#[derive(Resource, Default)]
pub struct Manifolds {
    /// Manifolds in the deterministic pair order.
    pub manifolds: Vec<Manifold>,
}

impl Manifolds {
    /// Builds an empty manifold buffer pre-sized for `capacity` manifolds.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            manifolds: Vec::with_capacity(capacity),
        }
    }
}

/// Dense, row-indexed snapshot of one body's state for the solve (plan IM-1).
///
/// `BodyState` carries the HOT integrator fields the solve mutates plus the
/// COLD mass fields the solve reads — gathered once at the seam boundary so the
/// solver works over a packed SoA-friendly array (the ideal Phase-10 constraint
/// buffer). `#[repr(C)]` + `Copy` for a flat cache-friendly layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyState {
    /// WORLD inverse inertia TENSOR `R₀ · inv_inertia_local · R₀ᵀ`, derived at
    /// gather; read by the solve and refreshed per substep as the orientation
    /// advances. Placed first so the larger-aligned fields lead the struct.
    pub inv_inertia: Mat3,
    /// Orientation-free LOCAL inverse inertia (the principal-axis diagonal
    /// tensor of the collider shape), derived at gather from the shape +
    /// inverse mass. The solver rotates it into world space each substep
    /// (`R · inv_inertia_local · Rᵀ`); kept here so that rotation needs no
    /// re-derivation. `Mat3::ZERO` for static / `inv_mass == 0` bodies.
    pub inv_inertia_local: Mat3,
    /// World position (mirrors [`RigidBody::position`]).
    pub position: Vec3,
    /// Linear velocity (mirrors [`RigidBody::linear_velocity`]).
    pub linear_velocity: Vec3,
    /// Angular velocity (mirrors [`RigidBody::angular_velocity`]).
    pub angular_velocity: Vec3,
    /// Orientation (mirrors [`RigidBody::rotation`]).
    pub rotation: Quat,
    /// Inverse mass (`0` = immovable); read by the solve.
    pub inv_mass: f32,
    /// Restitution; read by the solve.
    pub restitution: f32,
    /// Friction; read by the solve.
    pub friction: f32,
    /// The body's simulation role.
    pub body_type: BodyType,
    /// The collider shape, projected at gather so broad/narrowphase have the
    /// body's real geometry (P2 W2). The broadphase reads its bounding radius
    /// and the sphere-sphere narrowphase reads its sphere radius — neither phase
    /// may assume a fixed size. Placed last (after the scalar fields) so the
    /// tightly-packed hot fields lead the struct.
    pub shape: ColliderShape,
}

impl BodyState {
    /// Builds a snapshot row from the hot [`RigidBody`] + cold [`RigidBodyMass`]
    /// + [`Collider`] columns (the gather projection, IM-1; P2 W1).
    ///
    /// Derives the orientation-free LOCAL inverse inertia from the collider
    /// shape and inverse mass, then rotates it into the world tensor by the
    /// body's spawn orientation `R₀`:
    ///
    /// - Solid sphere of radius `r`: `I = (2/5)·m·r²`, so the local inverse
    ///   inertia is the isotropic diagonal `inv_mass · 5 / (2·r²)` on each axis.
    /// - Box of half-extents `(hx, hy, hz)` (full extents `(w, h, d) = 2·h`):
    ///   `Ixx = (1/12)·m·(h²+d²)`, `Iyy = (1/12)·m·(w²+d²)`,
    ///   `Izz = (1/12)·m·(w²+h²)`, inverted per axis.
    /// - Static / `inv_mass == 0` (or a degenerate `r ≤ 0`): [`Mat3::ZERO`]
    ///   (infinite inertia, no angular response).
    ///
    /// `inv_inertia` (the WORLD tensor) is then `R₀ · inv_inertia_local · R₀ᵀ`,
    /// auto-overriding any value authored on
    /// [`RigidBodyMass::inv_inertia`](crate::components::RigidBodyMass::inv_inertia)
    /// (which is retained for custom authoring but recomputed here).
    #[inline]
    pub fn from_columns(body: &RigidBody, mass: &RigidBodyMass, collider: &Collider) -> Self {
        let inv_inertia_local = local_inv_inertia(collider.shape, mass.inv_mass);
        // World tensor = R₀ · I⁻¹_local · R₀ᵀ (rotates the principal-axis
        // diagonal into the body's spawn orientation).
        let r0 = Mat3::from_quat(body.rotation);
        let inv_inertia = r0 * inv_inertia_local * r0.transpose();
        Self {
            inv_inertia,
            inv_inertia_local,
            position: body.position,
            linear_velocity: body.linear_velocity,
            angular_velocity: body.angular_velocity,
            rotation: body.rotation,
            inv_mass: mass.inv_mass,
            restitution: mass.restitution,
            friction: mass.friction,
            body_type: mass.body_type,
            shape: collider.shape,
        }
    }
}

/// Derives the orientation-free LOCAL inverse inertia of a collider shape from
/// its geometry and the body's inverse mass (P2 W1).
///
/// Returns [`Mat3::ZERO`] (infinite inertia) for an immovable body
/// (`inv_mass == 0`) or a degenerate shape (non-positive sphere radius), so no
/// torque produces an angular response. See [`BodyState::from_columns`] for the
/// per-shape formulae.
#[inline]
fn local_inv_inertia(shape: ColliderShape, inv_mass: f32) -> Mat3 {
    // Immovable body: infinite inertia, no angular response (single branch).
    if inv_mass == 0.0 {
        return Mat3::ZERO;
    }
    match shape {
        ColliderShape::Sphere { radius } => {
            if radius <= 0.0 {
                return Mat3::ZERO;
            }
            // Solid sphere I = (2/5)·m·r² ⇒ I⁻¹ = (5 / (2·r²)) / m
            //                              = inv_mass · 5 / (2·r²) (isotropic).
            let inv = inv_mass * 5.0 / (2.0 * radius * radius);
            Mat3::from_diagonal(Vec3::new(inv, inv, inv))
        }
        ColliderShape::Aabb { half_extents } => {
            // Full extents (w, h, d) = 2·half_extents.
            let w = 2.0 * half_extents.x;
            let h = 2.0 * half_extents.y;
            let d = 2.0 * half_extents.z;
            // Solid box principal inertia (·m): Ixx=(1/12)(h²+d²), etc. With
            // I = (1/12)·m·(..), I⁻¹ = 12·inv_mass / (..) per axis.
            let inv_axis = |sum_sq: f32| {
                if sum_sq > 0.0 {
                    12.0 * inv_mass / sum_sq
                } else {
                    0.0
                }
            };
            Mat3::from_diagonal(Vec3::new(
                inv_axis(h * h + d * d),
                inv_axis(w * w + d * d),
                inv_axis(w * w + h * h),
            ))
        }
    }
}

/// A growable per-row touched mask indexed by [`BodyIndex`] = row (plan IM-1).
///
/// Built from the `boyko_utils` [`BitSet256`] 256-bit chunk so it scales past
/// 256 rows (a `BitSet256` alone caps at 256; `BitSet<T>` caps at 128). Each
/// chunk is a fixed 256-bit word block; the `Vec<BitSet256>` grows in chunk
/// granularity and its capacity is reused across steps. The solver sets bit
/// `i` for every row it mutates; [`physics_apply`](crate::systems::physics_apply)
/// writes back only set rows.
#[derive(Default)]
pub struct TouchedMask {
    /// One 256-bit chunk per 256 rows; chunk `i >> 8` holds bit `i & 255`.
    chunks: Vec<BitSet256>,
}

impl TouchedMask {
    /// Builds an empty mask pre-sized for `rows` bodies (no later realloc in
    /// steady state).
    #[inline]
    pub fn with_capacity(rows: usize) -> Self {
        Self {
            chunks: Vec::with_capacity(rows.div_ceil(BITS_PER_CHUNK)),
        }
    }

    /// Clears every bit and resizes the mask to hold exactly `rows` bits,
    /// reusing the chunk capacity (no realloc once warmed).
    #[inline]
    pub fn reset(&mut self, rows: usize) {
        let needed = rows.div_ceil(BITS_PER_CHUNK);
        self.chunks.clear();
        self.chunks.resize(needed, BitSet256::new());
    }

    /// Marks row `index` as touched.
    #[inline]
    pub fn set(&mut self, index: usize) {
        debug_assert!(
            index < self.chunks.len() * BITS_PER_CHUNK,
            "invariant: touched index {index} out of range; call reset(rows) first"
        );
        self.chunks[index >> 8].set(index & (BITS_PER_CHUNK - 1));
    }

    /// Returns `true` if row `index` was touched.
    #[inline]
    pub fn get(&self, index: usize) -> bool {
        let chunk = index >> 8;
        if chunk >= self.chunks.len() {
            return false;
        }
        self.chunks[chunk].get(index & (BITS_PER_CHUNK - 1))
    }
}

/// The dense, row-indexed solver scratch — the gather snapshot + touched mask
/// (plan IM-1).
///
/// All buffers are indexed by [`BodyIndex`] = archetype row, assigned by the
/// gather stage in archetype-row order. `bodies` is the SoA snapshot the solver
/// mutates; `touched` flags the rows
/// [`physics_apply`](crate::systems::physics_apply) writes back. Every buffer is
/// cleared and refilled each step, capacity reused.
///
/// A row→entity map (for the gameplay [`Contact`](crate::components::Contact)
/// producer) is intentionally NOT carried here in the foundation: `Entity` is not
/// yet a `QueryData`, so the gather cannot populate it, and shipping an
/// always-empty buffer whose "parallel to `bodies`" invariant is false from day
/// one is a footgun (review M2). Phase 10 adds it back together with the `Contact`
/// producer once `Entity`-as-`QueryData` lands.
#[derive(Resource, Default)]
pub struct SolverScratch {
    /// Dense snapshot, one row per body in archetype-row order.
    pub bodies: Vec<BodyState>,
    /// Per-row touched mask, indexed by [`BodyIndex`] = row.
    pub touched: TouchedMask,
    /// Per-contact-point relative normal APPROACH velocity captured BEFORE the
    /// first substep, consumed by the TGS solver's post-loop restitution pass
    /// (P2 W2). Indexed in the solver's flattened contact-point order (manifold
    /// order × point order); rebuilt and refilled each solve, capacity reused
    /// (no per-step alloc). Left empty by the no-op / non-owning solvers.
    pub vn_initial: Vec<f32>,
}

impl SolverScratch {
    /// Builds scratch buffers pre-sized for up to `rows` bodies (no later
    /// reallocation in steady state).
    pub fn with_capacity(rows: usize) -> Self {
        Self {
            bodies: Vec::with_capacity(rows),
            touched: TouchedMask::with_capacity(rows),
            // One initial normal-velocity slot per body is a cheap first-frame
            // reserve; the TGS solver grows it to the live contact-point count
            // and reuses that capacity thereafter.
            vn_initial: Vec::with_capacity(rows),
        }
    }

    /// Clears the snapshot for a fresh gather, reusing capacity. The touched
    /// mask is reset by the gather once the row count is known; `vn_initial` is
    /// rebuilt by the solver, so it is cleared here for a fresh solve.
    #[inline]
    pub fn clear(&mut self) {
        self.bodies.clear();
        self.vn_initial.clear();
    }
}

#[cfg(test)]
mod tests {
    //! W1 acceptance gate (plan §MAJOR W1): the `from_columns` /
    //! `local_inv_inertia` inertia DERIVATION. The `math.rs` suite covers the
    //! `Mat3` ops in isolation; these tests pin the per-shape local-tensor
    //! VALUES and the world-tensor `R₀ · I⁻¹_local · R₀ᵀ` construction that the
    //! gather builds — the values the solver's effective mass depends on.

    use super::*;
    use crate::components::{BodyType, ColliderShape};

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
            body_type: if inv_mass == 0.0 {
                BodyType::Static
            } else {
                BodyType::Dynamic
            },
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

        let state = BodyState::from_columns(&body, &mass, &collider);
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
        let collider = collider_shape(ColliderShape::Aabb {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider);
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

        let state = BodyState::from_columns(&body, &mass, &collider);
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

        let state = BodyState::from_columns(&body, &mass, &collider);
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
        let collider = collider_shape(ColliderShape::Aabb {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider);
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
        let collider = collider_shape(ColliderShape::Aabb {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });

        let state = BodyState::from_columns(&body, &mass, &collider);
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
    }
}
