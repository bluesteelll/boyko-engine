//! `emit_ssao_variants` — generates the Render P7-Q2 PRE-COMPILED SSAO quality variants
//! (Mechanism C): one `sdf_ssao_<quality>.comp.hlsl` per [`boyko_shaderdsl::ssao::
//! SSAO_PRESETS`] row, each BAKING that preset's `static const` tuning so every `[unroll]`
//! slice/step loop stays fully unrolled (ZERO per-pixel runtime cost; the host selects a
//! variant by binding its pipeline, NEVER a dynamic loop bound).
//!
//! The variants are single-sourced from the committed base `sdf_ssao.comp.hlsl`: this bin
//! reads the base, swaps ONLY its `static const SSAO_*` tuning header (the 6 lines from
//! `SSAO_RADIUS` through `SSAO_EPS`) for the per-preset header
//! ([`boyko_shaderdsl::ssao::ssao_glue_header`]), and writes the variant next to the base.
//! The eDSL-GENERATED horizon-step span and ALL hand-written glue (the forward neighbour
//! reconstruct, the rotation/step-phase dither, the `occ → ao` fold) are carried VERBATIM
//! from the base — only the header (and thereby the loop bounds, which read `SSAO_SLICES`/
//! `SSAO_STEPS`) varies. So the Medium variant is byte-identical to the base (the no-op
//! proof) and the generated span text is identical across all three.
//!
//! Run: `cargo run -p boyko_shaderdsl --features emit --bin emit_ssao_variants`
//!
//! Then DXC each variant with the frozen recipe (the same one in every shader header):
//!   `dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//!        sdf_ssao_<quality>.comp.hlsl -Fo sdf_ssao_<quality>.comp.spv`
//! (cwd = the shaders dir, so the relative `#include "ray_gen.hlsli"` resolves). The
//! `ssao_edsl_sync` per-variant byte-identity loop pins each committed variant `.spv` to a
//! fresh re-DXC of the re-emitted variant `.hlsl`.

use std::path::PathBuf;

use boyko_shaderdsl::ssao::{self, SsaoQuality};

fn main() {
    // The shaders dir is resolved relative to this crate's manifest: it is a sibling crate
    // under the same workspace (`../boyko_rhi_vulkan/shaders`). The bin is a developer tool,
    // not shipped, so the cross-crate path is acceptable (it mirrors the recipe header).
    let shaders = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("boyko_rhi_vulkan")
        .join("shaders");

    let base_path = shaders.join("sdf_ssao.comp.hlsl");
    let base = std::fs::read_to_string(&base_path).unwrap_or_else(|e| {
        panic!(
            "invariant: the base SSAO shader {} must exist: {e}",
            base_path.display()
        )
    });

    for quality in SsaoQuality::ALL {
        let variant = ssao::variant_hlsl(&base, quality.params());
        let out = shaders.join(format!("sdf_ssao_{}.comp.hlsl", quality.suffix()));
        std::fs::write(&out, &variant)
            .unwrap_or_else(|e| panic!("invariant: failed to write {} : {e}", out.display()));
        println!("wrote {} ({} bytes)", out.display(), variant.len());
    }
}
