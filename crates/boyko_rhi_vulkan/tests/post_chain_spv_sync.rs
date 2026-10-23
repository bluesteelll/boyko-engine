//! Render wave PX1a (`px1a/cut.md` C5, D-S2, D-S4) — **the byte gate of the post chain's
//! hand-written shaders**: each committed `.spv` is the re-DXC of its `.hlsl` under the frozen
//! recipe (`-spirv -T <profile> -E main -fspv-target-env=vulkan1.3`, no `-O`; a `-D` only on the
//! sRGB present variant; each source's header states it, and `docs/SHADER-VARIANT-MANIFEST.md` has
//! its row).
//!
//! The post seam moves these sources across PX1a's commits (`taa_resolve` loses `gAaOut` at C5, the
//! transitional pass-through kernel lived C5–C6, the HDR boundary splices land at C7, the sRGB
//! variant at C10), and none of them had a sync gate: a source edit without its re-DXC would ship
//! the old binary with no red anywhere.
//!
//! It also pins every eDSL-owned span spliced into a hand-written shader (`px1a/cut.md` D-S2;
//! orchestrator ruling 67 W5) to its emitter, `boyko_shaderdsl::emit::post_splices()`: the HDR
//! boundary of `taa_resolve` / `ssaa_downsample`, `lit_format.hlsli`'s store bound and the present
//! blit's `srgb_eotf`. The re-DXC test SKIPS with an `eprintln` when no `dxc` resolves — a skipped
//! run is not a pass, and the lane's tester runs it on the workstation, where the pinned VulkanSDK
//! 1.4.350.0 is installed.
//!
//! C10 (D-P1) adds the `*_SRGB` swapchain arm's host gates: the present module is chosen by the
//! swapchain format (an `*_SRGB` format must never get the UNORM module — the double encode), the
//! test-only forced pick takes an sRGB format or refuses, and the variant carries the decode.

use std::path::PathBuf;
use std::process::Command;

mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};

/// `(source, profile, -D defines, committed .spv)`.
const SHADERS: [(&str, &str, &[&str], &str); 8] = [
    ("taa_resolve.comp.hlsl", "cs_6_0", &[], "taa_resolve.comp.spv"),
    ("ssaa_downsample.fs.hlsl", "ps_6_0", &[], "ssaa_downsample.fs.spv"),
    ("fullscreen_sample.fs.hlsl", "ps_6_0", &[], "fullscreen_sample.fs.spv"),
    ("fullscreen_sample.fs.hlsl", "ps_6_0", &["SWAPCHAIN_SRGB=1"], "fullscreen_sample_srgb.fs.spv"),
    ("fxaa.fs.hlsl", "ps_6_0", &[], "fxaa.fs.spv"),
    ("smaa_edge.fs.hlsl", "ps_6_0", &[], "smaa_edge.fs.spv"),
    ("smaa_weight.fs.hlsl", "ps_6_0", &[], "smaa_weight.fs.spv"),
    ("smaa_blend.fs.hlsl", "ps_6_0", &[], "smaa_blend.fs.spv"),
];

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// The pinned SDK's `dxc`, then `$VULKAN_SDK/Bin`, then `PATH`.
fn find_dxc() -> Option<PathBuf> {
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    let pinned = PathBuf::from(format!("C:/VulkanSDK/1.4.350.0/Bin/{bare}"));
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let c = PathBuf::from(sdk).join("Bin").join(bare);
        if c.exists() {
            return Some(c);
        }
    }
    Command::new(bare).arg("--version").output_within(DXC_DEADLINE).is_ok().then(|| PathBuf::from(bare))
}

#[test]
fn every_post_chain_spv_is_the_redxc_of_its_source() {
    let Some(dxc) = find_dxc() else {
        eprintln!("SKIP post_chain_spv_sync: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let dir = shaders_dir();
    let mut stale = Vec::new();
    for (src, profile, defines, spv) in SHADERS {
        let out = child_guard::scratch_path(&format!("{spv}.redxc"));
        let mut cmd = Command::new(&dxc);
        cmd.current_dir(&dir).args(["-spirv", "-T", profile, "-E", "main", "-fspv-target-env=vulkan1.3"]);
        for d in defines {
            cmd.args(["-D", d]);
        }
        let status = cmd
            .arg(src)
            .arg("-Fo")
            .arg(&out)
            .status_within(DXC_DEADLINE)
            .expect("invariant: dxc was located and must run");
        assert!(status.success(), "dxc failed on {src} under the frozen recipe");
        let fresh = std::fs::read(&out).expect("invariant: dxc wrote the re-DXC .spv");
        let _ = std::fs::remove_file(&out);
        let committed = std::fs::read(dir.join(spv)).unwrap_or_else(|e| panic!("{spv}: {e}"));
        if fresh != committed {
            stale.push(format!("{spv} ({} bytes committed, {} re-DXC'd) is stale against {src}", committed.len(), fresh.len()));
        }
    }
    assert!(stale.is_empty(), "re-compile the post-chain shaders under their recipes:\n{}", stale.join("\n"));
}

/// The table names nine distinct `.spv`, each committed beside its source.
#[test]
fn the_shader_table_names_every_committed_spv() {
    for (i, (src, _, _, spv)) in SHADERS.iter().enumerate() {
        assert!(SHADERS[i + 1..].iter().all(|k| k.3 != *spv), "{spv} twice");
        assert!(shaders_dir().join(spv).exists(), "{spv} is not committed");
        assert!(shaders_dir().join(src).exists(), "{src} is not committed");
    }
}

/// D-S5 step 1: the resolve no longer declares or stores the display-referred `gAaOut` copy —
/// `post_final` reads `gHistOut`'s image as the scene input.
#[test]
fn taa_resolve_writes_only_its_history() {
    let src = std::fs::read_to_string(shaders_dir().join("taa_resolve.comp.hlsl")).expect("invariant: the source is committed");
    let code: String = src.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    assert!(!code.contains("gAaOut"), "taa_resolve.comp.hlsl still declares or stores gAaOut outside a comment");
    assert!(!code.contains("register(u4)"), "taa_resolve.comp.hlsl still binds a storage image at @4");
}

/// D-S2 / ruling 67 W5: every eDSL-owned span spliced into a hand-written shader is, byte for
/// byte, its emitter's output (`emit_post --splice` rewrites them; a hand-edit is RED here), and
/// each spliced file holds its span exactly once.
#[test]
fn every_spliced_post_span_is_its_emitter_output() {
    let mut drift = Vec::new();
    for (file, name, body) in boyko_shaderdsl::emit::post_splices() {
        let src = std::fs::read_to_string(shaders_dir().join(file))
            .unwrap_or_else(|e| panic!("{file}: {e}"))
            .replace("\r\n", "\n");
        let begin = format!("// === GENERATED {name} BEGIN ===\n");
        assert_eq!(src.matches(&begin).count(), 1, "{file}: the `{name}` span is not there exactly once");
        let committed =
            boyko_shaderdsl::emit::span_body(&src, name).unwrap_or_else(|| panic!("{file}: no `{name}` END"));
        assert!(!body.is_empty(), "{file}: the `{name}` emitter printed nothing");
        if committed != body {
            drift.push(format!("{file} `{name}`: committed {} bytes, the emitter prints {}", committed.len(), body.len()));
        }
    }
    assert!(
        drift.is_empty(),
        "re-run `cargo run -p boyko_shaderdsl --features emit --bin emit_post -- --splice`:\n{}",
        drift.join("\n")
    );
}

/// The two spans' leaves are the shader's: `taa_resolve` sanitizes every `lit` tap and guards
/// its history write; `ssaa_downsample` sanitizes its four taps; neither keeps a display-space
/// decode / encode (`lit` is scene-linear since the switch commit).
#[test]
fn the_hdr_boundary_is_called_where_the_cut_puts_it() {
    let code = |f: &str| -> String {
        let src = std::fs::read_to_string(shaders_dir().join(f)).expect("invariant: the source is committed");
        src.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
    };
    let taa = code("taa_resolve.comp.hlsl");
    assert_eq!(taa.matches("hdr_sanitize(gLit.Load(").count(), 2, "taa_resolve: the current tap and the 3x3 tap");
    assert!(taa.contains("hdr_history_guard(out_color, cur_lit)"), "taa_resolve: the history write guard");
    assert!(taa.contains("hist_raw *= 1.0 + pre_ratio_delta;"), "taa_resolve: the history re-exposure before the clip");
    let ssaa = code("ssaa_downsample.fs.hlsl");
    assert_eq!(ssaa.matches("hdr_sanitize(g_lit.Load(").count(), 4, "ssaa_downsample: four sanitized taps");
    assert!(!ssaa.contains("pow("), "ssaa_downsample: no gamma decode / encode around a scene-linear box");
}

// ---- C10 (D-P1): the `*_SRGB` swapchain arm --------------------------------------------------------

use boyko_rhi_vulkan::compute::{fullscreen_sample_fs_spirv, fullscreen_sample_srgb_fs_spirv, present_sample_fs_spirv};
use boyko_rhi_vulkan::ffi::{
    VK_COLOR_SPACE_SRGB_NONLINEAR_KHR, VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_SRGB,
    VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_UNDEFINED, VkSurfaceFormatKhr,
};
use boyko_rhi_vulkan::present::{SurfaceFormatPick, choose_surface_format, swapchain_format_is_srgb};

/// `VK_FORMAT_A2B10G10R10_UNORM_PACK32` — a format neither pick prefers (a first-advertised fallback).
const VK_FORMAT_A2B10G10R10_UNORM_PACK32: i32 = 64;

/// The number of `GLSL.std.450 Pow` (`OpExtInst`, opcode 12, instruction 26) in a SPIR-V module.
fn pow_count(words: &[u32]) -> usize {
    let mut i = 5; // past the header
    let mut n = 0;
    while i < words.len() {
        let (count, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(count > 0, "a zero-length SPIR-V instruction at word {i}");
        if op == 12 && count >= 5 && words[i + 4] == 26 {
            n += 1;
        }
        i += count;
    }
    n
}

/// THE C10 red-first: the present blit's module is chosen by the swapchain format — an `*_SRGB`
/// swapchain gets the `srgb_eotf` decode variant (else the hardware's encode on write encodes the
/// display image a second time, about +40 LSB in the mid-tones); a UNORM one the plain sample.
#[test]
fn an_srgb_swapchain_format_selects_the_srgb_present_module() {
    for f in [VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_R8G8B8A8_SRGB] {
        assert!(swapchain_format_is_srgb(f), "VkFormat {f} is an sRGB swapchain format");
        let m = present_sample_fs_spirv(f);
        assert!(
            m != fullscreen_sample_fs_spirv(),
            "VkFormat {f} (an *_SRGB swapchain) selects the UNORM present module: the display is encoded twice"
        );
        assert!(m == fullscreen_sample_srgb_fs_spirv(), "VkFormat {f} selects the srgb_eotf variant");
    }
    for f in [VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_A2B10G10R10_UNORM_PACK32] {
        assert!(!swapchain_format_is_srgb(f), "VkFormat {f} is not an sRGB swapchain format");
        assert!(present_sample_fs_spirv(f) == fullscreen_sample_fs_spirv(), "VkFormat {f} selects the plain sample");
    }
}

fn sf(format: i32) -> VkSurfaceFormatKhr {
    VkSurfaceFormatKhr { format, color_space: VK_COLOR_SPACE_SRGB_NONLINEAR_KHR }
}

/// The test seam (`TestForce` key `srgb_swapchain`): the forced pick takes `B8G8R8A8_SRGB`, else
/// `R8G8B8A8_SRGB`, and REFUSES a surface with neither — so a fixture that forces the sRGB regime
/// can never measure the UNORM one. The production pick is unchanged (UNORM first).
#[test]
fn the_forced_srgb_pick_takes_an_srgb_format_or_refuses() {
    let nl = VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;
    let all = [sf(VK_FORMAT_B8G8R8A8_UNORM), sf(VK_FORMAT_B8G8R8A8_SRGB), sf(VK_FORMAT_R8G8B8A8_SRGB)];
    assert_eq!(choose_surface_format(&all, SurfaceFormatPick::Production), Some((VK_FORMAT_B8G8R8A8_UNORM, nl)));
    assert_eq!(choose_surface_format(&all, SurfaceFormatPick::ForceSrgb), Some((VK_FORMAT_B8G8R8A8_SRGB, nl)));
    let rgba = [sf(VK_FORMAT_R8G8B8A8_UNORM), sf(VK_FORMAT_R8G8B8A8_SRGB)];
    assert_eq!(choose_surface_format(&rgba, SurfaceFormatPick::ForceSrgb), Some((VK_FORMAT_R8G8B8A8_SRGB, nl)));
    let unorm_only = [sf(VK_FORMAT_A2B10G10R10_UNORM_PACK32), sf(VK_FORMAT_B8G8R8A8_UNORM)];
    assert_eq!(choose_surface_format(&unorm_only, SurfaceFormatPick::ForceSrgb), None, "no sRGB format: refused");
    assert_eq!(choose_surface_format(&unorm_only, SurfaceFormatPick::Production), Some((VK_FORMAT_B8G8R8A8_UNORM, nl)));
    let fallback = [sf(VK_FORMAT_A2B10G10R10_UNORM_PACK32)];
    assert_eq!(
        choose_surface_format(&fallback, SurfaceFormatPick::Production),
        Some((VK_FORMAT_A2B10G10R10_UNORM_PACK32, nl)),
        "production falls back to the first advertised format"
    );
    let any = [VkSurfaceFormatKhr { format: VK_FORMAT_UNDEFINED, color_space: nl }];
    assert_eq!(choose_surface_format(&any, SurfaceFormatPick::Production), Some((VK_FORMAT_B8G8R8A8_UNORM, nl)));
    assert_eq!(choose_surface_format(&any, SurfaceFormatPick::ForceSrgb), Some((VK_FORMAT_B8G8R8A8_SRGB, nl)));
    assert_eq!(choose_surface_format(&[], SurfaceFormatPick::Production), None);
}

/// The variant carries the decode and the default module does not: `srgb_eotf`'s three `Pow`s
/// (one per channel) are in the sRGB module only, and the sRGB span sits inside the source's
/// `#if SWAPCHAIN_SRGB` arm (so the default compile cannot pick it up).
#[test]
fn the_srgb_variant_carries_the_decode_and_the_default_module_does_not() {
    assert_eq!(pow_count(fullscreen_sample_fs_spirv()), 0, "the default present module decodes nothing");
    assert_eq!(pow_count(fullscreen_sample_srgb_fs_spirv()), 3, "srgb_eotf: one Pow per channel");
    let src = std::fs::read_to_string(shaders_dir().join("fullscreen_sample.fs.hlsl"))
        .expect("invariant: the source is committed")
        .replace("\r\n", "\n");
    let gate = src.find("#if SWAPCHAIN_SRGB\n").expect("the #if SWAPCHAIN_SRGB arm");
    let span = src.find("// === GENERATED srgb_eotf BEGIN ===").expect("the srgb_eotf span");
    let endif = src[gate..].find("#endif").map(|e| gate + e).expect("the arm's #endif");
    assert!(gate < span && span < endif, "the srgb_eotf span sits inside the #if SWAPCHAIN_SRGB arm");
}
