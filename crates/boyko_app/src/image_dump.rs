//! **OBS (D) — the image dump** (`BOYKO_IMAGE_DUMP=<name>[,<name>…]`; render-wave plan H0
//! Scope 6): device readback of named frame-graph images, one frame or a burst.
//!
//! The names are the images' literal graph names — `depth` (Deferred), `viewt`, `lit`,
//! `taa_hist` (the `[fi]` write slot), `forward_depth`, `vb_depth` — and each declarator copies
//! the ones it declares. `BOYKO_IMAGE_DUMP_PATH=<dir>` is where `<name>_<frame>.bin` files go;
//! `_SETTLE` (default 30) and `_FRAMES` (default 1, at most 256) are the `host_dump` burst shape.
//!
//! # The boot decisions (the instrument's, not the frame's)
//!
//! Armed, the renderer creates Deferred `depth`, `viewt` and `taa_hist` with `TRANSFER_SRC` and
//! widens every `SeededImage` / `SeededBuffer` seed with the dump's `TRANSFER` read
//! (`Renderer::arm_image_dump`). A shipped run does neither, so its seeds, usage sets and barrier
//! streams are untouched — and, for the same reason, **a dump run's barrier stream is not evidence
//! about production barriers** (H0 rev 5): a cross-frame gate reads a production receipt.
//!
//! # Refusals
//!
//! An unknown name, an empty list, a missing `_PATH`, or `BOYKO_HZB_DUMP` / `BOYKO_HOST_DUMP` in
//! the same run (each appends its own last pass) is a named boot refusal: the run exits before its
//! first frame rather than ordering two dump passes or dumping nothing.
//!
//! # The staging ring
//!
//! `host_dump`'s: [`RING`] `= DRAIN_FRAMES + 1` slots, one staging per requested name in each,
//! allocated on the slot's first request (cold); a slot is read after `DRAIN_FRAMES >
//! FRAMES_IN_FLIGHT` more presented frames, by which time its frame's fence was re-waited. The
//! bookkeeping ([`Schedule`]) is pure and unit-tested below.
//!
//! # The file
//!
//! A 32-byte little-endian header — [`MAGIC`], `VkFormat`, width, height, mip, layers, the engine
//! frame index the recording frame carried, bytes per texel — then the tightly packed texels of
//! mip 0, layer 0, as the copy wrote them. Every header field is what the RECORDER used.

use core::fmt::Write as _;
use std::io::Write as _;

use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::ffi::{VkBuffer, VkExtent2D};
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::present::{
    IMAGE_DUMP_MAX, ImageDumpCopy, ImageDumpName, ImageDumpRequest, Renderer, image_dump_texel_bytes,
};

use crate::graph_receipt_dump::{parse_burst, parse_settle};

/// Presented frames after a capture before its staging is read: `FRAMES_IN_FLIGHT (2) + 1`.
const DRAIN_FRAMES: u32 = 3;

/// Staging slots: a slot is read at the end of frame `k + DRAIN_FRAMES` and re-requested at the
/// start of frame `k + RING`, so `DRAIN_FRAMES + 1` slots never alias.
const RING: usize = DRAIN_FRAMES as usize + 1;

/// The file header's size.
pub(crate) const HEADER_BYTES: usize = 32;

/// The file header's first word, `"OBSD"` read little-endian.
pub(crate) const MAGIC: u32 = u32::from_le_bytes(*b"OBSD");

/// Why the boot refused the instrument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A name that is no frame-graph image the dump can copy.
    UnknownName(String),
    /// `BOYKO_IMAGE_DUMP` names nothing.
    NoNames,
    /// `BOYKO_IMAGE_DUMP_PATH` is unset.
    NoPath,
    /// Another last-pass dump instrument is armed in the same run.
    Exclusive(&'static str),
}

/// The parsed configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Config {
    pub(crate) names: [bool; IMAGE_DUMP_MAX],
    pub(crate) dir: String,
    pub(crate) frames: u32,
    pub(crate) settle: u32,
}

/// The pure decode of the instrument's environment (`hzb` / `host`: whether `BOYKO_HZB_DUMP` /
/// `BOYKO_HOST_DUMP` are set).
pub(crate) fn configure(
    names: &str,
    path: Option<&str>,
    frames: Option<&str>,
    settle: Option<&str>,
    hzb: bool,
    host: bool,
) -> Result<Config, Refusal> {
    if hzb {
        return Err(Refusal::Exclusive("BOYKO_HZB_DUMP"));
    }
    if host {
        return Err(Refusal::Exclusive("BOYKO_HOST_DUMP"));
    }
    let mut set = [false; IMAGE_DUMP_MAX];
    for word in names.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        match ImageDumpName::parse(word) {
            Some(n) => set[n as usize] = true,
            None => return Err(Refusal::UnknownName(word.to_owned())),
        }
    }
    if !set.iter().any(|&b| b) {
        return Err(Refusal::NoNames);
    }
    let dir = path.map(str::trim).filter(|p| !p.is_empty()).ok_or(Refusal::NoPath)?;
    Ok(Config { names: set, dir: dir.to_owned(), frames: parse_burst(frames), settle: parse_settle(settle) })
}

/// A captured frame whose staging is draining.
#[derive(Clone, Copy, Debug)]
struct Pending {
    copies: [ImageDumpCopy; IMAGE_DUMP_MAX],
    drain_left: u32,
}

/// The pure settle → burst → drain bookkeeping (`host_dump`'s `Schedule`, over this
/// instrument's per-frame metadata): [`request_slot`](Self::request_slot) once before the render
/// call, [`after_present`](Self::after_present) once after it.
#[derive(Debug)]
struct Schedule {
    total: u32,
    settle_left: u32,
    requested: u32,
    written: u32,
    carried: bool,
    pending: [Option<Pending>; RING],
}

impl Schedule {
    fn new(total: u32, settle: u32) -> Self {
        debug_assert!(total >= 1, "invariant: the burst length is clamped to 1..=MAX_BURST");
        Self { total, settle_left: settle, requested: 0, written: 0, carried: false, pending: [None; RING] }
    }

    /// The ring slot this frame's copies go into: `None` while settling or once every frame of
    /// the burst has been requested. An unpresented attempt re-requests the SAME slot.
    fn request_slot(&mut self) -> Option<usize> {
        if self.settle_left > 0 || self.requested >= self.total {
            return None;
        }
        let k = self.requested as usize % RING;
        debug_assert!(
            self.pending[k].is_none(),
            "invariant: a ring slot is never re-requested while its capture is still draining"
        );
        self.carried = true;
        Some(k)
    }

    /// Advances after a frame attempt; returns, per slot, the capture whose drain elapsed THIS
    /// frame. A capture at presented frame `k` is ready after frame `k + DRAIN_FRAMES` presents.
    fn after_present(
        &mut self,
        presented: bool,
        copies: [ImageDumpCopy; IMAGE_DUMP_MAX],
    ) -> [Option<[ImageDumpCopy; IMAGE_DUMP_MAX]>; RING] {
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
        for (slot, p) in self.pending.iter_mut().enumerate() {
            if let Some(pend) = p {
                pend.drain_left -= 1;
                if pend.drain_left == 0 {
                    ready[slot] = Some(pend.copies);
                    *p = None;
                    self.written += 1;
                }
            }
        }
        if self.carried {
            let k = self.requested as usize % RING;
            debug_assert!(self.pending[k].is_none(), "invariant: the requested slot was free");
            self.pending[k] = Some(Pending { copies, drain_left: DRAIN_FRAMES });
            self.requested += 1;
            self.carried = false;
        }
        ready
    }

    fn done(&self) -> bool {
        self.written == self.total
    }
}

/// The image-dump driver the frame loop threads through its steady path.
pub(crate) struct ImageDump {
    config: Config,
    schedule: Schedule,
    /// Per ring slot, one staging per requested name (`None` until the slot's first request).
    ring: [[Option<BoundBuffer>; IMAGE_DUMP_MAX]; RING],
    /// The reusable per-file path buffer.
    file_path: String,
    /// Files written so far (for the completion line).
    files: u32,
}

impl ImageDump {
    /// Arms the dump iff `BOYKO_IMAGE_DUMP` is set. `Err` is a named boot refusal (the caller
    /// reports it and ends the run before the first frame). Cold: called once before the loop.
    pub(crate) fn from_env() -> Result<Option<Self>, Refusal> {
        let Ok(names) = std::env::var("BOYKO_IMAGE_DUMP") else {
            return Ok(None);
        };
        let config = configure(
            &names,
            std::env::var("BOYKO_IMAGE_DUMP_PATH").ok().as_deref(),
            std::env::var("BOYKO_IMAGE_DUMP_FRAMES").ok().as_deref(),
            std::env::var("BOYKO_IMAGE_DUMP_SETTLE").ok().as_deref(),
            std::env::var_os("BOYKO_HZB_DUMP").is_some(),
            std::env::var_os("BOYKO_HOST_DUMP").is_some(),
        )?;
        boyko_log::info!(
            boyko_log::Host,
            "BOYKO_IMAGE_DUMP armed -> {} (frames={} settle={})",
            boyko_log::dsp!(config.dir, 192),
            config.frames,
            config.settle
        );
        let schedule = Schedule::new(config.frames, config.settle);
        Ok(Some(Self {
            config,
            schedule,
            ring: core::array::from_fn(|_| [const { None }; IMAGE_DUMP_MAX]),
            file_path: String::with_capacity(256),
            files: 0,
        }))
    }

    /// The per-frame request: `Some` on a capture frame, naming one staging per requested name
    /// (created on the slot's first request, sized for `extent` — the composite the targets are
    /// built at — at the name's staging texel size). `None` on every other frame.
    pub(crate) fn request(&mut self, ctx: &VulkanContext, extent: VkExtent2D) -> Option<ImageDumpRequest> {
        let k = self.schedule.request_slot()?;
        let mut request = ImageDumpRequest { staging: [VkBuffer::NULL; IMAGE_DUMP_MAX], capacity: [0; IMAGE_DUMP_MAX] };
        for name in ImageDumpName::ALL {
            let i = name as usize;
            if !self.config.names[i] {
                continue;
            }
            let size = u64::from(extent.width) * u64::from(extent.height) * name.staging_texel_bytes();
            let slot = &mut self.ring[k][i];
            if slot.as_ref().is_some_and(|b| b.size < size) {
                // SAFETY: the slot is not pending (`Schedule::request_slot` asserts it), so no
                // in-flight submission references its staging — its last capture was read after a
                // drain that re-waited the capturing frame's fence, or the frame that carried its
                // request recorded nothing (`Ok(false)` passes through `Renderer::recreate`'s
                // `device_wait_idle`). Created on `ctx`; destroyed exactly once (the `take`).
                unsafe {
                    RhiDevice::destroy_buffer(ctx, slot.take().expect("invariant: checked Some above"));
                }
            }
            if slot.is_none() {
                *slot = Some(
                    RhiDevice::create_buffer(
                        ctx,
                        &BufferDesc {
                            size,
                            usage: BufferUsage::TRANSFER_DST,
                            location: MemoryLocation::HostVisibleCoherent,
                        },
                    )
                    .expect("invariant: host-visible image-dump staging create"),
                );
            }
            let staging = slot.as_ref().expect("invariant: the staging exists after the create above");
            request.staging[i] = staging.buffer;
            request.capacity[i] = staging.size;
        }
        Some(request)
    }

    /// Advances after a frame attempt, writing every capture whose drain elapsed. Returns `true`
    /// once every frame of the burst is written.
    pub(crate) fn after_present(&mut self, ctx: &VulkanContext, presented: bool, renderer: &Renderer<'_>) -> bool {
        let copies = renderer.last_image_dump().unwrap_or([ImageDumpCopy::NONE; IMAGE_DUMP_MAX]);
        let ready = self.schedule.after_present(presented, copies);
        for (k, c) in ready.into_iter().enumerate() {
            if let Some(c) = c {
                self.write_slot(ctx, k, &c);
            }
        }
        self.schedule.done()
    }

    /// Writes slot `k`'s drained captures, one file per copied name; a requested name the frame
    /// did not copy (the path does not declare it, or nothing produced it) is logged, not written.
    #[cold]
    #[inline(never)]
    fn write_slot(&mut self, ctx: &VulkanContext, k: usize, copies: &[ImageDumpCopy; IMAGE_DUMP_MAX]) {
        for name in ImageDumpName::ALL {
            let i = name as usize;
            if !self.config.names[i] {
                continue;
            }
            let c = copies[i];
            if !c.copied {
                boyko_log::info!(
                    boyko_log::Host,
                    "image dump: `{}` was not copied on this capture (not declared on this path, or not produced)",
                    name.graph_name()
                );
                continue;
            }
            let staging = self.ring[k][i].as_ref().expect("invariant: a copied name's slot holds its staging");
            let texel = image_dump_texel_bytes(c.vk_format).expect("invariant: the recorder copies decodable formats only");
            assert!(
                c.bytes <= staging.size,
                "image dump `{}`: {} bytes exceed the {}-byte staging",
                name.graph_name(),
                c.bytes,
                staging.size
            );
            let mapped = RhiDevice::buffer_mapped_ptr(ctx, staging).expect("invariant: host-visible dump staging is mapped");
            // SAFETY: `mapped` points to `staging.size >= c.bytes` mapped host-coherent bytes
            // (asserted above). The capturing frame's fence was re-waited (`Schedule` releases a
            // slot only after `DRAIN_FRAMES > FRAMES_IN_FLIGHT` presented frames) and the copy
            // was followed by a TRANSFER→HOST barrier, so the write is complete and visible and no
            // submission still writes the buffer; the slice is read-only and dies in this call.
            let texels: &[u8] = unsafe { core::slice::from_raw_parts(mapped.as_ptr(), c.bytes as usize) };
            let header = header_bytes(&c, texel);
            self.file_path.clear();
            // `String` never fails `fmt::Write`.
            let _ = write!(self.file_path, "{}/{}_{}.bin", self.config.dir, name.graph_name(), c.frame_index);
            let written = std::fs::File::create(&self.file_path).and_then(|mut f| {
                f.write_all(&header)?;
                f.write_all(texels)
            });
            match written {
                Ok(()) => self.files += 1,
                Err(e) => {
                    let err = crate::diag::debug_into(&e);
                    crate::diag::report_dump_write_failed("image dump", &self.file_path, err.as_str());
                }
            }
        }
    }

    /// Destroys every staging of the ring (the burst is written).
    pub(crate) fn finish(mut self, ctx: &VulkanContext) {
        for slot in self.ring.iter_mut().flatten() {
            if let Some(staging) = slot.take() {
                // SAFETY: created on `ctx`; every submission that referenced it completed (each
                // capture's fence was re-waited per the drain before its write, and no request is
                // outstanding once the machine is done); destroyed exactly once (the `take`).
                unsafe { RhiDevice::destroy_buffer(ctx, staging) };
            }
        }
        boyko_log::info!(
            boyko_log::Host,
            "image dump written -> {} ({} file(s))",
            boyko_log::dsp!(self.config.dir, 192),
            self.files
        );
    }
}

/// The 32-byte file header of one copy.
pub(crate) fn header_bytes(c: &ImageDumpCopy, texel_bytes: u32) -> [u8; HEADER_BYTES] {
    let words = [
        MAGIC,
        c.vk_format as u32,
        c.width,
        c.height,
        c.mip,
        c.layers,
        c.frame_index,
        texel_bytes,
    ];
    let mut out = [0u8; HEADER_BYTES];
    for (chunk, w) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        *chunk = w.to_le_bytes();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copied(frame: u32) -> [ImageDumpCopy; IMAGE_DUMP_MAX] {
        let mut c = [ImageDumpCopy::NONE; IMAGE_DUMP_MAX];
        c[ImageDumpName::Lit as usize] = ImageDumpCopy { copied: true, frame_index: frame, ..ImageDumpCopy::NONE };
        c
    }

    #[test]
    fn names_parse_and_refusals_are_named() {
        let c = configure("lit, depth", Some("D:/x"), None, None, false, false).expect("valid");
        assert!(c.names[ImageDumpName::Lit as usize] && c.names[ImageDumpName::DeferredDepth as usize]);
        assert_eq!((c.frames, c.settle), (1, 30));
        assert_eq!(configure("lit,nope", Some("D:/x"), None, None, false, false), Err(Refusal::UnknownName("nope".to_owned())));
        assert_eq!(configure(" , ", Some("D:/x"), None, None, false, false), Err(Refusal::NoNames));
        assert_eq!(configure("lit", None, None, None, false, false), Err(Refusal::NoPath));
        assert_eq!(configure("lit", Some("D:/x"), None, None, true, false), Err(Refusal::Exclusive("BOYKO_HZB_DUMP")));
        assert_eq!(configure("lit", Some("D:/x"), None, None, false, true), Err(Refusal::Exclusive("BOYKO_HOST_DUMP")));
        assert_eq!(configure("taa_hist", Some("D:/x"), Some("4"), Some("0"), false, false).map(|c| (c.frames, c.settle)), Ok((4, 0)));
    }

    /// A burst of 6 over a 2-frame settle: each capture is written exactly `DRAIN_FRAMES`
    /// presented frames after it, slots never alias, and a skipped frame re-requests its slot.
    #[test]
    fn the_ring_drains_every_capture_once_and_never_aliases() {
        let mut s = Schedule::new(6, 2);
        let mut written = Vec::new();
        let mut frame = 0u32;
        let mut attempts = 0;
        while !s.done() {
            attempts += 1;
            assert!(attempts < 100, "the schedule never completed");
            let slot = s.request_slot();
            // Skip one attempt mid-burst: nothing may advance.
            let presented = attempts != 5;
            for (k, r) in s.after_present(presented, copied(frame)).into_iter().enumerate() {
                if let Some(c) = r {
                    written.push((k, c[ImageDumpName::Lit as usize].frame_index, frame));
                }
            }
            if let Some(k) = slot
                && presented
            {
                assert!(k < RING);
            }
            if presented {
                frame += 1;
            }
        }
        let frames: Vec<u32> = written.iter().map(|w| w.1).collect();
        assert_eq!(frames, [2, 3, 4, 5, 6, 7], "every captured frame once, in order");
        for (_, captured, at) in &written {
            assert_eq!(*at, captured + DRAIN_FRAMES, "a capture is read DRAIN_FRAMES presented frames later");
        }
    }

    /// The image-dump clause of `render_gbuffer_frame` (OBS review W1) rests on a staging slot being
    /// reused only after its frame's fence was re-waited: the drain must outlast the frames in flight.
    #[test]
    fn the_drain_outlasts_the_frames_in_flight() {
        assert!(
            DRAIN_FRAMES as usize > boyko_rhi_vulkan::present::FRAMES_IN_FLIGHT,
            "a staging slot would be reused while its frame is still in flight"
        );
    }

    #[test]
    fn the_header_carries_the_recorded_fields() {
        let c = ImageDumpCopy {
            copied: true,
            vk_format: 97,
            width: 3,
            height: 2,
            mip: 0,
            layers: 1,
            frame_index: 30,
            bytes: 48,
        };
        let h = header_bytes(&c, 8);
        let w = |i: usize| u32::from_le_bytes(h[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        assert_eq!([w(0), w(1), w(2), w(3), w(4), w(5), w(6), w(7)], [MAGIC, 97, 3, 2, 0, 1, 30, 8]);
    }
}
