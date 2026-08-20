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
//! Five sources, SEVEN artifacts: the two draw stages each carry a `-D DEPTH_LINEAR` variant
//! (`particle_draw_dlin.{vs,fs}.spv`) — the Deferred path's fragment-written depth encode, one row
//! in `docs/SHADER-VARIANT-MANIFEST.md`. The define is INERT in the base compile, so the five base
//! `.spv` are byte-frozen by construction.
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
/// `VkDrawIndexedIndirectCommand` starts at 24). Zeroed and unread at P0.
const ALPHA_INSTANCE_COUNT_OFFSET: u32 = 28;

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

/// The `VkDispatchIndirectCommand` word index of the EMIT dispatch inside `p_dispatch_args`
/// (offset 0).
const DISPATCH_EMIT_WORD: u32 = 0;
/// The `VkDispatchIndirectCommand` word index of the SIM dispatch (offset 16).
const DISPATCH_SIM_WORD: u32 = 4;

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
     // u16 flags`. `cached_field_d` is the P1 Lipschitz cache, written 0 at P0.\n\
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
    let additive_word = ADDITIVE_INSTANCE_COUNT_OFFSET / 4;
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
// and, on the additive class's own RENDER counter (`p_draw_args`'s `instanceCount`, at the byte
// offset the generator was fed):
//
//   r_base  = WaveReadLaneFirst(InterlockedAdd(additive.instanceCount, r_count))
//   r_pos   = r_base + r_lane
//
// EXACTNESS: reservations from one `InterlockedAdd` counter are contiguous and disjoint, so the
// multiset of `(base, count)` pairs partitions `[0, sum)` exactly and the counter's final value is
// the sum -- independent of the order in which waves retire, because integer addition is
// commutative. That is why NO `InterlockedMax` and no mirror is needed (plan M1), and plan gate
// #14 asserts this module carries ZERO `OpAtomicUMax`.
//
// P0 is ADDITIVE-ONLY, so the blend-class select is a compile-time constant and `r_count`/`r_lane`
// are exactly `w_count`/`w_lane`. They are spelled as their own names anyway: P2's alpha class
// makes them different numbers, and the seam is where the reader needs to see it.
//
// # The atomic budget, per WAVE (plan D5)
//
//   all dying                     -> 1     (dead_count)
//   all surviving, one class      -> 2     (alive_count_next, additive.instanceCount)
//   mixed survive/die             -> 3     (+ dead_count)
//
// At 1M survivors that is ~62 500 ops ~= 32 us, against ~0.5 ms for the naive per-lane form.
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
// # Compile (offline + hermetic; committed `.spv` is byte-gated)
//
//   C:\VulkanSDK\1.4.350.0\Bin\dxc.exe -spirv -T cs_6_0 -E main \
//       -fspv-target-env=vulkan1.3 particle_sim.comp.hlsl -Fo particle_sim.comp.spv
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
//
// # Push constants (8 B)
//
//   [0,4)  uint  steps    -- the host-clamped substep count (plan M3); ONE number, two consumers
//   [4,8)  float timestep -- `ParticleClock`'s CONSTANT dt (plan D6)

struct SimPush {{
    uint  steps;
    float timestep;
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

{words}
// The additive class's RENDER counter, at the word the generator derived from
// `PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET` ({ADDITIVE_INSTANCE_COUNT_OFFSET} bytes) -- plan gate #8. The `InterlockedAdd` on it
// yields BOTH this lane's render position and, at retirement, the class's final instance count.
static const uint DRAW_ADDITIVE_INSTANCE_WORD = {additive_word}u;

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
    [loop]
    for (uint s = 0u; s < steps; ++s) {{
        particle_integrate(pos, vel, life, e.gravity, e.damping, pc.timestep);
        rot = particle_rot_advance(rot, e.rot_mul_cos, e.rot_mul_sin);
    }}

    bool survives = (life > 0.0);

    // Every wave quantity is computed BEFORE either branch, so the ballots see the full set of
    // in-range lanes exactly as the plan's NORMATIVE block describes them.
    uint w_count = WaveActiveCountBits(survives);
    uint w_lane  = WavePrefixCountBits(survives);
    uint d_count = WaveActiveCountBits(!survives);
    uint d_lane  = WavePrefixCountBits(!survives);
    // P0 is additive-only: the class predicate is the compile-time `true`, so the RENDER
    // reservation is the LIST reservation's twin. P2 gives them different predicates.
    uint r_count = w_count;
    uint r_lane  = w_lane;

    uint w_base_raw = 0u;
    uint r_base_raw = 0u;
    uint d_base_raw = 0u;
    if (WaveIsFirstLane()) {{
        // The `> 0u` guards are what keep an all-surviving wave at TWO atomics and an all-dying
        // wave at ONE (plan D5's budget), rather than three unconditional ones.
        if (w_count > 0u) {{
            InterlockedAdd(p_counters[CTR_ALIVE_NEXT], w_count, w_base_raw);
            InterlockedAdd(p_draw_args[DRAW_ADDITIVE_INSTANCE_WORD], r_count, r_base_raw);
        }}
        if (d_count > 0u) {{
            InterlockedAdd(p_counters[CTR_DEAD_COUNT], d_count, d_base_raw);
        }}
    }}
    // Broadcast OUTSIDE the leader branch -- every lane needs the reservation base.
    uint w_base = WaveReadLaneFirst(w_base_raw);
    uint r_base = WaveReadLaneFirst(r_base_raw);
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
    uint r_pos = r_base + r_lane;

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
