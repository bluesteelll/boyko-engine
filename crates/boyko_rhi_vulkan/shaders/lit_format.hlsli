// lit_format.hlsli -- render wave PX1a: the HDR scene colour `lit`, its ONE HLSL spelling.
//
// Since PX1a every `lit` producer stores pre-exposed SCENE-LINEAR radiance (no tonemap, no OETF,
// no display clamp) and ONE pass, `post_final`, applies the tonemap and the OETF into `display`.
// Every producer includes this file for the three things that make a `lit` store:
//
// - `LIT_FORMAT_QUALIFIER` -- the storage-image format of every `gLit` declaration
//   (`[[vk::image_format(LIT_FORMAT_QUALIFIER)]]`). RGBA16F: PX0's decision
//   (`docs/measurements/2026-10-16-px0-lit-format-decision.md`, row 2). Its twins are
//   `boyko_rhi_vulkan::present::LIT_FORMAT` (Rust) and `boyko_shaderdsl::post::LIT_STORAGE_FORMAT`
//   (the generators); `tests/lit_format_census.rs` holds the three equal.
// - `LIT_BACKGROUND_SCENE` -- the scene-linear background a pixel no producer covers holds (the
//   Deferred no-SkyLight arm, Forward's sky without a SkyLight; render wave PX1a D-S3, AD-13). The
//   binary16-representable value whose `OETF(ACES_Hill(.))` lands at the CENTRE of the 8-bit
//   background the display-referred store wrote before PX1a, (13, 13, 25) on the blessing device
//   (|display * 255 - centre| <= 0.002 per channel). Its Rust twin is
//   `boyko_rhi_vulkan::present::LIT_BACKGROUND_SCENE` (`FORWARD_LIT_CLEAR` / `VB_LIT_CLEAR` read
//   it); `tests/hdr_fixtures.rs` (test 5) pins the value, its representability and the twin.
// - `lit_store_saturate` -- the store bound (orchestrator ruling 67 W5): per channel
//   `c > HDR_MAX ? HDR_MAX : c`, so an overflowing highlight stores the brightest finite value
//   instead of a +Inf every HDR reader would read as a fault, while a NaN stays a NaN. GENERATED
//   from `boyko_shaderdsl::post::lit_store_saturate` by `cargo run -p boyko_shaderdsl --features
//   emit --bin emit_post -- --splice`; a hand-edit fails `tests/post_chain_spv_sync.rs`.
//   `tests/lit_store_clamp_census.rs` requires every `lit` store to pass through it.

#ifndef LIT_FORMAT_HLSLI
#define LIT_FORMAT_HLSLI

#define LIT_FORMAT_QUALIFIER "rgba16f"

static const float3 LIT_BACKGROUND_SCENE = float3(0.01222991943359375, 0.0118865966796875, 0.03192138671875);

// === GENERATED lit_store_saturate BEGIN ===
static const float HDR_MAX = 64512.0;

float3 lit_store_saturate(float3 c) {
    return float3(((c.x > HDR_MAX) ? (HDR_MAX) : (c.x)), ((c.y > HDR_MAX) ? (HDR_MAX) : (c.y)), ((c.z > HDR_MAX) ? (HDR_MAX) : (c.z)));
}
// === GENERATED lit_store_saturate END ===

#endif // LIT_FORMAT_HLSLI
