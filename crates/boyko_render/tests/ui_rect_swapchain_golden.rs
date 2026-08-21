//! GUI P5a part 2 — the INTEGRATED swapchain-hook UI-rect GPU golden (RTX 3060,
//! validation clean).
//!
//! Unlike the part-1 offscreen golden (`ui_rect_gpu_golden.rs`, which drives the
//! `record_ui_rects` recorder directly into a private offscreen scope), THIS golden
//! drives the SHIPPED INTEGRATED LIVE-FRAME PATH end-to-end through the real swapchain
//! present hook:
//!
//! ```text
//! RhiContext::ui_setup(swapchain_format, ui_rect_vs/fs, rows)         // own the UI pipeline + per-FIF rings
//!   -> pack_ui_instance(..) into a Vec<UiInstance>                    // CPU pack (the upload system's core)
//!   -> Renderer::frame_index() + wait_frame_in_flight()              // pick + fence the FIF slot (host_upload_frame contract)
//!   -> RhiContext::ui_upload(&instances, ortho_of_LIVE_extent, fif)  // memcpy into the current-FIF host-mapped ring + POD UiFramePlan
//!   -> RhiContext::ui_pass(&plan)                                    // re-resolve pipeline+bind-group by frame_index (MF-7) -> concrete UiPass
//!   -> Renderer::present_sampled(.., Some(&pass))                    // the swapchain hook: composite scene, then a FRESH begin_rendering(LoadOp::Load) UI sub-pass, ONE draw(6, N, 0, 0)
//!   -> swapchain-image readback -> per-texel golden
//! ```
//!
//! This is the EXACT on-screen entry point the render host calls: it makes the
//! `present_sampled` UI sub-pass (`record_present_sampled`'s `Some(ui)` arm,
//! `swapchain.rs`) LIVE — the part-1 golden never exercised it.
//!
//! # The scene + the proof (Decision-9 gates, all on the integrated path)
//!
//! A SAMPLED composite texture holds a solid OPAQUE BLUE "scene"; the present hook
//! draws it full-extent (`LoadOp::Clear` + the composite), then the UI sub-pass opens
//! its OWN `begin_rendering(LoadOp::Load)` at the FULL swapchain extent and records the
//! UI rects over it. The UI rects (authored straight RGBA8, opaque so premultiply is
//! identity):
//! - a RED rect at `(8,8)` size 16  — position/size + color + ortho-from-live-extent,
//! - a GREEN rect (HIGHER StackIndex) overlapping a BLUE rect (LOWER StackIndex) so the
//!   overlap texel proves painter's z-order (top-most wins),
//! - a YELLOW rect carrying a `ComputedClip` SMALLER than the rect so a texel inside the
//!   rect but OUTSIDE the clip falls back to the BLUE scene (in-shader clip cuts).
//!
//! Decisive per-texel assertions (swapchain BGRA-order-aware, +/-tol):
//! - a RED-rect interior texel == RED          (position + size + color, ortho = LIVE extent),
//! - a GREEN/BLUE overlap texel == GREEN       (StackIndex painter's order, top-most wins),
//! - a clipped-away texel == the BLUE SCENE     (in-shader ComputedClip cut),
//! - a texel covered by NO UI rect == the BLUE SCENE  (LoadOp::Load preserved the scene; the UI pass loaded, did not clear),
//! - the validation messenger == ZERO messages across ALL presented frames (the GPU oracle).
//!
//! # No-realloc (Decision 7)
//!
//! `ui_setup`'s `initial_rows` is set `>=` the instance count, so the per-FIF rings
//! never grow across the multi-frame present loop; a steady-state present does NOT
//! realloc the ring frame-to-frame. (`upload`'s `grow_slot` is `#[cold]` and only runs
//! on overflow; with `initial_rows >= N` it is never reached — asserted indirectly by a
//! clean multi-frame present at a fixed N.)
//!
//! # CI gate (graceful skip)
//!
//! `#[cfg(windows)]`; no window / no Vulkan loader / no GPU / no WSI / a non-UNORM or
//! sub-64 swapchain extent → a graceful SKIP (mirrors `window_present_hybrid.rs`).

#![cfg(windows)]

mod common;

use core::slice;

use boyko_rhi::enums::{AddressMode, BarrierAccess, BarrierStage, DescriptorKind, Filter};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, Format, CullMode, GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc,
    ImageLayout, ImageSubresourceRange, ImageUsage, MemoryLocation, PrimitiveTopology, RhiDevice,
    SamplerDesc, ShaderStage, TextureDesc, TextureDimension,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::{VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_UNORM};
use boyko_rhi_vulkan::swapchain::{FrameWriteToken, Renderer, SampledComposite, Surface, Swapchain};
use boyko_rhi_vulkan::window::Window;

use boyko_render::{
    pack_ui_instance, PackInput, RhiContext, UiInstance, UiOrtho,
};

/// The window's requested client size. The WSI may CLAMP the swapchain extent wider
/// (a driver-minimum surface extent, e.g. 120x64 on this RTX 3060). The ortho is built
/// from the LIVE swapchain extent so a rect lands at the right swapchain texel
/// regardless of the clamp; the UI rects are authored in the top-left `< 64` px region
/// that always fits.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;

/// UI-ADVANCED S2 (S-D6): SHA-256 of the full swapchain readback, blessed on the 64 B
/// `UiInstance` build (commit A of the S2 two-commit protocol) — the widening must
/// reproduce it exactly (gate G2-3). Unlike the offscreen goldens' fixed 64×64 RGBA
/// frames, this readback's extent AND byte order are WSI-decided (the driver clamps
/// the surface extent; the swapchain picks BGRA or RGBA), so the pin carries both and
/// is asserted only when the live frame matches the blessed shape — a different WSI
/// shape gets a loud NOTE, never a silent pass-as-checked. Re-bless:
/// `BOYKO_UI_GOLDEN_BLESS=1`.
const UI_GOLDEN_SHA256: &str = "23145246c9a642c96eb3abce5c0d7a5dbbb0e1bd9febf3499e7e7b0f7bcffdd7";
/// The swapchain extent the hash above was blessed at (WSI-clamped: the RTX 3060
/// driver's minimum surface extent widens the requested 64×64 to 120×64).
const UI_GOLDEN_EXTENT: (u32, u32) = (120, 64);
/// The swapchain readback byte order the hash above was blessed at
/// (`VK_FORMAT_B8G8R8A8_UNORM`, format 44).
const UI_GOLDEN_IS_BGRA: bool = true;

/// Per-channel tolerance on the readback bytes: the float->UNORM sample round-trip of
/// the composite + the premultiplied blend ROP make a bit-exact match brittle; the
/// authored colors differ by 200+ so +/-2/255 still tells them apart unambiguously.
const CHANNEL_TOL: i32 = 2;

/// The opaque BLUE "scene" the composite texture holds (straight RGBA8, premultiply is
/// identity at alpha 255). The UI pass must PRESERVE this where it draws nothing or
/// clips away (LoadOp::Load).
const SCENE_RGBA: [u8; 4] = [0x20, 0x40, 0xFF, 0xFF];
/// UI rect colors (straight RGBA8, opaque).
const RED: u32 = 0xFF00_00FF;
const RED_RGBA: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];
const GREEN: u32 = 0xFF00_FF00;
const GREEN_RGBA: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
/// A LOWER-StackIndex rect under the GREEN one (authored CYAN: byte0=R=00, byte1=G=FF,
/// byte2=B=FF, byte3=A=FF, so a reversed z-order would show cyan at the overlap).
const UNDER: u32 = 0xFFFF_FF00;
/// A clipped rect's fill (YELLOW: R=FF, G=FF, B=00, A=FF).
const YELLOW: u32 = 0xFF00_FFFF;

/// Straight `[R,G,B,A]` bytes of a UI color packed as `R | G<<8 | B<<16 | A<<24`
/// (`byte0=R .. byte3=A`). Decomposed for the per-texel asserts.
fn rgba_bytes(packed: u32) -> [u8; 4] {
    [
        (packed & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        ((packed >> 16) & 0xFF) as u8,
        ((packed >> 24) & 0xFF) as u8,
    ]
}

// --- Committed fullscreen-sample SPIR-V (reused from boyko_rhi_vulkan/shaders), the
//     present hook's composite pass. A 4-byte-aligned wrapper re-viewed as &[u32]. ---

#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const u32`;
        // `N` is a 4-byte multiple (const-asserted); the `&self` borrow keeps the
        // 'static blob alive for the slice; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

static SAMPLE_VS_SPV: SpirvBlob<744> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../boyko_rhi_vulkan/shaders/fullscreen_sample.vs.spv"
)));
static SAMPLE_FS_SPV: SpirvBlob<764> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../boyko_rhi_vulkan/shaders/fullscreen_sample.fs.spv"
)));

/// Maps the swapchain `i32` format to "readback bytes are BGRA" (skips other formats).
fn swapchain_readback_is_bgra(vk_format: i32) -> Option<bool> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(true),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(false),
        _ => None,
    }
}

/// The basic-slice `Format` of the swapchain (for the UI pipeline's `color_format`).
fn swapchain_basic_format(vk_format: i32) -> Option<Format> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }
}

/// Decodes one readback texel `[c0,c1,c2,c3]` to straight `[R,G,B]` applying the
/// swapchain channel order (BGRA reads `[B,G,R,A]`).
fn readback_rgb(texel: [u8; 4], is_bgra: bool) -> [i32; 3] {
    if is_bgra {
        [texel[2] as i32, texel[1] as i32, texel[0] as i32]
    } else {
        [texel[0] as i32, texel[1] as i32, texel[2] as i32]
    }
}

/// `true` if a readback texel agrees with a straight RGBA8 `[R,G,B,A]` golden within
/// `CHANNEL_TOL` per RGB channel (swapchain byte-order-aware; alpha is not on screen).
fn readback_close(texel: [u8; 4], golden_rgba: [u8; 4], is_bgra: bool) -> bool {
    let g = readback_rgb(texel, is_bgra);
    (0..3).all(|c| (g[c] - golden_rgba[c] as i32).abs() <= CHANNEL_TOL)
}

fn assert_readback_close(texel: [u8; 4], golden_rgba: [u8; 4], is_bgra: bool, label: &str) {
    assert!(
        readback_close(texel, golden_rgba, is_bgra),
        "{label}: readback {texel:02x?} (bgra={is_bgra}) != golden {golden_rgba:02x?} within +/-{CHANNEL_TOL}",
    );
}

/// The byte index of texel `(x,y)` in a tightly-packed 4-byte/texel `w`-wide readback.
fn texel_base(x: u32, y: u32, w: u32) -> usize {
    ((y * w + x) * 4) as usize
}

/// One opaque, border-less UI rect at logical-px `(x,y,w,h)` straight `color`, with an
/// optional clip AABB `(x,y,w,h)`. `scale_factor == 1.0` (logical px == physical px).
fn rect(x: f32, y: f32, w: f32, h: f32, color: u32, clip: Option<[f32; 4]>) -> UiInstance {
    pack_ui_instance(
        &PackInput {
            rect: [x, y, w, h],
            color,
            border_color: 0,
            corner_radius: [0.0; 4],
            border_width: [0.0; 4],
            clip,
            text_uv: None,
            image: None,
        },
        1.0,
    )
}

/// Uploads a solid `SCENE_RGBA` fill into the SAMPLED `R8G8B8A8` composite texture
/// once (its own fenced submit), leaving it in `SHADER_READ_ONLY_OPTIMAL`.
fn upload_solid_scene(
    device: &VulkanContext,
    texture: &boyko_rhi_vulkan::texture::VulkanTexture,
    w: u32,
    h: u32,
) {
    use boyko_rhi::RhiQueue;
    let texels = (w * h) as usize;
    let bytes = texels * 4;
    let staging = device
        .create_buffer(&BufferDesc {
            size: bytes as u64,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("scene staging buffer");
    let ptr = device
        .buffer_mapped_ptr(&staging)
        .expect("scene staging mapped");
    let mut host = vec![0u8; bytes];
    for t in 0..texels {
        host[t * 4..t * 4 + 4].copy_from_slice(&SCENE_RGBA);
    }
    // SAFETY: `ptr` maps `bytes` host-coherent bytes; `host` is a distinct `bytes`-long
    // alloc; no GPU work is in flight on `staging` yet (the submit is below).
    unsafe {
        core::ptr::copy_nonoverlapping(host.as_ptr(), ptr.as_ptr(), bytes);
    }

    let queue = device.rhi_queue();
    let fence = device.create_fence(false).expect("scene fence");
    let mut enc = device.create_command_encoder().expect("scene encoder");
    use boyko_rhi::RhiCommandEncoder;
    enc.begin().expect("begin scene");
    enc.image_barrier(&ImageBarrierDesc {
        texture,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::TRANSFER_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::TransferDstOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    let regions = [BufferImageCopy {
        buffer_offset: 0,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: w,
        image_extent_h: h,
        image_extent_d: 1,
    }];
    enc.copy_buffer_to_image(&staging, texture, ImageLayout::TransferDstOptimal, &regions);
    enc.image_barrier(&ImageBarrierDesc {
        texture,
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::FRAGMENT_SHADER,
        src_access: BarrierAccess::TRANSFER_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::TransferDstOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    enc.end().expect("end scene");
    queue.submit(&enc, &fence).expect("submit scene");
    device.wait_fence(&fence, u64::MAX).expect("wait scene");
    // SAFETY: the scene-upload submit completed (fence-waited); encoder/fence/staging
    // were created here and are each destroyed exactly once; the texture is the
    // caller's.
    unsafe {
        device.destroy_command_encoder(enc);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
    }
}

#[test]
fn ui_rects_render_through_the_swapchain_present_hook_golden() {
    let mut window = match Window::open("boyko_render UI swapchain golden", WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP ui_rect_swapchain_golden: cannot open a window ({e:?})");
            return;
        }
    };
    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP ui_rect_swapchain_golden: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    println!("Vulkan device (windowed, validation on): {}", ctx.device_name());
    if !ctx.validation_enabled() {
        // The box-level BOYKO_DISABLE_VALIDATION escape hatch (the validation layer is
        // crash-prone on some machines) removes the layer this gate exists to exercise -
        // SKIP, mirroring the no-device SKIP convention, instead of failing the suite.
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }

    // The UI capability OWNS the `VulkanContext`; the windowed `Surface`/`Swapchain`/
    // `Renderer` borrow the SAME device via `rhi.context()`. Because the windowed
    // handles hold a `&'ctx VulkanContext` borrow for their WHOLE life (used in every
    // present), and `RhiContext::ui_upload` is `&mut self`, the two CANNOT overlap.
    // The integrated path is therefore driven in two phases on ONE device owner:
    //   Phase A (probe): create a throwaway surface+swapchain to read the LIVE extent +
    //     format, then DROP them (NLL releases the `&rhi` borrow).
    //   Phase B (`&mut rhi`): `ui_setup` + the composite-scene resources + `ui_upload`
    //     into BOTH FIF ring slots (the UI scene is static), capturing two POD
    //     `UiFramePlan`s — all the mutable work, BEFORE any windowed handle re-borrows.
    //   Phase C (`&rhi`): re-create surface+swapchain+renderer and present, re-resolving
    //     the UI handles by `frame_index` via `ui_pass` (immutable, coexists with the
    //     windowed handles' immutable borrow). This is the EXACT `present_sampled(..,
    //     Some(&UiPass))` swapchain hook — the live-frame integrated render path.
    let mut rhi = RhiContext::new(ctx);

    // ===== Phase A: probe the live swapchain extent + format, then release. =====
    // SAFETY: `window` outlives the surface (dropped before the window below).
    let (live, swap_format, is_bgra) = {
        let surface = match unsafe { Surface::new(rhi.context(), window.hinstance(), window.hwnd()) } {
            Ok(s) => s,
            Err(e) => {
                eprintln!("SKIP ui_rect_swapchain_golden: surface creation failed ({e:?})");
                return;
            }
        };
        let swapchain = match Swapchain::new(rhi.context(), &surface, window.width(), window.height()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("SKIP ui_rect_swapchain_golden: swapchain creation failed ({e:?})");
                return;
            }
        };
        println!(
            "swapchain: {} images, extent {}x{}, format {}",
            swapchain.image_count(),
            swapchain.extent().width,
            swapchain.extent().height,
            swapchain.format()
        );
        if swapchain.extent().width < WIDTH || swapchain.extent().height < HEIGHT {
            eprintln!(
                "SKIP ui_rect_swapchain_golden: swapchain extent {}x{} is smaller than {WIDTH}x{HEIGHT}",
                swapchain.extent().width,
                swapchain.extent().height,
            );
            return;
        }
        let Some(is_bgra) = swapchain_readback_is_bgra(swapchain.format()) else {
            eprintln!("SKIP ui_rect_swapchain_golden: swapchain format {} has no host-decodable UNORM order", swapchain.format());
            return;
        };
        let Some(swap_format) = swapchain_basic_format(swapchain.format()) else {
            eprintln!("SKIP ui_rect_swapchain_golden: swapchain format has no basic-slice Format variant");
            return;
        };
        (swapchain.extent(), swap_format, is_bgra)
        // `surface` + `swapchain` drop here -> the `&rhi` borrow is released, freeing
        // `rhi` for the Phase-B `&mut` work below.
    };

    // ===== Phase B: the composite scene + the UI capability + the FIF uploads. =====
    // --- The composite "scene": a solid BLUE SAMPLED texture, full swapchain extent so
    //     present_sampled's composite pass writes BLUE across the whole image. The
    //     composite resources are owned handles on the device `rhi` owns; they are torn
    //     down explicitly at the end (after the renderer + UI capability). ---
    let composite_texture = RhiDevice::create_texture(
        rhi.context(),
        &TextureDesc {
            width: live.width,
            height: live.height,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        },
    )
    .expect("SAMPLED composite scene texture");
    upload_solid_scene(rhi.context(), &composite_texture, live.width, live.height);

    let sampler = RhiDevice::create_sampler(
        rhi.context(),
        &SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
            mip: boyko_rhi::MipMode::None,
            compare: None,
        },
    )
    .expect("nearest/clamp sampler");
    let composite_layout = RhiDevice::create_bind_group_layout(
        rhi.context(),
        &BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::CombinedImageSampler,
                stage: ShaderStage::FRAGMENT,
            }],
        },
    )
    .expect("composite bind-group layout");
    let sample_vs = RhiDevice::create_shader_module(rhi.context(), SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vs module");
    let sample_fs = RhiDevice::create_shader_module(rhi.context(), SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fs module");
    let composite_pipeline = RhiDevice::create_graphics_pipeline(
        rhi.context(),
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_format],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: Some(&composite_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        },
    )
    .expect("fullscreen-sample pipeline (swapchain format)");
    // SAFETY: both modules were created above and are consumed by the pipeline; each is
    // destroyed exactly once here.
    unsafe {
        RhiDevice::destroy_shader_module(rhi.context(), sample_fs);
        RhiDevice::destroy_shader_module(rhi.context(), sample_vs);
    }
    let composite_bind_group = RhiDevice::create_bind_group(
        rhi.context(),
        &BindGroupDesc {
            layout: &composite_layout,
            entries: &[BindGroupEntry::CombinedImage {
                texture: &composite_texture,
                sampler: &sampler,
            }],
        },
    )
    .expect("composite bind group");

    let composite = SampledComposite {
        texture: &composite_texture,
        sampler: &sampler,
        bind_group: &composite_bind_group,
        pipeline: &composite_pipeline,
        // The composite texture IS the full live swapchain extent, so the scene fills
        // the whole image (no top-left sub-rect clamp) — the UI then draws over ALL of
        // it under LoadOp::Load.
        texture_extent: live,
    };

    // --- The UI capability setup: build the UI pipeline (swapchain format) + per-FIF
    //     rings on the device `rhi` owns. ---
    // initial_rows >= the instance count below, so the rings NEVER grow across the
    // present loop (Decision 7: no steady-state realloc; grow_slot is never reached).
    const UI_ROWS: u32 = 8;
    // A minimal 1×1 MTSDF font so `ui_setup` can build its 3-binding bind-group (the
    // atlas binding is always present — GUI P5b Decision T4-C). This rect-only golden
    // emits no glyphs, so the atlas content is irrelevant; it just must exist.
    let font = {
        use boyko_fontbake::atlas::{AtlasImage, AtlasKind, AtlasMeta, BakedFont};
        BakedFont {
            meta: AtlasMeta {
                distance_range_texels: 6.0,
                pixels_per_em: 48.0,
                atlas_w: 1,
                atlas_h: 1,
                ascender_em: 0.8,
                descender_em: -0.2,
                line_gap_em: 0.0,
                kind: AtlasKind::Mtsdf,
            },
            glyphs: Vec::new(),
            cmap: Vec::new(),
            kern: Vec::new(),
            atlas: AtlasImage {
                width: 1,
                height: 1,
                pixels: vec![0u8; 4],
            },
        }
    };
    rhi.ui_setup(
        swap_format,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        UI_ROWS,
        Some(&font),
        boyko_render::UiSamplerMode::Smooth,
        // No bindless table on this on-screen harness: the UI gets its private
        // fallback set 1, which is exactly the G3-4 shape a host without a
        // `BindlessTextureTable` boots into.
        None,
    )
    .expect("ui_setup (UI pipeline + per-FIF rings, swapchain format)");

    // The UI scene (authored straight RGBA8, opaque). All within the top-left < 64 px
    // region so they fit any WSI-clamped extent >= 64.
    //   - RED at (8,8) 16x16            -> interior (16,16) == RED (pos+size+color+ortho)
    //   - UNDER (cyan) at (28,28) 16x16, StackIndex 0 (LOW)
    //   - GREEN at (32,32) 16x16, StackIndex 5 (HIGH) overlapping UNDER
    //       overlap interior (36,36) == GREEN (z-order top wins; a reversed order shows cyan)
    //   - YELLOW at (8,40) 16x16 with clip (8,40) 8x16  -> left half fill, right half clipped
    //       (12,48) == YELLOW (inside clip), (20,48) == SCENE (outside clip, in rect)
    //
    // Painter's order: the upload sorts by (StackIndex, append). The UNDER rect has a
    // LOWER StackIndex than GREEN, so GREEN paints last -> wins the overlap. RED and
    // YELLOW do not overlap anything. The array order here IS painter's order (the
    // swapchain hook draws array order back-to-front).
    let instances = [
        rect(8.0, 8.0, 16.0, 16.0, RED, None),
        rect(28.0, 28.0, 16.0, 16.0, UNDER, None), // StackIndex 0 (drawn first)
        rect(32.0, 32.0, 16.0, 16.0, GREEN, None), // StackIndex 5 (drawn last -> on top)
        rect(8.0, 40.0, 16.0, 16.0, YELLOW, Some([8.0, 40.0, 8.0, 16.0])),
    ];
    assert!(
        instances.len() as u32 <= UI_ROWS,
        "test invariant: instances fit initial_rows so the ring never grows"
    );

    // The UI scene is STATIC, so upload it into BOTH FIF ring slots up front (no GPU is
    // reading either slot yet — the present loop has not started). The ortho denominator
    // is the LIVE swapchain extent (Decision 9). This front-loads ALL `&mut rhi` work so
    // the Phase-C windowed handles (immutable `&rhi`) and `ui_pass` (immutable) can run
    // without a mutable/immutable borrow clash.
    let ortho = UiOrtho::for_extent(live.width, live.height);
    let mut plans: [Option<boyko_render::UiFramePlan>; boyko_render::UI_FRAMES_IN_FLIGHT] =
        [None; boyko_render::UI_FRAMES_IN_FLIGHT];
    for (fif, slot) in plans.iter_mut().enumerate() {
        // SAFETY: setup-time seeding — the present loop has not started, so no submitted
        // GPU work references either FIF ring slot.
        let token = unsafe { FrameWriteToken::forge_unfenced(fif) };
        let plan = rhi
            .ui_upload(&instances, ortho, &token)
            .expect("ui_upload into the FIF ring slot");
        assert_eq!(plan.instance_count, instances.len() as u32, "all instances uploaded");
        assert_eq!(plan.frame_index, fif, "the plan carries the FIF slot index");
        *slot = Some(plan);
    }
    let plans = plans.map(|p| p.expect("every FIF slot uploaded"));

    // ===== Phase C: re-create the windowed handles + present through the hook. =====
    // SAFETY: `window` outlives the surface (dropped before the window below).
    let surface = unsafe { Surface::new(rhi.context(), window.hinstance(), window.hwnd()) }
        .expect("re-create surface for the present loop");
    let mut swapchain = Swapchain::new(rhi.context(), &surface, window.width(), window.height())
        .expect("re-create swapchain for the present loop");
    assert_eq!(
        swapchain.extent().width, live.width,
        "the re-created swapchain extent must match the probe extent (stable surface)"
    );
    assert_eq!(swapchain.extent().height, live.height, "re-created swapchain height stable");
    println!(
        "phase-C swapchain: {} images, extent {}x{}, format {}",
        swapchain.image_count(),
        swapchain.extent().width,
        swapchain.extent().height,
        swapchain.format()
    );
    let mut renderer =
        Renderer::new(rhi.context(), &surface, &swapchain).expect("renderer (command pool + sync)");

    // --- Present a handful of frames; request the swapchain readback on ONE. ---
    let mut readback_done = false;
    let mut readback_extent = swapchain.extent();
    let staging_size = (live.width * live.height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        rhi.context(),
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("readback staging buffer");
    // Sentinel-fill the staging (0xCD) — the anti-stale tripwire. The shared host block
    // RECYCLES freed ranges: this staging lands where Phase B's scene-upload staging was
    // freed (shifted by ui_setup's 1088 B of carve-outs), so its INITIAL mapped contents
    // are the STALE host-authored scene bytes — which once masqueraded as a rendered
    // frame while the GPU copy had not even executed (the host read raced the readback
    // frame's fence). The sentinel makes any such unexecuted-copy read unmistakable, and
    // the post-wait assert below trips on it instead of chasing ghost pixels.
    {
        let p = RhiDevice::buffer_mapped_ptr(rhi.context(), &staging).expect("staging mapped");
        // SAFETY: `p` maps `staging_size` host-coherent bytes; no GPU work references
        // the staging yet (created just above, first submit is below).
        unsafe { core::ptr::write_bytes(p.as_ptr(), 0xCD, staging_size as usize) };
    }

    for i in 0..5u32 {
        window.pump_events();
        window.refresh_size();

        let cur = swapchain.extent();
        let extent_stable = cur.width == live.width && cur.height == live.height;

        // The host contract: FENCE the current FIF slot (the ring was last read two
        // presents back) — the minted token carries the slot index — then re-resolve
        // the UI handles by frame_index (MF-7) into the concrete UiPass for THAT
        // slot's pre-uploaded plan. The token is consumed by `present_sampled` below
        // (R0b: the by-value consume ends this frame's host-write window).
        let token = renderer
            .wait_frame_in_flight()
            .expect("wait the current FIF slot's in-flight fence");
        let fif = token.slot();
        let pass = rhi.ui_pass(&plans[fif]).expect("ui_pass after ui_setup");

        let want_readback = i == 3 && !readback_done && extent_stable;
        let rb = if want_readback { Some(&staging) } else { None };

        // SAFETY: surface/swapchain are live on the same device as `renderer`; the
        // composite resources are live + the texture is resident SHADER_READ_ONLY;
        // `pass`'s pipeline/bind-group are re-resolved live from `rhi` (the RhiContext
        // outlives this submit) with the swapchain color format; a `Some(rb)` staging
        // buffer is host-visible and >= one swapchain image.
        let presented = unsafe {
            renderer.present_sampled(
                token,
                &surface,
                &mut swapchain,
                &composite,
                window.width(),
                window.height(),
                [0.0; 4],
                rb,
                Some(&pass),
            )
        }
        .unwrap_or_else(|e| panic!("UI present frame {i} failed: {e:?}"));

        if want_readback && presented {
            readback_done = true;
            readback_extent = swapchain.extent();
        }
    }

    // The oracle: a clean integrated UI present records ZERO validation messages. Gated on
    // `validation_enabled()` (the window_present_gbuffer precedent) so the pixel golden below
    // still runs under the BOYKO_DISABLE_VALIDATION escape hatch (no messenger exists then).
    if rhi.context().validation_enabled() {
        let state = rhi
            .context()
            .debug_state()
            .expect("validation enabled => a debug-messenger state is present");
        assert_eq!(
            state.total(),
            0,
            "validation reported {} message(s) during the integrated UI present — see [vk-validation]",
            state.total()
        );
    } else {
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) - messenger oracle skipped");
    }

    // The golden: the readback frame's swapchain image must show the UI rects over the
    // preserved BLUE scene.
    assert!(
        readback_done,
        "no readback frame presented (swapchain kept recreating) — cannot assert the UI golden"
    );
    // FENCE THE READBACK before touching the staging: drop the renderer NOW — its Drop
    // waits the device idle, which completes the readback frame's whole submission
    // (composite + UI draws + the image→buffer copy). The old code read the staging
    // right here with only ONE later present having run: that present fence-waited the
    // SIBLING slot, so frame 3's copy was never waited — under validation pacing the
    // host reliably read the staging's initial (stale recycled) bytes and the golden
    // chased a frame that was never there (proven by an all-0xCD sentinel readback).
    // Correctness by construction beats fence arithmetic: wait-idle, then read.
    drop(renderer);
    let w = readback_extent.width;
    let h = readback_extent.height;
    let dst = RhiDevice::buffer_mapped_ptr(rhi.context(), &staging).expect("staging mapped");
    let byte_count = (w * h * 4) as usize;
    let mut out = vec![0u8; byte_count];
    // SAFETY: `dst` maps `staging_size` (>= byte_count) host-coherent bytes; the
    // renderer was dropped above (device wait-idle), so the readback frame's copy is
    // complete + coherent; `out` is a distinct alloc.
    unsafe {
        core::ptr::copy_nonoverlapping(dst.as_ptr(), out.as_mut_ptr(), byte_count);
    }
    // The tripwire pairs with the 0xCD sentinel fill at creation: an all-sentinel
    // readback means the copy never executed before this read (a sync bug in the test
    // or the present path) — fail HERE with the true cause, not on a pixel mismatch.
    assert!(
        out.iter().any(|&b| b != 0xCD),
        "readback staging still holds the creation sentinel — the image→buffer copy never \
         executed before the host read (readback-fence sync bug)"
    );
    // Diagnostic raw-frame dump (env-gated): the whole readback as raw bytes so an
    // external histogram can classify a failing frame without single-texel guesswork.
    if let Some(p) = std::env::var_os("BOYKO_UIRECT_DUMP") {
        std::fs::write(&p, &out).expect("raw readback dump");
        println!("uirect raw dump -> {:?} ({}x{})", p, w, h);
    }
    let read = |x: u32, y: u32| -> [u8; 4] {
        let b = texel_base(x, y, w);
        [out[b], out[b + 1], out[b + 2], out[b + 3]]
    };

    // (G1+G2+G11) RED rect interior (16,16) == RED — placed by the SSBO min/size under
    // the ortho built from the LIVE swapchain extent (Decision 9).
    assert_readback_close(read(16, 16), RED_RGBA, is_bgra, "RED rect interior (pos+size+color, ortho=live extent)");

    // (G5) GREEN over UNDER overlap interior (36,36) == GREEN — painter's z-order, the
    // HIGHER StackIndex wins; a reversed order would show the cyan UNDER rect.
    assert_readback_close(read(36, 36), GREEN_RGBA, is_bgra, "GREEN/UNDER overlap (StackIndex z-order, top wins)");

    // (G6) clipped-away texel (20,48): inside the YELLOW rect (x in [8,24)) but OUTSIDE
    // its clip (x in [8,16)) -> the in-shader clip cut it, so the BLUE SCENE shows.
    assert_readback_close(read(20, 48), SCENE_RGBA, is_bgra, "clipped-away texel == BLUE scene (in-shader ComputedClip cut)");
    // ...and a texel INSIDE the clip (12,48) == YELLOW (the clip kept the left half).
    assert_readback_close(read(12, 48), rgba_bytes(YELLOW), is_bgra, "in-clip texel == YELLOW fill");

    // (Decision 9 LoadOp::Load) a texel covered by NO UI rect keeps the BLUE SCENE — the
    // UI pass LOADED the composited scene, did not clear it.
    assert_readback_close(read(2, 2), SCENE_RGBA, is_bgra, "uncovered texel == BLUE scene (UI pass LoadOp::Load preserved it)");

    // S-D6: the full-image pin, gated on the WSI shape it was blessed at (see the
    // constant's doc). In bless mode the helper prints the hash and this arm prints the
    // shape to record beside it.
    if std::env::var_os("BOYKO_UI_GOLDEN_BLESS").is_some() {
        println!(
            "BLESS ui_rect_swapchain_golden: extent {w}x{h}, is_bgra = {is_bgra} (the hash is \
             over the RAW readback bytes; the BMP dump assumes RGBA, so R/B appear swapped \
             in the viewer when is_bgra — the geometry is what to eyeball)"
        );
        common::assert_ui_golden_image_pin("ui_rect_swapchain_golden", &out, w, h, UI_GOLDEN_SHA256);
    } else if (w, h) == UI_GOLDEN_EXTENT && is_bgra == UI_GOLDEN_IS_BGRA {
        common::assert_ui_golden_image_pin("ui_rect_swapchain_golden", &out, w, h, UI_GOLDEN_SHA256);
    } else {
        eprintln!(
            "NOTE ui_rect_swapchain_golden: live readback {w}x{h} bgra={is_bgra} differs from \
             the blessed WSI shape {UI_GOLDEN_EXTENT:?} bgra={UI_GOLDEN_IS_BGRA} — the S-D6 \
             image hash was NOT checked on this host (the texel asserts above did run)"
        );
    }

    // Clean reverse-order teardown. The renderer was ALREADY dropped before the readback
    // read above (the wait-idle that fences the copy); the remaining windowed handles
    // drop here, THEN the UI capability (`destroy_all`, also waits idle), THEN the
    // composite resources, THEN `rhi`.
    drop(swapchain);
    drop(surface);
    rhi.destroy_all();
    let device = rhi.context();
    // SAFETY: the renderer was dropped (waits idle) and the UI capability drained
    // (destroy_all waits idle); no submission references these; the device is still
    // alive (owned by `rhi.context`); each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_buffer(device, staging);
        RhiDevice::destroy_bind_group(device, composite_bind_group);
        RhiDevice::destroy_graphics_pipeline(device, composite_pipeline);
        RhiDevice::destroy_bind_group_layout(device, composite_layout);
        RhiDevice::destroy_sampler(device, sampler);
        RhiDevice::destroy_texture(device, composite_texture);
    }
    drop(rhi);
    drop(window);
}
