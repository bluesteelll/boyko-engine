
mod sparse_collection;
// The submodule shares the name of its parent module; this is intentional
// for API clarity (users import `sparse_map::SparseMap`).
#[allow(clippy::module_inception)]
pub mod sparse_map;
pub mod sparse_slot_map;
