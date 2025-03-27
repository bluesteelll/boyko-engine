use crate::ecs::identifiers::primitives::{ChunkId, InlandChunkId};

/// Struct for indexing components within a chunk-based storage system
/// Represents a two-level addressing scheme for component access
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitId {
    /// Index of the chunk containing the component
    pub id_chunk: ChunkId,

    /// Index of the component within the chunk
    pub id_inland: InlandChunkId,
}

impl UnitId {
    /// Creates a new component index with the specified chunk and inland indices
    ///
    /// # Parameters
    /// * `id_chunk` - The index of the chunk
    /// * `id_inland` - The index of the component within the chunk
    #[inline]
    pub fn new(id_chunk: ChunkId, id_inland: InlandChunkId) -> Self {
        Self {
            id_chunk,
            id_inland,
        }
    }

    /// Returns the chunk index as a usize
    #[inline]
    pub fn chunk_index(&self) -> ChunkId {
        self.id_chunk
    }

    /// Returns the inland index as a usize
    #[inline]
    pub fn inland_index(&self) -> InlandChunkId {
        self.id_inland
    }
}