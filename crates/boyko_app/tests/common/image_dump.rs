//! OBS (render-wave plan H0 Scope 6) — the `BOYKO_IMAGE_DUMP` file reader and texel decoders
//! (`R8G8B8A8`, `R16G16B16A16_SFLOAT`, `R32_SFLOAT`, `R32G32B32A32_SFLOAT`, `D32_SFLOAT`), and the
//! depth worker's scene.
//!
//! Pulled into the image-dump binaries with `#[path = "common/image_dump.rs"] mod image_dump;`.
//! The reader is stdlib only: it parses the 32-byte header the recorder stamped (magic, format,
//! extent, mip, layers, frame index, texel bytes) and decodes texels by that format, so a dump in
//! a format the reader does not know is RED here rather than misread.

#![allow(dead_code)] // each binary uses a subset of this module

use std::path::{Path, PathBuf};

use boyko_app::prelude::*;
use boyko_render::{GeometryLegs, RenderPath, RenderPathConfig};

/// `VK_FORMAT_R8G8B8A8_UNORM`.
pub const VK_R8G8B8A8_UNORM: u32 = 37;
/// `VK_FORMAT_B8G8R8A8_UNORM`.
pub const VK_B8G8R8A8_UNORM: u32 = 44;
/// `VK_FORMAT_R16G16B16A16_SFLOAT`.
pub const VK_R16G16B16A16_SFLOAT: u32 = 97;
/// `VK_FORMAT_R32_SFLOAT`.
pub const VK_R32_SFLOAT: u32 = 100;
/// `VK_FORMAT_R32G32B32A32_SFLOAT`.
pub const VK_R32G32B32A32_SFLOAT: u32 = 109;
/// `VK_FORMAT_D32_SFLOAT`.
pub const VK_D32_SFLOAT: u32 = 126;

/// The header's first word, `"OBSD"` read little-endian.
pub const MAGIC: u32 = u32::from_le_bytes(*b"OBSD");

/// One dump file: the recorder's header fields and the raw texels.
#[derive(Clone, Debug)]
pub struct DumpFile {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub mip: u32,
    pub layers: u32,
    pub frame: u32,
    pub texel_bytes: u32,
    pub texels: Vec<u8>,
}

/// Converts an IEEE half to `f32` — a one-line delegate to `boyko_shaderdsl::half`, the ONE binary16
/// decoder (render wave PX0, OPEN-ITEMS #3; kept `pub` because `obs_image_dump_selfcheck.rs` asserts
/// on it, `px0/critique.md` W5).
pub fn f16_to_f32(h: u16) -> f32 {
    boyko_shaderdsl::half::f16_bits_to_f32(u32::from(h))
}

impl DumpFile {
    /// Parses a dump file's bytes — the header and its length check are `boyko_shaderdsl::dump`'s
    /// (one parser for every dump reader; render wave PX0, OPEN-ITEMS #3), the format table
    /// `boyko_shaderdsl::pixfmt`'s.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let (h, texels) = boyko_shaderdsl::dump::parse(bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            format: h.format,
            width: h.width,
            height: h.height,
            mip: h.mip,
            layers: h.layers,
            frame: h.frame,
            texel_bytes: h.texel_bytes,
            texels: texels.to_vec(),
        })
    }

    /// Reads and parses `path`; a missing or malformed file is RED.
    pub fn load(path: &Path) -> Self {
        let bytes = std::fs::read(path).unwrap_or_else(|e| {
            panic!("no image dump at {} ({e}) — the capture never ran or was not copied; RED", path.display())
        });
        Self::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn at(&self, x: u32, y: u32) -> &[u8] {
        assert!(x < self.width && y < self.height, "texel ({x}, {y}) outside {}x{}", self.width, self.height);
        let o = ((y * self.width + x) * self.texel_bytes) as usize;
        &self.texels[o..o + self.texel_bytes as usize]
    }

    /// An 8-bit texel as `[r, g, b, a]` (BGRA is reordered).
    pub fn rgba8(&self, x: u32, y: u32) -> [u8; 4] {
        let t = self.at(x, y);
        match self.format {
            VK_R8G8B8A8_UNORM => [t[0], t[1], t[2], t[3]],
            VK_B8G8R8A8_UNORM => [t[2], t[1], t[0], t[3]],
            f => panic!("rgba8 on VkFormat {f}"),
        }
    }

    /// A texel as four `f32`s (one-channel formats fill `[v, 0, 0, 1]`; 8-bit ones are /255) —
    /// `boyko_shaderdsl::pixfmt::decode_texel`, the one texel decoder.
    pub fn texel_f32(&self, x: u32, y: u32) -> [f32; 4] {
        boyko_shaderdsl::pixfmt::decode_texel(self.format, self.at(x, y))
            .unwrap_or_else(|| panic!("texel_f32 on VkFormat {}", self.format))
    }

    /// A depth texel (`D32_SFLOAT`).
    pub fn depth(&self, x: u32, y: u32) -> f32 {
        assert_eq!(self.format, VK_D32_SFLOAT, "depth() on VkFormat {}", self.format);
        self.texel_f32(x, y)[0]
    }
}

/// Encodes a synthetic dump (the inverse of [`DumpFile::parse`]) — the fixtures' writer.
pub fn encode(format: u32, width: u32, height: u32, frame: u32, texel_bytes: u32, texels: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + texels.len());
    for w in [MAGIC, format, width, height, 0, 1, frame, texel_bytes] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out.extend_from_slice(texels);
    out
}

/// Every `<name>_<frame>.bin` in `dir`, sorted by frame.
pub fn files_of(dir: &Path, name: &str) -> Vec<(u32, PathBuf)> {
    let prefix = format!("{name}_");
    let mut out: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("no dump dir {} ({e})", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|p| {
            let stem = p.file_name()?.to_str()?.strip_suffix(".bin")?.strip_prefix(&prefix)?.parse().ok()?;
            Some((stem, p))
        })
        .collect();
    out.sort_by_key(|(f, _)| *f);
    out
}

// ===============================================================================================
// The depth worker's scene
// ===============================================================================================

/// The depth scene's window edge.
pub const DEPTH_WIN: u32 = 256;
/// The camera's distance from the origin along `+z`.
pub const DEPTH_CAM_Z: f32 = 5.0;
/// The cube's edge (its camera-facing face sits at `z = DEPTH_CUBE / 2`).
pub const DEPTH_CUBE: f32 = 1.0;
/// The camera's vertical field of view.
pub const DEPTH_FOV_Y: f32 = core::f32::consts::FRAC_PI_3;
/// The near plane.
pub const DEPTH_NEAR: f32 = 0.1;
/// The far plane.
pub const DEPTH_FAR: f32 = 100.0;

fn depth_setup(mut commands: Commands, mut meshes: NonSendResMut<Assets<MeshGpu>>, dev: NonSendRes<GpuDevice>) {
    let cube = meshes.cube(dev.get(), DEPTH_CUBE);
    commands.spawn(MeshBundle::new(cube, Transform::IDENTITY));
    commands.spawn(SkyLight::new([0.3, 0.3, 0.3], [0.1, 0.1, 0.1]));
    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 0.0, DEPTH_CAM_Z), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform { translation: pose.translation, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: DEPTH_FOV_Y, aspect: 1.0, near: DEPTH_NEAR, far: DEPTH_FAR },
    });
}

/// One cube of edge [`DEPTH_CUBE`] at the origin, its front face toward a camera on `+z` at
/// [`DEPTH_CAM_Z`]; mesh leg only, no AA (no jitter), so the centre texel's depth is a closed form.
pub fn build_depth_app(title: &'static str, path: RenderPath) -> App {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(title, DEPTH_WIN, DEPTH_WIN));
    app.add_startup_system(depth_setup);
    app.insert_resource(RenderPathConfig { path, legs: GeometryLegs::Mesh });
    app
}

/// The Euclidean ray length from the camera to the cube's front face through the centre of
/// texel `(x, y)` (row 0 at the top).
pub fn centre_ray_t(x: u32, y: u32) -> f32 {
    let tan = (DEPTH_FOV_Y * 0.5).tan();
    let ndc_x = (x as f32 + 0.5) / DEPTH_WIN as f32 * 2.0 - 1.0;
    let ndc_y = 1.0 - (y as f32 + 0.5) / DEPTH_WIN as f32 * 2.0;
    let dir = [ndc_x * tan, ndc_y * tan, 1.0f32];
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    (DEPTH_CAM_Z - DEPTH_CUBE * 0.5) * len
}

/// The reverse-Z depth of a point at view depth `z` (`boyko_render::view::forward_view_z_from_depth`
/// inverted: `z = B / (d − A)` ⇒ `d = B / z + A`, `A = −near/(far−near)`, `B = near·far/(far−near)`).
pub fn reverse_z(z: f32) -> f32 {
    let range = DEPTH_FAR - DEPTH_NEAR;
    let a = -DEPTH_NEAR / range;
    let b = DEPTH_NEAR * DEPTH_FAR / range;
    b / z + a
}
