//! RK-16 (render-wave RD3): an indirect compute dispatch recorded THROUGH the RHI trait reaches
//! `vkCmdDispatchIndirect`, on both binding paths the Vulkan encoder has.
//!
//! # What is asserted
//!
//! The probe kernel (`shaders/rk16_indirect_probe.comp.hlsl`) runs one thread per workgroup and
//! writes `group + 1` into `Out[group]` for the first [`SLOTS`] groups. The args buffer holds a
//! DECOY triple `(5, 1, 1)` at byte 0 and the real `(3, 1, 1)` at byte [`ARGS_OFFSET`]; the
//! dispatch names [`ARGS_OFFSET`]. The readback is the oracle and it tells the failures apart:
//!
//! - `[1, 2, 3, 0, 0, 0, 0, 0]`: three groups ran, from the args at `offset` (green);
//! - all zeros: no group ran (the `#[cold]` no-op trait default, RK-16);
//! - `[1, 2, 3, 4, 5, 0, 0, 0]`: the decoy ran (an override that ignores `offset`).
//!
//! Each case also asserts that the validation messenger recorded ZERO messages. An indirect
//! dispatch recorded without the packed path's fixed-set rebind runs with set 0 UNBOUND (`begin`
//! resets the cached binding), which validation reports and the readback cannot be relied on to
//! show; an unconditional rebind on the vocabulary path clobbers the vocabulary set, which
//! validation also reports. So both cases are validation-ON gates.
//!
//! - [`rk16_packed_dispatch_indirect`]: `bind_storage_buffer` → `dispatch_indirect` (the shared
//!   pre-dispatch rebind runs).
//! - [`rk16_vocabulary_dispatch_indirect`]: `bind_descriptor_set_compute` → `dispatch_indirect`
//!   (the rebind is skipped and the vocabulary set stays bound).
//!
//! # The readback route (OBS host-read rule)
//!
//! The probe output is not read through its own mapping. A `COMPUTE_SHADER/SHADER_WRITE →
//! TRANSFER/TRANSFER_READ` barrier, then the trait's `copy_buffer` into a MAPPED readback buffer:
//! the Vulkan `copy_buffer` records the `TRANSFER → HOST` barrier itself for a mapped destination
//! (`rhi_impl/encoder.rs`, `host_read.rs`), which the trait vocabulary cannot name directly. The
//! readback is pre-filled with [`SENTINEL`], so a copy that never landed is a fixture failure
//! distinct from "no group ran".
//!
//! # How to run (and why the device cases never SKIP)
//!
//! ```text
//! cargo test -p boyko_rhi_vulkan --test rk16_dispatch_indirect -- --ignored --test-threads=1
//! ```
//!
//! with `BOYKO_DISABLE_VALIDATION` UNSET (not merely empty — the boot tests `var_os(..).is_some()`).
//! The device cases are run only deliberately, so they PANIC instead of skipping when there is no
//! device, no validation layer, or the escape hatch is set: a skip would be a vacuous green. A
//! device sweep that sets `BOYKO_DISABLE_VALIDATION=1` for its other binaries must run this one
//! without it.
//!
//! Without `--ignored` the binary runs one plain test, the probe's `.spv` byte gate
//! ([`rk16_indirect_probe_spv_matches_source`]): `running 3 tests`, `1 passed; 0 failed; 2 ignored`.

use core::ptr::NonNull;
use std::path::{Path, PathBuf};
use std::process::Command;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BarrierDesc, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry,
    BufferBarrier, BufferCopy, BufferDesc, BufferUsage, ComputePipelineDesc, DescriptorKind,
    MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The committed probe module, loaded without touching `compute.rs` (its byte gate is
/// [`rk16_indirect_probe_spv_matches_source`]).
const PROBE_SPV: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/rk16_indirect_probe.comp.spv"));

/// The probe's output slots (`RK16_SLOTS` in the kernel).
const SLOTS: usize = 8;
/// The output buffer's byte size.
const OUT_BYTES: u64 = (SLOTS * 4) as u64;
/// The decoy group counts at byte 0 of the args buffer: an override that drops `offset` runs these.
const DECOY: [u32; 3] = [5, 1, 1];
/// The real group counts, at [`ARGS_OFFSET`].
const ARGS: [u32; 3] = [3, 1, 1];
/// The buffer-relative byte offset of [`ARGS`]: 4-aligned, and `ARGS_OFFSET + 12 <= ARGS_BYTES`.
const ARGS_OFFSET: u64 = 16;
/// The args buffer's byte size: the decoy at 0, padding, the real triple at 16, padding.
const ARGS_BYTES: u64 = 32;
/// Three groups from [`ARGS`] wrote `group + 1`.
const EXPECTED: [u32; SLOTS] = [1, 2, 3, 0, 0, 0, 0, 0];
/// The five groups of [`DECOY`] wrote `group + 1`.
const DECOY_READ: [u32; SLOTS] = [1, 2, 3, 4, 5, 0, 0, 0];
/// The readback's pre-fill: any slot still holding it means the copy did not land.
const SENTINEL: u32 = 0xDEAD_BEEF;

/// Which binding path the dispatch is recorded on.
#[derive(Clone, Copy, Debug)]
enum ProbePath {
    /// `bind_storage_buffer` (the fixed set, rebound before the dispatch).
    Packed,
    /// `bind_descriptor_set_compute` (a vocabulary bind group; no fixed-set rebind).
    Vocabulary,
}

/// What one probe run observed.
struct ProbeRun {
    readback: [u32; SLOTS],
    validation_messages: u32,
}

/// The committed module as SPIR-V words (`include_bytes!` gives no 4-byte alignment, so the words
/// are assembled rather than reinterpreted).
fn probe_words() -> Vec<u32> {
    let (words, rest) = PROBE_SPV.as_chunks::<4>();
    assert!(
        rest.is_empty(),
        "invariant: a SPIR-V module is a whole number of words ({} bytes)",
        PROBE_SPV.len()
    );
    words.iter().map(|w| u32::from_le_bytes(*w)).collect()
}

/// Boots a validation-ON context or PANICS: these cases are run deliberately, so a skip would be
/// a vacuous green (module doc).
fn boot_validation_on(test: &str) -> VulkanContext {
    assert!(
        std::env::var_os("BOYKO_DISABLE_VALIDATION").is_none(),
        "{test}: BOYKO_DISABLE_VALIDATION is set (even an empty value disables validation); this \
         gate is validation-ON - run it with the variable unset"
    );
    let ctx = VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    })
    .unwrap_or_else(|e| {
        panic!("{test}: no Vulkan device or validation layer ({e:?}); this gate does not skip")
    });
    assert!(ctx.validation_enabled(), "{test}: the context booted without the validation layer");
    ctx
}

/// Writes `words` at the start of a host-coherent mapping.
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let base = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: every caller passes a mapping of at least `words.len() * 4` bytes, so `base + i`
        // is in bounds; no GPU work is in flight while the host seeds it; `write_unaligned`
        // tolerates the sub-allocated offset's alignment.
        unsafe { base.add(i).write_unaligned(w) };
    }
}

/// Reads [`SLOTS`] words from the start of a host-coherent mapping.
fn read_slots(base: NonNull<u8>) -> [u32; SLOTS] {
    let base = base.as_ptr().cast::<u32>();
    core::array::from_fn(|i| {
        // SAFETY: the readback buffer is `OUT_BYTES` = `SLOTS * 4` bytes, so `base + i` for
        // `i < SLOTS` is in bounds; the fence wait completed the copy and the `TRANSFER → HOST`
        // barrier recorded after it, so the bytes are visible and stable; `read_unaligned`
        // tolerates the sub-allocated offset's alignment.
        unsafe { base.add(i).read_unaligned() }
    })
}

/// Records and runs the probe once on `path`, through the trait only.
fn run_probe(test: &str, path: ProbePath) -> ProbeRun {
    let ctx = boot_validation_on(test);
    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    let args = device
        .create_buffer(&BufferDesc {
            size: ARGS_BYTES,
            usage: BufferUsage::INDIRECT,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("args buffer");
    let out = device
        .create_buffer(&BufferDesc {
            size: OUT_BYTES,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("probe output buffer");
    let readback = device
        .create_buffer(&BufferDesc {
            size: OUT_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("readback buffer");
    write_words(
        device.buffer_mapped_ptr(&args).expect("args buffer is mapped"),
        &[DECOY[0], DECOY[1], DECOY[2], 0, ARGS[0], ARGS[1], ARGS[2], 0],
    );
    write_words(device.buffer_mapped_ptr(&out).expect("output is mapped"), &[0; SLOTS]);
    let readback_ptr = device.buffer_mapped_ptr(&readback).expect("readback is mapped");
    write_words(readback_ptr, &[SENTINEL; SLOTS]);

    let words = probe_words();
    let module = device.create_shader_module(&words).expect("probe shader module");
    let layout = match path {
        ProbePath::Packed => None,
        ProbePath::Vocabulary => Some(
            device
                .create_bind_group_layout(&BindGroupLayoutDesc {
                    entries: &[BindGroupLayoutEntry {
                        binding: 0,
                        count: 1,
                        kind: DescriptorKind::StorageBuffer,
                        stage: ShaderStage::COMPUTE,
                    }],
                })
                .expect("probe bind group layout"),
        ),
    };
    let bind_group = layout.as_ref().map(|layout| {
        device
            .create_bind_group(&BindGroupDesc {
                layout,
                entries: &[BindGroupEntry::StorageBuffer { buffer: &out }],
            })
            .expect("probe bind group")
    });
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            // The RHI requires a non-empty multiple-of-4 push range; the probe reads none.
            push_constant_bytes: 4,
            bind_group_layout: layout.as_ref(),
            spec_constants: &[],
        })
        .expect("probe pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    encoder.begin().expect("begin");
    encoder.bind_compute_pipeline(&pipeline);
    match &bind_group {
        None => encoder.bind_storage_buffer(&out, 0, 0),
        Some(group) => encoder.bind_descriptor_set_compute(group, &pipeline),
    }
    encoder.dispatch_indirect(&args, ARGS_OFFSET);
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        buffers: &[BufferBarrier {
            buffer: &out,
            src_access: BarrierAccess::SHADER_WRITE,
            dst_access: BarrierAccess::TRANSFER_READ,
        }],
    });
    encoder.copy_buffer(&out, &readback, &[BufferCopy { src_offset: 0, dst_offset: 0, size: OUT_BYTES }]);
    encoder.end().expect("end");
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let run = ProbeRun {
        readback: read_slots(readback_ptr),
        validation_messages: ctx
            .debug_state()
            .expect("validation is on, so a debug-messenger state is present")
            .total(),
    };

    // SAFETY: every resource below was created on `device` and is destroyed exactly once; the
    // only submission completed (fence-waited above), so none is in use by the GPU.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        if let Some(group) = bind_group {
            device.destroy_bind_group(group);
        }
        if let Some(layout) = layout {
            device.destroy_bind_group_layout(layout);
        }
        device.destroy_shader_module(module);
        device.destroy_buffer(readback);
        device.destroy_buffer(out);
        device.destroy_buffer(args);
    }
    drop(ctx);
    run
}

/// Asserts the probe's readback and the validation count, naming the failure class.
fn assert_probe(test: &str, path: ProbePath, run: &ProbeRun) {
    let got = run.readback;
    let diagnosis = if got == EXPECTED {
        "three groups ran, from the args at `offset`"
    } else if got == [0; SLOTS] {
        "zero groups wrote - dispatch_indirect recorded nothing (the trait default is a no-op, RK-16)"
    } else if got == DECOY_READ {
        "five groups wrote - the override read the decoy at byte 0 instead of `offset`"
    } else if got.contains(&SENTINEL) {
        "the readback still holds the sentinel - the copy into the readback did not land (fixture)"
    } else {
        "an unexpected readback"
    };
    assert_eq!(
        got, EXPECTED,
        "{test} ({path:?}): {diagnosis}; {} validation message(s) (see the [vk-validation] log)",
        run.validation_messages
    );
    assert_eq!(
        run.validation_messages, 0,
        "{test} ({path:?}): the readback is right but the validation layer reported {} message(s) \
         - see the [vk-validation] log",
        run.validation_messages
    );
}

/// Case 1, the packed path: `bind_storage_buffer` → `dispatch_indirect`. Covers the shared
/// pre-dispatch fixed-set rebind and the buffer-relative offset.
#[test]
#[ignore = "gpu: a Vulkan device with the validation layer, BOYKO_DISABLE_VALIDATION unset; run per binary with -- --ignored --test-threads=1, validation ON"]
fn rk16_packed_dispatch_indirect() {
    let test = "rk16_packed_dispatch_indirect";
    let run = run_probe(test, ProbePath::Packed);
    assert_probe(test, ProbePath::Packed, &run);
}

/// Case 2, the vocabulary path: `bind_descriptor_set_compute` → `dispatch_indirect`. Covers the
/// rebind helper's skip: with no `bind_storage_buffer` in the recording the fixed set must not be
/// bound over the vocabulary set.
#[test]
#[ignore = "gpu: a Vulkan device with the validation layer, BOYKO_DISABLE_VALIDATION unset; run per binary with -- --ignored --test-threads=1, validation ON"]
fn rk16_vocabulary_dispatch_indirect() {
    let test = "rk16_vocabulary_dispatch_indirect";
    let run = run_probe(test, ProbePath::Vocabulary);
    assert_probe(test, ProbePath::Vocabulary, &run);
}

// ── The probe's byte gate ────────────────────────────────────────────────────────────────

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC runs).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates `dxc`: the pinned Vulkan-SDK path first, then `$VULKAN_SDK/Bin`, then `PATH` (the
/// `sdf_mesh_shadow_spv_sync.rs` idiom). `None` makes the byte gate SKIP.
fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output_within(DXC_DEADLINE).is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Re-DXCs the probe under the frozen recipe in its header (`-spirv -T cs_6_0 -E main
/// -fspv-target-env=vulkan1.3`, no `-O`, no `-D`) into a fresh scratch file and returns the bytes.
/// The scratch path is checked absent before the compile, so a recipe whose `-Fo` pointed
/// anywhere else (the committed artifact included) fails here instead of comparing a file with
/// itself.
fn redxc(dxc: &Path, dir: &Path, committed: &Path) -> Vec<u8> {
    let out_spv = child_guard::scratch_path("rk16_indirect_probe.comp.spv.redxc.spv");
    assert!(!out_spv.exists(), "invariant: the scratch path is fresh");
    assert_ne!(&*out_spv, committed, "invariant: the re-DXC never writes the committed artifact");
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir).args(["-spirv", "-T", "cs_6_0", "-E", "main"]);
    cmd.args(["-fspv-target-env=vulkan1.3", "rk16_indirect_probe.comp.hlsl", "-Fo"]).arg(&out_spv);
    let status = cmd.status_within(DXC_DEADLINE).expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling rk16_indirect_probe.comp.hlsl under the frozen recipe");
    std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv to the scratch path")
}

/// `(opcode, word count)` of every instruction after the 5-word SPIR-V header.
fn instructions(words: &[u32]) -> Vec<(u32, &[u32])> {
    let mut out = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count > 0 && at + count <= words.len(), "malformed SPIR-V at word {at}");
        out.push((words[at] & 0xFFFF, &words[at..at + count]));
        at += count;
    }
    out
}

/// The committed `.spv` is the re-DXC of its source, and it is the probe rather than a module
/// that compiled to nothing.
#[test]
fn rk16_indirect_probe_spv_matches_source() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP rk16_indirect_probe_spv_matches_source: no dxc on this host");
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("rk16_indirect_probe.comp.spv");
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    assert!(committed == PROBE_SPV, "the include_bytes! copy is the committed file (rebuild)");
    let fresh = redxc(&dxc, &dir, &committed_path);
    assert!(
        committed == fresh,
        "rk16_indirect_probe.comp.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of \
         its source under the frozen recipe - either the committed .spv is stale (re-run the \
         recipe in the shader's header and commit it) or the host dxc is not the pinned \
         VulkanSDK 1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );

    // Anti-vacuity, proportionate to a ten-line kernel: a valid module with a GLCompute entry
    // point named `main`, a binding-0 resource, and at least one `OpStore` (the write the readback
    // observes). A body that compiled away (a macro typo) has no store.
    let words = probe_words();
    assert_eq!(words.first(), Some(&0x0723_0203), "SPIR-V magic number");
    let insts = instructions(&words);
    const OP_ENTRY_POINT: u32 = 15;
    const OP_DECORATE: u32 = 71;
    const OP_STORE: u32 = 62;
    const EXECUTION_MODEL_GLCOMPUTE: u32 = 5;
    const DECORATION_BINDING: u32 = 33;
    let main_le = u32::from_le_bytes(*b"main");
    assert!(
        insts.iter().any(|(op, w)| *op == OP_ENTRY_POINT
            && w.get(1) == Some(&EXECUTION_MODEL_GLCOMPUTE)
            && w.get(3) == Some(&main_le)),
        "the probe declares a GLCompute entry point named `main`"
    );
    assert!(
        insts.iter().any(|(op, w)| *op == OP_DECORATE
            && w.get(2) == Some(&DECORATION_BINDING)
            && w.get(3) == Some(&0)),
        "the probe declares a binding-0 resource"
    );
    assert!(
        insts.iter().any(|(op, _)| *op == OP_STORE),
        "the probe stores nothing - its body compiled away"
    );
}

// `status_within` / `output_within` (a deadline on each dxc this file spawns) and `scratch_path`
// (a temp file no other run shares) live in `tests/child_guard/mod.rs`, whose module doc records
// the hung-dxc stall behind them. Declared last, as in every re-DXC gate.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
