//! SutraDB HNSW: vector index, vector literal type, predicate index registry.
//!
//! This crate has zero dependency on sutra-sparql. It is a pure data structure crate.

pub mod error;
pub mod index;
pub mod node;
pub mod vector;

pub use error::{HnswError, Result};
pub use index::{HnswConfig, HnswIndex, SearchResult};
pub use vector::{cosine_similarity, squared_euclidean};
