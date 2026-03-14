//! HTTP server for SPARQL protocol.
//!
//! Implements a subset of the SPARQL 1.1 Protocol (W3C Recommendation):
//! - GET  /sparql?query=...  (query via URL parameter)
//! - POST /sparql            (query in request body)
//! - POST /triples           (insert N-Triples data)
//! - POST /vectors/declare   (declare a vector predicate)
//! - POST /vectors           (insert a vector)
//! - GET  /health            (health check)
//!
//! Results are returned as JSON (application/sparql-results+json).

use std::sync::{Arc, Mutex};

use axum::extract::{Query as AxumQuery, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use sutra_core::{TermDictionary, TripleStore};
use sutra_hnsw::VectorRegistry;

use crate::error::ProtoError;

/// Shared application state.
pub struct AppState {
    pub store: Mutex<TripleStore>,
    pub dict: Mutex<TermDictionary>,
    pub vectors: Mutex<VectorRegistry>,
}

/// Build the axum router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sparql", get(sparql_get).post(sparql_post))
        .route("/triples", post(insert_triples))
        .route("/vectors/declare", post(declare_vector_predicate))
        .route("/vectors", post(insert_vector))
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

    let store = state
        .store
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let dict = state
        .dict
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let mut vectors = state
        .vectors
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let result = sutra_sparql::execute_with_vectors(&query, &store, &dict, &mut vectors)?;

    let bindings: Vec<serde_json::Value> = result
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut obj = serde_json::Map::new();
            for col in &result.columns {
                if let Some(&id) = row.get(col) {
                    let value = resolve_term_to_json(id, &dict);
                    obj.insert(col.clone(), value);
                }
            }
            // Include similarity scores if present
            if !result.scores[i].is_empty() {
                let scores_obj: serde_json::Map<String, serde_json::Value> = result.scores[i]
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::json!(*v)))
                    .collect();
                obj.insert("_scores".to_string(), serde_json::Value::Object(scores_obj));
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

// ─── Insert Triples ──────────────────────────────────────────────────────────

const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

/// Response from the POST /triples endpoint.
#[derive(Serialize)]
pub struct InsertTriplesResponse {
    pub inserted: usize,
    pub errors: Vec<String>,
}

/// POST /triples — accepts N-Triples in the request body.
async fn insert_triples(
    State(state): State<Arc<AppState>>,
    body: String,
) -> Result<Json<InsertTriplesResponse>, ProtoError> {
    let mut dict = state
        .dict
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let mut store = state
        .store
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;

    let mut inserted = 0usize;
    let mut errors = Vec::new();

    for (line_no, line) in body.lines().enumerate() {
        let parsed = match sutra_core::parse_ntriples_line(line) {
            Some(t) => t,
            None => continue, // blank / comment
        };

        let (subj_str, pred_str, obj_str) = parsed;

        let s_id = dict.intern(&subj_str);
        let p_id = dict.intern(&pred_str);
        let o_id = intern_object(&mut dict, &obj_str);

        match store.insert(sutra_core::Triple::new(s_id, p_id, o_id)) {
            Ok(()) => inserted += 1,
            Err(e) => errors.push(format!("line {}: {}", line_no + 1, e)),
        }
    }

    Ok(Json(InsertTriplesResponse { inserted, errors }))
}

/// Intern an object term, handling typed literals specially.
fn intern_object(dict: &mut TermDictionary, obj: &str) -> sutra_core::TermId {
    // Check for typed literals: "value"^^<datatype>
    if let Some(caret_pos) = obj.find("\"^^<") {
        let value_str = &obj[1..caret_pos]; // strip leading quote
        let datatype_start = caret_pos + 4; // skip "^^<
        let datatype_end = obj.len() - 1; // strip trailing >
        if datatype_end > datatype_start {
            let datatype = &obj[datatype_start..datatype_end];
            if datatype == XSD_INTEGER {
                if let Ok(n) = value_str.parse::<i64>() {
                    if let Some(id) = sutra_core::inline_integer(n) {
                        return id;
                    }
                }
            }
            if datatype == XSD_BOOLEAN {
                match value_str {
                    "true" => return sutra_core::inline_boolean(true),
                    "false" => return sutra_core::inline_boolean(false),
                    _ => {}
                }
            }
        }
    }
    dict.intern(obj)
}

// ─── Declare Vector Predicate ────────────────────────────────────────────────

/// Request body for POST /vectors/declare.
#[derive(Deserialize)]
pub struct DeclareVectorRequest {
    pub predicate: String,
    pub dimensions: usize,
    #[serde(default = "default_m")]
    pub m: usize,
    #[serde(default = "default_ef_construction")]
    pub ef_construction: usize,
    #[serde(default = "default_metric")]
    pub metric: String,
}

fn default_m() -> usize {
    16
}
fn default_ef_construction() -> usize {
    200
}
fn default_metric() -> String {
    "cosine".to_string()
}

#[derive(Serialize)]
pub struct DeclareVectorResponse {
    pub status: String,
    pub predicate_id: u64,
}

/// POST /vectors/declare — declare a vector predicate with HNSW parameters.
async fn declare_vector_predicate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeclareVectorRequest>,
) -> Result<Json<DeclareVectorResponse>, ProtoError> {
    let metric = match req.metric.to_lowercase().as_str() {
        "cosine" => sutra_hnsw::DistanceMetric::Cosine,
        "euclidean" => sutra_hnsw::DistanceMetric::Euclidean,
        "dot" | "dotproduct" | "dot_product" => sutra_hnsw::DistanceMetric::DotProduct,
        other => {
            return Err(ProtoError::BadRequest(format!("unknown metric: {}", other)));
        }
    };

    let predicate_id = {
        let mut dict = state
            .dict
            .lock()
            .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
        dict.intern(&req.predicate)
    };

    let config = sutra_hnsw::VectorPredicateConfig {
        predicate_id,
        dimensions: req.dimensions,
        m: req.m,
        ef_construction: req.ef_construction,
        metric,
    };

    let mut vectors = state
        .vectors
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    vectors
        .declare(config)
        .map_err(|e| ProtoError::BadRequest(format!("vector declare error: {}", e)))?;

    Ok(Json(DeclareVectorResponse {
        status: "ok".to_string(),
        predicate_id,
    }))
}

// ─── Insert Vector ───────────────────────────────────────────────────────────

/// Request body for POST /vectors.
#[derive(Deserialize)]
pub struct InsertVectorRequest {
    pub predicate: String,
    pub subject: String,
    pub vector: Vec<f32>,
}

#[derive(Serialize)]
pub struct InsertVectorResponse {
    pub status: String,
    pub triple_id: u64,
}

/// POST /vectors — insert a vector embedding for a subject on a predicate.
///
/// Every vector is a triple: `<subject> <predicate> <vector_literal>`.
///
/// The vector literal is the **object** of the triple. The HNSW index is
/// keyed by the object's TermId. Multiple subjects can point to the same
/// vector (e.g. "bank" the institution and "bank" the riverbank can both
/// link to the same embedding). VECTOR_SIMILAR finds matching vector objects,
/// then you join via the graph to find which subjects connect to them.
///
/// A vector never exists in the database without at least one triple
/// pointing to it.
async fn insert_vector(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InsertVectorRequest>,
) -> Result<Json<InsertVectorResponse>, ProtoError> {
    let (predicate_id, subject_id, object_id) = {
        let mut dict = state
            .dict
            .lock()
            .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
        let p = dict.intern(&req.predicate);
        let s = dict.intern(&req.subject);
        // The vector literal is the object — it's a primitive value in the graph
        let vec_str: Vec<String> = req.vector.iter().map(|f| format!("{:.6}", f)).collect();
        let literal = format!("\"{}\"^^<http://sutra.dev/f32vec>", vec_str.join(" "));
        let o = dict.intern(&literal);
        (p, s, o)
    };

    // Insert the triple: <subject> <predicate> <vector_literal>
    {
        let mut store = state
            .store
            .lock()
            .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
        // Ignore duplicate triple errors (allows multiple subjects to point to same vector)
        let _ = store.insert(sutra_core::Triple::new(subject_id, predicate_id, object_id));
    }

    // Insert into HNSW index, keyed by the object_id (the vector literal's identity).
    // If this vector was already inserted (another subject pointing to same vector),
    // the HNSW insert may error — that's fine, the vector is already indexed.
    let mut vectors = state
        .vectors
        .lock()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let _ = vectors.insert(predicate_id, req.vector, object_id);

    Ok(Json(InsertVectorResponse {
        status: "ok".to_string(),
        triple_id: object_id,
    }))
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
            store: Mutex::new(store),
            dict: Mutex::new(dict),
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

    #[tokio::test]
    async fn insert_ntriples() {
        let state = test_state();
        let app = router(state.clone());
        let body = concat!(
            "<http://example.org/s1> <http://example.org/p1> <http://example.org/o1> .\n",
            "<http://example.org/s2> <http://example.org/p2> \"hello\" .\n",
            "# comment line\n",
            "\n",
            "<http://example.org/s3> <http://example.org/p3> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        );
        let req = Request::builder()
            .method("POST")
            .uri("/triples")
            .header("content-type", "text/plain")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["inserted"], 3);
        assert_eq!(json["errors"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn insert_duplicate_triple_reports_error() {
        let state = test_state();
        let app = router(state.clone());
        // Insert the same triple that's already in the store
        let body =
            "<http://example.org/Alice> <http://example.org/knows> <http://example.org/Bob> .\n";
        let req = Request::builder()
            .method("POST")
            .uri("/triples")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["inserted"], 0);
        assert_eq!(json["errors"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn declare_and_insert_vector() {
        let state = test_state();

        // Declare vector predicate
        let app = router(state.clone());
        let declare_body = serde_json::json!({
            "predicate": "http://example.org/hasEmbedding",
            "dimensions": 3,
            "m": 4,
            "ef_construction": 20,
            "metric": "cosine"
        });
        let req = Request::builder()
            .method("POST")
            .uri("/vectors/declare")
            .header("content-type", "application/json")
            .body(Body::from(declare_body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["status"], "ok");

        // Insert vector
        let app = router(state.clone());
        let insert_body = serde_json::json!({
            "predicate": "http://example.org/hasEmbedding",
            "subject": "http://example.org/entity1",
            "vector": [0.1, 0.2, 0.3]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/vectors")
            .header("content-type", "application/json")
            .body(Body::from(insert_body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(json["status"], "ok");
    }
}
