//! SutraDB core: triple storage engine, indexes, IRI interning, RDF-star IDs.

pub mod error;
pub mod id;
pub mod store;
pub mod triple;

pub use error::{CoreError, Result};
pub use id::{TermDictionary, TermId, INVALID_ID};
pub use store::TripleStore;
pub use triple::Triple;
