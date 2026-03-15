//! Predicate-to-HNSW index registry.
//!
//! Maps vector predicate TermIds to their HNSW indexes. One index per
//! predicate (e.g. `:hasEmbedding` → HnswIndex with 1536 dimensions).

use std::collections::HashMap;

use sutra_core::TermId;

use crate::error::{HnswError, Result};
use crate::index::{HnswConfig, HnswIndex, SearchResult};
use crate::vector::DistanceMetric;

/// Configuration for declaring a vector predicate.
pub struct VectorPredicateConfig {
    /// The interned predicate ID.
    pub predicate_id: TermId,
    /// Fixed vector dimensionality for this predicate.
    pub dimensions: usize,
    /// HNSW M parameter: max connections per node per layer.
    pub m: usize,
    /// HNSW ef_construction parameter: beam width during index build.
    pub ef_construction: usize,
    /// Distance metric for similarity computation.
    pub metric: DistanceMetric,
}

/// Registry mapping predicate IDs to their HNSW indexes.
///
/// This is the top-level entry point for vector operations in the storage
/// engine. Each vector predicate gets its own independent HNSW index with
/// its own dimensionality, metric, and tuning parameters.
pub struct VectorRegistry {
    indexes: HashMap<TermId, HnswIndex>,
}

impl VectorRegistry {
    /// Create a new empty registry with no declared predicates.
    pub fn new() -> Self {
        Self {
            indexes: HashMap::new(),
        }
    }

    /// Declare a new vector predicate, creating its HNSW index.
    ///
    /// Returns an error if the predicate has already been declared.
    pub fn declare(&mut self, config: VectorPredicateConfig) -> Result<()> {
        if self.indexes.contains_key(&config.predicate_id) {
            return Err(HnswError::PredicateAlreadyDeclared(config.predicate_id));
        }

        let hnsw_config = HnswConfig::with_metric(
            config.m,
            config.ef_construction,
            config.dimensions,
            config.metric,
        );
        let index = HnswIndex::new(hnsw_config);
        self.indexes.insert(config.predicate_id, index);
        Ok(())
    }

    /// Get a mutable reference to the index for a predicate.
    pub fn get_mut(&mut self, predicate_id: TermId) -> Option<&mut HnswIndex> {
        self.indexes.get_mut(&predicate_id)
    }

    /// Get an immutable reference to the index for a predicate.
    pub fn get(&self, predicate_id: TermId) -> Option<&HnswIndex> {
        self.indexes.get(&predicate_id)
    }

    /// Check if a predicate has a vector index declared.
    pub fn has_index(&self, predicate_id: TermId) -> bool {
        self.indexes.contains_key(&predicate_id)
    }

    /// List all registered predicate IDs.
    pub fn predicates(&self) -> Vec<TermId> {
        self.indexes.keys().copied().collect()
    }

    /// Insert a vector for a triple into the appropriate predicate index.
    ///
    /// Returns an error if no index has been declared for the predicate,
    /// or if the vector dimensions don't match the index's declared dimensions.
    pub fn insert(
        &mut self,
        predicate_id: TermId,
        vector: Vec<f32>,
        triple_id: TermId,
    ) -> Result<()> {
        let index = self
            .indexes
            .get_mut(&predicate_id)
            .ok_or(HnswError::NoIndexForPredicate(predicate_id))?;
        index.insert(vector, triple_id)
    }

    /// Search for nearest neighbors on a predicate's index.
    ///
    /// Returns an error if no index has been declared for the predicate,
    /// or if the query vector dimensions don't match.
    /// Search is `&self` — concurrent reads are safe because the visited list
    /// is allocated per-call inside HnswIndex::search (Qdrant pattern).
    pub fn search(
        &self,
        predicate_id: TermId,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>> {
        let index = self
            .indexes
            .get(&predicate_id)
            .ok_or(HnswError::NoIndexForPredicate(predicate_id))?;
        index.search(query, k, ef_search)
    }

    /// Delete a vector by triple ID from a predicate's index.
    ///
    /// Returns `false` if the predicate has no index or the triple ID was
    /// not found (or was already deleted).
    pub fn delete(&mut self, predicate_id: TermId, triple_id: TermId) -> bool {
        match self.indexes.get_mut(&predicate_id) {
            Some(index) => index.delete(triple_id),
            None => false,
        }
    }
}

impl VectorRegistry {
    /// Generate all HNSW edge triples across all predicate indexes.
    ///
    /// Returns `(predicate_id, edge_triple)` pairs so the caller knows
    /// which vector predicate each edge belongs to.
    pub fn all_edge_triples(&self) -> Vec<(TermId, crate::edges::HnswEdgeTriple)> {
        let mut all = Vec::new();
        for (&pred_id, index) in &self.indexes {
            for edge in index.edge_triples() {
                all.push((pred_id, edge));
            }
        }
        all
    }

    /// Generate edge triples for a specific source node across all predicates.
    pub fn edge_triples_for_source(
        &self,
        source_triple_id: TermId,
    ) -> Vec<(TermId, crate::edges::HnswEdgeTriple)> {
        let mut all = Vec::new();
        for (&pred_id, index) in &self.indexes {
            for edge in index.edge_triples_for_source(source_triple_id) {
                all.push((pred_id, edge));
            }
        }
        all
    }

    /// Generate edge triples for a specific target node across all predicates.
    pub fn edge_triples_for_target(
        &self,
        target_triple_id: TermId,
    ) -> Vec<(TermId, crate::edges::HnswEdgeTriple)> {
        let mut all = Vec::new();
        for (&pred_id, index) in &self.indexes {
            for edge in index.edge_triples_for_target(target_triple_id) {
                all.push((pred_id, edge));
            }
        }
        all
    }

    /// Total edge count across all predicate indexes.
    pub fn total_edge_count(&self) -> usize {
        self.indexes.values().map(|idx| idx.edge_count()).sum()
    }
}

impl Default for VectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRED_EMBEDDING: TermId = 1000;
    const PRED_SUMMARY: TermId = 2000;

    fn default_config(predicate_id: TermId, dimensions: usize) -> VectorPredicateConfig {
        VectorPredicateConfig {
            predicate_id,
            dimensions,
            m: 4,
            ef_construction: 20,
            metric: DistanceMetric::Cosine,
        }
    }

    // --- Declaration ---

    #[test]
    fn declare_new_predicate() {
        let mut reg = VectorRegistry::new();
        assert!(reg.declare(default_config(PRED_EMBEDDING, 3)).is_ok());
        assert!(reg.has_index(PRED_EMBEDDING));
    }

    #[test]
    fn declare_duplicate_predicate_errors() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();
        let result = reg.declare(default_config(PRED_EMBEDDING, 3));
        assert!(matches!(
            result,
            Err(HnswError::PredicateAlreadyDeclared(PRED_EMBEDDING))
        ));
    }

    #[test]
    fn declare_multiple_predicates() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 128)).unwrap();
        reg.declare(default_config(PRED_SUMMARY, 64)).unwrap();

        assert!(reg.has_index(PRED_EMBEDDING));
        assert!(reg.has_index(PRED_SUMMARY));
        assert!(!reg.has_index(9999));

        let mut preds = reg.predicates();
        preds.sort();
        assert_eq!(preds, vec![PRED_EMBEDDING, PRED_SUMMARY]);
    }

    // --- Get accessors ---

    #[test]
    fn get_returns_index() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        assert!(reg.get(PRED_EMBEDDING).is_some());
        assert_eq!(reg.get(PRED_EMBEDDING).unwrap().dimensions(), 3);
        assert!(reg.get(9999).is_none());
    }

    #[test]
    fn get_mut_returns_mutable_index() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        let index = reg.get_mut(PRED_EMBEDDING).unwrap();
        assert!(index.is_empty());
        assert!(reg.get_mut(9999).is_none());
    }

    // --- Insert ---

    #[test]
    fn insert_into_declared_predicate() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        assert!(reg.insert(PRED_EMBEDDING, vec![1.0, 0.0, 0.0], 100).is_ok());
        assert_eq!(reg.get(PRED_EMBEDDING).unwrap().len(), 1);
    }

    #[test]
    fn insert_into_undeclared_predicate_errors() {
        let mut reg = VectorRegistry::new();
        let result = reg.insert(PRED_EMBEDDING, vec![1.0, 0.0, 0.0], 100);
        assert!(matches!(
            result,
            Err(HnswError::NoIndexForPredicate(PRED_EMBEDDING))
        ));
    }

    #[test]
    fn insert_dimension_mismatch() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        let result = reg.insert(PRED_EMBEDDING, vec![1.0, 0.0], 100);
        assert!(matches!(
            result,
            Err(HnswError::DimensionMismatch {
                expected: 3,
                got: 2
            })
        ));
    }

    // --- Search ---

    #[test]
    fn search_declared_predicate() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        reg.insert(PRED_EMBEDDING, vec![1.0, 0.0, 0.0], 100)
            .unwrap();
        reg.insert(PRED_EMBEDDING, vec![0.0, 1.0, 0.0], 101)
            .unwrap();
        reg.insert(PRED_EMBEDDING, vec![0.0, 0.0, 1.0], 102)
            .unwrap();

        let results = reg.search(PRED_EMBEDDING, &[1.0, 0.0, 0.0], 1, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].triple_id, 100);
    }

    #[test]
    fn search_undeclared_predicate_errors() {
        let mut reg = VectorRegistry::new();
        let result = reg.search(PRED_EMBEDDING, &[1.0, 0.0, 0.0], 1, 10);
        assert!(matches!(
            result,
            Err(HnswError::NoIndexForPredicate(PRED_EMBEDDING))
        ));
    }

    #[test]
    fn search_empty_index_errors() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        let result = reg.search(PRED_EMBEDDING, &[1.0, 0.0, 0.0], 1, 10);
        assert!(matches!(result, Err(HnswError::EmptyIndex)));
    }

    // --- Delete ---

    #[test]
    fn delete_existing_triple() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();
        reg.insert(PRED_EMBEDDING, vec![1.0, 0.0, 0.0], 100)
            .unwrap();

        assert!(reg.delete(PRED_EMBEDDING, 100));
        assert_eq!(reg.get(PRED_EMBEDDING).unwrap().active_count(), 0);
    }

    #[test]
    fn delete_nonexistent_triple() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        assert!(!reg.delete(PRED_EMBEDDING, 999));
    }

    #[test]
    fn delete_undeclared_predicate() {
        let mut reg = VectorRegistry::new();
        assert!(!reg.delete(PRED_EMBEDDING, 100));
    }

    #[test]
    fn deleted_excluded_from_search() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();

        reg.insert(PRED_EMBEDDING, vec![1.0, 0.0, 0.0], 100)
            .unwrap();
        reg.insert(PRED_EMBEDDING, vec![0.9, 0.1, 0.0], 101)
            .unwrap();
        reg.insert(PRED_EMBEDDING, vec![0.0, 1.0, 0.0], 102)
            .unwrap();

        reg.delete(PRED_EMBEDDING, 100);

        let results = reg.search(PRED_EMBEDDING, &[1.0, 0.0, 0.0], 3, 10).unwrap();
        assert!(results.iter().all(|r| r.triple_id != 100));
        assert_eq!(results[0].triple_id, 101);
    }

    // --- Multiple predicates ---

    #[test]
    fn independent_predicate_indexes() {
        let mut reg = VectorRegistry::new();
        reg.declare(default_config(PRED_EMBEDDING, 3)).unwrap();
        reg.declare(default_config(PRED_SUMMARY, 2)).unwrap();

        // Insert into different predicates with different dimensions
        reg.insert(PRED_EMBEDDING, vec![1.0, 0.0, 0.0], 100)
            .unwrap();
        reg.insert(PRED_SUMMARY, vec![1.0, 0.0], 200).unwrap();

        // Each index is independent
        assert_eq!(reg.get(PRED_EMBEDDING).unwrap().len(), 1);
        assert_eq!(reg.get(PRED_SUMMARY).unwrap().len(), 1);

        // Search on one doesn't affect the other
        let results = reg.search(PRED_EMBEDDING, &[1.0, 0.0, 0.0], 1, 10).unwrap();
        assert_eq!(results[0].triple_id, 100);

        let results = reg.search(PRED_SUMMARY, &[1.0, 0.0], 1, 10).unwrap();
        assert_eq!(results[0].triple_id, 200);

        // Wrong dimensions for a predicate still errors
        assert!(reg.insert(PRED_EMBEDDING, vec![1.0, 0.0], 101).is_err());
        assert!(reg.insert(PRED_SUMMARY, vec![1.0, 0.0, 0.0], 201).is_err());
    }

    // --- Default ---

    #[test]
    fn default_is_empty() {
        let reg = VectorRegistry::default();
        assert!(reg.predicates().is_empty());
        assert!(!reg.has_index(PRED_EMBEDDING));
    }
}
