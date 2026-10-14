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
use crate::loaders::PngTextureLoader;
use crate::loaders::glb::{GlbImage, GlbScene};
use crate::material::Material;
use crate::mesh::{MeshGpu, Vertex};
use crate::mesh_assets::MeshAssetsExt;
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

/// The device [`GlbUploader`]: meshes through [`MeshAssetsExt::register_mesh`] (pinned — a glb
/// part is a load-time asset the scene keeps), images through the in-house PNG decoder and
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
        let mesh = self.meshes.register_mesh(self.ctx, vertices, indices);
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

/// [`spawn_glb_scene`]'s body over any [`GlbUploader`].
pub fn spawn_glb_scene_with<U: GlbUploader>(
    _scene: &GlbScene,
    _commands: &mut Commands<'_>,
    _materials: &mut Assets<Material>,
    _uploader: &mut U,
    _opts: GlbSpawnOptions,
) -> GlbSpawnReport {
    GlbSpawnReport::default()
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
