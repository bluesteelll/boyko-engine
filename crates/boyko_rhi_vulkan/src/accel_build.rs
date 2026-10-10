//! HW-RT rung R2a-2 — the buffer-based BLAS/TLAS build orchestration.
//!
//! Gated `#[cfg(feature = "hwrt")]` (the `pub mod` in `lib.rs`). This module drives the
//! R2a-1 FFI verbs (`build_sizes` / `create_accel` / `buffer_device_address` /
//! `accel_device_address` + the encoder's `cmd_build_acceleration_structures`) through the
//! full GPU build sequence for a bottom-level ([`build_blas`]) and a top-level
//! ([`build_tlas`]) acceleration structure: query sizes → allocate the AS backing + scratch
//! buffers → create the AS → record + submit + fence-wait the build → cache the device
//! address.
//!
//! It is BUFFER-based, NOT `MeshRegistry`-based: `boyko_rhi_vulkan` cannot depend upward on
//! `boyko_render`, so the caller passes the raw vertex/index (BLAS) or BLAS-address (TLAS)
//! inputs. The per-frame `MeshRegistry`→instance→TLAS data path is R2a-3.
//!
//! # Lifetime contract
//! A [`BuiltBlas`]/[`BuiltTlas`] OWNS its backing buffer (+ the TLAS's instance buffer). The
//! backing buffer MUST outlive the AS handle, so `destroy_*` frees the AS FIRST, then its
//! buffers, with the device idle (caller contract). The scratch buffer is transient — freed
//! inside the build once the fence signals the build finished reading it.

use boyko_rhi::{
    AsBuildEntry, AsGeometryDesc, AsIndexType, AsKind, BufferDesc, BufferUsage, MemoryLocation,
    RhiCommandEncoder, RhiDevice, RhiQueue,
};

use crate::accel::{BoundAccelStruct, pack_instance};
use crate::device::VulkanContext;
use crate::error::VulkanError;
use crate::memory::BoundBuffer;
use crate::rhi_impl::VulkanQueue;

// ═════════════════════════════════════════════════════════════════════════════
// Error-path RAII (2026-07 audit)
// ═════════════════════════════════════════════════════════════════════════════
//
// Both builders allocate a backing buffer, create an acceleration structure over it, then
// allocate scratch — and every step after the first can fail with `?`. Before this, each of
// those five early returns dropped the handles on the floor: a `VkBuffer` plus its
// suballocation, and on the later paths a `VkAccelerationStructureKHR` too. A device-lost or
// out-of-memory retry loop would therefore bleed VRAM until the device died for a second,
// unrelated reason.
//
// The guards below make cleanup structural rather than something five `?` sites must each
// remember. Drop order is reverse declaration order, which is exactly the module's lifetime
// contract: declare `backing` first and its `accel` second, and the AS is destroyed BEFORE
// the buffer it lives in.

/// Destroys a [`BoundBuffer`] on scope exit unless [`take`](BufferGuard::take) claims it.
struct BufferGuard<'a> {
    ctx: &'a VulkanContext,
    buf: Option<BoundBuffer>,
}

impl<'a> BufferGuard<'a> {
    #[inline]
    fn new(ctx: &'a VulkanContext, buf: BoundBuffer) -> Self {
        Self { ctx, buf: Some(buf) }
    }

    /// The guarded buffer, still guarded.
    #[inline]
    fn get(&self) -> &BoundBuffer {
        self.buf.as_ref().expect("invariant: BufferGuard is occupied until `take`")
    }

    /// Hands ownership back to the caller; the guard becomes inert.
    #[inline]
    fn take(mut self) -> BoundBuffer {
        self.buf.take().expect("invariant: BufferGuard is taken at most once")
    }
}

impl Drop for BufferGuard<'_> {
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            // SAFETY: the guard owned `buf` (created by `create_addr_buffer` on this same
            // `ctx`), so this is its single destruction. It runs only on an error path, i.e.
            // before any build was submitted for it, so no GPU work can still reference it —
            // except the scratch buffer, whose success path `take`s it out of the guard and
            // frees it explicitly after the build fence has signalled.
            unsafe { self.ctx.destroy_buffer(buf) };
        }
    }
}

/// Destroys a [`BoundAccelStruct`] on scope exit unless [`take`](AccelGuard::take) claims it.
struct AccelGuard<'a> {
    ctx: &'a VulkanContext,
    accel: Option<BoundAccelStruct>,
}

impl<'a> AccelGuard<'a> {
    #[inline]
    fn new(ctx: &'a VulkanContext, accel: BoundAccelStruct) -> Self {
        Self { ctx, accel: Some(accel) }
    }

    /// The guarded acceleration structure, still guarded.
    #[inline]
    fn get(&self) -> &BoundAccelStruct {
        self.accel.as_ref().expect("invariant: AccelGuard is occupied until `take`")
    }

    /// Hands ownership back to the caller; the guard becomes inert.
    #[inline]
    fn take(mut self) -> BoundAccelStruct {
        self.accel.take().expect("invariant: AccelGuard is taken at most once")
    }
}

impl Drop for AccelGuard<'_> {
    fn drop(&mut self) {
        if let Some(accel) = self.accel.take() {
            // SAFETY: the guard owned `accel` (created by `ctx.create_accel`), so this is its
            // single destruction, and it runs only on an error path — before the build was
            // submitted, so no in-flight command buffer references it. The backing buffer
            // outlives this call: its guard is declared FIRST and therefore drops LAST.
            unsafe { self.ctx.destroy_accel(accel) };
        }
    }
}

/// Rounds `x` up to the next multiple of `align` (which MUST be a power of two —
/// `as_scratch_align` is, e.g. 128 on Ampere). `align == 0` is treated as 1 (no rounding).
#[inline]
fn round_up(x: u64, align: u64) -> u64 {
    debug_assert!(align.is_power_of_two() || align == 1, "align must be a power of two");
    if align <= 1 {
        return x;
    }
    (x + align - 1) & !(align - 1)
}

/// The vertex/index inputs describing one triangle mesh to build a BLAS from (HW-RT rung
/// R2a-2). The buffers MUST carry `ACCEL_BUILD_INPUT | SHADER_DEVICE_ADDRESS` usage over
/// device-address-flagged memory (the shared host block when
/// [`VulkanContext::ray_query_enabled`]).
pub struct BlasBuildInput<'a> {
    /// The model-space vertex buffer (position at byte offset 0, `R32G32B32_SFLOAT`).
    pub vertex_buffer: &'a BoundBuffer,
    /// The index buffer (`index_count` indices, three per triangle), read at [`Self::index_type`].
    pub index_buffer: &'a BoundBuffer,
    /// The number of vertices (`max_vertex = vertex_count - 1`).
    pub vertex_count: u32,
    /// The number of indices (`primitive_count = index_count / 3`).
    pub index_count: u32,
    /// The byte stride between consecutive vertices (`40` for `MeshRegistry::Vertex`; any
    /// stride is valid as long as the position sits at offset 0).
    pub vertex_stride: u64,
    /// The index width the BLAS reads `index_buffer` at (R2a-3): `Uint16` or `Uint32`, chosen
    /// by the mesh's O3 crossover. The BLAS reads the mesh's EXISTING index buffer at its real
    /// width — NO duplicate `u32` buffer (Principle 0, less VRAM).
    pub index_type: AsIndexType,
}

/// A built bottom-level acceleration structure + its owned backing buffer + cached device
/// address (HW-RT rung R2a-2). The `backing` buffer MUST outlive `accel`; both are freed by
/// [`destroy_blas`]. `device_address` is non-zero (asserted at build) — it is what a TLAS
/// instance's `accelerationStructureReference` references.
pub struct BuiltBlas {
    /// The bottom-level acceleration structure.
    pub accel: BoundAccelStruct,
    /// The buffer the AS lives in (MUST outlive `accel`; freed in [`destroy_blas`]).
    pub backing: BoundBuffer,
    /// The AS device address (`!= 0`), referenced by a TLAS instance.
    pub device_address: u64,
}

/// A built top-level acceleration structure + its owned backing + instance buffers + cached
/// device address (HW-RT rung R2a-2). Both buffers MUST outlive `accel`; all three are freed
/// by [`destroy_tlas`]. `device_address` is non-zero (asserted at build).
pub struct BuiltTlas {
    /// The top-level acceleration structure.
    pub accel: BoundAccelStruct,
    /// The buffer the AS lives in (MUST outlive `accel`; freed in [`destroy_tlas`]).
    pub backing: BoundBuffer,
    /// The `VkAccelerationStructureInstanceKHR[]` array the build read (MUST outlive the
    /// build; freed in [`destroy_tlas`]).
    pub instance_buffer: BoundBuffer,
    /// The AS device address (`!= 0`).
    pub device_address: u64,
}

/// The identity 3×4 row-major affine (`float[3][4]`, translation in column 3), the transform
/// every R2a-2 smoke TLAS instance uses (real per-instance transforms arrive at R2a-3 from
/// the M3 ring). Row-major ↔ `VkTransformMatrixKHR`, so it is a direct 48-byte copy.
const IDENTITY_3X4: [[f32; 4]; 3] =
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];

/// A PERSISTENT top-level acceleration structure sized ONCE for a MAX instance capacity, built
/// into every frame from a caller-supplied instance array (HW-RT rung R2a-3). Unlike
/// [`BuiltTlas`] (which owns its own instance buffer and is built once), this is the durable
/// per-FIF-slot TLAS the host's [`TlasResources`](../../boyko_app/gpu_scene/index.html) rebuilds
/// each frame: the backing is sized for `capacity` instances (`build_sizes` with the MAX), the
/// scratch holds a build's scratch + alignment slack, and the per-frame build supplies the actual
/// (`<= capacity`) instance array address + count. The instance array is NOT owned here (it is
/// the compute-written array in `TlasResources`); the backing + scratch ARE owned.
///
/// # Lifetime contract
/// The backing MUST outlive `accel`; both (+ scratch) are freed by [`destroy_persistent_tlas`]
/// with the device idle (caller contract), the AS FIRST.
pub struct PersistentTlas {
    /// The top-level acceleration structure (built into every frame; sized for `capacity`).
    pub accel: BoundAccelStruct,
    /// The AS backing buffer (MUST outlive `accel`; freed in [`destroy_persistent_tlas`]).
    pub backing: BoundBuffer,
    /// The build scratch buffer (over-allocated by `as_scratch_align` for the aligned address).
    pub scratch: BoundBuffer,
    /// The aligned scratch device address (`round_up(base, as_scratch_align)`), cached once —
    /// the per-frame build's `AsBuildEntry::scratch_address`.
    pub scratch_addr: u64,
}

/// Creates a [`PersistentTlas`] sized for `capacity` instances (HW-RT rung R2a-3): a single
/// `build_sizes(Tlas, primitive_count = capacity)` query, then the AS backing + AS + an
/// over-allocated (alignment-slack) scratch buffer. NO build is recorded here — the caller
/// records a build into [`PersistentTlas::accel`] each frame with the actual (`<= capacity`)
/// instance array + count.
///
/// # Errors
/// A [`VulkanError`] if ray query is off, a buffer/AS create fails, or the scratch buffer's
/// device address comes back `0` (a mis-flagged buffer — fail fast).
pub fn create_persistent_tlas(
    ctx: &VulkanContext,
    capacity: u32,
) -> Result<PersistentTlas, VulkanError> {
    debug_assert!(capacity >= 1, "invariant: a persistent TLAS needs at least one instance slot");
    // Size with the MAX (capacity): the per-frame build's `primitiveCount` must be <= the count
    // used for sizing (VUID). The instance-array address is left 0 for the size query (the size
    // depends only on `primitive_count`, not on the array contents).
    let geom = AsGeometryDesc {
        vertex_data: 0,
        index_data: 0,
        vertex_stride: 0,
        max_vertex: 0,
        primitive_count: capacity,
        index_type: AsIndexType::Uint32,
    };
    let sizes = ctx.build_sizes(AsKind::Tlas, &geom)?;

    // The TLAS backing + scratch are GPU-ONLY (the build writes/reads them, never the CPU) →
    // DEVICE-LOCAL VRAM. The device-local block carries the DEVICE_ADDRESS alloc flag under hwrt,
    // so the scratch device address still resolves.
    // Backing first, AS second: drop order frees the AS before the buffer it lives in, which is
    // what `destroy_persistent_tlas` does on the success path too.
    let backing = BufferGuard::new(ctx, create_addr_buffer(
        ctx,
        sizes.as_size,
        BufferUsage::ACCEL_STRUCTURE_STORAGE | BufferUsage::SHADER_DEVICE_ADDRESS,
        MemoryLocation::DeviceLocal,
    )?);
    let accel = AccelGuard::new(
        ctx,
        ctx.create_accel(AsKind::Tlas, backing.get().buffer, sizes.as_size)?,
    );

    let align = ctx.as_scratch_align().max(1);
    let scratch = BufferGuard::new(ctx, create_addr_buffer(
        ctx,
        sizes.build_scratch + align,
        BufferUsage::STORAGE | BufferUsage::SHADER_DEVICE_ADDRESS,
        MemoryLocation::DeviceLocal,
    )?);
    let scratch_base = ctx.buffer_device_address(scratch.get().buffer)?;
    if scratch_base == 0 {
        return Err(VulkanError::Unsupported("persistent TLAS scratch buffer has a zero device address"));
    }
    let scratch_addr = round_up(scratch_base, align);

    Ok(PersistentTlas {
        accel: accel.take(),
        backing: backing.take(),
        scratch: scratch.take(),
        scratch_addr,
    })
}

/// Destroys a [`PersistentTlas`]: the AS FIRST, then its backing + scratch buffers (HW-RT rung
/// R2a-3).
///
/// # Safety
/// The device is idle / the caller fence-waited every submission that built or traced this
/// TLAS (`ctx.wait_idle()`), and it is destroyed exactly once (by-value move). Both buffers are
/// still live at this call (the AS references its backing until destroyed).
pub unsafe fn destroy_persistent_tlas(ctx: &VulkanContext, tlas: PersistentTlas) {
    // SAFETY: caller contract — the GPU no longer uses `tlas.accel`; the AS is destroyed before
    // its backing (its memory lives in `backing`), and the scratch is freed last.
    unsafe {
        ctx.destroy_accel(tlas.accel);
        ctx.destroy_buffer(tlas.backing);
        ctx.destroy_buffer(tlas.scratch);
    }
}

/// The device address of `buffer` (HW-RT rung R2a-3): a public wrapper over the R2a-1 verb, so
/// the host's [`TlasResources`](../../boyko_app/gpu_scene/index.html) can cache the compute-written
/// instance array's device address once at create (the per-frame build's instance-array address).
/// The buffer MUST carry `SHADER_DEVICE_ADDRESS` usage over device-address-flagged memory.
///
/// # Errors
/// A [`VulkanError`] if ray query is off; the returned address is `0` for a mis-flagged buffer
/// (the caller must fail fast on `0`).
pub fn buffer_device_address(
    ctx: &VulkanContext,
    buffer: &BoundBuffer,
) -> Result<u64, VulkanError> {
    ctx.buffer_device_address(buffer.buffer)
}

/// Allocates a device-addressable buffer of `size` bytes with `usage` from `location`,
/// failing fast if `size == 0`.
///
/// GPU-ONLY AS buffers (BLAS/TLAS backing, build scratch) pass
/// [`MemoryLocation::DeviceLocal`] — they are never CPU-read, so VRAM residency avoids
/// streaming them over BAR/PCIe; the device-local block carries
/// `VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT` under `hwrt` on a ray-query device, so the
/// device address the build needs still resolves. Only the ONE CPU-touched AS buffer —
/// `build_tlas`'s host-packed instance array — stays [`MemoryLocation::HostVisibleCoherent`].
fn create_addr_buffer(
    ctx: &VulkanContext,
    size: u64,
    usage: BufferUsage,
    location: MemoryLocation,
) -> Result<BoundBuffer, VulkanError> {
    ctx.create_buffer(&BufferDesc { size, usage, location })
}

/// Records + submits + fence-waits a single AS build (`entries[0]` into `dest[0]`), then
/// frees the transient encoder + fence. The scratch buffer is the caller's to free AFTER
/// this returns (the fence guarantees the build finished reading it).
fn record_build(
    ctx: &VulkanContext,
    queue: &VulkanQueue,
    entry: &AsBuildEntry,
    dest: &BoundAccelStruct,
) -> Result<(), VulkanError> {
    let fence = ctx.create_fence(false)?;
    let mut encoder = ctx.create_command_encoder()?;
    encoder.begin()?;
    encoder.cmd_build_acceleration_structures(core::slice::from_ref(entry), &[dest]);
    encoder.end()?;
    queue.submit(&encoder, &fence)?;
    ctx.wait_fence(&fence, u64::MAX)?;
    // SAFETY: the fence signaled (`wait_fence` above), so the build submission completed and
    // the GPU no longer uses the encoder or the fence; each is created here and destroyed
    // exactly once by-value.
    unsafe {
        ctx.destroy_command_encoder(encoder);
        ctx.destroy_fence(fence);
    }
    Ok(())
}

/// Builds a bottom-level acceleration structure from one triangle mesh (HW-RT rung R2a-2).
///
/// The R2a-2 proof that the R2a-1 FFI sequence builds a real AS on hardware: query the sizes,
/// allocate the AS backing + an over-allocated (alignment-slack) scratch buffer, create the
/// AS, record + submit + fence-wait the GPU build, then read back the (non-zero) AS device
/// address.
///
/// # Errors
/// A [`VulkanError`] if ray query is off (the verbs report `Unsupported`), a buffer/AS create
/// fails, or a device address comes back `0` (a mis-flagged buffer — fail fast, do not build
/// against a garbage address).
pub fn build_blas(
    ctx: &VulkanContext,
    queue: &VulkanQueue,
    input: &BlasBuildInput,
) -> Result<BuiltBlas, VulkanError> {
    debug_assert!(
        input.vertex_count > 0 && input.index_count >= 3 && input.index_count.is_multiple_of(3),
        "invariant: a triangle BLAS has ≥1 vertex and a positive multiple-of-3 index count \
         (else max_vertex underflows / primitive_count is wrong)"
    );
    let vertex_addr = ctx.buffer_device_address(input.vertex_buffer.buffer)?;
    let index_addr = ctx.buffer_device_address(input.index_buffer.buffer)?;
    if vertex_addr == 0 || index_addr == 0 {
        return Err(VulkanError::Unsupported(
            "BLAS build input buffer has a zero device address (missing SHADER_DEVICE_ADDRESS \
             usage / DEVICE_ADDRESS alloc flag)",
        ));
    }

    let geom = AsGeometryDesc {
        vertex_data: vertex_addr,
        index_data: index_addr,
        vertex_stride: input.vertex_stride,
        max_vertex: input.vertex_count - 1,
        primitive_count: input.index_count / 3,
        index_type: input.index_type,
    };
    let sizes = ctx.build_sizes(AsKind::Blas, &geom)?;

    // The AS backing buffer + the AS created over it. Both backing + scratch are GPU-ONLY
    // (written/read by the build, never CPU-touched) → DEVICE-LOCAL VRAM; the device-local block
    // carries the DEVICE_ADDRESS alloc flag under hwrt, so the scratch device address resolves.
    // Guarded in declaration order so the AS (declared second) is destroyed BEFORE its
    // backing buffer (declared first) on every error path below — the module's lifetime
    // contract, enforced by drop order instead of by five `?` sites remembering it.
    let backing = BufferGuard::new(ctx, create_addr_buffer(
        ctx,
        sizes.as_size,
        BufferUsage::ACCEL_STRUCTURE_STORAGE | BufferUsage::SHADER_DEVICE_ADDRESS,
        MemoryLocation::DeviceLocal,
    )?);
    let accel = AccelGuard::new(
        ctx,
        ctx.create_accel(AsKind::Blas, backing.get().buffer, sizes.as_size)?,
    );

    // The scratch buffer: over-allocate by `align` so the aligned scratch address still has
    // `build_scratch` bytes past it (the scratch address MUST satisfy `as_scratch_align`, NOT
    // the buffer's memreq alignment — research-confirmed).
    let align = ctx.as_scratch_align().max(1);
    let scratch = BufferGuard::new(ctx, create_addr_buffer(
        ctx,
        sizes.build_scratch + align,
        BufferUsage::STORAGE | BufferUsage::SHADER_DEVICE_ADDRESS,
        MemoryLocation::DeviceLocal,
    )?);
    let scratch_base = ctx.buffer_device_address(scratch.get().buffer)?;
    if scratch_base == 0 {
        return Err(VulkanError::Unsupported("BLAS scratch buffer has a zero device address"));
    }
    let scratch_addr = round_up(scratch_base, align);

    let entry = AsBuildEntry { kind: AsKind::Blas, geometry: geom, scratch_address: scratch_addr };
    record_build(ctx, queue, &entry, accel.get())?;

    // The scratch is done (the build fence signaled); free it.
    // SAFETY: `scratch` was created by `create_addr_buffer` on `ctx`'s device-local block and
    // is destroyed exactly once here; the build fence completed, so the GPU no longer reads it.
    unsafe { ctx.destroy_buffer(scratch.take()) };

    let device_address = ctx.accel_device_address(accel.get())?;
    if device_address == 0 {
        return Err(VulkanError::Unsupported("built BLAS reports a zero device address"));
    }
    Ok(BuiltBlas { accel: accel.take(), backing: backing.take(), device_address })
}

/// Builds a top-level acceleration structure over one instance per BLAS device address (HW-RT
/// rung R2a-2). Each instance uses [`IDENTITY_3X4`], `customIndex = i`, `mask = 0xFF`,
/// `sbtOffset = 0`, `flags = 0` (real per-instance transforms arrive at R2a-3).
///
/// Must be a SEPARATE submit from the BLAS builds: the BLAS-build fences guarantee those
/// structures are finished before this build reads their addresses (no intra-command-buffer
/// barrier needed in R2a-2).
///
/// # Errors
/// A [`VulkanError`] as [`build_blas`]; additionally if `blas_addresses` is empty (an empty
/// TLAS is meaningless) or the instance buffer's device address comes back `0`.
pub fn build_tlas(
    ctx: &VulkanContext,
    queue: &VulkanQueue,
    blas_addresses: &[u64],
) -> Result<BuiltTlas, VulkanError> {
    if blas_addresses.is_empty() {
        return Err(VulkanError::Unsupported("TLAS build needs at least one BLAS instance"));
    }

    // Pack one `VkAccelerationStructureInstanceKHR` per BLAS (a build-time Vec — setup, not a
    // hot loop).
    let instances: Vec<_> = blas_addresses
        .iter()
        .enumerate()
        .map(|(i, &blas_addr)| pack_instance(IDENTITY_3X4, i as u32, 0xFF, 0, 0, blas_addr))
        .collect();

    // The instance array buffer (`64 B` per instance) — an AS build input. This is the ONE
    // CPU-touched AS buffer (the host memcpys the `pack_instance` output into it below), so it
    // MUST stay HOST-VISIBLE COHERENT (device-local memory is not mappable).
    let instance_bytes = core::mem::size_of_val(instances.as_slice()) as u64;
    // Guarded from creation: every `?` below this point used to leak it outright.
    let instance_buffer = BufferGuard::new(ctx, create_addr_buffer(
        ctx,
        instance_bytes,
        BufferUsage::ACCEL_BUILD_INPUT | BufferUsage::SHADER_DEVICE_ADDRESS,
        MemoryLocation::HostVisibleCoherent,
    )?);
    let dst = instance_buffer
        .get()
        .mapped
        .ok_or(VulkanError::Unsupported("TLAS instance buffer is not host-mapped"))?;
    // SAFETY: `dst` points to `instance_bytes` mapped host-coherent bytes (the shared host
    // block always maps); `instances` is a distinct `instance_bytes`-byte slice of 64-byte
    // `#[repr(C)]` `VkAccelerationStructureInstanceKHR` (no padding, layout pinned in
    // abi_guard) — the two regions do not overlap (a fresh device allocation vs the owned
    // Vec). VISIBILITY: the backing memory is host-COHERENT (no explicit flush needed) and the
    // subsequent `vkQueueSubmit` (in `record_build`) carries the implicit host-write→device-read
    // domain dependency, so the copied instances are visible to the build. A future R6 residency
    // move to non-coherent host-visible or device-local memory MUST add a
    // `vkFlushMappedMemoryRanges` / a staging copy + barrier here.
    unsafe {
        core::ptr::copy_nonoverlapping(
            instances.as_ptr().cast::<u8>(),
            dst.as_ptr(),
            instance_bytes as usize,
        );
    }

    let instance_addr = ctx.buffer_device_address(instance_buffer.get().buffer)?;
    if instance_addr == 0 {
        return Err(VulkanError::Unsupported("TLAS instance buffer has a zero device address"));
    }

    let geom = AsGeometryDesc {
        vertex_data: instance_addr,
        index_data: 0,
        vertex_stride: 0,
        max_vertex: 0,
        primitive_count: blas_addresses.len() as u32,
        // A TLAS geometry ignores the index type (instance array, not triangles); any value.
        index_type: AsIndexType::Uint32,
    };
    let sizes = ctx.build_sizes(AsKind::Tlas, &geom)?;

    // The TLAS backing + scratch are GPU-ONLY → DEVICE-LOCAL VRAM (the CPU-packed instance array
    // above is the only host-visible AS buffer). The device-local block carries the DEVICE_ADDRESS
    // alloc flag under hwrt, so the scratch device address resolves.
    // Backing first, AS second: drop order then frees the AS before the buffer it lives in.
    let backing = BufferGuard::new(ctx, create_addr_buffer(
        ctx,
        sizes.as_size,
        BufferUsage::ACCEL_STRUCTURE_STORAGE | BufferUsage::SHADER_DEVICE_ADDRESS,
        MemoryLocation::DeviceLocal,
    )?);
    let accel = AccelGuard::new(
        ctx,
        ctx.create_accel(AsKind::Tlas, backing.get().buffer, sizes.as_size)?,
    );

    let align = ctx.as_scratch_align().max(1);
    let scratch = BufferGuard::new(ctx, create_addr_buffer(
        ctx,
        sizes.build_scratch + align,
        BufferUsage::STORAGE | BufferUsage::SHADER_DEVICE_ADDRESS,
        MemoryLocation::DeviceLocal,
    )?);
    let scratch_base = ctx.buffer_device_address(scratch.get().buffer)?;
    if scratch_base == 0 {
        return Err(VulkanError::Unsupported("TLAS scratch buffer has a zero device address"));
    }
    let scratch_addr = round_up(scratch_base, align);

    let entry = AsBuildEntry { kind: AsKind::Tlas, geometry: geom, scratch_address: scratch_addr };
    record_build(ctx, queue, &entry, accel.get())?;

    // SAFETY: as in `build_blas` — the build fence completed, so the GPU no longer reads the
    // scratch; `scratch` is destroyed exactly once here.
    unsafe { ctx.destroy_buffer(scratch.take()) };

    let device_address = ctx.accel_device_address(accel.get())?;
    if device_address == 0 {
        return Err(VulkanError::Unsupported("built TLAS reports a zero device address"));
    }
    Ok(BuiltTlas {
        accel: accel.take(),
        backing: backing.take(),
        instance_buffer: instance_buffer.take(),
        device_address,
    })
}

/// Destroys a [`BuiltBlas`]: the AS FIRST, then its backing buffer (HW-RT rung R2a-2).
///
/// # Safety
/// The device is idle / the caller fence-waited every submission that built or traced this
/// BLAS (`ctx.wait_idle()`), and it is destroyed exactly once (by-value move). The backing
/// buffer is still live at this call (the AS references it until destroyed).
pub unsafe fn destroy_blas(ctx: &VulkanContext, blas: BuiltBlas) {
    // SAFETY: caller contract — the GPU no longer uses `blas.accel`; the AS is destroyed
    // before its backing (the AS's memory lives in `backing`, which must outlive it).
    unsafe {
        ctx.destroy_accel(blas.accel);
        ctx.destroy_buffer(blas.backing);
    }
}

/// Destroys a [`BuiltTlas`]: the AS FIRST, then its backing + instance buffers (HW-RT rung
/// R2a-2).
///
/// # Safety
/// Identical contract to [`destroy_blas`]: the device is idle / every submission using this
/// TLAS completed, and it is destroyed exactly once. Both buffers are still live at this call.
pub unsafe fn destroy_tlas(ctx: &VulkanContext, tlas: BuiltTlas) {
    // SAFETY: caller contract — the GPU no longer uses `tlas.accel`; the AS is destroyed
    // before its backing (its memory lives in `backing`), and the instance buffer is freed
    // last (it was only read during the build, already complete).
    unsafe {
        ctx.destroy_accel(tlas.accel);
        ctx.destroy_buffer(tlas.backing);
        ctx.destroy_buffer(tlas.instance_buffer);
    }
}
