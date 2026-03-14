//! Integration tests for sutra-hnsw: HNSW index with various scenarios.

use sutra_hnsw::*;

#[test]
fn cosine_search_quality() {
    // Build an index with vectors pointing in known directions
    let mut index = HnswIndex::with_seed(HnswConfig::new(8, 50, 3), 42);

    // Cardinal directions
    index.insert(vec![1.0, 0.0, 0.0], 1).unwrap(); // +X
    index.insert(vec![0.0, 1.0, 0.0], 2).unwrap(); // +Y
    index.insert(vec![0.0, 0.0, 1.0], 3).unwrap(); // +Z
    index.insert(vec![-1.0, 0.0, 0.0], 4).unwrap(); // -X
    index.insert(vec![0.0, -1.0, 0.0], 5).unwrap(); // -Y
    index.insert(vec![0.0, 0.0, -1.0], 6).unwrap(); // -Z

    // Diagonals
    index.insert(vec![1.0, 1.0, 0.0], 7).unwrap(); // XY
    index.insert(vec![1.0, 0.0, 1.0], 8).unwrap(); // XZ
    index.insert(vec![0.0, 1.0, 1.0], 9).unwrap(); // YZ

    // Query: find nearest to +X
    let results = index.search(&[1.0, 0.0, 0.0], 3, 20).unwrap();
    assert_eq!(results[0].triple_id, 1); // exact match

    // The next two should be the XY and XZ diagonals (both have X component)
    let next_two: Vec<u64> = results[1..3].iter().map(|r| r.triple_id).collect();
    assert!(next_two.contains(&7) || next_two.contains(&8));
}

#[test]
fn euclidean_search_quality() {
    let config = HnswConfig::with_metric(8, 50, 2, DistanceMetric::Euclidean);
    let mut index = HnswIndex::with_seed(config, 42);

    // Points in 2D
    index.insert(vec![0.0, 0.0], 1).unwrap(); // origin
    index.insert(vec![1.0, 0.0], 2).unwrap();
    index.insert(vec![5.0, 5.0], 3).unwrap();
    index.insert(vec![10.0, 10.0], 4).unwrap();

    // Nearest to origin
    let results = index.search(&[0.0, 0.0], 4, 20).unwrap();
    assert_eq!(results[0].triple_id, 1); // origin itself
    assert_eq!(results[1].triple_id, 2); // distance 1
    assert_eq!(results[2].triple_id, 3); // distance ~7
    assert_eq!(results[3].triple_id, 4); // distance ~14
}

#[test]
fn search_after_many_deletions() {
    let mut index = HnswIndex::with_seed(HnswConfig::new(8, 50, 3), 42);

    // Insert 50 vectors
    for i in 0..50u64 {
        let angle = (i as f32) * 0.1;
        index
            .insert(vec![angle.cos(), angle.sin(), 0.0], i)
            .unwrap();
    }

    // Delete half of them
    for i in (0..50u64).step_by(2) {
        index.delete(i);
    }

    assert!((index.deleted_ratio() - 0.5).abs() < 0.02);

    // Search should still work and return only non-deleted
    let results = index.search(&[1.0, 0.0, 0.0], 10, 30).unwrap();
    for r in &results {
        assert!(
            r.triple_id % 2 == 1,
            "got deleted triple_id {}",
            r.triple_id
        );
    }
}

#[test]
fn vector_normalization_for_cosine() {
    // Verify that cosine metric normalizes at insert time
    let mut index = HnswIndex::with_seed(HnswConfig::new(4, 20, 3), 42);

    // These two vectors point in the same direction but different magnitudes
    index.insert(vec![1.0, 0.0, 0.0], 1).unwrap();
    index.insert(vec![100.0, 0.0, 0.0], 2).unwrap();

    // For cosine, they should be equally similar to [1,0,0]
    let results = index.search(&[1.0, 0.0, 0.0], 2, 10).unwrap();
    assert_eq!(results.len(), 2);
    // Both should have score ~1.0 (same direction)
    assert!((results[0].score - 1.0).abs() < 1e-4);
    assert!((results[1].score - 1.0).abs() < 1e-4);
}

#[test]
fn dot_product_metric() {
    let config = HnswConfig::with_metric(4, 20, 2, DistanceMetric::DotProduct);
    let mut index = HnswIndex::with_seed(config, 42);

    index.insert(vec![1.0, 0.0], 1).unwrap();
    index.insert(vec![2.0, 0.0], 2).unwrap();
    index.insert(vec![0.0, 1.0], 3).unwrap();

    // For dot product, [2,0] has higher dot product with [1,0] than [1,0] does
    let results = index.search(&[1.0, 0.0], 3, 10).unwrap();
    assert_eq!(results[0].triple_id, 2); // dot = 2.0
    assert_eq!(results[1].triple_id, 1); // dot = 1.0
    assert_eq!(results[2].triple_id, 3); // dot = 0.0
}

#[test]
fn empty_and_single_element() {
    let mut index = HnswIndex::with_seed(HnswConfig::new(4, 20, 2), 42);

    // Empty search fails
    assert!(index.search(&[1.0, 0.0], 1, 10).is_err());

    // Single element
    index.insert(vec![1.0, 0.0], 1).unwrap();
    let results = index.search(&[0.0, 1.0], 1, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].triple_id, 1);
}

#[test]
fn k_larger_than_index() {
    let mut index = HnswIndex::with_seed(HnswConfig::new(4, 20, 2), 42);

    index.insert(vec![1.0, 0.0], 1).unwrap();
    index.insert(vec![0.0, 1.0], 2).unwrap();

    // Ask for 10 but only 2 exist
    let results = index.search(&[1.0, 0.0], 10, 10).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn high_dimensional_vectors() {
    let dim = 128;
    let mut index = HnswIndex::with_seed(HnswConfig::new(16, 100, dim), 42);

    // Insert 200 vectors
    for i in 0..200u64 {
        let v: Vec<f32> = (0..dim)
            .map(|d| ((i * 7 + d as u64 * 13) % 100) as f32 / 100.0)
            .collect();
        index.insert(v, i).unwrap();
    }

    let query: Vec<f32> = (0..dim).map(|d| (d as f32) / dim as f32).collect();
    let results = index.search(&query, 10, 50).unwrap();
    assert_eq!(results.len(), 10);

    // Results should be sorted by score
    for w in results.windows(2) {
        assert!(w[0].score >= w[1].score - 1e-6);
    }
}
