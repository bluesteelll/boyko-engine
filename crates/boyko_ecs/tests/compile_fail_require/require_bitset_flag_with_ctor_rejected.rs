// KE11 (ballot AB-6 arm (a)) — the refusal covers the `C = expr` entry form too,
// not only the bare path.
//
// The sibling fixture `require_bitset_flag_rejected.rs` pins the bare-path arm
// (`#[require(FlagDep)]` ⇒ `FlagDep::default()`). This one pins the explicit-ctor
// arm (`#[require(FlagDep = FlagDep)]`), which is the form an author reaches for
// after the `Default` route is refused — and it must be refused for the SAME
// reason, since supplying a value changes nothing about a flag having no bytes
// to store. The refusal therefore hangs off the parsed entry's TYPE path, which
// both arms share, rather than off the bare-path arm's `Default` lowering.
//
// The error is spanned at the left-hand type path of the assignment.
//
// Expected diagnostic: E0080 "evaluation panicked: #[require(...)] of a bitset
// flag: ...".

use boyko_macros::Component;

/// A bitset flag: no bytes to construct.
#[derive(Component, Default)]
#[component(storage = "bitset")]
struct FlagDep;

#[derive(Component)]
#[require(FlagDep = FlagDep)]
#[repr(C)]
struct Bad(u32);

fn main() {}
