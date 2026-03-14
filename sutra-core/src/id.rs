//! IRI interning and RDF-star ID scheme.
//!
//! All IRIs, blank nodes, and literals are interned to `u64` IDs at write time.
//! Quoted triples (RDF-star) are content-addressed via xxHash3 of their (S, P, O) tuple.
//!
//! # Inline literals (inspired by Jena TDB2)
//!
//! Small typed literals (integers, booleans) can be encoded directly into
//! the 64-bit TermId without a dictionary lookup. The high bit distinguishes
//! inline values from dictionary pointers:
//!
//! - Bit 63 = 0: dictionary pointer (regular term)
//! - Bit 63 = 1: inline value
//! - Bits 62-56: type tag (7 bits, 128 possible inline types)
//! - Bits 55-0: payload (56 bits)
//!
//! This avoids dictionary lookups for numeric filters and comparisons,
//! which is a major performance win for SPARQL queries over typed data.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use xxhash_rust::xxh3::xxh3_64;

/// A 64-bit interned identifier for any RDF term.
pub type TermId = u64;

/// Sentinel value meaning "no such term."
pub const INVALID_ID: TermId = 0;

// --- Inline literal encoding ---

const INLINE_BIT: u64 = 1 << 63;
const TYPE_TAG_SHIFT: u32 = 56;
const TYPE_TAG_MASK: u64 = 0x7F << TYPE_TAG_SHIFT;
const PAYLOAD_MASK: u64 = (1 << 56) - 1;

/// Type tags for inline literals.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineType {
    Integer = 0x01,
    Boolean = 0x02,
    // Future: Float = 0x03, Date = 0x04, etc.
}

/// Check if a TermId is an inline literal (not a dictionary pointer).
pub fn is_inline(id: TermId) -> bool {
    id & INLINE_BIT != 0
}

/// Encode a signed integer as an inline TermId.
/// The integer must fit in 56 bits signed (roughly ±36 quadrillion).
/// Returns `None` if out of range.
pub fn inline_integer(value: i64) -> Option<TermId> {
    // 56-bit signed range: -(2^55) to (2^55 - 1)
    if value < -(1i64 << 55) || value >= (1i64 << 55) {
        return None;
    }
    let payload = (value as u64) & PAYLOAD_MASK;
    Some(INLINE_BIT | ((InlineType::Integer as u64) << TYPE_TAG_SHIFT) | payload)
}

/// Decode an inline integer TermId back to i64.
/// Returns `None` if the TermId is not an inline integer.
pub fn decode_inline_integer(id: TermId) -> Option<i64> {
    if !is_inline(id) {
        return None;
    }
    let tag = (id & TYPE_TAG_MASK) >> TYPE_TAG_SHIFT;
    if tag != InlineType::Integer as u64 {
        return None;
    }
    let payload = id & PAYLOAD_MASK;
    // Sign-extend from 56 bits to 64 bits
    let value = if payload & (1 << 55) != 0 {
        (payload | !PAYLOAD_MASK) as i64
    } else {
        payload as i64
    };
    Some(value)
}

/// Encode a boolean as an inline TermId.
pub fn inline_boolean(value: bool) -> TermId {
    INLINE_BIT | ((InlineType::Boolean as u64) << TYPE_TAG_SHIFT) | (value as u64)
}

/// Decode an inline boolean TermId.
pub fn decode_inline_boolean(id: TermId) -> Option<bool> {
    if !is_inline(id) {
        return None;
    }
    let tag = (id & TYPE_TAG_MASK) >> TYPE_TAG_SHIFT;
    if tag != InlineType::Boolean as u64 {
        return None;
    }
    Some((id & PAYLOAD_MASK) != 0)
}

/// Get the inline type tag, if this is an inline value.
pub fn inline_type(id: TermId) -> Option<InlineType> {
    if !is_inline(id) {
        return None;
    }
    let tag = ((id & TYPE_TAG_MASK) >> TYPE_TAG_SHIFT) as u8;
    match tag {
        0x01 => Some(InlineType::Integer),
        0x02 => Some(InlineType::Boolean),
        _ => None,
    }
}

// --- Term Dictionary ---

/// Bidirectional dictionary that maps string terms to integer IDs and back.
///
/// Thread safety: designed for single-writer usage. For concurrent access,
/// wrap in an `RwLock` at the store level.
pub struct TermDictionary {
    forward: HashMap<String, TermId>,
    reverse: HashMap<TermId, String>,
    next_id: AtomicU64,
}

impl TermDictionary {
    /// Create an empty dictionary. IDs start at 1 (0 is reserved as invalid).
    pub fn new() -> Self {
        Self {
            forward: HashMap::new(),
            reverse: HashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Intern a string term, returning its ID. If already interned, returns the existing ID.
    pub fn intern(&mut self, term: &str) -> TermId {
        if let Some(&id) = self.forward.get(term) {
            return id;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // Safety: dictionary IDs must never collide with inline IDs.
        // Since dictionary IDs start at 1 and grow, and inline IDs have
        // bit 63 set, there's no overlap in the practical range.
        debug_assert!(!is_inline(id), "dictionary ID space exhausted");
        self.forward.insert(term.to_owned(), id);
        self.reverse.insert(id, term.to_owned());
        id
    }

    /// Look up a term by its ID. Returns None for inline literal IDs.
    pub fn resolve(&self, id: TermId) -> Option<&str> {
        self.reverse.get(&id).map(|s| s.as_str())
    }

    /// Look up an ID by its string term.
    pub fn lookup(&self, term: &str) -> Option<TermId> {
        self.forward.get(term).copied()
    }

    /// Number of interned terms.
    pub fn len(&self) -> usize {
        self.forward.len()
    }

    /// Whether the dictionary is empty.
    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
}

impl Default for TermDictionary {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a content-addressed ID for a quoted triple (RDF-star).
///
/// The ID is the xxHash3 of the concatenation of subject, predicate, and object IDs.
/// This gives us a deterministic u64 for any (S, P, O) tuple.
pub fn quoted_triple_id(subject: TermId, predicate: TermId, object: TermId) -> TermId {
    let mut buf = [0u8; 24];
    buf[0..8].copy_from_slice(&subject.to_le_bytes());
    buf[8..16].copy_from_slice(&predicate.to_le_bytes());
    buf[16..24].copy_from_slice(&object.to_le_bytes());
    let hash = xxh3_64(&buf);
    // Ensure we never return 0 (INVALID_ID) or an inline-flagged value
    let id = hash & !INLINE_BIT;
    if id == 0 { 1 } else { id }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- TermDictionary ---

    #[test]
    fn intern_and_resolve() {
        let mut dict = TermDictionary::new();
        let id1 = dict.intern("http://example.org/Alice");
        let id2 = dict.intern("http://example.org/Bob");
        let id1_again = dict.intern("http://example.org/Alice");

        assert_eq!(id1, id1_again);
        assert_ne!(id1, id2);
        assert_eq!(dict.resolve(id1), Some("http://example.org/Alice"));
        assert_eq!(dict.resolve(id2), Some("http://example.org/Bob"));
        assert_eq!(dict.len(), 2);
    }

    #[test]
    fn lookup_missing() {
        let dict = TermDictionary::new();
        assert_eq!(dict.lookup("nonexistent"), None);
        assert_eq!(dict.resolve(999), None);
    }

    #[test]
    fn dictionary_ids_not_inline() {
        let mut dict = TermDictionary::new();
        for i in 0..100 {
            let id = dict.intern(&format!("http://example.org/{}", i));
            assert!(!is_inline(id), "dictionary ID should not have inline bit set");
        }
    }

    // --- Quoted triple IDs ---

    #[test]
    fn quoted_triple_id_deterministic() {
        let id_a = quoted_triple_id(1, 2, 3);
        let id_b = quoted_triple_id(1, 2, 3);
        let id_c = quoted_triple_id(3, 2, 1);

        assert_eq!(id_a, id_b);
        assert_ne!(id_a, id_c);
        assert_ne!(id_a, INVALID_ID);
        assert!(!is_inline(id_a), "quoted triple ID should not be inline");
    }

    #[test]
    fn quoted_triple_id_not_zero() {
        // Hash any reasonable input; result should never be 0
        for i in 0..100u64 {
            let id = quoted_triple_id(i, i + 1, i + 2);
            assert_ne!(id, INVALID_ID);
        }
    }

    // --- Inline integers ---

    #[test]
    fn inline_integer_roundtrip() {
        for &val in &[0i64, 1, -1, 42, -42, 1000000, -1000000] {
            let id = inline_integer(val).unwrap();
            assert!(is_inline(id));
            assert_eq!(inline_type(id), Some(InlineType::Integer));
            assert_eq!(decode_inline_integer(id), Some(val));
        }
    }

    #[test]
    fn inline_integer_max_range() {
        let max = (1i64 << 55) - 1;
        let min = -(1i64 << 55);

        assert!(inline_integer(max).is_some());
        assert_eq!(decode_inline_integer(inline_integer(max).unwrap()), Some(max));

        assert!(inline_integer(min).is_some());
        assert_eq!(decode_inline_integer(inline_integer(min).unwrap()), Some(min));

        // Out of range
        assert!(inline_integer(max + 1).is_none());
        assert!(inline_integer(min - 1).is_none());
    }

    #[test]
    fn inline_integer_not_confused_with_dict() {
        let mut dict = TermDictionary::new();
        let dict_id = dict.intern("http://example.org/x");
        let inline_id = inline_integer(42).unwrap();

        assert!(!is_inline(dict_id));
        assert!(is_inline(inline_id));
        assert_ne!(dict_id, inline_id);
    }

    // --- Inline booleans ---

    #[test]
    fn inline_boolean_roundtrip() {
        let true_id = inline_boolean(true);
        let false_id = inline_boolean(false);

        assert!(is_inline(true_id));
        assert!(is_inline(false_id));
        assert_ne!(true_id, false_id);

        assert_eq!(decode_inline_boolean(true_id), Some(true));
        assert_eq!(decode_inline_boolean(false_id), Some(false));

        assert_eq!(inline_type(true_id), Some(InlineType::Boolean));
    }

    #[test]
    fn decode_wrong_type() {
        let int_id = inline_integer(42).unwrap();
        let bool_id = inline_boolean(true);

        assert_eq!(decode_inline_boolean(int_id), None);
        assert_eq!(decode_inline_integer(bool_id), None);
    }

    #[test]
    fn decode_non_inline() {
        assert_eq!(decode_inline_integer(42), None);
        assert_eq!(decode_inline_boolean(42), None);
        assert_eq!(inline_type(42), None);
    }
}
