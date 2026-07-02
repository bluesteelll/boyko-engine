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

use super::graph_bridge::GbufferPassPlan;
use super::scene_types::{GBufferScene, SampledComposite, Scene, UiPass};
use super::swapchain::swapchain_image_for;
use super::targets::{GBufferFrame, GBufferTargets};
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
    pub(crate) gbuffer_pass_plan: Option<GbufferPassPlan>,
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
        let mut frame_graph = crate::framegraph::FrameGraph::with_capacity(16, 16, 64);
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
            frame_graph,
            gbuffer_pass_plan: None,
        })
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
    /// # Errors
    /// [`SwapchainError::VkError`] if `vkWaitForFences` fails.
    pub fn wait_frame_in_flight(&self) -> Result<(), SwapchainError> {
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
        Ok(())
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
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        composite: &SampledComposite<'_>,
        width: u32,
        height: u32,
        clear: [f32; 4],
        readback: Option<&BoundBuffer>,
        ui: Option<&UiPass<'_>>,
    ) -> Result<bool, SwapchainError> {
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
    /// `present_extent` is the composite's native size for the top-left 1:1 present
    /// (`min(swapchain_extent, present_extent)` clamps the present viewport/scissor, so
    /// the per-texel golden is exact regardless of the WSI extent clamp — the same
    /// 1:1-top-left contract [`SampledComposite`] uses). Pass the extent the marcher
    /// dispatched at (the clamped swapchain extent the caller sized `frame`'s targets
    /// + `scene.camera_uniform` + `scene.dispatch_group_count_x` to).
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
    /// `swapchain.extent` * 4 bytes (4 B/texel).
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn render_gbuffer_frame(
        &mut self,
        ctx: &VulkanContext,
        surface: &Surface<'_>,
        swapchain: &mut Swapchain<'ctx>,
        scene: &GBufferScene<'_>,
        frame: &mut GBufferFrame,
        width: u32,
        height: u32,
        clear: [f32; 4],
        present_extent: VkExtent2D,
        readback: Option<&BoundBuffer>,
    ) -> Result<bool, SwapchainError> {
        // Thin adapter over the shared [`drive_frame`](Self::drive_frame) skeleton, with
        // `frame` (the G-buffer targets) threaded as the payload: pre-record syncs the
        // targets, the record body re-declares the whole-frame graph then records the
        // raster→depth-sample→march→present-blit (or →readback) 3-pass.
        // SAFETY: `drive_frame`'s contract — the record closure records a complete,
        // submittable command buffer into the recordable `cmd` naming the acquired
        // swapchain `image`/`view`; `scene`'s resources + the synced `targets` were
        // created on this device; any readback buffer is host-visible and ≥ the image's
        // byte size (caller contract).
        unsafe {
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
                    GBufferTargets::sync_gbuffer(&mut frame.targets, ctx, scene, present_extent)
                },
                |this, frame, cmd, image, view, extent| {
                    // The framegraph drives the frame: re-declare the WHOLE G-buffer
                    // frame (config-gated from `scene`) into `self.frame_graph` and store
                    // the resulting `GbufferPassPlan` — BEFORE the `&self`
                    // `record_gbuffer` borrow, which then reads the compiled per-pass
                    // barrier plan through it. Zero-alloc (`reset` retains capacity); a
                    // per-frame `compile` is cheap for a ~11-pass line.
                    this.declare_gbuffer_graph(scene);

                    let targets = frame.targets.as_ref().expect(
                        "invariant: sync_gbuffer made the targets present before record",
                    );

                    this.record_gbuffer(
                        cmd,
                        image,
                        view,
                        extent,
                        present_extent,
                        clear,
                        scene,
                        targets,
                        readback,
                    )
                },
            )
        }
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
