//! The host half of the light-table GENERATION protocol (host plan D5/R4; render wave PX1a,
//! H4 Scope 7, arch W8 / perf O11).
//!
//! Writer side ([`boyko_render`]): `collect_lights` bumps the world's `LightTableGeneration`
//! exactly once per ACTUAL rows rewrite (spawn / toggle / removal / header-gate flip) and
//! `LightHeaderGeneration` once per header-only rewrite (the pre-exposure, the tonemapper, a gate
//! word); a static frame bumps neither. Host side (here): [`light_copy_plan`] turns the per-slot
//! STAGING record, the DEVICE-table record and the two generations into this frame's staging
//! write and recorded copy — never a hash, never a host byte-compare (the D5 pin: deterministic
//! writer-side generations only).
//!
//! Pure and GPU-free by design so the protocol is unit-testable headlessly
//! (`tests/light_copy_plan.rs`); the runner is its only production caller.

/// Which part of the light table a plan step touches: nothing, the 64-B header alone, or the
/// whole table (header + rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightRegion {
    /// Nothing.
    None,
    /// Bytes `[0, 64)` — the `LightHeaderGpu` (exposure, counts, the gate words).
    Header,
    /// The whole staged table.
    Full,
}

/// One frame's light-table work (render wave PX1a, H4 Scope 7, arch W8 / perf O11): what to
/// write into the fenced slot's staging, and what to copy from it into the device table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightCopyPlan {
    /// The staging write into the frame's slot.
    pub staging: LightRegion,
    /// The recorded staging → device-table copy.
    pub copy: LightRegion,
}

/// Per in-flight slot: the row and header generations whose bytes that slot's staging holds.
/// Seeded `u64::MAX` (not a generation), so every slot writes the whole table on its first frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightSlotState<const N: usize> {
    /// `LightTableGeneration` of the rows in each slot's staging.
    pub staging_rows_gen: [u64; N],
    /// `LightHeaderGeneration` of the header in each slot's staging.
    pub staging_header_gen: [u64; N],
}

impl<const N: usize> LightSlotState<N> {
    /// The boot seed: no slot holds a generation.
    pub const BOOT: Self = Self { staging_rows_gen: [u64::MAX; N], staging_header_gen: [u64::MAX; N] };
}

/// The generations the DEVICE light table holds — advanced only after a frame that presented
/// (`Ok(true)`), so a skipped or recreated frame leaves the record behind and the next presented
/// frame copies what the device missed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightDeviceState {
    /// `LightTableGeneration` of the rows the device table holds.
    pub device_rows_gen: u64,
    /// `LightHeaderGeneration` of the header the device table holds.
    pub device_header_gen: u64,
}

impl LightDeviceState {
    /// The boot seed: the device table holds nothing the writer produced.
    pub const BOOT: Self = Self { device_rows_gen: u64::MAX, device_header_gen: u64::MAX };
}

/// The light-table work for in-flight slot `slot` this frame, keyed on what the slot's staging
/// and the device table last received — never on this frame's delta (a change staged on a frame
/// the render skipped is still copied by the next presented frame). Records the slot's staging
/// as current; the caller advances `device` after the frame presents.
///
/// - **staging** (slot `slot`): `Full` iff the slot's rows lag `rows_gen`, else `Header` iff its
///   header lags `header_gen`, else `None`; afterwards the slot's staging is current.
/// - **copy** (device-keyed): `Full` iff the device rows lag, else `Header` (64 B) iff the device
///   header lags, else `None`. Copying from a slot whose staging was just made current is always
///   sound: the slot holds the whole current table, whatever region was written into it.
///
/// Under a steady auto-exposure this is a 64-B staging write and a 64-B copy a frame; a static
/// scene writes and copies nothing (the byte-identical idle command stream, L0-r0's 0 %-gate).
#[inline]
pub fn light_copy_plan<const N: usize>(
    slots: &mut LightSlotState<N>,
    slot: usize,
    device: &LightDeviceState,
    rows_gen: u64,
    header_gen: u64,
) -> LightCopyPlan {
    debug_assert!(slot < N, "invariant: the frame slot indexes the in-flight ring");
    let staging = region(slots.staging_rows_gen[slot] != rows_gen, slots.staging_header_gen[slot] != header_gen);
    slots.staging_rows_gen[slot] = rows_gen;
    slots.staging_header_gen[slot] = header_gen;
    let copy = region(device.device_rows_gen != rows_gen, device.device_header_gen != header_gen);
    LightCopyPlan { staging, copy }
}

/// The region a lag implies: the rows lagging owes the whole table (which carries the header).
#[inline]
const fn region(rows_lag: bool, header_lag: bool) -> LightRegion {
    if rows_lag {
        LightRegion::Full
    } else if header_lag {
        LightRegion::Header
    } else {
        LightRegion::None
    }
}
