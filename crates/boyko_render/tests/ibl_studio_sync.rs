//! IBL1 R0 (`ibl1/cut.md` C8; ruling 123 (3)) — **the committed studio environment is the bake's
//! output, byte for byte, and the bake measures what the design says**.
//!
//! `crates/boyko_render/assets/ibl/studio_256.envoct` is a derived, sync-gated artifact (the `.spv`
//! / SMAA-LUT class): `boyko_shaderdsl::ibl::bake_studio_into` regenerates it here, and the bake is
//! IEEE-only (no platform libm — `ibl_bake_is_libm_free`), so the bytes are the same on every host.
//!
//! - `ibl_studio_sync`: the re-bake equals the committed blob (and `STUDIO_ENV_BLOB`), byte for
//!   byte. A one-value edit of the studio table (a panel's radiance 8 → 8.01) reds it.
//! - `ibl_studio_numbers`: (a) the source map's texel solid angles sum to 4π ± 1e-3 (the exact
//!   octahedral Jacobian); (b) the luminance of `E(+y)` is 1 ± 1e-3 (the normalization rule:
//!   intensity 1 ⇔ unit irradiance on an up-facing surface); (d) G4 — every border texel of every
//!   committed level equals its octahedral-continuation source texel, bit for bit; (e) G3 — the
//!   header's level count is the chain the read side's `env_lod` spans (pR 1 reads the last level);
//!   (f) reported, never gated: `E` on the six axes, `k`, and the hemisphere diffuse model's error
//!   at the four horizontal normals (SH-9 is decided on it at the HDRI rung). The furnace (c) and
//!   the per-level roughness gate (W3) are `boyko_shaderdsl::ibl`'s unit tests (they bake small
//!   synthetic sources through the same path).
//!
//! Both share ONE bake (a `OnceLock`): the full 256 × 7 chain is the costly part of the run, and the
//! budget is ≤ 60 s in the debug test profile (`ibl1/cut.md` D-P2a; the tester measures it).

use std::sync::OnceLock;

use boyko_render::environment::{
    ENV_BLOB_HEADER_BYTES, ENV_FORMAT_B10G11R11, STUDIO_ENV_BLOB, STUDIO_ENV_BLOB_BYTES, parse_env_blob,
};
use boyko_shaderdsl::ibl::{
    EnvBakeStats, IRRADIANCE_NORMALS, STUDIO_BAKE, STUDIO_BLOB_BYTES, STUDIO_LEVELS, STUDIO_SOURCE_TILE, STUDIO_TILE,
    bake_studio_into, env_chain_words, env_lod_body, env_source_scratch_len, oct_env_border_source, write_env_blob,
};

/// The committed file, read from disk (the `include_bytes!` in `environment.rs` is the same file).
fn committed() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/ibl/studio_256.envoct");
    std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The one bake this binary runs: the blob and the bake's stats.
fn baked() -> &'static (Vec<u8>, EnvBakeStats) {
    static BAKE: OnceLock<(Vec<u8>, EnvBakeStats)> = OnceLock::new();
    BAKE.get_or_init(|| {
        let start = std::time::Instant::now();
        let mut chain = vec![0u32; env_chain_words(STUDIO_TILE, STUDIO_LEVELS)];
        let mut scratch = vec![0f32; env_source_scratch_len(STUDIO_SOURCE_TILE)];
        let stats = bake_studio_into(&mut chain, &mut scratch);
        let mut blob = vec![0u8; STUDIO_BLOB_BYTES];
        write_env_blob(&STUDIO_BAKE, &stats, &chain, &mut blob);
        println!("bake: {:.2} s", start.elapsed().as_secs_f64());
        (blob, stats)
    })
}

#[test]
fn ibl_studio_sync() {
    assert_eq!(STUDIO_ENV_BLOB_BYTES, STUDIO_BLOB_BYTES, "the two crates agree on the blob size");
    assert_eq!(ENV_BLOB_HEADER_BYTES, boyko_shaderdsl::ibl::ENV_HEADER_BYTES);
    assert_eq!(ENV_FORMAT_B10G11R11, boyko_shaderdsl::ibl::ENV_FORMAT_B10G11R11);
    let (blob, _) = baked();
    let file = committed();
    assert_eq!(file.as_slice(), STUDIO_ENV_BLOB.as_slice(), "the include_bytes! blob is the committed file");
    if file != *blob {
        let first = file.iter().zip(blob.iter()).position(|(a, b)| a != b);
        panic!(
            "studio_256.envoct is not the re-bake ({} vs {} bytes; first difference at byte {first:?}) — re-run \
             `cargo run --release -p boyko_shaderdsl --bin bake_ibl_studio` and commit the blob",
            file.len(),
            blob.len()
        );
    }
}

#[test]
fn ibl_studio_numbers() {
    let (_, stats) = baked();
    // (a) the exact octahedral Jacobian.
    let four_pi = 4.0 * std::f64::consts::PI;
    assert!((stats.solid_angle_sum - four_pi).abs() <= 1e-3, "solid-angle sum {} vs 4pi", stats.solid_angle_sum);
    // (b) the normalization rule.
    let e_up = stats.irradiance[2];
    let lum = 0.2126 * e_up[0] + 0.7152 * e_up[1] + 0.0722 * e_up[2];
    assert!((lum - 1.0).abs() <= 1e-3, "luminance of E(+y) = {lum} (want 1)");
    // (d) G4 on the committed blob, every level.
    let asset = parse_env_blob(STUDIO_ENV_BLOB).expect("the committed blob parses");
    let words: Vec<u32> = asset.chain.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
    let (tile, levels) = (asset.header.tile, asset.header.levels);
    let mut off = 0usize;
    let mut checked = 0usize;
    for k in 0..levels {
        let n = tile >> k;
        for y in 0..n {
            for x in 0..n {
                let (sx, sy) = oct_env_border_source(x, y, n);
                if (sx, sy) != (x, y) {
                    assert_eq!(
                        words[off + (y * n + x) as usize],
                        words[off + (sy * n + sx) as usize],
                        "level {k}: border texel ({x},{y}) is not its continuation ({sx},{sy})"
                    );
                    checked += 1;
                }
            }
        }
        off += (n * n) as usize;
    }
    assert!(checked > 1000, "anti-vacuity: {checked} border texels checked");
    // (e) G3: the header's chain is the one the read side's lod spans.
    assert_eq!((tile, levels), (STUDIO_TILE, STUDIO_LEVELS));
    let l1 = (levels - 1) as f32;
    assert_eq!(env_lod_body::<f32>(1.0, l1), l1, "pR 1 reads the last committed level");
    assert_eq!(asset.header.sky_lane, stats.sky_lane, "the header carries the bake's sky lane");
    assert_eq!(asset.header.ground_lane, stats.ground_lane, "the header carries the bake's ground lane");
    // (f) reported, never gated.
    println!("k = {}", stats.k);
    for (n, e) in IRRADIANCE_NORMALS.iter().zip(stats.irradiance.iter()) {
        println!("E({n:?}) = {e:?}");
    }
    for (i, name) in [(0usize, "+x"), (1, "-x"), (4, "+z"), (5, "-z")] {
        let truth = stats.irradiance[i];
        let model: [f32; 3] = core::array::from_fn(|c| std::f32::consts::PI * (stats.sky_lane[c] + stats.ground_lane[c]) * 0.5);
        let rel: [f32; 3] = core::array::from_fn(|c| (model[c] - truth[c]) / truth[c]);
        println!("hemisphere diffuse model at {name}: relative error {rel:?}");
    }
}
