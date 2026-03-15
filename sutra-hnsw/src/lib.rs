//! SutraDB HNSW: vector index, vector literal type, predicate index registry.
//!
//! This crate has zero dependency on sutra-sparql. It is a pure data structure crate.

pub mod edges;
pub mod error;
pub mod index;
pub mod node;
pub mod registry;
pub mod vector;

pub use edges::{
    HnswEdgeTriple, HNSW_LAYER_IRI, HNSW_NEIGHBOR_IRI, HNSW_PREDICATE_IRI, HNSW_SIMILARITY_IRI,
};
pub use error::{HnswError, Result};
pub use index::{HnswConfig, HnswIndex, SearchResult};
pub use registry::{VectorPredicateConfig, VectorRegistry};
pub use vector::{
    cosine_similarity, dot_product, l2_norm, normalize, normalized, squared_euclidean,
    DistanceMetric,
};
