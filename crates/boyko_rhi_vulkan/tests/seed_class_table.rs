//! OBS host gate for the seed class table (render-wave plan H0 Scope 4; OPEN-ITEMS 1, source
//! `critique_verify_r5.md` I3): the table is DATA, and every row of it can fail.
//!
//! # What it asserts
//!
//! 1. **Every row declared bare is a violation**, on every declarator of its mask — a single
//!    resource shared by both frames in flight declared with the per-FIF idiom (`add_image` /
//!    `add_buffer`) is exactly the B-002/B-003 race the table exists to catch. The four resources
//!    OPEN-ITEMS 1 names (`fog_sun_far`, `fog_scatter[0]`, `exposure_hist`, `easu_out`) and the
//!    production carrier `material_table` are asserted to BE rows, by name, so the gate cannot go
//!    vacuous by losing them.
//! 2. **Every row declared by its class is clean** (the class-appropriate declarator, a seed with
//!    the row's minimum stages, a writer seed where the row needs one) — including a mip chain
//!    declared with ONE level against an image row (`hzb_pyramid` on an HZB-off frame; critique
//!    W3: the kind column is {image, buffer}).
//! 3. **A seed missing its minimum stages is a violation**, for every row that states a minimum.
//! 4. **The table is well-formed:** one row per (name, declarator); pre-registered wave rows carry
//!    the all-declarators mask (critique W5); the classes are CrossFrame or FrameLocal only.

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
    VK_IMAGE_LAYOUT_GENERAL, VK_PIPELINE_STAGE_TRANSFER_BIT,
};
use boyko_rhi_vulkan::framegraph::{FrameGraph, ResId, ResSync, SeedClass, SubRange};
use boyko_rhi_vulkan::present::seed_class::{
    class_table_violation, declarator_name, ClassRule, SeedClassRow, SeedKind, DECLARATORS,
    DECL_ALL, SEED_CLASS_TABLE,
};

/// One frame of `declarator` declaring `row`'s name through `declare`, written once (so the
/// frame is well-formed whatever the class).
fn frame(row: &SeedClassRow, declare: impl FnOnce(&mut FrameGraph) -> ResId) -> FrameGraph {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let res = declare(&mut g);
    g.add_pass("writer");
    match row.kind {
        SeedKind::Image => g.image_access(
            res,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        ),
        SeedKind::Buffer => {
            g.buffer_access(res, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT)
        }
    }
    g
}

/// The seed a row's class-correct declaration carries: its minimum stages (any stage when the
/// owning rung has not filled it yet), as a writer seed where the row needs one.
fn row_seed(row: &SeedClassRow, stages: u32) -> ResSync {
    let stages = if stages == 0 { VK_PIPELINE_STAGE_TRANSFER_BIT } else { stages };
    if row.writer_seed {
        ResSync { visible_stages: stages, visible_access: VK_ACCESS_SHADER_READ_BIT, ..ResSync::seeded_writer(stages, VK_ACCESS_SHADER_WRITE_BIT) }
    } else {
        ResSync::seeded_readers(stages, VK_ACCESS_SHADER_READ_BIT)
    }
}

/// Declares `row.name` by its class with `seed`, as an image with `mips` levels or a buffer.
fn declare_by_class(g: &mut FrameGraph, row: &SeedClassRow, seed: ResSync, mips: u32) -> ResId {
    match (row.kind, row.class) {
        (SeedKind::Image, SeedClass::CrossFrame) if mips == 1 => g.add_image_seeded(row.name, seed),
        (SeedKind::Image, SeedClass::CrossFrame) => g.add_image_mipped(row.name, mips, seed),
        (SeedKind::Image, SeedClass::FrameLocal) if mips == 1 => {
            g.add_image_seeded_frame_local(row.name, seed)
        }
        (SeedKind::Image, SeedClass::FrameLocal) => {
            g.add_image_mipped_frame_local(row.name, mips, seed)
        }
        (SeedKind::Buffer, SeedClass::CrossFrame) => g.add_buffer_seeded(row.name, seed),
        (SeedKind::Buffer, SeedClass::FrameLocal) => g.add_buffer_seeded_frame_local(row.name, seed),
        (_, class) => panic!("row '{}' carries class {class:?}; rows are CrossFrame or FrameLocal", row.name),
    }
}

/// Every (row, declarator) pair of the table.
fn pairs() -> impl Iterator<Item = (&'static SeedClassRow, u8)> {
    SEED_CLASS_TABLE
        .iter()
        .flat_map(|row| DECLARATORS.iter().filter(move |&&d| row.declarators & d != 0).map(move |&d| (row, d)))
}

#[test]
fn every_row_declared_bare_is_a_violation() {
    for name in ["fog_sun_far", "fog_scatter[0]", "exposure_hist", "easu_out", "material_table"] {
        assert!(
            SEED_CLASS_TABLE.iter().any(|r| r.name == name),
            "OPEN-ITEMS 1: '{name}' must be a row of SEED_CLASS_TABLE"
        );
    }
    let mut checked = 0;
    let mut silent = Vec::new();
    for (row, d) in pairs() {
        let g = frame(row, |g| match row.kind {
            SeedKind::Image => g.add_image(row.name),
            SeedKind::Buffer => g.add_buffer(row.name),
        });
        match class_table_violation(&g, d) {
            Some(v) => {
                assert_eq!(v.rule, ClassRule::WrongClass, "'{}' on {}", row.name, declarator_name(d));
                assert_eq!(v.derived, SeedClass::Bare);
            }
            None => silent.push(format!("{} on {}", row.name, declarator_name(d))),
        }
        checked += 1;
    }
    assert!(silent.is_empty(), "rows declared bare that the class table did NOT catch: {silent:?}");
    assert!(checked >= SEED_CLASS_TABLE.len(), "anti-vacuity: {checked} (row, declarator) pairs");
}

#[test]
fn every_row_declared_by_its_class_is_clean() {
    for (row, d) in pairs() {
        let g = frame(row, |g| declare_by_class(g, row, row_seed(row, row.seed_stages_min), 1));
        assert_eq!(
            class_table_violation(&g, d),
            None,
            "'{}' declared by its class on {}",
            row.name,
            declarator_name(d)
        );
    }
}

/// Critique W3: a chain's level count is a frame-dependent shape (`hzb_pyramid` has one level on
/// an HZB-off frame), not a class property — an image row is clean with one level or many.
#[test]
fn an_image_row_is_clean_with_one_level_or_many() {
    let row = SEED_CLASS_TABLE
        .iter()
        .find(|r| r.name == "hzb_pyramid")
        .expect("invariant: hzb_pyramid is a HEAD row");
    for mips in [1, 4] {
        let mut g = FrameGraph::with_capacity(4, 4, 4);
        let res = declare_by_class(&mut g, row, row_seed(row, row.seed_stages_min), mips);
        g.add_pass("hzb_build");
        g.image_access(
            res,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::color_mips(mips),
        );
        assert_eq!(class_table_violation(&g, row.declarators), None, "hzb_pyramid with {mips} level(s)");
    }
}

#[test]
fn a_seed_missing_its_min_stages_is_a_violation() {
    let mut checked = 0;
    for (row, d) in pairs().filter(|(r, _)| r.seed_stages_min != 0) {
        // The lowest stage bit the row does NOT require, so the seed is non-empty but short.
        let other = (0..32).map(|b| 1u32 << b).find(|b| b & row.seed_stages_min == 0).expect("a free bit");
        let g = frame(row, |g| declare_by_class(g, row, row_seed(row, other), 1));
        let v = class_table_violation(&g, d)
            .unwrap_or_else(|| panic!("'{}' on {} with seed stages {other:#x}", row.name, declarator_name(d)));
        assert_eq!(v.rule, ClassRule::SeedMissingMinStages, "'{}'", row.name);
        checked += 1;
    }
    assert!(checked >= 40, "anti-vacuity: {checked} rows with a minimum");
}

#[test]
fn a_reader_seed_on_a_writer_row_is_a_violation() {
    let mut checked = 0;
    for (row, d) in pairs().filter(|(r, _)| r.writer_seed) {
        let seed = ResSync::seeded_readers(row.seed_stages_min, VK_ACCESS_SHADER_READ_BIT);
        let g = frame(row, |g| declare_by_class(g, row, seed, 1));
        let v = class_table_violation(&g, d).unwrap_or_else(|| panic!("'{}' with a reader seed", row.name));
        assert_eq!(v.rule, ClassRule::ReaderSeedOnWriterRow, "'{}'", row.name);
        checked += 1;
    }
    assert!(checked >= 10, "anti-vacuity: {checked} writer rows");
}

#[test]
fn rows_are_unique_per_declarator_and_wave_rows_cover_every_declarator() {
    for (i, a) in SEED_CLASS_TABLE.iter().enumerate() {
        assert!(
            matches!(a.class, SeedClass::CrossFrame | SeedClass::FrameLocal),
            "'{}': a row's class is CrossFrame or FrameLocal",
            a.name
        );
        assert_ne!(a.declarators & DECL_ALL, 0, "'{}': an empty declarator mask", a.name);
        if !a.expect_present && a.owner != "HEAD" {
            assert_eq!(a.declarators, DECL_ALL, "'{}': a pre-registered row carries the all-declarators mask", a.name);
        }
        for b in &SEED_CLASS_TABLE[i + 1..] {
            assert!(
                a.name != b.name || a.declarators & b.declarators == 0,
                "'{}' has two rows for one declarator",
                a.name
            );
        }
    }
}
