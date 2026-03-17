//! SutraDB core: triple storage engine, indexes, IRI interning, RDF-star IDs.

pub mod config;
pub mod error;
pub mod id;
pub mod jsonld;
pub mod ntriples;
pub mod persistent;
pub mod pseudotable;
pub mod rdfxml;
pub mod store;
pub mod triple;
pub mod turtle;

pub use config::{DatabaseConfig, HnswEdgeMode, RdfMode};
pub use error::{CoreError, Result};
pub use id::{
    decode_inline_boolean, decode_inline_integer, inline_boolean, inline_integer, inline_type,
    is_inline, quoted_triple_id, InlineType, TermDictionary, TermId, INVALID_ID,
};
pub use jsonld::parse_jsonld;
pub use ntriples::{parse_nquads_line, parse_ntriples_line};
pub use persistent::PersistentStore;
pub use pseudotable::{
    discover_pseudo_tables, extract_node_properties, intersect_scan_results, scan_column_eq,
    scan_column_not_null, scan_column_range, ColumnStats, Property, PropertyPosition, PropertySet,
    PseudoTable, PseudoTableRegistry, ScanResult, Segment,
};
pub use rdfxml::parse_rdfxml;
pub use store::TripleStore;
pub use triple::Triple;
pub use turtle::parse_turtle;
