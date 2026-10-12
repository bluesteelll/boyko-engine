//! OBS host test 5 (render-wave plan H0 Scope 4, AD-15): the seeded-layout invariant, the three
//! frame-local declarators and the derived seed class — derivation tests of `FrameGraph::compile`'s
//! own checks over the public `framegraph` API, which is legitimately host (no replica of a
//! production declarator is built here).
//!
//! # What it asserts
//!
//! 1. **Seeded-layout invariant.** An image declared with a layout seed must end the frame, on
//!    every mip, in that layout: a `GENERAL`-seeded image whose last access is
//!    `SHADER_READ_ONLY_OPTIMAL` fires; one ending in `GENERAL` is clean; a dump-shaped
//!    `TRANSFER_SRC` copy followed by a restore READ at the seed layout is clean; an image
//!    declared through `add_image_seeded_restored` is exempt.
//! 2. **Frame-local classes.** Each frame-local declarator keeps `compile`'s unwritten-read guard
//!    live (an image, a mip chain per mip, a buffer); the CrossFrame declarators with the same
//!    seed stay exempt; and the derived barrier stream of a frame-local and a cross-frame
//!    declaration with the same seed and accesses is IDENTICAL — the class changes the guard
//!    only.
//! 3. **The derived class** of every declarator, and the read-only accessors the receipt reads.
//!
//! The `should_panic` cases are `cfg(debug_assertions)`: the checks ARE debug assertions, and CI
//! runs a debug × release matrix (`framegraph_gbuffer_equiv.rs`'s rationale).

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT,
    VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
};
use boyko_rhi_vulkan::framegraph::{
    FrameGraph, PassId, ResId, ResSync, SeedClass, SubRange,
};

const COMPUTE: u32 = VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT;
const FRAGMENT: u32 = VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT;
const TRANSFER: u32 = VK_PIPELINE_STAGE_TRANSFER_BIT;

fn graph() -> FrameGraph {
    FrameGraph::with_capacity(8, 8, 16)
}

/// A storage write of `res` at `COMPUTE`, `GENERAL`, over `sub`.
fn storage_write(g: &mut FrameGraph, res: ResId, sub: SubRange) {
    g.image_access(res, COMPUTE, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, sub);
}

/// A sampled read of `res` at `FRAGMENT`, `SHADER_READ_ONLY_OPTIMAL`, over `sub`.
fn sampled_read(g: &mut FrameGraph, res: ResId, sub: SubRange) {
    g.image_access(
        res,
        FRAGMENT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        sub,
    );
}

fn general_seed() -> ResSync {
    ResSync::seeded_readers_at_layout(VK_IMAGE_LAYOUT_GENERAL, COMPUTE, VK_ACCESS_SHADER_READ_BIT)
}

fn reader_seed() -> ResSync {
    ResSync::seeded_readers(FRAGMENT | COMPUTE, VK_ACCESS_SHADER_READ_BIT)
}

// --- 1. the seeded-layout invariant ------------------------------------------------------------

/// perf W1's `taa_hist` case: a `GENERAL`-seeded image whose last reader samples it at
/// `SHADER_READ_ONLY_OPTIMAL` ends the frame outside its seed layout; next frame's barrier would
/// claim `oldLayout = GENERAL` for an image that is not.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "INVARIANT SEEDED-LAYOUT violated — image 'hist' (mip 0)")]
fn general_seeded_image_ending_in_shader_read_only_fires() {
    let mut g = graph();
    let hist = g.add_image_seeded("hist", general_seed());
    g.add_pass("resolve");
    storage_write(&mut g, hist, SubRange::COLOR);
    g.add_pass("present_sample");
    sampled_read(&mut g, hist, SubRange::COLOR);
    g.compile();
}

#[test]
fn general_seeded_image_ending_in_general_is_clean() {
    let mut g = graph();
    let hist = g.add_image_seeded("hist", general_seed());
    g.add_pass("resolve");
    storage_write(&mut g, hist, SubRange::COLOR);
    g.add_pass("reader");
    g.image_access(hist, COMPUTE, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    g.compile();
    assert_eq!(g.resolved_layout(hist), VK_IMAGE_LAYOUT_GENERAL);
}

/// The image-dump shape (H0 Scope 6): a seeded image whose boundary layout is
/// `SHADER_READ_ONLY_OPTIMAL` is copied at `TRANSFER_SRC_OPTIMAL`, then a restore READ at the
/// boundary layout ends the frame where the seed says.
#[test]
fn transfer_src_copy_then_restore_read_at_seed_layout_is_clean() {
    let mut g = graph();
    let seed = ResSync::seeded_readers_at_layout(
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        FRAGMENT,
        VK_ACCESS_SHADER_READ_BIT,
    );
    let display = g.add_image_seeded("display", seed);
    g.add_pass("post_final");
    storage_write(&mut g, display, SubRange::COLOR);
    g.add_pass("present_sample");
    sampled_read(&mut g, display, SubRange::COLOR);
    g.add_pass("image_dump");
    g.image_access(
        display,
        TRANSFER,
        VK_ACCESS_TRANSFER_READ_BIT,
        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        SubRange::COLOR,
    );
    sampled_read(&mut g, display, SubRange::COLOR);
    g.compile();
    assert_eq!(g.resolved_layout(display), VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
}

/// The explicit opt-out: the same violating frame as the first test, declared restored.
#[test]
fn restored_image_is_exempt() {
    let mut g = graph();
    let hist = g.add_image_seeded_restored("hist", general_seed(), "test: a host-side restore");
    g.add_pass("resolve");
    storage_write(&mut g, hist, SubRange::COLOR);
    g.add_pass("present_sample");
    sampled_read(&mut g, hist, SubRange::COLOR);
    g.compile();
    assert_eq!(g.resolved_layout(hist), VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
}

/// Per mip: a two-level chain whose level 1 ends outside the seed layout fires naming mip 1.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "INVARIANT SEEDED-LAYOUT violated — image 'chain' (mip 1)")]
fn seeded_layout_is_checked_per_mip() {
    let mut g = graph();
    let chain = g.add_image_mipped("chain", 2, general_seed());
    g.add_pass("build");
    storage_write(&mut g, chain, SubRange::color_mips(2));
    g.add_pass("sample_level_1");
    sampled_read(
        &mut g,
        chain,
        SubRange { base_mip: 1, mip_count: 1, ..SubRange::color_mips(2) },
    );
    g.compile();
}

// --- 2. the frame-local classes ----------------------------------------------------------------

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient image 'fl_image'")]
fn frame_local_image_read_without_writer_fires() {
    let mut g = graph();
    let img = g.add_image_seeded_frame_local("fl_image", reader_seed());
    g.add_pass("reader");
    sampled_read(&mut g, img, SubRange::COLOR);
    g.compile();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient image 'fl_chain'")]
fn frame_local_mipped_image_read_without_writer_fires() {
    let mut g = graph();
    let chain = g.add_image_mipped_frame_local("fl_chain", 3, reader_seed());
    g.add_pass("reader");
    sampled_read(&mut g, chain, SubRange::color_mips(3));
    g.compile();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient buffer 'fl_buffer'")]
fn frame_local_buffer_read_without_writer_fires() {
    let mut g = graph();
    let buf = g.add_buffer_seeded_frame_local("fl_buffer", reader_seed());
    g.add_pass("reader");
    g.buffer_access(buf, COMPUTE, VK_ACCESS_SHADER_READ_BIT);
    g.compile();
}

/// The guard runs per mip: writing level 0 does not make level 2 written.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient image 'fl_chain' (mip 2)")]
fn frame_local_mipped_write_level_0_read_level_2_fires_on_level_2() {
    let mut g = graph();
    let chain = g.add_image_mipped_frame_local("fl_chain", 3, reader_seed());
    g.add_pass("write_level_0");
    storage_write(&mut g, chain, SubRange::COLOR);
    g.add_pass("read_level_2");
    sampled_read(
        &mut g,
        chain,
        SubRange { base_mip: 2, mip_count: 1, ..SubRange::color_mips(3) },
    );
    g.compile();
}

/// The exemption the frame-local class removes: the CrossFrame declarators with the same seed
/// accept a read with no prior write.
#[test]
fn cross_frame_declarators_stay_exempt_from_the_unwritten_read_guard() {
    let mut g = graph();
    let img = g.add_image_seeded("cf_image", reader_seed());
    let chain = g.add_image_mipped("cf_chain", 3, reader_seed());
    let buf = g.add_buffer_seeded("cf_buffer", reader_seed());
    g.add_pass("readers");
    sampled_read(&mut g, img, SubRange::COLOR);
    sampled_read(&mut g, chain, SubRange::color_mips(3));
    g.buffer_access(buf, COMPUTE, VK_ACCESS_SHADER_READ_BIT);
    g.compile();
}

/// A derived barrier stream: the image barriers, the buffer barriers (each `Debug`-printed) and
/// the per-pass ranges.
type Stream = (Vec<String>, Vec<String>, Vec<(u32, u32, u32, u32)>);

/// Builds one write-then-read frame over a resource declared by `declare`, and returns the
/// derived stream.
fn stream_of(declare: impl FnOnce(&mut FrameGraph) -> ResId, is_image: bool, mips: u32) -> Stream {
    let mut g = graph();
    let res = declare(&mut g);
    let sub = if mips == 1 { SubRange::COLOR } else { SubRange::color_mips(mips) };
    g.add_pass("write");
    if is_image {
        storage_write(&mut g, res, sub);
    } else {
        g.buffer_access(res, COMPUTE, VK_ACCESS_SHADER_WRITE_BIT);
    }
    g.add_pass("read");
    if is_image {
        sampled_read(&mut g, res, sub);
    } else {
        g.buffer_access(res, FRAGMENT, VK_ACCESS_SHADER_READ_BIT);
    }
    g.compile();
    (
        g.img_barriers().iter().map(|b| format!("{b:?}")).collect(),
        g.buf_barriers().iter().map(|b| format!("{b:?}")).collect(),
        g.pass_barriers()
            .iter()
            .map(|r| (r.img_begin, r.img_count, r.buf_begin, r.buf_count))
            .collect(),
    )
}

/// The seed is a reader seed, so the first write derives a WAR barrier whose source scope IS the
/// seed — a start state that ignored the seed would derive `TOP_OF_PIPE` and differ here.
#[test]
fn frame_local_image_stream_equals_cross_frame_stream() {
    let fl = stream_of(|g| g.add_image_seeded_frame_local("img", reader_seed()), true, 1);
    let cf = stream_of(|g| g.add_image_seeded("img", reader_seed()), true, 1);
    assert_eq!(fl, cf);
    assert!(!fl.0.is_empty(), "anti-vacuity: the write derived an image barrier");
}

#[test]
fn frame_local_mipped_image_stream_equals_cross_frame_stream() {
    let fl = stream_of(|g| g.add_image_mipped_frame_local("chain", 4, reader_seed()), true, 4);
    let cf = stream_of(|g| g.add_image_mipped("chain", 4, reader_seed()), true, 4);
    assert_eq!(fl, cf);
    assert!(!fl.0.is_empty(), "anti-vacuity: the write derived an image barrier");
}

#[test]
fn frame_local_buffer_stream_equals_cross_frame_stream() {
    let fl = stream_of(|g| g.add_buffer_seeded_frame_local("buf", reader_seed()), false, 1);
    let cf = stream_of(|g| g.add_buffer_seeded("buf", reader_seed()), false, 1);
    assert_eq!(fl, cf);
    assert!(!fl.1.is_empty(), "anti-vacuity: the write derived a buffer barrier");
}

/// The seed is what the stream is derived from: the same frame-local frame under an UNDEFINED
/// seed differs (so the equalities above are not a stream compared with itself).
#[test]
fn the_seed_moves_the_frame_local_stream() {
    let seeded = stream_of(|g| g.add_image_seeded_frame_local("img", reader_seed()), true, 1);
    let bare = stream_of(|g| g.add_image("img"), true, 1);
    assert_ne!(seeded, bare);
}

// --- 3. the derived class and the accessors ----------------------------------------------------

#[test]
fn every_declarator_derives_its_class() {
    let mut g = graph();
    let bare = g.add_image("bare");
    let bare_buf = g.add_buffer("bare_buf");
    let cross = g.add_image_seeded("cross", reader_seed());
    let cross_chain = g.add_image_mipped("cross_chain", 2, reader_seed());
    let cross_buf = g.add_buffer_seeded("cross_buf", reader_seed());
    let provenance = g.add_buffer_seeded("provenance", ResSync::undefined());
    let local = g.add_image_seeded_frame_local("local", reader_seed());
    let local_chain = g.add_image_mipped_frame_local("local_chain", 2, reader_seed());
    let local_buf = g.add_buffer_seeded_frame_local("local_buf", reader_seed());
    for (res, class) in [
        (bare, SeedClass::Bare),
        (bare_buf, SeedClass::Bare),
        (cross, SeedClass::CrossFrame),
        (cross_chain, SeedClass::CrossFrame),
        (cross_buf, SeedClass::CrossFrame),
        (provenance, SeedClass::ProvenanceOnly),
        (local, SeedClass::FrameLocal),
        (local_chain, SeedClass::FrameLocal),
        (local_buf, SeedClass::FrameLocal),
    ] {
        assert_eq!(g.res_class(res), class, "class of '{}'", g.res_name(res));
    }
}

#[test]
fn accessors_report_the_declared_graph() {
    let mut g = graph();
    let img = g.add_image_seeded("img", general_seed());
    let chain = g.add_image_mipped("chain", 3, reader_seed());
    let buf = g.add_buffer("buf");
    g.add_pass("first");
    storage_write(&mut g, img, SubRange::COLOR);
    g.buffer_access(buf, COMPUTE, VK_ACCESS_SHADER_WRITE_BIT);
    g.add_pass("second");
    storage_write(&mut g, chain, SubRange::color_mips(3));
    g.compile();

    assert_eq!(g.res_count(), 3);
    assert_eq!(g.res_state_total(), 5);
    assert!(g.res_is_image(img) && g.res_is_image(chain) && !g.res_is_image(buf));
    assert_eq!((g.res_mips(img), g.res_mips(chain), g.res_mips(buf)), (1, 3, 1));
    assert!(g.res_seeded(img) && g.res_seeded(chain) && !g.res_seeded(buf));
    assert_eq!(g.res_seed(img).layout, VK_IMAGE_LAYOUT_GENERAL);
    assert_eq!(g.pass_count(), 2);
    assert_eq!((g.pass_name(PassId(0)), g.pass_name(PassId(1))), ("first", "second"));

    let first: Vec<_> = g.pass_accesses(PassId(0)).collect();
    assert_eq!(first.len(), 2);
    assert_eq!((first[0].res, first[0].stage, first[0].layout), (img, COMPUTE, VK_IMAGE_LAYOUT_GENERAL));
    assert_eq!((first[1].res, first[1].access), (buf, VK_ACCESS_SHADER_WRITE_BIT));
    let second: Vec<_> = g.pass_accesses(PassId(1)).rev().collect();
    assert_eq!((second[0].res, second[0].sub.mip_count), (chain, 3));
    assert_eq!(second[0].access, VK_ACCESS_SHADER_WRITE_BIT);
}
