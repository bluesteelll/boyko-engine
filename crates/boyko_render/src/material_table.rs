//! [`MaterialTable`] — the GPU-resident mirror of `Assets<MaterialGpu>` (asset-system
//! rung A1: the mesh-materials fold).
//!
//! Rung A1 replaces the standalone `MaterialRegistry` (mesh-materials rung M(-1)) with
//! a CPU-authority / GPU-mirror split, the same shape every future asset type follows:
//!
//! - **CPU authority**: [`Assets<MaterialGpu>`](boyko_ecs::ecs::core::asset::Assets), a
//!   world-global [`Resource`](boyko_ecs::ecs::core::resources::resource::Resource) —
//!   the SAME generic asset-kernel table every future asset type (mesh, texture, …)
//!   will share. Minting a material is `Assets::add`, which returns a
//!   [`Handle<MaterialGpu>`](boyko_ecs::ecs::core::asset::Handle); the render carrier
//!   (a [`MaterialId`](crate::material::MaterialId)) truncates that handle's `u32`
//!   index to the G-buffer's 16-bit width ([`MaterialId::from_handle`](crate::material::MaterialId::from_handle)).
//! - **GPU mirror** (`MaterialTable`, this type): the device-resident SSBO + a
//!   per-in-flight-frame staging ring, and NOTHING ELSE — no host authority of its
//!   own. It reads `Assets<MaterialGpu>` to seed/refresh the device bytes, mirroring
//!   the light table's staging-ring discipline (`boyko_app::gpu_scene`'s
//!   `light_staging` + the L0-r0 generation protocol).
//!
//! `MaterialTable` is `!Send` (it owns RHI buffers, device-bound and
//! single-thread-touch), so it is registered as a
//! [`NonSendResource`](boyko_ecs::ecs::core::resources::resource::NonSendResource)
//! alongside [`MeshRegistry`](crate::mesh_registry::MeshRegistry).
//!
//! # Boot-seed vs. steady-state refresh
//!
//! [`boot_seed`](MaterialTable::boot_seed) is the ONE-TIME device-table allocation +
//! seed, run after every startup mint into `Assets<MaterialGpu>` landed and BEFORE the
//! first frame's descriptor sets bind [`table`](MaterialTable::table)
//! (`boyko_app::runner`'s boot-ordering contract). It is UNCONDITIONAL — never gated
//! on `Assets::dirty_gen()` — see its doc for the boot-seed race a dirty-gen gate would
//! reopen.
//!
//! [`flush_if_dirty`](MaterialTable::flush_if_dirty) is the steady-state per-frame
//! refresh: it re-stages ONLY the fenced in-flight slot (never all FIF slots at once —
//! the M(-1) `MaterialRegistry::set_material`'s finding: re-seeding every ring slot
//! unconditionally is a latent host-write-before-fence WAR once a recorder reads a
//! sibling slot). At this rung (A1) no caller ever mutates a material after boot, so
//! this never actually runs — it exists fenced-correct now so a later rung (materials
//! editable post-boot) is safe from day one.
//!
//! A follow-up rung wires the GPU-side staging→table copy into the recorder (the
//! `boyko_rhi_vulkan` present pass that already copies the light table on a dirty
//! frame); this type only provides the host-side staging + the dirty read
//! (`seen_gen` vs. `Assets::dirty_gen()`) that copy will consult.

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::swapchain::{FRAMES_IN_FLIGHT, FrameWriteToken};

use crate::material::MaterialGpu;

/// The device-resident [`MaterialGpu`] SSBO + its per-in-flight staging ring — the GPU
/// mirror of the world's [`Assets<MaterialGpu>`] CPU authority.
pub struct MaterialTable {
    /// The device SSBO (`STORAGE | TRANSFER_DST`), hard-sized to
    /// `assets.high_water()` (NOT `assets.len()` — see [`boot_seed`](Self::boot_seed)'s
    /// doc) at [`boot_seed`](Self::boot_seed). `None` before the boot seed runs —
    /// there is no valid binding target yet.
    table: Option<BoundBuffer>,
    /// The per-in-flight-frame staging ring (`TRANSFER_SRC`, host-coherent) a future
    /// recorder copies from on a dirty frame, mirroring the light table's staging ring
    /// (`boyko_app::gpu_scene`'s `light_staging`). `None` before the boot seed runs.
    staging: Option<[BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// The `Assets::dirty_gen()` value this table's bytes were last refreshed from.
    /// [`boot_seed`](Self::boot_seed) sets it to the assets' generation AFTER seeding
    /// (not a hardcoded `0`): a fresh `Assets<T>` also starts at generation `0` (only
    /// `get_mut` bumps it — `add` does not), so seeding this from the real value keeps
    /// the invariant correct regardless of whether that changes later.
    seen_gen: u64,
}

impl NonSendResource for MaterialTable {}

impl MaterialTable {
    /// An un-seeded table with no device resources yet. Call
    /// [`boot_seed`](Self::boot_seed) once every startup mint into
    /// `Assets<MaterialGpu>` has landed.
    #[inline]
    pub fn new() -> Self {
        Self { table: None, staging: None, seen_gen: 0 }
    }

    /// Allocates the device table hard-sized to `assets.high_water()` materials (NOT
    /// `assets.len()` — see below) and seeds it — UNCONDITIONALLY, never gated on
    /// `assets.dirty_gen()` — with `assets`'s current CPU authority, plus the
    /// per-in-flight-frame staging ring [`flush_if_dirty`](Self::flush_if_dirty)
    /// writes into. Call ONCE, after every startup mint (`Assets::add`) landed and
    /// BEFORE the first frame's descriptor sets bind [`table`](Self::table)
    /// (`boyko_app::runner`'s boot-ordering contract).
    ///
    /// # Why unconditional (not gated on dirty_gen)
    ///
    /// A dirty-gen GATE (`if assets.dirty_gen() != seen_gen { seed }`) would be wrong
    /// here: a freshly-populated `Assets<MaterialGpu>` starts at `dirty_gen() == 0`
    /// (only `get_mut` bumps it — `add` does not), so a table whose `seen_gen` also
    /// starts at `0` would see the comparison as "already caught up" and skip the seed
    /// entirely, leaving the SSBO unseeded when the one-shot descriptor bind captures
    /// it at the first `sync_gbuffer` — a black-material bug. This fn always seeds
    /// directly from `assets`'s rows; `seen_gen` is set to `assets.dirty_gen()` AFTER,
    /// purely so the FIRST [`flush_if_dirty`](Self::flush_if_dirty) call correctly
    /// sees no edit pending.
    ///
    /// # Why `high_water()`, not `len()` (the W1 fix)
    ///
    /// Each occupied `(Handle, &MaterialGpu)` row is written at BYTE offset
    /// `handle.index() * size_of::<MaterialGpu>()` — the row's ABSOLUTE slot position,
    /// not its rank among live rows. [`Assets::len`](boyko_ecs::ecs::core::asset::Assets::len)
    /// is the LIVE count, which can be smaller than `handle.index() + 1` the moment a
    /// hole exists (some OTHER row was freed without being reused) — sizing the buffer
    /// by `len()` and then writing at `handle.index()` is an out-of-bounds write past
    /// the mapped block the instant a hole exists elsewhere.
    /// [`Assets::high_water`](boyko_ecs::ecs::core::asset::Assets::high_water) is the
    /// slot-row high-water mark (`records.len()`, including holes), so every live
    /// handle's index is unconditionally in range — no append-only assumption needed.
    /// The buffer is zero-initialized before the per-row seed so an unreferenced hole
    /// reads a benign all-zero record, never neighbor-row or uninitialized bytes.
    ///
    /// # Panics
    ///
    /// - `debug_assert!`s it has not already run (a double-seed would leak the first
    ///   table/staging ring) and that `assets` is non-empty (the runner mints slot 0's
    ///   default material before this call).
    /// - Panics (`expect`) on an RHI create/map failure — a device OOM at scene-boot
    ///   time is a setup failure, not a recoverable per-frame error (the
    ///   [`MeshRegistry::register_mesh`](crate::mesh_registry::MeshRegistry::register_mesh)
    ///   precedent).
    pub fn boot_seed(&mut self, assets: &Assets<MaterialGpu>, ctx: &VulkanContext) {
        debug_assert!(
            self.table.is_none() && self.staging.is_none(),
            "invariant: boot_seed runs exactly once"
        );
        debug_assert!(
            !assets.is_empty(),
            "invariant: slot 0's default material is minted before boot_seed"
        );

        let stride = core::mem::size_of::<MaterialGpu>();
        let bytes = (assets.high_water() * stride) as u64;

        let table = RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size: bytes,
                usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
                location: MemoryLocation::HostVisibleCoherent,
            },
        )
        .expect("invariant: material table storage buffer create");
        let mapped = RhiDevice::buffer_mapped_ptr(ctx, &table)
            .expect("invariant: host-visible material table is mapped");
        // SAFETY: `mapped` targets `bytes == assets.high_water() * stride` valid
        // mapped host-coherent bytes (the buffer was just created with exactly that
        // size); no GPU work is in flight yet (boot-time seeding).
        unsafe {
            core::ptr::write_bytes(mapped.as_ptr(), 0, bytes as usize);
            Self::seed_rows(mapped.as_ptr(), assets, stride);
        }

        let staging: [BoundBuffer; FRAMES_IN_FLIGHT] = core::array::from_fn(|_| {
            let b = RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: bytes,
                    usage: BufferUsage::TRANSFER_SRC,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: material staging ring slot create");
            let mapped = RhiDevice::buffer_mapped_ptr(ctx, &b)
                .expect("invariant: host-visible material staging is mapped");
            // SAFETY: same contract as the table seed above — `bytes`-sized, boot-time.
            unsafe {
                core::ptr::write_bytes(mapped.as_ptr(), 0, bytes as usize);
                Self::seed_rows(mapped.as_ptr(), assets, stride);
            }
            b
        });

        self.table = Some(table);
        self.staging = Some(staging);
        self.seen_gen = assets.dirty_gen();
    }

    /// Re-stages `assets`'s CURRENT bytes into ONLY the fenced in-flight slot
    /// (`token.slot()`) if `assets.dirty_gen()` has advanced since this table last saw
    /// it — the per-frame steady-state refresh. Does nothing (no host write at all) on
    /// an up-to-date generation, mirroring the light table's no-rewrite idle
    /// invariant.
    ///
    /// # Why the fenced slot ONLY (not every ring slot)
    ///
    /// M(-1)'s `MaterialRegistry::set_material` re-seeded every FIF staging slot
    /// unconditionally on every edit — harmless while no recorder ever read the ring,
    /// but a latent host-write-before-fence WAR the moment a recorder copies a sibling
    /// slot's staging on the GPU timeline. Writing ONLY `token.slot()` restores the
    /// token discipline every other per-slot host writer in this crate follows
    /// ([`upload_light_table`](crate::upload_light_table) /
    /// [`upload_camera_ring`](crate::upload_camera_ring)): the borrowed
    /// [`FrameWriteToken`] proves THIS slot's in-flight fence was waited this frame,
    /// so the slot's previous occupant's recorded copy (if any) already retired.
    ///
    /// At rung A1 no caller ever mutates `Assets<MaterialGpu>` after
    /// [`boot_seed`](Self::boot_seed) (only slot 0 is ever registered), so
    /// `assets.dirty_gen()` never advances and this never actually re-stages — it is
    /// implemented fenced-correct now so a later rung (materials editable post-boot)
    /// is safe from day one.
    ///
    /// # Panics
    ///
    /// - `expect`s [`boot_seed`](Self::boot_seed) already ran — no staging ring exists
    ///   otherwise, a caller bug.
    /// - Hard `assert!`s `assets.high_water() * size_of::<MaterialGpu>()` (NOT
    ///   `assets.len() * size_of::<MaterialGpu>()` — see [`boot_seed`](Self::boot_seed)'s
    ///   W1 note) fits the staging slot's size before the memcpy (the
    ///   [`upload_light_table`](crate::upload_light_table) discipline — a bound check
    ///   that gates unsafe memory access must not compile out in release).
    pub fn flush_if_dirty(&mut self, assets: &Assets<MaterialGpu>, token: &FrameWriteToken) {
        let generation = assets.dirty_gen();
        if generation == self.seen_gen {
            return;
        }
        self.seen_gen = generation;

        let staging = self
            .staging
            .as_ref()
            .expect("invariant: flush_if_dirty runs after boot_seed");
        let stride = core::mem::size_of::<MaterialGpu>();
        let slot = &staging[token.slot()];
        let mapped = slot
            .mapped
            .expect("invariant: the material staging slot is host-visible mapped");
        let bytes = (assets.high_water() * stride) as u64;
        assert!(
            bytes <= slot.size,
            "material table overflow: {bytes} high-water bytes exceed the {}-byte \
             staging slot (Assets<MaterialGpu> must stay within the boot_seed-time \
             high-water mark)",
            slot.size
        );
        // SAFETY: `mapped` targets >= `slot.size` valid mapped host-coherent bytes
        // (`BoundBuffer`'s own contract), and `bytes <= slot.size` is hard-asserted
        // above — every write `seed_rows` performs stays in range (it writes at
        // `handle.index() * stride < assets.high_water() * stride == bytes`). The
        // borrowed `FrameWriteToken` + the slot-identity contract
        // (`staging[token.slot()]`) prove this slot's in-flight fence was waited THIS
        // frame, so the previous occupant's recorded staging→table copy (if any)
        // already retired — race-free, lock-free.
        unsafe { Self::seed_rows(mapped.as_ptr(), assets, stride) };
    }

    /// The device-resident material SSBO (vocab binding 7 / resolve binding 4).
    ///
    /// # Panics
    /// Panics if called before [`boot_seed`](Self::boot_seed) — a boot-ordering bug
    /// (`boyko_app::runner` calls it before the first frame reads this).
    #[inline]
    pub fn table(&self) -> &BoundBuffer {
        self.table
            .as_ref()
            .expect("invariant: boot_seed runs before table() is read")
    }

    /// Destroys the device table + staging ring through `ctx` (a no-op if
    /// [`boot_seed`](Self::boot_seed) never ran).
    ///
    /// # Safety
    /// The caller MUST have made the device idle (e.g. via the renderer's `Drop` /
    /// `wait_idle`) so no in-flight submit still references the table or staging ring;
    /// each buffer is destroyed exactly once. Mirrors
    /// [`MeshRegistry::destroy`](crate::mesh_registry::MeshRegistry::destroy).
    pub unsafe fn destroy(&mut self, ctx: &VulkanContext) {
        if let Some(table) = self.table.take() {
            // SAFETY: the device is idle (caller contract); `table` was created by
            // `boot_seed` on this same `ctx`; `take` ensures it is destroyed exactly
            // once (a repeat `destroy` call sees `None`).
            unsafe { ctx.destroy_buffer(table) };
        }
        if let Some(staging) = self.staging.take() {
            for slot in staging {
                // SAFETY: same contract as the table above, per staging slot.
                unsafe { ctx.destroy_buffer(slot) };
            }
        }
    }

    /// Writes every occupied `(Handle, &MaterialGpu)` row of `assets` into `dst`, at
    /// BYTE offset `handle.index() * stride` — the row's ABSOLUTE slot position, not
    /// its rank among live rows (a hole elsewhere is left untouched, not compacted
    /// over).
    ///
    /// # Safety
    /// The caller guarantees `dst` targets at least `assets.high_water() * stride`
    /// valid, writable, mapped host-coherent bytes (NOT `assets.len() * stride` — a
    /// live handle's index can exceed the live count the moment a hole exists; see
    /// [`boot_seed`](Self::boot_seed)'s W1 note), and that no in-flight GPU work reads
    /// that range concurrently (boot-time seeding, or a fenced staging slot whose
    /// previous occupant's copy already retired).
    unsafe fn seed_rows(dst: *mut u8, assets: &Assets<MaterialGpu>, stride: usize) {
        let high_water = assets.high_water();
        for (handle, material) in assets.iter() {
            let row = handle.index() as usize;
            // Hard assert (not debug-only): this is the ONE per-row bound that
            // actually gates the unsafe write below — an `Assets<MaterialGpu>` bug
            // that let `handle.index() >= high_water` through must not compile out in
            // release and silently corrupt neighbor GPU memory.
            assert!(
                row < high_water,
                "invariant: Handle::index() {row} exceeds Assets<MaterialGpu>'s own \
                 high_water() {high_water} — an Assets internal-consistency bug"
            );
            // SAFETY: `dst` targets >= `high_water * stride` valid mapped bytes (this
            // fn's caller contract); `row < high_water` (hard-asserted above) keeps
            // `row * stride .. + stride` in bounds; `material` is a distinct host
            // reference `Assets` owns (no overlap with `dst`).
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (material as *const MaterialGpu).cast::<u8>(),
                    dst.add(row * stride),
                    stride,
                );
            }
        }
    }
}

impl Default for MaterialTable {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
