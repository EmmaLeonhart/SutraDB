//! Triple representation and RDF-star quoted triples.
//!
//! A triple is the fundamental unit of data in SutraDB: (subject, predicate, object).
//! All three components are stored as interned `TermId` values.

use crate::id::TermId;

/// A single RDF triple, stored as three interned IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Triple {
    /// Subject: an IRI, blank node, or quoted triple ID.
    pub subject: TermId,
    /// Predicate: always an IRI.
    pub predicate: TermId,
    /// Object: an IRI, blank node, literal, or quoted triple ID.
    pub object: TermId,
}

impl Triple {
    /// Create a new triple from three term IDs.
    pub fn new(subject: TermId, predicate: TermId, object: TermId) -> Self {
        Self {
            subject,
            predicate,
            object,
        }
    }

    /// Encode this triple as a 24-byte key in SPO order.
    pub fn spo_key(&self) -> [u8; 24] {
        let mut key = [0u8; 24];
        key[0..8].copy_from_slice(&self.subject.to_be_bytes());
        key[8..16].copy_from_slice(&self.predicate.to_be_bytes());
        key[16..24].copy_from_slice(&self.object.to_be_bytes());
        key
    }

    /// Encode this triple as a 24-byte key in POS order.
    pub fn pos_key(&self) -> [u8; 24] {
        let mut key = [0u8; 24];
        key[0..8].copy_from_slice(&self.predicate.to_be_bytes());
        key[8..16].copy_from_slice(&self.object.to_be_bytes());
        key[16..24].copy_from_slice(&self.subject.to_be_bytes());
        key
    }

    /// Encode this triple as a 24-byte key in OSP order.
    pub fn osp_key(&self) -> [u8; 24] {
        let mut key = [0u8; 24];
        key[0..8].copy_from_slice(&self.object.to_be_bytes());
        key[8..16].copy_from_slice(&self.subject.to_be_bytes());
        key[16..24].copy_from_slice(&self.predicate.to_be_bytes());
        key
    }

    /// Decode a triple from a 24-byte SPO key.
    pub fn from_spo_key(key: &[u8; 24]) -> Self {
        Self {
            subject: u64::from_be_bytes(key[0..8].try_into().unwrap()),
            predicate: u64::from_be_bytes(key[8..16].try_into().unwrap()),
            object: u64::from_be_bytes(key[16..24].try_into().unwrap()),
        }
    }

    /// Decode a triple from a 24-byte POS key.
    pub fn from_pos_key(key: &[u8; 24]) -> Self {
        Self {
            subject: u64::from_be_bytes(key[16..24].try_into().unwrap()),
            predicate: u64::from_be_bytes(key[0..8].try_into().unwrap()),
            object: u64::from_be_bytes(key[8..16].try_into().unwrap()),
        }
    }

    /// Decode a triple from a 24-byte OSP key.
    pub fn from_osp_key(key: &[u8; 24]) -> Self {
        Self {
            subject: u64::from_be_bytes(key[8..16].try_into().unwrap()),
            predicate: u64::from_be_bytes(key[16..24].try_into().unwrap()),
            object: u64::from_be_bytes(key[0..8].try_into().unwrap()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_spo() {
        let t = Triple::new(100, 200, 300);
        let key = t.spo_key();
        let t2 = Triple::from_spo_key(&key);
        assert_eq!(t, t2);
    }

    #[test]
    fn round_trip_pos() {
        let t = Triple::new(100, 200, 300);
        let key = t.pos_key();
        let t2 = Triple::from_pos_key(&key);
        assert_eq!(t, t2);
    }

    #[test]
    fn round_trip_osp() {
        let t = Triple::new(100, 200, 300);
        let key = t.osp_key();
        let t2 = Triple::from_osp_key(&key);
        assert_eq!(t, t2);
    }

    #[test]
    fn key_ordering() {
        // SPO keys should sort by subject first
        let t1 = Triple::new(1, 5, 9);
        let t2 = Triple::new(2, 3, 7);
        assert!(t1.spo_key() < t2.spo_key());

        // POS keys should sort by predicate first
        let t3 = Triple::new(9, 1, 5);
        let t4 = Triple::new(7, 2, 3);
        assert!(t3.pos_key() < t4.pos_key());
    }
}
