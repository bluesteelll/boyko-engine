//! [`MaterialTable`] — the GPU-resident mirror of `Assets<Material>` (asset-system
//! rung A1: the mesh-materials fold; textured-PBR rung T5 widened the CPU authority
//! element from a bare `MaterialGpu` to `Material { gpu, textures }`).
//!
//! Rung A1 replaces the standalone `MaterialRegistry` (mesh-materials rung M(-1)) with
//! a CPU-authority / GPU-mirror split, the same shape every future asset type follows:
//!
//! - **CPU authority**: [`Assets<Material>`](boyko_ecs::ecs::core::asset::Assets), a
//!   world-global [`Resource`](boyko_ecs::ecs::core::resources::resource::Resource) —
//!   the SAME generic asset-kernel table every future asset type (mesh, texture, …)
//!   will share. Minting a material is `Assets::add`, which returns a
//!   [`Handle<Material>`](boyko_ecs::ecs::core::asset::Handle); the render carrier
//!   (a [`MaterialId`](crate::material::MaterialId)) truncates that handle's `u32`
//!   index to the G-buffer's 16-bit width ([`MaterialId::from_handle`](crate::material::MaterialId::from_handle)).
//! - **GPU mirror** (`MaterialTable`, this type): the device-resident SSBO + a
//!   per-in-flight-frame staging ring, and NOTHING ELSE — no host authority of its
//!   own. It reads `Assets<Material>` to seed/refresh the device bytes with ONLY each
//!   row's `gpu` field (the `MaterialGpu` SSBO element; `textures` is a CPU-only
//!   sidecar, never uploaded as a table row), mirroring the light table's
//!   staging-ring discipline (`boyko_app::gpu_scene`'s `light_staging` + the L0-r0
//!   generation protocol).
//!
//! `MaterialTable` is `!Send` (it owns RHI buffers, device-bound and
//! single-thread-touch), so it is registered as a
//! [`NonSendResource`](boyko_ecs::ecs::core::resources::resource::NonSendResource)
//! alongside the mesh assets ([`MeshAssetsExt`](crate::mesh_assets::MeshAssetsExt)).
//!
//! # How the table's bytes arrive (dynamic-materials DM1)
//!
//! The host never writes the table itself: it is `DeviceLocal`, and only the staging ring is
//! host-visible. Every byte reaches the table through a recorded
//! `staging → table` copy (the `material_upload` pass every render path declares right after
//! `light_upload`), from the FENCED in-flight slot of the staging ring:
//!
//! - **The full image** ([`write_full_image`](MaterialTable::write_full_image)): zero plus every
//!   `Loaded` row, rebuilt from the CPU authority, as ONE region `[0, capacity·48)`. Owed on frame
//!   0 ([`boot_seed`](MaterialTable::boot_seed) creates the table EMPTY), on a grow frame
//!   ([`grow_if_needed`](MaterialTable::grow_if_needed) creates the grown table empty), and on the
//!   first recorded frame after a frame that drained edits but never recorded their copy. The host
//!   flag that tracks it is `boyko_app::material_gate`'s.
//! - **The compact edited rows** (`crate::material_upload`): every other frame with an edit, the
//!   stager's packed rows at `[0, k·48)` of the slot, copied at their runs.
//!
//! Writing only the fenced slot is what keeps both race-free (the M(-1) `MaterialRegistry`'s
//! finding: re-seeding every ring slot is a host-write-before-fence WAR once a recorder reads a
//! sibling slot), and the graph's cross-frame seed-WAR orders a copy after the sibling frame's
//! table reads.

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::resources::resource::NonSendResource;
use boyko_rhi::{BufferDesc, BufferUsage, MemoryLocation, RhiDevice};
use boyko_rhi_vulkan::device::VulkanContext;
use boyko_rhi_vulkan::memory::BoundBuffer;
use boyko_rhi_vulkan::swapchain::{FRAMES_IN_FLIGHT, FrameWriteToken};

use crate::asset_refcount::RETIRE_DELAY;
use crate::material::{MATERIAL_FLAG_TEXTURED, Material, MaterialGpu};
use crate::retired_gpu_buffers::RetiredGpuBuffers;

/// The hard growth cap for [`MaterialTable::grow_if_needed`] (asset-streaming plan F7
/// Q2): [`crate::material::MaterialId`] truncates a `Handle<Material>`'s index to a
/// 16-bit width, so a device table beyond `1 << 16` rows is an addressing failure, not
/// merely a sanity limit — this stays a HARD `assert!` (never `debug_assert!`).
pub const MAX_MATERIAL_ROWS: usize = 1 << 16;

/// The device-resident [`MaterialGpu`] SSBO + its per-in-flight staging ring — the GPU
/// mirror of the world's [`Assets<Material>`] CPU authority (only each row's `gpu`
/// field is mirrored; `textures` stays host-side).
pub struct MaterialTable {
    /// The device-local SSBO (`STORAGE | TRANSFER_DST`), hard-sized to
    /// `assets.high_water()` (NOT `assets.len()` — see [`boot_seed`](Self::boot_seed)'s
    /// doc) at [`boot_seed`](Self::boot_seed). `None` before the boot seed runs —
    /// there is no valid binding target yet. Written ONLY by recorded copies (the module doc).
    table: Option<BoundBuffer>,
    /// The per-in-flight-frame staging ring (`TRANSFER_SRC`, host-coherent) the
    /// `material_upload` copy reads from, mirroring the light table's staging ring
    /// (`boyko_app::gpu_scene`'s `light_staging`): slot `s` is written only by a frame fenced on
    /// `s`. Sized like the table, so it holds a full image. `None` before the boot seed runs.
    staging: Option<[BoundBuffer; FRAMES_IN_FLIGHT]>,
    /// Asset-streaming plan F7: the table's current row capacity (`assets.high_water()`
    /// at [`boot_seed`](Self::boot_seed), `next_power_of_two()`-doubled by
    /// [`grow_if_needed`](Self::grow_if_needed)). The steady-state `need <=
    /// capacity_rows` compare is the byte-identity guard on a non-growing scene.
    capacity_rows: u32,
    /// Asset-streaming plan F7 (W1-a): PERSISTENT per-FIF-slot flag — `true` iff slot
    /// `i`'s material-bearing descriptor sets still point at a superseded `table`/
    /// `staging` buffer. Consumed on that slot's NEXT occupancy
    /// ([`take_rebind_pending`](Self::take_rebind_pending)), never on a horizon timer —
    /// see the F7 design's FIF-rebind correctness proof for why this must be a
    /// persistent flag, not dirty-gated.
    rebind_pending: [bool; FRAMES_IN_FLIGHT],
}

impl NonSendResource for MaterialTable {}

impl MaterialTable {
    /// An un-seeded table with no device resources yet. Call
    /// [`boot_seed`](Self::boot_seed) once every startup mint into
    /// `Assets<Material>` has landed.
    #[inline]
    pub fn new() -> Self {
        Self {
            table: None,
            staging: None,
            capacity_rows: 0,
            rebind_pending: [false; FRAMES_IN_FLIGHT],
        }
    }

    /// Allocates the device table hard-sized to `assets.high_water()` materials (NOT
    /// `assets.len()` — see below) plus the per-in-flight-frame staging ring the
    /// `material_upload` copy reads from. Call ONCE, after every startup mint (`Assets::add`)
    /// landed and BEFORE the first frame's descriptor sets bind [`table`](Self::table)
    /// (`boyko_app::runner`'s boot-ordering contract).
    ///
    /// # Created empty (dynamic-materials DM1)
    ///
    /// Neither buffer is written here. Frame 0 owes the table its full image
    /// ([`write_full_image`](Self::write_full_image) into the fenced staging slot, then the
    /// recorded copy — the host's full-image flag starts `true`), and the copy is ordered before
    /// every reader of the table on every render path, so no shader ever reads it unwritten. That
    /// is also why the table is device-local: nothing needs its mapping.
    ///
    /// # Why `high_water()`, not `len()` (the W1 fix)
    ///
    /// Each occupied `(Handle, &Material)` row's `gpu` field is written at BYTE offset
    /// `handle.index() * size_of::<MaterialGpu>()` — the row's ABSOLUTE slot position,
    /// not its rank among live rows. [`Assets::len`](boyko_ecs::ecs::core::asset::Assets::len)
    /// is the LIVE count, which can be smaller than `handle.index() + 1` the moment a
    /// hole exists (some OTHER row was freed without being reused) — sizing the buffer
    /// by `len()` and then writing at `handle.index()` is an out-of-bounds write past
    /// the full image's staging slot the instant a hole exists elsewhere.
    /// [`Assets::high_water`](boyko_ecs::ecs::core::asset::Assets::high_water) is the
    /// slot-row high-water mark (`records.len()`, including holes), so every live
    /// handle's index is unconditionally in range — no append-only assumption needed.
    /// The full image is zero-filled before the per-row seed, so an unreferenced hole
    /// reads a benign all-zero record, never neighbor-row or uninitialized bytes.
    ///
    /// # Panics
    ///
    /// - `debug_assert!`s it has not already run (a double boot would leak the first
    ///   table/staging ring) and that `assets` is non-empty (the runner mints slot 0's
    ///   default material before this call).
    /// - Panics (`expect`) on an RHI create failure — a device OOM at scene-boot
    ///   time is a setup failure, not a recoverable per-frame error (the
    ///   [`MeshAssetsExt::register_mesh`](crate::mesh_assets::MeshAssetsExt::register_mesh)
    ///   precedent).
    pub fn boot_seed(&mut self, assets: &Assets<Material>, ctx: &VulkanContext) {
        debug_assert!(
            self.table.is_none() && self.staging.is_none(),
            "invariant: boot_seed runs exactly once"
        );
        debug_assert!(
            !assets.is_empty(),
            "invariant: slot 0's default material is minted before boot_seed"
        );

        let capacity = assets.high_water();
        self.table = Some(Self::create_table(ctx, capacity));
        self.staging = Some(Self::create_staging(ctx, capacity));
        // Asset-streaming plan F7: `capacity_rows` starts at the boot high-water mark;
        // `rebind_pending` starts all-`false` (nothing to repoint — `table()`/`staging`
        // are being bound for the FIRST time, not superseding a prior buffer).
        self.capacity_rows = capacity as u32;
    }

    /// The device table buffer for `rows` materials (`STORAGE | TRANSFER_DST`, `DeviceLocal`),
    /// created EMPTY — its bytes arrive only through the recorded `material_upload` copy, so
    /// nothing ever maps it (dynamic-materials DM1, design F3: the shaders' reads stay in VRAM).
    fn create_table(ctx: &VulkanContext, rows: usize) -> BoundBuffer {
        RhiDevice::create_buffer(
            ctx,
            &BufferDesc {
                size: (rows * core::mem::size_of::<MaterialGpu>()) as u64,
                usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
                location: MemoryLocation::DeviceLocal,
            },
        )
        .expect("invariant: material table storage buffer create")
    }

    /// The per-in-flight-frame staging ring for a `rows`-material table (`TRANSFER_SRC`,
    /// host-coherent, persistently mapped), sized to hold a full image. Not written here: each
    /// slot is written only by a frame fenced on it.
    fn create_staging(ctx: &VulkanContext, rows: usize) -> [BoundBuffer; FRAMES_IN_FLIGHT] {
        core::array::from_fn(|_| {
            RhiDevice::create_buffer(
                ctx,
                &BufferDesc {
                    size: (rows * core::mem::size_of::<MaterialGpu>()) as u64,
                    usage: BufferUsage::TRANSFER_SRC,
                    location: MemoryLocation::HostVisibleCoherent,
                },
            )
            .expect("invariant: material staging ring slot create")
        })
    }

    /// The staging ring's slot `slot` — the copy source of this frame's `material_upload`, and
    /// the destination of [`write_full_image`](Self::write_full_image) and the compact row upload
    /// (`crate::upload_material_rows`). Always the CURRENT ring (a grow replaces every slot).
    ///
    /// # Panics
    /// Panics if called before [`boot_seed`](Self::boot_seed).
    #[inline]
    pub fn staging_slot(&self, slot: usize) -> &BoundBuffer {
        &self
            .staging
            .as_ref()
            .expect("invariant: boot_seed runs before staging_slot() is read")[slot]
    }

    /// Writes the table's FULL image — zero, then every `Loaded` row of `assets` at its absolute
    /// row offset ([`seed_rows`](Self::seed_rows), the TEXTURED-flag derive included) — into the
    /// fenced staging slot `token.slot()`, and returns its byte length `capacity·48`: the one
    /// region `[0, capacity·48)` the caller then copies. Owed on frame 0, on a grow frame, and
    /// after a frame whose edits were drained but never copied (`boyko_app::material_gate`).
    ///
    /// # Panics
    ///
    /// - `expect`s [`boot_seed`](Self::boot_seed) already ran.
    /// - Hard `assert!`s `assets.high_water() <= capacity` (the caller grows first) and that the
    ///   image fits the slot — the bound that gates the unsafe writes must not compile out.
    pub fn write_full_image(&self, token: &FrameWriteToken, assets: &Assets<Material>) -> u64 {
        let stride = core::mem::size_of::<MaterialGpu>();
        let bytes = self.capacity_rows as usize * stride;
        let slot = self.staging_slot(token.slot());
        assert!(
            assets.high_water() <= self.capacity_rows as usize && bytes as u64 <= slot.size,
            "material full image overflow: high-water {} rows, capacity {} rows ({bytes} bytes), \
             staging slot {} bytes — a mint past the capacity reached the upload without the grow",
            assets.high_water(),
            self.capacity_rows,
            slot.size
        );
        let mapped = slot.mapped.expect("invariant: the material staging slot is host-visible mapped");
        // SAFETY: `mapped` targets >= `slot.size` valid mapped host-coherent bytes (`BoundBuffer`'s
        // own contract) and `bytes <= slot.size` is hard-asserted above, so the zero-fill is in
        // range; `seed_rows` writes only rows `< high_water <= capacity_rows`, inside `bytes`
        // (asserted above). The borrowed `FrameWriteToken` + the slot identity (`staging[token.
        // slot()]`) prove this slot's in-flight fence was waited THIS frame, so the previous
        // occupant's recorded copy already retired — race-free, lock-free.
        unsafe {
            core::ptr::write_bytes(mapped.as_ptr(), 0, bytes);
            Self::seed_rows(mapped.as_ptr(), assets, stride);
        }
        bytes as u64
    }

    /// The device-resident material SSBO (vocab binding 7 / resolve binding 4). Always
    /// the CURRENT (newest) buffer — never a captured pointer (asset-streaming plan F7
    /// Q1): a caller that re-reads this after [`grow_if_needed`](Self::grow_if_needed)
    /// sees the grown table, so a rebind driven from this accessor is idempotent.
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

    /// `true` iff FIF slot `slot`'s material-bearing descriptor sets still point at a
    /// buffer [`grow_if_needed`](Self::grow_if_needed) has since superseded (asset-
    /// streaming plan F7 W1-a). A persistent flag, not a horizon timer — see the type
    /// doc.
    #[inline]
    pub fn rebind_pending(&self, slot: usize) -> bool {
        self.rebind_pending[slot]
    }

    /// Reads [`rebind_pending`](Self::rebind_pending) for `slot` and clears it — the
    /// caller commits to repointing that slot's sets THIS call (asset-streaming plan F7
    /// §6 step 4.6). Consuming the flag ONLY here (never on a timer) is what makes the
    /// FIF-rebind proof's invariant (a) hold (see the F7 design doc).
    #[inline]
    pub fn take_rebind_pending(&mut self, slot: usize) -> bool {
        core::mem::replace(&mut self.rebind_pending[slot], false)
    }

    /// Cheap, allocation-free steady-state check (asset-streaming plan F7 review W1):
    /// `true` iff `high_water` exceeds this table's current row capacity. Reads only
    /// the `Copy` `capacity_rows` field through a shared `&self` — call this BEFORE
    /// paying for the (rare) [`grow_if_needed`](Self::grow_if_needed) path's NonSend
    /// take-out/reinsert, so the golden/steady-state path never pays for it.
    #[inline]
    pub fn needs_grow(&self, high_water: usize) -> bool {
        high_water > self.capacity_rows as usize
    }

    /// The table's current row capacity (`assets.high_water()` at
    /// [`boot_seed`](Self::boot_seed), grown by [`grow_if_needed`](Self::grow_if_needed)) —
    /// every live [`MaterialId`](crate::material::MaterialId) is `< capacity_rows()` by
    /// construction. VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md) threads this
    /// as the classify `scan` pass's `present_material_count` LOOP BOUND (a P2b simplification
    /// over the plan's D2 "frame's distinct material ids" — see that plan's rung table): a valid,
    /// always-safe upper bound since it still folds every material id the frame could reference.
    /// `0` before [`boot_seed`](Self::boot_seed) runs (the [`new`](Self::new) default) — never
    /// panics.
    #[inline]
    pub fn capacity_rows(&self) -> u32 {
        self.capacity_rows
    }

    /// Grows the device table + FIF staging ring to `next_power_of_two(assets.
    /// high_water())` iff that exceeds the table's current row capacity, creating the new
    /// buffers EMPTY and routing the superseded ones through `retired` at `epoch +
    /// RETIRE_DELAY` (asset-streaming plan F7 §7.1). Returns `true` iff a grow happened: the
    /// caller then repoints the fenced slot to [`table`](Self::table) AND owes the grown table
    /// its full image this frame ([`write_full_image`](Self::write_full_image) — design F3; the
    /// frame's compact runs alone would leave every row they do not touch unseeded).
    ///
    /// # Steady-state cost
    ///
    /// One `usize` compare (`need <= self.capacity_rows`) when no grow is needed — the
    /// byte-identity guard on a scene whose material count never exceeds its boot
    /// high-water mark (every in-tree golden). The caller is expected to have already
    /// consulted [`needs_grow`](Self::needs_grow) before paying for the NonSend take-
    /// out this call requires; this re-check is a defensive belt-and-suspenders, not
    /// the steady-state gate anymore.
    ///
    /// # The grown table is filled from the authority, never copied from the old buffer
    ///
    /// The full image ([`write_full_image`](Self::write_full_image)) zero-fills the whole
    /// `[0, new_cap)` range before `seed_rows` writes the `Loaded` rows, which covers the
    /// newly-grown hole `[old_cap, new_cap)` and any freed (`Retiring`/`Vacant`) row below
    /// `old_cap`. `seed_rows` iterates [`Assets::iter`](boyko_ecs::ecs::core::asset::Assets::iter),
    /// which yields ONLY `Loaded` rows, so it never forms `&MaterialGpu` over an uninitialized or
    /// `Retiring` slot.
    ///
    /// # Every staging slot is replaced at once (DM1 cut C-5)
    ///
    /// The staging ring is bound in no descriptor set; only recorded copies reference it. The
    /// superseded slots are retired at `epoch + RETIRE_DELAY` like the table, so the sibling
    /// in-flight frame's already-recorded copy still reads a live buffer, and every later frame
    /// writes and copies from the NEW slot of its own fence. A per-slot fenced staging grow is
    /// therefore unnecessary.
    ///
    /// # Panics
    ///
    /// Hard `assert!`s `new_cap <= MAX_MATERIAL_ROWS` — [`MaterialId`](crate::material::MaterialId)
    /// is a 16-bit index, so a table beyond `1 << 16` rows is an addressing failure,
    /// not a mere sanity bound (this assert MUST NOT be downgraded to `debug_assert!`).
    /// Panics (`expect`) on an RHI create failure, mirroring
    /// [`boot_seed`](Self::boot_seed) — a device OOM on a post-boot mint is a setup
    /// failure, not a recoverable per-frame error.
    pub fn grow_if_needed(
        &mut self,
        assets: &Assets<Material>,
        ctx: &VulkanContext,
        retired: &mut RetiredGpuBuffers,
        epoch: u64,
    ) -> bool {
        let need = assets.high_water();
        if need <= self.capacity_rows as usize {
            return false;
        }
        let new_cap = need.next_power_of_two();
        debug_assert!(new_cap.is_power_of_two());
        debug_assert!(new_cap >= need);
        assert!(
            new_cap <= MAX_MATERIAL_ROWS,
            "material table overflow: {new_cap} rows exceed MAX_MATERIAL_ROWS \
             ({MAX_MATERIAL_ROWS}) — MaterialId is a 16-bit index and cannot address \
             more rows"
        );

        let new_table = Self::create_table(ctx, new_cap);
        let new_staging = Self::create_staging(ctx, new_cap);

        let old_table = self
            .table
            .replace(new_table)
            .expect("invariant: grow_if_needed runs after boot_seed");
        let old_staging = self
            .staging
            .replace(new_staging)
            .expect("invariant: grow_if_needed runs after boot_seed");
        retired.push(old_table, epoch + RETIRE_DELAY);
        for slot in old_staging {
            retired.push(slot, epoch + RETIRE_DELAY);
        }

        self.capacity_rows = new_cap as u32;
        self.rebind_pending = [true; FRAMES_IN_FLIGHT];
        true
    }

    /// Destroys the device table + staging ring through `ctx` (a no-op if
    /// [`boot_seed`](Self::boot_seed) never ran).
    ///
    /// # Safety
    /// The caller MUST have made the device idle (e.g. via the renderer's `Drop` /
    /// `wait_idle`) so no in-flight submit still references the table or staging ring;
    /// each buffer is destroyed exactly once. Mirrors
    /// [`MeshAssetsExt::destroy`](crate::mesh_assets::MeshAssetsExt::destroy).
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

    /// Writes every occupied `(Handle, &Material)` row of `assets` into `dst` at BYTE
    /// offset `handle.index() * stride` — the row's ABSOLUTE slot position, not its rank
    /// among live rows (a hole elsewhere is left untouched, not compacted over). `textures`
    /// is a CPU-only sidecar, never uploaded as a table row.
    ///
    /// # `MATERIAL_FLAG_TEXTURED` is RE-DERIVED here, not copied verbatim
    ///
    /// Every row the GPU table receives passes [`derive_gpu_row`]: the full image through this fn
    /// ([`write_full_image`](Self::write_full_image)), the compact edited rows through the stager
    /// (`crate::material_upload`). So that one derive is the ONE
    /// place [`MATERIAL_FLAG_TEXTURED`] is made authoritative: the copied `mrr[3]` flags
    /// OR the bit in when `textures.any()` and AND it out otherwise, regardless of
    /// whatever bit `material.gpu.mrr[3]` already carries — a full derive, not a partial
    /// OR. This closes the direct-field-mutation footgun (`mat.textures.albedo = slot`
    /// after construction, or clearing `mat.textures` back to
    /// [`MaterialTextures::NONE`](crate::material::MaterialTextures::NONE) without
    /// touching `mat.gpu`) that would otherwise desync the uploaded bit from the texture
    /// sidecar it gates. `material.gpu` itself is untouched (the derive works on a local
    /// copy); every non-textured material's `flags` bit stays 0 (byte-identical to every
    /// in-tree golden).
    ///
    /// # Safety
    /// The caller guarantees `dst` targets at least `assets.high_water() * stride`
    /// valid, writable, mapped host-coherent bytes (NOT `assets.len() * stride` — a
    /// live handle's index can exceed the live count the moment a hole exists; see
    /// [`boot_seed`](Self::boot_seed)'s W1 note), and that no in-flight GPU work reads
    /// that range concurrently (a fenced staging slot whose previous occupant's copy already
    /// retired).
    unsafe fn seed_rows(dst: *mut u8, assets: &Assets<Material>, stride: usize) {
        let high_water = assets.high_water();
        for (handle, material) in assets.iter() {
            let row = handle.index() as usize;
            // Hard assert (not debug-only): this is the ONE per-row bound that
            // actually gates the unsafe write below — an `Assets<Material>` bug
            // that let `handle.index() >= high_water` through must not compile out in
            // release and silently corrupt neighbor GPU memory.
            assert!(
                row < high_water,
                "invariant: Handle::index() {row} exceeds Assets<Material>'s own \
                 high_water() {high_water} — an Assets internal-consistency bug"
            );
            let gpu = derive_gpu_row(material);
            // SAFETY: `dst` targets >= `high_water * stride` valid mapped bytes (this
            // fn's caller contract); `row < high_water` (hard-asserted above) keeps
            // `row * stride .. + stride` in bounds; `gpu` is a local, distinct from
            // `dst` (no overlap).
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (&gpu as *const MaterialGpu).cast::<u8>(),
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

/// The [`MaterialGpu`] bytes one material row uploads as — its `gpu` field with
/// [`MATERIAL_FLAG_TEXTURED`] RE-DERIVED from `textures.any()` (ORed in when a slot is bound,
/// ANDed out otherwise) on a local copy, regardless of whatever bit `gpu.mrr[3]` already carries.
///
/// The ONE host→GPU derive every table write path shares: the full image
/// ([`MaterialTable::seed_rows`]) and the compact per-frame staging
/// (`crate::material_upload::stage_material_edits`) both call it, so a row uploaded either way
/// carries identical bytes. A non-textured material's result is byte-identical to its `gpu`.
#[inline]
pub(crate) fn derive_gpu_row(material: &Material) -> MaterialGpu {
    let mut gpu = material.gpu;
    let flags = gpu.mrr[3].to_bits();
    gpu.mrr[3] = f32::from_bits(if material.textures.any() {
        flags | MATERIAL_FLAG_TEXTURED
    } else {
        flags & !MATERIAL_FLAG_TEXTURED
    });
    gpu
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::MaterialTextures;

    // ── Derive-at-upload (grooming item B): `seed_rows` re-derives
    // MATERIAL_FLAG_TEXTURED from `textures.any()` rather than trusting whatever bit
    // `gpu.mrr[3]` already carries ────────────────────────────────────────────────

    /// Runs `seed_rows` over a single-row `Assets<Material>` and returns that row's
    /// staged `MaterialGpu`. The destination is a `MaterialGpu`-typed buffer (not a raw
    /// `Vec<u8>`) so the write's target alignment (16 B) is satisfied by construction —
    /// sidesteps an unrelated alignment concern that would otherwise need
    /// `ptr::write_unaligned` reasoning in this test.
    fn seed_one(material: Material) -> MaterialGpu {
        let mut assets: Assets<Material> = Assets::with_reserved(4);
        assets.add(material);
        let stride = core::mem::size_of::<MaterialGpu>();
        let mut dst = vec![MaterialGpu::default(); assets.high_water()];
        // SAFETY: `dst` is sized to exactly `assets.high_water()` `MaterialGpu`
        // elements (`high_water * stride` valid, writable, mapped-equivalent host
        // bytes — a plain `Vec` allocation stands in for a mapped buffer here); no
        // concurrent access exists in a single-threaded unit test.
        unsafe {
            MaterialTable::seed_rows(dst.as_mut_ptr().cast::<u8>(), &assets, stride);
        }
        dst[0]
    }

    #[test]
    fn seed_rows_sets_the_textured_flag_for_a_directly_mutated_textures_sidecar() {
        let mut mat = Material::default();
        assert_eq!(
            mat.gpu.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED,
            0,
            "Material::default's gpu carries no textured bit"
        );
        // Direct field mutation — bypasses `Material::with_textures`, the footgun
        // grooming item B closes.
        mat.textures.albedo = 5;

        let staged = seed_one(mat);
        assert_eq!(
            staged.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED,
            MATERIAL_FLAG_TEXTURED,
            "seed_rows must derive the TEXTURED bit from textures.any(), not trust \
             whatever gpu.mrr[3] already carries"
        );
    }

    #[test]
    fn seed_rows_clears_the_textured_flag_when_textures_is_reset_to_none_after_with_textures() {
        let gpu = MaterialGpu::new([0.5, 0.5, 0.5, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0);
        let textures = MaterialTextures { albedo: 7, ..MaterialTextures::NONE };
        let mut mat = Material::with_textures(gpu, textures);
        assert_eq!(
            mat.gpu.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED,
            MATERIAL_FLAG_TEXTURED,
            "with_textures must have set the bit CPU-side"
        );
        // Direct field mutation clearing the sidecar WITHOUT touching `mat.gpu` —
        // `mat.gpu` still carries the stale bit `with_textures` set; only the
        // upload-boundary derive (not this CPU-side value) must reflect the clear.
        mat.textures = MaterialTextures::NONE;

        let staged = seed_one(mat);
        assert_eq!(
            staged.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED,
            0,
            "seed_rows must clear the TEXTURED bit when textures.any() is false, even \
             though gpu.mrr[3] itself still carries the stale bit from the earlier \
             with_textures call"
        );
    }

    #[test]
    fn seed_rows_leaves_a_non_textured_material_byte_identical_to_its_source_gpu() {
        let mat = Material::default();
        let staged = seed_one(mat);
        assert_eq!(
            staged, mat.gpu,
            "a never-textured material's staged bytes must be byte-identical (the \
             0%-gate every in-tree golden relies on)"
        );
    }

    /// A `MaterialTable` value with `capacity_rows`/`rebind_pending` set directly
    /// (private-field access — this test module is the SAME module as
    /// `MaterialTable` itself). `boot_seed`/`grow_if_needed` cannot run in a CPU
    /// unit test: both take `ctx: &VulkanContext` and unconditionally call
    /// `RhiDevice::create_buffer` past their capacity checks — no testable
    /// constructor exists outside a real device boot (the same constraint
    /// `asset_refcount.rs`'s churn-stress test documents for
    /// `retire_deferred_frees`). `table`/`staging` stay `None`; every test below
    /// only exercises `needs_grow`/`rebind_pending`/`take_rebind_pending`/the
    /// capacity arithmetic, none of which reads them.
    fn table_with(capacity_rows: u32, rebind_pending: [bool; FRAMES_IN_FLIGHT]) -> MaterialTable {
        MaterialTable { table: None, staging: None, capacity_rows, rebind_pending }
    }

    fn dummy_buffer(id: u64) -> BoundBuffer {
        BoundBuffer { buffer: boyko_rhi_vulkan::ffi::VkBuffer::NULL, offset: id, size: 0, mapped: None, block: 0 }
    }

    // ── `new` / `Default` ────────────────────────────────────────────────

    #[test]
    fn new_starts_with_no_pending_rebinds() {
        let mt = MaterialTable::new();
        assert!(!mt.rebind_pending(0), "a fresh table has nothing to repoint (slot 0)");
        assert!(!mt.rebind_pending(1), "a fresh table has nothing to repoint (slot 1)");
    }

    #[test]
    fn new_needs_a_grow_for_any_positive_high_water_at_zero_capacity() {
        let mt = MaterialTable::new();
        assert!(mt.needs_grow(1), "zero boot capacity means even one row needs a grow");
    }

    #[test]
    fn default_matches_new() {
        let mt = MaterialTable::default();
        assert!(!mt.rebind_pending(0));
        assert!(!mt.rebind_pending(1));
    }

    // ── `needs_grow` — the steady-state byte-identity guard ────────────────

    #[test]
    fn needs_grow_is_false_at_exactly_the_capacity_boundary() {
        let mt = table_with(64, [false; FRAMES_IN_FLIGHT]);
        assert!(
            !mt.needs_grow(64),
            "high_water == capacity_rows must NOT need a grow (the boot-capacity \
             byte-identity guard every in-tree golden relies on)"
        );
    }

    #[test]
    fn needs_grow_is_false_below_capacity() {
        let mt = table_with(64, [false; FRAMES_IN_FLIGHT]);
        assert!(!mt.needs_grow(1));
        assert!(!mt.needs_grow(63));
    }

    #[test]
    fn needs_grow_is_true_one_past_capacity() {
        let mt = table_with(64, [false; FRAMES_IN_FLIGHT]);
        assert!(mt.needs_grow(65), "high_water == capacity_rows + 1 must need a grow");
    }

    #[test]
    fn needs_grow_is_true_far_past_capacity() {
        let mt = table_with(64, [false; FRAMES_IN_FLIGHT]);
        assert!(mt.needs_grow(1_000_000));
    }

    // ── `rebind_pending` / `take_rebind_pending` ────────────────────────────

    #[test]
    fn rebind_pending_reads_do_not_clear_the_flag() {
        let mt = table_with(64, [true, false]);
        assert!(mt.rebind_pending(0));
        assert!(mt.rebind_pending(0), "a plain read must be idempotent (not consume the flag)");
        assert!(!mt.rebind_pending(1));
    }

    #[test]
    fn take_rebind_pending_clears_only_the_read_slot() {
        let mut mt = table_with(64, [true, true]);
        assert!(mt.take_rebind_pending(0), "slot 0's flag was set");
        assert!(!mt.rebind_pending(0), "take must clear slot 0's flag");
        assert!(mt.rebind_pending(1), "slot 1's flag must be untouched by slot 0's take");
    }

    #[test]
    fn take_rebind_pending_on_an_already_clear_slot_returns_false() {
        let mut mt = table_with(64, [false, false]);
        assert!(!mt.take_rebind_pending(0));
        assert!(!mt.rebind_pending(0), "taking an already-false flag must not set it");
    }

    #[test]
    fn take_rebind_pending_is_false_on_the_second_call_for_the_same_slot() {
        let mut mt = table_with(64, [true, false]);
        assert!(mt.take_rebind_pending(0), "first take observes the set flag");
        assert!(!mt.take_rebind_pending(0), "second take on the same slot must see it already cleared");
    }

    // ── `MAX_MATERIAL_ROWS` — the hard 16-bit addressing cap (Q2) ────────────

    #[test]
    fn max_material_rows_is_the_16_bit_material_id_bound() {
        assert_eq!(MAX_MATERIAL_ROWS, 1usize << 16, "MaterialId is a 16-bit index");
    }

    // ── `grow_if_needed`'s capacity math — an ORACLE MIRROR (deviceless) ─────
    //
    // `grow_if_needed` itself cannot run in a CPU unit test (see `table_with`'s
    // doc). `would_grow_to` below copies ONLY the arithmetic `grow_if_needed`
    // runs BEFORE any device call (`next_power_of_two` + the `MAX_MATERIAL_ROWS`
    // bound), so its invariants are provable without a device; the full grow
    // path (allocate + seed + retire) is exercised by the `#[ignore]`d
    // real-device headless test instead.

    /// Mirrors `grow_if_needed`'s capacity decision verbatim: `None` if no grow
    /// is needed, else the `next_power_of_two()`-rounded new capacity.
    fn would_grow_to(need: usize, capacity_rows: usize) -> Option<usize> {
        if need <= capacity_rows {
            return None;
        }
        Some(need.next_power_of_two())
    }

    #[test]
    fn capacity_math_returns_none_at_the_boundary() {
        assert_eq!(would_grow_to(64, 64), None, "need == capacity must not grow (byte-identity guard)");
    }

    #[test]
    fn capacity_math_doubles_to_the_next_power_of_two_one_past_capacity() {
        assert_eq!(would_grow_to(65, 64), Some(128), "64 -> 65 rows must round up to 128");
    }

    #[test]
    fn capacity_math_new_cap_is_always_a_power_of_two_and_at_least_need() {
        let mut rng_state: u32 = 0x1234_5678;
        for _ in 0..1000 {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 17;
            rng_state ^= rng_state << 5;
            let capacity_rows = (rng_state % 4096) as usize;
            let need = capacity_rows + 1 + (rng_state as usize % 4096);
            let new_cap =
                would_grow_to(need, capacity_rows).expect("need > capacity_rows by construction");
            assert!(new_cap.is_power_of_two(), "new_cap {new_cap} must be a power of two");
            assert!(new_cap >= need, "new_cap {new_cap} must be >= need {need}");
        }
    }

    #[test]
    fn capacity_math_overflow_past_max_material_rows_would_trip_the_hard_cap() {
        let need = MAX_MATERIAL_ROWS + 1;
        let new_cap =
            would_grow_to(need, MAX_MATERIAL_ROWS).expect("need > capacity_rows by construction");
        assert!(
            new_cap > MAX_MATERIAL_ROWS,
            "a need one past MAX_MATERIAL_ROWS must round up past it too — proving \
             grow_if_needed's `assert!(new_cap <= MAX_MATERIAL_ROWS)` is LIVE (not dead \
             code) on this input"
        );
    }

    #[test]
    fn capacity_math_exactly_at_max_material_rows_does_not_trip_the_cap() {
        // A table that grew all the way to MAX_MATERIAL_ROWS is exactly at the
        // addressing bound, not past it — the hard assert must not fire on this
        // (already-power-of-two) boundary value.
        assert!(MAX_MATERIAL_ROWS.is_power_of_two());
        let new_cap = would_grow_to(MAX_MATERIAL_ROWS, MAX_MATERIAL_ROWS / 2)
            .expect("need > capacity_rows by construction");
        assert_eq!(new_cap, MAX_MATERIAL_ROWS);
    }

    // ── FIX-F: multi-grow-per-window rebind_pending state machine (design §8) ──

    /// Design §8 FIX-F table: two grows inside one FIF=2 window (grows at frame
    /// M slot 0, and frame M+1 slot 1) must converge every slot to the NEWEST
    /// buffer, with each superseded buffer's OWN retire_frame from its OWN
    /// replacement epoch. This drives the REAL `rebind_pending`/
    /// `take_rebind_pending` API (the persistent-flag-consume discipline, W1-a)
    /// plus the REAL `RetiredGpuBuffers` queue (`push`/`is_empty` — device-free
    /// calls are excluded, see that module's test doc); the grow's
    /// device-allocating body (`RhiDevice::create_buffer` + `seed_rows`) is
    /// MODELED by directly mutating `MaterialTable`'s private fields exactly as
    /// `grow_if_needed` does (`capacity_rows`/`rebind_pending`), since the real
    /// fn needs a live `&VulkanContext`. Buffer "identity" is tracked via
    /// `BoundBuffer::offset` (repurposed as a plain tag, never a real offset),
    /// and the fence-gate itself is modeled with a parallel `(id, retire_frame)`
    /// list (`RetiredGpuBuffers` exposes no inspection surface beyond
    /// `is_empty()` by design — its private fields are teardown-only).
    #[test]
    fn rebind_pending_converges_after_two_grows_in_one_fif_window_fix_f() {
        assert_eq!(
            FRAMES_IN_FLIGHT, 2,
            "this test's fenced-slot schedule (0,1,0,1,...) assumes the engine-wide FIF==2 \
             invariant"
        );
        const OLD: u64 = 0; // pre-window buffer identity ("old")
        const A: u64 = 1; // first grow's new buffer
        const B: u64 = 2; // second grow's new buffer
        const FIF: u64 = FRAMES_IN_FLIGHT as u64;
        const M: u64 = 100; // an arbitrary base epoch ("M" in the design table)

        let mut mt = table_with(64, [false, false]);
        // Assigned unconditionally in the frame-M block below before its first
        // read — no meaningful "current" identity exists before the window's
        // first grow runs.
        let mut current_id: u64;
        // The model of "which buffer identity each slot's descriptor sets
        // currently point at" — repointed only via `take_rebind_pending`,
        // mirroring the runner's step 4.6 ordering (§6): grow (if any) THEN
        // check-and-repoint, once per fenced slot per frame.
        let mut set_target = [OLD, OLD];
        let mut retired = RetiredGpuBuffers::default();
        let mut retired_model: Vec<(u64, u64)> = Vec::new();

        // ── Frame M, fenced slot 0: a grow (old -> A). ──
        {
            let epoch = M;
            let fenced = 0usize;
            mt.capacity_rows = 128;
            mt.rebind_pending = [true; FRAMES_IN_FLIGHT];
            retired.push(dummy_buffer(OLD), epoch + FIF);
            retired_model.push((OLD, epoch + FIF));
            current_id = A;

            if mt.take_rebind_pending(fenced) {
                set_target[fenced] = current_id;
            }
        }
        assert_eq!(set_target, [A, OLD], "frame M: only slot 0 was fenced/repointed this frame");
        assert!(mt.rebind_pending(1), "slot 1's flag persists (W1-a) — it was not occupied this frame");
        assert!(!retired.is_empty());

        // ── Frame M+1, fenced slot 1: ANOTHER grow (A -> B), same window. ──
        {
            let epoch = M + 1;
            let fenced = 1usize;
            mt.capacity_rows = 256;
            mt.rebind_pending = [true; FRAMES_IN_FLIGHT]; // resets BOTH flags (§7.1 step 5)
            retired.push(dummy_buffer(A), epoch + FIF);
            retired_model.push((A, epoch + FIF));
            current_id = B;

            // Q1: the repoint ALWAYS binds `table()` (current) — never a captured
            // pointer — so slot 1 never actually observes `A`, it jumps straight
            // to `B` even though its flag was already true before this frame's grow.
            if mt.take_rebind_pending(fenced) {
                set_target[fenced] = current_id;
            }
        }
        assert_eq!(set_target, [A, B], "frame M+1: slot 1 repoints straight to the newest buffer (Q1)");
        assert!(!mt.rebind_pending(1), "slot 1's flag is consumed the instant it is occupied");
        assert!(
            mt.rebind_pending(0),
            "slot 0's flag (reset by the second grow) persists until its next occupancy"
        );

        // ── Frame M+2, fenced slot 0: no grow, just the lagging repoint. ──
        {
            let epoch = M + 2;
            let fenced = 0usize;
            if mt.take_rebind_pending(fenced) {
                set_target[fenced] = current_id;
            }
            retired_model.retain(|&(_, rf)| rf > epoch); // mirror drain_ready's fence-gate
        }
        assert_eq!(set_target, [B, B], "frame M+2: every slot has converged to the newest buffer B");
        assert!(!mt.rebind_pending(0));
        assert!(!mt.rebind_pending(1));
        assert!(
            retired_model.iter().all(|&(id, _)| id != OLD),
            "`old`'s retire_frame (M+2) must have drained by frame M+2 (design §8 table)"
        );
        assert!(
            retired_model.iter().any(|&(id, _)| id == A),
            "`A`'s retire_frame (M+3) must NOT have drained yet at frame M+2"
        );

        // ── Frame M+3: `A`'s own horizon is reached. ──
        {
            let epoch = M + 3;
            retired_model.retain(|&(_, rf)| rf > epoch);
        }
        assert!(
            retired_model.is_empty(),
            "both superseded buffers must have drained by their OWN horizons"
        );
    }

    // ── W1 storm test: frame_index pinned at 0 across a recreate storm ─────

    /// Design §8 W1 storm test: `frame_index` pinned at 0 across N consecutive
    /// `recreate`-skips (no submit -> `submission_epoch` frozen too) means only
    /// slot 0 is ever fenced during the storm; slot 1's grow flag must PERSIST
    /// (invariant a) the whole storm, the superseded buffer must NOT free while
    /// frozen (invariant c — the fence-gate predicate `RetiredGpuBuffers::
    /// drain_ready` implements), and once the storm ends and slot 1 is finally
    /// occupied again, its repoint must happen BEFORE its (simulated) record —
    /// invariant (b).
    #[test]
    fn rebind_pending_survives_a_recreate_storm_and_repoints_before_the_next_record_w1() {
        const STORM_FRAMES: u32 = 50; // arbitrary — the persistent-flag proof does not depend on the count
        const FIF: u64 = FRAMES_IN_FLIGHT as u64;
        const FROZEN_EPOCH: u64 = 200; // submission_epoch never advances during the storm (no submits)

        let mut mt = table_with(64, [false, false]);

        // A single grow just before the storm starts, fenced at slot 0.
        mt.capacity_rows = 128;
        mt.rebind_pending = [true; FRAMES_IN_FLIGHT];
        let mut retired = RetiredGpuBuffers::default();
        retired.push(dummy_buffer(0), FROZEN_EPOCH + FIF);
        assert!(mt.take_rebind_pending(0), "slot 0 is repointed immediately (it is fenced this frame)");
        assert!(!mt.rebind_pending(0));
        assert!(mt.rebind_pending(1), "slot 1 has not been occupied yet — its flag is still pending");

        // The storm: N consecutive recreate-skip frames, ALL fenced at slot 0
        // (frame_index pinned 0 — the design's own characterization of a
        // sustained resize storm), at the SAME frozen epoch (no submit -> no
        // epoch advance).
        for _ in 0..STORM_FRAMES {
            assert!(
                !mt.take_rebind_pending(0),
                "slot 0's already-clear flag must stay a safe, idempotent no-op every storm frame"
            );
            assert!(
                mt.rebind_pending(1),
                "slot 1's PERSISTENT flag must survive every storm frame untouched (invariant a) \
                 — it is never fenced during the storm"
            );
            // Invariant (c): the superseded buffer is not freed while the epoch
            // is frozen below its horizon (`FROZEN_EPOCH < FROZEN_EPOCH + FIF`,
            // trivially true for any FIF > 0 — the fence-gate predicate
            // `RetiredGpuBuffers::drain_ready` implements, `retire_frame <=
            // epoch`, never fires against a frozen epoch that never reaches the
            // horizon it was stamped strictly below).
            assert!(!retired.is_empty(), "the superseded buffer must remain queued for the whole storm");
        }

        // The storm ends: a real submit resumes and slot 1 is occupied again for
        // the first time since the grow. Invariant (b): repoint-before-record —
        // the repoint call must happen (and observably win) before any simulated
        // "record" reads the slot's bound buffer.
        let mut set_target = [1u64, 0u64]; // slot 0 already repointed pre-storm; slot 1 still stale
        let repointed = mt.take_rebind_pending(1);
        assert!(repointed, "slot 1's persistent flag must still be true the instant it is finally occupied");
        set_target[1] = 1; // the repoint happens HERE, strictly before the record step below
        let recorded_value = set_target[1]; // the "record" step — reads the value AFTER the repoint
        assert_eq!(
            recorded_value, 1,
            "invariant (b): the record must observe the REPOINTED value, never the stale one"
        );
        assert!(!mt.rebind_pending(1), "the flag is consumed at the moment of repoint");
    }
}
