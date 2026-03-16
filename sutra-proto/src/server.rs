//! HTTP server for SPARQL protocol.
//!
//! Implements a subset of the SPARQL 1.1 Protocol (W3C Recommendation):
//! - GET  /sparql?query=...  (query via URL parameter)
//! - POST /sparql            (query in request body)
//! - GET  /graph             (export all triples as Turtle — for Protégé)
//! - POST /triples           (insert N-Triples data)
//! - POST /vectors/declare   (declare a vector predicate)
//! - POST /vectors           (insert a vector)
//! - GET  /health            (health check)
//!
//! Results are returned as JSON (application/sparql-results+json).

use std::sync::{Arc, RwLock};

use axum::extract::{Query as AxumQuery, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use sutra_core::{PersistentStore, TermDictionary, TripleStore};
use sutra_hnsw::VectorRegistry;

use crate::error::ProtoError;

/// Shared application state.
///
/// Uses RwLock for read-heavy workloads (concurrent SPARQL queries).
/// The in-memory stores are the working set for the SPARQL executor.
/// When `persistent` is Some, all writes go to both in-memory and disk.
pub struct AppState {
    pub store: RwLock<TripleStore>,
    pub dict: RwLock<TermDictionary>,
    pub vectors: RwLock<VectorRegistry>,
    /// Optional persistent backing store. When present, all mutations
    /// are written through to disk. On startup, in-memory stores are
    /// hydrated from the persistent store.
    pub persistent: Option<PersistentStore>,
    /// Optional passcode for simple authentication (server mode only).
    /// When set, all requests (except /health) must include
    /// `Authorization: Bearer <passcode>` header.
    pub passcode: Option<String>,
}

/// Build the axum router with all endpoints.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sparql", get(sparql_get).post(sparql_post))
        .route("/graph", get(export_graph))
        .route("/sparql.csv", get(sparql_csv_get).post(sparql_csv_post))
        .route("/sparql.tsv", get(sparql_tsv_get).post(sparql_tsv_post))
        .route("/sparql.xml", get(sparql_xml_get).post(sparql_xml_post))
        .route("/triples", post(insert_triples))
        .route("/vectors/declare", post(declare_vector_predicate))
        .route("/vectors", post(insert_vector))
        .route("/health", get(health))
        .route("/vectors/health", get(vectors_health))
        .route("/.well-known/void", get(service_description))
        .route("/service-description", get(service_description))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Simple passcode authentication middleware.
/// Skips auth for /health endpoint. When passcode is not configured, all requests pass.
async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    // No passcode configured — allow all
    let passcode = match &state.passcode {
        Some(p) => p,
        None => return next.run(req).await,
    };

    // Health endpoint is always accessible
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }

    // Check Authorization: Bearer <passcode>
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(h) if h.starts_with("Bearer ") && &h[7..] == passcode => next.run(req).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            "Unauthorized: include Authorization: Bearer <passcode> header",
        )
            .into_response(),
    }
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

// ─── SPARQL CSV/TSV ─────────────────────────────────────────────────────────

async fn sparql_csv_get(
    State(state): State<Arc<AppState>>,
    AxumQuery(params): AxumQuery<SparqlQueryParams>,
) -> Result<impl IntoResponse, ProtoError> {
    sparql_delimited(&params.query, &state, ",", "text/csv; charset=utf-8")
}

async fn sparql_csv_post(
    State(state): State<Arc<AppState>>,
    body: String,
) -> Result<impl IntoResponse, ProtoError> {
    sparql_delimited(&body, &state, ",", "text/csv; charset=utf-8")
}

async fn sparql_tsv_get(
    State(state): State<Arc<AppState>>,
    AxumQuery(params): AxumQuery<SparqlQueryParams>,
) -> Result<impl IntoResponse, ProtoError> {
    sparql_delimited(
        &params.query,
        &state,
        "\t",
        "text/tab-separated-values; charset=utf-8",
    )
}

async fn sparql_tsv_post(
    State(state): State<Arc<AppState>>,
    body: String,
) -> Result<impl IntoResponse, ProtoError> {
    sparql_delimited(
        &body,
        &state,
        "\t",
        "text/tab-separated-values; charset=utf-8",
    )
}

fn sparql_delimited(
    query_str: &str,
    state: &AppState,
    delimiter: &str,
    content_type: &'static str,
) -> Result<impl IntoResponse, ProtoError> {
    let mut query = sutra_sparql::parse(query_str)?;
    sutra_sparql::optimize(&mut query);

    let store = state
        .store
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let dict = state
        .dict
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let vectors = state
        .vectors
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let result = sutra_sparql::execute_with_vectors(&query, &store, &dict, &vectors)?;

    let mut output = String::new();

    // Header row
    output.push_str(&result.columns.join(delimiter));
    output.push('\n');

    // Data rows
    for row in &result.rows {
        let vals: Vec<String> = result
            .columns
            .iter()
            .map(|col| {
                row.get(col)
                    .map(|&id| resolve_term_for_csv(id, &dict))
                    .unwrap_or_default()
            })
            .collect();
        output.push_str(&vals.join(delimiter));
        output.push('\n');
    }

    Ok(([(header::CONTENT_TYPE, content_type)], output))
}

fn resolve_term_for_csv(id: sutra_core::TermId, dict: &TermDictionary) -> String {
    if let Some(n) = sutra_core::decode_inline_integer(id) {
        return n.to_string();
    }
    if let Some(b) = sutra_core::decode_inline_boolean(id) {
        return b.to_string();
    }
    dict.resolve(id)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("_:id{}", id))
}

// ─── SPARQL XML ─────────────────────────────────────────────────────────────

async fn sparql_xml_get(
    State(state): State<Arc<AppState>>,
    AxumQuery(params): AxumQuery<SparqlQueryParams>,
) -> Result<impl IntoResponse, ProtoError> {
    sparql_xml(&params.query, &state)
}

async fn sparql_xml_post(
    State(state): State<Arc<AppState>>,
    body: String,
) -> Result<impl IntoResponse, ProtoError> {
    sparql_xml(&body, &state)
}

fn sparql_xml(query_str: &str, state: &AppState) -> Result<impl IntoResponse, ProtoError> {
    let mut query = sutra_sparql::parse(query_str)?;
    sutra_sparql::optimize(&mut query);

    let store = state
        .store
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let dict = state
        .dict
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let vectors = state
        .vectors
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let result = sutra_sparql::execute_with_vectors(&query, &store, &dict, &vectors)?;

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <sparql xmlns=\"http://www.w3.org/2005/sparql-results#\">\n  <head>\n",
    );
    for col in &result.columns {
        xml.push_str(&format!("    <variable name=\"{}\"/>\n", col));
    }
    xml.push_str("  </head>\n  <results>\n");

    for row in &result.rows {
        xml.push_str("    <result>\n");
        for col in &result.columns {
            if let Some(&id) = row.get(col) {
                let val = resolve_term_for_csv(id, &dict);
                let escaped = val
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                if sutra_core::is_inline(id)
                    || dict.resolve(id).is_some_and(|s| s.starts_with('"'))
                {
                    xml.push_str(&format!(
                        "      <binding name=\"{}\"><literal>{}</literal></binding>\n",
                        col, escaped
                    ));
                } else {
                    xml.push_str(&format!(
                        "      <binding name=\"{}\"><uri>{}</uri></binding>\n",
                        col, escaped
                    ));
                }
            }
        }
        xml.push_str("    </result>\n");
    }
    xml.push_str("  </results>\n</sparql>\n");

    Ok((
        [(
            header::CONTENT_TYPE,
            "application/sparql-results+xml; charset=utf-8",
        )],
        xml,
    ))
}

/// Execute a SPARQL query and return JSON results.
fn execute_sparql(query_str: &str, state: &AppState) -> Result<Json<SparqlResults>, ProtoError> {
    let mut query = sutra_sparql::parse(query_str)?;

    // Handle SPARQL Update (INSERT DATA / DELETE DATA)
    if query.query_type == sutra_sparql::QueryType::InsertData {
        return execute_insert_data(&query, state);
    }
    if query.query_type == sutra_sparql::QueryType::DeleteData {
        return execute_delete_data(&query, state);
    }

    sutra_sparql::optimize(&mut query);

    // Read locks: concurrent SPARQL queries don't block each other
    let store = state
        .store
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let dict = state
        .dict
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let vectors = state
        .vectors
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let result = sutra_sparql::execute_with_vectors(&query, &store, &dict, &vectors)?;

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
/// Execute INSERT DATA { triple patterns }.
fn execute_insert_data(
    query: &sutra_sparql::Query,
    state: &AppState,
) -> Result<Json<SparqlResults>, ProtoError> {
    use sutra_sparql::parser::Pattern;

    let mut dict = state
        .dict
        .write()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let mut store = state
        .store
        .write()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;

    let mut inserted = 0i64;
    for pattern in &query.patterns {
        if let Pattern::Triple {
            subject,
            predicate,
            object,
        } = pattern
        {
            let s_id = resolve_term_to_id(subject, &mut dict, &query.prefixes)?;
            let p_id = resolve_term_to_id(predicate, &mut dict, &query.prefixes)?;
            let o_id = resolve_term_to_id(object, &mut dict, &query.prefixes)?;

            // Check for schema declarations: sutra:declareVectorPredicate
            let pred_str = dict.resolve(p_id).unwrap_or("").to_string();
            if pred_str == "http://sutra.dev/dimensions"
                || pred_str.contains("declareVectorPredicate")
            {
                // This is a vector schema triple — try to auto-declare
                if let Some(dims) = sutra_core::decode_inline_integer(o_id) {
                    let mut vectors = state
                        .vectors
                        .write()
                        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
                    if !vectors.has_index(s_id) {
                        let config = sutra_hnsw::VectorPredicateConfig {
                            predicate_id: s_id,
                            dimensions: dims as usize,
                            m: 16,
                            ef_construction: 200,
                            metric: sutra_hnsw::DistanceMetric::Cosine,
                        };
                        let _ = vectors.declare(config);
                    }
                }
            }

            if store
                .insert(sutra_core::Triple::new(s_id, p_id, o_id))
                .is_ok()
            {
                if let Some(ref ps) = state.persistent {
                    let _ = ps.insert(sutra_core::Triple::new(s_id, p_id, o_id));
                }
                inserted += 1;
            }
        }
    }

    let mut row = std::collections::HashMap::new();
    if let Some(id) = sutra_core::inline_integer(inserted) {
        row.insert("mutationCount".to_string(), id);
    }

    Ok(Json(SparqlResults {
        head: SparqlHead {
            vars: vec!["mutationCount".to_string()],
        },
        results: SparqlBindings {
            bindings: vec![
                serde_json::json!({"mutationCount": {"type": "literal", "value": inserted.to_string()}}),
            ],
        },
    }))
}

/// Execute DELETE DATA { triple patterns }.
fn execute_delete_data(
    query: &sutra_sparql::Query,
    state: &AppState,
) -> Result<Json<SparqlResults>, ProtoError> {
    use sutra_sparql::parser::Pattern;

    let mut dict = state
        .dict
        .write()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let mut store = state
        .store
        .write()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;

    let mut deleted = 0i64;
    for pattern in &query.patterns {
        if let Pattern::Triple {
            subject,
            predicate,
            object,
        } = pattern
        {
            let s_id = resolve_term_to_id(subject, &mut dict, &query.prefixes)?;
            let p_id = resolve_term_to_id(predicate, &mut dict, &query.prefixes)?;
            let o_id = resolve_term_to_id(object, &mut dict, &query.prefixes)?;

            if store.remove(&sutra_core::Triple::new(s_id, p_id, o_id)) {
                if let Some(ref ps) = state.persistent {
                    let _ = ps.remove(&sutra_core::Triple::new(s_id, p_id, o_id));
                }
                deleted += 1;
            }
        }
    }

    Ok(Json(SparqlResults {
        head: SparqlHead {
            vars: vec!["mutationCount".to_string()],
        },
        results: SparqlBindings {
            bindings: vec![
                serde_json::json!({"mutationCount": {"type": "literal", "value": deleted.to_string()}}),
            ],
        },
    }))
}

/// Resolve a parsed Term to a TermId, interning if necessary.
fn resolve_term_to_id(
    term: &sutra_sparql::parser::Term,
    dict: &mut TermDictionary,
    prefixes: &std::collections::HashMap<String, String>,
) -> std::result::Result<sutra_core::TermId, ProtoError> {
    use sutra_sparql::parser::Term;
    match term {
        Term::Iri(iri) => Ok(dict.intern(iri)),
        Term::PrefixedName { prefix, local } => {
            let ns = prefixes
                .get(prefix.as_str())
                .ok_or_else(|| ProtoError::BadRequest(format!("unknown prefix: {}", prefix)))?;
            Ok(dict.intern(&format!("{}{}", ns, local)))
        }
        Term::Literal(s) => Ok(dict.intern(&format!("\"{}\"", s))),
        Term::TypedLiteral { value, datatype } => {
            let full = format!("\"{}\"^^<{}>", value, datatype);
            Ok(intern_object(dict, &full))
        }
        Term::IntegerLiteral(n) => sutra_core::inline_integer(*n)
            .ok_or_else(|| ProtoError::BadRequest("integer out of range".into())),
        Term::A => Ok(dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")),
        _ => Err(ProtoError::BadRequest(
            "variables not allowed in INSERT/DELETE DATA".into(),
        )),
    }
}

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

// ─── Export Graph (Turtle) ───────────────────────────────────────────────────

/// Query parameters for GET /graph.
#[derive(Deserialize)]
pub struct GraphQueryParams {
    /// Optional: request a specific format. Defaults to Turtle.
    #[serde(default)]
    format: Option<String>,
}

/// GET /graph — export all triples as Turtle.
///
/// Protégé can load this via File > Open from URL > http://localhost:3030/graph
/// Also useful for any tool that speaks RDF: curl, rdflib, Apache Jena, etc.
async fn export_graph(
    State(state): State<Arc<AppState>>,
    AxumQuery(params): AxumQuery<GraphQueryParams>,
) -> Result<impl IntoResponse, ProtoError> {
    let store = state
        .store
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let dict = state
        .dict
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;

    let use_ntriples = params
        .format
        .as_deref()
        .map(|f| f == "nt" || f == "ntriples")
        .unwrap_or(false);

    let mut output = String::new();

    if use_ntriples {
        // N-Triples: one triple per line, no prefixes
        for triple in store.iter() {
            let s = resolve_term_for_turtle(triple.subject, &dict);
            let p = resolve_term_for_turtle(triple.predicate, &dict);
            let o = resolve_term_for_turtle(triple.object, &dict);
            output.push_str(&format!("{} {} {} .\n", s, p, o));
        }

        Ok((
            [(header::CONTENT_TYPE, "application/n-triples; charset=utf-8")],
            output,
        ))
    } else {
        // Turtle: collect common prefixes, then grouped triples
        let mut prefixes: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();

        // Scan all terms for common prefixes
        let known_prefixes = [
            ("rdf:", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
            ("rdfs:", "http://www.w3.org/2000/01/rdf-schema#"),
            ("owl:", "http://www.w3.org/2002/07/owl#"),
            ("xsd:", "http://www.w3.org/2001/XMLSchema#"),
            ("skos:", "http://www.w3.org/2004/02/skos/core#"),
            ("dc:", "http://purl.org/dc/elements/1.1/"),
            ("dcterms:", "http://purl.org/dc/terms/"),
            ("foaf:", "http://xmlns.com/foaf/0.1/"),
            ("schema:", "http://schema.org/"),
            ("wdt:", "http://www.wikidata.org/prop/direct/"),
            ("wd:", "http://www.wikidata.org/entity/"),
            ("sutra:", "http://sutra.dev/"),
        ];

        // Check which prefixes are actually used
        for triple in store.iter() {
            for id in [triple.subject, triple.predicate, triple.object] {
                if let Some(term) = dict.resolve(id) {
                    for &(prefix, iri) in &known_prefixes {
                        if term.starts_with(iri) && !prefixes.contains_key(prefix) {
                            prefixes.insert(prefix.to_string(), iri.to_string());
                        }
                    }
                }
            }
        }

        // Write prefix declarations
        for (prefix, iri) in &prefixes {
            output.push_str(&format!("@prefix {} <{}> .\n", prefix, iri));
        }
        if !prefixes.is_empty() {
            output.push('\n');
        }

        // Write triples grouped by subject
        let mut current_subject: Option<String> = None;

        for triple in store.iter() {
            let s = resolve_term_for_turtle(triple.subject, &dict);
            let p = resolve_term_for_turtle(triple.predicate, &dict);
            let o = resolve_term_for_turtle(triple.object, &dict);

            // Apply prefix compression
            let s_compact = compact_iri(&s, &prefixes);
            let p_compact = compact_iri(&p, &prefixes);
            let o_compact = compact_iri(&o, &prefixes);

            match &current_subject {
                Some(prev) if *prev == s => {
                    // Same subject: continue with semicolon
                    output.push_str(&format!(" ;\n    {} {}", p_compact, o_compact));
                }
                _ => {
                    // New subject: close previous, start new
                    if current_subject.is_some() {
                        output.push_str(" .\n\n");
                    }
                    output.push_str(&format!("{}\n    {} {}", s_compact, p_compact, o_compact));
                    current_subject = Some(s);
                }
            }
        }
        if current_subject.is_some() {
            output.push_str(" .\n");
        }

        Ok((
            [(header::CONTENT_TYPE, "text/turtle; charset=utf-8")],
            output,
        ))
    }
}

/// Resolve a TermId to its Turtle representation.
fn resolve_term_for_turtle(id: sutra_core::TermId, dict: &TermDictionary) -> String {
    if let Some(n) = sutra_core::decode_inline_integer(id) {
        return format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>", n);
    }

    if let Some(b) = sutra_core::decode_inline_boolean(id) {
        return format!("\"{}\"^^<http://www.w3.org/2001/XMLSchema#boolean>", b);
    }

    if let Some(term) = dict.resolve(id) {
        if term.starts_with('"') {
            // Already a literal with quotes — pass through
            term.to_string()
        } else {
            // IRI — wrap in angle brackets
            format!("<{}>", term)
        }
    } else {
        format!("_:id{}", id)
    }
}

/// Compact an IRI using known prefixes: `<http://...#Foo>` → `prefix:Foo`
fn compact_iri(term: &str, prefixes: &std::collections::BTreeMap<String, String>) -> String {
    // Only compact IRIs (wrapped in <>)
    if let Some(iri) = term.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
        for (prefix, namespace) in prefixes {
            if let Some(local) = iri.strip_prefix(namespace.as_str()) {
                return format!("{}{}", prefix, local);
            }
        }
    }
    term.to_string()
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
        .write()
        .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
    let mut store = state
        .store
        .write()
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

        let triple = sutra_core::Triple::new(s_id, p_id, o_id);
        match store.insert(triple) {
            Ok(()) => {
                // Write through to persistent store
                if let Some(ref ps) = state.persistent {
                    let _ = ps.intern(&subj_str);
                    let _ = ps.intern(&pred_str);
                    let _ = ps.intern(&obj_str);
                    let _ = ps.insert(triple);
                }
                inserted += 1;
            }
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
            .write()
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
        .write()
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
            .write()
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
            .write()
            .map_err(|e| ProtoError::BadRequest(format!("lock poisoned: {}", e)))?;
        let triple = sutra_core::Triple::new(subject_id, predicate_id, object_id);
        // Ignore duplicate triple errors (allows multiple subjects to point to same vector)
        let _ = store.insert(triple);

        // Write through to persistent store
        if let Some(ref ps) = state.persistent {
            let vec_str: Vec<String> = req.vector.iter().map(|f| format!("{:.6}", f)).collect();
            let literal = format!("\"{}\"^^<http://sutra.dev/f32vec>", vec_str.join(" "));
            let _ = ps.intern(&req.predicate);
            let _ = ps.intern(&req.subject);
            let _ = ps.intern(&literal);
            let _ = ps.insert(triple);
        }
    }

    // Insert into HNSW index, keyed by the object_id (the vector literal's identity).
    // If this vector was already inserted (another subject pointing to same vector),
    // the HNSW insert may error — that's fine, the vector is already indexed.
    let mut vectors = state
        .vectors
        .write()
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

/// GET /vectors/health — HNSW index health diagnostics.
async fn vectors_health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, ProtoError> {
    let vectors = state
        .vectors
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;
    let dict = state
        .dict
        .read()
        .map_err(|e| ProtoError::BadRequest(format!("lock: {}", e)))?;

    let mut indexes = Vec::new();
    for pred_id in vectors.predicates() {
        if let Some(index) = vectors.get(pred_id) {
            let pred_name = dict.resolve(pred_id).unwrap_or("unknown");
            indexes.push(serde_json::json!({
                "predicate": pred_name,
                "predicate_id": pred_id,
                "total_nodes": index.len(),
                "active_nodes": index.active_count(),
                "deleted_ratio": index.deleted_ratio(),
                "dimensions": index.dimensions(),
                "metric": format!("{:?}", index.metric()),
                "needs_compaction": index.deleted_ratio() > 0.3,
            }));
        }
    }

    Ok(Json(serde_json::json!({
        "index_count": indexes.len(),
        "total_edge_count": vectors.total_edge_count(),
        "indexes": indexes,
    })))
}

/// GET /service-description — SPARQL service description (Turtle).
async fn service_description(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let store = state.store.read().ok();
    let triple_count = store.as_ref().map(|s| s.len()).unwrap_or(0);

    let ttl = format!(
        r#"@prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
@prefix void: <http://rdfs.org/ns/void#> .

<> a sd:Service ;
    sd:endpoint <sparql> ;
    sd:supportedLanguage sd:SPARQL11Query ;
    sd:resultFormat <http://www.w3.org/ns/formats/SPARQL_Results_JSON> ,
                    <http://www.w3.org/ns/formats/SPARQL_Results_CSV> ,
                    <http://www.w3.org/ns/formats/SPARQL_Results_TSV> ;
    sd:feature sd:BasicFederatedQuery ;
    sd:defaultDataset [
        a sd:Dataset ;
        sd:defaultGraph [
            a sd:Graph , void:Dataset ;
            void:triples {} ;
        ]
    ] .
"#,
        triple_count
    );

    ([(header::CONTENT_TYPE, "text/turtle; charset=utf-8")], ttl)
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
            store: RwLock::new(store),
            dict: RwLock::new(dict),
            vectors: RwLock::new(VectorRegistry::new()),
            persistent: None,
            passcode: None,
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
    async fn export_graph_turtle() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/graph")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.contains("text/turtle"));

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();

        // Should contain the Alice-knows-Bob triple
        assert!(text.contains("Alice"));
        assert!(text.contains("knows"));
        assert!(text.contains("Bob"));
    }

    #[tokio::test]
    async fn export_graph_ntriples() {
        let app = router(test_state());
        let req = Request::builder()
            .uri("/graph?format=nt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let content_type = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.contains("n-triples"));

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();

        // N-Triples: each line ends with " ."
        for line in text.lines() {
            assert!(line.trim().ends_with('.'), "bad line: {}", line);
        }
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
