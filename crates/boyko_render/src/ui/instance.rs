//! The std430 per-instance UI quad record (`UiInstance`) + the ortho push block
//! (`UiOrtho`) the GUI P5a render path uploads to the GPU.
//!
//! `UiInstance` is the POD record the shader's `StructuredBuffer<UiInstance>` reads
//! by `SV_InstanceID` (proven readable in BOTH the vertex and fragment stages off a
//! single VERTEX|FRAGMENT-visible descriptor by the Rung-0.5 GPU golden). It is
//! PACK-SCRATCH ONLY (GUI P5a Decision 6): the upload system materializes a reused
//! `Vec<UiInstance>`, stable-sorts it by `StackIndex`, and bulk-memcpys it into the
//! mapped per-frame ring slot — it is NOT an ECS column and NOT a per-chunk
//! `cast_slice` (the global z-sort forbids a per-chunk blit).

/// One instanced UI quad on the GPU — the std430 record the shader reads by
/// `SV_InstanceID`. `#[repr(C, align(16))]` POD, all-f32 (no f16 / no u8 packing,
/// so no 16-bit-storage device feature is needed). Physical px (the `scale_factor`
/// is folded at pack); PREMULTIPLIED color (authors write straight RGBA8 in
/// `UiBackground`; premultiplied at pack).
///
/// # std430 layout contract (the compile-time oracle)
///
/// Field order places the two `float2`s first (off 0, 8 — so the first `float4`
/// lands on a 16 B boundary at off 16), then the two `float4`s (off 16, 32), then
/// four scalars (off 48..60). The total stride is **64 B**, a multiple of 16, so
/// the std430 array stride is legal with NO internal padding and NO tail pad. The
/// HLSL `struct UiInstance` mirrors these offsets; the per-field `offset_of!`
/// const-asserts below are the build-time oracle that catches a Rust↔HLSL offset
/// drift the size assert alone would miss.
///
/// `align(16)` is forced explicitly so the Rust struct's alignment matches the
/// std430 array's 16 B stride alignment (a `[f32; 4]` field is only 4-aligned in
/// Rust, unlike an HLSL `float4`, so without `align(16)` the whole struct would be
/// 4-aligned — the per-field byte offsets stay identical either way, but the
/// explicit align documents and pins the array-stride alignment).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub struct UiInstance {
    /// Top-left corner, physical px.
    pub min_px: [f32; 2],
    /// Width + height, physical px.
    pub size_px: [f32; 2],
    /// Clip AABB `min.xy, max.xy`, physical px (valid iff `CLIP_PRESENT`).
    pub clip: [f32; 4],
    /// Per-corner radius `tl, tr, br, bl`, physical px.
    pub corner_radius: [f32; 4],
    /// PREMULTIPLIED RGBA8 fill (`byte0=R .. byte3=A`).
    pub color: u32,
    /// PREMULTIPLIED-at-pack RGBA8 border color.
    pub border_color: u32,
    /// Uniform border width, physical px (P5a is uniform; per-side is deferred).
    pub border_width: f32,
    /// Bit flags: bit0 `BORDER_ANY`, bit1 `CLIP_PRESENT`, the rest reserved.
    pub flags: u32,
}

/// The byte size of [`UiInstance`] (the std430 array stride).
pub const UI_INSTANCE_SIZE: usize = 64;

/// `UiInstance.flags` bit0 — the rect has a (uniform, P5a) border to draw. Set at
/// pack when `border_width > 0`; gates the fragment shader's border branch.
pub const FLAG_BORDER_ANY: u32 = 1 << 0;
/// `UiInstance.flags` bit1 — the rect carries a finite clip AABB in `clip`. Set at
/// pack from a present `ComputedClip`; gates the fragment shader's clip branch (a
/// sentinel-free uniform branch — unclipped rects never evaluate `clip_coverage`).
pub const FLAG_CLIP_PRESENT: u32 = 1 << 1;

// --- std430 layout oracle (compile-time). The size/align pin the array stride;
//     the per-field `offset_of!` asserts pin every field's byte offset against the
//     HLSL `struct UiInstance` (catching a swapped/shifted field a size match
//     would not). ---
const _: () = assert!(size_of::<UiInstance>() == UI_INSTANCE_SIZE);
const _: () = assert!(align_of::<UiInstance>() == 16);
const _: () = assert!(core::mem::offset_of!(UiInstance, min_px) == 0);
const _: () = assert!(core::mem::offset_of!(UiInstance, size_px) == 8);
const _: () = assert!(core::mem::offset_of!(UiInstance, clip) == 16);
const _: () = assert!(core::mem::offset_of!(UiInstance, corner_radius) == 32);
const _: () = assert!(core::mem::offset_of!(UiInstance, color) == 48);
const _: () = assert!(core::mem::offset_of!(UiInstance, border_color) == 52);
const _: () = assert!(core::mem::offset_of!(UiInstance, border_width) == 56);
const _: () = assert!(core::mem::offset_of!(UiInstance, flags) == 60);

impl UiInstance {
    /// Re-views a packed `&[UiInstance]` as the contiguous `&[u8]` the upload
    /// memcpys into the mapped ring slot — the no-bytemuck POD view (GUI P5a uses a
    /// hand-rolled view, not a `Pod`/`cast_slice` dependency).
    ///
    /// The returned slice borrows `instances` and MUST NOT outlive it; the upload
    /// path uses it only for the immediate `memcpy`.
    #[inline]
    pub fn slice_as_bytes(instances: &[UiInstance]) -> &[u8] {
        // SAFETY: `UiInstance` is `#[repr(C, align(16))]` all-POD (f32/u32), with no
        // padding (const-asserted 64 B / 16-align / per-field offsets above), so the
        // byte image of `instances` is a valid initialized `[u8]` of exactly
        // `len * UI_INSTANCE_SIZE` bytes. The `&[UiInstance]` borrow keeps the
        // backing alive for the returned slice's lifetime; the slice is read-only.
        unsafe {
            core::slice::from_raw_parts(
                instances.as_ptr().cast::<u8>(),
                instances.len() * UI_INSTANCE_SIZE,
            )
        }
    }
}

/// The pixel→NDC ortho transform pushed as a VERTEX-stage push constant (16 B). A
/// mat-free vec2 multiply-add in the vertex shader: `ndc = pos_px * scale +
/// translate`.
///
/// `#[repr(C)]` POD (`scale` @0, `translate` @8, 16 B).
///
/// # Canonical convention (the ONE source of truth)
///
/// `scale = (2/w, +2/h)`, `translate = (-1, -1)`, so:
/// - `(0, 0)` (top-left pixel)     → NDC `(-1, -1)`
/// - `(w, h)` (bottom-right pixel) → NDC `(+1, +1)`
///
/// A POSITIVE y scale gives a top-left pixel origin in Vulkan's y-DOWN NDC. This is
/// the convention the code implements and the only one this type expresses.
///
/// # Plan deviation (recorded)
///
/// GUI P5a plan A2 specified the GL-style `scale = (2/w, **-2/h**)`,
/// `translate = (-1, **+1**)` (negative y, bottom-left-style). The **Rung-0.5 GPU
/// oracle overrides A2**: that GL formula lands pixel row 0 at the framebuffer
/// *bottom* on the in-house Vulkan path (`ssbo_graphics_probe`, RTX 3060,
/// validation clean). The positive-y convention above is therefore canonical; the
/// `-2/h, +1` form is the **rejected** one — do not reintroduce it.
///
/// The denominator (`w`, `h`) is the EXTENT THE UI PASS RENDERS INTO (the swapchain
/// `VkExtent2D`), per GUI P5a Decision 9 / A2.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiOrtho {
    /// Per-axis NDC scale `(2/w, +2/h)` — positive y for the top-left origin.
    pub scale: [f32; 2],
    /// NDC translation `(-1, -1)` — maps the top-left pixel `(0,0)` to NDC `(-1,-1)`.
    pub translate: [f32; 2],
}

const _: () = assert!(size_of::<UiOrtho>() == 16);

impl UiOrtho {
    /// The pixel→NDC ortho for a `(width, height)` physical-px render target,
    /// top-left origin (Vulkan y-down NDC; positive y scale). `(0,0)` maps to NDC
    /// `(-1,-1)` (top-left texel) and `(width,height)` to `(+1,+1)` (bottom-right).
    ///
    /// `width`/`height` MUST be the extent of the image the UI pass renders into
    /// (the swapchain `VkExtent2D`), so a rect at the bottom-right corner lands at
    /// the bottom-right texel of that same image (Decision 9 contract).
    #[inline]
    pub fn for_extent(width: u32, height: u32) -> UiOrtho {
        debug_assert!(width > 0 && height > 0, "invariant: UI ortho extent is non-zero");
        UiOrtho {
            scale: [2.0 / width as f32, 2.0 / height as f32],
            translate: [-1.0, -1.0],
        }
    }

    /// Re-views this 16-byte POD as the `&[u8]` the draw recorder pushes as a
    /// VERTEX-stage push constant.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `UiOrtho` is `#[repr(C)]` POD (f32 only, 16 B, no padding —
        // const-asserted), so its byte image is a valid `[u8; 16]`; the `&self`
        // borrow keeps it alive for the returned slice; the slice is read-only.
        unsafe { core::slice::from_raw_parts((self as *const UiOrtho).cast::<u8>(), size_of::<UiOrtho>()) }
    }
}

/// Premultiplies a STRAIGHT RGBA8 color (`byte0=R .. byte3=A`) — author space — into
/// the PREMULTIPLIED RGBA8 the shader expects (`rgb *= a`), keeping the byte order.
///
/// Premultiplied alpha (RmlUi/WebRender) composes AA edges + nested clips correctly
/// under the engine's `src=ONE` blend, where straight alpha would fringe. Rounded
/// with `+ 127` before the `/ 255` divide so the conversion is symmetric.
#[inline]
pub const fn premultiply_rgba8(straight: u32) -> u32 {
    let r = straight & 0xFF;
    let g = (straight >> 8) & 0xFF;
    let b = (straight >> 16) & 0xFF;
    let a = (straight >> 24) & 0xFF;
    // (channel * a + 127) / 255 — rounded, exact for a == 255 (identity) and a == 0.
    let pr = (r * a + 127) / 255;
    let pg = (g * a + 127) / 255;
    let pb = (b * a + 127) / 255;
    pr | (pg << 8) | (pb << 16) | (a << 24)
}
