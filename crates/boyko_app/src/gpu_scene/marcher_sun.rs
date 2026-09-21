//! [`MarcherSun`] — the Deferred SDF marcher's per-frame sun, derived from the staged light table's
//! primary directional (defect R2, `docs/render/light-table-defects/R2-DESIGN.md` D1/D2).
//!
//! The marcher (`sdf_gbuffer_composite.hlsl`) casts the SDF soft shadow toward its push field
//! `light_dir` and writes it to `gMaterial.r`, which the resolve reads as the PRIMARY directional's
//! visibility — the first directional row of the light table — and, while punctual shadows are off,
//! as every point/spot light's visibility. So the push must carry that same row, every frame: the
//! runner reads it with `LightTableStaging::primary_directional_dir` and [`MarcherSun::from_primary`]
//! turns it into the two push values.

use boyko_rhi_vulkan::compute::{DEFAULT_LIGHT_DIR, LIGHTING_FLAG_AO, LIGHTING_FLAG_SHADOWS};

/// The squared length at or below which a direction counts as degenerate — `normalize3`'s own
/// zero-length threshold (`boyko_render::light`), so the host rejects exactly what the fold cannot
/// normalise.
const DEGENERATE_LEN_SQ: f32 = 1e-12;

/// The marcher push's two sun-dependent values for one frame. 16 B, built on the stack per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MarcherSun {
    /// `FineMarcherPush::lighting_flags`: `SHADOWS | AO` with a sun, `AO` alone without one.
    pub(crate) lighting_flags: u32,
    /// `FineMarcherPush::light_dir`: the primary's `dir_kind.xyz` bits verbatim (TO the light), or
    /// the unread [`DEFAULT_LIGHT_DIR`] placeholder when the SHADOWS bit is clear.
    pub(crate) light_dir: [f32; 3],
}

impl MarcherSun {
    /// The push values for this frame's primary directional (`None` = the table has none).
    ///
    /// * A usable sun ⇒ `SHADOWS | AO` and its bits UNCHANGED — the frame the marcher always
    ///   produced on a sunlit scene, now toward the resolve's own sun. The bits are not normalised or
    ///   negated: both sides normalise the same input.
    /// * No sun, or a direction that is non-finite or zero-length (`len² <= 1e-12`) ⇒ `AO` alone: no
    ///   sun means no sun shadow, and AO does not depend on a light. The guard matters on the GPU —
    ///   a NaN `L` passes the marcher's `dot(n, L) <= 0` back-face test, and `max(NaN, step)` then
    ///   yields `step`, so every pixel would march the full iteration budget. `light_dir` becomes
    ///   the placeholder, which the marcher does not read with the bit clear.
    #[inline]
    pub(crate) fn from_primary(primary: Option<[f32; 3]>) -> Self {
        match primary {
            Some(dir) if is_usable(dir) => {
                Self { lighting_flags: LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO, light_dir: dir }
            }
            _ => Self { lighting_flags: LIGHTING_FLAG_AO, light_dir: DEFAULT_LIGHT_DIR },
        }
    }
}

/// Whether `dir` can be normalised: every lane finite and the length above the degenerate floor.
#[inline]
fn is_usable(dir: [f32; 3]) -> bool {
    let len_sq = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    dir.iter().all(|c| c.is_finite()) && len_sq > DEGENERATE_LEN_SQ
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T7: a valid sun pushes `SHADOWS | AO` and its bits unchanged.
    #[test]
    fn a_valid_sun_pushes_shadows_and_ao_with_its_bits_unchanged() {
        // Deliberately not unit length: the host must not normalise.
        let dir = [-0.45_f32, 0.82, 0.36];
        let m = MarcherSun::from_primary(Some(dir));
        assert_eq!(m.lighting_flags, LIGHTING_FLAG_SHADOWS | LIGHTING_FLAG_AO);
        assert_eq!(m.light_dir.map(f32::to_bits), dir.map(f32::to_bits));
    }

    /// T8: no sun clears SHADOWS and keeps AO (M4 keeps SHADOWS; M4′ pushes `0`, which kills AO).
    #[test]
    fn no_sun_clears_shadows_and_keeps_ao() {
        let m = MarcherSun::from_primary(None);
        assert_eq!(m.lighting_flags & LIGHTING_FLAG_SHADOWS, 0, "no sun, no sun shadow");
        assert_eq!(m.lighting_flags, LIGHTING_FLAG_AO, "AO does not depend on a light");
        assert_eq!(m.light_dir, DEFAULT_LIGHT_DIR);
    }

    /// T9: a NaN, infinite or zero-length direction is treated as no sun (M5 drops the guard).
    #[test]
    fn a_degenerate_sun_is_treated_as_no_sun() {
        let none = MarcherSun::from_primary(None);
        for dir in [
            [f32::NAN, 1.0, 0.0],
            [0.0, f32::INFINITY, 0.0],
            [0.0, 0.0, 0.0],
            [1e-7, 0.0, 0.0],
        ] {
            assert_eq!(MarcherSun::from_primary(Some(dir)), none, "{dir:?} must push as no sun");
        }
        // The floor is exclusive: just above it is a usable direction.
        let tiny = [2e-6_f32, 0.0, 0.0];
        assert_eq!(MarcherSun::from_primary(Some(tiny)).lighting_flags & LIGHTING_FLAG_SHADOWS, LIGHTING_FLAG_SHADOWS);
    }
}
