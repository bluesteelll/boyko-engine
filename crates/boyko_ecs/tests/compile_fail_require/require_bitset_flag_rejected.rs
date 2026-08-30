// KE11 (ballot AB-6 arm (a)) — `#[require(<a bitset flag>)]` is a COMPILE error.
//
// `RequiredCtor` is an `unsafe fn(*mut u8)` whose only job is to materialize
// BYTES into an uninitialized storage slot. A `storage = "bitset"` flag has no
// bytes, so there is nothing for a ctor to write: the construct is meaningless,
// not merely unimplemented. The capability the author wants ("attaching X sets
// flag F") already exists under its own name — the component `flags (...)`
// group, backed by the `FLAGS_DIRECT` table — and the message says so.
//
// The derive cannot resolve `FlagDep` to a `StorageKind` (it holds a token, not
// a type), so the refusal is emitted as a `const _: () = assert!(...)` on the
// `Component::STORAGE_IS_BITSET` trait const and evaluated by the COMPILER. It
// is spanned at the offending entry inside `#[require(...)]`, which is what the
// three-entry list below pins.
//
// # This fixture is also the "dense is NOT refused" control
//
// `Bad` requires THREE components: a table one, a DENSE one, and the flag.
// trybuild matches the `.stderr` exactly, so the baseline showing exactly ONE
// error — the flag's — is the standing proof that neither the table entry nor
// the dense entry is caught by this refusal. A refusal that over-fired onto
// `DenseDep` would break the kernel-side KE11 landing (which makes
// `#[require(<dense>)]` actually work); this control makes that regression
// impossible to land silently.
//
// Expected diagnostic: E0080 "evaluation panicked: #[require(...)] of a bitset
// flag: ...", pointing at `FlagDep` in the `#[require(...)]` list.

use boyko_macros::Component;

/// Ordinary table storage — must NOT be refused.
#[derive(Component, Default)]
#[repr(C)]
struct TableDep(u32);

/// Dense storage: has bytes, merely kept in a `DenseStore` instead of an
/// archetype column — must NOT be refused.
#[derive(Component, Default)]
#[component(storage = "dense")]
#[repr(C)]
struct DenseDep(u32);

/// A bitset flag: no bytes at all — the one entry that must be refused.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct FlagDep;

#[derive(Component)]
#[require(TableDep, DenseDep, FlagDep)]
#[repr(C)]
struct Bad(u32);

fn main() {}
