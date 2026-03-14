//! HNSW index implementation.
//!
//! This is the core approximate nearest neighbor index. One index exists
//! per vector predicate (e.g. `:hasEmbedding`). The index is keyed by
//! triple ID, not by a standalone vector ID.
//!
//! # Design notes (informed by Qdrant)
//!
//! - Vectors are preprocessed at insert time according to the distance metric.
//!   For cosine similarity, this means normalizing to unit length so that
//!   search only needs dot product.
//! - A visited list is reused across searches to avoid per-search allocation.
//! - A HashMap maps triple_id → node_idx for O(1) deletion lookups.
//! - The RNG is seeded per-index for reproducible layer assignment.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use sutra_core::TermId;

use crate::error::{HnswError, Result};
use crate::node::HnswNode;
use crate::vector::DistanceMetric;

/// A search result: (similarity score, triple ID).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub score: f32,
    pub triple_id: TermId,
}

/// Configuration for an HNSW index.
#[derive(Debug, Clone)]
pub struct HnswConfig {
    /// Maximum connections per node per layer (levels > 0).
    pub m: usize,
    /// Maximum connections for layer 0 (typically 2*M).
    pub m0: usize,
    /// Beam width during index construction.
    pub ef_construction: usize,
    /// Fixed vector dimensionality.
    pub dimensions: usize,
    /// Distance metric.
    pub metric: DistanceMetric,
}

impl HnswConfig {
    /// Create a config with the given parameters and cosine metric.
    pub fn new(m: usize, ef_construction: usize, dimensions: usize) -> Self {
        Self {
            m,
            m0: m * 2,
            ef_construction,
            dimensions,
            metric: DistanceMetric::Cosine,
        }
    }

    /// Create a config with a specific distance metric.
    pub fn with_metric(
        m: usize,
        ef_construction: usize,
        dimensions: usize,
        metric: DistanceMetric,
    ) -> Self {
        Self {
            m,
            m0: m * 2,
            ef_construction,
            dimensions,
            metric,
        }
    }
}

/// An HNSW index for a single vector predicate.
pub struct HnswIndex {
    config: HnswConfig,
    /// All nodes in insertion order. Node index = position in this vec.
    nodes: Vec<HnswNode>,
    /// Map from triple_id to node index for O(1) lookups.
    triple_to_node: HashMap<TermId, u32>,
    /// Index of the entry point node (top of the graph).
    entry_point: Option<u32>,
    /// Maximum layer currently in the graph.
    max_layer: u8,
    /// Scaling factor for random layer assignment: 1 / ln(M).
    ml: f64,
    /// RNG state for layer assignment (xorshift64).
    rng_state: u64,
    /// Reusable visited list to avoid allocation on the search hot path.
    /// Reset (cleared) before each search.
    visited: Vec<bool>,
}

impl HnswIndex {
    /// Create a new empty HNSW index with the given configuration.
    pub fn new(config: HnswConfig) -> Self {
        let ml = 1.0 / (config.m as f64).ln();
        Self {
            config,
            nodes: Vec::new(),
            triple_to_node: HashMap::new(),
            entry_point: None,
            max_layer: 0,
            ml,
            rng_state: 0x517cc1b727220a95, // well-distributed seed
            visited: Vec::new(),
        }
    }

    /// Create an index with a specific RNG seed (for reproducible tests).
    pub fn with_seed(config: HnswConfig, seed: u64) -> Self {
        let mut idx = Self::new(config);
        idx.rng_state = if seed == 0 { 1 } else { seed };
        idx
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

    /// Number of non-deleted nodes.
    pub fn active_count(&self) -> usize {
        self.nodes.iter().filter(|n| !n.deleted).count()
    }

    /// The distance metric used by this index.
    pub fn metric(&self) -> DistanceMetric {
        self.config.metric
    }

    /// Assign a random layer using geometric distribution.
    /// Uses xorshift64 for fast, reproducible pseudo-randomness.
    fn random_layer(&mut self) -> u8 {
        // xorshift64
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        let r = (self.rng_state as f64) / (u64::MAX as f64);
        (-r.ln() * self.ml).floor() as u8
    }

    /// Insert a vector into the index, associated with the given triple ID.
    ///
    /// The vector is preprocessed according to the distance metric (e.g.
    /// normalized for cosine). The original vector is not preserved.
    pub fn insert(&mut self, mut vector: Vec<f32>, triple_id: TermId) -> Result<()> {
        if vector.len() != self.config.dimensions {
            return Err(HnswError::DimensionMismatch {
                expected: self.config.dimensions,
                got: vector.len(),
            });
        }

        // Preprocess (normalize for cosine, no-op otherwise)
        self.config.metric.preprocess(&mut vector);

        let new_layer = self.random_layer();
        let new_node = HnswNode::new(vector, new_layer, triple_id);
        let new_idx = self.nodes.len() as u32;
        self.nodes.push(new_node);
        self.triple_to_node.insert(triple_id, new_idx);

        // First node becomes the entry point
        if self.entry_point.is_none() {
            self.entry_point = Some(new_idx);
            self.max_layer = new_layer;
            return Ok(());
        }

        let mut current_ep = self.entry_point.unwrap();

        // Phase 1: Greedily descend from top layer to new_layer + 1
        for layer in (new_layer as usize + 1..=self.max_layer as usize).rev() {
            current_ep = self.greedy_closest(new_idx, current_ep, layer as u8);
        }

        // Phase 2: Insert at layers new_layer down to 0
        let ef = self.config.ef_construction;
        for layer in (0..=std::cmp::min(new_layer, self.max_layer) as usize).rev() {
            let candidates = self.search_layer_internal(new_idx, current_ep, ef, layer as u8);
            let max_conn = if layer == 0 {
                self.config.m0
            } else {
                self.config.m
            };
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
                        self.shrink_connections(neighbor_idx, layer as u8, max_conn);
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
    ///
    /// `ef_search` controls the beam width (higher = better recall, slower).
    /// The query vector is preprocessed according to the distance metric.
    pub fn search(
        &mut self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>> {
        if query.len() != self.config.dimensions {
            return Err(HnswError::DimensionMismatch {
                expected: self.config.dimensions,
                got: query.len(),
            });
        }

        let ep = self.entry_point.ok_or(HnswError::EmptyIndex)?;

        // Preprocess query vector
        let mut query_vec = query.to_vec();
        self.config.metric.preprocess(&mut query_vec);

        // Greedy descent from top to layer 1
        let mut current_ep = ep;
        for layer in (1..=self.max_layer as usize).rev() {
            current_ep = self.greedy_closest_by_vec(&query_vec, current_ep, layer as u8);
        }

        // Search layer 0 with ef_search beam width
        let candidates = self.search_layer_by_vec(&query_vec, current_ep, ef_search, 0);

        // Return top-k results, excluding deleted nodes
        let mut results: Vec<SearchResult> = candidates
            .into_iter()
            .filter(|(_, idx)| !self.nodes[*idx as usize].deleted)
            .map(|(sim, idx)| SearchResult {
                score: sim,
                triple_id: self.nodes[idx as usize].triple_id,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(k);
        Ok(results)
    }

    /// Mark a node as deleted by its triple ID. O(1) via HashMap lookup.
    pub fn delete(&mut self, triple_id: TermId) -> bool {
        if let Some(&node_idx) = self.triple_to_node.get(&triple_id) {
            let node = &mut self.nodes[node_idx as usize];
            if !node.deleted {
                node.deleted = true;
                return true;
            }
        }
        false
    }

    /// Fraction of nodes that are deleted. Used to decide when to trigger compaction.
    pub fn deleted_ratio(&self) -> f64 {
        if self.nodes.is_empty() {
            return 0.0;
        }
        let deleted = self.nodes.iter().filter(|n| n.deleted).count();
        deleted as f64 / self.nodes.len() as f64
    }

    // --- Internal helpers ---

    /// Compute score between a raw vector and a node.
    fn score_vec_node(&self, query: &[f32], target_idx: u32) -> f32 {
        self.config
            .metric
            .score(query, &self.nodes[target_idx as usize].vector)
    }

    /// Greedy search: find the single closest non-deleted node to `query_idx` at `layer`.
    fn greedy_closest(&self, query_idx: u32, start: u32, layer: u8) -> u32 {
        let query_vec = &self.nodes[query_idx as usize].vector;
        self.greedy_closest_by_vec(query_vec, start, layer)
    }

    /// Greedy search by raw vector.
    fn greedy_closest_by_vec(&self, query: &[f32], start: u32, layer: u8) -> u32 {
        let mut current = start;
        let mut best_score = self.score_vec_node(query, start);

        loop {
            let mut changed = false;
            let layer_idx = layer as usize;

            // Avoid cloning: copy neighbor indices out first
            let neighbor_count = self.nodes[current as usize]
                .neighbors
                .get(layer_idx)
                .map_or(0, |n| n.len());

            for i in 0..neighbor_count {
                let neighbor = self.nodes[current as usize].neighbors[layer_idx][i];
                if self.nodes[neighbor as usize].deleted {
                    continue;
                }
                let score = self.score_vec_node(query, neighbor);
                if score > best_score {
                    best_score = score;
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

    /// Beam search at a single layer (using node index for query).
    fn search_layer_internal(
        &mut self,
        query_idx: u32,
        start: u32,
        ef: usize,
        layer: u8,
    ) -> Vec<(f32, u32)> {
        let query_vec = self.nodes[query_idx as usize].vector.clone();
        self.search_layer_by_vec(&query_vec, start, ef, layer)
    }

    /// Beam search at a single layer using a raw query vector.
    ///
    /// Uses a reusable visited list to avoid allocation on the hot path.
    fn search_layer_by_vec(
        &mut self,
        query: &[f32],
        start: u32,
        ef: usize,
        layer: u8,
    ) -> Vec<(f32, u32)> {
        // Resize and clear visited list
        let n = self.nodes.len();
        self.visited.resize(n, false);
        for v in self.visited.iter_mut() {
            *v = false;
        }

        let start_score = self.score_vec_node(query, start);

        // candidates: max-heap — best unexplored candidates
        let mut candidates: BinaryHeap<OrdF32Pair> = BinaryHeap::new();
        // results: min-heap — worst of the ef-best results is at top
        let mut results: BinaryHeap<Reverse<OrdF32Pair>> = BinaryHeap::new();

        self.visited[start as usize] = true;
        candidates.push(OrdF32Pair(start_score, start));
        results.push(Reverse(OrdF32Pair(start_score, start)));

        while let Some(OrdF32Pair(c_score, c_idx)) = candidates.pop() {
            let worst_result = results.peek().map(|r| r.0 .0).unwrap_or(f32::NEG_INFINITY);
            if c_score < worst_result && results.len() >= ef {
                break;
            }

            let layer_idx = layer as usize;
            let neighbor_count = self.nodes[c_idx as usize]
                .neighbors
                .get(layer_idx)
                .map_or(0, |n| n.len());

            for i in 0..neighbor_count {
                let neighbor = self.nodes[c_idx as usize].neighbors[layer_idx][i];
                if self.visited[neighbor as usize] {
                    continue;
                }
                self.visited[neighbor as usize] = true;

                let score = self.score_vec_node(query, neighbor);
                let worst_result = results.peek().map(|r| r.0 .0).unwrap_or(f32::NEG_INFINITY);

                if score > worst_result || results.len() < ef {
                    candidates.push(OrdF32Pair(score, neighbor));
                    results.push(Reverse(OrdF32Pair(score, neighbor)));
                    if results.len() > ef {
                        results.pop();
                    }
                }
            }
        }

        results
            .into_iter()
            .map(|Reverse(OrdF32Pair(score, idx))| (score, idx))
            .collect()
    }

    /// Select the best `max_conn` neighbors from candidates by score.
    fn select_neighbors(&self, candidates: &[(f32, u32)], max_conn: usize) -> Vec<u32> {
        let mut sorted: Vec<_> = candidates.to_vec();
        sorted.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(max_conn);
        sorted.into_iter().map(|(_, idx)| idx).collect()
    }

    /// Shrink a node's connections at a given layer to `max_conn`.
    fn shrink_connections(&mut self, node_idx: u32, layer: u8, max_conn: usize) {
        let layer_idx = layer as usize;
        let node_vec = self.nodes[node_idx as usize].vector.clone();
        let neighbors: Vec<u32> = self.nodes[node_idx as usize].neighbors[layer_idx].clone();

        let mut scored: Vec<(f32, u32)> = neighbors
            .iter()
            .map(|&n| {
                let score = self
                    .config
                    .metric
                    .score(&node_vec, &self.nodes[n as usize].vector);
                (score, n)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index(dim: usize) -> HnswIndex {
        HnswIndex::with_seed(HnswConfig::new(4, 20, dim), 42)
    }

    fn make_euclidean_index(dim: usize) -> HnswIndex {
        HnswIndex::with_seed(
            HnswConfig::with_metric(4, 20, dim, DistanceMetric::Euclidean),
            42,
        )
    }

    // --- Basic functionality ---

    #[test]
    fn insert_single() {
        let mut index = make_index(3);
        index.insert(vec![1.0, 0.0, 0.0], 100).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index.active_count(), 1);
    }

    #[test]
    fn insert_multiple() {
        let mut index = make_index(3);
        for i in 0..10 {
            index.insert(vec![i as f32, 0.0, 1.0], 100 + i).unwrap();
        }
        assert_eq!(index.len(), 10);
    }

    #[test]
    fn dimension_mismatch_insert() {
        let mut index = make_index(3);
        let result = index.insert(vec![1.0, 0.0], 100);
        assert!(matches!(
            result,
            Err(HnswError::DimensionMismatch {
                expected: 3,
                got: 2
            })
        ));
    }

    #[test]
    fn dimension_mismatch_search() {
        let mut index = make_index(3);
        index.insert(vec![1.0, 0.0, 0.0], 100).unwrap();
        let result = index.search(&[1.0, 0.0], 5, 10);
        assert!(matches!(result, Err(HnswError::DimensionMismatch { .. })));
    }

    #[test]
    fn empty_index_search() {
        let mut index = make_index(3);
        let result = index.search(&[1.0, 0.0, 0.0], 5, 10);
        assert!(matches!(result, Err(HnswError::EmptyIndex)));
    }

    // --- Search quality ---

    #[test]
    fn search_finds_exact_match() {
        let mut index = make_index(3);
        index.insert(vec![1.0, 0.0, 0.0], 100).unwrap();
        index.insert(vec![0.0, 1.0, 0.0], 101).unwrap();
        index.insert(vec![0.0, 0.0, 1.0], 102).unwrap();

        let results = index.search(&[1.0, 0.0, 0.0], 1, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].triple_id, 100);
        assert!((results[0].score - 1.0).abs() < 1e-5); // cosine of identical = 1
    }

    #[test]
    fn search_returns_k_results() {
        let mut index = make_index(3);
        for i in 0..20 {
            let angle = (i as f32) * 0.3;
            index
                .insert(vec![angle.cos(), angle.sin(), 0.0], 100 + i)
                .unwrap();
        }

        let results = index.search(&[1.0, 0.0, 0.0], 5, 20).unwrap();
        assert_eq!(results.len(), 5);

        // Results should be sorted by score descending
        for w in results.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn search_similarity_ordering() {
        let mut index = make_index(2);
        // Insert vectors at known angles
        index.insert(vec![1.0, 0.0], 100).unwrap(); // 0°
        index.insert(vec![0.7, 0.7], 101).unwrap(); // ~45°
        index.insert(vec![0.0, 1.0], 102).unwrap(); // 90°
        index.insert(vec![-1.0, 0.0], 103).unwrap(); // 180°

        let results = index.search(&[1.0, 0.0], 4, 10).unwrap();
        // Should be ordered: 100, 101, 102, 103
        assert_eq!(results[0].triple_id, 100); // most similar (same direction)
        assert_eq!(results[results.len() - 1].triple_id, 103); // least similar (opposite)
    }

    #[test]
    fn search_with_larger_ef_finds_more() {
        let mut index = make_index(8);
        // Insert 100 random-ish vectors
        for i in 0..100u64 {
            let v: Vec<f32> = (0..8)
                .map(|d| ((i * 7 + d * 13) % 100) as f32 / 100.0)
                .collect();
            index.insert(v, i).unwrap();
        }

        let results_small_ef = index.search(&[0.5; 8], 10, 10).unwrap();
        let results_large_ef = index.search(&[0.5; 8], 10, 50).unwrap();

        // Both should return 10 results
        assert_eq!(results_small_ef.len(), 10);
        assert_eq!(results_large_ef.len(), 10);

        // Larger ef should find at least as good results
        assert!(results_large_ef[0].score >= results_small_ef[0].score - 1e-6);
    }

    // --- Deletion ---

    #[test]
    fn delete_by_triple_id() {
        let mut index = make_index(2);
        index.insert(vec![1.0, 0.0], 100).unwrap();
        index.insert(vec![0.0, 1.0], 101).unwrap();

        assert!(index.delete(100));
        assert_eq!(index.active_count(), 1);
        assert_eq!(index.len(), 2); // still present, just marked
    }

    #[test]
    fn delete_nonexistent() {
        let mut index = make_index(2);
        index.insert(vec![1.0, 0.0], 100).unwrap();
        assert!(!index.delete(999));
    }

    #[test]
    fn delete_double() {
        let mut index = make_index(2);
        index.insert(vec![1.0, 0.0], 100).unwrap();
        assert!(index.delete(100));
        assert!(!index.delete(100)); // already deleted
    }

    #[test]
    fn deleted_excluded_from_search() {
        let mut index = make_index(2);
        index.insert(vec![1.0, 0.0], 100).unwrap();
        index.insert(vec![0.9, 0.1], 101).unwrap();
        index.insert(vec![0.0, 1.0], 102).unwrap();

        index.delete(100);

        let results = index.search(&[1.0, 0.0], 3, 10).unwrap();
        assert!(results.iter().all(|r| r.triple_id != 100));
        // 101 should now be the best match
        assert_eq!(results[0].triple_id, 101);
    }

    #[test]
    fn deleted_ratio() {
        let mut index = make_index(2);
        index.insert(vec![1.0, 0.0], 100).unwrap();
        index.insert(vec![0.0, 1.0], 101).unwrap();
        index.insert(vec![0.5, 0.5], 102).unwrap();
        index.insert(vec![0.3, 0.7], 103).unwrap();

        assert!((index.deleted_ratio() - 0.0).abs() < 1e-6);
        index.delete(100);
        assert!((index.deleted_ratio() - 0.25).abs() < 1e-6);
        index.delete(101);
        assert!((index.deleted_ratio() - 0.5).abs() < 1e-6);
    }

    // --- Distance metrics ---

    #[test]
    fn euclidean_metric() {
        let mut index = make_euclidean_index(2);
        index.insert(vec![0.0, 0.0], 100).unwrap();
        index.insert(vec![1.0, 0.0], 101).unwrap();
        index.insert(vec![10.0, 10.0], 102).unwrap();

        let results = index.search(&[0.0, 0.0], 3, 10).unwrap();
        assert_eq!(results[0].triple_id, 100); // closest to origin
        assert_eq!(results[1].triple_id, 101);
        assert_eq!(results[2].triple_id, 102); // farthest
    }

    // --- Stress ---

    #[test]
    fn insert_many() {
        let mut index = make_index(4);
        for i in 0..500u64 {
            let v: Vec<f32> = (0..4)
                .map(|d| ((i * 17 + d * 31) % 1000) as f32 / 1000.0)
                .collect();
            index.insert(v, i).unwrap();
        }
        assert_eq!(index.len(), 500);

        let results = index.search(&[0.5, 0.5, 0.5, 0.5], 10, 50).unwrap();
        assert_eq!(results.len(), 10);
    }

    // --- Reproducibility ---

    #[test]
    fn seeded_index_is_reproducible() {
        let build = || {
            let mut idx = HnswIndex::with_seed(HnswConfig::new(4, 20, 3), 12345);
            idx.insert(vec![1.0, 0.0, 0.0], 1).unwrap();
            idx.insert(vec![0.0, 1.0, 0.0], 2).unwrap();
            idx.insert(vec![0.0, 0.0, 1.0], 3).unwrap();
            idx.search(&[0.5, 0.5, 0.0], 3, 10).unwrap()
        };

        let r1 = build();
        let r2 = build();
        assert_eq!(r1.len(), r2.len());
        for (a, b) in r1.iter().zip(r2.iter()) {
            assert_eq!(a.triple_id, b.triple_id);
            assert!((a.score - b.score).abs() < 1e-6);
        }
    }
}
