//! The diagnostic-code registry.
//!
//! One `codes! { … }` invocation generates a `pub const` per code, a **dense** table sorted by
//! number, a dense index per code, and [`explain`]. A `"boyko-…"` literal outside this registry is
//! a build failure — enforced by the registry walker, not by convention.
//!
//! # Class is a TYPE, not a field
//!
//! `warn!` takes a [`WarnCode`], `error!` takes an [`ErrorCode`], and [`PanicCode`] is a third
//! type. A class mismatch does not compile. A single `Code(u16)` with a `class: u8` field would
//! have made "warn with a panic code" a runtime concern, and a runtime concern about a
//! *diagnostic* is one nobody sees until the diagnostic matters.
//!
//! # `Live` / `Pending` / `Historical`, and why the grandfathered codes are all `Pending`
//!
//! Nine codes already exist in this engine's sources — as **string literals** in panic messages
//! and `#[should_panic]` expectations, never as identifiers. The registry's orphan check scans for
//! the *identifier* (`codes::B1802`), which does not appear anywhere until the migration rungs
//! rewrite those call sites. So:
//!
//! - [`CodeStatus::Live`] — the orphan check requires at least one identifier use, **and** a
//!   `docs/diagnostics/<code>.md` page. Register a `Live` code and emit it nowhere ⇒ red.
//! - [`CodeStatus::Pending`] — the premature-emitter check requires **zero** identifier uses. Emit
//!   a `Pending` code ⇒ red, which forces its row to flip to `Live` in the same commit that lands
//!   the emitter. A `Pending` row cannot rot silently: the day it acquires an emitter it reds.
//! - [`CodeStatus::Historical`] — zero emitters permitted, no page required, never becomes `Live`.
//!   For codes that exist only in frozen artifacts this repository will not edit.
//!
//! At L2 **all nine grandfathered codes were `Pending`**, which is what let that rung land alone.
//! The registry corpus file said of `B9004`/`B9005` that "both are `Live` from L2 (they have
//! emitters today)"; that contradicted its own orphan rule, because *those* occurrences were
//! literals and comments, not identifiers, and a `Live` row with no identifier use reds
//! immediately. The check semantics won. Their doc pages were written anyway — a page for a
//! `Pending` row is permitted and the debt was already identified.
//!
//! **L6 flipped seven of the nine**, by giving [`PanicCode`] a `Display` and rewriting each panic
//! message to carry the *constant* rather than the literal. The rendered text is byte-identical,
//! so every `#[should_panic(expected = "boyko-B…")]` in the engine still matches; what changed is
//! that the identifier now exists in source, which is the only thing the orphan check can see.
//! `B1801`/`B1802` stay `Pending("L8b")` — they are `boyko_app`'s, and that is L8b's rung.

use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

/// How often a `W`/`E` code may be delivered.
///
/// Declared **on the code**, applied **per site**. Declaring it on the code is what makes the
/// policy visible in the registry — a reader can see that `W2102` is `Once` without finding every
/// emitter — while applying it per site keeps two unrelated call sites from silencing each other.
///
/// # How the column reaches a site *(L8a wired)*
///
/// The emission macros read it. `warn!`/`error!` and their `_kv!`/`dyn_` forms gate on the three
/// ceilings and then on [`__log_rate_admits!`](crate::__log_rate_admits), which binds
/// `policy()` into a **`const`** at the call site — so the four arms a site does not declare are
/// deleted rather than branched over, and a row declaring
/// [`Every`](RatePolicy::Every) still costs exactly nothing.
///
/// Two of the five reach [`rate::admit`](crate::rate::admit): [`EveryN`](RatePolicy::EveryN) and
/// [`MinIntervalMs`](RatePolicy::MinIntervalMs), the two that need state shared across sites.
///
/// # What is still a human link, stated rather than implied
///
/// [`Once`](RatePolicy::Once) and [`OnceCounted`](RatePolicy::OnceCounted) are answered by an
/// [`OnceSite`] **the site declares**, not by a latch the macro places, and that is a decision:
/// a `static` inside a macro expansion cannot be named, and `OnceSite::reset` exists precisely
/// so an observer can reset the latch it is about to test. Auto-latching would buy redundancy at
/// the price of making every `Once` site untestable in isolation.
///
/// So a row declaring `Once` is still honoured only because a human placed a latch at each
/// emitter. Until L8a-wired that was true of `EveryN` and `MinIntervalMs` too — and worse, since
/// *no* latch exists for those, a row declaring one was a **declaration with no effect** and
/// nothing could notice. `codes.rs` carried
/// `no_live_row_declares_a_policy_the_emission_path_cannot_honour` to forbid such a row outright;
/// that gate is DELETED with this rung, and `tests/l8a_rate_policy_wired.rs` replaces it by
/// exercising all four policies through the real macro.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RatePolicy {
    /// Every occurrence.
    Every,
    /// The first occurrence at each site; later ones are dropped **without being counted**.
    ///
    /// The steady state is a pure `Relaxed` load of a site-local latch: no store, no shared line.
    Once,
    /// As [`Once`](RatePolicy::Once), but each suppressed occurrence pays one RMW so the count is
    /// real. A code whose suppressed count genuinely matters declares this and pays for it, at
    /// its own declaration site, with the cost written in the row.
    OnceCounted,
    /// One in `n`. **`n` must be a power of two**, so the test is `count & (n - 1)`.
    EveryN(u32),
    /// At most one per `ms` milliseconds.
    MinIntervalMs(u32),
}

/// Whether a row has emitters yet, and whether it ever will.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodeStatus {
    /// Has emitters. Owes a doc page.
    Live,
    /// Reserved for a named rung; must have **no** emitters yet.
    Pending(&'static str),
    /// Exists only in frozen artifacts. No emitters, no page, never `Live`.
    Historical,
}

impl CodeStatus {
    /// Whether the orphan check requires an identifier use of this row.
    #[must_use]
    pub const fn requires_emitter(self) -> bool {
        matches!(self, CodeStatus::Live)
    }

    /// Whether the premature-emitter check requires **zero** identifier uses.
    #[must_use]
    pub const fn forbids_emitter(self) -> bool {
        matches!(self, CodeStatus::Pending(_) | CodeStatus::Historical)
    }
}

/// One registry row.
pub struct DiagInfo {
    /// The printed number, e.g. `1802` for `boyko-B1802`.
    pub number: u16,
    /// `b'B'`, `b'E'` or `b'W'`.
    pub class: u8,
    /// One line, imperative, no trailing period.
    pub summary: &'static str,
    /// Delivery policy.
    pub rate: RatePolicy,
    /// Migration state.
    pub status: CodeStatus,
}

/// A `W`-class code: a condition the caller probably did not intend.
#[derive(Clone, Copy, Debug)]
pub struct WarnCode {
    num: u16,
    idx: CodeIdx,
    /// The declared delivery policy, copied from this code's registry row.
    ///
    /// **Carried on the value rather than looked up from the table**, and that is what
    /// lets the emission macros fold the policy away: a code is a `const` at its call
    /// site, so `policy()` is a constant expression and the four arms the site does not
    /// declare are deleted before a branch is ever emitted. A table lookup would put a
    /// scan on the emitting thread, and a downstream code is not in the engine's table
    /// at all -- so the lookup form would silently report `Every` for every game code.
    ///
    /// It is not a second source of truth: `codes!` and `declare_codes!` write this
    /// field and the row's `rate` column from the SAME row token.
    rate: RatePolicy,
}

/// An `E`-class code: the engine could not do what was asked.
#[derive(Clone, Copy, Debug)]
pub struct ErrorCode {
    num: u16,
    idx: CodeIdx,
    /// The declared delivery policy, copied from this code's registry row.
    ///
    /// **Carried on the value rather than looked up from the table**, and that is what
    /// lets the emission macros fold the policy away: a code is a `const` at its call
    /// site, so `policy()` is a constant expression and the four arms the site does not
    /// declare are deleted before a branch is ever emitted. A table lookup would put a
    /// scan on the emitting thread, and a downstream code is not in the engine's table
    /// at all -- so the lookup form would silently report `Every` for every game code.
    ///
    /// It is not a second source of truth: `codes!` and `declare_codes!` write this
    /// field and the row's `rate` column from the SAME row token.
    rate: RatePolicy,
}

/// A `B`-class code: a broken invariant. Appears only inside a `#[cold] fn … -> !` or a `panic!`.
#[derive(Clone, Copy, Debug)]
pub struct PanicCode {
    num: u16,
    idx: CodeIdx,
    /// The declared delivery policy, copied from this code's registry row.
    ///
    /// **Carried on the value rather than looked up from the table**, and that is what
    /// lets the emission macros fold the policy away: a code is a `const` at its call
    /// site, so `policy()` is a constant expression and the four arms the site does not
    /// declare are deleted before a branch is ever emitted. A table lookup would put a
    /// scan on the emitting thread, and a downstream code is not in the engine's table
    /// at all -- so the lookup form would silently report `Every` for every game code.
    ///
    /// It is not a second source of truth: `codes!` and `declare_codes!` write this
    /// field and the row's `rate` column from the SAME row token.
    rate: RatePolicy,
}

/// Equality is by **number**, for all three classes.
///
/// Not derived, and it could not be: [`CodeIdx::Dynamic`] holds a `&'static AtomicU16`, so a
/// derived `PartialEq` would compare *cells* — making a code unequal to itself across two
/// declarations and equal to another only by pointer accident. The class is already a *type*, so
/// comparing numbers within one type compares the whole identity.
macro_rules! code_eq {
    ($T:ident) => {
        impl PartialEq for $T {
            #[inline]
            fn eq(&self, other: &Self) -> bool {
                self.num == other.num
            }
        }
        impl Eq for $T {}
    };
}
code_eq!(WarnCode);
code_eq!(ErrorCode);
code_eq!(PanicCode);

macro_rules! code_newtype {
    ($T:ident, $class:expr) => {
        impl $T {
            /// The printed number.
            #[inline]
            #[must_use]
            pub const fn number(self) -> u16 {
                self.num
            }

            /// Build a **downstream** code: a number and the cell its index mints into.
            ///
            /// `pub` because [`declare_codes!`](crate::declare_codes) expands in another crate and
            /// must construct these from outside this module. There is deliberately no public
            /// constructor taking a raw `CodeIdx::Static`: an engine row's index is the position
            /// [`codes!`] computed for it, and a caller who could forge one could point a code at
            /// another code's rate slot — the aliasing the whole mint exists to prevent.
            #[inline]
            #[must_use]
            pub const fn downstream(
                number: u16,
                rate: RatePolicy,
                cell: &'static AtomicU16,
            ) -> Self {
                Self { num: number, idx: CodeIdx::Dynamic(cell), rate }
            }

            /// Where this code's rate slot lives: `Static` for an engine row, `Dynamic` for a
            /// downstream one that mints on first use (Decision 19).
            #[inline]
            #[must_use]
            pub const fn idx(self) -> CodeIdx {
                self.idx
            }

            /// This code's declared delivery policy.
            ///
            /// `const`, and that is the whole reason the rate gate costs nothing at the 74 sites
            /// that declare no damping: [`__log_rate_admits!`](crate::__log_rate_admits) binds this
            /// into a `const` at the call site, so the arm the site does not take is not compiled.
            ///
            /// **`PanicCode` has one too, and no caller.** A `B` code's site is a `panic!`, not an
            /// emission macro, so nothing folds its policy. It is here because the *row* has a
            /// `rate` column and the newtype mirrors the row — a `B`-shaped special case in
            /// `code_newtype!` would be more surface than the unread method, and the value is read
            /// by anyone reading the registry.
            #[inline]
            #[must_use]
            pub const fn policy(self) -> RatePolicy {
                self.rate
            }

            /// The class byte.
            #[inline]
            #[must_use]
            pub const fn class(self) -> u8 {
                $class
            }

            /// This code's dense registry index — the rate-table slot.
            ///
            /// Dense and equal to the row's position in the sorted table, so the rate array needs
            /// no map and no hash. Returns [`CODE_IDX_EXHAUSTED`] for a code the registry does not
            /// carry, which is unreachable for engine codes (they are minted by the macro) and is
            /// the downstream path's failure mode.
            #[inline]
            #[must_use]
            pub fn code_idx(self) -> u32 {
                resolve_idx(self.idx)
            }
        }
    };
}

code_newtype!(WarnCode, b'W');
code_newtype!(ErrorCode, b'E');
code_newtype!(PanicCode, b'B');

/// Prints `boyko-Bnnnn` — byte for byte the prefix every `B`-class panic message carried as a
/// string literal before L6.
///
/// **Only `PanicCode` has one, and the asymmetry is the point.** `W`/`E` codes reach their sites
/// through the emission macros, which take `.number()` and let the sink print the prefix from the
/// site's own `prefix`/`class`/`code`; a `Display` for those two would be public surface with no
/// caller, and a value nothing reads is a value nothing can prove wrong. A `B` code has no macro —
/// its site is a `panic!` and the message body is the only place the code can appear — so this is
/// what lets the migration replace the literal `"boyko-B9001: …"` with the **identifier**, which
/// is what the registry's orphan check scans for.
///
/// **The prefix is hard-coded.** Downstream tables (L11a) mint codes with their own prefix through
/// the exported `codes!`, and that rung owns giving them a prefix-carrying `Display`. Threading a
/// prefix through this impl for a table that does not exist yet would be a second answer to a
/// question nobody has asked.
///
/// A caller must write it **positionally** — `panic!("{}: …", B9001)`, never `panic!("{B9001}: …")`.
/// An inline format argument lives inside the string literal, so the walker's LIT stream sees it
/// and its CODE stream does not, and the row would still read as an orphan.
impl core::fmt::Display for PanicCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "boyko-B{:04}", self.num)
    }
}

/// The index a code gets when no rate slot could be assigned.
///
/// Engine codes are minted at compile time and can never see it. A downstream table that exhausts
/// the dynamic index space gets this, and a site holding it falls back to
/// [`RatePolicy::Every`] semantics with no rate state — a **degradation that is counted**, never a
/// panic and never a silent re-use of somebody else's slot.
pub const CODE_IDX_EXHAUSTED: u32 = u32::MAX;

/// Declare the registry.
///
/// Rows must be in **strictly increasing number order**, which is checked by a `const` assert:
/// a duplicate or an out-of-order row does not compile. Two rows may never share a number **even
/// across classes** — the table is dense with `index == code_idx`, so `W0110` and a hypothetical
/// `E0110` would collide in the rate array rather than merely read oddly.
macro_rules! codes {
    ($(
        ($num:expr, $class:ident, $Ident:ident, $rate:expr, $status:expr, $summary:literal)
    ),* $(,)?) => {
        $(
            #[doc = concat!("`boyko-", stringify!($class), stringify!($num), "` — ", $summary, ".")]
            pub const $Ident: code_class_ty!($class) = code_class_new!(
                $class,
                $num,
                // The same token that fills this row's `rate` column below, so the value a site
                // folds and the value the registry prints cannot disagree.
                $rate,
                // The row's own position, resolved when the table compiles -- which is what makes
                // "an engine `code_idx` is a compile-time constant" structural rather than a
                // promise: nothing a downstream crate mints can move it.
                CodeIdx::Static(const_row_of(
                    &DIAGNOSTICS_TABLE,
                    stringify!($class).as_bytes()[0],
                    $num,
                ))
            );

            // THE SITE'S POLICY AND THE REGISTRY'S ARE ONE TOKEN, AND THIS IS WHAT MAKES THAT
            // STRUCTURAL. `code_class_new!` could drop or transpose its rate argument and every
            // test in this crate would stay green -- the registry would print one policy while
            // every call site folded another, and only a behavioural test of a damped code could
            // see it. A compile-time assert per row is the cheap form of that test.
            const _: () = assert!(
                rate_eq(
                    $Ident.policy(),
                    DIAGNOSTICS_TABLE[const_row_of(
                        &DIAGNOSTICS_TABLE,
                        stringify!($class).as_bytes()[0],
                        $num,
                    ) as usize]
                        .rate
                ),
                concat!(
                    "the `",
                    stringify!($Ident),
                    "` const and its registry row disagree about the rate policy"
                )
            );
        )*

        /// Every row, sorted by number. `index == code_idx`.
        ///
        /// A `const` array with a `static` slice view below it, and the split is load-bearing:
        /// const evaluation may read a `const` and may NOT read a `static`, and each code's index
        /// is computed by a `const fn` scan over this table when the table compiles. The form the
        /// macro reaches for first -- `${index()}` -- is an unstable meta-variable expression.
        pub const DIAGNOSTICS_TABLE: [DiagInfo; [$($num),*].len()] = [$(
            DiagInfo {
                number: $num,
                class: stringify!($class).as_bytes()[0],
                summary: $summary,
                rate: $rate,
                status: $status,
            },
        )*];

        /// The slice view every reader uses. Same data, one indirection, no second source of truth.
        pub static DIAGNOSTICS: &[DiagInfo] = &DIAGNOSTICS_TABLE;

        const _: () = assert!(
            numbers_strictly_increasing(&[$($num),*]),
            "registry rows must be in strictly increasing number order: a duplicate, a \
             cross-class collision, or an out-of-order row"
        );
        const _: () = assert!(
            rate_policies_are_representable(&[$($rate),*]),
            "EveryN(n) requires n to be a power of two, so the test is `count & (n - 1)`: an \
             arbitrary n mis-samples across the counter wrap, invisibly in a bench and wrongly \
             in a session"
        );
    };
}

/// The row position of `(class, number)` in a code table, at COMPILE time.
///
/// A `const fn`, so the engine's indices are constants rather than a startup scan. It takes the
/// table by reference because a `const fn` may read a `const` and may not read a `static` -- which
/// is why [`codes!`] emits `DIAGNOSTICS_TABLE` as a `const` with a slice view beside it.
///
/// Panics at COMPILE time on a row the table does not carry, which is unreachable by construction:
/// the only caller is the macro that generated the row it is looking for.
#[must_use]
pub const fn const_row_of(table: &[DiagInfo], class: u8, number: u16) -> u16 {
    let mut i = 0;
    while i < table.len() {
        if table[i].class == class && table[i].number == number {
            return i as u16;
        }
        i += 1;
    }
    panic!("invariant: codes! generated a const for a row it did not put in the table")
}

/// Maps a class token in [`codes!`] to its newtype.
///
/// `macro_rules!` is textually scoped, and what has to be in scope is the **invocation** site,
/// not the other macro's definition — both are defined above the single `codes! { … }` call at
/// the bottom of this file, which is where expansion happens. No `use` is needed and one was
/// removed: it was an unused import that `-D warnings` correctly refused.
/// Build the newtype for a class token. Separate from [`code_class_ty!`] because a macro call may
/// not stand where a struct literal's PATH goes -- `code_class_ty!(W) { num: .. }` is a parse
/// error, which is the entire reason this exists.
macro_rules! code_class_new {
    (B, $num:expr, $rate:expr, $idx:expr) => { PanicCode { num: $num, idx: $idx, rate: $rate } };
    (E, $num:expr, $rate:expr, $idx:expr) => { ErrorCode { num: $num, idx: $idx, rate: $rate } };
    (W, $num:expr, $rate:expr, $idx:expr) => { WarnCode { num: $num, idx: $idx, rate: $rate } };
}

macro_rules! code_class_ty {
    (B) => {
        PanicCode
    };
    (E) => {
        ErrorCode
    };
    (W) => {
        WarnCode
    };
}

/// `true` when every number is greater than the one before it.
/// Whether two policies are the same policy, in a `const` context.
///
/// `PartialEq` is derived on [`RatePolicy`] but is not `const`, and the one place this is needed is
/// a `const _` assert. Written out rather than reached for by deriving something cleverer: the
/// wildcard arm is the whole safety of it — a new variant added without a row here compares
/// unequal to itself and reds the assert, which is the correct direction for a check.
#[must_use]
pub const fn rate_eq(a: RatePolicy, b: RatePolicy) -> bool {
    match (a, b) {
        (RatePolicy::Every, RatePolicy::Every)
        | (RatePolicy::Once, RatePolicy::Once)
        | (RatePolicy::OnceCounted, RatePolicy::OnceCounted) => true,
        (RatePolicy::EveryN(x), RatePolicy::EveryN(y)) => x == y,
        (RatePolicy::MinIntervalMs(x), RatePolicy::MinIntervalMs(y)) => x == y,
        _ => false,
    }
}

/// Whether ONE row's policy is representable. The slice form is a `const` assert at declaration;
/// this is its per-row companion, for a downstream table checking itself at run time.
#[must_use]
pub const fn rate_policy_is_representable(rate: RatePolicy) -> bool {
    match rate {
        RatePolicy::EveryN(n) => n != 0 && n.is_power_of_two(),
        _ => true,
    }
}

pub const fn numbers_strictly_increasing(nums: &[u16]) -> bool {
    let mut i = 1;
    while i < nums.len() {
        if nums[i - 1] >= nums[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `true` when every `EveryN(n)` has a power-of-two `n`.
///
/// An arbitrary `n` forces `count % n`, which mis-samples across the `u32` counter wrap — about
/// twelve hours at 100 K·s⁻¹, so invisible in a three-hundred-frame bench and wrong in a session.
/// The power-of-two form is *also* cheaper: an `and` instead of a division. Strictly better on
/// both axes, which is why it is a compile error rather than a lint.
const fn rate_policies_are_representable(rates: &[RatePolicy]) -> bool {
    let mut i = 0;
    while i < rates.len() {
        if let RatePolicy::EveryN(n) = rates[i]
            && !n.is_power_of_two()
        {
            return false;
        }
        i += 1;
    }
    true
}

/// The dense index of `(class, number)`, or [`CODE_IDX_EXHAUSTED`].
///
/// Linear over a table of tens; called once per site at first use and cached in the site's own
/// cell, never per emission.
#[must_use]
pub fn code_idx_of(class: u8, number: u16) -> u32 {
    let mut i = 0;
    while i < DIAGNOSTICS.len() {
        if DIAGNOSTICS[i].class == class && DIAGNOSTICS[i].number == number {
            return i as u32;
        }
        i += 1;
    }
    CODE_IDX_EXHAUSTED
}

/// The registry row for `(class, number)`.
#[must_use]
pub fn explain(class: u8, number: u16) -> Option<&'static DiagInfo> {
    let idx = code_idx_of(class, number);
    if idx == CODE_IDX_EXHAUSTED { None } else { Some(&DIAGNOSTICS[idx as usize]) }
}

// ─────────────────── L11a: the dynamic half of the index space ───────────────────

/// Dense rate-slot indices. Must equal [`crate::rate::MAX_RATE_SLOTS`], and the `const` assert
/// below is what keeps the two from drifting: this module hands out indices, that one allocates
/// the slots they name, and a mint past the array's end is an aliased slot or an out-of-bounds
/// read depending on which side is larger.
pub const MAX_CODES: u16 = crate::rate::MAX_RATE_SLOTS as u16;
const _: () = assert!(MAX_CODES as usize == crate::rate::MAX_RATE_SLOTS);

/// Engine rows occupy the low indices densely, so downstream minting starts above them.
///
/// Not a runtime read: the engine table is a `const` table, so where downstream space begins is a
/// compile-time fact — which is the L11a invariant that an engine `code_idx` **remains a
/// compile-time constant** and cannot be perturbed by anything a downstream crate does.
pub const DOWNSTREAM_IDX_BASE: u16 = DIAGNOSTICS.len() as u16;
const _: () = assert!(DOWNSTREAM_IDX_BASE < MAX_CODES);
// The sentinel must lie outside every representable index, or it IS an index -- the exact hazard
// `TargetId::INVALID`'s deletion avoids one module over. A COMPILE-TIME claim, so it is asserted
// here rather than at run time: clippy correctly refused the runtime form in
// `tests/l11a_code_minting.rs` as always-true, which is what a check that cannot fail looks like.
const _: () = assert!((MAX_CODES as u32) < CODE_IDX_EXHAUSTED);

/// The un-minted state of a downstream code's index cell. Zero, so a `static` costs `.bss` only.
const UNASSIGNED: u16 = 0;
/// The claim a racing minter sees while the winner is publishing. Never an index.
const RESERVED: u16 = u16::MAX - 1;

/// How many downstream indices have been handed out.
static CODE_OCCUPANCY: AtomicU16 = AtomicU16::new(0);

/// Where a code's rate slot lives.
///
/// **Two variants and not one, because the two halves are known at different times.** An engine
/// code's index is its row in the `const` table; a downstream code's is minted on first use, so it
/// needs a cell to be minted *into*. Collapsing them into a runtime lookup would put a scan on the
/// path that the `Static` arm resolves at compile time.
#[derive(Clone, Copy, Debug)]
pub enum CodeIdx {
    /// An engine row: the index IS the row, fixed when the table compiled.
    Static(u16),
    /// A downstream code: `UNASSIGNED` until first use, then a dense index, forever.
    Dynamic(&'static AtomicU16),
}

/// Resolve a code's dense index, minting one on first use for the downstream half.
///
/// # The reserve-then-publish protocol, and what it is defending
///
/// Sixteen threads reaching one un-minted code must produce **exactly one** index and leak none.
/// A bare `fetch_add` per caller leaks: every racer takes a slot and all but one are abandoned. A
/// bare `compare_exchange` on the index itself cannot work either — the winner does not yet know
/// what value to publish, because obtaining it is the very thing being serialised.
///
/// So: `CAS UNASSIGNED -> RESERVED` picks the winner, the winner alone does `fetch_add` on
/// [`CODE_OCCUPANCY`], and it publishes with `Release`. Losers spin on the cell until it stops
/// reading `RESERVED`, which is bounded by one `fetch_add` on another core.
///
/// # Exhaustion never aliases
///
/// Past [`MAX_CODES`] the mint returns [`CODE_IDX_EXHAUSTED`], **a reserved sentinel and never an
/// index**, and the caller degrades to `RatePolicy::Every` semantics with no rate state. The
/// alternative a first draft of this design is always tempted by — `fetch_add(1) % MAX_CODES` —
/// silently gives two codes one slot, which is a rate limiter throttling an unrelated code's
/// storm. `E0115` fires once and `LogStats.codes_unindexed` counts every later emission.
#[inline]
#[must_use]
pub fn resolve_idx(idx: CodeIdx) -> u32 {
    match idx {
        CodeIdx::Static(i) => u32::from(i),
        CodeIdx::Dynamic(cell) => mint(cell),
    }
}

/// The cold half of [`resolve_idx`], out of line so the `Static` arm stays a move.
#[cold]
#[inline(never)]
fn mint(cell: &'static AtomicU16) -> u32 {
    loop {
        match cell.load(Ordering::Acquire) {
            UNASSIGNED => {
                if cell
                    .compare_exchange(UNASSIGNED, RESERVED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    let next = CODE_OCCUPANCY.fetch_add(1, Ordering::AcqRel);
                    let idx = DOWNSTREAM_IDX_BASE.saturating_add(next);
                    let published = if next >= MAX_CODES - DOWNSTREAM_IDX_BASE || idx >= MAX_CODES {
                        // Exhausted. The sentinel is published so later callers stop re-minting,
                        // and it is NOT an index -- see this function's exhaustion clause.
                        u16::MAX
                    } else {
                        idx
                    };
                    cell.store(published, Ordering::Release);
                    if published == u16::MAX {
                        crate::rate::report_space_exhausted();
                        return CODE_IDX_EXHAUSTED;
                    }
                    if code_space_nearly_full() {
                        crate::rate::report_space_nearly_full();
                    }
                    return u32::from(published);
                }
            }
            // A racer holds the claim; its publish is one `fetch_add` away.
            RESERVED => core::hint::spin_loop(),
            u16::MAX => return CODE_IDX_EXHAUSTED,
            already => return u32::from(already),
        }
    }
}

/// Downstream indices handed out so far. The census's question, and `W0114`'s subject.
#[must_use]
pub fn code_occupancy() -> u16 {
    CODE_OCCUPANCY.load(Ordering::Relaxed)
}

/// `true` once the downstream index space is at least nine tenths spent — `W0114`'s condition.
///
/// Computed against the DOWNSTREAM capacity rather than [`MAX_CODES`], because the engine rows are
/// not a downstream crate's to spend and a warning about a budget somebody else already used would
/// fire on an empty table.
#[must_use]
pub fn code_space_nearly_full() -> bool {
    let capacity = u32::from(MAX_CODES - DOWNSTREAM_IDX_BASE);
    u32::from(code_occupancy()) * 10 >= capacity * 9
}

/// A per-site `Once` latch.
///
/// **Per site, not per code.** Two unrelated call sites sharing a code must not silence each
/// other, and the steady state must be a pure load: `Relaxed` on an `AtomicBool` that is already
/// `true` is one instruction and touches no line any other thread writes. A shared rate slot would
/// have put every `Once` site of one code on the same cache line, contended precisely during the
/// storm the policy exists to damp.
///
/// **Usually a `static` beside the emitter, but not always a `static`.** L8a's `W2205` puts two
/// of these in a `Resource` instead, because the thing that must warn once there is a per-World
/// boot snapshot: a `static` would let one world's first divergence silence another's. "Per site"
/// is about not sharing a latch with an unrelated site — it does not require file scope.
#[derive(Debug)]
pub struct OnceSite {
    fired: AtomicBool,
}

impl OnceSite {
    /// A latch that has not fired.
    #[must_use]
    pub const fn new() -> OnceSite {
        OnceSite { fired: AtomicBool::new(false) }
    }

    /// `true` the first time only.
    ///
    /// The load short-circuits the exchange, so the steady state performs **no store**. The
    /// exchange is `Relaxed` on both sides: this latch orders nothing but itself, and a lost race
    /// between two threads at a site's very first occurrence delivers the record once, which is
    /// what `Once` promises.
    #[inline]
    pub fn claim(&self) -> bool {
        if self.fired.load(Ordering::Relaxed) {
            return false;
        }
        !self.fired.swap(true, Ordering::Relaxed)
    }

    /// Whether this site has fired. Read by the census walk, never on the hot path.
    #[must_use]
    pub fn has_fired(&self) -> bool {
        self.fired.load(Ordering::Relaxed)
    }

    /// Un-fire this latch. **Test builds only** — behind the `test-probe` feature, so it does not
    /// exist in a shipping binary and cannot be reached from one.
    ///
    /// # Why this has to exist *(L8a)*
    ///
    /// A `Once` latch is **process** state, and a test binary is one process. An observer that
    /// asserts "the first occurrence reports" is asserting about a latch some *other* test may
    /// already have spent — `boyko_render`'s light-table observer failed `left: 0, right: 1` on
    /// every run because four sibling tests fold NaN lights for entirely unrelated reasons, and
    /// they had every right to. No amount of locking fixes that: the latch cannot be un-spent by
    /// waiting.
    ///
    /// So an observer resets the latch it is about to test, which makes it independent of what
    /// ran before it. That is why each of this migration's reporters keeps its latch as a NAMED
    /// module-level `static` rather than a `static` inside the function: state a test cannot name
    /// is state a test cannot control, and an observer that cannot control its preconditions is
    /// one whose green means "in this order, this time".
    #[cfg(feature = "test-probe")]
    pub fn reset(&self) {
        self.fired.store(false, Ordering::Relaxed);
    }
}

impl Default for OnceSite {
    fn default() -> Self {
        OnceSite::new()
    }
}

// ───────────────────────────────── the registry ──────────────────────────────────
//
// NINE grandfathered codes, measured against the tree rather than inherited: a scan for
// `boyko-[BEW][0-9]{4}` over `crates/**/*.rs` returns 89 occurrences and exactly these nine
// distinct codes. Every one is `Pending`, because today's occurrences are string literals and
// doc comments -- the orphan check scans IDENTIFIERS, and no identifier exists until the
// migration rungs rewrite the call sites.
//
// Every `summary` below is taken from the message the engine ACTUALLY prints, read out of the
// source, not written from the code's name. The first draft of this table invented three of them
// -- B0002 as "a system parameter the world cannot supply" (it is an intra-system access conflict
// on one resource), B9002 as "conflicting access to one resource" (it is a cycle in the SET
// hierarchy) and B9004 as "a set that was never registered" (it is two ordered sets sharing a
// member). A registry summary that disagrees with the panic text sends a reader looking for the
// wrong thing, and nothing downstream would ever have flagged it.
//
// `B9003` is a PERMANENT GAP. It exists in `docs/archive/**` only; the archive is outside the
// corpus and a code that promises no future emitter must not be seeded as `Pending`, which is a
// promise. The gap is recorded here so the next reader does not "fix" the sequence.
//
// EIGHTEEN `92xx` rows are reserved for the profiler, consecutive and with no gaps, so the code
// space is claimed before that subsystem's rungs need it. Measured first: the `9xxx` band is
// already occupied by `B9001`/`B9002`/`B9004`/`B9005`/`B9101`, but `92xx` itself is free in
// source -- zero `92xx` literals under `crates/` or `src/`.
//
// THE EIGHTEEN SUMMARIES BELOW WERE INVENTED ONCE AND ARE REPAIRED HERE (profiling rung 2).
// L2 reserved the block correctly and then wrote eighteen plausible sentences from the code
// numbers, which is the defect check 4's own message names -- "inventing a summary here is how
// three rows of this registry came to disagree with the messages the engine prints" -- committed
// at six times that scale, in the one place no check could see it: a `Pending` row owes no doc
// page (check 2) and no emitter (check 3a), so nothing compared the sentence against anything.
//
// The direction of the repair is not a judgement call. `W9207` is pinned as INVARIANT TSC ABSENT
// in five documents (`docs/diagnostics/SEAM.md:179`, both plan files, `logging/02-SINK-LIFECYCLE.md`
// and `logging/05-LADDER-GATES.md`), and logging's own `W0101` was STRUCK in its favour -- so the
// invented "a GPU query pool returned fewer results than were issued" would have left the engine's
// only invariant-TSC code naming something else while the condition it was struck for had no code
// at all. `9213` is `E9213` in the corpus (six mentions, four files) and was seeded `W9213`.
// Against that, the eighteen rows had zero readers. Repairing them is strictly smaller than
// repairing the corpus, and the summaries below are now the corpus's own conditions
// (`docs/diagnostics/profiling/05-LADDER-GATES.md` §Integration), not sentences composed here.
//
// The `Pending(<rung>)` annotations were repaired with them, for the same reason: they named the
// rungs of the invented conditions.
codes! {
    (2,    B, B0002, RatePolicy::Every, CodeStatus::Live,
        "Intra-system access conflict on one resource"),
    // The FIRST `Live` row in this registry. Its summary is read from the emitter
    // (`sink/file.rs::render_cap`) rather than composed from the code's name -- which is the rule
    // three rows of this registry's first draft broke.
    (103,  W, W0103, RatePolicy::Once,  CodeStatus::Live,
        "The file sink reached its byte cap and stopped writing"),
    // ── L11a: the downstream TARGET band's one report ─────────────────────────
    // `Every`, not `Once`: a binary with three colliding pairs has three different things to
    // tell the reader, and a latch would name one of them.
    (104,  E, E0104, RatePolicy::Every, CodeStatus::Live,
        "Two downstream log targets claim one id, and both are named"),
    // ── L10: the dynamic band's one diagnostic ───────────────────────────────────────────────
    // `Every`, and the reason is the same one `W0901` records: the subject is a NAME. Past
    // exhaustion every later registration fails, and each failure is a different mod whose logging
    // is now silently absent -- `Once` would name the first victim and hide the rest.
    //
    // This code covers ALL THREE of `register_dynamic_target`'s refusals (band full, empty name,
    // over-long name), with the reason as an argument. The corpus documents only the first
    // (`04-GAME-FACING.md`: "`None` => band exhausted"), which is narrower than the function it
    // annotates -- a caller acting on that comment would read a rejected 60-byte name as a lost
    // band. One code because they are one function's one failure return; the argument is what
    // tells the two apart.
    (106,  E, E0106, RatePolicy::Every, CodeStatus::Live,
        "A dynamic target could not be registered, and the reason names which refusal"),
    // ── L14: runtime sink control's one report ───────────────────────────────────────────────
    // `Every`: an operator who types three commands and has two refused needs to know WHICH
    // two, and a latch would name one of them.
    (107,  E, E0107, RatePolicy::Every, CodeStatus::Live,
        "A sink control request was refused because the request ring is full"),
    // ── L15: the crash path ──────────────────────────────────────────────────────────────────
    // `Once`: one process has one crash destination, and a second attempt fails for the same
    // reason. Reported at ENABLE, where there is still a healthy process to receive it.
    (109,  E, E0109, RatePolicy::Once,  CodeStatus::Live,
        "The crash file could not be opened; a panic will leave no record"),
    // ── L14: the census's unsunk report ──────────────────────────────────────────────────────
    // `Once`: the condition is a CONFIGURATION, not an event -- every later unsunk row has the
    // same cause, and one misconfiguration must not become a storm of reports about it.
    (111,  W, W0111, RatePolicy::Once,  CodeStatus::Live,
        "A target is armed but no active sink accepts it; its silence is not evidence"),
    // ── L13a: rotation's one report ───────────────────────────────────────────────────────────
    // `Once`: a session that rotates a hundred times has ONE fact to report -- the file holds
    // the tail and not the whole run -- and repeating it per rotation would bury it.
    (112,  W, W0112, RatePolicy::Once,  CodeStatus::Live,
        "File rotation discarded part of this session; the file holds the tail, not the run"),
    // ── L12: sampling's one report ────────────────────────────────────────────────────────────
    // `Once` PER PROCESS, not per target: the condition is "an operator's own shift is now
    // discarding records", and they configured it deliberately. One line telling them the
    // census counts are no longer totals is the whole job; one per target would be a storm
    // about a setting they chose.
    (113,  W, W0113, RatePolicy::Once,  CodeStatus::Live,
        "Sampling is discarding records, so delivered counts are no longer totals"),
    // ── L11a: the downstream index space's two reports ───────────────────────────────────────
    // Both `Once`, and for the same reason the condition is a THRESHOLD rather than an event: past
    // 90 % every later mint is also past 90 %, and past exhaustion every later mint also fails.
    // `Every` here would turn one budget problem into a storm of reports about it -- the shape
    // `W0501` already rejected for the query-type table.
    (114,  W, W0114, RatePolicy::Once,  CodeStatus::Live,
        "The downstream diagnostic-code index space is nine tenths spent"),
    (115,  E, E0115, RatePolicy::Once,  CodeStatus::Live,
        "The downstream diagnostic-code index space is exhausted; a code has no rate slot"),
    // ── L13b: the binary sink's site dictionary ──────────────────────────────────────────────
    // `Once`: past a full table EVERY later site writes inline, so the condition holds for the
    // rest of the run. The records are larger and none is lost, which is one fact.
    (116,  W, W0116, RatePolicy::Once,  CodeStatus::Live,
        "The binary sink's site dictionary is full; later sites are written inline"),
    // ── L16: the ECS handoff's refusals ──────────────────────────────────────────────────────
    // `Every` -- but every DRAIN, not every refusal: the count is the pass's delta, so consecutive
    // reports sum to the total instead of each restating it.
    (117,  W, W0117, RatePolicy::Every, CodeStatus::Live,
        "The ECS handoff refused frames this pass; the in-frame view is short"),
    // ── L15: the panic hook's own failure ────────────────────────────────────────────────────
    // `Every`: two threads panicking are two facts, and a latch would report one of them.
    (118,  E, E0118, RatePolicy::Every, CodeStatus::Live,
        "The panic hook could not claim the drain role; the crash file is short"),
    // ── L6's five new rows ──────────────────────────────────────────────────────────────────
    // Every summary below is read out of the message its emitter actually prints. `E0201` is the
    // pool's only diagnostic; `W0501`/`B0502` are the two halves of one table filling up, and the
    // warning exists because the panic alone reported 1023 silent mints and then a process kill.
    (201,  E, E0201, RatePolicy::Every, CodeStatus::Live,
        "A fire-and-forget task panicked, so the process is aborting"),
    (501,  W, W0501, RatePolicy::Once,  CodeStatus::Live,
        "The query-type table is three quarters full"),
    (502,  B, B0502, RatePolicy::Every, CodeStatus::Live,
        "The query-type table is exhausted; no id is left to mint"),
    (701,  W, W0701, RatePolicy::Once,  CodeStatus::Live,
        "An event lane was full, so the send was refused"),
    (801,  E, E0801, RatePolicy::Every, CodeStatus::Live,
        "An asset failed to load and its handle was marked failed"),
    // ── L8a: `boyko_serialize` ───────────────────────────────────────────────────────────────
    // `Every`, because the subject is a COMPONENT TYPE: a save carrying three undecodable dense
    // stores has three different things to tell the reader, and `Once` would name one of them.
    // The emitter loses its `#[cfg(debug_assertions)]` here -- `LoadReport::dense_stores_skipped`
    // is already incremented on the line above the call in every profile, so the condition costs
    // release nothing it was not already paying.
    (901,  W, W0901, RatePolicy::Every, CodeStatus::Live,
        "A dense store carried no decodable data, so its members were skipped"),
    // ── L8a: `boyko_physics` ────────────────────────────────────────────────────────────────
    // Three sites in `soft/self_collision.rs`, and the `#[cfg(debug_assertions)]` question was
    // decided PER SITE by measuring what release already computes -- not by applying L7b's
    // un-gating uniformly:
    //   W1301  the `radius <= 0.0` test is the release guard that disables the pass  -> un-gated
    //   W1303  `table != 0 && n > 4 * table` is two compares, once per resolve call  -> un-gated
    //   W1302  needs `min(body.c_rest)` -- an O(constraints) scan release does NOT
    //          otherwise perform, per resolve call, per body                         -> STAYS
    //          `#[cfg(debug_assertions)]`
    // L7b's rule is "a release-build degrade-to-disabled must be observable"; it is not "delete
    // every debug gate", and the third site is what tells the two apart.
    (1301, W, W1301, RatePolicy::Once,  CodeStatus::Live,
        "Self-collision is skipped because the particle radius is not positive"),
    (1302, W, W1302, RatePolicy::Once,  CodeStatus::Live,
        "The self-collision cell size exceeds the smallest constraint rest length"),
    (1303, W, W1303, RatePolicy::Once,  CodeStatus::Live,
        "The self-collision spatial hash is overloaded, so bucket chains are long"),
    (1501, W, W1501, RatePolicy::Once,  CodeStatus::Live,
        "Ordering references a system set that has no members"),
    // L8b flipped both. They are **`boyko_ecs`'s**, not `boyko_app`'s -- this block's header calls
    // `18xx` the app block and the ledger reads that as the `boyko_app` crate, but the two panic
    // sites measure at `ecs/core/app/app.rs:887` and `:899`. The block is about `App`, the type,
    // which lives in the kernel. Recorded because "L8b migrates `boyko_app`" and "L8b flips
    // B1801/B1802" are two true statements about two different crates.
    (1801, B, B1801, RatePolicy::Every, CodeStatus::Live,
        "A plugin was added more than once"),
    (1802, B, B1802, RatePolicy::Every, CodeStatus::Live,
        "An App method was called after finish(), in the run phase"),
    // `Every`, and the subject is why: each occurrence names a DIFFERENT mistyped value, so a latch
    // would report the first launch flag anyone got wrong and silence every later one. The site
    // runs once per process in practice, which is a fact about the caller and not a policy.
    (1803, W, W1803, RatePolicy::Every, CodeStatus::Live,
        "A launch flag named a runtime preset the table does not carry"),
    // ── L8c: `boyko_rhi` ────────────────────────────────────────────────────────────────────
    // The FIRST code in the `20xx` band proper, and the crate is here because `print_census.rs`
    // found it -- not because any ledger row named it. Five migration rungs each reported a
    // complete crate list; `boyko_rhi` was in none of them, and its one print sat in a `Drop`
    // tripwire that its own comment says must "survive in RELEASE".
    //
    // `Every`, and it cannot be otherwise: the site is `ResourceRegistry::drop`, which runs at most
    // once per registry, and a process with two leaking registries has two leaks to report.
    (2001, E, E2001, RatePolicy::Every, CodeStatus::Live,
        "A resource registry was dropped with live resources, so every one of them leaked"),
    // L7. Its condition is "validation was requested and this process is NOT getting it" -- the
    // escape hatch took it, or `VK_EXT_validation_features` is absent. NOT "the node was not
    // chained", which the corpus's L7 row implies and which the tree refutes: the node IS chained
    // and works here. See `logging/ladder`'s L7 block for the measurement that forced the re-cut.
    (2101, E, E2101, RatePolicy::Once,  CodeStatus::Live,
        "Validation was requested but this process is not getting it"),
    // L7b. `Once` is per SITE (F11), and this is the code F11 was raised for: three independent
    // capability degradations share this number, and a code-scoped latch would have reported one
    // and silently lost two. Each site owns its own `OnceSite`.
    (2102, W, W2102, RatePolicy::Once,  CodeStatus::Live,
        "A device format feature is missing, so an optional render feature is disabled"),
    // `Every`, not `Once`, and the reason is the call frequency rather than the severity: these
    // fire from the render-target builder, which runs at boot and at each resize -- not per frame.
    // A `Once` here would report the first resize that ran out of device memory and stay silent
    // through every later one, which is the failure mode the operator most needs to see repeated.
    (2103, E, E2103, RatePolicy::Every, CodeStatus::Live,
        "A mandatory render target or descriptor set failed to build; the frame will be refused"),
    (2104, W, W2104, RatePolicy::Once,  CodeStatus::Live,
        "A textured material is suppressed by the motion-vector pipeline this frame"),
    (2105, W, W2105, RatePolicy::Once,  CodeStatus::Live,
        "The requested present mode is not advertised, so the swapchain fell back to fifo"),
    // Split from `E2103` by a MEASUREMENT, not by taste: `record_vb` consumes the four `E2103`
    // subjects with `.expect(..)` (`present/passes/vb.rs:3706,3776,4291`, and `thin_normal` via the
    // two match arms that require it), and the three subjects here with `if let Some(..)`
    // (`:3988-3990`, `:4088`). One group kills the frame; the other loses an opt-in effect and
    // renders. The class letter IS the level in this registry, so one code could not carry both.
    (2106, W, W2106, RatePolicy::Every, CodeStatus::Live,
        "An optional shadow chain's sets failed to build, so the chain is skipped this frame"),
    // ── L8a: `boyko_render` ─────────────────────────────────────────────────────────────────
    // The ledger gave `light_system.rs`'s TWO sites one code, `W2201`, with a `dropped_count`
    // argument. Measuring the fold refuted both halves of that row:
    //
    //   * The two sites are not one condition. `finish_folded_overflow` fires because the scene
    //     has more lights than the table holds; `report_dropped_non_finite_light` fires because a
    //     light carries a NaN. Same class, same level -- but check 2 makes a code a page with ONE
    //     `## How to fix`, and "reduce the light count" is not "find the NaN". So `W2201` keeps
    //     the capacity condition and `W2204` takes the data one.
    //   * `dropped_count` is not knowable at the capacity site. The fold takes `impl Iterator`s,
    //     its doc pins "walked exactly once each", and on overflow it RETURNS -- so nothing has
    //     seen the remainder and counting it would mean draining iterators the contract says are
    //     not drained. `W2201` reports the cap and the rows that made it, which is what the site
    //     actually knows. The count IS real at the `W2204` site and is now carried there.
    (2201, W, W2201, RatePolicy::Once,  CodeStatus::Live,
        "More lights are enabled than the GPU light table holds, so the extras are dropped"),
    // Two sites (`bindless.rs`, `mesh_geometry_table.rs`) with the same shape and a different
    // table named -- the `W2102` argument, not the `E2103` one: one condition, one fix, one page.
    (2202, W, W2202, RatePolicy::Once,  CodeStatus::Live,
        "A bindless table exhausted its slots and aliased its reserved fallback slot"),
    // `Every`, and it is a deliberate flood. `run_unsafe` has no `Result` channel, so a device
    // fault that recurs every frame can only reach the operator as a record per frame; the ring
    // damps it by dropping and counting, which is the damping this design has. It is NOT damped
    // by this column -- see the note on `RatePolicy` above.
    // MinIntervalMs, and the decision is the corpus's rather than mine: `05-LADDER-GATES.md`
    // records this site as one that "genuinely wants per-second damping" and settles for `Every`
    // because the macros did not read this column. They do now.
    //
    // The site's own comment argued for `Every` -- and argued it FROM THE SAME ABSENCE: "the flood
    // is bounded by the ring, which drops and counts -- not by the registry's rate column, which
    // the emission macros do not read". What it was defending is that a recurring device fault
    // must stay VISIBLE, and one line per second keeps that: this is not a latch, and an hour of
    // broken frames does not look like a good one.
    //
    // What `Every` cost is not this code's own bytes but everyone else's: the lane ring is SHARED,
    // so a fault recurring at 60 fps evicts other subsystems' records for the whole session. The
    // suppressed occurrences are counted and the census prints them (`LOG-CENSUS limiter`), which
    // is the half that did not exist when the site's comment was written.
    (2203, E, E2203, RatePolicy::MinIntervalMs(1_000), CodeStatus::Live,
        "A GPU dispatch failed inside a system that has no error channel"),
    (2204, W, W2204, RatePolicy::Once,  CodeStatus::Live,
        "Lights with a non-finite position or range were dropped from the GPU light table"),
    // Two sites, one per frozen consumer (DDGI and SSAO). The latch each replaces was an
    // UNCONDITIONAL `swap` on the divergent path: once the config had diverged, every per-frame
    // reader stored `true` over `true` forever, dirtying a shared line from an `#[inline]` hot
    // reader. `OnceSite::claim` short-circuits on a `Relaxed` load, so the steady state is a load.
    (2205, W, W2205, RatePolicy::Once,  CodeStatus::Live,
        "A render-path config changed after boot, but the frozen consumer set keeps the boot value"),
    // The DECODE failure only. A material folder's five slots are all optional and a missing file
    // is the documented normal case, so absence is an `info!` with no code -- warning on it would
    // put a `Warn` in every log for every folder that ships four maps instead of five.
    (2206, W, W2206, RatePolicy::Once,  CodeStatus::Live,
        "A material texture file exists but failed to decode, so the scalar fallback is used"),
    // ── L8a: `boyko_image` ──────────────────────────────────────────────────────────────────
    // `Every` for both: each occurrence names a different chunk or a different stream, the decode
    // CONTINUES past them, and a file with two corrupt chunks is a different report from a file
    // with one. Neither is per-frame -- they run once per decode.
    (2601, W, W2601, RatePolicy::Every, CodeStatus::Live,
        "A PNG chunk's CRC-32 does not match, and decoding continued anyway"),
    (2602, W, W2602, RatePolicy::Every, CodeStatus::Live,
        "A zlib stream's Adler-32 does not match, and the decoded pixels were kept"),
    // ── L8b: `boyko_app` (the host) and `boyko_demo` ────────────────────────────────────────
    //
    // Ten rows over 15 of the crate's 45 print sites; the other 30 are `info!` with no code
    // (Decision 7). The grouping is by CONDITION, because check 2 makes a code a page with ONE
    // `## How to fix` -- so the three boot stages share `E3002` with the stage as an ARGUMENT
    // (`E2103`'s precedent, where four ring failures share one code and name the ring), while the
    // five degradations do NOT share one, because "lower the SSAA scale" is not "install a driver
    // that reports usable timestamps".
    //
    // `E3001` is `boyko_demo`'s, pinned by the ledger before this block was written, which is why
    // the host's own codes start at `3002` rather than at the top of its block.
    //
    // THE THREE `E` ROWS BELOW ARE THE ONLY CODES IN THIS REGISTRY WHOSE SITES ALSO WRITE TO
    // STDERR DIRECTLY, and the reason is measured rather than stylistic. With `BOYKO_LOG` unset
    // every target is `Off`, so a migrated `error!` produces nothing at all -- not a dropped
    // record, not a counted loss; the macro's gate folds and the site is one predicted branch.
    // That is SPECIFIED (`logging/sink-lifecycle` Decision 25: *"a flag-off run of any other
    // preset configures nothing either"*) and GATED (`log_host_reachable.rs` pins
    // `flush() == NoConsumer`), so it is not a defect -- but these three sites are the process
    // telling an operator why it is exiting, and an engine that exits silently because diagnostics
    // were not requested is strictly worse than the unconditional `eprintln!` each replaced. Each
    // therefore follows `boyko_threadpool::worker::abort_on_task_panic`'s already-blessed shape:
    // emit, and if `flush()` answers `NoConsumer`, write the same text to `stderr`. The DEGRADE
    // rows do not, because a degrade is a diagnostic and diagnostics are opt-in by design.
    (3001, E, E3001, RatePolicy::Every, CodeStatus::Live,
        "The demo could not start, so the process is exiting"),
    (3002, E, E3002, RatePolicy::Every, CodeStatus::Live,
        "A host boot stage failed, so the process is exiting without entering the frame loop"),
    (3003, E, E3003, RatePolicy::Every, CodeStatus::Live,
        "The frame loop hit a terminal device error, so the renderer is torn down and the host exits"),
    (3004, E, E3004, RatePolicy::Every, CodeStatus::Live,
        "Windowing is not implemented for this platform, so the windowed runner exits at once"),
    // `Once` per site: the two SSAA refusals are different conditions of one knob (the extent or
    // VRAM probe failed / the scale is not one this build offers) and each owns its latch.
    (3005, W, W3005, RatePolicy::Once,  CodeStatus::Live,
        "The requested SSAA scale is unavailable on this device, so supersampling is off"),
    // `Every`, and NOT `Once`, for the reason `W2102` records at F11 in its own shape: the emitter
    // is a `for degrade in reasons()` loop over a SET, so one latch would report the first reason
    // and silently lose every other one -- the failure mode that made `Once` per-site in the first
    // place, here reached through iteration rather than through separate sites.
    (3006, W, W3006, RatePolicy::Every, CodeStatus::Live,
        "The requested render path was degraded because a consumer or a device capability is missing"),
    (3007, W, W3007, RatePolicy::Once,  CodeStatus::Live,
        "The VB geometry table could not be created, so the visibility-buffer geometry leg is off"),
    (3008, W, W3008, RatePolicy::Once,  CodeStatus::Live,
        "A profiling knob was set but the device cannot serve it, so the instrument stays disabled"),
    (3009, W, W3009, RatePolicy::Once,  CodeStatus::Live,
        "An environment override names a value this build does not recognise, so the default is used"),
    // `Every`: five call sites, one per dump kind, and each names a different path. A run that
    // armed three dumps and could write none of them has three things to report, not one.
    (3010, E, E3010, RatePolicy::Every, CodeStatus::Live,
        "A diagnostic dump or artifact could not be written, and the run continued"),

    (9001, B, B9001, RatePolicy::Every, CodeStatus::Live,
        "The schedule contains a cycle of systems"),
    (9002, B, B9002, RatePolicy::Every, CodeStatus::Live,
        "The set hierarchy contains a cycle of sets"),
    (9004, B, B9004, RatePolicy::Every, CodeStatus::Live,
        "Two ordered sets share a member, so a system would run both before and after itself"),
    (9005, B, B9005, RatePolicy::Every, CodeStatus::Live,
        "Ordering references a system key that is not in this schedule"),
    (9101, B, B9101, RatePolicy::Every, CodeStatus::Live,
        "Schedule::run was called with a different world than the one it was built on"),

    (9201, W, W9201, RatePolicy::Once,  CodeStatus::Live          ,
        "The engine zone registry is exhausted; further zones run unregistered"),
    (9202, W, W9202, RatePolicy::Once,  CodeStatus::Live          ,
        "The GPU timestamp pair budget is exhausted; further brackets are unrecorded"),
    (9203, W, W9203, RatePolicy::Once,  CodeStatus::Live          ,
        "A profiling lane region overflowed, or a sample had no lane to be charged to"),
    // Class `E`, not `W`, and the difference is load-bearing rather than cosmetic: the profiling
    // corpus writes this one as the literal `boyko-E9204`, and check 1 forbids two rows sharing a
    // number EVEN ACROSS CLASSES, because the table is dense with `index == code_idx`. A `W9204`
    // row beside that literal would be both undeclared (check 4) and un-addable (check 1). The
    // ladder's shorthand says "W9201..W9218"; the literal that actually exists wins. `9213` is the
    // other row where the shorthand and the corpus disagree, and it resolves the same way.
    (9204, E, E9204, RatePolicy::Once,  CodeStatus::Live          ,
        "The profiler is already bound to another world"),
    (9205, W, W9205, RatePolicy::Once,  CodeStatus::Live          ,
        "Zones were lost in this window"),
    (9206, W, W9206, RatePolicy::Once,  CodeStatus::Live          ,
        "A contrast could not be resolved"),
    (9207, W, W9207, RatePolicy::Once,  CodeStatus::Live          ,
        "The CPU advertises no invariant TSC, so tick magnitudes are not trustworthy"),
    (9208, W, W9208, RatePolicy::Once,  CodeStatus::Live          ,
        "The engine zone registry is at or past 90 % occupancy"),
    (9209, W, W9209, RatePolicy::Once,  CodeStatus::Live          ,
        "Samples arrived after their frame had left the retained window and were dropped"),
    (9210, W, W9210, RatePolicy::Once,  CodeStatus::Live          ,
        "The user zone budget or the dynamic name arena is exhausted"),
    (9211, W, W9211, RatePolicy::Once,  CodeStatus::Live          ,
        "The fold's working set exceeds L1d because the zone stride is too large"),
    (9212, W, W9212, RatePolicy::Once,  CodeStatus::Live          ,
        "register_zone refused a dynamic zone that asked for an engine scope"),
    (9213, E, E9213, RatePolicy::Once,  CodeStatus::Live          ,
        "The profiler was re-armed with a different geometry than the live one"),
    (9214, W, W9214, RatePolicy::Once,  CodeStatus::Live          ,
        "The telemetry path is unwritable, so streaming is off for this session"),
    (9215, W, W9215, RatePolicy::Once,  CodeStatus::Live          ,
        "A telemetry write failed and streaming was disabled"),
    (9216, W, W9216, RatePolicy::Once,  CodeStatus::Live          ,
        "The clock's epoch broke; the in-flight window was discarded and the clock recalibrated"),
    (9217, W, W9217, RatePolicy::Once,  CodeStatus::Live          ,
        "GPU timestamp slots were still in flight at teardown and were abandoned"),
    (9218, W, W9218, RatePolicy::Once,  CodeStatus::Live          ,
        "A telemetry quantile subscription was refused past the per-session cap"),
    // VB-SV0 DP6-0b. The block's nineteenth row, appended rather than inserted: check 1 wants
    // strictly increasing numbers, and 9219 is the first free one after 9218.
    (9219, W, W9219, RatePolicy::Once,  CodeStatus::Live          ,
        "A GPU zone bracket recorded an END with no matching BEGIN"),
    (9220, W, W9220, RatePolicy::Once,  CodeStatus::Live          ,
        "A GPU zone's declared record order was violated on a frame"),
}

/// Declare a **downstream** diagnostic-code table: a game's, a mod's, or a tool's *(Decision 19)*.
///
/// ```ignore
/// boyko_log::declare_codes! {
///     prefix = "acme",
///     (1, W, ACME_W0001, RatePolicy::Once,  "the widget budget is nearly spent"),
///     (2, E, ACME_E0002, RatePolicy::Every, "a widget could not be built"),
/// }
/// ```
///
/// # What a downstream table gets, and what it does not
///
/// It gets its own `pub const` per code, its own `DiagInfo` table, and its own **prefix** — so its
/// records print `acme-W0001` and never `boyko-W0001`. Codes from two tables with the same number
/// are different codes, which is the point of the prefix being a table property rather than a
/// global.
///
/// Every code is a [`CodeIdx::Dynamic`], minted on first use out of the shared 512-slot space. The
/// cells live in one array per table, indexed by the same compile-time row the engine uses, so a
/// table costs `2 × rows` bytes of `.bss` and no allocation.
///
/// **Decision 7 is not relaxed here.** A `Warn`/`Error` still MUST carry a code, and a code is
/// still a promise of a documented page — which is why `codes_tidy!` exists to run the same eight
/// checks over a caller's own corpus, and why the engine's own checks say in their failure text
/// that they prove nothing about a downstream registry.
#[macro_export]
macro_rules! declare_codes {
    (
        prefix = $prefix:literal,
        $(($num:expr, $class:ident, $Ident:ident, $rate:expr, $summary:literal)),* $(,)?
    ) => {
        /// This table's rows, sorted by number.
        pub const DIAGNOSTICS_TABLE: [$crate::codes::DiagInfo; [$($num),*].len()] = [$(
            $crate::codes::DiagInfo {
                number: $num,
                class: stringify!($class).as_bytes()[0],
                summary: $summary,
                rate: $rate,
                status: $crate::codes::CodeStatus::Live,
            },
        )*];

        /// The slice view. Same data, one indirection, no second source of truth.
        pub static DIAGNOSTICS: &[$crate::codes::DiagInfo] = &DIAGNOSTICS_TABLE;

        /// This table's printed prefix. Carried so a sink renders `acme-W0001`, never `boyko-`.
        pub const PREFIX: &str = $prefix;

        /// One mint cell per row. `.bss`, two bytes each, zero until a code is first used.
        static CODE_CELLS: [::core::sync::atomic::AtomicU16; [$($num),*].len()] =
            [const { ::core::sync::atomic::AtomicU16::new(0) }; [$($num),*].len()];

        $(
            #[doc = concat!("`", $prefix, "-", stringify!($class), stringify!($num), "` — ", $summary, ".")]
            pub const $Ident: $crate::code_class_ty_pub!($class) =
                <$crate::code_class_ty_pub!($class)>::downstream(
                    $num,
                    // The same token that fills this row's `rate` column above. A downstream code
                    // is NOT in the engine's table, so a policy looked up rather than carried
                    // would read `Every` for every row here and damp nothing.
                    $rate,
                    // The row's own position, resolved when this table compiles -- the same
                    // const scan the engine table uses, so a downstream code's cell is as
                    // statically placed as an engine code's index.
                    &CODE_CELLS[$crate::codes::const_row_of(
                        &DIAGNOSTICS_TABLE,
                        stringify!($class).as_bytes()[0],
                        $num,
                    ) as usize],
                );

            // The engine table's per-row gate, for a caller's table. Same argument, and it must be
            // repeated here rather than inherited: `declare_codes!` builds its consts through
            // `downstream()` rather than `code_class_new!`, so it is a SECOND place the two values
            // could drift apart.
            const _: () = assert!(
                $crate::codes::rate_eq(
                    $Ident.policy(),
                    DIAGNOSTICS_TABLE[$crate::codes::const_row_of(
                        &DIAGNOSTICS_TABLE,
                        stringify!($class).as_bytes()[0],
                        $num,
                    ) as usize]
                        .rate
                ),
                concat!(
                    "the `",
                    stringify!($Ident),
                    "` const and its registry row disagree about the rate policy"
                )
            );
        )*

        // Same rule as the engine table, and for the same reason: two rows sharing a number would
        // share a `const_row_of` answer and therefore a mint cell.
        const _: () = assert!(
            $crate::codes::numbers_strictly_increasing(&[$($num),*]),
            "downstream registry rows must be in strictly increasing number order"
        );
    };
}

/// Generate a downstream table's own tidy checks *(Decision 19)*.
///
/// ```ignore
/// mod acme { boyko_log::declare_codes! { prefix = "acme", … } }
///
/// boyko_log::codes_tidy!(table = acme::DIAGNOSTICS, prefix = acme::PREFIX, doc_root = "docs/diag");
/// ```
///
/// # WHAT IT GENERATES, AND WHAT IT CANNOT — stated because the difference is the whole value
///
/// The engine's registry runs **eight** checks. This macro generates the five that are decidable
/// from the table and the filesystem, and it **does not pretend to the other three**:
///
/// | Engine check | Here |
/// |---|---|
/// | 0 rows present / non-vacuous | **generated** — and it is the guard, see below |
/// | 1 numbers strictly increasing, no duplicates | **generated** (also a `const` assert at declaration) |
/// | 2 every `Live` row has a doc page | **generated**, against the caller's `doc_root` |
/// | 4 summaries are non-empty and rate policies representable | **generated** |
/// | 7 class byte is one of `B`/`E`/`W` | **generated** |
/// | 3 every row has an emitter naming it as an IDENTIFIER | **not generated** |
/// | 5 every `Warn`/`Error` row is named by a test | **not generated** |
/// | 6 `B`-class codes appear only in panic position | **not generated** |
///
/// The three absent ones all need a **source walker** over the caller's own crate — comment and
/// literal stripping, the cross-file `#[cfg(test)] mod` rule, the `src/bin/` exclusion. The engine's
/// walker is a test-only module and shipping it through a macro would make every downstream crate
/// depend on this crate's test layout. Claiming those three anyway would be worse than omitting
/// them: a caller would read "tidy" and believe their codes were checked for emitters when nothing
/// had looked.
///
/// **The vacuity guard is check 0 and it is not decoration.** A table that declares no rows passes
/// every other check trivially, which is precisely how a tidy gate comes to certify nothing —
/// the failure this corpus has found at five separate rungs.
#[macro_export]
macro_rules! codes_tidy {
    (table = $table:expr, prefix = $prefix:expr, doc_root = $doc_root:expr $(,)?) => {
        /// The downstream registry's own tidy checks. Named so a failure says whose table it was.
        #[test]
        fn codes_tidy_downstream_registry() {
            let table: &[$crate::codes::DiagInfo] = $table;
            let prefix: &str = $prefix;

            // ── check 0: NON-VACUOUS ─────────────────────────────────────────────────────────
            assert!(
                !table.is_empty(),
                "the {prefix} code table is EMPTY, so every check below passes without looking at \
                 anything -- which is how a tidy gate comes to certify nothing"
            );
            assert!(!prefix.is_empty(), "a table's prefix is what makes its codes ITS codes");

            // ── check 1: strictly increasing, therefore no duplicates ────────────────────────
            for w in table.windows(2) {
                assert!(
                    w[0].number < w[1].number,
                    "{prefix} rows must be in strictly increasing number order; {} then {} is a \
                     duplicate or an out-of-order row, and two rows sharing a number share a mint \
                     cell",
                    w[0].number,
                    w[1].number
                );
            }

            // ── checks 4 and 7: the row's own contents ───────────────────────────────────────
            for row in table {
                assert!(
                    matches!(row.class, b'B' | b'E' | b'W'),
                    "{prefix}-{}{:04}: class byte {:?} is not one of B/E/W",
                    row.class as char,
                    row.number,
                    row.class as char
                );
                assert!(
                    !row.summary.is_empty(),
                    "{prefix}-{}{:04} has an empty summary -- a code with no sentence is a code \
                     nobody can act on",
                    row.class as char,
                    row.number
                );
                assert!(
                    $crate::codes::rate_policy_is_representable(row.rate),
                    "{prefix}-{}{:04}: EveryN(n) requires a power of two, or the `count & (n-1)` \
                     test mis-samples across the counter wrap",
                    row.class as char,
                    row.number
                );
            }

            // ── check 2: every row has a documented page ─────────────────────────────────────
            let root = ::std::path::Path::new($doc_root);
            let mut missing: Vec<String> = Vec::new();
            for row in table {
                let name = format!("{}{:04}.md", row.class as char, row.number);
                if !root.join(&name).exists() {
                    missing.push(name);
                }
            }
            assert!(
                missing.is_empty(),
                "these {prefix} codes have no page under {}: {missing:?}. A code is a PROMISE of a \
                 documented page -- Decision 7 is not relaxed for downstream tables.",
                root.display()
            );
        }
    };
}

/// [`code_class_ty!`] in a form an exported macro can name. Same mapping, `$crate`-qualified.
#[doc(hidden)]
#[macro_export]
macro_rules! code_class_ty_pub {
    (B) => { $crate::codes::PanicCode };
    (E) => { $crate::codes::ErrorCode };
    (W) => { $crate::codes::WarnCode };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_strictly_increasing_rejects_what_it_must() {
        // The real table's check is a compile error, so the checker itself is exercised on the
        // inputs the table must never contain.
        assert!(numbers_strictly_increasing(&[]));
        assert!(numbers_strictly_increasing(&[2, 1501, 9218]));
        assert!(!numbers_strictly_increasing(&[2, 2]), "a duplicate must be rejected");
        assert!(!numbers_strictly_increasing(&[1501, 2]), "out of order must be rejected");
    }

    #[test]
    fn rate_policy_checker_rejects_a_non_power_of_two() {
        assert!(rate_policies_are_representable(&[RatePolicy::EveryN(1)]));
        assert!(rate_policies_are_representable(&[RatePolicy::EveryN(1024), RatePolicy::Once]));
        assert!(!rate_policies_are_representable(&[RatePolicy::EveryN(3)]));
        assert!(!rate_policies_are_representable(&[RatePolicy::EveryN(0)]), "0 is not a power of two");
    }

    #[test]
    fn the_table_is_dense_and_index_equals_code_idx() {
        // The rate array is indexed by `code_idx` with no map and no hash, so this equality IS
        // the addressing scheme. If it ever stops holding, every rate lookup reads another code's
        // slot -- silently, and only under load.
        for (i, row) in DIAGNOSTICS.iter().enumerate() {
            assert_eq!(
                code_idx_of(row.class, row.number),
                i as u32,
                "row {i} (class {}, number {}) does not resolve to its own index",
                row.class as char,
                row.number
            );
        }
    }

    #[test]
    fn every_pending_row_names_its_rung_and_the_live_set_is_pinned() {
        // At L2 this asserted that EVERY row was `Pending` -- the property that let that rung land
        // alone. L4 lands the first `Live` row, so the claim moves rather than loosens: a Pending
        // row must still name its rung, and the Live set is enumerated here so a row that goes
        // Live without its emitter, or an emitter that lands without flipping its row, reds.
        //
        // IT DID RED, and the miss is worth recording where it happened. Profiling rung 2 flipped
        // seven rows and was verified with `cargo test -p boyko-log --test code_registry` -- which
        // selects ONE integration target and does not build this lib test at all. The pin was red
        // for a whole rung. The lesson is not "run more tests": it is that a target filter is a
        // claim about coverage, and this project has a standing note that `--test <name>` and
        // `--lib` are different worlds.
        //
        // ⚠️ IT HAPPENED AGAIN AT RUNG 13, THE SAME WAY, TO SOMEONE WHO HAD READ THE PARAGRAPH
        // ABOVE. Three rows were flipped and verified with `cargo test -p boyko-log --test
        // code_registry`; all twelve checks in THAT target passed, and this pin -- five feet away
        // in `src/` -- was never built. It reds only in the full `--workspace --all-targets` sweep.
        //
        // The note is therefore not "remember the lesson". It is that A TARGET FILTER CANNOT BE
        // CHECKED BY THINKING ABOUT IT: the only thing that establishes what a filter covered is
        // running the unfiltered form. Every flip of a `CodeStatus` owes one `--workspace` sweep
        // before it is called verified, and no amount of care about the filter substitutes for it.
        const LIVE: &[(u8, u16)] = &[
            (b'B', 2),    // L6  -- intra-system access conflict on one resource
            (b'W', 103),  // L4  -- the file sink's byte cap
            (b'E', 106),  // L10 -- a dynamic target could not be registered (all three refusals)
            (b'E', 104),  // L11a -- two downstream targets claim one id
            (b'E', 107),  // L14  -- sink control request ring full
            (b'E', 109),  // L15  -- the crash file could not be opened
            (b'W', 111),  // L14  -- a target is armed but no active sink accepts it
            (b'W', 112),  // L13a -- rotation discarded part of the session
            (b'W', 117),  // L16  -- the ECS handoff refused frames this pass
            (b'E', 118),  // L15  -- the panic hook could not claim the drain role
            (b'W', 113),  // L12  -- sampling is discarding records
            (b'W', 116),  // L13b -- binary site dictionary full, sites written inline
            (b'W', 114),  // L11a -- downstream code index space at 90 %
            (b'E', 115),  // L11a -- downstream code index space exhausted
            (b'E', 201),  // L6  -- a fire-and-forget task panicked, process aborting
            (b'W', 501),  // L6  -- query-type table at 75 %
            (b'B', 502),  // L6  -- query-type table exhausted
            (b'W', 701),  // L6  -- an event lane was full, the send was refused
            (b'E', 801),  // L6  -- an asset failed to load
            (b'W', 901),  // L8a -- a dense store carried no decodable data
            (b'W', 1301), // L8a -- self-collision skipped, radius not positive
            (b'W', 1302), // L8a -- self-collision cell size exceeds the smallest rest length
            (b'W', 1303), // L8a -- self-collision spatial hash overloaded
            (b'W', 1501), // L6  -- ordering references an empty system set
            (b'B', 1801), // L8b -- a plugin was added more than once (boyko_ecs's, not the host's)
            (b'B', 1802), // L8b -- an App config method called after finish()
            (b'W', 1803), // the host's preset flag named nothing
            (b'E', 2001), // L8c -- a resource registry dropped with live resources (found by the census)
            (b'E', 2101), // L7a -- validation requested but not delivered
            (b'W', 2102), // L7b -- a device format feature is missing (three sites, one code)
            (b'E', 2103), // L7b -- a mandatory target/set failed to build
            (b'W', 2104), // L7b -- textured material suppressed by motion vectors
            (b'W', 2105), // L7b -- present mode not advertised, fell back to fifo
            (b'W', 2106), // L7b -- an optional shadow chain's sets failed to build
            (b'W', 2201), // L8a -- more lights than the GPU table holds
            (b'W', 2202), // L8a -- a bindless table exhausted its slots (two sites, one code)
            (b'E', 2203), // L8a -- a GPU dispatch failed with no error channel to report it
            (b'W', 2204), // L8a -- non-finite lights dropped from the GPU table
            (b'W', 2205), // L8a -- a render-path config changed after the consumer set froze
            (b'W', 2206), // L8a -- a material texture failed to decode
            (b'W', 2601), // L8a -- PNG chunk CRC-32 mismatch
            (b'W', 2602), // L8a -- zlib Adler-32 mismatch
            (b'E', 3001), // L8b -- boyko_demo could not start
            (b'E', 3002), // L8b -- a host boot stage failed (three sites, the stage is an argument)
            (b'E', 3003), // L8b -- a terminal device error in the frame loop
            (b'E', 3004), // L8b -- windowing unimplemented for this platform
            (b'W', 3005), // L8b -- the requested SSAA scale is unavailable (two sites)
            (b'W', 3006), // L8b -- the render path was degraded (once per REASON, hence `Every`)
            (b'W', 3007), // L8b -- the VB geometry table could not be created
            (b'W', 3008), // L8b -- a profiling knob the device cannot serve
            (b'W', 3009), // L8b -- an unrecognised environment override value (two sites)
            (b'E', 3010), // L8b -- a diagnostic dump could not be written (five sites)
            (b'B', 9001), // L6  -- schedule cycle
            (b'B', 9002), // L6  -- set-hierarchy cycle
            (b'B', 9004), // L6  -- two ordered sets share a member
            (b'B', 9005), // L6  -- ordering references an unknown system key
            (b'B', 9101), // L6  -- Schedule::run against a different world
            (b'W', 9201), // P3  -- engine zone registry exhausted
            (b'W', 9202), // L8c -- GPU timestamp pair budget exhausted (flag route)
            (b'W', 9203), // P2  -- region overflow / unclaimed drops
            (b'E', 9204), // P2  -- profiler already bound to another world
            (b'W', 9205), // L8c -- zones lost in this window (direct call, post-fold)
            (b'W', 9206), // L8c -- a contrast could not be resolved (direct call, post-fold)
            (b'W', 9207), // P2  -- invariant TSC absent
            (b'W', 9208), // P3  -- engine zone registry at 90 %
            (b'W', 9209), // P2  -- late samples dropped
            (b'W', 9210), // P10 -- user zone budget / dyn name arena exhausted
            (b'W', 9211), // P2  -- fold working set exceeds L1d
            (b'W', 9212), // P10 -- register_zone refused an engine scope
            (b'E', 9213), // P2  -- re-arm with a different geometry
            (b'W', 9214), // P13 -- telemetry path unwritable
            (b'W', 9215), // P13 -- telemetry write failed, streaming disabled
            (b'W', 9216), // P2  -- clock epoch break
            (b'W', 9217), // L8c -- GPU slots abandoned at teardown (direct call, post-fold)
            (b'W', 9218), // P13 -- telemetry quantile subscription refused past the cap
            // VB-SV0 DP6-0b. Both appended after the L2 block; `W9220` takes the DIRECT-call route
            // (its site is `boyko_app`'s window reducer, which depends on the emitter), `W9219` the
            // flag route (its site is `boyko_rhi_vulkan`, which cannot reach it).
            //
            // ⚠️ These two rows were added to the ECS-side prose and to `code_registry.rs`'s
            // corpus checks in the same commit as their emitters, and BOTH pins in THIS file were
            // missed — `cargo test -p boyko-log --test code_registry` passed 16/16 while
            // `--lib` reds. That is the documented target-selection blind spot: `--test <name>`
            // does not build `--lib`, so a green from one says nothing about the other.
            (b'W', 9219), // DP6-0b -- a GPU zone END with no matching BEGIN (flag route)
            (b'W', 9220), // DP6-0b -- a GPU zone's declared record order was violated (direct call)
        ];
        for row in DIAGNOSTICS {
            match row.status {
                CodeStatus::Pending(rung) => {
                    assert!(!rung.is_empty(), "a Pending row must name the rung that lands it");
                    assert!(
                        !LIVE.contains(&(row.class, row.number)),
                        "row {}{} is in the pinned Live set but its status is Pending",
                        row.class as char,
                        row.number
                    );
                }
                CodeStatus::Live => assert!(
                    LIVE.contains(&(row.class, row.number)),
                    "row {}{} went Live without being added to this test's pinned set",
                    row.class as char,
                    row.number
                ),
                other => panic!(
                    "row {}{} is {other:?}; only Pending and Live exist at this rung",
                    row.class as char, row.number
                ),
            }
        }
    }

    #[test]
    fn the_nine_grandfathered_codes_are_all_present() {
        // Measured against the tree: `boyko-[BEW][0-9]{4}` over `crates/**/*.rs` returns exactly
        // these nine distinct codes. Pinned so a later edit cannot quietly drop one -- a dropped
        // row makes its literal undeclared, which the walker's check 4 would then red on.
        for (class, number) in
            [(b'B', 2), (b'W', 1501), (b'B', 1801), (b'B', 1802), (b'B', 9001), (b'B', 9002),
             (b'B', 9004), (b'B', 9005), (b'B', 9101)]
        {
            assert!(
                explain(class, number).is_some(),
                "grandfathered code {}{number:04} is missing from the registry",
                class as char
            );
        }
    }

    #[test]
    fn b9003_is_a_permanent_gap() {
        // It exists only in `docs/archive/**`, which is outside the corpus. Seeding it as
        // `Pending` would promise a future emitter that will never arrive. Asserted so that a
        // later reader "completing the sequence" fails a test instead of shipping a lie.
        assert!(explain(b'B', 9003).is_none());
    }

    #[test]
    fn the_profiling_block_is_twenty_consecutive_rows() {
        // VB-SV0 DP6-0b took the block 18 -> 20 (`W9219`, `W9220`). The NAME carries the count for
        // the same reason the assertion does: a test called "...eighteen..." asserting twenty is a
        // row nobody can read, and the next reader has to open the body to learn which number is
        // current.
        //
        // ⚠️ This pin and the Live-set pin above both live in `--lib`, and the delta that added the
        // two rows was gated with `--test code_registry` alone — 16/16 green while these two were
        // red. `--test <name>` does not build `--lib`; a filter is an assertion about coverage and
        // it was wider than the filter.
        let block: Vec<u16> =
            DIAGNOSTICS.iter().filter(|r| (9201..=9299).contains(&r.number)).map(|r| r.number).collect();
        assert_eq!(block.len(), 20, "the 92xx reservation is twenty rows");
        for (i, n) in block.iter().enumerate() {
            assert_eq!(*n, 9201 + i as u16, "the block must be consecutive with no gaps");
        }
    }

    #[test]
    fn classes_are_distinct_types_so_a_mismatch_cannot_compile() {
        // The runtime half of a compile-time property: the numbers agree with the newtype, which
        // is what makes `warn!(…, B1802)` a type error rather than a wrong line in a log.
        assert_eq!(B1802.number(), 1802);
        assert_eq!(B1802.class(), b'B');
        assert_eq!(W1501.class(), b'W');
        assert_eq!(W9201.class(), b'W');
    }

    #[test]
    fn explain_resolves_a_known_code_and_refuses_an_unknown_one() {
        let info = explain(b'W', 1501).expect("W1501 is registered");
        assert_eq!(info.rate, RatePolicy::Once);
        assert!(!info.summary.is_empty());
        assert!(explain(b'W', 1802).is_none(), "1802 is a B code; the class must be part of the key");
        assert!(explain(b'E', 9999).is_none());
        assert_eq!(code_idx_of(b'E', 9999), CODE_IDX_EXHAUSTED);
    }

    #[test]
    fn a_once_latch_fires_exactly_once_and_then_only_loads() {
        let l = OnceSite::new();
        assert!(!l.has_fired());
        assert!(l.claim(), "the first claim wins");
        assert!(!l.claim(), "every later claim loses");
        assert!(!l.claim());
        assert!(l.has_fired());
    }

    #[test]
    fn concurrent_first_claims_deliver_exactly_one() {
        // `Once` promises delivery once, not delivery on a particular thread. A latch that let
        // two racing threads both win would double-log the first occurrence of every code the
        // engine reports from parallel systems.
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;

        let latch = Arc::new(OnceSite::new());
        let wins = Arc::new(AtomicUsize::new(0));
        let mut hs = Vec::new();
        for _ in 0..8 {
            let (l, w) = (Arc::clone(&latch), Arc::clone(&wins));
            hs.push(std::thread::spawn(move || {
                for _ in 0..1000 {
                    if l.claim() {
                        w.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }));
        }
        for h in hs {
            h.join().expect("claimant thread panicked");
        }
        assert_eq!(wins.load(Ordering::SeqCst), 1, "exactly one claim may win");
    }
}
