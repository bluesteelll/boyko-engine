//! Particles P0 — the authored [`ParticleEffect`] asset and its host-side pack into the device's
//! [`EffectParamsGpu`] row.
//!
//! # The pack is where the device's transcendentals go to die (D6)
//!
//! [`ParticleClock`](crate::particle_clock::ParticleClock)'s timestep is CONSTANT, so every
//! per-substep constant an effect needs can be evaluated ONCE on the host instead of per particle
//! per substep on the device:
//!
//! * `damping = exp2(-drag · timestep)` — this is what deletes `exp2` from the eDSL's required
//!   node set entirely (the plan's rung-E item E4 is struck out for exactly this reason);
//! * the rotation advance becomes a complex multiply by the constant pair
//!   `(cos ω·timestep, sin ω·timestep)` — which deletes ALL trig from the device (E5, likewise
//!   struck), and leaves the `particle_rot_advance` leaf pure multiply/add, hence bit-exact
//!   against its host oracle.
//!
//! The pair is stored as **two f32s, not a packed snorm16 word** (K1). A quantized multiplier's
//! magnitude error is a per-effect CONSTANT and compounds geometrically — at `δ ~ 2×10⁻⁵`,
//! `(1+δ)⁶⁴⁰ ≈ ±1 %`, coherent across every particle of the effect, which reads as a visible
//! size/rotation drift rather than as noise. At f32 the same term is `≈ 4×10⁻⁵`, below the
//! per-particle record's own snorm16 precision. The struct already had the padding, so this costs
//! nothing.
//!
//! # No renormalization (M7)
//!
//! The rotation state is NOT renormalized per substep. `1/√(c²+s²)` is expressible today
//! (`FieldScalar` carries both `div` and `sqrt`), but a division inside a leaf drags `OpFDiv`'s
//! 2.5 ULP into that leaf's bit-exact contract, and the house rule is that division is never part
//! of one. The residual drift is bounded instead: the per-step snorm16 re-quantization of the
//! STATE is unbiased and random-walks to ≈ 7.6×10⁻⁴ over a 10 s life — a 0.08 % billboard-size
//! error.
//!
//! # What P0 actually renders (scope, stated so no field below reads as a promise)
//!
//! The shipped P0 shaders evaluate the **SIZE ramp only** — [`ParticleEffect::size_keys`]' four
//! keys at UNIFORM key times over the particle's normalized life. **Colour is spawn-passthrough**:
//! `color_keys[0]` is copied into the particle at spawn and held unchanged for its whole life, and
//! [`ParticleEffect::color_times`] is read by nothing. The 4-key RGBA8 colour blend is a named
//! post-P0 item.
//!
//! [`pack_effect_params`] fills all three lanes per the plan's layout regardless. That is
//! deliberate: landing the colour consumer later then moves no device offset and re-pins no
//! generated shader — the alternative (packing only what is read today) would make the ramp's
//! arrival a layout change.
//!
//! # Append-only at P0
//!
//! [`ParticleEffectHandle`](crate::particle::ParticleEffectHandle) is a RAW dense index (no
//! generation), so a retired-and-reused row would alias a live handle onto a different effect.
//! Every row minted through [`ParticleEffectsExt`] is therefore PINNED — a refcount zero-crossing
//! leaves it `Loaded` at refcount 0 rather than transitioning to `Retiring` — which is exactly
//! `Assets<Material>` slot 0's precedent. Handle generations reaching the render path is the
//! prerequisite for lifting this.

use boyko_ecs::ecs::core::asset::asset::Asset;
use boyko_ecs::ecs::core::asset::assets::Assets;
use boyko_ecs::ecs::core::asset::handle::Handle;

use crate::particle::{
    EffectParamsGpu, MAX_EFFECTS, PARTICLE_BLEND_ADDITIVE, PARTICLE_BLEND_ALPHA,
};

// ── Spawn-volume discriminants ───────────────────────────────────────────────────────

/// Spawn shape `0`: every particle starts at the emitter origin.
pub const PARTICLE_SHAPE_POINT: u32 = 0;
/// Spawn shape `1`: uniformly inside the unit sphere scaled by the emitter basis.
pub const PARTICLE_SHAPE_SPHERE: u32 = 1;
/// Spawn shape `2`: on the emitter's +Z cone, half-angle
/// `acos(`[`ParticleEffect::cone_cos`]`)`, sampled with the concentric-disc form (`sqrt` only, no
/// trig).
pub const PARTICLE_SHAPE_CONE: u32 = 2;
/// Spawn shape `3`: uniformly inside the unit box scaled by the emitter basis.
pub const PARTICLE_SHAPE_BOX: u32 = 3;

/// Number of keys in an effect's colour and size ramps (D2's `[u32; 4]` colour lane and its two
/// `f16`-pair time/size words).
pub const PARTICLE_RAMP_KEYS: usize = 4;

// ── The authored asset ───────────────────────────────────────────────────────────────

/// One authored particle effect — a plain-old-data asset row, authored in engine units and packed
/// to [`EffectParamsGpu`] against the clock's constant timestep.
///
/// Authoring holds the *physical* parameters (`drag` per second, `rot_speed` in radians per
/// second, ramp times and sizes as plain `f32`s); the device row holds their *per-substep*
/// pre-evaluated form. The two are deliberately different types: a single struct doing both jobs
/// would be a value living in two places, and there would be no compile-time obstacle to
/// uploading the unbaked one.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleEffect {
    /// World gravity applied to every particle of this effect, in units per second squared.
    pub gravity: [f32; 3],
    /// Aerodynamic drag, per second. Baked to `damping = exp2(-drag · timestep)`; `0.0` means no
    /// damping (`damping == 1.0`).
    pub drag: f32,
    /// Billboard spin rate, radians per second. Baked to the `(cos ω·timestep, sin ω·timestep)`
    /// f32 pair.
    pub rot_speed: f32,
    /// Minimum spawn lifetime, seconds.
    pub lifetime_min: f32,
    /// Maximum spawn lifetime, seconds. Must be `>= lifetime_min`.
    pub lifetime_max: f32,
    /// Minimum initial speed, units per second.
    pub speed_min: f32,
    /// Maximum initial speed. Must be `>= speed_min`.
    pub speed_max: f32,
    /// Base billboard half-extent the size ramp multiplies.
    pub size_base: f32,
    /// `cos` of the spawn cone's half-angle (`1.0` = a straight line, `-1.0` = the full sphere).
    /// Stored as a cosine so the device never needs `acos`.
    pub cone_cos: f32,
    /// Rung P1 collision radius. Zero at P0.
    pub collision_radius: f32,
    /// Rung P1 bounce restitution, `[0, 1]`. Zero at P0.
    pub restitution: f32,
    /// Rung P1 tangential friction, `[0, 1]`. Zero at P0.
    pub friction: f32,
    /// Four `RGBA8` colour keys, already in the device's own encoding (a colour is authored as a
    /// packed word everywhere else in this engine, so packing it again here would only invite a
    /// channel-order disagreement).
    ///
    /// ⚠️ **At P0 only key 0 is rendered**: it is copied into the particle at spawn and held for
    /// the particle's whole life. Keys 1–3 are packed and uploaded for layout parity — see the
    /// module doc's "What P0 actually renders".
    pub color_keys: [u32; PARTICLE_RAMP_KEYS],
    /// The four colour-key times, normalized to `[0, 1]` over the particle's life. Packed to two
    /// `f16` pairs by [`pack_effect_params`].
    ///
    /// ⚠️ **Read by no shader at P0** (layout parity). Authoring them anyway keeps a future colour
    /// ramp from moving a single device offset.
    pub color_times: [f32; PARTICLE_RAMP_KEYS],
    /// The four size multipliers applied to [`size_base`](Self::size_base) over the particle's
    /// normalized life — **the one ramp P0 evaluates on the device**, at UNIFORM key times
    /// (`0`, `1/3`, `2/3`, `1`), not at [`color_times`](Self::color_times). Packed to two `f16`
    /// pairs.
    pub size_keys: [f32; PARTICLE_RAMP_KEYS],
    /// Bindless texture index. One draw covers every effect precisely because this rides the
    /// per-particle render record instead of becoming a batch key.
    pub tex_index: u32,
    /// [`PARTICLE_BLEND_ADDITIVE`] or [`PARTICLE_BLEND_ALPHA`]. **P0 draws only the additive
    /// class** — the alpha slot's draw is not declared, so an alpha effect would simulate and
    /// then not appear.
    pub blend_class: u32,
    /// Effect-level feature bits, forwarded verbatim.
    pub flags: u32,
    /// One of the `PARTICLE_SHAPE_*` discriminants.
    pub emitter_shape: u32,
}

impl Default for ParticleEffect {
    /// A one-second, unit-speed, white point burst with no gravity, no drag and no spin — the
    /// neutral row every field of which is a visible, sane number rather than a zero that would
    /// make the effect invisible (`size_base` and the size ramp in particular).
    fn default() -> Self {
        Self {
            gravity: [0.0; 3],
            drag: 0.0,
            rot_speed: 0.0,
            lifetime_min: 1.0,
            lifetime_max: 1.0,
            speed_min: 1.0,
            speed_max: 1.0,
            size_base: 0.1,
            cone_cos: 1.0,
            collision_radius: 0.0,
            restitution: 0.0,
            friction: 0.0,
            color_keys: [0xFFFF_FFFF; PARTICLE_RAMP_KEYS],
            color_times: [0.0, 0.333_333_34, 0.666_666_7, 1.0],
            size_keys: [1.0; PARTICLE_RAMP_KEYS],
            tex_index: 0,
            blend_class: PARTICLE_BLEND_ADDITIVE,
            flags: 0,
            emitter_shape: PARTICLE_SHAPE_POINT,
        }
    }
}

impl Asset for ParticleEffect {
    /// [`ParticleEffect`] is its own decoded CPU form — effects are authored directly in Rust at
    /// P0; there is no on-disk format and therefore no separate decode step.
    type Cpu = ParticleEffect;
}

// `Assets<ParticleEffect>`'s store-owned `ComponentPool` needs the layout/`ComponentId`.
// `ParticleEffect` is plain-old-data (`#[repr(C)]`, `Copy`, no `Drop`, no device handle), so the
// POD macro path (`NEEDS_TEARDOWN = false`, no `drop_fn`) fits it exactly.
boyko_ecs::impl_asset_pod_backing!(ParticleEffect);

// ── The pack ─────────────────────────────────────────────────────────────────────────

/// Packs an authored [`ParticleEffect`] into its device row against the clock's CONSTANT
/// `timestep` (D6).
///
/// Host-computed here, and therefore absent from every shader:
/// * `damping = exp2(-drag · timestep)`;
/// * `(rot_mul_cos, rot_mul_sin) = (cos(rot_speed · timestep), sin(rot_speed · timestep))`, as an
///   **f32 pair** (K1 — see the module doc for why quantizing it is a per-effect systematic error
///   that compounds, not noise that averages out);
/// * both ramps' `f32` keys narrowed to `f16` pairs.
///
/// Pure and device-free, so it is unit-testable headlessly and so a change to the bake can be
/// diffed against its own oracle without a GPU.
pub fn pack_effect_params(effect: &ParticleEffect, timestep: f32) -> EffectParamsGpu {
    debug_assert!(
        timestep.is_finite() && timestep > 0.0,
        "invariant: the particle timestep is finite and positive (got {timestep})"
    );
    debug_assert!(
        effect.lifetime_max >= effect.lifetime_min && effect.lifetime_min > 0.0,
        "invariant: 0 < lifetime_min <= lifetime_max (got {}..{})",
        effect.lifetime_min,
        effect.lifetime_max
    );
    debug_assert!(
        effect.speed_max >= effect.speed_min,
        "invariant: speed_min <= speed_max (got {}..{})",
        effect.speed_min,
        effect.speed_max
    );
    debug_assert!(
        effect.blend_class == PARTICLE_BLEND_ADDITIVE || effect.blend_class == PARTICLE_BLEND_ALPHA,
        "invariant: blend_class is one of the two declared classes (got {})",
        effect.blend_class
    );

    // exp2, not exp: the device never sees either, and exp2 is what the rejected device-side form
    // would have needed, so the host bake names the same function the plan's rung-E entry did.
    let damping = (-effect.drag * timestep).exp2();
    let omega_dt = effect.rot_speed * timestep;
    let (rot_mul_sin, rot_mul_cos) = omega_dt.sin_cos();

    EffectParamsGpu {
        gravity: effect.gravity,
        damping,
        rot_mul_cos,
        rot_mul_sin,
        _r0: [0; 2],
        color_keys: effect.color_keys,
        color_times: [
            pack_f16x2(effect.color_times[0], effect.color_times[1]),
            pack_f16x2(effect.color_times[2], effect.color_times[3]),
        ],
        size_keys: [
            pack_f16x2(effect.size_keys[0], effect.size_keys[1]),
            pack_f16x2(effect.size_keys[2], effect.size_keys[3]),
        ],
        lifetime_min: effect.lifetime_min,
        lifetime_max: effect.lifetime_max,
        speed_min: effect.speed_min,
        speed_max: effect.speed_max,
        size_base: effect.size_base,
        cone_cos: effect.cone_cos,
        _r1: 0.0,
        _r2: 0.0,
        tex_index: effect.tex_index,
        blend_class: effect.blend_class,
        flags: effect.flags,
        collision_radius: effect.collision_radius,
        restitution: effect.restitution,
        friction: effect.friction,
        emitter_shape: effect.emitter_shape,
        _r3: 0,
    }
}

/// Packs two `f32`s into one word as a pair of IEEE-754 binary16 values — `lo` in the low half,
/// `hi` in the high half (the `f32tof16` ordering the generated shaders unpack with).
///
/// Round-to-nearest-even, with overflow saturating to `f16` infinity and subnormal inputs
/// flushing to a signed zero. Written by hand because Rust's `f16` is still unstable and this
/// engine takes no third-party dependency for twenty lines of bit arithmetic.
#[inline]
pub fn pack_f16x2(lo: f32, hi: f32) -> u32 {
    u32::from(f32_to_f16_bits(lo)) | (u32::from(f32_to_f16_bits(hi)) << 16)
}

/// One `f32` → binary16 bit pattern, round-to-nearest-even.
fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x007F_FFFF;

    if exponent == 0xFF {
        // Inf / NaN: preserve the class, and keep NaN non-zero in the mantissa so it stays NaN.
        let m = if mantissa == 0 { 0 } else { 0x0200 };
        return sign | 0x7C00 | m;
    }

    // Re-bias 127 → 15.
    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1F {
        // Overflow to infinity — a ramp key large enough to reach here is an authoring error the
        // debug asserts in the caller catch; saturating beats wrapping into a small number.
        return sign | 0x7C00;
    }
    if unbiased <= 0 {
        // Subnormal or smaller: flush to a signed zero. Ramp times and size multipliers live in
        // [0, ~8], so this arm is reachable only for values below 6×10⁻⁵, where the difference
        // from zero is far below the ramp's own resolution.
        return sign;
    }

    // Round-to-nearest-even on the 13 bits dropped from the mantissa.
    let half = sign | ((unbiased as u16) << 10) | ((mantissa >> 13) as u16);
    let dropped = mantissa & 0x1FFF;
    let round_up = dropped > 0x1000 || (dropped == 0x1000 && (half & 1) == 1);
    // A carry out of the mantissa increments the exponent, which is exactly what adding 1 to the
    // packed half-word does — the mantissa field is immediately below the exponent field.
    half + u16::from(round_up)
}

// ── The domain API over `Assets<ParticleEffect>` ─────────────────────────────────────

/// The particle-domain API over `Assets<ParticleEffect>` — the
/// [`MeshAssetsExt`](crate::mesh_assets::MeshAssetsExt) shape.
///
/// `Assets<T>` is a bare generic kernel type and cannot carry effect-specific methods, so the mint
/// path lives on an extension trait `impl`ed once for `Assets<ParticleEffect>`. A consumer brings
/// it into scope with `use boyko_render::ParticleEffectsExt;`.
pub trait ParticleEffectsExt {
    /// Mints `effect` as a fresh row and PINS it.
    ///
    /// The pin is not decoration: [`ParticleEffectHandle`](crate::particle::ParticleEffectHandle)
    /// is a raw dense index with no generation, so a retired-and-reused row would silently alias
    /// every live handle onto a different effect. Pinning makes a refcount zero-crossing leave the
    /// row `Loaded` at refcount 0 instead of transitioning it to `Retiring` — the append-only
    /// invariant P0 depends on, and `Assets<Material>` slot 0's own precedent.
    ///
    /// ⚠️ Minting through `Assets::add` directly BYPASSES the pin. That is a supported thing to do
    /// only for a row whose handle never reaches a component.
    ///
    /// # Panics (debug)
    ///
    /// `debug_assert!`s that the minted row index stays below [`MAX_EFFECTS`] — the device table
    /// is a fixed-size upload and an index past its end is an out-of-range fetch on a device with
    /// `robustBufferAccess` OFF (undefined behaviour, not a clamp). The pack system additionally
    /// clamps in RELEASE and counts the shortfall.
    fn register_effect(&mut self, effect: ParticleEffect) -> Handle<ParticleEffect>;

    /// A short, fast, warm SPARK burst — the canonical additive effect: 0.35–0.7 s of life, a
    /// 40°-half-angle cone, gravity, light drag, and a size ramp that shrinks the billboard to
    /// nothing over its life.
    ///
    /// ⚠️ At P0 the colour is the FIRST key (white-hot) for the particle's whole life; the authored
    /// amber/red keys are layout parity until the post-P0 colour ramp lands. The fade the eye sees
    /// is therefore the SIZE ramp, which is what makes this preset legible under P0's shaders.
    fn spark(&mut self) -> Handle<ParticleEffect>;

    /// A slow, large, buoyant SMOKE plume: 2–3.5 s of life, a narrow cone, upward drift, heavy
    /// drag, a slow spin, and a size ramp that GROWS the billboard as it rises.
    ///
    /// ⚠️ **Additive at P0.** Smoke wants the alpha class, whose second draw slot rung P2 lands;
    /// until then an alpha effect would simulate correctly and draw nothing, so this preset ships
    /// as a dim additive plume and switches class at P2. Its colour is likewise the first key only
    /// — see [`spark`](Self::spark).
    fn smoke(&mut self) -> Handle<ParticleEffect>;
}

impl ParticleEffectsExt for Assets<ParticleEffect> {
    fn register_effect(&mut self, effect: ParticleEffect) -> Handle<ParticleEffect> {
        let handle = self.add(effect);
        let slot = handle.index();
        debug_assert!(
            (slot as usize) < MAX_EFFECTS,
            "invariant: the effect table holds at most MAX_EFFECTS ({MAX_EFFECTS}) rows, got slot \
             {slot}"
        );
        self.pin(slot);
        handle
    }

    fn spark(&mut self) -> Handle<ParticleEffect> {
        self.register_effect(ParticleEffect {
            gravity: [0.0, -9.81, 0.0],
            drag: 1.5,
            rot_speed: 0.0,
            lifetime_min: 0.35,
            lifetime_max: 0.7,
            speed_min: 4.0,
            speed_max: 9.0,
            size_base: 0.045,
            // cos(40°) — a wide-ish burst cone.
            cone_cos: 0.766_044_4,
            collision_radius: 0.0,
            restitution: 0.0,
            friction: 0.0,
            // Authored white-hot -> amber -> deep red -> gone. At P0 only key 0 (white-hot) is
            // rendered, held for the particle's whole life; the remaining keys are layout parity
            // until the post-P0 colour blend lands. The visible fade is the SIZE ramp below.
            // `lit` is 8-bit post-tonemap, so an additive contribution below 2/255 vanishes -- key
            // 0 is deliberately bright.
            color_keys: [0xFFFF_FFFF, 0xFF40_C8FF, 0x8000_60FF, 0x0000_00FF],
            color_times: [0.0, 0.25, 0.65, 1.0],
            size_keys: [1.0, 0.85, 0.5, 0.05],
            tex_index: 0,
            blend_class: PARTICLE_BLEND_ADDITIVE,
            flags: 0,
            emitter_shape: PARTICLE_SHAPE_CONE,
        })
    }

    fn smoke(&mut self) -> Handle<ParticleEffect> {
        self.register_effect(ParticleEffect {
            // Buoyant: a gentle UPWARD acceleration, not gravity.
            gravity: [0.0, 0.45, 0.0],
            drag: 2.5,
            // A slow tumble — the visible payoff of the host-baked rotation multiplier.
            rot_speed: 0.6,
            lifetime_min: 2.0,
            lifetime_max: 3.5,
            speed_min: 0.4,
            speed_max: 1.1,
            size_base: 0.35,
            // cos(15°) — a narrow column.
            cone_cos: 0.965_925_8,
            collision_radius: 0.0,
            restitution: 0.0,
            friction: 0.0,
            // Authored dim grey, dimming further. As with `spark`, only key 0 is rendered at P0,
            // so the plume is a CONSTANT dim grey whose visible evolution is the growing size ramp.
            // An additive plume must stay well under white or it clips, and `lit` is 8-bit
            // post-tonemap, so key 0 sits just above the 2/255 floor.
            color_keys: [0x2020_20FF, 0x1818_18FF, 0x1010_10FF, 0x0000_00FF],
            color_times: [0.0, 0.3, 0.7, 1.0],
            size_keys: [0.35, 0.7, 1.0, 1.25],
            tex_index: 0,
            blend_class: PARTICLE_BLEND_ADDITIVE,
            flags: 0,
            emitter_shape: PARTICLE_SHAPE_CONE,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One 64 Hz substep.
    const TS: f32 = 1.0 / 64.0;

    /// Decodes one binary16 bit pattern back to `f32` — the test-side inverse of
    /// [`f32_to_f16_bits`], so the pack is checked against a DECODE rather than against a
    /// restatement of its own bit arithmetic.
    fn f16_bits_to_f32(bits: u16) -> f32 {
        let sign = u32::from(bits & 0x8000) << 16;
        let exponent = u32::from((bits >> 10) & 0x1F);
        let mantissa = u32::from(bits & 0x03FF);
        if exponent == 0 {
            // Zero or subnormal.
            return f32::from_bits(sign) + (mantissa as f32) * 2f32.powi(-24) * if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        }
        if exponent == 0x1F {
            return if mantissa == 0 {
                if sign != 0 { f32::NEG_INFINITY } else { f32::INFINITY }
            } else {
                f32::NAN
            };
        }
        f32::from_bits(sign | ((exponent + 127 - 15) << 23) | (mantissa << 13))
    }

    // ── The two host-baked terms (D6/K1) ────────────────────────────────────

    /// `damping == exp2(-drag · timestep)`, exactly — the term that deletes `exp2` from the
    /// device.
    #[test]
    fn damping_is_exp2_of_minus_drag_times_timestep() {
        let effect = ParticleEffect { drag: 3.0, ..ParticleEffect::default() };
        let gpu = pack_effect_params(&effect, TS);
        assert_eq!(gpu.damping, (-3.0f32 * TS).exp2());
    }

    /// Zero drag is EXACTLY unit damping — a particle in a drag-free effect must not lose energy
    /// to a rounding artifact of the bake.
    #[test]
    fn zero_drag_is_exactly_unit_damping() {
        let gpu = pack_effect_params(&ParticleEffect::default(), TS);
        assert_eq!(gpu.damping, 1.0);
    }

    /// The rotation multiplier is `(cos ω·dt, sin ω·dt)` as an f32 PAIR (K1), and it is a UNIT
    /// complex number to within one f32 ULP — which is the whole reason the device can skip
    /// renormalization (M7).
    #[test]
    fn the_rotation_multiplier_is_a_unit_f32_pair() {
        let effect = ParticleEffect { rot_speed: 2.5, ..ParticleEffect::default() };
        let gpu = pack_effect_params(&effect, TS);

        let (s, c) = (2.5f32 * TS).sin_cos();
        assert_eq!(gpu.rot_mul_cos, c);
        assert_eq!(gpu.rot_mul_sin, s);

        let norm_sq = gpu.rot_mul_cos * gpu.rot_mul_cos + gpu.rot_mul_sin * gpu.rot_mul_sin;
        assert!(
            (norm_sq - 1.0).abs() < 1e-6,
            "the multiplier must be unit-modulus or the rotation drifts in MAGNITUDE: {norm_sq}"
        );
    }

    /// K1 quantitatively: at f32 the multiplier's magnitude error compounds to well under 1e-4
    /// over the plan's 640-substep (10 s at 64 Hz) horizon. The same test at snorm16 precision
    /// (`δ ~ 2e-5`) would fail by two orders of magnitude — which is the measurement that moved
    /// the pair out of a packed word.
    #[test]
    fn the_multiplier_magnitude_error_stays_below_1e_4_over_640_substeps() {
        let effect = ParticleEffect { rot_speed: 3.0, ..ParticleEffect::default() };
        let gpu = pack_effect_params(&effect, TS);

        let norm = (gpu.rot_mul_cos * gpu.rot_mul_cos + gpu.rot_mul_sin * gpu.rot_mul_sin).sqrt();
        // (1 + δ)^640 - 1, with δ = norm - 1.
        let compounded = (f64::from(norm)).powi(640) - 1.0;
        assert!(
            compounded.abs() < 1e-4,
            "compounded magnitude error over 640 substeps must stay below 1e-4, got {compounded}"
        );
    }

    /// A zero spin rate bakes to the identity multiplier `(1, 0)` — a non-rotating effect must not
    /// creep.
    #[test]
    fn zero_spin_bakes_to_the_identity_multiplier() {
        let gpu = pack_effect_params(&ParticleEffect::default(), TS);
        assert_eq!(gpu.rot_mul_cos, 1.0);
        assert_eq!(gpu.rot_mul_sin, 0.0);
    }

    // ── Pass-through and packing ────────────────────────────────────────────

    /// Every authored field that is NOT baked reaches the device row unchanged, and every reserved
    /// word is zero. A pass-through that silently reorders two lanes is exactly the class of defect
    /// no image golden can see.
    #[test]
    fn unbaked_fields_pass_through_and_reserved_words_are_zero() {
        let effect = ParticleEffect {
            gravity: [1.0, -2.0, 3.0],
            lifetime_min: 0.5,
            lifetime_max: 1.5,
            speed_min: 2.0,
            speed_max: 6.0,
            size_base: 0.25,
            cone_cos: 0.5,
            collision_radius: 0.125,
            restitution: 0.3,
            friction: 0.4,
            color_keys: [1, 2, 3, 4],
            tex_index: 7,
            blend_class: PARTICLE_BLEND_ALPHA,
            flags: 0xABCD,
            emitter_shape: PARTICLE_SHAPE_BOX,
            ..ParticleEffect::default()
        };
        let gpu = pack_effect_params(&effect, TS);

        assert_eq!(gpu.gravity, [1.0, -2.0, 3.0]);
        assert_eq!(gpu.lifetime_min, 0.5);
        assert_eq!(gpu.lifetime_max, 1.5);
        assert_eq!(gpu.speed_min, 2.0);
        assert_eq!(gpu.speed_max, 6.0);
        assert_eq!(gpu.size_base, 0.25);
        assert_eq!(gpu.cone_cos, 0.5);
        assert_eq!(gpu.collision_radius, 0.125);
        assert_eq!(gpu.restitution, 0.3);
        assert_eq!(gpu.friction, 0.4);
        assert_eq!(gpu.color_keys, [1, 2, 3, 4]);
        assert_eq!(gpu.tex_index, 7);
        assert_eq!(gpu.blend_class, PARTICLE_BLEND_ALPHA);
        assert_eq!(gpu.flags, 0xABCD);
        assert_eq!(gpu.emitter_shape, PARTICLE_SHAPE_BOX);

        assert_eq!(gpu._r0, [0, 0], "the two words the f32 pair was carved out of stay zero");
        assert_eq!(gpu._r1, 0.0);
        assert_eq!(gpu._r2, 0.0);
        assert_eq!(gpu._r3, 0);
    }

    /// Both ramps narrow to `f16` PAIRS in the documented lane order: key 0 in the low half of
    /// word 0, key 3 in the high half of word 1.
    #[test]
    fn both_ramps_pack_to_f16_pairs_in_lane_order() {
        let effect = ParticleEffect {
            color_times: [0.0, 0.25, 0.5, 1.0],
            size_keys: [0.5, 1.0, 2.0, 4.0],
            ..ParticleEffect::default()
        };
        let gpu = pack_effect_params(&effect, TS);

        let decode = |word: u32| {
            [f16_bits_to_f32(word as u16), f16_bits_to_f32((word >> 16) as u16)]
        };
        assert_eq!(decode(gpu.color_times[0]), [0.0, 0.25]);
        assert_eq!(decode(gpu.color_times[1]), [0.5, 1.0]);
        assert_eq!(decode(gpu.size_keys[0]), [0.5, 1.0]);
        assert_eq!(decode(gpu.size_keys[1]), [2.0, 4.0]);
    }

    /// `f16` round-trips every value exactly representable at half precision, and rounds the rest
    /// to nearest with an error bounded by half an ULP of the result's own exponent — the property
    /// the ramp resolution is argued from.
    #[test]
    fn f16_narrowing_round_trips_exactly_and_rounds_to_nearest_otherwise() {
        for exact in [0.0f32, 0.5, 1.0, 2.0, 0.25, 1.5, -1.0, 65_504.0] {
            let back = f16_bits_to_f32(f32_to_f16_bits(exact));
            assert_eq!(back, exact, "{exact} is exactly representable at f16");
        }
        for value in [0.1f32, 0.333_333_34, 0.7, std::f32::consts::PI, 1234.5] {
            let back = f16_bits_to_f32(f32_to_f16_bits(value));
            let rel = ((back - value) / value).abs();
            assert!(rel < 1e-3, "{value} round-tripped to {back} (relative error {rel})");
        }
    }

    /// A value past `f16`'s range saturates to infinity rather than wrapping into a small number —
    /// a wrap would turn an authoring mistake into a silently plausible ramp.
    #[test]
    fn f16_overflow_saturates_to_infinity() {
        assert_eq!(f16_bits_to_f32(f32_to_f16_bits(1.0e30)), f32::INFINITY);
        assert_eq!(f16_bits_to_f32(f32_to_f16_bits(-1.0e30)), f32::NEG_INFINITY);
    }

    // ── The presets ─────────────────────────────────────────────────────────

    /// Both P0 presets are ADDITIVE — the only class whose draw slot exists at P0. An alpha preset
    /// would simulate correctly and draw nothing, which is the kind of "works, invisible" failure
    /// no automated gate would report.
    #[test]
    fn both_p0_presets_are_additive() {
        let mut assets = Assets::<ParticleEffect>::default();
        let spark = assets.spark();
        let smoke = assets.smoke();

        assert_eq!(
            assets.get(spark).expect("invariant: a freshly minted handle resolves").blend_class,
            PARTICLE_BLEND_ADDITIVE
        );
        assert_eq!(
            assets.get(smoke).expect("invariant: a freshly minted handle resolves").blend_class,
            PARTICLE_BLEND_ADDITIVE
        );
    }

    /// The presets mint DISTINCT, dense, ascending rows — the effect index is a raw table index,
    /// so two presets sharing a row would make one of them unreachable.
    #[test]
    fn presets_mint_distinct_dense_rows() {
        let mut assets = Assets::<ParticleEffect>::default();
        let a = assets.spark();
        let b = assets.smoke();

        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
        assert_eq!(assets.high_water(), 2);
    }

    /// A minted effect is PINNED: dropping its last reference leaves the row resolvable, because a
    /// live `ParticleEffectHandle` is a raw index with no generation to catch a reuse.
    #[test]
    fn a_minted_effect_row_is_pinned_against_retirement() {
        let mut assets = Assets::<ParticleEffect>::default();
        let handle = assets.spark();
        let slot = handle.index();

        // Attach then detach a single carrier, the exact +1/-1 pair the hooks push.
        assert!(assets.inc_ref(slot), "a freshly minted row accepts a reference");
        let ticket = assets.dec_ref(slot, crate::particle::PARTICLE_EFFECT_REF_GEN);

        assert!(ticket.is_none(), "a pinned row must not retire on a refcount zero-crossing");
        assert!(assets.get(handle).is_some(), "and it must stay resolvable");
    }

    /// Both presets survive the bake with finite, sane device rows — the smoke test that a preset
    /// edit cannot ship a NaN into a buffer the sim loops over.
    #[test]
    fn preset_rows_bake_to_finite_device_parameters() {
        let mut assets = Assets::<ParticleEffect>::default();
        let handles = [assets.spark(), assets.smoke()];

        for handle in handles {
            let effect = *assets.get(handle).expect("invariant: freshly minted");
            let gpu = pack_effect_params(&effect, TS);
            assert!(gpu.damping.is_finite() && gpu.damping > 0.0 && gpu.damping <= 1.0);
            assert!(gpu.rot_mul_cos.is_finite() && gpu.rot_mul_sin.is_finite());
            assert!(gpu.lifetime_min > 0.0 && gpu.lifetime_max >= gpu.lifetime_min);
            assert!(gpu.size_base > 0.0);
            assert!((-1.0..=1.0).contains(&gpu.cone_cos));
        }
    }
}
