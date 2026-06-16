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
//!    pattern → fence → readback → assert ([`compute`]).
//! 4. **0d** — a SECOND compute pass transforms the same buffer, chained through
//!    a `vkCmdPipelineBarrier`, submitted once → diff vs a CPU golden
//!    ([`compute::ComputeHarness::run_chained`]).
//!
//! The single readback in 0c/0d is the TEST ORACLE, not a per-frame path; the
//! validation messenger asserted to zero messages is the soundness oracle that
//! substitutes for Miri on the raw-FFI path (plan §6).
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
//! // `bound.mapped` is a CPU pointer to the buffer's first byte.
//! # let _ = bound;
//! ```

pub mod compute;
pub mod debug;
pub mod device;
pub mod ffi;
pub mod memory;
pub mod suballocator;
