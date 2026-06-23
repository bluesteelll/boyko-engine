//! Custom triggers — user-defined events dispatched immediately through the
//! observer fire backbone (Feature 2 D3).
//!
//! A [`Trigger`] is any `'static` user type fired via
//! [`EcsMaster::trigger`]/[`EcsMaster::trigger_global`]. Unlike Phase-12
//! buffered events (bulk, frame-deferred, polled), a trigger runs INLINE at
//! `trigger`-time on the firing thread, entity-targeted, with optional
//! propagation up `ChildOf`.
//!
//! # Id space (deviation from the original "reuse EventId" wording)
//!
//! The Phase-12 `EventId` mint requires the full `Event` trait
//! (`Participants`/`Parameters`/`layout`), which a plain `Trigger: 'static`
//! cannot satisfy. So triggers get their OWN dense [`TriggerId`] mint — same
//! boyko-native shape as `register_event_new` (an atomic counter + per-slot
//! `OnceLock`, process-stable, no `HashMap`, no per-lookup alloc), keyed by
//! `TypeId`. This honours the design's intent (a dense atomic-minted id) without
//! forcing `Trigger` into the `Event` machinery.
//!
//! [`EcsMaster::trigger`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::trigger
//! [`EcsMaster::trigger_global`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::trigger_global

use std::any::TypeId;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::component::observers::traversal::{PropagationMode, Traversal};
use crate::ecs::core::component::observers::ObserverId;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::relationship::Relationship;

/// Maximum number of distinct custom-trigger types in a process.
pub const MAX_TRIGGERS: usize = 256;

// FIX O2: every minted `TriggerId` (`0..MAX_TRIGGERS`) is packed into the low 31
// bits of a `DispatchKey` (the high bit is `DispatchKey::CUSTOM_FLAG`). Guard at
// compile time that a future `MAX_TRIGGERS` bump cannot mint an id that collides
// with the custom-flag high bit. `MAX_TRIGGERS - 1` is the largest possible id.
const _: () = assert!(
    MAX_TRIGGERS < (1usize << 31),
    "MAX_TRIGGERS must stay below 2^31 so a minted TriggerId never collides with \
     DispatchKey::CUSTOM_FLAG (1 << 31)"
);

/// Dense, process-stable id for a custom-trigger type, minted by
/// `trigger_id_of`. Packed into a `DispatchKey` for entity-targeted custom
/// observers.
///
pub type TriggerId = u32;

/// Per-trigger-type slot record (just the `TypeId`, for collision detection).
static TRIGGER_INFO: [OnceLock<TypeId>; MAX_TRIGGERS] =
    [const { OnceLock::new() }; MAX_TRIGGERS];

/// Monotonic counter for [`TriggerId`]s.
static NEXT_TRIGGER_ID: AtomicUsize = AtomicUsize::new(0);

/// Marker for a user type usable with [`EcsMaster::trigger`].
///
/// `'static` so its [`TriggerId`] is stable. Per FIX O3 it is **not** `Send +
/// Sync`: the event value is read by `*const u8` synchronously on ONE thread
/// within `trigger` (it never crosses a thread boundary), so dropping the
/// `Send + Sync` bound admits `Rc`/non-`Sync` payloads at no soundness cost.
///
/// [`EcsMaster::trigger`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::trigger
pub trait Trigger: 'static {
    /// Whether this event bubbles up [`Self::Traversal`] without an explicit
    /// `propagate(true)`. `const` so the propagation loop const-folds away for
    /// non-bubbling events (the 0%-gate at the type level).
    ///
    /// Consulted ONLY when [`Self::PROPAGATION`] is
    /// [`PropagationMode::Up`] (the bubble direction); the
    /// [`Down`](PropagationMode::Down) broadcast seeds its per-node propagate
    /// flag `true` (fan-out-all by default) independently of this constant.
    const AUTO_PROPAGATE: bool = false;

    /// The propagation shape: [`None`](PropagationMode::None) (target only —
    /// the default, byte-identical to the pre-broadcast machinery),
    /// [`Up`](PropagationMode::Up) (bubble up [`Self::Traversal`]), or
    /// [`Down`](PropagationMode::Down) (broadcast over [`Self::Broadcast`]'s
    /// reverse collection). `const` so the fire loop const-folds the branch —
    /// the `None`/`Up` arms keep their pre-broadcast code generation.
    const PROPAGATION: PropagationMode = PropagationMode::None;

    /// The relationship the event bubbles along (the `Up` direction). Set to
    /// [`ChildOfTraversal`](crate::ecs::core::component::observers::traversal::ChildOfTraversal)
    /// for the default parent bubble.
    type Traversal: Traversal;

    /// The relationship whose reverse [`RelationshipTarget`](crate::ecs::core::relationship::RelationshipTarget)
    /// collection is fanned out for a [`Down`](PropagationMode::Down) broadcast.
    ///
    /// Consulted ONLY when [`Self::PROPAGATION`] is `Down`. For a `None` / `Up`
    /// trigger this binding is never read — set it to
    /// [`ChildOf`](crate::ecs::core::hierarchy::ChildOf) (the conventional
    /// placeholder; the associated type is compile-time only, so a never-read
    /// binding is asm-identical to the pre-broadcast trigger). For a `Down`
    /// broadcast over `ChildOf` the descent visits every transitive child.
    ///
    /// A required associated type (no default) to keep the crate on stable Rust
    /// — associated-type defaults are unstable, and the sibling
    /// [`Self::Traversal`] is already required, so every existing `Trigger`
    /// impl already names its associated types.
    type Broadcast: Relationship;
}

/// Runner for a custom trigger.
///
/// `event` is an erased `*const u8` to the user event value (read by the
/// monomorphised registrant). Same view shape as `ObserverFn` so the fire loop
/// is shared; the [`TriggerContext`] travels by value (re-entrancy-safe).
pub type TriggerFn = unsafe fn(DeferredEcsMaster<'_>, TriggerContext, *const u8);

/// Context handed to every custom-trigger runner — travels BY VALUE through the
/// bubble walk (FIX W9: no `target`/`original_target` in TLS).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TriggerContext {
    /// The entity the event currently targets (advances as it bubbles).
    pub target: Entity,
    /// The original target before any propagation.
    pub original_target: Entity,
    /// The dense id of the fired trigger type.
    pub trigger_id: TriggerId,
}

/// One registered GLOBAL trigger observer: its stable id + runner.
#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct TriggerEntry {
    pub(crate) id: ObserverId,
    pub(crate) runner: TriggerFn,
}

/// Mints a NEW dense [`TriggerId`] on EVERY call (it is NOT idempotent — it
/// bumps the monotonic counter and claims a fresh per-slot `OnceLock` each time).
///
/// Callers MUST cache the result per type — the sole intended caller is
/// [`static_trigger_id`], which memoises it in a per-`E` `OnceLock`. A direct
/// second call for the same `E` would claim a second slot (a different id) and a
/// third would eventually exhaust the registry; the `OnceLock::set` guard's
/// `unreachable!` only catches a same-slot collision, which cannot occur for a
/// freshly `fetch_add`-ed index. Mirrors `register_event_new`: an atomic counter
/// + a per-slot `OnceLock`, lock-free, no `HashMap`.
pub(crate) fn trigger_id_of<E: Trigger>() -> TriggerId {
    let raw = NEXT_TRIGGER_ID.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw < MAX_TRIGGERS,
        "TriggerRegistry exhausted: reached {raw}, MAX_TRIGGERS = {MAX_TRIGGERS}"
    );
    match TRIGGER_INFO[raw].set(TypeId::of::<E>()) {
        Ok(()) => raw as TriggerId,
        Err(_) => {
            // Counter raced ahead of an already-registered slot is impossible
            // (fetch_add is unique); a same-slot collision means the slot is
            // occupied by a different type — never for a fresh `raw`.
            unreachable!("a freshly fetch_add-ed TriggerId slot cannot be occupied")
        }
    }
}

/// Per-world registry of GLOBAL (non-entity) custom-trigger observers, keyed by
/// dense [`TriggerId`]. Lazy `Option<Box>` — same zero-cost gate as
/// `ObserverRegistry`.
pub(crate) struct TriggerRegistry {
    inner: Option<Box<TriggerLists>>,
    next_id: u64,
}

struct TriggerLists {
    /// `TriggerId -> global observers`. Grows as trigger types register.
    by_trigger: Vec<Vec<TriggerEntry>>,
}

impl TriggerRegistry {
    /// Creates an empty registry — zero allocation.
    #[inline]
    pub(crate) fn new() -> Self {
        Self { inner: None, next_id: 0 }
    }

    /// Registers a global observer `runner` for trigger id `tid`, returning its
    /// stable [`ObserverId`]. Lazily allocates on first use and grows the dense
    /// `by_trigger` Vec to cover `tid`.
    pub(crate) fn add(&mut self, tid: TriggerId, runner: TriggerFn) -> ObserverId {
        let id = ObserverId(self.next_id);
        self.next_id += 1;
        let lists = self
            .inner
            .get_or_insert_with(|| Box::new(TriggerLists { by_trigger: Vec::new() }));
        let idx = tid as usize;
        if idx >= lists.by_trigger.len() {
            lists.by_trigger.resize_with(idx + 1, Vec::new);
        }
        lists.by_trigger[idx].push(TriggerEntry { id, runner });
        id
    }

    /// Removes the global trigger observer with `id`, returning `true` if found.
    pub(crate) fn remove(&mut self, id: ObserverId) -> bool {
        let Some(lists) = self.inner.as_mut() else {
            return false;
        };
        for list in lists.by_trigger.iter_mut() {
            if let Some(pos) = list.iter().position(|e| e.id == id) {
                list.swap_remove(pos);
                return true;
            }
        }
        false
    }

    /// Returns the `i`-th global runner for `tid`, COPIED OUT by value, or
    /// `None` past the end (the fire loop re-derives `&self` per turn).
    #[inline]
    pub(crate) fn nth_runner(&self, tid: TriggerId, i: usize) -> Option<TriggerFn> {
        let lists = self.inner.as_ref()?;
        let list = lists.by_trigger.get(tid as usize)?;
        list.get(i).map(|e| e.runner)
    }

    /// `true` iff at least one GLOBAL observer is registered for `tid`.
    ///
    /// The cold 0%-probe half for the relation-edge observers: a world that
    /// never registered a global trigger observer takes the lazy-`None`
    /// early-out (one `Option::is_none()`).
    #[inline]
    pub(crate) fn has(&self, tid: TriggerId) -> bool {
        self.inner
            .as_ref()
            .and_then(|l| l.by_trigger.get(tid as usize))
            .is_some_and(|list| !list.is_empty())
    }
}

impl Default for TriggerRegistry {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide intern table mapping each distinct `Trigger` type's [`TypeId`]
/// to its dense [`TriggerId`].
///
/// FIX F2: a function-local `static CACHE: OnceLock<TriggerId>` in the body of a
/// generic fn is NOT monomorphised per `E` — one shared static backs every
/// instantiation, so the first trigger type mints id 0 and every other type
/// returns that same id (the Phase-12.5 "static SLOT in a generic-fn body
/// collapses across monomorphisations" class). A `TypeId`-keyed intern gives
/// each distinct `E` a distinct stable id. Cold (registration-only — runs once
/// per type, never on the trigger hot path), so the `Mutex`/`HashMap` is
/// acceptable here; it mirrors the Phase-12.5 query-type-id intern.
static TRIGGER_IDS: OnceLock<Mutex<HashMap<TypeId, TriggerId>>> = OnceLock::new();

/// Returns the process-stable [`TriggerId`] for `E`, cached per type.
///
/// The id is interned by [`TypeId::of::<E>()`] in [`TRIGGER_IDS`], so each
/// distinct trigger type gets a distinct dense id (mirrors `Event::event_id`'s
/// per-type stability). The first sight of `E` mints via [`trigger_id_of`];
/// subsequent calls return the cached id.
pub(crate) fn static_trigger_id<E: Trigger>() -> TriggerId {
    let map = TRIGGER_IDS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("invariant: TRIGGER_IDS mutex poisoned");
    *guard
        .entry(TypeId::of::<E>())
        .or_insert_with(trigger_id_of::<E>)
}

/// Fires every GLOBAL custom-trigger observer registered for `tid`
/// (Feature 2, OBS-FIRE-LOOP).
///
/// Cold: re-derives `&world.triggers` per turn, copies the [`TriggerFn`] out by
/// value, drops the `&` BEFORE minting the view. `event` points at the live
/// event value pinned on the `trigger` stack frame for the whole walk.
#[cold]
#[inline(never)]
pub(crate) fn fire_global_triggers(
    world: NonNull<EcsMaster>,
    tid: TriggerId,
    ctx: TriggerContext,
    event: *const u8,
) {
    let mut i = 0usize;
    loop {
        let runner: TriggerFn = {
            // SAFETY (OBS-FIRE / F2): `world` aliases no live reborrow (minted
            //   after every `world`-derived `&mut` was dropped). This `&` is
            //   re-derived per turn and dropped at this block's close, BEFORE the
            //   view is minted. Single-threaded apply window; the registry is
            //   mutated only via `&mut self`, which cannot be live here.
            let reg = unsafe { &world.as_ref().triggers };
            let Some(r) = reg.nth_runner(tid, i) else {
                break;
            };
            r
        };
        // SAFETY (SAFETY-1 / SAFETY-4): `world` aliases no live reborrow at the
        //   mint; single-threaded apply window; the read-only view withholds
        //   every structural + `&mut`-into-pool method.
        let view = unsafe { DeferredEcsMaster::from_world(world) };
        // SAFETY (TriggerFn contract): apply-window + non-aliasing; `event` is a
        //   read-only pointer to the live event value pinned for the walk.
        unsafe {
            runner(view, ctx, event);
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::observers::traversal::ChildOfTraversal;
    use crate::ecs::core::hierarchy::ChildOf;

    struct TidA;
    impl Trigger for TidA {
        type Traversal = ChildOfTraversal;
        type Broadcast = ChildOf;
    }
    struct TidB;
    impl Trigger for TidB {
        type Traversal = ChildOfTraversal;
        type Broadcast = ChildOf;
    }
    struct TidC;
    impl Trigger for TidC {
        type Traversal = ChildOfTraversal;
        type Broadcast = ChildOf;
    }

    /// TESTER FINDING (F2 — engine bug, see report): distinct `Trigger` types
    /// must mint DISTINCT dense `TriggerId`s. `static_trigger_id` memoises in a
    /// `static CACHE: OnceLock<TriggerId>` declared in the body of a GENERIC fn
    /// — but a function-local `static` is NOT monomorphised per `E`; one shared
    /// static backs every instantiation, so the FIRST trigger type mints id 0
    /// and EVERY other type returns that same cached id. This collapses all
    /// custom-trigger keys to one `DispatchKey`, so an entity observing two
    /// trigger types fires BOTH for either (the Phase-12.5 `static SLOT in a
    /// generic-fn body collapses across monomorphisations` class).
    #[test]
    fn distinct_trigger_types_get_distinct_ids() {
        let a = static_trigger_id::<TidA>();
        let b = static_trigger_id::<TidB>();
        let c = static_trigger_id::<TidC>();
        assert_ne!(a, b, "TidA and TidB must mint distinct TriggerIds");
        assert_ne!(b, c, "TidB and TidC must mint distinct TriggerIds");
        assert_ne!(a, c, "TidA and TidC must mint distinct TriggerIds");
    }
}
