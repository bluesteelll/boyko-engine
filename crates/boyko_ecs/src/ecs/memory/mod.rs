// `vm`, `vm_column` and `utils` live in `boyko_memory` since rung C1 (unified plan KC-01); these
// re-exports keep every `crate::ecs::memory::…` path compiling, each at its old visibility.
pub use boyko_memory::utils;
pub mod component_pool;
pub mod device_column;
pub(crate) use boyko_memory::vm;
pub(crate) use boyko_memory::vm_column;
