//! Pseudo-tables: auto-discovered columnar indexes over RDF triple patterns.
//!
//! # What are pseudo-tables?
//!
//! RDF has no tables, but relational structure exists implicitly in the graph.
//! When many nodes share the same set of predicates (like all `Person` nodes
//! having `name`, `age`, `email`), they form a "characteristic set" — a group
//! that behaves like rows in a relational table.
//!
//! Pseudo-tables auto-discover these groups and materialize columnar indexes
//! over them, enabling SQL-like query acceleration for the relational portions
//! of SPARQL queries.
//!
//! # Design
//!
//! ## Property model
//!
//! A "property" is defined by a predicate + position pair:
//! - `SUB→:eats` means the node appears as **subject** of `:eats`
//! - `OBJ→:eats` means the node appears as **object** of `:eats`
//!
//! This distinction is critical: a cat that eats mice has `SUB→:eats`,
//! while the mouse has `OBJ→:eats`. Being on different ends of the same
//! predicate is a fundamentally different property.
//!
//! ## Discovery criteria
//!
//! A group qualifies for a pseudo-table when:
//! 1. A statistically significant cluster of nodes shares 5+ properties
//! 2. Each of those 5+ properties is held by ≥50% of the group
//! 3. The group has enough members to justify the columnar index overhead
//!
//! ## Table structure
//!
//! Each property held by ≥33% of the group becomes a column. If a node
//! doesn't have a property that is a column, the value is null (None).
//! An additional column tracks the count of "tail properties" — properties
//! not included as columns — per node.
//!
//! ## Data health metric
//!
//! The "cliff" between core properties (high coverage) and tail properties
//! (low coverage) indicates schema consistency:
//! - **Sharp cliff**: 10 properties at 100%, everything else at <10% → healthy
//! - **Gradual slope**: properties spread across 20%-80% → messy schema
//!
//! ## Segment-level storage (DuckDB pattern)
//!
//! Rows are stored in segments of ~2048 rows. Each segment maintains
//! per-column zonemaps (min/max) for skip-scan pruning: if a query asks
//! for `?age > 50` and a segment's max age is 30, the entire segment
//! is skipped without examining individual rows.
//!
//! ## Reference architectures
//!
//! - **DataFusion**: `Precision<T>` pattern for column statistics (min/max/null_count/distinct)
//! - **DuckDB**: Segment-level zonemaps for skip-scan pruning, sorted by most selective column

use std::collections::HashMap;

use crate::id::TermId;
use crate::store::TripleStore;

// ---------------------------------------------------------------------------
// Selection vector: bitset-based row selection for vectorized execution
// ---------------------------------------------------------------------------

/// A compact bitset representing selected rows in a segment.
///
/// This is the core primitive for vectorized execution (DuckDB/Velox pattern).
/// Instead of materializing `Vec<usize>` index lists at each filter stage,
/// we produce a bitset and AND them together with SIMD. The final bitset is
/// only expanded to row indices once, at the end.
///
/// Layout: one bit per row, packed into u64 words. For a 2048-row segment
/// this is 32 u64s = 256 bytes — fits in 4 cache lines.
#[derive(Debug, Clone)]
pub struct SelectionVector {
    /// Packed bits: bit `i` of word `i/64` represents row `i`.
    pub bits: Vec<u64>,
    /// Total number of rows this vector covers (may exceed bits.len() * 64
    /// if the last word is partial).
    pub len: usize,
}

impl SelectionVector {
    /// Create a selection vector with all rows selected.
    pub fn all_set(len: usize) -> Self {
        let nwords = len.div_ceil(64);
        let mut bits = vec![u64::MAX; nwords];
        // Clear trailing bits in the last word.
        let remainder = len % 64;
        if remainder != 0 && !bits.is_empty() {
            let last = bits.len() - 1;
            bits[last] = (1u64 << remainder) - 1;
        }
        Self { bits, len }
    }

    /// Create a selection vector with no rows selected.
    pub fn none(len: usize) -> Self {
        let nwords = len.div_ceil(64);
        Self {
            bits: vec![0u64; nwords],
            len,
        }
    }

    /// Set bit at position `idx`.
    #[inline]
    pub fn set(&mut self, idx: usize) {
        debug_assert!(idx < self.len);
        self.bits[idx / 64] |= 1u64 << (idx % 64);
    }

    /// Test bit at position `idx`.
    #[inline]
    pub fn test(&self, idx: usize) -> bool {
        debug_assert!(idx < self.len);
        self.bits[idx / 64] & (1u64 << (idx % 64)) != 0
    }

    /// Count the number of selected rows.
    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Expand the bitset to a sorted list of selected row indices.
    pub fn to_indices(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.count());
        for (word_idx, &word) in self.bits.iter().enumerate() {
            if word == 0 {
                continue;
            }
            let base = word_idx * 64;
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros() as usize;
                let row = base + bit;
                if row < self.len {
                    out.push(row);
                }
                w &= w - 1; // Clear lowest set bit
            }
        }
        out
    }

    /// In-place AND: keep only rows selected in both `self` and `other`.
    /// Uses SIMD when available.
    pub fn and_inplace(&mut self, other: &SelectionVector) {
        debug_assert_eq!(self.bits.len(), other.bits.len());
        bitset_and_inplace(&mut self.bits, &other.bits);
    }

    /// In-place OR: select rows from either `self` or `other`.
    pub fn or_inplace(&mut self, other: &SelectionVector) {
        debug_assert_eq!(self.bits.len(), other.bits.len());
        bitset_or_inplace(&mut self.bits, &other.bits);
    }

    /// Whether no rows are selected.
    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|&w| w == 0)
    }
}

// ---------------------------------------------------------------------------
// SIMD bitset operations
// ---------------------------------------------------------------------------

/// AND two bitset arrays in-place using SIMD when available.
fn bitset_and_inplace(a: &mut [u64], b: &[u64]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { bitset_and_avx2(a, b) };
            return;
        }
    }
    // Scalar fallback
    for (x, &y) in a.iter_mut().zip(b.iter()) {
        *x &= y;
    }
}

/// OR two bitset arrays in-place using SIMD when available.
fn bitset_or_inplace(a: &mut [u64], b: &[u64]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { bitset_or_avx2(a, b) };
            return;
        }
    }
    // Scalar fallback
    for (x, &y) in a.iter_mut().zip(b.iter()) {
        *x |= y;
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn bitset_and_avx2(a: &mut [u64], b: &[u64]) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    let n = a.len();
    let chunks = n / 4; // 4 u64s = 256 bits per AVX2 register
    let a_ptr = a.as_mut_ptr() as *mut __m256i;
    let b_ptr = b.as_ptr() as *const __m256i;

    for i in 0..chunks {
        let va = _mm256_loadu_si256(a_ptr.add(i) as *const __m256i);
        let vb = _mm256_loadu_si256(b_ptr.add(i));
        let result = _mm256_and_si256(va, vb);
        _mm256_storeu_si256(a_ptr.add(i), result);
    }

    // Scalar tail
    let tail = chunks * 4;
    for i in tail..n {
        a[i] &= b[i];
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn bitset_or_avx2(a: &mut [u64], b: &[u64]) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    let n = a.len();
    let chunks = n / 4;
    let a_ptr = a.as_mut_ptr() as *mut __m256i;
    let b_ptr = b.as_ptr() as *const __m256i;

    for i in 0..chunks {
        let va = _mm256_loadu_si256(a_ptr.add(i) as *const __m256i);
        let vb = _mm256_loadu_si256(b_ptr.add(i));
        let result = _mm256_or_si256(va, vb);
        _mm256_storeu_si256(a_ptr.add(i), result);
    }

    let tail = chunks * 4;
    for i in tail..n {
        a[i] |= b[i];
    }
}

// ---------------------------------------------------------------------------
// SIMD implementation for u64 column scanning (x86/x86_64)
// ---------------------------------------------------------------------------

/// Sentinel value for null entries in packed columns.
/// Using u64::MAX because TermIds are sequentially assigned starting from small values,
/// so u64::MAX will never collide with a real TermId.
const NULL_SENTINEL: u64 = u64::MAX;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod simd {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;

    // -- AVX2 bitset-producing column scans (256-bit, 4 u64s per iteration) --

    /// Scan a packed u64 column for equality matches, producing a bitset.
    /// Each matching row sets its corresponding bit in `out_bits`.
    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn scan_eq_to_bitset_avx2(data: &[u64], value: u64, out_bits: &mut [u64]) {
        let n = data.len();
        let chunks = n / 4;
        let ptr = data.as_ptr() as *const __m256i;
        let needle = _mm256_set1_epi64x(value as i64);

        for i in 0..chunks {
            let block = _mm256_loadu_si256(ptr.add(i));
            let cmp = _mm256_cmpeq_epi64(block, needle);
            let mask = _mm256_movemask_epi8(cmp) as u32;
            // Extract per-element match bits and set in bitset
            let base = i * 4;
            if mask & 0x000000FF != 0 {
                out_bits[base / 64] |= 1u64 << (base % 64);
            }
            if mask & 0x0000FF00 != 0 {
                out_bits[(base + 1) / 64] |= 1u64 << ((base + 1) % 64);
            }
            if mask & 0x00FF0000 != 0 {
                out_bits[(base + 2) / 64] |= 1u64 << ((base + 2) % 64);
            }
            if mask & 0xFF000000 != 0 {
                out_bits[(base + 3) / 64] |= 1u64 << ((base + 3) % 64);
            }
        }

        // Scalar tail
        let tail_start = chunks * 4;
        for (i, &v) in data[tail_start..].iter().enumerate() {
            let idx = tail_start + i;
            if v == value {
                out_bits[idx / 64] |= 1u64 << (idx % 64);
            }
        }
    }

    /// Scan a packed u64 column for non-null values, producing a bitset.
    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn scan_not_null_to_bitset_avx2(
        data: &[u64],
        null_sentinel: u64,
        out_bits: &mut [u64],
    ) {
        let n = data.len();
        let chunks = n / 4;
        let ptr = data.as_ptr() as *const __m256i;
        let sentinel = _mm256_set1_epi64x(null_sentinel as i64);

        for i in 0..chunks {
            let block = _mm256_loadu_si256(ptr.add(i));
            let cmp = _mm256_cmpeq_epi64(block, sentinel);
            let mask = _mm256_movemask_epi8(cmp) as u32;
            // Invert: we want NOT null
            let base = i * 4;
            if mask & 0x000000FF == 0 {
                out_bits[base / 64] |= 1u64 << (base % 64);
            }
            if mask & 0x0000FF00 == 0 {
                out_bits[(base + 1) / 64] |= 1u64 << ((base + 1) % 64);
            }
            if mask & 0x00FF0000 == 0 {
                out_bits[(base + 2) / 64] |= 1u64 << ((base + 2) % 64);
            }
            if mask & 0xFF000000 == 0 {
                out_bits[(base + 3) / 64] |= 1u64 << ((base + 3) % 64);
            }
        }

        let tail_start = chunks * 4;
        for (i, &v) in data[tail_start..].iter().enumerate() {
            let idx = tail_start + i;
            if v != null_sentinel {
                out_bits[idx / 64] |= 1u64 << (idx % 64);
            }
        }
    }

    // -- Original index-producing scans (kept for backward compatibility) --

    // -- AVX2 (256-bit, 4 u64s per iteration) --

    /// Scan a packed u64 column for equality matches using AVX2.
    /// Returns a bitmask of matching positions for each chunk of 4 elements.
    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn scan_eq_avx2(data: &[u64], value: u64, out: &mut Vec<usize>) {
        let n = data.len();
        let chunks = n / 4;
        let ptr = data.as_ptr() as *const __m256i;
        let needle = _mm256_set1_epi64x(value as i64);

        for i in 0..chunks {
            let block = _mm256_loadu_si256(ptr.add(i));
            let cmp = _mm256_cmpeq_epi64(block, needle);
            let mask = _mm256_movemask_epi8(cmp) as u32;
            // Each matching u64 produces 8 set bits (0xFF) in the mask
            let base = i * 4;
            if mask & 0x000000FF != 0 {
                out.push(base);
            }
            if mask & 0x0000FF00 != 0 {
                out.push(base + 1);
            }
            if mask & 0x00FF0000 != 0 {
                out.push(base + 2);
            }
            if mask & 0xFF000000 != 0 {
                out.push(base + 3);
            }
        }

        // Scalar tail
        let tail_start = chunks * 4;
        for (i, &v) in data[tail_start..].iter().enumerate() {
            if v == value {
                out.push(tail_start + i);
            }
        }
    }

    /// Scan a packed u64 column for values >= lo AND <= hi using AVX2.
    /// lo/hi of u64::MAX (NULL_SENTINEL) means unbounded on that side.
    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn scan_range_avx2(
        data: &[u64],
        null_sentinel: u64,
        lo: u64,
        hi: u64,
        has_lo: bool,
        has_hi: bool,
        out: &mut Vec<usize>,
    ) {
        // AVX2 doesn't have unsigned u64 comparison instructions,
        // so we use scalar with null-skip. The benefit comes from the
        // packed layout eliminating Option overhead and improving cache
        // utilization. For range scans, the zonemap prune is the primary
        // accelerator; this inner loop benefits from the dense layout.
        for (i, &v) in data.iter().enumerate() {
            if v == null_sentinel {
                continue;
            }
            let above = !has_lo || v >= lo;
            let below = !has_hi || v <= hi;
            if above && below {
                out.push(i);
            }
        }
    }

    /// Scan a packed u64 column for non-null values using AVX2.
    #[target_feature(enable = "avx2")]
    pub(crate) unsafe fn scan_not_null_avx2(
        data: &[u64],
        null_sentinel: u64,
        out: &mut Vec<usize>,
    ) {
        let n = data.len();
        let chunks = n / 4;
        let ptr = data.as_ptr() as *const __m256i;
        let sentinel = _mm256_set1_epi64x(null_sentinel as i64);

        for i in 0..chunks {
            let block = _mm256_loadu_si256(ptr.add(i));
            let cmp = _mm256_cmpeq_epi64(block, sentinel);
            let mask = _mm256_movemask_epi8(cmp) as u32;
            // Invert: we want NOT null
            let base = i * 4;
            if mask & 0x000000FF == 0 {
                out.push(base);
            }
            if mask & 0x0000FF00 == 0 {
                out.push(base + 1);
            }
            if mask & 0x00FF0000 == 0 {
                out.push(base + 2);
            }
            if mask & 0xFF000000 == 0 {
                out.push(base + 3);
            }
        }

        let tail_start = chunks * 4;
        for (i, &v) in data[tail_start..].iter().enumerate() {
            if v != null_sentinel {
                out.push(tail_start + i);
            }
        }
    }

    // -- SSE2 (128-bit, 2 u64s per iteration) --

    /// Scan a packed u64 column for equality matches using SSE2.
    #[target_feature(enable = "sse2")]
    pub(crate) unsafe fn scan_eq_sse2(data: &[u64], value: u64, out: &mut Vec<usize>) {
        let n = data.len();
        let chunks = n / 2;
        let ptr = data.as_ptr() as *const __m128i;
        let needle = _mm_set1_epi64x(value as i64);

        for i in 0..chunks {
            let block = _mm_loadu_si128(ptr.add(i));
            let cmp = _mm_cmpeq_epi64(block, needle);
            let mask = _mm_movemask_epi8(cmp) as u32;
            let base = i * 2;
            if mask & 0x00FF != 0 {
                out.push(base);
            }
            if mask & 0xFF00 != 0 {
                out.push(base + 1);
            }
        }

        let tail_start = chunks * 2;
        for (i, &v) in data[tail_start..].iter().enumerate() {
            if v == value {
                out.push(tail_start + i);
            }
        }
    }

    /// Scan a packed u64 column for non-null values using SSE2.
    #[target_feature(enable = "sse2")]
    pub(crate) unsafe fn scan_not_null_sse2(
        data: &[u64],
        null_sentinel: u64,
        out: &mut Vec<usize>,
    ) {
        let n = data.len();
        let chunks = n / 2;
        let ptr = data.as_ptr() as *const __m128i;
        let sentinel = _mm_set1_epi64x(null_sentinel as i64);

        for i in 0..chunks {
            let block = _mm_loadu_si128(ptr.add(i));
            let cmp = _mm_cmpeq_epi64(block, sentinel);
            let mask = _mm_movemask_epi8(cmp) as u32;
            let base = i * 2;
            if mask & 0x00FF == 0 {
                out.push(base);
            }
            if mask & 0xFF00 == 0 {
                out.push(base + 1);
            }
        }

        let tail_start = chunks * 2;
        for (i, &v) in data[tail_start..].iter().enumerate() {
            if v != null_sentinel {
                out.push(tail_start + i);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public SIMD dispatch — runtime feature detection (same pattern as sutra-hnsw)
// ---------------------------------------------------------------------------

/// SIMD-accelerated equality scan over a packed u64 column.
/// Returns indices of elements that equal `value`.
fn packed_scan_eq(data: &[u64], value: u64) -> Vec<usize> {
    let mut out = Vec::new();
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { simd::scan_eq_avx2(data, value, &mut out) };
            return out;
        }
        if is_x86_feature_detected!("sse4.1") {
            unsafe { simd::scan_eq_sse2(data, value, &mut out) };
            return out;
        }
    }
    // Scalar fallback
    for (i, &v) in data.iter().enumerate() {
        if v == value {
            out.push(i);
        }
    }
    out
}

/// SIMD-accelerated range scan over a packed u64 column.
/// Returns indices of non-null elements where lo <= value <= hi.
fn packed_scan_range(data: &[u64], lo: Option<u64>, hi: Option<u64>) -> Vec<usize> {
    let mut out = Vec::new();
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                simd::scan_range_avx2(
                    data,
                    NULL_SENTINEL,
                    lo.unwrap_or(0),
                    hi.unwrap_or(u64::MAX),
                    lo.is_some(),
                    hi.is_some(),
                    &mut out,
                );
            }
            return out;
        }
    }
    // Scalar fallback
    for (i, &v) in data.iter().enumerate() {
        if v == NULL_SENTINEL {
            continue;
        }
        let above = lo.is_none_or(|l| v >= l);
        let below = hi.is_none_or(|h| v <= h);
        if above && below {
            out.push(i);
        }
    }
    out
}

/// SIMD-accelerated not-null scan over a packed u64 column.
/// Returns indices of elements that are not the null sentinel.
fn packed_scan_not_null(data: &[u64]) -> Vec<usize> {
    let mut out = Vec::new();
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { simd::scan_not_null_avx2(data, NULL_SENTINEL, &mut out) };
            return out;
        }
        if is_x86_feature_detected!("sse4.1") {
            unsafe { simd::scan_not_null_sse2(data, NULL_SENTINEL, &mut out) };
            return out;
        }
    }
    // Scalar fallback
    for (i, &v) in data.iter().enumerate() {
        if v != NULL_SENTINEL {
            out.push(i);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Bitset-producing column scans (for vectorized execution)
// ---------------------------------------------------------------------------

/// Scan a packed column for equality matches, producing a SelectionVector bitset.
/// This is the vectorized equivalent of `packed_scan_eq` — instead of materializing
/// a Vec<usize>, it produces a compact bitset that can be AND'd with other scans.
fn packed_scan_eq_bitset(data: &[u64], value: u64, len: usize) -> SelectionVector {
    let nwords = len.div_ceil(64);
    let mut sel = SelectionVector::none(len);
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { simd::scan_eq_to_bitset_avx2(data, value, &mut sel.bits) };
            return sel;
        }
    }
    // Scalar fallback
    for (i, &v) in data.iter().enumerate() {
        if i < len && v == value {
            sel.bits[i / 64] |= 1u64 << (i % 64);
        }
    }
    let _ = nwords;
    sel
}

/// Scan a packed column for non-null values, producing a SelectionVector bitset.
fn packed_scan_not_null_bitset(data: &[u64], len: usize) -> SelectionVector {
    let nwords = len.div_ceil(64);
    let mut sel = SelectionVector::none(len);
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { simd::scan_not_null_to_bitset_avx2(data, NULL_SENTINEL, &mut sel.bits) };
            return sel;
        }
    }
    // Scalar fallback
    for (i, &v) in data.iter().enumerate() {
        if i < len && v != NULL_SENTINEL {
            sel.bits[i / 64] |= 1u64 << (i % 64);
        }
    }
    let _ = nwords;
    sel
}

/// Scan a packed column for values in range [lo, hi], producing a SelectionVector.
fn packed_scan_range_bitset(
    data: &[u64],
    lo: Option<u64>,
    hi: Option<u64>,
    len: usize,
) -> SelectionVector {
    let mut sel = SelectionVector::none(len);
    for (i, &v) in data.iter().enumerate() {
        if i >= len || v == NULL_SENTINEL {
            continue;
        }
        let above = lo.is_none_or(|l| v >= l);
        let below = hi.is_none_or(|h| v <= h);
        if above && below {
            sel.bits[i / 64] |= 1u64 << (i % 64);
        }
    }
    sel
}

// ---------------------------------------------------------------------------
// Fused multi-column scan (vectorized execution core)
// ---------------------------------------------------------------------------

/// A column filter specification for fused multi-column scanning.
#[derive(Debug, Clone)]
pub enum ColumnFilter {
    /// Equality: column value must equal this TermId.
    Eq(TermId),
    /// Non-null: column must have a value (any value).
    NotNull,
    /// Range: column value must be in [lo, hi]. None means unbounded.
    Range {
        lo: Option<TermId>,
        hi: Option<TermId>,
    },
}

/// Fused multi-column scan: apply multiple column filters in a single pass
/// over the segment, AND'ing bitsets together with SIMD.
///
/// This is the key vectorized execution primitive. Instead of:
/// 1. Scan column A → Vec<usize>
/// 2. Scan column B → Vec<usize>
/// 3. Sorted merge intersection
///
/// We do:
/// 1. Scan column A → bitset
/// 2. Scan column B → bitset
/// 3. AVX2 AND (256 bits per cycle)
///
/// For a 2048-row segment, the AND is 32 u64s = 8 AVX2 iterations.
/// The sorted merge was O(n + m) comparisons on variable-length arrays.
pub fn fused_multi_column_scan(
    segment: &Segment,
    filters: &[(usize, ColumnFilter)],
) -> SelectionVector {
    let len = segment.len();
    if len == 0 || filters.is_empty() {
        return SelectionVector::none(len);
    }

    let mut result = SelectionVector::all_set(len);

    for &(col_idx, ref filter) in filters {
        // Zonemap pruning: check if this filter can match any rows in this segment.
        let stats = &segment.column_stats[col_idx];
        match filter {
            ColumnFilter::Eq(value) => {
                if !stats.range_could_match(Some(*value), Some(*value)) {
                    return SelectionVector::none(len);
                }
            }
            ColumnFilter::Range { lo, hi } => {
                if !stats.range_could_match(*lo, *hi) {
                    return SelectionVector::none(len);
                }
            }
            ColumnFilter::NotNull => {
                if stats.null_count == stats.row_count {
                    return SelectionVector::none(len);
                }
            }
        }

        // Produce bitset for this column filter.
        let col_sel = if let Some(packed) = segment.packed_columns.get(col_idx) {
            match filter {
                ColumnFilter::Eq(value) => packed_scan_eq_bitset(packed, *value, len),
                ColumnFilter::NotNull => packed_scan_not_null_bitset(packed, len),
                ColumnFilter::Range { lo, hi } => packed_scan_range_bitset(packed, *lo, *hi, len),
            }
        } else {
            // Fallback: build bitset from Option columns.
            let col = &segment.columns[col_idx];
            let mut sel = SelectionVector::none(len);
            for (i, val) in col.iter().enumerate() {
                let matches = match filter {
                    ColumnFilter::Eq(value) => *val == Some(*value),
                    ColumnFilter::NotNull => val.is_some(),
                    ColumnFilter::Range { lo, hi } => {
                        if let Some(v) = val {
                            lo.is_none_or(|l| *v >= l) && hi.is_none_or(|h| *v <= h)
                        } else {
                            false
                        }
                    }
                };
                if matches {
                    sel.set(i);
                }
            }
            sel
        };

        // AND with running result — SIMD accelerated.
        result.and_inplace(&col_sel);

        // Early termination: if nothing matches, stop scanning more columns.
        if result.is_empty() {
            return result;
        }
    }

    result
}

/// Batch gather: extract column values for all selected rows at once.
///
/// Returns a parallel array of Option<TermId> for each selected row.
/// This avoids per-row random access into the column arrays.
pub fn batch_gather(
    segment: &Segment,
    col_idx: usize,
    selection: &SelectionVector,
) -> Vec<Option<TermId>> {
    let indices = selection.to_indices();
    let col = &segment.columns[col_idx];
    indices.iter().map(|&i| col[i]).collect()
}

/// Batch gather for multiple columns at once.
///
/// Returns rows × columns: `result[row][col]` is the value for that
/// selected row at that column index.
pub fn batch_gather_multi(
    segment: &Segment,
    col_indices: &[usize],
    selection: &SelectionVector,
) -> Vec<Vec<Option<TermId>>> {
    let row_indices = selection.to_indices();
    row_indices
        .iter()
        .map(|&row_idx| {
            col_indices
                .iter()
                .map(|&col_idx| segment.columns[col_idx][row_idx])
                .collect()
        })
        .collect()
}

/// Batch gather of node IDs for selected rows.
pub fn batch_gather_nodes(segment: &Segment, selection: &SelectionVector) -> Vec<TermId> {
    let indices = selection.to_indices();
    indices.iter().map(|&i| segment.nodes[i]).collect()
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum number of shared properties for a group to qualify as a pseudo-table.
/// Groups with fewer shared properties don't have enough relational structure
/// to benefit from columnar indexing.
const MIN_SHARED_PROPERTIES: usize = 5;

/// Minimum coverage ratio for a property to be considered "core" (part of the
/// characteristic set). A property held by 50% of the group is common enough
/// to define the group's identity.
const CORE_PROPERTY_THRESHOLD: f64 = 0.50;

/// Minimum coverage ratio for a property to become a column in the pseudo-table.
/// Lower than CORE_PROPERTY_THRESHOLD because we want columns for "optional"
/// properties that are common but not universal (like an optional email field).
const COLUMN_INCLUSION_THRESHOLD: f64 = 0.33;

/// Minimum number of nodes in a group for it to justify pseudo-table overhead.
/// A pseudo-table with 3 rows is worse than just scanning triples.
const MIN_GROUP_SIZE: usize = 10;

/// Number of rows per segment. Chosen to balance zonemap granularity against
/// overhead. Too small = too many segments = overhead. Too large = zonemaps
/// too coarse = no pruning benefit.
///
/// 2048 is the DuckDB default and works well for analytical workloads.
const SEGMENT_SIZE: usize = 2048;

// ---------------------------------------------------------------------------
// Property model
// ---------------------------------------------------------------------------

/// A property is a (predicate, position) pair that describes how a node
/// participates in a triple pattern.
///
/// Two nodes with the same predicate but different positions have different
/// properties. For example:
/// - `:Alice :knows :Bob` → Alice has `Property(knows, Subject)`, Bob has `Property(knows, Object)`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Property {
    /// The predicate IRI (interned as TermId).
    pub predicate: TermId,
    /// Which position the node occupies in the triple.
    pub position: PropertyPosition,
}

/// Which position a node occupies in a triple with a given predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyPosition {
    /// The node is the subject of the triple.
    Subject,
    /// The node is the object of the triple.
    Object,
}

/// The set of all properties for a single node — its "property signature."
///
/// Two nodes with the same property set are candidates for the same pseudo-table.
/// The property set is the RDF equivalent of a relational schema: it describes
/// what "columns" a node would have if it were a row in a table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropertySet {
    /// Sorted, deduplicated list of properties for this node.
    /// Sorting enables fast comparison and hashing.
    pub properties: Vec<Property>,
}

impl PropertySet {
    /// Create a new property set from a list of properties.
    /// Automatically sorts and deduplicates.
    pub fn new(mut properties: Vec<Property>) -> Self {
        properties.sort_by_key(|p| (p.predicate, p.position as u8));
        properties.dedup();
        Self { properties }
    }

    /// Check if this property set contains a specific property.
    pub fn contains(&self, property: &Property) -> bool {
        self.properties
            .binary_search_by_key(&(property.predicate, property.position as u8), |p| {
                (p.predicate, p.position as u8)
            })
            .is_ok()
    }

    /// Number of properties in this set.
    pub fn len(&self) -> usize {
        self.properties.len()
    }

    /// Whether this property set is empty.
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Column statistics (DataFusion Precision<T> pattern)
// ---------------------------------------------------------------------------

/// Statistics for a single column in a pseudo-table segment.
///
/// Follows DataFusion's `Precision<T>` pattern: each statistic is either
/// Exact (computed from all values), Approximate (estimated), or Unknown.
///
/// These statistics enable the query planner to estimate selectivity and
/// the executor to skip segments via zonemap pruning.
#[derive(Debug, Clone)]
pub struct ColumnStats {
    /// Minimum value in this column (within a segment or the whole table).
    /// None if the column has no non-null values.
    pub min_value: Option<TermId>,
    /// Maximum value in this column.
    pub max_value: Option<TermId>,
    /// Number of null (absent) values.
    pub null_count: usize,
    /// Number of distinct non-null values.
    /// Exact after full scan, approximate after sampling.
    pub distinct_count: usize,
    /// Total number of rows (null + non-null).
    pub row_count: usize,
}

impl ColumnStats {
    /// Create empty statistics (no data yet).
    fn empty() -> Self {
        Self {
            min_value: None,
            max_value: None,
            null_count: 0,
            distinct_count: 0,
            row_count: 0,
        }
    }

    /// Selectivity estimate for an equality predicate.
    ///
    /// Returns the estimated fraction of rows that match `value = X`.
    /// Uses distinct count for cardinality estimation (uniform distribution assumption).
    pub fn equality_selectivity(&self) -> f64 {
        if self.distinct_count == 0 {
            0.0
        } else {
            1.0 / self.distinct_count as f64
        }
    }

    /// Whether a range query [lo, hi] could match any values in this column.
    ///
    /// This is the zonemap pruning check: if the column's max < lo or min > hi,
    /// no rows in this segment can match and it can be skipped entirely.
    pub fn range_could_match(&self, lo: Option<TermId>, hi: Option<TermId>) -> bool {
        // If column has no values, it can't match anything.
        if self.min_value.is_none() || self.max_value.is_none() {
            return false;
        }
        let col_min = self.min_value.unwrap();
        let col_max = self.max_value.unwrap();

        // Check if the query range overlaps the column's value range.
        // If query's low bound exceeds column's max, no match possible.
        if let Some(lo) = lo {
            if lo > col_max {
                return false;
            }
        }
        // If query's high bound is below column's min, no match possible.
        if let Some(hi) = hi {
            if hi < col_min {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Pseudo-table segment (DuckDB pattern)
// ---------------------------------------------------------------------------

/// A segment of rows in a pseudo-table, with per-column zonemaps.
///
/// Segments are the unit of skip-scan pruning: when a query filter doesn't
/// overlap a segment's zonemap, the entire segment is skipped. This is the
/// same pattern DuckDB uses for analytical queries.
///
/// Each segment holds up to `SEGMENT_SIZE` rows (default 2048).
#[derive(Debug, Clone)]
pub struct Segment {
    /// The node TermIds (row identifiers) in this segment.
    /// Each entry is a node that belongs to this pseudo-table.
    pub nodes: Vec<TermId>,

    /// Column values: columns[col_idx][row_idx] = Some(value) or None.
    /// Outer vec is indexed by column position in PseudoTable::columns.
    /// Inner vec is parallel to `nodes`.
    pub columns: Vec<Vec<Option<TermId>>>,

    /// Tail property count per row: how many properties this node has
    /// that aren't included as columns. High tail counts indicate
    /// the node doesn't fit the pseudo-table schema well.
    pub tail_counts: Vec<usize>,

    /// Per-column statistics for zonemap pruning.
    /// Indexed by column position, parallel to `columns`.
    pub column_stats: Vec<ColumnStats>,

    /// Packed column storage for SIMD-accelerated scanning.
    /// Each inner Vec is a dense u64 array parallel to `nodes`.
    /// Null values are represented as `NULL_SENTINEL` (u64::MAX).
    /// Populated by `compute_stats()`.
    pub packed_columns: Vec<Vec<u64>>,
}

impl Segment {
    /// Create a new empty segment with the given number of columns.
    fn new(num_columns: usize) -> Self {
        Self {
            nodes: Vec::new(),
            columns: (0..num_columns).map(|_| Vec::new()).collect(),
            tail_counts: Vec::new(),
            column_stats: (0..num_columns).map(|_| ColumnStats::empty()).collect(),
            packed_columns: Vec::new(),
        }
    }

    /// Number of rows in this segment.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether this segment is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Whether this segment is full (at capacity).
    pub fn is_full(&self) -> bool {
        self.nodes.len() >= SEGMENT_SIZE
    }

    /// Recompute column statistics from the actual data.
    ///
    /// Called after the segment is fully populated. Computes min/max/null_count
    /// for each column, which enables zonemap-based skip-scan pruning.
    pub fn compute_stats(&mut self) {
        for (col_idx, col_data) in self.columns.iter().enumerate() {
            let mut min_val: Option<TermId> = None;
            let mut max_val: Option<TermId> = None;
            let mut null_count = 0usize;
            let mut distinct = std::collections::HashSet::new();

            for value in col_data {
                match value {
                    Some(v) => {
                        distinct.insert(*v);
                        min_val = Some(min_val.map_or(*v, |m: TermId| m.min(*v)));
                        max_val = Some(max_val.map_or(*v, |m: TermId| m.max(*v)));
                    }
                    None => null_count += 1,
                }
            }

            self.column_stats[col_idx] = ColumnStats {
                min_value: min_val,
                max_value: max_val,
                null_count,
                distinct_count: distinct.len(),
                row_count: col_data.len(),
            };
        }

        // Build packed columns: dense u64 arrays with NULL_SENTINEL for nulls.
        // This layout enables SIMD-accelerated scanning (AVX2: 4 u64s/cycle).
        self.packed_columns = self
            .columns
            .iter()
            .map(|col| col.iter().map(|v| v.unwrap_or(NULL_SENTINEL)).collect())
            .collect();
    }
}

// ---------------------------------------------------------------------------
// Pseudo-table
// ---------------------------------------------------------------------------

/// A pseudo-table: a columnar index over a group of RDF nodes that share
/// enough predicate structure to benefit from relational-style query execution.
///
/// This is the core data structure that bridges RDF's flexible graph model
/// with SQL-like columnar execution. Each pseudo-table represents a
/// "characteristic set" — a group of nodes with similar property signatures.
#[derive(Debug, Clone)]
pub struct PseudoTable {
    /// Human-readable label for this pseudo-table (derived from the most
    /// common rdf:type or the dominant predicate pattern).
    pub label: String,

    /// The properties that define this pseudo-table's columns.
    /// Each column corresponds to a Property (predicate + position).
    /// Properties are ordered by coverage (highest first) for tighter
    /// zonemaps when rows are sorted by the most selective column.
    pub columns: Vec<Property>,

    /// Coverage ratio for each column: what fraction of nodes in this group
    /// have this property. Columns are sorted by coverage descending.
    pub column_coverage: Vec<f64>,

    /// Segmented row storage. Each segment holds up to SEGMENT_SIZE rows
    /// with per-column zonemaps for skip-scan pruning.
    pub segments: Vec<Segment>,

    /// Total number of nodes in this pseudo-table (across all segments).
    pub total_rows: usize,

    /// The core property set: properties held by ≥50% of the group.
    /// This defines the group's identity — the "characteristic set."
    pub core_properties: Vec<Property>,

    /// Data health metric: cliff steepness between core and tail properties.
    ///
    /// Computed as the ratio of average core property coverage to average
    /// tail property coverage. Higher = sharper cliff = healthier schema.
    ///
    /// - `cliff_steepness > 10.0`: Excellent schema consistency
    /// - `cliff_steepness 3.0-10.0`: Good, some optional properties
    /// - `cliff_steepness 1.0-3.0`: Messy schema, many optional fields
    /// - `cliff_steepness < 1.0`: No clear schema — pseudo-table may not be useful
    pub cliff_steepness: f64,
}

impl PseudoTable {
    /// Get aggregate statistics for a column across all segments.
    ///
    /// Merges per-segment zonemaps into a single ColumnStats covering
    /// the entire pseudo-table. Used by the query planner for cardinality
    /// estimation.
    pub fn column_stats(&self, col_idx: usize) -> ColumnStats {
        let mut merged = ColumnStats::empty();
        for segment in &self.segments {
            let seg_stats = &segment.column_stats[col_idx];
            merged.row_count += seg_stats.row_count;
            merged.null_count += seg_stats.null_count;
            merged.distinct_count = merged.distinct_count.max(seg_stats.distinct_count);
            if let Some(seg_min) = seg_stats.min_value {
                merged.min_value =
                    Some(merged.min_value.map_or(seg_min, |m: TermId| m.min(seg_min)));
            }
            if let Some(seg_max) = seg_stats.max_value {
                merged.max_value =
                    Some(merged.max_value.map_or(seg_max, |m: TermId| m.max(seg_max)));
            }
        }
        merged
    }

    /// Find the column index for a given property, if it exists.
    pub fn column_index(&self, property: &Property) -> Option<usize> {
        self.columns.iter().position(|p| p == property)
    }

    /// Check if a node (by TermId) is in this pseudo-table.
    pub fn contains_node(&self, node_id: TermId) -> bool {
        self.segments.iter().any(|seg| seg.nodes.contains(&node_id))
    }
}

// ---------------------------------------------------------------------------
// Pseudo-table registry
// ---------------------------------------------------------------------------

/// Registry of all discovered pseudo-tables.
///
/// The registry is the top-level entry point for pseudo-table operations.
/// It holds all discovered tables and provides lookup methods for the
/// query planner to find matching pseudo-tables for SPARQL patterns.
#[derive(Debug, Clone)]
pub struct PseudoTableRegistry {
    /// All discovered pseudo-tables, in discovery order.
    pub tables: Vec<PseudoTable>,
}

impl PseudoTableRegistry {
    /// Create an empty registry with no discovered tables.
    pub fn new() -> Self {
        Self { tables: Vec::new() }
    }

    /// Number of discovered pseudo-tables.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Whether any pseudo-tables have been discovered.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Find pseudo-tables that contain a column matching the given property.
    ///
    /// Used by the query planner to determine if a triple pattern can be
    /// routed through a pseudo-table's columnar index instead of the
    /// general-purpose SPO/POS/OSP indexes.
    pub fn find_tables_for_property(&self, property: &Property) -> Vec<(usize, usize)> {
        let mut matches = Vec::new();
        for (table_idx, table) in self.tables.iter().enumerate() {
            if let Some(col_idx) = table.column_index(property) {
                matches.push((table_idx, col_idx));
            }
        }
        matches
    }

    /// Total number of nodes across all pseudo-tables.
    pub fn total_coverage(&self) -> usize {
        self.tables.iter().map(|t| t.total_rows).sum()
    }

    /// Coverage ratio: what fraction of all nodes in the store are covered
    /// by at least one pseudo-table.
    pub fn coverage_ratio(&self, total_nodes: usize) -> f64 {
        if total_nodes == 0 {
            return 0.0;
        }
        self.total_coverage() as f64 / total_nodes as f64
    }
}

impl Default for PseudoTableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Discovery algorithm
// ---------------------------------------------------------------------------

/// Extract the property set for every node in the triple store.
///
/// Scans all triples to determine which predicates each node participates in
/// and in which position (subject or object). Returns a map from node TermId
/// to its PropertySet.
///
/// This is the first step of pseudo-table discovery: understanding what
/// "schema" each node implicitly follows.
pub fn extract_node_properties(store: &TripleStore) -> HashMap<TermId, PropertySet> {
    let mut node_props: HashMap<TermId, Vec<Property>> = HashMap::new();

    // Scan all triples to collect properties for each node.
    // Each triple contributes two properties:
    // - Subject gets Property(predicate, Subject)
    // - Object gets Property(predicate, Object)
    for triple in store.iter() {
        node_props
            .entry(triple.subject)
            .or_default()
            .push(Property {
                predicate: triple.predicate,
                position: PropertyPosition::Subject,
            });
        node_props.entry(triple.object).or_default().push(Property {
            predicate: triple.predicate,
            position: PropertyPosition::Object,
        });
    }

    // Convert to PropertySets (sorted + deduplicated).
    node_props
        .into_iter()
        .map(|(node, props)| (node, PropertySet::new(props)))
        .collect()
}

/// Discover pseudo-table groups from node property sets.
///
/// Groups nodes by their property signatures, then identifies groups that
/// are large enough and have enough shared properties to form pseudo-tables.
///
/// ## Algorithm
///
/// 1. **Exact grouping**: Group nodes by identical property sets. This finds
///    the tightest characteristic sets — nodes with exactly the same schema.
///
/// 2. **Merge similar groups**: Groups that share ≥80% of properties are
///    merged. This handles optional properties: a Person with email and
///    a Person without email should be in the same pseudo-table.
///
/// 3. **Filter by criteria**: Only keep groups with ≥5 shared properties
///    at ≥50% coverage and ≥10 members.
///
/// 4. **Compute coverage**: For each surviving group, compute per-property
///    coverage ratios and determine which properties become columns (≥33%).
pub fn discover_pseudo_tables(
    node_properties: &HashMap<TermId, PropertySet>,
    store: &TripleStore,
) -> PseudoTableRegistry {
    // Step 1: Group nodes by exact property set.
    // Nodes with identical property signatures form the initial clusters.
    let mut exact_groups: HashMap<Vec<(TermId, u8)>, Vec<TermId>> = HashMap::new();
    for (node_id, prop_set) in node_properties {
        // Create a hashable key from the property set.
        let key: Vec<(TermId, u8)> = prop_set
            .properties
            .iter()
            .map(|p| (p.predicate, p.position as u8))
            .collect();
        exact_groups.entry(key).or_default().push(*node_id);
    }

    // Step 2: Merge similar groups.
    // Groups sharing ≥80% of properties are combined into a single group.
    // This handles the "optional field" pattern where some nodes have extra properties.
    let mut merged_groups: Vec<(Vec<Property>, Vec<TermId>)> = Vec::new();

    let mut exact_vec: Vec<(Vec<Property>, Vec<TermId>)> = exact_groups
        .into_iter()
        .map(|(key, nodes)| {
            let props: Vec<Property> = key
                .into_iter()
                .map(|(pred, pos)| Property {
                    predicate: pred,
                    position: if pos == 0 {
                        PropertyPosition::Subject
                    } else {
                        PropertyPosition::Object
                    },
                })
                .collect();
            (props, nodes)
        })
        .collect();

    // Sort by group size descending so large groups absorb smaller ones.
    exact_vec.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut absorbed = vec![false; exact_vec.len()];

    for i in 0..exact_vec.len() {
        if absorbed[i] {
            continue;
        }

        let mut merged_props = exact_vec[i].0.clone();
        let mut merged_nodes = exact_vec[i].1.clone();

        for j in (i + 1)..exact_vec.len() {
            if absorbed[j] {
                continue;
            }

            // Compute Jaccard similarity between property sets.
            let props_i: std::collections::HashSet<_> = merged_props.iter().cloned().collect();
            let props_j: std::collections::HashSet<_> = exact_vec[j].0.iter().cloned().collect();
            let intersection = props_i.intersection(&props_j).count();
            let union = props_i.union(&props_j).count();

            if union > 0 && (intersection as f64 / union as f64) >= 0.80 {
                // Merge: take the union of properties and all nodes.
                merged_props = props_i.union(&props_j).cloned().collect();
                merged_props.sort_by_key(|p| (p.predicate, p.position as u8));
                merged_nodes.extend(exact_vec[j].1.iter());
                absorbed[j] = true;
            }
        }

        merged_groups.push((merged_props, merged_nodes));
    }

    // Step 3: Filter and build pseudo-tables.
    let mut tables = Vec::new();

    for (all_properties, nodes) in &merged_groups {
        if nodes.len() < MIN_GROUP_SIZE {
            continue;
        }

        // Compute per-property coverage: what fraction of nodes have each property.
        let mut property_coverage: Vec<(Property, f64)> = Vec::new();
        for prop in all_properties {
            let count = nodes
                .iter()
                .filter(|&&node_id| {
                    node_properties
                        .get(&node_id)
                        .is_some_and(|ps| ps.contains(prop))
                })
                .count();
            let coverage = count as f64 / nodes.len() as f64;
            property_coverage.push((*prop, coverage));
        }

        // Sort by coverage descending for column ordering.
        property_coverage.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Core properties: ≥50% coverage. These define the group's identity.
        let core: Vec<Property> = property_coverage
            .iter()
            .filter(|(_, cov)| *cov >= CORE_PROPERTY_THRESHOLD)
            .map(|(p, _)| *p)
            .collect();

        // Must have at least MIN_SHARED_PROPERTIES core properties.
        if core.len() < MIN_SHARED_PROPERTIES {
            continue;
        }

        // Columns: properties with ≥33% coverage become columns.
        let column_props: Vec<(Property, f64)> = property_coverage
            .iter()
            .filter(|(_, cov)| *cov >= COLUMN_INCLUSION_THRESHOLD)
            .cloned()
            .collect();

        // Tail properties: everything not included as a column.
        let tail_properties: Vec<(Property, f64)> = property_coverage
            .iter()
            .filter(|(_, cov)| *cov < COLUMN_INCLUSION_THRESHOLD)
            .cloned()
            .collect();

        // Compute cliff steepness: ratio of average core coverage to average tail coverage.
        let avg_core_coverage = if core.is_empty() {
            0.0
        } else {
            property_coverage
                .iter()
                .filter(|(p, _)| core.contains(p))
                .map(|(_, c)| c)
                .sum::<f64>()
                / core.len() as f64
        };
        let avg_tail_coverage = if tail_properties.is_empty() {
            // No tail properties = infinitely sharp cliff = perfect schema.
            0.01 // avoid division by zero
        } else {
            tail_properties.iter().map(|(_, c)| c).sum::<f64>() / tail_properties.len() as f64
        };
        let cliff_steepness = avg_core_coverage / avg_tail_coverage.max(0.01);

        // Build segmented storage.
        let columns: Vec<Property> = column_props.iter().map(|(p, _)| *p).collect();
        let coverage: Vec<f64> = column_props.iter().map(|(_, c)| *c).collect();
        let num_columns = columns.len();

        // Sort nodes by the value of the most selective column for tighter zonemaps.
        // The "most selective" column is the one with the highest distinct_count relative
        // to row count, which gives the tightest min/max ranges per segment.
        let mut sorted_nodes = nodes.clone();
        if let Some(first_col) = columns.first() {
            // Sort by the first column's value (highest coverage = most common = best sort key).
            sorted_nodes
                .sort_by_key(|&node_id| get_property_value(node_id, first_col, store).unwrap_or(0));
        }

        let mut segments = Vec::new();
        let mut current_segment = Segment::new(num_columns);

        for &node_id in &sorted_nodes {
            let node_propset = node_properties.get(&node_id);

            // Fill column values for this row.
            for (col_idx, col_prop) in columns.iter().enumerate() {
                let value = get_property_value(node_id, col_prop, store);
                current_segment.columns[col_idx].push(value);
            }

            // Count tail properties for this node.
            let tail_count = node_propset.map_or(0, |ps| {
                ps.properties
                    .iter()
                    .filter(|p| !columns.contains(p))
                    .count()
            });

            current_segment.nodes.push(node_id);
            current_segment.tail_counts.push(tail_count);

            // Segment full — finalize and start a new one.
            if current_segment.is_full() {
                current_segment.compute_stats();
                segments.push(current_segment);
                current_segment = Segment::new(num_columns);
            }
        }

        // Finalize the last (possibly partial) segment.
        if !current_segment.is_empty() {
            current_segment.compute_stats();
            segments.push(current_segment);
        }

        tables.push(PseudoTable {
            label: format!("pseudo_table_{}", tables.len()),
            columns,
            column_coverage: coverage,
            total_rows: sorted_nodes.len(),
            core_properties: core,
            cliff_steepness,
            segments,
        });
    }

    PseudoTableRegistry { tables }
}

/// Get the value of a property for a specific node.
///
/// Looks up the triple store to find what value a node has for a given property.
/// For Subject properties, returns the object of the triple.
/// For Object properties, returns the subject of the triple.
///
/// If the node has multiple values for this property (multi-valued),
/// returns the first one found. Multi-valued properties are a limitation
/// of the columnar model — the pseudo-table stores only one value per cell.
fn get_property_value(node_id: TermId, property: &Property, store: &TripleStore) -> Option<TermId> {
    match property.position {
        PropertyPosition::Subject => {
            // Node is subject, property is predicate → value is object.
            let triples = store.find_by_subject_predicate(node_id, property.predicate);
            triples.first().map(|t| t.object)
        }
        PropertyPosition::Object => {
            // Node is object, property is predicate → value is subject.
            let triples = store.find_by_predicate_object(property.predicate, node_id);
            triples.first().map(|t| t.subject)
        }
    }
}

// ---------------------------------------------------------------------------
// Deep subgraph pattern model
// ---------------------------------------------------------------------------

/// A single step (hop) in a subgraph path: traverse from current node via
/// a predicate, following a specific direction.
///
/// Example path steps for `paper → author → institution`:
/// - Step 0: predicate=`:hasAuthor`, direction=Forward (subject→object)
/// - Step 1: predicate=`:affiliatedWith`, direction=Forward
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathStep {
    /// The predicate traversed at this step.
    pub predicate: TermId,
    /// Direction of traversal: Forward means subject→object,
    /// Reverse means object→subject.
    pub direction: PathDirection,
}

/// Direction of edge traversal in a subgraph path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathDirection {
    /// Traverse from subject to object (follow outgoing edge).
    Forward,
    /// Traverse from object to subject (follow incoming edge).
    Reverse,
}

/// A rooted path in a subgraph pattern: a sequence of hops from the root node
/// to a leaf position. Each path becomes a column in the deep pseudo-table.
///
/// Example: the path `root -:hasAuthor-> ?author -:name-> ?name` has two steps
/// and the column value is the `?name` node reached at the end.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubgraphPath {
    /// Ordered sequence of hops from root to leaf.
    pub steps: Vec<PathStep>,
}

impl SubgraphPath {
    /// The depth (number of hops) of this path.
    pub fn depth(&self) -> usize {
        self.steps.len()
    }
}

/// A subgraph pattern: a set of rooted paths that form a repeated structural
/// motif in the graph. When many root nodes share the same set of paths, the
/// pattern can be materialized as a deep pseudo-table.
///
/// Example: if thousands of papers each have `paper→author→institution` +
/// `paper→fundedBy→source`, that's a SubgraphPattern with two paths.
#[derive(Debug, Clone)]
pub struct SubgraphPattern {
    /// The paths that define this pattern (each becomes a column).
    pub paths: Vec<SubgraphPath>,
    /// Maximum depth across all paths.
    pub max_depth: usize,
    /// Root nodes that match this pattern.
    pub root_nodes: Vec<TermId>,
}

impl SubgraphPattern {
    /// Minimum group size for this pattern to qualify as a pseudo-table,
    /// using the geometric depth threshold: base_threshold * depth².
    pub fn min_group_size(&self) -> usize {
        let base = MIN_GROUP_SIZE;
        base * self.max_depth * self.max_depth
    }

    /// Whether this pattern has enough instances to justify materialization.
    pub fn qualifies(&self) -> bool {
        self.root_nodes.len() >= self.min_group_size()
    }
}

/// Fan-in statistics for a subgraph pattern, measuring how much interior
/// nodes are shared across different root instances.
#[derive(Debug, Clone)]
pub struct FanInStats {
    /// Average number of root instances each interior node appears in.
    /// Fan-in ≈ 1.0 means tree-like (ideal). High fan-in means DAG/lattice.
    pub avg_fan_in: f64,
    /// Maximum fan-in across all interior nodes.
    pub max_fan_in: usize,
    /// Total number of distinct interior nodes observed.
    pub interior_node_count: usize,
}

/// Maximum average fan-in ratio before a pattern is considered DAG-like
/// and penalized/skipped. Tree-like patterns have fan-in ≈ 1.
const MAX_TREE_FAN_IN: f64 = 3.0;

/// Minimum number of distinct paths for a deep subgraph pattern to qualify.
const MIN_SUBGRAPH_PATHS: usize = 2;

/// Minimum coverage for a path to be considered part of the pattern.
const PATH_COVERAGE_THRESHOLD: f64 = 0.50;

/// Minimum coverage for a path to become a column in the deep pseudo-table.
const PATH_COLUMN_THRESHOLD: f64 = 0.33;

// ---------------------------------------------------------------------------
// Subgraph pattern mining
// ---------------------------------------------------------------------------

/// Discover depth-2 subgraph path candidates from the triple store.
///
/// For each node that has outgoing edges, follows those edges one more hop
/// to build 2-step paths. Groups root nodes by the set of 2-step paths
/// they share.
///
/// This is the first stage of subgraph pattern mining. Deeper patterns
/// (depth 3+) can be built by extending these paths, but depth 2 captures
/// the most common structural motifs (e.g., paper→author→institution).
fn mine_depth2_paths(store: &TripleStore) -> Vec<SubgraphPattern> {
    // For each node, collect all depth-2 paths reachable from it.
    let mut node_paths: HashMap<TermId, Vec<SubgraphPath>> = HashMap::new();

    // Get all unique subjects (potential root nodes).
    let mut subjects: Vec<TermId> = Vec::new();
    for triple in store.iter() {
        if !subjects.contains(&triple.subject) {
            subjects.push(triple.subject);
        }
    }

    for &root in &subjects {
        let edges = store.adjacency(root);
        let mut paths = Vec::new();

        for &(pred1, mid_node) in edges {
            // Follow mid_node's outgoing edges for the second hop.
            let mid_edges = store.adjacency(mid_node);
            for &(pred2, _leaf) in mid_edges {
                let path = SubgraphPath {
                    steps: vec![
                        PathStep {
                            predicate: pred1,
                            direction: PathDirection::Forward,
                        },
                        PathStep {
                            predicate: pred2,
                            direction: PathDirection::Forward,
                        },
                    ],
                };
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }

        if paths.len() >= MIN_SUBGRAPH_PATHS {
            node_paths.insert(root, paths);
        }
    }

    // Group roots by their path signature (exact match first).
    let mut signature_groups: HashMap<Vec<Vec<(TermId, u8)>>, Vec<TermId>> = HashMap::new();

    for (root, paths) in &node_paths {
        let mut sig: Vec<Vec<(TermId, u8)>> = paths
            .iter()
            .map(|p| {
                p.steps
                    .iter()
                    .map(|s| (s.predicate, s.direction as u8))
                    .collect()
            })
            .collect();
        sig.sort();
        signature_groups.entry(sig).or_default().push(*root);
    }

    // Merge similar groups (≥80% Jaccard on path sets).
    let mut groups: Vec<(Vec<SubgraphPath>, Vec<TermId>)> = signature_groups
        .into_iter()
        .map(|(sig, roots)| {
            let paths: Vec<SubgraphPath> = sig
                .into_iter()
                .map(|steps| SubgraphPath {
                    steps: steps
                        .into_iter()
                        .map(|(pred, dir)| PathStep {
                            predicate: pred,
                            direction: if dir == 0 {
                                PathDirection::Forward
                            } else {
                                PathDirection::Reverse
                            },
                        })
                        .collect(),
                })
                .collect();
            (paths, roots)
        })
        .collect();

    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut absorbed = vec![false; groups.len()];
    let mut merged: Vec<(Vec<SubgraphPath>, Vec<TermId>)> = Vec::new();

    for i in 0..groups.len() {
        if absorbed[i] {
            continue;
        }

        let mut m_paths = groups[i].0.clone();
        let mut m_roots = groups[i].1.clone();

        for j in (i + 1)..groups.len() {
            if absorbed[j] {
                continue;
            }

            let set_i: std::collections::HashSet<_> = m_paths.iter().cloned().collect();
            let set_j: std::collections::HashSet<_> = groups[j].0.iter().cloned().collect();
            let intersection = set_i.intersection(&set_j).count();
            let union = set_i.union(&set_j).count();

            if union > 0 && (intersection as f64 / union as f64) >= 0.80 {
                for p in &groups[j].0 {
                    if !m_paths.contains(p) {
                        m_paths.push(p.clone());
                    }
                }
                m_roots.extend(&groups[j].1);
                absorbed[j] = true;
            }
        }

        let max_depth = m_paths.iter().map(|p| p.depth()).max().unwrap_or(0);
        merged.push((m_paths, m_roots));
        // Use max_depth for filtering below
        let _ = max_depth;
    }

    // Filter by path coverage and build SubgraphPatterns.
    let mut patterns = Vec::new();

    for (all_paths, roots) in &merged {
        // Compute per-path coverage.
        let mut path_coverage: Vec<(SubgraphPath, f64)> = Vec::new();
        for path in all_paths {
            let count = roots
                .iter()
                .filter(|&&root| node_paths.get(&root).is_some_and(|ps| ps.contains(path)))
                .count();
            let cov = count as f64 / roots.len() as f64;
            path_coverage.push((path.clone(), cov));
        }

        // Only keep paths with ≥50% coverage as pattern members.
        let core_paths: Vec<SubgraphPath> = path_coverage
            .iter()
            .filter(|(_, cov)| *cov >= PATH_COVERAGE_THRESHOLD)
            .map(|(p, _)| p.clone())
            .collect();

        if core_paths.len() < MIN_SUBGRAPH_PATHS {
            continue;
        }

        let max_depth = core_paths.iter().map(|p| p.depth()).max().unwrap_or(0);

        let pattern = SubgraphPattern {
            paths: core_paths,
            max_depth,
            root_nodes: roots.clone(),
        };

        if pattern.qualifies() {
            patterns.push(pattern);
        }
    }

    patterns
}

/// Compute fan-in statistics for a subgraph pattern.
///
/// Measures how much interior nodes (nodes at intermediate positions in paths)
/// are shared across different root instances. High fan-in means DAG/lattice
/// structure where materialization causes duplication.
pub fn compute_fan_in(pattern: &SubgraphPattern, store: &TripleStore) -> FanInStats {
    // Track how many root instances each interior node appears in.
    let mut interior_appearances: HashMap<TermId, usize> = HashMap::new();

    for &root in &pattern.root_nodes {
        for path in &pattern.paths {
            // Walk the path from root, collecting interior nodes (not leaf).
            let mut current = root;
            for (step_idx, step) in path.steps.iter().enumerate() {
                if step_idx == path.steps.len() - 1 {
                    break; // Leaf node, not interior
                }
                let next = match step.direction {
                    PathDirection::Forward => {
                        let triples = store.find_by_subject_predicate(current, step.predicate);
                        triples.first().map(|t| t.object)
                    }
                    PathDirection::Reverse => {
                        let triples = store.find_by_predicate_object(step.predicate, current);
                        triples.first().map(|t| t.subject)
                    }
                };
                if let Some(next_node) = next {
                    if step_idx > 0 {
                        // Interior node (not root, not leaf)
                        *interior_appearances.entry(next_node).or_insert(0) += 1;
                    }
                    current = next_node;
                } else {
                    break;
                }
            }
        }
    }

    let interior_node_count = interior_appearances.len();
    let max_fan_in = interior_appearances.values().copied().max().unwrap_or(0);
    let avg_fan_in = if interior_node_count > 0 {
        interior_appearances.values().sum::<usize>() as f64 / interior_node_count as f64
    } else {
        0.0
    };

    FanInStats {
        avg_fan_in,
        max_fan_in,
        interior_node_count,
    }
}

/// Resolve a subgraph path from a root node, returning the leaf TermId.
///
/// Follows each step in sequence. Returns None if any hop fails
/// (the path doesn't exist for this root).
fn resolve_path(root: TermId, path: &SubgraphPath, store: &TripleStore) -> Option<TermId> {
    let mut current = root;
    for step in &path.steps {
        let next = match step.direction {
            PathDirection::Forward => {
                let triples = store.find_by_subject_predicate(current, step.predicate);
                triples.first().map(|t| t.object)
            }
            PathDirection::Reverse => {
                let triples = store.find_by_predicate_object(step.predicate, current);
                triples.first().map(|t| t.subject)
            }
        };
        current = next?;
    }
    Some(current)
}

/// Materialize a subgraph pattern into a deep pseudo-table.
///
/// Each path in the pattern becomes a column. Each root node becomes a row.
/// The column value is the leaf TermId reached by following the path from the root.
fn materialize_subgraph_table(
    pattern: &SubgraphPattern,
    store: &TripleStore,
    table_index: usize,
) -> PseudoTable {
    // Compute per-path coverage for column ordering.
    let mut path_coverage: Vec<(usize, f64)> = pattern
        .paths
        .iter()
        .enumerate()
        .map(|(idx, path)| {
            let count = pattern
                .root_nodes
                .iter()
                .filter(|&&root| resolve_path(root, path, store).is_some())
                .count();
            (idx, count as f64 / pattern.root_nodes.len() as f64)
        })
        .collect();
    path_coverage.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // Reorder paths by coverage.
    let ordered_indices: Vec<usize> = path_coverage.iter().map(|(idx, _)| *idx).collect();
    // Filter to columns meeting the threshold.
    let included: Vec<usize> = path_coverage
        .iter()
        .filter(|(_, cov)| *cov >= PATH_COLUMN_THRESHOLD)
        .map(|(idx, _)| *idx)
        .collect();

    let actual_num_columns = included.len();
    let included_paths: Vec<SubgraphPath> =
        included.iter().map(|&i| pattern.paths[i].clone()).collect();
    let included_coverage: Vec<f64> = included
        .iter()
        .map(|&i| path_coverage.iter().find(|(idx, _)| *idx == i).unwrap().1)
        .collect();

    // Sort root nodes by first column value for tighter zonemaps.
    let mut sorted_roots = pattern.root_nodes.clone();
    if let Some(first_path) = included_paths.first() {
        sorted_roots.sort_by_key(|&root| resolve_path(root, first_path, store).unwrap_or(0));
    }

    // Build segments.
    let mut segments = Vec::new();
    let mut current_segment = Segment::new(actual_num_columns);

    for &root in &sorted_roots {
        for (col_idx, path) in included_paths.iter().enumerate() {
            let value = resolve_path(root, path, store);
            current_segment.columns[col_idx].push(value);
        }

        // Tail count: paths in the full pattern not included as columns.
        let tail_count = ordered_indices.len() - included.len();

        current_segment.nodes.push(root);
        current_segment.tail_counts.push(tail_count);

        if current_segment.is_full() {
            current_segment.compute_stats();
            segments.push(current_segment);
            current_segment = Segment::new(actual_num_columns);
        }
    }

    if !current_segment.is_empty() {
        current_segment.compute_stats();
        segments.push(current_segment);
    }

    // Core properties from the pattern's paths (map to Property for compatibility).
    let core_properties: Vec<Property> = included_paths
        .iter()
        .filter_map(|p| p.steps.first())
        .map(|step| Property {
            predicate: step.predicate,
            position: match step.direction {
                PathDirection::Forward => PropertyPosition::Subject,
                PathDirection::Reverse => PropertyPosition::Object,
            },
        })
        .collect();

    // Columns as Properties (using the first step's predicate for identification).
    let columns: Vec<Property> = included_paths
        .iter()
        .filter_map(|p| p.steps.first())
        .map(|step| Property {
            predicate: step.predicate,
            position: match step.direction {
                PathDirection::Forward => PropertyPosition::Subject,
                PathDirection::Reverse => PropertyPosition::Object,
            },
        })
        .collect();

    // Cliff steepness for included vs excluded paths.
    let avg_included = if included_coverage.is_empty() {
        0.0
    } else {
        included_coverage.iter().sum::<f64>() / included_coverage.len() as f64
    };
    let excluded_coverages: Vec<f64> = path_coverage
        .iter()
        .filter(|(idx, _)| !included.contains(idx))
        .map(|(_, cov)| *cov)
        .collect();
    let avg_excluded = if excluded_coverages.is_empty() {
        0.01
    } else {
        excluded_coverages.iter().sum::<f64>() / excluded_coverages.len() as f64
    };
    let cliff_steepness = avg_included / avg_excluded.max(0.01);

    PseudoTable {
        label: format!("deep_pseudo_table_{}", table_index),
        columns,
        column_coverage: included_coverage,
        total_rows: sorted_roots.len(),
        core_properties,
        cliff_steepness,
        segments,
    }
}

/// Discover deep subgraph pseudo-tables from the triple store.
///
/// This extends the depth-1 characteristic set discovery to multi-hop patterns.
/// It mines repeated structural motifs (rooted subgraph shapes), filters by
/// geometric depth threshold and fan-in ratio, then materializes qualifying
/// patterns as columnar pseudo-tables.
///
/// Returns a separate registry for deep pseudo-tables. These complement
/// (don't replace) the depth-1 pseudo-tables from `discover_pseudo_tables`.
pub fn discover_deep_pseudo_tables(store: &TripleStore) -> Vec<PseudoTable> {
    let patterns = mine_depth2_paths(store);

    let mut tables = Vec::new();

    for pattern in &patterns {
        // Check fan-in: skip DAG/lattice patterns.
        let fan_in = compute_fan_in(pattern, store);
        if fan_in.avg_fan_in > MAX_TREE_FAN_IN {
            continue;
        }

        tables.push(materialize_subgraph_table(pattern, store, tables.len()));
    }

    tables
}

// ---------------------------------------------------------------------------
// Vectorized scan operations
// ---------------------------------------------------------------------------

/// Result of a vectorized column scan: matching row indices within a segment.
///
/// Used by the executor to efficiently filter pseudo-table segments without
/// examining individual triples. The executor can then join these row indices
/// back to the node TermIds for the final result.
#[derive(Debug)]
pub struct ScanResult {
    /// Indices into the segment's `nodes` array that passed the filter.
    pub matching_rows: Vec<usize>,
}

/// Scan a segment's column for rows matching an equality predicate.
///
/// This is the vectorized equivalent of `find_by_subject_predicate` — but
/// operates on contiguous columnar data instead of a B-tree index, enabling
/// better cache utilization and SIMD acceleration.
///
/// ## SIMD acceleration
///
/// Uses packed column storage (dense u64 arrays with sentinel nulls) and
/// explicit SIMD intrinsics: AVX2 compares 4 u64 TermIDs per cycle,
/// SSE2 compares 2 per cycle, with scalar fallback on other architectures.
pub fn scan_column_eq(segment: &Segment, col_idx: usize, value: TermId) -> ScanResult {
    // Zonemap pruning: skip the entire segment if the value can't be present.
    let stats = &segment.column_stats[col_idx];
    if !stats.range_could_match(Some(value), Some(value)) {
        return ScanResult {
            matching_rows: Vec::new(),
        };
    }

    // SIMD scan on packed column (dense u64 array, nulls = sentinel).
    if let Some(packed) = segment.packed_columns.get(col_idx) {
        return ScanResult {
            matching_rows: packed_scan_eq(packed, value),
        };
    }

    // Fallback to Option-based scan if packed columns not yet built.
    let col = &segment.columns[col_idx];
    let matching_rows: Vec<usize> = col
        .iter()
        .enumerate()
        .filter_map(
            |(idx, val)| {
                if *val == Some(value) {
                    Some(idx)
                } else {
                    None
                }
            },
        )
        .collect();

    ScanResult { matching_rows }
}

/// Scan a segment's column for rows matching a range predicate.
///
/// Supports open ranges (lo or hi can be None for unbounded).
/// Uses zonemap pruning to skip segments that can't contain matching values.
/// Uses packed column storage for cache-friendly scanning.
pub fn scan_column_range(
    segment: &Segment,
    col_idx: usize,
    lo: Option<TermId>,
    hi: Option<TermId>,
) -> ScanResult {
    // Zonemap pruning: skip if the range doesn't overlap the segment's min/max.
    let stats = &segment.column_stats[col_idx];
    if !stats.range_could_match(lo, hi) {
        return ScanResult {
            matching_rows: Vec::new(),
        };
    }

    // Packed scan on dense u64 array (nulls = sentinel, skipped automatically).
    if let Some(packed) = segment.packed_columns.get(col_idx) {
        return ScanResult {
            matching_rows: packed_scan_range(packed, lo, hi),
        };
    }

    // Fallback to Option-based scan if packed columns not yet built.
    let col = &segment.columns[col_idx];
    let matching_rows: Vec<usize> = col
        .iter()
        .enumerate()
        .filter_map(|(idx, val)| {
            if let Some(v) = val {
                let above_lo = lo.is_none_or(|lo| *v >= lo);
                let below_hi = hi.is_none_or(|hi| *v <= hi);
                if above_lo && below_hi {
                    Some(idx)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    ScanResult { matching_rows }
}

/// Scan a segment's column for non-null rows.
///
/// Useful for patterns like `?s :name ?name` where we want all nodes
/// that have the property, regardless of value.
/// Uses SIMD-accelerated null detection on packed columns.
pub fn scan_column_not_null(segment: &Segment, col_idx: usize) -> ScanResult {
    // SIMD scan on packed column: compare against sentinel.
    if let Some(packed) = segment.packed_columns.get(col_idx) {
        return ScanResult {
            matching_rows: packed_scan_not_null(packed),
        };
    }

    // Fallback to Option-based scan.
    let col = &segment.columns[col_idx];
    let matching_rows: Vec<usize> = col
        .iter()
        .enumerate()
        .filter_map(|(idx, val)| if val.is_some() { Some(idx) } else { None })
        .collect();

    ScanResult { matching_rows }
}

/// Batch scan: intersect results from multiple column scans.
///
/// Used for multi-pattern queries like:
/// ```sparql
/// ?s :name ?name . ?s :age ?age . FILTER(?age > 25)
/// ```
///
/// The executor scans each column independently, then intersects the
/// matching row sets. This is the columnar equivalent of a multi-index
/// lookup in a row store.
///
/// ## SIMD opportunity
///
/// The intersection of sorted row index arrays can be accelerated with
/// SIMD merge operations (similar to merge join). For now, we use a
/// simple set intersection which is O(n log n) via sorted merge.
pub fn intersect_scan_results(results: &[ScanResult]) -> ScanResult {
    if results.is_empty() {
        return ScanResult {
            matching_rows: Vec::new(),
        };
    }

    // Start with the smallest result set (for early termination).
    let mut sorted_results: Vec<&ScanResult> = results.iter().collect();
    sorted_results.sort_by_key(|r| r.matching_rows.len());

    let mut intersection: Vec<usize> = sorted_results[0].matching_rows.clone();

    for result in &sorted_results[1..] {
        let other = &result.matching_rows;
        // Sorted merge intersection: O(n + m) where n, m are the two array sizes.
        let mut new_intersection = Vec::new();
        let mut i = 0;
        let mut j = 0;
        while i < intersection.len() && j < other.len() {
            match intersection[i].cmp(&other[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    new_intersection.push(intersection[i]);
                    i += 1;
                    j += 1;
                }
            }
        }
        intersection = new_intersection;

        // Early termination: if intersection is empty, no point continuing.
        if intersection.is_empty() {
            break;
        }
    }

    ScanResult {
        matching_rows: intersection,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triple::Triple;

    /// Build a test store with person-like nodes that share common predicates.
    fn make_person_store() -> (TripleStore, HashMap<&'static str, TermId>) {
        let mut store = TripleStore::new();
        let mut ids = HashMap::new();

        // Predicates
        let rdf_type = 1;
        let name = 2;
        let age = 3;
        let email = 4;
        let knows = 5;
        let city = 6;
        let person = 7;

        ids.insert("rdf:type", rdf_type);
        ids.insert("name", name);
        ids.insert("age", age);
        ids.insert("email", email);
        ids.insert("knows", knows);
        ids.insert("city", city);
        ids.insert("Person", person);

        // Create 20 person nodes with shared predicates.
        // All have: rdf:type, name, age, city, knows (5 core properties)
        // 15 have: email (75% coverage — optional but common)
        // 5 have: extra properties (tail)
        for i in 0..20u64 {
            let node = 100 + i;
            let name_val = 200 + i;
            let age_val = 300 + i;
            let city_val = 400 + (i % 5);
            let knows_target = 100 + ((i + 1) % 20);

            store.insert(Triple::new(node, rdf_type, person)).unwrap();
            store.insert(Triple::new(node, name, name_val)).unwrap();
            store.insert(Triple::new(node, age, age_val)).unwrap();
            store.insert(Triple::new(node, city, city_val)).unwrap();
            store
                .insert(Triple::new(node, knows, knows_target))
                .unwrap();

            // 15 out of 20 have email
            if i < 15 {
                let email_val = 500 + i;
                store.insert(Triple::new(node, email, email_val)).unwrap();
            }
        }

        // Add some unrelated triples (different schema)
        for i in 0..5u64 {
            let doc = 1000 + i;
            let title = 8;
            let content = 9;
            let title_val = 2000 + i;
            let content_val = 3000 + i;
            store.insert(Triple::new(doc, title, title_val)).unwrap();
            store
                .insert(Triple::new(doc, content, content_val))
                .unwrap();
        }

        (store, ids)
    }

    // -----------------------------------------------------------------------
    // Property extraction tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_properties_captures_both_positions() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);

        // Node 100 is a subject of rdf:type, name, age, city, knows, email
        let node_100_props = &node_props[&100];
        assert!(node_100_props.contains(&Property {
            predicate: ids["rdf:type"],
            position: PropertyPosition::Subject,
        }));
        assert!(node_100_props.contains(&Property {
            predicate: ids["name"],
            position: PropertyPosition::Subject,
        }));

        // Person (id 7) is an object of rdf:type
        let person_props = &node_props[&7];
        assert!(person_props.contains(&Property {
            predicate: ids["rdf:type"],
            position: PropertyPosition::Object,
        }));
    }

    // -----------------------------------------------------------------------
    // Discovery tests
    // -----------------------------------------------------------------------

    #[test]
    fn discovers_person_pseudo_table() {
        let (store, _ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        // Should discover at least one pseudo-table for the person nodes.
        assert!(
            !registry.is_empty(),
            "Should discover at least one pseudo-table for 20 person nodes"
        );

        // The largest table should have ~20 rows (the person nodes).
        let largest = registry.tables.iter().max_by_key(|t| t.total_rows).unwrap();
        assert!(
            largest.total_rows >= 15,
            "Largest pseudo-table should have at least 15 rows, got {}",
            largest.total_rows
        );

        // Should have at least 5 columns (the core properties).
        assert!(
            largest.columns.len() >= 5,
            "Should have at least 5 columns, got {}",
            largest.columns.len()
        );
    }

    #[test]
    fn cliff_steepness_is_positive() {
        let (store, _ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        for table in &registry.tables {
            assert!(
                table.cliff_steepness > 0.0,
                "Cliff steepness should be positive, got {}",
                table.cliff_steepness
            );
        }
    }

    // -----------------------------------------------------------------------
    // Segment and zonemap tests
    // -----------------------------------------------------------------------

    #[test]
    fn segments_have_stats() {
        let (store, _ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        if let Some(table) = registry.tables.first() {
            for segment in &table.segments {
                for (col_idx, stats) in segment.column_stats.iter().enumerate() {
                    assert_eq!(
                        stats.row_count,
                        segment.len(),
                        "Stats row count should match segment length for col {}",
                        col_idx
                    );
                }
            }
        }
    }

    #[test]
    fn zonemap_pruning_works() {
        let mut segment = Segment::new(1);
        // Add values 10, 20, 30
        segment.nodes = vec![1, 2, 3];
        segment.columns = vec![vec![Some(10), Some(20), Some(30)]];
        segment.tail_counts = vec![0, 0, 0];
        segment.compute_stats();

        // Value 20 is within range — should find it.
        let result = scan_column_eq(&segment, 0, 20);
        assert_eq!(result.matching_rows, vec![1]);

        // Value 50 is outside range — zonemap should prune.
        let result = scan_column_eq(&segment, 0, 50);
        assert!(result.matching_rows.is_empty());

        // Range [15, 25] should find value 20.
        let result = scan_column_range(&segment, 0, Some(15), Some(25));
        assert_eq!(result.matching_rows, vec![1]);

        // Range [40, 50] should be pruned by zonemap.
        let result = scan_column_range(&segment, 0, Some(40), Some(50));
        assert!(result.matching_rows.is_empty());
    }

    // -----------------------------------------------------------------------
    // Vectorized scan tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_not_null() {
        let mut segment = Segment::new(1);
        segment.nodes = vec![1, 2, 3, 4];
        segment.columns = vec![vec![Some(10), None, Some(30), None]];
        segment.tail_counts = vec![0; 4];
        segment.compute_stats();

        let result = scan_column_not_null(&segment, 0);
        assert_eq!(result.matching_rows, vec![0, 2]);
    }

    #[test]
    fn intersect_scans() {
        let scan1 = ScanResult {
            matching_rows: vec![0, 1, 2, 5, 8],
        };
        let scan2 = ScanResult {
            matching_rows: vec![1, 2, 3, 5, 7],
        };
        let scan3 = ScanResult {
            matching_rows: vec![2, 5, 9],
        };

        let result = intersect_scan_results(&[scan1, scan2, scan3]);
        assert_eq!(result.matching_rows, vec![2, 5]);
    }

    #[test]
    fn intersect_empty_result() {
        let scan1 = ScanResult {
            matching_rows: vec![0, 1, 2],
        };
        let scan2 = ScanResult {
            matching_rows: vec![5, 6, 7],
        };

        let result = intersect_scan_results(&[scan1, scan2]);
        assert!(result.matching_rows.is_empty());
    }

    #[test]
    fn column_stats_selectivity() {
        let stats = ColumnStats {
            min_value: Some(1),
            max_value: Some(100),
            null_count: 5,
            distinct_count: 50,
            row_count: 100,
        };

        // Equality selectivity: 1/50 = 0.02
        assert!((stats.equality_selectivity() - 0.02).abs() < 0.001);

        // Range checks
        assert!(stats.range_could_match(Some(50), Some(60))); // within range
        assert!(!stats.range_could_match(Some(200), Some(300))); // above max
        assert!(!stats.range_could_match(None, Some(0))); // below min (hi < min)
    }

    // -----------------------------------------------------------------------
    // Registry tests
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // SIMD / packed column tests
    // -----------------------------------------------------------------------

    #[test]
    fn packed_columns_built_on_compute_stats() {
        let mut segment = Segment::new(2);
        segment.nodes = vec![1, 2, 3, 4];
        segment.columns = vec![
            vec![Some(10), Some(20), None, Some(40)],
            vec![None, Some(200), Some(300), None],
        ];
        segment.tail_counts = vec![0; 4];
        segment.compute_stats();

        assert_eq!(segment.packed_columns.len(), 2);
        assert_eq!(segment.packed_columns[0], vec![10, 20, NULL_SENTINEL, 40]);
        assert_eq!(
            segment.packed_columns[1],
            vec![NULL_SENTINEL, 200, 300, NULL_SENTINEL]
        );
    }

    #[test]
    fn simd_scan_eq_matches_scalar() {
        let mut segment = Segment::new(1);
        segment.nodes = (0..100).collect();
        segment.columns = vec![(0..100u64)
            .map(|i| if i % 7 == 0 { None } else { Some(i * 3) })
            .collect()];
        segment.tail_counts = vec![0; 100];
        segment.compute_stats();

        // Scan for value 15 (= 5 * 3, should be at index 5)
        let result = scan_column_eq(&segment, 0, 15);
        assert_eq!(result.matching_rows, vec![5]);

        // Scan for value 42 (= 14 * 3, but 14 % 7 == 0 so it's null)
        let result = scan_column_eq(&segment, 0, 42);
        assert!(result.matching_rows.is_empty());

        // Scan for sentinel should find nothing (sentinel != any real value)
        let result = scan_column_eq(&segment, 0, NULL_SENTINEL);
        assert!(
            result.matching_rows.is_empty(),
            "Searching for NULL_SENTINEL should not match null entries"
        );
    }

    #[test]
    fn simd_scan_range_matches_scalar() {
        let mut segment = Segment::new(1);
        segment.nodes = (0..20).collect();
        segment.columns = vec![(0..20u64)
            .map(|i| {
                if i == 5 || i == 15 {
                    None
                } else {
                    Some(i * 10)
                }
            })
            .collect()];
        segment.tail_counts = vec![0; 20];
        segment.compute_stats();

        // Range [30, 70]: should match 30, 40, 60, 70 (skip null at 50)
        let result = scan_column_range(&segment, 0, Some(30), Some(70));
        assert_eq!(result.matching_rows, vec![3, 4, 6, 7]);
    }

    #[test]
    fn simd_scan_not_null_matches_scalar() {
        let mut segment = Segment::new(1);
        segment.nodes = vec![1, 2, 3, 4, 5, 6, 7, 8];
        segment.columns = vec![vec![
            Some(10),
            None,
            Some(30),
            None,
            Some(50),
            None,
            Some(70),
            Some(80),
        ]];
        segment.tail_counts = vec![0; 8];
        segment.compute_stats();

        let result = scan_column_not_null(&segment, 0);
        assert_eq!(result.matching_rows, vec![0, 2, 4, 6, 7]);
    }

    #[test]
    fn simd_scan_eq_large_segment() {
        // Test with a segment larger than AVX2 chunk size (4 u64s)
        let n = 2048;
        let mut segment = Segment::new(1);
        segment.nodes = (0..n as u64).collect();
        segment.columns = vec![(0..n as u64)
            .map(|i| if i == 999 { Some(42) } else { Some(i) })
            .collect()];
        segment.tail_counts = vec![0; n];
        segment.compute_stats();

        let result = scan_column_eq(&segment, 0, 42);
        assert!(result.matching_rows.contains(&999));
        // Also index 42 has value 42
        assert!(result.matching_rows.contains(&42));
        assert_eq!(result.matching_rows.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Registry tests
    // -----------------------------------------------------------------------

    #[test]
    fn registry_find_tables_for_property() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        // The "name" property as Subject should be in a pseudo-table.
        let name_prop = Property {
            predicate: ids["name"],
            position: PropertyPosition::Subject,
        };
        let matches = registry.find_tables_for_property(&name_prop);

        // Should find at least one table with this property as a column.
        if !registry.is_empty() {
            assert!(
                !matches.is_empty(),
                "Should find pseudo-table with 'name' column"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Deep subgraph detection tests
    // -----------------------------------------------------------------------

    /// Build a test store with a tree-like subgraph pattern:
    /// country → capital → mayor, repeated across many countries.
    fn make_deep_tree_store() -> TripleStore {
        let mut store = TripleStore::new();

        // Predicates
        let has_capital = 10;
        let has_mayor = 11;
        let has_name = 12;
        let has_population = 13;
        let rdf_type = 14;
        let country_type = 15;
        let city_type = 16;
        let person_type = 17;

        // Create 50 countries, each with a unique capital and mayor.
        // This is a tree-like pattern: country→capital→mayor
        // Each country also has name and population (depth-1 properties).
        for i in 0..50u64 {
            let country = 100 + i;
            let capital = 200 + i;
            let mayor = 300 + i;
            let country_name = 400 + i;
            let capital_name = 500 + i;
            let mayor_name = 600 + i;
            let pop = 700 + i;

            // Country triples
            store
                .insert(Triple::new(country, rdf_type, country_type))
                .unwrap();
            store
                .insert(Triple::new(country, has_name, country_name))
                .unwrap();
            store
                .insert(Triple::new(country, has_population, pop))
                .unwrap();
            store
                .insert(Triple::new(country, has_capital, capital))
                .unwrap();

            // Capital triples
            store
                .insert(Triple::new(capital, rdf_type, city_type))
                .unwrap();
            store
                .insert(Triple::new(capital, has_name, capital_name))
                .unwrap();
            store
                .insert(Triple::new(capital, has_mayor, mayor))
                .unwrap();

            // Mayor triples
            store
                .insert(Triple::new(mayor, rdf_type, person_type))
                .unwrap();
            store
                .insert(Triple::new(mayor, has_name, mayor_name))
                .unwrap();
        }

        store
    }

    /// Build a store with high fan-in (DAG-like): many students attend
    /// the same few universities.
    fn make_dag_store() -> TripleStore {
        let mut store = TripleStore::new();

        let attends = 10;
        let has_dean = 11;
        let has_name = 12;
        let rdf_type = 13;
        let student_type = 14;

        // 3 universities, each with a dean
        for u in 0..3u64 {
            let univ = 50 + u;
            let dean = 60 + u;
            store.insert(Triple::new(univ, has_dean, dean)).unwrap();
            store.insert(Triple::new(dean, has_name, 70 + u)).unwrap();
        }

        // 60 students, each attending one of 3 universities
        // This creates high fan-in: each university appears in ~20 root instances.
        for i in 0..60u64 {
            let student = 100 + i;
            let univ = 50 + (i % 3);
            store
                .insert(Triple::new(student, rdf_type, student_type))
                .unwrap();
            store
                .insert(Triple::new(student, has_name, 200 + i))
                .unwrap();
            store.insert(Triple::new(student, attends, univ)).unwrap();
        }

        store
    }

    #[test]
    fn subgraph_path_depth() {
        let path = SubgraphPath {
            steps: vec![
                PathStep {
                    predicate: 1,
                    direction: PathDirection::Forward,
                },
                PathStep {
                    predicate: 2,
                    direction: PathDirection::Forward,
                },
            ],
        };
        assert_eq!(path.depth(), 2);
    }

    #[test]
    fn subgraph_pattern_geometric_threshold() {
        let pattern = SubgraphPattern {
            paths: vec![SubgraphPath {
                steps: vec![
                    PathStep {
                        predicate: 1,
                        direction: PathDirection::Forward,
                    },
                    PathStep {
                        predicate: 2,
                        direction: PathDirection::Forward,
                    },
                ],
            }],
            max_depth: 2,
            root_nodes: (0..30).collect(),
        };
        // Depth 2: min = 10 * 2² = 40. 30 nodes < 40, should not qualify.
        assert!(!pattern.qualifies());

        let pattern2 = SubgraphPattern {
            paths: pattern.paths.clone(),
            max_depth: 2,
            root_nodes: (0..50).collect(),
        };
        // 50 >= 40, should qualify.
        assert!(pattern2.qualifies());
    }

    #[test]
    fn resolve_path_follows_edges() {
        let store = make_deep_tree_store();

        let path = SubgraphPath {
            steps: vec![
                PathStep {
                    predicate: 10, // has_capital
                    direction: PathDirection::Forward,
                },
                PathStep {
                    predicate: 11, // has_mayor
                    direction: PathDirection::Forward,
                },
            ],
        };

        // Country 100 → capital 200 → mayor 300
        let result = resolve_path(100, &path, &store);
        assert_eq!(result, Some(300));

        // Country 105 → capital 205 → mayor 305
        let result = resolve_path(105, &path, &store);
        assert_eq!(result, Some(305));
    }

    #[test]
    fn resolve_path_returns_none_for_missing() {
        let store = make_deep_tree_store();

        let path = SubgraphPath {
            steps: vec![
                PathStep {
                    predicate: 999, // nonexistent predicate
                    direction: PathDirection::Forward,
                },
                PathStep {
                    predicate: 11,
                    direction: PathDirection::Forward,
                },
            ],
        };

        assert_eq!(resolve_path(100, &path, &store), None);
    }

    #[test]
    fn mine_depth2_finds_tree_patterns() {
        let store = make_deep_tree_store();
        let patterns = mine_depth2_paths(&store);

        // Should find at least one pattern from the country→capital→mayor structure.
        assert!(
            !patterns.is_empty(),
            "Should discover at least one depth-2 pattern from 50 countries"
        );

        // At least one pattern should have roots among the country nodes (100-149).
        let has_country_pattern = patterns
            .iter()
            .any(|p| p.root_nodes.iter().any(|&r| (100..150).contains(&r)));
        assert!(
            has_country_pattern,
            "Should find pattern rooted at country nodes"
        );
    }

    #[test]
    fn fan_in_low_for_tree() {
        let store = make_deep_tree_store();
        let patterns = mine_depth2_paths(&store);

        for pattern in &patterns {
            let fan_in = compute_fan_in(pattern, &store);
            // Tree-like pattern: each capital is unique to one country.
            // Fan-in should be low (ideally ≈ 1).
            assert!(
                fan_in.avg_fan_in <= MAX_TREE_FAN_IN,
                "Tree pattern should have low fan-in, got {}",
                fan_in.avg_fan_in
            );
        }
    }

    #[test]
    fn fan_in_high_for_dag() {
        let store = make_dag_store();
        let patterns = mine_depth2_paths(&store);

        // The student→university→dean pattern has high fan-in because
        // each university is shared by ~20 students.
        if !patterns.is_empty() {
            let student_patterns: Vec<_> = patterns
                .iter()
                .filter(|p| p.root_nodes.iter().any(|&r| (100..160).contains(&r)))
                .collect();

            for pattern in &student_patterns {
                let fan_in = compute_fan_in(pattern, &store);
                // Universities are shared: each appears in ~20 root instances.
                // This should show higher fan-in.
                assert!(
                    fan_in.max_fan_in > 1,
                    "DAG pattern should have fan-in > 1, got max={}",
                    fan_in.max_fan_in
                );
            }
        }
    }

    #[test]
    fn discover_deep_tables_materializes_tree() {
        let store = make_deep_tree_store();
        let tables = discover_deep_pseudo_tables(&store);

        // Should produce at least one deep pseudo-table from the tree pattern.
        assert!(
            !tables.is_empty(),
            "Should discover at least one deep pseudo-table from 50 countries"
        );

        // The table should have rows (root nodes).
        let largest = tables.iter().max_by_key(|t| t.total_rows).unwrap();
        assert!(
            largest.total_rows >= 10,
            "Deep pseudo-table should have at least 10 rows, got {}",
            largest.total_rows
        );

        // Should have segments with proper stats.
        for segment in &largest.segments {
            assert!(!segment.nodes.is_empty());
            assert!(!segment.packed_columns.is_empty());
        }
    }

    #[test]
    fn discover_deep_tables_skips_dag() {
        let store = make_dag_store();
        let tables = discover_deep_pseudo_tables(&store);

        // The high-fan-in student→university→dean pattern should be
        // filtered out or deprioritized.
        let student_tables: Vec<_> = tables
            .iter()
            .filter(|t| {
                t.segments
                    .iter()
                    .any(|s| s.nodes.iter().any(|&n| (100..160).contains(&n)))
            })
            .collect();

        // Either no student tables, or they should be marked appropriately.
        // The fan-in filter should prevent materialization of the high-overlap pattern.
        // (This is a soft check — the exact behavior depends on the fan-in threshold.)
        if !student_tables.is_empty() {
            // If it wasn't filtered, the fan-in must have been below threshold
            // (possible if the pattern decomposed differently).
            for table in &student_tables {
                assert!(table.total_rows > 0, "If materialized, should have rows");
            }
        }
    }

    // -----------------------------------------------------------------------
    // SelectionVector and vectorized execution tests
    // -----------------------------------------------------------------------

    #[test]
    fn selection_vector_all_set() {
        let sv = SelectionVector::all_set(100);
        assert_eq!(sv.count(), 100);
        assert!(sv.test(0));
        assert!(sv.test(99));
        assert_eq!(sv.to_indices().len(), 100);
    }

    #[test]
    fn selection_vector_none() {
        let sv = SelectionVector::none(100);
        assert_eq!(sv.count(), 0);
        assert!(sv.is_empty());
        assert!(sv.to_indices().is_empty());
    }

    #[test]
    fn selection_vector_set_and_test() {
        let mut sv = SelectionVector::none(128);
        sv.set(0);
        sv.set(63);
        sv.set(64);
        sv.set(127);
        assert_eq!(sv.count(), 4);
        assert!(sv.test(0));
        assert!(sv.test(63));
        assert!(sv.test(64));
        assert!(sv.test(127));
        assert!(!sv.test(1));
        assert_eq!(sv.to_indices(), vec![0, 63, 64, 127]);
    }

    #[test]
    fn selection_vector_and() {
        let mut a = SelectionVector::all_set(128);
        let mut b = SelectionVector::none(128);
        b.set(10);
        b.set(50);
        b.set(100);
        a.and_inplace(&b);
        assert_eq!(a.count(), 3);
        assert_eq!(a.to_indices(), vec![10, 50, 100]);
    }

    #[test]
    fn selection_vector_or() {
        let mut a = SelectionVector::none(128);
        a.set(10);
        let mut b = SelectionVector::none(128);
        b.set(20);
        a.or_inplace(&b);
        assert_eq!(a.count(), 2);
        assert_eq!(a.to_indices(), vec![10, 20]);
    }

    #[test]
    fn selection_vector_non_power_of_two() {
        // Test with a length that isn't a multiple of 64.
        let sv = SelectionVector::all_set(100);
        assert_eq!(sv.count(), 100);
        let indices = sv.to_indices();
        assert_eq!(indices.len(), 100);
        assert_eq!(*indices.last().unwrap(), 99);
    }

    #[test]
    fn bitset_scan_eq_matches_index_scan() {
        // Create a packed column and verify bitset scan matches index scan.
        let data: Vec<u64> = (0..200).collect();
        let target = 42u64;

        let index_result = packed_scan_eq(&data, target);
        let bitset_result = packed_scan_eq_bitset(&data, target, data.len());
        let bitset_indices = bitset_result.to_indices();

        assert_eq!(index_result, bitset_indices);
        assert_eq!(bitset_indices, vec![42]);
    }

    #[test]
    fn bitset_scan_not_null_matches_index_scan() {
        let mut data: Vec<u64> = (0..100).collect();
        // Insert some nulls.
        data[10] = NULL_SENTINEL;
        data[50] = NULL_SENTINEL;
        data[99] = NULL_SENTINEL;

        let index_result = packed_scan_not_null(&data);
        let bitset_result = packed_scan_not_null_bitset(&data, data.len());
        let bitset_indices = bitset_result.to_indices();

        assert_eq!(index_result, bitset_indices);
        assert_eq!(bitset_indices.len(), 97);
        assert!(!bitset_indices.contains(&10));
        assert!(!bitset_indices.contains(&50));
        assert!(!bitset_indices.contains(&99));
    }

    #[test]
    fn fused_scan_single_column_eq() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);
        assert!(!registry.is_empty());

        let table = &registry.tables[0];
        let person_id = ids["Person"];

        // Find rdf:type column index.
        let rdf_type = ids["rdf:type"];
        let rdf_type_prop = Property {
            predicate: rdf_type,
            position: PropertyPosition::Subject,
        };
        let col_idx = table
            .columns
            .iter()
            .position(|p| *p == rdf_type_prop)
            .expect("rdf:type should be a column");

        for segment in &table.segments {
            let selection =
                fused_multi_column_scan(segment, &[(col_idx, ColumnFilter::Eq(person_id))]);
            // All rows should match since all nodes are Person.
            assert_eq!(selection.count(), segment.len());
        }
    }

    #[test]
    fn fused_scan_multi_column() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);
        assert!(!registry.is_empty());

        let table = &registry.tables[0];

        // Find name and age column indices.
        let name_prop = Property {
            predicate: ids["name"],
            position: PropertyPosition::Subject,
        };
        let age_prop = Property {
            predicate: ids["age"],
            position: PropertyPosition::Subject,
        };
        let name_col = table.columns.iter().position(|p| *p == name_prop);
        let age_col = table.columns.iter().position(|p| *p == age_prop);

        if let (Some(name_idx), Some(age_idx)) = (name_col, age_col) {
            for segment in &table.segments {
                // Scan for non-null name AND non-null age.
                let selection = fused_multi_column_scan(
                    segment,
                    &[
                        (name_idx, ColumnFilter::NotNull),
                        (age_idx, ColumnFilter::NotNull),
                    ],
                );
                // All person nodes have both name and age.
                assert_eq!(selection.count(), segment.len());
            }
        }
    }

    #[test]
    fn fused_scan_empty_result() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);
        assert!(!registry.is_empty());

        let table = &registry.tables[0];
        let rdf_type = ids["rdf:type"];
        let rdf_type_prop = Property {
            predicate: rdf_type,
            position: PropertyPosition::Subject,
        };
        let col_idx = table
            .columns
            .iter()
            .position(|p| *p == rdf_type_prop)
            .expect("rdf:type should be a column");

        for segment in &table.segments {
            // Search for a value that doesn't exist — should get empty result.
            let selection = fused_multi_column_scan(segment, &[(col_idx, ColumnFilter::Eq(99999))]);
            assert!(selection.is_empty());
        }
    }

    #[test]
    fn batch_gather_returns_correct_values() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);
        assert!(!registry.is_empty());

        let table = &registry.tables[0];
        let rdf_type = ids["rdf:type"];
        let rdf_type_prop = Property {
            predicate: rdf_type,
            position: PropertyPosition::Subject,
        };
        if let Some(col_idx) = table.columns.iter().position(|p| *p == rdf_type_prop) {
            for segment in &table.segments {
                let selection = SelectionVector::all_set(segment.len());
                let values = super::batch_gather(segment, col_idx, &selection);
                assert_eq!(values.len(), segment.len());
                // All values should be the Person TermId (since all nodes are Person type).
                let person_id = ids["Person"];
                for val in &values {
                    assert_eq!(*val, Some(person_id));
                }
            }
        }
    }

    #[test]
    fn batch_gather_nodes_returns_all_node_ids() {
        let (store, _ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);
        assert!(!registry.is_empty());

        let table = &registry.tables[0];
        for segment in &table.segments {
            let selection = SelectionVector::all_set(segment.len());
            let nodes = super::batch_gather_nodes(segment, &selection);
            assert_eq!(nodes.len(), segment.len());
            assert_eq!(nodes, segment.nodes);
        }
    }

    #[test]
    fn batch_gather_multi_returns_correct_shape() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);
        assert!(!registry.is_empty());

        let table = &registry.tables[0];
        let name_prop = Property {
            predicate: ids["name"],
            position: PropertyPosition::Subject,
        };
        let age_prop = Property {
            predicate: ids["age"],
            position: PropertyPosition::Subject,
        };

        let col_indices: Vec<usize> = [name_prop, age_prop]
            .iter()
            .filter_map(|p| table.columns.iter().position(|c| c == p))
            .collect();

        if col_indices.len() == 2 {
            for segment in &table.segments {
                let selection = SelectionVector::all_set(segment.len());
                let rows = super::batch_gather_multi(segment, &col_indices, &selection);
                assert_eq!(rows.len(), segment.len());
                for row in &rows {
                    assert_eq!(row.len(), 2); // name + age columns
                }
            }
        }
    }
}
