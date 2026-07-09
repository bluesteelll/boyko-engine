//! A process-wide action-NAME → dense-index table for the type-erased `.ui`
//! parser (GUI #27).
//!
//! # Why this exists
//! The `.ui` runtime parser (`boyko_ui::text`) is type-erased TEXT — it cannot
//! be generic over the game's `Actionlike` enum `A`, because `A` is only known
//! at the `boyko_input` / game-wiring layer. So `OnClick(Jump)` (an action by
//! NAME) cannot resolve to a dense `u16` index inside the parser. This module is
//! the seam: it is FILLED once where `A` IS in scope ([`register_action_names`],
//! called from `InputPlugin::<A>::build`) and READ by name from the parser
//! ([`resolve_action_name`]).
//!
//! # Relation to `.keys`
//! The `.keys` loader resolves names with a *generic* linear scan
//! (`grammar::action_from_name<A>`), which is possible there because `A` is in
//! scope. This table is a DIFFERENT, type-erased reader; it derives the SAME
//! name→index mapping from [`build_action_name_table`] so the two paths share
//! one source of truth (a name's index is `A::index()` in both). It is a small,
//! cold registration surface — the same shape as the `BIND_ACCESSORS` /
//! stable-name registries already in the engine — NOT a heavy per-frame registry.
//!
//! # Single-`A` contract (v1)
//! The table is write-once ([`OnceLock`]): the FIRST `register_action_names`
//! wins; a second call (a different `A`, or a re-registration) is a no-op. So the
//! `.ui` action-name space binds to the first-registered action enum. A game with
//! two `Actionlike` enums that authors `.ui` against the second will not resolve
//! its names (every `OnClick(Name)` becomes a recoverable per-line error, never a
//! panic). This mirrors the single-context pragmatism `.keys` v1 takes and is a
//! documented v1 limitation.

use std::sync::OnceLock;

use crate::action::actionlike::Actionlike;

/// The process-wide action-name table, sorted by name for binary search. Holds
/// borrowed `&'static str` names (from [`Actionlike::name`]) + the dense `u16`
/// index, so there is zero per-entry heap (one boxed slice, filled once).
///
/// Write-once: a generic-fn-body `static` would collapse across monomorphizations
/// (the per-`A` static trap), so the table is ONE concrete global filled from
/// `A`'s variants at registration.
static ACTION_NAMES: OnceLock<Box<[(&'static str, u16)]>> = OnceLock::new();

/// Builds the sorted name→index table for action enum `A`. Cold; allocates once.
///
/// The table is `(A::from_index(i).name(), i as u16)` for `i in 0..A::COUNT`,
/// sorted by name. A duplicate `name()` across variants is an upstream
/// `#[derive(Actionlike)]` bug; the resolver's binary search then resolves to
/// one of them — to MATCH the `.keys` first-match tie-break, the table is sorted
/// by name only, and the resolver returns the lowest index among equal names via
/// the stable-by-index pre-sort below.
///
/// Shared with the `.ui` registration so the name→index mapping has one source.
pub(crate) fn build_action_name_table<A: Actionlike>() -> Box<[(&'static str, u16)]> {
    debug_assert!(A::COUNT <= u16::MAX as usize, "invariant: action count exceeds u16");
    let mut table: Vec<(&'static str, u16)> = (0..A::COUNT)
        .filter_map(|i| A::from_index(i).map(|a| (a.name(), i as u16)))
        .collect();
    // Sort by (name, index): a stable name order for binary search, and for a
    // duplicated name the lowest index sorts first so a `(name, _)`-keyed search
    // landing on the run can be walked back to the lowest (first-match parity
    // with `.keys` `action_from_name`'s `.find`).
    table.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
    debug_assert!(
        table.windows(2).all(|w| w[0].0 != w[1].0),
        "warning: duplicate Actionlike::name() — the .ui resolver picks the lowest index"
    );
    table.into_boxed_slice()
}

/// Registers `A`'s action names for the type-erased `.ui` parser. Idempotent and
/// write-once: a second call (the same or a different `A`) is a no-op.
///
/// Call this once where `A` is in scope — `InputPlugin::<A>::build` does. After
/// registration, `.ui` `OnClick(Name)` / `OnHover(Name)` / `OnSubmit(Name)`
/// resolve `Name` to `A::index()` at load time.
///
/// See the [module docs](self) for the single-`A` v1 contract.
pub fn register_action_names<A: Actionlike>() {
    // `get_or_init` builds the table only on the FIRST call (write-once); a later
    // call observes the already-set value and discards nothing (the closure is not
    // run), so a second `A` is a cheap no-op.
    ACTION_NAMES.get_or_init(build_action_name_table::<A>);
}

/// Resolves an action name to its dense `u16` index, or `None` if the name is
/// unknown (or no enum was registered). Cold; binary search over the sorted
/// table. The resolver returns the LOWEST index for a duplicated name (first-match
/// parity with `.keys`).
///
/// Used by the `.ui` parser to lower `OnClick(Name)` to `OnClick(index)`,
/// byte-identical to the numeric `OnClick(index)` and the `ui!` form.
pub fn resolve_action_name(name: &str) -> Option<u16> {
    let table = ACTION_NAMES.get()?;
    let mut pos = table.binary_search_by(|(n, _)| (*n).cmp(name)).ok()?;
    // Walk back to the lowest index among an equal-name run (first-match parity).
    while pos > 0 && table[pos - 1].0 == name {
        pos -= 1;
    }
    Some(table[pos].1)
}
