//! Rung 1a — the specialization-constant GPU smoke test.
//!
//! NOTE: Requires the compiled `spec_constant_smoke.comp.spv` — the orchestrator
//! compiles it via dxc. Run:
//!   cargo test -p boyko_rhi_vulkan --features spec_constant_smoke --test spec_constant_smoke
//!
//! The proof that the RHI's new spec-constant path lowers a `SpecConstant` into a
//! live `VkSpecializationInfo` the driver honors at pipeline-create. The
//! `spec_constant_smoke.comp.hlsl` shader writes its `[[vk::constant_id(0)]] SPEC_N`
//! (default 3) into `buffer[0]`. This test builds the compute pipeline from the SAME
//! module TWICE:
//!   1. `spec_constants: &[]`  ⇒ EMPTY ⇒ the backend passes a LITERAL `ptr::null()`
//!      for `p_specialization_info` (the byte-neutral pre-spec-const path) ⇒ the
//!      readback must be the shader default, 3.
//!   2. `spec_constants: &[SpecConstant { id: 0, value: 7 }]` ⇒ the backend assembles
//!      a `VkSpecializationInfo` overriding constant_id 0 ⇒ the readback must be 7.
//!
//! The two readbacks are the oracle (empty ⇒ default; override ⇒ 7); validation-clean
//! is an additional check. It is the C4 linchpin's END-TO-END proof: a pure-host
//! "empty ⇒ null" assertion is not reachable without the FFI create call, so the
//! empty-vs-7 readback is what proves both the null path and the override path.
//!
//! `#![cfg(feature = "spec_constant_smoke")]`: the whole test compiles ONLY under
//! `--features spec_constant_smoke` (which also enables the `.spv` getter in
//! `compute.rs`) — so a default build never references the orchestrator-compiled
//! `.spv`. `#[ignore]`: it needs a live GPU; the orchestrator runs it with
//! `-- --ignored --test-threads=1`. On a GPU-less / loader-less host it SKIPs.
#![cfg(feature = "spec_constant_smoke")]

use core::ptr::NonNull;

use boyko_rhi::{
    BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage, SpecConstant,
};

use boyko_rhi_vulkan::compute::spec_constant_smoke_spirv;
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The shader default of `[[vk::constant_id(0)]] const uint SPEC_N = 3;` — what the
/// EMPTY-spec (byte-neutral null) pipeline must read back.
const SHADER_DEFAULT: u32 = 3;
/// The override value the second build binds to constant_id 0.
const OVERRIDE_VALUE: u32 = 7;

/// Boots a validation-enabled context, or returns `None` (with a SKIP log) when no
/// GPU / loader / validation layer is available (the rung tests' skip convention).
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
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

/// Asserts the validation messenger recorded ZERO messages; a no-op note when
/// validation is disabled via `BOYKO_DISABLE_VALIDATION`.
fn assert_validation_clean(ctx: &VulkanContext) {
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — messenger oracle skipped");
        return;
    }
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the spec-const smoke — see the [vk-validation] log",
        state.total()
    );
}

/// Reads the first `u32` from a buffer's persistent host-coherent mapping (valid
/// only after a fence-waited submit).
fn read_first_u32(base: NonNull<u8>) -> u32 {
    // SAFETY: `base` is the first byte of a >= 4-byte host-coherent mapping; a fence
    // wait preceded this read, so the GPU write is complete + coherent.
    // `read_unaligned` tolerates the sub-allocated offset's alignment.
    unsafe { base.as_ptr().cast::<u32>().read_unaligned() }
}

/// Dispatches the smoke shader once with the given `spec_constants` and returns the
/// value the single thread wrote into `buffer[0]`. Builds a full pipeline (shared
/// fixed compute layout — `bind_group_layout: None`, one STORAGE_BUFFER at binding 0),
/// records begin → bind → push → dispatch → end, submits, fence-waits, reads back,
/// and tears everything down in reverse resource order.
fn run_with_spec(ctx: &VulkanContext, spec_constants: &[SpecConstant]) -> u32 {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    // A single-`u32` host-visible+coherent storage buffer (the device routes it
    // through its shared block).
    let buffer = device
        .create_buffer(&BufferDesc {
            size: 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("smoke storage buffer");

    let module = device
        .create_shader_module(spec_constant_smoke_spirv())
        .expect("spec_constant_smoke shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            // A dummy 4-byte push (the shared compute layout rejects a 0-byte range);
            // the shader gates its single write on `tid.x < pc.count`.
            push_constant_bytes: 4,
            bind_group_layout: None,
            spec_constants,
        })
        .expect("spec_constant_smoke compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    // One thread; `count = 1` push so the single dispatched thread stores.
    let count: u32 = 1;
    encoder.begin().expect("begin");
    encoder.bind_compute_pipeline(&pipeline);
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &count.to_ne_bytes());
    encoder.dispatch(1, 1, 1);
    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mapped = device
        .buffer_mapped_ptr(&buffer)
        .expect("host-visible buffer is mapped");
    let value = read_first_u32(mapped);

    // Teardown in reverse resource order (no submission is pending — fence-waited).
    // SAFETY: every resource below was created on `device` and is destroyed exactly
    // once; the last submission completed (fence-waited above), so none is in use.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_buffer(buffer);
    }
    value
}

/// The Rung 1a oracle: an EMPTY spec slice reads back the shader default (3, the
/// byte-neutral null path); a `SpecConstant { id: 0, value: 7 }` override reads back 7.
#[test]
#[ignore = "requires a live GPU (run: --features spec_constant_smoke -- --ignored --test-threads=1)"]
fn spec_constant_empty_vs_override() {
    let Some(ctx) = boot_or_skip("spec_constant_empty_vs_override") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());

    // 1. EMPTY ⇒ literal null p_specialization_info ⇒ the shader default (3).
    let default_value = run_with_spec(&ctx, &[]);
    assert_eq!(
        default_value, SHADER_DEFAULT,
        "empty spec_constants ⇒ the shader default SPEC_N=3; got {default_value}"
    );

    // 2. Override constant_id 0 ⇒ 7.
    let overridden = run_with_spec(&ctx, &[SpecConstant { id: 0, value: OVERRIDE_VALUE }]);
    assert_eq!(
        overridden, OVERRIDE_VALUE,
        "SpecConstant {{ id: 0, value: 7 }} ⇒ the pipeline specializes SPEC_N to 7; got {overridden}"
    );

    println!(
        "Rung 1a OK: empty ⇒ {default_value} (default, null path), override id0=7 ⇒ {overridden} — spec constants honored on HW",
    );

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(&ctx);
    drop(ctx);
}
