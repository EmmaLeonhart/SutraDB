//! HNSW index implementation.
//!
//! This is the core approximate nearest neighbor index. One index exists
//! per vector predicate (e.g. `:hasEmbedding`). The index is keyed by
//! triple ID, not by a standalone vector ID.

use std::collections::BinaryHeap;
use std::cmp::Reverse;

use sutra_core::TermId;

use crate::error::{HnswError, Result};
use crate::node::HnswNode;
use crate::vector::cosine_similarity;

/// A search result: (similarity score, triple ID).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub score: f32,
    pub triple_id: TermId,
}

/// Configuration for an HNSW index.
#[derive(Debug, Clone)]
pub struct HnswConfig {
    /// Maximum connections per node per layer.
    pub m: usize,
    /// Maximum connections for layer 0 (typically 2*M).
    pub m0: usize,
    /// Beam width during index construction.
    pub ef_construction: usize,
    /// Fixed vector dimensionality.
    pub dimensions: usize,
}

impl HnswConfig {
    /// Create a config with the given parameters.
    pub fn new(m: usize, ef_construction: usize, dimensions: usize) -> Self {
        Self {
            m,
            m0: m * 2,
            ef_construction,
            dimensions,
        }
    }
}

/// An HNSW index for a single vector predicate.
pub struct HnswIndex {
    config: HnswConfig,
    /// All nodes in insertion order. Node index = position in this vec.
    nodes: Vec<HnswNode>,
    /// Index of the entry point node (top of the graph).
    entry_point: Option<u32>,
    /// Maximum layer currently in the graph.
    max_layer: u8,
    /// Scaling factor for random layer assignment: 1 / ln(M).
    ml: f64,
}

impl HnswIndex {
    /// Create a new empty HNSW index with the given configuration.
    pub fn new(config: HnswConfig) -> Self {
        let ml = 1.0 / (config.m as f64).ln();
        Self {
            config,
            nodes: Vec::new(),
            entry_point: None,
            max_layer: 0,
            ml,
        }
    }

    /// The configured dimensionality.
    pub fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    /// Number of nodes (including deleted).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Assign a random layer for a new node using the geometric distribution.
    fn random_layer(&self) -> u8 {
        let r: f64 = rand_f64();
        let l = (-r.ln() * self.ml).floor() as u8;
        l
    }

    /// Insert a vector into the index, associated with the given triple ID.
    pub fn insert(&mut self, vector: Vec<f32>, triple_id: TermId) -> Result<()> {
        if vector.len() != self.config.dimensions {
            return Err(HnswError::DimensionMismatch {
                expected: self.config.dimensions,
                got: vector.len(),
            });
        }

        let new_layer = self.random_layer();
        let new_node = HnswNode::new(vector, new_layer, triple_id);
        let new_idx = self.nodes.len() as u32;
        self.nodes.push(new_node);

        // First node becomes the entry point
        if self.entry_point.is_none() {
            self.entry_point = Some(new_idx);
            self.max_layer = new_layer;
            return Ok(());
        }

        let ep = self.entry_point.unwrap();
        let mut current_ep = ep;

        // Phase 1: Greedily descend from top layer to new_layer + 1
        for layer in (new_layer as usize + 1..=self.max_layer as usize).rev() {
            current_ep = self.greedy_closest(new_idx, current_ep, layer as u8);
        }

        // Phase 2: Insert at layers new_layer down to 0
        let ef = self.config.ef_construction;
        for layer in (0..=std::cmp::min(new_layer, self.max_layer) as usize).rev() {
            let candidates = self.search_layer(new_idx, current_ep, ef, layer as u8);
            let max_conn = if layer == 0 { self.config.m0 } else { self.config.m };
            let neighbors = self.select_neighbors(&candidates, max_conn);

            // Set this node's neighbors at this layer
            if layer < self.nodes[new_idx as usize].neighbors.len() {
                self.nodes[new_idx as usize].neighbors[layer] = neighbors.clone();
            }

            // Add bidirectional connections
            for &neighbor_idx in &neighbors {
                let neighbor = &mut self.nodes[neighbor_idx as usize];
                if layer < neighbor.neighbors.len() {
                    neighbor.neighbors[layer].push(new_idx);
                    // Trim if over capacity
                    if neighbor.neighbors[layer].len() > max_conn {
                        let query_vec = self.nodes[new_idx as usize].vector.clone();
                        self.shrink_connections(neighbor_idx, layer as u8, max_conn, &query_vec);
                    }
                }
            }

            if !candidates.is_empty() {
                current_ep = candidates[0].1;
            }
        }

        // Update entry point if new node has a higher layer
        if new_layer > self.max_layer {
            self.entry_point = Some(new_idx);
            self.max_layer = new_layer;
        }

        Ok(())
    }

    /// Search for the `k` nearest neighbors of the given query vector.
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Result<Vec<SearchResult>> {
        if query.len() != self.config.dimensions {
            return Err(HnswError::DimensionMismatch {
                expected: self.config.dimensions,
                got: query.len(),
            });
        }

        let ep = self.entry_point.ok_or(HnswError::EmptyIndex)?;

        // Create a temporary node for the query vector
        let query_idx = self.nodes.len() as u32;

        // Greedy descent from top to layer 1
        let mut current_ep = ep;
        for layer in (1..=self.max_layer as usize).rev() {
            current_ep = self.greedy_closest_by_vec(query, current_ep, layer as u8);
        }

        // Search layer 0 with ef_search beam width
        let candidates = self.search_layer_by_vec(query, current_ep, ef_search, 0);

        // Return top-k results
        let mut results: Vec<SearchResult> = candidates
            .into_iter()
            .filter(|(_, idx)| !self.nodes[*idx as usize].deleted)
            .map(|(sim, idx)| SearchResult {
                score: sim,
                triple_id: self.nodes[idx as usize].triple_id,
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);
        Ok(results)
    }

    /// Mark a node as deleted by its triple ID.
    pub fn delete(&mut self, triple_id: TermId) -> bool {
        for node in &mut self.nodes {
            if node.triple_id == triple_id && !node.deleted {
                node.deleted = true;
                return true;
            }
        }
        false
    }

    // --- Internal helpers ---

    /// Greedy search: find the single closest non-deleted node to `query_idx` at `layer`.
    fn greedy_closest(&self, query_idx: u32, start: u32, layer: u8) -> u32 {
        let query_vec = &self.nodes[query_idx as usize].vector;
        self.greedy_closest_by_vec(query_vec, start, layer)
    }

    /// Greedy search by raw vector.
    fn greedy_closest_by_vec(&self, query: &[f32], start: u32, layer: u8) -> u32 {
        let mut current = start;
        let mut best_sim = cosine_similarity(query, &self.nodes[start as usize].vector);

        loop {
            let mut changed = false;
            let layer_idx = layer as usize;
            let neighbors = if layer_idx < self.nodes[current as usize].neighbors.len() {
                self.nodes[current as usize].neighbors[layer_idx].clone()
            } else {
                vec![]
            };

            for &neighbor in &neighbors {
                if self.nodes[neighbor as usize].deleted {
                    continue;
                }
                let sim = cosine_similarity(query, &self.nodes[neighbor as usize].vector);
                if sim > best_sim {
                    best_sim = sim;
                    current = neighbor;
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        current
    }

    /// Beam search at a single layer, returning (similarity, node_index) pairs.
    fn search_layer(&self, query_idx: u32, start: u32, ef: usize, layer: u8) -> Vec<(f32, u32)> {
        let query_vec = &self.nodes[query_idx as usize].vector;
        self.search_layer_by_vec(query_vec, start, ef, layer)
    }

    /// Beam search at a single layer using a raw query vector.
    fn search_layer_by_vec(&self, query: &[f32], start: u32, ef: usize, layer: u8) -> Vec<(f32, u32)> {
        let start_sim = cosine_similarity(query, &self.nodes[start as usize].vector);

        // candidates: max-heap of (similarity, idx) — best candidates found
        let mut candidates: BinaryHeap<OrdF32Pair> = BinaryHeap::new();
        // results: min-heap — worst of the ef-best results is at top
        let mut results: BinaryHeap<Reverse<OrdF32Pair>> = BinaryHeap::new();

        let mut visited = vec![false; self.nodes.len()];
        visited[start as usize] = true;

        candidates.push(OrdF32Pair(start_sim, start));
        results.push(Reverse(OrdF32Pair(start_sim, start)));

        while let Some(OrdF32Pair(c_sim, c_idx)) = candidates.pop() {
            // If the best candidate is worse than the worst result, stop
            let worst_result_sim = results.peek().map(|r| r.0 .0).unwrap_or(f32::NEG_INFINITY);
            if c_sim < worst_result_sim && results.len() >= ef {
                break;
            }

            let layer_idx = layer as usize;
            let neighbors = if layer_idx < self.nodes[c_idx as usize].neighbors.len() {
                self.nodes[c_idx as usize].neighbors[layer_idx].clone()
            } else {
                vec![]
            };

            for &neighbor in &neighbors {
                if visited[neighbor as usize] {
                    continue;
                }
                visited[neighbor as usize] = true;

                let sim = cosine_similarity(query, &self.nodes[neighbor as usize].vector);
                let worst_result_sim = results.peek().map(|r| r.0 .0).unwrap_or(f32::NEG_INFINITY);

                if sim > worst_result_sim || results.len() < ef {
                    candidates.push(OrdF32Pair(sim, neighbor));
                    results.push(Reverse(OrdF32Pair(sim, neighbor)));
                    if results.len() > ef {
                        results.pop();
                    }
                }
            }
        }

        results
            .into_iter()
            .map(|Reverse(OrdF32Pair(sim, idx))| (sim, idx))
            .collect()
    }

    /// Select the best `max_conn` neighbors from candidates by similarity.
    fn select_neighbors(&self, candidates: &[(f32, u32)], max_conn: usize) -> Vec<u32> {
        let mut sorted: Vec<_> = candidates.to_vec();
        sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(max_conn);
        sorted.into_iter().map(|(_, idx)| idx).collect()
    }

    /// Shrink a node's connections at a given layer to `max_conn`.
    fn shrink_connections(&mut self, node_idx: u32, layer: u8, max_conn: usize, _query: &[f32]) {
        let layer_idx = layer as usize;
        let node_vec = self.nodes[node_idx as usize].vector.clone();
        let neighbors = &self.nodes[node_idx as usize].neighbors[layer_idx];

        let mut scored: Vec<(f32, u32)> = neighbors
            .iter()
            .map(|&n| {
                let sim = cosine_similarity(&node_vec, &self.nodes[n as usize].vector);
                (sim, n)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_conn);

        self.nodes[node_idx as usize].neighbors[layer_idx] =
            scored.into_iter().map(|(_, idx)| idx).collect();
    }
}

/// Wrapper for (f32, u32) that implements Ord for use in BinaryHeap.
#[derive(Debug, Clone, PartialEq)]
struct OrdF32Pair(f32, u32);

impl Eq for OrdF32Pair {}

impl PartialOrd for OrdF32Pair {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrdF32Pair {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Simple pseudo-random f64 in [0, 1) using thread-local state.
/// Good enough for layer assignment; not cryptographic.
fn rand_f64() -> f64 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(0x12345678_9abcdef0);
    }
    STATE.with(|s| {
        let mut x = s.get();
        // xorshift64
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x as f64) / (u64::MAX as f64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index(dim: usize) -> HnswIndex {
        HnswIndex::new(HnswConfig::new(4, 20, dim))
    }

    #[test]
    fn insert_and_search() {
        let mut index = make_index(3);

        // Insert 5 vectors
        index.insert(vec![1.0, 0.0, 0.0], 100).unwrap();
        index.insert(vec![0.9, 0.1, 0.0], 101).unwrap();
        index.insert(vec![0.0, 1.0, 0.0], 102).unwrap();
        index.insert(vec![0.0, 0.0, 1.0], 103).unwrap();
        index.insert(vec![0.8, 0.2, 0.0], 104).unwrap();

        assert_eq!(index.len(), 5);

        // Search for vector closest to [1, 0, 0]
        let results = index.search(&[1.0, 0.0, 0.0], 3, 10).unwrap();
        assert!(!results.is_empty());
        // The closest should be triple_id 100 (exact match)
        assert_eq!(results[0].triple_id, 100);
    }

    #[test]
    fn dimension_mismatch() {
        let mut index = make_index(3);
        let result = index.insert(vec![1.0, 0.0], 100);
        assert!(result.is_err());
    }

    #[test]
    fn delete_excludes_from_search() {
        let mut index = make_index(2);
        index.insert(vec![1.0, 0.0], 100).unwrap();
        index.insert(vec![0.9, 0.1], 101).unwrap();
        index.insert(vec![0.0, 1.0], 102).unwrap();

        assert!(index.delete(100));

        let results = index.search(&[1.0, 0.0], 3, 10).unwrap();
        assert!(results.iter().all(|r| r.triple_id != 100));
    }

    #[test]
    fn empty_index_search() {
        let index = make_index(3);
        let result = index.search(&[1.0, 0.0, 0.0], 5, 10);
        assert!(result.is_err());
    }
}
