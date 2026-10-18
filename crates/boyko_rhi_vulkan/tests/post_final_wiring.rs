//! Render wave PX1a (`plan/H4-PX1a-hdr-lit.md` Scope 2-3; `px1a/cut.md` D-G1, D-O2, D-O3; the binding
//! row DS-H4 (b) and (c) of `04-AMENDMENTS-R6.md` rev 6.4): **the post-final wiring tables, held on
//! the host.**
//!
//! - **(c) The receipt words.** The graph receipt's `[config]` ENDS with
//!   `boyko_shaderdsl::post::POST_FINAL_RECEIPT_KEYS`, in order: `golden_diff.py eval-post` reads the
//!   frame's `PostFinalGpu` words under exactly those keys. `boyko_rhi_vulkan` has no production
//!   edge to the eDSL, so `frame_driver.rs` spells the keys and this test pins the spelling.
//! - **(b) The dump names.** `BOYKO_IMAGE_DUMP` knows `display` (the 8-bit `post_final` output) and
//!   `lit_native` (SSAA's native-extent HDR box); `lit` and `lit_native` stage the texel size of
//!   `LIT_FORMAT` (RGBA16F: 8 B) — a 4-B staging would make the recorder skip every `lit` copy
//!   once the format moves, and every (D) gate on `lit` would read "not copied".
//! - **The seed rows.** `display` is FrameLocal on every declarator, present on every max-config
//!   frame, seeded with `FRAGMENT | COMPUTE` readers (present / FXAA / SMAA, CAS and `post_final`);
//!   `lit_native` is FrameLocal on Deferred and VB only (Forward has no SSAA), registered but not
//!   expected on the census's max config (TAA), seeded with its `COMPUTE` reader (`post_final`).

use boyko_rhi_vulkan::ffi::{VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT};
use boyko_rhi_vulkan::framegraph::SeedClass;
use boyko_rhi_vulkan::present::seed_class::{self, DECL_DEFERRED, DECL_FORWARD, DECL_VB, SeedKind};
use boyko_rhi_vulkan::present::{IMAGE_DUMP_MAX, ImageDumpName, RECEIPT_CONFIG_KEYS};
use boyko_shaderdsl::post::POST_FINAL_RECEIPT_KEYS;

/// `R16G16B16A16_SFLOAT`'s texel size — `LIT_FORMAT` (PX0's decision).
const LIT_TEXEL_BYTES: u64 = 8;

#[test]
fn the_receipt_config_ends_with_post_finals_words() {
    let n = POST_FINAL_RECEIPT_KEYS.len();
    let keys = &RECEIPT_CONFIG_KEYS[..];
    assert!(
        keys.len() >= n && keys[keys.len() - n..] == POST_FINAL_RECEIPT_KEYS[..],
        "the receipt's [config] does not end with post::POST_FINAL_RECEIPT_KEYS {POST_FINAL_RECEIPT_KEYS:?}"
    );
    let mut sorted = keys.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), keys.len(), "a [config] key is spelled twice");
}

#[test]
fn the_dump_knows_display_and_lit_native() {
    let display = ImageDumpName::parse("display").expect("no image-dump name `display`");
    assert_eq!(display.graph_name(), "display");
    assert!(!display.is_depth());
    assert_eq!(display.staging_texel_bytes(), 4, "display is R8G8B8A8_UNORM");
    let native = ImageDumpName::parse("lit_native").expect("no image-dump name `lit_native`");
    assert_eq!(native.graph_name(), "lit_native");
    assert!(!native.is_depth());
    assert_eq!(native.staging_texel_bytes(), LIT_TEXEL_BYTES, "lit_native stages LIT_FORMAT's texel");
}

#[test]
fn lit_stages_lit_formats_texel() {
    assert_eq!(
        ImageDumpName::Lit.staging_texel_bytes(),
        LIT_TEXEL_BYTES,
        "lit's dump staging is not LIT_FORMAT's texel size: every (D) gate on lit would skip its copy"
    );
}

#[test]
fn every_dump_name_round_trips_and_indexes_its_slot() {
    const { assert!(IMAGE_DUMP_MAX <= 8, "a dump name past bit 7 overflows `declare_image_dump`'s u8 name mask") };
    for (k, name) in ImageDumpName::ALL.into_iter().enumerate() {
        assert_eq!(name as usize, k, "{name:?} is not ALL[{k}]");
        assert_eq!(ImageDumpName::parse(name.graph_name()), Some(name));
    }
}

#[test]
fn display_is_a_present_frame_local_row_on_every_declarator() {
    for decl in [DECL_DEFERRED, DECL_FORWARD, DECL_VB] {
        let row = seed_class::row_for("display", decl).expect("invariant: display has a row (OBS pre-registered it)");
        assert!(row.expect_present, "display is not expected on declarator {decl}'s max-config frame");
        assert_eq!(row.declarators, DECL_DEFERRED | DECL_FORWARD | DECL_VB);
        assert_eq!(row.class, SeedClass::FrameLocal);
        assert_eq!(row.kind, SeedKind::Image);
        assert_eq!(
            row.seed_stages_min,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            "display's seed must name its FRAGMENT and COMPUTE readers"
        );
        assert_eq!(row.owner, "PX1a");
    }
}

#[test]
fn lit_native_is_a_frame_local_row_on_deferred_and_vb() {
    assert!(seed_class::row_for("lit_native", DECL_FORWARD).is_none(), "lit_native has a Forward row: Forward has no SSAA");
    for decl in [DECL_DEFERRED, DECL_VB] {
        let row = seed_class::row_for("lit_native", decl).expect("invariant: lit_native has a row");
        assert_eq!(row.declarators, DECL_DEFERRED | DECL_VB);
        assert!(!row.expect_present, "lit_native is SSAA-only: not on the census's max-config (TAA) frame");
        assert_eq!(row.class, SeedClass::FrameLocal);
        assert_eq!(row.seed_stages_min, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
        assert_eq!(row.owner, "PX1a");
    }
}
