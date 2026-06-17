//! Shared GPU test harness for the `boyko_render` Wave-B tests.
//!
//! Mirrors the `boyko_rhi_vulkan/tests/device_local_copy.rs` style: boot a
//! validation-enabled [`VulkanContext`] (or skip gracefully on a GPU-less host),
//! and assert the validation messenger recorded ZERO messages (the soundness
//! oracle that substitutes for Miri on the raw-FFI path, plan §6).

#![allow(dead_code)]

use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::component::component_registry::ResidencyKind;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// A test component type whose layout is registered into the global registry.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GpuPayload {
    /// Four bytes per row — a small, fixed stride.
    pub word: u32,
}

/// Byte stride of one [`GpuPayload`] row.
pub const STRIDE: u32 = core::mem::size_of::<GpuPayload>() as u32;

/// Boots a validation-enabled context, or returns `None` (with a SKIP log) when
/// no GPU / loader / validation layer is available.
pub fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages — the Wave-B oracle.
pub fn assert_validation_clean(ctx: &VulkanContext) {
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) — see the [vk-validation] log",
        state.total()
    );
}

/// Registers a `Gpu`-classed component id + builds a GPU-pure single-component
/// archetype on `ecs`, returning `(archetype_id, component_id)`.
///
/// The component is classed `Gpu` BEFORE the archetype is created so the mint
/// stamps `GPU_RESIDENT` (residency is read at archetype construction). The fresh
/// archetype is empty (`len == 0`), satisfying the device-flip O1 guard.
pub fn gpu_pure_archetype(ecs: &mut EcsMaster, raw_id: usize) -> (ArchetypeId, ComponentId) {
    let cid = ComponentId(raw_id);
    component_registry::register_layout::<GpuPayload>(raw_id);
    component_registry::classify_component_residency(raw_id, ResidencyKind::Gpu);
    let arch = ecs.create_archetype(&[cid]);
    (arch, cid)
}

/// A deterministic per-row byte pattern (4 bytes each), as a flat `Vec<u8>`.
pub fn pattern_bytes(rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows * STRIDE as usize);
    for i in 0..rows {
        let w = (i as u32).wrapping_mul(0x9E37_79B1) ^ 0xA5A5_0000;
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}
