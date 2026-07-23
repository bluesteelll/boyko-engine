//! Serialization & data-binding sub-registry (Phase S0 / GUI P4).
//!
//! Split out of the former single-file `component_registry` (pure mechanical
//! move — every item keeps its exact `component_registry::…` path via the
//! `pub use serialize::*` re-export in the parent `mod.rs`). Holds the
//! per-component serialize metadata (`SERIALIZE`), the data-binding accessor
//! table (`BIND_ACCESSORS`), the stable-name → id index (`STABLE_NAME_INDEX`),
//! the `SerPod` / autoref serialize-classification probes, and the `WireBridge`
//! encode/decode glue. Reaches into the core registry (parent module) only for
//! the shared `MAX_COMPONENTS` bound.

// Registration-time stable-name → id index (`STABLE_NAME_INDEX` below): written
// once per component type from the derive's `component_id()` closure and read
// once per file-local type at save/load by `boyko_serialize`. Never on the
// per-frame spawn/iter/schedule path — the hot 56 B `ComponentLayout` record
// stays untouched (TRIPWIRE 2), the metadata lives in this cold parallel table.
#[allow(clippy::disallowed_types)]
use std::collections::HashMap;
#[allow(clippy::disallowed_types)]
use std::sync::{Mutex, OnceLock};

use crate::ecs::core::component::component::Component;

use super::MAX_COMPONENTS;

// ═════════════════════════════════════════════════════════════════════════════
// Serialization (Phase S0 — registry substrate). Spec: `docs/SERIALIZATION-PLAN.md`
// §3.7 (data structures) + §5 C1–C3 + §7 Phase S0. One cold parallel table (mirror
// of `CLONE`) + a NEW stable-name → id index (C1). Touched ONLY at registration
// time (write-once `install_serialize_fn` / `register_stable_name`, one cold
// `OnceLock::set` + one `Mutex` insert per type) and from the future
// `boyko_serialize` crate (save/load — never on the per-frame spawn/iter/schedule
// path). 0%-gate (grep-proof obligation): the `get_serialize_info` /
// `resolve_stable_name` / `STABLE_NAME_INDEX` readers ⊆ `boyko_serialize`. The 56 B
// `ComponentLayout` hot record stays pinned (TRIPWIRE 2): the metadata lives in the
// parallel cold `SERIALIZE` table and the separate `STABLE_NAME_INDEX` static.
// ═════════════════════════════════════════════════════════════════════════════

/// Serialize one component instance: read the live value at `src`, append
/// position-independent bytes into `sink` (plan §3.7). A bare
/// `unsafe fn(*const u8, &mut SaveCursor)` (mirror of [`CloneFn`](super::CloneFn)) — no
/// `Box<dyn>`, no `Arc<dyn Fn>`. Installed ONLY for the
/// [`Serializability::SerializeViaFn`] encode path; a
/// [`Serializability::PlainOldBytes`] component installs `None` and is blitted
/// whole-column from the pool layout, never through this pointer (the POB fast
/// path).
///
/// # Safety (caller-guaranteed at the single save call site, S1)
/// - `src` points at a live, initialized value of THIS `ComponentId`'s type `C`,
///   aligned to `align_of::<C>()`, readable for `size_of::<C>()` bytes.
/// - `sink` is a valid, append-only cursor; the fn only appends and never reads
///   back prior bytes.
pub type SerializeFn =
    unsafe fn(src: *const u8, sink: &mut crate::ecs::core::serialize::SaveCursor<'_>);

/// Deserialize one component instance from `src` into the UNINITIALIZED `dst`
/// (one `ptr::write`, no drop of prior contents — plan §3.7). Returns `Err` on a
/// malformed stream (the caller rolls back; `dst` is left uninit — the W5
/// partial-row contract, mirroring [`CloneFn`](super::CloneFn)). Entity fields are written with
/// their SAVED ids; the separate [`LoadMapEntitiesFn`] pass remaps them.
///
/// # Safety (caller-guaranteed at the single load call site, S2)
/// - `dst` points at writable, **uninitialized** space of `>= size_of::<C>()`
///   bytes, aligned to `align_of::<C>()`.
/// - On a normal `Ok` return `dst` holds an initialized `C` written exactly once;
///   on `Err` (or a panic) `dst` is left uninitialized and the caller's rollback
///   guard must NOT drop it.
pub type DeserializeFn = unsafe fn(
    src: &mut crate::ecs::core::serialize::LoadCursor<'_>,
    dst: *mut u8,
) -> Result<(), crate::ecs::core::serialize::DecodeError>;

/// Load-direction entity remap (saved id → freshly-allocated `Entity`), plan
/// §3.7 / C4. Installed ONLY for entity-bearing components (v1: `ChildOf` and
/// explicit `#[entities]` fields); every other id leaves its slot unset, so the
/// remap pass touches only the annotated components.
///
/// Rewrites every remappable `Entity` field of the value at `dst` in place,
/// translating each SAVED id to the freshly-allocated `Entity` via `map`. Returns
/// [`DecodeError::UnmappedEntity`](crate::ecs::core::serialize::DecodeError::UnmappedEntity)
/// when a referenced saved id is absent from `map` (a dangling reference — the C4
/// loud-error path; never silently kept as a stale id).
///
/// # Safety (caller-guaranteed at the load remap call site, S2.5)
/// - `dst` points at a live, initialized value of THIS `ComponentId`'s type.
/// - `map` outlives the call and is not aliased mutably.
pub type LoadMapEntitiesFn = unsafe fn(
    dst: *mut u8,
    map: &crate::ecs::core::serialize::LoadEntityMap,
) -> Result<(), crate::ecs::core::serialize::DecodeError>;

/// Per-component serialization classification (plan §3.7 / C3). STRICTER than the
/// clone [`Cloneability`](super::Cloneability): serialization ingests **untrusted bytes**, so the
/// blittable [`PlainOldBytes`](Serializability::PlainOldBytes) class additionally
/// requires every field to have an all-bits-valid representation. Drives the
/// blit-vs-fn-ptr-vs-skip branch on its own.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Serializability {
    /// `#[repr(C)]`/`#[repr(transparent)]` AND every field transitively in
    /// `{integers, floats, raw pointers}` — NO `bool`, `char`, enum, niche type,
    /// or `Entity`. `serialize_fn` / `deserialize_fn` are `None`; the whole column
    /// is blitted with one `copy_nonoverlapping` (the POB fast path). Strictly
    /// narrower than `Cloneability::TriviallyCopyable` (which only needs `Copy`):
    /// a `Copy` type with a `bool`/`char`/enum/niche field is NOT POB because
    /// those bytes are not all-bits-valid on an untrusted load (C3).
    PlainOldBytes = 0,
    /// Owning (`String`/`Vec`/heap) OR bit-restricted (`bool`/`char`/enum/niche)
    /// OR entity-bearing — must run the per-element `serialize_fn` /
    /// `deserialize_fn` (length-prefixed, position-independent, validates on
    /// read). The decode path validates every bit-restricted field, making the
    /// copy path sound on untrusted bytes (C3).
    SerializeViaFn = 1,
    /// Not serializable (not `Clone`, or `#[component(no_serialize)]`) —
    /// `serialize_fn` / `deserialize_fn` are `None`. The saver skips it; the
    /// loader default-constructs (via the `REQUIRES` ctor) or rejects in strict
    /// mode. The backward-compatible default for every non-opted-in component.
    Ignore = 2,
}

/// Cold per-component serialization metadata (plan §3.7). Lives in the parallel
/// `SERIALIZE` table, NOT in `ComponentLayout` (keeps TRIPWIRE 2's 56 B).
/// `Copy + Send + Sync` (fn-ptrs + POD + `&'static str`), like [`CloneInfo`](super::CloneInfo).
///
/// Per plan O1, the exact size is NOT load-bearing (this is a cold record), so —
/// unlike [`CloneInfo`](super::CloneInfo)'s asserted 16 B — there is no `const_assert` on this type.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SerializeInfo {
    /// `Some(serialize_via_serde::<C>)` ONLY for [`Serializability::SerializeViaFn`];
    /// `None` for [`Serializability::PlainOldBytes`] (blit path) and
    /// [`Serializability::Ignore`]. (S0 installs `None` everywhere — the encode
    /// glue lands in S1; the classification + table shape are pinned now.)
    pub serialize_fn: Option<SerializeFn>,
    /// `Some(deserialize_via_serde::<C>)` ONLY for
    /// [`Serializability::SerializeViaFn`]; `None` otherwise. (S0: `None` — see
    /// `serialize_fn`.)
    pub deserialize_fn: Option<DeserializeFn>,
    /// `Some(remap)` ONLY for entity-bearing components (v1: `ChildOf` + explicit
    /// `#[entities]`); `None` otherwise (C4). (S0: `None` — the hand-written
    /// `ChildOf` remap install lands with the loader.)
    pub map_entities_fn: Option<LoadMapEntitiesFn>,
    /// The branch discriminator. Drives blit-vs-fn-ptr-vs-skip on its own.
    pub serializability: Serializability,
    /// User-bumpable on any intentional layout/semantic change — the human-facing
    /// version gate (plan §3.5 / C2). Default `0`.
    pub format_version: u16,
    /// Derive-computed blit-validity guard (plan §3.6 / C2): a best-effort hash of
    /// `(size, align, repr, per-field offsets, field_count)`. Guards "the struct
    /// changed shape since the save"; `format_version` is the human-facing gate.
    pub layout_fingerprint: u64,
    /// The stable serialization key (plan §3.5 / C1): default = the fully-qualified
    /// type name, overridable via `#[component(stable_name = "...")]`. The on-disk
    /// type key — `ComponentId` is process-unstable, the name is the stable option.
    pub stable_name: &'static str,
    /// The 64-bit hash of [`Self::stable_name`], the `STABLE_NAME_INDEX` key.
    pub stable_name_hash: u64,
}

/// Phase S0 — parallel cold table of per-component serialization metadata (plan
/// §3.7). Touched ONLY at registration time (write-once `install_serialize_fn`)
/// and from the future `boyko_serialize` save/load path — never on
/// spawn/iter/schedule. Mirrors the [`CLONE`] declaration exactly.
static SERIALIZE: [OnceLock<SerializeInfo>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Returns the registered serialization metadata for `component_id`, or `None`
/// when no metadata was installed (a hand-written impl that never opted in).
///
/// Cold: read ONLY from the future `boyko_serialize` crate — never on the
/// per-frame hot path (the 0%-gate). One acquire-load + branch, mirroring
/// [`get_clone_info`](super::get_clone_info).
#[inline]
pub fn get_serialize_info(component_id: usize) -> Option<&'static SerializeInfo> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    SERIALIZE[component_id].get()
}

/// Installs `C`'s serialization metadata into `SERIALIZE[component_id]` (Phase
/// S0). Builds a [`SerializeInfo`] from the type's compile-time serialization
/// consts ([`Component::SERIALIZABILITY`], [`Component::FORMAT_VERSION`],
/// [`Component::LAYOUT_FINGERPRINT`]) + methods ([`Component::stable_name`] /
/// [`Component::serializability_runtime`] / [`Component::serialize_fn`] /
/// [`Component::deserialize_fn`] / [`Component::map_entities_fn`]) and writes it
/// once via `OnceLock::set`, mirroring [`install_clone_fn`](super::install_clone_fn).
///
/// **PUBLIC** (the derive expands into downstream crates where `pub(crate)` is
/// unreachable — the same rationale as [`install_clone_fn`](super::install_clone_fn)).
/// Called from the
/// derive's `component_id()` closure **UNGATED** (like `install_clone_fn`): the
/// 0%-gate is preserved because the write is one cold `OnceLock::set` per type per
/// process, behind the existing `component_id()` `OnceLock`, and never on a
/// per-frame path. Ungating it means the serialize path never has to special-case
/// a missing entry — a plain non-serializable component installs
/// `Serializability::Ignore` with `None` fns (the trait defaults).
#[inline]
pub fn install_serialize_fn<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let stable_name = C::stable_name();
    // Read the METHOD (not the const): the derive overrides
    // `serializability_runtime()` with the autoref-probe result (a const cannot run
    // autoref). Hand-written impls default the method to their `SERIALIZABILITY`
    // const.
    let serializability = C::serializability_runtime();
    // Gate the encode/decode fn-ptrs on the RUNTIME classification — the single
    // source of truth for "POB installs None" (plan §3.7). The derive's
    // `serialize_fn()` / `deserialize_fn()` return `Some(glue)` whenever every field
    // is `Wire` (it does NOT see the runtime POB/ViaFn split), so a genuinely
    // `PlainOldBytes` component (all SerPod primitives, which are also all `Wire`)
    // would otherwise install a live encoder. Only `SerializeViaFn` keeps the
    // `Some`; `PlainOldBytes` (blit path) and `Ignore` drop to `None`.
    let is_via_fn = serializability == Serializability::SerializeViaFn;
    let (serialize_fn, deserialize_fn) = if is_via_fn {
        (C::serialize_fn(), C::deserialize_fn())
    } else {
        (None, None)
    };
    let info = SerializeInfo {
        serialize_fn,
        deserialize_fn,
        map_entities_fn: C::map_entities_fn(),
        serializability,
        format_version: C::FORMAT_VERSION,
        layout_fingerprint: C::LAYOUT_FINGERPRINT,
        stable_name,
        stable_name_hash: fnv1a_64(stable_name.as_bytes()),
    };
    // Write-once; a same-id re-install is a silent no-op (first writer wins).
    let _ = SERIALIZE[component_id].set(info);
}

/// Type-erased per-component data-binding accessor (GUI P4 Decision 7).
///
/// Installed once per `#[derive(Bindable)]` type into the parallel
/// [`BIND_ACCESSORS`] table and read ONLY off the change-gated UI bind path
/// (`boyko_ui::binding::ui_bind_apply`) — never on a still frame, never on the
/// per-frame hot path. Mirrors [`SerializeInfo`]'s "cold parallel table"
/// discipline.
///
/// Both function pointers take a `*const u8` obtained by the caller from
/// `EcsMaster::get_component_raw(source, component_id)`, which returns `Some`
/// only when `source` is alive AND its archetype hosts that exact
/// `ComponentId`. Because a `ComponentId` *is* the type's identity, the bytes at
/// the pointer are a live, aligned instance of the registered type — the
/// trampolines' `// SAFETY:` precondition (no `TypeId` / `Any` check needed).
///
/// `Copy + Send + Sync` (two `fn` pointers), so the `[OnceLock<BindAccessor>;
/// MAX_COMPONENTS]` table is `Send + Sync` exactly like [`SERIALIZE`].
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct BindAccessor {
    /// Formats `field` of the component at `*const u8` into the sink. The caller
    /// guarantees the pointer is a live, aligned row of the registered type.
    pub fmt: fn(*const u8, u8, &mut dyn core::fmt::Write) -> core::fmt::Result,
    /// Returns `field` of the component at `*const u8` as an `f32`. The caller
    /// guarantees the pointer is a live, aligned row of the registered type.
    pub value: fn(*const u8, u8) -> f32,
}

/// GUI P4 — parallel cold table of per-component data-binding accessors
/// (Decision 7). Touched ONLY at registration time (write-once
/// [`install_bind_accessor`]) and from the change-gated `boyko_ui` bind-apply
/// path — never on spawn/iter/schedule or a still frame. Mirrors the
/// [`SERIALIZE`] declaration exactly.
static BIND_ACCESSORS: [OnceLock<BindAccessor>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Returns the registered [`BindAccessor`] for `component_id`, or `None` when no
/// accessor was installed (a component without `#[derive(Bindable)]`).
///
/// Cold: read ONLY from the change-gated `boyko_ui` bind-apply path — never on a
/// still frame or the per-frame hot path. One acquire-load + branch, mirroring
/// [`get_serialize_info`].
#[inline]
pub fn get_bind_accessor(component_id: usize) -> Option<&'static BindAccessor> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    BIND_ACCESSORS[component_id].get()
}

/// Installs `acc` into `BIND_ACCESSORS[component_id]` (GUI P4 Decision 7).
///
/// **PUBLIC** so the `#[derive(Bindable)]` expansion (which lives in downstream
/// crates where `pub(crate)` is unreachable) can call it — the same rationale as
/// [`install_serialize_fn`]. Write-once via `OnceLock::set`; a same-id
/// re-install is a silent no-op (first writer wins), so calling it ungated from
/// the derive's registration closure is safe and 0%-gate-preserving (one cold
/// `OnceLock::set` per type per process).
#[inline]
pub fn install_bind_accessor(component_id: usize, acc: BindAccessor) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    if component_id >= MAX_COMPONENTS {
        return;
    }
    let _ = BIND_ACCESSORS[component_id].set(acc);
}

/// 64-bit FNV-1a hash of a byte string (the `STABLE_NAME_INDEX` keying, C1).
///
/// A `const fn` so the derive could fold it at compile time and so it is reusable
/// by the future `boyko_serialize` file-key path. FNV-1a is chosen over a
/// heavier hash because the index is COLD (registration + once-per-load-type
/// only) and collisions are explicitly disambiguated by a full-name compare in
/// [`resolve_stable_name`] — the hash only buckets candidates.
#[inline]
pub const fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// C1 — process-global stable-name → `ComponentId` index. Does NOT exist before
/// S0 (`TAG_NAMES` interns only dynamic-tag names; derived components are keyed by
/// `TypeId` in `ComponentLayout`, with no name→id reverse index). Built fresh
/// here.
///
/// Keyed by the stable-name HASH (never `TypeId` — two builds of "the same"
/// component differ in `TypeId` but must resolve to the same stable name).
/// Collisions are disambiguated by comparing the full `stable_name` string on a
/// hash hit (see [`resolve_stable_name`]); the bucket value is the list of
/// candidate `ComponentId`s that hashed equal.
///
/// COLD: touched only at registration ([`register_stable_name`]) and once per
/// file-local type at load ([`resolve_stable_name`]) — never on the per-frame hot
/// path. `Mutex + HashMap` is justified per the `TAG_NAMES` precedent (one
/// concrete global, not a generic-fn-body static — sidesteps the
/// monomorphization-collapse trap). A `Vec<usize>` (not a `SmallVec`) is the
/// bucket: `boyko_utils` ships no small-vec, and per-hash candidate counts are
/// ~1 in practice, so the allocation is negligible on this cold path.
// Registration-time index (one concrete global, not a generic-fn-body static —
// the rust#22991 collapse trap); readers ⊆ `boyko_serialize` save/load.
#[allow(clippy::type_complexity, clippy::disallowed_types)]
static STABLE_NAME_INDEX: OnceLock<Mutex<HashMap<u64, Vec<usize>>>> = OnceLock::new();

/// Lazily initializes and returns the [`STABLE_NAME_INDEX`] table.
// Cold accessor for the registration-time index above; not a per-frame path.
#[allow(clippy::disallowed_types)]
fn stable_name_index() -> &'static Mutex<HashMap<u64, Vec<usize>>> {
    STABLE_NAME_INDEX.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers `C`'s stable name in the `STABLE_NAME_INDEX` (C1). Called from the
/// derive's `component_id()` closure UNGATED (like [`install_clone_fn`](super::install_clone_fn)),
/// once per
/// component per process. COLD — never a frame path.
///
/// The index maps `stable_name_hash → candidate ComponentIds`. A second
/// registration of the SAME id under the same hash is deduped (idempotent — the
/// `component_id()` `OnceLock` already guarantees one call per type, but a
/// hand-written impl calling twice stays correct). A DIFFERENT id sharing the hash
/// (a genuine collision, or two distinct types with the same stable name) is
/// appended; [`resolve_stable_name`] disambiguates by the full name.
pub fn register_stable_name<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let hash = fnv1a_64(C::stable_name().as_bytes());
    let mut index = stable_name_index()
        .lock()
        .expect("invariant: STABLE_NAME_INDEX lock poisoned only after a panic under the guard");
    let bucket = index.entry(hash).or_default();
    if !bucket.contains(&component_id) {
        bucket.push(component_id);
    }
}

/// Load-time resolution (C1): maps a saved `(stable_name_hash, stable_name)` to
/// the running process's `ComponentId`, or `None` if no registered component
/// carries that stable name. Called ONCE per file-local type at load (never per
/// entity). COLD.
///
/// On a hash hit it compares the FULL `stable_name` of each candidate to confirm —
/// a hash collision (or two distinct types hashing equal) is resolved by the
/// string compare, so only the component whose registered `stable_name` exactly
/// equals `name` is returned. `None` covers both "never registered" and "hash hit
/// but no full-name match".
pub fn resolve_stable_name(hash: u64, name: &str) -> Option<usize> {
    let index = stable_name_index()
        .lock()
        .expect("invariant: STABLE_NAME_INDEX lock poisoned only after a panic under the guard");
    let bucket = index.get(&hash)?;
    for &candidate_id in bucket {
        // Confirm the full name on the hash hit — disambiguates collisions.
        if let Some(info) = get_serialize_info(candidate_id)
            && info.stable_name == name
        {
            return Some(candidate_id);
        }
    }
    None
}

// ── Autoref serialize-classification probes (Phase S0, derive support) ──────────
//
// STRICTER than the clone `CloneProbe` (C3). The `#[derive(Component)]` macro
// cannot resolve a type to ask "is every field all-bits-valid?". These three
// zero-sized probe arms use AUTOREF SPECIALIZATION (the dtolnay rule, identical
// mechanism to `CloneProbe`) to pick the right `Serializability` at the type
// level, reflection-free — but the POB arm is gated on a NEW `Pod` marker
// (`SerPod`) that is implemented ONLY for the all-bits-valid primitives, NOT on
// the loose `C: Copy` the clone path uses. So a `Copy` type that contains a
// `bool`/`char`/enum/niche/`Entity` field does NOT satisfy `SerPod` and falls to
// the `SerializeViaFn` arm (whose `deserialize_fn` validates those fields on an
// untrusted read).
//
//   * `SerPobArm for &&SerializeProbe<C, true>` (gated `C: SerPod` AND
//     `POB_ELIGIBLE == true`) — the MOST-ref'd `Self`, HIGHEST priority. A
//     provably-all-bits-valid `#[repr(C/transparent)]` type with no
//     bool/char/enum/niche/Entity wins here → `PlainOldBytes`. The macro passes
//     `POB_ELIGIBLE == false` for a non-`repr(C)` type or one with an `Entity`
//     field, removing this arm as a candidate (a const mismatch, not a bound
//     failure — so the type cleanly falls through).
//   * `SerViaFnArm for &SerializeProbe<C, POB_ELIGIBLE>` (gated `C: Clone`) —
//     middle priority. A `Clone` type that is NOT provably-POB lands here →
//     `SerializeViaFn`.
//   * `SerIgnoreArm for SerializeProbe<C, POB_ELIGIBLE>` (no bound) — the
//     by-value `Self`, LOWEST priority. A non-`Clone` type reaches it → `Ignore`.
//
// The call site (derive codegen) invokes the probe through THREE refs
// (`(&&&probe).method()`), exactly like `CloneProbe`: the resolver selects the
// highest-priority APPLICABLE arm. The ref count MUST stay `&&&` to agree with the
// arm receiver depths below.

/// Sealed marker for the all-bits-valid primitive types that may appear in a
/// [`Serializability::PlainOldBytes`] component (C3). Implemented ONLY for
/// integers, floats, and raw pointers — every bit pattern of these is a valid
/// value, so loading one from untrusted bytes can never be UB. **Deliberately NOT
/// implemented** for `bool`, `char`, fieldless enums, or niche-optimized types
/// (`NonZeroU32`, `Option<NonNull<_>>`, …): those have invalid bit patterns, so a
/// corrupt byte would instantiate an invalid value.
///
/// The derive does NOT emit a per-struct `SerPod` impl (a conditional `impl SerPod
/// for Struct where Field: SerPod {}` is rejected by the compiler when a concrete
/// field is not `SerPod` — it eagerly evaluates the false bound rather than
/// silently dropping the impl). Instead the derive passes the struct's FIELD TUPLE
/// `(F0, F1, …)` as a TYPE PARAMETER of [`SerializeProbe`], and the
/// [`SerPobArm`] is gated `F: SerPodTuple` — a GENERIC bound on the probe arm,
/// which the autoref resolver can leave UN-matched (deferring, not erroring) when a
/// field is not `SerPod`, demoting the type to [`Serializability::SerializeViaFn`]
/// (C3). [`SerPodTuple`] proves "every element is `SerPod`" via generic tuple
/// impls, so a `bool`/`char`/enum/niche field fails the bound and the POB arm is
/// simply skipped.
///
/// # Safety
/// Implementing `SerPod` for a type asserts that EVERY bit pattern of
/// `size_of::<Self>()` bytes is a valid `Self`. The leaf impls below uphold this
/// for the language-primitive types; do NOT implement it for any type with a
/// validity invariant.
pub unsafe trait SerPod: 'static {}

// Leaf impls: all-bits-valid language primitives only.
//
// SAFETY (each): every bit pattern of these widths is a valid value of the type.
// Integers and floats have no invalid representations; a raw pointer may hold any
// address bits (validity of the POINTEE is not a property of the pointer value).
unsafe impl SerPod for u8 {}
unsafe impl SerPod for u16 {}
unsafe impl SerPod for u32 {}
unsafe impl SerPod for u64 {}
unsafe impl SerPod for u128 {}
unsafe impl SerPod for usize {}
unsafe impl SerPod for i8 {}
unsafe impl SerPod for i16 {}
unsafe impl SerPod for i32 {}
unsafe impl SerPod for i64 {}
unsafe impl SerPod for i128 {}
unsafe impl SerPod for isize {}
unsafe impl SerPod for f32 {}
unsafe impl SerPod for f64 {}
// SAFETY: a raw pointer value is any address-sized bit pattern; every such
// pattern is a valid `*const T` / `*mut T` (the pointer's VALIDITY as a
// dereferenceable address is a separate, runtime concern, not a type-validity
// invariant — so reading one from bytes is sound).
unsafe impl<T: 'static> SerPod for *const T {}
unsafe impl<T: 'static> SerPod for *mut T {}

// SAFETY: an array `[T; N]` has NO padding between or after its elements
// (`size_of::<[T; N]>() == N * size_of::<T>()`), so its bytes are exactly `N`
// contiguous `T` values laid end to end. If every bit pattern of `size_of::<T>()`
// bytes is a valid `T` (`T: SerPod`), then every bit pattern of
// `size_of::<[T; N]>()` bytes is a valid `[T; N]`. (`T: SerPod` already implies
// `T: 'static`, so `[T; N]: 'static` holds.) This is the standard `Pod`-for-arrays
// rule (cf. bytemuck/zerocopy `Pod for [T; N]`). Its absence silently demoted
// every component with an array field — `[f32; N]` transforms, vectors, the
// common case — from the whole-column `PlainOldBytes` blit to the per-row
// `SerializeViaFn` encode path (the C3 demotion), so adding it restores the fast
// path the POB design intended.
unsafe impl<T: SerPod, const N: usize> SerPod for [T; N] {}

/// "Every element of this tuple is [`SerPod`]" — the field-validity proof for the
/// [`SerPobArm`] (C3). Implemented by GENERIC tuple impls (arity 0..=16, the
/// realistic component field count), so the POB arm's `F: SerPodTuple` bound is a
/// bound on a probe TYPE PARAMETER the autoref resolver can leave un-matched
/// (deferring, not erroring) when an element is not `SerPod`. The derive passes the
/// struct's field tuple `(F0, F1, …)` as that parameter; a unit struct passes `()`
/// (vacuously `SerPodTuple` — a blittable ZST tag).
#[doc(hidden)]
pub trait SerPodTuple {}

macro_rules! impl_serpod_tuple {
    ($($name:ident),*) => {
        impl<$($name: SerPod),*> SerPodTuple for ($($name,)*) {}
    };
}

impl SerPodTuple for () {}
impl_serpod_tuple!(A);
impl_serpod_tuple!(A, B);
impl_serpod_tuple!(A, B, C);
impl_serpod_tuple!(A, B, C, D);
impl_serpod_tuple!(A, B, C, D, E);
impl_serpod_tuple!(A, B, C, D, E, F);
impl_serpod_tuple!(A, B, C, D, E, F, G);
impl_serpod_tuple!(A, B, C, D, E, F, G, H);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_serpod_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

/// Probe wrapper for autoref serialize classification (see module note). `Fields`
/// is the struct's field tuple `(F0, F1, …)` (the macro fills it; a unit struct
/// passes `()`); `POB_ELIGIBLE` is the macro's "`#[repr(C/transparent)]` AND no
/// `Entity` field" syntactic flag. The POB arm fires only when BOTH `POB_ELIGIBLE
/// == true` AND `Fields: SerPodTuple` (every field all-bits-valid) AND `C: Copy`;
/// otherwise the type falls to [`Serializability::SerializeViaFn`] / `Ignore`.
#[doc(hidden)]
pub struct SerializeProbe<C, Fields, const POB_ELIGIBLE: bool>(
    pub core::marker::PhantomData<(C, Fields)>,
);

impl<C, Fields, const POB_ELIGIBLE: bool> SerializeProbe<C, Fields, POB_ELIGIBLE> {
    /// Constructs the probe (called by derive-generated code only).
    #[doc(hidden)]
    #[inline]
    pub const fn new() -> Self {
        Self(core::marker::PhantomData)
    }
}

/// By-value-`Self` fallback arm (no bound): a non-`Clone` type is `Ignore`. The
/// LEAST-specific `Self` (fewest refs), LOWEST priority — wins only when neither
/// the POB arm nor the `Clone` arm applies.
#[doc(hidden)]
pub trait SerIgnoreArm {
    #[doc(hidden)]
    fn serializability(&self) -> Serializability;
}

impl<C, Fields, const POB_ELIGIBLE: bool> SerIgnoreArm
    for SerializeProbe<C, Fields, POB_ELIGIBLE>
{
    #[inline]
    fn serializability(&self) -> Serializability {
        Serializability::Ignore
    }
}

/// `&`-`Self` arm gated `C: Clone` (middle priority): a `Clone` type that is NOT
/// provably-POB is `SerializeViaFn`. More specific than the by-value `Ignore`
/// fallback, less specific than the `&&`-`Self` POB arm.
#[doc(hidden)]
pub trait SerViaFnArm {
    #[doc(hidden)]
    fn serializability(&self) -> Serializability;
}

impl<C: Clone + 'static, Fields, const POB_ELIGIBLE: bool> SerViaFnArm
    for &SerializeProbe<C, Fields, POB_ELIGIBLE>
{
    #[inline]
    fn serializability(&self) -> Serializability {
        Serializability::SerializeViaFn
    }
}

/// `&&`-`Self` arm gated `C: Copy`, `Fields: SerPodTuple`, AND `POB_ELIGIBLE ==
/// true` (the MOST specific `Self`, HIGHEST priority): a `#[repr(C/transparent)]`
/// `Copy` type with no `Entity` field and ALL fields all-bits-valid is
/// `PlainOldBytes`. Being most-specific it wins before the `&`-`Self` `Clone` arm
/// (`Copy ⊆ Clone`) can match; a type with a non-`SerPod` field fails the
/// `Fields: SerPodTuple` bound (the autoref resolver leaves this arm un-matched
/// and DEFERS to the `Clone` arm — it does NOT error, the C3 silent demotion), and
/// a non-repr-C / Entity-bearing type carries `POB_ELIGIBLE == false`.
///
/// `Copy` is required HERE (not as a `SerPod` supertrait): a POB column is
/// byte-copied whole, so the type must be `Copy`; `SerPod` stays a pure
/// all-bits-valid marker so the `Fields: SerPodTuple` field proof composes cleanly.
#[doc(hidden)]
pub trait SerPobArm {
    #[doc(hidden)]
    fn serializability(&self) -> Serializability;
}

impl<C: Copy + 'static, Fields: SerPodTuple> SerPobArm for &&SerializeProbe<C, Fields, true> {
    #[inline]
    fn serializability(&self) -> Serializability {
        Serializability::PlainOldBytes
    }
}

// ── Owning / bit-restricted encode glue (Phase S1.5, plan §3.1 / §3.7) ──────────
//
// The `SerializeViaFn` encode path runs a per-element `serialize_fn` /
// `deserialize_fn` that walks a component's fields through the `Wire` codec. The
// derive CANNOT emit a verbatim `field.wire_write(sink)` body unconditionally: that
// would impose `FieldTy: Wire` on EVERY derived component (a concrete inherent impl
// with an unsatisfiable bound is a hard `E0277`, not a silent skip — confirmed), so
// an existing `Clone` component with a non-`Wire` field (`Box<u32>`, `Rc<u32>`, …)
// would fail to compile. Instead the derive emits a thin, bound-free `WireBridge`
// (struct ↔ field-tuple) and the GENERIC glue below carries the `Wire` bound on the
// field tuple. The encode-fn autoref arm (`WireFnProbe`) selects the glue ptr ONLY
// when `C::Owned: WireTuple` holds (every field `Wire`) AND the type is not
// POB-eligible — otherwise it DEFERS to the `None` fallback (the graceful demotion
// the `SerPodTuple` POB gate already uses, mirroring the house style).

/// Struct ↔ field-tuple bridge the derive emits for a serializable component (plan
/// §3.7). Carries NO `Wire` bound (so it compiles for ANY plain struct, including
/// one with non-`Wire` fields, and — crucially — one that implements `Drop`) — the
/// `Wire` requirement lives on the generic [`serialize_via_wire`] /
/// [`deserialize_via_wire`] glue's `WireRefTuple` / `WireTuple` bounds, which the
/// encode-fn autoref arm checks and defers on.
///
/// The derive maps a component `struct C { f0: F0, f1: F1, … }` to:
/// - `Owned = (F0, F1, …)` and `from_owned(t) = C { f0: t.0, f1: t.1, … }` (the
///   §3.7 "fields in declaration order" read constructor);
/// - `Refs<'a> = (&'a F0, &'a F1, …)` and `as_refs(&self) = (&self.f0, …)` — a tuple
///   of **borrows**, so the write path never MOVES a field out of `self` (which a
///   `Drop` component forbids — `E0509`) and never needs `Clone`.
///
/// A unit struct maps to `Owned = ()` / `Refs<'a> = ()`.
pub trait WireBridge: Sized {
    /// The component's fields as an OWNED tuple, in declaration order — the decode
    /// target (`from_owned` rebuilds the struct from it).
    type Owned;

    /// The component's fields as a tuple of BORROWS, in declaration order — the
    /// encode source (`as_refs` produces it without moving any field, so a `Drop`
    /// component is fine). The GAT lifetime ties the borrows to `&self`.
    type Refs<'a>
    where
        Self: 'a;

    /// Borrows the component's fields into the ref tuple (the encode source). No
    /// move-out, no `Clone`, no `Wire` bound — pure field borrows.
    fn as_refs(&self) -> Self::Refs<'_>;

    /// Reconstructs the component from a decoded owned field tuple (the `C { … }`
    /// constructor, §3.7). Pure value move into the fields — no allocation, no
    /// `Wire` bound.
    fn from_owned(owned: Self::Owned) -> Self;
}

/// Owning / bit-restricted serialize glue (plan §3.1 / §3.7): read `&C`, borrow its
/// fields into the ref tuple, and write each through `WireRefTuple`. The single
/// monomorphized free fn the derive installs as the [`SerializeFn`] for a
/// [`Serializability::SerializeViaFn`] component — no vtable, no `Box<dyn>`, no
/// clone (mirrors [`clone_via_clone`](super::clone_via_clone)'s reach-no-world-state boundary).
///
/// # W7 — cannot reach world state
/// Receives ONLY `*const u8` / a `&mut SaveCursor`; it has no world view, so the
/// user `Wire::wire_write` code it runs cannot create the F2 protected-tag conflict
/// (same boundary as [`clone_via_clone`](super::clone_via_clone)).
///
/// # Safety
///
/// The caller must uphold the [`SerializeFn`] contract:
/// - `src` is a live, aligned, initialized `C` (established at the save call site by
///   the column row-pointer walk); we form a shared `&C` only.
/// - `sink` is a valid, append-only cursor; the fn only appends.
pub unsafe fn serialize_via_wire<C>(
    src: *const u8,
    sink: &mut crate::ecs::core::serialize::SaveCursor<'_>,
) where
    C: WireBridge,
    for<'a> C::Refs<'a>: crate::ecs::core::serialize::WireRefTuple,
{
    use crate::ecs::core::serialize::WireRefTuple as _;
    // SAFETY: `src` is a valid, live, aligned, initialized `C` (the `SerializeFn`
    // contract, established at the column row-pointer call site). The shared `&C`
    // lives only for the borrow + write; the source row is read-only during the save
    // (the saver never mutates the world), so no `&mut C` aliases it.
    let value = unsafe { &*src.cast::<C>() };
    value.as_refs().ref_tuple_write(sink);
}

/// Owning / bit-restricted deserialize glue (plan §3.1 / §3.7): read each field
/// through `WireTuple` in declaration order, then `ptr::write` the reconstructed
/// `C` into the UNINITIALIZED `dst`. The single monomorphized free fn the derive
/// installs as the [`DeserializeFn`] for a [`Serializability::SerializeViaFn`]
/// component.
///
/// On a malformed stream the field read fails BEFORE the `ptr::write`, so `dst` is
/// left uninitialized and the caller (S2) must not drop it — the W5 partial-row
/// contract.
///
/// # Safety
///
/// The caller must uphold the [`DeserializeFn`] contract:
/// - `dst` points at writable, **uninitialized** space of `>= size_of::<C>()`
///   bytes, aligned to `align_of::<C>()`.
/// - On `Ok`, `dst` holds an initialized `C` written exactly once (no prior value
///   is dropped); on `Err`, `dst` is left uninitialized.
pub unsafe fn deserialize_via_wire<C>(
    src: &mut crate::ecs::core::serialize::LoadCursor<'_>,
    dst: *mut u8,
) -> Result<(), crate::ecs::core::serialize::DecodeError>
where
    C: WireBridge,
    C::Owned: crate::ecs::core::serialize::WireTuple,
{
    // Read every field first; on a malformed stream this returns `Err` and `dst` is
    // never written (the value is built fully before any write). The trait method is
    // called fully-qualified, so no `use` is needed here.
    let owned = <C::Owned as crate::ecs::core::serialize::WireTuple>::tuple_read(src)?;
    let value = C::from_owned(owned);
    // SAFETY: `dst` is writable, uninitialized, aligned space for one `C` (the
    // `DeserializeFn` contract). `ptr::write` initializes it WITHOUT dropping the
    // uninitialized prior contents; `value` is moved in exactly once.
    unsafe {
        core::ptr::write(dst.cast::<C>(), value);
    }
    Ok(())
}

/// Autoref probe selecting the [`SerializeFn`] / [`DeserializeFn`] pair for a
/// component (plan §3.7 / C3 graceful demotion). The `&`-`Self` "some" arm requires
/// `C: WireBridge`, `for<'a> C::Refs<'a>: WireRefTuple`, and `C::Owned: WireTuple`
/// (every field `Wire`); the by-value "none" arm (no bound) is the fallback.
///
/// This probe does NOT key on the syntactic POB-eligibility flag: a
/// `#[repr(C)]`-but-not-all-bits-valid struct (e.g. a `String` field) is
/// `POB_ELIGIBLE == true` syntactically yet classified `SerializeViaFn` at runtime,
/// so suppressing the encoder on the syntactic flag would wrongly leave it
/// `None`. Instead the encoder is selected whenever every field is `Wire`, and
/// [`install_serialize_fn`] gates the install on the RUNTIME `Serializability`:
/// `SerializeViaFn` keeps the `Some`, while `PlainOldBytes` (blit path) / `Ignore`
/// store `None` — the single source of truth for "POB installs None".
///
/// Invoked through TWO refs (`(&&probe).serialize_fn_ptr()`): the resolver tries the
/// more-specific `&`-`Self` "some" arm first; if its bounds hold it returns
/// `Some(glue)`, otherwise it DEFERS to the by-value "none" arm (`None`) — never a
/// hard error (the §5 C3 / `SerPodTuple` graceful-demotion discipline).
#[doc(hidden)]
pub struct WireFnProbe<C>(pub core::marker::PhantomData<C>);

impl<C> WireFnProbe<C> {
    /// Constructs the probe (derive-generated code only).
    #[doc(hidden)]
    #[inline]
    pub const fn new() -> Self {
        Self(core::marker::PhantomData)
    }
}

/// By-value-`Self` fallback arm (no bound): a component with a non-`Wire` field
/// installs `None`. The LEAST-specific `Self`, LOWEST priority — wins only when the
/// `&`-`Self` "some" arm's bound does not hold.
#[doc(hidden)]
pub trait WireFnNoneArm {
    #[doc(hidden)]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn>;
    #[doc(hidden)]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn>;
}

impl<C> WireFnNoneArm for WireFnProbe<C> {
    #[inline]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn> {
        None
    }

    #[inline]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn> {
        None
    }
}

/// `&`-`Self` "some" arm gated `C: WireBridge`, `for<'a> C::Refs<'a>: WireRefTuple`,
/// `C::Owned: WireTuple` (the MORE specific `Self`, HIGHER priority): a component
/// whose every field is `Wire` installs the [`serialize_via_wire`] /
/// [`deserialize_via_wire`] glue. A type with a non-`Wire` field fails the
/// `WireRefTuple` / `WireTuple` bound and the resolver leaves this arm un-matched,
/// DEFERRING to the `None` fallback (C3 graceful demotion). A genuinely POB type's
/// fields are all SerPod primitives (which are all `Wire`), so this arm produces a
/// `Some` for it too — [`install_serialize_fn`] then drops it to `None` because the
/// runtime `Serializability` is `PlainOldBytes` (the blit path).
#[doc(hidden)]
pub trait WireFnSomeArm {
    #[doc(hidden)]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn>;
    #[doc(hidden)]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn>;
}

impl<C> WireFnSomeArm for &WireFnProbe<C>
where
    C: WireBridge + 'static,
    for<'a> C::Refs<'a>: crate::ecs::core::serialize::WireRefTuple,
    C::Owned: crate::ecs::core::serialize::WireTuple,
{
    #[inline]
    fn serialize_fn_ptr(&self) -> Option<SerializeFn> {
        Some(serialize_via_wire::<C> as SerializeFn)
    }

    #[inline]
    fn deserialize_fn_ptr(&self) -> Option<DeserializeFn> {
        Some(deserialize_via_wire::<C> as DeserializeFn)
    }
}
