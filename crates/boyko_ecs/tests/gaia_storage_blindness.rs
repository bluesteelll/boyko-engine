//! **A FOURTH poolless-storage site, found by the Gaia census: a `bitset` component
//! as a field of a user `#[derive(Bundle)]` struct panics at spawn.** RED BY DESIGN;
//! not fixed here.
//!
//! # The class
//!
//! The Aether v2 + Gaia corpus is organised around one recurring kernel mechanism:
//! *code that resolves a per-archetype `ComponentPool` by `ComponentId` without first
//! screening the storage kind*. `StorageKind::Dense` and `StorageKind::Bitset` own no
//! such pool — both are excluded from every archetype signature — so the resolve
//! misses, and the site consumes the miss as if it were an answer. Known instances:
//!
//! | site | how it fails | owner |
//! |---|---|---|
//! | `Or<(…)>` with a dense arm | the arm is never true / never excludes | Aether **KE1**, ✅ landed R0 |
//! | `any_changed_since` / `get_component_changed_tick` | the change gate is never true; a `boyko_ui` bind never updates | Gaia **GK-2**, deferred |
//! | `resolve_required_missing` + the `has_requires` ctor pass | `#[require]` of a poolless id is a boot-time panic | Aether **KE11**, disposition on ballot **AB-6** |
//! | `QueryView::get` / `get_mut` never call `F::filter_fetch` | a dense `With`/`Without` applies no gate at all | Aether **KE13**, UNOWNED |
//! | **this file** | a `bitset` field in a user `Bundle` is a spawn-time panic | **UNOWNED** |
//!
//! # Why this one is not already covered
//!
//! KE11's row cites `BundleColumnCache::resolve_and_cache` as the model of
//! CORRECTNESS — *"unlike the sibling loop in `BundleColumnCache::resolve_and_cache`,
//! which diverts a `StorageKind::Dense` id to `DENSE_POOL_SENTINEL` before
//! resolving"*. **That sibling is itself only half-screened.** Its loop
//! (`bundle_column_cache.rs`, the `for (i, &cid) in component_ids.iter().enumerate()`
//! pass that builds `pool_ids_owned` / `dense_mask`) matches on
//! `StorageKind::Dense` alone and falls through to
//! `pool_id_for(cid).expect("invariant: … hosts every TABLE component …")` for a
//! `Bitset` id. KE11 widened its own two sites from "a bitset tag" to *both* poolless
//! kinds after measuring that the row was "half the class" — and then cited, as its
//! correctness model, a function with exactly the other half of that same gap.
//!
//! This site is also reached WITHOUT any `#[require]`, so KE11's fixtures cannot
//! reach it: they spawn a bundle whose component `#[require]`s a poolless id and die
//! at `resolve_required_missing`. Here the poolless id is a *direct member* of
//! `B::component_ids()`, and the panic is a different `.expect` in a different
//! function.
//!
//! # The refusal exists on ONE Bundle entrance and not the other
//!
//! This is the sharpest way to state it. `#[derive(Component)]` deliberately
//! SUPPRESSES the automatic single-component `Bundle` impl for a poolless component —
//! `boyko_macros` `component.rs`, the `hooks.no_bundle || hooks.storage_bitset ||
//! hooks.storage_dense` gate, whose comment says the tag "has no pool" — and
//! `boyko_ecs/tests/d4_typed_write.rs` states the intent in writing: *"It can NOT be a
//! `Bundle` field (the derive suppresses the single-component Bundle emission for
//! `storage = "bitset"`)"*.
//!
//! But `#[derive(Bundle)]` over a USER struct carries no such screen — `bundle.rs`
//! contains no mention of `bitset` at all. So the intended refusal closes the door an
//! author is least likely to try and leaves open the one they will:
//!
//! ```ignore
//! cmds.spawn(MyFlag);                                   // does not compile — refused
//! cmds.spawn(MyBundle { flag: MyFlag, tag: MyTag(1) }); // compiles — PANICS at spawn
//! ```
//!
//! and the panic blames an internal invariant rather than the author's line.
//!
//! # Why this matters to BOTH campaigns, not only the kernel
//!
//! * **Aether.** `aether_lang`'s `flag` construct lowers to exactly this attribute —
//!   `expand.rs` emits `#[component(storage = "bitset")]` for a `flag` declaration.
//!   Aether v2 ratifies a `bundle` construct; an author who writes a `flag` into a
//!   `bundle` reaches this panic, and the diagnostic they get names none of their
//!   own tokens. That is the `unwritable-over-diagnosable` ruling failing on a form
//!   the language itself makes writable.
//! * **Gaia.** Ballot **F9** asks how flags are carried in the baked format. Whatever
//!   it answers, a baked flag has to reach a live entity through some insert path,
//!   and the bundle path is the one the format's rows most resemble.
//!
//! # Why this is `#[ignore]`d rather than fixed
//!
//! Three dispositions are available and they are not equivalent — divert to a
//! sentinel and raise the bit on the attach path (the `dense_mask` treatment, which
//! would make the spelling above WORK); refuse it in `#[derive(Bundle)]` at compile
//! time (which makes it unwritable, the ruling this repo prefers); or refuse it at
//! runtime with an author-blaming message. That is the same question KE11's
//! disposition ballot **AB-6** asks about the same mechanism, and it is the owner's.
//! Choosing here would pre-empt it.
//!
//! Run with:
//! `cargo test -p boyko-ecs --test gaia_storage_blindness -- --ignored`

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` out of the `Send + Sync` one-shot system closure.
// Integration test — compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

/// A `flag`: `StorageKind::Bitset`, owning no per-archetype pool. This is what
/// `aether_lang`'s `flag Stunned;` lowers to.
#[derive(Component, Default, Clone, Copy)]
#[component(storage = "bitset")]
#[repr(C)]
struct GFlag780;

/// A `StorageKind::Dense` component — the differential control. Dense is the poolless
/// kind `resolve_and_cache` DOES screen, so the same bundle shape over a dense field
/// works. That contrast is what shows this is a missing screen rather than a
/// deliberate refusal of poolless kinds in bundles.
#[derive(Component, Default, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct GDense781 {
    v: u32,
}

/// An ordinary table payload, so each bundle below is multi-component and the
/// archetype is real.
#[derive(Component, Default, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct GTable782 {
    v: u32,
}

/// The control bundle: table + DENSE. Compiles and spawns.
#[derive(Bundle)]
struct GDenseBundle {
    t: GTable782,
    d: GDense781,
}

/// The subject: table + BITSET. Compiles — and that is the defect.
#[derive(Bundle)]
struct GFlagBundle {
    t: GTable782,
    f: GFlag780,
}

/// Spawns `bundle` through `Commands`, returning the handle.
fn spawn<B: boyko_ecs::ecs::core::bundle::bundle::Bundle + Send + Sync + 'static>(
    world: &mut EcsMaster,
    bundle: B,
) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let b = Mutex::new(Some(bundle));
    world.run_system(move |mut cmds: Commands| {
        let b = b.lock().unwrap().take().expect("spawn closure runs once");
        *probe.lock().unwrap() = Some(cmds.spawn(b).id());
    });
    sink.lock().unwrap().expect("spawned handle")
}

/// FLOOR. The identical bundle shape over the OTHER poolless kind spawns cleanly,
/// because `resolve_and_cache` diverts a dense id to `DENSE_POOL_SENTINEL` before
/// resolving. Not ignored: it is what makes the ignored test below a MISSING SCREEN
/// finding rather than "poolless kinds are simply not allowed in bundles".
#[test]
fn control_a_dense_field_in_a_user_bundle_spawns() {
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, GDenseBundle { t: GTable782 { v: 1 }, d: GDense781 { v: 2 } });

    assert_eq!(
        world.get_component::<GTable782>(e).copied(),
        Some(GTable782 { v: 1 }),
        "control: the table half landed"
    );
    assert_eq!(
        world.get_component::<GDense781>(e).copied(),
        Some(GDense781 { v: 2 }),
        "control: the DENSE half landed — the poolless kind that IS screened in \
         resolve_and_cache"
    );
}

/// FLOOR. The bitset component is registered and usable through its own sanctioned
/// out-of-band path, so the ignored test below cannot be dismissed as "the flag type
/// is broken".
#[test]
fn control_a_flag_works_through_the_out_of_band_enable_path() {
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, GDenseBundle { t: GTable782 { v: 3 }, d: GDense781 { v: 4 } });

    assert!(!world.is_enabled::<GFlag780>(e), "the bit starts low");
    world.enable::<GFlag780>(e);
    assert!(
        world.is_enabled::<GFlag780>(e),
        "control: `EcsMaster::enable` raises the flag — the type is fine; the BUNDLE \
         ENTRANCE is what is missing a screen"
    );
}

/// **The finding.** A user `#[derive(Bundle)]` carrying a `bitset` field must either
/// spawn, or be refused at compile time. Today it does neither: it compiles, and
/// spawning it panics inside the kernel.
///
/// Hand oracle: the spawn succeeds and the entity carries both members (the dense
/// control proves that is the behaviour for the sibling poolless kind). Observed
/// today: a panic at `bundle_column_cache.rs`'s
/// `pool_id_for(cid).expect("invariant: B::cached_archetype_id returned an archetype
/// that hosts every TABLE component in B::component_ids() … dense ids are skipped
/// above")` — a message that blames a registration-contract invariant, names no
/// author token, and whose own parenthetical ("dense ids are skipped above") is the
/// evidence that the bitset case was simply not considered at this site.
///
/// A test asserting "this panics" would be the wrong shape: it would go green on any
/// panic anywhere, and it would go RED on the fix, so it would have to be rewritten
/// by whoever repairs this. Asserting the correct behaviour and marking it deferred
/// keeps the oracle pointed at the outcome.
#[test]
#[ignore = "deferred: an UNOWNED fourth site of the poolless-storage class — \
            BundleColumnCache::resolve_and_cache screens StorageKind::Dense but not \
            StorageKind::Bitset, so a bitset field in a user #[derive(Bundle)] \
            compiles and panics at spawn. Disposition (sentinel-divert / compile-time \
            refusal in the Bundle derive / author-blaming runtime refusal) is the same \
            question KE11's ballot AB-6 asks about the same mechanism. RED BY DESIGN; \
            see this file's header"]
fn a_flag_field_in_a_user_bundle_spawns() {
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, GFlagBundle { t: GTable782 { v: 5 }, f: GFlag780 });

    assert_eq!(
        world.get_component::<GTable782>(e).copied(),
        Some(GTable782 { v: 5 }),
        "the table half must land"
    );
    assert!(
        world.is_enabled::<GFlag780>(e),
        "the flag bit must be raised by the bundle that declared it, exactly as the \
         dense control's dense member lands"
    );
}
