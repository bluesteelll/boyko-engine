//! Phase-6 S0 — the device-local image (`VkImage` + `VkImageView` + a dedicated
//! `VkDeviceMemory` allocation) backing [`RhiDevice::create_texture`](boyko_rhi::RhiDevice::create_texture).
//!
//! A texture is a 2D (rung 1) or 3D (deferred) `OPTIMAL`-tiling color image — or a
//! 2D depth image (rung 4, `DEPTH_STENCIL_ATTACHMENT` usage → DEPTH-aspect view) —
//! bound to its own `DEVICE_LOCAL` allocation, plus one full-subresource `VkImageView`.
//! Unlike a buffer (which sub-allocates from the shared block), each image gets a
//! **dedicated** `vkAllocateMemory` (S0 has a handful of attachments — a dedicated
//! allocation is the simplest sound binding, and `OPTIMAL`-tiling images have
//! their own alignment/`memory_type_bits` that the buffer sub-allocator was not
//! sized for). The image is never CPU-mapped; the only CPU touch is the fenced
//! test readback through a host-visible staging buffer + `vkCmdCopyImageToBuffer`.
//!
//! # Ownership & teardown (mirrors `BoundBuffer`, plan A5/D2)
//!
//! [`VulkanTexture`] is **not** `Copy`/`Clone`: destruction is by-value
//! ([`VulkanContext::destroy_texture`](crate::device::VulkanContext)) so the move
//! encodes "destroyed exactly once". Teardown is reverse creation order: view →
//! image → memory. The originating [`VulkanContext`](crate::device::VulkanContext) must still be alive when the
//! texture is destroyed (the destroy goes through the context's device fn-table).
//!
//! # Array depth textures (CSM Increment 0)
//!
//! When [`TextureDesc::array_layers`](boyko_rhi::TextureDesc::array_layers) `> 1`
//! (a DEPTH-format image), the texture owns a VIEW SET instead of a single view:
//! `N` per-layer `VK_IMAGE_VIEW_TYPE_2D` RENDER views (each cascade renders into its
//! own layer) plus ONE `VK_IMAGE_VIEW_TYPE_2D_ARRAY` SAMPLE view (the resolve samples
//! `float3(uv, layer)`). For `array_layers == 1` (every existing texture) the path is
//! byte-identical: `view` is the single full-subresource view, the render-view set
//! holds that same view in slot 0, and `array_view` is `NULL` (no array view created).
//!
//! # Mip levels + a decoupled view format (textured-PBR T2)
//!
//! [`TextureDesc::mip_levels`](boyko_rhi::TextureDesc::mip_levels) `> 1` builds a full
//! mip chain (`VkImageCreateInfo::mip_levels`); the single-layer full-subresource
//! `view` then spans `[0, mip_levels)` rather than just mip 0, so a sampler can select
//! any LOD. [`TextureDesc::view_format`](boyko_rhi::TextureDesc::view_format) `Some(f)`
//! with `f != format` additionally sets `VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT` on the
//! image and creates every view in `f` instead of the image's own `format` (the
//! sRGB-view trick — see `boyko_render::texture`'s `ColorSpace`). `mip_levels == 1` +
//! `view_format == None` (every pre-T2 texture) is byte-identical to the prior
//! single-mip, `flags: 0`, own-format path. A multi-layer texture (CSM) is never
//! combined with `mip_levels > 1` (a `debug_assert!` in `create` traps it) — its
//! per-layer RENDER views always target mip 0 only.
//!
//! # Explicit views — [`VulkanTextureView`] (VG R3 step S1)
//!
//! Every view described above is created by the texture, FOR the texture, at
//! `base_mip_level = 0`. None of them can name mip `k` alone, and a GPU mip chain is
//! built exactly one level at a time. [`VulkanTextureView`] is the explicit,
//! caller-owned view that closes that gap: created from a
//! [`TextureViewDesc`](boyko_rhi::TextureViewDesc) with
//! `base_mip_level = desc.base_mip` and `level_count = desc.mip_count`, it is the one
//! view shape no path above can produce. It follows [`VulkanTexture`]'s ownership
//! discipline verbatim — not `Copy`/`Clone`, destroyed by value, so the move encodes
//! "destroyed exactly once".
//!
//! ## ⚠️ THE OWNERSHIP RULE
//!
//! **A [`VulkanTextureView`] MUST be owned by the same struct that owns its
//! [`VulkanTexture`], and destroyed BEFORE it.**
//!
//! A view is a CHILD object of the image. Destroying the image first leaves the view
//! naming a dead `VkImage`; destroying the image while a view of it is live is
//! `VUID-vkDestroyImage-image-01000`. Only co-ownership makes "before" expressible at
//! the site that has to honor it — a view held by a different struct than its image has
//! no teardown order to be right about.
//!
//! The rule is enforced the way `// SAFETY:` is enforced: by a marker comment that one
//! grep enumerates. Every field holding a view carries, on the line above it,
//!
//! ```text
//! // VIEW-OWNER: <the texture field this is a view OF>; destroyed before it in <teardown fn>.
//! ```
//!
//! and the census is
//!
//! ```text
//! rg "^\s*// VIEW-OWNER:" crates/
//! ```
//!
//! The pattern matches only real marker comments: a `///` doc line and a `//!`
//! module-doc line both fail it (the third `/`, and the `!`, break `// VIEW-OWNER:`), so
//! this section does not pollute its own census. S1 landed the capability with no owner,
//! so the census was empty; as of VG R3 piece 1 step P1-2 it has exactly ONE entry —
//! `present::targets::HzbTargets::level_views` (the depth pyramid's per-mip views), owned
//! beside the `pyramid` image they view and destroyed before it in `HzbTargets::destroy`.
//!
//! ## Debt: the view duality, and its named trigger
//!
//! After S1 this crate has TWO kinds of image view: the IMPLICIT texture-owned ones
//! (`view`, `layer_views`, `array_view`, created inside `VulkanTexture::create` and
//! never handed out as owned values) and the EXPLICIT [`VulkanTextureView`]. The
//! duality is deliberate, and it is debt: `layer_views` is a per-layer view set that the
//! explicit form could express as `base_layer: i, layer_count: 1`.
//!
//! It is NOT paid down here. Migrating `layer_views` in a foundation step would be an
//! unrelated change riding along inside it, and it would move golden pins for no reason.
//! The trigger is named instead: **the next feature that needs a per-layer OR per-mip
//! view migrates `layer_views` to [`VulkanTextureView`] in the same rung.** Until then
//! the implicit set stays exactly as it is.

use core::ptr;

use boyko_rhi::{TextureDesc, TextureViewDesc, TextureViewDimension};

use crate::device::DeviceFns;
use crate::error::VulkanError;
use crate::ffi::*;
use crate::memory::select_memory_type;

/// The maximum number of array layers ANY multi-layer depth texture may carry — the fixed inline
/// capacity of the per-layer render-view set. Sized for the LARGEST consumer: the Shadow Phase 5
/// Inc-1-GPU spot/point shadow ATLAS (`boyko_render::shadow_atlas::M_SLOTS` == 16 layers). The CSM
/// cascade texture (4 layers ≤ 16) shares this same view set — the unused tail views are `NULL`
/// (negligible cost). A `desc.array_layers` must be in `1..=MAX_TEXTURE_LAYERS`.
pub const MAX_TEXTURE_LAYERS: usize = 16;

/// The cascaded-shadow-map cascade count (CSM Increment 0/3). Retained as the CSM-specific layer
/// budget (the CSM texture is created with `array_layers == MAX_CASCADES`); it MUST be
/// `<= MAX_TEXTURE_LAYERS` so the cascade views fit the shared inline view set.
pub const MAX_CASCADES: usize = 4;

const _: () = assert!(
    MAX_CASCADES <= MAX_TEXTURE_LAYERS,
    "invariant: the CSM cascade count must fit the shared per-layer render-view set"
);

/// An owned device-local image (color, or depth for a `DEPTH_STENCIL_ATTACHMENT`
/// usage) + its view(s) + the dedicated `VkDeviceMemory` it is bound to
/// ([`RhiApi::Texture`](boyko_rhi::RhiApi::Texture)).
///
/// For a single-layer image (`array_layers == 1`, every existing texture) this is
/// today's shape: one `image`, one full-subresource `view`, one `memory`. For a
/// multi-layer depth image (`array_layers > 1`, CSM Increment 0) it additionally owns
/// the per-layer RENDER view set + the array SAMPLE view (see the module docs).
///
/// # Safety
///
/// The originating [`VulkanContext`](crate::device::VulkanContext) MUST still be
/// alive when this texture is used (as a barrier/attachment/copy source) or
/// destroyed: each goes through the context's device fn-table. No compile-time
/// `'ctx` tie this phase (plan F1; the structural fix is deferred to Phase 2-3).
pub struct VulkanTexture {
    /// The `VkImage` handle; destroyed by `destroy_texture`. Read by the encoder's
    /// `image_barrier` / `copy_image_to_buffer`.
    pub(crate) image: VkImage,
    /// The full-subresource `VkImageView`; destroyed before the image. Read by the
    /// encoder's `begin_rendering` (the color/depth attachment). For a single-layer
    /// image this is the only view; for a multi-layer image it ALIASES
    /// `layer_views[0]` (the layer-0 render view), so the existing `.view` read stays
    /// valid (it samples/renders layer 0) — it is NOT destroyed separately (the
    /// `layer_views` teardown owns it).
    pub(crate) view: VkImageView,
    /// The dedicated device-local allocation backing the image; freed last.
    pub(crate) memory: VkDeviceMemory,
    /// The per-layer `VK_IMAGE_VIEW_TYPE_2D` RENDER views (CSM Increment 0): slot `i`
    /// is the view of array layer `i` (`baseArrayLayer = i`, `layerCount = 1`), so a
    /// shadow pass renders cascade / atlas-slot `i` into it. Only the first
    /// `active_layers` slots are valid (`NULL` tail). For a single-layer image slot 0 ==
    /// `view`. Sized to [`MAX_TEXTURE_LAYERS`] (the shadow-atlas's 16-layer max; the
    /// 4-layer CSM texture leaves the tail `NULL`).
    pub(crate) layer_views: [VkImageView; MAX_TEXTURE_LAYERS],
    /// The number of valid `layer_views` (`1..=MAX_TEXTURE_LAYERS`). `1` for every
    /// single-layer image.
    pub(crate) active_layers: u32,
    /// The `VK_IMAGE_VIEW_TYPE_2D_ARRAY` SAMPLE view over all `active_layers` layers
    /// (CSM Increment 0): the resolve samples `float3(uv, layer)` through it. `NULL`
    /// for a single-layer image (no array view is created there).
    pub(crate) array_view: VkImageView,
    /// The `VkFormat` every view of this image is created in — the decoupled
    /// [`TextureDesc::view_format`] when it is `Some`, else [`TextureDesc::format`]
    /// (VG R3 step S1). Retained because an image handle carries no queryable metadata
    /// in this raw-FFI backend, and an explicit [`VulkanTextureView`] with
    /// `TextureViewDesc::format == None` inherits exactly this value.
    pub(crate) view_format: i32,
    /// The `VkImageAspectFlags` every view of this image uses — DEPTH for a
    /// depth-stencil-attachment image, COLOR otherwise (VG R3 step S1). Retained for the
    /// same reason as `view_format`: an explicit view MUST reuse the parent image's
    /// aspect, and a mismatched aspect faults `vkCreateImageView` under validation.
    pub(crate) aspect_mask: VkFlags,
    /// The image's `mipLevels` (VG R3 step S1). Retained so an explicit view's
    /// `[base_mip, base_mip + mip_count)` range can be `debug_assert`ed against the
    /// image it is a view of.
    pub(crate) mip_levels: u32,
}

impl VulkanTexture {
    /// Creates a 2D/3D image per `desc` (color, or depth when the usage carries
    /// `DEPTH_STENCIL_ATTACHMENT`), allocates + binds a dedicated device-local
    /// block, and creates the view(s).
    ///
    /// For `desc.array_layers == 1` (every existing texture): one full-subresource
    /// view (byte-identical to the prior path). For `desc.array_layers > 1` (a
    /// multi-layer DEPTH image, CSM Increment 0): `N` per-layer
    /// `VK_IMAGE_VIEW_TYPE_2D` RENDER views + one `VK_IMAGE_VIEW_TYPE_2D_ARRAY` SAMPLE
    /// view (see the module docs).
    ///
    /// On any partial failure every object created so far is torn down in reverse
    /// order before the error returns (no leak on the error path).
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device + its command table; `mem_props`
    /// must be that physical device's memory properties.
    pub(crate) unsafe fn create(
        device: VkDevice,
        fns: &DeviceFns,
        mem_props: &VkPhysicalDeviceMemoryProperties,
        desc: &TextureDesc,
    ) -> Result<Self, VulkanError> {
        debug_assert!(
            desc.width > 0 && desc.height > 0 && desc.depth > 0,
            "invariant: texture extent must be non-zero in every dimension"
        );
        debug_assert!(
            desc.array_layers >= 1 && (desc.array_layers as usize) <= MAX_TEXTURE_LAYERS,
            "invariant: texture array_layers must be in 1..=MAX_TEXTURE_LAYERS"
        );
        // Release-safe clamp: the inline view set never exceeds `MAX_TEXTURE_LAYERS`, and a
        // floor of 1 keeps the single-view path valid even if a (debug-asserted)
        // out-of-range count slipped through a release build.
        let layers = (desc.array_layers as usize).clamp(1, MAX_TEXTURE_LAYERS) as u32;
        let is_array = layers > 1;

        debug_assert!(desc.mip_levels >= 1, "invariant: texture mip_levels must be >= 1");
        debug_assert!(
            !(is_array && desc.mip_levels > 1),
            "invariant: a multi-layer texture does not support multiple mip levels \
             (T2 mip chains are for single-layer sampled textures only)"
        );

        let image_type = desc.dimension.as_i32();
        let format = desc.format.as_i32();
        // Textured-PBR T2 Decision D2: `view_format` decouples the SAMPLED view's
        // format from the image's own (the sRGB-view trick). `None` (every pre-T2
        // texture) keeps the view in the image's own format — byte-identical.
        let mutable_format = desc.view_format.is_some_and(|vf| vf != desc.format);
        let view_format = desc.view_format.map_or(format, |vf| vf.as_i32());
        let create_flags: VkFlags = if mutable_format {
            VK_IMAGE_CREATE_MUTABLE_FORMAT_BIT
        } else {
            0
        };
        // The agnostic `ImageUsage` bits equal the `VK_IMAGE_USAGE_*` bits (identity
        // cast, asserted in `abi_guard.rs`).
        let usage: VkFlags = desc.usage.bits();

        // The view aspect is DEPTH for a depth-stencil-attachment image (rung 4),
        // else COLOR (rungs 1..3 color images + the deferred D3 storage image). A
        // mismatched aspect makes `vkCreateImageView` fault under validation; this
        // routes the single new depth case while leaving the color path byte-identical.
        let is_depth = (usage & VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT) != 0;
        let aspect_mask = if is_depth {
            VK_IMAGE_ASPECT_DEPTH_BIT
        } else {
            VK_IMAGE_ASPECT_COLOR_BIT
        };

        // The full-subresource (single-layer) view type. A multi-layer image's
        // per-layer render views are always `VK_IMAGE_VIEW_TYPE_2D` (one layer each).
        let view_type = match image_type {
            VK_IMAGE_TYPE_3D => VK_IMAGE_VIEW_TYPE_3D,
            // `VK_IMAGE_TYPE_2D` (the rung-1 path) and any other 2D-shaped value.
            _ => VK_IMAGE_VIEW_TYPE_2D,
        };

        let image_info = VkImageCreateInfo {
            s_type: VkStructureType::ImageCreateInfo,
            p_next: ptr::null(),
            flags: create_flags,
            image_type,
            format,
            extent: VkExtent3D {
                width: desc.width,
                height: desc.height,
                depth: desc.depth,
            },
            mip_levels: desc.mip_levels,
            array_layers: layers,
            samples: VK_SAMPLE_COUNT_1_BIT,
            tiling: VK_IMAGE_TILING_OPTIMAL,
            usage,
            sharing_mode: VK_SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            p_queue_family_indices: ptr::null(),
            initial_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        };

        let mut image = VkImage::NULL;
        // SAFETY: `device` is live; `image_info` is a fully-initialized `#[repr(C)]`
        // struct whose only pointer (`p_queue_family_indices`) is null for count 0;
        // `&mut image` is a valid out-pointer; NULL allocator.
        let raw = unsafe { (fns.create_image)(device, &image_info, ptr::null(), &mut image) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateImage", result));
        }

        // Dedicated device-local allocation sized to the image's requirements.
        let mut reqs = VkMemoryRequirements {
            size: 0,
            alignment: 1,
            memory_type_bits: 0,
        };
        // SAFETY: `image` was just created on `device`; `&mut reqs` is a valid
        // out-pointer for the `#[repr(C)]` `VkMemoryRequirements`.
        unsafe { (fns.get_image_memory_requirements)(device, image, &mut reqs) };

        let Some(memory_type_index) = select_memory_type(
            mem_props,
            VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT,
            reqs.memory_type_bits,
        ) else {
            // SAFETY: `image` was created above and is not yet bound; destroy it
            // once on this error path so it never leaks.
            unsafe { (fns.destroy_image)(device, image, ptr::null()) };
            return Err(VulkanError::NoSuitableMemoryType);
        };

        let alloc_info = VkMemoryAllocateInfo {
            s_type: VkStructureType::MemoryAllocateInfo,
            p_next: ptr::null(),
            allocation_size: reqs.size,
            memory_type_index,
        };
        let mut memory = VkDeviceMemory::NULL;
        // SAFETY: `device` is live; `alloc_info` is fully initialized for a
        // device-local type that satisfies the image's `memory_type_bits`;
        // `&mut memory` is a valid out-pointer; NULL allocator.
        let raw = unsafe { (fns.allocate_memory)(device, &alloc_info, ptr::null(), &mut memory) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: `image` is created-but-unbound; destroy it once before the
            // error returns.
            unsafe { (fns.destroy_image)(device, image, ptr::null()) };
            return Err(VulkanError::Vk("vkAllocateMemory(texture)", result));
        }

        // SAFETY: `image` is unbound; `memory` is a fresh dedicated allocation of
        // `reqs.size` bytes of a type in `reqs.memory_type_bits`; binding at
        // offset 0 satisfies the image's alignment. `vkBindImageMemory` binds once.
        let raw = unsafe { (fns.bind_image_memory)(device, image, memory, 0) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            // SAFETY: bind failed; free the allocation then destroy the unbound
            // image, each once, in reverse order on this error path.
            unsafe {
                (fns.free_memory)(device, memory, ptr::null());
                (fns.destroy_image)(device, image, ptr::null());
            }
            return Err(VulkanError::Vk("vkBindImageMemory", result));
        }

        // The view components are identity for both the single-layer and array paths.
        let components = VkComponentMapping {
            r: VK_COMPONENT_SWIZZLE_IDENTITY,
            g: VK_COMPONENT_SWIZZLE_IDENTITY,
            b: VK_COMPONENT_SWIZZLE_IDENTITY,
            a: VK_COMPONENT_SWIZZLE_IDENTITY,
        };

        // The per-layer RENDER views. For `layers == 1` this is exactly today's path:
        // one full-subresource `VK_IMAGE_VIEW_TYPE_2D` view (`baseArrayLayer = 0`,
        // `layerCount = 1`). For `layers > 1` it is `layers` per-layer views, each
        // `baseArrayLayer = i`, `layerCount = 1` — every cascade renders into its own
        // layer. On any view's failure, every view created so far + the image + memory
        // are torn down in reverse order (no leak).
        //
        // T2: the single-layer view's `level_count` spans the WHOLE mip chain
        // (`desc.mip_levels`) so it doubles as the SAMPLED bindless view a fragment
        // shader LODs across; every existing single-layer caller has `mip_levels ==
        // 1`, so `level_count` stays `1` there — byte-identical. A multi-layer
        // RENDER view always targets mip 0 only (`level_count: 1`), since an
        // attachment renders into exactly one mip (the debug_assert above rejects
        // `is_array && mip_levels > 1`, so this is never `> 1` for `is_array`
        // anyway). The view's `format` is the decoupled `view_format` (T2 Decision
        // D2; equals `format` when `desc.view_format` is `None`).
        let view_level_count = if is_array { 1 } else { desc.mip_levels };
        let mut layer_views = [VkImageView::NULL; MAX_TEXTURE_LAYERS];
        for i in 0..layers as usize {
            let view_info = VkImageViewCreateInfo {
                s_type: VkStructureType::ImageViewCreateInfo,
                p_next: ptr::null(),
                flags: 0,
                image,
                view_type,
                format: view_format,
                components,
                subresource_range: VkImageSubresourceRange {
                    aspect_mask,
                    base_mip_level: 0,
                    level_count: view_level_count,
                    base_array_layer: i as u32,
                    layer_count: 1,
                },
            };
            let mut layer_view = VkImageView::NULL;
            // SAFETY: `device` is live; `view_info` names the live `image` with a
            // single-layer range at `baseArrayLayer = i < layers` (within the image's
            // `arrayLayers`); `&mut layer_view` is a valid out-pointer; NULL allocator.
            let raw =
                unsafe { (fns.create_image_view)(device, &view_info, ptr::null(), &mut layer_view) };
            let result = VkResult::from_raw(raw);
            if !result.is_success() {
                // SAFETY: tear down the `i` views created so far, then the bound image
                // + memory, each once, in reverse order on this error path.
                unsafe {
                    for v in layer_views.iter().take(i) {
                        (fns.destroy_image_view)(device, *v, ptr::null());
                    }
                    (fns.free_memory)(device, memory, ptr::null());
                    (fns.destroy_image)(device, image, ptr::null());
                }
                return Err(VulkanError::Vk("vkCreateImageView(texture layer)", result));
            }
            layer_views[i] = layer_view;
        }

        // The array SAMPLE view (`VK_IMAGE_VIEW_TYPE_2D_ARRAY`, all `layers` layers) —
        // ONLY for a multi-layer image (CSM Increment 0). A single-layer image keeps
        // `array_view == NULL` (no array view), so its path is byte-identical to today.
        let array_view = if is_array {
            let array_view_info = VkImageViewCreateInfo {
                s_type: VkStructureType::ImageViewCreateInfo,
                p_next: ptr::null(),
                flags: 0,
                image,
                view_type: VK_IMAGE_VIEW_TYPE_2D_ARRAY,
                format: view_format,
                components,
                subresource_range: VkImageSubresourceRange {
                    aspect_mask,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: layers,
                },
            };
            let mut av = VkImageView::NULL;
            // SAFETY: `device` is live; `array_view_info` names the live `image` with a
            // `layerCount = layers` range from `baseArrayLayer = 0` (the whole array);
            // `&mut av` is a valid out-pointer; NULL allocator.
            let raw =
                unsafe { (fns.create_image_view)(device, &array_view_info, ptr::null(), &mut av) };
            let result = VkResult::from_raw(raw);
            if !result.is_success() {
                // SAFETY: tear down every per-layer view, then the bound image +
                // memory, each once, in reverse order on this error path.
                unsafe {
                    for v in layer_views.iter().take(layers as usize) {
                        (fns.destroy_image_view)(device, *v, ptr::null());
                    }
                    (fns.free_memory)(device, memory, ptr::null());
                    (fns.destroy_image)(device, image, ptr::null());
                }
                return Err(VulkanError::Vk("vkCreateImageView(texture array)", result));
            }
            av
        } else {
            VkImageView::NULL
        };

        Ok(Self {
            image,
            // `view` is layer 0's render view (== the full-subresource view for a
            // single-layer image): the existing `.view` reads stay byte-identical.
            view: layer_views[0],
            memory,
            layer_views,
            active_layers: layers,
            array_view,
            // S1 metadata: recorded from the values the views above were just built
            // with, so an explicit `VulkanTextureView` inherits the same format/aspect
            // rather than re-deriving them from a desc the caller no longer holds.
            view_format,
            aspect_mask,
            mip_levels: desc.mip_levels,
        })
    }

    /// The full-subresource `VkImageView` (single-layer images: the only view; a
    /// multi-layer image: layer 0's render view). T4 bindless: the raw handle
    /// [`crate::bindless::write_bindless_texture`] writes into a texture-array
    /// slot — the bindless verb takes a raw `VkImageView` (not a whole
    /// `&VulkanTexture`) because a bindless slot outlives the specific texture
    /// that wrote it (a later `register` call may repoint the same slot at a
    /// different texture entirely).
    #[inline]
    pub fn view(&self) -> VkImageView {
        self.view
    }

    /// The array SAMPLE view (`VK_IMAGE_VIEW_TYPE_2D_ARRAY`) over all layers — for a
    /// multi-layer depth texture the resolve samples `float3(uv, layer)` through it.
    /// Returns `NULL` for a single-layer image (CSM Increment 0).
    ///
    /// `#[allow(dead_code)]`: this accessor is the CSM Increment-0 RHI capability; the
    /// resolve's array sample wires to it in Increment 1+. The capability is validated
    /// now by the array-texture unit test.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn array_sample_view(&self) -> VkImageView {
        self.array_view
    }

    /// The per-layer `VK_IMAGE_VIEW_TYPE_2D` RENDER view for array layer `i` — a
    /// shadow pass renders cascade `i` into it (CSM Increment 0).
    ///
    /// `#[allow(dead_code)]`: this accessor is the CSM Increment-0 RHI capability; the
    /// shadow depth pass renders into each layer via it in Increment 1+. The
    /// capability is validated now by the array-texture + depth-only-draw unit tests.
    ///
    /// # Panics (debug)
    /// `i` must be `< active_layers` (a `debug_assert!`); in release an out-of-range
    /// `i` returns the `NULL` tail slot rather than reading out of bounds.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn layer_render_view(&self, i: u32) -> VkImageView {
        debug_assert!(
            i < self.active_layers,
            "invariant: layer index must be < active_layers"
        );
        self.layer_views
            .get(i as usize)
            .copied()
            .unwrap_or(VkImageView::NULL)
    }

    /// Tears down every view (the array sample view + each per-layer render view),
    /// the image, and the dedicated allocation in reverse creation order, consuming
    /// `self`.
    ///
    /// `self.view` is NOT destroyed separately: it aliases `layer_views[0]`, which the
    /// per-layer loop already destroys. The `array_view` is `NULL` (skipped) for a
    /// single-layer image, so that path destroys exactly the one view it created —
    /// byte-identical to the prior single-`view` teardown.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the texture was created on; no GPU
    /// work referencing the image is in flight (caller fence-waited / `wait_idle`);
    /// it is destroyed exactly once (the by-value `self` enforces the latter).
    pub(crate) unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
        // SAFETY: per the contract `device` is live and nothing references the image.
        // Destroy the array sample view (NULL for a single-layer image — `vkDestroy*`
        // on `VK_NULL_HANDLE` is a defined no-op), then every per-layer render view
        // (`active_layers` valid slots; the `view` field aliases slot 0 and is NOT
        // destroyed again), then the image, then free the dedicated allocation — each
        // exactly once in reverse creation order.
        unsafe {
            (fns.destroy_image_view)(device, self.array_view, ptr::null());
            for v in self.layer_views.iter().take(self.active_layers as usize) {
                (fns.destroy_image_view)(device, *v, ptr::null());
            }
            (fns.destroy_image)(device, self.image, ptr::null());
            (fns.free_memory)(device, self.memory, ptr::null());
        }
    }
}

/// Lowers a [`TextureViewDesc`] onto the `VkImageViewCreateInfo` that creates it (VG R3
/// step S1) — the whole agnostic→Vulkan mapping of the explicit-view path, in one pure
/// function so it can be asserted without a device.
///
/// `image` is the parent's `VkImage`; `aspect_mask` and `inherited_format` are the
/// parent's `aspect_mask` / `view_format` — the aspect is never a desc field (a view's
/// aspect must equal its image's) and the format is one only when the caller asks to
/// reinterpret. `components` is the identity swizzle and `flags` is 0, matching the
/// texture's own views.
pub(crate) fn texture_view_create_info(
    image: VkImage,
    aspect_mask: VkFlags,
    inherited_format: i32,
    desc: &TextureViewDesc,
) -> VkImageViewCreateInfo {
    VkImageViewCreateInfo {
        s_type: VkStructureType::ImageViewCreateInfo,
        p_next: ptr::null(),
        flags: 0,
        image,
        view_type: desc.dimension.as_i32(),
        format: desc.format.map_or(inherited_format, |f| f.as_i32()),
        components: VkComponentMapping {
            r: VK_COMPONENT_SWIZZLE_IDENTITY,
            g: VK_COMPONENT_SWIZZLE_IDENTITY,
            b: VK_COMPONENT_SWIZZLE_IDENTITY,
            a: VK_COMPONENT_SWIZZLE_IDENTITY,
        },
        subresource_range: VkImageSubresourceRange {
            aspect_mask,
            base_mip_level: desc.base_mip,
            level_count: desc.mip_count,
            base_array_layer: desc.base_layer,
            layer_count: desc.layer_count,
        },
    }
}

/// An owned EXPLICIT `VkImageView` over a sub-range of a [`VulkanTexture`]
/// ([`RhiApi::TextureView`](boyko_rhi::RhiApi::TextureView), VG R3 step S1).
///
/// Created with `base_mip_level = desc.base_mip` and `level_count = desc.mip_count` —
/// the shape none of the texture's own views can produce, since every one of those is
/// pinned to mip 0 (see the module docs).
///
/// # Ownership & teardown (mirrors [`VulkanTexture`])
///
/// **Not** `Copy`/`Clone`: destruction is by value
/// ([`RhiDevice::destroy_texture_view`](boyko_rhi::RhiDevice::destroy_texture_view)), so
/// the move encodes "destroyed exactly once". The non-`Copy`ness is load-bearing rather
/// than stylistic — a `Copy` view would let two values name one `VkImageView` and be
/// destroyed twice. Beyond that, THE OWNERSHIP RULE in the module docs applies: this
/// view is owned by whichever struct owns its texture, and is destroyed BEFORE it.
///
/// # Safety
///
/// The originating [`VulkanContext`](crate::device::VulkanContext) MUST still be alive
/// when this view is used (written into a bind group) or destroyed: each goes through
/// the context's device fn-table. No compile-time `'ctx` tie this phase (plan F1; the
/// [`VulkanTexture`] precedent).
pub struct VulkanTextureView {
    /// The `VkImageView` over the desc's subresource range; destroyed by
    /// `destroy_texture_view`, which per THE OWNERSHIP RULE runs before the parent
    /// texture's `destroy`.
    pub(crate) view: VkImageView,
}

impl VulkanTextureView {
    /// Creates the explicit view of `texture` described by `desc`.
    ///
    /// The parent supplies what a desc must not: the image handle, the aspect (a view's
    /// aspect must equal its image's), and the format an inheriting (`format: None`)
    /// desc adopts.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device + its command table, and `texture` must be
    /// a live texture created on that same device (its `image` is named by the view).
    pub(crate) unsafe fn create(
        device: VkDevice,
        fns: &DeviceFns,
        texture: &VulkanTexture,
        desc: &TextureViewDesc,
    ) -> Result<Self, VulkanError> {
        debug_assert!(
            desc.mip_count >= 1 && desc.layer_count >= 1,
            "invariant: an image view spans at least one mip level and one array layer"
        );
        debug_assert!(
            desc.base_mip.saturating_add(desc.mip_count) <= texture.mip_levels,
            "invariant: the view's mip range must lie within the image's mip chain"
        );
        debug_assert!(
            desc.base_layer.saturating_add(desc.layer_count) <= texture.active_layers,
            "invariant: the view's layer range must lie within the image's array layers"
        );
        // The constraint `TextureViewDesc::dimension`'s own doc declares as a MUST, checked here
        // rather than left to the caller — and `Default` is precisely why it needs checking: the
        // default names a NON-array dimension, so raising `layer_count` alone, which reads like the
        // obvious way to view several layers, silently produces a multi-layer view through a
        // single-layer view type. Vulkan requires `layerCount == 1` for every view type outside the
        // ARRAY family (`VUID-VkImageViewCreateInfo-imageViewType-04973`), and the validation layers
        // are the only other thing that would catch it — which is exactly the configuration this
        // engine disables under `BOYKO_DISABLE_VALIDATION` on its GPU test legs.
        debug_assert!(
            desc.layer_count == 1 || matches!(desc.dimension, TextureViewDimension::D2Array),
            "invariant: a view type outside the ARRAY family spans exactly one array layer; \
             `dimension` must be D2Array to span {} of them",
            desc.layer_count
        );

        let view_info =
            texture_view_create_info(texture.image, texture.aspect_mask, texture.view_format, desc);
        let mut view = VkImageView::NULL;
        // SAFETY: `device` is live and `texture` was created on it (caller contract), so
        // `view_info.image` names a live image; the subresource range lies within that
        // image's mip chain / array layers and carries the image's own aspect (the three
        // `debug_assert`s above plus `texture.aspect_mask`, which `create` recorded from
        // the values the image's own views were built with); `&mut view` is a valid
        // out-pointer; NULL allocator.
        let raw = unsafe { (fns.create_image_view)(device, &view_info, ptr::null(), &mut view) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(VulkanError::Vk("vkCreateImageView(explicit view)", result));
        }
        Ok(Self { view })
    }

    /// The raw `VkImageView` handle — the value a descriptor write or an attachment
    /// names. Public for the same reason [`VulkanTexture`]'s `view()` accessor is: a
    /// cross-crate caller hands the handle to a path that takes raw handles rather than
    /// a whole owned view.
    #[inline]
    pub fn view(&self) -> VkImageView {
        self.view
    }

    /// Destroys the view, consuming `self`.
    ///
    /// # Safety
    ///
    /// `device`/`fns` must be the live device the view was created on; no GPU work
    /// referencing it is in flight (caller fence-waited / `wait_idle`'d); the parent
    /// texture is still alive (THE OWNERSHIP RULE: the view is destroyed BEFORE its
    /// image, since `vkDestroyImage` on an image with a live view is
    /// `VUID-vkDestroyImage-image-01000`); it is destroyed exactly once (the by-value
    /// `self` enforces the latter).
    pub(crate) unsafe fn destroy(self, device: VkDevice, fns: &DeviceFns) {
        // SAFETY: per the contract `device` is live, nothing references the view, and
        // its image outlives this call — so destroying it here is the correct half of
        // the reverse-creation order (view, then image). Exactly once: `self` is by
        // value and `VulkanTextureView` is not `Copy`/`Clone`.
        unsafe { (fns.destroy_image_view)(device, self.view, ptr::null()) };
    }
}

#[cfg(test)]
mod tests {
    use boyko_rhi::{Format, TextureViewDesc, TextureViewDimension};

    use crate::ffi::{
        VK_COMPONENT_SWIZZLE_IDENTITY, VK_FORMAT_D32_SFLOAT, VK_FORMAT_R32_SFLOAT,
        VK_FORMAT_R8G8B8A8_SRGB, VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_ASPECT_COLOR_BIT,
        VK_IMAGE_ASPECT_DEPTH_BIT, VK_IMAGE_VIEW_TYPE_2D, VK_IMAGE_VIEW_TYPE_2D_ARRAY,
        VK_IMAGE_VIEW_TYPE_3D, VkFlags, VkImage, VkStructureType,
    };

    use super::texture_view_create_info;

    /// One row of the desc→`VkImageViewCreateInfo` mapping table: the agnostic input,
    /// the parent-supplied aspect/inherited format, and the Vulkan values the lowering
    /// must produce.
    struct Row {
        what: &'static str,
        desc: TextureViewDesc,
        aspect: VkFlags,
        inherited_format: i32,
        want_view_type: i32,
        want_format: i32,
        want_range: (u32, u32, u32, u32),
    }

    /// The desc's Vulkan mapping, table-driven over the axes that exist: mip range,
    /// layer range, view shape, and format inheritance vs reinterpretation.
    ///
    /// Device-free by construction — [`texture_view_create_info`] is a pure lowering, so
    /// a `VK_NULL_HANDLE` image is a fine stand-in: no Vulkan call is made and the image
    /// handle is only copied through.
    #[test]
    fn desc_lowers_to_the_expected_subresource_range() {
        let rows = [
            Row {
                what: "the default desc: mip 0 alone, layer 0 alone, 2D, inherited format",
                desc: TextureViewDesc::default(),
                aspect: VK_IMAGE_ASPECT_COLOR_BIT,
                inherited_format: VK_FORMAT_R32_SFLOAT,
                want_view_type: VK_IMAGE_VIEW_TYPE_2D,
                want_format: VK_FORMAT_R32_SFLOAT,
                want_range: (0, 1, 0, 1),
            },
            Row {
                what: "a single INTERIOR mip — the pyramid-level shape",
                desc: TextureViewDesc {
                    base_mip: 3,
                    mip_count: 1,
                    ..TextureViewDesc::default()
                },
                aspect: VK_IMAGE_ASPECT_COLOR_BIT,
                inherited_format: VK_FORMAT_R32_SFLOAT,
                want_view_type: VK_IMAGE_VIEW_TYPE_2D,
                want_format: VK_FORMAT_R32_SFLOAT,
                want_range: (3, 1, 0, 1),
            },
            Row {
                what: "a multi-level tail starting above 0",
                desc: TextureViewDesc {
                    base_mip: 2,
                    mip_count: 5,
                    ..TextureViewDesc::default()
                },
                aspect: VK_IMAGE_ASPECT_COLOR_BIT,
                inherited_format: VK_FORMAT_R32_SFLOAT,
                want_view_type: VK_IMAGE_VIEW_TYPE_2D,
                want_format: VK_FORMAT_R32_SFLOAT,
                want_range: (2, 5, 0, 1),
            },
            Row {
                what: "a single INTERIOR array layer at a non-zero mip (both axes at once)",
                desc: TextureViewDesc {
                    base_mip: 1,
                    mip_count: 1,
                    base_layer: 2,
                    layer_count: 1,
                    ..TextureViewDesc::default()
                },
                aspect: VK_IMAGE_ASPECT_DEPTH_BIT,
                inherited_format: VK_FORMAT_D32_SFLOAT,
                want_view_type: VK_IMAGE_VIEW_TYPE_2D,
                want_format: VK_FORMAT_D32_SFLOAT,
                want_range: (1, 1, 2, 1),
            },
            Row {
                what: "a 2D-ARRAY slice of four layers",
                desc: TextureViewDesc {
                    base_layer: 1,
                    layer_count: 4,
                    dimension: TextureViewDimension::D2Array,
                    ..TextureViewDesc::default()
                },
                aspect: VK_IMAGE_ASPECT_DEPTH_BIT,
                inherited_format: VK_FORMAT_D32_SFLOAT,
                want_view_type: VK_IMAGE_VIEW_TYPE_2D_ARRAY,
                want_format: VK_FORMAT_D32_SFLOAT,
                want_range: (0, 1, 1, 4),
            },
            Row {
                what: "a 3D view",
                desc: TextureViewDesc {
                    dimension: TextureViewDimension::D3,
                    ..TextureViewDesc::default()
                },
                aspect: VK_IMAGE_ASPECT_COLOR_BIT,
                inherited_format: VK_FORMAT_R8G8B8A8_UNORM,
                want_view_type: VK_IMAGE_VIEW_TYPE_3D,
                want_format: VK_FORMAT_R8G8B8A8_UNORM,
                want_range: (0, 1, 0, 1),
            },
            Row {
                what: "format REINTERPRETATION overrides the inherited format",
                desc: TextureViewDesc {
                    format: Some(Format::R8G8B8A8Srgb),
                    ..TextureViewDesc::default()
                },
                aspect: VK_IMAGE_ASPECT_COLOR_BIT,
                inherited_format: VK_FORMAT_R8G8B8A8_UNORM,
                want_view_type: VK_IMAGE_VIEW_TYPE_2D,
                want_format: VK_FORMAT_R8G8B8A8_SRGB,
                want_range: (0, 1, 0, 1),
            },
            Row {
                what: "an explicit format EQUAL to the inherited one lowers the same",
                desc: TextureViewDesc {
                    base_mip: 7,
                    format: Some(Format::R32Sfloat),
                    ..TextureViewDesc::default()
                },
                aspect: VK_IMAGE_ASPECT_COLOR_BIT,
                inherited_format: VK_FORMAT_R32_SFLOAT,
                want_view_type: VK_IMAGE_VIEW_TYPE_2D,
                want_format: VK_FORMAT_R32_SFLOAT,
                want_range: (7, 1, 0, 1),
            },
        ];

        for row in &rows {
            let info = texture_view_create_info(
                VkImage::NULL,
                row.aspect,
                row.inherited_format,
                &row.desc,
            );
            let r = info.subresource_range;

            assert_eq!(info.view_type, row.want_view_type, "viewType: {}", row.what);
            assert_eq!(info.format, row.want_format, "format: {}", row.what);
            assert_eq!(
                (
                    r.base_mip_level,
                    r.level_count,
                    r.base_array_layer,
                    r.layer_count
                ),
                row.want_range,
                "subresourceRange: {}",
                row.what
            );
            assert_eq!(
                r.aspect_mask, row.aspect,
                "aspectMask comes from the TEXTURE, not the desc: {}",
                row.what
            );

            // The invariant half: an explicit view is spelled like the texture's own
            // views everywhere the desc does not speak.
            assert!(
                matches!(info.s_type, VkStructureType::ImageViewCreateInfo),
                "sType: {}",
                row.what
            );
            assert!(info.p_next.is_null(), "pNext must stay null: {}", row.what);
            assert_eq!(info.flags, 0, "flags: {}", row.what);
            // `VkImage` is a bare handle newtype with no `Debug`, so this is `assert!`
            // over `==` rather than `assert_eq!`.
            assert!(
                info.image == VkImage::NULL,
                "image is copied through: {}",
                row.what
            );
            let c = info.components;
            assert_eq!(
                [c.r, c.g, c.b, c.a],
                [VK_COMPONENT_SWIZZLE_IDENTITY; 4],
                "components must stay the identity swizzle: {}",
                row.what
            );
        }
    }

    /// The one shape the whole step exists for: `base_mip = k, mip_count = 1` is
    /// EXACTLY what no texture-owned view produces. `VulkanTexture::create` builds every
    /// view at `base_mip_level: 0` with `level_count` either `desc.mip_levels` (single
    /// layer) or `1` (array), so for `k > 0` the explicit lowering must differ from both.
    #[test]
    fn per_mip_view_is_a_shape_the_texture_owned_views_cannot_produce() {
        let mip_levels = 6u32;
        for k in 1..mip_levels {
            let info = texture_view_create_info(
                VkImage::NULL,
                VK_IMAGE_ASPECT_COLOR_BIT,
                VK_FORMAT_R32_SFLOAT,
                &TextureViewDesc {
                    base_mip: k,
                    mip_count: 1,
                    ..TextureViewDesc::default()
                },
            );
            let r = info.subresource_range;
            assert_eq!(r.base_mip_level, k);
            assert_eq!(r.level_count, 1);
            // The texture-owned single-layer view: base 0, count `mip_levels`.
            assert_ne!(
                (r.base_mip_level, r.level_count),
                (0, mip_levels),
                "per-mip view must differ from the texture's full-chain view"
            );
            // The texture-owned per-layer (array) view: base 0, count 1.
            assert_ne!(
                (r.base_mip_level, r.level_count),
                (0, 1),
                "per-mip view must differ from the texture's per-layer view"
            );
        }
    }
}
