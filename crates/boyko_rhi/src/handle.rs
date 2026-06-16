//! Typed generational resource handles + the `Slot ↔ u64` packing bridge + the
//! [`ResourceRegistry`].
//!
//! This is the THIN, backend-agnostic registry layer (plan central decision):
//! it maps a generational [`Slot`] to an owned RHI resource (one
//! [`SparseSlotMap`] per resource kind, SoA) and projects a `Slot` to an opaque
//! `u64` the Phase-4 core seam (`DeviceColumnHandle`, defined in `boyko_ecs`)
//! will consume. The hal trait never names `u64`; the registry never appears in
//! core. No `dyn`, no `Box`, no `HashMap` — `resolve_*` is a generation-checked
//! array index (the Phase-7 ~3 ns lookup).

use boyko_utils::identifiers::primitives::Generation;
use boyko_utils::identifiers::slot::Slot;
use boyko_utils::sparse_map::sparse_slot_map::SparseSlotMap;

use crate::api::RhiApi;
use crate::device::RhiDevice;

/// Typed handle to a buffer in a [`ResourceRegistry`].
///
/// `#[repr(transparent)]` over [`Slot`]: the generational index *is* the handle,
/// with no extra footprint.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferHandle(pub Slot);

/// Typed handle to a compute pipeline in a [`ResourceRegistry`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ComputePipelineHandle(pub Slot);

/// Typed handle to a shader module in a [`ResourceRegistry`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShaderHandle(pub Slot);

/// Typed handle to a fence in a [`ResourceRegistry`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FenceHandle(pub Slot);

/// Packs a [`Slot`] into an opaque `u64`: generation in the high 32 bits, index
/// in the low 32 bits.
///
/// The check is a **release-present `assert!`** (NOT `debug_assert!`, plan
/// C2/D6): a vanished check would silently truncate a `> 2^32` index and alias a
/// live handle, defeating the ABA guarantee. The registry treats the index
/// domain as capped at `u32::MAX` — a documented hard limit of 2^32 live
/// resources per kind, orders of magnitude above any real device-resource count.
#[inline]
pub fn slot_to_u64(s: Slot) -> u64 {
    assert!(
        s.index() <= u32::MAX as usize,
        "invariant: RHI resource index exceeds u32 handle domain"
    );
    ((s.generation() as u64) << 32) | (s.index() as u64)
}

/// Unpacks an opaque `u64` (produced by [`slot_to_u64`]) back into a [`Slot`].
///
/// Inverse of [`slot_to_u64`]: low 32 bits → index, high 32 bits → generation.
#[inline]
pub fn u64_to_slot(h: u64) -> Slot {
    Slot::new((h & 0xFFFF_FFFF) as usize, (h >> 32) as Generation)
}

/// A per-resource-kind generational allocator + storage.
///
/// `SparseSlotMap` exposes `create_slot(index) + insert(slot, value)` but no
/// "allocate a free index and push" primitive, so this wrapper owns the
/// free-index discipline: a `free` stack of reclaimed external indices plus a
/// `watermark` for never-before-used indices. ABA safety lives in the underlying
/// map (it bumps the generation on `remove`); this layer only chooses *which*
/// external index a fresh allocation reuses.
struct Kind<U> {
    map: SparseSlotMap<U>,
    /// External indices freed by `take`, available for reuse (LIFO).
    free: Vec<usize>,
    /// The next never-allocated external index.
    watermark: usize,
}

impl<U> Kind<U> {
    #[inline]
    fn new() -> Self {
        Self {
            map: SparseSlotMap::new(),
            free: Vec::new(),
            watermark: 0,
        }
    }

    /// Allocates a fresh external index (reusing a freed one if available) and
    /// inserts `value`, returning the generational [`Slot`].
    #[inline]
    fn register(&mut self, value: U) -> Slot {
        let index = match self.free.pop() {
            Some(reused) => reused,
            None => {
                let next = self.watermark;
                self.watermark += 1;
                next
            }
        };
        // `create_slot` returns the generation the next allocation at `index`
        // must carry (0 if pristine, the bumped successor if a tombstone), so the
        // insert always succeeds and the returned Slot is the live handle.
        let slot = self.map.create_slot(index);
        let replaced = self.map.insert(slot, value);
        debug_assert!(
            replaced.is_none(),
            "invariant: a freshly-allocated index must not already hold a value"
        );
        slot
    }

    #[inline]
    fn resolve(&self, slot: Slot) -> Option<&U> {
        self.map.get(slot)
    }

    /// Removes and returns the value for `slot` (if live), reclaiming its index.
    #[inline]
    fn take(&mut self, slot: Slot) -> Option<U> {
        let value = self.map.remove(slot)?;
        // The map bumped the generation on remove, so the reclaimed index is safe
        // to hand back out — a stale handle to the old generation will not match.
        self.free.push(slot.index());
        Some(value)
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Owns every RHI resource behind a generational handle, one [`SparseSlotMap`]
/// per kind (plan D6).
///
/// Generic over the concrete backend `A` so every map stores the backend's own
/// owned type (`SparseSlotMap<A::Buffer>` etc.) — monomorphized, no erasure.
/// `resolve_*` is the ~3 ns generation-checked lookup; `register_*`/`take_*` are
/// the allocate/free pair.
///
/// # Teardown
/// The owned resources cannot self-`Drop` (a backend resource needs `&Device` to
/// be destroyed), so the owner MUST call [`ResourceRegistry::destroy_all`] before
/// dropping the registry — a structural, release-present teardown step (plan W4).
/// A `debug_assert!`-every-map-empty on `Drop` is a secondary leak tripwire, not
/// the primary guard.
pub struct ResourceRegistry<A: RhiApi> {
    buffers: Kind<A::Buffer>,
    pipelines: Kind<A::ComputePipeline>,
    shaders: Kind<A::ShaderModule>,
    fences: Kind<A::Fence>,
}

impl<A: RhiApi> Default for ResourceRegistry<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: RhiApi> ResourceRegistry<A> {
    /// Creates an empty registry.
    #[inline]
    pub fn new() -> Self {
        Self {
            buffers: Kind::new(),
            pipelines: Kind::new(),
            shaders: Kind::new(),
            fences: Kind::new(),
        }
    }

    /// Registers an owned buffer, returning its handle.
    #[inline]
    pub fn register_buffer(&mut self, buffer: A::Buffer) -> BufferHandle {
        BufferHandle(self.buffers.register(buffer))
    }

    /// Resolves a buffer handle to a borrow, or `None` if stale/destroyed.
    #[inline]
    pub fn resolve_buffer(&self, handle: BufferHandle) -> Option<&A::Buffer> {
        self.buffers.resolve(handle.0)
    }

    /// Removes a buffer (for explicit destroy), returning the owned value.
    #[inline]
    pub fn take_buffer(&mut self, handle: BufferHandle) -> Option<A::Buffer> {
        self.buffers.take(handle.0)
    }

    /// Registers an owned compute pipeline, returning its handle.
    #[inline]
    pub fn register_compute_pipeline(
        &mut self,
        pipeline: A::ComputePipeline,
    ) -> ComputePipelineHandle {
        ComputePipelineHandle(self.pipelines.register(pipeline))
    }

    /// Resolves a compute-pipeline handle to a borrow, or `None` if stale.
    #[inline]
    pub fn resolve_compute_pipeline(
        &self,
        handle: ComputePipelineHandle,
    ) -> Option<&A::ComputePipeline> {
        self.pipelines.resolve(handle.0)
    }

    /// Removes a compute pipeline (for explicit destroy).
    #[inline]
    pub fn take_compute_pipeline(
        &mut self,
        handle: ComputePipelineHandle,
    ) -> Option<A::ComputePipeline> {
        self.pipelines.take(handle.0)
    }

    /// Registers an owned shader module, returning its handle.
    #[inline]
    pub fn register_shader(&mut self, shader: A::ShaderModule) -> ShaderHandle {
        ShaderHandle(self.shaders.register(shader))
    }

    /// Resolves a shader handle to a borrow, or `None` if stale.
    #[inline]
    pub fn resolve_shader(&self, handle: ShaderHandle) -> Option<&A::ShaderModule> {
        self.shaders.resolve(handle.0)
    }

    /// Removes a shader module (for explicit destroy).
    #[inline]
    pub fn take_shader(&mut self, handle: ShaderHandle) -> Option<A::ShaderModule> {
        self.shaders.take(handle.0)
    }

    /// Registers an owned fence, returning its handle.
    #[inline]
    pub fn register_fence(&mut self, fence: A::Fence) -> FenceHandle {
        FenceHandle(self.fences.register(fence))
    }

    /// Resolves a fence handle to a borrow, or `None` if stale.
    #[inline]
    pub fn resolve_fence(&self, handle: FenceHandle) -> Option<&A::Fence> {
        self.fences.resolve(handle.0)
    }

    /// Removes a fence (for explicit destroy).
    #[inline]
    pub fn take_fence(&mut self, handle: FenceHandle) -> Option<A::Fence> {
        self.fences.take(handle.0)
    }

    /// Destroys every owned resource through `device`, in reverse resource-order,
    /// after waiting for the GPU to go idle (plan W4).
    ///
    /// Mirrors the existing `ComputeHarness::drop` / `Renderer::drop` discipline:
    /// `wait_idle` first, then destroy in reverse dependency order
    /// (fences → pipelines → shaders → buffers). This is a structural,
    /// release-present teardown step the owner must call.
    ///
    /// `&mut self` because draining the maps mutates them; after this returns,
    /// every map is empty.
    pub fn destroy_all(&mut self, device: &A::Device) {
        // Belt-and-braces: ensure no submission is still touching a resource
        // before any destroy. Errors here are non-recoverable at teardown; the
        // backend logs them — we proceed to free regardless to avoid leaking.
        let _ = device.wait_idle();

        // Reverse resource-order teardown. Each `take` reclaims the index and
        // the bumped generation makes the freed handle permanently stale.
        drain_kind(&mut self.fences, |fence| {
            // SAFETY: `wait_idle` above guarantees the GPU is no longer using
            // `fence`; the `take` removes it from the map so it is destroyed
            // exactly once (no other owner can reach it).
            unsafe { device.destroy_fence(fence) }
        });
        drain_kind(&mut self.pipelines, |pipeline| {
            // SAFETY: `wait_idle` above guarantees no submission using `pipeline`
            // is pending; `take` ensures it is destroyed exactly once.
            unsafe { device.destroy_compute_pipeline(pipeline) }
        });
        drain_kind(&mut self.shaders, |module| {
            // SAFETY: `wait_idle` above guarantees no pipeline referencing
            // `module` is in flight; `take` ensures it is destroyed exactly once.
            unsafe { device.destroy_shader_module(module) }
        });
        drain_kind(&mut self.buffers, |buffer| {
            // SAFETY: `wait_idle` above guarantees the GPU is no longer using
            // `buffer`; `take` ensures it is destroyed exactly once.
            unsafe { device.destroy_buffer(buffer) }
        });

        debug_assert!(
            self.fences.is_empty()
                && self.pipelines.is_empty()
                && self.shaders.is_empty()
                && self.buffers.is_empty(),
            "invariant: every resource map must be empty after destroy_all"
        );
    }
}

impl<A: RhiApi> Drop for ResourceRegistry<A> {
    fn drop(&mut self) {
        // Secondary leak tripwire (plan W4): a non-empty map on drop means the
        // owner skipped `destroy_all`, leaking GPU resources. The structural
        // guard is the required `destroy_all` call, not this assert.
        debug_assert!(
            self.fences.is_empty()
                && self.pipelines.is_empty()
                && self.shaders.is_empty()
                && self.buffers.is_empty(),
            "invariant: ResourceRegistry dropped with live resources — call destroy_all first"
        );
    }
}

/// Drains every live entry of `kind`, invoking `destroy` (which consumes the
/// owned value) on each, leaving the kind's map empty.
///
/// Pulled out as a free function so the four `destroy_all` arms share one loop
/// without a closure borrowing `self`. Walks indices `[0, watermark)`; the map's
/// `create_slot(index)` recovers the live generation so `take` resolves any live
/// occupant and skips freed/never-used indices via the `Option` return.
#[inline]
fn drain_kind<U>(kind: &mut Kind<U>, mut destroy: impl FnMut(U)) {
    for index in 0..kind.watermark {
        let slot = kind.map.create_slot(index);
        if let Some(value) = kind.take(slot) {
            destroy(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{BufferDesc, ComputePipelineDesc};
    use crate::device::RhiDevice;
    use crate::encoder::RhiCommandEncoder;
    use crate::enums::ShaderStage;
    use crate::error::RhiError;
    use crate::queue::RhiQueue;

    // ===== Packing-bridge tests =====

    fn assert_round_trip(index: usize, generation: Generation) {
        let s = Slot::new(index, generation);
        let packed = slot_to_u64(s);
        // Index lands in the low 32 bits, generation in the high 32 bits.
        assert_eq!(packed & 0xFFFF_FFFF, index as u64, "index must occupy low 32");
        assert_eq!(packed >> 32, generation as u64, "generation must occupy high 32");
        assert_eq!(u64_to_slot(packed), s, "round-trip must be the identity");
    }

    #[test]
    fn pack_round_trip_boundaries() {
        assert_round_trip(0, 0);
        assert_round_trip(0, u32::MAX);
        assert_round_trip(u32::MAX as usize, 0);
        assert_round_trip(u32::MAX as usize, u32::MAX);
    }

    #[test]
    #[should_panic(expected = "exceeds u32 handle domain")]
    fn pack_index_over_domain_panics() {
        // The release-present assert must reject an index that would truncate.
        let over = Slot::new((u32::MAX as usize) + 1, 0);
        let _ = slot_to_u64(over);
    }

    // ===== Mock backend (proves the trait surface is implementable, no GPU) =====

    /// Zero-sized backend marker for the registry tests.
    struct MockApi;

    /// A trivial device whose `destroy_*` are no-ops (the test resources are
    /// plain `u32`s, so there is nothing to free).
    struct MockDevice;

    impl RhiApi for MockApi {
        type Device = MockDevice;
        type Queue = MockQueue;
        type CommandEncoder = MockEncoder;
        type Buffer = u32;
        type ShaderModule = u32;
        type ComputePipeline = u32;
        type Fence = u32;
        type Surface = ();
        type Swapchain = ();
        type Semaphore = ();
        type Texture = ();
        type Sampler = ();
        type GraphicsPipeline = ();
        type BindGroup = ();
        type BindGroupLayout = ();
    }

    impl RhiDevice<MockApi> for MockDevice {
        type Error = RhiError;

        fn create_buffer(&self, _desc: &BufferDesc) -> Result<u32, RhiError> {
            Ok(0)
        }
        unsafe fn destroy_buffer(&self, _buffer: u32) {}
        fn buffer_mapped_ptr(&self, _buffer: &u32) -> Option<core::ptr::NonNull<u8>> {
            None
        }
        fn create_shader_module(&self, _spirv: &[u32]) -> Result<u32, RhiError> {
            Ok(0)
        }
        unsafe fn destroy_shader_module(&self, _module: u32) {}
        fn create_compute_pipeline(
            &self,
            _desc: &ComputePipelineDesc<MockApi>,
        ) -> Result<u32, RhiError> {
            Ok(0)
        }
        unsafe fn destroy_compute_pipeline(&self, _pipeline: u32) {}
        fn create_fence(&self, _signaled: bool) -> Result<u32, RhiError> {
            Ok(0)
        }
        unsafe fn destroy_fence(&self, _fence: u32) {}
        fn wait_fence(&self, _fence: &u32, _timeout_ns: u64) -> Result<(), RhiError> {
            Ok(())
        }
        fn reset_fence(&self, _fence: &u32) -> Result<(), RhiError> {
            Ok(())
        }
        fn create_command_encoder(&self) -> Result<MockEncoder, RhiError> {
            Ok(MockEncoder)
        }
        unsafe fn destroy_command_encoder(&self, _enc: MockEncoder) {}
        fn wait_idle(&self) -> Result<(), RhiError> {
            Ok(())
        }
    }

    struct MockQueue;

    impl RhiQueue<MockApi> for MockQueue {
        type Error = RhiError;
        fn submit(&self, _encoder: &MockEncoder, _signal_fence: &u32) -> Result<(), RhiError> {
            Ok(())
        }
    }

    struct MockEncoder;

    impl RhiCommandEncoder<MockApi> for MockEncoder {
        type Error = RhiError;
        fn begin(&mut self) -> Result<(), RhiError> {
            Ok(())
        }
        fn end(&mut self) -> Result<(), RhiError> {
            Ok(())
        }
        fn bind_compute_pipeline(&mut self, _pipeline: &u32) {}
        fn bind_storage_buffer(&mut self, _buffer: &u32, _set: u32, _binding: u32) {}
        fn push_constants(&mut self, _stage: ShaderStage, _offset: u32, _bytes: &[u8]) {}
        fn dispatch(&mut self, _gx: u32, _gy: u32, _gz: u32) {}
        fn pipeline_barrier(&mut self, _barrier: &crate::descriptor::BarrierDesc<MockApi>) {}
    }

    // ===== Registry behavioral tests =====

    #[test]
    fn register_resolve_take_stale_then_destroy_all() {
        let device = MockDevice;
        let mut reg: ResourceRegistry<MockApi> = ResourceRegistry::new();

        let h = reg.register_buffer(7);
        assert_eq!(reg.resolve_buffer(h), Some(&7), "register then resolve");

        let taken = reg.take_buffer(h);
        assert_eq!(taken, Some(7), "take returns the owned value");
        assert_eq!(reg.resolve_buffer(h), None, "resolving a taken handle is None");

        // ABA: re-register the same index, the OLD handle must stay stale.
        let h2 = reg.register_buffer(99);
        assert_eq!(h2.0.index(), h.0.index(), "the freed index is reused");
        assert_ne!(h2.0.generation(), h.0.generation(), "generation is bumped");
        assert_eq!(reg.resolve_buffer(h), None, "stale handle still resolves None");
        assert_eq!(reg.resolve_buffer(h2), Some(&99), "fresh handle resolves");

        reg.destroy_all(&device);
        assert_eq!(reg.resolve_buffer(h2), None, "destroy_all empties the map");
    }

    #[test]
    fn all_kinds_register_and_resolve() {
        let mut reg: ResourceRegistry<MockApi> = ResourceRegistry::new();
        let b = reg.register_buffer(1);
        let p = reg.register_compute_pipeline(2);
        let s = reg.register_shader(3);
        let f = reg.register_fence(4);
        assert_eq!(reg.resolve_buffer(b), Some(&1));
        assert_eq!(reg.resolve_compute_pipeline(p), Some(&2));
        assert_eq!(reg.resolve_shader(s), Some(&3));
        assert_eq!(reg.resolve_fence(f), Some(&4));

        let device = MockDevice;
        reg.destroy_all(&device);
    }

    #[test]
    fn handle_packs_through_u64_bridge() {
        let mut reg: ResourceRegistry<MockApi> = ResourceRegistry::new();
        let h = reg.register_buffer(42);
        // A handle's Slot survives the opaque-u64 round-trip the Phase-4 core
        // seam will use.
        let packed = slot_to_u64(h.0);
        assert_eq!(u64_to_slot(packed), h.0);
        assert_eq!(reg.resolve_buffer(BufferHandle(u64_to_slot(packed))), Some(&42));

        let device = MockDevice;
        reg.destroy_all(&device);
    }

    #[test]
    fn randomized_sequence_never_aliases_or_resolves_stale() {
        // Deterministic LCG (Numerical Recipes constants) — no time-based seed.
        let mut state: u64 = 0x1234_5678_9abc_def1;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };

        let mut reg: ResourceRegistry<MockApi> = ResourceRegistry::new();
        // Track the handles we believe are live + their expected value, plus a
        // graveyard of handles we have taken (must always resolve None).
        let mut live: Vec<(BufferHandle, u32)> = Vec::new();
        let mut dead: Vec<BufferHandle> = Vec::new();
        let mut value_counter: u32 = 0;

        for _ in 0..5_000 {
            // Every dead handle must stay stale on every iteration.
            for d in &dead {
                assert_eq!(reg.resolve_buffer(*d), None, "dead handle must never resolve");
            }
            // Every live handle must resolve to its exact value (no aliasing).
            for (h, v) in &live {
                assert_eq!(reg.resolve_buffer(*h), Some(v), "live handle must resolve its value");
            }

            let op = next() % 3;
            if op == 0 || live.is_empty() {
                // Register.
                value_counter = value_counter.wrapping_add(1);
                let v = value_counter;
                let h = reg.register_buffer(v);
                // A freshly minted handle must not equal any dead handle.
                assert!(!dead.contains(&h), "a reused index must carry a fresh generation");
                live.push((h, v));
            } else {
                // Take a pseudo-random live handle.
                let idx = (next() as usize) % live.len();
                let (h, v) = live.swap_remove(idx);
                assert_eq!(reg.take_buffer(h), Some(v), "take returns the live value");
                dead.push(h);
            }
        }

        // Tear down whatever survived.
        let device = MockDevice;
        reg.destroy_all(&device);
    }
}
