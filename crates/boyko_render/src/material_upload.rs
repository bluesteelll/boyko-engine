//! [`MaterialUploadStaging`] and [`stage_material_edits`] — how a Tier-1 material edit reaches
//! the GPU table (dynamic-materials DM1, design F1 option A4, "compact staging").
//!
//! `Assets<Material>` marks every row whose GPU image may differ from the CPU authority (the
//! kernel's edited set: `get_mut`, `add`, `fill`, `remove`, `retire`). Once per `Main` frame the
//! stager drains that set in ascending row order into this resource:
//!
//! - [`rows`](MaterialUploadStaging::rows): the edited rows' upload bytes, PACKED in drain order
//!   (`k`-th drained row at `[k·48, (k+1)·48)`). A `Loaded` row stages
//!   [`derive_gpu_row`](crate::material_table::derive_gpu_row) of its value; any other row
//!   (freed, `Loading`, `Failed`, `Retiring`) stages zeros — the value a full image
//!   (`MaterialTable::seed_rows`) writes for it.
//! - [`runs`](MaterialUploadStaging::runs): one copy region per run of consecutive rows. The drain
//!   is ascending, so a run is contiguous on both sides: `src = k·48` in the compact bytes, `dst =
//!   row·48` in the table. The element type is [`BufferCopy`], layout-matched to `VkBufferCopy`,
//!   so the recorder hands the whole list to ONE `vkCmdCopyBuffer`.
//!
//! The runner memcpys [`rows`](MaterialUploadStaging::rows) into the fenced staging slot `s` and
//! records the runs. **Invariant (design F1):** every byte range copied from slot `s` in frame N
//! was written into slot `s` in frame N — the runs reference only `[0, k·48)`, which the runner
//! writes that frame, so the invariant holds by construction. (A row-MIRRORED slot copied as one
//! `[min_row, max_row]` range does not: it re-copies a row the other slot wrote a frame later.
//! This module's tests pin both.)
//!
//! **Idle frame:** `edited_any()` is false, so the stager clears nothing it did not fill and
//! stages nothing — the runner then declares no copy (the `light_upload` idiom).
//!
//! The stager is ordered after `gather_mesh_draws` (`boyko_app::plugins`), so every material id
//! the gather scattered this frame belongs to a row minted before the drain: a row minted into
//! the table's headroom (live defect D-1) is staged on the frame it can first be drawn.

use boyko_ecs::ecs::constants::pool_reserve_rows;
use boyko_ecs::ecs::core::asset::{Assets, register_asset_layout};
use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_macros::Resource;
use boyko_rhi::BufferCopy;

use crate::material::{Material, MaterialGpu};
use crate::material_table::derive_gpu_row;

/// Bytes per material row: one [`MaterialGpu`] (48, const-asserted in `material.rs`).
pub const MATERIAL_ROW_BYTES: u64 = core::mem::size_of::<MaterialGpu>() as u64;

/// The all-zero row a freed (or never-filled) material stages — the bytes a full image
/// (`MaterialTable::seed_rows`) leaves for a row [`Assets::iter`] does not yield.
const ZERO_ROW: MaterialGpu = MaterialGpu { base_color: [0.0; 4], mrr: [0.0; 4], emissive: [0.0; 4] };

/// The compact, per-frame material upload staging: this frame's edited rows packed in drain
/// order, and the copy runs that scatter them into the table (see the module doc).
///
/// A `Send` resource on the ECS's own storage (two `ScratchColumn`s — Principle 0: no side
/// store), refilled by [`stage_material_edits`] once per `Main` frame and read by the runner
/// through a shared borrow. Both columns reserve address space only until first written, and
/// keep their committed pages across frames, so a steady stream of edits allocates nothing.
#[derive(Resource)]
pub struct MaterialUploadStaging {
    /// The edited rows' upload bytes, in drain (ascending row) order.
    rows: ScratchColumn<MaterialGpu>,
    /// One region per run of consecutive rows: `src` into [`Self::rows`], `dst` into the table.
    runs: ScratchColumn<BufferCopy>,
}

impl Default for MaterialUploadStaging {
    /// Empty staging. Registers one `ComponentId` per element type (memoized process-wide by
    /// [`register_asset_layout`]) and reserves each column at [`pool_reserve_rows`] of its element
    /// size — the `MeshRenderScratch` construction idiom.
    fn default() -> Self {
        let row_id = register_asset_layout::<MaterialGpu>(None);
        let run_id = register_asset_layout::<BufferCopy>(None);
        Self {
            rows: ScratchColumn::new(row_id, pool_reserve_rows(core::mem::size_of::<MaterialGpu>())),
            runs: ScratchColumn::new(run_id, pool_reserve_rows(core::mem::size_of::<BufferCopy>())),
        }
    }
}

impl MaterialUploadStaging {
    /// The edited rows staged this frame, packed in drain order.
    #[inline]
    pub fn rows(&self) -> &[MaterialGpu] {
        self.rows.as_read_slice()
    }

    /// This frame's copy regions (`src` into [`Self::rows`]' bytes, `dst` into the table), one per
    /// run of consecutive rows, in ascending `dst` order.
    #[inline]
    pub fn runs(&self) -> &[BufferCopy] {
        self.runs.as_read_slice()
    }

    /// The number of rows staged this frame.
    #[inline]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// `true` iff nothing is staged this frame.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Empties both columns (no free: the committed pages stay for the next edit frame).
    #[inline]
    pub fn clear(&mut self) {
        self.rows.build_view().clear();
        self.runs.build_view().clear();
    }

    /// Replaces the staging with `assets`' edited set: clears both columns, then drains every
    /// marked row in ascending order into [`Self::rows`], merging consecutive rows into one run.
    /// Clears every mark (`Assets::drain_edited`).
    pub fn stage_from(&mut self, assets: &mut Assets<Material>) {
        let mut rows = self.rows.build_view();
        let mut runs = self.runs.build_view();
        rows.clear();
        runs.clear();
        assets.drain_edited(|row, material| {
            let k = rows.push(material.map_or(ZERO_ROW, derive_gpu_row));
            let src = u64::from(k) * MATERIAL_ROW_BYTES;
            let dst = u64::from(row) * MATERIAL_ROW_BYTES;
            // Rows drain ascending and pack densely, so the previous run extends iff this row
            // follows its last destination row; its source end is `src` by construction.
            if let Some(last) = runs.as_mut_slice().last_mut()
                && last.dst_offset + last.size == dst
            {
                debug_assert_eq!(last.src_offset + last.size, src, "invariant: compact rows are dense");
                last.size += MATERIAL_ROW_BYTES;
            } else {
                runs.push(BufferCopy { src_offset: src, dst_offset: dst, size: MATERIAL_ROW_BYTES });
            }
        });
    }
}

/// Dynamic-materials DM1: drains `Assets<Material>`'s edited set into
/// [`MaterialUploadStaging`] — the whole CPU side of a Tier-1 edit's path to the GPU table.
///
/// An idle frame (nothing marked) stages nothing and takes no mutable access to the asset table;
/// a non-empty staging left from the previous frame is cleared, so the runner never re-copies it.
/// Registered by `boyko_app::plugins::register_main_frame_systems`, ordered after
/// `gather_mesh_draws` (see the module doc).
// SystemParams are consumed by value by the SystemParam contract.
#[allow(clippy::needless_pass_by_value)]
pub fn stage_material_edits(
    mut assets: ResMut<Assets<Material>>,
    mut staging: ResMut<MaterialUploadStaging>,
) {
    if assets.edited_any() {
        staging.stage_from(&mut assets);
    } else if !staging.is_empty() {
        staging.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_ecs::ecs::core::asset::Handle;

    const STRIDE: u64 = MATERIAL_ROW_BYTES;

    /// A material whose every lane carries `v`, so a row's bytes name the value it came from.
    fn mat(v: f32) -> Material {
        Material::from(MaterialGpu { base_color: [v; 4], mrr: [v, v, v, 0.0], emissive: [v; 4] })
    }

    fn run(src_row: u64, dst_row: u64, rows: u64) -> BufferCopy {
        BufferCopy { src_offset: src_row * STRIDE, dst_offset: dst_row * STRIDE, size: rows * STRIDE }
    }

    /// Applies this frame's staging to a CPU model of the device table, exactly as the recorded
    /// copy would: each run copies `size` bytes from the compact rows at `src` to the table at
    /// `dst`. Grows the model with zero rows when the table grew (the full-image path is C5's; this
    /// checks the compact path alone).
    fn apply(table: &mut Vec<MaterialGpu>, staging: &MaterialUploadStaging, high_water: usize) {
        if table.len() < high_water {
            table.resize(high_water, ZERO_ROW);
        }
        for r in staging.runs() {
            let (src, dst, n) = ((r.src_offset / STRIDE) as usize, (r.dst_offset / STRIDE) as usize, (r.size / STRIDE) as usize);
            table[dst..dst + n].copy_from_slice(&staging.rows()[src..src + n]);
        }
    }

    /// The full image of `assets` — zero plus every `Loaded` row's derived bytes, the predicate
    /// `MaterialTable::seed_rows` writes.
    fn full_image(assets: &Assets<Material>) -> Vec<MaterialGpu> {
        let mut image = vec![ZERO_ROW; assets.high_water()];
        for (h, m) in assets.iter() {
            image[h.index() as usize] = derive_gpu_row(m);
        }
        image
    }

    /// The C-8 invariant, with W2's explicit Retiring clause: every row equals the full image,
    /// except a `Loaded`→`Retiring` row, which may hold either its last `Loaded` bytes or zeros
    /// (the compact path keeps the former — `dec_ref` does not mark — and a full image writes the
    /// latter; nothing binds a `Retiring` row, so neither is observable).
    ///
    /// Returns how many rows passed through the `Retiring` clause (held their last `Loaded` bytes
    /// where the full image has zeros), so a caller can prove the clause was exercised.
    fn check(table: &[MaterialGpu], assets: &Assets<Material>) -> Result<usize, String> {
        let image = full_image(assets);
        let mut retiring_kept = 0;
        for (row, want) in image.iter().enumerate() {
            let got = table.get(row).copied().unwrap_or(ZERO_ROW);
            if got == *want {
                continue;
            }
            let retiring_value = assets
                .get_by_index(row as u32)
                .filter(|_| !assets.iter().any(|(h, _)| h.index() as usize == row))
                .map(derive_gpu_row);
            if retiring_value == Some(got) && *want == ZERO_ROW {
                retiring_kept += 1;
                continue;
            }
            return Err(format!("row {row}: table {got:?}, authority {want:?}"));
        }
        Ok(retiring_kept)
    }

    #[test]
    fn an_idle_frame_stages_nothing() {
        let mut assets: Assets<Material> = Assets::with_reserved(4);
        assets.add(mat(1.0));
        let mut staging = MaterialUploadStaging::default();
        staging.stage_from(&mut assets);
        assert_eq!(staging.row_count(), 1, "PREMISE: the mint is staged once");
        staging.stage_from(&mut assets);
        assert_eq!(staging.row_count(), 0, "an idle frame stages no row");
        assert!(staging.runs().is_empty(), "an idle frame stages no run");
    }

    /// The cut's worked example: edits {2, 3, 4, 9} give 4 packed rows and 2 runs.
    #[test]
    fn consecutive_rows_merge_into_one_run_and_the_rows_pack_densely() {
        let mut assets: Assets<Material> = Assets::with_reserved(16);
        let handles: Vec<Handle<Material>> = (0..10).map(|i| assets.add(mat(i as f32))).collect();
        let mut staging = MaterialUploadStaging::default();
        staging.stage_from(&mut assets);
        assert_eq!(staging.runs(), &[run(0, 0, 10)], "the ten mints are one run");

        for &row in &[9usize, 3, 2, 4] {
            assets.get_mut(handles[row]).expect("invariant: live").gpu.emissive[0] = 100.0 + row as f32;
        }
        staging.stage_from(&mut assets);
        assert_eq!(staging.row_count(), 4);
        assert_eq!(staging.runs(), &[run(0, 2, 3), run(3, 9, 1)]);
        let emissive: Vec<f32> = staging.rows().iter().map(|r| r.emissive[0]).collect();
        assert_eq!(emissive, vec![102.0, 103.0, 104.0, 109.0], "rows pack in ascending row order");
    }

    #[test]
    fn a_freed_row_stages_zeros() {
        let mut assets: Assets<Material> = Assets::with_reserved(4);
        assets.add(mat(1.0));
        let b = assets.add(mat(2.0));
        let mut staging = MaterialUploadStaging::default();
        staging.stage_from(&mut assets);
        assets.remove(b).expect("invariant: b is live");
        staging.stage_from(&mut assets);
        assert_eq!(staging.rows(), &[ZERO_ROW]);
        assert_eq!(staging.runs(), &[run(0, 1, 1)]);
    }

    /// Design F1's slot invariant, the pinned layout of DM1 test (2): rows a < b < c; frame N−2
    /// (slot 0) writes b = B0, frame N−1 (slot 1) b = B1, frame N (slot 0) a and c only. The compact
    /// path leaves b = B1. A row-MIRRORED slot copied as one `[min, max]` range re-copies slot 0's
    /// stale b = B0 — the hazard the invariant check must be able to see (the mutation control).
    #[test]
    fn the_slot_invariant_holds_compact_and_fails_row_mirrored() {
        let mut assets: Assets<Material> = Assets::with_reserved(8);
        let h: Vec<Handle<Material>> = (0..4).map(|i| assets.add(mat(i as f32))).collect();
        let (a, b, c) = (h[1], h[2], h[3]);
        let mut staging = MaterialUploadStaging::default();
        let mut table = full_image(&assets);
        staging.stage_from(&mut assets);

        // The mutant: two row-mirrored slots, each copied into its own table as [min, max].
        let mut mirrored = [full_image(&assets), full_image(&assets)];
        let mut mutant_table = full_image(&assets);
        let edits: [&[(Handle<Material>, f32)]; 3] = [&[(b, 50.0)], &[(b, 60.0)], &[(a, 70.0), (c, 80.0)]];
        for (frame, frame_edits) in edits.iter().enumerate() {
            let slot = frame % 2;
            for &(handle, v) in frame_edits.iter() {
                assets.get_mut(handle).expect("invariant: live").gpu.emissive = [v; 4];
            }
            staging.stage_from(&mut assets);
            apply(&mut table, &staging, assets.high_water());

            let (lo, hi) = (
                staging.runs().first().map(|r| (r.dst_offset / STRIDE) as usize).expect("an edit frame"),
                staging.runs().last().map(|r| ((r.dst_offset + r.size) / STRIDE) as usize).expect("an edit frame"),
            );
            let mut k = 0;
            for r in staging.runs() {
                for i in 0..(r.size / STRIDE) as usize {
                    mirrored[slot][(r.dst_offset / STRIDE) as usize + i] = staging.rows()[k];
                    k += 1;
                }
            }
            mutant_table[lo..hi].copy_from_slice(&mirrored[slot][lo..hi]);
        }

        assert_eq!(check(&table, &assets), Ok(0), "compact staging keeps the table equal to the authority");
        assert_eq!(table[2].emissive, [60.0; 4], "b keeps B1");
        assert!(check(&mutant_table, &assets).is_err(), "the row-mirrored [min, max] copier must fail the check");
        assert_eq!(mutant_table[2].emissive, [50.0; 4], "the mutant re-copies slot 0's stale B0");
    }

    /// The C-8 invariant under random op sequences: a table seeded with the boot full image and
    /// then fed only the per-frame compact runs equals the authority's full image after every frame
    /// (W2: a `Retiring` row may be either). Every asset op that can mark or not mark a row is in
    /// the alphabet — `dec_ref` and `retire` included — so the `Retiring` clause is exercised.
    /// Seeded xorshift rather than `proptest`: boyko-render carries no `proptest` dev-dependency
    /// and DM1 adds none.
    #[test]
    fn compact_staging_keeps_the_table_equal_to_the_authority_under_random_ops() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut retiring_kept = 0usize;
        for seq in 0..200 {
            let mut assets: Assets<Material> = Assets::with_reserved(8);
            let boot = 1 + (next() % 6) as usize;
            let mut minted: Vec<Handle<Material>> = (0..boot).map(|i| assets.add(mat(i as f32))).collect();
            let mut staging = MaterialUploadStaging::default();
            staging.stage_from(&mut assets); // the boot marks; the table below is the boot seed
            let mut table = full_image(&assets);
            // Live refcounts per occupant, keyed by the handle (slot + generation) that took them,
            // so a `dec_ref` never outruns the increments of the row's CURRENT occupant.
            let mut refs: Vec<(Handle<Material>, u32)> = Vec::new();
            for frame in 0..40 {
                for _ in 0..(next() % 5) {
                    let pick = minted[(next() as usize) % minted.len()];
                    let v = (next() % 1000) as f32;
                    match next() % 9 {
                        0 | 1 => minted.push(assets.add(mat(v))),
                        2 => minted.push(assets.reserve()),
                        3 => {
                            let _ = assets.fill(pick, mat(v));
                        }
                        4 => assets.fail(pick),
                        5 => {
                            if let Some(m) = assets.get_mut(pick) {
                                m.gpu.emissive = [v; 4];
                            }
                        }
                        6 => {
                            let _ = assets.remove(pick);
                        }
                        7 => {
                            // Only the current occupant (`state` resolves it and excludes Retiring).
                            if assets.state(pick).is_some() && assets.inc_ref(pick.index()) {
                                match refs.iter_mut().find(|(h, _)| *h == pick) {
                                    Some((_, count)) => *count += 1,
                                    None => refs.push((pick, 1)),
                                }
                            }
                        }
                        _ => {
                            if let Some((_, count)) = refs.iter_mut().find(|(h, c)| *h == pick && *c > 0) {
                                *count -= 1;
                                // A stale generation no-ops inside `dec_ref` (the F5 gen-check).
                                if assets.dec_ref(pick.index(), pick.generation()).is_some() && next() % 2 == 0 {
                                    let _ = assets.retire(pick.index());
                                }
                            }
                        }
                    }
                }
                staging.stage_from(&mut assets);
                apply(&mut table, &staging, assets.high_water());
                match check(&table, &assets) {
                    Ok(kept) => retiring_kept += kept,
                    Err(e) => panic!("sequence {seq}, frame {frame}: {e}"),
                }
            }
        }
        assert!(
            retiring_kept > 0,
            "the sweep never produced a Retiring row holding its last Loaded bytes — the W2 clause \
             went unexercised, so this test would pass a rule that ignored it"
        );
    }
}
