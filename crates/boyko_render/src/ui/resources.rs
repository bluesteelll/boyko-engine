//! The owned UI render capability sub-owner (`UiRenderResources`) — GUI P5a
//! Rung 3 / Decision 7 + 8.
//!
//! This is the first-class, `Drop`-wired owner of every GPU resource the on-screen
//! UI pass needs (a NAMED owner on [`RhiContext`], NOT a side store — Principle 0):
//!
//! - the UI graphics pipeline (built once for a `color_format`, blend = premultiplied),
//! - the bind-group layout (one `StorageBuffer` @ set0/binding0, VERTEX|FRAGMENT),
//! - one persistent host-mapped grow-only STORAGE ring + one bind-group PER
//!   [`FRAMES_IN_FLIGHT`] slot, each created once and
//!   selected by `frame_index` (Decision 7).
//!
//! It is owned by [`RhiContext`] as a field, so its teardown rides
//! [`RhiContext::destroy_all`] and `RhiContext::Drop` — nothing leaks past the
//! device owner (the Decision-8 leak fix).
//!
//! All resources are created/destroyed through the real [`RhiDevice`] verbs on
//! `&VulkanContext` (reached via `RhiContext::split_mut().0`), so this module names
//! the device only by reference and never owns it (one device owner, one teardown
//! order).

use boyko_rhi::enums::{BlendState, DescriptorKind, Format, ShaderStage};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferUsage, GraphicsPipelineDesc, MemoryLocation, PrimitiveTopology, RhiDevice,
};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::rhi_impl::{
    VulkanBindGroup, VulkanBindGroupLayout, VulkanGraphicsPipeline, VulkanShaderModule,
};

use crate::error::GpuColumnError;
use crate::ui::instance::{UiOrtho, UI_INSTANCE_SIZE};
use crate::ui::FRAMES_IN_FLIGHT;
// `RhiContext` is referenced only in doc links (this sub-owner is a field on it);
// importing it lets the bare `[`RhiContext`]` intra-doc links resolve.
#[allow(unused_imports)]
use crate::gpu_column::RhiContext;

/// The VERTEX-stage push-constant range the UI pipeline declares (one [`UiOrtho`],
/// 16 B). The fragment shader reads only the SSBO, so the ortho is pushed VERTEX-only
/// (matching the backend's VERTEX-only graphics push range — `rhi_impl.rs`).
const UI_PUSH_CONSTANT_BYTES: u32 = size_of::<UiOrtho>() as u32;

/// One persistent-mapped, grow-only STORAGE ring slot + its bind-group (Decision 7).
///
/// Created once per frame-in-flight at [`UiRenderResources`] setup; the buffer is
/// host-visible + host-coherent and mapped once (never unmapped). On overflow the
/// whole slot (buffer + bind-group) is recreated at the pow2-rounded capacity (the
/// affected slot only) — `create_bind_group` writes the descriptor set ONCE at
/// create, so the grow MUST rebuild the bind-group (there is no update verb).
struct UiRingSlot {
    /// The host-mapped STORAGE buffer this slot's instances are memcpy'd into.
    buffer: BoundBuffer,
    /// The bind-group binding `buffer` at set0/binding0 (created once; rebuilt on grow).
    bind_group: VulkanBindGroup,
    /// The slot's current byte capacity (grows pow2 only on overflow).
    cap_bytes: u64,
}

/// The owned, `Drop`-wired UI render capability (Decision 8): the pipeline +
/// bind-group layout + the per-FIF host-mapped rings + bind-groups.
///
/// Owned as a field on [`RhiContext`] so its teardown is wired into
/// `RhiContext::destroy_all` + `RhiContext::Drop` — a first-class kernel capability
/// with explicit teardown (NOT a side store). `!Send + !Sync` by its owning
/// `RhiContext` (touched only on the dispatcher thread).
pub(crate) struct UiRenderResources {
    /// The UI graphics pipeline (built once for the setup `color_format`; blend =
    /// premultiplied). Re-resolved each frame by `frame_index` indirection through
    /// the owning [`RhiContext`] (MF-7) — the on-screen recorder never caches it.
    pipeline: VulkanGraphicsPipeline,
    /// The shared SSBO bind-group layout (one `StorageBuffer` @ set0/binding0,
    /// VERTEX|FRAGMENT). Every per-FIF bind-group is allocated against it.
    layout: VulkanBindGroupLayout,
    /// The two committed shader modules (vertex + fragment), retained for teardown
    /// ordering (the pipeline owns the compiled stages; the modules are destroyed
    /// after the pipeline at teardown).
    vertex_module: VulkanShaderModule,
    fragment_module: VulkanShaderModule,
    /// One persistent-mapped grow-only ring + bind-group per frame-in-flight.
    slots: [UiRingSlot; FRAMES_IN_FLIGHT],
}

impl UiRenderResources {
    /// Builds the UI pipeline + bind-group layout + per-FIF host-mapped rings +
    /// bind-groups, once (Rung 3 step 9). Every resource is created through the real
    /// [`RhiDevice`] verbs on `device`.
    ///
    /// `color_format` is the format of the image the UI pass renders into (the
    /// swapchain surface format for the on-screen path, `R8G8B8A8Unorm` for the
    /// offscreen golden — Decision 9's two-pipeline-from-one-shader contract).
    /// `initial_rows` is each ring's starting capacity in `UiInstance` records (the
    /// rings grow pow2 on overflow).
    ///
    /// On any partial failure every resource already created here is torn down
    /// before the error returns (no leak), since none is owned by the manager.
    ///
    /// # Errors
    /// [`GpuColumnError::Rhi`] on any shader-module / pipeline / layout / buffer /
    /// bind-group create failure.
    pub(crate) fn create(
        device: &VulkanContext,
        color_format: Format,
        spirv_vs: &[u32],
        spirv_fs: &[u32],
        initial_rows: u32,
    ) -> Result<Self, GpuColumnError> {
        debug_assert!(initial_rows > 0, "invariant: UI ring initial_rows is non-zero");

        let vertex_module = device.create_shader_module(spirv_vs)?;
        let fragment_module = match device.create_shader_module(spirv_fs) {
            Ok(m) => m,
            Err(e) => {
                // SAFETY: `vertex_module` was just created on `device`, is owned
                // exclusively here (never registered), and no pipeline references it
                // (the fragment module failed first), so destroying it once is sound.
                unsafe { device.destroy_shader_module(vertex_module) };
                return Err(GpuColumnError::Rhi(e));
            }
        };

        let layout = match device.create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::StorageBuffer,
                // The Rung-0.5-proven combination: a STORAGE buffer visible in BOTH
                // the vertex (transform) and fragment (SDF/clip) stages.
                stage: ShaderStage::VERTEX | ShaderStage::FRAGMENT,
            }],
        }) {
            Ok(l) => l,
            Err(e) => {
                // SAFETY: both modules were just created on `device`, owned
                // exclusively here, referenced by no live pipeline; destroy each once.
                unsafe {
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        let pipeline = match device.create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vertex_module,
            vertex_entry: c"main",
            fragment_module: &fragment_module,
            fragment_entry: c"main",
            color_formats: &[color_format],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            // Vertexless quad (`SV_VertexID`), the Rung-0.5 shape.
            vertex_layout: None,
            push_constant_bytes: UI_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&layout),
            blend: Some(BlendState::PREMULTIPLIED_ALPHA),
        }) {
            Ok(p) => p,
            Err(e) => {
                // SAFETY: the layout + both modules were just created on `device`,
                // owned exclusively here, referenced by no live pipeline (the create
                // failed); destroy each once in reverse creation order.
                unsafe {
                    device.destroy_bind_group_layout(layout);
                    device.destroy_shader_module(fragment_module);
                    device.destroy_shader_module(vertex_module);
                }
                return Err(GpuColumnError::Rhi(e));
            }
        };

        // Build the per-FIF rings. On a mid-array failure, every slot built so far
        // (plus the pipeline/layout/modules) is torn down before returning.
        let mut built: Vec<UiRingSlot> = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let init_bytes = initial_rows as u64 * UI_INSTANCE_SIZE as u64;
        for _ in 0..FRAMES_IN_FLIGHT {
            match Self::create_slot(device, &layout, init_bytes) {
                Ok(slot) => built.push(slot),
                Err(e) => {
                    for slot in built {
                        // SAFETY: each `slot`'s buffer + bind-group were created on
                        // `device`, are owned exclusively here, and were never
                        // submitted to (setup-time), so destroying each once is sound.
                        unsafe {
                            device.destroy_bind_group(slot.bind_group);
                            device.destroy_buffer(slot.buffer);
                        }
                    }
                    // SAFETY: the pipeline/layout/modules above were created on
                    // `device`, owned exclusively here, never submitted; destroy each
                    // once in reverse creation order.
                    unsafe {
                        device.destroy_graphics_pipeline(pipeline);
                        device.destroy_bind_group_layout(layout);
                        device.destroy_shader_module(fragment_module);
                        device.destroy_shader_module(vertex_module);
                    }
                    return Err(e);
                }
            }
        }

        let slots: [UiRingSlot; FRAMES_IN_FLIGHT] = built
            .try_into()
            .unwrap_or_else(|_| unreachable!("invariant: built exactly FRAMES_IN_FLIGHT slots"));

        Ok(Self {
            pipeline,
            layout,
            vertex_module,
            fragment_module,
            slots,
        })
    }

    /// Ensures slot `frame_index` can hold `instance_count` records, growing pow2 on
    /// overflow (fence-wait the device, recreate the buffer + rebuild this slot's
    /// bind-group), then memcpys `packed` into the mapped slot (Rung 3 step 10 / A1
    /// steps 4-5).
    ///
    /// `packed` is the contiguous byte image of the `instance_count` records (the
    /// no-bytemuck POD view from [`UiInstance::slice_as_bytes`](crate::ui::UiInstance::slice_as_bytes));
    /// `packed.len()` MUST equal `instance_count * UI_INSTANCE_SIZE`.
    ///
    /// # Caller contract (the write-after-read fence — GUI P5a)
    ///
    /// The caller MUST have waited slot `frame_index`'s present in-flight fence
    /// (`Renderer::wait_frame_in_flight` for the SAME `frame_index`) BEFORE this call,
    /// so the GPU's last read of this persistently-mapped, host-coherent ring slot (the
    /// submit two presents back, with `FRAMES_IN_FLIGHT == 2`) is complete. Without
    /// that wait the memcpy below is a write-after-read race on a buffer the GPU may
    /// still be reading. The grow path (`grow_slot`) `wait_idle`s the whole device, so
    /// a grow frame is covered regardless; the steady-state (no-grow) memcpy relies on
    /// the caller's per-slot fence wait.
    ///
    /// # Errors
    /// [`GpuColumnError`] on a grow buffer-create / bind-group-create failure or a
    /// missing ring mapping.
    ///
    /// [`UiInstance::slice_as_bytes`]: crate::ui::UiInstance::slice_as_bytes
    pub(crate) fn upload(
        &mut self,
        device: &VulkanContext,
        packed: &[u8],
        instance_count: u32,
        frame_index: usize,
    ) -> Result<(), GpuColumnError> {
        debug_assert!(frame_index < FRAMES_IN_FLIGHT, "invariant: frame_index in range");
        debug_assert_eq!(
            packed.len(),
            instance_count as usize * UI_INSTANCE_SIZE,
            "invariant: packed byte length matches instance_count * UI_INSTANCE_SIZE"
        );

        let need = packed.len() as u64;
        // Clamp the slot index defensively (a release-time out-of-range would index
        // out of bounds; the debug_assert above traps it in debug).
        let frame_index = frame_index.min(FRAMES_IN_FLIGHT - 1);

        if need > self.slots[frame_index].cap_bytes {
            self.grow_slot(device, frame_index, need)?;
        }

        // memcpy into the mapped slot. A zero-instance frame still resolves the
        // mapping (the ring stays valid) but copies nothing.
        if need > 0 {
            let slot = &self.slots[frame_index];
            let dst = device
                .buffer_mapped_ptr(&slot.buffer)
                .ok_or(GpuColumnError::StagingNotMapped)?;
            debug_assert!(
                need <= slot.cap_bytes,
                "invariant: instance bytes fit the (grown) ring capacity"
            );
            // SAFETY: `dst` is the persistently-mapped first byte of slot
            // `frame_index`'s host-visible + host-coherent ring, whose `cap_bytes`
            // is `>= need` (grown just above on overflow). `packed` is a distinct
            // allocation (the pack scratch), so the regions never overlap; `&mut
            // self` makes this the unique writer. The GPU's last read of this SAME
            // ring slot (the submit two presents back) is complete: the caller waited
            // slot `frame_index`'s present in-flight fence before this call (the
            // documented caller contract above) — and a grow frame additionally
            // `wait_idle`s the whole device in `grow_slot`. Host-coherent ⇒ no flush.
            unsafe {
                core::ptr::copy_nonoverlapping(packed.as_ptr(), dst.as_ptr(), packed.len());
            }
        }
        Ok(())
    }

    /// Re-resolves the current-frame UI pipeline + bind-group by `frame_index`
    /// (MF-7) — the on-screen recorder reads the handles indirectly each frame, never
    /// a cached raw handle, so a grow that rebuilt slot `frame_index`'s bind-group
    /// between upload and draw is transparent.
    #[inline]
    pub(crate) fn handles(
        &self,
        frame_index: usize,
    ) -> (&VulkanGraphicsPipeline, &VulkanBindGroup) {
        debug_assert!(frame_index < FRAMES_IN_FLIGHT, "invariant: frame_index in range");
        let frame_index = frame_index.min(FRAMES_IN_FLIGHT - 1);
        (&self.pipeline, &self.slots[frame_index].bind_group)
    }

    /// Tears down every owned resource (Rung 3 — wired into `RhiContext::destroy_all`
    /// and `RhiContext::Drop`). Consumes `self`, so it runs exactly once; the caller
    /// `take()`s the `Option<UiRenderResources>` so a second `destroy_all`/`Drop`
    /// finds `None` and is a no-op (idempotent like the manager).
    ///
    /// `device.wait_idle()` is called first so no in-flight present submission is
    /// still reading any ring; then each resource is destroyed in reverse creation
    /// order (slots → pipeline → layout → modules).
    pub(crate) fn destroy(self, device: &VulkanContext) {
        // Belt-and-braces: the caller's teardown contract already drains the device,
        // but a `wait_idle` here makes the destroy sound regardless of caller order.
        let _ = device.wait_idle();
        for slot in self.slots {
            // SAFETY: each slot's bind-group + buffer were created on `device`, owned
            // exclusively here, and the device is idle (waited above), so no GPU work
            // references them; each is moved by value ⇒ destroyed exactly once.
            unsafe {
                device.destroy_bind_group(slot.bind_group);
                device.destroy_buffer(slot.buffer);
            }
        }
        // SAFETY: the pipeline/layout/modules were created on `device`, owned
        // exclusively here, and the device is idle; each is moved by value ⇒
        // destroyed exactly once, in reverse creation order (the pipeline before its
        // layout + modules).
        unsafe {
            device.destroy_graphics_pipeline(self.pipeline);
            device.destroy_bind_group_layout(self.layout);
            device.destroy_shader_module(self.fragment_module);
            device.destroy_shader_module(self.vertex_module);
        }
    }

    // ===== internals =====

    /// Creates one host-mapped STORAGE ring of `cap_bytes` + its bind-group against
    /// `layout`. The buffer is host-visible + host-coherent (mapped once at create).
    fn create_slot(
        device: &VulkanContext,
        layout: &VulkanBindGroupLayout,
        cap_bytes: u64,
    ) -> Result<UiRingSlot, GpuColumnError> {
        let buffer = device.create_buffer(&BufferDesc {
            size: cap_bytes,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })?;
        let bind_group = match device.create_bind_group(&BindGroupDesc {
            layout,
            entries: &[BindGroupEntry::StorageBuffer { buffer: &buffer }],
        }) {
            Ok(bg) => bg,
            Err(e) => {
                // SAFETY: `buffer` was just created on `device`, owned exclusively
                // here, never submitted; destroy it once on this error edge.
                unsafe { device.destroy_buffer(buffer) };
                return Err(GpuColumnError::Rhi(e));
            }
        };
        Ok(UiRingSlot {
            buffer,
            bind_group,
            cap_bytes,
        })
    }

    /// Grows slot `frame_index` to `>= need` bytes (pow2-rounded): fence-wait the
    /// device, create the new buffer + bind-group, then destroy the old ones
    /// (Decision 7 grow path). `#[cold]` — a setup-class cost, off the steady-state
    /// I-cache.
    #[cold]
    fn grow_slot(
        &mut self,
        device: &VulkanContext,
        frame_index: usize,
        need: u64,
    ) -> Result<(), GpuColumnError> {
        // Drain the device so the slot being recreated is not read by any in-flight
        // present submission (the `RhiContext` does not own the per-FIF present fence;
        // `wait_idle` is the available device-level drain, and a grow is setup-class).
        let _ = device.wait_idle();

        let new_cap = need.next_power_of_two().max(self.slots[frame_index].cap_bytes);
        let new_slot = Self::create_slot(device, &self.layout, new_cap)?;

        // Swap in the new slot, then destroy the old buffer + bind-group.
        let old = core::mem::replace(&mut self.slots[frame_index], new_slot);
        // SAFETY: `old`'s bind-group + buffer were created on `device`, owned
        // exclusively here, and the device was drained (`wait_idle` above), so no GPU
        // work references them; each is moved by value ⇒ destroyed exactly once.
        unsafe {
            device.destroy_bind_group(old.bind_group);
            device.destroy_buffer(old.buffer);
        }
        Ok(())
    }
}

// NOTE (no `unsafe`): `UiRenderResources` holds backend handles + a `BoundBuffer`
// whose `mapped` is a raw `NonNull<u8>` (so it is `!Send + !Sync` automatically),
// exactly what its owning `!Send + !Sync` `RhiContext` requires — the UI
// rings/pipeline are touched only on the dispatcher thread. No `unsafe impl`.
