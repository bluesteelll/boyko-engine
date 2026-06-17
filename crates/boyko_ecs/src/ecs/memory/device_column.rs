//! Phase 4 Seam 3 — the device-resident column primitive (CR-C / IM-1 / IM-5).
//!
//! `boyko_ecs` is GPU-capable but **graphics-pure**: this module names no
//! `boyko_rhi`/Vulkan type. A GPU-resident [`ComponentPool`] stores its rows in
//! device memory behind an opaque [`DeviceColumnHandle`] (a bare `u64` that a
//! future `boyko_render` packs a slot/registry index into); the `boyko_ecs` side
//! only owns the handle and the device-side row counters.
//!
//! Phase 4 mints NO device pool (the residency table is empty until a GPU
//! component registers), so [`DeviceColumn`] is a forward seam: it constructs
//! (the stub) and is `Send + Sync`, and `ComponentPool`'s `PoolBacking::Device`
//! arm holds a `Box<DeviceColumn>`. Phase 5 fills the RHI allocate/grow/release.
//!
//! [`ComponentPool`]: crate::ecs::memory::component_pool::ComponentPool

/// Opaque, graphics-pure handle to a device-memory column (Phase 4 Seam 3, Q2).
///
/// `#[repr(transparent)]` over a bare `u64` so it is Miri-safe (a plain integer,
/// no provenance) and stays compiled in on every target — UNLIKE
/// [`DeviceColumn`], which is `#[cfg(not(miri))]`. A future `boyko_render`'s
/// `slot <-> u64` bridge packs a device-column registry index (and any
/// generation/tag bits it needs) into this; `boyko_ecs` treats it as an opaque
/// token it neither interprets nor dereferences.
///
/// The `u64` is `Copy` POD — never a pointer — so a `ComponentPool` carrying one
/// is trivially `Send + Sync` with respect to this field (IM-5).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceColumnHandle(pub u64);

/// A GPU-resident column's `boyko_ecs`-side ownership record (Phase 4 Seam 3).
///
/// Holds the opaque [`DeviceColumnHandle`] plus the device-side live/committed
/// row counts — the device-resident twins of `ComponentPool::len` /
/// `committed_rows`. Per CR-C the Host counterparts (`ComponentPool::len`) stay
/// `0` for a device pool's life, so the CPU drop loop is a no-op and the device
/// row count lives ONLY here.
///
/// `#[cfg(not(miri))]`: Miri cannot execute the RHI syscalls a real device
/// column needs, and Phase 4 never mints one, so under Miri `PoolBacking`
/// collapses to a single-variant `Host` enum with `VmReservation`'s exact layout
/// (P7). Boxed inside `PoolBacking::Device` so the enum stays ≤ 16 B (IM-1).
///
/// `#[allow(dead_code)]`: the WHOLE read-surface (`handle`/`device_len`/
/// `device_cap` + their fields) is a Phase-5 forward seam — Phase 4 mints no
/// device pool through any production path, so nothing reads these yet. The test
/// `make_device_backed_for_test` exercises `new` + the `PoolBacking::Device`
/// arm; the accessors land live in Phase 5's RHI lowering.
#[cfg(not(miri))]
#[allow(dead_code)]
pub(crate) struct DeviceColumn {
    /// Opaque device-column token (graphics-pure `u64`). The handle MAY change
    /// on a Phase-5 grow (realloc on the device) — sound because no CPU code
    /// caches a device row pointer (D3).
    handle: DeviceColumnHandle,
    /// Device-side live row count (the device twin of `ComponentPool::len`).
    /// Phase 4 stub: `0`.
    device_len: usize,
    /// Device-side committed row capacity (the device twin of
    /// `ComponentPool::committed_rows`). Phase 4 stub: `0`.
    device_cap: usize,
}

// `#[allow(dead_code)]`: a Phase-5 forward seam (see the struct doc). `new` is
// reached by `make_device_backed_for_test` under `cfg(test)`, but is dead in a
// plain `cargo build`; the accessors land live in Phase 5.
#[cfg(not(miri))]
#[allow(dead_code)]
impl DeviceColumn {
    /// Constructs a device column wrapping `handle` with empty device-side
    /// counters (Phase 4 stub — Phase 5 wires the RHI allocate that produces
    /// `handle` and a non-zero `device_cap`).
    #[inline]
    pub(crate) fn new(handle: DeviceColumnHandle) -> Self {
        Self {
            handle,
            device_len: 0,
            device_cap: 0,
        }
    }

    /// Returns the opaque device-column handle (Phase 5: the RHI lowering binds
    /// the device column through it).
    #[inline]
    pub(crate) fn handle(&self) -> DeviceColumnHandle {
        self.handle
    }

    /// Overwrites the opaque device-column handle (Phase 5 MF-2/3).
    ///
    /// Called by `ComponentPool::set_device_handle` after a `boyko_render` grow
    /// reallocs the device column and mints a NEW handle. Mutating the handle is
    /// sound because no CPU code caches a device row pointer (D3): the handle is
    /// an opaque token resolved indirectly each frame through the
    /// `(archetype, component)` key (MF-7), never persisted raw across a grow.
    #[inline]
    pub(crate) fn set_handle(&mut self, handle: DeviceColumnHandle) {
        self.handle = handle;
    }

    /// Returns the device-side live row count (the device twin of
    /// `ComponentPool::len`).
    #[inline]
    pub(crate) fn device_len(&self) -> usize {
        self.device_len
    }

    /// Returns the device-side committed row capacity (the device twin of
    /// `ComponentPool::committed_rows`).
    #[inline]
    pub(crate) fn device_cap(&self) -> usize {
        self.device_cap
    }
}

// IM-5: the device backing must be thread-transferable for `ComponentPool`'s
// `unsafe impl Send + Sync` to stand. `DeviceColumn` is `Send + Sync` by its
// fields alone (a `Copy` POD `u64` handle + two `usize`), so this is a
// compile-time witness, NOT an `unsafe impl` lie — if a future field broke it,
// this `_assert` would fail to compile.
#[cfg(not(miri))]
fn _assert()
where
    DeviceColumn: Send + Sync,
{
}
