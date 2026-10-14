//! The material route markers (RD2 / TRANSPARENCY R0a).
//!
//! A material's rail — opaque, alpha-tested or sorted translucent — is a STRUCTURAL fact of
//! the entity that draws it: [`Translucent`] or [`AlphaMasked`] is present on the entity, or
//! neither is. The engine's glb spawn path
//! ([`spawn_glb_scene`](crate::loaders::glb_spawn::spawn_glb_scene)) derives it from
//! [`Material::route`](crate::material::Material::route) when it spawns the entity — the one
//! authority (09-10 D3).
//!
//! Both markers are ordinary components, never `EnableTag`s: a flag filter's `With<F>` matches
//! a disabled flag too, which is the wrong polarity for a rail split.
//!
//! Until the translucent rail lands (R1) nothing reads either marker: a `Translucent` entity
//! renders on the opaque rail exactly as it did before RD2.

use boyko_macros::Component;

/// The entity's material routes to the sorted translucent rail (`alphaMode = BLEND` or
/// `transmission > 0`).
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Translucent;

/// The entity's material routes to the alpha-tested rail (`alphaMode = MASK`).
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct AlphaMasked;

const _: () = assert!(size_of::<Translucent>() == 0);
const _: () = assert!(size_of::<AlphaMasked>() == 0);
