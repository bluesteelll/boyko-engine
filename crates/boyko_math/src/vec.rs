//! Vector primitives: [`Vec2`], [`Vec3`], [`Vec4`].
//!
//! [`Vec3`] is lifted **verbatim** (algorithm- and instruction-identical) from
//! the physics foundation so the migrated physics stays **bit-deterministic**:
//! [`Vec3::normalize`] is literally `len_sq.sqrt().recip()` (exact `sqrt`, NOT a
//! hardware `rsqrt`), and no operation here uses `f32::mul_add`/FMA or any
//! fast-math path. [`Vec2`]/[`Vec4`] are new and obey the same no-FMA discipline.

use std::ops::{Add, Mul, Sub};

/// A 2D vector / point in world units.
///
/// `#[repr(C)]` so the byte layout is stable (natural `f32` alignment, 8 B).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
}

impl Vec2 {
    /// The zero vector.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// The all-ones vector (the default scale).
    pub const ONE: Self = Self { x: 1.0, y: 1.0 };

    /// Constructs a vector from its components.
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Dot product `self · other`.
    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y
    }

    /// 2D cross product (the z-component of the 3D cross), `self × other`.
    #[inline]
    pub fn cross(self, other: Self) -> f32 {
        self.x * other.y - self.y * other.x
    }

    /// Squared length (`self · self`). Cheaper than [`length`](Self::length)
    /// when only a comparison is needed — avoids the `sqrt`.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length `|self|`.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// `true` when every component is finite (no `NaN`, no `±Inf`).
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Returns `self` scaled to unit length, or [`Vec2::ZERO`] when `self` is
    /// (near) zero-length.
    ///
    /// Uses exact `sqrt` then reciprocal (`len_sq.sqrt().recip()`) — NO
    /// hardware `rsqrt` — matching the bit-deterministic [`Vec3`] convention.
    #[inline]
    pub fn normalize(self) -> Self {
        let len_sq = self.length_squared();
        if len_sq <= f32::MIN_POSITIVE {
            return Self::ZERO;
        }
        let inv_len = len_sq.sqrt().recip();
        self * inv_len
    }

    /// Component-wise (Hadamard) product `(self.x·rhs.x, self.y·rhs.y)`.
    #[inline]
    pub fn componentwise_mul(self, rhs: Self) -> Self {
        Self {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
        }
    }

    /// Per-component absolute value `(|x|, |y|)`.
    #[inline]
    pub fn abs(self) -> Self {
        Self {
            x: self.x.abs(),
            y: self.y.abs(),
        }
    }
}

impl Add for Vec2 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Sub for Vec2 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

/// A 3D vector / point in world units.
///
/// `#[repr(C)]` so the byte layout is stable (it rides inside POD component
/// columns and contact-manifold currency). **Lifted verbatim** from the physics
/// foundation — every method is algorithm- and instruction-identical, so the
/// migrated physics is bit-for-bit unchanged.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

impl Vec3 {
    /// The zero vector.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    /// The all-ones vector (the default scale).
    pub const ONE: Self = Self {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    };

    /// Constructs a vector from its components.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Dot product `self · other`.
    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Cross product `self × other` (right-handed).
    ///
    /// Needed for the 3D angular terms (`r × impulse`, `ω × r`) the quaternion
    /// integration and the solver build on.
    #[inline]
    pub fn cross(self, other: Self) -> Self {
        Self {
            x: self.y * other.z - self.z * other.y,
            y: self.z * other.x - self.x * other.z,
            z: self.x * other.y - self.y * other.x,
        }
    }

    /// Squared length (`self · self`). Cheaper than [`length`](Self::length)
    /// when only a comparison is needed — avoids the `sqrt`.
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length `|self|`.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// `true` when every component is finite (no `NaN`, no `±Inf`).
    ///
    /// Used by the SDF narrowphase to reject a degenerate/non-finite field
    /// gradient (defense-in-depth alongside the zero-length seam-skip) before it
    /// can emit a `NaN`-normal contact that would poison the solver.
    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Returns `self` scaled to unit length, or [`Vec3::ZERO`] when `self` is
    /// (near) zero-length.
    ///
    /// Guards against a divide-by-zero on a degenerate vector (the
    /// coincident-body case in narrowphase) rather than producing `NaN`.
    ///
    /// Bit-determinism: literally `len_sq.sqrt().recip()` — exact `sqrt` then
    /// reciprocal, NOT a hardware `rsqrt`. MUST NOT be "optimized".
    #[inline]
    pub fn normalize(self) -> Self {
        let len_sq = self.length_squared();
        if len_sq <= f32::MIN_POSITIVE {
            return Self::ZERO;
        }
        let inv_len = len_sq.sqrt().recip();
        self * inv_len
    }

    /// Reads the component at axis `axis` (`0 = x`, `1 = y`, `2 = z`).
    ///
    /// Used by the box narrowphase to address the principal axis a SAT face axis
    /// selects without a per-axis `match`.
    ///
    /// # Panics (debug only)
    ///
    /// `debug_assert!`s `axis < 3`; an out-of-range axis is a bug.
    #[inline]
    pub fn axis(self, axis: usize) -> f32 {
        debug_assert!(axis < 3, "invariant: Vec3 axis index must be 0..3");
        match axis {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    /// Component-wise (Hadamard) product `(self.x·rhs.x, …)`.
    ///
    /// Used to scale a box's local half-extents by a per-axis sign vector when
    /// building OBB corners.
    #[inline]
    pub fn componentwise_mul(self, rhs: Self) -> Self {
        Self {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
            z: self.z * rhs.z,
        }
    }

    /// Per-component absolute value `(|x|, |y|, |z|)`.
    #[inline]
    pub fn abs(self) -> Self {
        Self {
            x: self.x.abs(),
            y: self.y.abs(),
            z: self.z.abs(),
        }
    }

    /// Per-component clamp into `[-limit.x, limit.x] × …` (a symmetric box).
    ///
    /// The sphere-box closest-point uses this to clamp a sphere center expressed
    /// in a box's local frame to the box's half-extents.
    #[inline]
    pub fn clamp_symmetric(self, limit: Self) -> Self {
        Self {
            x: self.x.clamp(-limit.x, limit.x),
            y: self.y.clamp(-limit.y, limit.y),
            z: self.z.clamp(-limit.z, limit.z),
        }
    }

    /// The 2D projection `(x, y)` of this vector (drops `z`).
    #[inline]
    pub const fn xy(self) -> Vec2 {
        Vec2 {
            x: self.x,
            y: self.y,
        }
    }
}

impl Add for Vec3 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl Sub for Vec3 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

/// A 4D vector — also the GPU/std140 `vec4` lane and a single SSE `xmm`.
///
/// `#[repr(C, align(16))]`: 16 B, 16-aligned so it occupies exactly one SIMD
/// lane and packs directly into a std140/WGSL `vec4` slot.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec4 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
    /// W component.
    pub w: f32,
}

impl Vec4 {
    /// The zero vector.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };

    /// Constructs a vector from its components.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Builds a `Vec4` from a [`Vec3`] and a `w` component.
    #[inline]
    pub const fn from_vec3(v: Vec3, w: f32) -> Self {
        Self {
            x: v.x,
            y: v.y,
            z: v.z,
            w,
        }
    }

    /// The 3D projection `(x, y, z)` of this vector (drops `w`).
    #[inline]
    pub const fn xyz(self) -> Vec3 {
        Vec3 {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }

    /// Dot product `self · other`.
    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z + self.w * other.w
    }

    /// Squared length (`self · self`).
    #[inline]
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length `|self|`.
    #[inline]
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }
}

impl Add for Vec4 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
            w: self.w + rhs.w,
        }
    }
}

impl Sub for Vec4 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
            w: self.w - rhs.w,
        }
    }
}

impl Mul<f32> for Vec4 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
            w: self.w * rhs,
        }
    }
}
