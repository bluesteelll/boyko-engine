//! Particles P0 (D13, D14) + P1 (D9) — the ECS-native owner-set arming knobs for the GPU particle
//! subsystem: [`ParticleMode`] (simulate and draw at all?) and [`ParticleCollision`] (collide
//! against the SDF field?), two INDEPENDENT axes, both `#[default] Off`.
//!
//! Principle 0: ECS-native — [`ParticleConfig`] is a `#[derive(Resource)]` singleton (the cold
//! owner-set config, NOT a side `std::Vec`/`HashMap`), mirroring
//! [`OcclusionConfig`](crate::occlusion_config::OcclusionConfig) exactly: one enum knob, a
//! structural predicate, no stored flag.
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether the engine runs the particle passes is keyed off the [`ParticleMode`] enum, NOT a
//! separate flag — [`ParticleMode::Off`] IS "disabled". [`ParticleConfig::enabled`] is a derived
//! predicate (`mode != Off`), not stored state, exactly as
//! [`OcclusionConfig::enabled`](crate::occlusion_config::OcclusionConfig::enabled) is.
//!
//! # The 0%-gate, and why the default is `Off`
//!
//! [`ParticleConfig::default`] is [`ParticleMode::Off`] — byte-identical to a world that never
//! inserts the Resource (plan invariant 2, *structural absence*): no pass, no `ResId`, no
//! pipeline, no buffer, no shader loaded, and every committed `goldens/PINS.toml` hash unchanged
//! **by construction** rather than by measurement. This is what lets a host compose
//! [`ParticlePlugin`](crate::particle_plugin::ParticlePlugin) unconditionally.
//!
//! # `capacity` bounds MEMORY only (D14)
//!
//! `CAP` is read once at boot and frozen; per-frame work is `O(alive)`, never `O(CAP)`. Exceeding
//! it clamps inside the GPU kickoff pass and increments
//! [`ParticleCounters::clamped_spawns`](crate::particle::ParticleCounters). The default
//! [`PARTICLE_DEFAULT_CAPACITY`] costs 24.1 MB of VRAM at the plan's 92 B/particle.

use boyko_macros::Resource;

/// The boot-frozen pool capacity default (D14) — 262 144 particles ≈ 24.1 MB of VRAM at the
/// plan's 92 B/particle (48 B sim record + 32 B render record + 12 B of list entries).
///
/// `CAP` bounds MEMORY only: the kickoff pass clamps a spawn request that would exceed the
/// free-list and counts the shortfall, and every per-frame dispatch is sized from the live
/// counts, never from this number (plan R9).
pub const PARTICLE_DEFAULT_CAPACITY: u32 = 262_144;

// ---- ParticleMode (the owner-set knob; capability is structural) ---------------------

/// Whether — and how — the engine simulates and draws GPU particles. `#[repr(u32)]` so the
/// discriminant is a stable arm word (the [`OcclusionMode`](crate::occlusion_config::OcclusionMode)
/// rule).
///
/// Two variants at P0. Rung P3 adds a third (`GpuLit`, per-particle froxel lookup evaluated in
/// the sim — D11); the wildcard-free matches in this module's tests are what make that addition a
/// COMPILE error here rather than a silent inheritance of whichever arm a `_` would have
/// swallowed.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ParticleMode {
    /// No particles — the 0%-gate and the DEFAULT: nothing is declared, nothing is recorded, no
    /// device buffer is allocated, and the frame is byte-identical to a world that never heard of
    /// this subsystem.
    ///
    /// ⚠️ `Off` means **do not SIMULATE OR DRAW**, never *do not SPAWN*: a
    /// [`ParticleEmitter`](crate::particle::ParticleEmitter) row still exists and still carries its
    /// accumulator, so the component's meaning does not depend on this knob. The cost of an armed
    /// emitter in a disarmed world is that the emitter systems never run at all.
    #[default]
    Off,
    /// The P0 shipping mode: GPU-resident simulation (kickoff → emit → sim, all indirect) drawing
    /// UNLIT additive billboards composited into `lit` through one
    /// `vkCmdDrawIndexedIndirect` on all four render paths.
    GpuUnlit,
}

impl ParticleMode {
    /// The ARTIFACT spelling — `"off"` / `"gpu_unlit"`.
    ///
    /// One table, for [`OcclusionMode::as_str`](crate::occlusion_config::OcclusionMode::as_str)'s
    /// reason: a diagnostic capture has to answer "which configuration produced this?" from the
    /// artifact rather than from a knob that was read once at boot, and a second spelling
    /// elsewhere would be a second text that can disagree with this one.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            ParticleMode::Off => "off",
            ParticleMode::GpuUnlit => "gpu_unlit",
        }
    }

    /// Every mode, for exhaustive iteration in tests and in artifact readers. The array's LENGTH
    /// is the arity, so a variant left out of it fails the tests below rather than going untested.
    pub const ALL: [ParticleMode; 2] = [ParticleMode::Off, ParticleMode::GpuUnlit];
}

// ---- ParticleCollision (rung P1's own arming axis) -----------------------------------

/// Whether — and against what — simulated particles collide (`docs/PARTICLES-PLAN.md` rung P1 /
/// D9). `#[repr(u32)]` for [`ParticleMode`]'s reason: the discriminant is a stable arm word.
///
/// # Its own axis, not a [`ParticleMode`] variant
///
/// Collision is ORTHOGONAL to how particles are shaded: rung P3's lit mode will want colliding and
/// non-colliding particles alike, and folding the two knobs into one enum would make that a cross
/// product of variants rather than two independent bits.
///
/// # A COMPILE-TIME arm, resolved once at boot
///
/// `Sdf` selects a different `particle_sim` SPIR-V (`-D SDF_COLLIDE`), not a runtime branch inside
/// one shader — plan F24's rule, measured: the VB-SV0 inline detour cost +75 % with its feature OFF
/// and no byte gate could see it. So this knob is read exactly once, where the pipeline is built,
/// and a live flip is NOT honoured (the field's doc says boot-frozen for the same reason
/// [`ParticleConfig::capacity`]'s does).
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ParticleCollision {
    /// Particles pass through everything — the DEFAULT, and the base `particle_sim` module. No
    /// field binding, no field evaluation, no `cached_field_d` traffic.
    #[default]
    Off,
    /// Particles collide against the engine's ONE SDF field (the `SdfPrimitive` edit list), with
    /// the per-effect `collision_radius` / `restitution` / `friction` deciding the contact.
    ///
    /// An effect whose `collision_radius` is 0 still passes through everything but the interior of
    /// a surface, so this arm is the SUBSYSTEM's switch and the effect row is the per-effect one.
    Sdf,
}

impl ParticleCollision {
    /// The ARTIFACT spelling — `"off"` / `"sdf"`. See [`ParticleMode::as_str`] for why the
    /// spelling is a table rather than a `Debug` formatting.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            ParticleCollision::Off => "off",
            ParticleCollision::Sdf => "sdf",
        }
    }

    /// Every collision mode, for exhaustive iteration in tests and artifact readers.
    pub const ALL: [ParticleCollision; 2] = [ParticleCollision::Off, ParticleCollision::Sdf];
}

// ---- ParticleConfig (the owner-set Resource — mirrors OcclusionConfig) ---------------

/// The global particle-subsystem arming config — a `World`-singleton Resource the owner sets.
///
/// Carries the [`ParticleMode`] knob (enablement is structural — `mode != Off` — so there is no
/// separate flag) plus the boot-frozen pool [`capacity`](Self::capacity).
///
/// Read **once at boot** by the host that builds the device bundle, and live per frame by the
/// declarators' conditional tail (D13/F9b). A live flip of `mode` is honoured by the declarators;
/// a live flip of `capacity` is NOT — the pool is sized once (D14), which is why the field's doc
/// says "boot-frozen" rather than "tunable".
#[derive(Resource, Clone, Copy, Debug)]
pub struct ParticleConfig {
    /// The owner-set arming. [`Off`](ParticleMode::Off) (the default) ⇒ the 0%-gate.
    pub mode: ParticleMode,
    /// The boot-frozen pool capacity in particles (D14). Bounds MEMORY only — per-frame work is
    /// `O(alive)`. Default [`PARTICLE_DEFAULT_CAPACITY`].
    pub capacity: u32,
    /// Rung P1's collision arming — boot-frozen, because it picks the sim's SPIR-V rather than a
    /// runtime branch. Default [`ParticleCollision::Off`].
    pub collision: ParticleCollision,
}

impl Default for ParticleConfig {
    #[inline]
    fn default() -> Self {
        // Off == today (the 0%-gate anchor): a default world simulates and draws no particles, and
        // an armed world that says nothing about collision gets the base sim module.
        Self {
            mode: ParticleMode::Off,
            capacity: PARTICLE_DEFAULT_CAPACITY,
            collision: ParticleCollision::Off,
        }
    }
}

impl ParticleConfig {
    /// Whether the particle subsystem runs — the structural predicate `mode != Off` (NOT stored
    /// state). True ⇒ the passes are declared and recorded and the device bundle is built at boot;
    /// false ⇒ the 0%-gate.
    #[inline]
    pub const fn enabled(&self) -> bool {
        !matches!(self.mode, ParticleMode::Off)
    }

    /// Whether the sim collides against the SDF field — the structural predicate
    /// `collision != Off`, the ONE value the boot site turns into `-D SDF_COLLIDE`'s pipeline pick.
    ///
    /// Independent of [`enabled`](Self::enabled): a disarmed subsystem builds no pipeline at all,
    /// so this predicate is only ever consulted on the armed path.
    #[inline]
    pub const fn collides(&self) -> bool {
        !matches!(self.collision, ParticleCollision::Off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config literal at the two defaults, so a test that varies ONE knob does not have to
    /// restate the others (and so a third axis lands in one place).
    fn cfg(mode: ParticleMode, capacity: u32) -> ParticleConfig {
        ParticleConfig { mode, capacity, collision: ParticleCollision::Off }
    }

    #[test]
    fn particle_config_default_is_off_the_zero_gate() {
        let cfg = ParticleConfig::default();
        assert_eq!(cfg.mode, ParticleMode::Off);
        assert!(!cfg.enabled(), "the default config is the 0%-gate (no particle passes)");
        assert_eq!(
            cfg.capacity, PARTICLE_DEFAULT_CAPACITY,
            "the default pool capacity is the plan's D14 number"
        );
        assert_eq!(cfg.collision, ParticleCollision::Off);
        assert!(!cfg.collides(), "rung P1's arm is default-OFF (the base sim module)");
    }

    /// The collision axis's artifact spelling gets the SAME three claims its sibling's does —
    /// total, unique, and indexable by the `#[repr(u32)]` discriminant.
    ///
    /// Written rather than the surface deleted because `as_str` exists for a reason a diagnostic
    /// reader depends on ([`ParticleMode::as_str`]'s doc): a capture must be able to answer "which
    /// configuration produced this?" from the artifact. An untested spelling is the version of that
    /// surface that can silently answer wrongly — two arms sharing a word, or an `ALL` that has
    /// stopped listing every variant, are both invisible until someone reads a log and believes it.
    #[test]
    fn the_collision_artifact_spelling_is_total_and_unique() {
        assert_eq!(ParticleCollision::ALL.len(), 2, "ALL must list every collision mode");
        for (i, a) in ParticleCollision::ALL.iter().enumerate() {
            assert_eq!(*a as u32, i as u32, "{a:?} must sit at its own discriminant in ALL");
            assert!(!a.as_str().is_empty(), "{a:?} has no word");
            for b in &ParticleCollision::ALL[i + 1..] {
                assert_ne!(a.as_str(), b.as_str(), "{a:?} and {b:?} share a word");
            }
        }
        // The two axes are independent, so their words may coincide without ambiguity ("off" is
        // both) — but only while a reader is told WHICH axis a word came from. Asserted here so the
        // coincidence is a recorded property rather than an accident nobody looked at.
        assert_eq!(ParticleCollision::Off.as_str(), ParticleMode::Off.as_str());
    }

    #[test]
    fn default_collision_is_off() {
        // The second route into `Off`, exactly as `default_mode_is_off` below: the hand-written
        // `ParticleConfig::default` names the variant literally, while `ParticleCollision::default`
        // comes from the `#[default]` attribute. Neither implies the other.
        assert_eq!(ParticleCollision::default(), ParticleCollision::Off);
    }

    /// A WILDCARD-FREE match over the collision axis, so a future arm (a mesh-BVH collider, a
    /// height field) fails to COMPILE here rather than inheriting whichever answer a `_` swallowed.
    #[test]
    fn every_collision_variant_states_its_own_answer_without_a_wildcard() {
        for collision in ParticleCollision::ALL {
            let want = match collision {
                ParticleCollision::Off => false,
                ParticleCollision::Sdf => true,
            };
            assert_eq!(
                ParticleConfig { collision, ..ParticleConfig::default() }.collides(),
                want,
                "{collision:?}"
            );
            assert_eq!(
                collision as u32 != ParticleCollision::Off as u32,
                want,
                "{collision:?}: collides() must track the `#[repr(u32)]` discriminant"
            );
        }
    }

    /// The collision axis is orthogonal to the arming one — the property that keeps rung P1 from
    /// becoming a `ParticleMode` cross product.
    #[test]
    fn collision_is_orthogonal_to_the_arming_predicate() {
        let armed_no_collide = cfg(ParticleMode::GpuUnlit, 1);
        assert!(armed_no_collide.enabled() && !armed_no_collide.collides());
        let disarmed_collide =
            ParticleConfig { collision: ParticleCollision::Sdf, ..ParticleConfig::default() };
        assert!(!disarmed_collide.enabled() && disarmed_collide.collides());
    }

    #[test]
    fn default_mode_is_off() {
        // The SECOND route into `Off`, pinned for `OcclusionConfig`'s own reason:
        // `ParticleConfig::default` is a hand-written impl naming `ParticleMode::Off` literally,
        // while `ParticleMode::default()` comes from the `#[default]` attribute. Neither derives
        // from the other, so neither test implies the other.
        assert_eq!(ParticleMode::default(), ParticleMode::Off);
    }

    #[test]
    fn enabled_agrees_with_the_discriminant_on_every_variant() {
        // Capability is structural: `enabled()` must be exactly "the discriminant is not Off's",
        // checked against the `#[repr(u32)]` word rather than against a restatement of the same
        // `matches!` the impl uses.
        for mode in ParticleMode::ALL {
            let expected = mode as u32 != ParticleMode::Off as u32;
            assert_eq!(
                cfg(mode, PARTICLE_DEFAULT_CAPACITY).enabled(),
                expected,
                "{mode:?}: enabled() must track the discriminant"
            );
        }
    }

    /// The artifact spelling is total, unique, and indexable by the `#[repr(u32)]` discriminant —
    /// the three properties a reader of a `[host] particle_mode` field depends on.
    #[test]
    fn the_artifact_spelling_is_total_and_unique() {
        assert_eq!(ParticleMode::ALL.len(), 2, "ALL must list every mode");
        for (i, a) in ParticleMode::ALL.iter().enumerate() {
            assert_eq!(*a as u32, i as u32, "{a:?} must sit at its own discriminant in ALL");
            assert!(!a.as_str().is_empty(), "{a:?} has no word");
            for b in &ParticleMode::ALL[i + 1..] {
                assert_ne!(a.as_str(), b.as_str(), "{a:?} and {b:?} share a word");
            }
        }
    }

    /// A WILDCARD-FREE match over the enum, so adding a variant (rung P3's `GpuLit`) fails to
    /// COMPILE here rather than silently inheriting whichever arm a `_` would have swallowed. The
    /// bodies restate the intended answer for each variant; the compile error is the actual gate.
    #[test]
    fn every_variant_states_its_own_answer_without_a_wildcard() {
        for mode in ParticleMode::ALL {
            let want = match mode {
                ParticleMode::Off => false,
                ParticleMode::GpuUnlit => true,
            };
            assert_eq!(
                cfg(mode, PARTICLE_DEFAULT_CAPACITY).enabled(),
                want,
                "{mode:?}"
            );
        }
    }

    /// `capacity` is orthogonal to `mode`: changing one must not move the other's answer. A
    /// disarmed config with a huge capacity still allocates nothing (structural absence), and an
    /// armed config with a tiny capacity is still armed.
    #[test]
    fn capacity_is_orthogonal_to_the_arming_predicate() {
        assert!(!cfg(ParticleMode::Off, 1 << 20).enabled());
        assert!(cfg(ParticleMode::GpuUnlit, 1).enabled());
    }
}
