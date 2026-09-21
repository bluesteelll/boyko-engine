//! The shadow-map POISON knob — `BOYKO_SHADOW_POISON=<depth>` and
//! `BOYKO_SHADOW_POISON_PROBE=<path.toml>` — the diagnostic seam of
//! `tests/unwritten_shadow_map_gate.rs`.
//!
//! # What it is for
//!
//! A frame that samples a shadow layer no pass wrote renders whatever that memory holds. On the
//! development machine that memory happened to hold values that looked plausible, so the defect
//! it caused (a mesh-less `VB × Sdf` boot sampling a cascade nothing had rendered) passed every
//! golden but one, and that one only because a lane moved the bytes. The knob makes such a read
//! OBSERVABLE: fill every never-written layer with a chosen depth, render twice with two
//! different fills, and compare. A frame that depends on the fill sampled memory no pass wrote.
//!
//! # The two variables
//!
//! * **`BOYKO_SHADOW_POISON=<depth>`** — after the boot layout seed, every layer of the CSM
//!   cascade array and of the punctual shadow atlas is cleared to `<depth>` by a CLEAR-only
//!   dynamic-rendering scope per layer (`VulkanCommandEncoder::clear_depth_layers`), which needs
//!   no `TRANSFER_DST` usage. Both images additionally carry `TRANSFER_SRC` while the knob is set,
//!   so the probe below can read them back; every run of the gate sets the knob, so the usage is
//!   the same across the runs it compares. `<depth>` must parse as an `f32` inside `[0, 1]`, the
//!   D32 clear range.
//! * **`BOYKO_SHADOW_POISON_PROBE=<path>`** — on the frame the `BOYKO_HOST_DUMP` capture
//!   completes (the same settle → request → drain count, [`crate::hzb_dump::SETTLE_FRAMES`] +
//!   1 + [`crate::hzb_dump::DRAIN_FRAMES`] presented frames), the device is idled and the CENTRE
//!   texel of every layer of both maps is copied back, beside the host's own view of the frame
//!   stream (the resolved path and legs, the armed-frame counters, header word 7, the slotted
//!   light rows and the rows the shader would sample the atlas for, the active cascade / atlas
//!   counts). One TOML file; the probe is a loop-exit
//!   driver like `VbCullProbe`, so the run ends once it and every other armed capture finished.
//!
//! The centre texels are what let the gate prove its poison REACHED the maps: on a boot that
//! renders neither map they must equal the poison's bits under two different poisons, which no
//! dead poison and no dead readback can satisfy at once.
//!
//! # Fail loud, at boot
//!
//! A value that does not parse, lies outside `[0, 1]` (or is NaN), or a probe path set without
//! the poison, panics at boot — each would otherwise produce a run that looks armed and proves
//! nothing.
//!
//! Entirely cold: without the variables nothing is created, no usage bit changes and the frame
//! loop pays one `Option` check per frame.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::hzb_dump::{DRAIN_FRAMES, SETTLE_FRAMES};

/// The poison variable: the depth every shadow-map layer is cleared to at boot.
pub(crate) const POISON_VAR: &str = "BOYKO_SHADOW_POISON";
/// The probe variable: where the readback TOML is written.
pub(crate) const PROBE_VAR: &str = "BOYKO_SHADOW_POISON_PROBE";

/// The parsed knob — read once, at scene boot, by `CsmResources::create`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShadowPoison {
    /// `BOYKO_SHADOW_POISON`, asserted inside `[0, 1]` (the D32 clear range).
    depth: f32,
    /// `BOYKO_SHADOW_POISON_PROBE`; setting it without the poison panics.
    probe: Option<PathBuf>,
}

impl ShadowPoison {
    /// Reads both variables. `None` when the poison is unset (the steady path).
    ///
    /// # Panics
    ///
    /// On an unparseable or out-of-range poison, and on a probe path set without a poison.
    #[cold]
    #[inline(never)]
    pub(crate) fn from_env() -> Option<Self> {
        let poison = std::env::var(POISON_VAR).ok();
        let knob = Self::parse(poison.as_deref(), std::env::var_os(PROBE_VAR));
        if let Some(k) = &knob {
            boyko_log::info!(
                boyko_log::Host,
                "{} armed: every shadow-map layer cleared to {} at boot (probe: {})",
                POISON_VAR,
                k.depth,
                k.probe.is_some()
            );
        }
        knob
    }

    /// The pure half of [`Self::from_env`], separated so the fail-loud rules are testable
    /// without touching the process environment.
    fn parse(poison: Option<&str>, probe: Option<OsString>) -> Option<Self> {
        let Some(raw) = poison else {
            assert!(
                probe.is_none(),
                "{PROBE_VAR} is set but {POISON_VAR} is not: the probe would read back maps that \
                 were never poisoned, and its texels would prove nothing"
            );
            return None;
        };
        let depth: f32 = raw.trim().parse().unwrap_or_else(|e| {
            panic!("{POISON_VAR}={raw:?} is not an f32 ({e}); the poison run would be unarmed")
        });
        assert!(
            (0.0..=1.0).contains(&depth),
            "{POISON_VAR}={depth} lies outside the D32 clear range [0, 1]"
        );
        Some(Self { depth, probe: probe.map(PathBuf::from) })
    }

    /// The depth every shadow-map layer is cleared to.
    pub(crate) fn depth(&self) -> f32 {
        self.depth
    }

    /// The probe's output path, when the probe is armed.
    pub(crate) fn probe(&self) -> Option<&Path> {
        self.probe.as_deref()
    }
}

/// Everything one probe record carries — the host's view of the captured frame stream beside
/// the texels, so the gate can check that the run it reads is the run it asked for.
pub(crate) struct ShadowProbeRecord<'a> {
    /// The RESOLVED render path (`Debug` spelling), read off the boot carrier.
    pub(crate) path: &'a str,
    /// The RESOLVED geometry legs (`Debug` spelling).
    pub(crate) legs: &'a str,
    /// `ResolvedRenderPath::mesh_leg`.
    pub(crate) mesh_leg: bool,
    /// Presented frames the driver counted.
    pub(crate) presented_frames: u32,
    /// `HostFrameStats::frames` at the capture (published one step later than the counters it
    /// sits beside, so it trails `presented_frames` by the capture frame itself).
    pub(crate) frames: u64,
    /// `HostFrameStats::csm_armed_frames`.
    pub(crate) csm_armed_frames: u64,
    /// `HostFrameStats::punctual_armed_frames`.
    pub(crate) punctual_armed_frames: u64,
    /// Word 7 of the STAGED light-table header — the shadow gate word (bit 2 CSM, bit 3 punctual).
    pub(crate) header_word7: u32,
    /// Staged point/spot rows whose kind word carries `CASTS_SHADOW_BIT`, i.e. a real atlas slot.
    pub(crate) slotted_rows: u32,
    /// Staged point/spot rows the shader WOULD sample the atlas for on an armed frame: its own
    /// predicate, `light_atlas_slot(kind) != SLOT_NONE`, which does not read `CASTS_SHADOW_BIT`.
    /// Equal to `slotted_rows` exactly when every un-slotted row carries the `SLOT_NONE` field.
    pub(crate) sampled_rows: u32,
    /// `ResolvedCsm::active_count` at the capture.
    pub(crate) csm_active_count: u32,
    /// `ResolvedShadowAtlas::active_layers` at the capture.
    pub(crate) atlas_active_layers: u32,
    /// The poison's own bits (`f32::to_bits`).
    pub(crate) poison_bits: u32,
    /// The cascade layers' centre texels.
    pub(crate) cascade_center_bits: &'a [u32],
    /// The atlas layers' centre texels.
    pub(crate) atlas_center_bits: &'a [u32],
}

/// Reads header word 7 and counts two kinds of punctual row off the STAGED light table — the
/// bytes the next upload would ship — returning `(word7, slotted, sampled)`. The layout is
/// `LightHeaderGpu` (16 words, word 7 = `sky_diffuse.w`) followed by 12-word `GpuLight` rows whose
/// word 3 is the kind word.
///
/// * `slotted` — rows carrying `CASTS_SHADOW_BIT`, i.e. packed with a real slot. The directional
///   and sky rows never carry it, so counting over every row counts exactly those.
/// * `sampled` — point/spot rows whose slot field is not `SLOT_NONE`: the shader's own predicate
///   (header bit 3, then `light_atlas_slot(kind) != SLOT_NONE`). The kind filter is required,
///   because the directional and sky rows keep a `0` field that nothing reads.
pub(crate) fn staged_shadow_words(bytes: &[u8]) -> (u32, u32, u32) {
    const HEADER_BYTES: usize = 64;
    const ROW_BYTES: usize = 48;
    let word = |off: usize| -> u32 {
        u32::from_le_bytes(
            bytes[off..off + 4]
                .try_into()
                .expect("invariant: a four-byte slice converts to a word"),
        )
    };
    let header_word7 = word(7 * 4);
    let rows = bytes.len().saturating_sub(HEADER_BYTES) / ROW_BYTES;
    let kind_word = |r: usize| word(HEADER_BYTES + r * ROW_BYTES + 3 * 4);
    let slotted = (0..rows)
        .filter(|&r| kind_word(r) & boyko_render::CASTS_SHADOW_BIT != 0)
        .count();
    let sampled = (0..rows)
        .filter(|&r| {
            let k = kind_word(r);
            let kind = k & 0xFFFF;
            (kind == boyko_render::LIGHT_KIND_POINT || kind == boyko_render::LIGHT_KIND_SPOT)
                && boyko_render::light_atlas_slot(k) != boyko_render::SLOT_NONE
        })
        .count();
    let count = |n: usize| u32::try_from(n).expect("invariant: the light table holds < 2^32 rows");
    (header_word7, count(slotted), count(sampled))
}

/// Renders a record as the probe's TOML. Integers only (bits in hex), so the reader needs no
/// float parsing and no rounding can make two runs look equal.
pub(crate) fn format_record(r: &ShadowProbeRecord<'_>) -> String {
    let hex_list = |v: &[u32]| {
        let items: Vec<String> = v.iter().map(|b| format!("0x{b:08x}")).collect();
        format!("[{}]", items.join(", "))
    };
    format!(
        "# {PROBE_VAR} record, written by boyko_app::shadow_poison and read by\n\
         # crates/boyko_app/tests/unwritten_shadow_map_gate.rs\n\
         path = \"{}\"\n\
         legs = \"{}\"\n\
         mesh_leg = {}\n\
         presented_frames = {}\n\
         frames = {}\n\
         csm_armed_frames = {}\n\
         punctual_armed_frames = {}\n\
         header_word7 = 0x{:08x}\n\
         slotted_rows = {}\n\
         sampled_rows = {}\n\
         csm_active_count = {}\n\
         atlas_active_layers = {}\n\
         poison_bits = 0x{:08x}\n\
         cascade_center_bits = {}\n\
         atlas_center_bits = {}\n",
        r.path,
        r.legs,
        r.mesh_leg,
        r.presented_frames,
        r.frames,
        r.csm_armed_frames,
        r.punctual_armed_frames,
        r.header_word7,
        r.slotted_rows,
        r.sampled_rows,
        r.csm_active_count,
        r.atlas_active_layers,
        r.poison_bits,
        hex_list(r.cascade_center_bits),
        hex_list(r.atlas_center_bits),
    )
}

/// The probe's loop-exit driver: counts presented frames to the frame the `BOYKO_HOST_DUMP`
/// capture completes on, then reports ready once.
pub(crate) struct ShadowPoisonProbe {
    path: PathBuf,
    /// Presented frames left before the probe fires.
    remaining: u32,
    /// Presented frames counted so far.
    presented: u32,
}

impl ShadowPoisonProbe {
    /// Arms the driver for `path`.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, remaining: SETTLE_FRAMES + 1 + DRAIN_FRAMES, presented: 0 }
    }

    /// Advances after a frame attempt (`presented == true` iff the frame presented). Returns
    /// `true` exactly once, on the frame the capture is due.
    pub(crate) fn after_present(&mut self, presented: bool) -> bool {
        if !presented || self.remaining == 0 {
            return false;
        }
        self.presented += 1;
        self.remaining -= 1;
        self.remaining == 0
    }

    /// Presented frames counted so far.
    pub(crate) fn presented(&self) -> u32 {
        self.presented
    }

    /// Writes the record, consuming the driver (the probe is one-shot).
    ///
    /// # Panics
    ///
    /// If the file cannot be written: the gate reads ONLY this file, so a run that ended without
    /// it must not look like a run that completed.
    #[cold]
    #[inline(never)]
    pub(crate) fn finish(self, record: &ShadowProbeRecord<'_>) {
        let text = format_record(record);
        let written = std::fs::File::create(&self.path).and_then(|mut f| f.write_all(text.as_bytes()));
        if let Err(e) = written {
            panic!("{PROBE_VAR}: cannot write {} ({e})", self.path.display());
        }
        boyko_log::info!(
            boyko_log::Host,
            "shadow poison probe written -> {}",
            boyko_log::dsp!(self.path.display().to_string(), 192)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_is_the_steady_path() {
        assert_eq!(ShadowPoison::parse(None, None), None);
    }

    #[test]
    fn a_poison_and_a_probe_parse() {
        let k = ShadowPoison::parse(Some(" 0.25 "), Some(OsString::from("x.toml")))
            .expect("a valid poison arms the knob");
        assert_eq!(k.depth(), 0.25);
        assert_eq!(k.probe(), Some(Path::new("x.toml")));
    }

    #[test]
    #[should_panic(expected = "BOYKO_SHADOW_POISON_PROBE is set but BOYKO_SHADOW_POISON is not")]
    fn a_probe_without_a_poison_panics() {
        let _ = ShadowPoison::parse(None, Some(OsString::from("x.toml")));
    }

    #[test]
    #[should_panic(expected = "not an f32")]
    fn an_unparseable_poison_panics() {
        let _ = ShadowPoison::parse(Some("deep"), None);
    }

    #[test]
    #[should_panic(expected = "outside the D32 clear range")]
    fn an_out_of_range_poison_panics() {
        let _ = ShadowPoison::parse(Some("1.5"), None);
    }

    #[test]
    fn the_driver_fires_once_on_the_dump_frame() {
        let mut p = ShadowPoisonProbe::new(PathBuf::from("x.toml"));
        let due = SETTLE_FRAMES + 1 + DRAIN_FRAMES;
        for _ in 1..due {
            assert!(!p.after_present(true));
            assert!(!p.after_present(false), "a skipped frame is not counted");
        }
        assert!(p.after_present(true));
        assert_eq!(p.presented(), due);
        assert!(!p.after_present(true), "one-shot");
    }

    #[test]
    fn staged_words_read_word7_and_count_slotted_and_sampled_rows() {
        // Row 0 stays zero: a directional row, whose `0` slot field no shader reads.
        let rows = [
            0,
            // A spot packed at slot 0: slotted, and sampled.
            boyko_render::CASTS_SHADOW_BIT | 2,
            // An un-slotted point as `from_point` builds it (`SLOT_NONE` in the field): neither.
            0x003E_0001,
            // An un-slotted point whose field decodes 0 (the pre-R1 word): sampled, not slotted.
            1,
        ];
        let mut bytes = vec![0u8; 64 + rows.len() * 48];
        bytes[28..32].copy_from_slice(&0b1100u32.to_le_bytes());
        for (r, kind) in rows.iter().enumerate() {
            let off = 64 + r * 48 + 12;
            bytes[off..off + 4].copy_from_slice(&kind.to_le_bytes());
        }
        assert_eq!(staged_shadow_words(&bytes), (0b1100, 1, 2));
    }
}
