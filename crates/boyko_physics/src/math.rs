//! 2D math primitives for the physics foundation (plan D1, "2D-first via a
//! `math` alias").
//!
//! [`Vec2`] is the single vector type the whole crate is built on; a future 3D
//! variant is a type swap here (the manifold's `MAX_CONTACT_POINTS` const and
//! the vector type are the only two axes that change), so no public API breaks
//! when 3D lands.

use std::ops::{Add, Mul, Sub};

/// Maximum contact points a single [`Manifold`](crate::manifold::Manifold)
/// stores (plan D1).
///
/// `2` is the Box2D-v3 verified maximum for 2D convex-convex contact (Erin
/// Catto, Contact Manifolds, GDC 2007). It is a `const` so a future 3D variant
/// flips it to `4` with the `points` array following — the only manifold-shape
/// change 3D requires.
pub const MAX_CONTACT_POINTS: usize = 2;

/// A 2D vector / point in world units.
///
/// `#[repr(C)]` so the byte layout is stable (it rides inside the POD
/// [`Manifold`](crate::manifold::Manifold) currency and the
/// [`RigidBody`](crate::components::RigidBody) component column).
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

    /// Returns `self` scaled to unit length, or [`Vec2::ZERO`] when `self` is
    /// (near) zero-length.
    ///
    /// Guards against a divide-by-zero on a degenerate vector (the
    /// coincident-body case in narrowphase) rather than producing `NaN`.
    #[inline]
    pub fn normalize(self) -> Self {
        let len_sq = self.length_squared();
        if len_sq <= f32::MIN_POSITIVE {
            return Self::ZERO;
        }
        let inv_len = len_sq.sqrt().recip();
        self * inv_len
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
