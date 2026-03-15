use reqwest::blocking::Client;
use serde_json::json;

use crate::error::{Result, SutraError};
use crate::types::{
    DeclareVectorResult, InsertResult, InsertVectorResult, SparqlResults,
};

/// A blocking client for communicating with a SutraDB instance.
///
/// # Example
///
/// ```no_run
/// use sutradb::SutraClient;
///
/// let client = SutraClient::new("http://localhost:7878");
/// let alive = client.health().unwrap();
/// assert!(alive);
/// ```
pub struct SutraClient {
    http: Client,
    endpoint: String,
}

impl SutraClient {
    /// Create a new client pointing at the given SutraDB endpoint.
    ///
    /// The endpoint should be the base URL without a trailing slash,
    /// e.g. `"http://localhost:7878"`.
    pub fn new(endpoint: &str) -> Self {
        let endpoint = endpoint.trim_end_matches('/').to_string();
        let http = Client::builder()
            .user_agent(format!("sutradb-rust-sdk/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build HTTP client");

        Self { http, endpoint }
    }

    /// Check whether the SutraDB instance is reachable and healthy.
    ///
    /// Returns `Ok(true)` if the server responds with a success status,
    /// `Ok(false)` if it responds with a non-success status, or an error
    /// if the request itself fails.
    pub fn health(&self) -> Result<bool> {
        let url = format!("{}/health", self.endpoint);
        let resp = self.http.get(&url).send()?;
        Ok(resp.status().is_success())
    }

    /// Execute a SPARQL query and return the parsed JSON result set.
    ///
    /// The query is sent as `application/sparql-query` in the request body
    /// via POST to the `/sparql` endpoint.
    pub fn sparql(&self, query: &str) -> Result<SparqlResults> {
        let url = format!("{}/sparql", self.endpoint);
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/sparql-query")
            .header("Accept", "application/sparql-results+json")
            .body(query.to_string())
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(SutraError::Server {
                status: status.as_u16(),
                message: body,
            });
        }

        let results: SparqlResults = resp.json()?;
        Ok(results)
    }

    /// Insert triples in N-Triples format.
    ///
    /// The payload should be valid N-Triples (one triple per line,
    /// each terminated with ` .`).
    pub fn insert_triples(&self, ntriples: &str) -> Result<InsertResult> {
        let url = format!("{}/triples", self.endpoint);
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/n-triples")
            .body(ntriples.to_string())
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(SutraError::Server {
                status: status.as_u16(),
                message: body,
            });
        }

        let result: InsertResult = resp.json()?;
        Ok(result)
    }

    /// Declare a vector predicate with the given dimensionality and
    /// optional HNSW parameters.
    ///
    /// This must be called before inserting vectors for a given predicate.
    pub fn declare_vector(
        &self,
        predicate: &str,
        dimensions: u32,
        hnsw_m: Option<u32>,
        hnsw_ef_construction: Option<u32>,
    ) -> Result<DeclareVectorResult> {
        let url = format!("{}/vectors/declare", self.endpoint);

        let mut body = json!({
            "predicate": predicate,
            "dimensions": dimensions,
        });

        if let Some(m) = hnsw_m {
            body["hnswM"] = json!(m);
        }
        if let Some(ef) = hnsw_ef_construction {
            body["hnswEfConstruction"] = json!(ef);
        }

        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(SutraError::Server {
                status: status.as_u16(),
                message: body,
            });
        }

        let result: DeclareVectorResult = resp.json()?;
        Ok(result)
    }

    /// Insert a vector for the given subject under the specified predicate.
    ///
    /// The predicate must have been previously declared with
    /// [`declare_vector`](Self::declare_vector), and the vector length must
    /// match the declared dimensionality.
    pub fn insert_vector(
        &self,
        predicate: &str,
        subject: &str,
        vector: &[f32],
    ) -> Result<InsertVectorResult> {
        let url = format!("{}/vectors", self.endpoint);

        let body = json!({
            "predicate": predicate,
            "subject": subject,
            "vector": vector,
        });

        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(SutraError::Server {
                status: status.as_u16(),
                message: body,
            });
        }

        let result: InsertVectorResult = resp.json()?;
        Ok(result)
    }

    /// Return the base endpoint URL this client is configured with.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_client_strips_trailing_slash() {
        let client = SutraClient::new("http://localhost:7878/");
        assert_eq!(client.endpoint(), "http://localhost:7878");
    }

    #[test]
    fn new_client_preserves_clean_url() {
        let client = SutraClient::new("http://localhost:7878");
        assert_eq!(client.endpoint(), "http://localhost:7878");
    }
}
