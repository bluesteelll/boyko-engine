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

mod device;
mod fly;
#[cfg(windows)]
mod gpu_scene;
#[cfg(windows)]
mod host;
#[cfg(windows)]
mod host_dump;
// VG R3 piece 1 step P1-2: the HZB oracle → backend-scalars seam. `#[cfg(windows)]` for the same
// reason `gpu_scene` is — its only caller is the windowed frame loop, and windowing is
// Windows-first (`runner::run_windowed`'s non-Windows arm exits immediately), so an ungated module
// would be dead code on every other target.
#[cfg(windows)]
mod hzb_plan;
mod runner;
#[cfg(windows)]
mod vg_census_dump;
mod window_info;

/// The host half of the light-table generation protocol (host plan D5/R4):
/// the pure per-slot [`light_upload_due`](light_gate::light_upload_due) gate
/// the runner drives its staging-ring rewrites with.
pub mod light_gate;
pub mod plugins;
pub mod prelude;

pub use device::GpuDevice;
pub use fly::{FlyAction, FlyCameraPlugin, fly_default_map};
pub use plugins::EnginePlugins;
pub use window_info::{HostFrameStats, WindowInfo};
