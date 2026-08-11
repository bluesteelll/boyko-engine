//! **The reference profiling overlay** — profiling rung 15, gated by `G19`.
//!
//! One ECS system that turns the profiler's windowed statistics into on-screen text, and the
//! reference implementation a game copies. It is a **read path and nothing else**: it never arms,
//! disarms, folds or subscribes. Everything it displays was already measured by the time it runs.
//!
//! # The property this file exists to have: the read path allocates nothing
//!
//! Not a nicety — an overlay that allocates per row per frame turns the profiler into a thing that
//! perturbs the frame it is reporting on, and the perturbation grows with the number of rows a
//! developer is watching. `G19` is the gate, with a positive control, and the mechanism is three
//! choices made here rather than one clever trick:
//!
//! 1. **The text lands in [`UiTextBuffer`]**, a 256 B POD component with an inline `[u8; 247]` and
//!    a `core::fmt::Write` impl. `write!` into a `fmt::Write` allocates nothing; `format!` would,
//!    and is why the control system in `G19` uses it.
//! 2. **The zone is an id, not a name.** [`ProfiledZone`] is `boyko_ecs`'s own component and its
//!    doc says exactly why it exists: *"a zone name resolved to an id once at setup, so a reader
//!    never calls the `#[cold]` by-name lookup on a frame path"*. A new overlay-specific component
//!    would have been a second answer to a question the kernel had already answered — the parallel
//!    data system Principle 0 forbids.
//! 3. **A scratch [`UiTextBuffer`] on the stack**, so the write is compared before it is committed.
//!    That is not about allocation, it is about the tick: `UiTextBuffer` is what
//!    `ui_text_measure_system` gates on with `Changed<UiTextBuffer>`, so overwriting it
//!    unconditionally would re-shape every row every frame even when the displayed number did not
//!    move. The comparison is 256 B of stack and a `PartialEq`.
//!
//! # What it prints, and why it asks the store what the number MEANS
//!
//! A cell's `total` is ticks for a span, increments for a counter and a level for a gauge — and
//! **the zone descriptor does not say which**. Profiling rung 13 measured that and added the
//! store's observed-kind map for it. So this overlay reads [`Profiler::observed_kind`] and labels
//! the figure accordingly; printing `us` beside a counter would be a unit this code cannot know it
//! has. A zone that has never run has no observed kind, and that is displayed as such rather than
//! as a zero.
//!
//! # Why the plugin refuses instead of the system panicking
//!
//! `Res<Profiler>` panics when the resource is absent, and `Option<Res<R>>` is not a `SystemParam`
//! in this kernel (`common_conditions.rs` names it as something a `resource_exists` condition
//! *would* need; neither exists). The absent case is real: `ProfilerPlugin` deliberately does not
//! insert the store in a **second** world, because the lane rings are process-global and two worlds
//! folding them would each take half the samples.
//!
//! So [`ProfilingOverlayPlugin`] checks for the store at build time and, if it is absent, registers
//! **no system at all** — the same shape `ProfilerPlugin` uses for the same reason, one layer down.
//! The cost is an ordering requirement, stated rather than assumed: **add `ProfilerPlugin` first.**

use core::fmt::Write as _;

use boyko_ecs::ecs::core::app::{App, CoreSchedule, Plugin};
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::profiling::{CellLabel, ProfiledZone, Profiler};
use boyko_ecs::ecs::core::system::Res;

use crate::binding::UiTextBuffer;

/// The overlay's own zone name column width, in bytes of the formatted line.
///
/// Not a layout constant — the UI's layout is `ComputedRect`'s. It is the width the name is padded
/// to so the numbers of successive rows line up in a monospaced font, which is the only reason a
/// text overlay needs a column width at all.
const NAME_COLUMN: usize = 22;

/// Updates one text row per [`ProfiledZone`] entity from the profiler's last COMPLETE frame.
///
/// # Which frame, and why not the live one
///
/// [`Profiler::latency`] reports `cpu_frames_behind`, and the answer is `1` by the store's
/// live-frame cut: the frame currently being recorded is still accumulating, so reading it would
/// show a row that grows while you look at it and would disagree with the artifact for the same
/// frame number. The overlay is therefore one frame stale **by construction**, which is what a
/// windowed statistic is for and what D25's latency table exists to state.
///
/// # Allocation-free, and the three things that would break it
///
/// `write!` into `UiTextBuffer` (inline, `fmt::Write`); `zone_desc` returns a `&'static ZoneDesc`
/// out of a `.bss` registry; the scratch is a stack `UiTextBuffer`. Introducing a `String`, a
/// `format!`, or a `Vec` of rows would each break it, and `G19`'s control system is a `format!` so
/// the gate can tell a passing measurement from an uninstalled hook.
pub fn profiling_overlay_system(
    mut rows: Query<(&ProfiledZone, Mut<UiTextBuffer>)>,
    profiler: Res<Profiler>,
) {
    // A disarmed profiler has no window to read. Say so once per row rather than printing zeros:
    // a structural zero is indistinguishable from a measured zero, which is the discipline this
    // whole campaign has applied since rung 2.
    let armed = profiler.is_armed();

    let row_index = complete_row(&profiler);

    for (zone, mut buffer) in rows.iter_mut() {
        let mut scratch = UiTextBuffer::default();
        write_row(&mut scratch, &profiler, zone.0, row_index, armed);

        // Set-if-changed through the `Mut` guard. An `&mut UiTextBuffer` item would NOT bump the
        // change tick even when the value moved, so the measure pass would never re-shape a row
        // whose number had changed; a blind `*buffer = scratch` bumps it every frame, so every row
        // re-shapes forever. Only this comparison gives a steady overlay a quiet steady state.
        if *buffer != scratch {
            *buffer = scratch;
        }
    }
}

/// The store row of the last frame whose cells are complete, or `None` if there is not one yet.
///
/// Resolved ONCE per pass rather than per row. Per-row would be correct and would also let the
/// frame advance between two rows of one screenful, so the top of the overlay could describe a
/// different frame from the bottom — a disagreement a reader has no way to see.
///
/// `cpu_frames_behind` is `1` by the store's live-frame cut: the frame being recorded is still
/// accumulating, so reading it shows a row that grows while you look at it.
#[must_use]
pub fn complete_row(profiler: &Profiler) -> Option<u32> {
    let latency = profiler.latency();
    latency
        .live_frame
        .checked_sub(latency.cpu_frames_behind)
        .and_then(|f| profiler.row_of(f))
}

/// Formats one row into `out`.
///
/// **Public because `G19` is its second caller.** The gate needs the formatting path without a
/// world and without a scheduler: a query iteration and a `Mut` deref are kernel code with their
/// own gates, and folding them into this one would make a failure here ambiguous between "the
/// overlay allocated" and "iterating a query allocated". What is left is exactly the part an
/// overlay author can get wrong.
pub fn write_row(
    out: &mut UiTextBuffer,
    profiler: &Profiler,
    zone: u16,
    row_index: Option<u32>,
    armed: bool,
) {
    out.clear();

    // The name comes from the process-wide zone registry, not from anything the overlay stores.
    // `None` means the id was never minted -- a row pointing at a zone that does not exist, which
    // is an authoring mistake and is shown as one rather than silently rendering blank.
    let Some(desc) = boyko_diag::profiling_abi::zone_desc(zone) else {
        let _ = write!(out, "zone {zone} not registered");
        return;
    };

    let name = desc.name;
    let _ = write!(out, "{name}");
    for _ in name.len()..NAME_COLUMN {
        let _ = out.write_char(' ');
    }

    if !armed {
        let _ = write!(out, "disarmed");
        return;
    }
    let Some(row) = row_index else {
        // Armed, but no complete frame yet -- the first frame of a session. Distinct from
        // `disarmed`, because the two are different states and a reader acts differently on them.
        let _ = write!(out, "no complete frame");
        return;
    };
    let Some(cell) = profiler.cell(row, zone) else {
        let _ = write!(out, "out of window");
        return;
    };

    if cell.count == 0 {
        // The zone exists and the frame is complete: it simply did not run. That is a MEASUREMENT
        // of zero, and it is printed differently from the three states above, which are absences.
        let _ = write!(out, "     0 x");
        return;
    }

    // The unit is an OBSERVATION, not a declaration: a `ZoneDesc` carries no kind, which profiling
    // rung 13 measured and which is why the store grew a one-byte-per-zone observed-kind map. A
    // fixed `us` here would be a unit this code cannot know it has.
    let unit = match profiler.observed_kind(zone) {
        Some(boyko_diag::sample::SampleKind::Span) => "tk",
        Some(boyko_diag::sample::SampleKind::Counter) => "ct",
        Some(boyko_diag::sample::SampleKind::Gauge) => "lv",
        None => "??",
    };

    let _ = write!(out, "{:>6} x {:>12}{unit}", cell.count, cell.total);

    // The label is the store's own statement about what the figures are worth, and it is appended
    // only when it is NOT the ordinary case -- an overlay that printed `Measured` on every row
    // would train its reader to stop looking at the column. In practice this is `OverRange`:
    // `Empty` is unreachable here (the `count == 0` branch above took it), and that is stated
    // rather than assumed, because a fourth label added later would land in this arm and be shown.
    if cell.label != CellLabel::Measured {
        let _ = write!(out, " [{:?}]", cell.label);
    }
}

/// Registers [`profiling_overlay_system`] — **if** this world has a profiler.
///
/// Add it after `ProfilerPlugin`. If the store is absent (a second world, where `ProfilerPlugin`
/// refuses to bind), no system is registered and the overlay's rows simply keep whatever text they
/// had. That is a quieter failure than a per-frame panic and it is the same choice `ProfilerPlugin`
/// makes one layer down, for the same reason.
#[derive(Default)]
pub struct ProfilingOverlayPlugin;

impl Plugin for ProfilingOverlayPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<Profiler>() {
            return;
        }
        app.add_systems_in(CoreSchedule::Main, profiling_overlay_system);
    }

    fn name(&self) -> &'static str {
        "boyko_ui::ProfilingOverlayPlugin"
    }
}
