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
pub const GRID_SPACING: f32 = 1.30;

/// Columns in the layout grid.
///
/// ⚠️ A GRID rather than the row this file first used, and the reason is the ladder rather than
/// aesthetics. Rung 0 is 512² (1:1) while every other rung is 16:9, so one camera pose frames a
/// WIDE arrangement quite differently across the ladder — the same reason
/// `[k1_instrument].histogram_shift_excludes_rungs` names rung 0 as a different frustum. A compact
/// square-ish arrangement is what makes one committed pose mean the same framing at every rung.
/// Measured on the row layout: 4.9% of the frame covered at `orbit_mid`, which clears the
/// non-degeneracy floors and is still a thin reading.
pub const GRID_COLUMNS: usize = 3;

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
/// Both poses are set from the layout's GEOMETRY — the grid's half-extent against the vertical
/// field of view — and not from any statistic they produce. `orbit_mid` sits at 5.34 world units,
/// where a 50° vertical field spans 2.49 units against the grid's 1.95 half-extent, so the whole
/// corpus is in frame with margin; `approach_close` sits at 1.84, spanning 0.86, so it sees the
/// central assets at the highest screen density the scene offers.
pub const PATHS: [CameraPath; 2] = [
    CameraPath {
        id: "orbit_mid",
        eye: [2.20, 1.60, 4.60],
        target: [0.0, 0.0, 0.0],
        fov_y_degrees: 50.0,
    },
    CameraPath {
        id: "approach_close",
        eye: [0.45, 0.35, 1.75],
        target: [0.0, 0.0, 0.0],
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

/// One decoded asset, plus the placement that normalises it onto the row.
pub struct PlacedAsset {
    pub id: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Uniform scale that makes the largest bounding-box extent [`NORMALISED_SIZE`].
    pub scale: f32,
    /// Model-space translation applied BEFORE the scale, centring the asset on its own bounds.
    pub centre: [f32; 3],
    /// The grid slot's world position.
    pub slot: [f32; 3],
}

impl PlacedAsset {
    pub fn triangles(&self) -> u64 {
        self.indices.len() as u64 / 3
    }
}

/// The world position of grid slot `i` of `n`, centred on the origin.
pub fn grid_slot(i: usize, n: usize) -> [f32; 3] {
    let cols = GRID_COLUMNS.min(n.max(1));
    let rows = n.div_ceil(cols);
    let (col, row) = (i % cols, i / cols);
    [
        (col as f32 - (cols as f32 - 1.0) * 0.5) * GRID_SPACING,
        ((rows as f32 - 1.0) * 0.5 - row as f32) * GRID_SPACING,
        0.0,
    ]
}

/// Decodes every manifest asset and computes its normalised grid placement.
///
/// # Panics
///
/// Panics if the payload is absent — every caller checks [`payload_present`] first and SKIPS by
/// name, because a payload-dependent gate part that does not name itself as skipped is
/// indistinguishable from one that passed.
pub fn decode_corpus() -> Vec<PlacedAsset> {
    let assets = manifest_assets();
    let n = assets.len();
    assets
        .iter()
        .enumerate()
        .map(|(i, a)| {
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
                slot: grid_slot(i, n),
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
