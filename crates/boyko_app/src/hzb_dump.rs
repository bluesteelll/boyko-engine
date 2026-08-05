//! **VG R3 piece 1 step P1-6 — the armed HZB pyramid dump** (`BOYKO_HZB_DUMP=<path.bin>`).
//!
//! The host half of the recording seam plan §5 specifies for gate G8: it arms ONE frame's copy of
//! the engine's own `vb_depth` and every mip of the engine's own pyramid into a host-visible
//! staging, waits for that frame's fence, and writes the raw bytes to the named path. Entirely
//! COLD — the steady loop pays one `Option` check per frame, and without the variable nothing is
//! created and **no extra command is recorded**, which is what keeps every golden pin
//! byte-identical armed and unarmed.
//!
//! # What G8 will do with the file, and why it needs one at all
//!
//! Gate G3 (`hzb_build_oracle_gate.rs`) proves the SHADER equals `boyko_render::hzb::build_pyramid`
//! — but it creates its own depth, its own image, its own views, its own sets and its own
//! barriers, so it is structurally blind to a wrong source, a wrong extent, a stale descriptor, a
//! missing barrier or a build pass that never ran. G8 rebuilds from the DUMPED depth and compares
//! against the DUMPED pyramid, which is the only comparison that can see those. This module writes
//! the file; the comparison is step P1-8.
//!
//! # Why the driver is a sibling of `vg_census_dump` rather than a mode of it
//!
//! The same reason that module gives for not being a mode of `host_dump`: the two readbacks copy
//! different resources, at different extents, with different payload shapes, and are armed on
//! different runs. What IS shared is the settle → request → drain fence discipline, and it is
//! shared by DERIVATION — a host read of a per-FIF resource is this project's recorded cross-frame
//! bug class, and the drain below outlives the in-flight ring by one frame for exactly the reason
//! [`DRAIN_FRAMES`] states. (`vb_depth` IS such a ring; the pyramid is not, which makes the drain
//! load-bearing for the depth half specifically.)

use core::num::NonZeroU32;
use std::io::Write as _;

use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::VkExtent2D;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::present::{HzbDumpLayout, HzbPlan};

/// Presented frames rendered before the dump is requested — the same settle window the frame dump
/// and the density census use, and for the same reasons (propagation, light reconcile, the
/// light-table upload and the CSM fit all settle within the first few frames; the slack covers the
/// window manager's first present hitches). It matters here specifically because G8's non-vacuity
/// clause requires the scene to actually COVER the framebuffer: a mid-settle frame with a
/// partially-populated draw list would dump a depth of mostly-cleared texels, and "every dumped
/// depth texel is `> 0.0`" would red for an instrument reason rather than a pyramid one.
const SETTLE_FRAMES: u32 = 30;

/// Presented frames rendered AFTER the dump frame before the staging is mapped:
/// `FRAMES_IN_FLIGHT (2) + 1`. The dump frame's slot fence has necessarily been re-waited by then —
/// the frame loop waits a slot's fence before reusing it, and after `FRAMES_IN_FLIGHT` further
/// presents every slot has been reused at least once.
const DRAIN_FRAMES: u32 = 3;

/// The settle → request → drain progression. `Settle`/`Drain` count REMAINING presented frames;
/// `Request` keeps re-requesting across `Ok(false)` recreate-skips until a dump frame presents.
enum DumpState {
    Settle(u32),
    Request,
    Drain(NonZeroU32),
}

/// The pyramid-dump driver the frame loop threads through its steady path.
pub(crate) struct HzbDump {
    /// Destination path (the `BOYKO_HZB_DUMP` value), written once.
    path: String,
    state: DumpState,
    /// The host-visible readback staging, created lazily on the request frame (sized to the plan
    /// and composite extent at that moment) and destroyed in [`finish`](Self::finish).
    staging: Option<BoundBuffer>,
    /// The layout the staging was sized to. `None` until the first successful request.
    layout: Option<HzbDumpLayout>,
}

impl HzbDump {
    /// Arms the dump iff `BOYKO_HZB_DUMP` is set (the value is the output path). Cold: called once
    /// before the frame loop.
    pub(crate) fn from_env() -> Option<Self> {
        let path = std::env::var("BOYKO_HZB_DUMP").ok()?;
        eprintln!("boyko_app: BOYKO_HZB_DUMP armed -> {path}");
        Some(Self {
            path,
            state: DumpState::Settle(SETTLE_FRAMES),
            staging: None,
            layout: None,
        })
    }

    /// The per-frame dump request: `Some(&staging)` on the request frame(s), `None` otherwise.
    ///
    /// `plan` is THIS frame's [`HzbPlan`] (the runner's single `HzbLayout` call). `None` — the
    /// `HzbMode::Off` 0%-gate, or an extent the oracle refused — yields `None` here as well: there
    /// is no pyramid to dump, and arming the probe cannot conjure one. The state machine does not
    /// advance past `Request` in that case, so a run that arms the dump on an unarmed pyramid
    /// simply never produces a file, which is a skip that names itself rather than an empty dump
    /// that decodes as a black pyramid.
    ///
    /// `extent` must be the **composite** (`present_extent`), the extent the depth ring is sized
    /// to and the extent the recorder's copy region names. Passing the client extent would size
    /// the staging to a quarter of what is copied.
    pub(crate) fn request(
        &mut self,
        ctx: &VulkanContext,
        extent: VkExtent2D,
        plan: Option<HzbPlan>,
    ) -> Option<&BoundBuffer> {
        if !matches!(self.state, DumpState::Request) {
            return None;
        }
        let plan = plan?;
        let layout = HzbDumpLayout::new(plan, [extent.width, extent.height]);

        // A stale staging is one sized to a DIFFERENT layout — a resize, or a live pyramid arm
        // change. Compared on the whole layout rather than on the extent alone, because the plan
        // is what the pyramid region's size comes from and the two can move independently in
        // principle (the extent is the plan's input, not a substitute for it).
        let stale = self.layout.is_some_and(|l| l != layout);
        if stale {
            // SAFETY: the staging has never been referenced by a COMPLETED submission — it is only
            // handed to the scene on request frames, and a request frame that presented moves the
            // state to `Drain` (this branch is unreachable after it); an `Ok(false)` recreate-skip
            // records no work. Created on `ctx`; destroyed exactly once.
            unsafe {
                RhiDevice::destroy_buffer(
                    ctx,
                    self.staging.take().expect("invariant: a stale layout implies a staging"),
                );
            }
        }
        if self.staging.is_none() {
            let staging = RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: layout.total_bytes(),
                    usage: BufferUsage::TRANSFER_DST,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: host-visible HZB dump staging create");
            prefill_with_poison(ctx, &staging, layout.total_bytes());
            self.staging = Some(staging);
            self.layout = Some(layout);
        }
        self.staging.as_ref()
    }

    /// Advances the settle → request → drain machine after a frame attempt (`presented == true` iff
    /// `render_gbuffer_frame` returned `Ok(true)`). Returns `true` when the drained dump is
    /// host-readable — the caller then runs [`finish`](Self::finish).
    ///
    /// ⚠️ A `Request` frame only advances if a staging was actually handed out. Without it the
    /// frame recorded no copy, and treating it as a dump frame would drain and write the poison
    /// prefill as if it were data.
    pub(crate) fn after_present(&mut self, presented: bool) -> bool {
        if !presented {
            return false;
        }
        match self.state {
            DumpState::Settle(n) => {
                self.state = if n > 1 { DumpState::Settle(n - 1) } else { DumpState::Request };
                false
            }
            DumpState::Request => {
                if self.staging.is_none() {
                    return false;
                }
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

    /// Writes the drained staging to the dump path and destroys the staging (consuming the driver
    /// — the dump is one-shot).
    ///
    /// The bytes are written VERBATIM, header included, exactly as
    /// [`HzbDumpLayout`] lays them out. No reduction, no reformatting: unlike the density census
    /// (whose readback is 66 MB at the top rung and is streamed and hashed), this payload IS the
    /// evidence — P1-8 rebuilds from the depth half and compares against the pyramid half, so a
    /// digest would be a gate that can only ever say "different".
    pub(crate) fn finish(mut self, ctx: &VulkanContext) {
        let staging = self.staging.take().expect("invariant: finish follows a drained request");
        let layout = self.layout.expect("invariant: a staging implies the layout it was sized to");
        let byte_len = layout.total_bytes() as usize;

        let mapped = RhiDevice::buffer_mapped_ptr(ctx, &staging)
            .expect("invariant: host-visible HZB dump staging is mapped");

        // SAFETY: `mapped` points to exactly `byte_len` mapped host-coherent bytes (the staging was
        // created at `layout.total_bytes()`); the dump frame's fence was re-waited (DRAIN_FRAMES
        // exceeds FRAMES_IN_FLIGHT), so the GPU's transfer writes are complete and no submission
        // still writes the buffer; the slice does not outlive the `destroy_buffer` below.
        let bytes = unsafe { core::slice::from_raw_parts(mapped.as_ptr(), byte_len) };
        let write_result = write_dump(&self.path, bytes);

        // SAFETY: created on `ctx` in `request`; the only submission referencing it completed (the
        // fence was re-waited per the drain); destroyed exactly once (taken out of the Option). The
        // slice above is not used past this point.
        unsafe {
            RhiDevice::destroy_buffer(ctx, staging);
        }

        let [sw, sh] = layout.source();
        match write_result {
            Ok(()) => eprintln!(
                "boyko_app: HZB dump written -> {} ({sw}x{sh}, levels={}, {byte_len} B)",
                self.path,
                layout.plan().levels
            ),
            Err(e) => eprintln!("boyko_app: HZB dump write FAILED ({e}) -> {}", self.path),
        }
    }
}

/// Fills a freshly created staging with a POISON pattern the dump can never legitimately contain.
///
/// `0xFFFFFFFF` is `f32::NAN` (a quiet negative NaN), and NaN is exactly what neither payload may
/// hold: a reverse-Z depth attachment is clamped to `[minDepth, maxDepth]` and cannot carry one,
/// and `hzb_build`'s reduce collapses any NaN input to `-INFINITY` rather than propagating it (the
/// property `hzb_build_nan_collapses_to_negative_infinity` measures). So a region the recorder
/// failed to copy reads as NaN at every texel and G8 reds by name, instead of reducing
/// uninitialized host memory to a plausible pyramid. Zero-filling would be strictly worse: `0.0` is
/// the reverse-Z FAR PLANE and the boot clear, i.e. a perfectly legal depth value.
fn prefill_with_poison(ctx: &VulkanContext, staging: &BoundBuffer, size: u64) {
    let mapped = RhiDevice::buffer_mapped_ptr(ctx, staging)
        .expect("invariant: host-visible HZB dump staging is mapped");
    // SAFETY: the mapping covers `size` bytes (the buffer was just created at that size) and this
    // runs before the staging is handed to any submission, so nothing else reads or writes it.
    // `write_bytes` fills whole bytes, so no alignment beyond 1 is required.
    unsafe {
        mapped.as_ptr().write_bytes(0xFF, size as usize);
    }
}

/// Writes the dump payload to `path`.
///
/// Created rather than appended: a stale file from a previous run that a new run failed to
/// overwrite would be read by P1-8 as this run's evidence.
fn write_dump(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)
}
