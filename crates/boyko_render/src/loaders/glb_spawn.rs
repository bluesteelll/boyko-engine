//! The engine-owned glb spawn path (RD2 / TRANSPARENCY R0a): one function turns a decoded
//! [`GlbScene`] into entities.
//!
//! [`spawn_glb_scene`] uploads each referenced image once per colour space it is used in,
//! builds one [`Material`] per glTF material (alpha mode, cutoff, `doubleSided` and the KHR
//! factors included), spawns a [`MeshBundle`] + [`ShadowCaster`] per primitive and inserts the
//! route marker [`Material::route`] derives — the single authority for a glb's route
//! (09-10 D3). Before RD2 two examples carried private copies of this code and neither inserted
//! a marker.
//!
//! # Device seam
//!
//! The two device operations (a mesh upload, an image decode + upload) go through the
//! [`GlbUploader`] trait, monomorphised: [`spawn_glb_scene`] is the device entry point (it
//! builds a [`GlbDeviceUploader`]); [`spawn_glb_scene_with`] is the same body over any uploader,
//! which is how a host test drives the real spawn logic without a device.
//!
//! Load-time only: the function allocates function-local scratch (the image-slot table, a
//! vertex buffer reused across primitives) and keeps none of it — no side store survives the
//! call (principle 0's "transient function-local scratch").

use boyko_ecs::ecs::core::asset::{AssetLoader, Assets};
use boyko_ecs::ecs::core::system::Commands;
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_scene::Transform;
use boyko_scene::render_caps::{MaterialHandle, MeshHandle};

use crate::bindless::BindlessTextureTable;
use crate::bundles::MeshBundle;
use crate::csm_marker::ShadowCaster;
use crate::loaders::PngTextureLoader;
use crate::loaders::glb::{GlbAlphaMode, GlbImage, GlbMaterial, GlbScene};
use crate::material::{
    AlphaMode, BlendClass, Material, MaterialGpu, MaterialRoute, MaterialTextures, MaterialX,
};
use crate::mesh::{MeshGpu, Vertex};
use crate::mesh_assets::build_mesh_gpu;
use crate::mesh_face::winding_census;
use crate::route::{AlphaMasked, Translucent};
use crate::texture::{ColorSpace, TextureAssetsExt, TextureGpu};

/// What [`spawn_glb_scene`] does besides spawning.
#[derive(Clone, Copy, Debug)]
pub struct GlbSpawnOptions {
    /// The placement every spawned entity takes.
    pub transform: Transform,
    /// Shift every primitive by the centre of the whole scene's AABB
    /// ([`glb_scene_bounds`]), so `transform` places the model by its centre.
    pub recentre: bool,
    /// R1's hook: dilate the colour of `α = 0` base-colour texels of non-opaque materials at
    /// load. RD2 ships the switch only (R1 is its first consumer); `true` is a debug assertion.
    pub dilate_non_opaque_base: bool,
    /// The material a primitive without a glTF material takes.
    pub fallback_material: MaterialHandle,
}

impl Default for GlbSpawnOptions {
    fn default() -> Self {
        Self {
            transform: Transform::IDENTITY,
            recentre: false,
            dilate_non_opaque_base: false,
            fallback_material: MaterialHandle(0),
        }
    }
}

/// Per-load bookkeeping [`spawn_glb_scene`] returns — counts only, a stack value.
///
/// Every import decision that drops or duplicates data is counted here (AD-21: counted, never
/// silent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlbSpawnReport {
    /// Drawable entities spawned (one per non-empty primitive).
    pub entities: u32,
    /// Entities on the opaque route (no marker).
    pub opaque: u32,
    /// Entities carrying `AlphaMasked`.
    pub masked: u32,
    /// Entities carrying `Translucent`.
    pub translucent: u32,
    /// `Material` rows minted (one per referenced glTF material).
    pub materials: u32,
    /// Image uploads (an image used in both colour spaces counts twice).
    pub images_uploaded: u32,
    /// Referenced image uploads that failed (empty bytes, a MIME this engine does not decode,
    /// a decode error) — the material keeps its factor for that channel.
    pub images_undecodable: u32,
    /// Primitives with no triangles, not spawned.
    pub primitives_skipped: u32,
    /// KHR texture references whose `texCoord` was not 0, read as `texCoord` 0.
    pub khr_texcoord1_ignored: u32,
    /// `KHR_materials_specular` texture references, not imported (T-TAIL).
    pub specular_textures_ignored: u32,
    /// Images referenced both as sRGB colour and as linear data, uploaded once per space.
    pub dual_colour_space_images: u32,
    /// Materials with `dispersion > 0` and `thickness = 0` (dispersion needs a volume).
    pub dispersion_without_volume: u32,
    /// RD6 / D-DS: spawned primitives whose triangles are mostly wound clockwise for their own
    /// normals ([`winding_census`](crate::mesh_face::winding_census)) — counted, never re-wound
    /// (glTF normals are free attributes).
    pub winding_disagrees_primitives: u32,
    /// RD6 / D-DS: spawned primitives whose node placement mirrored them (negative determinant),
    /// re-wound with their tangent handedness negated at bake.
    pub mirrored_placements_rewound: u32,
}

/// The device operations the spawn path needs — monomorphised, so the host test's uploader and
/// the device one compile to direct calls.
pub trait GlbUploader {
    /// Uploads one primitive's model-space geometry and returns its mesh carrier.
    fn upload_mesh(&mut self, vertices: &[Vertex], indices: &[u32]) -> MeshHandle;
    /// Decodes and uploads one embedded image in `color_space`, returning its bindless slot, or
    /// `None` when the image cannot be decoded (the caller counts it).
    fn upload_image(&mut self, image: &GlbImage, color_space: ColorSpace) -> Option<u32>;
}

/// The device [`GlbUploader`]: meshes through [`build_mesh_gpu`] into the world's mesh table
/// (pinned — a glb part is a load-time asset the scene keeps; the same upload
/// [`register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh) performs, minus its
/// debug winding assertion on code-authored geometry), images through the in-house PNG decoder and
/// [`TextureAssetsExt::register_texture`].
pub struct GlbDeviceUploader<'a> {
    /// The world's mesh table.
    pub meshes: &'a mut Assets<MeshGpu>,
    /// The world's texture table.
    pub textures: &'a mut Assets<TextureGpu>,
    /// The bindless descriptor table the textures register into.
    pub bindless: &'a mut BindlessTextureTable,
    /// The device.
    pub ctx: &'a VulkanContext,
}

impl GlbUploader for GlbDeviceUploader<'_> {
    fn upload_mesh(&mut self, vertices: &[Vertex], indices: &[u32]) -> MeshHandle {
        // `build_mesh_gpu` + `add`, NOT `register_mesh`: `register_mesh` debug-asserts the
        // counter-clockwise convention on code-authored geometry, while FILE content is counted
        // by the caller (`GlbSpawnReport::winding_disagrees_primitives`), never refused — glTF
        // normals are free attributes.
        let mesh = MeshHandle(self.meshes.add(build_mesh_gpu(self.ctx, vertices, indices, None)).index());
        self.meshes.pin(mesh.0);
        mesh
    }

    fn upload_image(&mut self, image: &GlbImage, color_space: ColorSpace) -> Option<u32> {
        // PNG is the one container this engine decodes in-house (`boyko_image`); anything else
        // is counted by the caller, not guessed at.
        if image.bytes.is_empty() || !image.mime.eq_ignore_ascii_case("image/png") {
            return None;
        }
        let mut data = PngTextureLoader::decode(&image.bytes).ok()?;
        data.color_space = color_space;
        let handle = self.textures.register_texture(self.ctx, self.bindless, &data);
        Some(self.textures.texture(handle).bindless_slot)
    }
}

/// Spawns a decoded glb through the device: the production entry point. See the module doc;
/// [`spawn_glb_scene_with`] is the body.
#[allow(clippy::too_many_arguments)] // the five world tables + device are the call's real inputs
pub fn spawn_glb_scene(
    scene: &GlbScene,
    commands: &mut Commands<'_>,
    meshes: &mut Assets<MeshGpu>,
    materials: &mut Assets<Material>,
    textures: &mut Assets<TextureGpu>,
    bindless: &mut BindlessTextureTable,
    ctx: &VulkanContext,
    opts: GlbSpawnOptions,
) -> GlbSpawnReport {
    let mut uploader = GlbDeviceUploader { meshes, textures, bindless, ctx };
    spawn_glb_scene_with(scene, commands, materials, &mut uploader, opts)
}

/// [`spawn_glb_scene`]'s body over any [`GlbUploader`]: see the module doc.
///
/// Only materials a non-empty primitive references are minted (once each, pinned — a glb is a
/// load-time asset the scene keeps), and only their images are uploaded: once per colour space
/// a referenced slot samples it in (sRGB for base colour, emissive and the diffuse-transmission
/// colour; linear for normal, metallic-roughness, occlusion, transmission, thickness and the
/// diffuse-transmission strength). A primitive with no material, or one naming a material the
/// file does not have, takes [`GlbSpawnOptions::fallback_material`].
pub fn spawn_glb_scene_with<U: GlbUploader>(
    scene: &GlbScene,
    commands: &mut Commands<'_>,
    materials: &mut Assets<Material>,
    uploader: &mut U,
    opts: GlbSpawnOptions,
) -> GlbSpawnReport {
    debug_assert!(
        !opts.dilate_non_opaque_base,
        "GlbSpawnOptions::dilate_non_opaque_base is R1's hook; RD2 ships the switch without the dilation"
    );
    let mut report = GlbSpawnReport::default();

    // Which glTF materials a drawable primitive references.
    let mut used = vec![false; scene.materials.len()];
    for part in &scene.parts {
        if let Some(i) = part.material
            && !part.mesh.indices.is_empty()
            && i < used.len()
        {
            used[i] = true;
        }
    }

    // Each referenced image, uploaded once per colour space it is sampled in.
    let mut uses = vec![[false; 2]; scene.images.len()];
    for (m, _) in scene.materials.iter().zip(&used).filter(|(_, u)| **u) {
        for (image, space) in image_uses(m) {
            if let Some(lanes) = image.and_then(|i| uses.get_mut(i)) {
                lanes[space_lane(space)] = true;
            }
        }
    }
    let mut slots = vec![[0u32; 2]; scene.images.len()];
    for (i, image) in scene.images.iter().enumerate() {
        let [srgb, linear] = uses[i];
        report.dual_colour_space_images += u32::from(srgb && linear);
        for (wanted, space) in [(srgb, ColorSpace::Srgb), (linear, ColorSpace::Linear)] {
            if !wanted {
                continue;
            }
            match uploader.upload_image(image, space) {
                Some(slot) => {
                    slots[i][space_lane(space)] = slot;
                    report.images_uploaded += 1;
                }
                None => report.images_undecodable += 1,
            }
        }
    }

    // One `Material` row per referenced glTF material.
    let mut handles: Vec<Option<(MaterialHandle, MaterialRoute)>> = vec![None; scene.materials.len()];
    for (i, m) in scene.materials.iter().enumerate().filter(|(i, _)| used[*i]) {
        let material = material_from_glb(m, &slots);
        report.khr_texcoord1_ignored += m.khr.texcoord1_refs;
        report.specular_textures_ignored += m.khr.specular_textures;
        report.dispersion_without_volume += u32::from(m.khr.dispersion > 0.0 && m.khr.thickness == 0.0);
        let route = material.route();
        let handle = materials.add(material);
        materials.pin(handle.index());
        handles[i] = Some((MaterialHandle::from_handle(handle), route));
        report.materials += 1;
    }
    let fallback_route = materials
        .get_by_index(u32::from(opts.fallback_material.0))
        .map_or(MaterialRoute::Opaque, Material::route);

    // One drawable per non-empty primitive, with its route marker.
    let centre = match (opts.recentre, glb_scene_bounds(scene)) {
        (true, Some((lo, hi))) => [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5],
        _ => [0.0; 3],
    };
    let mut shifted: Vec<Vertex> = Vec::new();
    for part in &scene.parts {
        if part.mesh.indices.is_empty() {
            report.primitives_skipped += 1;
            continue;
        }
        let vertices: &[Vertex] = if opts.recentre {
            shifted.clear();
            shifted.extend(part.mesh.vertices.iter().map(|v| {
                let mut v = *v;
                for (axis, c) in centre.iter().enumerate() {
                    v.position[axis] -= c;
                }
                v
            }));
            &shifted
        } else {
            &part.mesh.vertices
        };
        let mesh = uploader.upload_mesh(vertices, &part.mesh.indices);
        report.mirrored_placements_rewound += u32::from(part.mirrored_rewound);
        report.winding_disagrees_primitives +=
            u32::from(winding_census(vertices, &part.mesh.indices).mostly_clockwise());
        let (material, route) = part
            .material
            .and_then(|i| handles.get(i).copied().flatten())
            .unwrap_or((opts.fallback_material, fallback_route));

        let mut bundle = MeshBundle::new(mesh, opts.transform);
        bundle.material = material;
        let mut entity = commands.spawn(bundle);
        entity.insert(ShadowCaster);
        match route {
            MaterialRoute::Opaque => report.opaque += 1,
            MaterialRoute::Masked => {
                entity.insert(AlphaMasked);
                report.masked += 1;
            }
            MaterialRoute::Translucent => {
                entity.insert(Translucent);
                report.translucent += 1;
            }
        }
        report.entities += 1;
    }
    report
}

/// The lane of `space` in a per-image `[sRGB, linear]` pair.
fn space_lane(space: ColorSpace) -> usize {
    usize::from(space != ColorSpace::Srgb)
}

/// Every image slot a material samples, with the colour space glTF fixes for that slot.
fn image_uses(m: &GlbMaterial) -> [(Option<usize>, ColorSpace); 9] {
    use ColorSpace::{Linear, Srgb};
    [
        (m.base_color_image, Srgb),
        (m.emissive_image, Srgb),
        (m.khr.diffuse_transmission_color_tex, Srgb),
        (m.normal_image, Linear),
        (m.metallic_roughness_image, Linear),
        (m.occlusion_image, Linear),
        (m.khr.transmission_tex, Linear),
        (m.khr.thickness_tex, Linear),
        (m.khr.diffuse_transmission_tex, Linear),
    ]
}

/// The engine [`Material`] for one glTF material, its image slots resolved through `slots`
/// (the `[sRGB, linear]` bindless slots per image; `0` = none or undecodable).
fn material_from_glb(m: &GlbMaterial, slots: &[[u32; 2]]) -> Material {
    use ColorSpace::{Linear, Srgb};
    let slot = |image: Option<usize>, space: ColorSpace| -> u32 {
        image.and_then(|i| slots.get(i)).map_or(0, |s| s[space_lane(space)])
    };
    let mut material = Material::with_textures(
        MaterialGpu::new(m.base_color_factor, m.metallic, m.roughness, 0.5, m.emissive_factor, 0),
        MaterialTextures {
            albedo: slot(m.base_color_image, Srgb),
            normal: slot(m.normal_image, Linear),
            metal_rough: slot(m.metallic_roughness_image, Linear),
            ao: slot(m.occlusion_image, Linear),
            emissive: slot(m.emissive_image, Srgb),
        },
    );
    material.alpha = match m.alpha_mode {
        GlbAlphaMode::Opaque => AlphaMode::Opaque,
        GlbAlphaMode::Mask => AlphaMode::Mask { cutoff: m.alpha_cutoff },
        GlbAlphaMode::Blend => AlphaMode::Blend(BlendClass::PremulOver),
    };
    material.double_sided = m.double_sided;
    let k = &m.khr;
    material.x = MaterialX {
        transmission: k.transmission,
        ior: k.ior,
        thickness: k.thickness,
        attenuation_distance: k.attenuation_distance,
        attenuation_color: k.attenuation_color,
        specular: k.specular,
        specular_color: k.specular_color,
        dispersion: k.dispersion,
        diffuse_transmission_color: k.diffuse_transmission_color,
        diffuse_transmission: k.diffuse_transmission,
        transmission_tex: slot(k.transmission_tex, Linear),
        thickness_tex: slot(k.thickness_tex, Linear),
        diffuse_transmission_tex: slot(k.diffuse_transmission_tex, Linear),
        diffuse_transmission_color_tex: slot(k.diffuse_transmission_color_tex, Srgb),
    };
    material
}

/// The AABB of every part's vertices, as `(min, max)`; `None` for a scene with no vertex.
pub fn glb_scene_bounds(scene: &GlbScene) -> Option<([f32; 3], [f32; 3])> {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut any = false;
    for v in scene.parts.iter().flat_map(|p| p.mesh.vertices.iter()) {
        any = true;
        for axis in 0..3 {
            lo[axis] = lo[axis].min(v.position[axis]);
            hi[axis] = hi[axis].max(v.position[axis]);
        }
    }
    any.then_some((lo, hi))
}
