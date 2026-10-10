// Profiling rung 11, `G12` clause 3 (the compile half) — B2's refuted design, named.
//
// Rev 3 of the profiling plan used ONE component for both roles: `ProfilingScope { bit, name }` as
// the kernel enable tag. The struct below is that type verbatim. It must not compile.
//
// A general "a fielded bitset tag is rejected" fixture already lives beside this one
// (`storage_bitset_with_fields_rejected.rs`). This one is not a duplicate of it: that fixture
// proves the MECHANISM, this one proves the INSTANCE — the specific shape a shipped design
// proposed, so a future revision that reaches for it again meets a gate that says its name rather
// than a gate about bitset tags in general.
//
// The compile error is the visible half of B2 and the less important one. The invisible half is
// what `g12_a_table_storage_id_forced_through_the_enable_path_projects_zero` measures: the READ
// path (`enable_tag_api.rs:201-215`) carries no storage-kind assert, so forcing the id through
// does not panic — it answers `false` for every entity and projects an all-zero mask, in every
// build, silently.

use boyko_macros::Component;

#[derive(Component)]
#[component(storage = "bitset")]
struct ProfilingScope {
    bit: u8,
    name: &'static str,
}

fn main() {}
