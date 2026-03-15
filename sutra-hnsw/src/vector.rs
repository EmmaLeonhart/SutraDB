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
//!
//! # SIMD acceleration
//!
//! Distance functions use SIMD intrinsics when available (AVX2, SSE), with
//! automatic fallback to scalar code. Feature detection happens once at
//! startup via `std::arch::is_x86_feature_detected!`.

// ---------------------------------------------------------------------------
// SIMD implementation (x86/x86_64)
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod simd {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    // -- AVX2 (256-bit, 8 floats per iteration) --

    #[target_feature(enable = "avx2,fma")]
    pub(crate) unsafe fn dot_product_avx2(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len();
        let chunks = n / 8;
        let remainder = n % 8;

        let mut sum = _mm256_setzero_ps();
        let a_ptr = a.as_ptr();
        let b_ptr = b.as_ptr();

        for i in 0..chunks {
            let offset = i * 8;
            let va = _mm256_loadu_ps(a_ptr.add(offset));
            let vb = _mm256_loadu_ps(b_ptr.add(offset));
            sum = _mm256_fmadd_ps(va, vb, sum);
        }

        // Horizontal sum of 8 floats
        let hi = _mm256_extractf128_ps(sum, 1);
        let lo = _mm256_castps256_ps128(sum);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        let result = _mm_add_ss(sums, shuf2);
        let mut total = _mm_cvtss_f32(result);

        // Scalar tail
        let tail_start = chunks * 8;
        for i in 0..remainder {
            total += a[tail_start + i] * b[tail_start + i];
        }
        total
    }

    #[target_feature(enable = "avx2,fma")]
    pub(crate) unsafe fn squared_euclidean_avx2(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len();
        let chunks = n / 8;
        let remainder = n % 8;

        let mut sum = _mm256_setzero_ps();
        let a_ptr = a.as_ptr();
        let b_ptr = b.as_ptr();

        for i in 0..chunks {
            let offset = i * 8;
            let va = _mm256_loadu_ps(a_ptr.add(offset));
            let vb = _mm256_loadu_ps(b_ptr.add(offset));
            let diff = _mm256_sub_ps(va, vb);
            sum = _mm256_fmadd_ps(diff, diff, sum);
        }

        // Horizontal sum
        let hi = _mm256_extractf128_ps(sum, 1);
        let lo = _mm256_castps256_ps128(sum);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        let result = _mm_add_ss(sums, shuf2);
        let mut total = _mm_cvtss_f32(result);

        let tail_start = chunks * 8;
        for i in 0..remainder {
            let d = a[tail_start + i] - b[tail_start + i];
            total += d * d;
        }
        total
    }

    #[target_feature(enable = "avx2,fma")]
    pub(crate) unsafe fn l2_norm_avx2(v: &[f32]) -> f32 {
        let n = v.len();
        let chunks = n / 8;
        let remainder = n % 8;

        let mut sum = _mm256_setzero_ps();
        let ptr = v.as_ptr();

        for i in 0..chunks {
            let offset = i * 8;
            let vv = _mm256_loadu_ps(ptr.add(offset));
            sum = _mm256_fmadd_ps(vv, vv, sum);
        }

        // Horizontal sum
        let hi = _mm256_extractf128_ps(sum, 1);
        let lo = _mm256_castps256_ps128(sum);
        let sum128 = _mm_add_ps(lo, hi);
        let shuf = _mm_movehdup_ps(sum128);
        let sums = _mm_add_ps(sum128, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        let result = _mm_add_ss(sums, shuf2);
        let mut total = _mm_cvtss_f32(result);

        let tail_start = chunks * 8;
        for i in 0..remainder {
            total += v[tail_start + i] * v[tail_start + i];
        }
        total.sqrt()
    }

    // -- SSE (128-bit, 4 floats per iteration) --

    #[target_feature(enable = "sse")]
    pub(crate) unsafe fn dot_product_sse(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len();
        let chunks = n / 4;
        let remainder = n % 4;

        let mut sum = _mm_setzero_ps();
        let a_ptr = a.as_ptr();
        let b_ptr = b.as_ptr();

        for i in 0..chunks {
            let offset = i * 4;
            let va = _mm_loadu_ps(a_ptr.add(offset));
            let vb = _mm_loadu_ps(b_ptr.add(offset));
            let prod = _mm_mul_ps(va, vb);
            sum = _mm_add_ps(sum, prod);
        }

        // Horizontal sum of 4 floats
        let shuf = _mm_movehdup_ps(sum);
        let sums = _mm_add_ps(sum, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        let result = _mm_add_ss(sums, shuf2);
        let mut total = _mm_cvtss_f32(result);

        let tail_start = chunks * 4;
        for i in 0..remainder {
            total += a[tail_start + i] * b[tail_start + i];
        }
        total
    }

    #[target_feature(enable = "sse")]
    pub(crate) unsafe fn squared_euclidean_sse(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len();
        let chunks = n / 4;
        let remainder = n % 4;

        let mut sum = _mm_setzero_ps();
        let a_ptr = a.as_ptr();
        let b_ptr = b.as_ptr();

        for i in 0..chunks {
            let offset = i * 4;
            let va = _mm_loadu_ps(a_ptr.add(offset));
            let vb = _mm_loadu_ps(b_ptr.add(offset));
            let diff = _mm_sub_ps(va, vb);
            let sq = _mm_mul_ps(diff, diff);
            sum = _mm_add_ps(sum, sq);
        }

        let shuf = _mm_movehdup_ps(sum);
        let sums = _mm_add_ps(sum, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        let result = _mm_add_ss(sums, shuf2);
        let mut total = _mm_cvtss_f32(result);

        let tail_start = chunks * 4;
        for i in 0..remainder {
            let d = a[tail_start + i] - b[tail_start + i];
            total += d * d;
        }
        total
    }

    #[target_feature(enable = "sse")]
    pub(crate) unsafe fn l2_norm_sse(v: &[f32]) -> f32 {
        let n = v.len();
        let chunks = n / 4;
        let remainder = n % 4;

        let mut sum = _mm_setzero_ps();
        let ptr = v.as_ptr();

        for i in 0..chunks {
            let offset = i * 4;
            let vv = _mm_loadu_ps(ptr.add(offset));
            let sq = _mm_mul_ps(vv, vv);
            sum = _mm_add_ps(sum, sq);
        }

        let shuf = _mm_movehdup_ps(sum);
        let sums = _mm_add_ps(sum, shuf);
        let shuf2 = _mm_movehl_ps(sums, sums);
        let result = _mm_add_ss(sums, shuf2);
        let mut total = _mm_cvtss_f32(result);

        let tail_start = chunks * 4;
        for i in 0..remainder {
            total += v[tail_start + i] * v[tail_start + i];
        }
        total.sqrt()
    }
}

// ---------------------------------------------------------------------------
// Public API — dispatches to best available SIMD at runtime
// ---------------------------------------------------------------------------

/// Compute the L2 (Euclidean) norm of a vector.
pub fn l2_norm(v: &[f32]) -> f32 {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return unsafe { simd::l2_norm_avx2(v) };
        }
        if is_x86_feature_detected!("sse") {
            return unsafe { simd::l2_norm_sse(v) };
        }
    }
    l2_norm_scalar(v)
}

/// Dot product of two vectors.
///
/// When both vectors are pre-normalized (unit length), this equals cosine similarity.
/// This is the primary distance function used during HNSW search.
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return unsafe { simd::dot_product_avx2(a, b) };
        }
        if is_x86_feature_detected!("sse") {
            return unsafe { simd::dot_product_sse(a, b) };
        }
    }
    dot_product_scalar(a, b)
}

/// Compute squared Euclidean distance between two vectors.
///
/// Cheaper than full Euclidean distance (avoids sqrt) and preserves
/// ordering for nearest-neighbor comparisons.
pub fn squared_euclidean(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            return unsafe { simd::squared_euclidean_avx2(a, b) };
        }
        if is_x86_feature_detected!("sse") {
            return unsafe { simd::squared_euclidean_sse(a, b) };
        }
    }
    squared_euclidean_scalar(a, b)
}

// ---------------------------------------------------------------------------
// Scalar fallbacks (also used on non-x86 targets)
// ---------------------------------------------------------------------------

fn l2_norm_scalar(v: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for &x in v {
        sum += x * x;
    }
    sum.sqrt()
}

fn dot_product_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..a.len() {
        sum += a[i] * b[i];
    }
    sum
}

fn squared_euclidean_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for i in 0..a.len() {
        let d = a[i] - b[i];
        sum += d * d;
    }
    sum
}

// ---------------------------------------------------------------------------
// Higher-level functions
// ---------------------------------------------------------------------------

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

/// Compute cosine similarity between two vectors (not pre-normalized).
///
/// Returns a value in [-1, 1]. Returns 0.0 if either vector has zero magnitude.
/// Prefer `dot_product` on pre-normalized vectors for the hot path.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    // Use SIMD-accelerated dot product and norms
    let dot = dot_product(a, b);
    let norm_a = l2_norm(a);
    let norm_b = l2_norm(b);

    let denom = norm_a * norm_b;
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
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

    // -- SIMD correctness tests --
    // These verify that SIMD results match scalar results for various vector sizes.

    #[test]
    fn simd_dot_product_matches_scalar() {
        // Test various sizes: smaller than SSE, SSE-aligned, AVX-aligned, and odd tails
        for size in [1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 33, 100, 384, 768, 1536] {
            let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.01).collect();
            let b: Vec<f32> = (0..size).map(|i| (i as f32) * 0.02 + 0.5).collect();

            let simd_result = dot_product(&a, &b);
            let scalar_result = dot_product_scalar(&a, &b);

            // SIMD and scalar accumulate in different order, so allow relative tolerance
            let tol = scalar_result.abs() * 1e-6 + 1e-6;
            assert!(
                (simd_result - scalar_result).abs() < tol,
                "dot_product mismatch at size {size}: simd={simd_result}, scalar={scalar_result}"
            );
        }
    }

    #[test]
    fn simd_squared_euclidean_matches_scalar() {
        for size in [1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 33, 100, 384, 768, 1536] {
            let a: Vec<f32> = (0..size).map(|i| (i as f32) * 0.01).collect();
            let b: Vec<f32> = (0..size).map(|i| (i as f32) * 0.02 + 0.5).collect();

            let simd_result = squared_euclidean(&a, &b);
            let scalar_result = squared_euclidean_scalar(&a, &b);

            let tol = scalar_result.abs() * 1e-6 + 1e-6;
            assert!(
                (simd_result - scalar_result).abs() < tol,
                "squared_euclidean mismatch at size {size}: simd={simd_result}, scalar={scalar_result}"
            );
        }
    }

    #[test]
    fn simd_l2_norm_matches_scalar() {
        for size in [1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 33, 100, 384, 768, 1536] {
            let v: Vec<f32> = (0..size).map(|i| (i as f32) * 0.01 + 0.1).collect();

            let simd_result = l2_norm(&v);
            let scalar_result = l2_norm_scalar(&v);

            let tol = scalar_result.abs() * 1e-6 + 1e-6;
            assert!(
                (simd_result - scalar_result).abs() < tol,
                "l2_norm mismatch at size {size}: simd={simd_result}, scalar={scalar_result}"
            );
        }
    }

    #[test]
    fn simd_realistic_embedding_dimensions() {
        // Test with common embedding dimensions
        let dims = [384, 768, 1536]; // MiniLM, BERT/ada, text-embedding-3-large
        for dim in dims {
            let a: Vec<f32> = (0..dim)
                .map(|i| ((i * 7 + 3) as f32 % 100.0) / 100.0 - 0.5)
                .collect();
            let b: Vec<f32> = (0..dim)
                .map(|i| ((i * 13 + 7) as f32 % 100.0) / 100.0 - 0.5)
                .collect();

            let a_norm = normalized(&a);
            let b_norm = normalized(&b);

            // Cosine similarity via pre-normalized dot product
            let score = dot_product(&a_norm, &b_norm);
            assert!(
                score >= -1.0 && score <= 1.0,
                "cosine out of range at dim {dim}: {score}"
            );

            // Euclidean distance should be non-negative
            let dist = squared_euclidean(&a, &b);
            assert!(dist >= 0.0, "negative euclidean at dim {dim}");
        }
    }
}
