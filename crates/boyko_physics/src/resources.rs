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

use crate::components::{BodyType, RigidBody, RigidBodyMass};
use crate::manifold::{BodyIndex, Manifold};
use crate::math::Vec2;

/// Number of bits in one [`BitSet256`] chunk.
const BITS_PER_CHUNK: usize = 256;

/// Global, immutable physics tunables (plan D1).
///
/// `substeps` is reserved for the Phase-10 solver (default `1`); the foundation
/// runs a single solve pass. A non-breaking field add now is cheaper than a
/// later ABI change (OQ5).
#[derive(Resource, Clone, Copy, Debug)]
pub struct PhysicsConfig {
    /// Constant acceleration applied to dynamic bodies each step (world
    /// units/s²).
    pub gravity: Vec2,
    /// Solver substep count, reserved for Phase 10 (default `1`).
    pub substeps: u32,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            // Earth-like downward gravity by default.
            gravity: Vec2::new(0.0, -9.81),
            substeps: 1,
        }
    }
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
    /// World position (mirrors [`RigidBody::position`]).
    pub position: Vec2,
    /// Linear velocity (mirrors [`RigidBody::linear_velocity`]).
    pub linear_velocity: Vec2,
    /// Orientation in radians (mirrors [`RigidBody::rotation`]).
    pub rotation: f32,
    /// Angular velocity (mirrors [`RigidBody::angular_velocity`]).
    pub angular_velocity: f32,
    /// Inverse mass (`0` = immovable); read by the solve.
    pub inv_mass: f32,
    /// Inverse inertia; read by the solve.
    pub inv_inertia: f32,
    /// Restitution; read by the solve.
    pub restitution: f32,
    /// Friction; read by the solve.
    pub friction: f32,
    /// The body's simulation role.
    pub body_type: BodyType,
}

impl BodyState {
    /// Builds a snapshot row from the hot [`RigidBody`] + cold [`RigidBodyMass`]
    /// columns (the gather projection, IM-1).
    #[inline]
    pub fn from_columns(body: &RigidBody, mass: &RigidBodyMass) -> Self {
        Self {
            position: body.position,
            linear_velocity: body.linear_velocity,
            rotation: body.rotation,
            angular_velocity: body.angular_velocity,
            inv_mass: mass.inv_mass,
            inv_inertia: mass.inv_inertia,
            restitution: mass.restitution,
            friction: mass.friction,
            body_type: mass.body_type,
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
}

impl SolverScratch {
    /// Builds scratch buffers pre-sized for up to `rows` bodies (no later
    /// reallocation in steady state).
    pub fn with_capacity(rows: usize) -> Self {
        Self {
            bodies: Vec::with_capacity(rows),
            touched: TouchedMask::with_capacity(rows),
        }
    }

    /// Clears the snapshot for a fresh gather, reusing capacity. The touched
    /// mask is reset by the gather once the row count is known.
    #[inline]
    pub fn clear(&mut self) {
        self.bodies.clear();
    }
}
