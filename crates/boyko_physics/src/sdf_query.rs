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
use boyko_sdf_math::{MAX_SDF_EDITS, SdfEdit, sdf_edit_list, sdf_edit_list_normal};

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
#[derive(Resource, Clone, Copy, Debug)]
pub struct SdfField {
    /// The ordered edit list (only `edits[..count]` are live). Capped at
    /// [`MAX_SDF_EDITS`] — the same ceiling the shader's edit-list buffer uses, so
    /// the CPU field can never describe a scene the GPU cannot render.
    edits: [SdfEdit; MAX_SDF_EDITS],
    /// Number of live edits in `edits` (`<= MAX_SDF_EDITS`).
    count: usize,
}

impl Default for SdfField {
    /// The empty field — no edits, so [`sample_sdf`] returns `+far` everywhere and
    /// no body collides.
    #[inline]
    fn default() -> Self {
        // `SdfEdit` is `Copy` but not `Default`; seed every slot with a zero-radius
        // sphere at the origin (an inert placeholder never read past `count`).
        let placeholder = SdfEdit::sphere([0.0, 0.0, 0.0], 0.0, boyko_sdf_math::sdf_op::UNION, 0.0);
        Self {
            edits: [placeholder; MAX_SDF_EDITS],
            count: 0,
        }
    }
}

impl SdfField {
    /// Builds a field from an ordered edit list, keeping the first
    /// [`MAX_SDF_EDITS`] edits (matching the shader's `min` clamp).
    ///
    /// The list is the CSG scene in fold order: the first edit seeds the field, each
    /// later one combines under its own [`op`](SdfEdit::op).
    #[inline]
    pub fn from_edits(edits: &[SdfEdit]) -> Self {
        let mut field = Self::default();
        let n = edits.len().min(MAX_SDF_EDITS);
        field.edits[..n].copy_from_slice(&edits[..n]);
        field.count = n;
        field
    }

    /// Appends one edit to the field, ignoring it once the field is full
    /// ([`MAX_SDF_EDITS`] edits) — matching the shader's edit-count clamp.
    #[inline]
    pub fn push(&mut self, edit: SdfEdit) {
        if self.count < MAX_SDF_EDITS {
            self.edits[self.count] = edit;
            self.count += 1;
        }
    }

    /// Clears the field to empty (reuses the inline storage, no realloc).
    #[inline]
    pub fn clear(&mut self) {
        self.count = 0;
    }

    /// The live edit slice (`edits[..count]`) — what the field math folds.
    #[inline]
    pub fn edits(&self) -> &[SdfEdit] {
        &self.edits[..self.count]
    }

    /// Number of live edits.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the field has no edits (samples to `+far` everywhere).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
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
