//! `emit_particles` — generates ALL FIVE committed GPU-particle shaders
//! (`docs/PARTICLES-PLAN.md` Rev 4, decision D12).
//!
//! ```text
//! particle_kickoff.comp.hlsl   [numthreads(1,1,1)]     the one-thread bookkeeping pass (A2)
//! particle_emit.comp.hlsl      [numthreads(256,1,1)]   ZERO global atomics (A3)
//! particle_sim.comp.hlsl       [numthreads(256,1,1)]   the wave-aggregated hot loop (A4)
//! particle_draw.vs.hlsl                                the billboard expansion (A5)
//! particle_draw.fs.hlsl                                additive, bindless-textured (A5)
//! ```
//!
//! Run: `cargo run -p boyko_shaderdsl --features emit --bin emit_particles`
//!
//! Five sources, NINE artifacts: the two draw stages each carry a `-D DEPTH_LINEAR` variant
//! (`particle_draw_dlin.{vs,fs}.spv`) — the Deferred path's fragment-written depth encode — and the
//! sim carries TWO, `-D SDF_COLLIDE` (`particle_sim_sdf.comp.spv`, rung P1's field collision) and
//! `-D SDF_COLLIDE_STATS` on top of it (`particle_sim_stats.comp.spv`, rung P1b's per-wave skip
//! census — a MEASUREMENT module, never a shipping one). Each has a row in
//! `docs/SHADER-VARIANT-MANIFEST.md`. Every define is INERT in the compiles below it, so the five
//! base `.spv` — and, across rung P1b, `particle_sim_sdf.comp.spv` too — are byte-frozen by
//! construction.
//!
//! Then DXC each file with the frozen recipe pinned in its own header, and commit the `.spv`.
//! `boyko_rhi_vulkan/tests/particle_edsl_sync.rs` pins both halves: the committed `.hlsl` to
//! these templates' generated spans, and each committed `.spv` to a fresh re-DXC of its source.
//!
//! # Why the generator owns the WHOLE file (F13/F14)
//!
//! The eDSL has no atomics, no `groupshared`, no stores and no texture sampling, so the skeleton
//! — bindings, `[numthreads]`, the wave block, the record writes — is hand-written HLSL. Owning
//! it here as a `format!` template (the `emit_probe_gi` idiom) is what keeps that glue
//! single-sourced with the eDSL spans it wraps AND with the numbers below: the group sizes, the
//! substep ceiling, the counter word indices and the two `instanceCount` byte offsets are
//! GENERATOR INPUTS, so no shader ever spells a word index that some host `offset_of!` also
//! spells (plan D4/D12, gate #8).
//!
//! # What is NOT generated
//!
//! The host-side `PARTICLE_LAYOUT_ENTRIES` table, the pipeline/layout creation and the framegraph
//! declaration are the integration change's; each shader carries the (set, binding, kind) table
//! its own resources declare, and that table is what the host's must match.

use std::path::PathBuf;

use boyko_shaderdsl::emit;

// ---- Generator inputs (mirrored host-side; plan D4/D12) -----------------------------------

/// The emit/sim workgroup width (R4 / plan D4). Pinned artifact-side by the `LocalSize` opcode
/// assertion in `particle_edsl_sync`.
const LOCAL_SIZE: u32 = 256;

/// `log2(LOCAL_SIZE)` — the kickoff's group-count round-up is a SHIFT, not a divide, so the
/// one-thread pass carries no `OpUDiv`.
const LOCAL_SIZE_LOG2: u32 = LOCAL_SIZE.trailing_zeros();

/// `PARTICLE_SUBSTEP_CEILING` (plan M3). The host clamps ONCE; the shader's `min` is the F25
/// hang guard against a corrupt push constant and can never bind on a well-formed one.
const SUBSTEP_CEILING: u32 = 64;

/// `MAX_EMITTERS` (plan D15/R8) — also the `groupshared` prefix array's length, hence the
/// emit pass's binary-search depth.
const MAX_EMITTERS: u32 = 256;

/// The 6-index / 4-vertex quad the indirect draw indexes (plan D4 — `vkCmdDrawIndirect` is not
/// loaded, so the draw is INDEXED against a 12-byte `u16` index buffer).
const QUAD_INDEX_COUNT: u32 = 6;

/// `PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET` — `offset_of!(ParticleDrawArgs, additive.instance_count)`.
/// Plan D4 pins it at **4**; gate #8 pins the host const AND the word index this generator
/// derives from it.
const ADDITIVE_INSTANCE_COUNT_OFFSET: u32 = 4;

/// `PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET` — the P2 slot's counter, at byte **28** (the second
/// `VkDrawIndexedIndirectCommand` starts at 24). Rung P2's blend partition is its first consumer:
/// the sim's alpha-class wave leaders reserve their render positions on it.
const ALPHA_INSTANCE_COUNT_OFFSET: u32 = 28;

/// `boyko_render::PARTICLE_BLEND_ALPHA` — the `EffectParamsGpu::blend_class` discriminant that
/// sends a survivor to the ALPHA half of the render buffer (rung P2, plan D10/M2).
///
/// The additive class is the OTHER value, and the shader tests for alpha rather than for additive
/// deliberately: a future third class must fall to the additive (order-independent, no sort) arm
/// rather than silently join the sorted one.
const PARTICLE_BLEND_ALPHA: u32 = 1;

/// The stored `(cos, sin)` pattern for the identity rotation: `cos = +1` quantizes to `32767`,
/// `sin = 0` to `0`, so the packed word is `0x00007FFF`. Emit seeds every particle with it.
const ROT_IDENTITY: u32 = 32767;

/// The `first_spawn` sentinel for a `groupshared` slot beyond the live emitter count — the
/// maximum `uint`, so the binary search's `<= gid` test can never select it.
const SPAWN_SENTINEL: u32 = u32::MAX;

// `ParticleCounters`' word indices (plan D2's field order). One cache line, `RWStructuredBuffer<uint>`
// rather than a typed record so `InterlockedAdd` can name an element directly.

/// `alive_count_cur` — written by kickoff only; the sim's guard bound AND the size kickoff
/// derived the sim dispatch from (plan: "the same field, read twice").
const CTR_ALIVE_CUR: u32 = 0;
/// `alive_count_next` — the LIST counter, written by the sim's wave leaders only.
const CTR_ALIVE_NEXT: u32 = 1;
/// `dead_count` — kickoff pre-decrements, the sim's dying wave leaders push.
const CTR_DEAD_COUNT: u32 = 2;
/// `dead_base` — kickoff only; emit reads `p_dead[dead_base + gid]`.
const CTR_DEAD_BASE: u32 = 3;
/// `emit_append_base` — kickoff only; emit writes `p_alive_read[emit_append_base + gid]`.
const CTR_EMIT_BASE: u32 = 4;
/// `real_emit_count` — kickoff only; emit's range guard.
const CTR_REAL_EMIT: u32 = 5;
/// `clamped_spawns` — kickoff only, a cold diagnostic accumulator (plan D15).
const CTR_CLAMPED: u32 = 6;

// Rung P1b's three census words (plan P1b item 1). `offset_of!(ParticleCounters, …) / 4` on the
// host, carved out of the counter line's PAD so no shipping counter moved. Emitted ONLY inside
// `#ifdef SDF_COLLIDE_STATS`, which is why the two shipping sim `.spv` cannot see them at all.

/// `waves_evaluated` — wave-substeps in which at least one lane needed the field, so the whole wave
/// paid the edit-list walk. Written by the `-D SDF_COLLIDE_STATS` sim's wave leaders only.
const CTR_WAVES_EVALUATED: u32 = 7;
/// `waves_skipped` — wave-substeps in which NO lane needed the field. Exclusive with
/// [`CTR_WAVES_EVALUATED`], so the two sum to the wave-substep count.
const CTR_WAVES_SKIPPED: u32 = 8;
/// `lanes_evaluated` — LANES that needed the field, summed over every wave-substep: the per-lane
/// numerator the wave-coherence argument predicts will overstate the saving.
const CTR_LANES_EVALUATED: u32 = 9;

/// The `-D SDF_COLLIDE` skip predicate, spelled ONCE.
///
/// Rung P1b's census re-states this test to take its ballot, and a census whose predicate had
/// drifted from the branch's would report a skip rate for a decision the shader never made. Emitting
/// both occurrences from this one string makes that drift unconstructible here, and
/// `particle_edsl_sync` re-checks it at the committed shader.
const SDF_SKIP_TEST: &str = "cached_d - travel_l > radius_l";

/// The `VkDispatchIndirectCommand` word index of the EMIT dispatch inside `p_dispatch_args`
/// (offset 0).
const DISPATCH_EMIT_WORD: u32 = 0;
/// The `VkDispatchIndirectCommand` word index of the SIM dispatch (offset 16).
const DISPATCH_SIM_WORD: u32 = 4;

/// The Set-0 binding the `-D SDF_COLLIDE` variant reads the SDF edit list at (rung P1 / plan D9).
///
/// The SAME number `sdf_mesh_shadow.comp.hlsl` gives its own `Buf` — every field CONSUMER in the
/// tree binds the edit list at 10 — and the next free slot in the particle compute vocabulary,
/// whose P0 bindings are 0..9. The host `PARTICLE_LAYOUT_ENTRIES` table mirrors it; the base
/// compile does not declare it at all (DXC never sees the block), so the descriptor is
/// bound-but-unread on a disarmed run.
const SDF_FIELD_BINDING: u32 = 10;

/// `boyko_rhi_vulkan::compute::CAM_MODE_PERSPECTIVE` — the raw `camera_mode` value the shared
/// 80-byte camera UBO carries for a perspective view (P2 `-D DEPTH_LINEAR`).
///
/// The draw's VS forwards `cam_mode = (camera_mode == CAM_MODE_PERSPECTIVE) ? 1.0 : 0.0` so the
/// fragment's depth encode selects the SAME arm `gbuffer_mrt.fs.hlsl` selects from its own
/// `cam_eye.w` lane. Pinned against the host const by `particle_edsl_sync`.
const CAM_MODE_PERSPECTIVE: u32 = 1;

/// `boyko_rhi_vulkan::compute::MESH_DEPTH_T_MAX` — the PERSPECTIVE mesh-depth normalizer the
/// Deferred path's depth buffer is encoded with (P2 `-D DEPTH_LINEAR`).
///
/// Not a free choice: it is the divisor `gbuffer_mrt.fs.hlsl:327` writes its `SV_Depth` with, and
/// the particle variant must land in the SAME units or the depth test compares two encodings. The
/// raster shaders `#include` nothing, so the literal is duplicated in each of them and pinned
/// host-side (`particle_edsl_sync`, the `instanced_vs_host_mirror` discipline).
const MESH_DEPTH_T_MAX: f32 = 64.0;

fn main() {
    // Compile-time guards on the generator's own inputs: a mis-set constant would silently
    // produce a shader whose word indices no host `offset_of!` agrees with.
    const _: () = assert!(LOCAL_SIZE.is_power_of_two());
    const _: () = assert!(MAX_EMITTERS.is_power_of_two());
    const _: () = assert!(ADDITIVE_INSTANCE_COUNT_OFFSET.is_multiple_of(4));
    const _: () = assert!(ALPHA_INSTANCE_COUNT_OFFSET.is_multiple_of(4));

    let shaders = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("boyko_rhi_vulkan")
        .join("shaders");

    let files: [(&str, String); 5] = [
        ("particle_kickoff.comp.hlsl", build_kickoff()),
        ("particle_emit.comp.hlsl", build_emit()),
        ("particle_sim.comp.hlsl", build_sim()),
        ("particle_draw.vs.hlsl", build_draw_vs()),
        ("particle_draw.fs.hlsl", build_draw_fs()),
    ];

    for (name, text) in &files {
        let out = shaders.join(name);
        std::fs::write(&out, text)
            .unwrap_or_else(|e| panic!("invariant: failed to write {} : {e}", out.display()));
        println!("wrote {} ({} bytes)", out.display(), text.len());
    }
    println!(
        "generator inputs: LOCAL_SIZE={LOCAL_SIZE} SUBSTEP_CEILING={SUBSTEP_CEILING} \
         MAX_EMITTERS={MAX_EMITTERS} additive.instanceCount @byte {ADDITIVE_INSTANCE_COUNT_OFFSET} \
         (word {}) alpha.instanceCount @byte {ALPHA_INSTANCE_COUNT_OFFSET} (word {})",
        ADDITIVE_INSTANCE_COUNT_OFFSET / 4,
        ALPHA_INSTANCE_COUNT_OFFSET / 4,
    );
}

/// The shared `ParticleCounters` word-index block, emitted from the `CTR_*` generator inputs so
/// no shader spells a literal index.
fn counter_words() -> String {
    format!(
        "// `ParticleCounters` (plan D2) is one 64-byte line addressed as `uint` WORDS -- a typed\n\
         // record cannot be the destination of `InterlockedAdd`. Every index below is a GENERATOR\n\
         // INPUT (`emit_particles.rs`), never typed here, so it cannot drift from the host struct.\n\
         static const uint CTR_ALIVE_CUR  = {CTR_ALIVE_CUR}u;\n\
         static const uint CTR_ALIVE_NEXT = {CTR_ALIVE_NEXT}u;\n\
         static const uint CTR_DEAD_COUNT = {CTR_DEAD_COUNT}u;\n\
         static const uint CTR_DEAD_BASE  = {CTR_DEAD_BASE}u;\n\
         static const uint CTR_EMIT_BASE  = {CTR_EMIT_BASE}u;\n\
         static const uint CTR_REAL_EMIT  = {CTR_REAL_EMIT}u;\n\
         static const uint CTR_CLAMPED    = {CTR_CLAMPED}u;\n"
    )
}

/// The `EffectParamsGpu` declaration (plan D2, 128 B). Shared verbatim by emit and sim.
///
/// Every member's offset is IDENTICAL under DXC's default structured-buffer layout and under
/// scalar layout — each `float3` is followed by a `float`, each `uintN` sits on its own natural
/// boundary — so the record cannot be read differently by two consumers compiled with different
/// layout flags.
fn effect_params_struct() -> &'static str {
    "// The per-effect row (plan D2, 128 B). `damping` and the rotation multiplier are\n\
     // HOST-PRECOMPUTED against the CONSTANT `ParticleClock` timestep (plan D6), which is what\n\
     // deletes `exp2` and all trig from the device. `rot_mul_cos`/`rot_mul_sin` are an f32 PAIR,\n\
     // not snorm16 (plan K1: a quantized multiplier's magnitude error is a per-effect CONSTANT\n\
     // that compounds as (1+d)^n to ~1% over 640 steps; at f32 it is <= 1 ULP).\n\
     struct EffectParamsGpu {\n\
     \x20   float3 gravity;      float damping;\n\
     \x20   float  rot_mul_cos;  float rot_mul_sin;\n\
     \x20   uint2  _r0;\n\
     \x20   uint4  color_keys;\n\
     \x20   uint2  color_times;  uint2 size_keys;\n\
     \x20   float  lifetime_min; float lifetime_max; float speed_min; float speed_max;\n\
     \x20   float  size_base;    float cone_cos;     float _r1;       float _r2;\n\
     \x20   uint   tex_index;    uint  blend_class;  uint  flags;     float collision_radius;\n\
     \x20   float  restitution;  float friction;     uint  emitter_shape; uint _r3;\n\
     };\n"
}

/// The `ParticleSim` record declaration (plan D2, 48 B) — the sim's working set, one fully
/// consumed 64-byte line per particle under the alive-list gather (R2).
fn particle_sim_struct() -> &'static str {
    "// The sim's working set (plan D2, 48 B, AoS). `size0_invlife` packs the spawn size and the\n\
     // RECIPROCAL total lifetime as two binary16 halves -- emit pays the one divide, once per\n\
     // particle, so the per-frame sim is divide-free. `effect_flags` packs `u16 effect_index |\n\
     // u16 flags`. `cached_field_d` is rung P1's Lipschitz cache: seeded 0 at spawn (so the first\n\
     // substep always evaluates the field) and maintained by the sim's `-D SDF_COLLIDE` arm --\n\
     // untouched, and never read, by the base compile.\n\
     struct ParticleSim {\n\
     \x20   float3 position; float life_remaining;\n\
     \x20   float3 velocity; float cached_field_d;\n\
     \x20   uint   color_rgba8;\n\
     \x20   uint   size0_invlife;\n\
     \x20   uint   effect_flags;\n\
     \x20   uint   rot_cs;\n\
     };\n"
}

/// The `ParticleRender` record declaration (plan D2, 32 B) — the draw's working set, read
/// SEQUENTIALLY by the VS with no gather.
fn particle_render_struct() -> &'static str {
    "// The draw's working set (plan D2, 32 B, AoS). Written by the sim at the CLASS-DENSE render\n\
     // index; read by the VS at `pc.index_base + pc.index_step * SV_InstanceID`. Ramp resolution\n\
     // happens ONCE per particle in the sim rather than 4x per particle in the VS.\n\
     struct ParticleRender {\n\
     \x20   float3 position; float size;\n\
     \x20   uint   color_rgba8;\n\
     \x20   uint   rot_cs;\n\
     \x20   uint   tex_index;\n\
     \x20   uint   flags;\n\
     };\n"
}

/// Assembles `particle_kickoff.comp.hlsl` — plan A2 / D3's kickoff block, verbatim.
fn build_kickoff() -> String {
    let words = counter_words();
    let additive_word = ADDITIVE_INSTANCE_COUNT_OFFSET / 4;
    let alpha_word = ALPHA_INSTANCE_COUNT_OFFSET / 4;
    format!(
        r#"// particle_kickoff.comp -- the GPU particle system's ONE-THREAD bookkeeping pass
// (`docs/PARTICLES-PLAN.md` Rev 4, algorithm A2 / decision D3).
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_particles`. Hand-edits
// fail `boyko_rhi_vulkan/tests/particle_edsl_sync.rs`.
//
// # What it does, in one thread
//
// Swaps the alive roles' COUNTS, clamps the requested spawn against the free list, publishes the
// two bases emit reads, and writes both indirect argument blocks. It is O(1) over three cache
// lines and entirely branchless.
//
// The reason it is ONE thread is the reason `particle_emit` needs ZERO atomics: because a single
// lane owns `dead_count` and `alive_count_cur` here, it can pre-DECREMENT one and pre-INCREMENT
// the other in the same pass, publishing `dead_base` and `emit_append_base` as plain values.
// Every emit lane then computes both of its indices arithmetically from `gid`.
//
// # The partition this pass maintains (plan, "The partition at each boundary")
//
// Let A = alive_count_cur, D = dead_count, E = real_emit_count, N = alive_count_next. `A + D ==
// CAP` holds ACROSS this pass because the reservation is accounted on BOTH sides simultaneously:
// `dead_count` drops by E in the same lane that raises `alive_count_cur` by E.
//
// # Compile (offline + hermetic; committed `.spv` is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 particle_kickoff.comp.hlsl -Fo particle_kickoff.comp.spv
//
// # Set / binding vocabulary -- MIRRORS the host `PARTICLE_LAYOUT_ENTRIES` table
//
//   (set, binding, kind)
//   (0, 0, STORAGE_BUFFER)  RWStructuredBuffer<uint>  p_counters       read+write
//   (0, 1, STORAGE_BUFFER)  RWStructuredBuffer<uint>  p_dispatch_args  write
//   (0, 2, STORAGE_BUFFER)  RWStructuredBuffer<uint>  p_draw_args      write
//
//   The three compute passes SHARE one Set-0 vocabulary (bindings 0..9); each declares only the
//   subset it uses, and DXC strips the rest. The host layout is the union.
//
// # Push constants (8 B, well under the shared COMPUTE range floor)
//
//   [0,4)  uint requested_spawn -- the host's already-clamped total across all emitters
//   [4,8)  uint capacity        -- CAP, the boot-frozen pool size (plan D14)

struct KickoffPush {{
    uint requested_spawn;
    uint capacity;
}};
[[vk::push_constant]] KickoffPush pc;

[[vk::binding(0, 0)]] RWStructuredBuffer<uint> p_counters      : register(u0);
[[vk::binding(1, 0)]] RWStructuredBuffer<uint> p_dispatch_args : register(u1);
[[vk::binding(2, 0)]] RWStructuredBuffer<uint> p_draw_args     : register(u2);

{words}
// The two `VkDispatchIndirectCommand` blocks inside `p_dispatch_args` (plan D4: offsets 0 and 16).
static const uint DISPATCH_EMIT_WORD = {DISPATCH_EMIT_WORD}u;
static const uint DISPATCH_SIM_WORD  = {DISPATCH_SIM_WORD}u;

// The two `VkDrawIndexedIndirectCommand` slots inside `p_draw_args`. The `instanceCount` word
// indices are DERIVED by the generator from `PARTICLE_{{ADDITIVE,ALPHA}}_INSTANCE_COUNT_OFFSET`
// ({ADDITIVE_INSTANCE_COUNT_OFFSET} and {ALPHA_INSTANCE_COUNT_OFFSET} bytes, both 4-aligned), which are the SAME `offset_of!` chains the host
// pins -- plan gate #8.
static const uint DRAW_ADDITIVE_BASE_WORD     = 0u;
static const uint DRAW_ADDITIVE_INSTANCE_WORD = {additive_word}u;
static const uint DRAW_ALPHA_BASE_WORD        = {alpha_base_word}u;
static const uint DRAW_ALPHA_INSTANCE_WORD    = {alpha_word}u;

// The workgroup widths this pass sizes the two indirect dispatches for. `>> LOG2` rather than
// `/ LOCAL_SIZE`: the width is a power of two, so the round-up is a shift and this module carries
// no integer divide.
static const uint LOCAL_SIZE      = {LOCAL_SIZE}u;
static const uint LOCAL_SIZE_LOG2 = {LOCAL_SIZE_LOG2}u;

// The 6-index quad every particle instance draws (plan D4).
static const uint QUAD_INDEX_COUNT = {QUAD_INDEX_COUNT}u;

[numthreads(1, 1, 1)]
void main() {{
    // The ROLE SWAP. `alive_count_next` was written by the PREVIOUS frame's sim; the seed on
    // `p_counters` is what carries its availability across the frame edge (plan seed-table row 6).
    uint a = p_counters[CTR_ALIVE_NEXT];
    p_counters[CTR_ALIVE_NEXT] = 0u;

    // D3's release-present clamp on the free list (plan D15/R8, and F25 -- `robustBufferAccess`
    // is OFF, so `dead_base + gid` addressing `p_dead` out of range is UB, not a clamp).
    //
    // D3 writes this clamp `max(0, dead_count)`. Over a `uint` that half is VACUOUS -- DXC folds
    // it and no instruction survives -- so the operative half, and the only one that can bind, is
    // the upper cap at CAP. Both directions of "clamp into the legal range" are therefore carried
    // by the `min` alone.
    uint d = min(p_counters[CTR_DEAD_COUNT], pc.capacity);

    // The spawn request, clamped against what the free list can actually serve. The shortfall
    // ACCUMULATES in a cold diagnostic word the host reads out of band.
    uint e = min(pc.requested_spawn, d);
    p_counters[CTR_CLAMPED] = p_counters[CTR_CLAMPED] + (pc.requested_spawn - e);

    uint dead_base = d - e;      // the PRE-DECREMENT: the E reserved slots sit at [dead_base, d)
    uint alive_cur = a + e;      // the PRE-INCREMENT: the E fresh list slots sit at [a, alive_cur)

    p_counters[CTR_DEAD_COUNT] = dead_base;
    p_counters[CTR_DEAD_BASE]  = dead_base;
    p_counters[CTR_EMIT_BASE]  = a;
    p_counters[CTR_REAL_EMIT]  = e;
    p_counters[CTR_ALIVE_CUR]  = alive_cur;

    // The two indirect DISPATCH blocks. The sim is sized from `alive_count_cur` -- the SAME field
    // its own range guard reads, not a second derivation that has to be kept in agreement.
    p_dispatch_args[DISPATCH_EMIT_WORD + 0u] = (e + (LOCAL_SIZE - 1u)) >> LOCAL_SIZE_LOG2;
    p_dispatch_args[DISPATCH_EMIT_WORD + 1u] = 1u;
    p_dispatch_args[DISPATCH_EMIT_WORD + 2u] = 1u;
    p_dispatch_args[DISPATCH_EMIT_WORD + 3u] = 0u;
    p_dispatch_args[DISPATCH_SIM_WORD  + 0u] = (alive_cur + (LOCAL_SIZE - 1u)) >> LOCAL_SIZE_LOG2;
    p_dispatch_args[DISPATCH_SIM_WORD  + 1u] = 1u;
    p_dispatch_args[DISPATCH_SIM_WORD  + 2u] = 1u;
    p_dispatch_args[DISPATCH_SIM_WORD  + 3u] = 0u;

    // Both indirect DRAW slots, reset for this frame. `indexCount` is written FIRST and
    // `instanceCount` is zeroed here because it is ALSO the sim's per-class render counter (plan
    // M1/M2): the `InterlockedAdd` that yields a lane's render position leaves the class's final
    // count behind, so the command processor reads a live count with no finish pass (closing R9).
    //
    // `firstInstance` is 0 in BOTH slots and must stay so -- F5b: `drawIndirectFirstInstance` is
    // not enabled on this device and a nonzero value there is a silent corruption class.
    p_draw_args[DRAW_ADDITIVE_BASE_WORD + 0u] = QUAD_INDEX_COUNT;  // indexCount
    p_draw_args[DRAW_ADDITIVE_INSTANCE_WORD]  = 0u;                // instanceCount
    p_draw_args[DRAW_ADDITIVE_BASE_WORD + 2u] = 0u;                // firstIndex
    p_draw_args[DRAW_ADDITIVE_BASE_WORD + 3u] = 0u;                // vertexOffset
    p_draw_args[DRAW_ADDITIVE_BASE_WORD + 4u] = 0u;                // firstInstance (F5b)
    p_draw_args[DRAW_ADDITIVE_BASE_WORD + 5u] = 0u;                // inter-command pad

    // The alpha slot is P2's. It is zeroed every frame at P0 -- an undeclared pass writes no
    // instances, so its `instanceCount` stays 0 and the command processor draws nothing from it.
    p_draw_args[DRAW_ALPHA_BASE_WORD + 0u] = QUAD_INDEX_COUNT;
    p_draw_args[DRAW_ALPHA_INSTANCE_WORD]  = 0u;
    p_draw_args[DRAW_ALPHA_BASE_WORD + 2u] = 0u;
    p_draw_args[DRAW_ALPHA_BASE_WORD + 3u] = 0u;
    p_draw_args[DRAW_ALPHA_BASE_WORD + 4u] = 0u;                   // firstInstance (F5b)
    p_draw_args[DRAW_ALPHA_BASE_WORD + 5u] = 0u;
}}
"#,
        alpha_base_word = ALPHA_INSTANCE_COUNT_OFFSET / 4 - 1,
    )
}

/// Assembles `particle_emit.comp.hlsl` — plan A3 / D8: zero global atomics, a `groupshared`
/// cooperative prefix load and an 8-step branchless binary search.
fn build_emit() -> String {
    let words = counter_words();
    let effects = effect_params_struct();
    let sim_rec = particle_sim_struct();
    let rng = emit::emit_hlsl_particle_rng();
    let spawn = emit::emit_hlsl_particle_spawn_state();
    let search_steps = MAX_EMITTERS.trailing_zeros();
    let search_half = MAX_EMITTERS / 2;
    format!(
        r#"// particle_emit.comp -- the GPU particle system's SPAWN pass
// (`docs/PARTICLES-PLAN.md` Rev 4, algorithm A3 / decision D8). `DispatchIndirect`, 256 threads.
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_particles`. The
// `// === GENERATED <name> BEGIN/END ===` spans are MACHINE-EMITTED from `boyko_shaderdsl`'s
// generic leaf bodies; a hand-edit of any of them fails
// `boyko_rhi_vulkan/tests/particle_edsl_sync.rs`.
//
// # ZERO global atomics, and why that is structural
//
// The one-thread kickoff already pre-decremented `dead_count` and pre-incremented
// `alive_count_cur`, publishing `dead_base` and `emit_append_base`. So lane `gid` computes BOTH
// of its indices arithmetically -- `slot = p_dead[dead_base + gid]`, `pos = emit_append_base +
// gid` -- and this pass performs no `InterlockedAdd` of any kind. Plan gate #14 asserts the
// committed module carries zero atomic opcodes, so a future edit that reaches for one reds here.
//
// # The prefix orders LANES ONLY (D8)
//
// `first_spawn` is a CPU prefix sum over the frame's emitters; the binary search below maps
// `gid -> emitter index` and nothing else. The slot comes from the dead list and the list
// position from the append base -- three independent indexings, none assuming structure in
// another. That is what makes the pass correct against a SHUFFLED `p_dead`, which is the state
// the dead stack is in from frame 2 onward.
//
// # No dead-stack race
//
// Emit only READS `p_dead`; only the sim pushes to it, in a later pass with a derived barrier
// between. The classic concurrent push/pop race is structurally impossible here (plan D3).
//
// # Compile (offline + hermetic; committed `.spv` is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 particle_emit.comp.hlsl -Fo particle_emit.comp.spv
//
// # Set / binding vocabulary -- MIRRORS the host `PARTICLE_LAYOUT_ENTRIES` table
//
//   (set, binding, kind)
//   (0, 0, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_counters    read
//   (0, 3, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_dead        read
//   (0, 4, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_alive_read  write
//   (0, 6, STORAGE_BUFFER)  RWStructuredBuffer<ParticleSim>     p_particle    write
//   (0, 8, STORAGE_BUFFER)  StructuredBuffer<EmitRequestGpu>    p_emit_req    read
//   (0, 9, STORAGE_BUFFER)  StructuredBuffer<EffectParamsGpu>   p_effects     read
//
// # Push constants (8 B)
//
//   [0,4)  uint emitter_count -- <= MAX_EMITTERS, the host's release-present clamp (plan D15)
//   [4,8)  uint frame_index   -- the per-frame half of the spawn seed; the per-emitter half is
//                               `EmitRequestGpu::rng_seed`, a CONSTANT (plan D2)

struct EmitPush {{
    uint emitter_count;
    uint frame_index;
}};
[[vk::push_constant]] EmitPush pc;

// The per-emitter spawn request (plan D2, 64 B). `first_spawn` is the CPU prefix sum (D8).
struct EmitRequestGpu {{
    float3 origin;  uint effect_index;
    float3 basis_x; uint spawn_count;
    float3 basis_y; uint first_spawn;
    float3 basis_z; uint rng_seed;
}};

{effects}
{sim_rec}
[[vk::binding(0, 0)]] RWStructuredBuffer<uint>              p_counters   : register(u0);
[[vk::binding(3, 0)]] RWStructuredBuffer<uint>              p_dead       : register(u3);
[[vk::binding(4, 0)]] RWStructuredBuffer<uint>              p_alive_read : register(u4);
[[vk::binding(6, 0)]] RWStructuredBuffer<ParticleSim>       p_particle   : register(u6);
[[vk::binding(8, 0)]] StructuredBuffer<EmitRequestGpu>      p_emit_req   : register(t8);
[[vk::binding(9, 0)]] StructuredBuffer<EffectParamsGpu>     p_effects    : register(t9);

{words}
// The `first_spawn` prefix lives in LDS for the search: one coalesced 1 KB load, one barrier,
// then eight LDS reads per lane instead of eight global ones.
static const uint MAX_EMITTERS   = {MAX_EMITTERS}u;
static const uint SEARCH_STEPS   = {search_steps}u;
static const uint SEARCH_HALF    = {search_half}u;
static const uint SPAWN_SENTINEL = {SPAWN_SENTINEL}u;

// The stored `(cos, sin)` pattern for zero rotation: `cos = +1` quantizes to 32767, `sin = 0` to
// 0 (plan D2's `rot_cs`). Every particle spawns unrotated.
static const uint ROT_IDENTITY = {ROT_IDENTITY}u;

groupshared uint gs_first_spawn[MAX_EMITTERS];

// === GENERATED particle_rng BEGIN ===
// The 32-bit PCG hash (`boyko_shaderdsl::particle::particle_rng_body`). Bit-exact by
// construction: pure integer arithmetic, identical on the CPU oracle and the device.
uint particle_rng(uint state) {{
{rng}}}
// === GENERATED particle_rng END ===

// === GENERATED particle_spawn_state BEGIN ===
// The trig-free, divide-free cone sample + speed/lifetime draw
// (`boyko_shaderdsl::particle::particle_spawn_state_body`): an elliptical grid mapping from the
// unit square onto the disc, then a Lambert azimuthal equal-area lift onto the cap `z >= cone_cos`
// whose unit length is an ALGEBRAIC identity rather than a normalization.
void particle_spawn_state(float3 basis_x, float3 basis_y, float3 basis_z,
                          float cone_cos, float speed_min, float speed_max,
                          float life_min, float life_max,
                          uint r_dir_x, uint r_dir_y, uint r_speed, uint r_life,
                          out float3 velocity, out float life) {{
{spawn}}}
// === GENERATED particle_spawn_state END ===

[numthreads({LOCAL_SIZE}, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID, uint lane : SV_GroupIndex) {{
    // The cooperative prefix load. EVERY lane of the group reaches the barrier -- the range guard
    // is BELOW it, because a `return` above a `GroupMemoryBarrierWithGroupSync` is undefined and
    // typically a device hang.
    //
    // The out-of-range arm reads `p_emit_req[lane]` eagerly if DXC lowers the ternary to a select
    // rather than a branch. That is IN BOUNDS: the request buffer holds MAX_EMITTERS entries and
    // `lane < MAX_EMITTERS` by the group width, so an eager load reads an indeterminate VALUE at a
    // valid address and the select discards it. (Group width == MAX_EMITTERS is what makes the
    // one-entry-per-lane load exact; both are generator inputs.)
    gs_first_spawn[lane] = (lane < pc.emitter_count) ? p_emit_req[lane].first_spawn
                                                     : SPAWN_SENTINEL;
    GroupMemoryBarrierWithGroupSync();

    uint gid = tid.x;
    if (gid >= p_counters[CTR_REAL_EMIT]) {{ return; }}

    // The 8-step BRANCHLESS binary search for the last emitter whose `first_spawn <= gid`.
    // `gs_first_spawn[0]` is 0 for a non-empty frame (a prefix sum starts at 0), so `lo` always
    // lands on a real emitter; the sentinel makes every slot past `emitter_count` unselectable.
    // `lo + (SEARCH_HALF >> s)` sums to MAX_EMITTERS-1 over the whole descent, so the probe index
    // is in range at every step and needs no bound test.
    uint lo = 0u;
    [unroll]
    for (uint s = 0u; s < SEARCH_STEPS; ++s) {{
        uint probe = lo + (SEARCH_HALF >> s);
        lo = (gs_first_spawn[probe] <= gid) ? probe : lo;
    }}

    EmitRequestGpu req = p_emit_req[lo];
    // `effect_index` is host-clamped below MAX_EFFECTS, which is the effect table's element count.
    EffectParamsGpu e = p_effects[req.effect_index];

    // The two independent indexings kickoff published (plan D8). Neither assumes anything about
    // the other, and neither assumes anything about the ORDER of the dead list.
    uint slot = p_dead[p_counters[CTR_DEAD_BASE] + gid];
    uint pos  = p_counters[CTR_EMIT_BASE] + gid;

    // The spawn seed mixes the emitter's CONSTANT `rng_seed`, the lane, and the frame -- so two
    // emitters spawning at the same lane on the same frame decorrelate, and one emitter's lane
    // decorrelates across frames. The chain then supplies four decorrelated words.
    uint r0 = particle_rng(req.rng_seed ^ gid ^ pc.frame_index);
    uint r1 = particle_rng(r0);
    uint r2 = particle_rng(r1);
    uint r3 = particle_rng(r2);

    float3 velocity;
    float  life;
    particle_spawn_state(req.basis_x, req.basis_y, req.basis_z, e.cone_cos,
                         e.speed_min, e.speed_max, e.lifetime_min, e.lifetime_max,
                         r0, r1, r2, r3, velocity, life);

    ParticleSim p;
    p.position        = req.origin;
    p.life_remaining  = life;
    p.velocity        = velocity;
    p.cached_field_d  = 0.0;
    p.color_rgba8     = e.color_keys.x;
    // The ONE divide in the whole subsystem's per-particle path, paid ONCE at spawn: the sim then
    // normalizes age by a MULTIPLY, so `particle_sim` carries no `OpFDiv` at all (plan gate #14).
    p.size0_invlife   = f32tof16(e.size_base) | (f32tof16(1.0 / life) << 16u);
    p.effect_flags    = req.effect_index | ((e.flags & 65535u) << 16u);
    p.rot_cs          = ROT_IDENTITY;

    p_particle[slot]    = p;
    p_alive_read[pos]   = slot;
}}
"#
    )
}

/// Assembles `particle_sim.comp.hlsl` — plan A4 / D3 / D5: the hot loop, the wave-level atomic
/// mechanism of the plan's NORMATIVE "Counter and list ownership" section.
fn build_sim() -> String {
    let words = counter_words();
    let effects = effect_params_struct();
    let sim_rec = particle_sim_struct();
    let render_rec = particle_render_struct();
    let integrate = emit::emit_hlsl_particle_integrate();
    let rot = emit::emit_hlsl_particle_rot_advance();
    let curve = emit::emit_hlsl_particle_curve_eval();
    let response = emit::emit_hlsl_particle_sdf_response();
    let additive_word = ADDITIVE_INSTANCE_COUNT_OFFSET / 4;
    let alpha_word = ALPHA_INSTANCE_COUNT_OFFSET / 4;
    format!(
        r#"// particle_sim.comp -- the GPU particle system's HOT LOOP
// (`docs/PARTICLES-PLAN.md` Rev 4, algorithm A4 / decisions D3, D5). `DispatchIndirect`, 256
// threads, O(alive_count_cur).
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_particles`. The
// `// === GENERATED <name> BEGIN/END ===` spans are MACHINE-EMITTED from `boyko_shaderdsl`'s
// generic leaf bodies; a hand-edit of any of them fails
// `boyko_rhi_vulkan/tests/particle_edsl_sync.rs`.
//
// # The wave-level atomic mechanism -- the plan's NORMATIVE block, symbol for symbol
//
//   w_count = WaveActiveCountBits(survives)      // survivors in THIS wave        (per-wave)
//   w_lane  = WavePrefixCountBits(survives)      // this lane's rank among them   (per-lane)
//   if (WaveIsFirstLane())
//       w_base_raw = InterlockedAdd(alive_count_next, w_count)     // returns the OLD value
//   w_base  = WaveReadLaneFirst(w_base_raw)      // the wave's LIST reservation base
//   idx     = w_base + w_lane                    // this lane's LIST position
//
// and, on EACH blend class's own RENDER counter (`p_draw_args`'s two `instanceCount` words, at the
// byte offsets the generator was fed) -- plan D5's own two-class form:
//
//   add_class = survives & !is_alpha              // BITWISE -- see the retirement block
//   r_count = WaveActiveCountBits(add_class)      // ADDITIVE survivors in THIS wave
//   r_lane  = WavePrefixCountBits(add_class)      // this lane's rank among them
//   q_count = w_count - r_count                   // ALPHA, by the partition (not a ballot)
//   q_lane  = w_lane  - r_lane
//   if (WaveIsFirstLane())
//       r_base_raw = InterlockedAdd(additive.instanceCount, r_count)   // if r_count > 0
//       q_base_raw = InterlockedAdd(alpha.instanceCount,    q_count)   // if q_count > 0
//   r_pos   = is_alpha ? (capacity - 1 - (q_base + q_lane))            // DOWN from the top
//                      : (r_base + r_lane)                            // UP from 0
//
// EXACTNESS: reservations from one `InterlockedAdd` counter are contiguous and disjoint, so the
// multiset of `(base, count)` pairs partitions `[0, sum)` exactly and the counter's final value is
// the sum -- independent of the order in which waves retire, because integer addition is
// commutative. That is why NO `InterlockedMax` and no mirror is needed (plan M1), and plan gate
// #14 asserts this module carries ZERO `OpAtomicUMax`.
//
// # The BLEND PARTITION (rung P2, plan D10/M2)
//
// `firstInstance` must be 0 in both draw slots (F5b), so the two classes cannot be distinguished by
// it. They are distinguished by WHERE IN `p_render` they are written, and the VS reads back with a
// push-constant affine (`index_base + index_step * SV_InstanceID`):
//
//   LIST index   -- SHARED. Every survivor of EITHER class takes `idx` from `alive_count_next`.
//                   This is what makes an alpha leak structurally impossible: kickoff reads only
//                   that counter, so a class allocating its list position anywhere else would
//                   vanish from the next frame's walk entirely.
//   RENDER index -- PER CLASS. Additive counts UP from 0; alpha counts DOWN from `capacity - 1`.
//                   The two ends share one buffer dynamically, so neither class carries a capacity
//                   cap of its own, and A5's sequential read is preserved in both directions.
//
// The ALPHA ballots are NOT taken: they are DERIVED. `q_count = w_count - r_count` and
// `q_lane = w_lane - r_lane` are exact, because the surviving lanes of a wave partition into the
// two classes and both prefix counts are over the same lane order. Two wave ops, not four.
//
// THE DERIVATION IS ALSO ROBUST UNDER NON-RECONVERGENCE, which is a stronger property than
// exactness and is written down so it is not rediscovered. Suppose the wave reaches these ballots
// already split into divergent groups. Each ballot then sees only its own group -- but `add_class`
// is FALSE on precisely the lanes a divergence here could have removed (it implies `survives`), so
// `w_count - r_count` and `w_lane - r_lane` are differences of ballots over the SAME active set,
// and each is still exactly the alpha count / rank within that set. The subtraction can never mix
// two different masks. It is the ELECTION that a split would corrupt, never this arithmetic --
// which is why the guard below is about keeping the BLOCK whole rather than about the operands.
//
// # The atomic budget, per WAVE (plan D5, WIDENED at rung P2 -- see below)
//
//   all dying                       -> 1   (dead_count)
//   all surviving, ONE class        -> 2   (alive_count_next, that class's instanceCount)
//   all surviving, BOTH classes     -> 3   (+ the other class's instanceCount)
//   mixed survive/die               -> +1  (dead_count)                       ==> 4 at worst
//
// ⚠️ THE UPPER BOUND MOVED FROM 3 TO 4 AT RUNG P2, and that is a DELIBERATE re-bless, not a drift:
// D10's partition specifies one `InterlockedAdd` counter PER CLASS, so a wave carrying survivors of
// both classes reserves on both. It is not a widening of the aggregation -- the ops are still ONE
// per wave per counter, never one per lane. The common case is still 2-3: the alive list groups
// particles spawned together, hence of one emitter, hence of one effect, hence of ONE class, so a
// mixed-class wave only occurs where two effects' spawns interleave in the list.
//
// At 1M survivors that is ~62 500 ops ~= 32 us, against ~0.5 ms for the naive per-lane form.
//
// The `-D SDF_COLLIDE_STATS` module DELIBERATELY EXCEEDS that budget -- 1-2 more per wave per
// substep, i.e. **3-6** per wave at the plan's steady state (one substep): `2 + 1` for an
// all-surviving single-class wave whose every lane skips the field, `4 + 2` for a mixed
// survive/die wave carrying both classes that evaluates. Rung P2 moved its UPPER bound only,
// 5 -> 6 -- the lower is still 3, because an additive-only wave still retires in two atomics.
// Statically the module carries 7 `OpAtomicIAdd` SITES, which is a different quantity from the
// per-wave count and is stated separately wherever both appear.
//
// Its manifest row states the exception rather than the census being widened to accommodate it: a
// census that forbids the instrument is a census that forbids measuring itself, and a widened
// bound would stop gating the two modules that ship.
//
// # Two counters, not one (plan N3c)
//
// The LIST count lives in `p_counters` (whose frame terminal is an undrained compute write, so
// next frame's kickoff gets a real RAW) and the RENDER counts in `p_draw_args` (whose terminal is
// the indirect fetch, so next frame's kickoff gets a real WAR). `ResSync` cannot express both on
// one resource. They are also genuinely different numbers the moment a second blend class exists.
//
// # The substep loop is WAVE-UNIFORM
//
// `pc.steps` is a push constant, already clamped ONCE on the host (plan M3), so every lane runs
// the same trip count and the loop never diverges. The `min` below is the F25 HANG GUARD against
// a corrupt push constant and nothing else -- it cannot bind on a well-formed frame.
//
// # No `OpFDiv` in this module
//
// Plan gate #14 requires the `particle_rot_advance` span to carry no divide. DXC inlines every
// helper into `%main`, so a per-function opcode scan of the artifact is UNREACHABLE -- there is
// one `OpFunction` header. The decidable form of that requirement is therefore MODULE-WIDE, and
// this module is written to satisfy it: the age normalization multiplies by the reciprocal
// lifetime `particle_emit` stored at spawn, and the group/index math is integer.
//
// The claim is about the BASE artifact. The `-D SDF_COLLIDE` variant `#include`s the frozen
// `sdf_field.hlsli`, whose `smin`/`sd_capsule` carry divides of their own; those are the FIELD's
// bytes, byte-shared with the marcher and the host oracle, and re-spelling them here to dodge an
// opcode count would fork the determinism contract that header exists to hold. Everything the
// PARTICLE side adds under that define is multiply-only -- which is the decidable claim
// `particle_edsl_sync` pins for the variant.
//
// # The `-D SDF_COLLIDE` variant (rung P1, plan D9)
//
// Off by default and STRUCTURALLY absent: with the define undefined DXC never sees the field
// binding, the include, the response leaf or the in-loop block, so the base `.spv` is byte-frozen
// and there is no dark tax to measure (plan F24). Armed, each substep either SKIPS the field or
// evaluates it once:
//
//   travel_l = length(vel) * dt * FIELD_LIPSCHITZ_L      // the most the field value can drop
//   if (cached_d - travel_l > radius_l)  cached_d -= travel_l;          // no evaluation
//   else                                 d = field_distance(pos);       // one evaluation
//                                        if (d < radius) resolve;
//                                        cached_d = d;
//
// # Why the Lipschitz constant MULTIPLIES the travel rather than dividing it
//
// `FIELD_LIPSCHITZ_L` is the worst-case |grad| of `field_distance` (`sdf_field.hlsli`: the IQ
// polynomial smin's blend peaks at sqrt(2) where two unit-gradient fields meet at 90 degrees).
// From |grad f| <= L: f(p + s) >= f(p) - L*s. So a move of `s` world units can cost up to `L*s`
// of REPORTED distance, and the conservative decrement is `travel * L`. Equivalently, in the
// euclidean units the header states its own rule in ("a cone-trace consumer divides the reported
// distance by L"), the test is `cached_d/L - travel > radius`; both sides are scaled by L here so
// every per-substep operation stays a multiply.
//
// Plan D9's one-line pseudocode writes the reciprocal form (`speed*timestep/L`). The two agree
// EXACTLY at L == 1 -- hard-CSG scenes, which is every fixture today -- and diverge only where a
// smooth (k > 0) edit makes the field super-Lipschitz, where the reciprocal form OVER-estimates
// the clearance and can skip a substep in which contact happened. This module implements the
// conservative direction, because the alternative is a tunneling class that no image gate sees.
//
// # The `-D SDF_COLLIDE_STATS` variant (rung P1b) -- the SKIP-RATE INSTRUMENT
//
// Gate #17 measured that the `ZONE_PARTICLE_SIM` armed-vs-disarmed delta, which the plan had named
// as the skip-rate instrument, is DOMINATED by a kernel-level term of the OPPOSITE SIGN at 4-6x the
// row's resolution: at 65 536 alive a strict superset of work runs 20.3% FASTER, and the isolated
// module swap alone is -5.6%. So the skip rate is not recoverable from a timing difference, and
// this variant counts it on the device instead.
//
// It is a THIRD COMPILED MODULE and not a runtime flag, for F24's measured reason (the VB-SV0
// inline detour cost +75% with its feature OFF and no byte gate could see it) and D1's `-D`
// precedent: a runtime-gated atomic span would be paid on every disarmed frame. With the define
// undefined DXC never sees the census, so BOTH shipping modules stay byte-frozen.
//
// The census is D5's wave aggregation VERBATIM -- one ballot taken where the wave is converged,
// folded to ONE `InterlockedAdd` by ONE lane -- and it reads the branch's OWN predicate, emitted
// from one generator input so the two spellings cannot drift.
//
// # Compile (offline + hermetic; committed `.spv` is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 particle_sim.comp.hlsl -Fo particle_sim.comp.spv
//   (SDF_COLLIDE variant: add `-D SDF_COLLIDE=1` -Fo particle_sim_sdf.comp.spv)
//   (SDF_COLLIDE_STATS variant: add `-D SDF_COLLIDE=1 -D SDF_COLLIDE_STATS=1` -Fo particle_sim_stats.comp.spv)
//
// # Set / binding vocabulary -- MIRRORS the host `PARTICLE_LAYOUT_ENTRIES` table
//
//   (set, binding, kind)
//   (0, 0, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_counters     read+write (atomic)
//   (0, 2, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_draw_args    read+write (atomic)
//   (0, 3, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_dead         write
//   (0, 4, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_alive_read   read
//   (0, 5, STORAGE_BUFFER)  RWStructuredBuffer<uint>            p_alive_write  write
//   (0, 6, STORAGE_BUFFER)  RWStructuredBuffer<ParticleSim>     p_particle     read+write
//   (0, 7, STORAGE_BUFFER)  RWStructuredBuffer<ParticleRender>  p_render       write
//   (0, 9, STORAGE_BUFFER)  StructuredBuffer<EffectParamsGpu>   p_effects      read
//   (0, {SDF_FIELD_BINDING}, STORAGE_BUFFER) StructuredBuffer<uint>             Buf            read  [SDF_COLLIDE only]
//
// # Push constants (12 B) -- UNCHANGED by the variant
//
//   [0,4)   uint  steps    -- the host-clamped substep count (plan M3); ONE number, two consumers
//   [4,8)   float timestep -- `ParticleClock`'s CONSTANT dt (plan D6)
//   [8,12)  uint  capacity -- CAP, the boot-frozen pool size (plan D14); the ALPHA class's render
//                             index mirror `capacity - 1 - q_pos`. The SAME value `particle_kickoff`
//                             is pushed and the SAME `ParticleGpuBundle::capacity` the VS's
//                             `index_base` is derived from -- one home, three consumers.
//
// The collision tuning is NOT pushed: `collision_radius`, `restitution` and `friction` are
// PER-EFFECT (plan D2's `EffectParamsGpu` already carries all three), so they arrive through the
// row the sim already fetched and the two pipelines share one push range and one layout.

struct SimPush {{
    uint  steps;
    float timestep;
    uint  capacity;
}};
[[vk::push_constant]] SimPush pc;

{effects}
{sim_rec}
{render_rec}
[[vk::binding(0, 0)]] RWStructuredBuffer<uint>              p_counters    : register(u0);
[[vk::binding(2, 0)]] RWStructuredBuffer<uint>              p_draw_args   : register(u2);
[[vk::binding(3, 0)]] RWStructuredBuffer<uint>              p_dead        : register(u3);
[[vk::binding(4, 0)]] RWStructuredBuffer<uint>              p_alive_read  : register(u4);
[[vk::binding(5, 0)]] RWStructuredBuffer<uint>              p_alive_write : register(u5);
[[vk::binding(6, 0)]] RWStructuredBuffer<ParticleSim>       p_particle    : register(u6);
[[vk::binding(7, 0)]] RWStructuredBuffer<ParticleRender>    p_render      : register(u7);
[[vk::binding(9, 0)]] StructuredBuffer<EffectParamsGpu>     p_effects     : register(t9);

#ifdef SDF_COLLIDE
// Rung P1 (plan D9). `sdf_field.hlsli`'s INCLUDE CONTRACT requires `Buf` to be declared and in
// scope BEFORE the include -- the field eval reads the packed edit-list header out of it. This
// pass is a strict FIELD-CONSUMER: it CALLS `field_distance`/`sdf_normal` read-only and never
// edits, exactly as `sdf_mesh_shadow.comp.hlsl` does, and it binds the list at the same binding
// number that pass gives its own `Buf`.
//
// The buffer is the engine's ONE edit list -- boot-static and read-only for the whole present loop
// (the same contract the marcher relies on), which is why it needs no framegraph `ResId`, no seed
// row and no barrier, and why arming this variant moves no derived barrier stream.
[[vk::binding({SDF_FIELD_BINDING}, 0)]] StructuredBuffer<uint> Buf : register(t0);
#include "sdf_field.hlsli"
#endif

{words}
// The two classes' RENDER counters, at the words the generator derived from
// `PARTICLE_{{ADDITIVE,ALPHA}}_INSTANCE_COUNT_OFFSET` ({ADDITIVE_INSTANCE_COUNT_OFFSET} and {ALPHA_INSTANCE_COUNT_OFFSET} bytes) -- plan gate #8. The
// `InterlockedAdd` on each yields BOTH this lane's render position within its class and, at
// retirement, that class's final instance count -- which is the field the command processor fetches,
// so there is no finish pass (closing R9).
static const uint DRAW_ADDITIVE_INSTANCE_WORD = {additive_word}u;
static const uint DRAW_ALPHA_INSTANCE_WORD    = {alpha_word}u;

// `boyko_render::PARTICLE_BLEND_ALPHA` -- the `blend_class` discriminant that sends a survivor to
// the TOP end of `p_render` (plan D10). Tested for alpha rather than for additive on purpose: a
// future third class falls to the additive arm, which needs no sort, rather than joining the sorted
// one by default.
static const uint BLEND_CLASS_ALPHA = {PARTICLE_BLEND_ALPHA}u;

#ifdef SDF_COLLIDE_STATS
// Rung P1b's three census words, at the indices the generator derived from
// `offset_of!(ParticleCounters, ...) / 4`. They were CARVED OUT OF THE PAD of the same 64-byte
// counter line, so no shipping counter moved to make room.
//
// Declared INSIDE the define on purpose: with `SDF_COLLIDE_STATS` undefined DXC never sees these
// three names, which is the same structural absence that keeps `particle_sim.comp.spv` and
// `particle_sim_sdf.comp.spv` byte-frozen across this rung.
//
// They ACCUMULATE from boot and are never cleared -- `particle_kickoff` is ONE module for all three
// sim variants and does not know about them. That is deliberate: the quantity is a RATIO, which is
// frame-count independent, and a per-frame reset would put a writer for a measurement word into a
// shipping shader.
static const uint CTR_WAVES_EVALUATED = {CTR_WAVES_EVALUATED}u;
static const uint CTR_WAVES_SKIPPED   = {CTR_WAVES_SKIPPED}u;
static const uint CTR_LANES_EVALUATED = {CTR_LANES_EVALUATED}u;
#endif

// `PARTICLE_SUBSTEP_CEILING` (plan M3). The HOST already clamped; this is the F25 hang guard.
static const uint SUBSTEP_CEILING = {SUBSTEP_CEILING}u;

// === GENERATED particle_integrate BEGIN ===
// One explicit-Euler substep (`boyko_shaderdsl::particle::particle_integrate_body`). `damping` is
// host-precomputed against the constant timestep (plan D6), which is what deletes `exp2` here.
void particle_integrate(inout float3 pos, inout float3 vel, inout float life,
                        float3 gravity, float damping, float dt) {{
{integrate}}}
// === GENERATED particle_integrate END ===

// === GENERATED particle_rot_advance BEGIN ===
// The rotation advance (`boyko_shaderdsl::particle::particle_rot_advance_body`): a complex
// multiply against the host-precomputed `(cos w*dt, sin w*dt)` f32 pair, re-quantized to snorm16
// with NO renormalization and NO divide (plan M7/K1).
uint particle_rot_advance(uint rot_cs, float mul_cos, float mul_sin) {{
{rot}}}
// === GENERATED particle_rot_advance END ===

// === GENERATED particle_curve_eval BEGIN ===
// The branch-free 4-key ramp over two packed binary16 pairs
// (`boyko_shaderdsl::particle::particle_curve_eval_body`). Evaluated ONCE per particle here
// rather than 4x per particle in the VS -- the larger half of D2's two-record win.
float particle_curve_eval(uint keys_lo, uint keys_hi, float t) {{
{curve}}}
// === GENERATED particle_curve_eval END ===

#ifdef SDF_COLLIDE
// === GENERATED particle_sdf_response BEGIN ===
// Rung P1's contact resolution (`boyko_shaderdsl::particle::particle_sdf_response_body`): plan
// D9's `p += n*(radius - d)` and `v' = (v - v_n)(1 - friction) - v_n*restitution`, where `v_n` is
// the velocity's INWARD component along the field normal. Pure multiply/add plus the one `dot`
// rung E3 added for it, and the `min(., 0.0)` SIGN GATE: on a re-contact frame the particle is
// already moving outward, and un-gated the `-v_n*restitution` term would flip that back inward --
// at restitution 1, a particle inside the shell could never escape. Branchless, one instruction,
// on the rare `d < radius` arm only.
void particle_sdf_response(inout float3 pos, inout float3 vel, float3 normal,
                           float d, float radius, float restitution, float friction) {{
{response}}}
// === GENERATED particle_sdf_response END ===
#endif

[numthreads({LOCAL_SIZE}, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {{
    uint i = tid.x;
    // The SAME field kickoff sized this dispatch from -- one value read twice, never two
    // derivations that have to be kept in agreement. Lanes past it retire BEFORE any wave
    // intrinsic, so they contribute to no ballot.
    if (i >= p_counters[CTR_ALIVE_CUR]) {{ return; }}

    uint slot = p_alive_read[i];
    ParticleSim p = p_particle[slot];
    EffectParamsGpu e = p_effects[p.effect_flags & 65535u];

    float3 pos  = p.position;
    float3 vel  = p.velocity;
    float  life = p.life_remaining;
    uint   rot  = p.rot_cs;

    uint steps = min(pc.steps, SUBSTEP_CEILING);
#ifdef SDF_COLLIDE
    // The Lipschitz cache rides the record's `.w` lane (plan D2). `particle_emit` seeds it 0, so a
    // particle's FIRST substep always evaluates the field and the cache is never read stale.
    float cached_d = p.cached_field_d;
    float radius   = e.collision_radius;
    // The skip test `cached_d/L - travel > radius`, with BOTH sides scaled by L once, here,
    // outside the loop -- `radius` and L are constants for this particle's whole step sequence, so
    // the per-substep work is a multiply and a compare (see the header for the bound's derivation).
    float radius_l = radius * FIELD_LIPSCHITZ_L;
#endif
    [loop]
    for (uint s = 0u; s < steps; ++s) {{
        particle_integrate(pos, vel, life, e.gravity, e.damping, pc.timestep);
        rot = particle_rot_advance(rot, e.rot_mul_cos, e.rot_mul_sin);
#ifdef SDF_COLLIDE
        // `particle_integrate` advanced the position by EXACTLY `vel * dt` (the post-damping
        // velocity it just wrote), so this is the displacement itself, not an estimate of it.
        float travel_l = length(vel) * pc.timestep * FIELD_LIPSCHITZ_L;
#ifdef SDF_COLLIDE_STATS
        // RUNG P1b'S SKIP-RATE CENSUS -- the instrument gate #17 proved a timing delta cannot be.
        //
        // D5's wave aggregation, VERBATIM: one ballot, folded to ONE `InterlockedAdd` by ONE lane,
        // with the `> 0u` guard that keeps the count minimal -- the same three moves the retirement
        // block below makes.
        //
        // ⚠️ PRECONDITION: EXACTLY ONE SUBSTEP PER DISPATCH. The wave is converged HERE only on the
        // FIRST iteration. From the second on, this point is reached from the previous iteration's
        // DIVERGENT skip branch, and Vulkan guarantees no reconvergence at a merge block without
        // `VK_KHR_shader_maximal_reconvergence` -- so a still-split wave would elect one leader PER
        // DIVERGENT GROUP and count the same wave-substep more than once, which destroys the
        // denominator `waves_skipped + waves_evaluated` is supposed to be.
        //
        // (Wave-uniform trip count and pre-ballot retirement of out-of-range lanes are both true
        // and NEITHER covers this: uniform iteration counts say nothing about whether the lanes
        // are executing that iteration together.)
        //
        // The host REFUSES to record a census frame at `steps != 1`
        // (`gpu_scene::particle::assert_one_substep_for_the_census`), so this precondition is
        // enforced rather than assumed.
        //
        // Read at WAVE granularity, and the two wave counters are EXCLUSIVE: the skip is a divergent
        // branch, so a wave whose lanes disagree executes BOTH sides and PAID the walk. It therefore
        // counts as EVALUATED, never as both, which is what makes `skipped + evaluated` the
        // wave-substep total and the skip rate a ratio that needs no fourth counter.
        //
        // `lanes_evaluated` is the per-LANE numerator beside it. The gap between the two rates is
        // exactly the wave's incoherence -- the quantity the plan says a per-lane figure hides.
        //
        // The predicate is the branch's OWN, emitted from one generator input (`SDF_SKIP_TEST`):
        // a census counting a decision the shader did not make would be worse than no census.
        uint eval_lanes = WaveActiveCountBits(!({SDF_SKIP_TEST}));
        if (WaveIsFirstLane()) {{
            if (eval_lanes > 0u) {{
                InterlockedAdd(p_counters[CTR_WAVES_EVALUATED], 1u);
                InterlockedAdd(p_counters[CTR_LANES_EVALUATED], eval_lanes);
            }} else {{
                InterlockedAdd(p_counters[CTR_WAVES_SKIPPED], 1u);
            }}
        }}
#endif
        if ({SDF_SKIP_TEST}) {{
            // THE SKIP. The Lipschitz bound proves the collision shell cannot have been reached,
            // so this LANE evaluates no field this substep.
            //
            // THE SAVING IS REALIZED PER WAVE, NOT PER LANE, and the distinction is the whole
            // measurement: this is a DIVERGENT branch, so a wave whose lanes disagree executes
            // BOTH sides and every lane in it pays the ~240-flop edit-list walk. One near particle
            // costs its wave the entire saving. The cache therefore pays off exactly to the degree
            // that nearness is wave-COHERENT -- which the alive-list gather makes the common case
            // (neighbouring list entries are particles spawned together, hence spatially close),
            // but which is a property of the scene and not of this code. A skip rate quoted per
            // lane would overstate the win by the wave's incoherence -- MEASURED at rung P1b:
            // 42.1% per wave against 63.2% per lane on the lab fixture.
            //
            // READ IT OFF THE `-D SDF_COLLIDE_STATS` CENSUS ABOVE, never off a timing delta. This
            // line used to name the `ZONE_PARTICLE_SIM` armed-vs-disarmed delta; gate #17 measured
            // that delta to be dominated by a kernel-level term of the OPPOSITE SIGN at 4-6x the
            // row's own resolution, so it cannot see this branch at all.
            cached_d = cached_d - travel_l;
        }} else {{
            float d = field_distance(pos);
            if (d < radius) {{
                // `sdf_normal` is the frozen central-difference gradient -- SIX more field
                // evaluations, and the reason it is inside the contact branch rather than beside
                // the distance fetch.
                float3 n = sdf_normal(pos);
                particle_sdf_response(pos, vel, n, d, radius, e.restitution, e.friction);
            }}
            // The PRE-response value, exactly as plan D9 caches it: after a resolve the particle
            // sits ON the shell, so the next substep must re-evaluate rather than trust a bound
            // taken from where it used to be.
            cached_d = d;
        }}
#endif
    }}

    bool survives = (life > 0.0);
    // The BLEND CLASS is per EFFECT, so it is already in the row this lane fetched -- no extra
    // load, no per-particle bit and no second table (plan D10/M2).
    bool is_alpha = (e.blend_class == BLEND_CLASS_ALPHA);
    // ⚠️ BITWISE `&`, NOT `&&`, AND THAT IS A SOUNDNESS REQUIREMENT RATHER THAN A STYLE CHOICE.
    //
    // `&&` SHORT-CIRCUITS, and DXC lowers a short-circuit into control flow: an `OpSelectionMerge`
    // + `OpBranchConditional` region around the second operand. MEASURED on the first cut of this
    // rung: two such regions appeared BETWEEN the first ballot and `OpGroupNonUniformElect`, which
    // splits the single basic block the retirement block's whole correctness argument rests on --
    // the SAME structure the substep-census block 80 lines above refuses to sit in, and the reason
    // rung P1b's census carries a HOST-SIDE hard refusal instead of a comment.
    //
    // What it would have cost, on a MIXED SURVIVE/DIE wave with no reconvergence guarantee (Vulkan
    // promises none at a merge block without `VK_KHR_shader_maximal_reconvergence`): the wave
    // arrives at the election in TWO divergent groups, each electing its own leader, and each
    // leader reads the CONVERGED full-wave `w_count`/`d_count` computed before the split. So
    // `alive_count_next` and `dead_count` are each advanced TWICE -- B3's `N + D == CAP` breaks and
    // the next kickoff walks list entries nothing wrote -- and, worse because it is silent, the
    // dying group's `r_count` is 0, so its `q_count = w_count - 0 > 0` and it adds `w_count` to
    // `alpha.instanceCount` IN AN ADDITIVE-ONLY SCENE, pointing the alpha draw at a never-written
    // region of `p_render`.
    //
    // No gate in this tree would have caught it: `LAB_LIFETIME` is 8 s against ~30 ms of virtual
    // time, so NOTHING retires in any fixture leg (`dead_count == 0` in every readback row) and a
    // divergent wave never occurs. The artifact pin below is what makes this decidable instead.
    //
    // `&` evaluates both operands unconditionally, so the block stays whole -- pinned at the
    // artifact by `the_retirement_ballots_and_the_election_share_one_basic_block`.
    bool add_class = survives & !is_alpha;

    // Every wave quantity is computed BEFORE either branch, so the ballots see the full set of
    // in-range lanes exactly as the plan's NORMATIVE block describes them.
    uint w_count = WaveActiveCountBits(survives);
    uint w_lane  = WavePrefixCountBits(survives);
    uint d_count = WaveActiveCountBits(!survives);
    uint d_lane  = WavePrefixCountBits(!survives);
    // The ADDITIVE class's own ballots -- the only extra pair this rung takes.
    uint r_count = WaveActiveCountBits(add_class);
    uint r_lane  = WavePrefixCountBits(add_class);
    // ...and the ALPHA class's BY SUBTRACTION, which is exact rather than an approximation: the
    // surviving lanes of a wave partition into the two classes, and both prefix counts run over the
    // same lane order, so the lanes counted by `w_lane` and not by `r_lane` are exactly the alpha
    // survivors ahead of this one. Two wave ops saved per wave over ballotting the complement.
    uint q_count = w_count - r_count;
    uint q_lane  = w_lane - r_lane;

    uint w_base_raw = 0u;
    uint r_base_raw = 0u;
    uint q_base_raw = 0u;
    uint d_base_raw = 0u;
    if (WaveIsFirstLane()) {{
        // The `> 0u` guards are what keep a SINGLE-CLASS surviving wave at TWO atomics and an
        // all-dying wave at ONE (plan D5's budget as widened at P2 -- see the header), rather than
        // four unconditional ones. A wave that carries no alpha survivor -- which is every wave of
        // every additive-only scene, i.e. every image pin shipped before this rung -- issues
        // EXACTLY the same atomics in the same order it did at P0.
        if (w_count > 0u) {{
            InterlockedAdd(p_counters[CTR_ALIVE_NEXT], w_count, w_base_raw);
        }}
        if (r_count > 0u) {{
            InterlockedAdd(p_draw_args[DRAW_ADDITIVE_INSTANCE_WORD], r_count, r_base_raw);
        }}
        if (q_count > 0u) {{
            InterlockedAdd(p_draw_args[DRAW_ALPHA_INSTANCE_WORD], q_count, q_base_raw);
        }}
        if (d_count > 0u) {{
            InterlockedAdd(p_counters[CTR_DEAD_COUNT], d_count, d_base_raw);
        }}
    }}
    // Broadcast OUTSIDE the leader branch -- every lane needs the reservation base.
    uint w_base = WaveReadLaneFirst(w_base_raw);
    uint r_base = WaveReadLaneFirst(r_base_raw);
    uint q_base = WaveReadLaneFirst(q_base_raw);
    uint d_base = WaveReadLaneFirst(d_base_raw);

    if (!survives) {{
        // The dying path returns the slot to the free list at its wave-aggregated position.
        p_dead[d_base + d_lane] = slot;
        return;
    }}

    p.position       = pos;
    p.velocity       = vel;
    p.life_remaining = life;
    p.rot_cs         = rot;
#ifdef SDF_COLLIDE
    // The cache is per-particle STATE, not a scratch value: carrying it across the frame edge is
    // what lets a distant particle skip the field for many frames, not merely many substeps.
    p.cached_field_d = cached_d;
#endif
    p_particle[slot] = p;

    float size0    = f16tof32(p.size0_invlife & 65535u);
    float inv_life = f16tof32(p.size0_invlife >> 16u);
    // Normalized age in [0,1]. `inv_life` is the RECIPROCAL emit stored at spawn, so this is a
    // multiply and this module stays free of `OpFDiv` (see the header).
    float age  = 1.0 - life * inv_life;
    float size = size0 * particle_curve_eval(e.size_keys.x, e.size_keys.y, age);

    // The LIST position (shared by both blend classes -- this is what makes an alpha leak
    // structurally impossible, plan M2) and the class-dense RENDER position.
    uint idx   = w_base + w_lane;
    // The two classes grow toward each other from opposite ends of ONE buffer: additive from 0
    // upward, alpha from `capacity - 1` downward. They cannot collide, because
    // `additive.instanceCount + alpha.instanceCount == alive_count_next <= capacity` -- the M2
    // identity plan gate #7 reads back. The VS walks each range with the matching
    // `(index_base, index_step)` push pair, `(0, +1)` and `(capacity - 1, -1)`, so BOTH reads stay
    // sequential in their own direction (plan A5/D10).
    //
    // The `min` is the F25 guard on the MIRROR, and the asymmetry is why it is here and not on the
    // additive arm: over `uint`, an alpha reservation past `capacity` makes `capacity - 1 - q_pos`
    // UNDERFLOW to ~0xFFFFFFFF -- an unbounded out-of-range store with `robustBufferAccess` OFF,
    // i.e. undefined behaviour rather than a clamp. The additive arm overshoots BOUNDEDLY (by at
    // most the overshoot itself) and needs no guard to stay in the same class of wrong. One
    // instruction, on a path that can only be reached if the M2 identity is already broken.
    uint q_pos = min(q_base + q_lane, pc.capacity - 1u);
    uint r_pos = is_alpha ? (pc.capacity - 1u - q_pos) : (r_base + r_lane);

    p_alive_write[idx] = slot;

    ParticleRender r;
    r.position    = pos;
    r.size        = size;
    r.color_rgba8 = p.color_rgba8;
    r.rot_cs      = rot;
    r.tex_index   = e.tex_index;
    r.flags       = p.effect_flags >> 16u;
    p_render[r_pos] = r;
}}
"#
    )
}

/// The `VsOut` interface — printed into BOTH draw stages from ONE source.
///
/// The two declarations must stay character-identical: SPIR-V matches a VS output to an FS input
/// by LOCATION, which DXC assigns in declaration order, so a field that drifted in one file would
/// silently re-wire an interpolant rather than fail to compile.
///
/// The `DEPTH_LINEAR` tail is the Deferred variant's (plan D7 / the P0 live-fire erratum):
/// `eye_rel` is `cam_eye.xyz - world`, forwarded as a PERSPECTIVE-CORRECT varying under the SAME
/// `WORLDDIST` semantic `gbuffer_mrt.vs.hlsl` uses — `cam_eye` is constant across the primitive,
/// so interpolating the difference reconstructs the true per-pixel `cam_eye - P`. `cam_mode`
/// selects the fragment's encode arm, mirroring that shader's `cam_eye.w` lane.
///
/// # Why `cam_mode` is NOT `nointerpolation`, though it is primitive-constant
///
/// Considered and rejected. It is one value for the whole frame, so `nointerpolation` would be
/// free-or-better on paper (one less interpolant slot to iterate). It is not taken because this
/// declaration's job is to be `gbuffer_mrt`'s depth interface, character for character: that
/// shader declares a plain `float cam_mode : CAMMODE` beside a plain `float3 eye_rel : WORLDDIST`,
/// and the `particle_edsl_sync` pin that compares the two shaders' depth expressions is only
/// meaningful while the INPUTS of those expressions are the same kind of varying. Interpolating a
/// constant is exact in either case (all three vertices carry identical bits), so the difference
/// is a slot, not a number — and a divergence here would be the first thing to re-derive if the
/// two encodes ever disagreed. If the slot is ever wanted back, change BOTH shaders together and
/// re-bless the `dlin` pair only.
fn vs_out_struct() -> &'static str {
    "struct VsOut {\n\
     \x20   float4 position  : SV_Position;\n\
     \x20   float2 uv        : TEXCOORD0;\n\
     \x20   nointerpolation float4 color : COLOR0;\n\
     \x20   nointerpolation uint tex_index : TEXIDX;\n\
     #ifdef DEPTH_LINEAR\n\
     \x20   float3 eye_rel   : WORLDDIST;   // cam_eye.xyz - world position (perspective-correct)\n\
     \x20   float  cam_mode  : CAMMODE;     // 0 = ortho, 1 = perspective\n\
     #endif\n\
     };\n"
}

/// Assembles `particle_draw.vs.hlsl` — plan A5's vertex half: a sequential render-record read and
/// the trig-free billboard expansion.
fn build_draw_vs() -> String {
    let render_rec = particle_render_struct();
    let vs_out = vs_out_struct();
    let corner = emit::emit_hlsl_particle_billboard_corner();
    format!(
        r#"// particle_draw.vs -- the GPU particle system's BILLBOARD EXPANSION
// (`docs/PARTICLES-PLAN.md` Rev 4, algorithm A5). One `DrawIndexedIndirect`, 4 vertices and 6
// indices per instance.
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_particles`. The
// `// === GENERATED particle_billboard_corner BEGIN/END ===` span is MACHINE-EMITTED; a hand-edit
// fails `boyko_rhi_vulkan/tests/particle_edsl_sync.rs`.
//
// # The render index is a PUSH-CONSTANT AFFINE, not `firstInstance`
//
// `firstInstance` MUST be 0 (F5b: `drawIndirectFirstInstance` is not enabled on this device and a
// nonzero value there is a silent corruption class), so the two blend classes P2 adds cannot be
// distinguished by it. Instead the VS computes `pc.index_base + pc.index_step * SV_InstanceID`:
// `(0, +1)` for additive -- the IDENTITY at P0, i.e. a strictly sequential read with no
// indirection -- and `(CAP-1, -1)` for P2's alpha class, which walks the same buffer from the
// far end. One pipeline, two push values, no shader variant.
//
// # No trig, no renormalization (plan M7)
//
// The rotation arrives as a stored `(cos, sin)` snorm16 pair the sim advanced by complex
// multiplication, so the corner placement is pure multiply/add. R9 is closed by construction:
// `instanceCount` is the sim's live survivor count, never CAP.
//
// # The `DEPTH_LINEAR` variant (the Deferred path's ONLY arm)
//
// Deferred's depth buffer does not hold hardware depth: `gbuffer_mrt.fs.hlsl` OVERWRITES it with
// the marcher-aligned euclidean encode. The projection that path hands this VS is the marcher's,
// whose `row2 == row3` pins `SV_Position.z` to exactly 1.0 for every vertex, so the projective
// depth this stage emits is meaningless there and `VK_COMPARE_OP_LESS` fails on every fragment --
// including over the cleared sky. `-D DEPTH_LINEAR` forwards `eye_rel` and `cam_mode` so the
// fragment can write the depth buffer's OWN encode through `SV_Depth`. No host-side matrix can
// substitute: `z_ndc` is a ratio of affine functions of the world position and a euclidean norm is
// not (`docs/PARTICLES-PLAN.md`, the P0 live-fire erratum).
//
// The three reverse-Z paths take the base compile and are byte-unperturbed by the define.
//
// # Compile (offline + hermetic; committed `.spv` is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T vs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 particle_draw.vs.hlsl -Fo particle_draw.vs.spv
//   (DEPTH_LINEAR variant: add `-D DEPTH_LINEAR=1` -Fo particle_draw_dlin.vs.spv)
//
// # Set / binding vocabulary -- MIRRORS the host `PARTICLE_LAYOUT_ENTRIES` table
//
//   (set, binding, kind)
//   (0, 0, STORAGE_BUFFER)  StructuredBuffer<ParticleRender>  p_render  read  (VERTEX)
//   (0, 1, UNIFORM_BUFFER)  cbuffer Camera                              read  (VERTEX)
//   (1, *, ...)                                               the bindless texture set the
//                                                             FRAGMENT half declares
//
// # Push constants (72 B, VERTEX stage; the graphics range is separate from the shared COMPUTE one)
//
//   [0,64)   float4x4 view_proj   -- the path's own view-projection rows
//   [64,68)  uint     index_base  -- 0 at P0 (additive); CAP-1 for P2's alpha slot
//   [68,72)  int      index_step  -- +1 at P0 (additive); -1 for P2's alpha slot

struct ParticleDrawPush {{
    float4x4 view_proj;
    uint     index_base;
    int      index_step;
}};
[[vk::push_constant]] ParticleDrawPush pc;

{render_rec}
[[vk::binding(0, 0)]] StructuredBuffer<ParticleRender> p_render : register(t0);

// binding 1: the extent/camera UNIFORM block -- the SAME 80-byte shape every other consumer
// declares. This pass reads `cam_right`/`cam_up` (the billboard basis), and under DEPTH_LINEAR
// also `cam_eye`/`camera_mode` -- the SAME `ViewUniform::camera_pos` the Deferred raster push
// carries at its own bytes [64,80), so the two eyes are one number.
[[vk::binding(1, 0)]] cbuffer Camera {{
    uint   count;
    uint   img_w_raw;
    uint   img_h_raw;
    uint   camera_mode;
    float4 cam_eye;
    float4 cam_forward;
    float4 cam_right;
    float4 cam_up;
}};

// The RGBA8 -> UNORM scale. A constant EXPRESSION, folded by DXC, so no divide reaches the module.
static const float UNORM_SCALE = 1.0 / 255.0;

#ifdef DEPTH_LINEAR
// `boyko_rhi_vulkan::compute::CAM_MODE_PERSPECTIVE`, mirrored (a generator input, not typed here).
static const uint CAM_MODE_PERSPECTIVE = {CAM_MODE_PERSPECTIVE}u;
#endif

{vs_out}
// === GENERATED particle_billboard_corner BEGIN ===
// The corner placement (`boyko_shaderdsl::particle::particle_billboard_corner_body`): decode the
// stored `(cos, sin)` pair, rotate and scale the corner offset, ride the camera basis.
void particle_billboard_corner(float3 center, float3 cam_right, float3 cam_up,
                               float cx, float cy, float size, uint rot_cs,
                               out float3 world_pos) {{
{corner}}}
// === GENERATED particle_billboard_corner END ===

VsOut main(uint vid : SV_VertexID, uint iid : SV_InstanceID) {{
    // The affine render index. At P0 this is the identity `0 + 1 * iid`, so the read is exactly
    // sequential and the record stream is consumed at ~80% cache-line utilization (plan D2).
    uint r_index = uint((int)pc.index_base + pc.index_step * (int)iid);
    ParticleRender r = p_render[r_index];

    // The quad corner, from the 6-index / 4-vertex index buffer's vertex ids: bit 0 selects +x,
    // bit 1 selects +y, so the four vertices are the corners of a unit quad centred on the
    // particle. The offsets are the leaf's INPUT, which keeps the vertex-id convention here and
    // out of the generated span.
    float cx = ((vid & 1u) != 0u) ?  0.5 : -0.5;
    float cy = ((vid & 2u) != 0u) ?  0.5 : -0.5;

    float3 world;
    particle_billboard_corner(r.position, cam_right.xyz, cam_up.xyz, cx, cy, r.size, r.rot_cs,
                              world);

    VsOut o;
    o.position = mul(pc.view_proj, float4(world, 1.0));
    // `cy` runs +y UP in world space while `v` runs DOWN in texture space, hence the flip.
    o.uv = float2(cx + 0.5, 0.5 - cy);
    // The packed RGBA8 the sim resolved, unpacked once per vertex. A record DECODE, in the same
    // class as the index arithmetic above -- no ramp math survives into this stage.
    o.color = float4((float)( r.color_rgba8        & 255u),
                     (float)((r.color_rgba8 >>  8u) & 255u),
                     (float)((r.color_rgba8 >> 16u) & 255u),
                     (float)((r.color_rgba8 >> 24u) & 255u)) * UNORM_SCALE;
    o.tex_index = r.tex_index;
#ifdef DEPTH_LINEAR
    // The two lanes the Deferred fragment's `SV_Depth` needs, and NOTHING else: the encode itself
    // lives in the fragment because the depth of a billboard's INTERIOR is not an affine function
    // of its corners' depths -- only the perspective-correct `eye_rel` is.
    o.eye_rel = cam_eye.xyz - world;
    o.cam_mode = (camera_mode == CAM_MODE_PERSPECTIVE) ? 1.0 : 0.0;
#endif
    return o;
}}
"#
    )
}

/// Assembles `particle_draw.fs.hlsl` — plan A5's fragment half: `color * tex[tex_index].Sample`,
/// composited additively by the pipeline's blend state.
fn build_draw_fs() -> String {
    let vs_out = vs_out_struct();
    format!(
        r#"// particle_draw.fs -- the GPU particle system's FRAGMENT half
// (`docs/PARTICLES-PLAN.md` Rev 4, algorithm A5). Unlit, additive, bindless-textured.
//
// GENERATED by `cargo run -p boyko_shaderdsl --features emit --bin emit_particles`. This stage
// carries NO generated span: it is one modulate and one sample, with no leaf-class math.
//
// # Additive is a PIPELINE state, not shader code
//
// The blend is `BlendState::ADDITIVE` on the pipeline (plan D7) with `depth_write = OFF` and
// `depth_test = ON`, so opaque geometry still occludes. Additive is commutative and, under the
// 8-bit saturation `lit` imposes, `sat(sat(x)+y) = min(1, x+y)` is order-independent -- which is
// why P0 ships UNSORTED, provably (plan D10).
//
// # Bindless, so ONE draw covers every effect
//
// `tex_index` is an index into the shared runtime-sized `Texture2D[]` table (the SAME layout
// object `gbuffer_mrt.fs.hlsl` binds at set 1), which is what closes R11 structurally: there is
// no per-effect batch key and no per-texture draw split. Slot 0 is the reserved error-texture
// slot, so an UNTEXTURED effect leaves `tex_index` at 0 and the sample is skipped entirely --
// the same `!= 0` gate every other bindless consumer in this tree uses.
//
// # The `DEPTH_LINEAR` variant -- the DEFERRED path's depth encode, and its early-Z cost
//
// Deferred's depth buffer holds neither hardware depth nor a projective one: `gbuffer_mrt.fs.hlsl`
// OVERWRITES it through `SV_Depth` with the marcher-aligned encode
//
//     depth = (cam_mode > 0.5) ? (length(eye_rel) / MESH_DEPTH_T_MAX) : position.z
//
// (`gbuffer_mrt.fs.hlsl:327`, `MESH_DEPTH_T_MAX = 64.0` at `:113`). The particle VS emits a
// PROJECTIVE `SV_Position.z` that the marcher matrix pins to exactly 1.0 (its `row2 == row3`), so
// on that path the base compile's fragments fail `VK_COMPARE_OP_LESS` everywhere, sky included.
// `-D DEPTH_LINEAR` writes the SAME two-arm expression, term for term, from the SAME eye
// (`ViewUniform::camera_pos`, reached here through the shared camera UBO instead of through the
// raster push) -- so a particle and a mesh fragment at one distance encode to one number.
//
// COST 1, accepted for this leg only: a fragment that writes `SV_Depth` cannot be early-Z tested,
// because the value the test needs does not exist until the shader has run. The billboards
// therefore pay full shading before the depth reject on Deferred. Bounded: the stage is one
// modulate and (at most) one bindless sample, the pipeline still writes NO depth
// (`depth_write = OFF`), and the three reverse-Z paths take the base compile and keep early-Z.
// `[earlydepthstencil]` is NOT an escape here -- it would test the interpolated 1.0, which is the
// defect this variant exists to remove.
//
// COST 2, a per-path DIVERGENCE rather than a tax: this encode's range IS the particle's far
// horizon on Deferred. Past `MESH_DEPTH_T_MAX` world units the quotient exceeds 1, the pipeline
// clamps the depth write to the [0,1] range, and `LESS` against any stored value (including the
// 1.0 clear over sky) then fails -- so Deferred particles vanish at 64 units while the three
// reverse-Z paths carry them to the camera's own far plane. It is the SAME horizon this path's
// raster meshes already have (they are encoded by the same divisor), which is why the number is
// chosen for room scale rather than for particles; a scene that needs particles further out moves
// `MESH_DEPTH_T_MAX` at BOTH sites (and re-blesses every Deferred pin) rather than here alone.
//
// # Compile (offline + hermetic; committed `.spv` is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T ps_6_0 -E main \
//       -fspv-target-env=vulkan1.3 particle_draw.fs.hlsl -Fo particle_draw.fs.spv
//   (DEPTH_LINEAR variant: add `-D DEPTH_LINEAR=1` -Fo particle_draw_dlin.fs.spv)
//
// # Set / binding vocabulary -- MIRRORS the host `PARTICLE_LAYOUT_ENTRIES` table
//
//   (set, binding, kind)
//   (1, 0, SAMPLED_IMAGE[])  Texture2D gTextures[]      read (FRAGMENT)
//   (1, 1, SAMPLER)          SamplerState gTexSampler   read (FRAGMENT)
//
//   Set 0 is the VERTEX half's (`p_render` @0, `Camera` @1) -- declared there, unread here.
//
// # Push constants
//
//   NONE in this stage. The 72-byte range the VS declares is VERTEX-only.

[[vk::binding(0, 1)]] Texture2D gTextures[] : register(t0, space1);
[[vk::binding(1, 1)]] SamplerState gTexSampler : register(s0, space1);

// Must stay byte-identical to `particle_draw.vs.hlsl`'s `VsOut` -- the two are one interface, and
// both are printed from ONE generator source (`vs_out_struct`).
{vs_out}
#ifdef DEPTH_LINEAR
// `boyko_rhi_vulkan::compute::MESH_DEPTH_T_MAX`, mirrored (a generator input, not typed here) --
// the SAME divisor `gbuffer_mrt.fs.hlsl` encodes this path's depth buffer with. The normalizer
// only has to AGREE; it cancels in nothing here, because this stage compares rather than decodes.
static const float MESH_DEPTH_T_MAX = {MESH_DEPTH_T_MAX:?};

struct PsOut {{
    float4 color : SV_Target;
    float  depth : SV_Depth;
}};

PsOut main(VsOut input) {{
#else
float4 main(VsOut input) : SV_Target {{
#endif
    float4 c = input.color;
    if (input.tex_index != 0u) {{
        c = c * gTextures[NonUniformResourceIndex(input.tex_index)].Sample(gTexSampler, input.uv);
    }}
#ifdef DEPTH_LINEAR
    PsOut o;
    o.color = c;
    // TERM FOR TERM `gbuffer_mrt.fs.hlsl:327`. The ortho arm keeps the interpolated
    // `SV_Position.z` for the same reason that shader does: an ortho projection bakes the
    // marcher's own axial encode into the matrix, so writing it back is the identity.
    o.depth = (input.cam_mode > 0.5) ? (length(input.eye_rel) / MESH_DEPTH_T_MAX)
                                     : input.position.z;
    return o;
#else
    return c;
#endif
}}
"#
    )
}
