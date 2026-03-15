//! Persistent triple store backed by sled.
//!
//! Uses sled's sorted key-value trees to implement SPO/POS/OSP indexes
//! that survive restarts. Same API as the in-memory `TripleStore`, but
//! data is durably written to disk.
//!
//! Each index is a separate sled `Tree` within a single `Db`. Keys are
//! 24-byte composite keys (3 x u64 in big-endian for correct sort order).
//! Values are empty — the key itself encodes the triple.

use std::path::Path;

use crate::error::{CoreError, Result};
use crate::id::TermId;
use crate::triple::Triple;

/// A persistent triple store backed by sled.
///
/// Three sled trees provide SPO/POS/OSP indexes with the same semantics
/// as the in-memory `TripleStore`. A fourth tree stores the term dictionary.
pub struct PersistentStore {
    #[allow(dead_code)]
    db: sled::Db,
    spo: sled::Tree,
    pos: sled::Tree,
    osp: sled::Tree,
    /// Term dictionary: forward map (string → u64 ID).
    terms_forward: sled::Tree,
    /// Term dictionary: reverse map (u64 ID → string).
    terms_reverse: sled::Tree,
    /// Next term ID counter, stored persistently.
    meta: sled::Tree,
}

const NEXT_ID_KEY: &[u8] = b"next_term_id";

impl PersistentStore {
    /// Open or create a persistent store at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = sled::open(path)?;
        let spo = db.open_tree("spo")?;
        let pos = db.open_tree("pos")?;
        let osp = db.open_tree("osp")?;
        let terms_forward = db.open_tree("terms_fwd")?;
        let terms_reverse = db.open_tree("terms_rev")?;
        let meta = db.open_tree("meta")?;

        // Initialize next_id if not present (start at 1, 0 is INVALID_ID)
        if meta.get(NEXT_ID_KEY)?.is_none() {
            meta.insert(NEXT_ID_KEY, &1u64.to_le_bytes())?;
        }

        Ok(Self {
            db,
            spo,
            pos,
            osp,
            terms_forward,
            terms_reverse,
            meta,
        })
    }

    /// Open a temporary store (for testing). Data is deleted on drop.
    pub fn temporary() -> Result<Self> {
        let db = sled::Config::new().temporary(true).open()?;
        let spo = db.open_tree("spo")?;
        let pos = db.open_tree("pos")?;
        let osp = db.open_tree("osp")?;
        let terms_forward = db.open_tree("terms_fwd")?;
        let terms_reverse = db.open_tree("terms_rev")?;
        let meta = db.open_tree("meta")?;

        meta.insert(NEXT_ID_KEY, &1u64.to_le_bytes())?;

        Ok(Self {
            db,
            spo,
            pos,
            osp,
            terms_forward,
            terms_reverse,
            meta,
        })
    }

    // --- Triple operations ---

    /// Insert a triple. Returns `Err(DuplicateTriple)` if already present.
    pub fn insert(&self, triple: Triple) -> Result<()> {
        let spo_key = triple.spo_key();
        if self.spo.contains_key(spo_key)? {
            return Err(CoreError::DuplicateTriple);
        }
        // Empty value — the key encodes the triple
        self.spo.insert(spo_key, &[])?;
        self.pos.insert(triple.pos_key(), &[])?;
        self.osp.insert(triple.osp_key(), &[])?;
        Ok(())
    }

    /// Remove a triple. Returns true if it was present.
    pub fn remove(&self, triple: &Triple) -> Result<bool> {
        let was_present = self.spo.remove(triple.spo_key())?.is_some();
        if was_present {
            self.pos.remove(triple.pos_key())?;
            self.osp.remove(triple.osp_key())?;
        }
        Ok(was_present)
    }

    /// Check whether a triple exists.
    pub fn contains(&self, triple: &Triple) -> Result<bool> {
        Ok(self.spo.contains_key(triple.spo_key())?)
    }

    /// Number of triples in the store.
    pub fn len(&self) -> usize {
        self.spo.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.spo.is_empty()
    }

    /// Find all triples with the given subject.
    pub fn find_by_subject(&self, subject: TermId) -> Vec<Triple> {
        let (lo, hi) = prefix_range_24(0, subject);
        self.spo
            .range(lo..=hi)
            .filter_map(|r| r.ok())
            .map(|(k, _)| Triple::from_spo_key(&key_to_array(&k)))
            .collect()
    }

    /// Find all triples with the given predicate.
    pub fn find_by_predicate(&self, predicate: TermId) -> Vec<Triple> {
        let (lo, hi) = prefix_range_24(0, predicate);
        self.pos
            .range(lo..=hi)
            .filter_map(|r| r.ok())
            .map(|(k, _)| Triple::from_pos_key(&key_to_array(&k)))
            .collect()
    }

    /// Find all triples with the given object.
    pub fn find_by_object(&self, object: TermId) -> Vec<Triple> {
        let (lo, hi) = prefix_range_24(0, object);
        self.osp
            .range(lo..=hi)
            .filter_map(|r| r.ok())
            .map(|(k, _)| Triple::from_osp_key(&key_to_array(&k)))
            .collect()
    }

    /// Find all triples with the given subject and predicate.
    pub fn find_by_subject_predicate(&self, subject: TermId, predicate: TermId) -> Vec<Triple> {
        let (lo, hi) = prefix_range_24_2(subject, predicate);
        self.spo
            .range(lo..=hi)
            .filter_map(|r| r.ok())
            .map(|(k, _)| Triple::from_spo_key(&key_to_array(&k)))
            .collect()
    }

    /// Iterate all triples in SPO order.
    pub fn iter(&self) -> impl Iterator<Item = Triple> + '_ {
        self.spo
            .iter()
            .filter_map(|r| r.ok())
            .map(|(k, _)| Triple::from_spo_key(&key_to_array(&k)))
    }

    // --- Term dictionary operations ---

    /// Intern a string term, returning its ID. If already interned, returns existing ID.
    pub fn intern(&self, term: &str) -> Result<TermId> {
        if let Some(id_bytes) = self.terms_forward.get(term.as_bytes())? {
            return Ok(u64::from_le_bytes(id_bytes.as_ref().try_into().unwrap()));
        }

        // Atomically increment the counter
        let id = self.next_id()?;
        let id_bytes = id.to_le_bytes();
        self.terms_forward.insert(term.as_bytes(), &id_bytes)?;
        self.terms_reverse.insert(id_bytes, term.as_bytes())?;
        Ok(id)
    }

    /// Look up a term by its ID.
    pub fn resolve(&self, id: TermId) -> Result<Option<String>> {
        match self.terms_reverse.get(id.to_le_bytes())? {
            Some(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            None => Ok(None),
        }
    }

    /// Look up an ID by its string term.
    pub fn lookup(&self, term: &str) -> Result<Option<TermId>> {
        match self.terms_forward.get(term.as_bytes())? {
            Some(id_bytes) => Ok(Some(u64::from_le_bytes(
                id_bytes.as_ref().try_into().unwrap(),
            ))),
            None => Ok(None),
        }
    }

    /// Load all terms from persistent storage into an in-memory TermDictionary.
    /// Returns the number of terms loaded.
    pub fn load_terms_into(&self, dict: &mut crate::id::TermDictionary) -> usize {
        let mut count = 0;
        for (key_bytes, val_bytes) in self.terms_forward.iter().flatten() {
            let term = String::from_utf8_lossy(&key_bytes).into_owned();
            let id = u64::from_le_bytes(val_bytes.as_ref().try_into().unwrap());
            dict.insert_with_id(&term, id);
            count += 1;
        }
        count
    }

    /// Find all triples with the given predicate and object.
    /// Uses the POS index for efficient lookup.
    pub fn find_by_predicate_object(&self, predicate: TermId, object: TermId) -> Vec<Triple> {
        let (lo, hi) = prefix_range_24_2(predicate, object);
        self.pos
            .range(lo..=hi)
            .filter_map(|r| r.ok())
            .map(|(k, _)| Triple::from_pos_key(&key_to_array(&k)))
            .collect()
    }

    /// Flush all pending writes to disk.
    pub fn flush(&self) -> Result<()> {
        self.spo.flush()?;
        self.pos.flush()?;
        self.osp.flush()?;
        self.terms_forward.flush()?;
        self.terms_reverse.flush()?;
        self.meta.flush()?;
        Ok(())
    }

    fn next_id(&self) -> Result<u64> {
        let old = self
            .meta
            .fetch_and_update(NEXT_ID_KEY, |old| {
                let current = u64::from_le_bytes(old.unwrap().try_into().unwrap());
                Some((current + 1).to_le_bytes().to_vec())
            })?
            .unwrap();
        Ok(u64::from_le_bytes(old.as_ref().try_into().unwrap()))
    }
}

/// Build a 24-byte prefix range for a single leading u64.
fn prefix_range_24(_offset: usize, value: TermId) -> ([u8; 24], [u8; 24]) {
    let mut lo = [0u8; 24];
    lo[0..8].copy_from_slice(&value.to_be_bytes());
    let mut hi = [0u8; 24];
    hi[0..8].copy_from_slice(&value.to_be_bytes());
    hi[8..24].fill(0xFF);
    (lo, hi)
}

/// Build a 24-byte prefix range for two leading u64s.
fn prefix_range_24_2(first: TermId, second: TermId) -> ([u8; 24], [u8; 24]) {
    let mut lo = [0u8; 24];
    lo[0..8].copy_from_slice(&first.to_be_bytes());
    lo[8..16].copy_from_slice(&second.to_be_bytes());
    let mut hi = [0u8; 24];
    hi[0..8].copy_from_slice(&first.to_be_bytes());
    hi[8..16].copy_from_slice(&second.to_be_bytes());
    hi[16..24].fill(0xFF);
    (lo, hi)
}

fn key_to_array(ivec: &sled::IVec) -> [u8; 24] {
    let mut arr = [0u8; 24];
    arr.copy_from_slice(ivec.as_ref());
    arr
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> PersistentStore {
        let store = PersistentStore::temporary().unwrap();
        store.insert(Triple::new(1, 10, 2)).unwrap();
        store.insert(Triple::new(1, 10, 3)).unwrap();
        store.insert(Triple::new(2, 10, 1)).unwrap();
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
        let store = make_store();
        let result = store.insert(Triple::new(1, 10, 2));
        assert!(result.is_err());
        assert_eq!(store.len(), 4);
    }

    #[test]
    fn find_by_subject() {
        let store = make_store();
        let results = store.find_by_subject(1);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn find_by_predicate() {
        let store = make_store();
        let results = store.find_by_predicate(10);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn find_by_object() {
        let store = make_store();
        let results = store.find_by_object(1);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn find_by_subject_predicate() {
        let store = make_store();
        let results = store.find_by_subject_predicate(1, 10);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn remove() {
        let store = make_store();
        assert!(store.remove(&Triple::new(1, 10, 2)).unwrap());
        assert_eq!(store.len(), 3);
        assert!(!store.contains(&Triple::new(1, 10, 2)).unwrap());
        assert_eq!(store.find_by_subject(1).len(), 2);
    }

    #[test]
    fn term_dictionary_roundtrip() {
        let store = PersistentStore::temporary().unwrap();
        let id1 = store.intern("http://example.org/Alice").unwrap();
        let id2 = store.intern("http://example.org/Bob").unwrap();
        let id1_again = store.intern("http://example.org/Alice").unwrap();

        assert_eq!(id1, id1_again);
        assert_ne!(id1, id2);
        assert_eq!(
            store.resolve(id1).unwrap().as_deref(),
            Some("http://example.org/Alice")
        );
        assert_eq!(
            store.resolve(id2).unwrap().as_deref(),
            Some("http://example.org/Bob")
        );
    }

    #[test]
    fn iter_all() {
        let store = make_store();
        let all: Vec<_> = store.iter().collect();
        assert_eq!(all.len(), 4);
    }
}
