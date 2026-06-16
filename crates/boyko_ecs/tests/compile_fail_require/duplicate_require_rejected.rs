// Feature 1 — a duplicate same-id `#[require(B, B)]` is a compile error (each
// required component may be listed at most once). The derive sees both `B`
// paths and rejects at compile time — strictly better than Bevy's runtime panic.
//
// Expected diagnostic: "duplicate #[require(...)] for the same component; each
// required component may be listed at most once".

use boyko_macros::Component;

#[derive(Component, Default)]
#[repr(C)]
struct Dep(u32);

#[derive(Component)]
#[require(Dep, Dep)]
#[repr(C)]
struct Bad(u32);

fn main() {}
