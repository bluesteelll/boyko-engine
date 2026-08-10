//! A dynamic descriptor arena spelled WITHOUT `MaybeUninit` must not compile.
//!
//! `ZoneDesc` holds a `&'static str`. All-zero bytes decode to a null reference, which is not a
//! valid `&str`, so `ZoneDesc: ZeroInit` is unimplementable and `SyncCells::zeroed()` refuses it.
//! Profiling rung 10's `DYN_DESCS` wraps the element in `MaybeUninit` for exactly this reason —
//! and this case is what stops the wrapper being deleted as ceremony.
//!
//! It must be a **static**, not a type alias: `SyncCells` carries no bound on the struct itself,
//! so `type Bad = SyncCells<ZoneDesc, 4>;` compiles and would be a gate that cannot fail
//! (`boyko_diag::storage::assert_zero_init_eligible`'s own doc says so).

use boyko_diag::profiling_abi::ZoneDesc;
use boyko_diag::storage::SyncCells;

static BAD_DESCS: SyncCells<ZoneDesc, 4> = SyncCells::zeroed();

fn main() {
    let _ = &BAD_DESCS;
}
