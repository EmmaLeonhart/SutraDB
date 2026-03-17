//! HNSW edge triple generation with labeled edge types.
//!
//! Exposes the internal HNSW neighbor connections as RDF triples so they
//! can be queried via SPARQL. The key design decision: HNSW edges are
//! labeled with **two distinct predicates** to encode the graph's layered
//! structure into the RDF model:
//!
//! ## Edge types
//!
//! **Horizontal edges** (`sutra:hnswHorizontalNeighbor`):
//! Connections between nodes on the **same HNSW layer**. These form the
//! "search graph" at each layer level. At layer 0 (the bottom), every
//! node has horizontal connections to its nearest neighbors. At higher
//! layers, only a sparse subset of nodes exist, forming a coarser graph.
//!
//! **Vertical edges** (`sutra:hnswLayerDescend`):
//! Connections from a node at layer L to the **same node** at layer L-1.
//! These encode the multi-layer descent that HNSW uses during search:
//! start at the top layer, greedily descend to the bottom. Every node
//! that exists on layer L also exists on all layers below it, so vertical
//! edges form a chain from a node's highest layer down to layer 0.
//!
//! ## Why label edges this way?
//!
//! The HNSW search algorithm is: "descend through layers (vertical), then
//! fan out among neighbors (horizontal)." By encoding this as two distinct
//! predicates, SPARQL property paths can naturally express the search:
//!
//! ```sparql
//! # Greedy descent followed by neighbor expansion:
//! ?entry sutra:hnswLayerDescend* / sutra:hnswHorizontalNeighbor+ ?result
//! ```
//!
//! This makes the HNSW topology navigable via standard SPARQL path expressions
//! without special-case executor code.
//!
//! ## Triple representation
//!
//! ```turtle
//! # Horizontal neighbor edge (same layer):
//! ?nodeA sutra:hnswHorizontalNeighbor ?nodeB .
//! << ?nodeA sutra:hnswHorizontalNeighbor ?nodeB >> sutra:hnswLayer 2 .
//! << ?nodeA sutra:hnswHorizontalNeighbor ?nodeB >> sutra:similarity 0.95 .
//!
//! # Vertical descent edge (layer L → layer L-1):
//! ?node sutra:hnswLayerDescend ?node .
//! << ?node sutra:hnswLayerDescend ?node >> sutra:hnswFromLayer 3 .
//! << ?node sutra:hnswLayerDescend ?node >> sutra:hnswToLayer 2 .
//! ```
//!
//! The generic `sutra:hnswNeighbor` predicate is preserved for backward
//! compatibility — it matches ALL edges regardless of type.

use sutra_core::TermId;

use crate::index::HnswIndex;

// ---------------------------------------------------------------------------
// Well-known predicate IRIs for HNSW edge triples
// ---------------------------------------------------------------------------

/// Generic neighbor predicate — matches both horizontal and vertical edges.
/// Preserved for backward compatibility with existing queries.
pub const HNSW_NEIGHBOR_IRI: &str = "http://sutra.dev/hnswNeighbor";

/// Horizontal neighbor edge: two nodes connected on the same HNSW layer.
/// This is the "fan out" operation in HNSW search — exploring neighbors
/// at a given layer level to find closer candidates.
pub const HNSW_HORIZONTAL_NEIGHBOR_IRI: &str = "http://sutra.dev/hnswHorizontalNeighbor";

/// Vertical descent edge: a node transitioning from a higher layer to a
/// lower layer. Source and target are the same node, but at different
/// layers. This is the "descend" operation in HNSW search — moving to
/// a finer-grained layer where more neighbors are available.
pub const HNSW_LAYER_DESCEND_IRI: &str = "http://sutra.dev/hnswLayerDescend";

/// Layer metadata predicate (used in RDF-star annotations).
pub const HNSW_LAYER_IRI: &str = "http://sutra.dev/hnswLayer";

/// Similarity score metadata predicate (used in RDF-star annotations).
pub const HNSW_SIMILARITY_IRI: &str = "http://sutra.dev/hnswSimilarity";

/// Source predicate metadata (which vector predicate this edge belongs to).
pub const HNSW_PREDICATE_IRI: &str = "http://sutra.dev/hnswPredicate";

// ---------------------------------------------------------------------------
// Edge type classification
// ---------------------------------------------------------------------------

/// The type of an HNSW edge, determined by its role in the search algorithm.
///
/// This classification enables SPARQL property paths to express HNSW search
/// semantics: descend vertically through layers, then fan out horizontally
/// among neighbors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HnswEdgeType {
    /// Horizontal: connects two different nodes on the same HNSW layer.
    /// This is the standard neighbor connection used for beam search at
    /// a given layer level.
    Horizontal,

    /// Vertical descent: connects a node at layer L to itself at layer L-1.
    /// This encodes the multi-layer structure of HNSW, where search starts
    /// at the top layer and descends to the bottom for finer-grained results.
    VerticalDescend,
}

// ---------------------------------------------------------------------------
// Edge triple data structure
// ---------------------------------------------------------------------------

/// A single HNSW edge exposed as triple components.
///
/// Represents either:
/// - Horizontal: `source sutra:hnswHorizontalNeighbor target` (different nodes, same layer)
/// - Vertical: `node sutra:hnswLayerDescend node` (same node, adjacent layers)
///
/// The `edge_type` field determines which predicate IRI to use when
/// materializing this edge as an RDF triple.
#[derive(Debug, Clone, PartialEq)]
pub struct HnswEdgeTriple {
    /// The triple_id of the source node in the HNSW graph.
    pub source: TermId,
    /// The triple_id of the target (neighbor) node.
    /// For vertical descent edges, target == source.
    pub target: TermId,
    /// Which HNSW layer this edge exists on.
    /// For vertical edges, this is the "from" layer (higher layer).
    pub layer: u8,
    /// Similarity score between source and target vectors.
    /// For vertical edges, this is 1.0 (same vector).
    pub similarity: f32,
    /// The type of this edge (horizontal neighbor or vertical descent).
    /// Determines which predicate IRI to use in the RDF representation.
    pub edge_type: HnswEdgeType,
}

impl HnswEdgeTriple {
    /// Get the predicate IRI for this edge based on its type.
    ///
    /// Returns the specific typed predicate, not the generic hnswNeighbor.
    /// Use HNSW_NEIGHBOR_IRI for backward-compatible queries that match all edges.
    pub fn predicate_iri(&self) -> &'static str {
        match self.edge_type {
            HnswEdgeType::Horizontal => HNSW_HORIZONTAL_NEIGHBOR_IRI,
            HnswEdgeType::VerticalDescend => HNSW_LAYER_DESCEND_IRI,
        }
    }
}

// ---------------------------------------------------------------------------
// Edge generation
// ---------------------------------------------------------------------------

impl HnswIndex {
    /// Generate all HNSW edges as triple-like structures.
    ///
    /// This is the core API for virtual edge exposure. Produces two types:
    ///
    /// 1. **Horizontal edges**: Each neighbor connection in every layer becomes
    ///    an `HnswEdgeTriple` with `edge_type = Horizontal`.
    ///
    /// 2. **Vertical descent edges**: For each node that exists on layer L > 0,
    ///    a descent edge from layer L to layer L-1 is generated with
    ///    `edge_type = VerticalDescend` and `source == target`.
    ///
    /// Skips deleted nodes and their edges.
    pub fn edge_triples(&self) -> Vec<HnswEdgeTriple> {
        let mut edges = Vec::new();

        for node in self.nodes.iter() {
            if node.deleted {
                continue;
            }

            // Generate horizontal edges (node → neighbor, same layer).
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
                        edge_type: HnswEdgeType::Horizontal,
                    });
                }
            }

            // Generate vertical descent edges (layer L → layer L-1).
            // A node on layer L also exists on all layers below it.
            // Each descent step is a separate edge for property path traversal.
            if node.layer > 0 {
                for from_layer in (1..=node.layer).rev() {
                    edges.push(HnswEdgeTriple {
                        source: node.triple_id,
                        target: node.triple_id, // same node, different layer
                        layer: from_layer,
                        similarity: 1.0, // self-similarity
                        edge_type: HnswEdgeType::VerticalDescend,
                    });
                }
            }
        }

        edges
    }

    /// Generate HNSW edges for a specific source node (by triple_id).
    ///
    /// Returns both horizontal edges (to neighbors) and vertical descent
    /// edges (to lower layers) from this node.
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

        // Horizontal edges: this node's neighbors at each layer.
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
                    edge_type: HnswEdgeType::Horizontal,
                });
            }
        }

        // Vertical descent edges: from each layer down to the next.
        if node.layer > 0 {
            for from_layer in (1..=node.layer).rev() {
                edges.push(HnswEdgeTriple {
                    source: source_triple_id,
                    target: source_triple_id,
                    layer: from_layer,
                    similarity: 1.0,
                    edge_type: HnswEdgeType::VerticalDescend,
                });
            }
        }

        edges
    }

    /// Generate HNSW edges targeting a specific node (by triple_id).
    ///
    /// This is the reverse lookup: find all nodes that have this node as a neighbor
    /// (horizontal edges), plus any vertical descent edges landing on this node.
    /// More expensive than `edge_triples_for_source` since it must scan all nodes
    /// for horizontal reverse lookups.
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

        // Horizontal reverse lookup: find all nodes that have this node as neighbor.
        for node in self.nodes.iter() {
            if node.deleted {
                continue;
            }

            for (layer, neighbors) in node.neighbors.iter().enumerate() {
                if neighbors.contains(&(target_node_idx as u32)) {
                    let similarity = self.config.metric.score(&node.vector, &target_node.vector);

                    edges.push(HnswEdgeTriple {
                        source: node.triple_id,
                        target: target_triple_id,
                        layer: layer as u8,
                        similarity,
                        edge_type: HnswEdgeType::Horizontal,
                    });
                }
            }
        }

        // Vertical descent edges landing on this node: these are self-edges
        // where source == target, so they appear in both source and target queries.
        if target_node.layer > 0 {
            for from_layer in (1..=target_node.layer).rev() {
                edges.push(HnswEdgeTriple {
                    source: target_triple_id,
                    target: target_triple_id,
                    layer: from_layer,
                    similarity: 1.0,
                    edge_type: HnswEdgeType::VerticalDescend,
                });
            }
        }

        edges
    }

    /// Count total number of edges (non-deleted) across all layers.
    ///
    /// Includes both horizontal neighbor edges and vertical descent edges.
    pub fn edge_count(&self) -> usize {
        let mut count = 0usize;
        for node in &self.nodes {
            if node.deleted {
                continue;
            }
            // Count horizontal edges.
            for layer_neighbors in &node.neighbors {
                for &neighbor_idx in layer_neighbors {
                    if !self.nodes[neighbor_idx as usize].deleted {
                        count += 1;
                    }
                }
            }
            // Count vertical descent edges.
            if node.layer > 0 {
                count += node.layer as usize;
            }
        }
        count
    }

    /// Count only horizontal neighbor edges (non-deleted).
    ///
    /// This matches the pre-v0.2 edge_count behavior for backward compatibility.
    pub fn horizontal_edge_count(&self) -> usize {
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

    /// Generate only horizontal edges (neighbor-to-neighbor, same layer).
    ///
    /// Useful for analysis that only cares about the neighbor graph structure,
    /// not the layer hierarchy.
    pub fn horizontal_edge_triples(&self) -> Vec<HnswEdgeTriple> {
        self.edge_triples()
            .into_iter()
            .filter(|e| e.edge_type == HnswEdgeType::Horizontal)
            .collect()
    }

    /// Generate only vertical descent edges (layer transitions).
    ///
    /// Useful for visualizing or analyzing the multi-layer structure of the
    /// HNSW graph — which nodes appear at which layers, and how many layers
    /// the graph has.
    pub fn vertical_edge_triples(&self) -> Vec<HnswEdgeTriple> {
        self.edge_triples()
            .into_iter()
            .filter(|e| e.edge_type == HnswEdgeType::VerticalDescend)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

        // Should have some edges (bidirectional connections + vertical)
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
        let edge_to_104 = edges_from_100
            .iter()
            .find(|e| e.target == 104 && e.edge_type == HnswEdgeType::Horizontal);
        if let Some(edge) = edge_to_104 {
            assert!(
                edge.similarity < 0.5,
                "Expected low similarity for orthogonal vectors, got {}",
                edge.similarity
            );
        }
    }

    // -----------------------------------------------------------------------
    // Edge type classification tests
    // -----------------------------------------------------------------------

    #[test]
    fn horizontal_edges_have_different_source_and_target() {
        let index = make_index_with_edges();
        let horizontal = index.horizontal_edge_triples();

        // All horizontal edges connect two DIFFERENT nodes
        for edge in &horizontal {
            assert_ne!(
                edge.source, edge.target,
                "Horizontal edge should connect different nodes"
            );
            assert_eq!(edge.edge_type, HnswEdgeType::Horizontal);
        }
    }

    #[test]
    fn vertical_edges_have_same_source_and_target() {
        let index = make_index_with_edges();
        let vertical = index.vertical_edge_triples();

        // All vertical edges are self-loops (same node, different layer)
        for edge in &vertical {
            assert_eq!(
                edge.source, edge.target,
                "Vertical edge should be a self-loop (same node)"
            );
            assert_eq!(edge.edge_type, HnswEdgeType::VerticalDescend);
            assert!(edge.layer > 0, "Vertical edge should be from layer > 0");
            assert!(
                (edge.similarity - 1.0).abs() < f32::EPSILON,
                "Vertical edge should have similarity 1.0"
            );
        }
    }

    #[test]
    fn edge_count_includes_both_types() {
        let index = make_index_with_edges();
        let horizontal_count = index.horizontal_edge_count();
        let vertical_count = index.vertical_edge_triples().len();
        let total_count = index.edge_count();

        assert_eq!(
            total_count,
            horizontal_count + vertical_count,
            "Total edge count should be horizontal + vertical"
        );
    }

    #[test]
    fn predicate_iri_matches_edge_type() {
        let index = make_index_with_edges();
        let edges = index.edge_triples();

        for edge in &edges {
            match edge.edge_type {
                HnswEdgeType::Horizontal => {
                    assert_eq!(edge.predicate_iri(), HNSW_HORIZONTAL_NEIGHBOR_IRI);
                }
                HnswEdgeType::VerticalDescend => {
                    assert_eq!(edge.predicate_iri(), HNSW_LAYER_DESCEND_IRI);
                }
            }
        }
    }
}
