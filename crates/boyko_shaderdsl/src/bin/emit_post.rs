//! `emit_post` — generates the committed post chain's final pass (render wave PXe):
//!
//! ```text
//! post_final.comp.hlsl    the traced post_final_main body + the declarations from the tables
//! ```
//!
//! Run: `cargo run -p boyko_shaderdsl --features emit --bin emit_post`
//!
//! One source, ONE artifact, no `-D` axis (every on/off is a mode word). DXC the file with the
//! frozen recipe pinned in its header and commit the `.spv`.
//! `boyko_rhi_vulkan/tests/post_edsl_sync.rs` pins the committed `.hlsl` to this template, the
//! committed `.spv` to a fresh re-DXC of its source, and the module's structure (the OETF `Pow`s,
//! the `[branch]` dispatch, the one fetch, the declared bindings, the stage trace).
//!
//! The template itself is `boyko_shaderdsl::emit::emit_post_final_comp` (library-side, so the sync
//! test compares the whole committed file to it); this bin only writes it.

use std::path::PathBuf;

use boyko_shaderdsl::emit;

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("boyko_rhi_vulkan")
        .join("shaders")
        .join("post_final.comp.hlsl");
    let text = emit::emit_post_final_comp();
    std::fs::write(&out, &text)
        .unwrap_or_else(|e| panic!("invariant: failed to write {} : {e}", out.display()));
    println!("wrote {} ({} bytes)", out.display(), text.len());
}
