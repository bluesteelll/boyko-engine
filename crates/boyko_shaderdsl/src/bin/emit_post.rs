//! `emit_post` — generates the committed post chain's final pass (render wave PXe) and, with
//! `--splice`, the eDSL-owned HDR boundary spans of the hand-written post shaders (render wave
//! PX1a, `px1a/cut.md` D-S2):
//!
//! ```text
//! post_final.comp.hlsl      the traced post_final_main body + the declarations from the tables
//! --splice:
//!   lit_format.hlsli          // === GENERATED lit_store_saturate ===  (every lit producer's store bound)
//!   taa_resolve.comp.hlsl     // === GENERATED post_hdr_boundary ===   (hdr_sanitize, hdr_history_guard)
//!   ssaa_downsample.fs.hlsl   // === GENERATED post_hdr_boundary ===
//!   fullscreen_sample.fs.hlsl // === GENERATED srgb_eotf ===          (the *_SRGB present variant, D-P1)
//! ```
//!
//! Run: `cargo run -p boyko_shaderdsl --features emit --bin emit_post [-- --splice]`
//!
//! One source, ONE artifact, no `-D` axis (every on/off is a mode word). DXC the file with the
//! frozen recipe pinned in its header and commit the `.spv`.
//! `boyko_rhi_vulkan/tests/post_edsl_sync.rs` pins the committed `.hlsl` to this template, the
//! committed `.spv` to a fresh re-DXC of its source, and the module's structure (the OETF `Pow`s,
//! the `[branch]` RCAS stage and dispatch, the fetch structure — the scene texel once, RCAS's four
//! ring taps inside the sharpen branch (render wave PX1b) — the declared bindings, the stage
//! trace).
//! `boyko_rhi_vulkan/tests/post_chain_spv_sync.rs` pins every spliced span to
//! `emit::post_splices()` and each spliced shader's `.spv` to its re-DXC.
//!
//! The template itself is `boyko_shaderdsl::emit::emit_post_final_comp` (library-side, so the sync
//! test compares the whole committed file to it); this bin only writes it.

use std::path::PathBuf;

use boyko_shaderdsl::emit;

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("boyko_rhi_vulkan").join("shaders")
}

/// Rewrites every `emit::post_splices()` span in place (the hand-written text around it untouched).
fn splice() {
    for (file, name, body) in emit::post_splices() {
        let path = shaders_dir().join(file);
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("invariant: failed to read {} : {e}", path.display()));
        // A checkout under `core.autocrlf` holds CRLF: splice on LF, write the file's own endings.
        let crlf = src.contains("\r\n");
        let out = emit::splice_span(&src.replace("\r\n", "\n"), name, &body)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let out = if crlf { out.replace('\n', "\r\n") } else { out };
        std::fs::write(&path, &out).unwrap_or_else(|e| panic!("invariant: failed to write {} : {e}", path.display()));
        println!("spliced `{name}` into {} ({} bytes)", path.display(), out.len());
    }
}

fn main() {
    if std::env::args().skip(1).any(|a| a == "--splice") {
        splice();
        return;
    }
    let out = shaders_dir().join("post_final.comp.hlsl");
    let text = emit::emit_post_final_comp();
    std::fs::write(&out, &text)
        .unwrap_or_else(|e| panic!("invariant: failed to write {} : {e}", out.display()));
    println!("wrote {} ({} bytes)", out.display(), text.len());
}
