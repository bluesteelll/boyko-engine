//! Required-components sub-registry (Feature 1 — `#[require(B, C)]`).
//!
//! Split out of the former single-file `component_registry` (pure mechanical
//! move — every item keeps its exact `component_registry::…` path via the
//! `pub use required::*` re-export in the parent `mod.rs`). Two parallel cold
//! tables (`REQUIRES_DIRECT` / `REQUIRES_ALL`) plus the memoized-DFS plan
//! builder with the W2 cycle break. Reaches into the core registry (parent
//! module) for `storage_kind` / `is_signature_storage` (dense/signature
//! predicates) and `get_layout` (cycle-panic diagnostics).

// The `BUILDING` cycle-detection stack below. Reached ONLY after
// `build_required_plan`'s memoized `REQUIRES_ALL[id].get()` fast path misses, i.e. once per
// component type per process; every later expansion returns from the lock-free `OnceLock`
// array without touching this. See docs/HOT-PATH-EXCEPTIONS.md.
#[allow(clippy::disallowed_types)]
use std::cell::RefCell;
use std::sync::OnceLock;

use crate::ecs::core::component::component::{Component, RequiredBuilder};
use crate::ecs::identifiers::primitives::ComponentId;

use super::{MAX_COMPONENTS, StorageKind, get_layout, is_signature_storage, storage_kind};

// ═════════════════════════════════════════════════════════════════════════════
// Required components (Feature 1 — `#[require(B, C)]`). Two parallel cold tables
// mirroring the HOOKS / STORAGE_KIND blocks above (D1): the 56 B `ComponentLayout`
// hot record stays pinned (TRIPWIRE 2). Touched ONLY at registration time
// (write-once `REQUIRES_DIRECT`) and at the bundle-resolution / first-expansion
// cold path (`REQUIRES_ALL` memoized DFS) — never on the per-frame hot read path.
// ═════════════════════════════════════════════════════════════════════════════

/// Capture-free constructor for a required component (D2). Mirrors [`DropFn`](super::DropFn):
/// a bare `unsafe fn(*mut u8)` that writes one fully-initialized value of the
/// required component's type into `dst`. F2-immune by construction — it never
/// sees the world.
///
/// The derive lowers `#[require(B)]` to
/// `unsafe fn __require_ctor_B(dst) { dst.cast::<B>().write(B::default()) }`
/// and `#[require(C = expr)]` to `...write({ expr })` (a capture-free
/// expression only — no `Arc<dyn>` / no closure environment).
///
/// # Safety
/// The caller must guarantee:
/// - `dst` points at properly-aligned, writable, **uninitialized** memory of at
///   least `size_of::<T>()` bytes for the required component type `T` whose
///   layout matches `RequiredEntry::component_id`.
/// - `dst` is exclusively owned for the duration of the call; the value written
///   is subsequently owned by the archetype (its `drop_fn` runs on teardown).
pub type RequiredCtor = unsafe fn(dst: *mut u8);

/// A capture-free resolver that returns the required component's
/// [`ComponentId`]. Stored UNCALLED in [`RequiredDirectEntry`] so that
/// registering a `#[require]` edge does NOT eagerly invoke the required type's
/// `component_id()` (BUG-REQ-CYCLE-1): a cycle would otherwise re-enter the
/// requiring type's own `component_id()` `OnceLock::get_or_init` on the same
/// thread and deadlock. The derive emits `B::component_id` (a fn item, no
/// parentheses); the id is resolved LAZILY in `build_required_plan`, which
/// runs at archetype-expansion time — OUTSIDE any `component_id` init.
pub type RequiredIdFn = fn() -> ComponentId;

/// One DIRECT `#[require]` edge as stored in `REQUIRES_DIRECT` (D2). 16 B POD:
/// an 8 B [`RequiredIdFn`] resolver + an 8 B [`RequiredCtor`]. Distinct from
/// [`RequiredEntry`] (which carries a RESOLVED [`ComponentId`]): the direct
/// table holds the id as an UNCALLED resolver to break the registration-time
/// re-entrancy (BUG-REQ-CYCLE-1). `#[repr(C)]` pins the field order; `Copy` so
/// the slice plumbing never invokes drop glue.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct RequiredDirectEntry {
    /// The required component's id resolver, called lazily at plan-build time.
    pub id_fn: RequiredIdFn,
    /// The capture-free constructor that materializes one instance.
    pub ctor: RequiredCtor,
}

/// One transitively-resolved required component (D2). 16 B POD: an 8 B
/// [`ComponentId`] + an 8 B function pointer. `#[repr(C)]` pins the field
/// order; the type is `Copy` so the closure/slice plumbing never invokes drop
/// glue. Produced by `build_required_plan` after resolving each
/// [`RequiredDirectEntry::id_fn`].
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RequiredEntry {
    /// The required component's id.
    pub component_id: ComponentId,
    /// The capture-free constructor that materializes one instance.
    pub ctor: RequiredCtor,
}

// W3: pair the 16 B assert with a `ComponentId` companion so the 16 is
// self-documenting (it is `size_of::<ComponentId>() + size_of::<fn ptr>()`),
// guarding a future `ComponentId` widening. Gated to 64-bit (the engine's
// supported platform) — see CLAUDE.md target platform. Both the resolved entry
// (`ComponentId` + ctor) and the raw direct entry (`RequiredIdFn` + ctor) are
// two-pointer-wide.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<ComponentId>() == 8);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<RequiredEntry>() == 16);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<RequiredDirectEntry>() == 16);

/// The full transitive closure of a component's required components (D3),
/// computed once and memoized in `REQUIRES_ALL`. The `entries` slice is
/// DFS-ordered (deps-before-dependent) and deduped by `component_id` (the W1
/// conflict rule resolves which ctor each id carries).
pub struct RequiredPlan {
    /// Transitive, DFS-ordered, deduped required entries. Empty for a
    /// component that declares no `#[require]` and is required by nothing it
    /// transitively pulls.
    pub entries: &'static [RequiredEntry],
}

/// Distinct error type for the required-components subsystem (W2). Surfaced via
/// a fail-loud panic at registration / first-expansion (present in release, not
/// a vanishing `debug_assert`).
#[derive(Debug)]
pub enum RequiredError {
    /// A `#[require]` cycle was detected (`A → … → A`). Memoization alone does
    /// NOT break it (the `REQUIRES_ALL` slot is `None` during recursion), so
    /// the "currently-building" stack catches the re-entry.
    Cycle {
        /// The id re-entered while already on the building stack.
        component_id: ComponentId,
    },
}

/// Required-components plan D1: write-once table of each component's DIRECT
/// `#[require]` declarations (the immediate edges only — the transitive closure
/// lives in [`REQUIRES_ALL`]). Mirrors the [`HOOKS`] declaration; populated by
/// [`install_required`] from the derive-generated registration path, gated on
/// `Component::HAS_REQUIRES`.
static REQUIRES_DIRECT: [OnceLock<&'static [RequiredDirectEntry]>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Required-components plan D3: memoized transitive closure per component. Built
/// lazily by [`build_required_plan`] on first expansion (the bundle-resolution
/// funnel), DFS over [`REQUIRES_DIRECT`]. A leaked `&'static RequiredPlan` per
/// component (bounded by `MAX_COMPONENTS`, the #53 bounded-leak precedent).
static REQUIRES_ALL: [OnceLock<&'static RequiredPlan>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

thread_local! {
    /// W2 cycle break: the "currently-building" id stack, DISTINCT from the
    /// memoized [`REQUIRES_ALL`] result. Re-entering an id already on this stack
    /// is a cycle (`build_required_plan` panics with [`RequiredError::Cycle`]).
    /// Thread-local because `build_required_plan` recurses on a single thread;
    /// the memoized result is published process-globally via `OnceLock::set`.
    /// Cold-path only: `build_required_plan` returns from the memoized
    /// `REQUIRES_ALL[id]` `OnceLock` BEFORE any guard is pushed, so an expansion
    /// of an already-planned component never borrows this cell.
    #[allow(clippy::disallowed_types)]
    static BUILDING: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// RAII guard for the [`BUILDING`] cycle-detection stack (MINOR robustness fix —
/// mirrors the codebase's `DeferredScopeGuard` / `CursorSync` RAII pattern).
///
/// The cycle-detection stack MUST be unwound even when [`build_required_plan`]
/// panics: `CommandQueue::apply` wraps the apply path in a `catch_unwind`, so a
/// caught [`RequiredError::Cycle`] (or any panic mid-build, e.g. a leaked-alloc
/// OOM) would otherwise leave stale ids on the stack — a later unrelated ACYCLIC
/// bundle's build could then false-positive a cycle on a re-pushed id. Popping in
/// `Drop` guarantees the stack is balanced on both the normal and the unwinding
/// path.
struct BuildingGuard {
    /// The id this guard pushed; verified by the `Drop` balance check.
    id: usize,
}

impl BuildingGuard {
    /// Pushes `component_id` onto the [`BUILDING`] stack, panicking with
    /// [`RequiredError::Cycle`] (via [`required_cycle_panic`]) if it is already
    /// present (a `#[require]` cycle). On the cycle path NO guard is created (the
    /// panic happens before the push), so the existing stack frames' guards still
    /// unwind their own ids correctly.
    #[inline]
    fn push(component_id: usize) -> Self {
        BUILDING.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&component_id) {
                required_cycle_panic(component_id);
            }
            stack.push(component_id);
        });
        Self { id: component_id }
    }
}

impl Drop for BuildingGuard {
    #[inline]
    fn drop(&mut self) {
        BUILDING.with(|stack| {
            let popped = stack.borrow_mut().pop();
            debug_assert_eq!(
                popped,
                Some(self.id),
                "BUILDING stack imbalance in build_required_plan"
            );
        });
    }
}

/// Installs `C`'s DIRECT `#[require]` declarations into
/// `REQUIRES_DIRECT[component_id]` (D1). Builds the entry slice via
/// [`Component::register_required`] and leaks it once (`&'static`), mirroring
/// [`install_hooks`](super::install_hooks)'s write-once discipline.
///
/// Called from the derive-generated `component_id()` ONLY when
/// `C::HAS_REQUIRES` is true (const-gated, like [`install_hooks`](super::install_hooks)): a plain
/// `#[derive(Component)]` leaves the slot UNSET, which reads as "no direct
/// requires" everywhere downstream — the 0%-gate. The leak is bounded by
/// `MAX_COMPONENTS` (one slice per requiring component per process).
#[inline]
pub fn install_required<C: Component>(component_id: usize) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    let mut builder = RequiredBuilder::new();
    C::register_required(&mut builder);
    let leaked: &'static [RequiredDirectEntry] = Box::leak(builder.into_entries());
    // Write-once; a same-id re-install is a silent no-op (first writer wins,
    // matching `install_hooks` / `register_new`).
    let _ = REQUIRES_DIRECT[component_id].set(leaked);
}

/// Returns the DIRECT `#[require]` declarations for `component_id`, or an empty
/// slice when the component declared none. Cold (registration / first-expansion
/// only).
#[inline]
fn get_required_direct(component_id: usize) -> &'static [RequiredDirectEntry] {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return &[];
    }
    REQUIRES_DIRECT[component_id].get().copied().unwrap_or(&[])
}

/// Returns the memoized transitive required-components plan for `component_id`
/// (D3), building it on first access. The returned `entries` slice is
/// DFS-ordered (deps-before-dependent) and deduped per the W1 conflict rule.
///
/// Cold: called at the bundle-resolution funnel (archetype expansion) and by
/// Feature 2 (cloning) to reconstruct a missing required component — exposed
/// `pub(crate)` for that reuse.
///
/// # Panics
///
/// [`RequiredError::Cycle`] (fail-loud, release-active) if a `#[require]` cycle
/// is reachable from `component_id` (W2).
#[inline]
pub(crate) fn get_required_plan(component_id: usize) -> &'static RequiredPlan {
    build_required_plan(component_id)
}

/// Builds (or returns the memoized) transitive required-components plan for
/// `component_id` (D3). Memoized DFS over [`REQUIRES_DIRECT`] with the W1
/// conflict rule and the W2 cycle break.
///
/// Algorithm:
/// 1. Fast path: return the memoized `REQUIRES_ALL[id]` if present.
/// 2. Push `id` onto the thread-local `BUILDING` stack (W2). If it is already
///    present → cycle → panic with [`RequiredError::Cycle`].
/// 3. For each DIRECT required entry `e` of `id`, in declaration order: first
///    resolve `e.id_fn()` to the required `ComponentId` (BUG-REQ-CYCLE-1: this
///    is the ONLY place the required type's `component_id()` is invoked, well
///    outside any mid-init `OnceLock`); then recurse to build that id's closure
///    and merge it (deps-before-dependent), keep-first on duplicate ids (W1
///    inherited rule: first-DFS-reached ctor wins); then merge `e` itself — if
///    the resolved id is already present (pulled transitively by an earlier
///    sibling), OVERRIDE its ctor with `e`'s (a DIRECT declaration wins over an
///    inherited one, W1 direct rule); otherwise push it.
/// 4. Pop the `BUILDING` stack, leak the deduped DFS-ordered slice, and
///    memoize via `OnceLock::set` (first writer wins on a race).
///
/// Exposed `pub(crate)` so Feature 2 (cloning) can reuse the closure +
/// missing-required diff keyed on the clone target id set.
pub(crate) fn build_required_plan(component_id: usize) -> &'static RequiredPlan {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    // 1. Memoized fast path.
    if let Some(plan) = REQUIRES_ALL[component_id].get() {
        return plan;
    }

    // 2. W2 cycle break: push onto the building stack, panic on re-entry. The
    // RAII `_guard` pops `component_id` on EVERY exit path — including a panic
    // unwinding through `CommandQueue::apply`'s `catch_unwind` — so a caught
    // cycle-panic cannot leave a stale id that false-positives a later acyclic
    // build (MINOR robustness fix).
    let _guard = BuildingGuard::push(component_id);

    // 3. DFS merge. `out` accumulates the deps-before-dependent, deduped set.
    let mut out: Vec<RequiredEntry> = Vec::new();
    for &direct in get_required_direct(component_id) {
        // BUG-REQ-CYCLE-1: resolve the required id LAZILY here, NOT at
        // registration time. `id_fn()` is `B::component_id` invoked outside any
        // `component_id` `get_or_init`, so the required type fully initializes
        // without re-entering the requiring type's mid-init `OnceLock`. A
        // genuine cycle instead re-enters `build_required_plan` on the BUILDING
        // stack below → the `BuildingGuard` panic fires (release-active).
        let dep_id_cid = (direct.id_fn)();
        let entry = RequiredEntry {
            component_id: dep_id_cid,
            ctor: direct.ctor,
        };
        let dep_id = dep_id_cid.0;
        // 3a. Merge the dependency's full closure FIRST (deps-before-dependent),
        // keep-first on duplicate ids (W1 inherited rule).
        let dep_plan = build_required_plan(dep_id);
        for &dep_entry in dep_plan.entries {
            if !out.iter().any(|e| e.component_id == dep_entry.component_id) {
                out.push(dep_entry);
            }
        }
        // 3b. Merge the direct entry itself. A DIRECT declaration OVERRIDES any
        // inherited ctor for the same id (W1 direct rule); otherwise push it.
        if let Some(existing) = out.iter_mut().find(|e| e.component_id == entry.component_id) {
            existing.ctor = entry.ctor;
        } else {
            out.push(entry);
        }
    }

    // 4. The DFS for this id is complete. `_guard` pops `component_id` from the
    // BUILDING stack when it drops at the end of this function (or on unwind).

    let leaked_entries: &'static [RequiredEntry] = Box::leak(out.into_boxed_slice());
    let leaked_plan: &'static RequiredPlan = Box::leak(Box::new(RequiredPlan {
        entries: leaked_entries,
    }));
    // Write-once memoization. A concurrent racer's plan is identical (the DFS is
    // deterministic for `id`), so the loser drops its leaked plan reference and
    // reads back the winner's.
    match REQUIRES_ALL[component_id].set(leaked_plan) {
        Ok(()) => leaked_plan,
        Err(_) => REQUIRES_ALL[component_id]
            .get()
            .expect("invariant: OnceLock::set Err implies the slot is occupied"),
    }
}

/// Required components (D4): true iff ANY id in `base_ids` declares (directly or
/// transitively) at least one required component. Cold (bundle-resolution funnel
/// only). For a require-free id set this is `MAX_COMPONENTS`-bounded but in
/// practice one [`get_required_plan`] memoized read per base id, all empty — so
/// the union loop at the call site runs zero inner iterations (the 0%-gate (3)).
#[inline]
pub(crate) fn any_requires(base_ids: &[ComponentId]) -> bool {
    base_ids
        .iter()
        .any(|cid| !get_required_plan(cid.0).entries.is_empty())
}

/// Dense plan D2: `true` iff ANY id in `ids` is `StorageKind::Dense`. The cheap
/// one-shot gate the structural-op routing (`InsertCommand` / migration helpers)
/// uses to fold out the entire dense branch for a table-only bundle (the
/// 0%-gate). Cold — read only at structural-op resolution, never the per-frame
/// path. One `Relaxed` `STORAGE_KIND` load + branch per id.
#[inline]
pub(crate) fn any_dense(ids: &[ComponentId]) -> bool {
    ids.iter()
        .any(|cid| matches!(storage_kind(cid.0), StorageKind::Dense))
}

/// Dense plan D2: `true` iff `cid` is a signature-storage id (i.e.
/// `StorageKind::Table`). The per-id companion of [`is_signature_storage`] that
/// also does the registry read, used by the structural-op fire / per-pool loops
/// that iterate an archetype's RETAINED `component_ids` (which keeps
/// non-signature ids) to SKIP a dense (or bitset) id — dense is fired/stored by
/// the dedicated D2 routing, never the table `component_ids` machinery. For a
/// table-only world every id is `Table`, so the skip is never taken (a cold
/// `Relaxed` load + branch on an already-cold path; the per-frame hot loops are
/// untouched — the 0%-gate).
#[inline]
pub(crate) fn is_signature_id(cid: ComponentId) -> bool {
    is_signature_storage(storage_kind(cid.0))
}

/// Required components (D4): invokes `push` once for every transitively-required
/// component id reachable from `base_ids` that is NOT already present in
/// `base_ids` and not already pushed (deduped against an internal `seen` set
/// seeded from `base_ids`). Used by the bundle-resolution funnel
/// (`cold_register_bundle_archetype` / `merged_archetype_id`) to compute the
/// EFFECTIVE archetype id set.
///
/// Cold path only — runs once per `(bundle, world)` (spawn) or per insert
/// migration resolve; the result is cached on the Phase-8.5 slot /
/// `BundleColumnRecord`.
///
/// Present⇒skip: a required id already in `base_ids` is never pushed (the
/// explicit value wins, no overwrite — D resolved-questions "present ⇒ skip").
#[inline]
pub(crate) fn for_each_required_id_excluding<F: FnMut(ComponentId)>(
    base_ids: &[ComponentId],
    mut push: F,
) {
    // `seen` carries the base ids plus everything pushed so far, so a diamond
    // (two base ids both requiring D) emits D once (D3 / W1 dedup).
    let mut seen: Vec<ComponentId> = base_ids.to_vec();
    for &cid in base_ids {
        for entry in get_required_plan(cid.0).entries {
            if !seen.contains(&entry.component_id) {
                seen.push(entry.component_id);
                push(entry.component_id);
            }
        }
    }
}

/// Required components (Feature 1): resolves the W1-conflict-resolved
/// [`RequiredCtor`] for `target_id` within the transitive closure of `base_ids`,
/// or `None` if `target_id` is not reachable as a required component from any
/// base id. Used by the insert-path constructor pass to look up the ctor for an
/// id it decided to construct.
///
/// Returns the FIRST matching entry (the W1 first-DFS / direct-override result
/// is already baked into each base id's memoized plan, so the first base whose
/// closure contains `target_id` carries the precedence-correct ctor — the
/// `for_each_required_id_excluding` iteration order is identical).
#[inline]
pub(crate) fn required_ctor_for(
    base_ids: &[ComponentId],
    target_id: ComponentId,
) -> Option<RequiredCtor> {
    for &cid in base_ids {
        for entry in get_required_plan(cid.0).entries {
            if entry.component_id == target_id {
                return Some(entry.ctor);
            }
        }
    }
    None
}

/// Cold fail-loud panic site for the W2 cycle break. Kept out of line so
/// [`build_required_plan`]'s body stays compact.
#[cold]
#[inline(never)]
fn required_cycle_panic(component_id: usize) -> ! {
    let name = get_layout(component_id)
        .map(|l| l.type_name)
        .unwrap_or("<unregistered>");
    panic!(
        "{:?}: a #[require] cycle is reachable from ComponentId {} ({}). \
         Required-component edges must form a DAG.",
        RequiredError::Cycle {
            component_id: ComponentId(component_id),
        },
        component_id,
        name,
    )
}
