//! SutraDB SPARQL: parser, query planner, executor, and hybrid VECTOR_SIMILAR extension.

pub mod error;
pub mod executor;
pub mod parser;
pub mod planner;

pub use error::{Result, SparqlError};
pub use executor::{execute, execute_with_config, execute_with_vectors, Bindings, QueryResult};
pub use parser::{parse, Aggregate, AggregateArg, AggregateFunction, Query, QueryType};
pub use planner::optimize;
