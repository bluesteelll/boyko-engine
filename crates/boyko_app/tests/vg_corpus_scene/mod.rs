//! **VG-R0 rung R0d — the corpus scene and its committed camera paths.**
//!
//! The manifest reader, the `.glb` decode-and-place pipeline, and the frozen camera-path
//! definitions the census runs the whole corpus at.
//!
//! # Where the two halves of a camera path live, and why they are split
//!
//! The path ID is committed in `assets/vg_corpus/CORPUS.toml`; the path's DEFINITION is a test
//! constant here (plan §5.7). That split is a RECORDED EXPOSURE rather than an oversight: no digest
//! in R0 hashes these constants, so re-aiming a committed path is neither a membership change nor a
//! row-count change and no R0 gate part sees it (§9.1). What IS gated is membership and
//! cardinality — R0b(e) asserts the floor at the rung that authors the set, R0d(d) asserts set
//! equality at the rung that consumes it.
//!
//! # Placement is NORMALISED, and that is a decision with a consequence
//!
//! The corpus spans four orders of magnitude in native size — an avocado and a chess set are not
//! authored in the same units — so each asset is scaled to a unit cube and laid out on a grid. The
//! census measures triangles per COVERED pixel, and covered pixels are what the camera sees, so
//! placing assets at their native scale would have one asset fill the frame and the rest occupy a
//! pixel each: a density reading about the manifest's unit conventions rather than about the
//! content. Normalisation is what makes the reading a property of the geometry.
//!
//! ⚠️ It is also NOT neutral, and the plan's §9.1 already records the axis it sits on: R0 has no
//! representativeness floor. Normalising equalises each asset's screen share, which is a choice
//! about what the corpus represents. It is recorded here rather than claimed away.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use boyko_ecs::ecs::core::asset::AssetLoader;
use boyko_render::loaders::GlbMeshLoader;
use boyko_render::mesh::Vertex;

/// The manifest, relative to `crates/boyko_app`.
pub const CORPUS_MANIFEST: &str = "../../assets/vg_corpus/CORPUS.toml";

/// Every asset is scaled so its largest bounding-box extent is this many world units.
pub const NORMALISED_SIZE: f32 = 1.0;

/// Centre-to-centre spacing, in world units. Greater than [`NORMALISED_SIZE`], so normalised
/// assets never interpenetrate — an overlap would make coverage depend on draw order.
pub const GRID_SPACING: f32 = 1.06;

/// Columns per depth layer. Five against three rows gives a 5.30 × 3.18 arrangement, aspect 1.67,
/// close to the 16:9 the four ladder rungs above rung 0 all use.
///
/// ⚠️ A GRID rather than the row this file first used, and the reason is the ladder rather than
/// aesthetics. Rung 0 is 512² (1:1) while every other rung is 16:9, so one camera pose frames a
/// WIDE arrangement quite differently across the ladder — the same reason
/// `[k1_instrument].histogram_shift_excludes_rungs` names rung 0 as a different frustum. Measured on
/// the row layout: 4.9% of the frame covered at `orbit_mid`.
pub const GRID_COLUMNS: usize = 5;

/// Rows per depth layer.
pub const GRID_ROWS: usize = 3;

/// Depth layers behind the first.
///
/// ⚠️ **THE R0b′ RECOMPOSITION, and it repairs a MEASUREMENT defect rather than changing what is
/// measured.** R0's arrangement was one flat layer of seven assets in a void, and it produced two
/// framings covering **8.1 %** and **22.2 %** of the screen. That is not what a rendered frame looks
/// like — in a real frame you never see void through a scene, you see more scene — and
/// `docs/VG-R11-UPPER-BOUND-INSTRUMENT.md` §5.3 records the consequence: a verdict from an
/// 8 %-covered frame inherits every criticism the other direction already carries. The flat layer
/// also had **no inter-asset occlusion at all** (spacing exceeded the asset diameter), so depth
/// complexity — the thing that decides whether an occlusion-culling instrument buys anything — was
/// structurally absent from the census.
///
/// Layers stack away from the camera and are offset laterally by half a cell, so a back layer shows
/// through the gaps of the one in front. That fills the frame and creates real occlusion.
///
/// **What this deliberately does NOT change: the CONTENT.** `CORPUS.toml`'s asset list, hashes and
/// published counts are untouched, so R0b(b)'s decoded-equals-published equality still holds on the
/// same seven assets. Swapping in sparser or denser content would have chosen the K1 verdict, which
/// is the vacuous-selection defect this campaign refuses everywhere else; recomposing the same
/// content does not. Instancing is free here: the seven MESHES are registered once and placed many
/// times, so vertex and index memory are unchanged and only the instance ring grows.
pub const DEPTH_LAYERS: usize = 3;

/// Centre-to-centre spacing between depth layers, in world units.
pub const LAYER_SPACING: f32 = 1.60;

/// One committed camera path's definition.
#[derive(Clone, Copy, Debug)]
pub struct CameraPath {
    /// The id committed in `CORPUS.toml`'s `camera_paths`. R0d(d) asserts set equality between
    /// these and the ids appearing in the census rows.
    pub id: &'static str,
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov_y_degrees: f32,
}

/// The two committed paths, chosen to span framings rather than to flatter (`CORPUS.toml`'s own
/// note): a mid-distance orbit is the canonical viewing distance a density claim is about, and a
/// close approach is where screen-space density is highest. Under MIN the orbit is expected to be
/// the binding one, which is the honest direction — a favourable verdict must clear the bar on the
/// WEAKEST framing.
///
/// Both poses are set from the layout's GEOMETRY and from ONE stated methodological goal — that the
/// frame be FILLED, which `docs/VG-R11-UPPER-BOUND-INSTRUMENT.md` §5.3 names as the axis R0 left
/// open. Neither is set from `D_est`, and the distinction is the whole of this rung's honesty: the
/// covered FRACTION is a property of the framing and is legitimate to aim at, while the density is
/// the measurement and is not.
///
/// The arrangement's front layer is 5.30 × 3.18 world units. A 50° vertical field spans the 3.18
/// height at `d = 1.59 / tan(25°) = 3.41`, so `orbit_mid` sits just inside that, off-axis, looking
/// into the middle depth layer — the whole point being that the back layers close the gaps the front
/// one leaves. `approach_close` sits at roughly a third of that distance, where the near assets
/// overflow the frame entirely and every pixel is geometry.
pub const PATHS: [CameraPath; 2] = [
    CameraPath {
        id: "orbit_mid",
        eye: [0.85, 0.60, 3.25],
        target: [0.0, 0.0, -1.60],
        fov_y_degrees: 50.0,
    },
    CameraPath {
        id: "approach_close",
        eye: [0.30, 0.22, 1.15],
        target: [0.0, 0.0, -1.60],
        fov_y_degrees: 50.0,
    },
];

/// Looks up a committed path by id.
pub fn path_by_id(id: &str) -> CameraPath {
    *PATHS
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("`{id}` is not a committed camera path"))
}

/// One manifest row, reduced to what the census needs.
#[derive(Clone, Debug)]
pub struct CorpusAsset {
    pub id: String,
    pub glb: PathBuf,
    pub published_triangles: u64,
}

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CORPUS_MANIFEST)
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').to_string()
}

/// The manifest's `camera_paths` enumeration — the domain R0d(d)'s set equality is against.
pub fn committed_camera_paths() -> Vec<String> {
    let src = std::fs::read_to_string(manifest_path()).expect("the corpus manifest is tracked");
    for line in src.lines() {
        let l = strip_comment(line).trim();
        if let Some((k, v)) = l.split_once('=')
            && k.trim() == "camera_paths"
        {
            return v
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(unquote)
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    panic!("the corpus manifest names no `camera_paths` -- the MIN's domain would be undefined");
}

/// The manifest's `[[asset]]` rows, in file order.
pub fn manifest_assets() -> Vec<CorpusAsset> {
    let src = std::fs::read_to_string(manifest_path()).expect("the corpus manifest is tracked");
    let dir = manifest_path().parent().expect("the manifest has a directory").to_path_buf();
    let mut out = Vec::new();
    let mut cur: Option<(Option<String>, Option<String>, Option<u64>)> = None;
    let flush = |cur: &mut Option<(Option<String>, Option<String>, Option<u64>)>,
                 out: &mut Vec<CorpusAsset>,
                 dir: &Path| {
        if let Some((id, glb, tris)) = cur.take() {
            out.push(CorpusAsset {
                id: id.expect("an [[asset]] row carries an id"),
                glb: dir.join(glb.expect("an [[asset]] row carries a glb path")),
                published_triangles: tris.expect("an [[asset]] row carries published_triangles"),
            });
        }
    };
    for line in src.lines() {
        let l = strip_comment(line).trim();
        if l == "[[asset]]" {
            flush(&mut cur, &mut out, &dir);
            cur = Some((None, None, None));
            continue;
        }
        if let Some(slot) = cur.as_mut()
            && let Some((k, v)) = l.split_once('=')
        {
            match k.trim() {
                "id" => slot.0 = Some(unquote(v)),
                "glb" => slot.1 = Some(unquote(v)),
                "published_triangles" => {
                    slot.2 = Some(v.trim().parse().expect("published_triangles is an integer"));
                }
                _ => {}
            }
        }
    }
    flush(&mut cur, &mut out, &dir);
    out
}

/// Whether the gitignored payload is present. The manifest is tracked and the `.glb` files are not,
/// so a clone without a `scripts/fetch_corpus.ps1` run has the former and none of the latter.
pub fn payload_present() -> bool {
    let assets = manifest_assets();
    !assets.is_empty() && assets.iter().all(|a| a.glb.is_file())
}

/// One decoded asset and the normalisation that makes it comparable to its siblings.
///
/// A slot is NOT a member: the arrangement places each asset in several slots, and the mesh is
/// registered with the device exactly once per asset. That split is what keeps the recomposition
/// free — vertex and index memory are a function of the seven assets, never of the slot count.
pub struct PlacedAsset {
    pub id: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Uniform scale that makes the largest bounding-box extent [`NORMALISED_SIZE`].
    pub scale: f32,
    /// Model-space bounds centre; the caller folds `-scale * centre` into the instance translation.
    pub centre: [f32; 3],
}

impl PlacedAsset {
    pub fn triangles(&self) -> u64 {
        self.indices.len() as u64 / 3
    }
}

/// The arrangement: for each slot, which decoded asset fills it.
///
/// Assets cycle through the slots in manifest order, so the mix is fixed by the manifest rather
/// than chosen per slot — there is no per-slot lever with which to place the dense assets where a
/// camera would flatter them.
pub fn slot_asset(slot: usize, asset_count: usize) -> usize {
    slot % asset_count.max(1)
}

/// Triangles the arrangement SUBMITS per frame — the sum over slots of the asset filling it.
pub fn submitted_triangles(assets: &[PlacedAsset]) -> u64 {
    (0..SLOT_COUNT).map(|s| assets[slot_asset(s, assets.len())].triangles()).sum()
}

/// Slots in the whole arrangement: `GRID_COLUMNS × GRID_ROWS × DEPTH_LAYERS`.
pub const SLOT_COUNT: usize = GRID_COLUMNS * GRID_ROWS * DEPTH_LAYERS;

/// The world position of arrangement slot `i`, centred laterally on the origin with layer 0 at
/// `z = 0` and later layers receding.
///
/// Odd layers are offset laterally by half a cell in both axes, so a back layer shows through the
/// gaps of the one in front instead of hiding exactly behind it.
pub fn slot_position(i: usize) -> [f32; 3] {
    let per_layer = GRID_COLUMNS * GRID_ROWS;
    let layer = i / per_layer;
    let within = i % per_layer;
    let (col, row) = (within % GRID_COLUMNS, within / GRID_COLUMNS);
    let stagger = if layer % 2 == 1 { 0.5 } else { 0.0 };
    [
        (col as f32 - (GRID_COLUMNS as f32 - 1.0) * 0.5 + stagger) * GRID_SPACING,
        ((GRID_ROWS as f32 - 1.0) * 0.5 - row as f32 + stagger) * GRID_SPACING,
        -(layer as f32) * LAYER_SPACING,
    ]
}

/// Decodes every manifest asset and computes its normalisation.
///
/// # Panics
///
/// Panics if the payload is absent — every caller checks [`payload_present`] first and SKIPS by
/// name, because a payload-dependent gate part that does not name itself as skipped is
/// indistinguishable from one that passed.
pub fn decode_corpus() -> Vec<PlacedAsset> {
    manifest_assets()
        .iter()
        .map(|a| {
            let bytes = std::fs::read(&a.glb)
                .unwrap_or_else(|e| panic!("corpus asset `{}` unreadable: {e}", a.id));
            let mesh = GlbMeshLoader::decode(&bytes)
                .unwrap_or_else(|e| panic!("corpus asset `{}` failed to decode: {e:?}", a.id));
            let (lo, hi) = bounds(&mesh.vertices);
            let extent = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
            let largest = extent[0].max(extent[1]).max(extent[2]).max(f32::MIN_POSITIVE);
            PlacedAsset {
                id: a.id.clone(),
                vertices: mesh.vertices,
                indices: mesh.indices,
                scale: NORMALISED_SIZE / largest,
                centre: [
                    (lo[0] + hi[0]) * 0.5,
                    (lo[1] + hi[1]) * 0.5,
                    (lo[2] + hi[2]) * 0.5,
                ],
            }
        })
        .collect()
}

/// The axis-aligned bounds of a vertex list.
pub fn bounds(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v.position[k]);
            hi[k] = hi[k].max(v.position[k]);
        }
    }
    (lo, hi)
}
