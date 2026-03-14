//! HTTP server for SPARQL protocol.
//!
//! Implements a subset of the SPARQL 1.1 Protocol (W3C Recommendation):
//! - GET  /sparql?query=...  (query via URL parameter)
//! - POST /sparql            (query in request body)
//! - GET  /health            (health check)
//!
//! Results are returned as JSON (application/sparql-results+json).

use std::sync::{Arc, Mutex};

use axum::extract::{Query as AxumQuery, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use sutra_core::{TermDictionary, TripleStore};
use sutra_hnsw::VectorRegistry;

use crate::error::ProtoError;

/// Shared application state.
pub struct AppState {
    pub store: TripleStore,
    pub dict: TermDictionary,
    pub vectors: Mutex<VectorRegistry>,
}

/// Build the axum router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sparql", get(sparql_get).post(sparql_post))
        .route("/health", get(health))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Query parameters for GET /sparql.
#[derive(Deserialize)]
pub struct SparqlQueryParams {
    query: String,
}

/// SPARQL results JSON format (simplified W3C format).
#[derive(Serialize)]
pub struct SparqlResults {
    pub head: SparqlHead,
    pub results: SparqlBindings,
}

#[derive(Serialize)]
pub struct SparqlHead {
    pub vars: Vec<String>,
}

#[derive(Serialize)]
pub struct SparqlBindings {
    pub bindings: Vec<serde_json::Value>,
}

/// GET /sparql?query=SELECT...
async fn sparql_get(
    State(state): State<Arc<AppState>>,
    AxumQuery(params): AxumQuery<SparqlQueryParams>,
) -> Result<Json<SparqlResults>, ProtoError> {
    execute_sparql(&params.query, &state)
}

/// POST /sparql with query in body.
async fn sparql_post(
    State(state): State<Arc<AppState>>,
    body: String,
) -> Result<Json<SparqlResults>, ProtoError> {
    let query = if let Some(encoded) = body.strip_prefix("query=") {
        urlencoding::decode(encoded)
            .map_err(|e| ProtoError::BadRequest(format!("invalid encoding: {}", e)))?
            .into_owned()
    } else {
        body
    };
    execute_sparql(&query, &state)
}

/// Execute a SPARQL query and return JSON results.
fn execute_sparql(query_str: &str, state: &AppState) -> Result<Json<SparqlResults>, ProtoError> {
    let mut query = sutra_sparql::parse(query_str)?;
    sutra_sparql::optimize(&mut query);

    let mut vectors = state
        .vectors
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let result =
        sutra_sparql::execute_with_vectors(&query, &state.store, &state.dict, &mut vectors)?;

    let bindings: Vec<serde_json::Value> = result
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut obj = serde_json::Map::new();
            for col in &result.columns {
                if let Some(&id) = row.get(col) {
                    let value = resolve_term_to_json(id, &state.dict);
                    obj.insert(col.clone(), value);
                }
            }
            // Include similarity scores if present
            if !result.scores[i].is_empty() {
                let scores_obj: serde_json::Map<String, serde_json::Value> = result.scores[i]
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::json!(*v)))
                    .collect();
                obj.insert(
                    "_scores".to_string(),
                    serde_json::Value::Object(scores_obj),
                );
            }
            serde_json::Value::Object(obj)
        })
        .collect();

    Ok(Json(SparqlResults {
        head: SparqlHead {
            vars: result.columns,
        },
        results: SparqlBindings { bindings },
    }))
}

/// Convert a TermId back to a JSON value for the SPARQL results format.
fn resolve_term_to_json(id: sutra_core::TermId, dict: &TermDictionary) -> serde_json::Value {
    if let Some(n) = sutra_core::decode_inline_integer(id) {
        return serde_json::json!({
            "type": "literal",
            "datatype": "http://www.w3.org/2001/XMLSchema#integer",
            "value": n.to_string()
        });
    }

    if let Some(b) = sutra_core::decode_inline_boolean(id) {
        return serde_json::json!({
            "type": "literal",
            "datatype": "http://www.w3.org/2001/XMLSchema#boolean",
            "value": b.to_string()
        });
    }

    if let Some(term) = dict.resolve(id) {
        if term.starts_with('"') {
            serde_json::json!({
                "type": "literal",
                "value": term.trim_matches('"')
            })
        } else {
            serde_json::json!({
                "type": "uri",
                "value": term
            })
        }
    } else {
        serde_json::json!({
            "type": "uri",
            "value": format!("_:id{}", id)
        })
    }
}

/// GET /health
async fn health() -> (StatusCode, &'static str) {
    (StatusCode::OK, "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use sutra_core::Triple;
    use tower::util::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let mut dict = TermDictionary::new();
        let mut store = TripleStore::new();

        let alice = dict.intern("http://example.org/Alice");
        let bob = dict.intern("http://example.org/Bob");
        let knows = dict.intern("http://example.org/knows");

        store.insert(Triple::new(alice, knows, bob)).unwrap();

        Arc::new(AppState {
            store,
            dict,
            vectors: Mutex::new(VectorRegistry::new()),
        })
    }

    #[tokio::test]
    async fn health_check() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sparql_get_query() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/sparql?query=SELECT%20*%20WHERE%20%7B%20%3Fs%20%3Fp%20%3Fo%20%7D")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["results"]["bindings"].is_array());
        assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn sparql_post_query() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/sparql")
            .header("content-type", "application/sparql-query")
            .body(Body::from("SELECT * WHERE { ?s ?p ?o }"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sparql_invalid_query() {
        let app = router(test_state());
        let req = Request::builder()
            .method("POST")
            .uri("/sparql")
            .body(Body::from("INVALID"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
