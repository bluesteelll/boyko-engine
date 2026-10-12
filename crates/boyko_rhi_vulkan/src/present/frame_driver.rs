//! The [`Renderer`]: per-frame sync ownership (`FrameSync` + command buffers +
//! render-finished semaphores), the shared `drive_frame` acquire→submit→present
//! skeleton, the four thin public frame adapters, `recreate`, `Drop`, and the
//! frame-sync creation helpers. The per-skeleton record bodies live in
//! [`super::passes`]; the graph declaration in [`super::graph_bridge`]. Split out of
//! the former monolithic `swapchain.rs` (audit W4).

use core::ptr;

use crate::device::{DeviceFns, SwapchainDeviceFns, VulkanContext};
use crate::ffi::*;
use crate::memory::BoundBuffer;

use super::graph_bridge::{ForwardPassPlan, GbufferPassPlan, VbPassPlan};
use super::passes::vb::VbRecordProbe;
use super::scene_types::{GBufferScene, SampledComposite, Scene, UiPass};
use super::swapchain::swapchain_image_for;
use super::targets::{GBufferFrame, GBufferTargets, TargetsProfile};
use super::{FRAMES_IN_FLIGHT, Surface, Swapchain, SwapchainError};

/// Per-frame-in-flight CPU↔GPU sync: an acquire semaphore + an in-flight fence.
pub(crate) struct FrameSync {
    /// Signalled by `vkAcquireNextImageKHR`, waited at COLOR_ATTACHMENT_OUTPUT.
    pub(crate) acquire: VkSemaphore,
    /// Signalled by the submit, waited by the CPU before reusing this frame slot.
    pub(crate) in_flight: VkFence,
}

/// Owns the command pool + one command buffer per frame, the per-frame sync, and
/// a render-finished semaphore per swapchain image, and drives the
/// acquire→record→submit→present loop with dynamic rendering + out-of-date
/// recreation.
///
/// Borrows the device tables (`'ctx`); `Drop` waits the device idle then tears
/// down all sync + the command pool in reverse order.
pub struct Renderer<'ctx> {
    pub(crate) device: VkDevice,
    pub(crate) fns: &'ctx DeviceFns,
    pub(crate) swap_fns: &'ctx SwapchainDeviceFns,
    pub(crate) queue: VkQueue,
    pub(crate) command_pool: VkCommandPool,
    /// One command buffer per frame in flight (allocated from `command_pool`,
    /// freed implicitly when the pool is destroyed).
    pub(crate) command_buffers: [VkCommandBuffer; FRAMES_IN_FLIGHT],
    pub(crate) frames: [FrameSync; FRAMES_IN_FLIGHT],
    /// One render-finished semaphore per swapchain image (sized to the swapchain;
    /// rebuilt when the swapchain is recreated).
    pub(crate) render_finished: Vec<VkSemaphore>,
    /// The current frame-in-flight slot (round-robin).
    pub(crate) frame_index: usize,
    /// Total committed submits (`vkQueueSubmit` calls that reached the ring
    /// advance below) — the fence clock the asset-retire gate (F6) is keyed on.
    /// Increments 1:1 with `frame_index` at the SAME site (below); unlike
    /// `frame_index` it is NOT reset by [`recreate`](Self::recreate) (it stays
    /// monotonic across swapchain rebuilds — `recreate`'s `device_wait_idle`
    /// already backstops the fence horizon for any in-flight retire).
    submission_epoch: u64,
    /// The in-house Render Dependency Graph. Re-declared PER FRAME over the WHOLE
    /// G-buffer frame in [`render_gbuffer_frame`](Self::render_gbuffer_frame) (a
    /// zero-alloc `reset`+re-declare+`compile`) and stored as the resulting
    /// [`GbufferPassPlan`] in [`gbuffer_pass_plan`](Self::gbuffer_pass_plan). It then
    /// DRIVES every G-buffer / shadow / buffer barrier of `record_gbuffer`
    /// unconditionally (the swapchain WSI barriers stay hand-recorded).
    pub(crate) frame_graph: crate::framegraph::FrameGraph,
    /// The PER-FRAME map from each declared G-buffer pass to its [`PassId`] in
    /// [`frame_graph`](Self::frame_graph), set every frame by
    /// [`render_gbuffer_frame`](Self::render_gbuffer_frame). Optional members are
    /// `None` when their pass was config-gated off this frame, so `record_gbuffer`
    /// records that pass's graph barriers only when it also records the pass body.
    /// `Some` only on a `Deferred`-resolved frame (`declare_deferred_graph`'s output);
    /// `None` on a `Forward`-resolved frame (see [`Self::forward_pass_plan`]).
    pub(crate) gbuffer_pass_plan: Option<GbufferPassPlan>,
    /// Multi-paradigm render-path plan, rung R4b-b: the `Forward` sibling of
    /// [`Self::gbuffer_pass_plan`] — re-declared PER FRAME by `declare_forward_graph` over the
    /// SAME shared [`Self::frame_graph`] (see [`super::graph_bridge::ForwardPassPlan`]'s doc for
    /// why this is a decoupled, private ResId space rather than an extension of
    /// [`GbufferPassPlan`]). `Some` only on a `Forward`-resolved frame; `None` on a
    /// `Deferred`-resolved frame.
    pub(crate) forward_pass_plan: Option<ForwardPassPlan>,
    /// Multi-paradigm render-path plan, rung R8: the `VisibilityBuffer` sibling of
    /// [`Self::forward_pass_plan`] — re-declared PER FRAME by `declare_vb_graph` over the SAME
    /// shared [`Self::frame_graph`] (see [`super::graph_bridge::VbPassPlan`]'s doc for why this
    /// is its OWN decoupled, private ResId space). `Some` only on a `VisibilityBuffer`-resolved
    /// frame; `None` otherwise.
    pub(crate) vb_pass_plan: Option<VbPassPlan>,
    /// Dynamic-materials DM1: this frame's recorder-site material-upload counts
    /// ([`super::MaterialUploadProbe`]). Reset at the top of every
    /// [`render_gbuffer_frame`](Self::render_gbuffer_frame) and incremented by the three
    /// recorders (`&self`) where they record the `material_upload` pass — a `Cell` because the
    /// recorders borrow the renderer shared, not because anything else touches it (one thread
    /// records).
    material_upload_probe: core::cell::Cell<super::MaterialUploadProbe>,
    /// OBS (H0 Scope 6): every `SeededImage` / `SeededBuffer` declaration ORs the image dump's
    /// `TRANSFER` read into its seed — a boot decision of the `BOYKO_IMAGE_DUMP` instrument,
    /// `false` in every shipped run (seeds and barrier streams untouched).
    pub(crate) obs_dump_widen: bool,
    /// OBS (R): the `BOYKO_GRAPH_RECEIPT` capture — `Some` only after
    /// [`arm_graph_receipt`](Self::arm_graph_receipt) (boot, armed runs only), refreshed inside
    /// every recorded frame. `None` in every shipped run: one branch per frame.
    pub(crate) receipt: Option<ReceiptCapture>,
    /// OBS (D): the `BOYKO_IMAGE_DUMP` arming — `Some` only after
    /// [`arm_image_dump`](Self::arm_image_dump) (boot, armed runs only); holds this frame's request
    /// and what the recorder copied. `None` in every shipped run: one branch per frame.
    pub(crate) image_dump: Option<ImageDumpArm>,
}

impl<'ctx> Renderer<'ctx> {
    /// Builds the command pool + per-frame command buffers + per-frame sync + one
    /// render-finished semaphore per image of `swapchain`.
    pub fn new(
        ctx: &'ctx VulkanContext,
        surface: &Surface<'_>,
        swapchain: &Swapchain<'_>,
    ) -> Result<Self, SwapchainError> {
        let fns = ctx.device_fns();
        let swap_fns = ctx.swapchain_fns().ok_or(SwapchainError::NotWindowed)?;
        let device = ctx.device();

        // --- Command pool (RESET_COMMAND_BUFFER so each frame can re-record). ---
        let cp_info = VkCommandPoolCreateInfo {
            s_type: VkStructureType::CommandPoolCreateInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
            queue_family_index: surface.present_family,
        };
        let mut command_pool = VkCommandPool::NULL;
        // SAFETY: `device` is live; `cp_info` is fully initialized for the present
        // family; `&mut command_pool` is a valid out-pointer.
        let raw = unsafe { (fns.create_command_pool)(device, &cp_info, ptr::null(), &mut command_pool) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkCreateCommandPool", result));
        }

        // --- One primary command buffer per frame in flight. ---
        let cb_alloc = VkCommandBufferAllocateInfo {
            s_type: VkStructureType::CommandBufferAllocateInfo,
            p_next: ptr::null(),
            command_pool,
            level: VK_COMMAND_BUFFER_LEVEL_PRIMARY,
            command_buffer_count: FRAMES_IN_FLIGHT as u32,
        };
        let mut command_buffers = [VkCommandBuffer::NULL; FRAMES_IN_FLIGHT];
        // SAFETY: `device` is live; `cb_alloc` names the live pool and requests
        // `FRAMES_IN_FLIGHT` buffers; `command_buffers.as_mut_ptr()` is a valid
        // out-pointer for exactly that many primary buffers.
        let raw = unsafe {
            (fns.allocate_command_buffers)(device, &cb_alloc, command_buffers.as_mut_ptr())
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: the pool was created above; destroying it frees the partial
            // buffers and the pool, once.
            unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
            return Err(SwapchainError::VkError("vkAllocateCommandBuffers", result));
        }

        // --- Per-frame acquire semaphore + signalled in-flight fence. ---
        // Fences start SIGNALLED so the first frame's wait returns immediately.
        let mut frames: [FrameSync; FRAMES_IN_FLIGHT] = [
            FrameSync { acquire: VkSemaphore::NULL, in_flight: VkFence::NULL },
            FrameSync { acquire: VkSemaphore::NULL, in_flight: VkFence::NULL },
        ];
        for slot in frames.iter_mut() {
            match create_semaphore(fns, device) {
                Ok(s) => slot.acquire = s,
                Err(e) => {
                    // SAFETY: tear down whatever was created so far + the pool.
                    unsafe { destroy_partial_frames(fns, device, &frames, &[]); };
                    unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
                    return Err(e);
                }
            }
            match create_fence_signalled(fns, device) {
                Ok(f) => slot.in_flight = f,
                Err(e) => {
                    unsafe { destroy_partial_frames(fns, device, &frames, &[]); };
                    unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
                    return Err(e);
                }
            }
        }

        // --- One render-finished semaphore per swapchain image. ---
        let mut render_finished = Vec::with_capacity(swapchain.image_count());
        for _ in 0..swapchain.image_count() {
            match create_semaphore(fns, device) {
                Ok(s) => render_finished.push(s),
                Err(e) => {
                    unsafe { destroy_partial_frames(fns, device, &frames, &render_finished); };
                    unsafe { (fns.destroy_command_pool)(device, command_pool, ptr::null()) };
                    return Err(e);
                }
            }
        }

        // --- The frame graph (Steps 1c–1e). ---
        // Preallocated for the MAXIMAL whole-frame declaration (14 resources: 9 images
        // + 5 buffers; ~11 passes; ~38 accesses — see `render_gbuffer_frame`). Sized
        // generously so the per-frame `reset`+re-declare is zero-alloc. This leading-
        // raster compile is a valid initial state; `render_gbuffer_frame` OVERWRITES
        // it every frame with the whole-frame plan (declaration order pins the ResIds
        // albedo=0..depth=3, etc.). The single
        // "raster" pass records, per its declared accesses, the two batched barriers the
        // hand path emits: the 3 color images UNDEFINED→COLOR_ATTACHMENT_OPTIMAL at
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT, then depth UNDEFINED→DEPTH_ATTACHMENT_OPTIMAL
        // at TOP_OF_PIPE→(EARLY|LATE)_FRAGMENT_TESTS.
        //
        // OBS names the reserve: `FRAME_GRAPH_CAPACITY` (end of this file). It is a FIRST-FRAME
        // ALLOCATION HINT, not a bound — `Vec::with_capacity` grows on demand — so an undersized
        // one costs a reallocation on the frame that overflows it and nothing else. The parent's
        // `(48, 32, 192)` was undersized: VB under `hwrt` with every tail armed declares 53
        // resources (69 mip-weighted entries); the receipt's `[counts]` gates the high-water mark.
        let mut frame_graph = crate::framegraph::FrameGraph::with_capacity(
            FRAME_GRAPH_CAPACITY.max_res,
            FRAME_GRAPH_CAPACITY.max_pass,
            FRAME_GRAPH_CAPACITY.max_acc,
        );
        let albedo = frame_graph.add_image("albedo");
        let normal = frame_graph.add_image("normal");
        let material = frame_graph.add_image("material");
        let depth = frame_graph.add_image("depth");
        frame_graph.add_pass("raster");
        frame_graph.image_access(
            albedo,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            crate::framegraph::SubRange::COLOR,
        );
        frame_graph.image_access(
            normal,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            crate::framegraph::SubRange::COLOR,
        );
        frame_graph.image_access(
            material,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            crate::framegraph::SubRange::COLOR,
        );
        frame_graph.image_access(
            depth,
            VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
            VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            crate::framegraph::SubRange::DEPTH,
        );
        frame_graph.compile();

        Ok(Self {
            device,
            fns,
            swap_fns,
            queue: ctx.queue(),
            command_pool,
            command_buffers,
            frames,
            render_finished,
            frame_index: 0,
            submission_epoch: 0,
            frame_graph,
            gbuffer_pass_plan: None,
            forward_pass_plan: None,
            vb_pass_plan: None,
            material_upload_probe: core::cell::Cell::new(super::MaterialUploadProbe::default()),
            obs_dump_widen: false,
            receipt: None,
            image_dump: None,
        })
    }

    /// OBS (D): arms the `BOYKO_IMAGE_DUMP` instrument — the boot decisions H0 Scope 6 names:
    /// every `SeededImage` / `SeededBuffer` declaration widens its seed with the dump's `TRANSFER`
    /// read ([`obs_dump_widen`](Self::obs_dump_widen)), and the targets created from now on carry
    /// `TRANSFER_SRC` on Deferred `depth`, `viewt` and `taa_hist`. Cold — called once, before the
    /// first frame (the targets are created on the first frame; arming later would leave them
    /// without the usage bit the copies need).
    pub fn arm_image_dump(&mut self) {
        debug_assert_eq!(
            self.submission_epoch, 0,
            "invariant: the image dump is armed before the first frame creates the targets"
        );
        self.obs_dump_widen = true;
        self.image_dump = Some(ImageDumpArm {
            request: None,
            copies: core::cell::Cell::new([ImageDumpCopy::NONE; IMAGE_DUMP_MAX]),
        });
    }

    /// OBS (D): this frame's dump request — the staging each named image is copied into by the
    /// NEXT [`render_gbuffer_frame`](Self::render_gbuffer_frame), which consumes it (a frame that
    /// does not record drops it; the instrument re-requests). A no-op unless armed.
    ///
    /// Storing the handles is safe; recording copies into them is not. The obligations on
    /// `request.staging` and `request.capacity` are the image-dump clause of
    /// [`render_gbuffer_frame`](Self::render_gbuffer_frame)'s `# Safety`, which the `unsafe` call
    /// that consumes the request accepts.
    pub fn request_image_dump(&mut self, request: ImageDumpRequest) {
        if let Some(arm) = self.image_dump.as_mut() {
            arm.request = Some(request);
        }
    }

    /// OBS (D): what the LAST [`render_gbuffer_frame`](Self::render_gbuffer_frame) copied, per
    /// [`ImageDumpName`] (index = `name as usize`); `None` unless armed.
    #[inline]
    pub fn last_image_dump(&self) -> Option<[ImageDumpCopy; IMAGE_DUMP_MAX]> {
        self.image_dump.as_ref().map(|a| a.copies.get())
    }

    /// OBS (R): arms the `BOYKO_GRAPH_RECEIPT` capture. Cold — called once at boot by the
    /// instrument; every later recorded frame refreshes [`last_receipt_capture`](Self::last_receipt_capture).
    pub fn arm_graph_receipt(&mut self) {
        self.receipt = Some(ReceiptCapture::EMPTY);
    }

    /// OBS (R): the capture of the LAST recorded frame (`None` unless the receipt was armed). Read
    /// beside [`last_frame_graph`](Self::last_frame_graph), after the frame returned.
    #[inline]
    pub fn last_receipt_capture(&self) -> Option<&ReceiptCapture> {
        self.receipt.as_ref()
    }

    /// Dynamic-materials DM1: what the recorder did with the LAST
    /// [`render_gbuffer_frame`](Self::render_gbuffer_frame)'s material upload — counted at the
    /// record sites ([`super::MaterialUploadProbe`]). All-zero after a frame that recorded
    /// nothing (an idle frame, or a call that returned before recording).
    #[inline]
    pub fn material_upload_probe(&self) -> super::MaterialUploadProbe {
        self.material_upload_probe.get()
    }

    /// OBS (R): the frame graph the LAST [`render_gbuffer_frame`](Self::render_gbuffer_frame)
    /// declared and compiled — the production declarator's plan, read by the
    /// `BOYKO_GRAPH_RECEIPT` instrument. It holds that frame's plan from `compile` until the next
    /// frame's `reset`.
    #[inline]
    pub fn last_frame_graph(&self) -> &crate::framegraph::FrameGraph {
        &self.frame_graph
    }

    /// Records one `material_upload` pass that handed `regions` regions to `vkCmdCopyBuffer` —
    /// called by the three recorders at the site that records the pass.
    #[inline]
    pub(crate) fn note_material_upload(&self, regions: u32) {
        let p = self.material_upload_probe.get();
        self.material_upload_probe.set(super::MaterialUploadProbe {
            passes: p.passes + 1,
            regions: p.regions + regions,
        });
    }

    /// The frame-in-flight slot index the NEXT [`present_sampled`](Self::present_sampled)
    /// / [`render_frame`](Self::render_frame) call will use (round-robin in
    /// `0..FRAMES_IN_FLIGHT`).
    ///
    /// The render host reads this to select WHICH per-frame UI ring slot + bind-group
    /// to upload into and re-resolve for the [`UiPass`] (GUI P5a MF-7): the host must
    /// pass this exact index to `RhiContext::ui_upload` / `ui_handles` so the slot it
    /// writes + binds matches the swapchain's in-flight fence this present waits on.
    #[inline]
    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    /// Total committed submits so far — the monotonic fence clock. Advances
    /// exactly once per successful `vkQueueSubmit` (the same site that advances
    /// [`frame_index`](Self::frame_index)), and only there: a pre-acquire
    /// out-of-date recreate returns before either counter moves. Used by the
    /// asset-retire gate: a resource enqueued at epoch `N` is safe to free once
    /// the host has observed epoch `>= N + FRAMES_IN_FLIGHT` (its last possible
    /// submit, `N`, is then guaranteed GPU-complete by the ring's own fence
    /// discipline).
    #[inline]
    pub fn submission_epoch(&self) -> u64 {
        self.submission_epoch
    }

    /// Blocks until the CURRENT [`frame_index`](Self::frame_index) slot's in-flight
    /// fence is signalled — i.e. the GPU has finished the submit two frames back that
    /// last used this slot's command buffer + per-frame resources.
    ///
    /// # The UI-ring write-after-read hazard this closes (GUI P5a)
    ///
    /// `present_sampled` ALSO waits this fence, but only at its START — AFTER the host
    /// has already memcpy'd this frame's instances into the per-frame UI ring slot.
    /// With `FRAMES_IN_FLIGHT == 2` that ring slot was last READ by the GPU in the
    /// submit two presents ago, whose fence is exactly this slot's in-flight fence; a
    /// host upload before that fence signals is a write-after-read race on a
    /// persistently-mapped, host-coherent buffer the GPU may still be reading.
    ///
    /// The G-buffer viewer's per-frame rings carry the SAME hazard (the camera UBO
    /// ring, the interp pair SSBO ring): the write does not race the SIBLING
    /// in-flight frame (it binds slot `s ^ 1`) but the slot's PREVIOUS OCCUPANT
    /// (frame N−2), whose late passes (marcher / deferred resolve) read the slot's
    /// buffer at GPU-execution time. With a static camera the overwrite is
    /// bitwise-identical and invisible; the moment the camera moves, the in-flight
    /// frame's lighting reconstructs world positions with a camera up to 2 frames
    /// NEWER than the one its G-buffer was rasterized with — a whole-face
    /// light/shadow flip that exists ONLY in motion (the `shadow_lag_dump`
    /// diagnostic's exact signature: ~200k differing px before this wait, 0 after).
    /// Every per-frame write into slot-indexed mapped memory must be preceded by
    /// this wait.
    ///
    /// The host therefore calls this IMMEDIATELY BEFORE `RhiContext::ui_upload` for the
    /// SAME `frame_index`, so the prior GPU read of that ring slot is complete before
    /// the memcpy. The fence is left SIGNALLED (not reset) — `present_sampled` resets
    /// it itself once it commits to a submit, so this extra wait is a pure no-op for
    /// `present_sampled`'s own discipline (an already-signalled fence wait returns
    /// immediately).
    ///
    /// Returns the [`FrameWriteToken`] write proof for the fenced slot — the
    /// required key for every per-slot host-write API (`RhiContext::ui_upload`,
    /// the interp pair ring): the write cannot be issued without the fence proof
    /// and cannot target a different slot than the one fenced. The token is
    /// affine: mid-frame writers borrow it, and the frame-ending submit
    /// ([`render_gbuffer_frame`](Self::render_gbuffer_frame) /
    /// [`present_sampled`](Self::present_sampled)) consumes it by value.
    ///
    /// # Errors
    /// [`SwapchainError::VkError`] if `vkWaitForFences` fails.
    pub fn wait_frame_in_flight(&self) -> Result<FrameWriteToken, SwapchainError> {
        let fence = self.frames[self.frame_index].in_flight;
        // SAFETY: `device` is live for `'ctx`; `&fence` names the current frame slot's
        // in-flight fence (created signalled in `new`, kept signalled between presents);
        // an infinite wait blocks until the last submit on this slot completed. The
        // fence is NOT reset here — `present_sampled` owns the reset on its commit path.
        let raw = unsafe {
            (self.fns.wait_for_fences)(self.device, 1, &fence, VK_TRUE, VK_TIMEOUT_INFINITE)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkWaitForFences", result));
        }
        Ok(FrameWriteToken { slot: self.frame_index })
    }

    /// The one shared acquire→record→submit→present skeleton behind every public
    /// frame method ([`render_frame`](Self::render_frame),
    /// [`render_scene_frame`](Self::render_scene_frame),
    /// [`present_sampled`](Self::present_sampled),
    /// [`render_gbuffer_frame`](Self::render_gbuffer_frame)). Each of those is a thin
    /// adapter that supplies its per-skeleton pre-record sync and record body through
    /// the two closures; the fence discipline, out-of-date recreation, submit, present,
    /// and frame-index advance below are IDENTICAL for all four.
    ///
    /// Control flow (unchanged from the four hand-written copies):
    /// 1. Wait this frame slot's in-flight fence (frees its command buffer + acquire
    ///    semaphore + any per-frame resources the previous submit on this slot used).
    /// 2. `pre_record`: the adapter syncs its extent-dependent targets NOW (after the
    ///    fence wait guarantees no in-flight frame still references the old resource,
    ///    before acquire). May also (re)declare a per-frame graph.
    /// 3. Acquire the next image. On `ERROR_OUT_OF_DATE_KHR`, recreate the swapchain
    ///    and return `Ok(false)` WITHOUT resetting the fence — the reset is deferred to
    ///    the commit path below so an out-of-date early return never leaves the fence
    ///    unsignalled (which would deadlock the next `vkWaitForFences` on this slot).
    /// 4. Reset the fence (we are now committing to a submit).
    /// 5. `record`: the adapter records its pass body into `cmd` against the acquired
    ///    swapchain `image`/`view` at `swapchain.extent`.
    /// 6. Submit (wait acquire @ COLOR_ATTACHMENT_OUTPUT, signal this image's
    ///    render-finished semaphore + this slot's in-flight fence).
    /// 7. Present (wait render-finished). On out-of-date / suboptimal, recreate and
    ///    return `Ok(false)`.
    /// 8. Advance `frame_index` (round-robin).
    ///
    /// Return / error semantics: `Ok(true)` presented, `Ok(false)` swapchain
    /// (re)created this call, `Err` terminal (a post-acquire failure leaves this slot's
    /// acquire semaphore signalled + its fence unsignalled — reuse would deadlock; the
    /// reset placement only rescues the pre-acquire out-of-date return).
    ///
    /// # Safety
    ///
    /// The `record` closure must record a complete, submittable command buffer into the
    /// `cmd` it is given (recordable because this slot's fence was just waited), naming
    /// only the swapchain `image`/`view` passed to it (which belong to `swapchain` and
    /// are the image acquired this frame) plus resources live on this device that
    /// outlive the submit. `pre_record` must leave any resource it (re)creates valid for
    /// the `record` that follows.
    ///
    /// `state` is the adapter's per-skeleton payload (e.g. `&mut Scene`, `(&scene,
    /// &mut frame)`) threaded to both closures by `&mut`, so an adapter can mutate it in
    /// `pre_record` (target sync) and read it in `record` without a double-borrow of the
    /// captured environment.
    #[allow(clippy::too_many_arguments)]
    unsafe fn drive_frame<S>(
        &mut self,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        width: u32,
        height: u32,
        state: &mut S,
        pre_record: impl FnOnce(&mut Self, &mut S) -> Result<(), SwapchainError>,
        record: impl FnOnce(
            &mut Self,
            &mut S,
            VkCommandBuffer,
            VkImage,
            VkImageView,
            VkExtent2D,
        ) -> Result<(), SwapchainError>,
    ) -> Result<bool, SwapchainError> {
        let fence = self.frames[self.frame_index].in_flight;
        let acquire_sem = self.frames[self.frame_index].acquire;

        // --- Wait this frame slot's in-flight fence (free its cmd buffer + resources). ---
        // SAFETY: `device` is live; `&fence` names this slot's fence; an infinite wait
        // blocks until this slot's previous submit completed, so its command buffer +
        // acquire semaphore (and any per-frame resource it used) are free to reuse.
        let raw = unsafe {
            (self.fns.wait_for_fences)(self.device, 1, &fence, VK_TRUE, VK_TIMEOUT_INFINITE)
        };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkWaitForFences", result));
        }

        // Pre-record sync: the fence wait above guarantees no in-flight frame still
        // references the old extent-dependent resource, so the adapter may recreate it
        // here (before acquire).
        pre_record(self, state)?;

        // --- Acquire the next image (signals this frame's acquire semaphore). ---
        let mut image_index: u32 = 0;
        // SAFETY: `device` + `swapchain` are live; an infinite timeout + this slot's
        // acquire semaphore (and null fence) is the standard acquire; `&mut image_index`
        // is a valid out-pointer.
        let raw = unsafe {
            (self.swap_fns.acquire_next_image)(
                self.device,
                swapchain.swapchain,
                VK_TIMEOUT_INFINITE,
                acquire_sem,
                VkFence::NULL,
                &mut image_index,
            )
        };
        let acquire_result = VkResult::from_raw(raw);
        if acquire_result == VkResult::ERROR_OUT_OF_DATE_KHR {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !acquire_result.is_success() && acquire_result != VkResult::SUBOPTIMAL_KHR {
            return Err(SwapchainError::VkError("vkAcquireNextImageKHR", acquire_result));
        }

        // Only reset the fence once we are committing to a submit (so an out-of-date
        // early return above does not leave the fence unsignalled, which would deadlock
        // the next wait).
        // SAFETY: `device` is live; `&fence` names this slot's already-waited fence;
        // resetting it is valid.
        let raw = unsafe { (self.fns.reset_fences)(self.device, 1, &fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkResetFences", result));
        }

        let cmd = self.command_buffers[self.frame_index];
        let image = swapchain_image_for(swapchain, image_index as usize);
        let view = swapchain.image_views[image_index as usize];
        let render_finished = self.render_finished[image_index as usize];
        let extent = swapchain.extent;

        // Record the adapter's pass body.
        // SAFETY: this slot's fence was just waited so `cmd` is recordable; the
        // image/view belong to `swapchain` (the image acquired this frame); the adapter
        // records only device-live resources per this fn's contract.
        record(self, state, cmd, image, view, extent)?;

        // --- Submit: wait acquire @ COLOR_ATTACHMENT_OUTPUT, signal render-finished + fence. ---
        let wait_stage: VkFlags = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
        let submit = VkSubmitInfo {
            s_type: VkStructureType::SubmitInfo,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: (&acquire_sem as *const VkSemaphore).cast(),
            p_wait_dst_stage_mask: &wait_stage,
            command_buffer_count: 1,
            p_command_buffers: &cmd,
            signal_semaphore_count: 1,
            p_signal_semaphores: (&render_finished as *const VkSemaphore).cast(),
        };
        // SAFETY: `queue` is the live present/graphics queue; one submit naming the
        // recorded `cmd`, waiting this frame's acquire semaphore at
        // COLOR_ATTACHMENT_OUTPUT, signalling this image's render-finished semaphore
        // + this frame's in-flight fence; all referenced locals outlive the call.
        let raw = unsafe { (self.fns.queue_submit)(self.queue, 1, &submit, fence) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkQueueSubmit", result));
        }

        // --- Present: wait render-finished. ---
        let present = VkPresentInfoKhr {
            s_type: VkStructureType::PresentInfoKhr,
            p_next: ptr::null(),
            wait_semaphore_count: 1,
            p_wait_semaphores: &render_finished,
            swapchain_count: 1,
            p_swapchains: &swapchain.swapchain,
            p_image_indices: &image_index,
            p_results: ptr::null_mut(),
        };
        // SAFETY: `queue` supports present (confirmed in `Surface::new`); the
        // present-info names the live swapchain + acquired `image_index`, waiting
        // this image's render-finished semaphore; all locals outlive the call.
        let raw = unsafe { (self.swap_fns.queue_present)(self.queue, &present) };
        let present_result = VkResult::from_raw(raw);

        self.frame_index = (self.frame_index + 1) % FRAMES_IN_FLIGHT;
        // Counts committed submits exactly (see `submission_epoch` doc); `u64`
        // never wraps in practice (~585 years at 1000 submits/s).
        self.submission_epoch = self.submission_epoch.wrapping_add(1);

        if present_result == VkResult::ERROR_OUT_OF_DATE_KHR
            || present_result == VkResult::SUBOPTIMAL_KHR
        {
            self.recreate(surface, swapchain, width, height)?;
            return Ok(false);
        }
        if !present_result.is_success() {
            return Err(SwapchainError::VkError("vkQueuePresentKHR", present_result));
        }
        Ok(true)
    }

    /// Renders + presents ONE cleared frame in `clear` (RGBA, 0..=1) to
    /// `swapchain`, recreating it on resize / out-of-date / suboptimal.
    ///
    /// Returns `Ok(true)` if the frame presented normally, `Ok(false)` if the
    /// swapchain was (re)created this call and the frame was skipped (the caller
    /// simply tries again next frame). A `ZeroExtent` (minimized window) is also
    /// reported as `Ok(false)`.
    ///
    /// An `Err` return is TERMINAL: drop the `Renderer` (recreate from scratch),
    /// do not call `render_frame` again. A failure *after* the image is acquired
    /// (record / submit error) leaves this frame slot's acquire semaphore
    /// signalled and its in-flight fence unsignalled — reuse would deadlock the
    /// next `vkWaitForFences` on that slot and trip the acquire-semaphore VUID.
    /// The reset placement above only protects the *out-of-date* early return,
    /// which has not yet acquired; it cannot rescue a post-acquire failure. The
    /// `window_clear` example and the integration test both treat `Err` as
    /// terminal.
    ///
    /// `width`/`height` are the window's current client size, used when a
    /// recreate is triggered.
    pub fn render_frame(
        &mut self,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        width: u32,
        height: u32,
        clear: [f32; 4],
    ) -> Result<bool, SwapchainError> {
        // Thin adapter over the shared [`drive_frame`](Self::drive_frame) skeleton:
        // no pre-record sync (the clear has no extent-dependent resource), and the
        // record body is the UNDEFINED→COLOR→PRESENT clear.
        // SAFETY: `drive_frame`'s contract — the record closure records a complete,
        // submittable command buffer into the recordable `cmd` naming only the acquired
        // swapchain `image`/`view`; `clear` is a finite RGBA. The recorded barriers +
        // dynamic rendering are the clear path.
        unsafe {
            self.drive_frame(
                surface,
                swapchain,
                width,
                height,
                &mut (),
                |_this, _state| Ok(()),
                |this, _state, cmd, image, view, extent| {
                    this.record_clear(cmd, image, view, extent, clear)
                },
            )
        }
    }

    /// Renders + presents ONE depth-tested SCENE frame (Phase-6 S1 rung 7 — the
    /// first real 3D geometry ON SCREEN) into `swapchain`, recreating it on resize /
    /// out-of-date / suboptimal.
    ///
    /// Unlike [`render_frame`](Self::render_frame) (which only clears), this binds
    /// `scene`'s graphics pipeline + vertex buffer + MVP push constant and draws a
    /// depth-tested mesh into the swapchain image against `scene`'s depth attachment
    /// (recreated to match the swapchain extent on resize, see [`Scene`]`::sync_depth`).
    ///
    /// `clear` is the background color the draw composites over.
    ///
    /// If `readback` is `Some`, on THIS frame — after the draw, before present — the
    /// rendered swapchain image is `vkCmdCopyImageToBuffer`'d into the supplied
    /// host-visible staging buffer (transitioning COLOR → TRANSFER_SRC → PRESENT
    /// instead of COLOR → PRESENT). This is the rung-7 acceptance test's golden
    /// readback path (proving real geometry reached the swapchain image, not just a
    /// clear); the steady present path passes `None` and pays nothing for it.
    ///
    /// Return / error semantics are identical to [`render_frame`](Self::render_frame):
    /// `Ok(true)` presented, `Ok(false)` swapchain (re)created this call, `Err`
    /// terminal.
    ///
    /// # Safety
    ///
    /// `scene`'s pipeline / vertex buffer were created on the same device as this
    /// renderer and outlive the call; `scene.depth` has been synced to `swapchain`'s
    /// current extent (the call does this via [`Scene`]`::sync_depth` when needed). A
    /// `Some(readback)` buffer must be a host-visible buffer of at least
    /// `extent.width * extent.height * 4` bytes (R8G8B8A8/B8G8R8A8 is 4 B/texel).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn render_scene_frame(
        &mut self,
        ctx: &VulkanContext,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        scene: &mut Scene,
        width: u32,
        height: u32,
        clear: [f32; 4],
        readback: Option<&BoundBuffer>,
    ) -> Result<bool, SwapchainError> {
        // The current swapchain extent to sync `scene.depth` against. Captured before
        // the driver runs: `sync_depth` happens after the fence wait and before acquire
        // (no recreate can move the extent in between).
        let sync_extent = swapchain.extent;
        // Thin adapter over the shared [`drive_frame`](Self::drive_frame) skeleton:
        // pre-record syncs the depth image to the swapchain extent; the record body is
        // the UNDEFINED→COLOR(+DEPTH)→draw→PRESENT (or →TRANSFER_SRC→readback→PRESENT)
        // scene path.
        // SAFETY: `drive_frame`'s contract — the record closure records a complete,
        // submittable command buffer into the recordable `cmd` naming the acquired
        // swapchain `image`/`view`; `scene` was created on this device and its depth is
        // synced to the swapchain extent by the pre-record closure; a `Some(readback)`
        // buffer is host-visible and ≥ the image's byte size (caller contract).
        unsafe {
            self.drive_frame(
                surface,
                swapchain,
                width,
                height,
                scene,
                // The fence wait above guarantees no in-flight frame still references
                // the old depth image, so recreating it here is safe. (The first call
                // creates it.)
                |_this, scene| scene.sync_depth(ctx, sync_extent),
                |this, scene, cmd, image, view, extent| {
                    this.record_scene(cmd, image, view, extent, clear, scene, readback)
                },
            )
        }
    }

    /// Presents the rung-11 SDF/mesh HYBRID COMPOSITE to the swapchain — the FIRST
    /// HYBRID FRAME ON SCREEN. The compute composite has already been uploaded into
    /// `composite.texture` (a SAMPLED `R8G8B8A8_UNORM` image left in
    /// `SHADER_READ_ONLY_OPTIMAL` by the caller's pre-loop one-time submit); this call
    /// only SAMPLES that resident texture in a fullscreen-sample graphics pass writing
    /// into the acquired swapchain image, so the GPU converts RGBA → the swapchain's
    /// format on the attachment write and the on-screen colors are correct on any
    /// swapchain format. There is no per-frame upload or per-frame transition of the
    /// composite texture — it is a pure read.
    ///
    /// The composite is presented at its NATIVE size
    /// ([`SampledComposite::texture_extent`]) in the TOP-LEFT of the swapchain image —
    /// the present pass's viewport/scissor are clamped to
    /// `min(swapchain_extent, texture_extent)`, so the composite maps 1:1 and is never
    /// stretched to a (possibly WSI-clamped) wider swapchain extent; the rest of the
    /// swapchain image stays `clear`. A scale-to-fill mode is a future addition.
    ///
    /// Because the composite texture is uploaded once and only ever read here, ALL
    /// frames-in-flight may sample it concurrently with no write-after-read hazard and
    /// no cross-frame fence/barrier on the texture (the per-frame sync below covers
    /// only the per-frame swapchain image + command buffer, exactly as the other
    /// present paths).
    ///
    /// Synchronization / recreate semantics are IDENTICAL to
    /// [`render_scene_frame`](Self::render_scene_frame): `Ok(true)` presented,
    /// `Ok(false)` swapchain (re)created this call (frame skipped), `Err` terminal.
    ///
    /// If `readback` is `Some`, on THIS frame — after the fullscreen draw, before
    /// present — the presented swapchain image is `vkCmdCopyImageToBuffer`'d into the
    /// supplied host-visible staging buffer (the rung-11 golden path, proving the
    /// hybrid composite reached the swapchain image); the steady path passes `None`.
    ///
    /// `token` is this frame's [`FrameWriteToken`] (minted by
    /// [`wait_frame_in_flight`](Self::wait_frame_in_flight)), consumed BY VALUE:
    /// the submit ends the frame's host-write window, so the affine consume makes
    /// a per-slot host write after this call a compile error (R0b). The token must
    /// belong to the slot being submitted (`token.slot() == frame_index`).
    ///
    /// # Safety
    ///
    /// Every resource borrowed by `composite` (texture / sampler / bind group /
    /// fullscreen pipeline) was created on the same device as this renderer and
    /// outlives the call; `composite.texture` is a SAMPLED image the caller has
    /// already uploaded the composite into and transitioned to
    /// `SHADER_READ_ONLY_OPTIMAL` (and never writes again); `composite.pipeline`'s
    /// `color_formats[0]` equals the swapchain surface format (W2-b) and its layout
    /// declares `composite.bind_group`'s set-0 layout; a `Some(readback)` buffer is
    /// host-visible and at least `extent.width * extent.height * 4` bytes (the
    /// swapchain image's size).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn present_sampled(
        &mut self,
        token: FrameWriteToken,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        composite: &SampledComposite<'_>,
        width: u32,
        height: u32,
        clear: [f32; 4],
        readback: Option<&BoundBuffer>,
        ui: Option<&UiPass<'_>>,
    ) -> Result<bool, SwapchainError> {
        debug_assert_eq!(
            token.slot(),
            self.frame_index,
            "invariant: the frame-write token must belong to the slot being submitted"
        );
        // The by-value consume ends this frame's host-write window (R0b).
        let _ = token;
        // Thin adapter over the shared [`drive_frame`](Self::drive_frame) skeleton: no
        // pre-record sync (the composite texture was uploaded once by the caller before
        // the present loop and is never written again — no extent-dependent target to
        // recreate here); the record body only SAMPLES that texture into the swapchain
        // (UNDEFINED → COLOR → PRESENT, or → TRANSFER_SRC → readback → PRESENT).
        // SAFETY: `drive_frame`'s contract — the record closure records a complete,
        // submittable command buffer into the recordable `cmd` naming the acquired
        // swapchain `image`/`view`; every `composite` resource is live on this device
        // and `composite.texture` is resident in SHADER_READ_ONLY_OPTIMAL (never
        // written here); a `Some(readback)` buffer is host-visible and ≥ the image's
        // byte size (caller contract).
        unsafe {
            self.drive_frame(
                surface,
                swapchain,
                width,
                height,
                &mut (),
                |_this, _state| Ok(()),
                |this, _state, cmd, image, view, extent| {
                    this.record_present_sampled(
                        cmd, image, view, extent, clear, composite, readback, ui,
                    )
                },
            )
        }
    }

    /// Renders + presents ONE on-screen Render-P1c **image-based** G-buffer frame: the
    /// P1b shared-depth marcher (the depth IMAGE source + the MRT G-buffer sink) driven
    /// ON SCREEN, killing the packed path's per-frame depth→buffer copy. Recreates the
    /// swapchain on resize / out-of-date / suboptimal (identical return semantics to
    /// [`render_scene_frame`](Self::render_scene_frame)).
    ///
    /// # The 3-pass on-screen frame (one command buffer, fence-only submit, §1b model)
    ///
    /// (A) raster the mesh quad → D32 depth IMAGE, (B) the SDF compute marcher samples
    /// that depth image + writes the FINAL composite into the ALBEDO storage image
    /// (byte-untouched from P1b), (C) present-blit: fullscreen-sample the ALBEDO into
    /// the acquired swapchain image, present. The deferred-lighting split (the marcher
    /// writing UNLIT attributes + a separate lighting pass) is DEFERRED to P7
    /// (multi-light/clustered) — P1b's marcher already writes the lit composite, so a
    /// P1c lighting pass would be a no-op passthrough that breaks the golden.
    ///
    /// There is NO `copy_image_to_buffer(depth)` and NO per-frame
    /// `vkUpdateDescriptorSets`: the marcher SAMPLES the depth image, and both
    /// descriptor sets are written ONCE per composite extent by
    /// [`GBufferTargets::sync_gbuffer`]. The G-buffer targets + the marcher's
    /// raster/dispatch are sized to `present_extent` (the composite), NOT the swapchain
    /// extent: a P0a/rung-11 WSI-clamped (wider) swapchain image never resizes the
    /// G-buffer — the present-blit maps the composite 1:1 into the swapchain's top-left.
    ///
    /// If `readback` is `Some`, on THIS frame the presented swapchain image is
    /// `vkCmdCopyImageToBuffer`'d into the supplied host-visible staging buffer (the
    /// on-screen golden readback path — proving the image-based composite reached the
    /// swapchain); the steady present path passes `None`.
    ///
    /// `vb_id_readback` is the VG-R0 rung R0c density census's SECOND, INDEPENDENT staging: a
    /// `Some` copies the VB path's `vb_id` image into it on THIS frame. Read by the `VisibilityBuffer`
    /// recorder alone — the Deferred and Forward paths own no `vb_id` and never see it. The two
    /// readbacks are deliberately separate parameters rather than one: they hash different images,
    /// at different extents (`extent` vs `present_extent`) and different texel widths (4 B vs 8 B),
    /// and the census is armed on frames the golden dump is not.
    ///
    /// `present_extent` is the composite's native size for the top-left 1:1 present
    /// (`min(swapchain_extent, present_extent)` clamps the present viewport/scissor, so
    /// the per-texel golden is exact regardless of the WSI extent clamp — the same
    /// 1:1-top-left contract [`SampledComposite`] uses). Pass the extent the marcher
    /// dispatched at (the clamped swapchain extent the caller sized `frame`'s targets
    /// + `scene.camera_uniform` + `scene.dispatch_group_count_x` to).
    ///
    /// `aa_extent` (SSAA) is the BOOT-FIXED native extent `aa_out` is sized to under
    /// `AaMode::Ssaa` (`host.native_extent`, NOT the live `width`/`height` — which track
    /// window resizes while the swapchain-only resize contract leaves `aa_out` fixed,
    /// exactly like `present_extent` already stays fixed under a resize). For
    /// Off/Fxaa/Smaa this SHOULD equal `present_extent` (byte-identical to before SSAA
    /// existed); the caller is responsible for that equality (this fn does not enforce
    /// it, since `aa_extent` is read only when `scene.ssaa.is_some()`).
    ///
    /// `token` is this frame's [`FrameWriteToken`] (minted by
    /// [`wait_frame_in_flight`](Self::wait_frame_in_flight)), consumed BY VALUE:
    /// the submit ends the frame's host-write window, so the affine consume makes
    /// a per-slot host write after this call a compile error (R0b). The token must
    /// belong to the slot being submitted (`token.slot() == frame_index`).
    ///
    /// # Safety
    ///
    /// Every `scene` resource was created on the same device as this renderer and
    /// outlives the call; `scene.edit_list` / `scene.camera_uniform` were host-seeded
    /// once before the present loop and are NEVER written again (the marcher only reads
    /// them — frames-in-flight dispatch against them with no host write-after-read);
    /// `frame`'s targets were synced to the swapchain extent (the call does this via
    /// [`GBufferTargets::sync_gbuffer`] when needed), and both
    /// `scene.dispatch_group_count_x` and `scene.camera_uniform`'s `count` were sized to
    /// that extent. Any readback buffer is host-visible and at least
    /// `swapchain.extent` * 4 bytes (4 B/texel); any `vb_id_readback` buffer is host-visible and
    /// at least `present_extent` * 8 bytes (`R32G32_UINT`, 8 B/texel — a DIFFERENT extent and a
    /// DIFFERENT texel width from the swapchain readback's, which is why the two contracts are
    /// stated separately).
    ///
    /// **The image-dump clause (OBS).** If the image dump is armed and a request is pending (handed
    /// by [`request_image_dump`](Self::request_image_dump) since the previous call; storing it
    /// checks nothing), every non-null `staging[i]` of that [`ImageDumpRequest`]:
    /// - is a live buffer created on this renderer's device, with its memory bound, host-visible,
    ///   and created with `TRANSFER_DST` usage;
    /// - is at least `capacity[i]` bytes large — the copy into it is bounded by `capacity[i]` and
    ///   by nothing else;
    /// - is referenced by no submission still pending on the device;
    /// - is neither destroyed nor used by another submission until the fence of the submission
    ///   carrying this frame has been waited (the next
    ///   [`wait_frame_in_flight`](Self::wait_frame_in_flight) of this slot, or a device-idle wait).
    ///
    /// This call consumes the request whether or not it records.
    ///
    /// VG R3 piece 2 step P2-6: `vb_record_probe` is gate G2's count sink
    /// ([`VbRecordProbe`](super::VbRecordProbe)) — host memory, no device resource, no contract
    /// beyond "the borrow outlives the call". It reaches only the `VisibilityBuffer` arm below;
    /// the Deferred/Forward recorders have no second raster scope to report and are handed
    /// nothing. `None` (every steady/golden/interactive frame) records byte-identical commands.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn render_gbuffer_frame(
        &mut self,
        token: FrameWriteToken,
        ctx: &VulkanContext,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        scene: &GBufferScene<'_>,
        frame: &mut GBufferFrame,
        width: u32,
        height: u32,
        clear: [f32; 4],
        present_extent: VkExtent2D,
        aa_extent: VkExtent2D,
        readback: Option<&BoundBuffer>,
        vb_id_readback: Option<&BoundBuffer>,
        vb_record_probe: Option<&mut VbRecordProbe>,
    ) -> Result<bool, SwapchainError> {
        debug_assert_eq!(
            token.slot(),
            self.frame_index,
            "invariant: the frame-write token must belong to the slot being submitted"
        );
        // The by-value consume ends this frame's host-write window (R0b).
        let _ = token;
        // Dynamic-materials DM1: a call that returns before recording reports nothing.
        self.material_upload_probe.set(super::MaterialUploadProbe::default());
        // OBS (D): likewise the image dump's copy record (`None` unless armed).
        if let Some(arm) = self.image_dump.as_ref() {
            arm.copies.set([ImageDumpCopy::NONE; IMAGE_DUMP_MAX]);
        }
        // OBS (D): the armed-only `TRANSFER_SRC` usage on the dumped Deferred targets — read here,
        // before the closure borrows `self`. No test sees this line: an unarmed run that passed
        // `true` would ship the usage bit with every gate green (see `dump_transfer_src`'s doc).
        let dump_usage = self.image_dump.is_some();
        // Thin adapter over the shared [`drive_frame`](Self::drive_frame) skeleton, with
        // `frame` (the G-buffer targets) threaded as the payload: pre-record syncs the
        // targets, the record body re-declares the whole-frame graph then records the
        // raster→depth-sample→march→present-blit (or →readback) 3-pass.
        // SAFETY: `drive_frame`'s contract — the record closure records a complete,
        // submittable command buffer into the recordable `cmd` naming the acquired
        // swapchain `image`/`view`; `scene`'s resources + the synced `targets` were
        // created on this device; any readback buffer is host-visible and ≥ the image's
        // byte size (caller contract); a pending image-dump request's staging meets the image-dump
        // clause of this fn's `# Safety` (caller contract): live on this device, host-visible,
        // `TRANSFER_DST`, at least `capacity[i]` bytes, referenced by no pending submission and
        // kept until this frame's fence is waited. The recorders state the same clause in their
        // own `# Safety`, and `record_image_dump_copies` bounds each copy by `capacity[i]`.
        let result = unsafe {
            self.drive_frame(
                surface,
                swapchain,
                width,
                height,
                frame,
                // Ensure the G-buffer targets (+ descriptor sets) match the COMPOSITE
                // (`present_extent`) — NOT the swapchain extent. The marcher dispatches +
                // rasterizes at `present_extent` (the golden composite size); the
                // present-blit maps that 1:1 into the swapchain's top-left, so a
                // WSI-clamped (wider) swapchain never resizes the G-buffer. The fence
                // wait above frees THIS slot; a REPLACE additionally waits idle for
                // sibling slots. (The first call creates them.) The descriptor sets are
                // written ONCE per composite extent.
                |_this, frame| {
                    // SSAA: `aa_out` is sized to the caller-supplied `aa_extent` (BOOT-FIXED
                    // native, not `present_extent` — 2× under SSAA) — see
                    // `GBufferTargets::sync_gbuffer`'s doc. For Off/Fxaa/Smaa,
                    // `aa_extent == present_extent` (byte-identical to before SSAA existed).
                    // Multi-paradigm render-path plan, rung R2: `TargetsProfile::from_scene`
                    // derives the (today: always-`DeferredFull`) profile the R3+ path-conditional
                    // allocation seam threads through.
                    GBufferTargets::sync_gbuffer(
                        &mut frame.targets,
                        ctx,
                        scene,
                        present_extent,
                        aa_extent,
                        TargetsProfile::from_scene(scene),
                        dump_usage,
                    )
                },
                |this, frame, cmd, image, view, extent| {
                    // The framegraph drives the frame: re-declare the WHOLE G-buffer
                    // frame (config-gated from `scene`) into `self.frame_graph` and store
                    // the resulting `GbufferPassPlan` — BEFORE the `&self`
                    // `record_gbuffer` borrow, which then reads the compiled per-pass
                    // barrier plan through it. Zero-alloc (`reset` retains capacity); a
                    // per-frame `compile` is cheap for a ~11-pass line. Multi-paradigm
                    // render-path plan, rung R2: dispatches through `declare_frame_graph`,
                    // which selects the per-path declarator from `scene.resolved_render_path
                    // .path` (today: always the `Deferred` arm, `declare_deferred_graph`).
                    this.declare_frame_graph(scene);

                    let targets = frame.targets.as_ref().expect(
                        "invariant: sync_gbuffer made the targets present before record",
                    );

                    // Multi-paradigm render-path plan, rung R4b-b: mirrors
                    // `declare_frame_graph`'s dispatch (`scene.path_is_forward()` is the SAME
                    // predicate that fn's caller — `declare_forward_graph` vs
                    // `declare_deferred_graph` — used, the W1 single-predicate discipline). A
                    // `Forward`-resolved frame records `record_forward` instead of
                    // `record_gbuffer`; the two are mutually exclusive per boot (Decision 1).
                    let recorded = if scene.path_is_vb() {
                        // Multi-paradigm render-path plan, rung R8: mirrors the `path_is_forward()`
                        // dispatch immediately below (the SAME W1 single-predicate discipline) — a
                        // `VisibilityBuffer`-resolved frame records `record_vb` instead of
                        // `record_forward`/`record_gbuffer`; the three are mutually exclusive per
                        // boot (Decision 1). VB REUSES `ForwardTargets` (depth ring + Set-1 shadow
                        // set) alongside its OWN `VbTargets` (`vb_id` ring) — `VbTargets`'s doc.
                        let fwd = targets.forward.as_ref().expect(
                            "invariant: TargetsProfile::VbMesh built ForwardTargets before record",
                        );
                        let vb = targets.vb.as_ref().expect(
                            "invariant: TargetsProfile::VbMesh built VbTargets before record",
                        );
                        this.record_vb(
                            cmd,
                            image,
                            view,
                            extent,
                            present_extent,
                            aa_extent,
                            clear,
                            scene,
                            targets,
                            fwd,
                            vb,
                            readback,
                            vb_id_readback,
                            // VG R3 piece 2 step P2-6: gate G2's count sink, moved in (the record
                            // closure is `FnOnce`). The VB arm is the only one that can report a
                            // second raster scope, so this is the only arm it reaches.
                            vb_record_probe,
                            // Rung R9d: the AS command table (for the VB split's own per-frame
                            // TLAS build), the SAME resolve `record_gbuffer`'s own call uses
                            // below.
                            #[cfg(feature = "hwrt")]
                            ctx.accel_fns_opt(),
                        )
                    } else if scene.path_is_forward() {
                        let fwd = targets.forward.as_ref().expect(
                            "invariant: TargetsProfile::ForwardMesh built ForwardTargets before record",
                        );
                        this.record_forward(
                            cmd,
                            image,
                            view,
                            extent,
                            present_extent,
                            clear,
                            scene,
                            targets,
                            fwd,
                            readback,
                        )
                    } else {
                        this.record_gbuffer(
                            cmd,
                            image,
                            view,
                            extent,
                            present_extent,
                            aa_extent,
                            clear,
                            scene,
                            targets,
                            readback,
                            // HW-RT rung R2a-3: the AS command table (for the per-frame TLAS build),
                            // resolved from `ctx` — `None` on a non-RT device (the recorder then skips
                            // the tlas block even if `scene.tlas` were somehow `Some`). Gated so the
                            // `not(hwrt)` signature is unchanged.
                            #[cfg(feature = "hwrt")]
                            ctx.accel_fns_opt(),
                        )
                    };
                    // OBS (R): refresh the receipt capture from THIS frame's scene and sink fills
                    // (`None` on every shipped run — one branch, the capture itself is cold).
                    if this.receipt.is_some() {
                        this.capture_receipt(scene, targets);
                    }
                    recorded
                },
            )
        };
        // OBS (D): a request is for ONE frame — consumed by this call whether or not it recorded.
        if let Some(arm) = self.image_dump.as_mut() {
            arm.request = None;
        }
        result
    }

    /// Waits the device idle, recreates the swapchain to `width`×`height`, and
    /// rebuilds the per-image render-finished semaphores (the image count may
    /// change). A `ZeroExtent` (minimized) is swallowed — the caller retries.
    fn recreate(
        &mut self,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        width: u32,
        height: u32,
    ) -> Result<(), SwapchainError> {
        // SAFETY: `device` is live; waiting idle guarantees no command buffer /
        // image view / semaphore is in use before we destroy + recreate them.
        unsafe { (self.fns.device_wait_idle)(self.device) };

        match swapchain.recreate(surface, width, height) {
            Ok(()) => {}
            // A minimized window has a zero extent: keep the old (now-idle)
            // swapchain and report "skipped"; the next frame retries.
            Err(SwapchainError::ZeroExtent) => return Ok(()),
            Err(e) => return Err(e),
        }

        // Rebuild render-finished semaphores to match the new image count.
        // SAFETY: device is idle; each old semaphore is destroyed once.
        unsafe {
            for &s in &self.render_finished {
                (self.fns.destroy_semaphore)(self.device, s, ptr::null());
            }
        }
        self.render_finished.clear();
        for _ in 0..swapchain.image_count() {
            self.render_finished.push(create_semaphore(self.fns, self.device)?);
        }
        self.frame_index = 0;
        Ok(())
    }
}

/// Compile-time proof that ONE frames-in-flight slot is currently safe to
/// host-write: its in-flight fence was waited THIS frame
/// ([`Renderer::wait_frame_in_flight`]), or nothing submitted can reference it
/// yet ([`Self::forge_unfenced`]).
///
/// Per-slot host-write APIs (the UI instance ring, the interp pair ring, the
/// camera UBO ring) take this token INSTEAD of a raw slot index — the
/// `DispatcherToken` pattern: the write cannot be issued without the fence
/// proof, and it cannot target a different slot than the one fenced. The token
/// is AFFINE (`!Clone`, `!Copy`, R0b): mid-frame per-slot writers borrow it
/// (`&FrameWriteToken`), and the frame-ending submit APIs
/// ([`Renderer::render_gbuffer_frame`] / [`Renderer::present_sampled`]) consume
/// it BY VALUE — so a per-slot host write after the submit, or a token retained
/// across frames, is a compile error, not a documented convention. Zero runtime
/// cost (one `usize` moved instead of copied). A write placed before the fence
/// wait does not race the SIBLING in-flight frame (it binds slot `s ^ 1`) but
/// the slot's PREVIOUS OCCUPANT (frame N−2 under `FRAMES_IN_FLIGHT == 2`) — the
/// motion-only whole-face shadow flip the `shadow_lag_dump` diagnostic pins.
#[derive(Debug)]
pub struct FrameWriteToken {
    slot: usize,
}

impl FrameWriteToken {
    /// The fenced frame slot — the ONLY ring slot the holder may host-write.
    #[inline]
    pub fn slot(&self) -> usize {
        self.slot
    }

    /// Forges the write proof WITHOUT a fence wait — for setup-time seeding only.
    ///
    /// # Safety
    ///
    /// The caller guarantees NO submitted GPU work can still reference slot
    /// `slot`'s per-frame resources: the buffers were just created, the present
    /// loop has not started, or the device was idled (`device_wait_idle` /
    /// swapchain recreate).
    #[inline]
    pub unsafe fn forge_unfenced(slot: usize) -> Self {
        Self { slot }
    }
}

impl Drop for Renderer<'_> {
    fn drop(&mut self) {
        // SAFETY: `device` is live; waiting idle ensures no command buffer /
        // semaphore / fence is in use. Then every sync object is destroyed once in
        // reverse creation order, and the command pool (which frees its command
        // buffers) last. The render-finished + per-frame semaphores and fences all
        // belong to this device.
        unsafe {
            (self.fns.device_wait_idle)(self.device);
            for &s in &self.render_finished {
                (self.fns.destroy_semaphore)(self.device, s, ptr::null());
            }
            for slot in &self.frames {
                (self.fns.destroy_fence)(self.device, slot.in_flight, ptr::null());
                (self.fns.destroy_semaphore)(self.device, slot.acquire, ptr::null());
            }
            (self.fns.destroy_command_pool)(self.device, self.command_pool, ptr::null());
        }
    }
}

// ---------------------------------------------------------------------------
// Frame-sync creation helpers.
// ---------------------------------------------------------------------------

/// Creates an unsignalled binary semaphore.
fn create_semaphore(fns: &DeviceFns, device: VkDevice) -> Result<VkSemaphore, SwapchainError> {
    let ci = VkSemaphoreCreateInfo {
        s_type: VkStructureType::SemaphoreCreateInfo,
        p_next: ptr::null(),
        flags: 0,
    };
    let mut sem = VkSemaphore::NULL;
    // SAFETY: `device` is live; `ci` is a fully-initialized create-info; `&mut
    // sem` is a valid out-pointer; NULL allocator.
    let raw = unsafe { (fns.create_semaphore)(device, &ci, ptr::null(), &mut sem) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(SwapchainError::VkError("vkCreateSemaphore", result));
    }
    Ok(sem)
}

/// `VkFenceCreateFlagBits::VK_FENCE_CREATE_SIGNALED_BIT`.
const VK_FENCE_CREATE_SIGNALED_BIT: VkFlags = 0x0000_0001;

/// Creates a fence in the SIGNALLED state (so the first per-frame wait returns
/// immediately rather than deadlocking on a never-submitted fence).
fn create_fence_signalled(fns: &DeviceFns, device: VkDevice) -> Result<VkFence, SwapchainError> {
    let ci = VkFenceCreateInfo {
        s_type: VkStructureType::FenceCreateInfo,
        p_next: ptr::null(),
        flags: VK_FENCE_CREATE_SIGNALED_BIT,
    };
    let mut fence = VkFence::NULL;
    // SAFETY: `device` is live; `ci` is a fully-initialized signalled create-info;
    // `&mut fence` is a valid out-pointer; NULL allocator.
    let raw = unsafe { (fns.create_fence)(device, &ci, ptr::null(), &mut fence) };
    let result = VkResult::from_raw(raw);
    if !result.is_success() {
        return Err(SwapchainError::VkError("vkCreateFence", result));
    }
    Ok(fence)
}

/// Destroys whatever frame-sync + render-finished objects were created so far on
/// a `Renderer::new` error path (NULL handles are skipped).
///
/// # Safety
///
/// Every non-null handle in `frames` / `render_finished` must have been created
/// on `device` and not yet destroyed.
unsafe fn destroy_partial_frames(
    fns: &DeviceFns,
    device: VkDevice,
    frames: &[FrameSync],
    render_finished: &[VkSemaphore],
) {
    // SAFETY: each non-null handle was created on `device` per this fn's contract
    // and is destroyed exactly once here.
    unsafe {
        for &s in render_finished {
            if !s.is_null() {
                (fns.destroy_semaphore)(device, s, ptr::null());
            }
        }
        for slot in frames {
            if !slot.in_flight.is_null() {
                (fns.destroy_fence)(device, slot.in_flight, ptr::null());
            }
            if !slot.acquire.is_null() {
                (fns.destroy_semaphore)(device, slot.acquire, ptr::null());
            }
        }
    }
}

/// The named frame-graph reserve (OBS, AD-12): the `(max_res, max_pass, max_acc)`
/// [`FrameGraph::with_capacity`](crate::framegraph::FrameGraph::with_capacity) is called with at
/// boot, replacing the literal `(48, 32, 192)`.
///
/// A FIRST-FRAME ALLOCATION HINT, not a bound: a frame past it regrows an arena once (cold) and is
/// never incorrect. `max_res` covers VB under `hwrt` with every tail armed (53 resources, 69
/// mip-weighted `(ResId, mip)` entries with the 17-level HZB pyramid) with headroom; `max_pass`
/// and `max_acc` double the parent's literal. The cost is a one-time boot reservation of roughly
/// 50 KB across the arenas (the access-sized ones dominate: five access columns plus the image
/// and buffer barrier arenas at `max_acc`). The `BOYKO_GRAPH_RECEIPT` instrument prints each
/// declarator's high-water marks beside these numbers; if a receipt shows a mark past one of
/// them, raise the constant to that mark plus headroom in the same commit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrameGraphCapacity {
    /// Resources (the per-resource arenas; the sync arenas are mip-weighted past it).
    pub max_res: usize,
    /// Passes.
    pub max_pass: usize,
    /// Accesses, summed over every pass of the frame (also sizes the barrier arenas).
    pub max_acc: usize,
}

/// The reserve every [`Renderer`] boots its frame graph with (see [`FrameGraphCapacity`]).
pub const FRAME_GRAPH_CAPACITY: FrameGraphCapacity =
    FrameGraphCapacity { max_res: 96, max_pass: 64, max_acc: 384 };

// --- OBS (R): the `BOYKO_GRAPH_RECEIPT` capture ---------------------------------------------

/// The `[config]` keys of a graph receipt, in the order they are written: the values of the
/// single-predicate functions the declarator and the recorder both read (`GBufferScene::path_*`),
/// then the per-frame arming facts the conditional tails key on. Each later rung APPENDS its keys
/// (a reordering would silently re-label a parsed column).
///
/// SkyLight presence is recorded as the presence of the two sky-background pipelines
/// (`forward_sky`, `vb_sky`): the scene carries no SkyLight bit of its own at HEAD.
pub const RECEIPT_CONFIG_KEYS: [&str; RECEIPT_CONFIG_LEN] = [
    "path",
    "legs",
    "mesh_leg",
    "sdf_leg",
    "path_is_forward",
    "path_is_forward_plus",
    "path_is_vb",
    "path_has_raster",
    "path_has_marcher",
    "path_has_mesh_depth_neutral_clear",
    "path_needs_depth_prepass",
    "path_has_sdf_forward",
    "path_sdf_forward_writes_viewt",
    "path_vb_split",
    "path_vb_fused",
    "path_vb_ssao",
    "path_vb_ddgi",
    "path_vb_hwrt_shadow",
    "path_vb_occlusion_split",
    "path_has_particles",
    "material_upload",
    "interp",
    "csm",
    "atlas_punctual",
    "ssao",
    "ddgi",
    "taa",
    "fxaa",
    "smaa",
    "ssaa",
    "rcas",
    "hzb",
    "tlas",
    "shadow_vis",
    "forward_sky",
    "vb_sky",
    "froxel_light_cull",
    "light_cull",
    "cluster_cull_hier",
    "lighting_flags",
    "hwrt",
];

/// How many `[config]` entries a receipt carries (the length of [`RECEIPT_CONFIG_KEYS`]).
pub const RECEIPT_CONFIG_LEN: usize = 41;

/// One frame's `[config]` values, index for index with [`RECEIPT_CONFIG_KEYS`] (`bool`s as 0/1).
/// POD: a burst copies one per presented frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReceiptConfig {
    /// The values, in key order.
    pub values: [u32; RECEIPT_CONFIG_LEN],
}

impl ReceiptConfig {
    /// The all-zero table (no frame captured yet).
    pub const EMPTY: Self = Self { values: [0; RECEIPT_CONFIG_LEN] };

    /// The value of `key`, or `None` for a key this build does not record.
    pub fn get(&self, key: &str) -> Option<u32> {
        RECEIPT_CONFIG_KEYS.iter().position(|k| *k == key).map(|i| self.values[i])
    }

    /// `(key, value)` pairs in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, u32)> + '_ {
        RECEIPT_CONFIG_KEYS.iter().copied().zip(self.values.iter().copied())
    }

    /// Reads one frame's scene through the same predicates its declarator and recorder use.
    pub(crate) fn capture(scene: &GBufferScene<'_>) -> Self {
        let rp = &scene.resolved_render_path;
        #[cfg(feature = "hwrt")]
        let (vb_hwrt_shadow, tlas, shadow_vis) =
            (scene.path_vb_hwrt_shadow(), scene.tlas.is_some(), scene.shadow.is_some());
        #[cfg(not(feature = "hwrt"))]
        let (vb_hwrt_shadow, tlas, shadow_vis) = (false, false, false);
        let values = [
            rp.path,
            rp.legs,
            u32::from(rp.mesh_leg),
            u32::from(rp.sdf_leg),
            u32::from(scene.path_is_forward()),
            u32::from(scene.path_is_forward_plus()),
            u32::from(scene.path_is_vb()),
            u32::from(scene.path_has_raster()),
            u32::from(scene.path_has_marcher()),
            u32::from(scene.path_has_mesh_depth_neutral_clear()),
            u32::from(scene.path_needs_depth_prepass()),
            u32::from(scene.path_has_sdf_forward()),
            u32::from(scene.path_sdf_forward_writes_viewt()),
            u32::from(scene.path_vb_split()),
            u32::from(scene.path_vb_fused()),
            u32::from(scene.path_vb_ssao()),
            u32::from(scene.path_vb_ddgi()),
            u32::from(vb_hwrt_shadow),
            u32::from(scene.path_vb_occlusion_split()),
            u32::from(scene.path_has_particles()),
            u32::from(scene.material_upload.is_some()),
            u32::from(scene.interp.is_some()),
            u32::from(scene.csm.is_some()),
            u32::from(scene.atlas_punctual.is_some()),
            u32::from(scene.ssao.is_some()),
            u32::from(scene.ddgi_update.is_some()),
            u32::from(scene.taa.is_some()),
            u32::from(scene.aa.is_some()),
            u32::from(scene.smaa.is_some()),
            u32::from(scene.ssaa.is_some()),
            u32::from(scene.rcas.is_some()),
            u32::from(scene.hzb.is_some()),
            u32::from(tlas),
            u32::from(shadow_vis),
            u32::from(scene.forward_sky_pipeline.is_some()),
            u32::from(scene.vb_sky_pipeline.is_some()),
            u32::from(rp.froxel_light_cull),
            u32::from(scene.cluster_cull.is_some()),
            u32::from(scene.cluster_cull_hier.is_some()),
            scene.lighting_flags,
            u32::from(cfg!(feature = "hwrt")),
        ];
        Self { values }
    }
}

/// The most sink slots of one kind a receipt records — every declarator's image and buffer array
/// fits (asserted where the capture copies them).
pub const RECEIPT_MAX_SINK_SLOTS: usize = 48;

/// One frame's receipt capture, refreshed inside every recorded frame while the
/// `BOYKO_GRAPH_RECEIPT` instrument is armed ([`Renderer::arm_graph_receipt`]).
///
/// It holds what the instrument cannot read from outside this crate: the `[config]` predicates
/// (`pub(crate)` on `GBufferScene`) and the sink fills' name / `Fif` columns. Inline POD — no
/// heap; a run without the instrument never builds one.
#[derive(Clone, Copy, Debug)]
pub struct ReceiptCapture {
    /// The last recorded frame's `[config]`.
    pub config: ReceiptConfig,
    /// The declarator that frame dispatched to (`seed_class::DECL_*`).
    pub declarator: u8,
    /// Frames captured since arming (a stale capture is visible as an unchanged count).
    pub frames: u64,
    /// `true` iff the sink columns below were filled: the slot metadata exists only in debug
    /// builds (release fills compile to handle-only arrays), so a release receipt has NO sink
    /// rows and must say so rather than read as an empty, clean table.
    pub sink_rows: bool,
    /// The declarator's image-array length: buffer slot `b` resolves `ResId(image_slots + b)`.
    pub image_slots: u16,
    /// The buffer-array length.
    pub buffer_slots: u16,
    /// The image slots' metadata (`None` past `image_slots`, or for a slot the fill left empty).
    pub image_sinks: [Option<super::seed_class::SinkMeta>; RECEIPT_MAX_SINK_SLOTS],
    /// The buffer slots' metadata.
    pub buffer_sinks: [Option<super::seed_class::SinkMeta>; RECEIPT_MAX_SINK_SLOTS],
}

impl ReceiptCapture {
    /// Nothing captured yet.
    pub const EMPTY: Self = Self {
        config: ReceiptConfig::EMPTY,
        declarator: 0,
        frames: 0,
        sink_rows: false,
        image_slots: 0,
        buffer_slots: 0,
        image_sinks: [None; RECEIPT_MAX_SINK_SLOTS],
        buffer_sinks: [None; RECEIPT_MAX_SINK_SLOTS],
    };

    /// Copies one declarator's sink metadata (debug builds only — the columns exist only there).
    #[cfg(debug_assertions)]
    pub(crate) fn set_sinks(
        &mut self,
        declarator: u8,
        images: &[Option<super::seed_class::SinkMeta>],
        buffers: &[Option<super::seed_class::SinkMeta>],
    ) {
        assert!(
            images.len() <= RECEIPT_MAX_SINK_SLOTS && buffers.len() <= RECEIPT_MAX_SINK_SLOTS,
            "invariant: RECEIPT_MAX_SINK_SLOTS covers every declarator's sink arrays"
        );
        self.declarator = declarator;
        self.sink_rows = true;
        self.image_slots = images.len() as u16;
        self.buffer_slots = buffers.len() as u16;
        self.image_sinks = [None; RECEIPT_MAX_SINK_SLOTS];
        self.buffer_sinks = [None; RECEIPT_MAX_SINK_SLOTS];
        self.image_sinks[..images.len()].copy_from_slice(images);
        self.buffer_sinks[..buffers.len()].copy_from_slice(buffers);
    }
}

// --- OBS (D): the `BOYKO_IMAGE_DUMP` instrument ---------------------------------------------

/// A frame-graph image the `BOYKO_IMAGE_DUMP` instrument can read back (H0 Scope 6). The env
/// word is the image's literal graph name; a declarator copies only the names it declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ImageDumpName {
    /// Deferred `depth` (D32, the Euclidean `t / 64` encode).
    DeferredDepth = 0,
    /// `viewt` (Deferred and VB; R32_SFLOAT).
    Viewt = 1,
    /// `lit` (every path).
    Lit = 2,
    /// `taa_hist` — the `[fi]` write slot (Deferred and VB, under TAA).
    TaaHist = 3,
    /// Forward / ForwardPlus `forward_depth` (D32 reverse-Z).
    ForwardDepth = 4,
    /// VB `vb_depth` (D32 reverse-Z).
    VbDepth = 5,
}

/// How many [`ImageDumpName`]s exist.
pub const IMAGE_DUMP_MAX: usize = 6;

impl ImageDumpName {
    /// Every name, index for index with `name as usize`.
    pub const ALL: [Self; IMAGE_DUMP_MAX] =
        [Self::DeferredDepth, Self::Viewt, Self::Lit, Self::TaaHist, Self::ForwardDepth, Self::VbDepth];

    /// The literal graph name — the env word and the dump file's stem.
    pub const fn graph_name(self) -> &'static str {
        match self {
            Self::DeferredDepth => "depth",
            Self::Viewt => "viewt",
            Self::Lit => "lit",
            Self::TaaHist => "taa_hist",
            Self::ForwardDepth => "forward_depth",
            Self::VbDepth => "vb_depth",
        }
    }

    /// The name an env word spells, if any.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|n| n.graph_name() == word)
    }

    /// `true` for the depth attachments (copied through the DEPTH aspect).
    pub const fn is_depth(self) -> bool {
        matches!(self, Self::DeferredDepth | Self::ForwardDepth | Self::VbDepth)
    }

    /// The bytes per texel the instrument sizes this name's staging for: 8 for `taa_hist`
    /// (RGBA16F), 4 for every other name. The recorder re-bounds every copy against the texel
    /// size of the image's actual format, so a format change can only skip a copy, never overrun.
    pub const fn staging_texel_bytes(self) -> u64 {
        match self {
            Self::TaaHist => 8,
            _ => 4,
        }
    }
}

/// The texel size of a `VkFormat` the image dump can copy, or `None` for a format it does not
/// decode (the copy is then skipped and reported as not copied).
pub const fn image_dump_texel_bytes(vk_format: i32) -> Option<u32> {
    match vk_format {
        VK_FORMAT_R8G8B8A8_UNORM
        | VK_FORMAT_R8G8B8A8_SRGB
        | VK_FORMAT_B8G8R8A8_UNORM
        | VK_FORMAT_B8G8R8A8_SRGB
        | VK_FORMAT_R32_SFLOAT
        | VK_FORMAT_D32_SFLOAT => Some(4),
        VK_FORMAT_R16G16B16A16_SFLOAT => Some(8),
        VK_FORMAT_R32G32B32A32_SFLOAT => Some(16),
        _ => None,
    }
}

/// One frame's dump request: per [`ImageDumpName`] (index = `name as usize`), the staging buffer
/// the copy writes and its capacity (`VkBuffer::NULL` = not requested). What each non-null handle
/// must satisfy is the image-dump clause of
/// [`Renderer::render_gbuffer_frame`](Renderer::render_gbuffer_frame)'s `# Safety`.
#[derive(Clone, Copy)]
pub struct ImageDumpRequest {
    /// The destination per name.
    pub staging: [VkBuffer; IMAGE_DUMP_MAX],
    /// Each destination's size in bytes. It must not exceed the buffer's real size: it is the
    /// only bound on the copy.
    pub capacity: [u64; IMAGE_DUMP_MAX],
}

impl ImageDumpRequest {
    /// `true` iff `name` was requested.
    #[inline]
    pub fn wants(&self, name: ImageDumpName) -> bool {
        !self.staging[name as usize].is_null()
    }
}

/// What the recorder copied for one name on the last frame — the dump file's header fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageDumpCopy {
    /// `true` iff the copy was recorded (the declarator declared the name and the staging held it).
    pub copied: bool,
    /// The image's `VkFormat`.
    pub vk_format: i32,
    /// The copied extent (mip 0).
    pub width: u32,
    /// See [`Self::width`].
    pub height: u32,
    /// The mip copied.
    pub mip: u32,
    /// The array layers copied.
    pub layers: u32,
    /// The engine frame index the recording frame carried.
    pub frame_index: u32,
    /// Bytes written into the staging (tightly packed rows).
    pub bytes: u64,
}

impl ImageDumpCopy {
    /// Nothing copied.
    pub const NONE: Self =
        Self { copied: false, vk_format: 0, width: 0, height: 0, mip: 0, layers: 0, frame_index: 0, bytes: 0 };
}

/// The armed instrument's per-frame state on the [`Renderer`].
pub(crate) struct ImageDumpArm {
    /// This frame's request (consumed by `render_gbuffer_frame`).
    pub(crate) request: Option<ImageDumpRequest>,
    /// What the recorder copied this frame — a `Cell` because the recorders borrow the renderer
    /// shared (one thread records), the `material_upload_probe` precedent.
    pub(crate) copies: core::cell::Cell<[ImageDumpCopy; IMAGE_DUMP_MAX]>,
}
