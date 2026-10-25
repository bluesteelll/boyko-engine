//! IBL1 R0 (`ibl1/cut.md` C8): bakes the procedural studio into the committed environment blob
//! `crates/boyko_render/assets/ibl/studio_256.envoct` (a 64-B header + the 256² × 7 bordered
//! octahedral B10G11R11 chain, 349,568 B) and prints what the bake measured.
//!
//! The bake is deterministic on every host (IEEE operations only, `boyko_shaderdsl::ibl`'s module
//! doc), so `boyko_render/tests/ibl_studio_sync.rs` re-bakes it and compares the bytes. Re-run this
//! after editing the studio table and commit the blob with it:
//!
//! ```text
//! cargo run --release -p boyko_shaderdsl --bin bake_ibl_studio --locked
//! ```
//!
//! An optional first argument overrides the output path.

use std::path::PathBuf;

use boyko_shaderdsl::ibl::{
    IRRADIANCE_NORMALS, STUDIO_BAKE, STUDIO_BLOB_BYTES, STUDIO_LEVELS, STUDIO_SOURCE_TILE, STUDIO_TILE,
    bake_studio_into, env_chain_words, env_source_scratch_len, write_env_blob,
};

fn main() {
    let out = std::env::args().nth(1).map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../boyko_render/assets/ibl/studio_256.envoct"),
        PathBuf::from,
    );
    let mut chain = vec![0u32; env_chain_words(STUDIO_TILE, STUDIO_LEVELS)];
    let mut scratch = vec![0f32; env_source_scratch_len(STUDIO_SOURCE_TILE)];
    let stats = bake_studio_into(&mut chain, &mut scratch);
    let mut blob = vec![0u8; STUDIO_BLOB_BYTES];
    write_env_blob(&STUDIO_BAKE, &stats, &chain, &mut blob);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("cannot create {}: {e}", dir.display()));
    }
    std::fs::write(&out, &blob).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    println!("wrote {} ({} bytes)", out.display(), blob.len());
    println!("normalization k = {}", stats.k);
    println!("solid-angle sum = {} (4pi = {})", stats.solid_angle_sum, 4.0 * std::f64::consts::PI);
    for (n, e) in IRRADIANCE_NORMALS.iter().zip(stats.irradiance.iter()) {
        println!("E({n:?}) = {e:?}");
    }
    println!("sky lane = {:?}, ground lane = {:?}", stats.sky_lane, stats.ground_lane);
    // The hemisphere diffuse model's irradiance at a horizontal normal is pi * (sky + ground) / 2
    // (hemi = 0.5); reported against the bake's true E there, never gated (SH-9 decides on it).
    for (i, name) in [(0usize, "+x"), (1, "-x"), (4, "+z"), (5, "-z")] {
        let model: [f32; 3] =
            core::array::from_fn(|c| std::f32::consts::PI * (stats.sky_lane[c] + stats.ground_lane[c]) * 0.5);
        let truth = stats.irradiance[i];
        let rel: [f32; 3] = core::array::from_fn(|c| (model[c] - truth[c]) / truth[c]);
        println!("hemisphere model at {name}: {model:?} vs E {truth:?} (relative error {rel:?})");
    }
}
