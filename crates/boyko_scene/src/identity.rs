//! Entity identity — the [`Name`] component and its setup-only string interner
//! (standard-library Phase S6).
//!
//! A [`Name`] is a stable, deduplicated id ([`NameId`]) into a process-global
//! string interner. Two interns of the same string yield the SAME [`NameId`], so
//! `Name` equality is a single `u32` compare — never a string compare — and a
//! `Query<&Name>` reads the id by value without ever touching the interner.
//!
//! # Principle 0 (the interner is cold setup-only METADATA, load-bearing)
//!
//! The interner is **not** a parallel data system. It is cold setup/spawn-time
//! metadata, exactly the justified exception the engine already grants its
//! registries (`ComponentRegistry`, `EventRegistry`): a process-global table
//! consulted ONLY when minting names (at spawn / setup), NEVER on a per-frame
//! path. Because it is cold it may use a `HashMap` behind a `Mutex` — but that is
//! a deliberate, documented boundary:
//!
//! * `intern` / `resolve` are cold; they take the `Mutex` briefly at setup.
//! * No hot-path system signature names the interner. A `Name` component carries
//!   its [`NameId`] inline; iterating `Query<&Name>` reads the `u32` directly and
//!   does NOT call `resolve`.
//! * Interned strings live for the process lifetime (no eviction) — correct for
//!   names, which are few and permanent.
//!
//! If this ever needs to be consulted per frame, that is a design bug — resolve
//! the [`NameId`] once at setup and store the result, do not call back here.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use boyko_macros::Component;

/// A stable, deduplicated id into the setup-only string interner.
///
/// `#[repr(transparent)]` over `u32`, so a [`Name`] column is byte-identical to a
/// `u32` array. Equality / hashing are plain `u32` operations — the dedup already
/// happened at [`intern`] time, so two `Name`s are equal iff their strings were.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameId(pub u32);

/// A human-readable entity name — a [`NameId`] into the setup-only interner.
///
/// `#[repr(transparent)]` over [`NameId`] (hence over `u32`): the component column
/// is a dense `u32` array. Mint one with [`intern`]; recover the string (cold,
/// setup/debug only) with [`resolve`].
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Name(pub NameId);

// Layout pin (house style): a `Name` column must stay a dense 4-byte `u32` lane.
const _: () = assert!(size_of::<Name>() == 4 && align_of::<Name>() == 4);
const _: () = assert!(size_of::<NameId>() == 4 && align_of::<NameId>() == 4);

/// The cold interner table (Principle-0 metadata — see the module docs).
///
/// `strings` stores the LEAKED `&'static str` (not `Box<str>`): the allocation is
/// immortal, so a *copy of the reference* handed out by [`resolve`] is soundly
/// `'static` and outlives the `Mutex` guard. `map` keys on the same `&'static str`
/// for O(1) dedup. The index into `strings` IS the `NameId`.
struct InternerState {
    /// Dedup table: interned string → its id.
    map: HashMap<&'static str, u32>,
    /// Reverse table: `strings[id]` is the (leaked, immortal) interned string.
    strings: Vec<&'static str>,
}

/// The process-global interner. Cold setup-only metadata, guarded by a `Mutex`
/// because it is never on a per-frame path (see the module docs).
static INTERNER: OnceLock<Mutex<InternerState>> = OnceLock::new();

#[inline]
fn interner() -> &'static Mutex<InternerState> {
    INTERNER.get_or_init(|| {
        Mutex::new(InternerState {
            map: HashMap::new(),
            strings: Vec::new(),
        })
    })
}

/// Interns a string, returning its stable [`Name`].
///
/// COLD — call at spawn / setup, never per frame. Deduplicates: a second
/// `intern` of an equal string returns the SAME [`Name`] (and thus the same
/// [`NameId`]). On first sight the string is leaked (process-lifetime) and
/// assigned the next sequential id.
///
/// # Panics
///
/// Panics only if the interner `Mutex` was poisoned by a prior panic while held
/// (a bug elsewhere), which cannot happen on the documented cold path.
pub fn intern(s: &str) -> Name {
    let mut state = interner()
        .lock()
        .expect("invariant: interner mutex is never held across a panic (cold setup path)");

    if let Some(&id) = state.map.get(s) {
        return Name(NameId(id));
    }

    // First sight: leak the string so the `&'static str` is immortal, then record
    // it. The id is the current length, so the `strings`/`map` stay in lock-step
    // by construction (the debug_assert documents that invariant + the ceiling).
    let id = state.strings.len();
    debug_assert!(id < u32::MAX as usize, "NameId space (u32) exhausted");
    debug_assert_eq!(id, state.strings.len());
    let leaked: &'static str = Box::leak(Box::<str>::from(s));
    state.strings.push(leaked);
    state.map.insert(leaked, id as u32);
    Name(NameId(id as u32))
}

/// Resolves a [`Name`] back to its interned string.
///
/// COLD — setup / debug only; nothing on a per-frame path calls this. Returns
/// `None` for a [`NameId`] that was never minted by [`intern`] (e.g. a hand-rolled
/// `Name(NameId(x))`). The returned `&'static str` is sound: the interned
/// allocation is leaked (immortal), and the *reference copy* handed out here
/// outlives the `Mutex` guard.
pub fn resolve(name: Name) -> Option<&'static str> {
    let state = interner()
        .lock()
        .expect("invariant: interner mutex is never held across a panic (cold setup path)");
    state.strings.get(name.0.0 as usize).copied()
}

/// The number of distinct interned strings.
///
/// `#[doc(hidden)]` test/diagnostic hook (cold): integration tests assert that a
/// per-frame `Query<&Name>` iteration does NOT grow this count (the Principle-0
/// boundary). Not part of the supported API surface.
#[doc(hidden)]
pub fn interner_len() -> usize {
    interner()
        .lock()
        .expect("invariant: interner mutex is never held across a panic (cold setup path)")
        .strings
        .len()
}
