//! HNSW edge triple generation.
//!
//! Exposes the internal HNSW neighbor connections as RDF triples so they
//! can be queried via SPARQL. Each neighbor connection becomes:
//!
//! ```turtle
//! ?nodeA sutra:hnswNeighbor ?nodeB .
//! ```
//!
//! With RDF-star metadata:
//!
//! ```turtle
//! << ?nodeA sutra:hnswNeighbor ?nodeB >> sutra:hnswLayer 0 .
//! << ?nodeA sutra:hnswNeighbor ?nodeB >> sutra:similarity 0.95 .
//! ```
//!
//! These triples can be virtual (generated on-the-fly) or materialized
//! (stored in the triple store).

use sutra_core::TermId;

use crate::index::HnswIndex;

/// Well-known predicate IRIs for HNSW edge triples.
pub const HNSW_NEIGHBOR_IRI: &str = "http://sutra.dev/hnswNeighbor";
pub const HNSW_LAYER_IRI: &str = "http://sutra.dev/hnswLayer";
pub const HNSW_SIMILARITY_IRI: &str = "http://sutra.dev/hnswSimilarity";
pub const HNSW_PREDICATE_IRI: &str = "http://sutra.dev/hnswPredicate";

/// A single HNSW edge exposed as triple components.
///
/// Represents: `source_triple_id sutra:hnswNeighbor target_triple_id`
/// with metadata: layer and similarity score.
#[derive(Debug, Clone, PartialEq)]
pub struct HnswEdgeTriple {
    /// The triple_id of the source node in the HNSW graph.
    pub source: TermId,
    /// The triple_id of the target (neighbor) node.
    pub target: TermId,
    /// Which HNSW layer this edge exists on.
    pub layer: u8,
    /// Similarity score between source and target vectors.
    pub similarity: f32,
}

impl HnswIndex {
    /// Generate all HNSW edges as triple-like structures.
    ///
    /// This is the core API for virtual edge exposure. Each neighbor
    /// connection in every layer becomes an `HnswEdgeTriple`.
    ///
    /// Skips deleted nodes and their edges.
    pub fn edge_triples(&self) -> Vec<HnswEdgeTriple> {
        let mut edges = Vec::new();

        for (_node_idx, node) in self.nodes.iter().enumerate() {
            if node.deleted {
                continue;
            }

            for (layer, neighbors) in node.neighbors.iter().enumerate() {
                for &neighbor_idx in neighbors {
                    let neighbor = &self.nodes[neighbor_idx as usize];
                    if neighbor.deleted {
                        continue;
                    }

                    let similarity = self.config.metric.score(&node.vector, &neighbor.vector);

                    edges.push(HnswEdgeTriple {
                        source: node.triple_id,
                        target: neighbor.triple_id,
                        layer: layer as u8,
                        similarity,
                    });
                }
            }
        }

        edges
    }

    /// Generate HNSW edges for a specific source node (by triple_id).
    ///
    /// Returns edges only from this node to its neighbors.
    pub fn edge_triples_for_source(&self, source_triple_id: TermId) -> Vec<HnswEdgeTriple> {
        let node_idx = match self.triple_to_node.get(&source_triple_id) {
            Some(&idx) => idx,
            None => return Vec::new(),
        };

        let node = &self.nodes[node_idx as usize];
        if node.deleted {
            return Vec::new();
        }

        let mut edges = Vec::new();
        for (layer, neighbors) in node.neighbors.iter().enumerate() {
            for &neighbor_idx in neighbors {
                let neighbor = &self.nodes[neighbor_idx as usize];
                if neighbor.deleted {
                    continue;
                }

                let similarity = self.config.metric.score(&node.vector, &neighbor.vector);

                edges.push(HnswEdgeTriple {
                    source: source_triple_id,
                    target: neighbor.triple_id,
                    layer: layer as u8,
                    similarity,
                });
            }
        }

        edges
    }

    /// Generate HNSW edges targeting a specific node (by triple_id).
    ///
    /// This is the reverse lookup: find all nodes that have this node as a neighbor.
    /// More expensive than `edge_triples_for_source` since it must scan all nodes.
    pub fn edge_triples_for_target(&self, target_triple_id: TermId) -> Vec<HnswEdgeTriple> {
        let target_node_idx = match self.triple_to_node.get(&target_triple_id) {
            Some(&idx) => idx,
            None => return Vec::new(),
        };

        let target_node = &self.nodes[target_node_idx as usize];
        if target_node.deleted {
            return Vec::new();
        }

        let mut edges = Vec::new();
        for (_node_idx, node) in self.nodes.iter().enumerate() {
            if node.deleted {
                continue;
            }

            for (layer, neighbors) in node.neighbors.iter().enumerate() {
                let target_u32 = target_node_idx as u32;
                if neighbors.contains(&target_u32) {
                    let similarity = self.config.metric.score(&node.vector, &target_node.vector);

                    edges.push(HnswEdgeTriple {
                        source: node.triple_id,
                        target: target_triple_id,
                        layer: layer as u8,
                        similarity,
                    });
                }
            }
        }

        edges
    }

    /// Count total number of edges (non-deleted) across all layers.
    pub fn edge_count(&self) -> usize {
        let mut count = 0usize;
        for node in &self.nodes {
            if node.deleted {
                continue;
            }
            for layer_neighbors in &node.neighbors {
                for &neighbor_idx in layer_neighbors {
                    if !self.nodes[neighbor_idx as usize].deleted {
                        count += 1;
                    }
                }
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::HnswConfig;

    fn make_index_with_edges() -> HnswIndex {
        let mut index = HnswIndex::with_seed(HnswConfig::new(4, 20, 3), 42);

        // Insert 5 vectors that form a clear neighborhood structure
        index.insert(vec![1.0, 0.0, 0.0], 100).unwrap(); // x-axis
        index.insert(vec![0.9, 0.1, 0.0], 101).unwrap(); // near x-axis
        index.insert(vec![0.0, 1.0, 0.0], 102).unwrap(); // y-axis
        index.insert(vec![0.0, 0.9, 0.1], 103).unwrap(); // near y-axis
        index.insert(vec![0.0, 0.0, 1.0], 104).unwrap(); // z-axis

        index
    }

    #[test]
    fn edge_triples_generated() {
        let index = make_index_with_edges();
        let edges = index.edge_triples();

        // Should have some edges (bidirectional connections)
        assert!(!edges.is_empty());

        // All edges should reference valid triple IDs
        let valid_ids = [100u64, 101, 102, 103, 104];
        for edge in &edges {
            assert!(valid_ids.contains(&edge.source));
            assert!(valid_ids.contains(&edge.target));
            assert!(edge.similarity >= -1.0 && edge.similarity <= 1.0);
        }
    }

    #[test]
    fn edge_triples_skip_deleted() {
        let mut index = make_index_with_edges();
        let edges_before = index.edge_triples();

        index.delete(100);
        let edges_after = index.edge_triples();

        // Should have fewer edges after deletion
        assert!(edges_after.len() < edges_before.len());

        // No edges should reference the deleted node
        for edge in &edges_after {
            assert_ne!(edge.source, 100);
            assert_ne!(edge.target, 100);
        }
    }

    #[test]
    fn edge_triples_for_source() {
        let index = make_index_with_edges();
        let edges = index.edge_triples_for_source(100);

        // All edges should have source = 100
        for edge in &edges {
            assert_eq!(edge.source, 100);
        }
    }

    #[test]
    fn edge_triples_for_source_nonexistent() {
        let index = make_index_with_edges();
        let edges = index.edge_triples_for_source(999);
        assert!(edges.is_empty());
    }

    #[test]
    fn edge_triples_for_target() {
        let index = make_index_with_edges();
        let edges = index.edge_triples_for_target(100);

        // All edges should have target = 100
        for edge in &edges {
            assert_eq!(edge.target, 100);
        }
    }

    #[test]
    fn edge_count() {
        let index = make_index_with_edges();
        let count = index.edge_count();
        let edges = index.edge_triples();

        assert_eq!(count, edges.len());
    }

    #[test]
    fn edge_similarity_values() {
        let index = make_index_with_edges();
        let edges_from_100 = index.edge_triples_for_source(100);

        // Find the edge to 101 (near x-axis) — should have high similarity
        let edge_to_101 = edges_from_100.iter().find(|e| e.target == 101);
        if let Some(edge) = edge_to_101 {
            assert!(
                edge.similarity > 0.9,
                "Expected high similarity for near-parallel vectors, got {}",
                edge.similarity
            );
        }

        // Find the edge to 104 (z-axis) — should have low similarity
        let edge_to_104 = edges_from_100.iter().find(|e| e.target == 104);
        if let Some(edge) = edge_to_104 {
            assert!(
                edge.similarity < 0.5,
                "Expected low similarity for orthogonal vectors, got {}",
                edge.similarity
            );
        }
    }
}
