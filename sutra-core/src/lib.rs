//! SutraDB core: triple storage engine, indexes, IRI interning, RDF-star IDs.

pub mod config;
pub mod error;
pub mod id;
pub mod ntriples;
pub mod persistent;
pub mod store;
pub mod triple;

pub use config::{DatabaseConfig, HnswEdgeMode, RdfMode};
pub use error::{CoreError, Result};
pub use id::{
    decode_inline_boolean, decode_inline_integer, inline_boolean, inline_integer, inline_type,
    is_inline, quoted_triple_id, InlineType, TermDictionary, TermId, INVALID_ID,
};
pub use ntriples::parse_ntriples_line;
pub use persistent::PersistentStore;
pub use store::TripleStore;
pub use triple::Triple;
