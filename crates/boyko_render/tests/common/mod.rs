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
///
/// Under the box-level `BOYKO_DISABLE_VALIDATION` escape hatch (the validation
/// layer is crash-prone on some machines) the context boots WITHOUT the layer, so
/// there is no messenger and no oracle to consult — the check degrades to a noted
/// no-op (mirroring the no-device SKIP convention) instead of failing the suite.
pub fn assert_validation_clean(ctx: &VulkanContext) {
    if !ctx.validation_enabled() {
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) - messenger oracle skipped");
        return;
    }
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

// ---------------------------------------------------------------------------
// UI-ADVANCED S2 (S-D6): the full-readback image pin for the UI GPU goldens.
// ---------------------------------------------------------------------------

/// Writes a 32-bit top-down BMP (RGBA input, swapped to the BGRA byte order BMP
/// stores) — the `ui_hud_screenshot` writer, shared here so the S-D6 bless dump
/// has a human-viewable artifact.
fn write_bmp(path: &std::path::Path, rgba: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    debug_assert_eq!(
        rgba.len(),
        (w * h * 4) as usize,
        "invariant: BMP body is w*h*4 bytes"
    );
    let pixel_bytes = w * h * 4;
    let pixel_offset: u32 = 54; // 14-byte file header + 40-byte info header.
    let file_size = pixel_offset + pixel_bytes;

    let mut buf = Vec::with_capacity(file_size as usize);
    // --- BITMAPFILEHEADER (14 bytes) ---
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    buf.extend_from_slice(&pixel_offset.to_le_bytes());
    // --- BITMAPINFOHEADER (40 bytes) ---
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    buf.extend_from_slice(&(-(h as i32)).to_le_bytes()); // biHeight (negative => top-down)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&pixel_bytes.to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    // --- pixel data: RGBA -> BGRA (the ONLY channel swap; no row flip) ---
    for texel in rgba.chunks_exact(4) {
        buf.extend_from_slice(&[texel[2], texel[1], texel[0], texel[3]]);
    }
    std::fs::write(path, buf)
}

/// The S-D6 image pin: SHA-256 of the WHOLE readback, asserted against the constant the
/// test file carries. A texel assertion cannot see a UV that moved by a texel — which is
/// exactly what D1's un-aliasing does to every glyph; the full-image hash is the cheapest
/// thing that sees it (`docs/UI-PLAN-SPRITES.md` S-D6, mutation M2-b).
///
/// `BOYKO_UI_GOLDEN_BLESS=1` prints the fresh hash and dumps a top-down BMP into
/// `target/screenshots/` for a human to look at, then returns WITHOUT asserting (the
/// bless run of a deliberately changed image must not red before the constant is
/// updated). The texel assertions in each golden stay — they say WHAT is wrong; this
/// hash says THAT something is.
pub fn assert_ui_golden_image_pin(name: &str, rgba: &[u8], w: u32, h: u32, expected_hex: &str) {
    let mut hasher = boyko_render::vg_census::Sha256::new();
    hasher.update(rgba);
    let got = hasher.finish_hex();
    if std::env::var_os("BOYKO_UI_GOLDEN_BLESS").is_some() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("screenshots");
        std::fs::create_dir_all(&dir).expect("create target/screenshots for the bless dump");
        let path = dir.join(format!("{name}.bless.bmp"));
        write_bmp(&path, rgba, w, h).expect("write the S-D6 bless BMP");
        println!("BLESS {name}: sha256 = {got} ({w}x{h} RGBA readback); BMP -> {}", path.display());
        return;
    }
    assert_eq!(
        got, expected_hex,
        "{name}: the full-readback SHA-256 moved (S-D6 image pin, {w}x{h}). Something changed \
         the rendered image that the texel probes did not see. If the change is DELIBERATE \
         (the rung's own text says the golden moves), re-bless with BOYKO_UI_GOLDEN_BLESS=1, \
         LOOK at the dumped BMP, and update the constant in the same commit — otherwise STOP: \
         this is the gate R1 says nothing else can fail."
    );
}
