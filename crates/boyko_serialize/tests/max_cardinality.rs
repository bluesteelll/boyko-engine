//! Phase S3 (§8 "MAX" validation edge) — a LEGITIMATE high-cardinality save must
//! round-trip in full.
//!
//! The W2 hostile-count tests forge a `u32::MAX` archetype/column count in the
//! header and assert the loader REJECTS them. This test is the opposite: it builds
//! a *real* world whose single archetype carries many distinct component types
//! (one entity, `N` columns), `save_world`s it, `load_world`s it into a fresh
//! `EcsMaster`, and asserts every column blits and every `v` field survives. It is
//! the upper-bound correctness witness for the column/archetype loops — the loader
//! must scale to a wide archetype, not just the 2–4-column fixtures the other suites
//! use.
//!
//! # Why `N` is budget-derived, not a literal `512`
//!
//! Component ids come from a *process-global* `AtomicUsize` counter (`NEXT_ID` in
//! `component_registry`), capped at `MAX_COMPONENTS = 512`. `register_new` does an
//! unconditional `fetch_add` and then `assert!(raw < MAX_COMPONENTS)` — exhausting
//! the counter is a HARD PANIC inside the registry, not a recoverable error. Each
//! integration-test *file* compiles to its own test binary → its own process →
//! a fresh `NEXT_ID`, so this file starts with a near-full 512-id budget. But the
//! budget is *shared* with whatever helper components are linked into THIS binary
//! (none today, but a future helper or a transitively-pulled component would eat
//! into it), so hard-coding `512` would silently start aborting inside
//! `register_new` the day the floor shifts.
//!
//! Instead the test reads how many ids are already minted (by scanning the public
//! `get_layout` table for the first free slot — that index is exactly the value the
//! next `register_new` would mint, i.e. the current `NEXT_ID`) and picks
//! `N = min(450, MAX_COMPONENTS - current_next_id - 16 /* margin */)`. The
//! 16-id margin absorbs any components the round-trip plumbing itself touches
//! after `N` is chosen. A loud `assert!(N >= 256, ...)` fails with a clear message
//! if the environment somehow arrives with a short budget, rather than letting the
//! test abort cryptically deep inside `register_new`.
//!
//! # The compile-time type pool
//!
//! Component ids are *per type*, so `N` distinct ids require `N` distinct Rust
//! types. `N` is only known at runtime (it depends on the live budget), but the
//! types must exist at compile time, so a fixed pool of `POOL = 450` distinct
//! `#[derive(Component)]` structs is generated up front (via a `macro_rules!`
//! token-list expansion — no `paste`, no proc-macro helper crate), and the test
//! registers / uses only the first `N` of them. Each pool type is
//! `#[repr(C)] struct CompK { v: u32 }`, so its on-disk column image is just the
//! 4 native-endian bytes of `v`, and it classifies `PlainOldBytes` (a blit column,
//! never a decode).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{MAX_COMPONENTS, get_layout};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Component;

use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

// ── Compile-time pool of `POOL` distinct component types ─────────────────────────
//
// `define_pool!` takes the explicit identifier list and, via a single
// `$( ... )+` repetition (NOT recursion — no token-muncher, so no recursion-limit
// concern), defines one `#[derive(Component)] #[repr(C)] struct Ident { v: u32 }`
// per name AND pushes a matching per-type accessor entry (id getter + `v`-reader)
// into the `POOL_ENTRIES` table. Emitting the structs and the accessor arms from the
// same repetition keeps the type list and the id/read tables impossible to
// desynchronize.

/// One pool type's runtime hooks: its (process-stable) `ComponentId` and a reader
/// that decodes the `v` field from a raw component pointer through the concrete
/// type (so the read-back is correct independent of any layout assumption).
struct PoolEntry {
    /// Mints (first call) / returns the id for this concrete type.
    id: fn() -> ComponentId,
    /// Reads the `v` field from a `*const u8` pointing at a live row of this type.
    ///
    /// # Safety
    /// The pointer must address an initialized, aligned instance of the matching
    /// pool struct (as returned by `get_component_raw` for this component's id).
    read_v: unsafe fn(*const u8) -> u32,
}

macro_rules! define_pool {
    // Entry: collect the accessor entries into a const array literal as we go.
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Component, Clone, Copy, PartialEq, Debug)]
            #[repr(C)]
            struct $name {
                v: u32,
            }
        )+

        /// The fixed compile-time pool of distinct component types, exposed as a flat
        /// table of per-type accessors. `POOL == POOL_ENTRIES.len()`.
        const POOL_ENTRIES: &[PoolEntry] = &[
            $(
                PoolEntry {
                    id: || <$name as Component>::component_id(),
                    // SAFETY: forwarded from `PoolEntry::read_v`'s contract — `ptr`
                    // addresses an initialized, aligned `$name` (a `#[repr(C)]`
                    // `{ v: u32 }` POD). `read_unaligned` would also be sound, but the
                    // ECS pool guarantees alignment, so a plain typed read is used.
                    read_v: |ptr: *const u8| unsafe { (*(ptr as *const $name)).v },
                },
            )+
        ];
    };
}

// 450 distinct pool types (`C0 ..= C449`). Written in dense rows of 25 so the list
// is auditable at a glance; the count is asserted against `POOL` below so a
// miscount fails to compile rather than silently shrinking the pool.
define_pool!(
    C0, C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14, C15, C16, C17, C18, C19, C20, C21, C22, C23, C24,
    C25, C26, C27, C28, C29, C30, C31, C32, C33, C34, C35, C36, C37, C38, C39, C40, C41, C42, C43, C44, C45, C46, C47, C48, C49,
    C50, C51, C52, C53, C54, C55, C56, C57, C58, C59, C60, C61, C62, C63, C64, C65, C66, C67, C68, C69, C70, C71, C72, C73, C74,
    C75, C76, C77, C78, C79, C80, C81, C82, C83, C84, C85, C86, C87, C88, C89, C90, C91, C92, C93, C94, C95, C96, C97, C98, C99,
    C100, C101, C102, C103, C104, C105, C106, C107, C108, C109, C110, C111, C112, C113, C114, C115, C116, C117, C118, C119, C120, C121, C122, C123, C124,
    C125, C126, C127, C128, C129, C130, C131, C132, C133, C134, C135, C136, C137, C138, C139, C140, C141, C142, C143, C144, C145, C146, C147, C148, C149,
    C150, C151, C152, C153, C154, C155, C156, C157, C158, C159, C160, C161, C162, C163, C164, C165, C166, C167, C168, C169, C170, C171, C172, C173, C174,
    C175, C176, C177, C178, C179, C180, C181, C182, C183, C184, C185, C186, C187, C188, C189, C190, C191, C192, C193, C194, C195, C196, C197, C198, C199,
    C200, C201, C202, C203, C204, C205, C206, C207, C208, C209, C210, C211, C212, C213, C214, C215, C216, C217, C218, C219, C220, C221, C222, C223, C224,
    C225, C226, C227, C228, C229, C230, C231, C232, C233, C234, C235, C236, C237, C238, C239, C240, C241, C242, C243, C244, C245, C246, C247, C248, C249,
    C250, C251, C252, C253, C254, C255, C256, C257, C258, C259, C260, C261, C262, C263, C264, C265, C266, C267, C268, C269, C270, C271, C272, C273, C274,
    C275, C276, C277, C278, C279, C280, C281, C282, C283, C284, C285, C286, C287, C288, C289, C290, C291, C292, C293, C294, C295, C296, C297, C298, C299,
    C300, C301, C302, C303, C304, C305, C306, C307, C308, C309, C310, C311, C312, C313, C314, C315, C316, C317, C318, C319, C320, C321, C322, C323, C324,
    C325, C326, C327, C328, C329, C330, C331, C332, C333, C334, C335, C336, C337, C338, C339, C340, C341, C342, C343, C344, C345, C346, C347, C348, C349,
    C350, C351, C352, C353, C354, C355, C356, C357, C358, C359, C360, C361, C362, C363, C364, C365, C366, C367, C368, C369, C370, C371, C372, C373, C374,
    C375, C376, C377, C378, C379, C380, C381, C382, C383, C384, C385, C386, C387, C388, C389, C390, C391, C392, C393, C394, C395, C396, C397, C398, C399,
    C400, C401, C402, C403, C404, C405, C406, C407, C408, C409, C410, C411, C412, C413, C414, C415, C416, C417, C418, C419, C420, C421, C422, C423, C424,
    C425, C426, C427, C428, C429, C430, C431, C432, C433, C434, C435, C436, C437, C438, C439, C440, C441, C442, C443, C444, C445, C446, C447, C448, C449,
);

/// Size of the compile-time pool (number of distinct generated component types).
const POOL: usize = 450;

const _: () = assert!(
    POOL_ENTRIES.len() == POOL,
    "the define_pool! identifier list must contain exactly POOL names"
);

// ── Budget derivation ────────────────────────────────────────────────────────────

/// A *conservative lower bound* on the id the next `register_new` would mint,
/// derived from the *public* registry surface: the lowest component id whose
/// `LAYOUTS` slot is still empty.
///
/// This is NOT guaranteed to equal the live `NEXT_ID`. The registry mints in two
/// phases — `register_new`/`try_register_dynamic` first bump the counter
/// (`fetch_add`/CAS), then populate `LAYOUTS[raw]` — so a slot reserved by an
/// in-flight registration can read as empty while `NEXT_ID` has already advanced
/// past it. The scan therefore stops at the first such gap and can only
/// *undercount* the true next id.
///
/// An undercount is sufficient here: it can only yield a *smaller* chosen `N`, and
/// (a) the `assert!(N >= 256)` floor plus (b) the 16-id margin applied at the call
/// site absorb any such slack. If the budget were ever genuinely too small the test
/// would fail loud at that assert rather than aborting inside `register_new`.
///
/// This avoids the `#[cfg(test)] pub(crate)` `next_id_for_test` accessor (invisible
/// to this integration-test binary) while reading a safe public quantity.
fn current_next_id() -> usize {
    (0..MAX_COMPONENTS)
        .find(|&id| get_layout(id).is_none())
        .unwrap_or(MAX_COMPONENTS)
}

/// The 4-byte native-endian column image of a `{ v: u32 }` pool component.
///
/// Every pool type is `#[repr(C)] { v: u32 }` (size 4, align 4, no padding), so the
/// whole-component byte image is just `v.to_ne_bytes()` — the same image
/// `save`/`load` blit verbatim.
#[inline]
fn comp_bytes(v: u32) -> [u8; 4] {
    v.to_ne_bytes()
}

/// The distinguishable `v` value carried by the `k`-th component.
#[inline]
fn value_for(k: usize) -> u32 {
    k as u32 * 7 + 1
}

// ════════════════════════════════════════════════════════════════════════════
// MAX-cardinality round-trip: one entity, N distinct POB columns, all survive.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn wide_archetype_max_cardinality_roundtrips() {
    // Budget: leave a 16-id margin under the live floor, cap at the compile-time
    // pool size, and require a substantial lower bound so a short budget fails loud
    // here instead of aborting inside `register_new`.
    let next = current_next_id();
    let headroom = MAX_COMPONENTS.saturating_sub(next).saturating_sub(16);
    let n = POOL.min(headroom);
    assert!(
        n >= 256,
        "test environment exhausted the component budget unexpectedly: \
         current_next_id={next}, MAX_COMPONENTS={MAX_COMPONENTS}, derived N={n} \
         (need >= 256)"
    );

    // The first `n` pool component ids, minted in pool order. Capturing them once
    // keeps the archetype signature, the spawn columns, and the read-back in sync.
    let ids: Vec<ComponentId> = POOL_ENTRIES[..n].iter().map(|e| (e.id)()).collect();
    assert_eq!(ids.len(), n);

    // ── Build the source world: ONE entity in the wide archetype ───────────────
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&ids);

    // Backing store for the per-column byte images; the `&[u8]` column slices below
    // borrow from here, so it must outlive the `create_entity` call.
    let images: Vec<[u8; 4]> = (0..n).map(|k| comp_bytes(value_for(k))).collect();
    let cols: Vec<(ComponentId, &[u8])> =
        ids.iter().zip(images.iter()).map(|(&id, img)| (id, img.as_slice())).collect();

    let src_entity = src.create_entity(arch, &cols).expect("create_entity (wide)");

    // Sanity: the source carries every value before we even save.
    for (k, &id) in ids.iter().enumerate() {
        let ptr = src.get_component_raw(src_entity, id).expect("src component present");
        // SAFETY: `ptr` is the live row of component `id` (the k-th pool type) for a
        // just-spawned entity; `read_v` decodes through that exact concrete type.
        let got = unsafe { (POOL_ENTRIES[k].read_v)(ptr) };
        assert_eq!(got, value_for(k), "source component {k} value mismatch");
    }

    // ── Save → fresh world → warm-up registration → load ───────────────────────
    let mut bytes = Vec::new();
    save_world(&src, &SaveOptions::default(), &mut bytes).expect("save wide world");

    let mut dst = EcsMaster::new();
    // W1 "register before load" contract: warm up the registry for the first `n`
    // pool types so the file ids resolve to live components (here the same process,
    // so the ids already exist, but the call is the documented contract and is
    // idempotent — it returns the cached id).
    for entry in &POOL_ENTRIES[..n] {
        let _ = (entry.id)();
    }

    let report =
        load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load wide world");

    // ── Report assertions: one archetype, N blits, one entity ──────────────────
    assert_eq!(report.archetypes_loaded, 1, "one wide archetype");
    assert_eq!(
        report.columns_blitted, n as u32,
        "every one of the {n} POB columns must blit"
    );
    assert_eq!(report.columns_decoded, 0, "all columns are POB blits, none decode");
    assert_eq!(report.entities_loaded, 1, "the single wide entity loads");
    assert_eq!(report.types_skipped, 0, "every file type resolves to a live component");
    assert_eq!(dst.entity_count(), 1, "exactly the one entity materialized");

    // ── Value survival: every component, read back by id ───────────────────────
    //
    // The loaded entity gets a fresh id, so it is located via the unique signature
    // (only one entity exists). Read every column back through its concrete type and
    // confirm `v` survived. The boundary columns (first, a middle one, last) are
    // additionally asserted by name so a regression there is pinpointed.
    let dst_entity = {
        // Find the lone live entity: query the first component, then read every id.
        // `get_component_raw` needs an `Entity`; obtain it from the only archetype
        // member via a single-component query over the boundary type.
        let mut found = None;
        for e in dst.iter_entities() {
            found = Some(e);
        }
        found.expect("exactly one loaded entity")
    };

    for (k, &id) in ids.iter().enumerate() {
        let ptr = dst
            .get_component_raw(dst_entity, id)
            .unwrap_or_else(|| panic!("loaded component {k} (id {id:?}) missing"));
        // SAFETY: `ptr` is the live row of component `id` for the loaded entity;
        // `read_v` decodes through the k-th pool type, the type that minted `id`.
        let got = unsafe { (POOL_ENTRIES[k].read_v)(ptr) };
        assert_eq!(got, value_for(k), "loaded component {k} value mismatch");
    }

    // Boundary spot-checks (first / middle / last), redundant with the full sweep
    // above but they localize a failure to a specific edge of the column loop.
    let mid = n / 2;
    for &k in &[0usize, mid, n - 1] {
        let ptr = dst.get_component_raw(dst_entity, ids[k]).expect("boundary component");
        // SAFETY: as above — `ptr` is the live row of the k-th pool type.
        let got = unsafe { (POOL_ENTRIES[k].read_v)(ptr) };
        assert_eq!(got, value_for(k), "boundary component {k} value mismatch");
    }
}
