//! `boyko_rhi_vulkan` — raw hand-FFI Vulkan backend for boyko-engine.
//!
//! This crate is **Slice 0** of the render foundation (see
//! `docs/RENDER-PHYSICS-GPU-PLAN.md` §0, §4, §5.4, §7 Phase 0, §11). It proves
//! the plan's highest-risk item — zero-readback chained GPU work on a
//! hand-rolled Vulkan backend — in miniature, with `boyko_ecs` untouched:
//!
//! 1. **0a** — a hand-rolled loader + `VkInstance` + `VkDevice` (one
//!    graphics+compute queue) via raw FFI mirroring
//!    `boyko_ecs::ecs::memory::vm.rs`, with the `VK_LAYER_KHRONOS_validation`
//!    messenger wired as the test oracle ([`debug`], [`device`]).
//! 2. **0b** — a `VkDeviceMemory` sub-allocator (free-list with coalescing) over
//!    one large host-visible + host-coherent block ([`memory`],
//!    [`suballocator`]), proven by a buffer write/read round-trip.
//! 3. **0c** — one compute dispatch from a committed `.spv` writes a known
//!    pattern → fence → readback → assert (the [`compute`] assets driven through
//!    the [`rhi_impl`] trait surface).
//! 4. **0d** — a SECOND compute pass transforms the same buffer, chained through
//!    a `vkCmdPipelineBarrier`, submitted once → diff vs a CPU golden (the
//!    [`rhi_impl::VulkanCommandEncoder::pipeline_barrier`] lowering).
//!
//! The single readback in 0c/0d is the TEST ORACLE, not a per-frame path; the
//! validation messenger asserted to zero messages is the soundness oracle that
//! substitutes for Miri on the raw-FFI path (plan §6).
//!
//! # `boyko_rhi` trait surface (Phase 1, compute path)
//!
//! [`rhi_impl`] implements the backend-agnostic [`boyko_rhi`] RHI traits for this
//! backend over the headless compute path: [`rhi_impl::Vulkan`] is the
//! [`RhiApi`](boyko_rhi::RhiApi) marker, [`device::VulkanContext`] is the
//! [`RhiDevice`](boyko_rhi::RhiDevice), [`rhi_impl::VulkanQueue`] the
//! [`RhiQueue`](boyko_rhi::RhiQueue), and [`rhi_impl::VulkanCommandEncoder`] the
//! hot [`RhiCommandEncoder`](boyko_rhi::RhiCommandEncoder). The on-screen path
//! ([`swapchain`]) stays concrete this phase (a Phase-2-3 seam).
//!
//! # Slice 1 — on-screen path (plan §7 Phase 1-3, D8 = our window)
//!
//! On top of Slice 0, [`window`] + [`swapchain`] add a fully in-house on-screen
//! path with ZERO third-party crates:
//!
//! - [`window::Window`] — a raw Win32 window via hand-FFI to `user32`/`kernel32`
//!   (`#[cfg(windows)]`; a non-Windows stub keeps the crate cross-target).
//! - [`swapchain::Surface`] — `vkCreateWin32SurfaceKHR` over the window's
//!   `HWND`/`HINSTANCE`, with a present-capable queue family + color format.
//! - [`swapchain::Swapchain`] — a FIFO swapchain of `COLOR_ATTACHMENT` images +
//!   one view per image, recreated on resize / out-of-date.
//! - [`swapchain::Renderer`] — acquire → record (barrier → Vulkan 1.3
//!   `vkCmdBeginRendering` CLEAR → `vkCmdEndRendering` → barrier) → submit →
//!   present, 2 frames in flight, no `VkRenderPass`/`VkFramebuffer`.
//!
//! Enabled by [`device::InstanceConfig::windowed`]; the headless
//! [`device::VulkanContext::boot`] path is unchanged when it is `false`.
//!
//! # Constraints
//!
//! - **std-only**: no third-party crates (no ash/vulkano/windows-sys/libc).
//! - Every `unsafe` block carries a concrete `// SAFETY:` comment.
//! - x86_64 target (non-dispatchable handles are `u64`).
//!
//! # Example
//!
//! ```no_run
//! use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
//! use boyko_rhi_vulkan::memory::HostVisibleBlock;
//! use boyko_rhi_vulkan::ffi::VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;
//!
//! // Boots a headless device; returns Err on a GPU-less machine.
//! let ctx = VulkanContext::boot(InstanceConfig::default()).expect("no GPU");
//! let mut block = HostVisibleBlock::new(
//!     ctx.device(),
//!     ctx.device_fns(),
//!     ctx.memory_properties(),
//!     16 * 1024 * 1024,
//! )
//! .expect("alloc");
//! let bound = block
//!     .create_bound_buffer(4096, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT)
//!     .expect("buffer");
//! // `bound.mapped` is `Some(ptr)` — the CPU pointer to the buffer's first byte
//! // (a host-visible block always maps; a device-local block carries `None`).
//! # let _ = bound;
//! ```

pub mod abi_guard;
pub mod compute;
pub mod debug;
pub mod device;
pub mod error;
pub mod ffi;
pub mod memory;
pub mod rhi_impl;
pub mod suballocator;
pub mod swapchain;
pub mod texture;
pub mod window;
