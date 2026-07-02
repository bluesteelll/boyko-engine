//! Phase-6 Slice S1 — RUNG 7 acceptance: the FIRST REAL 3D FRAME ON SCREEN. A
//! depth-tested, MVP-transformed triangle is rendered DIRECTLY INTO the windowed
//! swapchain image (not an offscreen texture) via Vulkan 1.3 dynamic rendering and
//! PRESENTED, for a handful of frames, with the validation layer as the soundness
//! oracle. On ONE frame — before present — the rendered swapchain image is copied
//! back into a host-visible staging buffer and golden-asserted: the centre texel is
//! the geometry's color and a corner is the clear, PROVING real geometry reached the
//! swapchain image rather than just a clear.
//!
//! This extends the Slice-1 `window_present.rs` clear-only present loop (REUSED for
//! window/surface/swapchain/renderer creation + the graceful-skip pattern) to a real
//! scene draw, reusing the committed rung-3/4 MVP vertex+fragment shaders + the
//! rung-4 depth path. It is the on-screen counterpart of the headless
//! `graphics_depth.rs` golden — the same geometry, now targeting the actual
//! swapchain image through the windowed acquire→record→submit→present loop.
//!
//! # The scene (channel-order-insensitive golden)
//!
//! A single covering triangle (model `(0,-1),(1,1),(-1,1)`) at Z = 0.25, colored
//! GREEN `(0,1,0,1)`, transformed by the diagonal MVP `diag(0.7,0.7,1,1)` so it
//! covers the image centre and misses the corners (exactly the rung-3/4 footprint).
//! The clear color is `(0.1, 0.2, 0.1, 1)`. BOTH the geometry color and the clear
//! have RED == BLUE, so the readback bytes are identical whether the swapchain format
//! is `R8G8B8A8` or `B8G8R8A8` — the golden needs no per-format channel-swap branch.
//! The depth attachment (cleared to the far plane 1.0) makes this a genuinely
//! depth-tested 3D frame (the rung-4 pipeline + depth image, now on the swapchain).
//!
//! # CI gate (graceful skip)
//!
//! Mirrors `window_present.rs`: no window / no Vulkan loader / no GPU / no validation
//! SDK / no WSI / no dynamic rendering → an `Err` from window-open / boot / surface /
//! swapchain creation is treated as a SKIP (print + return). The window is
//! short-lived (a handful of frames then torn down) so the test is CI-friendly. The
//! test is `#[cfg(windows)]`; on other targets it is a trivial pass.

#![cfg(windows)]

use core::slice;

use boyko_rhi::{
    BufferDesc, BufferUsage, Format, CullMode, GraphicsPipelineDesc, MemoryLocation, PrimitiveTopology,
    RhiDevice, VertexAttribute, VertexBufferLayout, VertexFormat,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::{
    VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_SRGB,
    VK_FORMAT_R8G8B8A8_UNORM,
};
use boyko_rhi_vulkan::swapchain::{Renderer, Scene, Surface, Swapchain, SCENE_MVP_BYTES};
use boyko_rhi_vulkan::window::Window;

/// The window's client size.
const WIDTH: u32 = 256;
const HEIGHT: u32 = 256;

/// The geometry color bytes (R, G, B, A) — opaque GREEN. R == B (== 0) so the
/// readback is identical under an R8G8B8A8 ↔ B8G8R8A8 channel swap.
const GEOMETRY_BYTES: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];

/// The clear color bytes (R, G, B, A). R == B (== 0x1A) so the corner readback is
/// channel-order-insensitive; distinct from the geometry color so covered vs
/// uncovered cannot be confused. Each byte is `byte / 255.0` so the float→UNORM
/// conversion round-trips exactly.
const CLEAR_BYTES: [u8; 4] = [0x1A, 0x33, 0x1A, 0xFF];

/// The triangle's model-space (and, via the z-passthrough MVP, written) depth.
const SCENE_Z: f32 = 0.25;

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset 12) —
/// the rung-3/4 vertex layout (28-byte stride). `#[repr(C)]` for the exact stride.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 28, "Vertex must be tightly packed at 28 bytes");

/// A byte color → the RGBA floats stored in each vertex / passed as a clear.
fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

/// The deterministic MVP: `diag(0.7, 0.7, 1, 1)` packed (symmetric → storage-
/// convention-insensitive). Scales the model `[-1,1]` x/y to the `[-0.7,0.7]`
/// covering footprint and passes Z through unchanged (z-row `[0,0,1,0]`), so the
/// model Z becomes the written depth.
#[rustfmt::skip]
fn mvp_bytes() -> [u8; SCENE_MVP_BYTES] {
    let m: [f32; 16] = [
        0.7, 0.0, 0.0, 0.0,
        0.0, 0.7, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    let mut out = [0u8; SCENE_MVP_BYTES];
    for (i, f) in m.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

// --- Committed rung-3/4 SPIR-V, reused unchanged (the depth comes from the
//     transformed `gl_Position.z`, so the MVP vertex+fragment shaders suffice). ---

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const
        // u32`; `N` is a 4-byte multiple (const-asserted above), so the blob is
        // exactly `N / 4` whole `u32` words; the `&self` borrow keeps the `'static`
        // blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`, 916 bytes), reused.
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`, 368 bytes), reused.
static MVP_FS_SPV: SpirvBlob<368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.fs.spv"
)));

/// Maps the swapchain's `i32` `VkFormat` to a `boyko_rhi::Format` for the pipeline's
/// declared color attachment (the W2-b contract: the pipeline's color format MUST
/// equal the begin_rendering attachment's format, here the swapchain image). Only the
/// common swapchain formats `pick_surface_format` selects are handled; an unexpected
/// one returns `None` so the test SKIPs rather than faulting validation.
fn swapchain_format_to_rhi(vk_format: i32) -> Option<Format> {
    match vk_format {
        // Declare the EXACT format so the pipeline's color format equals the swapchain
        // image (the W2-b contract). All four common surface formats `pick_surface_format`
        // selects now have a matching `boyko_rhi::Format` variant — the `_SRGB` variants
        // were added so an sRGB-preferring surface is no longer skipped here. NOTE: on an
        // sRGB swapchain the hardware applies linear→sRGB encoding on write; if a windowed
        // golden on such a surface shows shifted colors, the present shader must emit
        // linear values (a separate fix validated by that golden, not by this mapping).
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        f if f == VK_FORMAT_B8G8R8A8_SRGB => Some(Format::B8G8R8A8Srgb),
        f if f == VK_FORMAT_R8G8B8A8_SRGB => Some(Format::R8G8B8A8Srgb),
        _ => None,
    }
}

/// The byte index of texel `(x, y)` in a tightly-packed 4-byte/texel readback of a
/// `w`-wide image.
fn texel_base(x: u32, y: u32, w: u32) -> usize {
    ((y * w + x) * 4) as usize
}

#[test]
fn windowed_scene_present_is_validation_clean_and_renders_geometry() {
    // Open the window first — the surface borrows its HWND/HINSTANCE and must be
    // destroyed before it.
    let mut window = match Window::open("boyko_rhi_vulkan scene window", WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP windowed_scene_present: cannot open a window ({e:?})");
            return;
        }
    };

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP windowed_scene_present: windowed Vulkan unavailable ({e:?})");
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

    // SAFETY: `window` outlives the surface (dropped after it below); its
    // HWND/HINSTANCE are live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_scene_present: surface creation failed ({e:?})");
            return;
        }
    };

    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_scene_present: swapchain creation failed ({e:?})");
            return;
        }
    };
    assert!(swapchain.image_count() >= 1, "swapchain must expose >= 1 image");
    println!(
        "swapchain: {} images, extent {}x{}, format {}",
        swapchain.image_count(),
        swapchain.extent().width,
        swapchain.extent().height,
        swapchain.format()
    );

    // Map the runtime swapchain format to the pipeline's declared color format. An
    // unexpected/SRGB-only format is a graceful SKIP (no matching basic-slice variant).
    let Some(color_format) = swapchain_format_to_rhi(swapchain.format()) else {
        eprintln!(
            "SKIP windowed_scene_present: swapchain format {} (e.g. {VK_FORMAT_B8G8R8A8_SRGB} SRGB) \
             has no basic-slice Format variant",
            swapchain.format()
        );
        return;
    };

    let mut renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");

    // --- Build the scene: a depth-tested MVP pipeline + the covering triangle's
    //     vertex buffer + the diagonal MVP. The pipeline declares the SWAPCHAIN's
    //     color format + D32 depth (W2-b). ---
    let geom = floats(GEOMETRY_BYTES);
    let vertices = [
        Vertex { position: [0.0, -1.0, SCENE_Z], color: geom },
        Vertex { position: [1.0, 1.0, SCENE_Z], color: geom },
        Vertex { position: [-1.0, 1.0, SCENE_Z], color: geom },
    ];
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        &ctx,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    let vb_ptr = RhiDevice::buffer_mapped_ptr(&ctx, &vertex_buffer)
        .expect("host-visible vertex buffer is mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes (the
    // buffer was created at exactly that size); `vertices` is a distinct,
    // non-overlapping stack array of `vertex_bytes` bytes; the write completes before
    // any submit references the buffer (host-coherent: no flush needed).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    let vs = RhiDevice::create_shader_module(&ctx, MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = RhiDevice::create_shader_module(&ctx, MVP_FS_SPV.as_words())
        .expect("fragment shader module");

    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 12, format: VertexFormat::Float32x4 },
    ];
    let pipeline = RhiDevice::create_graphics_pipeline(
        &ctx,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[color_format],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: SCENE_MVP_BYTES as u32,
            bind_group_layout: None,
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        },
    )
    .expect("depth-testing graphics pipeline (swapchain color format)");

    // The shader modules are consumed by pipeline creation; destroy them now (the
    // pipeline retains its own compiled stages).
    // SAFETY: both modules were created on `ctx` above and are no longer needed once
    // the pipeline is created (the pipeline holds its own copy of the compiled code);
    // each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(&ctx, fs);
        RhiDevice::destroy_shader_module(&ctx, vs);
    }

    let mut scene = Scene::new(pipeline, vertex_buffer, 3, mvp_bytes());

    // A host-visible staging buffer sized for one full swapchain image (4 B/texel).
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        &ctx,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    // The extent the staging buffer was sized for. The readback below is requested ONLY
    // while the LIVE swapchain extent still equals this — a window GROW between this
    // alloc and the readback frame would make the copy region (sized at the live extent)
    // overrun the fixed-size staging buffer (review W1/W2).
    let alloc_extent = swapchain.extent();

    // --- Render + present a handful of scene frames. On ONE frame (a presented one,
    //     so the readback reflects what actually reached the screen) request the
    //     swapchain-image readback into `staging`. ---
    let clear = floats(CLEAR_BYTES);
    let mut readback_done = false;
    let mut readback_extent = swapchain.extent();
    for i in 0..5u32 {
        window.pump_events();
        window.refresh_size();

        // Request the readback on a single steady frame (not the first, so the
        // swapchain has settled, and only if not yet captured) — AND only while the live
        // extent still matches the staging-buffer size, so the copy can never overrun it
        // (review W1/W2; a resize simply skips the golden, the present still runs).
        let live = swapchain.extent();
        let extent_stable = live.width == alloc_extent.width && live.height == alloc_extent.height;
        let want_readback = i == 2 && !readback_done && extent_stable;
        let rb = if want_readback { Some(&staging) } else { None };

        // SAFETY: `ctx`/`surface`/`swapchain`/`scene` are live and created on the same
        // device as `renderer`; a `Some(rb)` staging buffer is host-visible and
        // `staging_size` (>= one swapchain image) bytes; the renderer syncs the
        // scene's depth image to the swapchain extent internally.
        let presented = unsafe {
            renderer.render_scene_frame(
                &ctx,
                &surface,
                &mut swapchain,
                &mut scene,
                window.width(),
                window.height(),
                clear,
                rb,
            )
        }
        .unwrap_or_else(|e| panic!("scene frame {i} failed: {e:?}"));

        // A readback is only valid if the frame actually presented (a `false` means
        // the swapchain was recreated this call and the draw was skipped).
        if want_readback && presented {
            readback_done = true;
            readback_extent = swapchain.extent();
        }
    }

    // The oracle: a clean windowed scene render+present records zero validation
    // messages across all frames.
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
        "validation layer reported {} message(s) during the windowed scene render/present — \
         see the [vk-validation] log",
        state.total()
    );

    // The golden: if a readback frame presented, the centre texel must be the geometry
    // color and a corner must be the clear — PROVING real geometry reached the
    // swapchain image (a clear-only path would leave the centre == clear too).
    if readback_done {
        let w = readback_extent.width;
        let h = readback_extent.height;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(&ctx, &staging)
            .expect("host-visible staging buffer is mapped");
        let byte_count = (w * h * 4) as usize;
        let mut out = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (>= `byte_count`) mapped
        // host-coherent bytes; the readback frame's submit completed before this read
        // (the renderer fence-waits the frame slot at the START of each subsequent
        // `render_scene_frame`, and three more frames followed frame 2, so frame 2's
        // copy is complete + coherent); `out` is a distinct, non-overlapping alloc.
        unsafe {
            core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), byte_count);
        }

        let centre = texel_base(w / 2, h / 2, w);
        let centre_texel = [out[centre], out[centre + 1], out[centre + 2], out[centre + 3]];
        assert_eq!(
            centre_texel, GEOMETRY_BYTES,
            "centre texel must be the geometry color (the MVP triangle covers the swapchain centre) — \
             proving real geometry was rendered to the swapchain image, not just cleared: got {centre_texel:02x?}, want {GEOMETRY_BYTES:02x?}"
        );

        let corner = texel_base(0, 0, w);
        let corner_texel = [out[corner], out[corner + 1], out[corner + 2], out[corner + 3]];
        assert_eq!(
            corner_texel, CLEAR_BYTES,
            "corner texel must be the clear color (the triangle does not cover the corner): got {corner_texel:02x?}, want {CLEAR_BYTES:02x?}"
        );
    } else {
        eprintln!(
            "NOTE windowed_scene_present: no readback frame presented (swapchain kept recreating); \
             validation was still asserted clean across all frames"
        );
    }

    // Clean reverse-order teardown: renderer (waits idle) → scene resources → staging
    // → swapchain → surface → window.
    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no
    // submission references the scene's resources; `ctx` is still alive; each scene
    // resource + the staging buffer is destroyed exactly once.
    unsafe {
        scene.destroy(&ctx);
        RhiDevice::destroy_buffer(&ctx, staging);
    }
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}
