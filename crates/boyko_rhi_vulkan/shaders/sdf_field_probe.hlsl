// GATE-1 field-function tripwire probe (PBR MVP-2 Phase 0).
//
// This shader exists ONLY to isolate the determinism-frozen field eval
// (`sdf_field.hlsli`) into a SPIR-V module that contains NOTHING but the field
// math + a trivial buffer write. It carries NO ray-gen, NO marcher loop, NO
// shading, NO material logic — so its disassembly is a function PURELY of the
// frozen field source. The PBR MVP-2 marcher refactor (helper split + ray-gen
// extraction + material pick + shading rewrite) edits the MARCHER, never the
// field; this probe is the empirical proof of that.
//
// DXC fully INLINES every helper into `main`, so the marcher's own SPIR-V is one
// flat `OpFunction` where field ops are interleaved with shading ops — a
// per-function diff of the marcher is impossible. This probe sidesteps that: it
// calls only `field_distance` + `sdf_normal` (the frozen gateway), so its whole
// instruction stream IS the field math. The host test `field_function_byte_identity`
// (tests/field_probe_gate.rs) `spirv-dis`-es the committed probe `.spv` and asserts
// it is BYTE-IDENTICAL to the committed baseline disassembly
// (`shaders/sdf_field_probe.baseline.dis`). Any perturbation of the field SPIR-V —
// the #1 risk of the refactor — trips that test loudly.
//
// # The descriptor set (set 0)
//
//   binding 0 : StructuredBuffer<uint> (READ-ONLY) — the packed edit-list header
//               (the same `encode_edit_list` format the marcher / golden use). The
//               field eval reads it; the INCLUDE CONTRACT of sdf_field.hlsli
//               requires `Buf` to be in scope before the `#include`.
//   binding 1 : RWStructuredBuffer<float> (STORAGE) — a 4-float sink: the field
//               distance + the 3 normal components at a fixed probe point. The
//               write KEEPS the field calls live (DXC would dead-strip them otherwise),
//               but adds no field-relevant ops the diff would see drift in.
//
// Compiled offline (hermetic build — no SDK at `cargo build` time) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 \
//       sdf_field_probe.hlsl -Fo sdf_field_probe.comp.spv

StructuredBuffer<uint> Buf : register(t0); // binding 0: edit-list header (READ-ONLY)

// The determinism-frozen field gateway. INCLUDE CONTRACT: `Buf` must be in scope
// first. This is the SAME header the marcher includes; this probe pins its SPIR-V.
#include "sdf_field.hlsli"

RWStructuredBuffer<float> Out : register(u1); // binding 1: 4-float field sink

[numthreads(1, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    // A fixed, data-independent probe point: the field eval still folds the whole
    // edit-list at `p`, so every primitive distance + boolean op + smin/smax +
    // central-difference gradient op is emitted. The point value is irrelevant —
    // the test diffs the OP STREAM, not the result.
    float3 p = float3(0.1, 0.2, 0.3);
    float d = field_distance(p);
    float3 n = sdf_normal(p);
    Out[0] = d;
    Out[1] = n.x;
    Out[2] = n.y;
    Out[3] = n.z;
}
