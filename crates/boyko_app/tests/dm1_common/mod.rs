//! Shared fixture for the DM1 windowed gates (dynamic-materials rung DM1: Tier 1 made real, live
//! defects D-1 and D-2 fixed — `docs/render/DYNAMIC-MATERIALS-DESIGN-SPACE.md` §7 DM1).
//!
//! Every DM1 windowed binary drives ONE `App` through the production runner and reads its frames
//! back through the host's own capture channel, `BOYKO_HOST_DUMP` in burst mode
//! (`crates/boyko_app/src/host_dump.rs`): `BOYKO_HOST_DUMP_SETTLE=0` captures from the very first
//! presented frame, `BOYKO_HOST_DUMP_FRAMES=N` captures `N` consecutive presented frames, and the
//! `<stem>_frames.txt` sidecar carries one `key=value` state line per captured frame. Nothing here
//! reads a private type: the capture is the same image the swapchain presented.
//!
//! # Keying edits on the frame the sidecar names
//!
//! A test's edits fire when [`HostFrameStats::frames`] equals a target. The runner increments that
//! counter at the tail of every recorded loop iteration, at the SAME site it increments the
//! `frame_index` the sidecar prints as `frame=`, so "the Main run that sees `frames == k`" is the
//! ECS half of the frame the sidecar calls `frame=k`. [`Capture::frame`] asserts that every frame
//! a verdict names was actually captured, so a skipped or minimized frame that shifted the two
//! apart is a named premise failure, not a silent off-by-one.
//!
//! # The colour discriminator
//!
//! Every material this fixture mints has a black base colour, so a lit pixel's colour is carried by
//! the material's EMISSIVE lane — a channel only the material table carries on every path (the
//! Deferred per-instance raster lane carries `base_color`, never emissive). A row that reached the
//! GPU renders its emissive hue; an all-zero row renders near-black; a stale row renders the hue it
//! had before. [`Hue::of`] classifies a pixel into exactly one of those outcomes with margins wide
//! enough that tonemap and gamma cannot move a pixel across a class boundary.

#![allow(dead_code)] // each DM1 binary uses a subset of this fixture

pub mod edit_reaches_gpu;
pub mod pm_falling_edge;

use std::path::{Path, PathBuf};

use boyko_app::prelude::*;
use boyko_render::{Material, RenderPath, RenderPathConfig, ResolvedRenderPath};

/// The square window edge, in pixels. Small on purpose: a burst of `N` frames writes `N` BMPs of
/// `WIN² × 4` bytes.
pub const WIN: u32 = 256;

/// The camera sits on `+z` at this distance, looking at the origin.
pub const CAM_Z: f32 = 6.0;

/// The vertical field of view, radians.
pub const FOV_Y: f32 = core::f32::consts::FRAC_PI_3;

/// The cube edge every mesh sample uses.
pub const CUBE: f32 = 1.0;

/// The radius every SDF sample sphere uses.
pub const SDF_RADIUS: f32 = 0.6;

/// An emissive bright enough to dominate every other light term, and dim enough that the ACES
/// tonemap does not desaturate it toward white (measured on the parent at `4.0`: a GREEN emissive
/// presented as `[211, 245, 166]`).
pub const GLOW: f32 = 1.5;

/// A pure-red emissive.
pub const RED: [f32; 3] = [GLOW, 0.0, 0.0];
/// A pure-green emissive.
pub const GREEN: [f32; 3] = [0.0, GLOW, 0.0];
/// A pure-blue emissive.
pub const BLUE: [f32; 3] = [0.0, 0.0, GLOW];
/// A dim neutral emissive — a lit pixel with no dominant hue.
pub const GRAY: [f32; 3] = [0.3, 0.3, 0.3];

/// A black-based dielectric whose only visible colour is `emissive`.
pub fn glow_material(emissive: [f32; 3]) -> Material {
    Material::new([0.0, 0.0, 0.0, 1.0], 0.0, 0.9, 0.5, emissive, 0)
}

/// Where each capture of one run is written.
pub struct DumpPaths {
    /// The directory the burst is written into (cleared first).
    pub dir: PathBuf,
    /// The sidecar path.
    pub sidecar: PathBuf,
    /// Frames the burst captures, from presented frame 0.
    pub frames: u32,
}

/// Arms the runner's burst capture for `frames` presented frames starting at frame 0, into a
/// fresh temp directory named after `test`, and caps the loop so a run that never completes its
/// burst still exits.
///
/// Must be called at the top of the test, before `App::new`.
pub fn arm_dump(test: &str, frames: u32) -> DumpPaths {
    let dir = std::env::temp_dir().join("boyko_dm1").join(test);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .unwrap_or_else(|e| panic!("the stale capture dir {} could not be cleared ({e})", dir.display()));
    }
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let bmp = dir.join("f.bmp");
    let sidecar = dir.join("f_frames.txt");
    // SAFETY: `std::env::set_var` is `unsafe` in Rust 2024 because a concurrent reader on another
    // thread would race it. There is none: this runs at the top of a one-test binary run with
    // `--test-threads=1`, before `App::new` — before any engine thread, threadpool worker or frame
    // loop exists — and the runner's own reads happen later on this same thread.
    unsafe {
        std::env::set_var("BOYKO_HOST_DUMP", &bmp);
        std::env::set_var("BOYKO_HOST_DUMP_SETTLE", "0");
        std::env::set_var("BOYKO_HOST_DUMP_FRAMES", frames.to_string());
        std::env::set_var("BOYKO_WINDOW_FRAMES", (frames + 64).to_string());
    }
    DumpPaths { dir, sidecar, frames }
}

/// Builds the app every DM1 binary runs: the engine plugins at [`WIN`]², the requested render
/// path, and the caller's scene.
pub fn build_app(title: &'static str, path: RenderPath, legs: boyko_render::GeometryLegs) -> App {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window(title, WIN, WIN));
    // Inserted AFTER `add_plugins`, which seeds `RenderPathPlugin`'s Deferred default.
    app.insert_resource(RenderPathConfig { path, legs });
    app
}

/// Spawns the fixed light rig and camera every DM1 scene shares: a sun toward the camera side, a
/// dim sky, and a perspective camera on `+z` looking at the origin.
pub fn spawn_view(commands: &mut Commands) {
    const SUN_DIR: [f32; 3] = [0.25, 0.45, 0.86];
    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 1.0, 1.0], 2.0),
    });
    commands.spawn(SkyLight::new([0.10, 0.10, 0.12], [0.05, 0.05, 0.05]));

    let pose = Affine3A::look_at_rh(Vec3::new(0.0, 0.0, CAM_Z), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective { fov_y: FOV_Y, aspect: 1.0, near: 0.1, far: 100.0 },
    });
}

/// Spawns a [`CUBE`]-edged cube of `mesh` at `(x, y, 0)` carrying material row `row`.
pub fn spawn_cube(commands: &mut Commands, mesh: MeshHandle, x: f32, y: f32, row: u32) {
    commands.spawn(MeshBundle {
        material: MaterialHandle(u16::try_from(row).expect("invariant: DM1 fixtures mint < 65536 rows")),
        ..MeshBundle::new(mesh, Transform::from_translation(Vec3::new(x, y, 0.0)))
    });
}

/// Spawns an SDF sphere at `(x, y, 0)` carrying material row `row`.
pub fn spawn_sdf_sphere(commands: &mut Commands, x: f32, y: f32, row: u32) {
    let id = u16::try_from(row).expect("invariant: DM1 fixtures mint < 65536 rows");
    commands.spawn(SdfPrimitive(SdfEdit::sphere([x, y, 0.0], SDF_RADIUS, sdf_op::UNION, 0.0).with_material(id)));
}

/// The window pixel that shows world point `(x, y, z)` under [`spawn_view`]'s camera (`z` toward
/// the camera). Rounded to the nearest texel centre; `y` grows downward like the image rows.
pub fn project(x: f32, y: f32, z: f32) -> (u32, u32) {
    let half = (CAM_Z - z) * (FOV_Y * 0.5).tan();
    let ndc_x = x / half;
    let ndc_y = y / half;
    let px = ((ndc_x + 1.0) * 0.5 * WIN as f32).floor();
    let py = ((1.0 - ndc_y) * 0.5 * WIN as f32).floor();
    (px as u32, py as u32)
}

/// The pixel at the centre of the camera-facing face of a cube at `(x, y, 0)`.
pub fn cube_pixel(x: f32, y: f32) -> (u32, u32) {
    project(x, y, CUBE * 0.5)
}

/// The pixel at the camera-facing pole of an SDF sphere at `(x, y, 0)`.
pub fn sdf_pixel(x: f32, y: f32) -> (u32, u32) {
    project(x, y, SDF_RADIUS)
}

/// A decoded 32-bpp bottom-up BMP, as the host dump writes it.
pub struct Bmp {
    pub width: u32,
    pub height: u32,
    data_offset: usize,
    bytes: Vec<u8>,
}

impl Bmp {
    fn parse(bytes: Vec<u8>, path: &Path) -> Self {
        assert!(bytes.len() >= 54 && &bytes[..2] == b"BM", "{} is not a BMP", path.display());
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().expect("invariant: 4 bytes"));
        let data_offset = u32_at(10) as usize;
        let width = u32_at(18);
        let height = i32::from_le_bytes(bytes[22..26].try_into().expect("invariant: 4 bytes"));
        let bpp = u16::from_le_bytes(bytes[28..30].try_into().expect("invariant: 2 bytes"));
        assert!(height > 0, "{}: the host dump writes bottom-up rows", path.display());
        assert_eq!(bpp, 32, "{}: the host dump writes 32 bpp", path.display());
        let height = height as u32;
        assert!(
            bytes.len() >= data_offset + (width * height * 4) as usize,
            "{}: truncated pixel data",
            path.display()
        );
        Self { width, height, data_offset, bytes }
    }

    /// The `[r, g, b]` texel at `(x, y)`, `y` counted from the TOP row.
    pub fn rgb(&self, (x, y): (u32, u32)) -> [u8; 3] {
        assert!(x < self.width && y < self.height, "pixel ({x}, {y}) outside {}x{}", self.width, self.height);
        let row = self.height - 1 - y;
        let o = self.data_offset + ((row * self.width + x) * 4) as usize;
        [self.bytes[o + 2], self.bytes[o + 1], self.bytes[o]]
    }
}

/// One captured frame: the sidecar's state line and its image.
pub struct CapturedFrame {
    /// The runner's `frame_index` (`frame=`).
    pub frame: u32,
    /// The in-flight slot the frame's uploads went into (`slot=`).
    pub slot: u32,
    /// Every `key=value` pair of the state line, in order.
    pub fields: Vec<(String, String)>,
    /// The presented image.
    pub image: Bmp,
}

impl CapturedFrame {
    /// The value of `key` on this frame's state line, if the line carries it.
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// Every frame one burst captured.
pub struct Capture {
    pub frames: Vec<CapturedFrame>,
}

impl Capture {
    /// Reads the sidecar and every BMP it names.
    pub fn load(paths: &DumpPaths) -> Self {
        let text = std::fs::read_to_string(&paths.sidecar).unwrap_or_else(|e| {
            panic!(
                "no sidecar at {} ({e}) — the run never reached its capture frames (a failed boot, \
                 a hung loop, or no device); that is RED here, never a skip",
                paths.sidecar.display()
            )
        });
        let mut frames = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let mut fields = Vec::new();
            for tok in line.split_whitespace().skip(1) {
                if let Some((k, v)) = tok.split_once('=') {
                    fields.push((k.to_owned(), v.to_owned()));
                }
            }
            let get = |k: &str| {
                fields
                    .iter()
                    .find(|(fk, _)| fk == k)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| panic!("sidecar line lacks `{k}=`: {line}"))
            };
            let frame: u32 = get("frame").parse().expect("invariant: frame= is an integer");
            let slot: u32 = get("slot").parse().expect("invariant: slot= is an integer");
            let file = paths.dir.join(get("file"));
            let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("no frame at {} ({e})", file.display()));
            let image = Bmp::parse(bytes, &file);
            assert!(
                image.width == WIN && image.height == WIN,
                "frame {frame}: captured {}x{}, the fixture's pixel map assumes {WIN}x{WIN}",
                image.width,
                image.height
            );
            frames.push(CapturedFrame { frame, slot, fields, image });
        }
        assert_eq!(
            frames.len() as u32,
            paths.frames,
            "the burst asked for {} frames, the sidecar holds {}",
            paths.frames,
            frames.len()
        );
        Self { frames }
    }

    /// The captured frame whose `frame=` is `k` — a premise failure if the burst did not capture
    /// it (a skipped or minimized frame shifted the capture away from the ECS frame counter).
    pub fn frame(&self, k: u32) -> &CapturedFrame {
        self.frames.iter().find(|f| f.frame == k).unwrap_or_else(|| {
            let got: Vec<u32> = self.frames.iter().map(|f| f.frame).collect();
            panic!("PREMISE: frame {k} was not captured (captured frames: {got:?})")
        })
    }

    /// Asserts the sample at `px` is bit-identical on every frame of `range` — a sample that moves
    /// without an edit (TAA, jitter, an animated light) cannot carry a before/after verdict.
    pub fn assert_stable(&self, name: &str, px: (u32, u32), range: core::ops::RangeInclusive<u32>) -> [u8; 3] {
        let first = self.frame(*range.start()).image.rgb(px);
        for k in range.clone() {
            let v = self.frame(k).image.rgb(px);
            assert_eq!(v, first, "PREMISE: sample `{name}` at {px:?} is not stable over frames {range:?} (frame {k} = {v:?}, frame {} = {first:?})", range.start());
        }
        first
    }

    /// The per-frame table of `px`'s colour and hue over `range`, for a verdict message.
    pub fn trace(&self, px: (u32, u32), range: core::ops::RangeInclusive<u32>) -> String {
        let mut out = String::new();
        for k in range {
            let f = self.frame(k);
            let v = f.image.rgb(px);
            out.push_str(&format!("\n    frame {k:>3} slot {} rgb {v:?} {:?}", f.slot, Hue::of(v)));
        }
        out
    }
}

/// The class a sample pixel falls into — see the module doc's discriminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    /// One channel dominates.
    Red,
    Green,
    Blue,
    /// Lit, no dominant channel.
    Neutral,
    /// Near-black: an all-zero material row (or unlit background).
    Dark,
    /// Between classes: neither verdict can be read off it.
    Ambiguous,
}

impl Hue {
    /// Classifies an 8-bit display-space texel.
    pub fn of([r, g, b]: [u8; 3]) -> Self {
        let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if max < 40 {
            return Self::Dark;
        }
        // A hue class needs its channel bright AND clear of both others by a margin the tonemap's
        // channel mixing cannot close; NEUTRAL needs all three within a narrow band.
        let dominant = |c: i32, o1: i32, o2: i32| c >= 100 && c - o1 >= 40 && c - o2 >= 40;
        if dominant(r, g, b) {
            Self::Red
        } else if dominant(g, r, b) {
            Self::Green
        } else if dominant(b, r, g) {
            Self::Blue
        } else if max - min <= 20 {
            Self::Neutral
        } else {
            Self::Ambiguous
        }
    }
}

/// Asserts the resolved render path is the one the fixture asked for — a degraded boot (a device
/// that cannot run the path, or legs collapsed to `Mesh`) proves nothing about that path.
pub fn assert_resolved(app: &App, path: RenderPath, sdf_leg_needed: bool) {
    let resolved = *app.world().resource::<ResolvedRenderPath>();
    assert_eq!(resolved.path, path, "PREMISE: asked for {path:?}, the boot resolved {:?}", resolved.path);
    assert!(resolved.mesh_leg, "PREMISE: the mesh leg is off on this boot");
    if sdf_leg_needed {
        assert!(resolved.sdf_leg, "PREMISE: the SDF leg is off on {path:?} (legs {:?})", resolved.legs);
    }
}

/// `true` iff the runner's loop ran at all — a windowless box returns from `run` before the first
/// frame, and every DM1 gate treats that as RED (the gate exists to render).
pub fn assert_ran(app: &App) {
    let frames = app.world().resource::<HostFrameStats>().frames;
    assert!(frames > 0, "the frame loop never ran (no windowed device?) — RED, never a skip");
}
