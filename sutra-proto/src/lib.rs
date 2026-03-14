//! SutraDB protocol: SPARQL HTTP protocol, Graph Store Protocol, REST API.

pub mod error;
pub mod server;

pub use server::{router, AppState};
