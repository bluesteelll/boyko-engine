/// Struct for indexing components within a chunk-based storage system
/// Represents a two-level addressing scheme for component access
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentIndex {
    /// Index of the chunk containing the component
    pub id_chunk: u32,

    /// Index of the component within the chunk
    pub id_inland: u32,
}

impl ComponentIndex {
    /// Creates a new component index with the specified chunk and inland indices
    ///
    /// # Parameters
    /// * `id_chunk` - The index of the chunk
    /// * `id_inland` - The index of the component within the chunk
    #[inline]
    pub fn new(id_chunk: usize, id_inland: usize) -> Self {
        Self {
            id_chunk: id_chunk as u32,
            id_inland: id_inland as u32,
        }
    }

    /// Returns the chunk index as a usize
    #[inline]
    pub fn chunk_index(&self) -> usize {
        self.id_chunk as usize
    }

    /// Returns the inland index as a usize
    #[inline]
    pub fn inland_index(&self) -> usize {
        self.id_inland as usize
    }
}