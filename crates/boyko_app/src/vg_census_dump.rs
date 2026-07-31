//! **VG-R0 rung R0c — the armed density-census readback** (`BOYKO_VG_CENSUS=<path.toml>`).
//!
//! The GPU half of R0c: the host driver that arms one `vb_id` → staging copy, waits for the
//! readback frame's own fence, reduces the result through [`boyko_render::vg_census::reduce`] and
//! writes the census row as TOML. Entirely COLD — the steady loop pays one `Option` check per
//! frame, and without the variable nothing is created and **no extra command is recorded**, which
//! is the byte-neutrality R0c gate (a) rests on.
//!
//! # Why this is a sibling of `host_dump` and not a mode of it
//!
//! The two readbacks hash different images, at different extents (the swapchain's `extent` vs the
//! composite `present_extent`), with different texel widths (4 B BGRA vs 8 B `R32G32_UINT`), and
//! they are armed on different runs. Sharing a driver would mean one state machine serving two
//! resolutions — the shape that fabricates a curve. What IS shared is the fence discipline, and it
//! is shared by *derivation*, not by copy: see [`DRAIN_FRAMES`].
//!
//! # The hazard this module is built around
//!
//! `vb_id` is a per-FIF **ring**, and a host read of a per-FIF resource is the exact shape of this
//! project's recorded cross-frame bug class. The readback frame's slot fence must be re-waited
//! before the staging is mapped. That is not asserted by any R0c gate part (the plan's §9.1
//! enumerates all five and none reads the armed frame's ordering), so it is discharged
//! structurally: the drain below outlives the in-flight ring by one frame, exactly as the blessed
//! golden readback path does.

use core::num::NonZeroU32;
use std::io::Write as _;

use boyko_render::vg_census::{self, CensusRow, Sha256, VB_ID_SENTINEL};
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::VkExtent2D;
use boyko_rhi_vulkan::memory::BoundBuffer;

/// Presented frames rendered before the census is requested — the same settle window the frame
/// dump uses, and for the same reasons (propagation, light reconcile, the light-table upload and
/// the CSM fit all settle within the first few frames; the slack covers the window manager's first
/// present hitches). Shared value, independently justified: a census taken mid-settle would read a
/// partially-populated draw list and understate `visible_tris`.
const SETTLE_FRAMES: u32 = 30;

/// Presented frames rendered AFTER the readback frame before the staging is mapped:
/// `FRAMES_IN_FLIGHT (2) + 1`. The readback frame's slot fence has necessarily been re-waited by
/// then — the frame loop waits a slot's fence before reusing it, and after `FRAMES_IN_FLIGHT`
/// further presents every slot has been reused at least once.
const DRAIN_FRAMES: u32 = 3;

/// Bytes per `vb_id` texel (`VK_FORMAT_R32G32_UINT`).
const VB_ID_TEXEL_BYTES: u64 = 8;

/// The settle → request → drain progression. `Settle`/`Drain` count REMAINING presented frames;
/// `Request` keeps re-requesting across `Ok(false)` recreate-skips until a census frame presents.
enum CensusState {
    Settle(u32),
    Request,
    Drain(NonZeroU32),
}

/// Everything the row needs that the readback itself cannot carry — supplied by the frame loop at
/// `finish` time rather than latched earlier, because each is a property of the censused frame.
pub(crate) struct CensusContext {
    /// Triangles the frame SUBMITTED (before frustum and backface culling): the numerator of the
    /// `submitted_per_covered_pixel` report-only statistic. It counts culled and off-screen
    /// geometry and adjudicates nothing — it is on the page so the instrument's own behaviour is
    /// visible.
    pub(crate) submitted_tris: u64,
    /// Whether the 2× SSAA composite is armed. **Asserted rather than trusted** by R0c's gate: the
    /// SSAA probe degrades to `Off` silently on a caps or VRAM miss, and the plan's §9.1 grant
    /// table routes the top TWO ladder rungs through the composite — so a rung that merely hoped
    /// for arming would measure `native`, red the extent conjunct, and give no indication why.
    pub(crate) ssaa_armed: bool,
    /// The window's client extent, before any SSAA doubling — recorded beside the achieved extent
    /// so the route a rung actually took is readable from the row rather than inferred.
    pub(crate) native_extent: (u32, u32),
    /// Whether this boot resolved a VisibilityBuffer path carrying a mesh leg. A `false` here means
    /// the recorder deliberately copied nothing (there is no raster to copy), and the row's
    /// sentinel-only readback is an instrument failure that NAMES ITSELF instead of reducing to a
    /// plausible number.
    pub(crate) vb_mesh_leg: bool,
}

/// The census driver the frame loop threads through its steady path.
pub(crate) struct VgCensusDump {
    /// Destination path (the `BOYKO_VG_CENSUS` value), written once.
    path: String,
    state: CensusState,
    /// The host-visible readback staging, created lazily on the request frame (sized to the
    /// composite extent at that moment) and destroyed in [`finish`](Self::finish).
    staging: Option<BoundBuffer>,
    /// The composite extent the staging was sized to — the census's achieved extent.
    extent: VkExtent2D,
}

impl VgCensusDump {
    /// Arms the census iff `BOYKO_VG_CENSUS` is set (the value is the output path). Cold: called
    /// once before the frame loop.
    pub(crate) fn from_env() -> Option<Self> {
        let path = std::env::var("BOYKO_VG_CENSUS").ok()?;
        eprintln!("boyko_app: BOYKO_VG_CENSUS armed -> {path}");
        Some(Self {
            path,
            state: CensusState::Settle(SETTLE_FRAMES),
            staging: None,
            extent: VkExtent2D { width: 0, height: 0 },
        })
    }

    /// The per-frame census request: `Some(&staging)` on the request frame(s) (creating the staging
    /// sized to the CURRENT composite `extent`), `None` otherwise.
    ///
    /// `extent` must be the **composite** (`present_extent`), which is what the `vb_id` ring is
    /// sized to — under armed SSAA that is 2× the client area, and passing the client extent here
    /// would size the staging to a quarter of the image the recorder copies.
    pub(crate) fn request(
        &mut self,
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Option<&BoundBuffer> {
        if !matches!(self.state, CensusState::Request) {
            return None;
        }
        let stale = self.staging.is_some()
            && (self.extent.width != extent.width || self.extent.height != extent.height);
        if stale {
            // SAFETY: the staging has never been referenced by a COMPLETED submission — it is only
            // handed to `render_gbuffer_frame` on request frames, and a request frame that
            // presented moves the state to `Drain` (this branch is unreachable after it); an
            // `Ok(false)` recreate-skip records no work. Created on `ctx`; destroyed exactly once.
            unsafe {
                RhiDevice::destroy_buffer(
                    ctx,
                    self.staging.take().expect("invariant: stale staging exists"),
                );
            }
        }
        if self.staging.is_none() {
            let size = u64::from(extent.width) * u64::from(extent.height) * VB_ID_TEXEL_BYTES;
            let staging = RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size,
                    usage: BufferUsage::TRANSFER_DST,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: host-visible census readback staging create");
            prefill_with_sentinel(ctx, &staging, size);
            self.staging = Some(staging);
            self.extent = extent;
        }
        self.staging.as_ref()
    }

    /// Advances the settle → request → drain machine after a frame attempt (`presented == true` iff
    /// `render_gbuffer_frame` returned `Ok(true)`). Returns `true` when the drained readback is
    /// host-readable — the caller then runs [`finish`](Self::finish) and exits its loop.
    pub(crate) fn after_present(&mut self, presented: bool) -> bool {
        if !presented {
            return false;
        }
        match self.state {
            CensusState::Settle(n) => {
                self.state = if n > 1 { CensusState::Settle(n - 1) } else { CensusState::Request };
                false
            }
            // A presented request frame recorded the vb_id→staging copy.
            CensusState::Request => {
                self.state = CensusState::Drain(
                    NonZeroU32::new(DRAIN_FRAMES).expect("invariant: DRAIN_FRAMES > 0"),
                );
                false
            }
            CensusState::Drain(n) => match NonZeroU32::new(n.get() - 1) {
                Some(left) => {
                    self.state = CensusState::Drain(left);
                    false
                }
                None => true,
            },
        }
    }

    /// Reduces the drained staging, writes the census row, and destroys the staging (consuming the
    /// driver — the census is one-shot).
    ///
    /// The readback is **streamed and hashed, never retained** (`[census].readback_retention`): the
    /// digest is folded from the mapped memory and the row is written; the 66 MB top-rung readback
    /// never reaches the disk.
    pub(crate) fn finish(mut self, ctx: &VulkanContext, cx: &CensusContext) {
        let staging = self.staging.take().expect("invariant: finish follows a drained request");
        let (w, h) = (self.extent.width, self.extent.height);
        let texel_count = (w as usize) * (h as usize);
        let byte_len = texel_count * VB_ID_TEXEL_BYTES as usize;

        let mapped = RhiDevice::buffer_mapped_ptr(ctx, &staging)
            .expect("invariant: host-visible census staging is mapped");
        let base = mapped.as_ptr();
        assert!(
            base.align_offset(align_of::<[u32; 2]>()) == 0,
            "invariant: a Vulkan host mapping is at least 8-byte aligned"
        );

        // SAFETY: `base` points to exactly `byte_len` mapped host-coherent bytes (the staging was
        // created at that size); the readback frame's fence was re-waited (DRAIN_FRAMES exceeds
        // FRAMES_IN_FLIGHT), so the GPU's transfer write is complete and no submission still writes
        // the buffer; the alignment for the `[u32; 2]` view is asserted above; neither slice
        // outlives the `destroy_buffer` below.
        let (bytes, texels) = unsafe {
            (
                core::slice::from_raw_parts(base, byte_len),
                core::slice::from_raw_parts(base.cast::<[u32; 2]>(), texel_count),
            )
        };

        // Hash first, from the mapping itself — no intermediate copy of a buffer this size.
        let mut hasher = Sha256::new();
        for chunk in bytes.chunks(1 << 20) {
            hasher.update(chunk);
        }
        let readback_sha256 = hasher.finish_hex();
        let row = vg_census::reduce(texels);

        // SAFETY: created on `ctx` in `request`; the only submission referencing it completed (the
        // fence was re-waited per the drain); destroyed exactly once (taken out of the Option). The
        // two slices above are not used past this point.
        unsafe {
            RhiDevice::destroy_buffer(ctx, staging);
        }

        match write_row(&self.path, &row, cx, (w, h), byte_len, &readback_sha256) {
            Ok(()) => eprintln!(
                "boyko_app: census row written -> {} ({w}x{h}, covered={}, visible_tris={})",
                self.path, row.covered_pixels, row.visible_tris
            ),
            Err(e) => eprintln!("boyko_app: census row write FAILED ({e}) -> {}", self.path),
        }
    }
}

/// Fills a freshly created staging with the `vb_id` MISS sentinel.
///
/// This is what makes "the recorder copied nothing" distinguishable from "the frame was empty".
/// A frame that resolves no VB mesh leg records no copy at all, and an un-prefilled staging would
/// then be reduced from *uninitialized* host memory — which could plausibly decode as covered
/// texels and fabricate a row. Zero-filling would be worse still: `instance_id == 0` is a VALID
/// key, so zeros read as one triangle covering the whole screen. The sentinel reduces to
/// `covered_pixels == 0`, which R0c(c′) reds on, by name.
fn prefill_with_sentinel(ctx: &VulkanContext, staging: &BoundBuffer, size: u64) {
    let mapped = RhiDevice::buffer_mapped_ptr(ctx, staging)
        .expect("invariant: host-visible census staging is mapped");
    let texels = (size / VB_ID_TEXEL_BYTES) as usize;
    // SAFETY: the mapping covers `size` bytes (the buffer was just created at that size) and this
    // runs before the staging is handed to any submission, so nothing else reads or writes it. The
    // pattern is written as whole `[u32; 2]` texels, exactly tiling the range.
    unsafe {
        let p = mapped.as_ptr().cast::<[u32; 2]>();
        for i in 0..texels {
            p.add(i).write_unaligned([VB_ID_SENTINEL, 0]);
        }
    }
}

/// Writes the census row as TOML.
///
/// The achieved extent is reported, never adjusted: `[census].assert_achieved_extent` is the
/// consumer's assertion, and an instrument that quietly reconciled a clamped window with the
/// requested rung would produce exactly the fabricated curve that field exists to prevent.
fn write_row(
    path: &str,
    row: &CensusRow,
    cx: &CensusContext,
    achieved: (u32, u32),
    readback_bytes: usize,
    readback_sha256: &str,
) -> std::io::Result<()> {
    let mut out = String::with_capacity(1024);
    out.push_str("# VG-R0 census row -- MACHINE-WRITTEN by boyko_app::vg_census_dump.\n");
    out.push_str("# One reading at one (camera path, ladder rung) pair. D_est and the convergence\n");
    out.push_str("# check are NOT here: each spans two rungs, so neither is readable at a pair.\n");
    out.push_str("schema_version = 1\n\n");

    out.push_str("[extent]\n");
    out.push_str(&format!("achieved_width = {}\n", achieved.0));
    out.push_str(&format!("achieved_height = {}\n", achieved.1));
    out.push_str(&format!("native_width = {}\n", cx.native_extent.0));
    out.push_str(&format!("native_height = {}\n", cx.native_extent.1));
    out.push_str(&format!("ssaa_armed = {}\n", cx.ssaa_armed));
    out.push_str(&format!("vb_mesh_leg = {}\n\n", cx.vb_mesh_leg));

    out.push_str("[row]\n");
    out.push_str(&format!("covered_pixels = {}\n", row.covered_pixels));
    out.push_str(&format!("visible_tris = {}\n", row.visible_tris));
    match row.modal_bucket {
        Some(b) => out.push_str(&format!("modal_bucket = {b}\n")),
        // An absent mode is recorded as an absent FIELD rather than as -1 or 0: both would be read
        // as a bucket index by anything that greps for one.
        None => out.push_str("# modal_bucket ABSENT -- the frame carries no visible triangle\n"),
    }
    out.push_str("histogram = [");
    for (i, n) in row.histogram.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&n.to_string());
    }
    out.push_str("]\n");
    out.push_str(&format!("submitted_tris = {}\n", cx.submitted_tris));
    out.push_str(&format!(
        "visible_tri_per_covered_pixel = {:.9}\n",
        row.visible_tri_per_covered_pixel()
    ));
    out.push_str(&format!(
        "submitted_per_covered_pixel = {:.9}\n\n",
        row.submitted_per_covered_pixel(cx.submitted_tris)
    ));

    out.push_str("[readback]\n");
    out.push_str("retention = \"stream_and_hash\"\n");
    out.push_str(&format!("bytes = {readback_bytes}\n"));
    out.push_str(&format!("sha256 = \"{readback_sha256}\"\n"));

    let mut f = std::fs::File::create(path)?;
    f.write_all(out.as_bytes())
}
