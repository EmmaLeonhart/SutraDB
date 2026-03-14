//! SutraDB core: triple storage engine, indexes, IRI interning, RDF-star IDs.

pub mod error;
pub mod id;
pub mod store;
pub mod triple;

pub use error::{CoreError, Result};
pub use id::{TermDictionary, TermId, INVALID_ID};
pub use id::{inline_integer, inline_boolean, decode_inline_integer, decode_inline_boolean};
pub use id::{is_inline, inline_type, InlineType, quoted_triple_id};
pub use store::TripleStore;
pub use triple::Triple;
