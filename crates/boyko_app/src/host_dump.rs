//! The env-gated host frame dump (`BOYKO_HOST_DUMP=<path.bmp>`) — the windowed
//! runner's diagnostic / owner-eval channel (the `window_present_gbuffer`
//! screenshot-dump pattern, lifted into the production host so the ECS-driven
//! frame stream itself can be captured; the R6 viewer parity gate reuses it).
//!
//! When the variable is set the frame loop renders normally for a settle
//! window ([`SETTLE_FRAMES`] presented frames, `BOYKO_HOST_DUMP_SETTLE=<S>`
//! overrides it), requests the renderer's swapchain→staging readback on the
//! next `N` consecutive presented frames (`BOYKO_HOST_DUMP_FRAMES=<N>`, default
//! 1, cap [`MAX_BURST`]), drains each captured frame's in-flight ring so its
//! fence is proven re-waited ([`DRAIN_FRAMES`], the golden readback discipline),
//! writes every captured image as a 32-bpp BMP, and exits the loop once all `N`
//! are written. Entirely COLD: the steady loop pays one `Option` check per
//! frame; without the variable nothing is created.
//!
//! # One machine, two shapes
//!
//! The one-shot dump (`BOYKO_HOST_DUMP_FRAMES` unset) is the `N = 1` special
//! case of the burst, not a second code path: it requests on presented frame
//! `S` (30), writes after presented frame `S + 3`, and writes to the path
//! VERBATIM — the golden sweep's hashes and file names are those of the
//! pre-burst instrument. With `BOYKO_HOST_DUMP_FRAMES` set (any `N`, 1
//! included) the files are `<stem>_<frame_index>.bmp` and a sidecar
//! `<stem>_frames.txt` carries one state line per captured frame (the
//! [`FrameMeta`] fields, the values the GPU consumed on that frame), so the
//! free-running per-frame pattern of a run is exact rather than sampled.
//!
//! # The staging ring
//!
//! Consecutive captures need overlapping drains, so one staging cannot serve.
//! [`RING`] `= DRAIN_FRAMES + 1` is the minimum that never aliases: slot
//! `k % RING` is read (its drain reaches zero) at the END of presented frame
//! `k + DRAIN_FRAMES` and re-requested at the START of frame `k + RING`, so no
//! slot is ever referenced by two submissions at once. The bookkeeping that
//! carries this argument ([`Schedule`]) is PURE — no Vulkan handle — and is
//! unit-tested below for both the `N = 1` and the burst timelines, which is the
//! proof every `// SAFETY:` on the mapped reads cites.
//!
//! Each slot captures its OWN swapchain extent at request time: a swapchain
//! recreate mid-burst resizes only the slots requested after it, and the write
//! of an earlier slot reads exactly the bytes that slot's staging was sized to.

use core::fmt::Write as _;
use std::io::Write as _;

use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::{VK_FORMAT_R8G8B8A8_UNORM, VkExtent2D};
use boyko_rhi_vulkan::memory::BoundBuffer;

/// Presented frames rendered before the first readback is requested —
/// propagation, light reconcile, the light-table upload, and the CSM fit are
/// all settled within the first few frames; 30 (~0.5 s under FIFO) adds slack
/// for the window manager's first-present hitches. `BOYKO_HOST_DUMP_SETTLE`
/// overrides it (0 captures from the very first presented frame).
const SETTLE_FRAMES: u32 = 30;

/// Presented frames rendered AFTER a readback frame before its staging is
/// read: `FRAMES_IN_FLIGHT (2) + 1` — the readback frame's slot fence has been
/// re-waited by then (the same drain rationale the golden readback tests pin).
const DRAIN_FRAMES: u32 = 3;

/// Staging slots: a slot is read at the end of frame `k + DRAIN_FRAMES` and
/// re-requested at the start of frame `k + RING`, so `DRAIN_FRAMES + 1` slots
/// never alias (see the module doc; [`Schedule`]'s tests pin it).
const RING: usize = DRAIN_FRAMES as usize + 1;

/// The largest `BOYKO_HOST_DUMP_FRAMES` accepted (a larger request is clamped
/// with a warning) — 256 frames is ~1.8 s at 144 Hz, more than any per-frame
/// pattern this instrument exists to resolve needs.
const MAX_BURST: u32 = 256;

/// The BMP file header + info header, in bytes (`BITMAPFILEHEADER` 14 +
/// `BITMAPINFOHEADER` 40).
const BMP_HEADER_BYTES: usize = 54;

/// The per-frame state line's fields — the values the GPU consumed on the
/// captured frame. Filled by the runner ONLY when the dump is armed (a
/// `#[cold]` helper), so the steady path builds none of it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameMeta {
    /// The runner's monotonic frame counter (`== SHADOW_FRAME_SEED` on hwrt frames).
    pub frame_index: u32,
    /// `token.slot()` — the in-flight ring slot the frame's uploads went into.
    pub slot: u32,
    /// `JitterState.phase` post-advance — the `HALTON_8` index the GPU got.
    pub jitter_phase: u32,
    /// `JitterState.armed` — `false` on every AA-off frame (the phase is inert).
    pub jitter_armed: bool,
    /// The host's CSM depth-pass arming (`depth_pass_armed`, `frame_csm_armed`).
    pub csm_armed: bool,
    /// The `CSM_MODE_BIT` of the LAST UPLOADED light-header word 7 — the CSM
    /// bit the GPU reads this frame (it can trail `csm_armed` by frames).
    pub header_csm: bool,
    /// `true` iff this frame re-uploaded the light staging (the gen-gate fired).
    pub light_uploaded: bool,
    /// `Some(frame_index)` iff the HWRT shadow-params UBO was uploaded this
    /// frame (hwrt build + `ray_query_enabled()`); `None` on software frames.
    pub seed: Option<u32>,
    /// The uploaded `SHADOW_ORIGIN_MODE` (hwrt frames); `None` on software.
    pub origin_mode: Option<u32>,
    /// The uploaded `SHADOW_RASTER_FWD.xyz` (hwrt frames); `None` on software.
    pub raster_fwd: Option<[f32; 3]>,
}

/// The hwrt half of a frame's state line — what the runner's 5d'' step uploaded
/// into the HWRT shadow-params UBO this frame. `None` on a software build or a
/// non-RT device (the UBO ring is not minted there).
#[derive(Clone, Copy, Debug)]
pub(crate) struct RayShadowUpload {
    /// The uploaded `SHADOW_FRAME_SEED` (the runner's `frame_index`).
    pub seed: u32,
}

/// A captured frame whose staging is draining: the metadata to write beside it
/// and the presented frames still to elapse before the mapped read is proven
/// safe.
#[derive(Clone, Copy, Debug)]
struct Pending {
    meta: FrameMeta,
    drain_left: u32,
}

/// The pure settle → burst → drain bookkeeping — the settle countdown, the
/// request index, the per-slot drain countdowns and the completion count —
/// split from the Vulkan buffer ops so the ring's aliasing argument is a CPU
/// unit test rather than a device run (critic W2).
///
/// Protocol per presented-frame attempt, mirroring the runner's call order:
/// [`request_slot`](Self::request_slot) once at the start (the slot whose
/// staging the frame's copy is recorded into, if any), then
/// [`after_present`](Self::after_present) once after the render call.
#[derive(Debug)]
struct Schedule {
    /// `N` — the burst length (1 for the one-shot).
    total: u32,
    /// Presented frames still to settle before the first request.
    settle_left: u32,
    /// Frames requested so far (the next request uses slot `requested % RING`).
    requested: u32,
    /// Frames written so far; the machine is done at `written == total`.
    written: u32,
    /// `true` between a `request_slot` that returned `Some` and the
    /// `after_present` that consumes it.
    carried: bool,
    /// Per ring slot: the captured frame draining in it, if any.
    pending: [Option<Pending>; RING],
}

impl Schedule {
    fn new(total: u32, settle: u32) -> Self {
        debug_assert!(total >= 1, "invariant: the burst length is clamped to 1..=MAX_BURST");
        Self {
            total,
            settle_left: settle,
            requested: 0,
            written: 0,
            carried: false,
            pending: [None; RING],
        }
    }

    /// The ring slot this frame's readback goes into: `None` while settling or
    /// once every frame of the burst has been requested. Re-requesting after an
    /// unpresented attempt returns the SAME slot (nothing advanced).
    fn request_slot(&mut self) -> Option<usize> {
        if self.settle_left > 0 || self.requested >= self.total {
            return None;
        }
        let k = self.requested as usize % RING;
        debug_assert!(
            self.pending[k].is_none(),
            "invariant: a ring slot is never re-requested while its capture is still draining (RING = DRAIN_FRAMES + 1)"
        );
        self.carried = true;
        Some(k)
    }

    /// Advances the machine after a frame attempt (`presented == true` iff
    /// `render_gbuffer_frame` returned `Ok(true)`; a skipped frame advances
    /// nothing, and a request it carried is re-issued next frame). Returns, per
    /// ring slot, the metadata of the capture whose drain elapsed THIS frame —
    /// its staging is host-readable now. A request carried by this frame starts
    /// its countdown on the NEXT presented frame, so a capture at presented
    /// frame `k` is ready after frame `k + DRAIN_FRAMES` presents.
    fn after_present(&mut self, presented: bool, meta: FrameMeta) -> [Option<FrameMeta>; RING] {
        let mut ready = [None; RING];
        if !presented {
            self.carried = false;
            return ready;
        }
        if self.settle_left > 0 {
            debug_assert!(!self.carried, "invariant: no request is issued while settling");
            self.settle_left -= 1;
            return ready;
        }
        // The captures already draining count this frame down; the one this frame
        // carried (if any) is registered AFTER, so it is not counted on its own frame.
        for (slot, p) in self.pending.iter_mut().enumerate() {
            if let Some(pend) = p {
                pend.drain_left -= 1;
                if pend.drain_left == 0 {
                    ready[slot] = Some(pend.meta);
                    *p = None;
                    self.written += 1;
                }
            }
        }
        if self.carried {
            let k = self.requested as usize % RING;
            debug_assert!(self.pending[k].is_none(), "invariant: the requested slot was free");
            self.pending[k] = Some(Pending { meta, drain_left: DRAIN_FRAMES });
            self.requested += 1;
            self.carried = false;
        }
        debug_assert!(
            self.written <= self.requested && self.requested <= self.total,
            "invariant: written <= requested <= total"
        );
        ready
    }

    /// `true` once every frame of the burst has been written.
    fn done(&self) -> bool {
        self.written == self.total
    }
}

/// One staging slot of the readback ring.
struct Slot {
    /// The host-visible readback staging, created lazily on the slot's first
    /// request (sized to the swapchain extent at that moment) and destroyed in
    /// [`HostDump::finish`].
    staging: Option<BoundBuffer>,
    /// The swapchain extent THIS slot's staging was sized to — captured at
    /// request time, the BMP dimensions of the frame it holds (critic W1: a
    /// mid-burst recreate must not make an earlier slot's read out-of-bounds).
    extent: VkExtent2D,
}

/// The dump driver the frame loop threads through its steady path — see the
/// module docs for the protocol.
pub(crate) struct HostDump {
    /// Destination path (the `BOYKO_HOST_DUMP` value): written verbatim when
    /// `!suffixed`, else the stem the per-frame names derive from.
    path: String,
    /// `true` iff `BOYKO_HOST_DUMP_FRAMES` was set — per-frame file names
    /// (`<stem>_<frame_index>.bmp`) and the `<stem>_frames.txt` sidecar.
    suffixed: bool,
    /// `path` without its `.bmp` extension — the per-frame name stem.
    stem: String,
    schedule: Schedule,
    ring: [Slot; RING],
    /// `true` when the swapchain readback bytes are RGBA (the BMP write then
    /// swaps R/B); `false` for the BGRA swapchain (bytes pass through). Set
    /// once at arm time from the swapchain format.
    rgba_source: bool,
    /// One BMP-sized buffer (header + rows), grown to the largest captured
    /// frame and reused for every write.
    scratch: Vec<u8>,
    /// The reusable per-frame file-name buffer.
    file_path: String,
    /// The reusable per-frame state-line buffer.
    line_buf: String,
    /// Mirror of the last uploaded light-header word 7 (see
    /// [`note_light_header`](Self::note_light_header)).
    last_header_w7: u32,
    /// The `<stem>_frames.txt` sidecar, open iff `suffixed` and the create
    /// succeeded (a failure is reported once and the lines go to the log only).
    lines: Option<std::fs::File>,
}

impl HostDump {
    /// Arms the dump iff `BOYKO_HOST_DUMP` is set (the value is the output
    /// path), reading `BOYKO_HOST_DUMP_FRAMES` / `BOYKO_HOST_DUMP_SETTLE` beside
    /// it. `vk_format` is the swapchain's raw `VkFormat` — the host boot only
    /// admits the two UNORM formats, so "not R8G8B8A8" means the readback bytes
    /// are already BGRA (the BMP's native pixel order). Cold: called once
    /// before the frame loop. A malformed knob value is a warning and the
    /// default, never a panic (a diagnostic knob must not take the run down).
    pub(crate) fn from_env(vk_format: i32) -> Option<Self> {
        let path = std::env::var("BOYKO_HOST_DUMP").ok()?;
        let (total, suffixed) = match std::env::var("BOYKO_HOST_DUMP_FRAMES") {
            Err(_) => (1, false),
            Ok(raw) => match raw.trim().parse::<u32>() {
                Ok(n) if (1..=MAX_BURST).contains(&n) => (n, true),
                Ok(n) => {
                    let clamped = n.clamp(1, MAX_BURST);
                    boyko_log::info!(
                        boyko_log::Host,
                        "BOYKO_HOST_DUMP_FRAMES={} is outside 1..={}; clamped to {}",
                        n,
                        MAX_BURST,
                        clamped
                    );
                    (clamped, true)
                }
                Err(_) => {
                    static W3009_FRAMES: boyko_log::codes::OnceSite = boyko_log::codes::OnceSite::new();
                    crate::diag::report_unrecognized_env_value(
                        &W3009_FRAMES,
                        "BOYKO_HOST_DUMP_FRAMES",
                        raw.as_str(),
                        "1 (one-shot, unsuffixed)",
                        "an integer in 1..=256",
                    );
                    (1, false)
                }
            },
        };
        let settle = match std::env::var("BOYKO_HOST_DUMP_SETTLE") {
            Err(_) => SETTLE_FRAMES,
            Ok(raw) => match raw.trim().parse::<u32>() {
                Ok(s) => s,
                Err(_) => {
                    static W3009_SETTLE: boyko_log::codes::OnceSite = boyko_log::codes::OnceSite::new();
                    crate::diag::report_unrecognized_env_value(
                        &W3009_SETTLE,
                        "BOYKO_HOST_DUMP_SETTLE",
                        raw.as_str(),
                        "30",
                        "a non-negative integer",
                    );
                    SETTLE_FRAMES
                }
            },
        };
        boyko_log::info!(
            boyko_log::Host,
            "BOYKO_HOST_DUMP armed -> {} (frames={} settle={} suffixed={})",
            boyko_log::dsp!(path, 192),
            total,
            settle,
            suffixed
        );
        let stem = path
            .strip_suffix(".bmp")
            .or_else(|| path.strip_suffix(".BMP"))
            .unwrap_or(path.as_str())
            .to_owned();
        let lines = if suffixed {
            let mut sidecar = String::with_capacity(stem.len() + 12);
            // `String` never fails `fmt::Write`.
            let _ = write!(sidecar, "{stem}_frames.txt");
            match std::fs::File::create(&sidecar) {
                Ok(f) => Some(f),
                Err(e) => {
                    let err = crate::diag::debug_into(&e);
                    crate::diag::report_dump_write_failed("frame dump sidecar", &sidecar, err.as_str());
                    None
                }
            }
        } else {
            None
        };
        Some(Self {
            path,
            suffixed,
            stem,
            schedule: Schedule::new(total, settle),
            ring: core::array::from_fn(|_| Slot {
                staging: None,
                extent: VkExtent2D { width: 0, height: 0 },
            }),
            rgba_source: vk_format == VK_FORMAT_R8G8B8A8_UNORM,
            scratch: Vec::new(),
            file_path: String::new(),
            line_buf: String::new(),
            last_header_w7: 0,
            lines,
        })
    }

    /// Records the light-header word 7 the runner just uploaded — called at the
    /// light upload site on upload frames only, so
    /// [`FrameMeta::header_csm`] reflects the value the GPU reads (the last
    /// uploaded header, not the staging's current one).
    pub(crate) fn note_light_header(&mut self, word7: u32) {
        self.last_header_w7 = word7;
    }

    /// The `CSM_MODE_BIT` of the last uploaded light-header word 7.
    pub(crate) fn header_csm(&self) -> bool {
        (self.last_header_w7 >> boyko_render::CSM_MODE_BIT) & 1 != 0
    }

    /// The per-frame readback request: `Some(&staging)` on a request frame
    /// (creating or resizing THIS slot's staging to the CURRENT swapchain
    /// `extent`), `None` otherwise. A recreate between two attempts on the same
    /// slot resizes it (the previous attempt's frame was skipped, so nothing
    /// referenced it); a recreate between bursts resizes the slot at its next
    /// request (its previous capture was written, its fence re-waited).
    pub(crate) fn request(
        &mut self,
        ctx: &VulkanContext,
        extent: VkExtent2D,
    ) -> Option<&BoundBuffer> {
        let k = self.schedule.request_slot()?;
        let slot = &mut self.ring[k];
        let stale = slot.staging.is_some()
            && (slot.extent.width != extent.width || slot.extent.height != extent.height);
        if stale {
            // SAFETY: the slot is not pending (`Schedule::request_slot` asserts
            // it), so its staging is referenced by no in-flight submission:
            // either its last request's frame returned `Ok(false)` — both such
            // returns in `Renderer::drive_frame` (the `render_gbuffer_frame`
            // skeleton: an OUT_OF_DATE acquire before any submit, and an
            // OUT_OF_DATE / SUBOPTIMAL present AFTER the readback copy was
            // submitted) pass through `Renderer::recreate`, whose
            // `device_wait_idle` drains every submission before the return —
            // or its capture was written after a drain that re-waited the
            // capturing frame's fence, and the slot cleared. Created on `ctx`;
            // destroyed exactly once (the `take`).
            unsafe {
                RhiDevice::destroy_buffer(
                    ctx,
                    slot.staging.take().expect("invariant: stale staging exists"),
                );
            }
        }
        if slot.staging.is_none() {
            let size = u64::from(extent.width) * u64::from(extent.height) * 4;
            slot.staging = Some(
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
            slot.extent = extent;
        }
        slot.staging.as_ref()
    }

    /// Advances the settle → burst → drain machine after a frame attempt
    /// (`presented == true` iff `render_gbuffer_frame` returned `Ok(true)`),
    /// writing every capture whose drain elapsed this frame. `meta` describes
    /// THIS frame (it is kept only when the frame carried a request). Returns
    /// `true` once all `N` frames are written — the caller then runs
    /// [`finish`](Self::finish) and exits its loop.
    pub(crate) fn after_present(
        &mut self,
        ctx: &VulkanContext,
        presented: bool,
        meta: FrameMeta,
    ) -> bool {
        let ready = self.schedule.after_present(presented, meta);
        for (k, m) in ready.into_iter().enumerate() {
            if let Some(m) = m {
                self.write_frame(ctx, k, &m);
            }
        }
        self.schedule.done()
    }

    /// Reads slot `k`'s drained staging and writes its BMP + state line.
    #[cold]
    #[inline(never)]
    fn write_frame(&mut self, ctx: &VulkanContext, k: usize, meta: &FrameMeta) {
        let slot = &self.ring[k];
        let staging = slot
            .staging
            .as_ref()
            .expect("invariant: a drained slot holds the staging its capture was copied into");
        let (w, h) = (slot.extent.width, slot.extent.height);
        let byte_len = (w as usize) * (h as usize) * 4;
        // Hard bound BEFORE the mapped read (the `upload_ray_shadow_ring`
        // discipline): the staging was created at exactly this slot's extent.
        assert!(
            byte_len as u64 <= staging.size,
            "frame dump slot {k}: {byte_len} bytes for {w}x{h} exceed the {}-byte staging",
            staging.size
        );
        let mapped = RhiDevice::buffer_mapped_ptr(ctx, staging)
            .expect("invariant: host-visible dump staging is mapped");
        // SAFETY: `mapped` points to >= `staging.size >= byte_len` mapped
        // host-coherent bytes (asserted above; the slot's extent is the one its
        // staging was created at, W1). The capturing frame's fence was re-waited
        // (`Schedule` releases a slot only after `DRAIN_FRAMES > FRAMES_IN_FLIGHT`
        // presented frames — the CPU-tested ring argument), so the GPU's
        // transfer write is complete and no submission still writes the buffer;
        // the slice is read-only and dies at the end of this call, before any
        // later request can touch the slot.
        let pixels: &[u8] = unsafe { core::slice::from_raw_parts(mapped.as_ptr(), byte_len) };
        write_bmp_into(&mut self.scratch, pixels, w, h, self.rgba_source);

        self.file_path.clear();
        if self.suffixed {
            // `String` never fails `fmt::Write`.
            let _ = write!(self.file_path, "{}_{}.bmp", self.stem, meta.frame_index);
        } else {
            self.file_path.push_str(&self.path);
        }
        match std::fs::File::create(&self.file_path).and_then(|mut f| f.write_all(&self.scratch)) {
            Ok(()) => boyko_log::info!(
                boyko_log::Host,
                "frame dump written -> {} ({}x{})",
                boyko_log::dsp!(self.file_path, 192),
                w,
                h
            ),
            Err(e) => {
                let err = crate::diag::debug_into(&e);
                crate::diag::report_dump_write_failed("frame dump", &self.file_path, err.as_str());
            }
        }

        // The state line names the file by its base name only: the sidecar sits beside the
        // frames, and a full path would push the line past the log record's bound.
        let file_name = self
            .file_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(self.file_path.as_str());
        self.line_buf.clear();
        // `String` never fails `fmt::Write`.
        let _ = write!(
            self.line_buf,
            "hostdump frame={} slot={} phase={} jitter_armed={} csm_armed={} header_csm={} light_uploaded={} seed={} origin_mode={} raster_fwd={} file={}",
            meta.frame_index,
            meta.slot,
            meta.jitter_phase,
            u8::from(meta.jitter_armed),
            u8::from(meta.csm_armed),
            u8::from(meta.header_csm),
            u8::from(meta.light_uploaded),
            OptU32(meta.seed),
            OptU32(meta.origin_mode),
            OptVec3(meta.raster_fwd),
            file_name
        );
        boyko_log::info!(boyko_log::Host, "{}", boyko_log::dsp!(self.line_buf, 256));
        if let Some(f) = self.lines.as_mut()
            && let Err(e) = f.write_all(self.line_buf.as_bytes()).and_then(|()| f.write_all(b"\n"))
        {
            let err = crate::diag::debug_into(&e);
            crate::diag::report_dump_write_failed("frame dump sidecar", &self.stem, err.as_str());
            self.lines = None;
        }
    }

    /// Destroys every staging of the ring (consuming the driver — every frame
    /// of the burst has been written by [`after_present`](Self::after_present)).
    pub(crate) fn finish(mut self, ctx: &VulkanContext) {
        for slot in self.ring.iter_mut() {
            if let Some(staging) = slot.staging.take() {
                // SAFETY: created on `ctx`; the only submissions referencing it
                // completed (each capture's fence re-waited per the drain before
                // its write, and no request is outstanding once the machine is
                // done); destroyed exactly once (taken out of the Option).
                unsafe {
                    RhiDevice::destroy_buffer(ctx, staging);
                }
            }
        }
        if let Some(mut f) = self.lines.take() {
            let _ = f.flush();
        }
    }
}

/// `Display` for an `Option<u32>` state-line field: the value, or `-` when the
/// field does not apply to this build/frame.
struct OptU32(Option<u32>);

impl core::fmt::Display for OptU32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{v}"),
            None => f.write_str("-"),
        }
    }
}

/// `Display` for an `Option<[f32; 3]>` state-line field: `(x,y,z)` or `-`.
struct OptVec3(Option<[f32; 3]>);

impl core::fmt::Display for OptVec3 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some([x, y, z]) => write!(f, "({x},{y},{z})"),
            None => f.write_str("-"),
        }
    }
}

/// Writes `pixels` (tightly packed 4 B/texel, row-major top-down) as a 32-bpp
/// uncompressed BMP into `out` (cleared first; grown only when a larger frame
/// than any before is written). BMP rows are bottom-up and its pixel order is
/// BGRA: `rgba_source` selects the R/B swap.
fn write_bmp_into(out: &mut Vec<u8>, pixels: &[u8], w: u32, h: u32, rgba_source: bool) {
    let row_bytes = (w as usize) * 4;
    let data_len = row_bytes * (h as usize);
    debug_assert_eq!(pixels.len(), data_len, "invariant: the pixel slice is exactly w*h*4");
    let file_len = BMP_HEADER_BYTES + data_len;

    out.clear();
    out.reserve(file_len);
    // BITMAPFILEHEADER (14 bytes).
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_len as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(BMP_HEADER_BYTES as u32).to_le_bytes());
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
            for px in src.as_chunks::<4>().0 {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        } else {
            out.extend_from_slice(src);
        }
    }
    debug_assert_eq!(out.len(), file_len);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(frame_index: u32) -> FrameMeta {
        FrameMeta {
            frame_index,
            slot: frame_index & 1,
            jitter_phase: (frame_index + 1) % 8,
            jitter_armed: true,
            csm_armed: true,
            header_csm: true,
            light_uploaded: false,
            seed: None,
            origin_mode: None,
            raster_fwd: None,
        }
    }

    /// Drives `sched` through one presented frame exactly as the runner does
    /// (request, then after_present) and returns the frames written this frame.
    fn step(sched: &mut Schedule, frame: u32, presented: bool) -> (Option<usize>, Vec<u32>) {
        let slot = sched.request_slot();
        let ready = sched.after_present(presented, meta(frame));
        let written: Vec<u32> = ready.iter().flatten().map(|m| m.frame_index).collect();
        (slot, written)
    }

    /// The one-shot (`N = 1`, the legacy contract): request on presented frame
    /// 30 into slot 0, written after frame 33 presents, done — nothing before.
    #[test]
    fn one_shot_requests_at_settle_and_writes_after_drain() {
        let mut s = Schedule::new(1, SETTLE_FRAMES);
        for frame in 0..30 {
            let (slot, written) = step(&mut s, frame, true);
            assert_eq!(slot, None, "frame {frame}: settling, no request");
            assert!(written.is_empty());
            assert!(!s.done());
        }
        let (slot, written) = step(&mut s, 30, true);
        assert_eq!(slot, Some(0));
        assert!(written.is_empty());
        for frame in 31..33 {
            let (slot, written) = step(&mut s, frame, true);
            assert_eq!(slot, None, "frame {frame}: the burst is fully requested");
            assert!(written.is_empty(), "frame {frame}: still draining");
            assert!(!s.done());
        }
        let (slot, written) = step(&mut s, 33, true);
        assert_eq!(slot, None);
        assert_eq!(written, vec![30], "written after frame 30 + DRAIN_FRAMES presents");
        assert!(s.done());
    }

    /// The burst (`N = 8`): requests on 30..37 into ring slots 0,1,2,3,0,1,2,3,
    /// each written after 33..40, no slot requested while pending, done after 40.
    #[test]
    fn burst_of_eight_never_aliases_a_pending_slot() {
        let mut s = Schedule::new(8, SETTLE_FRAMES);
        for frame in 0..30 {
            let (slot, _) = step(&mut s, frame, true);
            assert_eq!(slot, None);
        }
        let mut writes: Vec<(u32, u32)> = Vec::new(); // (written frame, at frame)
        for frame in 30..=40 {
            // Every pending slot must be free at request time — the aliasing argument.
            let pending_before: Vec<usize> =
                (0..RING).filter(|&k| s.pending[k].is_some()).collect();
            let (slot, written) = step(&mut s, frame, true);
            if frame <= 37 {
                let expect = (frame - 30) as usize % RING;
                assert_eq!(slot, Some(expect), "frame {frame} requests slot {expect}");
                assert!(!pending_before.contains(&expect), "frame {frame}: slot {expect} was pending");
            } else {
                assert_eq!(slot, None, "frame {frame}: burst fully requested");
            }
            for w in written {
                writes.push((w, frame));
            }
            assert_eq!(s.done(), frame == 40, "done exactly after frame 40");
        }
        let expected: Vec<(u32, u32)> = (30..=37).map(|k| (k, k + DRAIN_FRAMES)).collect();
        assert_eq!(writes, expected, "frame k is written after frame k + DRAIN_FRAMES");
    }

    /// A skipped frame (`presented == false`) advances nothing: the settle count,
    /// the request index and every drain countdown are unchanged, and the
    /// request the skipped frame carried is re-issued on the SAME slot.
    #[test]
    fn unpresented_frames_advance_nothing() {
        let mut s = Schedule::new(3, 2);
        // Settle: a skip does not count.
        let (slot, _) = step(&mut s, 0, false);
        assert_eq!(slot, None);
        assert_eq!(s.settle_left, 2);
        step(&mut s, 0, true);
        step(&mut s, 1, true);
        assert_eq!(s.settle_left, 0);
        // Request frame skipped: the same slot is handed out again.
        let (slot, written) = step(&mut s, 2, false);
        assert_eq!(slot, Some(0));
        assert!(written.is_empty());
        assert_eq!(s.requested, 0);
        assert!(s.pending.iter().all(Option::is_none));
        let (slot, _) = step(&mut s, 2, true);
        assert_eq!(slot, Some(0));
        assert_eq!(s.requested, 1);
        assert_eq!(s.pending[0].map(|p| p.drain_left), Some(DRAIN_FRAMES));
        // A skip during the drain leaves the countdown where it was.
        let (_, written) = step(&mut s, 3, false);
        assert!(written.is_empty());
        assert_eq!(s.pending[0].map(|p| p.drain_left), Some(DRAIN_FRAMES));
        assert_eq!(s.requested, 1, "the skipped frame's request is not registered");
    }

    /// `BOYKO_HOST_DUMP_SETTLE=0`: the very first presented frame is captured
    /// (the G-BLINK receipt shape) and a 3-frame burst is done after frame 5.
    #[test]
    fn zero_settle_captures_frame_zero() {
        let mut s = Schedule::new(3, 0);
        let (slot, _) = step(&mut s, 0, true);
        assert_eq!(slot, Some(0));
        let mut writes = Vec::new();
        for frame in 1..=5 {
            let (_, written) = step(&mut s, frame, true);
            for w in written {
                writes.push((w, frame));
            }
        }
        assert_eq!(writes, vec![(0, 3), (1, 4), (2, 5)]);
        assert!(s.done());
    }

    /// The BMP writer produces a 54-byte header + bottom-up BGRA rows, and a
    /// second call into the same scratch overwrites rather than appends.
    #[test]
    fn write_bmp_into_is_bottom_up_bgra_and_reuses_the_scratch() {
        let mut out = Vec::new();
        // 2x2 RGBA, row 0 = red, green; row 1 = blue, white.
        let px = [
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ];
        write_bmp_into(&mut out, &px, 2, 2, true);
        assert_eq!(out.len(), BMP_HEADER_BYTES + 16);
        assert_eq!(&out[0..2], b"BM");
        // Bottom-up: the file's first row is source row 1 (blue, white), R/B swapped.
        assert_eq!(&out[54..62], &[255, 0, 0, 255, 255, 255, 255, 255]);
        assert_eq!(&out[62..70], &[0, 0, 255, 255, 0, 255, 0, 255]);
        // BGRA source passes through.
        write_bmp_into(&mut out, &px, 2, 2, false);
        assert_eq!(out.len(), BMP_HEADER_BYTES + 16);
        assert_eq!(&out[54..62], &px[8..16]);
        assert_eq!(&out[62..70], &px[0..8]);
    }
}
