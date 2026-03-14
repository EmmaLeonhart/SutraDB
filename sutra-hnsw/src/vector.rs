//! Vector literal type (`sutra:f32vec`) and distance metrics.
//!
//! A vector is a fixed-dimension array of f32 values. The database treats
//! these as opaque numeric data — it does not know or care what embedding
//! model produced them.
//!
//! # Cosine similarity strategy (from Qdrant)
//!
//! Rather than computing cosine similarity directly (which requires two norms
//! per comparison), we normalize vectors at insert time and then use dot product
//! for all similarity computations. This is equivalent but much cheaper at
//! search time, which is the hot path.

/// Compute the L2 (Euclidean) norm of a vector.
pub fn l2_norm(v: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for &x in v {
        sum += x * x;
    }
    sum.sqrt()
}

/// Normalize a vector to unit length in-place.
/// Returns the original magnitude. If the vector is zero, it is left unchanged.
pub fn normalize(v: &mut [f32]) -> f32 {
    let norm = l2_norm(v);
    if norm > 0.0 {
        let inv = 1.0 / norm;
        for x in v.iter_mut() {
            *x *= inv;
        }
    }
    norm
}

/// Normalize a vector, returning a new owned vector.
/// If the input is zero, returns a zero vector.
pub fn normalized(v: &[f32]) -> Vec<f32> {
    let mut out = v.to_vec();
    normalize(&mut out);
    out
}

/// Dot product of two vectors.
///
/// When both vectors are pre-normalized (unit length), this equals cosine similarity.
/// This is the primary distance function used during HNSW search.
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut sum = 0.0f32;
    for i in 0..a.len() {
        sum += a[i] * b[i];
    }
    sum
}

/// Compute cosine similarity between two vectors (not pre-normalized).
///
/// Returns a value in [-1, 1]. Returns 0.0 if either vector has zero magnitude.
/// Prefer `dot_product` on pre-normalized vectors for the hot path.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

/// Compute squared Euclidean distance between two vectors.
///
/// Cheaper than full Euclidean distance (avoids sqrt) and preserves
/// ordering for nearest-neighbor comparisons.
pub fn squared_euclidean(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut sum = 0.0f32;
    for i in 0..a.len() {
        let d = a[i] - b[i];
        sum += d * d;
    }
    sum
}

/// Distance metric selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistanceMetric {
    /// Cosine similarity (vectors are normalized at insert time, dot product at search time).
    Cosine,
    /// Euclidean distance (squared, for ordering).
    Euclidean,
    /// Raw dot product (no normalization).
    DotProduct,
}

impl DistanceMetric {
    /// Preprocess a vector before insertion according to this metric.
    /// For Cosine, this normalizes the vector. For others, it's a no-op.
    pub fn preprocess(&self, vector: &mut [f32]) {
        if *self == DistanceMetric::Cosine {
            normalize(vector);
        }
    }

    /// Compute similarity/score between two vectors.
    /// Higher = more similar for all metrics.
    pub fn score(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            DistanceMetric::Cosine => dot_product(a, b), // pre-normalized
            DistanceMetric::DotProduct => dot_product(a, b),
            DistanceMetric::Euclidean => -squared_euclidean(a, b), // negate so higher = closer
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_then_dot_equals_cosine() {
        let a = vec![3.0, 4.0, 0.0];
        let b = vec![1.0, 2.0, 2.0];

        let direct = cosine_similarity(&a, &b);

        let a_norm = normalized(&a);
        let b_norm = normalized(&b);
        let via_dot = dot_product(&a_norm, &b_norm);

        assert!((direct - via_dot).abs() < 1e-5);
    }

    #[test]
    fn normalize_unit_length() {
        let mut v = vec![3.0, 4.0];
        normalize(&mut v);
        let len = l2_norm(&v);
        assert!((len - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_zero_vector() {
        let mut v = vec![0.0, 0.0, 0.0];
        let mag = normalize(&mut v);
        assert_eq!(mag, 0.0);
        assert_eq!(v, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn normalize_idempotent() {
        let mut v = vec![3.0, 4.0];
        normalize(&mut v);
        let first = v.clone();
        normalize(&mut v);
        for (a, b) in first.iter().zip(v.iter()) {
            assert!((a - b).abs() < 1e-7);
        }
    }

    #[test]
    fn squared_euclidean_zero() {
        let v = vec![1.0, 2.0, 3.0];
        assert!(squared_euclidean(&v, &v) < 1e-6);
    }

    #[test]
    fn squared_euclidean_known() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        assert!((squared_euclidean(&a, &b) - 25.0).abs() < 1e-6);
    }

    #[test]
    fn dot_product_known() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        assert!((dot_product(&a, &b) - 32.0).abs() < 1e-6);
    }

    #[test]
    fn distance_metric_cosine_preprocesses() {
        let mut v = vec![3.0, 4.0];
        DistanceMetric::Cosine.preprocess(&mut v);
        assert!((l2_norm(&v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn distance_metric_euclidean_no_preprocess() {
        let original = vec![3.0, 4.0];
        let mut v = original.clone();
        DistanceMetric::Euclidean.preprocess(&mut v);
        assert_eq!(v, original);
    }
}
