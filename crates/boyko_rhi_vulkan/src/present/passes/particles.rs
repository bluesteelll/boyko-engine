//! Particles P0 — the recorder (`docs/PARTICLES-PLAN.md` Rev 4).
//!
//! Two entry points, matching the two halves of the plan's four-boundary skeleton:
//! [`Renderer::record_particle_compute`] emits `upload → kickoff → emit → sim` EARLY in the frame
//! (so ~80–700 µs of particle compute overlaps the opaque work that follows, at zero cost —
//! declaration order is execution order and no barrier separates them), and
//! [`Renderer::record_particle_draw`] emits the single indirect billboard draw LATE, after the
//! path's last `lit` producer.
//!
//! # Every barrier here is DERIVED, none is hand-written
//!
//! Both fns take a `barriers` callback and invoke it once per pass, exactly at the pass's
//! position. The callback is the caller's own `record_{graph,forward,vb}_pass` — one per render
//! path, since the three paths resolve their `ResId`s through three different sinks — so this
//! module never touches `vkCmdPipelineBarrier` and never learns which path it is on. A generic
//! `F: FnMut(PassId)` rather than a `&mut dyn FnMut`: five calls a frame, monomorphized, no
//! indirect call in a recording path.
//!
//! # The one exception, and why it is not in this module
//!
//! The billboard quad's index buffer is boot-uploaded under a hand-written
//! `TRANSFER_WRITE → INDEX_READ` barrier, because it is not a framegraph resource at all (the
//! plan's seed table lists it as the single exception: written once, read-only forever). That
//! barrier lives at the boot site in `boyko_app::gpu_scene::particle`, beside the upload it
//! orders — not here, where nothing would ever re-emit it.
//!
//! # `firstInstance` is 0, and that is load-bearing
//!
//! `drawIndirectFirstInstance` is not enabled on this device, so a nonzero `firstInstance` in the
//! fetched command is a silent corruption class. The two blend classes rung P2 adds are therefore
//! distinguished by the VS's push-constant affine (`index_base + index_step * SV_InstanceID`),
//! never by that field — which is why the draw below pushes 72 bytes and reads one command.

use core::ptr;

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::graph_bridge::ParticlePassPlan;
use super::super::scene_types::ParticleActivation;

/// The dynamic-rendering attachments the particle draw composites into, resolved by the caller
/// from ITS OWN per-path targets.
///
/// The recorder is path-agnostic by construction: `color_view` is always the frame's `lit` slot
/// (the one image every path shares, C5) and `depth_view` is whichever depth image that path
/// owns. Passing them in rather than reaching for `GBufferTargets` keeps this module from having
/// to know that Forward and VisibilityBuffer share `ForwardTargets::depth` while Deferred has its
/// own.
#[derive(Clone, Copy)]
pub(crate) struct ParticleDrawTargets {
    /// The frame slot's `lit` image view — the additive blend destination, entered at
    /// `COLOR_ATTACHMENT_OPTIMAL` (the graph derived the transition; `lit` already carries
    /// `COLOR_ATTACHMENT` usage on every path, so no image-create change was needed).
    pub(crate) color_view: VkImageView,
    /// The path's depth image view, entered at `DEPTH_ATTACHMENT_OPTIMAL` and used READ-ONLY:
    /// the pipeline was built with `depth_write = false`, so opaque geometry occludes the
    /// billboards while they never occlude each other.
    pub(crate) depth_view: VkImageView,
    /// The full-target scissor/render area.
    pub(crate) area: VkRect2D,
    /// The full-target viewport.
    pub(crate) viewport: VkViewport,
}

impl Renderer<'_> {
    /// Particles P0: records `upload → kickoff → emit → sim`.
    ///
    /// `barriers` is invoked once per declared pass, immediately before that pass's commands, so
    /// the derived barriers land where the declarator put them. Passes the frame did not declare
    /// (`upload` with nothing to upload, `emit` with no spawns) emit neither a barrier call nor a
    /// command — the declare/record parity the plan's gate #6 pins, expressed as "read the same
    /// `Option`".
    ///
    /// # Safety
    ///
    /// * `cmd` is an OPEN command buffer, recording outside any dynamic-rendering scope (all four
    ///   commands here are illegal inside one).
    /// * Every handle in `act` is live: the pipelines, the descriptor set and the buffers were
    ///   created on this device by `build_particle_bundle` and are not destroyed until teardown.
    /// * `act.sets` is the set built for THIS frame's parity, and its bindings 4/5 name the two
    ///   physical alive buffers in the roles the declarator seeded them as. Binding the sibling
    ///   parity's set here would leave both alive-list hazards unordered while every barrier
    ///   count stayed identical.
    /// * The copy sizes were bounded at the call site against both the staging and the device
    ///   allocation (`emit_upload_bytes`/`effects_upload_bytes` are `min`-clamped by the host
    ///   before they reach the activation), so each `vkCmdCopyBuffer` region is in bounds at both
    ///   ends.
    pub(crate) unsafe fn record_particle_compute<F: FnMut(crate::framegraph::PassId)>(
        &self,
        cmd: VkCommandBuffer,
        act: &ParticleActivation<'_>,
        plan: &ParticlePassPlan,
        mut barriers: F,
    ) {
        debug_assert_eq!(
            act.parity,
            act.frame_index & 1,
            "invariant: the activation's parity is the host frame counter's low bit — the same \
             number that chose the bound descriptor set"
        );
        // Mirrors `boyko_render::PARTICLE_SUBSTEP_CEILING`, which this crate cannot NAME —
        // `boyko_render` sits above it in the dependency graph. Spelled here so the assert reads
        // as a claim about a known bound rather than as a bare literal; the value's ONE home is
        // still the const in that crate, and this file is the mirror that would red if it moved.
        const SUBSTEP_CEILING_MIRROR: u32 = 64;
        debug_assert!(
            act.steps <= SUBSTEP_CEILING_MIRROR,
            "invariant: ParticleClock clamps steps to PARTICLE_SUBSTEP_CEILING on the host, so \
             the shader's own min() can never bind"
        );

        // --- `particle_upload`: the staging→device copies, each half gated exactly as the
        //     declarator gated its `buffer_access`. A frame with neither armed declared no pass
        //     and records no copy: 0 bytes cross PCIe, which is one of the plan's metric rows.
        if let Some(p) = plan.upload {
            barriers(p);
            if act.emit_upload_bytes > 0 {
                let region = VkBufferCopy {
                    src_offset: 0,
                    dst_offset: 0,
                    size: act.emit_upload_bytes,
                };
                // SAFETY: recording is open and outside a render scope (this fn's contract). The
                // source is this frame slot's host-visible emit-request staging and the
                // destination the device-local table, both live for the frame; `size` was
                // clamped by the host against both allocations before it reached the activation,
                // so the region is in bounds at both ends. `region` is a local that outlives the
                // call.
                unsafe {
                    (self.fns.cmd_copy_buffer)(
                        cmd,
                        act.emit_req_staging.buffer,
                        act.emit_req_device.buffer,
                        1,
                        &region,
                    );
                }
            }
            if act.effects_upload_bytes > 0 {
                let region = VkBufferCopy {
                    src_offset: 0,
                    dst_offset: 0,
                    size: act.effects_upload_bytes,
                };
                // SAFETY: as the emit-request copy above, on this frame slot's effect-table
                // staging and the device-local effect table.
                unsafe {
                    (self.fns.cmd_copy_buffer)(
                        cmd,
                        act.effects_staging.buffer,
                        act.effects_device.buffer,
                        1,
                        &region,
                    );
                }
            }
        }

        // --- `particle_kickoff`: the ONE-THREAD pass, dispatched DIRECTLY. It cannot be indirect
        //     — it is the pass that writes the indirect argument blocks the other two are
        //     dispatched from.
        if let Some(p) = plan.kickoff {
            barriers(p);
            let push: [u32; 2] = [act.requested_spawn, act.capacity];
            // SAFETY: recording is open and outside a render scope. `kickoff_pipeline` is a live
            // COMPUTE pipeline whose layout declares `act.sets`' layout at set 0 and an 8-byte
            // COMPUTE push range at offset 0 — exactly the 8 bytes written here (a multiple of 4,
            // as `vkCmdPushConstants` requires). `push` is a local that outlives the call.
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    act.kickoff_pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    act.kickoff_pipeline.layout,
                    0,
                    1,
                    &act.sets.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    act.kickoff_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    core::mem::size_of_val(&push) as u32,
                    push.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, 1, 1, 1);
            }
        }

        // --- `particle_emit`: the FIRST production consumer of `vkCmdDispatchIndirect` in this
        //     engine (the fn pointer has been loaded since `device.rs`'s table was written and
        //     had zero call sites until now). The group count is `ceil(real_emit_count / 256)`,
        //     computed on the DEVICE by kickoff — the host never learns it.
        if let Some(p) = plan.emit {
            barriers(p);
            let push: [u32; 2] = [act.emitter_count, act.frame_index];
            // SAFETY: recording is open and outside a render scope. `emit_pipeline`'s layout
            // declares `act.sets`' layout at set 0 and an 8-byte COMPUTE push range. The indirect
            // argument buffer is the live `p_dispatch_args` allocation, created with
            // `INDIRECT_BUFFER` usage; offset 0 is the emit `VkDispatchIndirectCommand`, 16 B
            // inside a 32 B allocation, and 4-aligned as the spec requires. The command's group
            // counts were written by kickoff this frame and ordered against this fetch by the
            // barrier the callback above just emitted.
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    act.emit_pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    act.emit_pipeline.layout,
                    0,
                    1,
                    &act.sets.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    act.emit_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    core::mem::size_of_val(&push) as u32,
                    push.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch_indirect)(
                    cmd,
                    act.dispatch_args.buffer,
                    crate::compute::PARTICLE_DISPATCH_EMIT_OFFSET,
                );
            }
        }

        // --- `particle_sim`: the hot loop. `steps` is the host-clamped substep count and
        //     `timestep` the clock's constant `dt`; both travel as raw bytes so the `f32` reaches
        //     the shader bit-identically rather than through a conversion neither side agreed on.
        if let Some(p) = plan.sim {
            barriers(p);
            let mut push = [0u8; 8];
            push[..4].copy_from_slice(&act.steps.to_ne_bytes());
            push[4..].copy_from_slice(&act.timestep.to_ne_bytes());
            // SAFETY: recording is open and outside a render scope. `sim_pipeline`'s layout
            // declares `act.sets`' layout at set 0 and an 8-byte COMPUTE push range. The indirect
            // fetch reads the sim `VkDispatchIndirectCommand` at offset 16 of the live 32-byte
            // `p_dispatch_args` (4-aligned, in bounds), whose group count kickoff wrote this
            // frame behind the derived barrier.
            unsafe {
                (self.fns.cmd_bind_pipeline)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    act.sim_pipeline.pipeline,
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    act.sim_pipeline.layout,
                    0,
                    1,
                    &act.sets.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    act.sim_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    push.len() as u32,
                    push.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch_indirect)(
                    cmd,
                    act.dispatch_args.buffer,
                    crate::compute::PARTICLE_DISPATCH_SIM_OFFSET,
                );
            }
        }
    }

    /// Particles P0: records the single indirect billboard draw into `lit`.
    ///
    /// ONE `vkCmdDrawIndexedIndirect` covers every effect: the texture is a bindless index in the
    /// render record, so there is no per-effect batch key and no draw split. `instanceCount` is
    /// the sim's live survivor count, never the pool capacity.
    ///
    /// # Safety
    ///
    /// * `cmd` is an OPEN command buffer, recording OUTSIDE any dynamic-rendering scope — this fn
    ///   opens and closes its own.
    /// * `targets`' two views are live views of the images the graph just transitioned for this
    ///   pass: `color_view` at `COLOR_ATTACHMENT_OPTIMAL` and `depth_view` at
    ///   `DEPTH_ATTACHMENT_OPTIMAL`. A view of a different image, or one the graph did not name,
    ///   would be a recorded-vs-actual layout divergence (spec UB).
    /// * `act.draw_pipeline`'s layout declares `draw_set0`'s layout at set 0, `draw_set1`'s at
    ///   set 1, and a `VERTEX` push range of `PARTICLE_DRAW_PUSH_BYTES` at offset 0 — exactly the
    ///   range written here.
    /// * `act.quad_ib` holds six `u16` indices, boot-uploaded and already made visible to
    ///   `INDEX_READ` by the boot barrier; `act.draw_args` is the live 64-byte argument block
    ///   created with `INDIRECT_BUFFER` usage, whose additive command at offset 0 the sim's
    ///   `instanceCount` accumulation just filled behind the derived barrier.
    /// * `draw_count == 1` is not a choice: `multiDrawIndirect` is not enabled on this device, so
    ///   1 is the only useful legal value, and the stride argument is then unread.
    pub(crate) unsafe fn record_particle_draw<F: FnMut(crate::framegraph::PassId)>(
        &self,
        cmd: VkCommandBuffer,
        act: &ParticleActivation<'_>,
        plan: &ParticlePassPlan,
        targets: ParticleDrawTargets,
        mut barriers: F,
    ) {
        // Gate #6's other half: the caller reached here because `scene.path_has_particles()` held,
        // and that is EXACTLY the predicate the declarator armed the draw pass under. A `None`
        // here would mean the two sites disagree — the recorder about to emit a blend the graph
        // never ordered against the `lit` producer.
        debug_assert!(
            plan.draw.is_some(),
            "invariant: declare/record parity — an armed frame declares the particle_draw pass"
        );
        let Some(p) = plan.draw else { return };
        barriers(p);

        // `LOAD` on both attachments: the draw COMPOSITES into the frame's shaded colour and
        // TESTS against the opaque depth — a `CLEAR` on either would erase the frame. Depth's
        // `STORE` is honest rather than optimal: the pipeline writes no depth, so the store is a
        // no-op the driver may elide, and `STORE_OP_NONE` would need a capability check this rung
        // does not otherwise need.
        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.color_view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue { color: VkClearColorValue { float32: [0.0; 4] } },
        };
        let depth_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.depth_view,
            image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue {
                depth_stencil: VkClearDepthStencilValue { depth: 0.0, stencil: 0 },
            },
        };
        let rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: targets.area,
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: (&depth_attachment as *const VkRenderingAttachmentInfo).cast(),
            p_stencil_attachment: ptr::null(),
        };

        // SAFETY: every precondition is this fn's own contract, discharged above — recording is
        // open and outside a render scope, both views are live and in the layouts the graph left
        // them in, the pipeline's two sets and 72-byte VERTEX push range match what is bound and
        // written, and the index/argument buffers are live with the right usage flags. The three
        // `&` temporaries (`color_attachment`, `depth_attachment`, `rendering`) outlive the
        // bracketed calls. Begin/End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &rendering);
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                act.draw_pipeline.pipeline,
            );
            let sets = [act.draw_set0.descriptor_set, act.draw_set1];
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                act.draw_pipeline.layout,
                0,
                sets.len() as u32,
                sets.as_ptr(),
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                act.draw_pipeline.layout,
                VK_SHADER_STAGE_VERTEX_BIT,
                0,
                act.draw_push.len() as u32,
                act.draw_push.as_ptr().cast(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &targets.viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &targets.area);
            (self.fns.cmd_bind_index_buffer)(cmd, act.quad_ib.buffer, 0, VK_INDEX_TYPE_UINT16);
            (self.fns.cmd_draw_indexed_indirect)(
                cmd,
                act.draw_args.buffer,
                crate::compute::PARTICLE_DRAW_ADDITIVE_OFFSET,
                1,
                DRAW_INDEXED_INDIRECT_STRIDE,
            );
            (self.fns.cmd_end_rendering)(cmd);
        }
    }
}
