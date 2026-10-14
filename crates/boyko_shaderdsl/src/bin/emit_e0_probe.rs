//! `emit_e0_probe` — generates the committed TEST-ONLY device probe of the five render-wave E0
//! primitives (`Cf::{floor, frac, log2, exp2, pow}`):
//!
//! ```text
//! e0_prims_probe.comp.hlsl    one generated one-op function per primitive + the store loop
//! ```
//!
//! Run: `cargo run -p boyko_shaderdsl --features emit --bin emit_e0_probe`
//!
//! One source, ONE artifact, no `-D` axis. DXC the file with the frozen recipe pinned in its header
//! and commit the `.spv`. `boyko_rhi_vulkan/tests/e0_prims_edsl_sync.rs` pins the committed `.hlsl`
//! to this template, the committed `.spv` to a fresh re-DXC of its source and each op to one
//! `GLSL.std.450` instruction; `boyko_rhi_vulkan/tests/e0_prims_device.rs` dispatches it.
//!
//! The template itself is `boyko_shaderdsl::emit::emit_e0_prims_probe_comp` (library-side, so the
//! sync test compares the whole committed file to it); this bin only writes it.

use std::path::PathBuf;

use boyko_shaderdsl::emit;

fn main() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("boyko_rhi_vulkan")
        .join("shaders")
        .join("e0_prims_probe.comp.hlsl");
    let text = emit::emit_e0_prims_probe_comp();
    std::fs::write(&out, &text)
        .unwrap_or_else(|e| panic!("invariant: failed to write {} : {e}", out.display()));
    println!("wrote {} ({} bytes)", out.display(), text.len());
}
