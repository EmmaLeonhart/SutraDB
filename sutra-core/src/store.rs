//! In-memory triple store with SPO/POS/OSP indexes.
//!
//! This is the v0.1 implementation using `BTreeSet` as the index structure.
//! A future version will replace this with an LSM-tree for persistence and
//! better write throughput on bulk ingestion.

use std::collections::BTreeSet;

use crate::error::{CoreError, Result};
use crate::id::TermId;
use crate::triple::Triple;

/// An in-memory triple store backed by three sorted indexes.
///
/// Each index stores the same triples in a different key order so that
/// any access pattern (subject-first, predicate-first, object-first)
/// can be served by a range scan rather than a full scan.
pub struct TripleStore {
    /// Subject → Predicate → Object index.
    spo: BTreeSet<[u8; 24]>,
    /// Predicate → Object → Subject index.
    pos: BTreeSet<[u8; 24]>,
    /// Object → Subject → Predicate index.
    osp: BTreeSet<[u8; 24]>,
    /// Total number of triples stored.
    count: usize,
}

impl TripleStore {
    /// Create an empty triple store.
    pub fn new() -> Self {
        Self {
            spo: BTreeSet::new(),
            pos: BTreeSet::new(),
            osp: BTreeSet::new(),
            count: 0,
        }
    }

    /// Insert a triple. Returns `Err(DuplicateTriple)` if already present.
    pub fn insert(&mut self, triple: Triple) -> Result<()> {
        let spo_key = triple.spo_key();
        if !self.spo.insert(spo_key) {
            return Err(CoreError::DuplicateTriple);
        }
        self.pos.insert(triple.pos_key());
        self.osp.insert(triple.osp_key());
        self.count += 1;
        Ok(())
    }

    /// Remove a triple. Returns true if it was present.
    pub fn remove(&mut self, triple: &Triple) -> bool {
        let removed = self.spo.remove(&triple.spo_key());
        if removed {
            self.pos.remove(&triple.pos_key());
            self.osp.remove(&triple.osp_key());
            self.count -= 1;
        }
        removed
    }

    /// Check whether a triple exists.
    pub fn contains(&self, triple: &Triple) -> bool {
        self.spo.contains(&triple.spo_key())
    }

    /// Number of triples in the store.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Find all triples with the given subject.
    pub fn find_by_subject(&self, subject: TermId) -> Vec<Triple> {
        let mut lo = [0u8; 24];
        lo[0..8].copy_from_slice(&subject.to_be_bytes());

        let mut hi = [0u8; 24];
        hi[0..8].copy_from_slice(&subject.to_be_bytes());
        hi[8..24].fill(0xFF);

        self.spo.range(lo..=hi).map(Triple::from_spo_key).collect()
    }

    /// Find all triples with the given predicate.
    pub fn find_by_predicate(&self, predicate: TermId) -> Vec<Triple> {
        let mut lo = [0u8; 24];
        lo[0..8].copy_from_slice(&predicate.to_be_bytes());

        let mut hi = [0u8; 24];
        hi[0..8].copy_from_slice(&predicate.to_be_bytes());
        hi[8..24].fill(0xFF);

        self.pos.range(lo..=hi).map(Triple::from_pos_key).collect()
    }

    /// Find all triples with the given object.
    pub fn find_by_object(&self, object: TermId) -> Vec<Triple> {
        let mut lo = [0u8; 24];
        lo[0..8].copy_from_slice(&object.to_be_bytes());

        let mut hi = [0u8; 24];
        hi[0..8].copy_from_slice(&object.to_be_bytes());
        hi[8..24].fill(0xFF);

        self.osp.range(lo..=hi).map(Triple::from_osp_key).collect()
    }

    /// Find all triples with the given subject and predicate.
    pub fn find_by_subject_predicate(&self, subject: TermId, predicate: TermId) -> Vec<Triple> {
        let mut lo = [0u8; 24];
        lo[0..8].copy_from_slice(&subject.to_be_bytes());
        lo[8..16].copy_from_slice(&predicate.to_be_bytes());

        let mut hi = [0u8; 24];
        hi[0..8].copy_from_slice(&subject.to_be_bytes());
        hi[8..16].copy_from_slice(&predicate.to_be_bytes());
        hi[16..24].fill(0xFF);

        self.spo.range(lo..=hi).map(Triple::from_spo_key).collect()
    }

    /// Iterate all triples in SPO order.
    pub fn iter(&self) -> impl Iterator<Item = Triple> + '_ {
        self.spo.iter().map(Triple::from_spo_key)
    }
}

impl Default for TripleStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> TripleStore {
        let mut store = TripleStore::new();
        // :Alice :knows :Bob
        store.insert(Triple::new(1, 10, 2)).unwrap();
        // :Alice :knows :Charlie
        store.insert(Triple::new(1, 10, 3)).unwrap();
        // :Bob :knows :Alice
        store.insert(Triple::new(2, 10, 1)).unwrap();
        // :Alice :name "Alice"
        store.insert(Triple::new(1, 11, 100)).unwrap();
        store
    }

    #[test]
    fn insert_and_count() {
        let store = make_store();
        assert_eq!(store.len(), 4);
    }

    #[test]
    fn duplicate_rejected() {
        let mut store = make_store();
        let result = store.insert(Triple::new(1, 10, 2));
        assert!(result.is_err());
        assert_eq!(store.len(), 4);
    }

    #[test]
    fn find_by_subject() {
        let store = make_store();
        let results = store.find_by_subject(1);
        assert_eq!(results.len(), 3); // Alice has 3 triples as subject
    }

    #[test]
    fn find_by_predicate() {
        let store = make_store();
        let results = store.find_by_predicate(10); // :knows
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn find_by_object() {
        let store = make_store();
        let results = store.find_by_object(1); // things pointing to Alice
        assert_eq!(results.len(), 1); // Bob knows Alice
    }

    #[test]
    fn find_by_subject_predicate() {
        let store = make_store();
        let results = store.find_by_subject_predicate(1, 10); // Alice knows ?
        assert_eq!(results.len(), 2); // Bob and Charlie
    }

    #[test]
    fn remove() {
        let mut store = make_store();
        assert!(store.remove(&Triple::new(1, 10, 2)));
        assert_eq!(store.len(), 3);
        assert!(!store.contains(&Triple::new(1, 10, 2)));
        // Should still find Alice's other triples
        assert_eq!(store.find_by_subject(1).len(), 2);
    }

    #[test]
    fn iter_all() {
        let store = make_store();
        let all: Vec<_> = store.iter().collect();
        assert_eq!(all.len(), 4);
    }
}
