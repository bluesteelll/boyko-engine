//! The env-gated host frame dump (`BOYKO_HOST_DUMP=<path.bmp>`) — the windowed
//! runner's diagnostic / owner-eval channel (the `window_present_gbuffer`
//! screenshot-dump pattern, lifted into the production host so the ECS-driven
//! frame stream itself can be captured; the R6 viewer parity gate reuses it).
//!
//! When the variable is set the frame loop renders normally for a settle
//! window ([`SETTLE_FRAMES`] presented frames), requests the renderer's
//! swapchain→staging readback on one frame, drains the in-flight ring so the
//! readback frame's fence is proven re-waited ([`DRAIN_FRAMES`], the golden
//! readback discipline), writes the image as a 32-bpp BMP at the given path,
//! and exits the loop. Entirely COLD: the steady loop pays one `Option` check
//! per frame; without the variable nothing is created.

use core::num::NonZeroU32;
use std::io::Write as _;

use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::{VK_FORMAT_R8G8B8A8_UNORM, VkExtent2D};
use boyko_rhi_vulkan::memory::BoundBuffer;

/// Presented frames rendered before the readback is requested — propagation,
/// light reconcile, the light-table upload, and the CSM fit are all settled
/// within the first few frames; 30 (~0.5 s under FIFO) adds slack for the
/// window manager's first-present hitches.
const SETTLE_FRAMES: u32 = 30;

/// Presented frames rendered AFTER the readback frame before the staging is
/// read: `FRAMES_IN_FLIGHT (2) + 1` — the readback frame's slot fence has been
/// re-waited by then (the same drain rationale the golden readback tests pin).
const DRAIN_FRAMES: u32 = 3;

/// The settle → request → drain progression. `Settle`/`Drain` count REMAINING
/// presented frames; `Request` keeps re-requesting across `Ok(false)`
/// recreate-skips until a readback frame actually presents.
enum DumpState {
    Settle(u32),
    Request,
    Drain(NonZeroU32),
}

/// The dump driver the frame loop threads through its steady path — see the
/// module docs for the protocol.
pub(crate) struct HostDump {
    /// Destination path (the `BOYKO_HOST_DUMP` value), written once.
    path: String,
    state: DumpState,
    /// The host-visible readback staging, created lazily on the request frame
    /// (sized to the swapchain extent at that moment) and destroyed in
    /// [`finish`](Self::finish).
    staging: Option<BoundBuffer>,
    /// The swapchain extent the staging was sized to — the BMP dimensions.
    extent: VkExtent2D,
    /// `true` when the swapchain readback bytes are RGBA (the BMP write then
    /// swaps R/B); `false` for the BGRA swapchain (bytes pass through). Set
    /// once at arm time from the swapchain format.
    rgba_source: bool,
}

impl HostDump {
    /// Arms the dump iff `BOYKO_HOST_DUMP` is set (the value is the output
    /// path). `vk_format` is the swapchain's raw `VkFormat` — the host boot
    /// only admits the two UNORM formats, so "not R8G8B8A8" means the readback
    /// bytes are already BGRA (the BMP's native pixel order). Cold: called
    /// once before the frame loop.
    pub(crate) fn from_env(vk_format: i32) -> Option<Self> {
        let path = std::env::var("BOYKO_HOST_DUMP").ok()?;
        boyko_log::info!(boyko_log::Host, "BOYKO_HOST_DUMP armed -> {}", boyko_log::dsp!(path, 192));
        Some(Self {
            path,
            state: DumpState::Settle(SETTLE_FRAMES),
            staging: None,
            extent: VkExtent2D { width: 0, height: 0 },
            rgba_source: vk_format == VK_FORMAT_R8G8B8A8_UNORM,
        })
    }

    /// The per-frame readback request: `Some(&staging)` on the request frame(s)
    /// (creating the staging sized to the CURRENT swapchain `extent`), `None`
    /// otherwise. A recreate between two request attempts resizes the staging
    /// (the previous attempt's frame was skipped, so nothing referenced it).
    pub(crate) fn request(
        &mut self,
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Option<&BoundBuffer> {
        if !matches!(self.state, DumpState::Request) {
            return None;
        }
        let stale = self.staging.is_some()
            && (self.extent.width != extent.width || self.extent.height != extent.height);
        if stale {
            // SAFETY: the staging has never been referenced by a COMPLETED
            // submission — it is only handed to `render_gbuffer_frame` on
            // request frames, and a request frame that presented moves the
            // state to `Drain` (this branch is unreachable after it); an
            // `Ok(false)` recreate-skip records no work. Created on `ctx`;
            // destroyed exactly once (the `take`).
            unsafe {
                RhiDevice::destroy_buffer(
                    ctx,
                    self.staging.take().expect("invariant: stale staging exists"),
                );
            }
        }
        if self.staging.is_none() {
            let size = u64::from(extent.width) * u64::from(extent.height) * 4;
            self.staging = Some(
                RhiDevice::create_buffer(
                    ctx,
                    &BufferDesc {
                        size,
                        usage: BufferUsage::TRANSFER_DST,
                        location: MemoryLocation::HostVisibleCoherent,
                    },
                )
                .expect("invariant: host-visible dump readback staging create"),
            );
            self.extent = extent;
        }
        self.staging.as_ref()
    }

    /// Advances the settle → request → drain machine after a frame attempt
    /// (`presented == true` iff `render_gbuffer_frame` returned `Ok(true)`).
    /// Returns `true` when the drained readback is host-readable — the caller
    /// then runs [`finish`](Self::finish) and exits its loop.
    pub(crate) fn after_present(&mut self, presented: bool) -> bool {
        if !presented {
            return false;
        }
        match self.state {
            DumpState::Settle(n) => {
                if n > 1 {
                    self.state = DumpState::Settle(n - 1);
                } else {
                    self.state = DumpState::Request;
                }
                false
            }
            // A presented request frame recorded the swapchain→staging copy.
            DumpState::Request => {
                self.state = DumpState::Drain(
                    NonZeroU32::new(DRAIN_FRAMES).expect("invariant: DRAIN_FRAMES > 0"),
                );
                false
            }
            DumpState::Drain(n) => match NonZeroU32::new(n.get() - 1) {
                Some(left) => {
                    self.state = DumpState::Drain(left);
                    false
                }
                None => true,
            },
        }
    }

    /// Reads the drained staging, writes the 32-bpp BMP, and destroys the
    /// staging (consuming the driver — the dump is one-shot).
    pub(crate) fn finish(mut self, ctx: &VulkanContext) {
        let staging = self
            .staging
            .take()
            .expect("invariant: finish follows a drained request");
        let (w, h) = (self.extent.width, self.extent.height);
        let byte_len = (w as usize) * (h as usize) * 4;

        let mapped = RhiDevice::buffer_mapped_ptr(ctx, &staging)
            .expect("invariant: host-visible dump staging is mapped");
        let mut pixels = vec![0u8; byte_len];
        // SAFETY: `mapped` points to >= `byte_len` mapped host-coherent bytes
        // (the staging was created at exactly `w * h * 4`); the readback
        // frame's fence was re-waited (DRAIN_FRAMES > FRAMES_IN_FLIGHT), so
        // the GPU's transfer write is complete and no submission still writes
        // the buffer; `pixels` is a distinct fresh allocation.
        unsafe {
            core::ptr::copy_nonoverlapping(mapped.as_ptr(), pixels.as_mut_ptr(), byte_len);
        }
        // SAFETY: created on `ctx` above; the only submissions referencing it
        // completed (fence re-waited per the drain); destroyed exactly once
        // (taken out of the Option).
        unsafe {
            RhiDevice::destroy_buffer(ctx, staging);
        }

        match write_bmp(&self.path, &pixels, w, h, self.rgba_source) {
            Ok(()) => boyko_log::info!(
                boyko_log::Host,
                "frame dump written -> {} ({}x{})",
                boyko_log::dsp!(self.path, 192),
                w,
                h
            ),
            Err(e) => {
                let err = crate::diag::debug_into(&e);
                crate::diag::report_dump_write_failed("frame dump", &self.path, err.as_str());
            }
        }
    }
}

/// Writes `pixels` (tightly packed 4 B/texel, row-major top-down) as a 32-bpp
/// uncompressed BMP. BMP rows are bottom-up and its pixel order is BGRA:
/// `rgba_source` selects the R/B swap.
fn write_bmp(path: &str, pixels: &[u8], w: u32, h: u32, rgba_source: bool) -> std::io::Result<()> {
    let row_bytes = (w as usize) * 4;
    let data_len = row_bytes * (h as usize);
    let file_len = 54 + data_len;

    let mut out = Vec::with_capacity(file_len);
    // BITMAPFILEHEADER (14 bytes).
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_len as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    // BITMAPINFOHEADER (40 bytes): positive height = bottom-up rows.
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // ppm + palette fields, unused

    for row in (0..h as usize).rev() {
        let src = &pixels[row * row_bytes..(row + 1) * row_bytes];
        if rgba_source {
            for px in src.chunks_exact(4) {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        } else {
            out.extend_from_slice(src);
        }
    }

    let mut f = std::fs::File::create(path)?;
    f.write_all(&out)
}
