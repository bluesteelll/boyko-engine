//! Render wave RD6 (defect D-DS) — the device fixtures' shared scenes, the env knobs the drivers
//! set, and the screen-space geometry every D-DS gate reads its pixels at.
//!
//! Pulled into each binary with `#[path = "common/dds_scene.rs"] mod dds_scene;` (the OBS / RD1
//! idiom — `common/mod.rs` is not involved).
//!
//! # The scenes ([`ENV_SCENE`])
//!
//! - `card` — one vertical card (the built-in `plane`, turned to face `+Z`) at [`CARD_POS`], the
//!   camera on its axis at [`CARD_EYE`]; optional backdrop card [`BACKDROP_Z`] behind it. The sun
//!   shines along the view direction ([`SUN_VIEW`], from the camera side), so an authored front
//!   toward the camera is lit and one away from it is dark — with ambient and emission zeroed
//!   (rule xvi: no `SkyLight`, `LightingConfig`'s sky terms 0, emissive 0).
//! - `cube` — the built-in cube on the view axis seen from above-front: its X mirror is the same
//!   geometric cube, so a mirrored and an unmirrored instance render the same pixels.
//! - `caster` — a receiver floor (no `ShadowCaster`) and a horizontal 1 m card 1.5 m above it
//!   (`ShadowCaster`), lit from above by the sun ([`SUN_TOP`]) or by a shadow-casting point light
//!   ([`POINT_POS`]); `DDS_NOCARD=1` is the reference run without the card.
//!
//! # Determinism
//!
//! Static scene, no TAA, no DDGI, no particles: two runs captured at the same frame index render
//! the same bytes, which is what lets a driver compare one arm against another pixel for pixel.

#![allow(dead_code)] // each D-DS binary uses a subset of this module

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::texture_data::TextureData;
use boyko_render::{
    BindlessTextureTable, ColorSpace, GeometryLegs, LightingConfig, Material, MaterialGpu, MaterialTextures,
    MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig, ShadowDenoiseConfig, ShadowDenoiseMode,
    TextureAssetsExt, TextureGpu,
};

// ===============================================================================================
// The knobs
// ===============================================================================================

/// The scene: `card` (default), `cube`, `caster`.
pub const ENV_SCENE: &str = "BOYKO_DDS_SCENE";
/// The render path: `deferred` (default), `forward`, `forwardplus`, `vb`.
pub const ENV_PATH: &str = "BOYKO_DDS_PATH";
/// The card's facing: `front` (default; authored front toward the camera, or UP in `caster`) or
/// `back` (rotated 180°: toward the backdrop, or DOWN in `caster`).
pub const ENV_FACING: &str = "BOYKO_DDS_FACING";
/// `1`: the card's material is double-sided (default: the pinned default material).
pub const ENV_DS: &str = "BOYKO_DDS_DS";
/// `1`: the card / cube instance is mirrored in X (scale `(−1, 1, 1)`, an in-plane axis of the card).
pub const ENV_MIRROR: &str = "BOYKO_DDS_MIRROR";
/// `1`: the double-sided material is textured (an albedo and a flat normal map) — the TEX family.
pub const ENV_TEX: &str = "BOYKO_DDS_TEX";
/// `1`: a backdrop card behind the card (`card` scene).
pub const ENV_BACKDROP: &str = "BOYKO_DDS_BACKDROP";
/// `1`: omit the card (the reference run).
pub const ENV_NOCARD: &str = "BOYKO_DDS_NOCARD";
/// The `caster` scene's light: `sun` (default) or `point`.
pub const ENV_LIGHT: &str = "BOYKO_DDS_LIGHT";
/// `1`: the card's single-sided material carries a small emission (the ForwardPlus parity probe:
/// a drawn card is distinguishable from an undrawn pixel).
pub const ENV_EMISSIVE: &str = "BOYKO_DDS_EMISSIVE";
/// `1`: arm the temporal shadow denoiser (`hwrt` builds: the camera raster takes the MV / MVPM
/// families).
pub const ENV_DENOISE: &str = "BOYKO_DDS_DENOISE";

/// The square window edge every D-DS worker renders at.
pub const WIN: u32 = 256;
/// The vertical field of view.
pub const FOV_Y: f32 = core::f32::consts::FRAC_PI_3;
/// Near plane.
pub const NEAR: f32 = 0.1;
/// Far plane.
pub const FAR: f32 = 100.0;

/// What one D-DS worker boot arms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DdsArming {
    pub scene: DdsScene,
    pub path: RenderPath,
    pub back: bool,
    pub double_sided: bool,
    pub mirrored: bool,
    pub textured: bool,
    pub backdrop: bool,
    pub no_card: bool,
    pub point_light: bool,
    pub emissive: bool,
    pub denoise: bool,
}

/// The D-DS scenes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdsScene {
    Card,
    Cube,
    Caster,
}

impl DdsArming {
    /// Reads the arming from the environment the driver set.
    pub fn from_env() -> Self {
        let on = |k: &str| std::env::var(k).is_ok_and(|v| v.trim() == "1");
        let scene = match std::env::var(ENV_SCENE).as_deref().map(str::trim) {
            Ok("cube") => DdsScene::Cube,
            Ok("caster") => DdsScene::Caster,
            Ok("card") | Err(_) => DdsScene::Card,
            Ok(other) => panic!("{ENV_SCENE}={other} is not card|cube|caster"),
        };
        let path = match std::env::var(ENV_PATH).as_deref().map(str::trim) {
            Ok("forward") => RenderPath::Forward,
            Ok("forwardplus") => RenderPath::ForwardPlus,
            Ok("vb") => RenderPath::VisibilityBuffer,
            Ok("deferred") | Err(_) => RenderPath::Deferred,
            Ok(other) => panic!("{ENV_PATH}={other} is not deferred|forward|forwardplus|vb"),
        };
        let back = match std::env::var(ENV_FACING).as_deref().map(str::trim) {
            Ok("back") => true,
            Ok("front") | Err(_) => false,
            Ok(other) => panic!("{ENV_FACING}={other} is not front|back"),
        };
        let point_light = match std::env::var(ENV_LIGHT).as_deref().map(str::trim) {
            Ok("point") => true,
            Ok("sun") | Err(_) => false,
            Ok(other) => panic!("{ENV_LIGHT}={other} is not sun|point"),
        };
        Self {
            scene,
            path,
            back,
            double_sided: on(ENV_DS),
            mirrored: on(ENV_MIRROR),
            textured: on(ENV_TEX),
            backdrop: on(ENV_BACKDROP),
            no_card: on(ENV_NOCARD),
            point_light,
            emissive: on(ENV_EMISSIVE),
            denoise: on(ENV_DENOISE),
        }
    }

    /// The env word of `path`.
    pub fn path_word(path: RenderPath) -> &'static str {
        match path {
            RenderPath::Forward => "forward",
            RenderPath::ForwardPlus => "forwardplus",
            RenderPath::VisibilityBuffer => "vb",
            _ => "deferred",
        }
    }

    /// The depth image the path's `BOYKO_IMAGE_DUMP` names.
    pub fn depth_name(path: RenderPath) -> &'static str {
        match path {
            RenderPath::Forward | RenderPath::ForwardPlus => "forward_depth",
            RenderPath::VisibilityBuffer => "vb_depth",
            _ => "depth",
        }
    }
}

// ===============================================================================================
// The geometry
// ===============================================================================================

/// The `card` scene's camera (on the card's axis, looking down `−z`).
pub const CARD_EYE: Vec3 = Vec3::new(0.0, 1.0, 5.0);
/// The card's centre (and the camera's target).
pub const CARD_POS: Vec3 = Vec3::new(0.0, 1.0, 0.0);
/// The card's edge.
pub const CARD_SIZE: f32 = 1.6;
/// The backdrop card's `z` (2 units behind the card) and edge.
pub const BACKDROP_Z: f32 = -2.0;
/// The backdrop card's edge.
pub const BACKDROP_SIZE: f32 = 6.0;
/// The `card` scene's sun direction (TO the light): from the camera's side, along the view axis
/// (tilted so no term is exactly zero).
pub const SUN_VIEW: [f32; 3] = [0.1, 0.25, 1.0];

/// The `cube` scene's camera and target.
pub const CUBE_EYE: Vec3 = Vec3::new(0.0, 2.6, 4.0);
/// The cube's centre (on the view axis — its X mirror is the same geometric cube).
pub const CUBE_POS: Vec3 = Vec3::new(0.0, 0.8, 0.0);
/// The cube's edge.
pub const CUBE_EDGE: f32 = 1.2;
/// The `cube` scene's sun (TO the light).
pub const SUN_CUBE: [f32; 3] = [-0.45, 0.82, 0.36];

/// The `caster` scene's camera and target.
pub const CASTER_EYE: Vec3 = Vec3::new(0.0, 3.5, 5.0);
/// The `caster` camera's target.
pub const CASTER_TARGET: Vec3 = Vec3::new(0.0, 0.0, 0.0);
/// The horizontal caster card's centre (1.5 m above the floor) and edge.
pub const CASTER_CARD_POS: Vec3 = Vec3::new(0.0, 1.5, 0.0);
/// The caster card's edge.
pub const CASTER_CARD_SIZE: f32 = 1.0;
/// The floor's edge (at `y = 0`) and centre. Every floor vertex stays in front of the camera's
/// near plane: the VisibilityBuffer re-fetch reconstructs a near-plane-crossing triangle through
/// its simplified shrink-only clip, which displaces the reconstructed position (and with it every
/// shadow lookup) — a limitation of that path, not of what this fixture measures.
pub const FLOOR_SIZE: f32 = 8.0;
/// The floor's centre (see [`FLOOR_SIZE`]).
pub const FLOOR_POS: Vec3 = Vec3::new(0.0, 0.0, -1.0);
/// The `caster` scene's sun (TO the light): from above, tilted.
pub const SUN_TOP: [f32; 3] = [0.25, 1.0, 0.15];
/// The `caster` scene's shadow-casting point light.
pub const POINT_POS: Vec3 = Vec3::new(0.3, 4.0, 0.2);

fn normalize(v: [f32; 3]) -> Vec3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    Vec3::new(v[0] / l, v[1] / l, v[2] / l)
}

/// The quaternion of a rotation by `angle` radians about the unit `axis`.
pub fn axis_angle(axis: Vec3, angle: f32) -> Quat {
    let (s, c) = (angle * 0.5).sin_cos();
    Quat::new(axis.x * s, axis.y * s, axis.z * s, c)
}

/// The pixel (column, row; row 0 at the top) a world point projects to under a camera at `eye`
/// looking at `target` (world up `+y`) on a `win`-px square window, or `None` behind the eye.
pub fn project(eye: Vec3, target: Vec3, p: Vec3, win: u32) -> Option<(f32, f32)> {
    let f = normalize([target.x - eye.x, target.y - eye.y, target.z - eye.z]);
    let r = normalize([f.y * 0.0 - f.z * 1.0, f.z * 0.0 - f.x * 0.0, f.x * 1.0 - f.y * 0.0]);
    let u = Vec3::new(r.y * f.z - r.z * f.y, r.z * f.x - r.x * f.z, r.x * f.y - r.y * f.x);
    let d = Vec3::new(p.x - eye.x, p.y - eye.y, p.z - eye.z);
    let depth = d.x * f.x + d.y * f.y + d.z * f.z;
    if depth <= 1e-4 {
        return None;
    }
    let tan = (FOV_Y * 0.5).tan();
    let ndc_x = (d.x * r.x + d.y * r.y + d.z * r.z) / (depth * tan);
    let ndc_y = (d.x * u.x + d.y * u.y + d.z * u.z) / (depth * tan);
    let w = win as f32;
    Some(((ndc_x + 1.0) * 0.5 * w, (1.0 - ndc_y) * 0.5 * w))
}

/// An inclusive pixel rectangle `[x0, x1] × [y0, y1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    /// The bounding box of `pts` (pixel coordinates), shrunk by `erode` pixels on every side.
    pub fn bounding(pts: &[(f32, f32)], erode: f32) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let r = Self { x0: (x0 + erode).ceil() as u32, y0: (y0 + erode).ceil() as u32, x1: (x1 - erode).floor() as u32, y1: (y1 - erode).floor() as u32 };
        assert!(r.x1 > r.x0 && r.y1 > r.y0, "invariant: the eroded rectangle is non-empty ({r:?})");
        r
    }

    /// The rectangle scaled toward its centre by `keep` (0..1).
    pub fn shrunk(self, keep: f32) -> Self {
        let (cx, cy) = ((self.x0 + self.x1) as f32 * 0.5, (self.y0 + self.y1) as f32 * 0.5);
        let (hx, hy) = ((self.x1 - self.x0) as f32 * 0.5 * keep, (self.y1 - self.y0) as f32 * 0.5 * keep);
        Self { x0: (cx - hx).ceil() as u32, y0: (cy - hy).ceil() as u32, x1: (cx + hx).floor() as u32, y1: (cy + hy).floor() as u32 }
    }

    /// Every pixel of the rectangle.
    pub fn pixels(self) -> impl Iterator<Item = (u32, u32)> {
        (self.y0..=self.y1).flat_map(move |y| (self.x0..=self.x1).map(move |x| (x, y)))
    }
}

/// The four world corners of a card of edge `size` centred on `c` in the plane `z = c.z`.
fn vertical_card_corners(c: Vec3, size: f32) -> [Vec3; 4] {
    let h = size * 0.5;
    [
        Vec3::new(c.x - h, c.y - h, c.z),
        Vec3::new(c.x + h, c.y - h, c.z),
        Vec3::new(c.x + h, c.y + h, c.z),
        Vec3::new(c.x - h, c.y + h, c.z),
    ]
}

/// The `card` scene's card footprint, eroded by 2 px (every pixel inside is the card's).
pub fn card_mask() -> Rect {
    let pts: Vec<(f32, f32)> = vertical_card_corners(CARD_POS, CARD_SIZE)
        .iter()
        .map(|&p| project(CARD_EYE, CARD_POS, p, WIN).expect("the card is in front of the camera"))
        .collect();
    Rect::bounding(&pts, 2.0)
}

/// A band of the backdrop outside the card (left of it), eroded — the "backdrop drew" probe.
pub fn backdrop_band() -> Rect {
    let card = card_mask();
    let back: Vec<(f32, f32)> = vertical_card_corners(Vec3::new(CARD_POS.x, CARD_POS.y, BACKDROP_Z), BACKDROP_SIZE)
        .iter()
        .map(|&p| project(CARD_EYE, CARD_POS, p, WIN).expect("the backdrop is in front of the camera"))
        .collect();
    let b = Rect::bounding(&back, 4.0);
    Rect { x0: b.x0, y0: card.y0, x1: card.x0 - 6, y1: card.y1 }
}

/// The corner pixel no `card`-scene surface covers (the clear value probe).
pub const EMPTY_PIXEL: (u32, u32) = (2, 2);

/// The `caster` scene's predicted umbra on the floor: the card's corners projected along the
/// light onto `y = 0`, then to the screen; the inner 40 % of that box.
pub fn shadow_rect(point_light: bool) -> Rect {
    let h = CASTER_CARD_SIZE * 0.5;
    let c = CASTER_CARD_POS;
    let corners = [
        Vec3::new(c.x - h, c.y, c.z - h),
        Vec3::new(c.x + h, c.y, c.z - h),
        Vec3::new(c.x + h, c.y, c.z + h),
        Vec3::new(c.x - h, c.y, c.z + h),
    ];
    let to_floor = |p: Vec3| -> Vec3 {
        if point_light {
            let s = POINT_POS.y / (POINT_POS.y - p.y);
            Vec3::new(POINT_POS.x + (p.x - POINT_POS.x) * s, 0.0, POINT_POS.z + (p.z - POINT_POS.z) * s)
        } else {
            let l = normalize(SUN_TOP);
            Vec3::new(p.x - l.x * p.y / l.y, 0.0, p.z - l.z * p.y / l.y)
        }
    };
    let pts: Vec<(f32, f32)> = corners
        .iter()
        .map(|&p| project(CASTER_EYE, CASTER_TARGET, to_floor(p), WIN).expect("the shadow is in front of the camera"))
        .collect();
    Rect::bounding(&pts, 0.0).shrunk(0.4)
}

// ===============================================================================================
// The scene
// ===============================================================================================

/// The arming, as a resource the startup system reads.
#[derive(Resource)]
pub struct DdsArmingRes(pub DdsArming);

fn spawn_sun(commands: &mut Commands, dir: [f32; 3], illuminance: f32) {
    let sun_pose = Affine3A::look_at_rh(Vec3::ZERO, Vec3::new(dir[0], dir[1], dir[2]), Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(DirectionalLightObject {
        transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(sun_pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(dir, [1.0, 1.0, 1.0], illuminance),
    });
}

fn spawn_camera(commands: &mut Commands, eye: Vec3, target: Vec3) {
    let pose = Affine3A::look_at_rh(eye, target, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: FOV_Y, aspect: 1.0, near: NEAR, far: FAR },
    });
}

/// The card's material handle: the pinned default (single-sided), a double-sided grey, a
/// double-sided textured one, or a single-sided emissive one.
fn card_material(
    arming: DdsArming,
    materials: &mut Assets<Material>,
    textures: &mut Assets<TextureGpu>,
    bindless: &mut BindlessTextureTable,
    dev: &GpuDevice,
) -> MaterialHandle {
    if arming.textured {
        let white = TextureData { width: 4, height: 4, rgba8: vec![255; 64], color_space: ColorSpace::Srgb };
        let flat_n: Vec<u8> = (0..16).flat_map(|_| [128u8, 128, 255, 255]).collect();
        let flat = TextureData { width: 4, height: 4, rgba8: flat_n, color_space: ColorSpace::Linear };
        let albedo = textures.register_texture(dev.get(), bindless, &white);
        let normal = textures.register_texture(dev.get(), bindless, &flat);
        let mut m = Material::with_textures(
            MaterialGpu::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.6, 0.5, [0.0; 3], 0),
            MaterialTextures {
                albedo: textures.texture(albedo).bindless_slot,
                normal: textures.texture(normal).bindless_slot,
                metal_rough: 0,
                ao: 0,
                emissive: 0,
            },
        );
        m.double_sided = true;
        return MaterialHandle::from_handle(materials.add(m));
    }
    if arming.double_sided {
        let mut m = Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.6, 0.5, [0.0; 3], 0);
        m.double_sided = true;
        return MaterialHandle::from_handle(materials.add(m));
    }
    if arming.emissive {
        let m = Material::new([0.8, 0.8, 0.8, 1.0], 0.0, 0.6, 0.5, [0.05, 0.05, 0.05], 0);
        return MaterialHandle::from_handle(materials.add(m));
    }
    MaterialHandle(0)
}

/// Registers a built-in plane / cube — through the VB geometry table when the boot made one (a
/// VisibilityBuffer boot re-fetches every pixel's triangle from it, so a mesh must own a slot),
/// else the plain path.
struct Builtins<'a> {
    meshes: &'a mut Assets<MeshGpu>,
    table: &'a mut MeshGeometryTableSlot,
    dev: &'a GpuDevice,
}

impl Builtins<'_> {
    fn plane(&mut self, size: f32) -> MeshHandle {
        match self.table.0.as_mut() {
            Some(t) => self.meshes.plane_vb(self.dev.get(), size, t),
            None => self.meshes.plane(self.dev.get(), size),
        }
    }

    fn cube(&mut self, size: f32) -> MeshHandle {
        match self.table.0.as_mut() {
            Some(t) => self.meshes.cube_vb(self.dev.get(), size, t),
            None => self.meshes.cube(self.dev.get(), size),
        }
    }
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_arguments)]
fn dds_setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    dev: NonSendRes<GpuDevice>,
    arming: Res<DdsArmingRes>,
) {
    let a = arming.0;
    let mirror = if a.mirrored { Vec3::new(-1.0, 1.0, 1.0) } else { Vec3::ONE };
    let material = card_material(a, &mut materials, &mut textures, &mut bindless, &dev);
    let mut builtins = Builtins { meshes: &mut meshes, table: &mut geo_table, dev: &dev };
    match a.scene {
        DdsScene::Card => {
            if !a.no_card {
                let plane = builtins.plane(CARD_SIZE);
                // +90° about X turns the plane's +Y normal to +Z (toward the camera); `back` adds
                // 180° about the vertical, so the authored front faces −Z.
                let face_camera = axis_angle(Vec3::new(1.0, 0.0, 0.0), core::f32::consts::FRAC_PI_2);
                let rotation = if a.back {
                    axis_angle(Vec3::new(0.0, 1.0, 0.0), core::f32::consts::PI) * face_camera
                } else {
                    face_camera
                };
                let mut bundle = MeshBundle::new(plane, Transform { translation: CARD_POS, rotation, scale: mirror });
                bundle.material = material;
                commands.spawn(bundle);
            }
            if a.backdrop {
                let plane = builtins.plane(BACKDROP_SIZE);
                let rotation = axis_angle(Vec3::new(1.0, 0.0, 0.0), core::f32::consts::FRAC_PI_2);
                let t = Transform { translation: Vec3::new(CARD_POS.x, CARD_POS.y, BACKDROP_Z), rotation, scale: Vec3::ONE };
                commands.spawn(MeshBundle::new(plane, t));
            }
            spawn_sun(&mut commands, SUN_VIEW, 2.8);
            spawn_camera(&mut commands, CARD_EYE, CARD_POS);
        }
        DdsScene::Cube => {
            let cube = builtins.cube(CUBE_EDGE);
            let mut bundle = MeshBundle::new(cube, Transform { translation: CUBE_POS, rotation: Quat::IDENTITY, scale: mirror });
            bundle.material = material;
            commands.spawn(bundle);
            spawn_sun(&mut commands, SUN_CUBE, 2.8);
            spawn_camera(&mut commands, CUBE_EYE, CUBE_POS);
        }
        DdsScene::Caster => {
            let floor = builtins.plane(FLOOR_SIZE);
            // The floor carries its own (single-sided) material in every caster run, so the
            // reference run without the card selects the same Deferred raster family (a custom
            // material arms the per-instance-material pipeline for the whole frame).
            let floor_material = MaterialHandle::from_handle(materials.add(Material::new(
                [0.7, 0.7, 0.7, 1.0],
                0.0,
                0.6,
                0.5,
                [0.0; 3],
                0,
            )));
            let mut bundle = MeshBundle::new(floor, Transform::from_translation(FLOOR_POS));
            bundle.material = floor_material;
            commands.spawn(bundle);
            if !a.no_card {
                let plane = builtins.plane(CASTER_CARD_SIZE);
                // `front`: the authored front UP (toward the light); `back`: 180° about X, DOWN.
                let rotation = if a.back {
                    axis_angle(Vec3::new(1.0, 0.0, 0.0), core::f32::consts::PI)
                } else {
                    Quat::IDENTITY
                };
                let mut bundle = MeshBundle::new(plane, Transform { translation: CASTER_CARD_POS, rotation, scale: mirror });
                bundle.material = material;
                commands.spawn(bundle).insert(ShadowCaster);
            }
            if a.point_light {
                let e = commands
                    .spawn(PointLightObject {
                        transform: Transform::from_translation(POINT_POS),
                        global: GlobalTransform::IDENTITY,
                        light: PointLight::new([POINT_POS.x, POINT_POS.y, POINT_POS.z], [1.0, 1.0, 1.0], 800.0, 20.0),
                    })
                    .id();
                commands.entity(e).insert(CastsPunctualShadow);
            } else {
                spawn_sun(&mut commands, SUN_TOP, 2.8);
            }
            spawn_camera(&mut commands, CASTER_EYE, CASTER_TARGET);
        }
    }
}

/// Builds the D-DS app for `arming` on a [`WIN`]-px square window.
pub fn build_dds_app(title: &'static str, arming: DdsArming) -> App {
    build_dds_app_sized(title, arming, WIN, WIN)
}

/// [`build_dds_app`] on a `w`×`h` window (the WR probe's 2560×1440; the gates' masks assume
/// [`WIN`]²).
pub fn build_dds_app_sized(title: &'static str, arming: DdsArming, w: u32, h: u32) -> App {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(title, w, h));
    app.insert_resource(DdsArmingRes(arming));
    app.add_startup_system(dds_setup);
    // Configuration AFTER `add_plugins`, which seeds every default.
    app.insert_resource(RenderPathConfig { path: arming.path, legs: GeometryLegs::Mesh });
    // Rule (xvi): ambient zeroed — no `SkyLight` is spawned and the config's sky terms are 0.
    app.insert_resource(LightingConfig { sky_diffuse: [0.0; 3], sky_spec: [0.0; 3], ..LightingConfig::default() });
    app.insert_resource(CsmConfig { cascade_count: 3, ..CsmConfig::default() });
    if arming.point_light {
        app.insert_resource(ShadowConfig { enabled: true, ..ShadowConfig::default() });
    }
    if arming.denoise {
        app.insert_resource(ShadowDenoiseConfig { mode: ShadowDenoiseMode::Temporal, ..ShadowDenoiseConfig::default() });
    }
    app
}

/// Asserts the boot resolved the asked path and ran frames — a degraded boot proves nothing about
/// the path it was asked for.
pub fn assert_dds_resolved(app: &App, arming: DdsArming) {
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran (no windowed device?) — RED, never a skip");
    let resolved = *app.world().resource::<boyko_render::ResolvedRenderPath>();
    assert_eq!(resolved.path, arming.path, "PREMISE: asked for {:?}, the boot resolved {:?}", arming.path, resolved.path);
}
