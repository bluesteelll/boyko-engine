//! `boyko_app` — the host layer: the OS loop (Win32 pump), the device boot
//! chain, the windowed runner, and plugin composition (host plan D1).
//!
//! A user goes from `App::new()` to a correct, windowed frame loop with the
//! entire frame discipline enforced by construction:
//!
//! ```no_run
//! use boyko_app::prelude::*;
//!
//! let mut app = App::new();
//! app.add_plugins(EnginePlugins::window("my game", 800, 600));
//! app.run();
//! ```
//!
//! # Layering (host plan D1, the amended `boyko_render` invariant)
//!
//! `boyko_render` (the data bridge) and `boyko_app` (the host) are the only two
//! crates that name both the graphics RHI and the ECS core; `boyko_app` must
//! not define per-entity GPU data paths — those belong in `boyko_render`.
//! `boyko_render` answers "**what** to upload" (ECS-reading, token-typed upload
//! fns); `boyko_app` answers "**when**" (sequencing, token minting,
//! window/swapchain lifetime).
//!
//! # Device lifecycle (host plan D2)
//!
//! The runner boots the process-singleton device via
//! [`VulkanContext::boot_singleton`](boyko_rhi_vulkan::device::VulkanContext::boot_singleton),
//! shares the `&'static` handle with the host structs (the crate-private
//! `WindowHost`) and the World ([`GpuDevice`], `RhiContext::from_shared`), and
//! ends the lifecycle EXACTLY ONCE with `VulkanContext::destroy_singleton` as
//! the LAST statement of teardown, after every holder has been dropped or
//! evicted.


// `missing_const_for_thread_local` is a FALSE POSITIVE on clippy 1.98.0
// (2026-09-01), and this allow is the repair rather than a suppression: a sweep
// of the workspace found 63 `thread_local!` statics across 12 crates and ALL 63
// already use the `const { … }` form the lint asks for, so it has no true
// positive here to hide.
//
// Two cures were tried and neither works. The 1.97.1 -> 1.98.1 toolchain update
// was taken specifically for this; it changed which crates report but did not
// remove the lint, so "wait for upstream" is not a live plan. The
// neighbouring-doc-comment confusion this lint has had before is not the cause
// either: stripping the `///` lines above a flagged static leaves the bare
// `const { … }` form and it still fires.
//
// Placement is crate-level because an `#[allow]` written OUTSIDE a
// `thread_local!` invocation is reported as an `unused attribute` while the lint
// fires anyway. Delete when clippy stops reporting the const form; the sweep
// above is the check that this is still safe to delete blind.
#![allow(clippy::missing_const_for_thread_local)]

mod device;
// L8b's three terminal-exit reporters. NOT `#[cfg(windows)]`: `report_windowing_unsupported` is
// the non-Windows runner arm's only statement, so gating this module on Windows would delete the
// one platform that needs it.
mod diag;
mod fly;
#[cfg(windows)]
mod gpu_scene;
#[cfg(windows)]
mod host;
#[cfg(windows)]
mod host_dump;
// VG R3 piece 1 step P1-6: the `BOYKO_HZB_DUMP` readback driver (gate G8's recording seam, host
// half). `#[cfg(windows)]` for the same reason `hzb_plan` below is.
#[cfg(windows)]
mod hzb_dump;
// VG R3 piece 1 step P1-2: the HZB oracle → backend-scalars seam. `#[cfg(windows)]` for the same
// reason `gpu_scene` is — its only caller is the windowed frame loop, and windowing is
// Windows-first (`runner::run_windowed`'s non-Windows arm exits immediately), so an ungated module
// would be dead code on every other target.
#[cfg(windows)]
mod hzb_plan;
// VG R3 piece 4 rung P4-4: the `OcclusionConfig` × `OcclusionForce` → `VbOcclusionArm` seam, the
// twin of `hzb_plan` above and `#[cfg(windows)]` for the same reason — its only caller is the
// windowed frame loop.
#[cfg(windows)]
mod occlusion_arm;
// VG R3 piece 4 rung P4-4: the DIAGNOSTIC verdict-override Resource. NOT `#[cfg(windows)]`, unlike
// `occlusion_arm` above: it is part of the crate's public surface (fixtures insert it), it names no
// device and no OS, and gating it would make the type invisible to a cross-target doc build.
pub mod occlusion_force;
mod runner;
// KE16 App-12: the `timeBeginPeriod(1)` RAII hold. NOT `#[cfg(windows)]`, unlike its neighbours
// here: the module's own non-Windows arm is the platform statement (POSIX timed waits already
// carry nanosecond deadlines, so there is nothing to raise), and gating the module instead would
// force every caller to carry the `cfg` and would hide the type from a cross-target doc build.
// `pub` because the App-12 measurement in `tests/` constructs the guard across the crate boundary.
pub mod timer_resolution;
// VG R3 piece 3 step P3-5: the `BOYKO_VB_CULL_READBACK` capture driver AND the probe line's one
// serializer.
//
// The MODULE is not `#[cfg(windows)]`, unlike its sibling drivers, because the line format is pure
// string work with no device and no OS in it; the DRIVER inside it IS gated, like `hzb_dump`, since
// its only caller is the windowed frame loop.
//
// `#[doc(hidden)] pub` is a TEST SEAM: the emitter lives here and the parser lives in `tests/`, so
// the round-trip gate can only compare the two if it can call the real emitter across the crate
// boundary. Nothing outside the gate should use it.
#[doc(hidden)]
pub mod vb_cull_probe;
// VG R3 piece 2 step P2-6: the `BOYKO_VB_PROBE` recording probe (gate G2's host half).
// `#[cfg(windows)]` for the same reason `hzb_dump` above is — its only caller is the windowed
// frame loop.
#[cfg(windows)]
mod vb_probe_dump;
#[cfg(windows)]
mod vg_census_dump;
mod window_info;

/// The host half of the light-table generation protocol (host plan D5/R4):
/// the pure per-slot [`light_upload_due`](light_gate::light_upload_due) gate
/// the runner drives its staging-ring rewrites with.
pub mod light_gate;
/// Particles P0: the host half of the effect-table generation protocol — the pure per-slot
/// [`particle_effects_upload_due`](particle_gate::particle_effects_upload_due) gate the runner
/// drives its effect-staging rewrites with, `light_gate`'s twin.
pub mod particle_gate;
/// Particles P0 gates #7/#9: the pool-partition readback — the decoded counters an armed run
/// leaves in the `World` ([`ParticleCountersReadback`](particle_readback::ParticleCountersReadback))
/// plus the settle→capture driver behind `BOYKO_PARTICLE_READBACK_FRAME`.
pub mod particle_readback;
pub mod plugins;
/// Profiling rung 7: the measurement artifact that replaces the stdout channel.
pub mod profiling;
pub mod prelude;

// Profiling rung 13: this crate declares zones of its own (`__telemetry_reduce` and
// `__telemetry_write`), so it must name its partition — the same one line every engine crate that
// declares a zone writes. Without it `declare_zone!` fails on an unresolved
// `__BOYKO_ZONE_PARTITION`, which is the mechanism refusing an unpartitioned zone rather than
// silently filing it under the engine's.
boyko_diag::profiling_partition!(Engine);

pub use device::GpuDevice;
pub use fly::{FlyAction, FlyCameraPlugin, fly_default_map};
pub use occlusion_force::OcclusionForce;
pub use plugins::EnginePlugins;
pub use timer_resolution::TimerResolutionGuard;
pub use window_info::{
    HOST_TEARDOWN_WATCH_SLOTS, HostAllocationWatch, HostFrameStats, HostTeardownStats, WindowInfo,
};
