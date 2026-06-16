//! `boyko_rhi_vulkan` — raw hand-FFI Vulkan backend for boyko-engine.
//!
//! This crate is the **NO-SDK sub-step of Slice 0** of the render foundation
//! (see `docs/RENDER-PHYSICS-GPU-PLAN.md` §0, §4, §5.4, §7 Phase 0, §11). It
//! proves the plan's highest-risk item in miniature **without the Vulkan SDK**
//! (no shaders, no validation layers required):
//!
//! 1. A hand-rolled loader + `VkInstance` + `VkDevice` (one graphics+compute
//!    queue), all via raw FFI mirroring `boyko_ecs::ecs::memory::vm.rs`.
//! 2. A `VkDeviceMemory` sub-allocator (free-list with coalescing) over one
//!    large host-visible + host-coherent block.
//! 3. A host-visible buffer write/read round-trip: create a `VkBuffer`,
//!    sub-allocate an aligned offset, bind, map, write, read back, assert.
//!
//! Compute dispatch, SPIR-V, validation-layer-as-oracle and the chained-pass
//! barrier (Slice 0 steps 3-4) are **SDK-gated and deferred** — but the
//! [`device::InstanceConfig::enable_validation`] seam is wired so they can flip
//! on without reshaping the bootstrap.
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

pub mod device;
pub mod ffi;
pub mod memory;
pub mod suballocator;
