//! Render wave RD1 (defect R0b): [`SdfDepthMergeGpu`] — the SDF depth merge's boot-built GPU
//! objects (one set-0 layout, one depth-only `SV_Depth` pipeline). Owned by
//! [`super::GpuSceneBundles`]; built through [`super::GpuSceneBundles::ensure_sdf_depth_merge`] by
//! every transparent consumer class's bundle builder (the particle one at RD1), so a boot that arms
//! no consumer class builds NOTHING here — the 0%-gate.
//!
//! The per-FIF descriptor sets are NOT here: they bind `gViewT[fi]` and the camera ring slot, a
//! target-view set, so `GBufferTargets` builds and rebuilds them against [`SdfDepthMergeGpu`]'s
//! layout through the existing per-extent drain discipline.

use boyko_rhi_vulkan::compute::{SDF_DEPTH_MERGE_PUSH_BYTES, sdf_depth_merge_fs_spirv};

use super::particle::particle_depth_compare_for;
use super::*;

/// The SDF depth merge's boot-built GPU objects. See the module doc.
pub(super) struct SdfDepthMergeGpu {
    /// `{ Texture2D gViewT @0 (SAMPLED, bound as SampledImageAtGeneral), Camera UBO @1 }`, both
    /// FRAGMENT — `sdf_depth_merge.fs.hlsl`'s binding table.
    pub(super) layout: VulkanBindGroupLayout,
    /// The depth-only pipeline: `fullscreen_sample.vs` + `sdf_depth_merge.fs`, `D32_SFLOAT`, depth
    /// test + write ON with the path's compare op, a 20-B FRAGMENT push range.
    pub(super) pipeline: VulkanGraphicsPipeline,
    /// The path predicate the compare op was frozen from — a second `ensure` call with the other
    /// value is a bug (the path is boot-frozen), asserted there.
    deferred_path: bool,
}

impl SdfDepthMergeGpu {
    /// Builds the layout and the one pipeline for a boot whose resolved path is (`deferred_path`)
    /// or is not Deferred: `VK_COMPARE_OP_LESS` against Deferred's custom-linear depth,
    /// `VK_COMPARE_OP_GREATER` against the reverse-Z paths' — the particle draw's own rule
    /// ([`particle_depth_compare_for`]), because the merge and the consumers it serves must test
    /// one depth buffer the same way.
    ///
    /// # Panics
    /// Panics (`expect("invariant: ...")`) on any RHI create failure — the boot contract every
    /// bundle in this module follows.
    pub(super) fn create(ctx: &VulkanContext, deferred_path: bool) -> Self {
        let layout = RhiDevice::create_bind_group_layout(
            ctx,
            &BindGroupLayoutDesc {
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::SampledImage,
                        stage: ShaderStage::FRAGMENT,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        count: 1,
                        kind: DescriptorKind::UniformBuffer,
                        stage: ShaderStage::FRAGMENT,
                    },
                ],
            },
        )
        .expect("invariant: SDF depth-merge bind-group layout create");
        let vs = RhiDevice::create_shader_module(ctx, fullscreen_sample_vs_spirv())
            .expect("invariant: SDF depth-merge fullscreen vertex shader module create");
        let fs = RhiDevice::create_shader_module(ctx, sdf_depth_merge_fs_spirv())
            .expect("invariant: SDF depth-merge fragment shader module create");
        let pipeline = ctx
            .create_graphics_pipeline_depth_merge(
                &GraphicsPipelineDesc {
                    vertex_module: &vs,
                    vertex_entry: c"main",
                    fragment_module: &fs,
                    fragment_entry: c"main",
                    // Depth-only: no colour attachment, the path's `D32_SFLOAT` depth (Deferred's
                    // `depth` and the Forward-family / VB `forward.depth` are both D32).
                    color_formats: &[],
                    depth_format: Some(Format::D32Sfloat),
                    topology: PrimitiveTopology::TriangleList,
                    vertex_layout: None,
                    push_constant_bytes: SDF_DEPTH_MERGE_PUSH_BYTES,
                    bind_group_layout: Some(&layout),
                    blend: None,
                    cull_mode: CullMode::None,
                    depth_bias: None,
                },
                particle_depth_compare_for(deferred_path),
            )
            .expect("invariant: SDF depth-merge graphics pipeline create");
        // SAFETY: both modules were created on `ctx` above and consumed by the pipeline create;
        // each is destroyed exactly once; no GPU work referencing them is in flight (boot).
        unsafe {
            RhiDevice::destroy_shader_module(ctx, fs);
            RhiDevice::destroy_shader_module(ctx, vs);
        }
        Self { layout, pipeline, deferred_path }
    }

    /// The path predicate this bundle was built for.
    #[inline]
    pub(super) const fn deferred_path(&self) -> bool {
        self.deferred_path
    }

    /// Tears the bundle down.
    ///
    /// # Safety
    /// `ctx` is the live context the bundle was created on, the device is idle, and every
    /// descriptor set built against [`Self::layout`] is already destroyed.
    pub(super) unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: per the contract the device is idle and `ctx` is live; the pipeline goes before
        // the layout its own pipeline layout embeds; each is destroyed exactly once (by-value).
        unsafe {
            RhiDevice::destroy_graphics_pipeline(ctx, self.pipeline);
            RhiDevice::destroy_bind_group_layout(ctx, self.layout);
        }
    }
}
