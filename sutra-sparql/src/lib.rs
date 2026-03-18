//! SutraDB SPARQL: parser, query planner, executor, and hybrid VECTOR_SIMILAR extension.

pub mod error;
pub mod executor;
pub mod health;
pub mod parser;
pub mod planner;

pub use error::{Result, SparqlError};
pub use executor::{
    execute, execute_with_config, execute_with_timeout, execute_with_vectors, Bindings, QueryResult,
};
pub use health::{generate_health_report, HealthReport, HealthStatus};
pub use parser::{parse, Aggregate, AggregateArg, AggregateFunction, Query, QueryType};
pub use planner::{optimize, optimize_full, optimize_with_store};
