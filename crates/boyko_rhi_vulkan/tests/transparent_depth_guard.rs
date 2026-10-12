//! Render wave RD1 (defect R0b): host tests 3 and 4 over the transparent-depth rule
//! (`boyko_rhi_vulkan::present::transparent_depth`) — the pure functions every declarator calls.
//!
//! 3. **The predicate truth table** — path {Deferred, Forward, ForwardPlus, VisibilityBuffer} ×
//!    legs {Both, Mesh, Sdf} × capability {∅, particles} × frame ⊆ capability × TAA × the HZB dump
//!    armed × the disarm key: `sdf_depth_merge` and `sdf_forward_writes_viewt` against the rung's
//!    formulas, spelled independently here. The frame ⊄ capability case is asserted on the pure
//!    `frame_within_capability` in every build and as a debug panic where debug assertions exist.
//! 4. **The guards over synthetic frame graphs** built through the public `framegraph` API — the
//!    order guard classifies by DECLARED ACCESS (a ForwardPlus EQUAL read before the marcher, an
//!    HZB compute read, the merge's read+write and a transfer dump are not consumers; a pass named
//!    `translucent_draw` is, with no registration); the pairing check in both directions; and the
//!    merge's CLEAR tied to its derived first touch (critique W1).
//!
//! Rows that exercise a `debug_assertions`-only wrapper are `cfg`-gated, so the release test leg
//! (`cargo test --release --workspace`) compiles and runs every other row (critique O3).

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_READ_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT,
    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT,
    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
    VK_PIPELINE_STAGE_TRANSFER_BIT,
};
use boyko_rhi_vulkan::framegraph::{FrameGraph, PassId, ResId, SubRange};
use boyko_rhi_vulkan::present::transparent_depth::{
    SdfDepthMerge, TRANSPARENT_CLASS_PARTICLES, TransparentDepthCapability, TransparentDepthFrame,
    frame_within_capability, is_transparent_depth_consumer, merge_clear_violation, sdf_depth_merge,
    sdf_forward_writes_viewt, transparent_depth_order_violation, transparent_depth_pairing_violation,
};

/// The depth-attachment stage pair every depth attachment access uses.
const FRAG_TESTS: u32 =
    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;

// ---- Host test 3: the predicate truth table ------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    Deferred,
    Forward,
    ForwardPlus,
    Vb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Legs {
    Both,
    Mesh,
    Sdf,
}

const PATHS: [Path; 4] = [Path::Deferred, Path::Forward, Path::ForwardPlus, Path::Vb];
const LEGS: [Legs; 3] = [Legs::Both, Legs::Mesh, Legs::Sdf];
const CAPABILITIES: [TransparentDepthCapability; 2] =
    [TransparentDepthCapability::NONE, TransparentDepthCapability::PARTICLES];

/// Every frame bitset within `cap` (frame ⊆ capability by construction).
fn frames_within(cap: TransparentDepthCapability) -> Vec<TransparentDepthFrame> {
    (0..=u8::MAX)
        .filter(|bits| bits & !cap.bits() == 0)
        .map(|bits| TransparentDepthFrame::within(cap, bits))
        .collect()
}

#[test]
fn the_merge_predicate_and_the_viewt_variant_follow_the_rung_formulas() {
    let mut rows = 0usize;
    let (mut draws, mut clears) = (0usize, 0usize);
    for path in PATHS {
        for legs in LEGS {
            let sdf_leg = matches!(legs, Legs::Both | Legs::Sdf);
            let mesh_leg = matches!(legs, Legs::Both | Legs::Mesh);
            let vb_sdf_only = path == Path::Vb && legs == Legs::Sdf;
            let sdf_forward_marched = sdf_leg && path != Path::Deferred;
            for cap in CAPABILITIES {
                for frame in frames_within(cap) {
                    for taa in [false, true] {
                        for hzb_dump_armed in [false, true] {
                            // `GBufferScene::hzb_dump_declared`: the dump pass exists only on a VB
                            // frame with a mesh leg (and an armed pyramid, implied here).
                            let hzb_declared = hzb_dump_armed && path == Path::Vb && mesh_leg;
                            for disarmed in [false, true] {
                                let got = sdf_depth_merge(
                                    sdf_leg,
                                    vb_sdf_only,
                                    hzb_declared,
                                    frame,
                                    disarmed,
                                );
                                let want = if sdf_leg && !hzb_declared && frame.any() && !disarmed {
                                    SdfDepthMerge::Draw
                                } else if vb_sdf_only && frame.any() && disarmed {
                                    SdfDepthMerge::ClearOnly
                                } else {
                                    SdfDepthMerge::Absent
                                };
                                assert_eq!(
                                    got, want,
                                    "sdf_depth_merge: path {path:?} legs {legs:?} cap {cap:?} \
                                     frame {frame:?} taa {taa} hzb_declared {hzb_declared} \
                                     disarmed {disarmed}"
                                );
                                draws += usize::from(got == SdfDepthMerge::Draw);
                                clears += usize::from(got == SdfDepthMerge::ClearOnly);
                                assert_eq!(
                                    sdf_forward_writes_viewt(sdf_forward_marched, taa, cap),
                                    sdf_forward_marched && (taa || cap.any()),
                                    "sdf_forward_writes_viewt: path {path:?} legs {legs:?} cap \
                                     {cap:?} taa {taa}"
                                );
                                rows += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("truth table: {rows} rows, {draws} Draw, {clears} ClearOnly");
    // Anti-vacuity: every outcome is reached.
    assert!(draws > 0 && clears > 0 && rows > draws + clears);
}

#[test]
fn a_default_boot_arms_nothing() {
    // Default `EnginePlugins`: no consumer class ⇒ no merge on any path, and the marcher writes
    // `t` only for TAA (the pre-RD1 VIEWT rule).
    let cap = TransparentDepthCapability::NONE;
    for (sdf_leg, vb_sdf_only) in [(true, false), (true, true), (false, false)] {
        for disarmed in [false, true] {
            assert_eq!(
                sdf_depth_merge(sdf_leg, vb_sdf_only, false, TransparentDepthFrame::NONE, disarmed),
                SdfDepthMerge::Absent
            );
        }
    }
    assert!(!sdf_forward_writes_viewt(true, false, cap));
    assert!(sdf_forward_writes_viewt(true, true, cap));
    assert!(sdf_forward_writes_viewt(true, false, TransparentDepthCapability::PARTICLES));
    assert!(!sdf_forward_writes_viewt(false, true, TransparentDepthCapability::PARTICLES));
}

#[test]
fn the_capability_bits_are_the_class_indices() {
    let cap = TransparentDepthCapability::NONE.with(TRANSPARENT_CLASS_PARTICLES, true);
    assert_eq!(cap, TransparentDepthCapability::PARTICLES);
    assert!(cap.has(TRANSPARENT_CLASS_PARTICLES) && cap.any());
    assert_eq!(TransparentDepthCapability::NONE.with(TRANSPARENT_CLASS_PARTICLES, false), TransparentDepthCapability::NONE);
}

#[test]
fn a_frame_outside_its_capability_is_refused_by_the_pure_rule() {
    // Always-on half (every build): the subset rule itself.
    let none = TransparentDepthCapability::NONE;
    let p = TransparentDepthCapability::PARTICLES;
    let bit = 1u8 << TRANSPARENT_CLASS_PARTICLES;
    assert!(frame_within_capability(TransparentDepthFrame::within(p, bit), p));
    assert!(frame_within_capability(TransparentDepthFrame::NONE, none));
    // A particle frame (legal on a particle boot) checked against a boot that armed nothing.
    assert!(!frame_within_capability(TransparentDepthFrame::within(p, bit), none));
    // `within` debug-asserts; under release it builds the set, which the pure rule then refuses.
    #[cfg(not(debug_assertions))]
    assert!(!frame_within_capability(TransparentDepthFrame::within(none, bit), none));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "is not within the boot capability")]
fn a_frame_outside_its_capability_panics_in_a_debug_build() {
    let _ = TransparentDepthFrame::within(TransparentDepthCapability::NONE, 1 << TRANSPARENT_CLASS_PARTICLES);
}

// ---- Host test 4: the guards over synthetic frame graphs ------------------------------------------

/// The resources of one synthetic frame.
struct Ids {
    depth: ResId,
    lit: ResId,
    viewt: ResId,
}

/// The passes of one synthetic frame (the `Option`s are the optional ones).
#[derive(Default)]
struct Passes {
    forward_opaque: Option<PassId>,
    marcher: Option<PassId>,
    hzb: Option<PassId>,
    merge: Option<PassId>,
    consumer: Option<PassId>,
    dump: Option<PassId>,
}

/// What [`frame`] declares.
struct Shape {
    /// ForwardPlus's prepass + EQUAL `forward_opaque` before the marcher (otherwise the depth's
    /// first writer is a plain raster pass, or nothing under `first_touch_merge`).
    forward_plus: bool,
    /// No pass writes the depth before the merge (the VB×Sdf leg).
    first_touch_merge: bool,
    /// The HZB build's compute read of the depth after the marcher.
    hzb: bool,
    /// Declare the merge.
    merge: bool,
    /// The consumer's name and whether it reads the depth as a FRAGMENT-SHADER sampled image
    /// (a soft-particle style read) instead of as a read-only attachment.
    consumer: Option<(&'static str, bool)>,
    /// The image-dump TRANSFER read of the depth, last.
    dump: bool,
}

/// Declares one synthetic frame and compiles it.
fn frame(shape: &Shape) -> (FrameGraph, Ids, Passes) {
    let mut g = FrameGraph::with_capacity(8, 16, 64);
    let ids = Ids { depth: g.add_image("path_depth"), lit: g.add_image("lit"), viewt: g.add_image("viewt") };
    let mut p = Passes::default();
    if !shape.first_touch_merge {
        if shape.forward_plus {
            g.add_pass("depth_prepass");
            depth_attachment(&mut g, ids.depth, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
            let fo = g.add_pass("forward_opaque");
            // The EQUAL test: attachment READ only, declared BEFORE the marcher.
            depth_attachment(&mut g, ids.depth, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT);
            colour(&mut g, ids.lit, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT);
            p.forward_opaque = Some(fo);
        } else {
            g.add_pass("raster");
            depth_attachment(&mut g, ids.depth, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
            colour(&mut g, ids.lit, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT);
        }
    }
    let m = g.add_pass("sdf_forward_march");
    g.image_access(ids.lit, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    if !shape.first_touch_merge {
        g.image_access(ids.depth, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH);
    }
    g.image_access(ids.viewt, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    p.marcher = Some(m);
    if shape.hzb {
        let h = g.add_pass("hzb_build_0");
        g.image_access(ids.depth, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH);
        p.hzb = Some(h);
    }
    if shape.merge {
        let mg = g.add_pass("sdf_depth_merge");
        depth_attachment(
            &mut g,
            ids.depth,
            VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        );
        g.image_access(ids.viewt, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
        p.merge = Some(mg);
    }
    if let Some((name, sampled)) = shape.consumer {
        let c = g.add_pass(name);
        if sampled {
            g.image_access(ids.depth, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH);
        } else {
            depth_attachment(&mut g, ids.depth, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT);
        }
        colour(&mut g, ids.lit, VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT);
        p.consumer = Some(c);
    }
    if shape.dump {
        let d = g.add_pass("image_dump");
        g.image_access(ids.depth, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, SubRange::DEPTH);
        p.dump = Some(d);
    }
    g.compile();
    (g, ids, p)
}

fn depth_attachment(g: &mut FrameGraph, depth: ResId, access: u32) {
    g.image_access(depth, FRAG_TESTS, access, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::DEPTH);
}

fn colour(g: &mut FrameGraph, lit: ResId, access: u32) {
    g.image_access(lit, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, access, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR);
}

/// The full ForwardPlus-shaped frame host test 4 names: prepass + EQUAL opaque before the
/// marcher, an HZB compute read, the merge, `particle_draw`, the image dump.
fn full_forward_plus(merge: bool, consumer: &'static str) -> (FrameGraph, Ids, Passes) {
    frame(&Shape {
        forward_plus: true,
        first_touch_merge: false,
        hzb: true,
        merge,
        consumer: Some((consumer, false)),
        dump: true,
    })
}

#[test]
fn only_the_transparent_consumer_is_classified() {
    let (g, ids, p) = full_forward_plus(true, "particle_draw");
    let consumer = p.consumer.expect("declared");
    assert!(is_transparent_depth_consumer(&g, consumer, ids.depth), "particle_draw is a consumer");
    // The merge reads AND writes the depth: never a consumer of its own output.
    assert!(!is_transparent_depth_consumer(&g, p.merge.expect("declared"), ids.depth), "the merge classified itself");
    // Compute and transfer readers are not depth-tested.
    assert!(!is_transparent_depth_consumer(&g, p.hzb.expect("declared"), ids.depth), "an HZB compute read");
    assert!(!is_transparent_depth_consumer(&g, p.dump.expect("declared"), ids.depth), "a transfer dump");
    assert!(!is_transparent_depth_consumer(&g, p.marcher.expect("declared"), ids.depth), "the marcher");
    // ForwardPlus's EQUAL opaque IS attachment-read-only — excluded by POSITION (before the
    // marcher), which is exactly why the guard needs the producer handle.
    assert!(is_transparent_depth_consumer(&g, p.forward_opaque.expect("declared"), ids.depth));
    // A frame with the merge in place is in order.
    assert_eq!(transparent_depth_order_violation(&g, ids.depth, p.marcher, p.merge, false, false), None);
}

#[test]
fn a_consumer_without_the_merge_is_a_violation_naming_it() {
    let (g, ids, p) = full_forward_plus(false, "particle_draw");
    let v = transparent_depth_order_violation(&g, ids.depth, p.marcher, None, false, false);
    assert_eq!(v, p.consumer, "the order guard must name particle_draw");
    assert_eq!(g.pass_name(v.expect("checked above")), "particle_draw");
}

#[test]
fn a_consumer_declared_before_the_merge_is_a_violation() {
    // The merge exists but comes AFTER the consumer: hand-build that order.
    let mut g = FrameGraph::with_capacity(8, 16, 64);
    let depth = g.add_image("path_depth");
    g.add_pass("raster");
    depth_attachment(&mut g, depth, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
    let m = g.add_pass("sdf_forward_march");
    g.image_access(depth, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH);
    let c = g.add_pass("particle_draw");
    depth_attachment(&mut g, depth, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT);
    let mg = g.add_pass("sdf_depth_merge");
    depth_attachment(&mut g, depth, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
    g.compile();
    assert_eq!(transparent_depth_order_violation(&g, depth, Some(m), Some(mg), false, false), Some(c));
}

#[test]
fn the_hzb_dump_frame_and_the_disarm_key_are_exempt_by_name() {
    let (g, ids, p) = full_forward_plus(false, "particle_draw");
    assert_eq!(transparent_depth_order_violation(&g, ids.depth, p.marcher, None, true, false), None);
    assert_eq!(transparent_depth_order_violation(&g, ids.depth, p.marcher, None, false, true), None);
}

#[test]
fn a_frame_without_an_sdf_producer_is_never_a_violation() {
    let (g, ids, _) = full_forward_plus(false, "particle_draw");
    assert_eq!(transparent_depth_order_violation(&g, ids.depth, None, None, false, false), None);
}

#[test]
fn a_later_rungs_consumer_is_classified_with_no_registration() {
    // R1's translucent draw: an attachment read under a name this module never mentions.
    let (g, ids, p) = full_forward_plus(false, "translucent_draw");
    let v = transparent_depth_order_violation(&g, ids.depth, p.marcher, None, false, false);
    assert_eq!(v.map(|p| g.pass_name(p)), Some("translucent_draw"));
    // A sampled (soft-particle style) FRAGMENT read is a consumer too.
    let (g, ids, p) = frame(&Shape {
        forward_plus: false,
        first_touch_merge: false,
        hzb: false,
        merge: false,
        consumer: Some(("soft_particles", true)),
        dump: false,
    });
    let v = transparent_depth_order_violation(&g, ids.depth, p.marcher, None, false, false);
    assert_eq!(v.map(|p| g.pass_name(p)), Some("soft_particles"));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "pass 'particle_draw' tests the path depth after the frame's SDF producer")]
fn the_debug_order_check_panics_naming_the_pass() {
    use boyko_rhi_vulkan::present::transparent_depth::debug_check_transparent_depth_order;
    let (g, ids, p) = full_forward_plus(false, "particle_draw");
    debug_check_transparent_depth_order(&g, ids.depth, p.marcher, None, false, false);
}

#[cfg(debug_assertions)]
#[test]
fn the_debug_order_check_is_silent_on_an_exempt_or_ordered_frame() {
    use boyko_rhi_vulkan::present::transparent_depth::debug_check_transparent_depth_order;
    let (g, ids, p) = full_forward_plus(false, "particle_draw");
    debug_check_transparent_depth_order(&g, ids.depth, p.marcher, None, true, false);
    debug_check_transparent_depth_order(&g, ids.depth, p.marcher, None, false, true);
    let (g, ids, p) = full_forward_plus(true, "particle_draw");
    debug_check_transparent_depth_order(&g, ids.depth, p.marcher, p.merge, false, false);
}

// ---- Pairing ---------------------------------------------------------------------------------------

const P_BIT: u8 = 1 << TRANSPARENT_CLASS_PARTICLES;

fn particles_frame() -> TransparentDepthFrame {
    TransparentDepthFrame::within(TransparentDepthCapability::PARTICLES, P_BIT)
}

#[test]
fn pairing_holds_when_the_consumer_follows_its_frame_bit() {
    let c = TRANSPARENT_CLASS_PARTICLES;
    assert_eq!(transparent_depth_pairing_violation(particles_frame(), &[(c, true)]), None);
    assert_eq!(transparent_depth_pairing_violation(TransparentDepthFrame::NONE, &[(c, false)]), None);
}

#[test]
fn pairing_fails_in_both_directions_and_on_a_missing_row() {
    let c = TRANSPARENT_CLASS_PARTICLES;
    // The particles bit is set and no particle_draw is declared.
    assert_eq!(transparent_depth_pairing_violation(particles_frame(), &[(c, false)]), Some(c));
    // The reverse: particle_draw declared with the bit clear.
    assert_eq!(transparent_depth_pairing_violation(TransparentDepthFrame::NONE, &[(c, true)]), Some(c));
    // A frame bit the declarator has no row for.
    assert_eq!(transparent_depth_pairing_violation(particles_frame(), &[]), Some(c));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "transparent-depth pairing: consumer class 0")]
fn the_debug_pairing_check_panics_on_a_frame_bit_without_its_consumer() {
    use boyko_rhi_vulkan::present::transparent_depth::debug_check_transparent_depth_pairing;
    debug_check_transparent_depth_pairing(particles_frame(), &[(TRANSPARENT_CLASS_PARTICLES, false)]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "transparent-depth pairing: consumer class 0")]
fn the_debug_pairing_check_panics_on_a_consumer_without_its_frame_bit() {
    use boyko_rhi_vulkan::present::transparent_depth::debug_check_transparent_depth_pairing;
    debug_check_transparent_depth_pairing(TransparentDepthFrame::NONE, &[(TRANSPARENT_CLASS_PARTICLES, true)]);
}

// ---- The merge's CLEAR tied to its derived first touch (critique W1) -------------------------------

#[test]
fn the_first_touch_merge_must_clear_and_a_later_merge_must_load() {
    // VB×Sdf: nothing writes the depth before the merge, so the graph derives UNDEFINED → … at it.
    let (g, ids, p) = frame(&Shape {
        forward_plus: false,
        first_touch_merge: true,
        hzb: false,
        merge: true,
        consumer: Some(("particle_draw", false)),
        dump: false,
    });
    let merge = p.merge.expect("declared");
    assert!(!merge_clear_violation(&g, merge, ids.depth, true), "a first-touch merge that clears is right");
    assert!(merge_clear_violation(&g, merge, ids.depth, false), "a first-touch merge that LOADS reads undefined memory");
    // Every other leg: the opaque raster wrote the depth first.
    let (g, ids, p) = full_forward_plus(true, "particle_draw");
    let merge = p.merge.expect("declared");
    assert!(!merge_clear_violation(&g, merge, ids.depth, false), "a merge after the opaque depth loads");
    assert!(merge_clear_violation(&g, merge, ids.depth, true), "a CLEAR after the opaque depth erases it");
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "sdf_depth_merge loads 'path_depth' on its first touch this frame")]
fn the_debug_clear_check_panics_on_a_load_of_a_first_touched_depth() {
    use boyko_rhi_vulkan::present::transparent_depth::debug_check_merge_clear;
    let (g, ids, p) = frame(&Shape {
        forward_plus: false,
        first_touch_merge: true,
        hzb: false,
        merge: true,
        consumer: Some(("particle_draw", false)),
        dump: false,
    });
    debug_check_merge_clear(&g, p.merge.expect("declared"), ids.depth, false);
}
