//! The registry (CORE C2): `REFLECT`, [`install_type_info`], [`type_info_of`].
//!
//! One dense `ComponentId`-indexed table,
//! `[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]`, mirroring the kernel's
//! `LAYOUTS`/`HOOKS`/`SERIALIZE`/`BIND_ACCESSORS` quartet (CORE F2/F3) — and **never**
//! the `AtomicU8` shape (CORE D5): `STORAGE_KIND`'s `Relaxed` byte is right for a
//! classification datum with no payload behind it, and wrong here, because this table
//! **publishes a payload** (`&'static TypeInfo`) and therefore needs the
//! release/acquire edge `OnceLock` provides. A read is one acquire-load + branch — no
//! `Mutex`, no `static mut`, no data race.
//!
//! `MAX_COMPONENTS` is **IMPORTED, never redeclared** (CORE D5); the source census in
//! `tests/c2_registry_source_census.rs` reds on a local redeclaration, and CORE C2's
//! first RED mutation records what the redeclared form silently costs (a 512-slot
//! table over a smaller id space, with nothing left to red).

use std::sync::OnceLock;

use boyko_ecs::ecs::core::component::component_registry::MAX_COMPONENTS;

use crate::TypeInfo;

/// The dense reflection registry: `REFLECT[id]` holds `T`'s installed
/// `&'static TypeInfo`, keyed by the kernel's dense `ComponentId` index.
///
/// Write-once per slot (first writer wins, [`install_type_info`]); read via
/// [`type_info_of`]. *"Is `T` reflectable?"* has exactly one carrier:
/// `type_info_of(id).is_some()` (CORE D7 — no `IS_REFLECT` const exists).
static REFLECT: [OnceLock<&'static TypeInfo>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

// CORE C2 gate 4 / §3.3 last row — the static's own footprint, measured (printed by
// the `footprint_reported_and_pinned` unit test) and pinned at the measured per-slot
// size: one `OnceLock<&'static TypeInfo>` is 16 bytes on this target (a 4-byte `Once`
// state + padding + the 8-byte payload). A static in a dev-only crate, so the number
// is recorded, not budgeted; expressed via `MAX_COMPONENTS` so the pin moves with the
// kernel rather than carrying its own copy of the bound.
const _: () = assert!(
    size_of::<[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]>() == 16 * MAX_COMPONENTS,
    "the REFLECT table's footprint moved off its measured pin (16 bytes per slot) -- \
     re-measure and re-pin deliberately, in the same change that moved it"
);

/// Returns the installed [`TypeInfo`] for `component_id`, or `None` when no
/// `TypeInfo` was installed (a component without `#[component(reflect)]`) or the id
/// is out of bounds.
///
/// Cold: an editor/inspector read path, never the per-frame hot loop. One
/// acquire-load + branch, mirroring `get_bind_accessor` (CORE F6) — including the
/// bounds discipline: `debug_assert!` **plus** a release guard, so a stale or
/// corrupt id from a release editor build refuses instead of indexing out of
/// bounds.
#[inline]
pub fn type_info_of(component_id: usize) -> Option<&'static TypeInfo> {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} is out of bounds",
        component_id
    );
    if component_id >= MAX_COMPONENTS {
        return None;
    }
    REFLECT[component_id].get().copied()
}

/// Installs `info` into `REFLECT[component_id]` (CORE C2).
///
/// **PUBLIC** so the `#[component(reflect)]` expansion (which lives in downstream
/// crates where `pub(crate)` is unreachable) can call it — the same rationale as
/// `install_bind_accessor` (CORE F6), whose bounds discipline this fn copies
/// verbatim: `debug_assert!` plus a release `>= MAX_COMPONENTS` no-op guard.
/// Write-once via `OnceLock::set`; a same-id re-install is a **silent no-op (first
/// writer wins)**, so calling it ungated from the derive's registration closure is
/// safe — one cold `OnceLock::set` per type per process (§8: real cold cost, not a
/// hot-path regression).
///
/// `component_id: usize` per the in-tree installer convention (CORE D6/F5: the
/// derive calls installers as `…::component_id().0`). The name and signature are
/// G0's stub's, kept deliberately — the ship-absence census's needle B is this
/// name (GATES D5), and the calibration re-run recorded at this rung is what the
/// body replacement changes about that census.
#[inline(never)]
pub fn install_type_info(component_id: usize, info: &'static TypeInfo) {
    debug_assert!(
        component_id < MAX_COMPONENTS,
        "Component ID {} exceeds maximum allowed ({})",
        component_id,
        MAX_COMPONENTS
    );
    if component_id >= MAX_COMPONENTS {
        return;
    }
    let _ = REFLECT[component_id].set(info);
}

// Unit tests, not integration tests, ON PURPOSE — and the reason CHANGED at C3, so it
// is restated rather than inherited. Until C3 the placeholder `TypeInfo` was a
// `#[non_exhaustive]` ZST that nothing outside the crate could construct, and that
// unconstructibility was the argument. C3's real `TypeInfo` is constructible by any
// consumer (C7's derive bakes one from a downstream crate), so the argument is now the
// weaker but still sufficient one: these gates are about the TABLE, whose `REFLECT`
// static is private, and a unit test is where a private static's slots can be reasoned
// about without exporting a test-only door. The release-profile halves (gate 2) are
// `#[cfg(not(debug_assertions))]` and run under `cargo test -p boyko-reflect --release`
// — the gate greps for a non-vacuous `running [1-9]` there, because a filtered-out test
// is a vacuous pass.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_info::TypeKind;

    /// The descriptor's `type_id_fn` needs *a* function; the registry's gates never
    /// call it (`TypeId::of` is not `const`, which is why the slot is an fn pointer
    /// at all — C3).
    fn unit_type_id() -> std::any::TypeId {
        std::any::TypeId::of::<()>()
    }

    /// A minimal, field-less descriptor — the registry's gates care only that two
    /// `&'static TypeInfo` values are DISTINGUISHABLE BY ADDRESS, never about their
    /// contents (C3's model is validated at C3, not here).
    const fn empty_info(type_name: &'static str) -> TypeInfo {
        TypeInfo {
            type_name,
            type_id_fn: unit_type_id,
            size: 0,
            align: 1,
            fields: &[],
            kind: TypeKind::Struct,
            enum_info: None,
            default_in_place: None,
            drop_in_place: None,
        }
    }

    /// Two distinct `&'static TypeInfo` subjects for the write-once gate. Since C3
    /// `TypeInfo` is not a ZST, so two separate statics have distinct addresses by
    /// construction; the `#[repr(C)]` container is kept anyway because the gate
    /// asserts the distinctness precondition before using it, and a shared-address
    /// surprise must red as the instrument's own failure rather than as a false
    /// verdict about the registry.
    #[repr(C)]
    struct TwoInfos {
        a: TypeInfo,
        _pad: u64,
        b: TypeInfo,
    }

    static INFOS: TwoInfos =
        TwoInfos { a: empty_info("registry::tests::A"), _pad: 0, b: empty_info("registry::tests::B") };

    /// CORE C2 gate 1 — write-once idempotence: two installs with DIFFERENT info for
    /// one id; `type_info_of` returns the FIRST and the second is a silent no-op
    /// (F6's stated contract, the one the second RED breaks).
    #[test]
    fn write_once_first_writer_wins_second_install_is_a_silent_no_op() {
        let first: &'static TypeInfo = &INFOS.a;
        let second: &'static TypeInfo = &INFOS.b;
        assert!(
            !std::ptr::eq(first, second),
            "instrument precondition: the two subject infos must be distinguishable \
             by address, or this gate cannot see which writer won"
        );

        const ID: usize = 7;
        assert!(type_info_of(ID).is_none(), "slot {ID} must start empty");
        install_type_info(ID, first);
        install_type_info(ID, second);
        let got = type_info_of(ID).expect("an installed slot reads back Some");
        assert!(
            std::ptr::eq(got, first) && !std::ptr::eq(got, second),
            "FIRST WRITER MUST WIN: the second install of id {ID} overwrote the first \
             -- the table has last-writer-wins semantics, which is not OnceLock's \
             contract and not F6's"
        );
    }

    /// The read of an id nothing installed is `None` — the D7 carrier ("is `T`
    /// reflectable?" has exactly one carrier) answering "no".
    #[test]
    fn an_uninstalled_in_bounds_id_reads_none() {
        assert!(type_info_of(11).is_none());
    }

    /// CORE C2 gate 4 — the footprint, printed for the record (run with
    /// `--nocapture`) and pinned by the `const _` beside the static.
    #[test]
    fn footprint_reported_and_pinned() {
        let footprint = size_of::<[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]>();
        println!("size_of::<[OnceLock<&'static TypeInfo>; MAX_COMPONENTS]>() = {footprint}");
        println!("MAX_COMPONENTS = {MAX_COMPONENTS}");
        assert_eq!(footprint, 16 * MAX_COMPONENTS, "footprint moved off its measured pin");
    }

    /// The debug half of the bounds discipline: out of bounds is LOUD where
    /// `debug_assert!` exists (the release half is gate 2, below).
    #[test]
    #[should_panic(expected = "exceeds maximum allowed")]
    #[cfg(debug_assertions)]
    fn debug_install_out_of_bounds_panics() {
        install_type_info(MAX_COMPONENTS, &INFOS.a);
    }

    /// CORE C2 gate 2 — RELEASE profile only (`cargo test --release`): an
    /// out-of-bounds install is a NO-OP, not a panic and not an out-of-bounds index.
    /// This is the configuration where `debug_assert!` has vanished and the release
    /// guard is the only thing left (D11's reasoning applied to the registry).
    #[test]
    #[cfg(not(debug_assertions))]
    fn release_install_out_of_bounds_is_a_silent_no_op() {
        install_type_info(MAX_COMPONENTS, &INFOS.a);
        install_type_info(usize::MAX, &INFOS.b);
        // Nothing to read back -- the observable IS that neither call panicked nor
        // indexed out of bounds, and that the boundary read below still refuses.
        assert!(type_info_of(MAX_COMPONENTS).is_none());
    }

    /// CORE C2 gate 2, read half — RELEASE profile only: the out-of-bounds read is
    /// `None`, never an index panic.
    #[test]
    #[cfg(not(debug_assertions))]
    fn release_type_info_of_out_of_bounds_is_none() {
        assert!(type_info_of(MAX_COMPONENTS).is_none());
        assert!(type_info_of(usize::MAX).is_none());
    }
}
