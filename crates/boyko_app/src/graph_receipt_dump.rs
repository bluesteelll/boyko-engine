//! **OBS (R) — the production frame-graph receipt** (`BOYKO_GRAPH_RECEIPT=<path.toml>`;
//! render-wave plan H0 Scope 5).
//!
//! The `vb_probe_dump` idiom: a settle window, then the receipt is written from the first
//! PRESENTED frame after it, and the driver reports ready so the frame loop can exit. Nothing is
//! copied off the device — every number is host memory the production declarator and the armed
//! [`ReceiptCapture`] hold once `render_gbuffer_frame` returns — so there is no staging and no drain.
//!
//! # What the file holds
//!
//! - `[config]`: the single-predicate values the declarator and the recorder both read, captured
//!   inside `boyko_rhi_vulkan` (`Renderer::last_receipt_capture`; the predicates are `pub(crate)`).
//! - `[counts]`: resources, the mip-weighted `res_state_total`, passes, accesses, beside
//!   [`FRAME_GRAPH_CAPACITY`] — a frame past the named capacity is a cold regrow.
//! - `[[resource]]`: index, name, kind, mips, the RAW `seeded` bit and every seed field. `class`
//!   is production's derived label, written for a human reader only: a census re-derives the class
//!   from `seeded` and the seed, independently of `SeedClass::derive`.
//! - `[[pass]]` / `[[access]]` / `[[barrier]]`: the declared passes, their accesses in order and
//!   the barriers `compile` derived for each.
//! - `[[sink]]`: every slot of the dispatched declarator's sink fill — its name, its `Fif` column
//!   and the graph name of the `ResId` it resolves. **Debug builds only**: the slot metadata does
//!   not exist in a release build, which writes `sink_rows = false` and no row, so a reader can
//!   tell "no rows" from "clean".
//! - `[[frame]]`: one compact record per presented frame of the burst
//!   (`BOYKO_GRAPH_RECEIPT_FRAMES=N`, default 1, at most [`MAX_BURST`]) — its presented-frame
//!   ordinal, `submission_epoch`, the whole `[config]` (`cfg_<key>`) and the declared pass names
//!   in order. The first record is the full receipt's own frame.
//!
//! A run with any other dump instrument writes the receipt on that instrument's frames when the
//! two settle windows agree (`BOYKO_GRAPH_RECEIPT_SETTLE`, default 30 — the `host_dump` value).

use std::fs::File;
use std::io::{BufWriter, Write};

use boyko_rhi_vulkan::framegraph::{FrameGraph, PassId, ResId};
use boyko_rhi_vulkan::present::seed_class::declarator_name;
use boyko_rhi_vulkan::present::{
    FRAME_GRAPH_CAPACITY, RECEIPT_CONFIG_KEYS, ReceiptCapture, ReceiptConfig, Renderer,
};

/// Presented frames rendered before the receipt frame — the `host_dump` settle window.
pub(crate) const DEFAULT_SETTLE_FRAMES: u32 = 30;

/// The longest burst — `host_dump`'s `MAX_BURST`.
pub(crate) const MAX_BURST: u32 = 256;

/// The most pass names one `[[frame]]` record keeps (above [`FRAME_GRAPH_CAPACITY`]'s `max_pass` —
/// twice OBS's 64, one third over render wave PX0's 96, asserted below);
/// a frame declaring more is recorded with `pass_count` past the kept names, so the truncation is
/// visible in the file.
pub(crate) const MAX_RECORD_PASSES: usize = 128;

const _: () = assert!(
    MAX_RECORD_PASSES >= FRAME_GRAPH_CAPACITY.max_pass,
    "a [[frame]] record must keep every pass name a capacity-sized frame declares"
);

/// The receipt's schema. Bumped whenever a key changes meaning, so a stale parser fails loudly.
const SCHEMA_VERSION: u32 = 1;

/// One `[[frame]]` record: POD, so a burst costs one copy per frame into a boot-sized buffer.
#[derive(Clone, Copy)]
struct FrameRecord {
    ordinal: u64,
    epoch: u64,
    config: ReceiptConfig,
    pass_count: usize,
    passes: [&'static str; MAX_RECORD_PASSES],
}

/// The settle → capture progression. `Settle` counts REMAINING presented frames.
enum State {
    Settle(u32),
    Capture,
}

/// The receipt driver the frame loop threads through its steady path.
pub(crate) struct GraphReceiptDump {
    /// Destination path (the `BOYKO_GRAPH_RECEIPT` value).
    path: String,
    /// Burst length `N` (≥ 1).
    frames: u32,
    state: State,
    /// Presented frames seen so far — the records' ordinal.
    presented: u64,
    /// Opened on the receipt frame, which writes everything but the `[[frame]]` records.
    out: Option<BufWriter<File>>,
    /// The burst's records, allocated once at arm time with capacity `N`.
    records: Vec<FrameRecord>,
    /// Set by a failed write: the driver reports ready (so the run exits) and writes nothing more.
    failed: bool,
}

/// Parses a burst length (`None` = unset ⇒ 1); an out-of-range or malformed value is clamped or
/// defaulted, never a boot failure — a diagnostic knob must not stop the engine.
pub(crate) fn parse_burst(raw: Option<&str>) -> u32 {
    raw.and_then(|r| r.trim().parse::<u32>().ok()).map_or(1, |n| n.clamp(1, MAX_BURST))
}

/// Parses a settle window (`None` or malformed ⇒ [`DEFAULT_SETTLE_FRAMES`]).
pub(crate) fn parse_settle(raw: Option<&str>) -> u32 {
    raw.and_then(|r| r.trim().parse::<u32>().ok()).unwrap_or(DEFAULT_SETTLE_FRAMES)
}

impl GraphReceiptDump {
    /// Arms the receipt iff `BOYKO_GRAPH_RECEIPT` is set (the value is the output path). Cold:
    /// called once before the frame loop; the caller then arms the renderer's capture.
    pub(crate) fn from_env() -> Option<Self> {
        let path = std::env::var("BOYKO_GRAPH_RECEIPT").ok()?;
        let frames = parse_burst(std::env::var("BOYKO_GRAPH_RECEIPT_FRAMES").ok().as_deref());
        let settle = parse_settle(std::env::var("BOYKO_GRAPH_RECEIPT_SETTLE").ok().as_deref());
        boyko_log::info!(
            boyko_log::Host,
            "BOYKO_GRAPH_RECEIPT armed -> {} (frames={} settle={})",
            boyko_log::dsp!(path, 192),
            frames,
            settle
        );
        Some(Self {
            path,
            frames,
            state: State::Settle(settle),
            presented: 0,
            out: None,
            records: Vec::with_capacity(frames as usize),
            failed: false,
        })
    }

    /// Advances after a present. Returns `true` once the receipt and every burst record are
    /// written (or a write failed). A frame that did not present advances nothing: it may never
    /// have reached the record body, so the capture would describe an older frame.
    pub(crate) fn after_present(&mut self, presented: bool, renderer: &Renderer<'_>) -> bool {
        if !presented {
            return false;
        }
        let ordinal = self.presented;
        self.presented += 1;
        if let State::Settle(n) = self.state {
            if n > 0 {
                self.state = State::Settle(n - 1);
                return false;
            }
            self.state = State::Capture;
        }
        let Some(cap) = renderer.last_receipt_capture() else {
            // The runner arms the renderer in the same statement that builds this driver.
            return self.fail(&"the renderer's receipt capture is not armed");
        };
        let g = renderer.last_frame_graph();
        if self.out.is_none() {
            match File::create(&self.path) {
                Ok(f) => {
                    let mut w = BufWriter::new(f);
                    if let Err(e) = write_receipt(&mut w, g, cap, ordinal) {
                        return self.fail(&e);
                    }
                    self.out = Some(w);
                }
                Err(e) => return self.fail(&e),
            }
        }
        self.records.push(record(g, cap, ordinal, renderer.submission_epoch()));
        if self.records.len() < self.frames as usize {
            return false;
        }
        let Some(mut w) = self.out.take() else {
            return self.fail(&"the receipt writer was not opened");
        };
        let written = self.records.iter().try_for_each(|r| write_frame(&mut w, r)).and_then(|()| w.flush());
        if let Err(e) = written {
            return self.fail(&e);
        }
        true
    }

    #[cold]
    #[inline(never)]
    fn fail<E: core::fmt::Debug + ?Sized>(&mut self, e: &E) -> bool {
        if !self.failed {
            self.failed = true;
            let err = crate::diag::debug_into(e);
            crate::diag::report_dump_write_failed("graph receipt", &self.path, err.as_str());
        }
        true
    }

    /// Ends the driver. Consumes it, like its siblings, so a second write is not expressible.
    pub(crate) fn finish(self) {
        if !self.failed {
            boyko_log::info!(
                boyko_log::Host,
                "graph receipt written -> {} ({} frame record(s))",
                boyko_log::dsp!(self.path, 192),
                self.records.len()
            );
        }
    }
}

fn record(g: &FrameGraph, cap: &ReceiptCapture, ordinal: u64, epoch: u64) -> FrameRecord {
    let mut passes = [""; MAX_RECORD_PASSES];
    let pass_count = g.pass_count();
    for (p, slot) in passes.iter_mut().enumerate().take(pass_count) {
        *slot = g.pass_name(PassId(p as u16));
    }
    FrameRecord { ordinal, epoch, config: cap.config, pass_count, passes }
}

fn bool_word(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

/// Writes everything but the `[[frame]]` records, from the receipt frame's graph and capture.
fn write_receipt(
    w: &mut impl Write,
    g: &FrameGraph,
    cap: &ReceiptCapture,
    ordinal: u64,
) -> std::io::Result<()> {
    writeln!(w, "# OBS graph receipt -- MACHINE-WRITTEN by boyko_app::graph_receipt_dump (H0 Scope 5).")?;
    writeln!(w, "# [[resource]] carries the RAW `seeded` bit and the seed; `class` is production's")?;
    writeln!(w, "# derived label, for reading only -- a census re-derives the class itself.")?;
    writeln!(w, "# [[sink]] rows exist only in a debug build (`sink_rows = false` otherwise).")?;
    writeln!(w, "schema_version = {SCHEMA_VERSION}")?;
    writeln!(w, "build = \"{}\"", if cfg!(debug_assertions) { "debug" } else { "release" })?;
    writeln!(w, "sink_rows = {}", bool_word(cap.sink_rows))?;
    writeln!(w, "declarator = \"{}\"", declarator_name(cap.declarator))?;
    writeln!(w, "frames_captured = {}", cap.frames)?;
    writeln!(w, "ordinal = {ordinal}")?;

    writeln!(w, "\n[config]")?;
    for (k, v) in cap.config.iter() {
        writeln!(w, "{k} = {v}")?;
    }

    let accesses: usize = (0..g.pass_count()).map(|p| g.pass_accesses(PassId(p as u16)).len()).sum();
    writeln!(w, "\n[counts]")?;
    writeln!(w, "resources = {}", g.res_count())?;
    writeln!(w, "res_state_total = {}", g.res_state_total())?;
    writeln!(w, "passes = {}", g.pass_count())?;
    writeln!(w, "accesses = {accesses}")?;
    writeln!(w, "img_barriers = {}", g.img_barriers().len())?;
    writeln!(w, "buf_barriers = {}", g.buf_barriers().len())?;
    writeln!(w, "max_res = {}", FRAME_GRAPH_CAPACITY.max_res)?;
    writeln!(w, "max_pass = {}", FRAME_GRAPH_CAPACITY.max_pass)?;
    writeln!(w, "max_acc = {}", FRAME_GRAPH_CAPACITY.max_acc)?;

    for i in 0..g.res_count() {
        let res = ResId(i as u16);
        let seed = g.res_seed(res);
        writeln!(w, "\n[[resource]]")?;
        writeln!(w, "index = {i}")?;
        writeln!(w, "name = \"{}\"", g.res_name(res))?;
        writeln!(w, "kind = \"{}\"", if g.res_is_image(res) { "image" } else { "buffer" })?;
        writeln!(w, "mips = {}", g.res_mips(res))?;
        writeln!(w, "seeded = {}", bool_word(g.res_seeded(res)))?;
        writeln!(w, "seed_layout = {}", seed.layout)?;
        writeln!(w, "seed_flush_stages = {}", seed.flush_stages)?;
        writeln!(w, "seed_flush_access = {}", seed.flush_access)?;
        writeln!(w, "seed_visible_stages = {}", seed.visible_stages)?;
        writeln!(w, "seed_visible_access = {}", seed.visible_access)?;
        writeln!(w, "class = \"{}\"", g.res_class(res).name())?;
    }

    for (p, range) in g.pass_barriers().iter().enumerate() {
        let pass = PassId(p as u16);
        writeln!(w, "\n[[pass]]")?;
        writeln!(w, "index = {p}")?;
        writeln!(w, "name = \"{}\"", g.pass_name(pass))?;
        writeln!(w, "accesses = {}", g.pass_accesses(pass).len())?;
        writeln!(w, "img_barriers = {}", range.img_count)?;
        writeln!(w, "buf_barriers = {}", range.buf_count)?;
        for a in g.pass_accesses(pass) {
            writeln!(w, "\n[[access]]")?;
            writeln!(w, "pass = {p}")?;
            writeln!(w, "res = {}", a.res.0)?;
            writeln!(w, "stage = {}", a.stage)?;
            writeln!(w, "access = {}", a.access)?;
            writeln!(w, "layout = {}", a.layout)?;
            writeln!(w, "base_mip = {}", a.sub.base_mip)?;
            writeln!(w, "mip_count = {}", a.sub.mip_count)?;
        }
        let img = &g.img_barriers()[range.img_begin as usize..(range.img_begin + range.img_count) as usize];
        for b in img {
            writeln!(w, "\n[[barrier]]")?;
            writeln!(w, "pass = {p}")?;
            writeln!(w, "kind = \"image\"")?;
            writeln!(w, "res = {}", b.res.0)?;
            writeln!(w, "src_stage = {}", b.src_stage)?;
            writeln!(w, "dst_stage = {}", b.dst_stage)?;
            writeln!(w, "src_access = {}", b.src_access)?;
            writeln!(w, "dst_access = {}", b.dst_access)?;
            writeln!(w, "old_layout = {}", b.old_layout)?;
            writeln!(w, "new_layout = {}", b.new_layout)?;
            writeln!(w, "base_mip = {}", b.subresource.base_mip)?;
            writeln!(w, "mip_count = {}", b.subresource.mip_count)?;
        }
        let buf = &g.buf_barriers()[range.buf_begin as usize..(range.buf_begin + range.buf_count) as usize];
        for b in buf {
            writeln!(w, "\n[[barrier]]")?;
            writeln!(w, "pass = {p}")?;
            writeln!(w, "kind = \"buffer\"")?;
            writeln!(w, "res = {}", b.res.0)?;
            writeln!(w, "src_stage = {}", b.src_stage)?;
            writeln!(w, "dst_stage = {}", b.dst_stage)?;
            writeln!(w, "src_access = {}", b.src_access)?;
            writeln!(w, "dst_access = {}", b.dst_access)?;
        }
    }

    if cap.sink_rows {
        let images = &cap.image_sinks[..usize::from(cap.image_slots)];
        let buffers = &cap.buffer_sinks[..usize::from(cap.buffer_slots)];
        let rows = images.iter().enumerate().map(|(s, m)| ("image", s, s, m)).chain(
            buffers.iter().enumerate().map(|(s, m)| ("buffer", s, usize::from(cap.image_slots) + s, m)),
        );
        for (kind, slot, res, meta) in rows {
            let Some(meta) = meta else {
                continue;
            };
            let graph_name = if res < g.res_count() { g.res_name(ResId(res as u16)) } else { "" };
            writeln!(w, "\n[[sink]]")?;
            writeln!(w, "declarator = \"{}\"", declarator_name(cap.declarator))?;
            writeln!(w, "kind = \"{kind}\"")?;
            writeln!(w, "slot = {slot}")?;
            writeln!(w, "sink_name = \"{}\"", meta.name)?;
            writeln!(w, "fif = \"{}\"", meta.fif.name())?;
            writeln!(w, "res = {res}")?;
            writeln!(w, "graph_name = \"{graph_name}\"")?;
        }
    }
    Ok(())
}

/// Writes one `[[frame]]` record.
fn write_frame(w: &mut impl Write, r: &FrameRecord) -> std::io::Result<()> {
    writeln!(w, "\n[[frame]]")?;
    writeln!(w, "ordinal = {}", r.ordinal)?;
    writeln!(w, "epoch = {}", r.epoch)?;
    writeln!(w, "pass_count = {}", r.pass_count)?;
    write!(w, "passes = \"")?;
    for (i, name) in r.passes.iter().take(r.pass_count.min(MAX_RECORD_PASSES)).enumerate() {
        if i > 0 {
            write!(w, ",")?;
        }
        write!(w, "{name}")?;
    }
    writeln!(w, "\"")?;
    for (k, v) in RECEIPT_CONFIG_KEYS.iter().zip(r.config.values.iter()) {
        writeln!(w, "cfg_{k} = {v}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_and_settle_parse_with_defaults_and_clamps() {
        assert_eq!(parse_burst(None), 1);
        assert_eq!(parse_burst(Some("16")), 16);
        assert_eq!(parse_burst(Some(" 4 ")), 4);
        assert_eq!(parse_burst(Some("0")), 1);
        assert_eq!(parse_burst(Some("100000")), MAX_BURST);
        assert_eq!(parse_burst(Some("many")), 1);
        assert_eq!(parse_settle(None), DEFAULT_SETTLE_FRAMES);
        assert_eq!(parse_settle(Some("0")), 0);
        assert_eq!(parse_settle(Some("x")), DEFAULT_SETTLE_FRAMES);
    }
}
