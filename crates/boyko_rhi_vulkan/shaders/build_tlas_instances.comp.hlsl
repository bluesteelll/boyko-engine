// HW-RT rung R2a-3: the per-frame TLAS-instance PACKER compute PRE-PASS
// (`build_tlas_instances.comp.hlsl`).
//
// One invocation per DRAWABLE instance. Each thread reads the 48-byte 3x4 ROW-MAJOR model
// affine of instance `i` from the SHARED M3 instance ring (binding t0, the SAME ring the
// raster VS reads and the interp compute writes), resolves its BLAS device address by
// `mesh_ids[i]` (binding t1 -> t2), and STREAM-WRITES the 64-byte
// `VkAccelerationStructureInstanceKHR` record of instance `i` into the device-local output
// array (binding u3) the per-frame TLAS build reads. Zero CPU-pack, zero readback — the
// engine's CPU-orchestrate / GPU-execute model, mirroring `interp_instances.comp`.
//
// This is PLAIN HAND-HLSL, NOT eDSL: it is a byte copy of the affine + a bit-field pack of
// the per-instance lanes (mask / customIndex / sbtOffset / flags / BLAS reference), with NO
// transform math — the sanctioned exception the interp shader's non-`interp_trs` body already
// establishes. There is nothing for the `f32` Eval oracle to mirror; the R2a-3 GPU smoke is
// the oracle for the 64-byte record layout (the only reflection-unverified surface).
//
// `VkAccelerationStructureInstanceKHR` layout (64 B, matches accel_ffi.rs `abi_guard`):
//   [ 0..48)  float[3][4] transform          (row-major, DIRECT copy of `InstanceModelCol`)
//   [48..52)  uint  instanceCustomIndex:24 | mask:8   (LE: customIndex low, mask high byte)
//   [52..56)  uint  instanceShaderBindingTableRecordOffset:24 | flags:8
//   [56..64)  uint64 accelerationStructureReference   (the BLAS device address, LE lo@56/hi@60)
//
// Compiled offline (hermetic build — the `.spv` is committed) with:
//   dxc.exe -T cs_6_0 -E main -spirv -fspv-target-env=vulkan1.3 \
//       build_tlas_instances.comp.hlsl -Fo build_tlas_instances.comp.spv
// SM6.0 (no `uint64_t`, no `shaderInt64`): the 64-bit BLAS reference is written as two 32-bit
// LE halves, and the address table is read as `uint2` (lo, hi).

// The 3x4 ROW-MAJOR model affine — byte-identical to `boyko_render::InstanceModelCol`
// (`rows[i] = [linear_row_i.xyz | translation_i]`, 48 B), the SAME `InterpModel` shape the
// interp compute writes into the shared ring.
struct InstanceModelCol {
    float4 row0;
    float4 row1;
    float4 row2;
};

// binding t0: the SHARED M3 instance ring (read-only here) — `Instances[i]` is drawable `i`'s
// 48-byte model affine, in the SAME draw-order slot the raster VS indexes and the pack thread
// index `i` addresses. On an interp frame the interp compute wrote the dynamic slots BEFORE
// this pass (the graph derives the COMPUTE-WRITE -> COMPUTE-READ barrier on the ring).
StructuredBuffer<InstanceModelCol> Instances : register(t0);

// binding t1: the parallel per-instance MESH-ID (BLAS-index) lane — `MeshIds[i]` is instance
// `i`'s `MeshHandle.0`, scattered in lock-step with the ring (the gather's `mesh_ids` lane).
// A STORAGE SSBO (`StructuredBuffer<uint>`), NOT a typed `Buffer<uint>` — the host binds it as
// a storage buffer, so the reflection MUST agree (RISK P2-1).
StructuredBuffer<uint> MeshIds : register(t1);

// binding t2: the per-mesh BLAS device-address table — `BlasAddr[m]` is mesh `m`'s BLAS
// device address as a `uint2` (`.x` = low 32 bits, `.y` = high 32 bits; LE). Frame-invariant
// (a BLAS never moves), host-written only when a mesh registers. SM6.0 has no `uint64_t`, so
// the 64-bit address is carried as two 32-bit halves (RISK P2-2).
StructuredBuffer<uint2> BlasAddr : register(t2);

// binding u3: the device-local `VkAccelerationStructureInstanceKHR[]` output (64 B/instance)
// the per-frame TLAS build reads. A `RWByteAddressBuffer` so the 48-byte affine copies + the
// bit-field packs are explicit byte-offset stores (no std430 straddle ambiguity).
RWByteAddressBuffer OutInst : register(u3);

// The pack push constants: the drawable instance count (the bounds guard). Mirrors the host
// `BUILD_TLAS_INSTANCES_PUSH_BYTES` (4 B).
struct Push {
    uint count;
};
[[vk::push_constant]] Push pc;

[numthreads(64, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID) {
    uint i = tid.x;
    if (i >= pc.count) {
        return;
    }
    uint b = i * 64u;

    // The 3x4 ROW-MAJOR affine: a DIRECT 48-byte copy of `InstanceModelCol` into the record's
    // `transform` field (row-major <-> `VkTransformMatrixKHR`, NO transpose — the M3 ring row
    // convention is byte-identical to `VkTransformMatrixKHR`, developer-confirmed).
    InstanceModelCol m = Instances[i];
    OutInst.Store4(b +  0u, asuint(m.row0));
    OutInst.Store4(b + 16u, asuint(m.row1));
    OutInst.Store4(b + 32u, asuint(m.row2));

    // instanceCustomIndex:24 | mask:8 — mask = 0xFF (visible to every ray), customIndex = i
    // (the R2a-4 hit shader resolves `(ring[i] affine, mesh_ids[i] mesh)` from customIndex).
    OutInst.Store(b + 48u, (0xFFu << 24) | (i & 0x00FFFFFFu));
    // instanceShaderBindingTableRecordOffset:24 | flags:8 — both 0 (single hit group, no
    // per-instance flags this rung).
    OutInst.Store(b + 52u, 0u);

    // accelerationStructureReference = the mesh's BLAS device address (two 32-bit LE halves;
    // no `uint64_t` on SM6.0). `MeshIds[i]` indexes the frame-invariant address table.
    uint2 a = BlasAddr[MeshIds[i]];
    OutInst.Store(b + 56u, a.x);
    OutInst.Store(b + 60u, a.y);
}
