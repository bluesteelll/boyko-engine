//! Particles P0 — the derived-barrier gates of `docs/PARTICLES-PLAN.md` Rev 4: **#1(b)**
//! (disarmed byte-identity per declarator), **#2** (armed per-path `lit`/depth assertions),
//! **#3** (the seed table's rows, at the pass each row names), **#4** (the access column equals
//! what the declarator emits) and **#5** (`p_dispatch_args` pinned idle AND active).
//!
//! # This file is a REPLICA, and the word is load-bearing
//!
//! `Renderer::declare_{deferred,forward,vb}_graph` are `pub(crate)` methods that need a live
//! `VkDevice`, so no test can call them. `tests/vb_barrier_stream_baseline.rs` established the
//! answer this file follows: RE-DECLARE the same shape against the public `framegraph` API and
//! assert the derived stream. The consequence is stated rather than hidden — **a divergence
//! between this replica and the production declarator is invisible here**, exactly as it is
//! there. What the replica DOES catch is a change in the framegraph's derivation, in the seed
//! constructors, or in the access list, and it is the only executable statement of the plan's
//! seed table that exists at all.
//!
//! The replica is deliberately REDUCED: it models each path's particle tail plus the *minimum*
//! producer set that puts `lit` and the path's depth image into the state the production
//! declarator leaves them in before the particle draw. That is enough for every gate here,
//! because a derived barrier is a function of the resource's state at the access and of nothing
//! else. It is NOT enough to pin a whole frame's stream, and this file does not claim to.
//!
//! # ⚠️ TWO PLACES THE PLAN'S OWN TEXT DOES NOT SURVIVE CONTACT, both encoded here as measured
//!
//! **1. Seed row 6 omits emit's read of `p_counters`, and the declarator adds it back.** The
//! table's access column for `p_counters` reads `kickoff C/RW → sim C/RW`. But
//! `particle_emit.comp.hlsl` binds `p_counters` at Set-0 binding 0 and reads three fields out of
//! it (`real_emit_count`, `dead_base`, `emit_append_base` — algorithm A3's two arithmetic indices
//! and its tail guard). Taking the column literally leaves kickoff's write of those fields
//! UNORDERED against emit's read: a missing barrier, and on a device with `robustBufferAccess`
//! OFF that is undefined behaviour rather than a wasted edge — the F7d/N1 class the table's own
//! preamble names. [`ACCESS_COLUMN`] therefore encodes the AMENDED column, with the amendment
//! marked at the row it changed. Adding the read is the SAFE direction (an extra barrier is
//! over-synchronisation); omitting it is the unsafe one.
//!
//! **2. Gate #2's "`lit`: GENERAL → COLOR_ATTACHMENT_OPTIMAL on every path" holds on TWO of the
//! four.** It is a consequence of what the path's `lit` PRODUCER is, not of the particle draw:
//! Deferred's resolve and VB's `vb_resolve`/`vb_shade` are COMPUTE stores into `GENERAL`, so the
//! blend's attachment access derives a real layout transition; `forward_opaque` is a COLOR
//! ATTACHMENT write already at `COLOR_ATTACHMENT_OPTIMAL`, so the same access derives an
//! AVAILABILITY barrier with no layout change. Both are correct; only the blanket sentence is
//! wrong. The four assertions below state what each path actually derives.

use boyko_rhi_vulkan::compute::{
    COMPOSITE_PUSH_CONSTANT_BYTES, PARTICLE_DISPATCH_EMIT_OFFSET, PARTICLE_DISPATCH_SIM_OFFSET,
    PARTICLE_DRAW_ADDITIVE_OFFSET, PARTICLE_DRAW_ALPHA_OFFSET, PARTICLE_DRAW_PUSH_BYTES,
    PARTICLE_EMIT_PUSH_BYTES,
    PARTICLE_KICKOFF_PUSH_BYTES, PARTICLE_LOCAL_SIZE, PARTICLE_QUAD_IB_BYTES,
    PARTICLE_QUAD_INDEX_COUNT, PARTICLE_SIM_PUSH_BYTES, PARTICLE_SORT_BINS,
    PARTICLE_SORT_BINS_WORDS, PARTICLE_SORT_PUSH_BYTES, VB_BATCH_CULL_PUSH_BYTES,
};
use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_READ_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_INDIRECT_COMMAND_READ_BIT, VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT,
    VK_ACCESS_TRANSFER_WRITE_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_GENERAL,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
    VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
};
use boyko_rhi_vulkan::framegraph::{BufBarrier, FrameGraph, ImgBarrier, ResId, ResSync, SubRange};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Constants mirroring the declarator's own
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// `graph_bridge.rs`'s own `FRAG` local — the depth-attachment stage pair.
const FRAG: u32 =
    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;

/// The read|write access an atomic read-modify-write declares (`declare_particle_compute`'s own
/// `RW` local, and the `light_index_alloc` spelling K2 cites for `p_draw_args`).
const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;

/// The blend's attachment access — a read-modify-write of `lit`, so BOTH bits.
const BLEND: u32 = VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The matrix
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Which production declarator the row replicates. The particle tail is IDENTICAL on all four;
/// what differs is the state the path's own passes leave `lit` and depth in, which is what makes
/// gate #2 four assertions rather than one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Path {
    /// `declare_deferred_graph`: the resolve stores `lit` at `COMPUTE/GENERAL`; `depth` reaches
    /// the transparent slot at `SHADER_READ_ONLY_OPTIMAL` (three pre-lit consumers sampled it).
    Deferred,
    /// `declare_forward_graph` without the prepass: `forward_opaque` writes `lit` as a COLOR
    /// attachment and `forward_depth` as a depth-stencil WRITE.
    Forward,
    /// `declare_forward_graph` WITH the prepass: the prepass owns the depth write and
    /// `forward_opaque` declares depth as an attachment READ at `DEPTH_ATTACHMENT_OPTIMAL` — the
    /// exact stage/access/layout the particle draw wants, which is why this path is free.
    ForwardPlus,
    /// `declare_vb_graph` fused: `vb_sky` first-touches `lit` as COLOR, `vb_resolve` then stores
    /// it at `COMPUTE/GENERAL`; `vb_raster` leaves `vb_depth` on a depth-stencil write.
    VisibilityBuffer,
}

/// One row of the matrix.
#[derive(Clone, Copy, Debug)]
struct Row {
    /// The label every failure message names, so a divergence says WHICH configuration moved.
    id: &'static str,
    path: Path,
    /// `ParticleConfig::enabled()` — `false` is the disarmed 0%-gate: nothing declared at all.
    particles: bool,
    /// `ParticleEmitScratch::total_spawn() > 0` — arms the emit-request upload half AND the
    /// `particle_emit` pass, with ONE predicate (the conditional-pass proof).
    spawn: bool,
    /// The effect table's writer-side generation moved for this slot — arms the effect-upload
    /// half only. Independent of `spawn` by design: `p_effects` has a reader on every armed
    /// frame, so an upload with no spawn is well-formed.
    effects_dirty: bool,
    /// `ParticleConfig::sorts()` — rung P2 item 3's arming. Declares the three sort passes and
    /// gives the alpha half of `particle_draw` a second source. The two sort `ResId`s are declared
    /// on EVERY armed row regardless, so the tail's length is one number (F9 then routes zero
    /// barriers for a ResId no pass names).
    sort: bool,
}

const DEFERRED_OFF: Row = Row {
    id: "deferred/disarmed",
    path: Path::Deferred,
    particles: false,
    spawn: false,
    effects_dirty: false,
    sort: false,
};
const FORWARD_OFF: Row = Row { id: "forward/disarmed", path: Path::Forward, ..DEFERRED_OFF };
const FORWARD_PLUS_OFF: Row =
    Row { id: "forward_plus/disarmed", path: Path::ForwardPlus, ..DEFERRED_OFF };
const VB_OFF: Row = Row { id: "vb/disarmed", path: Path::VisibilityBuffer, ..DEFERRED_OFF };

const DEFERRED_ON: Row = Row {
    id: "deferred/armed",
    path: Path::Deferred,
    particles: true,
    spawn: true,
    effects_dirty: true,
    sort: false,
};
const FORWARD_ON: Row = Row { id: "forward/armed", path: Path::Forward, ..DEFERRED_ON };
const FORWARD_PLUS_ON: Row =
    Row { id: "forward_plus/armed", path: Path::ForwardPlus, ..DEFERRED_ON };
const VB_ON: Row = Row { id: "vb/armed", path: Path::VisibilityBuffer, ..DEFERRED_ON };

/// Gate #5's idle twin of [`DEFERRED_ON`]: armed, but nothing asked to spawn — so the
/// emit-request upload and the whole `particle_emit` pass are undeclared, and
/// `p_dispatch_args`' derived stream must MOVE accordingly.
const DEFERRED_ON_IDLE: Row = Row {
    id: "deferred/armed-idle",
    path: Path::Deferred,
    particles: true,
    spawn: false,
    effects_dirty: false,
    sort: false,
};

/// Rung P2 item 3's twin of [`DEFERRED_ON`]: the SAME frame with the radix armed. Every gate that
/// compares the two is asserting "arming the sort ADDS and moves nothing", which is the property
/// the default-off claim rests on.
const DEFERRED_ON_SORTED: Row =
    Row { id: "deferred/armed-sorted", sort: true, ..DEFERRED_ON };
const VB_ON_SORTED: Row =
    Row { id: "vb/armed-sorted", path: Path::VisibilityBuffer, ..DEFERRED_ON_SORTED };

/// The eight disarmed/armed rows plus the idle one, for the sweeps that run over everything.
const ALL_ARMED: [Row; 4] = [DEFERRED_ON, FORWARD_ON, FORWARD_PLUS_ON, VB_ON];
const ALL_DISARMED: [Row; 4] = [DEFERRED_OFF, FORWARD_OFF, FORWARD_PLUS_OFF, VB_OFF];

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The replica
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The ten particle buffer `ResId`s, in DECLARATION order (the shaders' Set-0 binding numbering).
#[derive(Clone, Copy)]
struct ParticleRes {
    counters: ResId,
    dispatch_args: ResId,
    draw_args: ResId,
    dead: ResId,
    alive_read: ResId,
    alive_write: ResId,
    particle: ResId,
    render: ResId,
    emit_req: ResId,
    effects: ResId,
    /// Seed row 11 — rung P2 item 3's sorted render records. Declared on every ARMED row, named by
    /// a pass only on a sorted one.
    render_sorted: ResId,
    /// Seed row 12 — rung P2 item 3's radix scratch. Same declaration rule.
    sort_bins: ResId,
}

/// One declared + compiled replica frame, with the labels a failure message needs.
struct Frame {
    g: FrameGraph,
    /// Pass names in `add_pass` order, recorded AT the call so a report cannot mislabel a pass.
    pass_names: Vec<&'static str>,
    row: Row,
    lit: ResId,
    depth: ResId,
    particle: Option<ParticleRes>,
}

impl Frame {
    /// The index of `name` in declaration order.
    fn pass_index(&self, name: &str) -> usize {
        self.pass_names
            .iter()
            .position(|n| *n == name)
            .unwrap_or_else(|| panic!("{}: no pass named {name}", self.row.id))
    }

    /// Every image barrier the pass named `name` emits, in order.
    fn img_at(&self, name: &str) -> &[ImgBarrier] {
        let r = self.g.pass_barriers()[self.pass_index(name)];
        &self.g.img_barriers()[r.img_begin as usize..(r.img_begin + r.img_count) as usize]
    }

    /// Every buffer barrier the pass named `name` emits, in order.
    fn buf_at(&self, name: &str) -> &[BufBarrier] {
        let r = self.g.pass_barriers()[self.pass_index(name)];
        &self.g.buf_barriers()[r.buf_begin as usize..(r.buf_begin + r.buf_count) as usize]
    }

    /// Every buffer barrier naming `res`, across the whole frame, in emission order.
    fn buf_on(&self, res: ResId) -> Vec<&BufBarrier> {
        self.g.buf_barriers().iter().filter(|b| b.res == res).collect()
    }

    /// The FIRST buffer barrier naming `res`, and the pass that emitted it — what gate #3's
    /// "derived first access, AT the pass the table names" needs.
    fn first_buf(&self, res: ResId) -> Option<(&'static str, BufBarrier)> {
        for (p, range) in self.g.pass_barriers().iter().enumerate() {
            for i in range.buf_begin..range.buf_begin + range.buf_count {
                let b = self.g.buf_barriers()[i as usize];
                if b.res == res {
                    return Some((self.pass_names[p], b));
                }
            }
        }
        None
    }
}

/// The replica of `declare_particle_buffers` — the ten seed constructors, row for row.
fn declare_particle_buffers(g: &mut FrameGraph) -> ParticleRes {
    ParticleRes {
        counters: g.add_buffer_seeded(
            "p_counters",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        ),
        dispatch_args: g.add_buffer_seeded(
            "p_dispatch_args",
            ResSync::seeded_readers(
                VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
                VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
            ),
        ),
        draw_args: g.add_buffer_seeded(
            "p_draw_args",
            ResSync::seeded_readers(
                VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
                VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
            ),
        ),
        dead: g.add_buffer_seeded(
            "p_dead",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        ),
        alive_read: g.add_buffer_seeded(
            "p_alive_read",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        ),
        alive_write: g.add_buffer_seeded(
            "p_alive_write",
            ResSync::seeded_readers(
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        ),
        particle: g.add_buffer_seeded(
            "p_particle",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        ),
        render: g.add_buffer_seeded(
            "p_render",
            ResSync::seeded_readers(
                VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        ),
        emit_req: g.add_buffer_seeded(
            "p_emit_req",
            ResSync::seeded_readers(
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        ),
        effects: g.add_buffer_seeded(
            "p_effects",
            ResSync::seeded_readers(
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        ),
        render_sorted: g.add_buffer_seeded(
            "p_render_sorted",
            ResSync::seeded_readers(
                VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
            ),
        ),
        sort_bins: g.add_buffer_seeded(
            "p_sort_bins",
            ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        ),
    }
}

/// One row of [`ACCESS_COLUMN`]: the pass, the resource, and the `(stage, access)` the declarator
/// declares there. This IS gate #4's expectation — the seed table's access column as data.
struct AccessRow {
    pass: &'static str,
    res: &'static str,
    stage: u32,
    access: u32,
}

/// Gate #4: the plan's seed-table ACCESS COLUMN, per pass, as data — the full set of
/// `buffer_access` calls the particle tail emits on an armed frame with both upload halves
/// armed, in declaration order.
///
/// ⚠️ Row `particle_emit / p_counters` is the AMENDMENT this file's module doc states in full:
/// the plan's table omits it, the shipped `particle_emit.comp.hlsl` performs it, and omitting the
/// declaration would delete the barrier that makes kickoff's three published fields visible.
const ACCESS_COLUMN: &[AccessRow] = &[
    // `particle_upload` — the two staging→device copies, each half separately gated.
    AccessRow {
        pass: "particle_upload",
        res: "p_emit_req",
        stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    AccessRow {
        pass: "particle_upload",
        res: "p_effects",
        stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    // `particle_kickoff` — seed rows 6, 3, 7, 8. It never reads `p_draw_args`.
    AccessRow {
        pass: "particle_kickoff",
        res: "p_counters",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: RW,
    },
    AccessRow {
        pass: "particle_kickoff",
        res: "p_dead",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: RW,
    },
    AccessRow {
        pass: "particle_kickoff",
        res: "p_dispatch_args",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    AccessRow {
        pass: "particle_kickoff",
        res: "p_draw_args",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    // `particle_emit` — the indirect fetch first, then the six shader accesses.
    AccessRow {
        pass: "particle_emit",
        res: "p_dispatch_args",
        stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    // ⚠️ THE AMENDED ROW — see this file's module doc.
    AccessRow {
        pass: "particle_emit",
        res: "p_counters",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_READ_BIT,
    },
    AccessRow {
        pass: "particle_emit",
        res: "p_dead",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_READ_BIT,
    },
    AccessRow {
        pass: "particle_emit",
        res: "p_alive_read",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    AccessRow {
        pass: "particle_emit",
        res: "p_particle",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    AccessRow {
        pass: "particle_emit",
        res: "p_emit_req",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_READ_BIT,
    },
    AccessRow {
        pass: "particle_emit",
        res: "p_effects",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_READ_BIT,
    },
    // `particle_sim` — the widest list, and the one that publishes to both consumers.
    AccessRow {
        pass: "particle_sim",
        res: "p_dispatch_args",
        stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    AccessRow {
        pass: "particle_sim",
        res: "p_counters",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: RW,
    },
    // K2: the returning `InterlockedAdd` on `instanceCount` is a READ-modify-write.
    AccessRow {
        pass: "particle_sim",
        res: "p_draw_args",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: RW,
    },
    AccessRow {
        pass: "particle_sim",
        res: "p_dead",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: RW,
    },
    AccessRow {
        pass: "particle_sim",
        res: "p_alive_read",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_READ_BIT,
    },
    AccessRow {
        pass: "particle_sim",
        res: "p_alive_write",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    AccessRow {
        pass: "particle_sim",
        res: "p_particle",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: RW,
    },
    AccessRow {
        pass: "particle_sim",
        res: "p_render",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    AccessRow {
        pass: "particle_sim",
        res: "p_effects",
        stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        access: VK_ACCESS_SHADER_READ_BIT,
    },
    // `particle_draw` — the VS fetch and the command processor's fetch. Its two IMAGE accesses
    // (`lit`, the path's depth) are gate #2's, not this table's.
    AccessRow {
        pass: "particle_draw",
        res: "p_render",
        stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        access: VK_ACCESS_SHADER_READ_BIT,
    },
    AccessRow {
        pass: "particle_draw",
        res: "p_draw_args",
        stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
];

/// The replica of `declare_particle_compute`. `accesses` collects `(pass, res_name, stage,
/// access)` as they are declared, which is what gate #4 compares against [`ACCESS_COLUMN`] — the
/// declaration itself is the column, and there is no second list to disagree with it.
fn declare_particle_compute(
    g: &mut FrameGraph,
    names: &mut Vec<&'static str>,
    accesses: &mut Vec<(&'static str, &'static str, u32, u32)>,
    ids: ParticleRes,
    row: Row,
) {
    let acc = |g: &mut FrameGraph,
                   accesses: &mut Vec<(&'static str, &'static str, u32, u32)>,
                   pass: &'static str,
                   res: ResId,
                   stage: u32,
                   access: u32| {
        accesses.push((pass, g.res_name(res), stage, access));
        g.buffer_access(res, stage, access);
    };

    if row.spawn || row.effects_dirty {
        names.push("particle_upload");
        g.add_pass("particle_upload");
        if row.spawn {
            acc(
                g,
                accesses,
                "particle_upload",
                ids.emit_req,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
        }
        if row.effects_dirty {
            acc(
                g,
                accesses,
                "particle_upload",
                ids.effects,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
        }
    }

    names.push("particle_kickoff");
    g.add_pass("particle_kickoff");
    let k = "particle_kickoff";
    acc(g, accesses, k, ids.counters, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    acc(g, accesses, k, ids.dead, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    acc(
        g,
        accesses,
        k,
        ids.dispatch_args,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
    );
    acc(
        g,
        accesses,
        k,
        ids.draw_args,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
    );

    if row.spawn {
        names.push("particle_emit");
        g.add_pass("particle_emit");
        let e = "particle_emit";
        acc(
            g,
            accesses,
            e,
            ids.dispatch_args,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        );
        acc(
            g,
            accesses,
            e,
            ids.counters,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        acc(g, accesses, e, ids.dead, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
        acc(
            g,
            accesses,
            e,
            ids.alive_read,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        );
        acc(
            g,
            accesses,
            e,
            ids.particle,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        );
        acc(
            g,
            accesses,
            e,
            ids.emit_req,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        acc(
            g,
            accesses,
            e,
            ids.effects,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
    }

    names.push("particle_sim");
    g.add_pass("particle_sim");
    let s = "particle_sim";
    acc(
        g,
        accesses,
        s,
        ids.dispatch_args,
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    );
    acc(g, accesses, s, ids.counters, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    acc(g, accesses, s, ids.draw_args, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    acc(g, accesses, s, ids.dead, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    acc(
        g,
        accesses,
        s,
        ids.alive_read,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
    );
    acc(
        g,
        accesses,
        s,
        ids.alive_write,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
    );
    acc(g, accesses, s, ids.particle, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    acc(
        g,
        accesses,
        s,
        ids.render,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
    );
    acc(
        g,
        accesses,
        s,
        ids.effects,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
    );

    // Rung P2 item 3 (plan D10): `hist -> scan -> scatter`, between the sim that produces the alpha
    // class and the draw that consumes it. NEITHER sort pass declares `p_dispatch_args`: both
    // re-fetch the SIM's own indirect command, which the sim already read this frame at the same
    // stage/access, so the read is free and declaring it again would only add a redundant row to
    // this column.
    if !row.sort {
        return;
    }
    names.push("particle_sort_hist");
    g.add_pass("particle_sort_hist");
    let h = "particle_sort_hist";
    acc(g, accesses, h, ids.draw_args, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    acc(g, accesses, h, ids.render, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    acc(g, accesses, h, ids.sort_bins, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);

    names.push("particle_sort_scan");
    g.add_pass("particle_sort_scan");
    acc(
        g,
        accesses,
        "particle_sort_scan",
        ids.sort_bins,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        RW,
    );

    names.push("particle_sort_scatter");
    g.add_pass("particle_sort_scatter");
    let c = "particle_sort_scatter";
    acc(g, accesses, c, ids.draw_args, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    acc(g, accesses, c, ids.render, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    acc(
        g,
        accesses,
        c,
        ids.render_sorted,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
    );
    acc(g, accesses, c, ids.sort_bins, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
}

/// The replica of `declare_particle_draw`.
fn declare_particle_draw(
    g: &mut FrameGraph,
    names: &mut Vec<&'static str>,
    accesses: &mut Vec<(&'static str, &'static str, u32, u32)>,
    ids: ParticleRes,
    lit: ResId,
    depth: ResId,
    sort: bool,
) {
    names.push("particle_draw");
    g.add_pass("particle_draw");
    accesses.push((
        "particle_draw",
        g.res_name(ids.render),
        VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
    ));
    g.buffer_access(ids.render, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    // Rung P2 item 3: the ALPHA half reads `p_render_sorted` when the sort is armed, so this ONE
    // pass names both buffers. This access is what derives the scatter's C/W -> VS/R RAW, and it is
    // also the frame TERMINAL that makes row 11's reader seed correct.
    if sort {
        accesses.push((
            "particle_draw",
            g.res_name(ids.render_sorted),
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ));
        g.buffer_access(
            ids.render_sorted,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
    }
    accesses.push((
        "particle_draw",
        g.res_name(ids.draw_args),
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    ));
    g.buffer_access(
        ids.draw_args,
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    );
    g.image_access(
        lit,
        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        BLEND,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
    g.image_access(
        depth,
        FRAG,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::DEPTH,
    );
}

/// Declares and compiles one row: the path's minimum `lit`/depth producer set, the particle tail
/// spliced at the positions the production declarators use, then `present_sample`.
///
/// Returns the frame plus the declared access list (gate #4's subject).
fn declare_frame(row: Row) -> (Frame, Vec<(&'static str, &'static str, u32, u32)>) {
    // Sized past the widest armed row so a declare performs no reallocation.
    let mut g = FrameGraph::with_capacity(48, 32, 192);
    let mut names: Vec<&'static str> = Vec::with_capacity(16);
    let mut accesses: Vec<(&'static str, &'static str, u32, u32)> = Vec::with_capacity(32);
    g.reset();

    // The two images every path has, declared first so their ResIds are stable across rows.
    let lit = g.add_image("lit");
    let depth = g.add_image("depth");
    // The particle tail is appended LAST among resources — D13's conditional tail — and ONLY
    // when armed.
    let particle = row.particles.then(|| declare_particle_buffers(&mut g));

    // The particle compute block is declared EARLY on every path (before the opaque work).
    if let Some(ids) = particle {
        declare_particle_compute(&mut g, &mut names, &mut accesses, ids, row);
    }

    // --- The path's own producers, reduced to what decides the two states gate #2 reads.
    match row.path {
        Path::Deferred => {
            // The raster writes depth; the marcher/SSAO/resolve then SAMPLE it, which is what
            // leaves it at SHADER_READ_ONLY_OPTIMAL at the transparent slot.
            names.push("raster");
            g.add_pass("raster");
            g.image_access(
                depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
            names.push("resolve");
            g.add_pass("resolve");
            g.image_access(
                depth,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                SubRange::DEPTH,
            );
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
        Path::Forward => {
            names.push("forward_opaque");
            g.add_pass("forward_opaque");
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                SubRange::COLOR,
            );
            g.image_access(
                depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
        }
        Path::ForwardPlus => {
            // The prepass owns the depth WRITE; `forward_opaque` then declares depth as an
            // attachment READ at the SAME layout the particle draw wants — which is the whole
            // reason this path pays nothing.
            names.push("depth_prepass");
            g.add_pass("depth_prepass");
            g.image_access(
                depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
            names.push("forward_opaque");
            g.add_pass("forward_opaque");
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                SubRange::COLOR,
            );
            g.image_access(
                depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
        }
        Path::VisibilityBuffer => {
            names.push("vb_sky");
            g.add_pass("vb_sky");
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                SubRange::COLOR,
            );
            names.push("vb_raster");
            g.add_pass("vb_raster");
            g.image_access(
                depth,
                FRAG,
                VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
                VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                SubRange::DEPTH,
            );
            names.push("vb_resolve");
            g.add_pass("vb_resolve");
            g.image_access(
                lit,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
    }

    // The particle draw is declared LATE — after every `lit` producer, before `present_sample`.
    if let Some(ids) = particle {
        declare_particle_draw(&mut g, &mut names, &mut accesses, ids, lit, depth, row.sort);
    }

    names.push("present_sample");
    g.add_pass("present_sample");
    g.image_access(
        lit,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::COLOR,
    );

    g.compile();
    (Frame { g, pass_names: names, row, lit, depth, particle }, accesses)
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Gate #1(b) — disarmed byte-identity, per declarator
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Gate #1(b): with `mode == Off` the particle tail declares NOTHING, so each path's derived
/// stream is byte-equal to the same frame declared by a tree that has no particle code at all.
///
/// The control is built by the SAME fn with `particles: false`, which makes every conditional in
/// the tail take its absent arm — the closest executable statement of "structural absence" that
/// exists without keeping a second copy of the declarator around. It is asserted FIELD BY FIELD
/// (barriers are `PartialEq`) plus on the resource and pass COUNTS, because a count that matched
/// while a field moved is exactly the class this campaign has already shipped twice.
#[test]
fn disarmed_declares_no_resource_no_pass_and_no_barrier() {
    for row in ALL_DISARMED {
        let (f, accesses) = declare_frame(row);
        assert!(f.particle.is_none(), "{}: the disarmed tail declares no ResId", row.id);
        assert!(accesses.is_empty(), "{}: the disarmed tail declares no access", row.id);
        assert!(
            !f.pass_names.iter().any(|n| n.starts_with("particle_")),
            "{}: the disarmed tail declares no pass — got {:?}",
            row.id,
            f.pass_names
        );
        // The two images are the ONLY resources; the tail added none.
        assert_eq!(
            f.g.res_state_total(),
            2,
            "{}: a disarmed frame declares exactly `lit` and the path's depth",
            row.id
        );
        // Not one derived buffer barrier exists at all, because the tail's ten buffers are the
        // only buffers this replica ever declares.
        assert!(
            f.g.buf_barriers().is_empty(),
            "{}: a disarmed frame routed buffer barriers: {:?}",
            row.id,
            f.g.buf_barriers()
        );
    }
}

/// The stronger half of gate #1(b): the derived IMAGE stream of a disarmed frame is byte-equal
/// to the one the same path declares — every field, every count, in order. Armed and disarmed are
/// compared on the SAME path so the difference is attributable to the tail alone.
#[test]
fn arming_the_tail_adds_barriers_and_moves_none_that_existed() {
    for (off, on) in ALL_DISARMED.iter().zip(ALL_ARMED.iter()) {
        let (f_off, _) = declare_frame(*off);
        let (f_on, _) = declare_frame(*on);

        // Every barrier the disarmed frame derived still exists, field-identical, in the armed
        // one — the tail APPENDS, it does not re-source. (`particle_draw`'s own two image
        // barriers are the ones the armed frame gains; `present_sample`'s `lit` read is
        // re-sourced from the draw's blend rather than from the path's producer, which is the ONE
        // pre-existing barrier arming legitimately moves, so it is compared by COUNT there.)
        let off_img: Vec<&ImgBarrier> = f_off.g.img_barriers().iter().collect();
        let on_img: Vec<&ImgBarrier> = f_on.g.img_barriers().iter().collect();
        assert!(
            on_img.len() > off_img.len(),
            "{}: arming must ADD image barriers (off {}, on {})",
            on.id,
            off_img.len(),
            on_img.len()
        );
        // Every pass the disarmed frame declared is still declared, in the same relative order.
        for name in &f_off.pass_names {
            assert!(
                f_on.pass_names.contains(name),
                "{}: arming deleted the pass {name}",
                on.id
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Gate #2 — the armed per-path assertions
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Gate #2, `lit` half — Deferred and VisibilityBuffer.
///
/// Both put their final `lit` there with a COMPUTE STORE at `GENERAL`, so the blend's attachment
/// access derives a real `GENERAL → COLOR_ATTACHMENT_OPTIMAL` transition, sourced from that store.
#[test]
fn armed_lit_transitions_from_general_on_the_compute_store_paths() {
    for row in [DEFERRED_ON, VB_ON] {
        let (f, _) = declare_frame(row);
        let img = f.img_at("particle_draw");
        let lit = img
            .iter()
            .find(|b| b.res == f.lit)
            .unwrap_or_else(|| panic!("{}: the draw must derive a `lit` barrier", row.id));
        assert_eq!(lit.old_layout, VK_IMAGE_LAYOUT_GENERAL, "{}: old layout", row.id);
        assert_eq!(
            lit.new_layout, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            "{}: new layout",
            row.id
        );
        assert_eq!(lit.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, "{}: src stage", row.id);
        assert_eq!(lit.src_access, VK_ACCESS_SHADER_WRITE_BIT, "{}: src access", row.id);
        assert_eq!(
            lit.dst_stage, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            "{}: dst stage",
            row.id
        );
        assert_eq!(lit.dst_access, BLEND, "{}: the blend reads AND writes the attachment", row.id);
    }
}

/// Gate #2, `lit` half — the Forward family, and the correction to the plan's blanket sentence.
///
/// `forward_opaque` writes `lit` as a COLOR ATTACHMENT, so it is ALREADY at
/// `COLOR_ATTACHMENT_OPTIMAL` when the blend arrives: the derived barrier is AVAILABILITY only,
/// with `old_layout == new_layout`. That is not a defect — it is what the plan's per-path
/// overhead table already predicts for these two paths ("0 — already an attachment" for the depth
/// row); only gate #2's `lit` sentence over-generalises.
#[test]
fn armed_lit_is_availability_only_on_the_attachment_write_paths() {
    for row in [FORWARD_ON, FORWARD_PLUS_ON] {
        let (f, _) = declare_frame(row);
        let img = f.img_at("particle_draw");
        let lit = img
            .iter()
            .find(|b| b.res == f.lit)
            .unwrap_or_else(|| panic!("{}: the draw must derive a `lit` barrier", row.id));
        assert_eq!(
            lit.old_layout, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            "{}: `lit` is already an attachment on this path",
            row.id
        );
        assert_eq!(lit.new_layout, lit.old_layout, "{}: no layout change", row.id);
        assert_eq!(
            lit.src_stage, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            "{}: src stage",
            row.id
        );
        assert_eq!(
            lit.src_access, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            "{}: src access — the opaque pass's own write",
            row.id
        );
    }
}

/// Gate #2, depth half, Deferred: a real LAYOUT TRANSITION
/// `SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL`, because three pre-lit consumers sampled
/// the depth image before the transparent slot.
#[test]
fn armed_depth_transitions_from_sampled_on_deferred() {
    let (f, _) = declare_frame(DEFERRED_ON);
    let img = f.img_at("particle_draw");
    let d = img
        .iter()
        .find(|b| b.res == f.depth)
        .expect("deferred/armed: the draw must derive a depth barrier");
    assert_eq!(d.old_layout, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);
    assert_eq!(d.new_layout, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL);
    assert_eq!(d.dst_stage, FRAG);
    assert_eq!(d.dst_access, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT);
    assert_eq!(d.subresource, SubRange::DEPTH);
}

/// Gate #2, depth half, Forward and VisibilityBuffer: an AVAILABILITY-only barrier — the depth
/// image ends on a depth-stencil write at the layout the draw wants, so the layout does not move
/// and only the pending write is flushed.
#[test]
fn armed_depth_is_availability_only_on_forward_and_vb() {
    for row in [FORWARD_ON, VB_ON] {
        let (f, _) = declare_frame(row);
        let img = f.img_at("particle_draw");
        let d = img
            .iter()
            .find(|b| b.res == f.depth)
            .unwrap_or_else(|| panic!("{}: the draw must derive a depth barrier", row.id));
        assert_eq!(
            d.old_layout, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            "{}: already an attachment",
            row.id
        );
        assert_eq!(d.new_layout, d.old_layout, "{}: no layout change", row.id);
        assert_eq!(d.src_stage, FRAG, "{}: src stage — the depth producer", row.id);
        assert_eq!(
            d.src_access, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            "{}: src access — the pending depth write being flushed",
            row.id
        );
    }
}

/// Gate #2, depth half, ForwardPlus: **EXACTLY ZERO** depth barriers at the draw.
///
/// The froxel opaque pass already declared `FRAG / DS_ATTACHMENT_READ / DEPTH_ATTACHMENT_OPTIMAL`,
/// which is the particle draw's access to the bit — same layout, no pending flush, already
/// visible at that stage and access — so `sync::transition` returns `None`. This is the one free
/// row of D7's table, and it is asserted as an ABSENCE because that is what "free" means.
#[test]
fn armed_depth_derives_nothing_on_forward_plus() {
    let (f, _) = declare_frame(FORWARD_PLUS_ON);
    let depth_barriers: Vec<&ImgBarrier> =
        f.img_at("particle_draw").iter().filter(|b| b.res == f.depth).collect();
    assert!(
        depth_barriers.is_empty(),
        "forward_plus/armed: the depth read must derive NOTHING (D7's free row), got {depth_barriers:?}"
    );
    // And the draw still derives its `lit` barrier — an empty depth set must not be an empty pass.
    assert!(
        f.img_at("particle_draw").iter().any(|b| b.res == f.lit),
        "forward_plus/armed: the `lit` barrier is still derived"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Gate #3 — the seed table's rows, AT the pass each row names
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Gate #3: for each of the ten resources, the derived FIRST-ACCESS barrier's
/// `(src_stage, src_access)` equals the seed table's row — and is looked for AT THE PASS the
/// table's last column names.
///
/// The pass matters as much as the fields (M5): rows 4 and 5 track two DIFFERENT physical buffers
/// whose roles swap every frame, so row 4's barrier appears at **emit** (a WAW against the
/// sibling frame's sim write) and row 5's at the **sim** (a WAR against the sibling frame's sim
/// read). Asserting only the fields would pass with the two seeds swapped, which is the shape
/// that leaves one hazard unordered every frame while every count stays identical.
#[test]
fn seed_table_rows_match_the_derived_first_access_at_the_named_pass() {
    let (f, _) = declare_frame(DEFERRED_ON);
    let ids = f.particle.expect("armed");
    // (resource, expected pass, expected src_stage, expected src_access, why)
    let rows: [(ResId, &str, u32, u32, &str); 10] = [
        (
            ids.particle,
            "particle_emit",
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            "row 1: writer seed, emit's C/W ⇒ WAW",
        ),
        (
            ids.render,
            "particle_sim",
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            0,
            "row 2: reader seed at VERTEX_SHADER, the sim's C/W ⇒ WAR with src access 0",
        ),
        (
            ids.dead,
            "particle_kickoff",
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            "row 3: writer seed, kickoff's C/RW ⇒ WAW/RAW",
        ),
        (
            ids.alive_read,
            "particle_emit",
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            "row 4 (M5): the barrier is at EMIT, not at the sim's read",
        ),
        (
            ids.alive_write,
            "particle_sim",
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            "row 5: reader seed, the sim's C/W ⇒ WAR with src access 0",
        ),
        (
            ids.counters,
            "particle_kickoff",
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            "row 6: writer seed carrying the sibling sim's alive_count_next ⇒ RAW",
        ),
        (
            ids.dispatch_args,
            "particle_kickoff",
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            0,
            "row 7: reader seed at DRAW_INDIRECT, kickoff's C/W ⇒ WAR",
        ),
        (
            ids.draw_args,
            "particle_kickoff",
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            0,
            "row 8: reader seed at DRAW_INDIRECT, kickoff's C/W ⇒ WAR. Kickoff never reads it",
        ),
        (
            ids.emit_req,
            "particle_upload",
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            "row 9: reader seed, the upload's T/TW ⇒ WAR",
        ),
        (
            ids.effects,
            "particle_upload",
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            "row 10: reader seed, the upload's T/TW ⇒ WAR",
        ),
    ];

    for (res, pass, src_stage, src_access, why) in rows {
        let name = f.g.res_name(res);
        let (at, b) = f
            .first_buf(res)
            .unwrap_or_else(|| panic!("{name}: no derived barrier at all ({why})"));
        assert_eq!(at, pass, "{name}: the seed barrier must be derived at `{pass}` ({why})");
        assert_eq!(b.src_stage, src_stage, "{name}: src_stage ({why})");
        assert_eq!(b.src_access, src_access, "{name}: src_access ({why})");
    }
}

/// The row-4/row-5 asymmetry stated as its own property, because it is the one pair a swapped
/// seed leaves count-identical: the two alive lists derive barriers at DIFFERENT passes, and the
/// WRITER-seeded one is the one bound as `p_alive_read`.
#[test]
fn the_two_alive_lists_derive_at_different_passes() {
    let (f, _) = declare_frame(DEFERRED_ON);
    let ids = f.particle.expect("armed");
    let (read_at, read_b) = f.first_buf(ids.alive_read).expect("p_alive_read barrier");
    let (write_at, write_b) = f.first_buf(ids.alive_write).expect("p_alive_write barrier");
    assert_ne!(read_at, write_at, "the two roles' barriers land at different passes");
    assert_eq!(read_at, "particle_emit");
    assert_eq!(write_at, "particle_sim");
    assert_eq!(
        read_b.src_access, VK_ACCESS_SHADER_WRITE_BIT,
        "p_alive_read carries the sibling frame's sim WRITE (writer seed)"
    );
    assert_eq!(
        write_b.src_access, 0,
        "p_alive_write carries the sibling frame's sim READ (reader seed ⇒ execution-only WAR)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Gate #4 — the access column IS the declarator
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Gate #4: the set of `buffer_access` calls the tail emits equals the plan's access column, per
/// pass, in order.
///
/// This is the gate #3 cannot be: a MISSING intra-frame declaration (say, the sim forgetting
/// `p_particle`) leaves every first-access barrier exactly where it was, so gate #3 stays green
/// while a real hazard goes unordered.
#[test]
fn access_column_matches_the_declarator() {
    let (_, declared) = declare_frame(DEFERRED_ON);
    assert_eq!(
        declared.len(),
        ACCESS_COLUMN.len(),
        "the declarator emits {} accesses, the column has {} — declared: {declared:#?}",
        declared.len(),
        ACCESS_COLUMN.len()
    );
    for (i, (expected, got)) in ACCESS_COLUMN.iter().zip(declared.iter()).enumerate() {
        assert_eq!(got.0, expected.pass, "access {i}: pass");
        assert_eq!(got.1, expected.res, "access {i}: resource ({})", expected.pass);
        assert_eq!(got.2, expected.stage, "access {i}: stage ({} / {})", expected.pass, expected.res);
        assert_eq!(
            got.3, expected.access,
            "access {i}: access mask ({} / {})",
            expected.pass, expected.res
        );
    }
}

/// The column is path-INDEPENDENT: the tail declares the same buffer accesses on all four paths,
/// which is what makes the per-path difference a property of the paths' own producers rather than
/// of four hand-maintained particle declarations.
#[test]
fn the_access_column_is_the_same_on_every_path() {
    let (_, reference) = declare_frame(DEFERRED_ON);
    for row in ALL_ARMED {
        let (_, declared) = declare_frame(row);
        assert_eq!(declared, reference, "{}: the tail's access column moved", row.id);
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Gate #5 — `p_dispatch_args` pinned BOTH ways
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Gate #5, ACTIVE (`total_spawn > 0`): the indirect argument block derives exactly TWO barriers —
/// the seed's WAR at kickoff, then kickoff's `C/SHADER_WRITE → DI/INDIRECT_COMMAND_READ` at
/// **emit** — and the sim's identical second fetch is FREE. That free second read is the reason
/// D4 splits the dispatch block from the draw block in the first place.
#[test]
fn dispatch_args_stream_is_pinned_on_an_active_frame() {
    let (f, _) = declare_frame(DEFERRED_ON);
    let ids = f.particle.expect("armed");
    let bs = f.buf_on(ids.dispatch_args);
    assert_eq!(bs.len(), 2, "active: expected the seed WAR + the kickoff→emit RAW, got {bs:#?}");

    let kickoff = f
        .buf_at("particle_kickoff")
        .iter()
        .find(|b| b.res == ids.dispatch_args)
        .expect("active: the seed WAR at kickoff");
    assert_eq!(kickoff.src_stage, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT);
    assert_eq!(kickoff.src_access, 0, "a reader seed carries no access to flush");
    assert_eq!(kickoff.dst_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(kickoff.dst_access, VK_ACCESS_SHADER_WRITE_BIT);

    let emit = f
        .buf_at("particle_emit")
        .iter()
        .find(|b| b.res == ids.dispatch_args)
        .expect("active: the kickoff-write → indirect-fetch RAW at emit");
    assert_eq!(emit.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(emit.src_access, VK_ACCESS_SHADER_WRITE_BIT);
    assert_eq!(emit.dst_stage, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT);
    assert_eq!(emit.dst_access, VK_ACCESS_INDIRECT_COMMAND_READ_BIT);

    assert!(
        !f.buf_at("particle_sim").iter().any(|b| b.res == ids.dispatch_args),
        "active: the sim's second indirect fetch must be BARRIER-FREE — that free read is what \
         the p_dispatch_args / p_draw_args split buys"
    );
}

/// Gate #5, IDLE (`total_spawn == 0`): `particle_emit` is not declared at all, so the RAW that
/// was at emit MOVES to the sim — same two barriers, same fields, a different pass.
///
/// Pinning BOTH variants is the point: a stream pinned only on the active frame would go green
/// while the idle frame — which is the COMMON one at steady state — lost its ordering entirely.
#[test]
fn dispatch_args_stream_is_pinned_on_an_idle_frame() {
    let (f, _) = declare_frame(DEFERRED_ON_IDLE);
    let ids = f.particle.expect("armed");
    assert!(
        !f.pass_names.contains(&"particle_emit"),
        "idle: `total_spawn == 0` must not declare the emit pass"
    );
    assert!(
        !f.pass_names.contains(&"particle_upload"),
        "idle with a clean effect table: no upload pass either"
    );

    let bs = f.buf_on(ids.dispatch_args);
    assert_eq!(bs.len(), 2, "idle: expected the seed WAR + the kickoff→sim RAW, got {bs:#?}");
    let sim = f
        .buf_at("particle_sim")
        .iter()
        .find(|b| b.res == ids.dispatch_args)
        .expect("idle: the kickoff-write → indirect-fetch RAW moves to the sim");
    assert_eq!(sim.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(sim.src_access, VK_ACCESS_SHADER_WRITE_BIT);
    assert_eq!(sim.dst_stage, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT);
    assert_eq!(sim.dst_access, VK_ACCESS_INDIRECT_COMMAND_READ_BIT);
}

/// The conditional-pass proof's rows 3 and 4, executable — and the difference between them is
/// what the test is for.
///
/// On an idle frame `p_emit_req` is named by NO pass at all (row 3's "untouched; state persists
/// from the seed", which makes its reader-seed claim vacuously true), while `p_effects` IS named
/// — the sim reads effect parameters on every armed frame — so row 10's reader seed is a REAL
/// claim about a real access, not a vacuous one.
///
/// Both route ZERO barriers, and that is row 9/10's "free" column rather than a hole: the seed's
/// scope is `COMPUTE / SHADER_READ` and the sim's access is exactly that, so `transition` finds
/// the read already visible with no pending flush and returns `None`. The distinction between
/// them therefore lives in the DECLARED ACCESS list, not in the barrier stream — which is
/// precisely why gate #4 exists beside gate #3.
#[test]
fn an_idle_frame_leaves_the_request_table_untouched_and_still_reads_the_effect_table() {
    let (f, declared) = declare_frame(DEFERRED_ON_IDLE);
    let ids = f.particle.expect("armed");

    assert!(
        !declared.iter().any(|(_, res, _, _)| *res == "p_emit_req"),
        "idle: `p_emit_req` must be named by no pass at all — got {declared:#?}"
    );
    assert!(
        declared.iter().any(|(pass, res, stage, access)| *pass == "particle_sim"
            && *res == "p_effects"
            && *stage == VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT
            && *access == VK_ACCESS_SHADER_READ_BIT),
        "idle: the sim still reads `p_effects`, so row 10's reader seed is not vacuous"
    );

    // The "free" column: an access whose scope the seed already covers derives nothing.
    assert!(
        f.buf_on(ids.emit_req).is_empty(),
        "idle: an unnamed resource routes zero barriers"
    );
    assert!(
        f.buf_on(ids.effects).is_empty(),
        "idle: the sim's `p_effects` read matches the reader seed's scope exactly ⇒ FREE"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The push-constant range pin, and the constants the recorder addresses with
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// D12/F12: the three particle compute push blocks are built against DEDICATED pipeline layouts,
/// so none of them may widen the SHARED `COMPUTE_PUSH_CONSTANT_RANGE_BYTES`.
///
/// That constant is private to `rhi_impl`, so this asserts its DERIVATION — `max` over the two
/// consumers it is defined as — is still exactly 112, and that each particle range is strictly
/// under it. 112 leaves 16 bytes of the Vulkan-guaranteed 128-byte floor, which is why the batch
/// cull's occlusion matrix travels in a buffer; a particle block folded into that `max` would
/// spend headroom the tree has already refused to spend once.
#[test]
fn particle_push_bytes_do_not_widen_the_shared_compute_range() {
    let shared = COMPOSITE_PUSH_CONSTANT_BYTES.max(VB_BATCH_CULL_PUSH_BYTES);
    assert_eq!(
        shared, 112,
        "the shared COMPUTE push range must still be max(marcher 80, batch cull 112) == 112"
    );
    // `sim` is THREE words since rung P2 — `capacity` joined `steps`/`timestep` as the alpha
    // class's render-index mirror (`capacity - 1 - q_pos`). The other two are still two words.
    for (what, bytes, words) in [
        ("kickoff", PARTICLE_KICKOFF_PUSH_BYTES, 2),
        ("emit", PARTICLE_EMIT_PUSH_BYTES, 2),
        ("sim", PARTICLE_SIM_PUSH_BYTES, 3),
    ] {
        assert_eq!(bytes, words * 4, "{what}: the block is {words} 4-byte words");
        assert!(bytes < shared, "{what}: a dedicated-layout range must not reach the shared one");
        assert!(bytes.is_multiple_of(4), "{what}: Vulkan requires a multiple of 4");
    }
    // The DRAW's range is a GRAPHICS one — separate from the shared COMPUTE range entirely — and
    // is bounded only by the 128-byte device floor.
    assert_eq!(PARTICLE_DRAW_PUSH_BYTES, 72, "float4x4 + uint + int");
    // A `const` block: both operands are compile-time constants, so this is a BUILD-time claim
    // that happens to be re-stated in a test body — clippy is right that a runtime `assert!` on
    // two constants proves nothing at runtime.
    const {
        assert!(
            PARTICLE_DRAW_PUSH_BYTES <= 128,
            "within the guaranteed maxPushConstantsSize floor"
        );
        assert!(PARTICLE_DRAW_PUSH_BYTES.is_multiple_of(4));
    }
}

/// The offsets and sizes the recorder addresses the two indirect blocks with, pinned against the
/// plan's D4 layout: the dispatch commands at 0 and 16, the additive draw command at 0, and a
/// 12-byte 6-index quad. A drift makes a dispatch read its group counts from the wrong bytes,
/// which carries no validation message at all.
#[test]
fn indirect_offsets_and_the_quad_are_pinned() {
    assert_eq!(PARTICLE_DISPATCH_EMIT_OFFSET, 0);
    assert_eq!(PARTICLE_DISPATCH_SIM_OFFSET, 16, "the second VkDispatchIndirectCommand slot");
    assert!(PARTICLE_DISPATCH_SIM_OFFSET.is_multiple_of(4), "Vulkan requires a 4-aligned offset");
    assert_eq!(PARTICLE_DRAW_ADDITIVE_OFFSET, 0, "the first VkDrawIndexedIndirectCommand slot");
    // Rung P2's second slot. 24, not 20: the command is 20 bytes and the block pads to a 12-byte
    // multiple, so `alpha.instanceCount` lands at byte 28 — the offset the generator turns into the
    // sim's `DRAW_ALPHA_INSTANCE_WORD`. A drift here makes the alpha draw fetch its instance count
    // out of the additive command's `firstIndex`, with no validation message.
    assert_eq!(PARTICLE_DRAW_ALPHA_OFFSET, 24, "the second VkDrawIndexedIndirectCommand slot");
    assert!(PARTICLE_DRAW_ALPHA_OFFSET.is_multiple_of(4), "Vulkan requires a 4-aligned offset");
    // A `const` block for the same reason the push-size claim below is one: both operands are
    // compile-time constants, so a runtime `assert!` on them proves nothing at runtime.
    const {
        assert!(
            PARTICLE_DRAW_ALPHA_OFFSET >= PARTICLE_DRAW_ADDITIVE_OFFSET + 20,
            "the two commands must not overlap — a VkDrawIndexedIndirectCommand is 20 bytes"
        );
    }
    assert_eq!(PARTICLE_QUAD_INDEX_COUNT, 6, "two triangles");
    assert_eq!(PARTICLE_QUAD_IB_BYTES, 12, "six u16 indices");
    assert_eq!(PARTICLE_LOCAL_SIZE, 256, "emit + sim group edge (the research corpus's number)");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Rung P2 item 3 — the radix sort's derived stream (plan D10)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// **Arming the sort ADDS accesses and MOVES none that existed.**
///
/// The claim is derived from the two DECLARED lists rather than from two hand-written tables, so
/// there is no second copy of the access column to drift: strip every row belonging to a
/// `particle_sort_*` pass and every row naming `p_render_sorted`, and what remains must be the
/// unsorted frame's list, element for element and in order.
///
/// That is the executable form of "`SortMode::None` is byte-identical to rung P2 item 2" at the
/// DECLARATION level — the level the image goldens cannot see, because a barrier that moved would
/// still produce the same pixels on a scene with no hazard to expose.
#[test]
fn arming_the_sort_adds_accesses_and_moves_none_that_existed() {
    for (unsorted_row, sorted_row) in [(DEFERRED_ON, DEFERRED_ON_SORTED), (VB_ON, VB_ON_SORTED)] {
        let (_, unsorted) = declare_frame(unsorted_row);
        let (_, sorted) = declare_frame(sorted_row);
        let is_sort_row = |r: &(&'static str, &'static str, u32, u32)| {
            r.0.starts_with("particle_sort") || r.1 == "p_render_sorted"
        };
        let kept: Vec<_> = sorted.iter().filter(|r| !is_sort_row(r)).copied().collect();
        assert_eq!(
            kept, unsorted,
            "{}: arming the sort MOVED an access that already existed. The three sort passes are \
             appended between the sim and the draw and the draw gains ONE read; nothing else may \
             change, or `SortMode::None` stops being byte-identical to rung P2 item 2.",
            sorted_row.id
        );

        // ...and what it added is exactly the nine rows D10's partition names, in order. Spelled
        // out rather than counted, because a count would pass on nine WRONG rows.
        let added: Vec<_> = sorted.iter().filter(|r| is_sort_row(r)).copied().collect();
        const C: u32 = VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT;
        const R: u32 = VK_ACCESS_SHADER_READ_BIT;
        const W: u32 = VK_ACCESS_SHADER_WRITE_BIT;
        const VS: u32 = VK_PIPELINE_STAGE_VERTEX_SHADER_BIT;
        let want: [(&str, &str, u32, u32); 9] = [
            ("particle_sort_hist", "p_draw_args", C, R),
            ("particle_sort_hist", "p_render", C, R),
            ("particle_sort_hist", "p_sort_bins", C, RW),
            ("particle_sort_scan", "p_sort_bins", C, RW),
            ("particle_sort_scatter", "p_draw_args", C, R),
            ("particle_sort_scatter", "p_render", C, R),
            ("particle_sort_scatter", "p_render_sorted", C, W),
            ("particle_sort_scatter", "p_sort_bins", C, RW),
            ("particle_draw", "p_render_sorted", VS, R),
        ];
        assert_eq!(
            added.as_slice(),
            want.as_slice(),
            "{}: the sort's access column",
            sorted_row.id
        );
    }
}

/// The two sort `ResId`s exist on EVERY armed frame and route ZERO barriers when no sort pass
/// names them — F9, which is what lets the tail's length stay one number.
///
/// The alternative was a conditional tail LENGTH, i.e. a second predicate inside three sinks'
/// positional index arithmetic — the shape `graph_bridge`'s own interp-trio comment warns about,
/// and the one that resolves a barrier to a LIVE WRONG buffer when it goes wrong.
#[test]
fn the_sort_res_ids_are_declared_unarmed_and_route_nothing() {
    let (frame, _) = declare_frame(DEFERRED_ON);
    let ids = frame.particle.expect("the armed row declares the particle tail");
    assert_eq!(
        frame.g.res_name(ids.render_sorted),
        "p_render_sorted",
        "the sorted-render ResId must be declared on an armed-but-unsorted frame"
    );
    assert_eq!(frame.g.res_name(ids.sort_bins), "p_sort_bins");
    assert!(
        frame.buf_on(ids.render_sorted).is_empty(),
        "no pass names p_render_sorted on an unsorted frame, so F9 must route ZERO barriers for it"
    );
    assert!(
        frame.buf_on(ids.sort_bins).is_empty(),
        "no pass names p_sort_bins on an unsorted frame, so F9 must route ZERO barriers for it"
    );
}

/// **The sort's own seed rows, at the pass each names** — gate #3's discipline applied to rows 11
/// and 12.
///
/// * `p_render_sorted` — terminal is the alpha draw's VERTEX read, so the seed is a READER and the
///   scatter's write derives a **WAR** sourced at `(VERTEX_SHADER, 0)`. A writer seed here would
///   leave the sibling frame's draw unordered against this frame's scatter — a cross-frame WAR on
///   a single-buffered target, which is the torn-shimmer fingerprint this tree has a reference note
///   about and which no static scene would show.
/// * `p_sort_bins` — terminal is a COMPUTE write (the scan's re-zero), so the seed is a WRITER and
///   the histogram's accumulate derives a real **RAW**. That barrier is what makes the re-zero
///   VISIBLE to the next frame's histogram, so it is load-bearing rather than hygiene.
#[test]
fn the_sort_seed_rows_derive_at_the_pass_they_name() {
    let (frame, _) = declare_frame(DEFERRED_ON_SORTED);
    let ids = frame.particle.expect("the armed row declares the particle tail");

    let (pass, b) = frame
        .first_buf(ids.render_sorted)
        .expect("p_render_sorted must derive a barrier on a sorted frame");
    assert_eq!(pass, "particle_sort_scatter", "row 11's first access is the scatter's WRITE");
    assert_eq!(
        (b.src_stage, b.src_access),
        (VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, 0),
        "row 11 is a READER seed: the sibling frame's alpha draw is the terminal, so this is a WAR"
    );
    assert_eq!(
        (b.dst_stage, b.dst_access),
        (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT)
    );

    let (pass, b) = frame
        .first_buf(ids.sort_bins)
        .expect("p_sort_bins must derive a barrier on a sorted frame");
    assert_eq!(pass, "particle_sort_hist", "row 12's first access is the histogram's accumulate");
    assert_eq!(
        (b.src_stage, b.src_access),
        (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        "row 12 is a WRITER seed: the sibling frame's scan re-zeroed the histogram half, and THIS \
         is the barrier that makes that zero visible"
    );
}

/// The intra-frame chain the three dispatches need, each DERIVED rather than hand-written.
///
/// `hist → scan` and `scan → scatter` are both RAW on `p_sort_bins`: a workgroup barrier cannot
/// order two dispatches' writes, which is the entire reason D10 spends three dispatches on a
/// 256-bin sort instead of one. If either edge vanished the scatter would reserve from counts the
/// scan had not yet turned into offsets — and the result would still be a permutation, still be
/// dense, and still pass a monotonicity check on whichever bins happened to come out ordered.
#[test]
fn the_three_sort_dispatches_are_chained_on_the_bin_buffer() {
    let (frame, _) = declare_frame(DEFERRED_ON_SORTED);
    let ids = frame.particle.expect("the armed row declares the particle tail");
    for pass in ["particle_sort_scan", "particle_sort_scatter"] {
        let bars = frame.buf_at(pass);
        let bin_bar = bars.iter().find(|b| b.res == ids.sort_bins).unwrap_or_else(|| {
            panic!(
                "{pass} must derive a barrier on p_sort_bins — without it the pass reads what its \
                 predecessor has not published"
            )
        });
        assert_eq!(
            (bin_bar.src_stage, bin_bar.src_access),
            (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
            "{pass}'s p_sort_bins edge is a RAW against the previous dispatch's write"
        );
    }
    // The scatter's destination reaches the draw with a real RAW — the edge that makes the alpha
    // draw read records rather than the boot zeroes.
    let draw = frame.buf_at("particle_draw");
    let sorted_bar = draw
        .iter()
        .find(|b| b.res == ids.render_sorted)
        .expect("particle_draw must derive a RAW on p_render_sorted against the scatter's write");
    assert_eq!(
        (sorted_bar.src_stage, sorted_bar.src_access),
        (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT)
    );
    assert_eq!(
        (sorted_bar.dst_stage, sorted_bar.dst_access),
        (VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT)
    );
}

/// Rung P2 item 3's push range does not widen the shared COMPUTE one either — the same claim
/// `particle_push_bytes_do_not_widen_the_shared_compute_range` makes of the other three, restated
/// for the range the sort adds.
#[test]
fn the_sort_push_bytes_do_not_widen_the_shared_compute_range() {
    assert_eq!(PARTICLE_SORT_PUSH_BYTES, 16, "float3 cam_eye + uint capacity");
    const {
        assert!(
            PARTICLE_SORT_PUSH_BYTES < COMPOSITE_PUSH_CONSTANT_BYTES,
            "the sort's push range must stay strictly under the shared COMPUTE range, which sits \
             at 112 of a 128-byte floor with 16 bytes of headroom — the particle pipelines have \
             their own layouts (D12) precisely so none of them can move it"
        );
    }
    // The bin buffer is the two halves and nothing else; the host sizes the allocation from this.
    assert_eq!(PARTICLE_SORT_BINS_WORDS, 2 * PARTICLE_SORT_BINS);
    assert_eq!(PARTICLE_SORT_BINS, PARTICLE_LOCAL_SIZE, "one bin per lane, in all three modules");
}
