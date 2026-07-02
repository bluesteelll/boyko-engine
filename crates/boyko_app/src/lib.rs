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
#[cfg(windows)]
mod gpu_scene;
#[cfg(windows)]
mod host;
mod runner;
mod window_info;

pub mod plugins;
pub mod prelude;

pub use device::GpuDevice;
pub use plugins::EnginePlugins;
pub use window_info::WindowInfo;
