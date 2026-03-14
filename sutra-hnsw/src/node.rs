//! HNSW node representation.
//!
//! Each node in the HNSW graph stores a vector, its layer assignment,
//! neighbor lists per layer, and a back-reference to the originating triple.

use sutra_core::TermId;

/// A single node in the HNSW graph.
pub struct HnswNode {
    /// The vector embedding.
    pub vector: Vec<f32>,
    /// The maximum layer this node appears in (0 = bottom layer only).
    pub layer: u8,
    /// Neighbor lists per layer. `neighbors[l]` contains the node indices
    /// of this node's neighbors at layer `l`, bounded by M (or 2*M for layer 0).
    pub neighbors: Vec<Vec<u32>>,
    /// Back-reference to the triple store: which triple does this vector belong to.
    pub triple_id: TermId,
    /// Lazy deletion flag. Deleted nodes are skipped during search but not
    /// removed from the graph structure until compaction.
    pub deleted: bool,
}

impl HnswNode {
    /// Create a new node at the given layer with an empty neighbor list.
    pub fn new(vector: Vec<f32>, layer: u8, triple_id: TermId) -> Self {
        let neighbors = (0..=layer).map(|_| Vec::new()).collect();
        Self {
            vector,
            layer,
            neighbors,
            triple_id,
            deleted: false,
        }
    }

    /// The dimensionality of this node's vector.
    pub fn dimensions(&self) -> usize {
        self.vector.len()
    }
}
