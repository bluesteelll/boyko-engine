// crates/boyko_ecs/src/ecs/memory/mod.rs

//! Упрощенный модуль управления памятью для Boyko ECS

pub mod arena;

pub mod utils;
mod free_mem_block;
pub mod chunk;
pub mod component_pool;
mod free_chunk_master;
mod component_index;
// Реэкспорт основных типов

