//! The runtime switch — profiling rung 11, D20 and A8.
//!
//! # What a scope is, and why it is an ENTITY
//!
//! [`ARM_MASK`] is one 64-bit word read by every surviving emission site. Bits `0..8` are the
//! channels, written by [`Profiler::arm`](super::store::Profiler::arm); bits `8..64` are the
//! **scopes**, and this module is where they come from. A scope's on/off state is not stored
//! anywhere in the profiler: it is the kernel enable bit of an ordinary entity, and the mask is a
//! **projection** of those bits taken once per fold.
//!
//! That is the project's capability/state rule applied literally:
//!
//! | | Component | Storage |
//! |---|---|---|
//! | **capability** — this entity is a scope | [`ProfilingScope`] `{ bit, name }` | ordinary table |
//! | **runtime on/off** | [`ProfilingScopeEnabled`] | the kernel's bitset enable bit |
//!
//! There is no mask setter, no mirror, no dirty flag and no second source of truth. A console
//! command, a dev menu, a network handler or a save-file loader turns a scope on by toggling one
//! bit on one entity, from an ordinary **parallel** system.
//!
//! # Why the projection is a step of the FOLD and not an observer
//!
//! The extension's design projected on the transition, via an `IsEnabled` observer. **That
//! mechanism does not exist and cannot be built without a kernel change**: `enable_tag_api.rs:77-88`
//! documents the enable path as *"O(1) warm: no migration, no structural-generation bump, no hook /
//! observer fire, no deferred drain"* — the absence of a fire is precisely what buys the O(1)
//! toggle. So the projection is step 0 of the fold ([`project`]), and a toggle lands at the **next**
//! frame, never the same one. `G12` asserts the next frame, for both write paths.
//!
//! # Why a fielded scope may not BE the enable tag (B2)
//!
//! An earlier revision used one component for both roles — `ProfilingScope { bit, name }` as the
//! enable tag. The derive refuses a fielded bitset tag outright
//! (`boyko_macros/src/component.rs:580-604`, `reject_non_zst_bitset_tag`), which is the visible
//! half. The invisible half is worse and is what the split is really for: the **read** path
//! `is_enabled → test_enable_bit` (`enable_tag_api.rs:201-215`) carries **no storage-kind assert at
//! all** — it looks up `archetype.enable_store.column(tag)`, finds `None` for a non-bitset id and
//! returns `false`. Forcing the id through would therefore not panic in debug; it would project an
//! all-zero mask **in every build, silently**, and a permanently disarmed profiler with no
//! diagnostic is the failure this whole corpus exists to prevent. `G12` clause 3 runs both halves.
//!
//! The one `debug_assert!` the corpus asks for is at scope **registration** ([`register_scope`]),
//! which is also the earliest point that can carry it: registration is where the tag's id is first
//! minted, and it is the only site on this path that touches the tag at all.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use boyko_diag::profiling_abi::{
    self, PROJECTED_SCOPE_BASE, PROJECTED_SCOPE_MASK, SCOPE_COUNT, USER_SCOPE_BASE,
};

use crate::ecs::core::bundle::self_bundle::impl_self_bundle;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::iters::query::filter_enable::Enabled;
use crate::ecs::identifiers::primitives::ComponentId;

/// The lowest scope bit the ECS projection owns — re-exported so a caller naming an engine scope
/// does not have to reach into `boyko_diag`.
///
/// Engine scopes are `ENGINE_SCOPE_BASE..USER_SCOPE_BASE`; a game's are
/// [`USER_SCOPE_BASE`]`..`[`SCOPE_COUNT`], and [`register_scope`] mints out of the second range
/// only. The engine's are **constants rather than mints**: this engine knows its own subsystems at
/// compile time, and a counter handing them out at run time would make the same subsystem a
/// different bit depending on plugin order.
pub const ENGINE_SCOPE_BASE: u32 = PROJECTED_SCOPE_BASE;

/// **CAPABILITY** — an entity carrying this component *is* a profiling scope.
///
/// Ordinary table storage, because it carries data. Its presence is what makes the entity visible
/// to [`project`]; whether the scope is armed is [`ProfilingScopeEnabled`]'s answer, not this one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct ProfilingScope {
    /// The [`ARM_MASK`] bit this scope owns, in `ENGINE_SCOPE_BASE..SCOPE_COUNT`.
    pub bit: u8,
    /// The scope's name, for a reader. The profiler never interprets it.
    pub name: &'static str,
}

impl ProfilingScope {
    /// The mask bit this scope contributes to a projection, or `0` when `bit` names no projectable
    /// scope.
    ///
    /// **Zero rather than a wrap, for two different out-of-range cases.** A `bit` at or above
    /// [`SCOPE_COUNT`] has no bit in a `u64` at all, and `1 << bit` on it would be undefined —
    /// masking it to `bit % 64` would arm a scope belonging to somebody else, which is the one
    /// outcome worse than arming nothing. A `bit` below [`ENGINE_SCOPE_BASE`] names a **channel**,
    /// which the projection does not own; [`profiling_abi::project_scopes`] would drop it anyway,
    /// and dropping it here as well is what makes the field's contract readable at the field.
    ///
    /// The `debug_assert!` is the *other* half of that answer and not a contradiction of it:
    /// [`register_scope`] cannot produce an out-of-range bit, so reaching one means a `ProfilingScope`
    /// was hand-built with a bit nobody minted. In a debug build that is worth stopping on, because
    /// the observable symptom in release — a scope that simply never measures — is one a reader
    /// would spend an afternoon attributing to the zone rather than to the component.
    #[inline]
    #[must_use]
    pub fn arm_bit(&self) -> u64 {
        let bit = u32::from(self.bit);
        debug_assert!(
            (ENGINE_SCOPE_BASE..SCOPE_COUNT).contains(&bit),
            "invariant: a ProfilingScope's bit is a projectable scope, not a channel or a \
             non-existent bit"
        );
        if bit >= SCOPE_COUNT {
            return 0;
        }
        (1u64 << bit) & PROJECTED_SCOPE_MASK
    }
}

/// **RUNTIME ON/OFF** — the kernel enable bit for a scope. Fieldless, by the macro's requirement.
///
/// Toggled by `commands.entity(e).enable::<ProfilingScopeEnabled>()` from any parallel system, or
/// by `world.enable::<ProfilingScopeEnabled>(e)` where `&mut EcsMaster` is already held. Both land
/// at the **next** frame's projection; see the module docs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProfilingScopeEnabled;

/// A zone name resolved to an id **once at setup**, so a reader never calls the `#[cold]` by-name
/// lookup on a frame path (D25).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct ProfiledZone(pub u16);

/// Why [`register_scope`] refused.
///
/// # There is deliberately no `92xx` code behind this
///
/// Every number the corpus allocates — `9201..9218` — is spoken for, and `register_scope` is a
/// **setup** call in Rust code that returns its refusal synchronously to the one caller that can
/// act on it. A code would be a second statement of a fact the `Result` already delivers at the
/// only site able to use it. That is not true of `register_zone`'s refusals (`W9210` / `W9212`),
/// which come from *data* — a manifest, a script — where the caller forwarding a `Result` is not
/// the author of the mistake.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeError {
    /// All 32 game scope bits are taken. `ARM_MASK` is one word, and D28 refuses a second one
    /// until a title exhausts the first — which is what this refusal reports having happened.
    Exhausted,
}

/// The next free game scope bit, over `USER_SCOPE_BASE..SCOPE_COUNT`.
///
/// Process-global rather than per-world, exactly like the zone registry and for the same reason:
/// `ARM_MASK` is process-global, so two worlds handing out the same bit would arm each other's
/// scopes. Saturating at [`SCOPE_COUNT`] so the counter cannot wrap back into a live bit.
static NEXT_GAME_SCOPE: AtomicU32 = AtomicU32::new(USER_SCOPE_BASE);

/// Mint a game scope and return the component that names it.
///
/// ```ignore
/// let ai = register_scope("ai")?;
/// let e = world.spawn_bundle(ai);
/// world.enable::<ProfilingScopeEnabled>(e);      // ...or from a system, via commands
/// ```
///
/// # Why this returns the COMPONENT and not the bare bit
///
/// The corpus spells it `register_scope(name) -> Result<u8, ScopeError>`, and the deviation is
/// deliberate. With a bare `u8` the caller must then write `ProfilingScope { bit, name }` itself,
/// naming the scope a second time — and nothing checks that the second name is the first one. A
/// scope whose component says `"ai"` while its bit was minted for `"audio"` is a mislabelled
/// measurement that no gate downstream can detect, because both halves are individually
/// well-formed. Returning the pair the caller was going to build removes the chance to build it
/// wrong; the bit is still `scope.bit` for anyone who wants it.
///
/// # `#[cold]`
///
/// Setup only. A game registers its scopes once, at boot or at plugin build.
#[cold]
pub fn register_scope(name: &'static str) -> Result<ProfilingScope, ScopeError> {
    // B2's `debug_assert`, at the earliest site that can carry it. `component_id()` is what mints
    // the tag, so this is also the call that makes the classification exist to be asserted — and
    // the read path (`test_enable_bit`) has no assert of its own, which is exactly why the corpus
    // puts one here.
    debug_assert_eq!(
        component_registry::storage_kind(ProfilingScopeEnabled::component_id().0),
        component_registry::StorageKind::Bitset,
        "invariant: ProfilingScopeEnabled is a bitset enable tag; a table-storage id would make \
         every is_enabled read false and project an all-zero mask silently"
    );

    let bit = NEXT_GAME_SCOPE.fetch_add(1, Ordering::Relaxed);
    if bit >= SCOPE_COUNT {
        // Saturate rather than let the counter climb: a wrap would eventually hand out a bit that
        // is live, and two scopes on one bit toggle each other.
        NEXT_GAME_SCOPE.store(SCOPE_COUNT, Ordering::Relaxed);
        return Err(ScopeError::Exhausted);
    }
    Ok(ProfilingScope { bit: bit as u8, name })
}

/// Put the game-scope mint back to its start. **Test builds only.**
///
/// # Why a saturating test must restore what it spent
///
/// `NEXT_GAME_SCOPE` is process-global and monotone, and one test in this crate
/// (`register_scope_mints_in_the_game_range_and_refuses_past_the_word`) **saturates it by design**
/// -- that IS its subject. Everything after it in the same binary then sees an exhausted space, and
/// the `g12` gates, whose zones are declared on the top two bits, fail with "a register_scope mint
/// has reached this gate's own bits".
///
/// That was recorded as a known order dependency and guarded with an assertion rather than fixed:
/// *"under the module lock the two orders are both legal, and only the assertion tells them
/// apart."* Both orders being legal is precisely what makes the suite flaky -- the gate passes or
/// fails on which test won the race, which is a gate that reports the scheduler. The saturating
/// test now restores the counter under the same serial lock it saturates it under, so both orders
/// are legal AND correct.
#[cfg(test)]
pub(crate) fn reset_game_scope_mint() {
    NEXT_GAME_SCOPE.store(USER_SCOPE_BASE, Ordering::Relaxed);
}

/// Game scope bits minted so far, for a gate or a report. `0` before the first [`register_scope`].
#[must_use]
pub fn minted_game_scopes() -> u32 {
    NEXT_GAME_SCOPE.load(Ordering::Relaxed).min(SCOPE_COUNT) - USER_SCOPE_BASE
}

/// **A8 — the scope projection.** Step 0 of the fold. Returns whether the mask changed.
///
/// # The query IS the projection
///
/// The corpus writes this as a loop over `scope_entity[b]` calling `is_enabled` per bit — which
/// needs a bit → entity table somebody has to keep. There is no such table here, and there should
/// not be: the kernel's own enable **filter** expresses the projection directly. The query yields
/// exactly the scopes that are on, so the loop is over the answer rather than over the search
/// space, and the association between a bit and its entity is the component itself. A `[Entity; 64]`
/// beside it would be a second statement of what the ECS already says — the mirror D20 forbids in
/// the same paragraph that forbids a mask setter.
///
/// # Cost
///
/// One cached-query lookup (`~5 ns` warm, per `EcsMaster::query`'s own documented cost) plus one
/// archetype walk over the scope entities, which a title has ~16 of and 64 at the maximum. It runs
/// inside `__fold`, i.e. inside `instrument_measured` and outside `__frame` (D16), so it is
/// **disclosed** rather than hidden.
///
/// The **first** call per world pays `query_cold_init`'s one-time ~1 µs, on the first armed frame.
/// That is the enable path in every sense that matters: a process that never arms never folds, and
/// never reaches this function.
pub fn project(world: &mut EcsMaster) -> bool {
    profiling_abi::project_scopes(projected_bits(world))
}

/// What the projection WOULD publish, without publishing it.
///
/// Exists for `G12`: an assertion about the mask cannot distinguish "the projection computed this"
/// from "something else left the mask this way", and a gate that cannot tell those apart is one the
/// next refactor makes vacuous.
///
/// **`project` is written in terms of this rather than beside it.** Two loops that must agree is the
/// shape this campaign has already paid for once — the fix for two values obliged to match is not to
/// check them, it is to have ONE. A gate reading a second, parallel implementation of the projection
/// would be green while the shipped one was wrong.
#[must_use]
pub fn projected_bits(world: &mut EcsMaster) -> u64 {
    let mut bits = 0u64;
    for scope in world
        .query::<&ProfilingScope, Enabled<ProfilingScopeEnabled>>()
        .iter()
    {
        bits |= scope.arm_bit();
    }
    bits
}

/// The published lag table (D25) — what a `Res<Profiler>` reader is looking at, in frames.
///
/// # Why a table and not a comment
///
/// The lag is **structural**, not incidental: the fold folds closed frames, so the freshest
/// complete frame is always the one before the live one. A reader driving dynamic resolution off a
/// windowed median wants that stated as a value it can read, not as a sentence in a doc it may not
/// have read. S1 forbids printing it; it is a datum.
///
/// # What this table does NOT carry, and why that is not an omission
///
/// D25's table has three rows: CPU at `N−1`, **GPU at `N−4 … N−2`**, and lifetime/histogram at
/// `N−1`. Only the first exists here.
///
/// * **The GPU row is not this store's to publish.** MEASURED at rung 11: nothing outside
///   `boyko_ecs` calls `boyko_diag::sample::push` (zero hits across `boyko_app`, `boyko_render` and
///   `boyko_rhi_vulkan`), and `Profiler::arm` has no non-test caller. The host's GPU channel folds
///   into the artifact reducer, not into this `Profiler` — so a GPU row here would describe a lag
///   this store's data cannot have, computed from `GPU_RING_DEPTH` and `RETIRE_GRACE_FRAMES`, two
///   constants in a crate this one does not depend on and must not.
/// * **The lifetime row arrives with rung 12**, which is what builds the accumulators. A field that
///   is structurally always the same value is indistinguishable from a measurement of that value,
///   and a reader cannot tell the difference — the discipline this module group has applied since
///   rung 2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LatencyTable {
    /// The live frame's absolute number, so a reader can turn the offset below into a frame.
    pub live_frame: u32,
    /// Frames behind [`live_frame`](Self::live_frame) that CPU spans, counters and gauges are
    /// complete through. `1`, by A2's live-frame cut.
    pub cpu_frames_behind: u32,
}

// ── registration ────────────────────────────────────────────────────────────────────────────────
//
// `boyko_macros` is a DEV-dependency of `boyko_ecs` (Cargo.toml), so `#[derive(Component)]` is
// unavailable to library `src/` code. These three impls are the hand-written mirror of exactly what
// the derive emits, on `ChildOf`'s precedent (`hierarchy/mod.rs:306`) — the difference for
// `ProfilingScopeEnabled` being `STORAGE_IS_BITSET` plus the `install_storage_kind` call that
// classifies the minted id, which is the pair `#[component(storage = "bitset")]` expands to.

impl Component for ProfilingScope {
    #[inline]
    fn component_id() -> ComponentId {
        static ID: OnceLock<ComponentId> = OnceLock::new();
        *ID.get_or_init(|| ComponentId(component_registry::register_new::<Self>()))
    }
}

impl Component for ProfiledZone {
    #[inline]
    fn component_id() -> ComponentId {
        static ID: OnceLock<ComponentId> = OnceLock::new();
        *ID.get_or_init(|| ComponentId(component_registry::register_new::<Self>()))
    }
}

impl Component for ProfilingScopeEnabled {
    #[inline]
    fn component_id() -> ComponentId {
        static ID: OnceLock<ComponentId> = OnceLock::new();
        *ID.get_or_init(|| {
            let raw = component_registry::register_new::<Self>();
            // The derive's `storage = "bitset"` arm, hand-mirrored: without this the id is minted
            // as ordinary table storage, `archetype.enable_store.column(tag)` answers `None`
            // forever, and every `is_enabled` reads `false` — B2's silent all-zero mask.
            component_registry::install_storage_kind::<Self>(raw);
            ComponentId(raw)
        })
    }

    const STORAGE_IS_BITSET: bool = true;
}

impl_self_bundle!(ProfilingScope);
impl_self_bundle!(ProfiledZone);

// A bitset tag gets NO `Bundle` impl — the derive suppresses it for `storage = "bitset"`, because a
// tag has no `ComponentPool` to insert into. It is toggled out of band, through `enable`/`disable`.
