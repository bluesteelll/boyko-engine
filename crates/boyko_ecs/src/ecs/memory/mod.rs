// crates/boyko_ecs/src/ecs/memory/mod.rs

//! Упрощенный модуль управления памятью для Boyko ECS

pub mod arena;
pub mod chunk;
pub mod component_pool;
pub mod utils;

// Реэкспорт основных типов
pub use arena::Arena;
pub use chunk::Chunk;
pub use component_pool::{ComponentPool, ComponentLocation};