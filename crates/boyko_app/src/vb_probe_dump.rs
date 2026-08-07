//! **VG R3 piece 2 step P2-6 — gate G2's recording probe** (`BOYKO_VB_PROBE=<path.toml>`).
//!
//! The host half of the [`VbRecordProbe`] route: it hands `record_vb` a count sink on ONE settled
//! frame, then writes what the RECORDER reported — plus the host's own independent numbers — to
//! the named path, and reports ready so the frame loop can exit.
//!
//! # Why this driver has no staging, no request-until-presented dance and no DRAIN
//!
//! Its three siblings ([`host_dump`](crate::host_dump), [`vg_census_dump`](crate::vg_census_dump),
//! [`hzb_dump`](crate::hzb_dump)) all copy a DEVICE resource into host-visible staging, so each
//! must outlive the in-flight ring by a frame before mapping it — this project's recorded
//! cross-frame bug class. This one copies nothing: every count is written by the host into host
//! memory *while the command buffer is being recorded*, so the numbers are complete the moment
//! `render_gbuffer_frame` returns. A drain here would delay the exit and prove nothing.
//!
//! What it DOES keep from that family is the settle window and the "the frame must have
//! PRESENTED" condition: an `Ok(false)` recreate-skip may not have run the record body at all, so
//! a probe drained off such a frame would report zeros that read as "the split never recorded".
//!
//! # What the artifact CANNOT claim
//!
//! That the GPU executed the late scope — [`VbRecordProbe`]'s own doc states that limit, and it is
//! restated in the written file so a reader of the artifact meets it without opening this module.

use std::io::Write as _;

use boyko_render::OcclusionMode;
use boyko_rhi_vulkan::present::{
    VB_CULL_OCC_ARMED, VB_CULL_OCC_FORCE_KEEP, VB_CULL_OCC_FORCE_LATE, VbRecordProbe,
};

use crate::occlusion_force::OcclusionForce;

/// Presented frames rendered before the probe frame — the settle window
/// [`hzb_dump`](crate::hzb_dump) uses, for the same reasons (propagation, light reconcile, the
/// light-table upload and the CSM fit all settle within the first frames).
///
/// It is not what makes the marker visible: every fixture in this piece marks at SPAWN, so the
/// capability is in the gather from the first rendered frame (an `insert` would have armed the
/// split one frame late — see `boyko_render::occlusion_marker`). The settle is here so the probe
/// reads a frame in the same steady state every other capture in this repository reads.
const SETTLE_FRAMES: u32 = 30;

/// The settle → probe progression. `Settle` counts REMAINING presented frames; `Probe` hands the
/// recorder its sink and re-arms across `Ok(false)` recreate-skips until a probe frame presents.
enum ProbeState {
    Settle(u32),
    Probe,
}

/// The host numbers written BESIDE the recorder's, never in place of them.
///
/// Every field here is derived on the host from a different site than the recorder's counters, so
/// the gate can compare two independent derivations (`late_draws` against `draw_batches`) instead
/// of comparing the recorder with itself.
pub(crate) struct VbProbeContext {
    /// `GBufferScene::mesh_draw.len()` — the host's own batch count for the probed frame.
    pub(crate) draw_batches: u32,
    /// `MeshRenderScratch::occlusion_instances()` — instances in this frame's ring carrying
    /// `OcclusionCulling`. `> 0` is the structural conjunct of `path_vb_occlusion_split()`.
    pub(crate) occlusion_instances: u32,
    /// Did this boot resolve `RenderPath::VisibilityBuffer`? Recorded, not assumed: a device that
    /// fails the VB capability probe degrades to `Deferred`, `record_vb` never runs, and the
    /// counts would be zero for an INSTRUMENT reason rather than a split one.
    pub(crate) vb_path: bool,
    /// …and did it resolve a MESH leg? A `VisibilityBuffer × Sdf` frame records no raster scope at
    /// all, which is likewise not a split failure.
    pub(crate) mesh_leg: bool,
    /// VG R3 piece 4 rung P4-4: the owner's `OcclusionConfig::mode` as the HOST read it on the
    /// probed frame.
    pub(crate) occ_mode: OcclusionMode,
    /// …and the diagnostic `OcclusionForce` beside it.
    ///
    /// # Why the artifact carries this at all
    ///
    /// Rung P4-4 deleted a boot-time `BOYKO_VG_OCC_FORCE` read from shipping code. That read
    /// justified itself partly by *"a knob that can change mid-run would make 'which regime
    /// produced this capture?' unanswerable from the artifact"* — and a live Resource reopens
    /// exactly that. The answer is not an assertion that the Resource held still (which would have
    /// to hold on hosts this repository does not own); it is to RECORD the regime, from two
    /// independent sites, on the frame it describes: this one, and
    /// [`VbRecordProbe::occ_flags`](boyko_rhi_vulkan::present::VbRecordProbe::occ_flags), which is
    /// stamped from the word the recorder actually PUSHED. A gate comparing the two compares two
    /// derivations — this module's stated design principle, applied to provenance instead of
    /// counts — and a mid-run flip becomes VISIBLE (a `[probe]`/`[host]` disagreement) instead of
    /// silently attributed.
    pub(crate) occ_force: OcclusionForce,
}

/// The probe driver the frame loop threads through its steady path.
pub(crate) struct VbProbeDump {
    /// Destination path (the `BOYKO_VB_PROBE` value), written once.
    path: String,
    state: ProbeState,
    /// The counts the RECORDER wrote. Zeroed until the probe frame; handed out by
    /// [`request`](Self::request) as `&mut`.
    probe: VbRecordProbe,
}

impl VbProbeDump {
    /// Arms the probe iff `BOYKO_VB_PROBE` is set (the value is the output path). Cold: called
    /// once before the frame loop.
    pub(crate) fn from_env() -> Option<Self> {
        let path = std::env::var("BOYKO_VB_PROBE").ok()?;
        eprintln!("boyko_app: BOYKO_VB_PROBE armed -> {path}");
        Some(Self { path, state: ProbeState::Settle(SETTLE_FRAMES), probe: VbRecordProbe::default() })
    }

    /// The per-frame request: `Some(&mut probe)` on the probe frame(s), `None` otherwise.
    ///
    /// The sink is RESET on every request rather than accumulated: a recreate-skip can leave a
    /// half-recorded frame's counts behind, and a `scopes` that summed two frames would read as a
    /// split on an unsplit run.
    pub(crate) fn request(&mut self) -> Option<&mut VbRecordProbe> {
        if !matches!(self.state, ProbeState::Probe) {
            return None;
        }
        self.probe = VbRecordProbe::default();
        Some(&mut self.probe)
    }

    /// Advances the state machine after a present. Returns `true` on the frame whose counts are
    /// complete — i.e. a probe frame that actually presented.
    ///
    /// A frame that did NOT present may never have reached the record body at all (an `Ok(false)`
    /// recreate-skip), so it advances nothing: the sink stays armed rather than reporting the
    /// zeros it would still be holding.
    pub(crate) fn after_present(&mut self, presented: bool) -> bool {
        if !presented {
            return false;
        }
        match self.state {
            ProbeState::Settle(n) => {
                self.state = if n > 1 { ProbeState::Settle(n - 1) } else { ProbeState::Probe };
                false
            }
            // The probe frame presented, so `record_vb` ran and every count is written. No drain
            // follows, because nothing was copied out of the device to wait for.
            ProbeState::Probe => true,
        }
    }

    /// Writes the artifact. Consumes the driver, exactly as its three siblings do, so a second
    /// write of the same run is not expressible.
    pub(crate) fn finish(self, cx: &VbProbeContext) {
        match write_probe(&self.path, &self.probe, cx) {
            Ok(()) => eprintln!(
                "boyko_app: vb record probe written -> {} (scopes={}, late_draws={}, \
                 late_cull_dispatches={}, late_seed_instances={}, probe occ_regime={}, host \
                 draw_batches={}, occlusion_instances={}, occ_mode={}, occ_force={})",
                self.path,
                self.probe.scopes,
                self.probe.late_draws,
                self.probe.late_cull_dispatches,
                self.probe.late_seed_instances,
                regime_word(self.probe.occ_flags),
                cx.draw_batches,
                cx.occlusion_instances,
                cx.occ_mode.as_str(),
                cx.occ_force.as_str()
            ),
            Err(e) => eprintln!("boyko_app: vb record probe write FAILED ({}): {e}", self.path),
        }
    }
}

/// The artifact: the same flat, section-scoped TOML subset the census row uses, so one reader
/// shape serves both.
///
/// `[probe]` is what the RECORDER wrote; `[host]` is what the host derived independently. They are
/// separate tables because the whole value of the file is that a reader can tell which side each
/// number came from.
fn write_probe(path: &str, probe: &VbRecordProbe, cx: &VbProbeContext) -> std::io::Result<()> {
    let mut out = String::with_capacity(1024);
    out.push_str("# VG R3 piece 2 gate G2 -- MACHINE-WRITTEN by boyko_app::vb_probe_dump.\n");
    out.push_str("# [probe] is authored by `Renderer::record_vb` AT the vkCmd* calls it counts.\n");
    out.push_str("# [host] is derived independently, so `late_draws == draw_batches` compares two\n");
    out.push_str("# sites rather than one site with itself.\n");
    out.push_str("# LIMIT: this says the HOST RECORDED the scope, never that the GPU executed it.\n");
    out.push_str("# On a converged static frame the late scope correctly draws ZERO instances, so\n");
    out.push_str("# its execution has no observable consequence and no gate here can see it.\n");
    out.push_str("# The GPU's own numbers come from BOYKO_VB_CULL_READBACK, not from this file.\n");
    // VG R3 piece 3 step P3-6: schema 2 renamed `late_instances` -> `late_seed_instances` and added
    // `late_cull_dispatches`. The version is BUMPED rather than left alone because the rename makes
    // a schema-1 reader's `late_instances` lookup fail loudly (`vb_occ_split_gate.rs`'s `field()`
    // panics on a missing key) instead of reading a field that has changed meaning.
    //
    // VG R3 piece 4 rung P4-4: schema 3 adds the REGIME's provenance — `[probe] occ_flags` /
    // `occ_regime` (the word the RECORDER pushed) and `[host] occ_mode` / `occ_force` (what the
    // host's live Resource reads said on the same frame). Bumped for the same discipline, and the
    // failure mode it prevents is specific: a schema-2 artifact read by a schema-3 reader would
    // find no `occ_regime` key, and a reader that defaulted it would certify the WRONG regime for
    // the capture — the exact class the deleted boot-time env read existed to rule out.
    out.push_str("schema_version = 3\n\n");

    out.push_str("[probe]\n");
    out.push_str(&format!("scopes = {}\n", probe.scopes));
    out.push_str(&format!("late_draws = {}\n", probe.late_draws));
    out.push_str(&format!("late_cull_dispatches = {}\n", probe.late_cull_dispatches));
    out.push_str(&format!("late_seed_instances = {}\n", probe.late_seed_instances));
    // The RECORDER's word, verbatim and then decoded. Both, because the raw word is what was
    // pushed (a reader can check the ARMED bit itself) while the decode is what a gate compares
    // against `[host] occ_force` and against the pin's own `BOYKO_VG_OCC_FORCE` value.
    out.push_str(&format!("occ_flags = {}\n", probe.occ_flags));
    out.push_str(&format!("occ_regime = \"{}\"\n", regime_word(probe.occ_flags)));
    out.push_str(&format!("occ_armed = {}\n\n", probe.occ_flags & VB_CULL_OCC_ARMED != 0));

    out.push_str("[host]\n");
    out.push_str(&format!("draw_batches = {}\n", cx.draw_batches));
    out.push_str(&format!("occlusion_instances = {}\n", cx.occlusion_instances));
    out.push_str(&format!("vb_path = {}\n", cx.vb_path));
    out.push_str(&format!("mesh_leg = {}\n", cx.mesh_leg));
    out.push_str(&format!("occ_mode = \"{}\"\n", cx.occ_mode.as_str()));
    out.push_str(&format!("occ_force = \"{}\"\n", cx.occ_force.as_str()));

    let mut f = std::fs::File::create(path)?;
    f.write_all(out.as_bytes())
}

/// The regime word carried by a RECORDED `occ_flags` value, decoded through the same
/// [`OcclusionForce`] table the fixtures' env decode uses.
///
/// Decoded from the FORCE bits alone: the ARMED bit is reported separately (`occ_armed`), because
/// "no force" and "not armed" are different facts and folding them into one word would make an
/// unsplit frame indistinguishable from an armed-and-unforced one — which is precisely the
/// distinction `[vb_occ_mixed_off]` versus `[vb_occ_mixed]` rests on.
///
/// Both bits set is a host-fold bug (`OcclusionForce::flags` cannot produce it, and both
/// `record_vb` and `GpuSceneBundles::scene` `debug_assert!` against it). It is spelled here rather
/// than silently reported as one of the two, so an artifact from a release build carrying the
/// impossible word says so.
fn regime_word(occ_flags: u32) -> &'static str {
    let keep = occ_flags & VB_CULL_OCC_FORCE_KEEP != 0;
    let late = occ_flags & VB_CULL_OCC_FORCE_LATE != 0;
    match (keep, late) {
        (false, false) => OcclusionForce::None.as_str(),
        (true, false) => OcclusionForce::KeepAll.as_str(),
        (false, true) => OcclusionForce::DeferAll.as_str(),
        (true, true) => "both",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every regime's own `flags()` decodes back to its own word, beside the ARMED bit and
    /// without it. This is the artifact's half of the `as_str()` ↔ env round-trip: the recorder
    /// stamps a WORD-less `u32` and this is the only place that turns it back into the spelling a
    /// gate compares against `[host] occ_force` and against the pin's `BOYKO_VG_OCC_FORCE`.
    #[test]
    fn every_regime_word_round_trips_through_the_pushed_flags() {
        for force in OcclusionForce::ALL {
            for armed in [0, VB_CULL_OCC_ARMED] {
                assert_eq!(
                    regime_word(armed | force.flags()),
                    force.as_str(),
                    "{force:?} (armed bit {armed}): the decoded word must not depend on ARMED"
                );
            }
        }
    }

    /// The impossible word is spelled, not silently collapsed into one of the two real regimes.
    #[test]
    fn both_force_bits_report_as_neither_regime() {
        let word = regime_word(
            VB_CULL_OCC_ARMED | VB_CULL_OCC_FORCE_KEEP | VB_CULL_OCC_FORCE_LATE,
        );
        assert_eq!(word, "both");
        assert_eq!(
            OcclusionForce::from_word(word),
            None,
            "`both` must not decode back to a regime -- a reader that accepted it would report a \
             host-fold bug as a measurement"
        );
    }
}
