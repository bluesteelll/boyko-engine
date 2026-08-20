//! Particles P0 (D13, D14) — the ECS-native owner-set arming knob for the GPU particle
//! subsystem.
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
}

impl Default for ParticleConfig {
    #[inline]
    fn default() -> Self {
        // Off == today (the 0%-gate anchor): a default world simulates and draws no particles.
        Self { mode: ParticleMode::Off, capacity: PARTICLE_DEFAULT_CAPACITY }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_config_default_is_off_the_zero_gate() {
        let cfg = ParticleConfig::default();
        assert_eq!(cfg.mode, ParticleMode::Off);
        assert!(!cfg.enabled(), "the default config is the 0%-gate (no particle passes)");
        assert_eq!(
            cfg.capacity, PARTICLE_DEFAULT_CAPACITY,
            "the default pool capacity is the plan's D14 number"
        );
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
                ParticleConfig { mode, capacity: PARTICLE_DEFAULT_CAPACITY }.enabled(),
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
                ParticleConfig { mode, capacity: PARTICLE_DEFAULT_CAPACITY }.enabled(),
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
        assert!(!ParticleConfig { mode: ParticleMode::Off, capacity: 1 << 20 }.enabled());
        assert!(ParticleConfig { mode: ParticleMode::GpuUnlit, capacity: 1 }.enabled());
    }
}
