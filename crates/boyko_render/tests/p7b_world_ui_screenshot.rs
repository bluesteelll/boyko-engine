//! GUI P7b — the world-space cursor-ray PICK + projection + CPU-proxy depth
//! OCCLUSION screenshot harness (RTX 3060 oracle) + CPU setup-validation asserts.
//!
//! # What this proves (one composite image)
//!
//! The REAL P7b ECS systems decide, on the CPU, which world-anchored UI nameplates
//! are drawn and at what projected screen pixel; the SDF marcher renders the scene
//! the pick bounds were authored against; and the two are composited so the owner
//! eyeballs "the nameplate floats over its object, and the label behind a sphere is
//! gone". ZERO new engine/core code: the pick / project / visibility systems
//! (`ui_world_pick_system` / `ui_world_project_system` / `ui_world_visibility_system`),
//! the camera resolve (`resolve_active_camera`), the `ViewUniform -> marcher` bridge
//! (`composite_perspective_from_view`), the MSDF emitter + GPU pack lane
//! (`emit_glyphs` / `pack_ui_instance`), and the offscreen UI render+readback recipe
//! all pre-exist and are proven (`p7b_pick.rs`, `camera_drives_render_gpu.rs`,
//! `ui_hud_screenshot.rs`).
//!
//! # The scene (perspective, camera at the world origin looking down -Z)
//!
//! Three SDF spheres (`run_marcher` folds a LIST of `SdfEdit::sphere`), each ALSO a
//! `UiPickable { Sphere{radius} }` whose radius MATCHES the rendered radius so the
//! pick bound coincides with the lit pixels:
//! The three spheres are placed SYMMETRICALLY (equal screen gaps) at a moderate
//! 45° FOV so off-axis perspective foreshortening (an off-axis sphere projects to a
//! radially-stretched ellipse — physically correct, but exaggerated by a wide FOV)
//! stays subtle and balanced left↔right:
//!  - `S0` at `(0, 0, -7.5)` r=1 — dead-ahead on the -Z axis, so it projects to the
//!    viewport CENTER and the center-pixel cursor ray picks it. It carries an
//!    `EntityAnchor` nameplate `"PICKED"` offset `+y` above it; the pick → visibility
//!    chain SHOWS that nameplate (it is the hovered object) and HIDES every other
//!    `EntityAnchor` root. The label is CENTER-aligned on the projected anchor x.
//!  - `S1` at `(2.8, 0, -7.5)` r=1 — the occluder for the HIDDEN label (right).
//!  - `S2` at `(-2.8, 0, -7.5)` r=1 — a second visible sphere (left), symmetric to S1.
//!
//! Two `WorldPos`-anchored labels, BOTH `depth_test = true` (they are NOT
//! hover-gated — only `EntityAnchor` roots are — so occlusion alone gates them):
//!  - `"HIDDEN"` anchored at `(5.6, 0, -15)` — exactly twice S1's eye-vector, so the
//!    eye→anchor ray passes THROUGH S1's center; `ui_world_pick_system` sets that
//!    root's `UiWorldOccluded` bit → it is NOT drawn (the occlusion proof).
//!  - `"VISIBLE"` anchored at `(-2.8, 2.4, -7.5)` — open sky directly above S2 with
//!    nothing between it and the eye → `UiWorldOccluded` CLEAR → drawn above the left
//!    sphere.
//!
//! # System order (the documented P7b pipeline, run as plain calls on `&mut`)
//!
//! `resolve_active_camera` → `propagate_transforms` → `ui_world_project_system` →
//! `ui_world_pick_system` → `ui_world_visibility_system`. The world systems are
//! EXCLUSIVE (`&mut EcsMaster`), so a test drives them in sequence with `run_system`
//! / direct calls — the exact pattern `p7a_world_anchor.rs` / `p7b_pick.rs` use. No
//! layout pass is wired: this test emits glyph quads DIRECTLY at each drawn root's
//! `UiWorldProjection.{screen_x,screen_y}`, which is the seam the layout pass would
//! seed anyway, so the screenshot shows the projected position without the extra
//! one-frame layout settle.
//!
//! # The composite (LOW GPU RISK — two independent readbacks, blended on the CPU)
//!
//! 1. `run_marcher` renders the SDF scene to a packed-RGB `Vec<u32>` (a compute-only
//!    dispatch, the `camera_drives_render_gpu` recipe verbatim); `unpack_rgb` →
//!    an RGBA8 background `bg`.
//! 2. The drawn nameplates' `UiInstance`s render through the UI path into an
//!    offscreen `R8G8B8A8Unorm` target on a TRANSPARENT clear `(0,0,0,0)`
//!    (the `ui_hud_screenshot` recipe verbatim) → `fg` RGBA8.
//! 3. The UI FS writes STRAIGHT (un-premultiplied) RGBA with coverage in the ALPHA
//!    channel (confirmed from `ui_hud_screenshot`: at `coverage == 1` over an OPAQUE
//!    clear the texel equals `FG_BYTES` exactly, i.e. `out = fg*a + clear*(1-a)` with
//!    `a == fg.a == 255`; over a transparent clear `out.a == coverage`). So the CPU
//!    composite is straight alpha-over: `out = bg*(1 - a) + fg.rgb*a`, `a = fg.a/255`.
//!    The nameplate color is bright yellow so it reads over the lit grey sphere.
//! 4. `write_bmp` writes `target/screenshots/p7b_world_ui.bmp`.
//!
//! # Split: CPU setup-validation runs in-workflow; the GPU screenshot is `#[ignore]`d
//!
//! The non-ignored `p7b_*_setup` tests run ONLY the ECS systems on the CPU (no
//! device) and assert the scene is built so the GPU run is meaningful: the pick
//! resolved to S0, S0's nameplate is drawn, HIDDEN is occluded, VISIBLE is not. They
//! catch a mis-built scene before the (crash-prone) GPU boot. The screenshot test is
//! `#[ignore]`d inside `mod gpu` — Vulkan boot can hang a headless run; the
//! orchestrator runs it on the RTX.
//!
//! The owner-run command (one line, RTX 3060):
//!
//! ```text
//! cargo test -p boyko-render --test p7b_world_ui_screenshot p7b_world_ui_screenshot \
//!   -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Output image: `D:\claude\BoykoEngine\target\screenshots\p7b_world_ui.bmp`

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;

use boyko_fontbake::atlas::{AtlasKind, BakedFont};
use boyko_fontbake::face::TtfFace;

use boyko_input::PhysicalInput;

use boyko_math::{Affine3A, Mat3, Quat, Vec3};

use boyko_scene::transform::GlobalTransform;
use boyko_scene::{
    ActiveCamera, Camera, Projection, ViewUniform, propagate_transforms, resolve_active_camera,
};

use boyko_ui::components::{ComputedRect, UiRoot};
use boyko_ui::resources::UiViewport;
use boyko_ui::text::{emit_glyphs, FontId, FontTable, GlyphInstance, TextAlign, UiText};
use boyko_ui::world::components::{UiPickShape, UiPickable, UiWorldOccluded};
use boyko_ui::world::{
    HoveredWorldEntity, UiWorldAnchor, UiWorldCulled, UiWorldHidden, UiWorldHoverState,
    UiWorldProjection, UiWorldScratch, WorldTarget, ui_world_pick_system, ui_world_project_system,
    ui_world_visibility_system,
};

use boyko_render::{
    PackInput, UiInstance, composite_perspective_from_view, pack_ui_instance,
};

use boyko_rhi_vulkan::compute::{SdfEdit, sdf_op};

use core::f32::consts::FRAC_PI_4;

// ════════════════════════════════════════════════════════════════════════════
// Fixed proof-image geometry (the perspective camera + scene constants)
// ════════════════════════════════════════════════════════════════════════════

/// Proof-image width (logical == physical px; scale 1.0).
const W: u32 = 768;
/// Proof-image height.
const H: u32 = 512;
/// Camera aspect ratio (`W / H` == 1.5) — kept in sync with the viewport so the
/// project system's aspect-mismatch `debug_assert!` is satisfied.
const ASPECT: f32 = W as f32 / H as f32;
/// Vertical field of view (45°): narrower than the legacy 60° so off-axis spheres
/// foreshorten less (the radial-ellipse perspective distortion stays subtle). The
/// marcher bridge reads the same `view.fov_y`, so render + projection stay in sync.
const FOV_Y: f32 = FRAC_PI_4;
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

/// Every sphere's world radius (also the matching `UiPickable` sphere radius).
const SPHERE_R: f32 = 1.0;

/// The common sphere depth (pushed back from the legacy -6 so the narrower 45° FOV
/// still frames all three spheres comfortably).
const SPHERE_Z: f32 = -7.5;
/// The symmetric left/right sphere offset from the optical axis (equal screen gaps).
const SIDE_X: f32 = 2.8;

/// `S0` (the PICKED object): dead-ahead on the -Z axis ⇒ projects to the viewport
/// center, so the center-pixel cursor ray hits it.
const S0_POS: [f32; 3] = [0.0, 0.0, SPHERE_Z];
/// `S1` (the occluder for the HIDDEN label): off to the RIGHT, symmetric to S2.
const S1_POS: [f32; 3] = [SIDE_X, 0.0, SPHERE_Z];
/// `S2` (a second on-screen sphere): off to the LEFT, symmetric to S1.
const S2_POS: [f32; 3] = [-SIDE_X, 0.0, SPHERE_Z];

/// The HIDDEN label's fixed world anchor: exactly `2 * S1_POS`, so the eye→anchor
/// ray (from the origin eye) is COLLINEAR with the eye→S1 ray and passes through
/// S1's center ⇒ the occlusion pass sets `UiWorldOccluded`. Behind S1 from the
/// camera (z = 2*SPHERE_Z < SPHERE_Z), so S1 is genuinely between eye and this point.
const HIDDEN_ANCHOR: [f32; 3] = [2.0 * SIDE_X, 0.0, 2.0 * SPHERE_Z];
/// The VISIBLE label's fixed world anchor: open sky directly above S2 (same x/z, +y)
/// with nothing between it and the eye ⇒ `UiWorldOccluded` stays clear; it projects
/// just above the left sphere.
const VISIBLE_ANCHOR: [f32; 3] = [-SIDE_X, 2.4, SPHERE_Z];

/// The upward offset of S0's `EntityAnchor` nameplate (above the sphere top).
const NAMEPLATE_OFFSET: [f32; 3] = [0.0, SPHERE_R + 0.6, 0.0];

/// The nameplate text for the picked object.
const PICKED_TEXT: &str = "PICKED";
/// The occluded (behind-S1) label text.
const HIDDEN_TEXT: &str = "HIDDEN";
/// The unoccluded label text.
const VISIBLE_TEXT: &str = "VISIBLE";

/// Render em size for the MSDF nameplates, logical px.
const TEXT_SIZE_PX: f32 = 40.0;
/// Bright yellow (straight RGBA8, opaque) so the nameplate reads over the lit grey
/// sphere; `byte0 = R`, `byte3 = A` (the `ui_hud_screenshot` `FG` convention).
const NAMEPLATE_COLOR: u32 = 0xFF00_FFFF; // R=FF G=FF B=00 A=FF (yellow)

/// The fixture font baked into the MSDF atlas (clean glyphs + a continuous SDF so
/// the FS anti-aliases the curves; OFL).
const MSDF_FIXTURE: &str = "Ubuntu-Light.ttf";
/// The codepoints the MSDF atlas covers — the union of all three labels' letters.
const MSDF_GLYPHS: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'H', 'I', 'K', 'L', 'N', 'P', 'S', 'T', 'V',
];

// ════════════════════════════════════════════════════════════════════════════
// MSDF font baking + glyph instance emission (mirrors ui_hud_screenshot W3)
// ════════════════════════════════════════════════════════════════════════════

/// Loads the checked-in `boyko_fontbake` fixture font bytes. The path is resolved
/// from THIS crate's manifest dir (`crates/boyko_render`) up to the fontbake crate's
/// `fixtures/` — the fixture lives with the baker, not this crate. Cloned from
/// `ui_hud_screenshot::msdf_fixture_bytes`.
fn msdf_fixture_bytes() -> std::io::Result<Vec<u8>> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("..")
        .join("boyko_fontbake")
        .join("fixtures")
        .join(MSDF_FIXTURE);
    std::fs::read(path)
}

/// Bakes a REAL MTSDF [`BakedFont`] from the fixture font over [`MSDF_GLYPHS`] via
/// the public `boyko_fontbake` bake API (`TtfFace::from_bytes` -> `bake_font`).
/// Cloned from `ui_hud_screenshot::msdf_baked_font`.
fn msdf_baked_font() -> BakedFont {
    let bytes = msdf_fixture_bytes().expect("read the checked-in Ubuntu-Light.ttf fixture");
    let face = TtfFace::from_bytes(&bytes).expect("parse the fixture font");
    boyko_fontbake::atlas::bake_font(&face, MSDF_GLYPHS, None)
}

/// Shapes `text` through the REAL emitter against `font` and packs each shaped glyph
/// into a GPU [`UiInstance`] via the SAME `pack_ui_instance` text lane the production
/// path uses, HORIZONTALLY CENTERED on the projected anchor x `origin_x` (a nameplate
/// reads centered over its object, not growing rightward from the anchor). The run is
/// shaped once at x=0 to MEASURE its pixel width, then every glyph quad is shifted by
/// `origin_x - width/2`; the vertical placement stays at the projected `origin_y`. The
/// emitter places each quad at the glyph's true plane bounds (sized to
/// [`TEXT_SIZE_PX`]) and carries its true atlas UV. Mirrors
/// `ui_hud_screenshot::msdf_hud_instances`, with the origin a parameter + centering.
fn msdf_instances_at(text: &str, font: &BakedFont, origin_x: f32, origin_y: f32) -> Vec<UiInstance> {
    let mut fonts = FontTable::new();
    fonts.load(font);

    let style = UiText {
        color: NAMEPLATE_COLOR,
        size_px: TEXT_SIZE_PX,
        font: FontId(0),
        align: TextAlign::Left,
        _pad: 0,
    };
    // Shape at x=0 so the run's left edge is ~0; the vertical baseline lands relative
    // to `origin_y` (the projected anchor pixel).
    let rect = ComputedRect {
        x: 0.0,
        y: origin_y,
        w: 0.0,
        h: 0.0,
    };
    let mut shaped: Vec<GlyphInstance> = Vec::new();
    emit_glyphs(
        &style,
        &rect,
        text,
        boyko_ui::components::StackIndex(0),
        None,
        &fonts,
        &mut shaped,
    );

    // The run's pixel width = the right edge of the last glyph (left edge is ~0). Shift
    // every quad left by half that, so the run is centered on the projected anchor x.
    // `GlyphInstance::rect` is `[x, y, w, h]`.
    let width = shaped
        .iter()
        .fold(0.0_f32, |m, g| m.max(g.rect[0] + g.rect[2]));
    let dx = origin_x - width * 0.5;

    shaped
        .iter()
        .map(|g| {
            let mut rect = g.rect;
            rect[0] += dx;
            pack_ui_instance(
                &PackInput {
                    rect,
                    color: g.color,
                    border_color: 0,
                    corner_radius: [0.0; 4],
                    border_width: [0.0; 4],
                    clip: None,
                    text_uv: Some(g.uv),
                },
                1.0,
            )
        })
        .collect()
}

// ════════════════════════════════════════════════════════════════════════════
// Marcher buffer layout (mirrors camera_drives_render_gpu.rs verbatim)
// ════════════════════════════════════════════════════════════════════════════

/// The three SDF spheres of the scene, folded UNION into one edit list (the marcher
/// folds a list, so multiple spheres render). The positions + radii are the SAME the
/// `UiPickable` bounds use, so the lit pixels and the pick bounds coincide.
fn sphere_scene() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere(S0_POS, SPHERE_R, sdf_op::UNION, 0.0),
        SdfEdit::sphere(S1_POS, SPHERE_R, sdf_op::UNION, 0.0),
        SdfEdit::sphere(S2_POS, SPHERE_R, sdf_op::UNION, 0.0),
    ]
}

/// Unpacks a marcher pixel `(0x00BBGGRR)` to `[r, g, b]` (0..=255). Cloned from
/// `camera_drives_render_gpu::unpack_rgb` (returning `u8` lanes for the composite).
fn unpack_rgb(packed: u32) -> [u8; 3] {
    [
        (packed & 0xFF) as u8,
        ((packed >> 8) & 0xFF) as u8,
        ((packed >> 16) & 0xFF) as u8,
    ]
}

// ════════════════════════════════════════════════════════════════════════════
// No-dep BMP writer (cloned from ui_hud_screenshot::write_bmp)
// ════════════════════════════════════════════════════════════════════════════

/// Writes `rgba` (`w*h*4` tightly-packed R8G8B8A8) as a dependency-free 32bpp BGRA
/// BMP. Top-down via a NEGATIVE `biHeight`, so the in-memory top-left texel is the
/// image top-left — NO row flip. The single channel swap (RGBA -> BGRA) is here ONLY.
/// Cloned verbatim from `ui_hud_screenshot::write_bmp`.
fn write_bmp(path: &Path, rgba: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    debug_assert_eq!(
        rgba.len(),
        (w * h * 4) as usize,
        "invariant: BMP body is w*h*4 bytes"
    );
    let pixel_bytes = w * h * 4;
    let pixel_offset: u32 = 54; // 14-byte file header + 40-byte info header.
    let file_size = pixel_offset + pixel_bytes;

    let mut buf = Vec::with_capacity(file_size as usize);
    // --- BITMAPFILEHEADER (14 bytes) ---
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    buf.extend_from_slice(&pixel_offset.to_le_bytes());
    // --- BITMAPINFOHEADER (40 bytes) ---
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    buf.extend_from_slice(&(-(h as i32)).to_le_bytes()); // biHeight (negative => top-down)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&pixel_bytes.to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    // --- pixel data: RGBA -> BGRA (the ONLY channel swap; no row flip) ---
    for px in rgba.chunks_exact(4) {
        buf.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &buf)
}

/// The screenshot output path under the workspace target dir (mirrors
/// `ui_hud_screenshot::screenshot_path` with the P7b name).
fn screenshot_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("target")
        .join("screenshots")
        .join("p7b_world_ui.bmp")
}

// ════════════════════════════════════════════════════════════════════════════
// The P7b ECS world (camera + scene spheres + nameplate roots + the systems)
// ════════════════════════════════════════════════════════════════════════════

/// The camera bundle (mirrors `camera_drives_render_gpu::CameraBundle`).
#[derive(Bundle)]
struct CameraBundle {
    camera: Camera,
    projection: Projection,
    global: GlobalTransform,
}

/// A built P7b world: the `EcsMaster` plus the handles a caller asserts against.
struct P7bWorld {
    world: EcsMaster,
    /// `S0`, the picked scene entity.
    s0: Entity,
    /// The `EntityAnchor` nameplate root tracking `S0`.
    root_picked: Entity,
    /// The `WorldPos` HIDDEN label root (occluded behind `S1`).
    root_hidden: Entity,
    /// The `WorldPos` VISIBLE label root (open space).
    root_visible: Entity,
}

/// Spawns an entity via a one-shot `Commands` system and harvests its handle (the
/// established Phase-11/19 `Arc<Mutex<..>>` probe; cloned from `p7b_pick.rs`).
fn spawn_via_commands<F>(world: &mut EcsMaster, f: F) -> Entity
where
    F: FnOnce(&mut Commands) -> Entity + Send + Sync + 'static,
{
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let f = Mutex::new(Some(f));
    world.run_system(move |mut cmds: Commands| {
        let f = f.lock().unwrap().take().expect("spawn closure runs once");
        let e = f(&mut cmds);
        *probe.lock().unwrap() = Some(e);
    });
    let e = sink.lock().unwrap().expect("spawned handle");
    assert!(world.has_entity(e), "spawned entity is live after apply");
    e
}

/// Spawns a perspective camera at the world origin (identity rotation, looking down
/// -Z). Mirrors `camera_drives_render_gpu::spawn_camera_at` with the proof-image
/// aspect/FOV.
fn spawn_camera(world: &mut EcsMaster) -> Entity {
    let camera = Camera::DEFAULT;
    let projection = Projection::Perspective {
        fov_y: FOV_Y,
        aspect: ASPECT,
        near: NEAR,
        far: FAR,
    };
    let global = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: Vec3::ZERO,
    });
    spawn_via_commands(world, move |cmds| {
        cmds.spawn(CameraBundle {
            camera,
            projection,
            global,
        })
        .id()
    })
}

/// Spawns a SCENE sphere entity at `pos`: a `GlobalTransform` (identity rotation,
/// unit scale) + a `UiPickable` sphere bound whose radius MATCHES the rendered SDF
/// sphere, so the pick bound coincides with the lit pixels. Returns the handle.
fn spawn_sphere(world: &mut EcsMaster, pos: [f32; 3], radius: f32) -> Entity {
    let gt = GlobalTransform(Affine3A::from_translation_rotation_scale(
        Vec3::new(pos[0], pos[1], pos[2]),
        Quat::IDENTITY,
        Vec3::ONE,
    ));
    spawn_via_commands(world, move |cmds| {
        let mut ec = cmds.spawn(UiPickable {
            shape: UiPickShape::Sphere { radius },
            layers: u32::MAX,
        });
        ec.insert(gt);
        ec.id()
    })
}

/// Spawns a world-anchor UI ROOT (the P7b nameplate). `UiWorldAnchor`'s
/// `#[require(UiWorldProjection)]` auto-inserts the projection column. A `UiRoot` +
/// `ComputedRect` make it a real root (mirrors `p7b_pick::spawn_anchor_root`, minus
/// the `UiLayout` since this test emits glyphs directly at the projection).
fn spawn_anchor_root(world: &mut EcsMaster, anchor: UiWorldAnchor) -> Entity {
    spawn_via_commands(world, move |cmds| {
        let mut ec = cmds.spawn(anchor);
        ec.insert(UiRoot);
        ec.insert(ComputedRect::default());
        ec.id()
    })
}

/// Builds the P7b world: seeds the camera/UI resources, spawns the camera + three
/// spheres + the three nameplate roots, then runs the documented P7b system order
/// ONCE so the projections / hover / occlusion bits are settled for the caller.
fn build_p7b_world() -> P7bWorld {
    let mut world = EcsMaster::new();

    // Camera resources (mirrors `camera_drives_render_gpu::camera_app`).
    world.insert_resource(ActiveCamera::default());
    world.insert_resource(ViewUniform::default());
    // The UI viewport must agree with the camera aspect (the project system's
    // aspect-mismatch debug_assert), at scale 1.0 (logical == physical px).
    world.insert_resource(UiViewport {
        width: W as f32,
        height: H as f32,
        scale_factor: 1.0,
        generation: 0,
    });
    // The pick reads the cursor from PhysicalInput; place it over S0's projected
    // center (S0 is on the -Z axis ⇒ viewport center) and mark the cursor active.
    let mut physical = PhysicalInput::new();
    physical.cursor_pos = [(W as f64) * 0.5, (H as f64) * 0.5];
    physical.cursor_inside = true;
    physical.window_focused = true;
    world.insert_resource(physical);
    world.insert_resource(HoveredWorldEntity::default());
    world.insert_resource(UiWorldHoverState::default());
    world.insert_resource(UiWorldScratch::default());
    // `propagate_transforms` needs its scratch resource seeded.
    world.insert_resource(boyko_scene::TransformPropagationScratch::default());

    let _camera = spawn_camera(&mut world);

    // The three scene spheres (S0 picked / S1 occluder / S2 left).
    let s0 = spawn_sphere(&mut world, S0_POS, SPHERE_R);
    let _s1 = spawn_sphere(&mut world, S1_POS, SPHERE_R);
    let _s2 = spawn_sphere(&mut world, S2_POS, SPHERE_R);

    // S0's nameplate: an EntityAnchor root tracking S0, offset up, NO depth test
    // (it is the hovered-object overlay; its visibility is hover-driven). Hover-
    // gating shows it because the cursor picks S0.
    let root_picked = spawn_anchor_root(
        &mut world,
        UiWorldAnchor {
            target: WorldTarget::EntityAnchor(s0),
            offset: NAMEPLATE_OFFSET,
            depth_test: false,
            ..Default::default()
        },
    );

    // HIDDEN: a WorldPos label behind S1, depth_test ON ⇒ occluded by S1.
    let root_hidden = spawn_anchor_root(
        &mut world,
        UiWorldAnchor {
            target: WorldTarget::WorldPos(HIDDEN_ANCHOR),
            depth_test: true,
            ..Default::default()
        },
    );

    // VISIBLE: a WorldPos label in open space, depth_test ON ⇒ NOT occluded.
    let root_visible = spawn_anchor_root(
        &mut world,
        UiWorldAnchor {
            target: WorldTarget::WorldPos(VISIBLE_ANCHOR),
            depth_test: true,
            ..Default::default()
        },
    );

    run_p7b_pipeline(&mut world);

    P7bWorld {
        world,
        s0,
        root_picked,
        root_hidden,
        root_visible,
    }
}

/// Runs the documented P7b system order once, as plain calls (the world systems are
/// exclusive — the simplest faithful driver, mirroring `p7b_pick::PickHarness::run`
/// extended with the camera + transform producers):
/// `resolve_active_camera` → `propagate_transforms` → `ui_world_project_system` →
/// `ui_world_pick_system` → `ui_world_visibility_system`.
fn run_p7b_pipeline(world: &mut EcsMaster) {
    world.run_system(resolve_active_camera);
    world.run_system(propagate_transforms);
    world.run_system(ui_world_project_system);
    world.run_system(ui_world_pick_system);
    world.run_system(ui_world_visibility_system);
}

/// A world-anchor root is DRAWN iff it is NOT culled, NOT hidden (hover), NOT
/// occluded (depth), and its projection is `visible`. The single P7b draw-gate the
/// layout pass also applies — read here directly so the screenshot emits exactly the
/// roots the engine would lay out.
fn root_is_drawn(world: &EcsMaster, root: Entity) -> bool {
    if world.is_enabled::<UiWorldCulled>(root)
        || world.is_enabled::<UiWorldHidden>(root)
        || world.is_enabled::<UiWorldOccluded>(root)
    {
        return false;
    }
    world
        .get_component::<UiWorldProjection>(root)
        .map(|p| p.visible)
        .unwrap_or(false)
}

/// Collects `(text, projection)` for every DRAWN nameplate root, in a fixed order
/// (picked, hidden, visible) so the emitted instances are deterministic. A root that
/// is culled / hidden / occluded / invisible is omitted — exactly the P7b output.
fn drawn_nameplates(p7b: &P7bWorld) -> Vec<(&'static str, UiWorldProjection)> {
    let mut out = Vec::new();
    for (text, root) in [
        (PICKED_TEXT, p7b.root_picked),
        (HIDDEN_TEXT, p7b.root_hidden),
        (VISIBLE_TEXT, p7b.root_visible),
    ] {
        if root_is_drawn(&p7b.world, root) {
            let proj = *p7b
                .world
                .get_component::<UiWorldProjection>(root)
                .expect("a drawn root has a projection");
            out.push((text, proj));
        }
    }
    out
}

// ════════════════════════════════════════════════════════════════════════════
// CPU setup-validation tests (run in-workflow; NOT ignored — no GPU device)
// ════════════════════════════════════════════════════════════════════════════

/// The pick resolved to S0 (the dead-ahead sphere under the center cursor), and S0's
/// hover-driven nameplate is shown while the other roots are not hover-hidden
/// (WorldPos roots are never hover-gated). Proves the cursor → pick → visibility
/// chain selected the intended object before the GPU run.
#[test]
fn p7b_pick_resolves_to_s0_setup() {
    let p7b = build_p7b_world();
    assert_eq!(
        p7b.world.resource::<HoveredWorldEntity>().0,
        Some(p7b.s0),
        "the center cursor ray picks the dead-ahead sphere S0"
    );
    assert!(
        !p7b.world.is_enabled::<UiWorldHidden>(p7b.root_picked),
        "S0's EntityAnchor nameplate is SHOWN (it tracks the picked entity)"
    );
}

/// HIDDEN is occluded by S1; VISIBLE is not occluded — the depth-proxy proof. Both
/// are `WorldPos` roots (not hover-gated), so occlusion alone decides their draw.
#[test]
fn p7b_occlusion_setup() {
    let p7b = build_p7b_world();
    assert!(
        p7b.world.is_enabled::<UiWorldOccluded>(p7b.root_hidden),
        "the HIDDEN label behind S1 is occluded (UiWorldOccluded set)"
    );
    assert!(
        !p7b.world.is_enabled::<UiWorldOccluded>(p7b.root_visible),
        "the VISIBLE label in open space is NOT occluded"
    );
    // Both anchors are on-screen / in-front (a meaningful, non-vacuous occlusion).
    assert!(
        p7b.world
            .get_component::<UiWorldProjection>(p7b.root_hidden)
            .expect("hidden root projection")
            .visible,
        "the HIDDEN anchor point is itself on-screen (occlusion, not frustum-cull, hides it)"
    );
    assert!(
        p7b.world
            .get_component::<UiWorldProjection>(p7b.root_visible)
            .expect("visible root projection")
            .visible,
        "the VISIBLE anchor point is on-screen"
    );
}

/// The drawn-nameplate set is exactly {PICKED, VISIBLE}: S0's picked nameplate and
/// the unoccluded VISIBLE label are drawn; HIDDEN (occluded) is omitted. This is the
/// precise set the screenshot emits, so a draw-gate regression fails in-workflow.
#[test]
fn p7b_drawn_set_setup() {
    let p7b = build_p7b_world();
    let drawn: Vec<&str> = drawn_nameplates(&p7b).iter().map(|(t, _)| *t).collect();
    assert!(drawn.contains(&PICKED_TEXT), "S0's PICKED nameplate is drawn");
    assert!(drawn.contains(&VISIBLE_TEXT), "the VISIBLE label is drawn");
    assert!(
        !drawn.contains(&HIDDEN_TEXT),
        "the HIDDEN label (occluded behind S1) is NOT drawn"
    );

    // The picked nameplate floats ABOVE S0's center: its projected y is above the
    // viewport center (the +y offset projects to a SMALLER screen-y, +y is down).
    let picked = drawn_nameplates(&p7b)
        .into_iter()
        .find(|(t, _)| *t == PICKED_TEXT)
        .expect("PICKED is drawn");
    assert!(
        picked.1.screen_y < H as f32 * 0.5,
        "the PICKED nameplate's +y offset projects ABOVE the viewport center (screen_y {} < {})",
        picked.1.screen_y,
        H as f32 * 0.5
    );
}

// ════════════════════════════════════════════════════════════════════════════
// The GPU screenshot test (#[ignore]) — owner-run on the RTX
// ════════════════════════════════════════════════════════════════════════════

#[cfg(not(miri))]
mod gpu {
    use super::*;

    use core::ptr::NonNull;

    use boyko_rhi::{
        BarrierAccess, BarrierStage, BufferDesc, BufferImageCopy, BufferUsage, ComputePipelineDesc,
        Format, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage,
        LoadOp, MemoryLocation, RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder,
        RhiDevice, RhiQueue, ShaderStage, StoreOp, TextureDesc, TextureDimension,
    };
    use boyko_rhi_vulkan::compute::{
        COMPOSITE_DEPTH_BASE_WORDS, COMPOSITE_PUSH_CONSTANT_BYTES, CompositePushConstants,
        LOCAL_SIZE_X, MESH_DEPTH_CLEAR, sdf_depth_composite_spirv,
    };
    use boyko_rhi_vulkan::device::VulkanContext;
    use boyko_render::{RhiContext, UiOrtho, record_ui_rects};

    use common::{assert_validation_clean, boot_or_skip};

    // ── Marcher buffer layout (mirrors camera_drives_render_gpu.rs verbatim) ──

    const DEPTH_BASE: usize = 196;
    const _: () = assert!(DEPTH_BASE == COMPOSITE_DEPTH_BASE_WORDS);

    fn buffer_words(w: u32, h: u32) -> usize {
        DEPTH_BASE + 2 * (w as usize) * (h as usize)
    }

    fn pixel_base_words(w: u32, h: u32) -> usize {
        DEPTH_BASE + (w as usize) * (h as usize)
    }

    fn group_count(w: u32, h: u32) -> u32 {
        ((w as u64 * h as u64) as u32).div_ceil(LOCAL_SIZE_X)
    }

    fn seed_buffer(base: NonNull<u8>, edits: &[SdfEdit], w: u32, h: u32) {
        let dst = base.as_ptr().cast::<u32>();
        let n_pixels = (w as usize) * (h as usize);
        // SAFETY: `dst` is the start of a `buffer_words(w,h)*4`-byte host-coherent
        // mapping (the buffer was created at exactly that size); every index written
        // is < that word count. No GPU work is in flight (submit happens after);
        // `write_unaligned` tolerates the sub-allocated offset.
        unsafe { dst.write_unaligned(edits.len() as u32) };
        for (i, e) in edits.iter().enumerate() {
            let off = 4 + i * 12;
            let words = [
                e.center[0].to_bits(),
                e.center[1].to_bits(),
                e.center[2].to_bits(),
                e.center[3].to_bits(),
                e.params[0].to_bits(),
                e.params[1].to_bits(),
                e.params[2].to_bits(),
                e.params[3].to_bits(),
                e.kind,
                e.op,
                e.smoothness.to_bits(),
                e._pad,
            ];
            for (j, &word) in words.iter().enumerate() {
                // SAFETY: `off + j < DEPTH_BASE` for the fixed-cap edit array, in-bounds.
                unsafe { dst.add(off + j).write_unaligned(word) };
            }
        }
        let clear_bits = MESH_DEPTH_CLEAR.to_bits();
        for i in 0..n_pixels {
            // SAFETY: `DEPTH_BASE + i` for `i < n_pixels` is the depth region, in-bounds.
            unsafe { dst.add(DEPTH_BASE + i).write_unaligned(clear_bits) };
        }
    }

    fn read_pixels(base: NonNull<u8>, w: u32, h: u32) -> Vec<u32> {
        let n = (w as usize) * (h as usize);
        let pbase = pixel_base_words(w, h);
        let p = base.as_ptr().cast::<u32>();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            // SAFETY: `pbase + i` for `i < n` is the pixel region, in-bounds; a fence
            // wait preceded this read so GPU writes are complete + coherent.
            out.push(unsafe { p.add(pbase + i).read_unaligned() });
        }
        out
    }

    /// Records + submits ONE compute-only marcher dispatch driven by `pc`,
    /// fence-waits, asserts validation-clean, and returns the readback pixels.
    /// Cloned from `camera_drives_render_gpu::run_marcher`.
    fn run_marcher(
        ctx: &VulkanContext,
        edits: &[SdfEdit],
        pc: CompositePushConstants,
        w: u32,
        h: u32,
        label: &str,
    ) -> Vec<u32> {
        let device: &VulkanContext = ctx;
        let queue = ctx.rhi_queue();

        let buffer = device
            .create_buffer(&BufferDesc {
                size: (buffer_words(w, h) as u64) * 4,
                usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("runtime-extent shared storage buffer");

        {
            let mapped = device.buffer_mapped_ptr(&buffer).expect("host-visible buffer mapped");
            seed_buffer(mapped, edits, w, h);
        }

        let cs = device
            .create_shader_module(sdf_depth_composite_spirv())
            .expect("composite compute shader module");
        let compute = device
            .create_compute_pipeline(&ComputePipelineDesc {
                module: &cs,
                entry: c"main",
                push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
                bind_group_layout: None,
            })
            .expect("composite compute pipeline");

        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("command encoder");

        encoder.begin().expect("begin");
        encoder.bind_compute_pipeline(&compute);
        encoder.bind_storage_buffer(&buffer, 0, 0);
        encoder.push_constants(ShaderStage::COMPUTE, 0, pc.as_bytes());
        encoder.dispatch(group_count(w, h), 1, 1);
        encoder.end().expect("end");

        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");

        let mapped = device.buffer_mapped_ptr(&buffer).expect("host-visible buffer mapped");
        let pixels = read_pixels(mapped, w, h);
        assert_eq!(pixels.len(), (w as usize) * (h as usize), "{label}: full readback");

        assert_validation_clean(ctx);

        // SAFETY: every resource was created on `device` and is destroyed exactly
        // once; the submission completed (fence-waited above), so none is GPU-in-use.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
            device.destroy_compute_pipeline(compute);
            device.destroy_shader_module(cs);
            device.destroy_buffer(buffer);
        }

        pixels
    }

    // ── Offscreen UI render+readback (mirrors ui_hud_screenshot::render_glyphs) ──

    /// Renders `instances` against `font` into a `w * h` offscreen `R8G8B8A8Unorm`
    /// target on a TRANSPARENT clear `(0,0,0,0)`, returning the `w*h*4`-byte RGBA8
    /// readback. Cloned from `ui_hud_screenshot::render_glyphs`; the ONLY change is
    /// the clear color (transparent, so the alpha carries glyph coverage for the CPU
    /// composite) — the GPU path itself is identical and proven.
    fn render_ui(
        rhi: &mut RhiContext,
        instances: &[UiInstance],
        font: &BakedFont,
        w: u32,
        h: u32,
    ) -> Vec<u8> {
        let size: u64 = (w as u64) * (h as u64) * 4;

        rhi.ui_setup(
            Format::R8G8B8A8Unorm,
            boyko_render::ui_rect_vs_spirv(),
            boyko_render::ui_rect_fs_spirv(),
            4,
            font,
        )
        .expect("ui_setup (UI pipeline + atlas upload + per-FIF rings)");

        let ortho = UiOrtho::for_extent(w, h);
        // SAFETY: the per-FIF rings were just created by `ui_setup`; nothing was ever
        // submitted against them, so slot 0 is free to host-write unfenced.
        let token = unsafe { boyko_rhi_vulkan::swapchain::FrameWriteToken::forge_unfenced(0) };
        let plan = rhi
            .ui_upload(instances, ortho, token)
            .expect("ui_upload (memcpy into the FIF ring + POD UiFramePlan)");
        debug_assert_eq!(
            plan.instance_count as usize,
            instances.len(),
            "invariant: every nameplate glyph instance uploaded"
        );

        let (pipeline, bind_group) = rhi
            .ui_handles(plan.frame_index)
            .expect("ui_handles after ui_setup");

        let device = rhi.context();
        let queue = device.rhi_queue();

        let output = device
            .create_texture(&TextureDesc {
                width: w,
                height: h,
                depth: 1,
                format: Format::R8G8B8A8Unorm,
                dimension: TextureDimension::D2,
                usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
                array_layers: 1,
            })
            .expect("offscreen output texture");

        let staging = device
            .create_buffer(&BufferDesc {
                size,
                usage: BufferUsage::TRANSFER_DST,
                location: MemoryLocation::HostVisibleCoherent,
            })
            .expect("host-visible readback staging buffer");

        let fence = device.create_fence(false).expect("fence");
        let mut encoder = device.create_command_encoder().expect("command encoder");
        let full = RenderArea {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };

        encoder.begin().expect("begin");

        encoder.image_barrier(&ImageBarrierDesc {
            texture: &output,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::ColorAttachmentOptimal,
            range: ImageSubresourceRange::COLOR,
        });

        // CLEAR pass: paint the TRANSPARENT background, then close it so the UI pass
        // opens its own LoadOp::Load scope (the ui_hud_screenshot recipe).
        let clear_attachment = [RenderingAttachment {
            texture: &output,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: [0.0, 0.0, 0.0, 0.0],
        }];
        encoder.begin_rendering(&RenderingDesc {
            render_area: full,
            colors: &clear_attachment,
            depth: None,
        });
        encoder.end_rendering();

        // UI pass: a fresh LoadOp::Load scope; the shared recorder records the
        // instanced glyph draw.
        let ui_attachment = [RenderingAttachment {
            texture: &output,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Load,
            store_op: StoreOp::Store,
            clear_color: [0.0; 4],
        }];
        encoder.begin_rendering(&RenderingDesc {
            render_area: full,
            colors: &ui_attachment,
            depth: None,
        });
        // SAFETY: recording is open inside a `begin_rendering(LoadOp::Load)` scope
        // whose single color attachment's format (`R8G8B8A8Unorm`) equals the UI
        // pipeline's `color_formats[0]`, at `full`; `pipeline`/`bind_group` are the
        // live current-frame (MF-7) UI handles whose ring holds `plan.instance_count`
        // valid records uploaded for `plan.frame_index` above; the pipeline declares
        // the UI bind-group layout (binding 0 SSBO, binding 1 atlas, binding 2 UBO)
        // and a 16-byte VERTEX push range.
        unsafe {
            record_ui_rects(&mut encoder, &full, &plan, pipeline, bind_group);
        }
        encoder.end_rendering();

        encoder.image_barrier(&ImageBarrierDesc {
            texture: &output,
            src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            dst_stage: BarrierStage::TRANSFER,
            src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            dst_access: BarrierAccess::TRANSFER_READ,
            old_layout: ImageLayout::ColorAttachmentOptimal,
            new_layout: ImageLayout::TransferSrcOptimal,
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
        encoder.copy_image_to_buffer(&output, ImageLayout::TransferSrcOptimal, &staging, &regions);

        encoder.end().expect("end");

        queue.submit(&encoder, &fence).expect("submit");
        device.wait_fence(&fence, u64::MAX).expect("wait_fence");

        let dst_ptr = device
            .buffer_mapped_ptr(&staging)
            .expect("host-visible staging buffer is mapped");
        let mut out = vec![0u8; size as usize];
        // SAFETY: `dst_ptr` points to `size` mapped host-coherent bytes; the fence
        // wait above ordered this read after the draw + copy completed; `out` is a
        // distinct, non-overlapping allocation of `size` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), size as usize);
        }

        // Teardown the transient offscreen resources (the fence ordered their last use).
        // SAFETY: each was created on `device`, its GPU work completed (fence-waited),
        // and each is destroyed exactly once here.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
            device.destroy_buffer(staging);
            device.destroy_texture(output);
        }

        out
    }

    /// CPU composite: straight alpha-over `out = bg*(1 - a) + fg.rgb*a`, where `a`
    /// is the UI foreground's ALPHA (the MSDF coverage over the transparent clear —
    /// confirmed from `ui_hud_screenshot`: the FS writes straight RGBA with coverage
    /// in alpha). `bg`/`fg` are `w*h*4` RGBA8; the result keeps `bg`'s opaque alpha.
    fn composite_over(bg: &[u8], fg: &[u8]) -> Vec<u8> {
        debug_assert_eq!(bg.len(), fg.len(), "composite inputs are the same extent");
        let mut out = vec![0u8; bg.len()];
        for (o, (b, f)) in out
            .chunks_exact_mut(4)
            .zip(bg.chunks_exact(4).zip(fg.chunks_exact(4)))
        {
            let a = f[3] as f32 / 255.0;
            for c in 0..3 {
                let blended = b[c] as f32 * (1.0 - a) + f[c] as f32 * a;
                o[c] = blended.round().clamp(0.0, 255.0) as u8;
            }
            o[3] = 255; // an opaque proof image
        }
        out
    }

    /// The owner-eval screenshot: builds the P7b world (CPU systems decide the drawn
    /// nameplates + their projected pixels), boots Vulkan, renders the SDF scene +
    /// the nameplates, composites them on the CPU, asserts zero validation messages,
    /// and writes the BMP. `#[ignore]`d — Vulkan boot can hang a headless run; the
    /// orchestrator runs it on the RTX (see the module header).
    #[test]
    #[ignore = "boots Vulkan on the GPU; owner-run on the RTX (see module header)"]
    fn p7b_world_ui_screenshot() {
        let Some(ctx) = boot_or_skip("p7b_world_ui_screenshot") else {
            return;
        };
        println!("Vulkan device (validation on): {}", ctx.device_name());
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

        // ── 1. Drive the REAL P7b systems on the CPU to decide the drawn set ──
        let p7b = build_p7b_world();
        assert_eq!(
            p7b.world.resource::<HoveredWorldEntity>().0,
            Some(p7b.s0),
            "the center cursor picks S0 (the GPU run is meaningful only if it did)"
        );
        let nameplates = drawn_nameplates(&p7b);
        let drawn_names: Vec<&str> = nameplates.iter().map(|(t, _)| *t).collect();
        println!("P7b drawn nameplates: {drawn_names:?}");
        assert!(drawn_names.contains(&PICKED_TEXT), "PICKED is drawn");
        assert!(drawn_names.contains(&VISIBLE_TEXT), "VISIBLE is drawn");
        assert!(!drawn_names.contains(&HIDDEN_TEXT), "HIDDEN is occluded (not drawn)");

        // ── 2. Bake the MSDF atlas + emit each drawn nameplate's glyph instances at
        //       its PROJECTED screen position (the real P7b output) ──
        let font = msdf_baked_font();
        assert_eq!(
            font.meta.kind,
            AtlasKind::Mtsdf,
            "the baked atlas is MTSDF (continuous distance => smooth glyphs)"
        );
        let mut instances: Vec<UiInstance> = Vec::new();
        for (text, proj) in &nameplates {
            instances.extend(msdf_instances_at(text, &font, proj.screen_x, proj.screen_y));
        }
        assert!(!instances.is_empty(), "at least the PICKED + VISIBLE nameplates emit glyphs");

        // ── 3. Render the SDF scene (bg) + the nameplates (fg), composite on CPU ──
        let view = *p7b.world.resource::<ViewUniform>();
        let pc = composite_perspective_from_view(&view, W, H);
        let edits = sphere_scene();
        let bg_packed = run_marcher(&ctx, &edits, pc, W, H, "p7b_scene");
        let mut bg = vec![0u8; (W * H * 4) as usize];
        for (px, &packed) in bg_packed.iter().enumerate() {
            let rgb = unpack_rgb(packed);
            let b = px * 4;
            bg[b] = rgb[0];
            bg[b + 1] = rgb[1];
            bg[b + 2] = rgb[2];
            bg[b + 3] = 255; // opaque background
        }

        let mut rhi = RhiContext::new(ctx);
        let fg = render_ui(&mut rhi, &instances, &font, W, H);
        debug_assert_eq!(fg.len(), (W * H * 4) as usize, "UI readback is W*H*4 bytes");

        let out = composite_over(&bg, &fg);

        assert_validation_clean(rhi.context());

        let path = screenshot_path();
        write_bmp(&path, &out, W, H).expect("write the P7b composite screenshot BMP");
        let abs = std::fs::canonicalize(&path).unwrap_or(path);
        println!("P7b world-UI screenshot written: {}", abs.display());

        rhi.destroy_all();
        drop(rhi);
    }
}
