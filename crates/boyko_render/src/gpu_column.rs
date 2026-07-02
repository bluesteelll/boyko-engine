//! The GPU-resident column manager + the `!Send` RHI context that owns it.
//!
//! Phase 5 Wave B mints REAL device-local component pools: [`GpuColumnManager`]
//! allocates a `DeviceLocal` (VRAM) buffer through the [`boyko_rhi`] registry,
//! packs its generational `Slot` into the opaque graphics-pure
//! [`DeviceColumnHandle`] the `boyko_ecs` core stores, and drives the A2 seam
//! [`Archetype::make_component_device_backed`](boyko_ecs::ecs::core::archetype::archetype::Archetype::make_component_device_backed) to flip the CPU pool to
//! device-backing (and null its column cache — the C1 fix that keeps the dangling
//! Host base CPU-unreachable).
//!
//! # The handle indirection (D1 / MF-7)
//!
//! The core stores ONLY a `u64`. The manager owns the registry (the `Slot ↔
//! BoundBuffer` map) plus a side [`GpuColumnMeta`] table keyed by the stable
//! `(ArchetypeId, ComponentId)` pair (NOT the registry buffer-slot index, which is
//! shared with staging and churns across grows — the X1/X2 fix). A grow reallocs
//! the device buffer and mints a NEW handle; the OLD `u64` resolves loudly to
//! `None` because the registry bumps the freed slot's generation (MF-7
//! take-before-register), and the pair-keyed table is upserted in place so exactly
//! one entry survives. No CPU code ever caches a device row pointer.
//!
//! # Single-thread discipline (`!Send`)
//!
//! [`RhiContext`] is `!Send + !Sync` (it owns the [`VulkanContext`], which is) and
//! `impl boyko_ecs::NonSendResource`. The orphan rule forces this impl to live
//! here (neither `NonSendResource` nor `VulkanContext` is local to the other
//! crate). For Wave B the context simply exists and owns the manager; the
//! dispatcher-side projection lands with `GpuSystem` in Wave C.

use boyko_ecs::ecs::core::component::component_registry::{self, ResidencyKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_ecs::ecs::memory::device_column::DeviceColumnHandle;

use boyko_rhi::{
    BarrierDesc, BufferBarrier, BufferCopy, BufferDesc, BufferHandle, BufferUsage,
    ComputePipelineDesc, ComputePipelineHandle, MemoryLocation, ResourceRegistry,
    RhiCommandEncoder, RhiDevice, RhiQueue, ShaderHandle, ShaderStage, slot_to_u64, u64_to_slot,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::rhi_impl::Vulkan;
use boyko_rhi_vulkan::swapchain::FrameWriteToken;

use crate::barrier::PlannedBarrier;
use crate::error::GpuColumnError;
use crate::ui::instance::{UiInstance, UiOrtho};
use crate::ui::plan::UiFramePlan;
use crate::ui::resources::UiRenderResources;

/// The `local_size_x` of the `gpu_integrate` compute shader (`[numthreads(64,1,1)]`).
///
/// The dispatch group count along X is `ceil(row_count / LOCAL_SIZE_X)`; the
/// shader's `if (i >= count) return;` bounds check absorbs the tail of a
/// non-multiple row count.
pub const LOCAL_SIZE_X: u32 = 64;

/// A `boyko_render`-side record for one device-resident column (cold, side-table).
///
/// `#[repr(C)]` for a stable, predictable layout (~32 B). The durable identity of
/// a column is the `(archetype, component)` pair (MF-7), NOT its `handle`, which a
/// grow rotates. Stored in `GpuColumnManager::meta` as a flat, pair-keyed
/// association — exactly ONE entry per `(archetype, component)`, looked up by the
/// pair (never by the registry buffer-slot index, which is shared with staging and
/// churns across grows — the X1/X2 fix).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuColumnMeta {
    /// The CURRENT opaque device-column handle (rotated on a grow).
    pub handle: DeviceColumnHandle,
    /// Byte stride of one component row (the element size).
    pub stride: u32,
    /// Device-side live row count (the device twin of `ComponentPool::len`).
    pub device_len: u32,
    /// Device-side committed row capacity (rows the device buffer can hold).
    pub device_cap: u32,
    /// The archetype this column belongs to (the durable key, part 1).
    pub archetype: ArchetypeId,
    /// The component this column stores (the durable key, part 2).
    pub component: ComponentId,
}

/// The result of a [`GpuColumnManager::resolve`] — the current handle plus its
/// row geometry. A pure-POD copy of the parts of [`GpuColumnMeta`] a frame-path
/// caller needs, so the borrow of the manager ends at the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedColumn {
    /// The CURRENT device-column handle for the resolved key.
    pub handle: DeviceColumnHandle,
    /// Byte stride of one component row.
    pub stride: u32,
    /// Device-side live row count.
    pub device_len: u32,
    /// Device-side committed row capacity.
    pub device_cap: u32,
}

/// The device handle behind [`RhiContext`] — owned or shared (host plan R2,
/// critic delta A2).
///
/// - [`Owned`](DeviceHandle::Owned): today's semantics, verbatim — the context
///   owns the [`VulkanContext`] and its drop (after `destroy_all` in
///   [`RhiContext`]'s `Drop`) destroys the device / instance / loader.
/// - [`Shared`](DeviceHandle::Shared): the process-singleton mode — the context
///   borrows the `&'static` handle pinned by
///   [`VulkanContext::boot_singleton`]; dropping the variant drops only the
///   reference, NEVER the device (the host ends the device's lifecycle via
///   `VulkanContext::destroy_singleton`).
///
/// The discriminant is touched only on setup/teardown paths ([`RhiContext::new`]
/// / [`RhiContext::from_shared`] / `Drop`); frame paths go through the borrowed
/// `&VulkanContext` that [`get`](DeviceHandle::get) returns, so the split costs
/// the hot path nothing.
// The size skew (`VulkanContext` ~1 KB vs a `&'static`) is irrelevant here:
// exactly ONE `RhiContext` exists per world (a singleton NonSend resource,
// never an array element), and boxing `Owned` would both deviate from the
// pinned "owned mode keeps today's semantics verbatim" contract (critic delta
// A2) and add a pointer chase to every owned-mode device access.
#[allow(clippy::large_enum_variant)]
enum DeviceHandle {
    /// The context owns the device; dropping it destroys the device.
    Owned(VulkanContext),
    /// The context shares the pinned process-singleton device; dropping it
    /// drops only the reference.
    Shared(&'static VulkanContext),
}

impl DeviceHandle {
    /// Borrows the device regardless of mode — one predictable match on the
    /// discriminant, used by setup/teardown paths; hot paths already hold the
    /// resulting `&VulkanContext`.
    #[inline]
    fn get(&self) -> &VulkanContext {
        match self {
            DeviceHandle::Owned(ctx) => ctx,
            DeviceHandle::Shared(ctx) => ctx,
        }
    }
}

/// The concrete `!Send` RHI handle the dispatcher reaches (MF-5).
///
/// Holds the [`VulkanContext`] (device + the validation messenger) — owned or
/// shared, see below — and the [`GpuColumnManager`]. `impl `[`NonSendResource`]:
/// the orphan rule REQUIRES this impl to live in `boyko_render` (neither trait
/// nor [`VulkanContext`] is local to the other crate). For Wave B it just exists
/// and owns the manager; Wave C's `GpuSystem` projects it off the world on the
/// dispatcher to record + submit.
///
/// # Device-ownership modes (host plan R2)
///
/// - **Owned** ([`new`](Self::new)): the pre-R2 semantics, byte-for-byte —
///   `Drop` runs `destroy_all` (columns + UI resources), then the owned
///   [`VulkanContext`] field drops, destroying the device / instance / loader.
/// - **Shared** ([`from_shared`](Self::from_shared)): the world-resident side of
///   the leaked `&'static` device singleton — `Drop` runs `destroy_all` but
///   NEVER touches the device lifecycle; the host ends it with
///   [`VulkanContext::destroy_singleton`] after evicting this resource.
///
/// `!Send + !Sync` in both modes by [`VulkanContext`]'s own raw-pointer fields —
/// the RHI is touched only on the owning (dispatcher) thread (§5.3).
pub struct RhiContext {
    /// The Vulkan device + queue origin + validation messenger (owned or
    /// shared — see the type-level mode split).
    context: DeviceHandle,
    /// The device-column manager (registry + meta + staging).
    manager: GpuColumnManager,
    /// The owned UI render capability (GUI P5a Decision 8): the UI pipeline +
    /// bind-group layout + per-FIF host-mapped rings + bind-groups. `None` until
    /// [`ui_setup`](Self::ui_setup) builds it; torn down (and re-`None`d) by
    /// [`destroy_all`](Self::destroy_all) + [`Drop`], so it is idempotent and never
    /// leaks past `RhiContext::Drop` (a NAMED owner, not a side store — Principle 0).
    ui: Option<UiRenderResources>,
}

impl RhiContext {
    /// Wraps a booted [`VulkanContext`] + a fresh [`GpuColumnManager`] in OWNED
    /// mode: teardown semantics are byte-for-byte the pre-R2 ones — `Drop` runs
    /// `destroy_all`, then the owned context drops (destroying the device).
    #[inline]
    pub fn new(context: VulkanContext) -> Self {
        Self {
            context: DeviceHandle::Owned(context),
            manager: GpuColumnManager::new(),
            ui: None,
        }
    }

    /// Wraps the pinned process-singleton device + a fresh [`GpuColumnManager`]
    /// in SHARED mode (host plan R2): `Drop` runs `destroy_all` (frees every
    /// column / UI resource) but NEVER touches the device lifecycle — the host
    /// runner ends it with [`VulkanContext::destroy_singleton`] AFTER evicting
    /// this resource from the world.
    #[inline]
    pub fn from_shared(ctx: &'static VulkanContext) -> Self {
        Self {
            context: DeviceHandle::Shared(ctx),
            manager: GpuColumnManager::new(),
            ui: None,
        }
    }

    /// Borrows the [`VulkanContext`] (the RHI device origin), whichever mode
    /// holds it.
    #[inline]
    pub fn context(&self) -> &VulkanContext {
        self.context.get()
    }

    /// Borrows the device-column manager.
    #[inline]
    pub fn manager(&self) -> &GpuColumnManager {
        &self.manager
    }

    /// Mutably borrows the device-column manager (the setup-path entry point).
    #[inline]
    pub fn manager_mut(&mut self) -> &mut GpuColumnManager {
        &mut self.manager
    }

    /// Splits into `(&VulkanContext, &mut GpuColumnManager)` so a manager call can
    /// take the device by reference while mutating the manager — the borrow split
    /// the setup/grow/upload paths need.
    #[inline]
    pub fn split_mut(&mut self) -> (&VulkanContext, &mut GpuColumnManager) {
        (self.context.get(), &mut self.manager)
    }

    /// SETUP-only: builds a compute pipeline from `spirv` (registered in the owned
    /// manager's registry) and returns its handle (Wave C).
    ///
    /// Convenience over `self.manager.create_compute_pipeline(self.context.get(), …)`:
    /// the `GpuSystem` setup path holds the projected `&mut RhiContext` and creates
    /// its `gpu_integrate` pipeline once through this.
    ///
    /// # Errors
    /// [`GpuColumnError::Rhi`] if shader-module / pipeline creation fails.
    #[inline]
    pub fn create_compute_pipeline(
        &mut self,
        spirv: &[u32],
    ) -> Result<ComputePipelineHandle, GpuColumnError> {
        self.manager.create_compute_pipeline(self.context.get(), spirv)
    }

    /// SETUP-only (GUI P5a Rung 3, Decision 8): builds the owned UI render
    /// capability — the UI graphics pipeline (for `color_format`, blend =
    /// premultiplied), the SSBO bind-group layout (VERTEX|FRAGMENT), and the per-FIF
    /// host-mapped grow-only STORAGE rings + bind-groups — once. Forwards every
    /// device verb through `split_mut().0` (the `&VulkanContext` device), mirroring
    /// [`create_compute_pipeline`](Self::create_compute_pipeline).
    ///
    /// `color_format` is the format of the image the UI pass renders into (the
    /// swapchain surface format for the on-screen path, `R8G8B8A8Unorm` for the
    /// offscreen golden — Decision 9). `spirv_vs`/`spirv_fs` are the committed
    /// `ui_rect.{vs,fs}.spv` word streams; `initial_rows` each ring's starting
    /// `UiInstance` capacity (grows pow2 on overflow). `font` is the loaded `.bfont`
    /// (GUI P5b): its MTSDF atlas is uploaded ONCE here as a sampled image + the
    /// per-atlas UBO, then sampled by every glyph (`FLAG_TEXT`) on the shared draw.
    ///
    /// Calling it twice tears down the prior resources first (idempotent setup), so
    /// a swapchain recreate that re-runs setup never leaks.
    ///
    /// # Errors
    /// [`GpuColumnError`] on any shader / pipeline / layout / buffer / bind-group
    /// create failure (every partially-created resource is torn down before return).
    pub fn ui_setup(
        &mut self,
        color_format: boyko_rhi::Format,
        spirv_vs: &[u32],
        spirv_fs: &[u32],
        initial_rows: u32,
        font: &boyko_fontbake::atlas::BakedFont,
    ) -> Result<(), GpuColumnError> {
        // Idempotent re-setup: drop the prior capability (its own `destroy` drains
        // the device + frees every resource) before building anew.
        if let Some(old) = self.ui.take() {
            old.destroy(self.context.get());
        }
        let (device, _manager) = self.split_mut();
        let resources = UiRenderResources::create(
            device,
            color_format,
            spirv_vs,
            spirv_fs,
            initial_rows,
            font,
        )?;
        self.ui = Some(resources);
        Ok(())
    }

    /// Frame path (GUI P5a Rung 3 / A1 steps 4-6): ensures slot `token.slot()`'s
    /// capacity (grow pow2 + rebuild that slot's bind-group on overflow), memcpys the
    /// packed `instances` into the mapped current-FIF ring, and returns the by-value
    /// [`UiFramePlan`] (no borrow escapes — Decision 9).
    ///
    /// `instances` is the packed, z-sorted scratch; its byte image is uploaded via
    /// the no-bytemuck POD view. `ortho` is the pixel→NDC transform for the swapchain
    /// extent the UI pass renders into. `token` is the per-slot write proof
    /// (`Renderer::wait_frame_in_flight`, or [`FrameWriteToken::forge_unfenced`] at
    /// setup time), BORROWED (R0b): this is a mid-frame write, so the caller keeps
    /// the token and later feeds it BY VALUE to the frame-ending submit
    /// (`Renderer::present_sampled`). The memcpy targets `token.slot()` and cannot
    /// be issued for a slot whose in-flight fence was not waited. The returned plan
    /// borrows NO RHI handle,
    /// so it is sound to stash across the dispatcher token drop; the swapchain
    /// recorder re-resolves the pipeline + bind-group by `frame_index` via
    /// [`ui_handles`](Self::ui_handles) (MF-7).
    ///
    /// # Errors
    /// [`GpuColumnError`] on a grow failure, a missing ring mapping, or if
    /// [`ui_setup`](Self::ui_setup) was never called.
    pub fn ui_upload(
        &mut self,
        instances: &[UiInstance],
        ortho: UiOrtho,
        token: &FrameWriteToken,
    ) -> Result<UiFramePlan, GpuColumnError> {
        let frame_index = token.slot();
        let instance_count = instances.len() as u32;
        let packed = UiInstance::slice_as_bytes(instances);
        // Disjoint-field split: borrow the device (`context`) immutably while
        // mutating the UI sub-owner (`ui`). A single struct destructure lets the
        // borrow checker see the two fields are disjoint (the same shape as
        // `split_mut`, which the manager paths use).
        let Self { context, ui, .. } = self;
        let ui = ui.as_mut().ok_or(GpuColumnError::StagingNotMapped)?;
        ui.upload(context.get(), packed, instance_count, frame_index)?;
        Ok(UiFramePlan {
            instance_count,
            ortho,
            frame_index,
        })
    }

    /// Re-resolves the current-FIF UI pipeline + bind-group by `frame_index` (MF-7)
    /// — used by the swapchain recorder in the SAME dispatcher window. Never a cached
    /// raw handle, so a grow that rebuilt slot `frame_index`'s bind-group between
    /// [`ui_upload`](Self::ui_upload) and the draw is transparent.
    ///
    /// Returns `None` if [`ui_setup`](Self::ui_setup) was never called.
    #[inline]
    pub fn ui_handles(
        &self,
        frame_index: usize,
    ) -> Option<(
        &boyko_rhi_vulkan::rhi_impl::VulkanGraphicsPipeline,
        &boyko_rhi_vulkan::rhi_impl::VulkanBindGroup,
    )> {
        self.ui.as_ref().map(|ui| ui.handles(frame_index))
    }

    /// Builds the concrete swapchain [`UiPass`](boyko_rhi_vulkan::swapchain::UiPass)
    /// the on-screen recorder records — the host-path linchpin that ties the stashed
    /// POD [`UiFramePlan`] back to live, current-frame-re-resolved (MF-7) RHI handles.
    ///
    /// The render host, after stashing `plan` from [`ui_upload`](Self::ui_upload),
    /// calls this in the SAME dispatcher window and passes the result to
    /// [`Renderer::present_sampled`](boyko_rhi_vulkan::swapchain::Renderer::present_sampled)
    /// as `Some(&pass)`. The pipeline + bind-group are re-resolved by
    /// `plan.frame_index` (never a cached raw handle), so a grow that rebuilt that
    /// slot's bind-group between upload and draw is transparent; the ortho byte view
    /// borrows `plan`, so the returned `UiPass` borrows BOTH `self` and `plan` and is
    /// dropped before either (the recorder uses it within the same frame).
    ///
    /// Returns `None` if [`ui_setup`](Self::ui_setup) was never called (no UI pass to
    /// record this frame).
    #[inline]
    pub fn ui_pass<'a>(
        &'a self,
        plan: &'a UiFramePlan,
    ) -> Option<boyko_rhi_vulkan::swapchain::UiPass<'a>> {
        let (pipeline, bind_group) = self.ui_handles(plan.frame_index)?;
        Some(boyko_rhi_vulkan::swapchain::UiPass {
            pipeline,
            bind_group,
            instance_count: plan.instance_count,
            ortho_bytes: plan.ortho.as_bytes(),
        })
    }

    /// Records + submits the `gpu_integrate` compute dispatch on the column
    /// resolved indirectly by `(archetype, component)` (MF-7), REPLAYING the
    /// lowered `barriers` plan before the dispatch, fence-waiting before it
    /// returns. Returns `Ok(false)` if the column does not resolve (skip),
    /// `Ok(true)` on a recorded + waited dispatch.
    ///
    /// The frame-path entry point `GpuSystem::run_dispatcher` calls after
    /// projecting the `&mut RhiContext`. Each [`PlannedBarrier`] in `barriers` is
    /// resolved to its current device buffer by its durable key and recorded as a
    /// `vkCmdPipelineBarrier` into the dispatch encoder (Wave D).
    ///
    /// # Errors
    /// [`GpuColumnError`] on a stale pipeline handle or any RHI failure.
    #[inline]
    pub fn dispatch_compute(
        &self,
        pipeline: ComputePipelineHandle,
        archetype: ArchetypeId,
        component: ComponentId,
        barriers: &[PlannedBarrier],
    ) -> Result<bool, GpuColumnError> {
        self.manager
            .dispatch_compute(self.context.get(), pipeline, archetype, component, barriers)
    }

    /// TEST-ONLY (Phase 5 Wave E `sync_validation` oracle): records two
    /// `gpu_integrate` passes over the same column in ONE submit, with an optional
    /// barrier between them. Forwards to
    /// [`GpuColumnManager::dispatch_compute_twice_one_submit`](GpuColumnManager::dispatch_compute_twice_one_submit).
    ///
    /// # Errors
    /// [`GpuColumnError`] on a stale handle or any RHI failure.
    #[cfg(any(test, feature = "test-readback"))]
    #[inline]
    pub fn dispatch_compute_twice_one_submit(
        &self,
        pipeline: ComputePipelineHandle,
        archetype: ArchetypeId,
        component: ComponentId,
        barrier_between: bool,
    ) -> Result<bool, GpuColumnError> {
        self.manager.dispatch_compute_twice_one_submit(
            self.context.get(),
            pipeline,
            archetype,
            component,
            barrier_between,
        )
    }

    /// TEST-ONLY: the owned manager's device→host readback count (Phase 5 Wave E
    /// zero-readback oracle). Forwards to
    /// [`GpuColumnManager::readback_count`](GpuColumnManager::readback_count).
    #[cfg(any(test, feature = "test-readback"))]
    #[inline]
    pub fn readback_count(&self) -> u64 {
        self.manager.readback_count()
    }

    /// TEST-ONLY: zeroes the owned manager's readback counter. Forwards to
    /// [`GpuColumnManager::reset_readback_count`](GpuColumnManager::reset_readback_count).
    #[cfg(any(test, feature = "test-readback"))]
    #[inline]
    pub fn reset_readback_count(&self) {
        self.manager.reset_readback_count();
    }

    /// Tears down every device resource (forwards to the manager).
    ///
    /// IDEMPOTENT (FIX-C2): a second call on an already-drained manager is a
    /// harmless no-op (the registry is empty + a `wait_idle` on an idle device is
    /// benign), so the explicit call AND the [`Drop`] impl below can BOTH run
    /// without double-free. Calling it explicitly is no longer mandatory — the
    /// [`Drop`] impl guarantees teardown in production — but tests still call it to
    /// assert the drained state before drop.
    pub fn destroy_all(&mut self) {
        // Tear down the UI capability FIRST (it owns its own pipeline/rings outside
        // the manager's registry — Decision 8 leak fix). `take()` makes it idempotent:
        // a second `destroy_all`/`Drop` finds `None` and does nothing. Its `destroy`
        // drains the device + frees every UI resource before `self.context` drops.
        if let Some(ui) = self.ui.take() {
            ui.destroy(self.context.get());
        }
        self.manager.destroy_all(self.context.get());
    }
}

impl Drop for RhiContext {
    /// FIX-C2: the production teardown path. The ECS world's `NonSend` slab drops
    /// the [`RhiContext`] resource; without this impl `destroy_all` was only ever
    /// called in tests, so in production EVERY device buffer leaked (the
    /// `VulkanContext` field would drop first — freeing device memory + the device
    /// — then the registry's `Drop` would find live buffers → release leak / debug
    /// panic).
    ///
    /// `Drop::drop` runs BEFORE the struct's fields drop, so `self.context` is
    /// still alive here (the registry's `destroy_all` lifetime contract is
    /// satisfied). Both the UI capability teardown (`take()` ⇒ idempotent) and
    /// `manager.destroy_all` are idempotent, so a prior explicit `destroy_all()`
    /// call does not double-free (GUI P5a Decision 8: the UI rings/pipeline are
    /// owned OUTSIDE the manager, so without this they would leak past Drop).
    ///
    /// Mode split (host plan R2): after this body runs, the
    /// [`DeviceHandle::Owned`] field drop destroys the device / instance /
    /// loader — byte-for-byte the pre-R2 semantics — while the
    /// [`DeviceHandle::Shared`] field drop releases only the `&'static`
    /// reference and NEVER touches the device lifecycle (the host ends it with
    /// [`VulkanContext::destroy_singleton`]).
    fn drop(&mut self) {
        if let Some(ui) = self.ui.take() {
            ui.destroy(self.context.get());
        }
        self.manager.destroy_all(self.context.get());
    }
}

// SAFETY (no `unsafe`): `RhiContext` is `!Send + !Sync` automatically in BOTH
// device-ownership modes — `VulkanContext` holds raw `*const DeviceFns` pointers
// (so it is neither `Send` nor `Sync`), the `DeviceHandle::Owned` variant embeds
// it by value, and the `DeviceHandle::Shared` variant holds a
// `&'static VulkanContext` (`&T` is `Send`/`Sync` only if `T: Sync`, which
// `VulkanContext` is not) — so the property propagates to `RhiContext` either
// way. The `NonSendResource` contract is exactly "only ever touched on the
// owning thread", which the `!Send` bound enforces structurally. No
// `unsafe impl` is added.
impl NonSendResource for RhiContext {}

/// Owns every device-resident column behind the [`boyko_rhi`] registry (D1).
///
/// - `registry`: the generational `Slot ↔ BoundBuffer` map over the [`Vulkan`]
///   backend. Resolving a handle is a generation-checked array index (~3 ns).
/// - `meta`: a flat side table keyed by the durable `(ArchetypeId, ComponentId)`
///   pair, NOT by the registry buffer-slot index (X1/X2). The buffer-slot index
///   space is SHARED with the staging buffer and churns across grows, so an
///   index-keyed table could let staging null a live column's entry or leave two
///   entries for one column. Keying by the pair makes the mapping single-valued by
///   construction: one entry per column, found by a cold linear scan over the tiny
///   GPU-resident set (Regime-C) — no per-frame `HashMap`/alloc.
/// - `staging` / `staging_cap`: ONE reused host-visible staging buffer for the
///   setup upload + the test-only readback, grown on demand. Registered in
///   `registry` so `destroy_all` frees it uniformly. Has NO `meta` entry — staging
///   is never a column, so it can never collide with a column's pair key.
pub struct GpuColumnManager {
    /// The generational device-buffer registry over the Vulkan backend.
    registry: ResourceRegistry<Vulkan>,
    /// Flat pair-keyed column table: exactly one [`GpuColumnMeta`] per live
    /// `(archetype, component)`. Searched by the pair (never by buffer-slot index).
    meta: Vec<GpuColumnMeta>,
    /// The reused host-visible staging buffer (registered in `registry`).
    staging: Option<BufferHandle>,
    /// Current staging-buffer capacity in bytes (0 when no staging exists).
    staging_cap: u64,
    /// TEST-ONLY: how many times [`readback_for_test`](Self::readback_for_test)
    /// has device→host copied since the last [`reset_readback_count`](Self::reset_readback_count).
    ///
    /// The zero-readback oracle (Phase 5 Wave E): a steady-state frame performs
    /// ZERO host readbacks, so this counter MUST stay 0 across every
    /// `Schedule::run` frame. Only the final test-oracle `readback_for_test`
    /// bumps it. Gated to test builds — it does not exist on the production frame
    /// path. `Relaxed`: the manager is `!Send` (touched only on its owning
    /// thread), so there is no cross-thread ordering to establish; the count is a
    /// plain single-threaded tally.
    #[cfg(any(test, feature = "test-readback"))]
    readback_count: core::sync::atomic::AtomicU64,
}

impl Default for GpuColumnManager {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuColumnManager {
    /// Creates an empty manager (no device buffers, no staging).
    #[inline]
    pub fn new() -> Self {
        Self {
            registry: ResourceRegistry::new(),
            meta: Vec::new(),
            staging: None,
            staging_cap: 0,
            #[cfg(any(test, feature = "test-readback"))]
            readback_count: core::sync::atomic::AtomicU64::new(0),
        }
    }

    /// TEST-ONLY: the number of device→host readbacks since the last
    /// [`reset_readback_count`](Self::reset_readback_count) (Phase 5 Wave E
    /// zero-readback oracle).
    ///
    /// A steady-state frame does ZERO readbacks (D2): the `zero_readback`
    /// integration test asserts this returns 0 after every `Schedule::run`
    /// frame, and exactly 1 after the single post-loop test-oracle readback.
    #[cfg(any(test, feature = "test-readback"))]
    #[inline]
    pub fn readback_count(&self) -> u64 {
        // Relaxed: a single-threaded tally on a `!Send` manager — no cross-thread
        // ordering to establish.
        self.readback_count.load(core::sync::atomic::Ordering::Relaxed)
    }

    /// TEST-ONLY: zeroes the readback counter (call before a measured frame
    /// window so [`readback_count`](Self::readback_count) reflects only that
    /// window).
    #[cfg(any(test, feature = "test-readback"))]
    #[inline]
    pub fn reset_readback_count(&self) {
        // Relaxed: see `readback_count` — single-threaded tally on a `!Send`
        // manager.
        self.readback_count
            .store(0, core::sync::atomic::Ordering::Relaxed);
    }

    /// Test-only: whether the registry holds NO live resource (every column +
    /// staging buffer destroyed). Used by the C2 drop-without-`destroy_all` leak
    /// test to confirm the production drop path drained everything.
    #[cfg(any(test, feature = "test-readback"))]
    #[inline]
    pub fn is_fully_drained(&self) -> bool {
        self.registry.is_fully_drained()
    }

    /// Allocates a `DeviceLocal` device buffer for `(archetype, component)`,
    /// registers it, records its [`GpuColumnMeta`], and flips the CPU pool to
    /// device-backing through the A2 seam.
    ///
    /// Steps:
    /// 1. `device.create_buffer(DeviceLocal, STORAGE)` → a never-mapped VRAM
    ///    `BoundBuffer` (the device-local path also adds `TRANSFER_SRC|DST`).
    /// 2. `registry.register_buffer` → a generational [`BufferHandle`] →
    ///    [`slot_to_u64`] → the opaque [`DeviceColumnHandle`] the core stores.
    /// 3. Record `GpuColumnMeta` at the packed slot index.
    /// 4. Drive the A2 funnel
    ///    [`Archetype::make_component_device_backed`](boyko_ecs::ecs::core::archetype::archetype::Archetype::make_component_device_backed):
    ///    flip the pool + null its column cache. SOUND post-A2 — the CPU-query
    ///    skip + the null-column direct-reader guard keep the now-dangling Host
    ///    base CPU-unreachable.
    ///
    /// `stride` is the byte size of one component row; `cap_rows` the initial row
    /// capacity. Returns the minted [`DeviceColumnHandle`].
    ///
    /// # Precondition (X3)
    /// `component` MUST be statically classed [`ResidencyKind::Gpu`]. Flipping an
    /// ordinary (`Cpu`) component to device backing would create a CPU-reachable
    /// dangling column (the C1 UAF class the A2 seam guards only for GPU-pure
    /// archetypes). Checked with a **release-present** `assert!` (setup-time, not a
    /// hot path), so the unsound state can never be built even in release — the
    /// caller is responsible for only minting device columns for `Gpu` components.
    /// A `Gpu`-classed component implies its archetype was stamped `GPU_RESIDENT`
    /// at mint (GPU_RESIDENT ⇔ all-components-Gpu, C2), so this also transitively
    /// guarantees the target archetype is GPU-resident.
    ///
    /// # Errors
    /// [`GpuColumnError::Rhi`] if the device buffer allocation fails.
    pub fn create_column(
        &mut self,
        device: &VulkanContext,
        ecs: &mut EcsMaster,
        archetype: ArchetypeId,
        component: ComponentId,
        stride: u32,
        cap_rows: u32,
    ) -> Result<DeviceColumnHandle, GpuColumnError> {
        debug_assert!(stride > 0, "create_column: zero stride");
        debug_assert!(cap_rows > 0, "create_column: zero capacity");
        // X3 (release-present): only a statically `Gpu`-classed component may be
        // flipped to device backing. A `Cpu` component would leave a CPU-reachable
        // dangling column. Setup-time, so a release assert costs nothing on the
        // hot path. A `Gpu` class also implies the archetype is `GPU_RESIDENT`
        // (C2: GPU_RESIDENT ⇔ all-components-Gpu).
        assert_eq!(
            component_registry::residency_class(component.0),
            ResidencyKind::Gpu,
            "create_column: component {} is not ResidencyKind::Gpu — flipping a \
             Cpu component to device backing would create a dangling CPU column (X3)",
            component.0
        );

        let size = stride as u64 * cap_rows as u64;
        let buffer = device.create_buffer(&BufferDesc {
            size,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::DeviceLocal,
        })?;

        let buffer_handle = self.registry.register_buffer(buffer);
        let handle = DeviceColumnHandle(slot_to_u64(buffer_handle.0));

        let meta = GpuColumnMeta {
            handle,
            stride,
            device_len: 0,
            device_cap: cap_rows,
            archetype,
            component,
        };
        // Pair-keyed upsert (X1/X2): one entry per (archetype, component).
        self.store_meta(meta);

        // Drive the A2 device-mint seam: flip the CPU pool to device-backing and
        // null its column cache (the C1 fix). Reached through the public
        // `archetype_master_mut().get_archetype_mut(id)` chain.
        //
        // `Archetype::make_component_device_backed` is `#[cfg(not(miri))]` in
        // boyko_ecs (it wraps the DeviceColumn RHI seam Miri cannot run), so the
        // call site is gated to match. Under Miri this whole function is
        // unreachable anyway — it requires a live `VulkanContext` device.
        #[cfg(not(miri))]
        {
            let arch = ecs
                .archetype_master_mut()
                .get_archetype_mut(archetype)
                .expect("invariant: create_column targets an existing archetype");
            arch.make_component_device_backed(component, handle);
        }
        // Under Miri the device-mint block above is compiled out, leaving `ecs`
        // unused; the mut-borrow keeps the signature honest without a warning.
        #[cfg(miri)]
        let _ = &mut *ecs;

        Ok(handle)
    }

    /// Resolves the CURRENT column geometry for `(archetype, component)`, or
    /// `None` if no live column matches (MF-7).
    ///
    /// Cost is an `O(N_device_columns)` COLD linear scan of the pair-keyed `meta`
    /// table (the GPU-resident set is tiny in the stable-residency foundation —
    /// Regime-C), then a generation-checked `resolve_buffer` (the ~3 ns registry
    /// index step) to confirm the found handle is still live. No per-frame
    /// `HashMap`/alloc — the scan is over a handful of entries. The table is
    /// single-valued per pair (X1/X2), so the FIRST match is THE column. A stale
    /// handle (post-grow) resolves `None` loudly. Returns no `Result`: a
    /// missing/stale column is `None`, not an error.
    pub fn resolve(
        &self,
        archetype: ArchetypeId,
        component: ComponentId,
    ) -> Option<ResolvedColumn> {
        let m = self
            .meta
            .iter()
            .find(|m| m.archetype == archetype && m.component == component)?;
        // Confirm the handle is still live (a grow bumps the generation, so a
        // stale entry would resolve to no buffer). The pair table is updated in
        // lock-step with grows, so this is belt-and-braces.
        let buffer_handle = BufferHandle(u64_to_slot(m.handle.0));
        self.registry.resolve_buffer(buffer_handle)?;
        debug_assert!(
            m.device_len <= m.device_cap,
            "resolve: device_len exceeds device_cap"
        );
        Some(ResolvedColumn {
            handle: m.handle,
            stride: m.stride,
            device_len: m.device_len,
            device_cap: m.device_cap,
        })
    }

    /// Whether `handle` still resolves to a LIVE device buffer in the registry.
    ///
    /// The direct MF-7 staleness probe: after a [`grow_column`](Self::grow_column)
    /// the OLD handle's `u64` carries a generation the reused slot no longer
    /// matches, so this returns `false` (loud). The NEW handle returns `true`.
    #[inline]
    pub fn is_handle_live(&self, handle: DeviceColumnHandle) -> bool {
        let buffer_handle = BufferHandle(u64_to_slot(handle.0));
        self.registry.resolve_buffer(buffer_handle).is_some()
    }

    /// SETUP-only: builds a compute pipeline from `spirv` and registers both the
    /// shader module and the pipeline in the manager's registry, returning the
    /// pipeline handle (Wave C).
    ///
    /// The shader module + pipeline are owned by the registry, so the existing
    /// [`destroy_all`](Self::destroy_all) tears them down uniformly (the registry
    /// destroys pipelines → shaders → buffers in order). The descriptor-set layout
    /// is the device's shared one — a single STORAGE_BUFFER at set 0 / binding 0,
    /// COMPUTE stage — and the pipeline carries a 4-byte (`u32`) push-constant
    /// range, matching the `gpu_integrate` shader contract.
    ///
    /// `spirv` must be the 4-byte-aligned `u32` word stream of a compute shader
    /// whose layout matches that fixed contract; the caller (`GpuSystem` setup)
    /// supplies the committed `gpu_integrate.comp.spv`.
    ///
    /// # Errors
    /// [`GpuColumnError::Rhi`] if the shader-module or pipeline creation fails. On
    /// a pipeline-create failure the just-created shader module is destroyed before
    /// the error returns (no leak), since it is not yet in the registry.
    pub fn create_compute_pipeline(
        &mut self,
        device: &VulkanContext,
        spirv: &[u32],
    ) -> Result<ComputePipelineHandle, GpuColumnError> {
        let module = device.create_shader_module(spirv)?;
        let pipeline = match device.create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        }) {
            Ok(p) => p,
            Err(e) => {
                // SAFETY: `module` was just created on `device`, was never
                // registered (owned exclusively here, destroyed once), and no
                // pipeline references it (the create failed), so no GPU work uses
                // it.
                unsafe { device.destroy_shader_module(module) };
                return Err(GpuColumnError::Rhi(e));
            }
        };

        // Register both so `destroy_all` reclaims them. The shader-module handle is
        // intentionally not returned: the pipeline owns the compiled stage and the
        // module is only needed for teardown ordering, which the registry handles.
        let _shader: ShaderHandle = self.registry.register_shader(module);
        Ok(self.registry.register_compute_pipeline(pipeline))
    }

    /// Records and submits the `gpu_integrate` compute dispatch on the device
    /// column resolved INDIRECTLY by `(archetype, component)` (MF-7), REPLAYING the
    /// lowered `barriers` plan first, then fence-waits (a straightforward
    /// submit+wait; deferred-wait overlap is a Phase-6 refinement).
    ///
    /// Resolves the current column each call (never caches the raw `u64`), then —
    /// BEFORE the dispatch — replays `barriers`: each [`PlannedBarrier`]'s durable
    /// `(archetype, component)` key is resolved to its CURRENT device buffer (the
    /// same indirect path, surviving a grow) and recorded as a
    /// `vkCmdPipelineBarrier` into the SAME encoder (Wave D). It then binds
    /// `pipeline`, binds the column's device buffer as storage binding 0, pushes
    /// the live row count as the shader's `count` push constant, and dispatches
    /// `ceil(device_len / LOCAL_SIZE_X)` workgroups along X. A `device_len == 0`
    /// column is a documented no-op: it returns `Ok(true)` WITHOUT recording a
    /// pass (a zero-group `dispatch(0, 1, 1)` would trip the encoder's
    /// non-zero-group debug_assert, and an empty pass buys nothing).
    ///
    /// Returns `Ok(false)` if the column does not resolve (a stale handle or a key
    /// with no live column) — the caller (`GpuSystem::run_dispatcher`) treats this
    /// as a skip and `debug_assert!`s against it. Returns `Ok(true)` on a recorded
    /// + fence-waited dispatch.
    ///
    /// # Errors
    /// [`GpuColumnError`] on a stale pipeline handle or any RHI failure (encoder,
    /// submit, fence wait).
    pub fn dispatch_compute(
        &self,
        device: &VulkanContext,
        pipeline: ComputePipelineHandle,
        archetype: ArchetypeId,
        component: ComponentId,
        barriers: &[PlannedBarrier],
    ) -> Result<bool, GpuColumnError> {
        // MF-7: resolve the target column indirectly by its durable key each call.
        // A grow rotates the handle but the (archetype, component) key is stable.
        let Some(resolved) = self.resolve(archetype, component) else {
            return Ok(false);
        };
        let buffer_handle = BufferHandle(u64_to_slot(resolved.handle.0));
        let buffer = self
            .registry
            .resolve_buffer(buffer_handle)
            .ok_or(GpuColumnError::StaleHandle)?;
        let pipe = self
            .registry
            .resolve_compute_pipeline(pipeline)
            .ok_or(GpuColumnError::StaleHandle)?;

        let count = resolved.device_len;

        // Zero-row column: a documented no-op. EARLY-RETURN before recording —
        // `encoder.dispatch(0, 1, 1)` would trip the encoder's non-zero-group
        // debug_assert, and recording an empty pass buys nothing. We have not yet
        // created the fence/encoder here, so there is nothing to tear down.
        if count == 0 {
            return Ok(true);
        }
        let group_count_x = count.div_ceil(LOCAL_SIZE_X);

        let fence = device.create_fence(false)?;
        // If encoder creation fails AFTER the fence was created, the fence would
        // leak (the Wave-B C1 leak class on the dispatch path). Destroy it before
        // propagating the error so every fence/encoder is destroyed exactly once
        // on every edge.
        let mut encoder = match device.create_command_encoder() {
            Ok(e) => e,
            Err(e) => {
                // SAFETY: `fence` was just created on `device` and is moved by
                // value here ⇒ destroyed exactly once; no GPU work references it
                // (nothing was submitted), so the destroy is not a UAF.
                unsafe { device.destroy_fence(fence) };
                return Err(GpuColumnError::Rhi(e));
            }
        };
        let queue = device.rhi_queue();

        // Track submit success separately from the wait result (FIX-U1 mirror): a
        // wait-Err after an Ok submit leaves GPU work in flight referencing the
        // encoder + fence, so they must not be torn down until the device is idle.
        let mut submitted = false;
        let record = (|| -> Result<(), GpuColumnError> {
            encoder.begin()?;
            // (Wave D) Replay the lowered barriers FIRST, so the planned
            // `src → dst` stage/access ordering is established on the GPU timeline
            // before this dispatch touches the column. Each PlannedBarrier's
            // durable `(archetype, component)` key is resolved to its CURRENT
            // device buffer (the same MF-7 indirect path, surviving a grow). A
            // barrier whose key does not resolve is SKIPPED (its producer column is
            // gone — nothing to order against). This is cold (apply-window), so the
            // per-barrier resolve is the same lookup as `target_key`; no per-frame
            // allocation (each barrier records a single-entry stack-local set).
            for b in barriers {
                let (b_arch, b_comp) = b.key();
                let Some(b_resolved) = self.resolve(b_arch, b_comp) else {
                    continue;
                };
                let b_handle = BufferHandle(u64_to_slot(b_resolved.handle.0));
                let Some(b_buf) = self.registry.resolve_buffer(b_handle) else {
                    continue;
                };
                let buffers = [BufferBarrier {
                    buffer: b_buf,
                    src_access: b.src_access,
                    dst_access: b.dst_access,
                }];
                encoder.pipeline_barrier(&BarrierDesc {
                    src_stage: b.src_stage,
                    dst_stage: b.dst_stage,
                    buffers: &buffers,
                });
            }
            encoder.bind_compute_pipeline(pipe);
            encoder.bind_storage_buffer(buffer, 0, 0);
            encoder.push_constants(ShaderStage::COMPUTE, 0, &count.to_ne_bytes());
            encoder.dispatch(group_count_x, 1, 1);
            encoder.end()?;
            queue.submit(&encoder, &fence)?;
            submitted = true;
            device.wait_fence(&fence, u64::MAX)?;
            Ok(())
        })();

        // FIX-U1 mirror: if the submit succeeded but the wait failed, drain the
        // device before destroying the transient encoder + fence (prefer
        // wait_idle-then-destroy over a UAF). On a never-submitted error nothing was
        // enqueued, so no extra wait is needed.
        if record.is_err() && submitted {
            let _ = device.wait_idle();
        }

        // Tear down the transient encoder + fence.
        // SAFETY: `encoder` + `fence` were created on `device` and each is moved by
        // value here ⇒ destroyed exactly once. No GPU work is in flight against
        // them at this point:
        //   * `record` is `Ok`: the fence wait completed the submission.
        //   * `record` is `Err` && `!submitted`: the submit never happened, so
        //     nothing was enqueued.
        //   * `record` is `Err` && `submitted`: the `wait_idle` above drained the
        //     in-flight submission (or the device is lost, making the destroy a
        //     defined no-op).
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
        record.map(|()| true)
    }

    /// TEST-ONLY (Phase 5 Wave E `sync_validation` oracle): records TWO
    /// `gpu_integrate` compute passes over the SAME device column into ONE command
    /// buffer and submits them in ONE `vkQueueSubmit`, with an optional
    /// `vkCmdPipelineBarrier` between them, then fence-waits.
    ///
    /// This is the ONLY path that exposes an INTRA-submit hazard: both passes read
    /// AND write the same SSBO (`Data[i] = Data[i] + 100`), so pass 2's reads
    /// depend on pass 1's writes (a read-after-write + write-after-write hazard on
    /// the same `(buffer, range)`). The per-op fence in [`dispatch_compute`](Self::dispatch_compute)
    /// does NOT order these two passes — they are in the SAME submit — so a
    /// `COMPUTE_SHADER`→`COMPUTE_SHADER` `SHADER_WRITE`→`SHADER_READ|SHADER_WRITE`
    /// barrier between them is LOAD-BEARING:
    ///
    /// * `barrier_between == true`  → the hazard is resolved; the synchronization-
    ///   validation layer stays silent and the result is the deterministic
    ///   `+200` (two `+100` passes applied in order).
    /// * `barrier_between == false` → the two passes are unsynchronized; Vulkan
    ///   synchronization validation flags a `SYNC-HAZARD-*` (the authoritative
    ///   oracle). The numeric result is then UNDEFINED (it may or may not corrupt
    ///   depending on the GPU's actual overlap), so a caller must use the
    ///   validation-layer signal — not the bytes — as the primary oracle.
    ///
    /// Returns `Ok(false)` if the column does not resolve; `Ok(true)` on a
    /// recorded-and-waited two-pass submit. Gated to test builds — it is NEVER on
    /// the frame path (which records exactly one pass per submit).
    ///
    /// # Errors
    /// [`GpuColumnError`] on a stale pipeline/column handle or any RHI failure.
    #[cfg(any(test, feature = "test-readback"))]
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_compute_twice_one_submit(
        &self,
        device: &VulkanContext,
        pipeline: ComputePipelineHandle,
        archetype: ArchetypeId,
        component: ComponentId,
        barrier_between: bool,
    ) -> Result<bool, GpuColumnError> {
        let Some(resolved) = self.resolve(archetype, component) else {
            return Ok(false);
        };
        let buffer_handle = BufferHandle(u64_to_slot(resolved.handle.0));
        let buffer = self
            .registry
            .resolve_buffer(buffer_handle)
            .ok_or(GpuColumnError::StaleHandle)?;
        let pipe = self
            .registry
            .resolve_compute_pipeline(pipeline)
            .ok_or(GpuColumnError::StaleHandle)?;

        let count = resolved.device_len;
        if count == 0 {
            return Ok(true);
        }
        let group_count_x = count.div_ceil(LOCAL_SIZE_X);

        let fence = device.create_fence(false)?;
        let mut encoder = match device.create_command_encoder() {
            Ok(e) => e,
            Err(e) => {
                // SAFETY: `fence` was just created on `device`, moved by value here
                // ⇒ destroyed once; nothing was submitted, so no GPU work
                // references it.
                unsafe { device.destroy_fence(fence) };
                return Err(GpuColumnError::Rhi(e));
            }
        };
        let queue = device.rhi_queue();

        let mut submitted = false;
        let record = (|| -> Result<(), GpuColumnError> {
            encoder.begin()?;

            // Pass 1: read+write the column.
            encoder.bind_compute_pipeline(pipe);
            encoder.bind_storage_buffer(buffer, 0, 0);
            encoder.push_constants(ShaderStage::COMPUTE, 0, &count.to_ne_bytes());
            encoder.dispatch(group_count_x, 1, 1);

            // The load-bearing barrier: order pass 1's SHADER_WRITE before pass
            // 2's SHADER_READ|SHADER_WRITE on the same buffer. Omitting it leaves
            // the intra-submit hazard for synchronization validation to catch.
            if barrier_between {
                let buffers = [BufferBarrier {
                    buffer,
                    src_access: BarrierAccess::SHADER_WRITE,
                    dst_access: BarrierAccess::SHADER_READ | BarrierAccess::SHADER_WRITE,
                }];
                encoder.pipeline_barrier(&BarrierDesc {
                    src_stage: BarrierStage::COMPUTE_SHADER,
                    dst_stage: BarrierStage::COMPUTE_SHADER,
                    buffers: &buffers,
                });
            }

            // Pass 2: read+write the column again (depends on pass 1).
            encoder.bind_compute_pipeline(pipe);
            encoder.bind_storage_buffer(buffer, 0, 0);
            encoder.push_constants(ShaderStage::COMPUTE, 0, &count.to_ne_bytes());
            encoder.dispatch(group_count_x, 1, 1);

            encoder.end()?;
            queue.submit(&encoder, &fence)?;
            submitted = true;
            device.wait_fence(&fence, u64::MAX)?;
            Ok(())
        })();

        if record.is_err() && submitted {
            let _ = device.wait_idle();
        }

        // SAFETY: `encoder` + `fence` were created on `device`, each moved by value
        // here ⇒ destroyed once. No GPU work is in flight: `Ok` ⇒ the wait
        // completed; `Err && !submitted` ⇒ nothing was enqueued; `Err && submitted`
        // ⇒ the `wait_idle` above drained it.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
        record.map(|()| true)
    }

    /// SETUP-only: stages `bytes` into the host-visible staging buffer, copies
    /// them into the device column behind `handle`, and fence-waits.
    ///
    /// Grows the staging buffer if `bytes` exceeds its capacity. This is the ONLY
    /// CPU→GPU transfer and it runs at setup, never on the frame path (D2). The
    /// device buffer is never mapped — the bytes reach VRAM through a real
    /// `vkCmdCopyBuffer`.
    ///
    /// # Errors
    /// [`GpuColumnError`] on a stale handle, a missing staging mapping, or any
    /// RHI failure (buffer create, encoder, submit, fence wait).
    pub fn upload_initial(
        &mut self,
        device: &VulkanContext,
        handle: DeviceColumnHandle,
        bytes: &[u8],
    ) -> Result<(), GpuColumnError> {
        let size = bytes.len() as u64;
        self.ensure_staging(device, size)?;
        let staging = self.staging.expect("invariant: ensure_staging set staging");

        // Write the bytes into the host-coherent staging buffer.
        {
            let staging_buf = self
                .registry
                .resolve_buffer(staging)
                .ok_or(GpuColumnError::StaleHandle)?;
            let dst = device
                .buffer_mapped_ptr(staging_buf)
                .ok_or(GpuColumnError::StagingNotMapped)?;
            // SAFETY: `dst` is the persistently-mapped first byte of a host-visible
            // + host-coherent staging buffer whose capacity is `>= size` (ensured
            // just above by `ensure_staging`). `bytes.len() == size` bytes fit, and
            // no other live alias touches this staging sub-region during the write
            // (the manager is `&mut self`). Host-coherent => no explicit flush.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst.as_ptr(), bytes.len());
            }
        }

        let device_handle = BufferHandle(u64_to_slot(handle.0));
        // Record + submit: copy staging → device, then fence-wait so the upload
        // completes before this returns (setup is synchronous).
        let regions = [BufferCopy { src_offset: 0, dst_offset: 0, size }];
        self.run_copy(device, staging, device_handle, &regions)?;

        // Update the live row count from the uploaded byte span (if the meta is
        // tracked — it always is for a manager-minted column).
        if let Some(meta) = self.meta_mut(device_handle)
            && meta.stride > 0
        {
            meta.device_len = (size / meta.stride as u64) as u32;
            debug_assert!(
                meta.device_len <= meta.device_cap,
                "upload_initial: uploaded rows exceed device_cap"
            );
        }
        Ok(())
    }

    /// ASYNC on-change upload (rung L0-r0): writes `bytes` into the manager's host-
    /// coherent staging buffer and records a staging→`dst` `copy_buffer` + a
    /// TRANSFER_WRITE→SHADER_READ buffer barrier (`TRANSFER`→`COMPUTE_SHADER`) into the
    /// caller-supplied `encoder` — **NO fence, NO submit, NO readback**.
    ///
    /// This is the fence-free counterpart of [`Self::upload_initial`] (which is
    /// setup-only and fence-waits): the caller records the copy into the SAME per-frame
    /// command stream as the consuming dispatch, so the barrier orders the copy before
    /// the marcher/resolve reads on the GPU timeline (Decision 4 / C3). The caller is
    /// responsible for `begin`/`end`/`submit` on the encoder and for gating the call on
    /// a dirty frame (idle frames record nothing → zero cost).
    ///
    /// `dst` is the scene-global light-table device buffer (a `BoundBuffer`); the bytes
    /// are `[LightHeaderGpu || GpuLight[]]`.
    ///
    /// # Errors
    /// [`GpuColumnError`] on a missing staging mapping or a staging (re)alloc failure.
    pub fn record_upload<E: RhiCommandEncoder<Vulkan>>(
        &mut self,
        device: &VulkanContext,
        encoder: &mut E,
        dst: &boyko_rhi_vulkan::memory::BoundBuffer,
        bytes: &[u8],
    ) -> Result<(), GpuColumnError> {
        let size = bytes.len() as u64;
        self.ensure_staging(device, size)?;
        let staging = self.staging.expect("invariant: ensure_staging set staging");

        // Stage the bytes (host-coherent → no flush). A plain memcpy — no GPU sync.
        let staging_buf = self
            .registry
            .resolve_buffer(staging)
            .ok_or(GpuColumnError::StaleHandle)?;
        let dst_ptr = device
            .buffer_mapped_ptr(staging_buf)
            .ok_or(GpuColumnError::StagingNotMapped)?;
        // SAFETY: `dst_ptr` is the persistently-mapped first byte of a host-visible +
        // host-coherent staging buffer whose capacity is `>= size` (just ensured). The
        // `bytes` slice is a distinct allocation, so the regions never overlap; the
        // manager is `&mut self`, so no other live alias touches this staging region.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst_ptr.as_ptr(), bytes.len());
        }

        // Record the fence-free copy + the TRANSFER_WRITE→SHADER_READ barrier so the
        // staged bytes are visible to the subsequent compute reads on the GPU timeline.
        let regions = [BufferCopy { src_offset: 0, dst_offset: 0, size }];
        encoder.copy_buffer(staging_buf, dst, &regions);
        let buffers = [BufferBarrier {
            buffer: dst,
            src_access: BarrierAccess::TRANSFER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
        }];
        encoder.pipeline_barrier(&BarrierDesc {
            src_stage: BarrierStage::TRANSFER,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            buffers: &buffers,
        });
        Ok(())
    }

    /// Reallocates the device column behind `old` to `new_cap_rows`, copying the
    /// old contents into the new buffer, and returns the NEW handle (MF-7).
    ///
    /// `#[inline(never)]`: a cold realloc, kept off the I-cache of any caller.
    ///
    /// # MF-7 ordering (take BEFORE register)
    /// The registry bumps a slot's generation on `take` then LIFO-reuses the freed
    /// index on the next `register`. So the old buffer MUST be taken BEFORE the new
    /// one is registered: the reused index then carries the bumped generation, so
    /// the stale `old` `u64` resolves `None` (loud). Register-then-take would mint
    /// a fresh index and leave `old` resolving to a LIVE (aliasing) buffer.
    ///
    /// # Errors
    /// [`GpuColumnError`] on a stale `old` handle or any RHI failure.
    #[inline(never)]
    pub fn grow_column(
        &mut self,
        device: &VulkanContext,
        ecs: &mut EcsMaster,
        old: DeviceColumnHandle,
        new_cap_rows: u32,
    ) -> Result<DeviceColumnHandle, GpuColumnError> {
        let old_handle = BufferHandle(u64_to_slot(old.0));
        let old_meta = *self
            .meta(old_handle)
            .ok_or(GpuColumnError::StaleHandle)?;
        debug_assert!(
            new_cap_rows >= old_meta.device_cap,
            "grow_column: new capacity must not shrink the column"
        );

        let new_size = old_meta.stride as u64 * new_cap_rows as u64;
        let copy_size = old_meta.stride as u64 * old_meta.device_len as u64;

        // Allocate the NEW device buffer first (so a failed create leaves the old
        // buffer intact and resolvable — no partial state).
        let new_buffer = device.create_buffer(&BufferDesc {
            size: new_size,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::DeviceLocal,
        })?;

        // FIX-C1: from here until `new_buffer` is registered it is a LIVE VkBuffer
        // + suballocation NOT in the registry, so `destroy_all` cannot reach it.
        // EVERY error edge below must destroy it exactly once before returning, or
        // it leaks permanently (`BoundBuffer` has no `Drop`). This mirrors the
        // rollback discipline in `memory.rs::create_bound_buffer` / encoder `new`.

        // Copy old → new BEFORE the old buffer is taken (both must be live to
        // resolve during the copy). The new buffer is a local `BoundBuffer` not yet
        // in the registry, so record against the raw buffers.
        if copy_size > 0 {
            let regions = [BufferCopy { src_offset: 0, dst_offset: 0, size: copy_size }];
            let old_buf = match self.registry.resolve_buffer(old_handle) {
                Some(b) => b,
                None => {
                    // SAFETY: `new_buffer` was just created on `device`, was never
                    // registered (owned exclusively here, destroyed once) and never
                    // submitted to, so no GPU work references it.
                    unsafe { device.destroy_buffer(new_buffer) };
                    return Err(GpuColumnError::StaleHandle);
                }
            };
            if let Err(e) = self.run_copy_raw(device, old_buf, &new_buffer, &regions) {
                // SAFETY: `new_buffer` was created on `device`, never registered
                // (owned exclusively here, destroyed once). `run_copy_raw` already
                // tore down its own transient encoder/fence and — on its
                // submit-failed / wait path — left no work in flight against
                // `new_buffer`, so no GPU read references it.
                unsafe { device.destroy_buffer(new_buffer) };
                return Err(e);
            }
        }

        // ===== MF-7: take BEFORE register. =====
        // `take_buffer(old)` bumps the freed slot's generation and pushes its index
        // onto the LIFO free list; the very next `register_buffer` pops THAT index
        // with the bumped generation. So the new handle reuses the old index but
        // with a fresh generation → the stale `old` u64 resolves `None` (loud).
        let old_owned = match self.registry.take_buffer(old_handle) {
            Some(b) => b,
            None => {
                // SAFETY: `new_buffer` was created on `device`, never registered
                // (owned exclusively here, destroyed once). The copy above (if any)
                // fence-waited, so no GPU work is in flight against `new_buffer`.
                unsafe { device.destroy_buffer(new_buffer) };
                return Err(GpuColumnError::StaleHandle);
            }
        };
        // The old column's pair entry is updated by the `store_meta` upsert below
        // (single-valued by pair), so there is no separate index-keyed clear.

        let new_handle_buf = self.registry.register_buffer(new_buffer);
        let new_handle = DeviceColumnHandle(slot_to_u64(new_handle_buf.0));

        // Destroy the old device buffer now that its contents are copied + its slot
        // is freed.
        // SAFETY: `old_owned` was just removed from the registry (owned exclusively
        // here, destroyed exactly once) and was created on `device`. The GPU is no
        // longer reading it on EITHER path (FIX-W1):
        //   * `copy_size > 0`: the `run_copy_raw` above fence-waited, ordering this
        //     destroy after the copy that read `old_owned`.
        //   * `copy_size == 0`: NO submission ever referenced `old_owned` (nothing
        //     was copied into the old device buffer when `device_len == 0`, and no
        //     copy reads from it here), so there is no in-flight GPU read to order
        //     against — the destroy is unconditionally safe.
        unsafe {
            device.destroy_buffer(old_owned);
        }

        // Record the rotated handle + grown capacity (carry the live row count).
        // Pair-keyed upsert (X1/X2): updates the SINGLE entry for this column's
        // (archetype, component) in place — no second entry can survive a grow.
        let new_meta = GpuColumnMeta {
            handle: new_handle,
            stride: old_meta.stride,
            device_len: old_meta.device_len,
            device_cap: new_cap_rows,
            archetype: old_meta.archetype,
            component: old_meta.component,
        };
        self.store_meta(new_meta);

        // Write the new handle back into the core pool through the A2 grow write
        // funnel (MF-2/3): it updates ONLY the boxed DeviceColumn.handle (no
        // re-flip, no Box churn, no column touch — the pool is already
        // device-backed and its column already null).
        //
        // `Archetype::set_component_device_handle` is `#[cfg(not(miri))]` in
        // boyko_ecs (it wraps the DeviceColumn RHI seam Miri cannot run), so the
        // call site is gated to match. Under Miri this whole function is
        // unreachable anyway — it requires a live `VulkanContext` device.
        #[cfg(not(miri))]
        {
            let arch = ecs
                .archetype_master_mut()
                .get_archetype_mut(old_meta.archetype)
                .expect("invariant: grow_column targets an existing archetype");
            arch.set_component_device_handle(old_meta.component, new_handle);
        }
        // Under Miri the device-write block above is compiled out, leaving `ecs`
        // unused; the mut-borrow keeps the signature honest without a warning.
        #[cfg(miri)]
        let _ = &mut *ecs;

        Ok(new_handle)
    }

    /// TEST-ONLY: copies the device column behind `handle` into the staging buffer,
    /// fence-waits, and reads the bytes back into a fresh `Vec`.
    ///
    /// The single CPU readback path — used by the round-trip oracle. NOT a frame
    /// path: steady state does zero readback (D2). Gated to test builds.
    ///
    /// # Precondition (FIX-U2)
    /// `len` MUST NOT exceed the column's byte capacity (`device_cap * stride`).
    /// The device→staging copy region is built from `len`, so a `len` larger than
    /// the device buffer would over-read it. Asserted below (test-only path, but
    /// the bound is explicit).
    ///
    /// # Errors
    /// [`GpuColumnError`] on a stale handle, a missing staging mapping, or any
    /// RHI failure.
    #[cfg(any(test, feature = "test-readback"))]
    pub fn readback_for_test(
        &mut self,
        device: &VulkanContext,
        handle: DeviceColumnHandle,
        len: usize,
    ) -> Result<Vec<u8>, GpuColumnError> {
        // Zero-readback oracle (Phase 5 Wave E): count every device→host readback
        // so a test can prove the steady-state frame path performed NONE. Relaxed:
        // single-threaded tally on a `!Send` manager.
        self.readback_count
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let size = len as u64;
        let device_handle = BufferHandle(u64_to_slot(handle.0));
        // FIX-U2: bound the read against the column's byte capacity so the
        // device→staging copy region can never over-read the device buffer.
        let column = self.meta(device_handle).ok_or(GpuColumnError::StaleHandle)?;
        let byte_cap = column.device_cap as u64 * column.stride as u64;
        assert!(
            size <= byte_cap,
            "readback_for_test: requested {size} bytes exceeds column byte capacity {byte_cap}"
        );
        self.ensure_staging(device, size)?;
        let staging = self.staging.expect("invariant: ensure_staging set staging");

        // Copy device → staging, then fence-wait.
        let regions = [BufferCopy { src_offset: 0, dst_offset: 0, size }];
        self.run_copy(device, device_handle, staging, &regions)?;

        // Map-read the staging buffer.
        let staging_buf = self
            .registry
            .resolve_buffer(staging)
            .ok_or(GpuColumnError::StaleHandle)?;
        let src = device
            .buffer_mapped_ptr(staging_buf)
            .ok_or(GpuColumnError::StagingNotMapped)?;
        let mut out = vec![0u8; len];
        // SAFETY: `src` is the persistently-mapped first byte of a host-coherent
        // staging buffer of capacity `>= size`; the `run_copy` above fence-waited,
        // so the GPU readback is complete + coherent; reading `len` bytes is
        // in-bounds; `out` is a distinct, non-overlapping `len`-byte allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), out.as_mut_ptr(), len);
        }
        Ok(out)
    }

    /// Tears down every device buffer + the staging buffer before drop (W4).
    ///
    /// `registry.destroy_all` waits the device idle, then destroys every
    /// registered buffer (the columns + the staging buffer) in reverse order.
    ///
    /// IDEMPOTENT (FIX-C2): if the registry is already fully drained (a prior
    /// explicit `destroy_all` ran, e.g. in a test that then drops the owning
    /// [`RhiContext`]), this early-returns BEFORE touching the device — so the
    /// `RhiContext::Drop` second call neither double-frees nor issues a redundant
    /// `wait_idle`. The first call does the real teardown.
    pub fn destroy_all(&mut self, device: &VulkanContext) {
        if self.registry.is_fully_drained() {
            // Already torn down (or never used). Keep the side state coherent and
            // return without re-touching the device.
            self.meta.clear();
            self.staging = None;
            self.staging_cap = 0;
            return;
        }
        self.registry.destroy_all(device);
        self.meta.clear();
        self.staging = None;
        self.staging_cap = 0;
        debug_assert!(
            self.registry.is_fully_drained(),
            "destroy_all: registry not fully drained"
        );
    }

    // ===== internals =====

    /// Ensures the staging buffer has capacity `>= need` bytes, (re)allocating it
    /// host-visible if absent or too small. The old staging buffer is destroyed
    /// before the new one is registered (no double-resolve risk — staging is never
    /// resolved across a regrow within one op).
    fn ensure_staging(&mut self, device: &VulkanContext, need: u64) -> Result<(), GpuColumnError> {
        if self.staging_cap >= need && self.staging.is_some() {
            return Ok(());
        }
        // Round up to avoid frequent re-allocs; never below `need`.
        let new_cap = need.max(self.staging_cap.saturating_mul(2)).max(1);
        let buffer = device.create_buffer(&BufferDesc {
            size: new_cap,
            usage: BufferUsage::TRANSFER_SRC | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })?;
        // Take + destroy the old staging buffer (if any) before registering the new
        // one. `take` bumps the freed slot's generation; the old handle is local to
        // the manager (never handed out) so there is no stale-resolve concern.
        //
        // X1/X2: staging has NO `meta` entry (it is never a column), so there is NO
        // `clear_meta` here. The old index-keyed `clear_meta(old_staging)` was the
        // silent-data-loss bug: the freed staging slot index is SHARED with column
        // slots, so it could null a LIVE column's entry. The pair-keyed table makes
        // that collision impossible — staging simply has no key.
        if let Some(old) = self.staging.take()
            && let Some(old_owned) = self.registry.take_buffer(old)
        {
            // SAFETY: `old_owned` was just removed from the registry (owned
            // exclusively here, destroyed once), created on `device`. Staging is
            // only used inside fence-waited `run_copy` calls, so no GPU work is in
            // flight against it at a regrow (each op fence-waits before returning).
            unsafe {
                device.destroy_buffer(old_owned);
            }
        }
        let handle = self.registry.register_buffer(buffer);
        self.staging = Some(handle);
        self.staging_cap = new_cap;
        Ok(())
    }

    /// Records + submits a single buffer copy between two REGISTERED buffers, then
    /// fence-waits (setup/readback are synchronous). Creates a fresh encoder +
    /// fence, destroys both by value when done.
    fn run_copy(
        &self,
        device: &VulkanContext,
        src: BufferHandle,
        dst: BufferHandle,
        regions: &[BufferCopy],
    ) -> Result<(), GpuColumnError> {
        let src_buf = self
            .registry
            .resolve_buffer(src)
            .ok_or(GpuColumnError::StaleHandle)?;
        let dst_buf = self
            .registry
            .resolve_buffer(dst)
            .ok_or(GpuColumnError::StaleHandle)?;
        self.run_copy_raw(device, src_buf, dst_buf, regions)
    }

    /// Records + submits a single copy between two raw `BoundBuffer`s (used when
    /// one endpoint is not yet registered, e.g. the new buffer in `grow_column`),
    /// then fence-waits. Tears down the transient encoder + fence.
    fn run_copy_raw(
        &self,
        device: &VulkanContext,
        src_buf: &boyko_rhi_vulkan::memory::BoundBuffer,
        dst_buf: &boyko_rhi_vulkan::memory::BoundBuffer,
        regions: &[BufferCopy],
    ) -> Result<(), GpuColumnError> {
        let fence = device.create_fence(false)?;
        // If encoder creation fails AFTER the fence was created, the fence would
        // leak (the Wave-B C1 leak class on the copy path). Destroy it before
        // propagating the error so every fence/encoder is destroyed exactly once
        // on every edge.
        let mut encoder = match device.create_command_encoder() {
            Ok(e) => e,
            Err(e) => {
                // SAFETY: `fence` was just created on `device` and is moved by
                // value here ⇒ destroyed exactly once; no GPU work references it
                // (nothing was submitted), so the destroy is not a UAF.
                unsafe { device.destroy_fence(fence) };
                return Err(GpuColumnError::Rhi(e));
            }
        };
        let queue = device.rhi_queue();

        // Track whether the submit succeeded SEPARATELY from the wait result: a
        // wait-Err AFTER an Ok submit leaves GPU work in flight referencing the
        // encoder + fence, so they must NOT be torn down until the device is idle
        // (FIX-U1).
        let mut submitted = false;
        let record = (|| -> Result<(), GpuColumnError> {
            encoder.begin()?;
            encoder.copy_buffer(src_buf, dst_buf, regions);
            encoder.end()?;
            queue.submit(&encoder, &fence)?;
            submitted = true;
            device.wait_fence(&fence, u64::MAX)?;
            Ok(())
        })();

        // FIX-U1: if the submit succeeded but the fence wait failed, work may still
        // be in flight against the encoder + fence. Block on `wait_idle` BEFORE
        // destroying them so the destroy is not a UAF (prefer wait_idle-then-destroy
        // over leak-or-UAF). When the submit never succeeded (begin/end/submit
        // error), no work was enqueued, so no extra wait is needed.
        if record.is_err() && submitted {
            // Best-effort: if `wait_idle` ALSO fails the device is lost, and
            // destroying children of a lost device is a defined no-op (mirrors the
            // registry `destroy_all` reasoning), so we proceed to destroy either
            // way.
            let _ = device.wait_idle();
        }

        // Tear down the transient encoder + fence.
        // SAFETY: `encoder` + `fence` were created on `device` and each is moved by
        // value here => destroyed exactly once. No GPU work is in flight against
        // them at this point:
        //   * `record` is `Ok`: the fence wait completed the submission.
        //   * `record` is `Err` && `!submitted`: the submit never happened (begin/
        //     end/submit error), so nothing was enqueued.
        //   * `record` is `Err` && `submitted`: the `wait_idle` above drained the
        //     in-flight submission (or the device is lost, making the destroy a
        //     defined no-op).
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
        }
        record
    }

    /// Upserts `meta` keyed by its durable `(archetype, component)` pair (X1/X2).
    ///
    /// Single-valued by construction: if an entry for the pair already exists (a
    /// grow rotated the handle), it is OVERWRITTEN in place; otherwise the entry is
    /// pushed. So exactly one entry per column survives, regardless of buffer-slot
    /// churn — no staging collision, no double entry after a grow.
    fn store_meta(&mut self, meta: GpuColumnMeta) {
        match self
            .meta
            .iter_mut()
            .find(|m| m.archetype == meta.archetype && m.component == meta.component)
        {
            Some(slot) => *slot = meta,
            None => self.meta.push(meta),
        }
    }

    /// Borrows the meta entry whose CURRENT handle equals `handle` (generation-
    /// exact). A stale handle (post-grow, whose generation no longer matches the
    /// stored current handle) returns `None`.
    fn meta(&self, handle: BufferHandle) -> Option<&GpuColumnMeta> {
        let target = slot_to_u64(handle.0);
        self.meta.iter().find(|m| m.handle.0 == target)
    }

    /// Mutably borrows the meta entry whose CURRENT handle equals `handle`
    /// (generation-exact like [`Self::meta`]).
    fn meta_mut(&mut self, handle: BufferHandle) -> Option<&mut GpuColumnMeta> {
        let target = slot_to_u64(handle.0);
        self.meta.iter_mut().find(|m| m.handle.0 == target)
    }
}
