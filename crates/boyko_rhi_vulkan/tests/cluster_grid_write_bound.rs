//! VB-P1j — the BASE cull arm's `ClusterGrid[fi]` WRITE bound, swept over boot/live skews.
//!
//! # The defect this pins
//!
//! `ClusterGrid` is allocated ONCE, at scene boot, with `ClusterConfig::cluster_count()` cells
//! (`boyko_app::gpu_scene::GpuSceneBundles::build_froxel_light_cull`) and is never re-allocated.
//! The BASE arm of `cluster_cull.hlsl` used to bound its write on `dim_x * dim_y * dim_z` read
//! from the LIVE light-table header, which `boyko_render::light::sync_cluster_light_gate`
//! republishes from the LIVE `ClusterConfig` resource every frame. The host meanwhile dispatches
//! the BOOT froxel count rounded up to the arm's 64-wide group
//! (`scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X)`, all three record sites).
//!
//! The three quantities are therefore free to disagree, and the reachable write bound was
//! `min(64 * ceil(boot_cc / 64), live_cc)` — which EXCEEDS `boot_cc` whenever `boot_cc % 64 != 0`
//! and the live dims grow. `robustBufferAccess` is OFF in this engine and no GPU-assisted
//! validation runs, so the resulting device write past the end of the allocation is silent.
//!
//! VB-P1j clamps the shader's `cluster_count` by `ClusterGrid.GetDimensions()` — the bound
//! DESCRIPTOR's own element count (SPIR-V `OpArrayLength`), not a host-side mirror of the boot
//! size — so no `fi` past the allocation can ever reach the write.
//!
//! # What this test IS, and what it is NOT
//!
//! It sweeps [`golden_base_thread_map`], the Rust re-implementation of the shader's prologue, and
//! it also evaluates the PRE-FIX formula inline so the mutation is *demonstrated* rather than
//! asserted: [`unclamped_valid`] is the old predicate, and the sweep shows it going out of bounds
//! by exactly the count `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §11 measured (16 cells at boot
//! 16x9x23 vs live 16x9x24) while the shipped predicate does not.
//!
//! It does NOT execute the shader. This is a HAND-BUILT model, exactly like
//! `golden_hier_thread_map`'s own sweep: if the HLSL and the mirror drift, only a device run
//! notices. The artifact-level half of the pin — that the clamp is PRESENT in the committed
//! `cluster_cull.comp.spv` — is `cluster_cull_spv_sync.rs`'s `op_array_length` census, which
//! counts the emitted `OpArrayLength` on the real module. Neither half alone is sufficient;
//! recording that openly is the point (this campaign has shipped a detector proven on hand-built
//! data before, and the debt is written down rather than hidden).

use boyko_rhi_vulkan::goldens::{BASE_GROUP_THREADS, golden_base_thread_map};

/// One boot/live configuration: the dims `ClusterGrid` was SIZED from, and the dims the LIVE
/// header carries when the cull dispatches.
struct Skew {
    /// Human-readable label, used in assertion messages.
    name: &'static str,
    /// The BOOT `ClusterConfig` dims — the allocation is `boot.0 * boot.1 * boot.2` cells.
    boot: (u32, u32, u32),
    /// The LIVE header dims the shader unpacks from `cluster_params` word 14.
    live: (u32, u32, u32),
}

/// The froxel count a dims triple denotes.
fn cc((x, y, z): (u32, u32, u32)) -> u32 {
    x * y * z
}

/// The number of threads the host actually dispatches: the BOOT froxel count rounded up to the
/// base arm's 64-wide group, mirroring every record site's
/// `scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X)`.
fn dispatched_threads(boot_cc: u32) -> u32 {
    boot_cc.div_ceil(BASE_GROUP_THREADS) * BASE_GROUP_THREADS
}

/// The PRE-FIX write predicate — `cluster_count = cp.dim_x * cp.dim_y * cp.dim_z` with NO
/// capacity clamp. Kept here (rather than deleted with the bug) so the red mutation is executable
/// evidence in the same run as the green assertion.
fn unclamped_valid(tid: u32, live: (u32, u32, u32)) -> bool {
    tid < cc(live)
}

/// The sweep matrix. Rows 1-2 are the plan's measured case and its mirror image; rows 3-5 cover
/// the aligned, degenerate and shrinking cases that must stay untouched.
const SKEWS: &[Skew] = &[
    // The plan's §11 measurement: boot 16x9x23 = 3312 (NOT a multiple of 64 — 3312 = 51*64 + 48),
    // live 16x9x24 = 3456. Dispatched threads = 52*64 = 3328, so the old bound admitted
    // fi in [3312, 3328) — 16 cells, 128 bytes past the end.
    Skew { name: "plan §11 measured grow (23 -> 24 slices)", boot: (16, 9, 23), live: (16, 9, 24) },
    // A larger grow on the same non-aligned boot: the old bound is capped by the DISPATCH, not by
    // the live dims, so the overrun stays 16 — the bound is `min(dispatch, live_cc)`.
    Skew { name: "large grow (23 -> 40 slices)", boot: (16, 9, 23), live: (16, 9, 40) },
    // The shipping configuration: boot == live. Nothing may change here — this is the row that
    // makes every golden hold.
    Skew { name: "shipping default (no skew)", boot: (16, 9, 24), live: (16, 9, 24) },
    // Shrink: the live grid is smaller than the allocation. Already safe before VB-P1j (the live
    // bound is the tighter one) and must STAY exactly as safe — the clamp must not widen it.
    Skew { name: "shrink (24 -> 12 slices)", boot: (16, 9, 24), live: (16, 9, 12) },
    // A degenerate all-zero live header (what `sync_cluster_light_gate` publishes on a non-VB
    // boot). No thread may be valid, and no delinearization may divide by zero.
    Skew { name: "degenerate zero-dims live header", boot: (16, 9, 24), live: (0, 0, 0) },
    // A 1-cell boot allocation with a full-size live header — the worst ratio, and the row where
    // an unclamped bound overruns by the entire dispatch group.
    Skew { name: "1-cell boot vs full live grid", boot: (1, 1, 1), live: (16, 9, 24) },
];

/// The shipped predicate never admits an `fi` outside the allocation, for any boot/live skew and
/// any thread the host actually dispatches.
#[test]
fn base_arm_write_stays_inside_the_allocation_under_every_skew() {
    for s in SKEWS {
        let capacity = cc(s.boot);
        let threads = dispatched_threads(capacity);
        for tid in 0..threads {
            let (_x, _y, _z, fi, valid) =
                golden_base_thread_map(tid, s.live.0, s.live.1, s.live.2, capacity);
            if valid {
                assert!(
                    fi < capacity,
                    "{}: thread {tid} writes ClusterGrid[{fi}] into a {capacity}-cell allocation \
                     (boot {:?}, live {:?}) — the VB-P1j capacity clamp is not holding",
                    s.name,
                    s.boot,
                    s.live,
                );
            }
        }
    }
}

/// The RED MUTATION, executed: the pre-fix predicate DOES leave the allocation, by exactly the
/// count the plan measured. If this test ever goes green it means the defect was never
/// reachable — i.e. this whole rung was pointless — so it is written to fail loudly in that case
/// rather than silently pass.
#[test]
fn the_prefix_predicate_demonstrably_leaves_the_allocation() {
    // (label, expected number of out-of-bounds cells the OLD predicate admitted)
    let expected: &[(&str, u32)] = &[
        ("plan §11 measured grow (23 -> 24 slices)", 16),
        ("large grow (23 -> 40 slices)", 16),
        ("shipping default (no skew)", 0),
        ("shrink (24 -> 12 slices)", 0),
        ("degenerate zero-dims live header", 0),
        ("1-cell boot vs full live grid", 63),
    ];
    for (s, (label, want)) in SKEWS.iter().zip(expected) {
        assert_eq!(s.name, *label, "SKEWS and the expectation table drifted out of step");
        let capacity = cc(s.boot);
        let threads = dispatched_threads(capacity);
        let oob = (0..threads).filter(|&tid| unclamped_valid(tid, s.live) && tid >= capacity).count();
        assert_eq!(
            oob as u32, *want,
            "{}: the PRE-FIX predicate admitted {oob} out-of-bounds cells into a {capacity}-cell \
             allocation (boot {:?}, live {:?}); the plan's §11 measurement says {want}. A count \
             of 0 where a positive one was expected means the defect is unreachable and the \
             sweep is no longer exercising it.",
            s.name,
            s.boot,
            s.live,
        );
    }
}

/// The clamp is a NO-OP whenever boot dims == live dims — the property that keeps every shipping
/// golden byte-identical. Asserted as an exhaustive per-thread equality against the pre-fix
/// predicate, not as a spot check.
#[test]
fn the_clamp_is_inert_when_boot_dims_equal_live_dims() {
    for dims in [(16, 9, 24), (16, 9, 23), (1, 1, 1), (32, 18, 48), (8, 8, 8)] {
        let capacity = cc(dims);
        let threads = dispatched_threads(capacity);
        for tid in 0..threads {
            let (_x, _y, _z, _fi, valid) =
                golden_base_thread_map(tid, dims.0, dims.1, dims.2, capacity);
            assert_eq!(
                valid,
                unclamped_valid(tid, dims),
                "dims {dims:?}: thread {tid} changed validity under the VB-P1j clamp even though \
                 boot dims == live dims — the clamp must be arithmetically inert here, which is \
                 what makes the cull's output (and every golden pinned on it) unchanged"
            );
        }
    }
}

/// Every in-bounds thread still delinearizes to the froxel the READERS linearize back to — the
/// clamp must not perturb the `(x, y, z)` mapping, only the set of threads that reach the write.
#[test]
fn the_clamp_does_not_perturb_the_froxel_mapping() {
    for s in SKEWS {
        let capacity = cc(s.boot);
        let (dx, _dy, dz) = s.live;
        for tid in 0..dispatched_threads(capacity) {
            let (x, y, z, fi, valid) =
                golden_base_thread_map(tid, s.live.0, s.live.1, s.live.2, capacity);
            if valid {
                assert_eq!(
                    boyko_rhi_vulkan::goldens::golden_cluster_index(x, y, z, dx, dz),
                    fi,
                    "{}: thread {tid} delinearized to ({x},{y},{z}) which re-linearizes to a \
                     DIFFERENT froxel than {fi} — the cull write and every ClusterGrid reader \
                     would then disagree about which cell belongs to which pixel",
                    s.name,
                );
            }
        }
    }
}
