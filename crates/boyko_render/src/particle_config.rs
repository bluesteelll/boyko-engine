//! Particles P0 (D13, D14) + P1 (D9) + P2 (D10/R10) — the ECS-native owner-set arming knobs for
//! the GPU particle subsystem: [`ParticleMode`] (simulate and draw at all?),
//! [`ParticleCollision`] (collide against the SDF field?) and [`ParticleSortMode`] (order the
//! ALPHA class back-to-front?), three INDEPENDENT axes, all three `#[default] Off`/`None`.
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
    /// **Rung P1b's INSTRUMENTED collide arm** — [`Sdf`](Self::Sdf)'s simulation, exactly, plus the
    /// per-wave skip census the `-D SDF_COLLIDE_STATS` module publishes into
    /// [`ParticleCounters`](crate::particle::ParticleCounters)' three stats words.
    ///
    /// **A MEASUREMENT arm, not a shipping one.** It is a third compiled module rather than a
    /// runtime flag over [`Sdf`](Self::Sdf) for F24's reason — a runtime-gated atomic span would be
    /// paid on every disarmed frame — and the consequence is that it runs 1–2 extra atomics per
    /// wave per substep, which is a cost a shipping configuration should not carry. The physics it
    /// simulates is bit-identical to [`Sdf`](Self::Sdf)'s: the stats span only reads the skip
    /// predicate the collide arm already computed.
    ///
    /// Selecting it is the ONLY way any of the three stats words becomes non-zero.
    SdfStats,
}

impl ParticleCollision {
    /// The ARTIFACT spelling — `"off"` / `"sdf"` / `"sdf_stats"`. See [`ParticleMode::as_str`] for
    /// why the spelling is a table rather than a `Debug` formatting.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            ParticleCollision::Off => "off",
            ParticleCollision::Sdf => "sdf",
            ParticleCollision::SdfStats => "sdf_stats",
        }
    }

    /// Whether this arm publishes rung P1b's per-wave skip census — the structural predicate that
    /// tells a reader of a counter block whether its three stats words mean anything.
    ///
    /// Distinct from [`ParticleConfig::collides`]: every stats arm collides, but not every colliding
    /// arm counts. A reader that confused the two would report a skip rate of `0/0` for a plain
    /// `Sdf` run as though the field had never been evaluated.
    #[inline]
    pub const fn counts_waves(self) -> bool {
        matches!(self, ParticleCollision::SdfStats)
    }

    /// Every collision mode, for exhaustive iteration in tests and artifact readers.
    pub const ALL: [ParticleCollision; 3] =
        [ParticleCollision::Off, ParticleCollision::Sdf, ParticleCollision::SdfStats];
}

// ---- ParticleSortMode (rung P2 item 3's own arming axis; D10 / R10) ------------------

/// Whether — and how — the ALPHA blend class is ordered back-to-front before it is drawn
/// (`docs/PARTICLES-PLAN.md` rung P2 / D10). `#[repr(u32)]` for [`ParticleMode`]'s reason.
///
/// # Only the ALPHA class is ever a subject, and that is STRUCTURAL
///
/// The additive class needs no sort and is not given one: `ONE/ONE` is commutative and, under the
/// 8-bit saturation `lit` imposes, `sat(sat(x) + y) = min(1, x + y)` is order-independent (D10,
/// research fact R5). So this knob names one class, and a scene with no alpha effect pays nothing
/// for arming it beyond three dispatches that see a zero instance count.
///
/// # Its own axis, not a [`ParticleMode`] variant
///
/// Sorting is orthogonal to shading and to collision, exactly as [`ParticleCollision`] is: rung
/// P3's lit mode will want sorted and unsorted alpha alike, and folding the knobs together would
/// make each new one a cross product of variants.
///
/// # ⚠️ R10 — a sorted class CANNOT carry motion vectors
///
/// Research fact R10 (the Godot pitfall): particle motion vectors are only reconstructible while a
/// particle's INDEX is stable frame to frame, and a depth sort re-permutes indices every frame by
/// construction. The rule is therefore a hard one — `SortMode != None` ⇒ particle motion vectors
/// disabled — and it lives on this enum as [`motion_vectors_allowed`](Self::motion_vectors_allowed)
/// so that rung P3's `-D MOTION` resolver reads the rule rather than restating it.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ParticleSortMode {
    /// No sort — the DEFAULT, and byte-identical to rung P2 item 2: no sort pass is declared, no
    /// sort buffer is allocated, no sort pipeline is created, and the alpha draw reads the render
    /// records in the order the sim's waves retired.
    ///
    /// That order is arbitrary but not wrong for every scene: alpha billboards that do not OVERLAP
    /// each other composite identically in any order, which is the same argument gate #16's
    /// `particle_additive` pin rests on for the additive class.
    #[default]
    None,
    /// One FFX-shaped radix pass over an 8-bit quantized log-depth key — histogram → 256-bin scan →
    /// scatter, three dispatches (D10).
    ///
    /// The key is INVERTED (bin 0 is the farthest), so the plain ascending sort the three passes
    /// implement lands the class back-to-front, which is the order `alpha_over` needs. Eight bits
    /// rather than a 4-pass 32-bit radix because the blend it feeds is 8-bit: D10 prices the wider
    /// sort at 3–4× the cost for precision the destination cannot represent.
    Radix,
}

impl ParticleSortMode {
    /// The ARTIFACT spelling — `"none"` / `"radix"`. See [`ParticleMode::as_str`] for why the
    /// spelling is a table rather than a `Debug` formatting.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            ParticleSortMode::None => "none",
            ParticleSortMode::Radix => "radix",
        }
    }

    /// **R10, as a predicate rather than as prose**: whether particle motion vectors may be
    /// produced under this arming.
    ///
    /// True for [`None`](Self::None) alone. A sort re-permutes `p_render` every frame, so slot `k`
    /// of frame N and slot `k` of frame N+1 are different particles and the difference of their
    /// positions is not a velocity — it is noise with the magnitude of the scene. Rung P3's
    /// `-D MOTION` resolver consults this; until it lands, the boot site asserts it, so the rule is
    /// live rather than filed.
    #[inline]
    pub const fn motion_vectors_allowed(self) -> bool {
        matches!(self, ParticleSortMode::None)
    }

    /// Every sort mode, for exhaustive iteration in tests and artifact readers.
    pub const ALL: [ParticleSortMode; 2] = [ParticleSortMode::None, ParticleSortMode::Radix];
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
    /// Rung P2 item 3's ALPHA-class sort arming — boot-frozen, because it decides whether the two
    /// sort buffers and the three sort pipelines exist at all (structural absence, D13's rule
    /// applied to a fourth axis). Default [`ParticleSortMode::None`].
    pub sort: ParticleSortMode,
}

impl Default for ParticleConfig {
    #[inline]
    fn default() -> Self {
        // Off == today (the 0%-gate anchor): a default world simulates and draws no particles, and
        // an armed world that says nothing about collision or sorting gets the base sim module and
        // the unsorted alpha class rung P2 item 2 shipped.
        Self {
            mode: ParticleMode::Off,
            capacity: PARTICLE_DEFAULT_CAPACITY,
            collision: ParticleCollision::Off,
            sort: ParticleSortMode::None,
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

    /// Whether the sim publishes rung P1b's per-wave skip census — forwards
    /// [`ParticleCollision::counts_waves`], so the predicate has ONE definition and the config is
    /// merely where a caller reaches it.
    #[inline]
    pub const fn counts_waves(&self) -> bool {
        self.collision.counts_waves()
    }

    /// Whether the ALPHA class is depth-sorted — the structural predicate `sort != None`, the ONE
    /// value the boot site turns into "allocate the two sort buffers, build the three sort
    /// pipelines, declare the three sort passes".
    ///
    /// Independent of [`enabled`](Self::enabled), for [`collides`](Self::collides)'s reason: a
    /// disarmed subsystem builds nothing at all, so this predicate is only consulted on the armed
    /// path.
    #[inline]
    pub const fn sorts(&self) -> bool {
        !matches!(self.sort, ParticleSortMode::None)
    }

    /// **R10** — whether particle motion vectors may be produced under this config. Forwards
    /// [`ParticleSortMode::motion_vectors_allowed`], so the rule has ONE definition.
    #[inline]
    pub const fn motion_vectors_allowed(&self) -> bool {
        self.sort.motion_vectors_allowed()
    }
}

// R10, as a BUILD-time statement rather than a runtime one: exactly one arm of the sort axis
// permits motion vectors, and it is the one that performs no permutation. A future `Wboit` arm —
// which reorders nothing and could legitimately carry them — has to state its own answer here,
// which is the point of spelling the count rather than the arm.
const _: () = {
    let mut allowed = 0;
    let mut i = 0;
    while i < ParticleSortMode::ALL.len() {
        if ParticleSortMode::ALL[i].motion_vectors_allowed() {
            allowed += 1;
        }
        i += 1;
    }
    assert!(
        allowed == 1,
        "R10: exactly one ParticleSortMode may carry motion vectors — the one that does not \
         re-permute p_render"
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    /// A config literal at the two defaults, so a test that varies ONE knob does not have to
    /// restate the others (and so a third axis lands in one place).
    fn cfg(mode: ParticleMode, capacity: u32) -> ParticleConfig {
        ParticleConfig {
            mode,
            capacity,
            collision: ParticleCollision::Off,
            sort: ParticleSortMode::None,
        }
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
        assert_eq!(cfg.sort, ParticleSortMode::None);
        assert!(!cfg.sorts(), "rung P2 item 3's arm is default-OFF (no sort pass, no sort buffer)");
    }

    /// The sort axis's artifact spelling gets the SAME three claims its two siblings' do.
    #[test]
    fn the_sort_artifact_spelling_is_total_and_unique() {
        assert_eq!(ParticleSortMode::ALL.len(), 2, "ALL must list every sort mode");
        for (i, a) in ParticleSortMode::ALL.iter().enumerate() {
            assert_eq!(*a as u32, i as u32, "{a:?} must sit at its own discriminant in ALL");
            assert!(!a.as_str().is_empty(), "{a:?} has no word");
            for b in &ParticleSortMode::ALL[i + 1..] {
                assert_ne!(a.as_str(), b.as_str(), "{a:?} and {b:?} share a word");
            }
        }
        // The "off" arm of THIS axis is spelled `none`, not `off`, and that is deliberate: an
        // artifact line carrying three axes reads `off/off/none`, so a reader can tell which axis a
        // word came from even when the line's field order is what drifted.
        assert_ne!(ParticleSortMode::None.as_str(), ParticleMode::Off.as_str());
    }

    #[test]
    fn default_sort_is_none() {
        // The second route into `None`, exactly as `default_collision_is_off`: the hand-written
        // `ParticleConfig::default` names the variant literally while `ParticleSortMode::default`
        // comes from the `#[default]` attribute.
        assert_eq!(ParticleSortMode::default(), ParticleSortMode::None);
    }

    /// A WILDCARD-FREE match over the sort axis, so a future arm (D10's deferred `Wboit`, or a
    /// wider key) fails to COMPILE here rather than inheriting whichever answer a `_` swallowed.
    #[test]
    fn every_sort_variant_states_its_own_answer_without_a_wildcard() {
        for sort in ParticleSortMode::ALL {
            let want = match sort {
                ParticleSortMode::None => false,
                ParticleSortMode::Radix => true,
            };
            assert_eq!(
                ParticleConfig { sort, ..ParticleConfig::default() }.sorts(),
                want,
                "{sort:?}"
            );
            assert_eq!(
                sort as u32 != ParticleSortMode::None as u32,
                want,
                "{sort:?}: sorts() must track the `#[repr(u32)]` discriminant"
            );
        }
    }

    /// **R10, exercised rather than filed.** `SortMode != None` ⇒ motion vectors disabled, on every
    /// arm, wildcard-free — and the two predicates are stated as EXACT COMPLEMENTS, because the
    /// defect this guards is one drifting into "sorts() is usually the opposite of
    /// motion_vectors_allowed()".
    #[test]
    fn r10_a_sorted_class_may_not_carry_motion_vectors() {
        for sort in ParticleSortMode::ALL {
            let allowed = match sort {
                ParticleSortMode::None => true,
                ParticleSortMode::Radix => false,
            };
            assert_eq!(sort.motion_vectors_allowed(), allowed, "{sort:?}: the enum's own rule");
            let cfg = ParticleConfig { sort, ..ParticleConfig::default() };
            assert_eq!(cfg.motion_vectors_allowed(), allowed, "{sort:?}: the config forwards it");
            assert_ne!(
                cfg.sorts(),
                cfg.motion_vectors_allowed(),
                "{sort:?}: R10 makes the two predicates exact complements — a permutation of \
                 `p_render` destroys the index stability a motion vector is reconstructed from"
            );
        }
    }

    /// The sort axis is orthogonal to the other two — the property that keeps rung P2 item 3 from
    /// becoming a [`ParticleMode`] × [`ParticleCollision`] cross product.
    #[test]
    fn sorting_is_orthogonal_to_the_other_two_axes() {
        let armed_unsorted = cfg(ParticleMode::GpuUnlit, 1);
        assert!(armed_unsorted.enabled() && !armed_unsorted.sorts());
        let disarmed_sorted =
            ParticleConfig { sort: ParticleSortMode::Radix, ..ParticleConfig::default() };
        assert!(!disarmed_sorted.enabled() && disarmed_sorted.sorts());
        let sorted_collider = ParticleConfig {
            mode: ParticleMode::GpuUnlit,
            collision: ParticleCollision::Sdf,
            sort: ParticleSortMode::Radix,
            ..ParticleConfig::default()
        };
        assert!(sorted_collider.enabled() && sorted_collider.collides() && sorted_collider.sorts());
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
        assert_eq!(ParticleCollision::ALL.len(), 3, "ALL must list every collision mode");
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
                ParticleCollision::SdfStats => true,
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

    /// Rung P1b's census predicate, wildcard-free for the same reason: a fourth collider arm must
    /// state whether it counts, rather than inheriting `SdfStats`' answer or `Sdf`'s.
    ///
    /// The two predicates are DIFFERENT partitions of the same enum — `collides()` splits after
    /// `Off`, `counts_waves()` splits before `SdfStats` — and that difference is the whole reason
    /// both exist. Asserted as the pair, because the defect this guards is one collapsing into the
    /// other.
    #[test]
    fn only_the_stats_arm_counts_waves_and_it_also_collides() {
        for collision in ParticleCollision::ALL {
            let counts = match collision {
                ParticleCollision::Off => false,
                ParticleCollision::Sdf => false,
                ParticleCollision::SdfStats => true,
            };
            let cfg = ParticleConfig { collision, ..ParticleConfig::default() };
            assert_eq!(cfg.counts_waves(), counts, "{collision:?}");
            assert_eq!(collision.counts_waves(), counts, "{collision:?}: the enum's own predicate");
            assert!(
                !counts || cfg.collides(),
                "{collision:?}: an arm that counts field evaluations must evaluate the field"
            );
        }
        // Not the same partition: `Sdf` collides and does not count. A `counts_waves` that had
        // been written as `collides()` would pass every assertion above except this one.
        let sdf = ParticleConfig { collision: ParticleCollision::Sdf, ..ParticleConfig::default() };
        assert!(sdf.collides() && !sdf.counts_waves());
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
