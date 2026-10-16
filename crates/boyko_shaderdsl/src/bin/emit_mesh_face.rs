//! `emit_mesh_face` — prints RD6 / D-DS's generated facing spans (`boyko_shaderdsl::face`
//! traced over the `Emit` backend), each wrapped in its `// === GENERATED <name> BEGIN/END ===`
//! sentinels, for splicing into the three opaque producer sources:
//!
//! - `mesh_face_consts` + `mesh_face_flip` → `forward_opaque.fs.hlsl`, `gbuffer_mrt.fs.hlsl`,
//!   `vb_geom_fetch.hlsli`;
//! - `vb_screen_front_ccw` → `vb_geom_fetch.hlsli` only.
//!
//! `boyko_rhi_vulkan/tests/mesh_face_edsl_sync.rs` re-runs these printers and fails on any
//! committed span that differs. After a re-splice, re-DXC every affected `.spv` per its
//! header's frozen recipe (the `*_spv_sync` gates pin the bytes).
//!
//! Run: `cargo run -p boyko_shaderdsl --features emit --bin emit_mesh_face`

fn span(name: &str, body: &str) -> String {
    format!("// === GENERATED {name} BEGIN === (boyko_shaderdsl::face)\n{body}// === GENERATED {name} END ===\n")
}

fn main() {
    print!("{}", span("mesh_face_consts", &boyko_shaderdsl::emit::emit_hlsl_mesh_face_consts()));
    println!();
    print!("{}", span("mesh_face_flip", &boyko_shaderdsl::emit::emit_hlsl_mesh_face_flip()));
    println!();
    print!("{}", span("vb_screen_front_ccw", &boyko_shaderdsl::emit::emit_hlsl_vb_screen_front_ccw()));
}
