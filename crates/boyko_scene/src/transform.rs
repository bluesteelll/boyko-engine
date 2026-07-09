//! The two foundational spatial components: [`Transform`] (LOCAL, decomposed,
//! designer-facing) and [`GlobalTransform`] (cached WORLD pose, packed affine).
//!
//! # Local vs world
//!
//! [`Transform`] is the pose **relative to the entity's parent** (or relative to
//! the scene root for an unparented entity). Gameplay writes it. It is stored
//! decomposed (translation / rotation / scale) so it round-trips cleanly through
//! a designer/editor and is cheap to author.
//!
//! [`GlobalTransform`] is the **world-from-local** pose, recomputed each frame by
//! [`propagate_transforms`](crate::propagation::propagate_transforms) from the
//! `Transform` chain along the `ChildOf` / `Children` hierarchy. It is the value
//! every world-space consumer (renderer, lights, camera) reads. Systems must not
//! write it by hand — the propagation pass is its sole writer.
//!
//! # 2D as a subset (D3)
//!
//! There is no separate `Transform2D`. A 2D entity uses the same [`Transform`]
//! with `translation.z == 0`, rotation about Z only, and `scale.z == 1`; the
//! `z` lane is inert through composition. 2D consumers read
//! `global.affine().translation` and project to `xy`.
//!
//! # Default validity (D5 / Bevy lesson)
//!
//! [`GlobalTransform::default`] is [`Affine3A::IDENTITY`] — a **valid** pose, not
//! NaN/garbage. An entity spawned this frame renders at the origin for at most
//! one frame, until the next [`propagate_transforms`] run composes its real
//! world pose. This pre-satisfies the "a required component's `Default` must be
//! valid before its producer runs" rule.

use boyko_macros::Component;
use boyko_math::{Affine3A, Quat, Vec3};

/// The LOCAL pose of an entity relative to its parent (or the scene root for an
/// unparented entity): translation, rotation, and per-axis scale.
///
/// Designer-facing and **decomposed** — gameplay writes the three fields
/// directly. It is read scalar by [`propagate_transforms`] (one affine compose
/// per dirty node), NOT a SIMD-load target, so the natural-`f32`-aligned 40-byte
/// layout is correct (no over-padding).
///
/// [`propagate_transforms`]: crate::propagation::propagate_transforms
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Local translation, in parent space.
    pub translation: Vec3,
    /// Local rotation (unit quaternion), in parent space.
    pub rotation: Quat,
    /// Local per-axis scale.
    pub scale: Vec3,
}

// Layout pin (house style — cf. `light.rs`, `CameraUniform`). `Vec3` is 12 B at
// natural `f32` alignment, `Quat` is 16 B: 12 + 16 + 12 = 40 B. A change here is
// a deliberate decision, not an accident.
const _: () = assert!(size_of::<Transform>() == 40);

impl Transform {
    /// The identity transform: zero translation, identity rotation, unit scale.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Builds a transform from a translation only (identity rotation, unit
    /// scale).
    #[inline]
    pub const fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Builds a transform from a rotation only (zero translation, unit scale).
    #[inline]
    pub const fn from_rotation(rotation: Quat) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Builds a transform from a per-axis scale only (zero translation, identity
    /// rotation).
    #[inline]
    pub const fn from_scale(scale: Vec3) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale,
        }
    }

    /// Folds this LOCAL pose into a packed [`Affine3A`] (`T · R · S`).
    ///
    /// This is the per-root recompose used by
    /// [`propagate_transforms`](crate::propagation::propagate_transforms): a
    /// root's `GlobalTransform` equals its `Transform` folded to an affine.
    #[inline]
    pub fn to_affine(self) -> Affine3A {
        Affine3A::from_translation_rotation_scale(self.translation, self.rotation, self.scale)
    }
}

impl Default for Transform {
    /// The default transform is [`Transform::IDENTITY`].
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The cached WORLD-from-local pose of an entity, as a packed [`Affine3A`].
///
/// Recomputed each frame by
/// [`propagate_transforms`](crate::propagation::propagate_transforms) (its sole
/// writer): a root's value is its [`Transform`] folded to an affine; a child's
/// value is `parent.global ∘ child.local`. World-space consumers read it through
/// [`affine`](Self::affine).
///
/// `#[repr(C, align(16))]`: the payload is `Affine3A` (48 B, 16-aligned). It is
/// read scalar in propagation (one affine compose per dirty node) — the SIMD/GPU
/// alignment is for the math type's own ABI, not because the propagation loop
/// SIMD-loads it.
#[repr(C, align(16))]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct GlobalTransform(pub Affine3A);

// Layout pin: `Affine3A` is a 48-B payload at 16-align (`Mat3` 36 B + `Vec3`
// 12 B = 48, already a multiple of 16). The earlier "64 B / one cache line"
// framing was wrong; the affine straddles at most two cache lines.
const _: () = assert!(size_of::<GlobalTransform>() == 48 && align_of::<GlobalTransform>() == 16);

impl GlobalTransform {
    /// The identity world pose ([`Affine3A::IDENTITY`]).
    pub const IDENTITY: Self = Self(Affine3A::IDENTITY);

    /// Returns the cached world affine by value (`Affine3A` is `Copy`).
    #[inline]
    pub fn affine(self) -> Affine3A {
        self.0
    }

    /// Returns the world-space translation (the affine's `translation` lane).
    #[inline]
    pub fn translation(self) -> Vec3 {
        self.0.translation
    }
}

impl Default for GlobalTransform {
    /// The default world pose is [`GlobalTransform::IDENTITY`] (a valid pose —
    /// see the module-level "Default validity" note).
    #[inline]
    fn default() -> Self {
        Self::IDENTITY
    }
}
