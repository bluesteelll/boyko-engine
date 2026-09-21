//! SDFDDGI D-B — the octahedral TILE-BORDER continuity HOST ORACLE (CPU-only — NO GPU required).
//!
//! # What this file decides
//!
//! A receiver normal that octahedral-encodes onto a tile EDGE (`±x`, `±y` encode to the tile-UV
//! extremes `e.x == 1.0` / `e.y == 1.0` / `0.0`) is read back by `ddgi_resolve.hlsli`'s
//! `ddgi_irr_uv` at an atlas texel coordinate that lands EXACTLY on the boundary between the last
//! valid-interior texel and the 1-texel BORDER ring, so a LINEAR `SampleLevel` draws **half** of
//! its value from a border texel. The border ring is not shaded: it is filled by a copy from the
//! interior (`sdf_probe_update.comp.hlsl`'s `border_copy_index`). Whether the GI term at those
//! normals is right therefore depends entirely on that copy naming the octahedrally-CONTINUOUS
//! interior texel.
//!
//! Two candidate causes were put to this file, and it separates them:
//!
//! - **Refuted here** — that `oct_encode` and `oct_decode` disagree at the tile edges. They do
//!   not: [`oct_encode_and_oct_decode_are_mutual_inverses_including_the_tile_edges`] round-trips
//!   both ways, edges and corners included, and passes.
//! - **Confirmed here** — that `border_copy_index` names the wrong interior texel for all four
//!   EDGE runs (its 4 CORNER cases are correct). See
//!   [`border_copy_source_is_the_octahedral_continuation_of_the_border_texel`].
//!
//! # The oracle: the octahedral square's edge identification
//!
//! In `[-1,1]²` signed tile space the octahedral map folds each edge ONTO ITSELF, reversed. The
//! right edge `(1, s)` decodes to `normalize(1 - |s|, 0, -|s|)`, which is independent of `sign(s)`
//! — so `(1, s)` and `(1, -s)` ARE the same direction, and a path leaving the square through the
//! right edge re-enters through the RIGHT edge at the mirrored row. The continuation rule is thus
//! a REFLECTION about the crossed edge with the tangential coordinate negated
//! ([`reflect_into_square`]):
//!
//! ```text
//! sx >  1  =>  sx = 2 - sx,  sy = -sy        sy >  1  =>  sy = 2 - sy,  sx = -sx
//! sx < -1  =>  sx = -2 - sx, sy = -sy        sy < -1  =>  sy = -2 - sy, sx = -sx
//! ```
//!
//! Applied to a CORNER texel (both coordinates outside) the two reflections compose, in either
//! order, to the point reflection `(sx, sy) -> (-sx, -sy)` — the DIAGONALLY-OPPOSITE interior
//! corner. That is why `border_copy_index`'s corner arms are right while its edge arms are not:
//! the "copy the opposite side" rule is correct for a corner and wrong for an edge.
//!
//! Transcendental-free throughout (`abs`/`clamp`/`select`/±/`*`/`/` + the decode's `normalize`),
//! and it boots no Vulkan context.
//!
//! # Why there is a fourth arm
//!
//! [`border_copy_index_as_committed`] is a TRANSCRIPTION of HLSL. A transcription that drifts from
//! the shader turns this whole file into a gate on itself — the "test header claims the shipped
//! path" failure — so
//! [`the_host_transcription_matches_the_committed_border_copy_index`] `include_str!`s the shader
//! and pins the eight `src = uint2(...)` expressions it spells, in source order, against
//! [`SRC_EXPRS_IN_SOURCE_ORDER`], which sits one line from each of the transcription's eight
//! returns. Its residual is stated rather than hidden: it catches an edit to ONE side, not a
//! matching wrong edit to both.

use boyko_rhi_vulkan::goldens::{
    ddgi_oct_encode, oct_decode, DDGI_IRR_TILE_EDGE, DDGI_IRR_VALID_EXTENT, DDGI_TILE_BORDER,
};

/// The depth tile's edge in texels (`sdf_probe_update.comp.hlsl:112`). Not re-exported by
/// `goldens` (the host oracle only parameterizes the irradiance tile), so it is mirrored here.
const DDGI_DEPTH_TILE_EDGE: u32 = 16;

/// The depth tile's valid interior extent (`sdf_probe_update.comp.hlsl:114`).
const DDGI_DEPTH_VALID_EXTENT: u32 = 14;

// ---- the shader's border map, transcribed VERBATIM (the subject under test) ------------------

/// A host transcription of `sdf_probe_update.comp.hlsl:246` `border_copy_index`, statement for
/// statement, as committed. Maps a border-ring index `bt` in `[0, 4*(tile_edge-1))` to its LOCAL
/// destination texel `dst` in `[0, tile_edge)²` and the LOCAL INTERIOR texel `src` in
/// `[0, valid_extent)²` whose value is copied into it.
///
/// This is the code under test, NOT a reference: it is transcribed so the disagreement can be
/// exhibited on the host, without a device.
fn border_copy_index_as_committed(bt: u32, tile_edge: u32, valid_extent: u32) -> ((u32, u32), (u32, u32)) {
    let v = valid_extent;
    let mut bt = bt;
    if bt < tile_edge {
        // Top row (local y = 0); bt is the local x (column).
        let bx = bt;
        let dst = (bx, 0);
        if bx == 0 {
            return (dst, (v - 1, v - 1)); // SRC_EXPRS[0]  uint2(v - 1u, v - 1u)
        }
        if bx == tile_edge - 1 {
            return (dst, (0, v - 1)); // SRC_EXPRS[1]  uint2(0u, v - 1u)
        }
        let cx = bx - DDGI_TILE_BORDER;
        return (dst, (v - 1 - cx, 0)); // SRC_EXPRS[2]  uint2(v - 1u - cx, 0u)
    }
    bt -= tile_edge;
    if bt < tile_edge {
        // Bottom row (local y = tile_edge - 1); bt is the local x (column).
        let bx = bt;
        let dst = (bx, tile_edge - 1);
        if bx == 0 {
            return (dst, (v - 1, 0)); // SRC_EXPRS[3]  uint2(v - 1u, 0u)
        }
        if bx == tile_edge - 1 {
            return (dst, (0, 0)); // SRC_EXPRS[4]  uint2(0u, 0u)
        }
        let cx = bx - DDGI_TILE_BORDER;
        return (dst, (v - 1 - cx, v - 1)); // SRC_EXPRS[5]  uint2(v - 1u - cx, v - 1u)
    }
    bt -= tile_edge;
    if bt < v {
        // Left col (local x = 0), corners excluded.
        let dst = (0, bt + DDGI_TILE_BORDER);
        return (dst, (0, v - 1 - bt)); // SRC_EXPRS[6]  uint2(0u, v - 1u - bt)
    }
    bt -= v;
    // Right col (local x = tile_edge - 1), corners excluded.
    let dst = (tile_edge - 1, bt + DDGI_TILE_BORDER);
    (dst, (v - 1, v - 1 - bt)) // SRC_EXPRS[7]  uint2(v - 1u, v - 1u - bt)
}

/// The eight `src = uint2(...)` expressions the committed `border_copy_index` must spell, IN
/// SOURCE ORDER — the text form of [`border_copy_index_as_committed`]'s eight returns, each tagged
/// `SRC_EXPRS[i]` at the return it transcribes.
///
/// Pinned against the shader itself by
/// [`the_host_transcription_matches_the_committed_border_copy_index`].
const SRC_EXPRS_IN_SOURCE_ORDER: [&str; 8] = [
    "uint2(v - 1u, v - 1u)",       // [0] top row, bx == 0            -> bottom-right interior corner
    "uint2(0u, v - 1u)",           // [1] top row, bx == tile_edge-1  -> bottom-left interior corner
    "uint2(v - 1u - cx, 0u)",      // [2] top row, interior column    -> TOP interior row, reversed
    "uint2(v - 1u, 0u)",           // [3] bottom row, bx == 0         -> top-right interior corner
    "uint2(0u, 0u)",               // [4] bottom row, bx == edge-1    -> top-left interior corner
    "uint2(v - 1u - cx, v - 1u)",  // [5] bottom row, interior column -> BOTTOM interior row, reversed
    "uint2(0u, v - 1u - bt)",      // [6] left column                 -> LEFT interior col, reversed
    "uint2(v - 1u, v - 1u - bt)",  // [7] right column                -> RIGHT interior col, reversed
];

// ---- the reference: what direction each texel position actually stands for -------------------

/// The SIGNED `[-1,1]²` octahedral coordinate a LOCAL tile texel `l` stands for, under the read
/// chain the resolve authors.
///
/// `ddgi_resolve.hlsli`'s `ddgi_irr_uv` computes `px = ox + BORDER + e.x * VALID_EXTENT` and
/// samples at `px / atlas_dim`, i.e. at continuous atlas-texel coordinate `px`. Local texel `l`
/// has its centre at continuous coordinate `l + 0.5`, so the tile-UV it stands for is
/// `e = (l + 0.5 - BORDER) / VALID_EXTENT`, and the signed coordinate is `2e - 1`. For an
/// interior texel this reproduces `goldens::ddgi_texel_direction`'s `(t + 0.5)/extent`; for a
/// border texel it lands just OUTSIDE `[-1,1]`, which is exactly the continuation the copy must
/// supply.
fn signed_coord_of_local_texel(l: u32, valid_extent: u32) -> f32 {
    let e = (l as f32 + 0.5 - DDGI_TILE_BORDER as f32) / valid_extent as f32;
    e * 2.0 - 1.0
}

/// Folds a signed octahedral coordinate pair back into `[-1,1]²` by the octahedral edge
/// identification (see the module header). A corner (both coordinates outside) composes the two
/// reflections into the point reflection through the origin.
fn reflect_into_square(sx: f32, sy: f32) -> (f32, f32) {
    let (mut sx, mut sy) = (sx, sy);
    if sx > 1.0 {
        sx = 2.0 - sx;
        sy = -sy;
    } else if sx < -1.0 {
        sx = -2.0 - sx;
        sy = -sy;
    }
    if sy > 1.0 {
        sy = 2.0 - sy;
        sx = -sx;
    } else if sy < -1.0 {
        sy = -2.0 - sy;
        sx = -sx;
    }
    (sx, sy)
}

/// The unit direction a signed `[-1,1]²` pair decodes to, through the tree's own
/// `goldens::oct_decode` (which takes the `[0,1]²` pair, hence the `*0.5 + 0.5` remap).
fn dir_of_signed(sx: f32, sy: f32) -> [f32; 3] {
    oct_decode([sx * 0.5 + 0.5, sy * 0.5 + 0.5])
}

/// The unit direction an INTERIOR texel `(tx, ty)` holds after the update pass shades it.
fn dir_of_interior_texel(tx: u32, ty: u32, valid_extent: u32) -> [f32; 3] {
    let sx = signed_coord_of_local_texel(tx + DDGI_TILE_BORDER, valid_extent);
    let sy = signed_coord_of_local_texel(ty + DDGI_TILE_BORDER, valid_extent);
    dir_of_signed(sx, sy)
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

// ---- arm 1: the encode/decode pair itself (this arm PASSES — it refutes a candidate) ---------

/// `oct_encode` and `oct_decode` are mutual inverses, INCLUDING at the tile edges and corners.
///
/// Sweeps every valid-interior texel centre of both the irradiance (6×6) and depth (14×14) tiles
/// through `decode -> encode`, and additionally the four edge-extreme directions `±x`/`±y` (which
/// encode to the tile-UV extremes `0.0`/`1.0`, the very coordinates the border tap straddles)
/// through `encode -> decode`.
///
/// This arm exists to REFUTE the hypothesis that the edge artefact comes from the encode/decode
/// pair disagreeing. It does not: the pair is the standard Cigolle octahedral map and round-trips
/// to within the ≤2-ULP gap `goldens::ddgi_oct_encode`'s own doc comment already documents (it
/// L1-normalizes by multiply-by-reciprocal where the eDSL body divides).
#[test]
fn oct_encode_and_oct_decode_are_mutual_inverses_including_the_tile_edges() {
    // Tolerance covers the documented ≤2-ULP encode gap plus the decode's `normalize`.
    const TOL: f32 = 1.0e-5;

    for (extent, label) in [
        (DDGI_IRR_VALID_EXTENT, "irr"),
        (DDGI_DEPTH_VALID_EXTENT, "depth"),
    ] {
        for ty in 0..extent {
            for tx in 0..extent {
                let dir = dir_of_interior_texel(tx, ty, extent);
                let e = ddgi_oct_encode(dir);
                let back = oct_decode(e);
                let cos = dot3(dir, back);
                assert!(
                    cos > 1.0 - TOL,
                    "{label} interior texel ({tx},{ty}): decode->encode->decode is not the \
                     identity (cos = {cos}); dir = {dir:?}, back = {back:?}"
                );
            }
        }
    }

    // The four EDGE-EXTREME directions: these are the normals whose atlas tap is 50 % border.
    for (dir, name) in [
        ([1.0_f32, 0.0, 0.0], "+x"),
        ([-1.0, 0.0, 0.0], "-x"),
        ([0.0, 1.0, 0.0], "+y"),
        ([0.0, -1.0, 0.0], "-y"),
    ] {
        let e = ddgi_oct_encode(dir);
        let back = oct_decode(e);
        let cos = dot3(dir, back);
        assert!(
            cos > 1.0 - TOL,
            "edge-extreme {name}: encode {e:?} does not decode back (cos = {cos}); back = {back:?}"
        );
    }
}

// ---- arm 2: the border copy (this arm is RED — the defect) ------------------------------------

/// Every BORDER texel must receive the value of the interior texel that is its octahedral
/// CONTINUATION — the reflection of its own position back into the square.
///
/// The border ring is what a LINEAR tap reads half of whenever a receiver normal encodes to a
/// tile-UV extreme, so a border texel holding a direction from elsewhere on the sphere is a
/// silent 50 %-wrong irradiance fetch at those normals — `+y` (every floor and every upward-facing
/// face) and `±x` among them.
///
/// The check is direction-space, not index-space: for each border index it decodes the direction
/// the destination texel POSITION stands for (via [`reflect_into_square`]) and the direction the
/// named SOURCE texel HOLDS, and requires them equal.
#[test]
fn border_copy_source_is_the_octahedral_continuation_of_the_border_texel() {
    const TOL: f32 = 1.0e-5;

    // Both tiles are counted BEFORE anything is asserted: a per-tile assert would stop at the
    // irradiance atlas and never report the depth atlas at all, and the depth border is what feeds
    // Chebyshev — the leak term. A gate that hides half the damage under-reports the defect.
    let mut report: Vec<String> = Vec::new();

    for (tile_edge, extent, label) in [
        (DDGI_IRR_TILE_EDGE, DDGI_IRR_VALID_EXTENT, "irr"),
        (DDGI_DEPTH_TILE_EDGE, DDGI_DEPTH_VALID_EXTENT, "depth"),
    ] {
        let border_count = 4 * (tile_edge - 1);
        let mut wrong = 0_u32;
        let mut first: Option<String> = None;

        for bt in 0..border_count {
            let (dst, src) = border_copy_index_as_committed(bt, tile_edge, extent);

            // What the destination POSITION stands for, continued back into the square.
            let (rx, ry) = reflect_into_square(
                signed_coord_of_local_texel(dst.0, extent),
                signed_coord_of_local_texel(dst.1, extent),
            );
            let want = dir_of_signed(rx, ry);

            // What the named SOURCE texel actually holds.
            let got = dir_of_interior_texel(src.0, src.1, extent);

            let cos = dot3(want, got);
            if cos <= 1.0 - TOL {
                wrong += 1;
                if first.is_none() {
                    first = Some(format!(
                        "{label} bt={bt}: dst local {dst:?} continues to interior direction \
                         {want:?}, but the copy names src interior {src:?} which holds {got:?} \
                         (cos = {cos})"
                    ));
                }
            }
        }

        if wrong != 0 {
            report.push(format!(
                "{label} tile: {wrong} of {border_count} border texels are copied from a texel \
                 that is NOT their octahedral continuation. First: {}",
                first.unwrap_or_default()
            ));
        }
    }

    assert!(
        report.is_empty(),
        "the border copy is not the octahedral continuation:\n{}",
        report.join("\n")
    );
}

// ---- arm 3: the consequence at the shipped read chain (also RED) ------------------------------

/// The direction every LOCAL texel of a tile HOLDS once the update pass has shaded the interior
/// and run the committed border copy over it: interior texels hold their own decoded direction,
/// border texels hold whatever `border_copy_index` named.
fn held_direction_table(tile_edge: u32, extent: u32) -> Vec<[f32; 3]> {
    let n = (tile_edge * tile_edge) as usize;
    let mut table = vec![[0.0_f32; 3]; n];
    for ty in 0..extent {
        for tx in 0..extent {
            let lx = tx + DDGI_TILE_BORDER;
            let ly = ty + DDGI_TILE_BORDER;
            table[(ly * tile_edge + lx) as usize] = dir_of_interior_texel(tx, ty, extent);
        }
    }
    for bt in 0..4 * (tile_edge - 1) {
        let (dst, src) = border_copy_index_as_committed(bt, tile_edge, extent);
        table[(dst.1 * tile_edge + dst.0) as usize] = dir_of_interior_texel(src.0, src.1, extent);
    }
    table
}

/// The two texels a LINEAR tap straddles on one axis, and the weight of the second, for a sample
/// at continuous tile-local texel coordinate `c` (pixel-centre convention: texel `i` is centred at
/// `i + 0.5`).
fn linear_pair(c: f32) -> (u32, u32, f32) {
    let shifted = c - 0.5;
    let i0 = shifted.floor();
    (i0 as u32, i0 as u32 + 1, shifted - i0)
}

/// A receiver normal must never draw irradiance from a texel facing AWAY from it.
///
/// The four edge-extreme normals `±x`/`±y` encode to a tile-UV extreme, which `ddgi_irr_uv` turns
/// into an atlas coordinate sitting exactly on the last-interior/border boundary — so half of the
/// LINEAR tap is a border texel. With the octahedral continuation in the border that half faces
/// essentially the same way as the normal (one texel step away). With the committed copy it faces
/// the OPPOSITE hemisphere, so half the fetched irradiance belongs to the far side of the probe.
///
/// `+y` is the normal of every floor and every upward-facing face in the scene, so this is not an
/// exotic corner of the domain — it is the most common receiver normal there is.
#[test]
fn an_edge_normal_never_taps_a_texel_facing_away_from_it() {
    for (tile_edge, extent, label) in [
        (DDGI_IRR_TILE_EDGE, DDGI_IRR_VALID_EXTENT, "irr"),
        (DDGI_DEPTH_TILE_EDGE, DDGI_DEPTH_VALID_EXTENT, "depth"),
    ] {
        let table = held_direction_table(tile_edge, extent);

        for (n, name) in [
            ([1.0_f32, 0.0, 0.0], "+x"),
            ([-1.0, 0.0, 0.0], "-x"),
            ([0.0, 1.0, 0.0], "+y"),
            ([0.0, -1.0, 0.0], "-y"),
        ] {
            // The shipped read chain: `px = BORDER + e.x * VALID_EXTENT` in tile-local texels.
            let e = ddgi_oct_encode(n);
            let (x0, x1, fx) = linear_pair(DDGI_TILE_BORDER as f32 + e[0] * extent as f32);
            let (y0, y1, fy) = linear_pair(DDGI_TILE_BORDER as f32 + e[1] * extent as f32);

            for (lx, wx) in [(x0, 1.0 - fx), (x1, fx)] {
                for (ly, wy) in [(y0, 1.0 - fy), (y1, fy)] {
                    let w = wx * wy;
                    if w <= 0.0 {
                        continue;
                    }
                    let held = table[(ly * tile_edge + lx) as usize];
                    let cos = dot3(n, held);
                    assert!(
                        cos > 0.0,
                        "{label} normal {name} (tile-UV {e:?}): the linear tap gives weight {w} to \
                         local texel ({lx},{ly}), which holds direction {held:?} — facing AWAY from \
                         the receiver (cos = {cos})"
                    );
                }
            }
        }
    }
}

// ---- arm 4: the transcription is the SHIPPED source, not a copy of it -------------------------

/// The committed probe-update shader, compiled into this test so a shader edit rebuilds it.
const PROBE_UPDATE_HLSL: &str = include_str!("../shaders/sdf_probe_update.comp.hlsl");

/// Extracts the `void border_copy_index(...) { ... }` body from `hlsl` by brace-matching from the
/// signature (the `extract_soft_shadow_ranged` idiom of `ddgi_probe_gi_sync.rs`). Panics if the
/// function is absent — a malformed shader, surfaced loudly rather than silently skipped.
fn extract_border_copy_index(hlsl: &str) -> String {
    let hlsl = hlsl.replace("\r\n", "\n");
    let sig = "void border_copy_index(uint bt, uint tile_edge, uint valid_extent, out uint2 dst, out uint2 src) {";
    let start = hlsl
        .find(sig)
        .expect("invariant: sdf_probe_update.comp.hlsl must contain the `border_copy_index` signature");
    let bytes = hlsl.as_bytes();
    let (mut depth, mut i, mut end) = (0i32, start, None);
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    hlsl[start..end.expect("invariant: `border_copy_index` has an unmatched brace")].to_string()
}

/// Every `src = <expr>;` right-hand side in `body`, in source order, whitespace-collapsed.
fn src_expressions(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("src = ") {
        let after = &rest[at + "src = ".len()..];
        let semi = after
            .find(';')
            .expect("invariant: every `src = ` assignment in `border_copy_index` ends in `;`");
        out.push(after[..semi].split_whitespace().collect::<Vec<_>>().join(" "));
        rest = &after[semi..];
    }
    out
}

/// The host transcription under test must be the SHIPPED map, not a copy that has drifted from it.
///
/// [`border_copy_index_as_committed`] is HLSL rewritten in Rust, so on its own it gates itself:
/// a shader edit that never reaches the transcription leaves arms 2 and 3 green while the GPU runs
/// something else — this repository's catalogued "the test header claims the shipped path" shape.
/// This arm closes the loop the only way a CPU-only test can: it reads the committed shader and
/// pins the eight `src = uint2(...)` expressions it spells, in source order, against
/// [`SRC_EXPRS_IN_SOURCE_ORDER`], which is written one line from each transcribed return.
///
/// **The residual is stated, not hidden.** This catches an edit to ONE side. It cannot catch a
/// matching WRONG edit to both, and it says nothing about the `dst` arms or the control flow — the
/// direction-space arms above are what judge the map's meaning. What it guarantees is that those
/// arms are judging the text the device compiles.
#[test]
fn the_host_transcription_matches_the_committed_border_copy_index() {
    let body = extract_border_copy_index(PROBE_UPDATE_HLSL);
    let found = src_expressions(&body);
    let found: Vec<&str> = found.iter().map(String::as_str).collect();
    assert_eq!(
        found,
        SRC_EXPRS_IN_SOURCE_ORDER.to_vec(),
        "the committed `border_copy_index` no longer spells the source expressions the host \
         transcription in this file reproduces — the transcription (and therefore every other arm \
         here) has drifted from the shipped shader.\ncommitted body:\n{body}"
    );
}
