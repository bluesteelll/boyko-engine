//! Render wave PX1a test 8b (`plan/H4-PX1a-hdr-lit.md` Tests 8, rev 3 arch W8 / perf O11; `px1a/cut.md`
//! D-E6, C2, C8): **the light copy is keyed on what the device table last received.**
//!
//! `light_copy_plan` decides, per frame, what the fenced slot's staging gets (`staging`) and what
//! the recorded copy moves into the device table (`copy`): `Full` when the rows lag, else `Header`
//! (64 B) when only the header lags, else nothing. The DEVICE record advances only after a frame
//! that presented — the scripts below drive the runner's three outcomes: a skipped (minimized)
//! frame that plans nothing, a frame that returned `Ok(false)` (acquire-time out-of-date: no
//! advance, and the next frame reuses the slot), and a presented frame (`Ok(true)`).

use boyko_app::light_gate::{LightCopyPlan, LightDeviceState, LightRegion, LightSlotState, light_copy_plan};

const FIF: usize = 2;

/// The host side of the protocol over a scripted frame sequence.
struct Host {
    slots: LightSlotState<FIF>,
    device: LightDeviceState,
    frame: usize,
}

impl Host {
    fn boot() -> Self {
        Self { slots: LightSlotState::BOOT, device: LightDeviceState::BOOT, frame: 0 }
    }

    /// A frame that renders: plans on its slot, then presents (`Ok(true)`) or not (`Ok(false)`:
    /// the device record stays, and the frame index does not advance).
    fn render(&mut self, rows: u64, header: u64, presented: bool) -> LightCopyPlan {
        let plan = light_copy_plan(&mut self.slots, self.frame % FIF, &self.device, rows, header);
        if presented {
            self.device = LightDeviceState { device_rows_gen: rows, device_header_gen: header };
            self.frame += 1;
        }
        plan
    }
}

fn plan(staging: LightRegion, copy: LightRegion) -> LightCopyPlan {
    LightCopyPlan { staging, copy }
}

use LightRegion::{Full, Header, None as Nothing};

#[test]
fn boot_writes_and_copies_the_whole_table_then_goes_quiet() {
    let mut h = Host::boot();
    assert_eq!(h.render(0, 0, true), plan(Full, Full), "slot 0's first frame");
    assert_eq!(h.render(0, 0, true), plan(Full, Nothing), "slot 1's first frame: the device already holds it");
    assert_eq!(h.render(0, 0, true), plan(Nothing, Nothing));
    assert_eq!(h.render(0, 0, true), plan(Nothing, Nothing));
}

#[test]
fn a_light_moved_on_a_skipped_frame_is_copied_whole_by_the_first_presented_frame() {
    let mut h = Host::boot();
    h.render(1, 1, true);
    h.render(1, 1, true);
    // A minimized frame: `collect_lights` rebuilt the rows (2), the runner planned nothing.
    // Then header-only frames, also skipped while minimized: the header moved on (3, 4).
    // The window comes back: the first presented frame must copy the rows it never copied.
    assert_eq!(h.render(2, 4, true), plan(Full, Full), "the first presented frame after the skip");
    assert_eq!(h.render(2, 4, true), plan(Full, Nothing), "the other slot catches up, the device is current");
    assert_eq!(h.render(2, 4, true), plan(Nothing, Nothing));
}

#[test]
fn a_rows_change_on_an_out_of_date_frame_is_copied_by_the_next_frame() {
    let mut h = Host::boot();
    h.render(1, 1, true);
    h.render(1, 1, true);
    assert_eq!(h.render(2, 1, false), plan(Full, Full), "the frame that returned Ok(false)");
    // The same slot again (the frame index did not advance): its staging is current, the device is not.
    assert_eq!(h.render(2, 1, true), plan(Nothing, Full), "the next frame still copies the rows");
    assert_eq!(h.render(2, 1, true), plan(Full, Nothing));
    assert_eq!(h.render(2, 1, true), plan(Nothing, Nothing));
}

#[test]
fn a_steady_auto_exposure_moves_64_bytes_a_frame() {
    let mut h = Host::boot();
    h.render(1, 1, true);
    h.render(1, 1, true);
    for header in 2..34 {
        assert_eq!(h.render(1, header, true), plan(Header, Header), "header generation {header}");
    }
}

#[test]
fn a_static_manual_scene_moves_nothing() {
    let mut h = Host::boot();
    h.render(5, 9, true);
    h.render(5, 9, true);
    for _ in 0..32 {
        assert_eq!(h.render(5, 9, true), plan(Nothing, Nothing));
    }
}

#[test]
fn a_rows_change_with_a_header_change_copies_whole_once() {
    let mut h = Host::boot();
    h.render(1, 1, true);
    h.render(1, 1, true);
    assert_eq!(h.render(2, 2, true), plan(Full, Full));
    assert_eq!(h.render(2, 3, true), plan(Full, Header), "the other slot lags the rows; the device only the header");
    assert_eq!(h.render(2, 3, true), plan(Header, Nothing));
}
