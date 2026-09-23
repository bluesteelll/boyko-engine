//! Rung B2's two record-only scans over a booted world: the **id census** (AL:M-A13 / MD:M-K3, the
//! RK-12 budget of `MAX_COMPONENTS` = 512 slots) and **G-RES** (plan gate UG-20: committed
//! kernel-column bytes after boot).
//!
//! Depends on `boyko_ecs` and `boyko_macros` only, so `boyko_demo`'s test includes it by `#[path]`
//! (the `boyko_ecs/benches/gj1_flag_cost.rs` precedent) — both crates link those two.
//!
//! # Every figure says what it is, and an unobservable one says ABSENT
//!
//! A number that reads 0 because the reader could not see the thing is this repository's most
//! catalogued false green, so a quantity with no public reader on this tree is printed
//! `ABSENT(<reason>)`, never `0`.
//!
//! # The id census
//!
//! * **`NEXT_ID` high-water** — a NON-consuming scan of `get_layout(0..MAX_COMPONENTS)`. Both
//!   production mints (`register_new` and the dynamic-tag CAS) write a layout into the slot they
//!   take, so the slots below the counter are contiguous from 0 and the counter is the first empty
//!   slot. [`id_cross_check`] then mints ONE probe with `register_new` and requires it to land on
//!   exactly that slot — the anti-vacuity that ties the scan to the counter the mint writes.
//! * **Pinned slots** — occupied slots above the first gap: `register_layout` pins, which only
//!   `boyko_physics` makes in production (its scratch band grows down from 511 to the private
//!   `SCRATCH_REGION_MIN_ID` = 384, which this file STATES as [`RESERVED_FLOOR_STATED`] rather than
//!   reads). Both the lowest OCCUPIED pinned slot and the stated reserved floor are printed.
//! * **`NEXT_RESOURCE_ID`** — the resource registry module is `pub(crate)`, so its slots cannot be
//!   scanned; each reading point mints one [`ResourceProbe`] through the public
//!   `resources::register_new` and reports the returned id minus the probes this census minted
//!   before it. That is a CONSUMING read, labelled as such.
//! * **`DYN_ID_CEILING`** does not exist on this tree (the allocator design withdrew it with the
//!   band, U-2): printed ABSENT.
//!
//! # G-RES
//!
//! * **Table columns** — every archetype, every component its signature holds:
//!   `committed_rows × size_of`. That is the committed DATA region only; each pool also commits
//!   two tick columns and a stagger pad per row the public API does not size, so the figure is a
//!   floor on committed table bytes, labelled `table_data_bytes_committed`.
//! * **Entity store** — `EntityMaster::committed_slots` (slots) and `EntityMaster::memory_usage`
//!   (resident bytes of the slot store plus the recycled stack).
//! * **Dense stores** — `DenseStore` exposes its high-water `len` and `stride_bytes` but no
//!   committed frontier, so the row is `len × stride` labelled HIGH-WATER and the committed figure
//!   is ABSENT.
//! * **ABSENT:** `ScratchColumn` committed bytes (the design's own G-RES subject: `ScratchColumn`
//!   exposes only `len` / `capacity`), the KC-04 counters (`THREAD_CTX_CLAIMS`, `…_REFUSED`,
//!   `…_PEAK`, `POOL_WORKERS_CLAMPED`, `POOL_RESERVE_DIPS` — not in code until KC-04 / D-M6), and
//!   D-E20's event lanes and reader-buffer bytes (D-E20 is not built).

use boyko_ecs::ecs::core::component::component_registry::{
    MAX_COMPONENTS, get_layout, register_new,
};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_macros::Resource;

/// `boyko_physics::scratch_ids::SCRATCH_REGION_MIN_ID` (`MAX_COMPONENTS - 128`), which is private
/// there: STATED here, not read. Printed beside the measured lowest pinned slot.
pub const RESERVED_FLOOR_STATED: usize = MAX_COMPONENTS - 128;

/// The one component-id probe [`id_cross_check`] mints.
struct IdCensusProbe;

/// One reading of the component-id space.
#[derive(Clone, Copy, Debug)]
pub struct IdScan {
    /// The first empty slot — `NEXT_ID`, read without minting.
    pub next_id: usize,
    /// Occupied slots above the first gap (`register_layout` pins).
    pub pinned: usize,
    /// The lowest occupied pinned slot, if any.
    pub lowest_pinned: Option<usize>,
}

/// Non-consuming scan of every component slot.
pub fn scan_ids() -> IdScan {
    let occupied: Vec<bool> = (0..MAX_COMPONENTS)
        .map(|i| get_layout(i).is_some())
        .collect();
    let next_id = occupied.iter().position(|o| !o).unwrap_or(MAX_COMPONENTS);
    let pinned_slots: Vec<usize> = (next_id..MAX_COMPONENTS).filter(|&i| occupied[i]).collect();
    IdScan {
        next_id,
        pinned: pinned_slots.len(),
        lowest_pinned: pinned_slots.first().copied(),
    }
}

/// Mints ONE probe id with `register_new` and requires it to be the scanned high-water: the scan
/// reads the same counter the mint writes. Call once, at the end of a process.
pub fn id_cross_check(scan: &IdScan) -> usize {
    let minted = register_new::<IdCensusProbe>();
    assert_eq!(
        minted, scan.next_id,
        "ID-CENSUS ANTI-VACUITY: the consuming probe minted slot {minted}, the non-consuming scan \
         read NEXT_ID = {} — the scan is not reading the counter the mint writes",
        scan.next_id
    );
    minted
}

/// The four resource probes, one per reading point.
#[derive(Resource)]
pub struct ResourceProbe0;
#[derive(Resource)]
pub struct ResourceProbe1;
#[derive(Resource)]
pub struct ResourceProbe2;
#[derive(Resource)]
pub struct ResourceProbe3;

/// `NEXT_RESOURCE_ID` at reading point `k` (0-based), by minting probe `k` and subtracting the `k`
/// probes minted before it.
pub fn next_resource_id(k: usize) -> usize {
    use boyko_ecs::ecs::core::resources::register_new as mint;
    let raw = match k {
        0 => mint::<ResourceProbe0>(),
        1 => mint::<ResourceProbe1>(),
        2 => mint::<ResourceProbe2>(),
        3 => mint::<ResourceProbe3>(),
        _ => panic!("ID-CENSUS: only four resource reading points are provisioned"),
    };
    raw - k
}

/// One `ID-CENSUS` line. `app` / `point` label the row; the structural RED (the counter at or into
/// the pinned band) fires here.
pub fn id_line(app: &str, point: &str, k: usize) -> IdScan {
    let scan = scan_ids();
    let res = next_resource_id(k);
    let lowest = scan
        .lowest_pinned
        .map_or_else(|| "none".to_string(), |l| l.to_string());
    let headroom_occupied = scan.lowest_pinned.map_or_else(
        || "n/a(no pinned slot)".to_string(),
        |l| (l as isize - scan.next_id as isize).to_string(),
    );
    println!(
        "ID-CENSUS app={app} point={point} next_id={} next_resource_id={res}(consuming probe #{k}) \
         pinned={} lowest_pinned={lowest} reserved_floor_stated={RESERVED_FLOOR_STATED} \
         dyn_id_ceiling=ABSENT(withdrawn with the band, U-2; not in code) \
         headroom_to_lowest_pinned={headroom_occupied} \
         headroom_to_reserved_floor={} headroom_to_512={}",
        scan.next_id,
        scan.pinned,
        RESERVED_FLOOR_STATED as isize - scan.next_id as isize,
        MAX_COMPONENTS as isize - scan.next_id as isize - scan.pinned as isize,
    );
    if let Some(l) = scan.lowest_pinned {
        assert!(
            scan.next_id < l,
            "ID-CENSUS: NEXT_ID {} has reached the lowest pinned slot {l} — the production counter \
             is in the pinned band",
            scan.next_id
        );
    }
    scan
}

/// One G-RES reading.
#[derive(Clone, Copy, Debug, Default)]
pub struct GRes {
    pub archetypes: usize,
    pub table_pools: usize,
    pub table_rows_committed: usize,
    pub table_data_bytes_committed: usize,
    pub entity_slots_committed: usize,
    pub entity_store_bytes: usize,
    pub dense_stores: usize,
    pub dense_rows_high_water: usize,
    pub dense_bytes_high_water: usize,
}

/// The G-RES scan over `world`'s public read API.
pub fn scan_g_res(world: &EcsMaster) -> GRes {
    let am = world.archetype_master();
    let n = am.archetype_count();
    let mut g = GRes::default();
    // Archetype ids need not be dense; walk raw ids until every counted archetype is found.
    let mut raw = 0usize;
    while g.archetypes < n && raw < (1 << 20) {
        if let Some(a) = am.get_archetype(ArchetypeId(raw)) {
            g.archetypes += 1;
            let mask = a.signature().mask();
            for cid in 0..MAX_COMPONENTS {
                if !mask.contains(ComponentId(cid)) {
                    continue;
                }
                if let Some(pool) = a.component_pools().get_pool(ComponentId(cid)) {
                    g.table_pools += 1;
                    g.table_rows_committed += pool.committed_rows();
                    g.table_data_bytes_committed +=
                        pool.committed_rows() * pool.component_layout().size();
                }
            }
        }
        raw += 1;
    }
    assert_eq!(
        g.archetypes, n,
        "G-RES: archetype_count() is {n} but the raw-id walk found {}",
        g.archetypes
    );
    let em = world.entity_master();
    g.entity_slots_committed = em.committed_slots();
    g.entity_store_bytes = em.memory_usage();
    let dense = world.dense_registry();
    for &id in dense.dense_ids() {
        if let Some(s) = dense.store(id) {
            g.dense_stores += 1;
            g.dense_rows_high_water += s.len();
            g.dense_bytes_high_water += s.len() * s.stride_bytes();
        }
    }
    g
}

/// One greppable `G-RES` line, with its anti-vacuity: a booted world with no archetype or no
/// committed table bytes means the scan read nothing.
pub fn g_res_line(app: &str, point: &str, world: &EcsMaster) -> GRes {
    let g = scan_g_res(world);
    println!(
        "G-RES app={app} point={point} archetypes={} table_pools={} table_rows_committed={} \
         table_data_bytes_committed={} entity_slots_committed={} entity_store_bytes={} \
         dense_stores={} dense_rows_high_water={} dense_bytes_high_water={} \
         dense_committed=ABSENT(DenseStore exposes no committed frontier) \
         scratch_committed=ABSENT(ScratchColumn exposes only len/capacity; PC4a) \
         kc04=ABSENT(THREAD_CTX_CLAIMS/REFUSED/PEAK, POOL_WORKERS_CLAMPED, POOL_RESERVE_DIPS not in code; KC-04/D-M6) \
         events=ABSENT(D-E20 lanes and reader buffers not built)",
        g.archetypes,
        g.table_pools,
        g.table_rows_committed,
        g.table_data_bytes_committed,
        g.entity_slots_committed,
        g.entity_store_bytes,
        g.dense_stores,
        g.dense_rows_high_water,
        g.dense_bytes_high_water,
    );
    assert!(
        g.archetypes > 0 && g.table_data_bytes_committed > 0,
        "G-RES ANTI-VACUITY: app={app} point={point} read {} archetypes and {} committed table \
         bytes — a booted world has both",
        g.archetypes,
        g.table_data_bytes_committed
    );
    g
}
