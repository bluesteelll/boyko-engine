//! Global registry of **non-`Send`** resource type metadata (Phase 4 Seam 2).
//!
//! A parallel of [`resource_registry`](super::resource_registry) for types
//! implementing [`NonSendResource`]. Each distinct `R` is assigned a unique
//! [`NonSendResourceId`] the first time `NonSendRes<R>` / `NonSendResMut<R>`
//! initialises its state in the current process. The assignment is lazy,
//! lock-free on the cached read path, and stable for the lifetime of the
//! process.
//!
//! # Why a separate registry (D6 / CR-A)
//!
//! Keeping NonSend ids in their own counter + slab keeps `Resource`'s
//! `Send + Sync` slab and registry **completely untouched** — `Res`/`ResMut`
//! pay zero cost, and "NonSend" is a type-level fact (different registry /
//! slab). The id space is independent of `ResourceId`.
//!
//! # Threading
//!
//! Registry mints are safe to call from any thread (the counter uses
//! `Relaxed`; cross-thread happens-before is provided by `OnceLock`). In
//! practice every NonSend resource is registered from the dispatcher thread
//! during system init — the same single-threaded first-touch pattern as
//! [`resource_registry`].

// The registry's functions are consumed by `NonSendRes` / `NonSendResMut`
// `SystemParam` init and by `EcsMaster`'s non-send facade. Until a downstream
// consumer exercises every path, a module-level `dead_code` allow mirrors the
// `resource_registry` precedent.
#![allow(dead_code)]

use std::any::TypeId;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ecs::core::resources::resource::NonSendResource;

/// Number of non-send resource slots in the global registry.
///
/// Matches [`RESOURCE_SLOT_COUNT`](super::resource_registry::RESOURCE_SLOT_COUNT)
/// (256) — the non-send slab is the same shape as the `Send` slab.
pub const NON_SEND_RESOURCE_SLOT_COUNT: usize = 256;

/// Type-erased drop function pointer for a non-send resource type `R`.
///
/// Stored in [`NonSendResourceInfo::drop_fn`] for types where
/// `mem::needs_drop::<R>()` is true. Invoked by the [`NonSendResources`] slab
/// on `insert` (replace path) and during `Drop`.
///
/// # Safety
/// The caller must guarantee:
/// - `ptr` points at a properly-aligned, fully-initialized instance of `R`.
/// - `ptr` is not aliased and the value will not be read or dropped again
///   after this call.
/// - The call happens on the value's owning (dispatcher) thread — `R` is
///   `!Send`, so dropping it off-thread would be unsound.
///
/// [`NonSendResources`]: super::nonsend_resources::NonSendResources
pub type NonSendDropFn = unsafe fn(*mut u8);

/// Type-erased drop glue for a non-send `R`.
///
/// # Safety
/// See [`NonSendDropFn`] contract above.
#[inline]
pub(crate) unsafe fn nonsend_drop_in_place_glue<R: 'static>(ptr: *mut u8) {
    // SAFETY: caller upholds the NonSendDropFn contract: ptr is aligned,
    // initialized, exclusively owned, dropped on the owning thread, and not
    // accessed again after this call.
    unsafe { core::ptr::drop_in_place::<R>(ptr.cast::<R>()) }
}

/// Static metadata for one non-send resource type.
///
/// Field order is cache-line friendly: hot fields (size, alignment, drop_fn)
/// at lower offsets; cold fields (type_name, type_id) at higher offsets.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct NonSendResourceInfo {
    /// Size in bytes — hot.
    pub size: usize,
    /// Alignment requirement — hot.
    pub alignment: usize,
    /// Drop function pointer; `None` iff `!mem::needs_drop::<R>()` — hot.
    pub drop_fn: Option<NonSendDropFn>,
    /// Cold: type name for diagnostics.
    pub type_name: &'static str,
    /// Cold: `TypeId` for runtime type validation.
    pub type_id: TypeId,
}

impl NonSendResourceInfo {
    /// Creates a new `NonSendResourceInfo` with static information about `R`.
    ///
    /// The `if needs_drop::<R>()` branch is const-folded per monomorphization.
    #[inline]
    pub fn new_static<R: NonSendResource>() -> Self {
        Self {
            size: std::mem::size_of::<R>(),
            alignment: std::mem::align_of::<R>(),
            drop_fn: if std::mem::needs_drop::<R>() {
                Some(nonsend_drop_in_place_glue::<R> as NonSendDropFn)
            } else {
                None
            },
            type_name: std::any::type_name::<R>(),
            type_id: TypeId::of::<R>(),
        }
    }
}

/// Static storage for non-send resource metadata. Each slot is independent
/// and initialized at most once via `OnceLock::set`.
static NON_SEND_RESOURCE_INFO: [OnceLock<NonSendResourceInfo>;
    NON_SEND_RESOURCE_SLOT_COUNT] =
    [const { OnceLock::new() }; NON_SEND_RESOURCE_SLOT_COUNT];

/// Monotonic counter for non-send resource ids minted via [`register_new`].
static NEXT_NON_SEND_RESOURCE_ID: AtomicUsize = AtomicUsize::new(0);

/// Allocates a fresh non-send resource id from the global counter and stores
/// `NonSendResourceInfo::new_static::<R>()` in the corresponding slot.
///
/// Called from `NonSendRes<R>` / `NonSendResMut<R>` `init_state` via a
/// per-monomorphization `OnceLock`. Each concrete `R` gets exactly one id
/// across the process lifetime.
///
/// # Panics
/// - If `NEXT_NON_SEND_RESOURCE_ID` reaches `NON_SEND_RESOURCE_SLOT_COUNT`.
/// - If the slot at the minted index is already occupied by a *different*
///   type.
pub fn register_new<R: NonSendResource>() -> usize {
    let type_id = TypeId::of::<R>();
    let raw = NEXT_NON_SEND_RESOURCE_ID.fetch_add(1, Ordering::Relaxed);
    assert!(
        raw < NON_SEND_RESOURCE_SLOT_COUNT,
        "NonSendResourceRegistry exhausted: NEXT_NON_SEND_RESOURCE_ID reached {raw}, \
         NON_SEND_RESOURCE_SLOT_COUNT = {NON_SEND_RESOURCE_SLOT_COUNT}"
    );
    let info = NonSendResourceInfo::new_static::<R>();
    match NON_SEND_RESOURCE_INFO[raw].set(info) {
        Ok(()) => raw,
        Err(_) => {
            let existing = NON_SEND_RESOURCE_INFO[raw]
                .get()
                .expect("invariant: OnceLock::set Err implies the slot is occupied");
            if existing.type_id == type_id {
                raw
            } else {
                panic!(
                    "NonSendResourceId {} occupied by type {}, refused to register {}",
                    raw,
                    existing.type_name,
                    std::any::type_name::<R>()
                )
            }
        }
    }
}

/// Retrieves metadata for a non-send resource by its raw `usize` id.
/// Returns `None` if the type has not been registered yet.
#[inline]
pub fn get_nonsend_resource_info(raw_id: usize) -> Option<&'static NonSendResourceInfo> {
    NON_SEND_RESOURCE_INFO.get(raw_id)?.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `!Send` test resource (holds a raw pointer → not `Send`).
    struct NonSendA(#[allow(dead_code)] *const u8);
    impl NonSendResource for NonSendA {}

    /// Second `!Send` test resource for the distinctness check.
    struct NonSendB(#[allow(dead_code)] *const u8);
    impl NonSendResource for NonSendB {}

    /// A `!Send` resource WITH a Drop impl — forces the `needs_drop` branch.
    struct NonSendDrop(#[allow(dead_code)] *const u8);
    impl Drop for NonSendDrop {
        fn drop(&mut self) {}
    }
    impl NonSendResource for NonSendDrop {}

    #[test]
    fn register_then_get_returns_info() {
        let id = register_new::<NonSendA>();
        let info = get_nonsend_resource_info(id).expect("info present after register");
        assert_eq!(info.size, std::mem::size_of::<NonSendA>());
        assert_eq!(info.alignment, std::mem::align_of::<NonSendA>());
        assert_eq!(info.type_id, TypeId::of::<NonSendA>());
        assert!(
            info.drop_fn.is_none(),
            "NonSendA has no Drop impl — drop_fn must be None"
        );
    }

    #[test]
    fn register_drop_type_has_drop_fn() {
        let id = register_new::<NonSendDrop>();
        let info = get_nonsend_resource_info(id).expect("info present");
        assert!(
            info.drop_fn.is_some(),
            "NonSendDrop has a Drop impl — drop_fn must be Some"
        );
    }

    #[test]
    fn distinct_types_get_distinct_ids() {
        let id_a = register_new::<NonSendA>();
        let id_b = register_new::<NonSendB>();
        assert_ne!(id_a, id_b);
    }
}
