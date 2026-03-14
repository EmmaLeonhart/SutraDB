//! IRI interning and RDF-star ID scheme.
//!
//! All IRIs, blank nodes, and literals are interned to `u64` IDs at write time.
//! Quoted triples (RDF-star) are content-addressed via xxHash3 of their (S, P, O) tuple.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use xxhash_rust::xxh3::xxh3_64;

/// A 64-bit interned identifier for any RDF term.
pub type TermId = u64;

/// Sentinel value meaning "no such term."
pub const INVALID_ID: TermId = 0;

/// Bidirectional dictionary that maps string terms to integer IDs and back.
///
/// Thread safety: this is designed for single-writer usage. For concurrent
/// access, wrap in an `RwLock` at the store level.
pub struct TermDictionary {
    /// Forward map: string → ID.
    forward: HashMap<String, TermId>,
    /// Reverse map: ID → string.
    reverse: HashMap<TermId, String>,
    /// Next ID to assign.
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
        self.forward.insert(term.to_owned(), id);
        self.reverse.insert(id, term.to_owned());
        id
    }

    /// Look up a term by its ID.
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
    // Ensure we never return 0 (reserved as INVALID_ID)
    if hash == 0 { 1 } else { hash }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn quoted_triple_id_deterministic() {
        let id_a = quoted_triple_id(1, 2, 3);
        let id_b = quoted_triple_id(1, 2, 3);
        let id_c = quoted_triple_id(3, 2, 1);

        assert_eq!(id_a, id_b);
        assert_ne!(id_a, id_c);
        assert_ne!(id_a, INVALID_ID);
    }
}
