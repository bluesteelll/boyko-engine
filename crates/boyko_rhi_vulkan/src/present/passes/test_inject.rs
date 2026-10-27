//! `Renderer::record_test_nan_inject`: render wave PX1a's TEST-ONLY NaN injection (H4 Scope 3,
//! test 12; `px1a/cut.md` C11) — one `vkCmdCopyBufferToImage` of the 8-byte binary16 NaN texel
//! (`targets::NAN_INJECT_TEXEL_BITS`) into `lit[fi]` at `targets::NAN_INJECT_TEXEL`, recorded by
//! the three path recorders after the last `lit` producer and before `particle_draw`, on the one
//! engine frame `Renderer::arm_nan_inject` named. Never recorded on a shipped run: the
//! `test_nan_inject` graph pass is declared only under the `inject_nan_lit` force key. The caller
//! emits the pass's derived barriers (`lit` → `TRANSFER_DST_OPTIMAL`) through its own sink first.
//!
//! Also render rung GB's `Renderer::record_test_gpu_hang`, the forced-hang gate's (G3) body seam.

use crate::ffi::*;

use super::super::frame_driver::Renderer;
use super::super::targets::{GBufferTargets, NAN_INJECT_TEXEL};

impl Renderer<'_> {
    /// Render rung GB (G3): the TEST-ONLY forced-hang body, recorded inside the `test_gpu_hang`
    /// scope on the one armed frame.
    ///
    /// **It records nothing yet, by the cut's decision (GB cut D10 / Q2).** A shader-free long
    /// dispatch is not guaranteed to reach a device loss under WDDM hardware scheduling with
    /// compute preemption, and the standard primitive — a dedicated long-running compute shader —
    /// is a new `.spv` + dxc recipe + `*_spv_sync` gate, which belongs to the shader-SDK lane. The
    /// seam (force key, opt-in, declaration, scope, report, gate) lands now; the primitive fills
    /// this one function after that lane merges. Until then the forced-hang gate's driver reports
    /// "no device loss within its deadline" — its own distinct outcome, never a pass.
    #[cold]
    #[inline(never)]
    pub(crate) fn record_test_gpu_hang(&self, cmd: VkCommandBuffer) {
        let _ = cmd;
    }

    /// Records the one-texel copy of `targets.nan_inject_src` into `lit[fi]` at
    /// [`NAN_INJECT_TEXEL`].
    ///
    /// # Safety
    ///
    /// `cmd` is recording, outside any rendering scope; the caller has ALREADY emitted the frame's
    /// `test_nan_inject` graph-pass barriers (`lit[fi]` at `TRANSFER_DST_OPTIMAL`, its producers'
    /// writes made available to the transfer); `targets` were built by `GBufferTargets::create`
    /// with the `inject_nan_lit` arming (so `lit` carries `TRANSFER_DST` and `nan_inject_src` is
    /// `Some`); `fi` is this frame's in-flight slot.
    pub(crate) unsafe fn record_test_nan_inject(&self, cmd: VkCommandBuffer, targets: &GBufferTargets, fi: usize) {
        let src = targets
            .nan_inject_src
            .as_ref()
            .expect("invariant: the test_nan_inject pass is declared only on a boot that built its source");
        let lit = &targets.lit[fi];
        debug_assert!(
            NAN_INJECT_TEXEL[0] < targets.extent.width && NAN_INJECT_TEXEL[1] < targets.extent.height,
            "invariant: the injected texel lies inside lit"
        );
        let region = VkBufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: VkImageSubresourceLayers {
                aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: VkOffset3D { x: NAN_INJECT_TEXEL[0] as i32, y: NAN_INJECT_TEXEL[1] as i32, z: 0 },
            image_extent: VkExtent3D { width: 1, height: 1, depth: 1 },
        };
        // SAFETY: per this fn's contract `cmd` is recording outside a rendering scope and `lit[fi]`
        // is at `TRANSFER_DST_OPTIMAL` with `TRANSFER_DST` usage; `src.buffer` is the live 8-byte
        // `TRANSFER_SRC` buffer `create` built and wrote (one `LIT_FORMAT` texel — exactly the
        // region's 1×1×1 footprint at offset 0); `NAN_INJECT_TEXEL` lies inside `lit` (asserted
        // above in a dev build; `lit` is at least the window's extent); `&region` outlives the call.
        unsafe {
            (self.fns.cmd_copy_buffer_to_image)(
                cmd,
                src.buffer,
                lit.image,
                VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                1,
                &region,
            );
        }
    }
}
