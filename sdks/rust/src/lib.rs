//! # SutraDB Rust Client
//!
//! A blocking HTTP client for [SutraDB](https://github.com/EmmaLeonhart/SutraDB),
//! the RDF-star triplestore with native HNSW vector indexing.
//!
//! ## Quick Start
//!
//! ```no_run
//! use sutradb::SutraClient;
//!
//! let client = SutraClient::new("http://localhost:7878");
//!
//! // Check health
//! assert!(client.health().unwrap());
//!
//! // Insert triples
//! client.insert_triples(r#"
//!     <http://example.org/paper1> <http://example.org/title> "Graph Databases" .
//! "#).unwrap();
//!
//! // Run a SPARQL query
//! let results = client.sparql("SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10").unwrap();
//! for row in &results.results.bindings {
//!     println!("{:?}", row);
//! }
//! ```

pub mod client;
pub mod error;
pub mod types;

pub use client::SutraClient;
pub use error::{Result, SutraError};
