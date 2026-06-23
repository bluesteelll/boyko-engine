//! CPU SDF-collision queries — the [`SdfField`] resource and the [`sample_sdf`]
//! wrapper (P2 W5).
//!
//! A rigid body collides against the analytic signed-distance field by SAMPLING
//! it: [`sample_sdf`] returns the signed distance at a world point plus the unit
//! field gradient (the contact normal). Both come from the
//! [`boyko_sdf_math`](boyko_sdf_math) leaf — the SAME analytic edit-list the GPU
//! renders — so the CPU collision evaluator and the GPU golden fold bit-identical
//! arithmetic with ZERO readback and ZERO graphics deps (the leaf is `no_std` +
//! zero-dep; depending on it does NOT pull Vulkan in).
//!
//! The scalar leaf eval ([`boyko_sdf_math::sdf_edit_list`] /
//! [`boyko_sdf_math::sdf_edit_list_normal`]) STAYS the single source of truth: this
//! module only stores the CPU-authoritative edit list and converts
//! [`Vec3`] ↔ `[f32; 3]` at the boundary. It does NOT reimplement the field math.

use boyko_macros::Resource;
use boyko_sdf_math::{
    MAX_SDF_EDITS, SdfEdit, SdfEditField, sdf_edit_list, sdf_edit_list_normal,
};

use crate::math::Vec3;

/// The CPU-authoritative SDF scene the physics narrowphase collides bodies against
/// (P2 W5 — §3.4 Option A).
///
/// Holds the SAME ordered [`SdfEdit`] list the GPU renders, in a fixed-capacity
/// inline array (no per-frame / per-query allocation, principle 5). The
/// [`physics_narrowphase_sdf`](crate::systems::physics_narrowphase_sdf) stage reads
/// it once per step; a body-only scene leaves it empty (the SDF stage is opt-in,
/// inserted only by [`add_physics_sdf`](crate::plugin::add_physics_sdf)).
///
/// `Default` is the EMPTY field (`count == 0`): an empty edit list evaluates to
/// `+SDF_FAR` everywhere, so no body ever collides against an empty field.
///
/// # Storage (brick campaign W0)
///
/// `SdfField` now embeds the ONE edit authority — [`SdfEditField`] — instead of a
/// standalone `[SdfEdit; 16]` + `count` pair (principle 0: a single edit source
/// shared with the GPU encoder and the brick reference). The hot `edits` array is
/// FIRST inside [`SdfEditField`] and byte-identical to the legacy array, so the
/// scalar [`sample_sdf`] and the AVX2 kernel still stream the EXACT same
/// `[SdfEdit; 16]` layout via [`edits()`](Self::edits) — the generated code is
/// asm-identical (the W4 0%-regression contract).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SdfField(SdfEditField);

impl SdfField {
    /// Builds a field from an ordered edit list, keeping the first
    /// [`MAX_SDF_EDITS`] edits (matching the shader's `min` clamp).
    ///
    /// The list is the CSG scene in fold order: the first edit seeds the field, each
    /// later one combines under its own [`op`](SdfEdit::op). Each edit's conservative
    /// AABB is recomputed (the brick classifier reads them); `gen` is bumped once.
    #[inline]
    pub fn from_edits(edits: &[SdfEdit]) -> Self {
        let mut field = Self::default();
        for &edit in edits.iter().take(MAX_SDF_EDITS) {
            field.0.push(edit);
        }
        field.0.bump_gen();
        field
    }

    /// Appends one edit to the field, ignoring it once the field is full
    /// ([`MAX_SDF_EDITS`] edits) — matching the shader's edit-count clamp. The
    /// edit's conservative AABB is recomputed; `gen` is bumped so a brick cache
    /// keyed on it re-bakes.
    #[inline]
    pub fn push(&mut self, edit: SdfEdit) {
        if self.0.push(edit) {
            self.0.bump_gen();
        }
    }

    /// Clears the field to empty (reuses the inline storage, no realloc). Bumps
    /// `gen` so a brick cache keyed on it invalidates.
    #[inline]
    pub fn clear(&mut self) {
        self.0.count = 0;
        self.0.bump_gen();
    }

    /// The live edit slice (`edits[..count]`) — what the field math folds. This is
    /// the hot array the AVX2 kernel streams; byte-identical to the legacy layout.
    #[inline]
    pub fn edits(&self) -> &[SdfEdit] {
        self.0.edits()
    }

    /// The embedded edit authority — the single source the GPU encoder and the
    /// brick reference ([`boyko_sdf_math::brick`]) read.
    #[inline]
    pub fn authority(&self) -> &SdfEditField {
        &self.0
    }

    /// Number of live edits.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.count as usize
    }

    /// Whether the field has no edits (samples to `+far` everywhere).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.count == 0
    }
}

/// Samples the SDF field at world point `p`, returning `(signed_distance, normal)`
/// (P2 W5).
///
/// `signed_distance` is the field value at `p` (negative inside a solid); `normal`
/// is the UNIT field gradient — the outward surface direction, which the SDF
/// narrowphase uses as the contact normal (pointing from the SDF surface toward the
/// body). Both delegate to the [`boyko_sdf_math`] leaf (the single source of truth),
/// converting [`Vec3`] ↔ `[f32; 3]` at the boundary.
///
/// On an empty field the distance is `+far` (no edits) and the gradient is the
/// leaf's zero-length-guarded normalize result; the narrowphase only emits a
/// contact when the distance indicates penetration, so an empty field produces no
/// contacts regardless of the returned normal.
#[inline]
pub fn sample_sdf(field: &SdfField, p: Vec3) -> (f32, Vec3) {
    let edits = field.edits();
    let q = [p.x, p.y, p.z];
    let distance = sdf_edit_list(edits, q);
    let grad = sdf_edit_list_normal(edits, q);
    (distance, Vec3::new(grad[0], grad[1], grad[2]))
}
