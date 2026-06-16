use std::any::TypeId;
use crate::ecs::core::component::component_registry::{
    CloneFn, Cloneability, DeserializeFn, LoadMapEntitiesFn, RequiredDirectEntry, Serializability,
    SerializeFn,
};
use crate::ecs::core::component::hooks::ComponentHooks;
use crate::ecs::identifiers::primitives::ComponentId;

/// Marker trait for ECS component types.
///
/// Implemented automatically via `#[derive(Component)]`. Each type gets a
/// unique [`ComponentId`] assigned on first call to [`component_id`] — see
/// the [`component_registry`] module docs for the startup warm-up contract
/// and collision-detection semantics.
///
/// # Panic safety
///
/// `<Self as Drop>::drop` (if any) **must not panic**. If a panic escapes
/// `Component::drop` during `ComponentPool::Drop` (executed during archetype
/// or `EcsMaster` teardown), and any other panic is already in progress,
/// the process aborts. This matches `Vec<T>`'s policy for its element type.
///
/// Concretely:
/// - Do not call user code that may panic from a `Drop` impl on a `Component`.
/// - Do not use `expect` / `unwrap` / `assert!` in `Drop`.
/// - Prefer owning heap data via `Vec` / `Box` / `Arc`, which have
///   well-defined non-panicking drop semantics.
///
/// `ComponentPool::Drop` is NOT wrapped in `catch_unwind`; the overhead
/// (~20-30 ns per element) is unacceptable on a teardown path. The escape
/// hatch for users who cannot guarantee panic-free `Drop`: wrap the entire
/// `EcsMaster` lifetime in `std::panic::catch_unwind` at the application
/// boundary, and on catch, discard the master and never touch it again.
///
/// [`component_id`]: Component::component_id
/// [`component_registry`]: crate::ecs::core::component::component_registry
pub trait Component: 'static + Sized {
    /// Returns the unique identifier for this component type.
    ///
    /// The first call mints the ID via the global registry; subsequent calls
    /// return the cached value from a per-type `OnceLock` — no atomic on the
    /// hot path after initialization.
    fn component_id() -> ComponentId;

    /// Phase 14a (plan §6.2) — compile-time elision flag. `false` by default,
    /// so components without a `#[component(...)]` attribute pay zero. Enables
    /// `if const { C::HAS_HOOKS }` short-circuits in monomorphic typed paths
    /// (a secondary layer; the runtime `ArchetypeFlags` is the load-bearing
    /// gate). A backward-compatible widening — every existing impl keeps
    /// `HAS_HOOKS = false`.
    const HAS_HOOKS: bool = false;

    /// EnableTag D4 — compile-time storage discriminator. `false` by default
    /// (table/signature storage, the only backend before EnableTag), so every
    /// existing `Component` impl keeps the default and pays zero. The
    /// `#[component(storage = "bitset")]` derive (Wave 5) overrides it to
    /// `true` for enable-bit tags.
    ///
    /// It feeds the `Added<C>` / `Changed<C>` per-monomorphization const-asserts
    /// (`filter::Added::assert_storage_supports_change_detection`): a bitset
    /// enable tag has NO per-row tick storage, so change detection on it is
    /// meaningless and is compile-rejected rather than silently compiling to a
    /// lie (the Phase-22 D1 "compile-but-lie" lesson). A backward-compatible
    /// widening — purely a compile-time const, zero ABI break, zero runtime cost.
    const STORAGE_IS_BITSET: bool = false;

    /// Required components (Feature 1) — compile-time elision flag. `false` by
    /// default, so components without a `#[require(...)]` attribute pay zero.
    /// Enables `if const { C::HAS_REQUIRES }` short-circuits and gates the
    /// derive-emitted `install_required::<Self>` call (derive XOR runtime, like
    /// `HAS_HOOKS`). A backward-compatible widening — every existing impl keeps
    /// `HAS_REQUIRES = false`.
    const HAS_REQUIRES: bool = false;

    /// Entity cloning (Feature 3, D2) — compile-time clone classification.
    /// Defaults to [`Cloneability::Ignore`] (no clone-fn installed) so EVERY
    /// existing / hand-written impl keeps the default and the component is
    /// non-cloneable until it opts in. CRUCIALLY this needs NO `Clone` bound on
    /// `Component` (existing non-`Clone` components keep compiling — the 0%-gate).
    ///
    /// The `#[derive(Component)]` macro overrides it: `TriviallyCopyable` for a
    /// `Copy`-no-`Entity`-field type, `CloneViaFn` for a `Clone` (owning, or
    /// `Copy`-with-`Entity`) type, `Ignore` for `#[component(no_clone)]` or a
    /// non-`Clone` type. A backward-compatible widening — purely a compile-time
    /// const, zero ABI break, zero runtime cost on the hot path.
    const CLONE_BEHAVIOR: Cloneability = Cloneability::Ignore;

    /// Entity cloning (Feature 3, D2) — the per-component clone glue fn-ptr.
    /// Defaulted to `None` (consistent with the `Ignore` const above); the derive
    /// overrides it with `Some(clone_via_clone::<Self>)` for a `CloneViaFn` type.
    /// Per O2 a `TriviallyCopyable` type ALSO keeps the `None` default — the
    /// whole-column batch memcpy is driven by the pool layout, never this pointer.
    ///
    /// Returning the fn-ptr (rather than BEING the fn) lets `install_clone_fn` read
    /// it generically into the cold `CLONE` table at registration time. Called once
    /// per type per process (behind the `component_id()` `OnceLock`); never on a
    /// per-frame path. A backward-compatible widening — every existing impl keeps
    /// the `None` default.
    #[inline]
    fn clone_fn() -> Option<CloneFn> {
        None
    }

    /// Entity cloning (Feature 3) — the per-component clone classification as a
    /// METHOD (vs the [`Self::CLONE_BEHAVIOR`] const), so the derive can compute it
    /// via autoref specialization (a `const` cannot run autoref method resolution).
    /// Defaults to the const (so hand-written impls that set only `CLONE_BEHAVIOR` —
    /// like `ChildOf` — keep working). The derive overrides BOTH this and
    /// [`Self::clone_fn`] from the same autoref probe. Read once at registration by
    /// `install_clone_fn`; never on a per-frame path.
    #[inline]
    fn clone_behavior() -> Cloneability {
        Self::CLONE_BEHAVIOR
    }

    /// Serialization (Phase S0, plan §3.7) — compile-time serialization
    /// classification. Defaults to [`Serializability::Ignore`] (no serialize fns
    /// installed) so EVERY existing / hand-written impl keeps the default and the
    /// component is non-serializable until it opts in. Needs NO `Clone` / `Copy`
    /// bound on `Component` (existing impls keep compiling — the 0%-gate).
    ///
    /// The `#[derive(Component)]` macro overrides it via a STRICTER autoref probe
    /// than clone: `PlainOldBytes` for a `#[repr(C/transparent)]` type whose every
    /// field is all-bits-valid (no `bool`/`char`/enum/niche/`Entity`),
    /// `SerializeViaFn` for any other `Clone` type, `Ignore` for
    /// `#[component(no_serialize)]` or a non-`Clone` type. A backward-compatible
    /// widening — purely a compile-time const, zero ABI break, zero hot-path cost.
    const SERIALIZABILITY: Serializability = Serializability::Ignore;

    /// Serialization (Phase S0, plan §3.5) — per-component on-disk format version,
    /// the human-facing layout/semantic-change gate. Default `0`; the derive
    /// overrides it from `#[component(format_version = N)]`. Read once at
    /// registration; never on a per-frame path.
    const FORMAT_VERSION: u16 = 0;

    /// Serialization (Phase S0, plan §3.6 / C2) — the derive-computed blit-validity
    /// fingerprint: a best-effort hash of `(size, align, repr, per-field offsets,
    /// field_count)`. Default `0` (hand-written / non-serializable impls); the
    /// derive overrides it. Guards "the struct changed shape since the save";
    /// `FORMAT_VERSION` is the human-facing gate. Read once at registration.
    const LAYOUT_FINGERPRINT: u64 = 0;

    /// Serialization (Phase S0, plan §3.5 / C1) — the stable serialization key.
    /// Defaults to the fully-qualified type name ([`std::any::type_name`]);
    /// the derive overrides it from `#[component(stable_name = "...")]`. The
    /// on-disk type key — `ComponentId` is process-unstable, the name is stable.
    /// Read once at registration (the [`register_stable_name`] index key).
    ///
    /// A METHOD, not a const: `std::any::type_name` is not yet stable as a
    /// `const fn` (same reason `debug_type_name` is a method). The compiler still
    /// folds the call to a string literal at codegen, so there is no overhead.
    ///
    /// [`register_stable_name`]: crate::ecs::core::component::component_registry::register_stable_name
    #[inline]
    fn stable_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Serialization (Phase S0) — the per-component serialize glue fn-ptr.
    /// Defaulted to `None` (consistent with the `Ignore` const); the derive
    /// overrides it with `Some(..)` for a `SerializeViaFn` type. A `PlainOldBytes`
    /// type ALSO keeps the `None` default — the whole-column blit is driven by the
    /// pool layout, never this pointer. Read once at registration by
    /// `install_serialize_fn`; never on a per-frame path. (S0: the derive returns
    /// `None` for now — the encode glue lands in S1; the table shape is pinned.)
    #[inline]
    fn serialize_fn() -> Option<SerializeFn> {
        None
    }

    /// Serialization (Phase S0) — the per-component deserialize glue fn-ptr.
    /// Defaulted to `None`; the derive overrides it for a `SerializeViaFn` type.
    /// (S0: `None` — see [`Self::serialize_fn`].)
    #[inline]
    fn deserialize_fn() -> Option<DeserializeFn> {
        None
    }

    /// Serialization (Phase S0, C4) — the load-direction entity-remap fn-ptr.
    /// Defaulted to `None`; installed ONLY for entity-bearing components (v1:
    /// `ChildOf` + explicit `#[entities]` fields). (S0: `None` — the hand-written
    /// `ChildOf` remap lands with the loader.)
    #[inline]
    fn map_entities_fn() -> Option<LoadMapEntitiesFn> {
        None
    }

    /// Serialization (Phase S0) — the serialization classification as a METHOD
    /// (vs the [`Self::SERIALIZABILITY`] const), so the derive can compute it via
    /// the autoref `SerializeProbe` (a const cannot run autoref). Defaults to the
    /// const (so hand-written impls that set only `SERIALIZABILITY` keep working).
    /// The derive overrides this from the same autoref probe. Read once at
    /// registration by `install_serialize_fn`; never on a per-frame path.
    #[inline]
    fn serializability_runtime() -> Serializability {
        Self::SERIALIZABILITY
    }

    /// Phase 14a (plan §6.2) — installs this component's lifecycle hooks into
    /// `hooks`. Defaulted empty; the `#[derive(Component)]` attribute and the
    /// runtime builder (Wave 5) override it. Called once at registration time
    /// (`install_hooks::<Self>`), before the component can appear in any
    /// archetype. A backward-compatible widening — every existing impl keeps
    /// the empty default.
    #[inline]
    fn register_hooks(_hooks: &mut ComponentHooks) {}

    /// Required components (Feature 1) — declares this component's DIRECT
    /// `#[require(...)]` edges into `builder`. Defaulted empty; the
    /// `#[derive(Component)]` `#[require(...)]` attribute overrides it. Called
    /// once at registration time (`install_required::<Self>`), before the
    /// component can appear in any archetype. A backward-compatible widening —
    /// every existing impl keeps the empty default.
    #[inline]
    fn register_required(_builder: &mut RequiredBuilder) {}

    #[inline]
    fn debug_type_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    #[inline]
    fn type_id() -> TypeId {
        TypeId::of::<Self>()
    }

    #[inline]
    fn mem_size() -> usize {
        std::mem::size_of::<Self>()
    }

    #[inline]
    fn alignment() -> usize {
        std::mem::align_of::<Self>()
    }
}

/// Collects a component's DIRECT `#[require(...)]` declarations at registration
/// time (Feature 1). The derive-generated [`Component::register_required`] body
/// calls [`RequiredBuilder::require`] once per `#[require]` key; the registry's
/// `install_required` then leaks the accumulated entries into the cold
/// `REQUIRES_DIRECT` table.
///
/// Duplicate same-id `#[require]` keys are rejected at COMPILE time by the
/// derive macro (the macro sees both paths), so this runtime builder performs no
/// dedup — it is a thin push-only accumulator.
#[derive(Default)]
pub struct RequiredBuilder {
    entries: Vec<RequiredDirectEntry>,
}

impl RequiredBuilder {
    /// Creates an empty builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Records one DIRECT required edge: the required component's id resolver
    /// `id_fn` (stored UNCALLED) and the capture-free `ctor`. Called once per
    /// `#[require]` key by the derive-generated `register_required`.
    ///
    /// BUG-REQ-CYCLE-1: `id_fn` is the required type's `component_id` passed as a
    /// fn item WITHOUT parentheses (`B::component_id`, not `B::component_id()`).
    /// It is invoked lazily at archetype-expansion time in `build_required_plan`,
    /// NOT during the requiring type's own `component_id()` `OnceLock` init —
    /// otherwise a `#[require]` cycle would re-enter that mid-init `OnceLock` on
    /// the same thread and deadlock.
    #[inline]
    pub fn require(
        &mut self,
        id_fn: crate::ecs::core::component::component_registry::RequiredIdFn,
        ctor: crate::ecs::core::component::component_registry::RequiredCtor,
    ) {
        self.entries.push(RequiredDirectEntry { id_fn, ctor });
    }

    /// Consumes the builder, returning the accumulated entries as a boxed slice
    /// ready to leak into the `REQUIRES_DIRECT` table.
    #[inline]
    pub fn into_entries(self) -> Box<[RequiredDirectEntry]> {
        self.entries.into_boxed_slice()
    }
}
