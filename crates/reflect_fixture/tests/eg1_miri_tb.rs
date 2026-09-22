//! **ECS EG1 gate 6** — the enumeration under Miri with **Tree Borrows**, with a sibling
//! structural migration interleaved between two enumerations.
//!
//! ⚠️ **This gate is the EXPERIMENT, not a formality.**
//! `docs/REFLECTION-PLAN-ECS.md`'s **D2** routes source 1 through the safe `&self`
//! accessor, which forms a **whole-struct `&Archetype`**. D2's original second reason —
//! *"by never touching the cached slab pointer the glue is outside BUG-MIGRATE-TB-1's
//! hazard"* — is **struck**: `ArchetypeMaster::get_archetype` delegates to a body whose
//! last line is `Some(unsafe { &*ptr })` over a pointer minted through the same
//! `UnsafeCell::raw_get` root the cached `EntityInland` pointer carries, so the two forms
//! are not distinguished by their root at all. What sanctions the route is the kernel's own
//! precedent for a permitted `&Archetype` read (`ecs_master/component_api.rs:392~`): the
//! hazard *"DOES NOT APPLY … no sibling structural migration and no slab dealloc can
//! interleave with the read"*. This file is what decides whether Tree Borrows agrees.
//!
//! Both outcomes have a disposition, stated by the plan (D23) so this rung cannot invent
//! one at build time: **green** — D2 stands on the corrected reason; **red** — source 1 has
//! no legal form (F8 makes the raw projection inexpressible and `component_ids()` requires
//! an `&Archetype`), and the only remedy is a fifth shipping-API item, which is an OWNER
//! call of B.13 #2's class. EG1 escalates; it does not widen the seam on its own.
//!
//! # The RED MUTATION this file carries — **R3″**, because R3′ does not fire
//!
//! `R3` as the plan originally wrote it (swap the safe accessor for `&*inland.archetype_ptr()`)
//! is dead twice over: no `pub` item in `boyko_ecs` hands out an `EntityInland`
//! (`EntityMaster::entities_inland` is `pub(crate)` and its only producer is a private fn),
//! so the mutation is a compile error rather than a Miri red; and the "safe accessor" arm
//! is *itself* `&*ptr` on a pointer with the same root, so both arms of the control were
//! the same construct.
//!
//! **R3′ — the plan's replacement — was MEASURED at this rung and does NOT red.** Moving
//! the `deref_before` block below the `add_tag` call hoists the `*const Archetype` across
//! the migration and dereferences it afterwards; under `-Zmiri-tree-borrows` that is
//! **accepted**, and the reason is the one D23 itself found: the pointer is minted through
//! `UnsafeCell::raw_get`, so it carries `SharedReadWrite` provenance, and a foreign write
//! does not disable an interior-mutable tag. R3′ therefore inherits the defect it was
//! written to repair — the hoist was applied to the wrong object.
//!
//! **R3″ is the mutation that discriminates, and it is the one to run.** Form the
//! **reference** before the migration and USE it after:
//!
//! ```text
//! let frozen = unsafe { &*archetype_ptr };          // BEFORE the migration
//! let before = frozen.component_ids().len();
//! ecs.add_tag(sibling, tag);                        // the foreign write
//! let after = frozen.component_ids().len();         // <-- must be UB
//! ```
//!
//! MEASURED under `-Zmiri-tree-borrows`:
//! *"Undefined Behavior: reborrow through `<tag>` … is forbidden … the accessed tag was
//! created here, in the initial state **Frozen** … later transitioned to **Disabled** due
//! to a foreign write access at offsets `[0x2118..0x2120]`"*, pointing at
//! `archetype.rs:1134` — `self.current_index -= 1`, the exact field and the exact write
//! BUG-MIGRATE-TB-1 is named for.
//!
//! That is the whole difference, and it is what makes this gate's green mean something:
//! **the instrument CAN see the hazard class.** It also says precisely why D2's route is
//! safe — the glue derives its `&Archetype` from `&EcsMaster`, so no `&mut EcsMaster` can
//! exist while the reference lives. Borrowck forbids the only shape Tree Borrows rejects.
//!
//! # ⚠️ `MIRIFLAGS` is not `force = true`, and it is NOT visible from inside the test
//!
//! `.cargo/config.toml`'s `[env]` sets `MIRIFLAGS = "-Zmiri-tree-borrows"` **without**
//! `force = true`, so a `MIRIFLAGS` already exported in the shell silently WINS and this
//! gate then runs under Stacked Borrows — green, and measuring the wrong model.
//!
//! ⚠️ **An in-test `std::env::var("MIRIFLAGS")` cannot check that, and MEASURED it lies:**
//! cargo-miri consumes the variable when it launches the interpreter and does not forward
//! it into the interpreted program's environment, so the test reads `None` on a run that is
//! demonstrably under Tree Borrows. The model is pinned at the INVOCATION and echoed by the
//! rung report. MEASURED at this rung, in both directions: this program is green under
//! Stacked Borrows too (so its own green does not identify the model), and the MIRIFLAGS
//! channel is live (a deliberately bogus flag appended to it makes miri fail with *"unknown
//! unstable option"*).
//!
//! # This binary is deliberately ALLOCATOR-FREE
//!
//! Gate 3's counting `#[global_allocator]` lives in `ecs_alloc.rs` and carries
//! `#![cfg(not(miri))]`: a `System`-forwarding global allocator is not transparent under
//! Miri + Tree Borrows on `x86_64-pc-windows-gnu` and aborts in libtest's shutdown with
//! `running 0 tests`. Folding the two gates into one binary makes exactly one of them
//! vacuous.
//!
//! # The invocation is part of the gate
//!
//! ```text
//! RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu cargo miri test \
//!     -p reflect-fixture --features reflect-fixture/reflect --test eg1_miri_tb
//! ```
//!
//! ⚠️ **The toolchain is named IN FULL, and that is load-bearing on this machine.**
//! `cargo +nightly miri test` — the form this header shipped with — resolves `+nightly` to
//! `nightly-x86_64-pc-windows-msvc`, because rustup's default host here is msvc, and
//! cargo-miri then dies building its own sysroot inside `link.exe`
//! (*"error: linking with `link.exe` failed"*, *"could not compile `std` (build script)"*,
//! exit 1). MEASURED at the EG1 verification. A gate whose printed invocation does not run
//! is a gate nobody re-runs, so the working form is the one written here.
//!
//! CI reaches it as `--all-targets` over the same package/feature pair.
#![cfg(feature = "reflect")]

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_ecs::prelude::{EcsMaster, Entity};
use boyko_macros::Component;
use boyko_reflect::ecs::{IdEntry, IdKind, components_of_into};

/// The signature-storage citizen.
#[derive(Component, Default)]
#[repr(C)]
struct TbTable {
    x: f32,
}

/// The dense citizen, so the enumeration walks source 2 as well as source 1.
#[derive(Component, Default)]
#[component(storage = "dense")]
#[repr(C)]
struct TbDense {
    lane: [f32; 4],
}

/// The sentinel a buffer is pre-filled with. `Bitset` is the one kind EG1 cannot produce.
const SENTINEL: IdEntry = IdEntry { id: ComponentId(usize::MAX), kind: IdKind::Bitset };

/// Enumerates into a stack buffer and returns the filled prefix, copied into a fixed array
/// so the comparison holds no borrow of the world.
fn enumerate(ecs: &EcsMaster, entity: Entity) -> ([IdEntry; 8], usize) {
    let mut buf = [SENTINEL; 8];
    let n = components_of_into(ecs, entity, &mut buf).expect("invariant: a live entity");
    (buf, n)
}

/// **EG1 gate 6.** Two enumerations of one entity with a **sibling structural migration**
/// between them, under whatever borrow model `MIRIFLAGS` selects.
///
/// The sibling shares the subject's archetype, so `add_tag` migrates it OUT — a
/// `swap_remove` on the very archetype the enumeration reads, plus the mint of a new slab
/// slot for the sibling's destination. That is the interleave the gate is about; a sibling
/// in an unrelated archetype would make it vacuous, so the shared archetype is asserted.
///
/// ⚠️ **The SPAWN ORDER below is forced, and the reason is a MEASURED kernel defect.**
/// `EcsMaster::add_tag` walks the *source archetype's* RETAINED `component_ids()` and asks
/// for a per-archetype pool for every id in it — but a **dense** id is retained in that list
/// and structurally has no pool, so the walk hits
/// `.expect("invariant: source hosts its own component id")` and panics in RELEASE
/// (`migrate_entity_attach_ids`; `remove_tag` is broken identically in
/// `migrate_entity_detach_ids`). The victim is not "an entity carrying a dense component" —
/// it is **every entity in an archetype that has ever hosted one**, because dedup keys on
/// the filtered mask and the retained list is the minting caller's. Measured in this
/// worktree with no reflection code on the stack.
///
/// Spawning the table-only sibling **first** mints a clean `[table]` archetype that the
/// table+dense subject then dedups into, so the shared archetype retains no dense id and the
/// migration is expressible. It is `boyko_ecs`'s defect to fix, not this campaign's — and
/// EG2/EG6 build `add_component_by_id` / `remove_component_by_id` on exactly those two
/// helpers, so it is theirs to carry.
#[test]
fn the_enumeration_survives_a_sibling_structural_migration() {
    // ⚠️ MEASURED, and it is why this is not a bare `MIRIFLAGS` echo. Under
    // `cargo miri test`, `MIRIFLAGS` is consumed by cargo-miri when it launches the
    // interpreter and is NOT forwarded into the interpreted program's environment: a
    // `std::env::var("MIRIFLAGS")` here prints `None` on a run that is demonstrably using
    // Tree Borrows. Printing that would tell a reader the opposite of the truth, so the
    // program reports every MIRI-shaped variable it can actually see and says where the
    // model is really pinned.
    let visible: Vec<(String, String)> =
        std::env::vars().filter(|(k, _)| k.starts_with("MIRI")).collect();
    println!(
        "EG1 gate 6: cfg!(miri) = {}, MIRI* env visible to the interpreted program = {visible:?} \
         (empty under Miri BY MEASUREMENT -- the borrow model is pinned at the INVOCATION, \
          by `.cargo/config.toml`'s non-forced `[env] MIRIFLAGS = \"-Zmiri-tree-borrows\"`, \
          which a shell-exported MIRIFLAGS silently overrides; the rung report echoes the \
          value the invocation actually ran with)",
        cfg!(miri)
    );

    let mut ecs = EcsMaster::new();
    let table = <TbTable as ComponentTrait>::component_id();
    let dense = <TbDense as ComponentTrait>::component_id();

    // ORDER IS FORCED — see the doc comment. The table-only sibling mints the archetype;
    // the table+dense subject dedups into it, because dedup keys on the filtered mask.
    let table_only = ecs.get_or_create_archetype(&[table]);
    let sibling = ecs
        .spawn_one(table_only, TbTable::default())
        .expect("invariant: the table-only push is accepted");
    let both = ecs.get_or_create_archetype(&[table, dense]);
    let subject = ecs
        .spawn_two(both, TbTable::default(), TbDense::default())
        .expect("invariant: the archetype accepts a push supplying its one signature id");

    let subject_archetype = ecs.entity_archetype_id(subject).expect("live");
    assert_eq!(
        ecs.entity_archetype_id(sibling),
        Some(subject_archetype),
        "the sibling must share the subject's archetype, or the migration below touches a \
         slab slot the enumeration never reads and this gate measures nothing"
    );
    assert!(
        ecs.dense_contains(subject, dense),
        "the subject must really be a member of the dense store, or source 2 is never \
         walked under the borrow model and half the gate is vacuous"
    );

    // ── enumeration 1 ────────────────────────────────────────────────────────────────
    let (before, n_before) = enumerate(&ecs, subject);
    assert_eq!(n_before, 2, "table + dense");

    // ── the R3″ ANCHOR ───────────────────────────────────────────────────────────────
    //
    // The `&Archetype` is formed here and DIES here, before the migration. R3″ hoists the
    // REFERENCE — `let frozen = unsafe { &*archetype_ptr };` above the block, used again
    // after `add_tag` — and changes nothing else. R3′ (moving this whole block below
    // `add_tag`, i.e. hoisting the POINTER) was measured at this rung and does NOT red:
    // the pointer is `UnsafeCell::raw_get`-rooted and a foreign write leaves an
    // interior-mutable tag alone. The reference is what freezes.
    let archetype_ptr = ecs
        .archetype_master()
        .get_archetype_ptr(subject_archetype)
        .expect("invariant: a live entity's archetype id resolves to a slab slot");
    let deref_before = {
        // SAFETY: `archetype_ptr` was minted from `&self` on the live `ArchetypeMaster`
        //   through `UnsafeCell::raw_get`; the slab base is heap-stable for the bundle's
        //   lifetime and the occupancy bit was checked inside the accessor, so it addresses
        //   an initialised `Archetype`. The `&Archetype` formed here covers the WHOLE struct
        //   and therefore FREEZES `current_index` — which is why it must die before any
        //   sibling structural write, and it does: the block ends before the migration, and
        //   in the glue itself the same reference is derived from `&EcsMaster`, so borrowck
        //   forbids a `&mut EcsMaster` existing while it lives.
        let archetype = unsafe { &*archetype_ptr };
        archetype.component_ids().len()
    };
    println!("EG1 gate 6: retained component_ids().len() before the migration = {deref_before}");

    // ── the sibling structural migration ─────────────────────────────────────────────
    let tag = ecs.register_tag("eg1_tb_sibling_marker");
    ecs.add_tag(sibling, tag);
    assert_ne!(
        ecs.entity_archetype_id(sibling),
        Some(subject_archetype),
        "the sibling must actually have LEFT the shared archetype, or `add_tag` was a \
         retag-in-place and no structural write interleaved at all"
    );

    // ── enumeration 2 ────────────────────────────────────────────────────────────────
    let (after, n_after) = enumerate(&ecs, subject);
    assert_eq!(
        (n_before, before),
        (n_after, after),
        "the subject's component set changed across a migration that touched only a \
         sibling row"
    );

    println!("EG1 gate 6: OUTCOME = the enumeration completed with no borrow-model error");
}
