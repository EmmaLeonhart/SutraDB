#![allow(clippy::type_complexity, clippy::too_many_arguments)]
//! Query executor.
//!
//! Evaluates parsed SPARQL queries against a TripleStore + TermDictionary +
//! VectorRegistry. Uses the Volcano/iterator model: each pattern produces
//! a stream of binding rows that are joined together.
//!
//! VECTOR_SIMILAR is a first-class pattern: the executor calls into the
//! HNSW index via the VectorRegistry, joining results back into the
//! binding table like any other index access.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sutra_core::{
    DatabaseConfig, HnswEdgeMode, Property, PropertyPosition, PseudoTableRegistry, TermDictionary,
    TermId, Triple, TripleStore,
};
use sutra_hnsw::VectorRegistry;

use crate::error::{Result, SparqlError};
use crate::parser::{
    Aggregate, AggregateArg, AggregateFunction, FilterExpr, PathModifier, Pattern, Query,
    QueryType, Term,
};

/// A single row of variable bindings.
pub type Bindings = HashMap<String, TermId>;

/// Result of executing a query: column names + rows of term IDs.
#[derive(Debug)]
pub struct QueryResult {
    /// Variable names in projection order.
    pub columns: Vec<String>,
    /// Rows of bindings. Each row maps variable name → TermId.
    pub rows: Vec<Bindings>,
    /// Optional similarity scores per row (populated by VECTOR_SIMILAR).
    /// Key is "variable:predicate", value is similarity score.
    pub scores: Vec<HashMap<String, f32>>,
}

/// Execution context holding all the state needed during query evaluation.
pub struct ExecutionContext<'a> {
    pub store: &'a TripleStore,
    pub dict: &'a TermDictionary,
    pub vectors: &'a VectorRegistry,
    pub prefixes: &'a HashMap<String, String>,
    pub config: &'a DatabaseConfig,
    /// Optional query timeout deadline.
    pub deadline: Option<Instant>,
    /// Optional pseudo-table registry for columnar query acceleration.
    /// When available, the executor checks if a triple pattern can be served
    /// by a pseudo-table scan (with zonemap pruning) instead of a general
    /// SPO/POS/OSP index scan. This is the RDF equivalent of a SQL engine
    /// choosing between a columnar scan and a B-tree lookup.
    pub pseudo_tables: Option<&'a PseudoTableRegistry>,
}

/// Execute a parsed query against an in-memory store with vector support.
pub fn execute_with_vectors(
    query: &Query,
    store: &TripleStore,
    dict: &TermDictionary,
    vectors: &VectorRegistry,
) -> Result<QueryResult> {
    let default_config = DatabaseConfig::default();
    execute_with_config(query, store, dict, vectors, &default_config)
}

/// Execute a parsed query with full configuration control.
pub fn execute_with_config(
    query: &Query,
    store: &TripleStore,
    dict: &TermDictionary,
    vectors: &VectorRegistry,
    config: &DatabaseConfig,
) -> Result<QueryResult> {
    execute_with_deadline(query, store, dict, vectors, config, None)
}

/// Core query execution logic.
fn execute_query_with_ctx(query: &Query, ctx: &mut ExecutionContext<'_>) -> Result<QueryResult> {
    // Start with a single empty binding
    let mut results: Vec<Bindings> = vec![HashMap::new()];
    let mut scores: Vec<HashMap<String, f32>> = vec![HashMap::new()];

    // LIMIT push-down: only safe when no ORDER BY, DISTINCT, or VECTOR_SIMILAR.
    // If there's a vector pattern, early truncation would discard candidates
    // that the vector search needs to match against.
    let has_vector_pattern = query
        .patterns
        .iter()
        .any(|p| matches!(p, Pattern::VectorSimilar { .. }));
    let pushable_limit = if query.order_by.is_empty() && !query.distinct && !has_vector_pattern {
        query.limit.map(|l| l + query.offset.unwrap_or(0))
    } else {
        None
    };

    // Evaluate each pattern, threading the pushable limit through
    for pattern in &query.patterns {
        let (new_results, new_scores) =
            evaluate_pattern(pattern, &results, &scores, ctx, pushable_limit)?;
        results = new_results;
        scores = new_scores;

        // Early termination when we have enough rows
        if let Some(limit) = pushable_limit {
            if results.len() >= limit {
                results.truncate(limit);
                scores.truncate(limit);
            }
        }
    }

    // DESCRIBE query: return all triples about the described resource
    if query.query_type == QueryType::Describe {
        let mut all_triples = Vec::new();
        let mut all_scores = Vec::new();

        for row in &results {
            // For each result row, get the described resource
            for var_or_iri in &query.projection {
                let resource_id = if let Some(&id) = row.get(var_or_iri) {
                    Some(id)
                } else {
                    // It's a direct IRI, not a variable
                    ctx.dict.lookup(var_or_iri)
                };

                if let Some(id) = resource_id {
                    // Get all triples where this is the subject
                    for triple in ctx.store.find_by_subject(id) {
                        let mut r = HashMap::new();
                        r.insert("subject".to_string(), triple.subject);
                        r.insert("predicate".to_string(), triple.predicate);
                        r.insert("object".to_string(), triple.object);
                        all_triples.push(r);
                        all_scores.push(HashMap::new());
                    }
                    // Get all triples where this is the object
                    for triple in ctx.store.find_by_object(id) {
                        let mut r = HashMap::new();
                        r.insert("subject".to_string(), triple.subject);
                        r.insert("predicate".to_string(), triple.predicate);
                        r.insert("object".to_string(), triple.object);
                        all_triples.push(r);
                        all_scores.push(HashMap::new());
                    }
                }
            }
        }

        return Ok(QueryResult {
            columns: vec![
                "subject".to_string(),
                "predicate".to_string(),
                "object".to_string(),
            ],
            rows: all_triples,
            scores: all_scores,
        });
    }

    // CONSTRUCT query: instantiate template with each result row
    if query.query_type == QueryType::Construct {
        let mut constructed = Vec::new();
        let mut constructed_scores = Vec::new();

        for row in &results {
            for pattern in &query.construct_template {
                if let Pattern::Triple {
                    subject,
                    predicate,
                    object,
                } = pattern
                {
                    let s = resolve_term(subject, row, ctx.dict, ctx.prefixes)?;
                    let p = resolve_term(predicate, row, ctx.dict, ctx.prefixes)?;
                    let o = resolve_term(object, row, ctx.dict, ctx.prefixes)?;

                    if let (Some(s_id), Some(p_id), Some(o_id)) = (s, p, o) {
                        let mut r = HashMap::new();
                        r.insert("subject".to_string(), s_id);
                        r.insert("predicate".to_string(), p_id);
                        r.insert("object".to_string(), o_id);
                        constructed.push(r);
                        constructed_scores.push(HashMap::new());
                    }
                }
            }
        }

        return Ok(QueryResult {
            columns: vec![
                "subject".to_string(),
                "predicate".to_string(),
                "object".to_string(),
            ],
            rows: constructed,
            scores: constructed_scores,
        });
    }

    // ASK query: return a single boolean row
    if query.query_type == QueryType::Ask {
        let has_results = !results.is_empty();
        let mut row = HashMap::new();
        row.insert(
            "result".to_string(),
            if has_results {
                sutra_core::inline_boolean(true)
            } else {
                sutra_core::inline_boolean(false)
            },
        );
        return Ok(QueryResult {
            columns: vec!["result".to_string()],
            rows: vec![row],
            scores: vec![HashMap::new()],
        });
    }

    // Apply GROUP BY + Aggregates
    if !query.group_by.is_empty() || !query.aggregates.is_empty() {
        let (grouped_results, grouped_scores) =
            apply_group_by_and_aggregates(&results, &query.group_by, &query.aggregates, ctx)?;
        results = grouped_results;
        scores = grouped_scores;
    }

    // Apply HAVING (filter on aggregated results)
    if let Some(ref having_expr) = query.having {
        let mut filtered = Vec::new();
        let mut filtered_scores = Vec::new();
        for (i, row) in results.iter().enumerate() {
            if evaluate_filter(having_expr, row, ctx) {
                filtered.push(row.clone());
                filtered_scores.push(scores[i].clone());
            }
        }
        results = filtered;
        scores = filtered_scores;
    }

    // Apply ORDER BY
    if !query.order_by.is_empty() {
        apply_order_by(&mut results, &mut scores, &query.order_by, ctx)?;
    }

    // Apply DISTINCT (only considers projected variables, per SPARQL spec)
    if query.distinct {
        let mut seen = std::collections::HashSet::new();
        let mut keep = Vec::new();
        let proj_vars = &query.projection;
        for (i, row) in results.iter().enumerate() {
            let key: Vec<_> = if proj_vars.is_empty() {
                // SELECT * — compare all variables
                let mut pairs: Vec<_> = row.iter().collect();
                pairs.sort_by_key(|(k, _)| (*k).clone());
                pairs.into_iter().map(|(k, v)| (k.clone(), *v)).collect()
            } else {
                // Compare only projected variables
                proj_vars
                    .iter()
                    .map(|v| (v.clone(), row.get(v).copied().unwrap_or(0)))
                    .collect()
            };
            let key_str = format!("{:?}", key);
            if seen.insert(key_str) {
                keep.push(i);
            }
        }
        results = keep.iter().map(|&i| results[i].clone()).collect();
        scores = keep.iter().map(|&i| scores[i].clone()).collect();
    }

    // Apply OFFSET
    if let Some(offset) = query.offset {
        if offset < results.len() {
            results = results[offset..].to_vec();
            scores = scores[offset..].to_vec();
        } else {
            results.clear();
            scores.clear();
        }
    }

    // Apply LIMIT
    if let Some(limit) = query.limit {
        results.truncate(limit);
        scores.truncate(limit);
    }

    // Determine columns
    let columns = if query.projection.is_empty() {
        let mut vars: Vec<String> = results.iter().flat_map(|row| row.keys().cloned()).collect();
        vars.sort();
        vars.dedup();
        vars
    } else {
        query.projection.clone()
    };

    Ok(QueryResult {
        columns,
        rows: results,
        scores,
    })
}

/// Execute a parsed query with a timeout in seconds.
/// Returns `Err(SparqlError::Timeout)` if the query exceeds the time limit.
pub fn execute_with_timeout(
    query: &Query,
    store: &TripleStore,
    dict: &TermDictionary,
    vectors: &VectorRegistry,
    timeout_secs: u64,
) -> Result<QueryResult> {
    let default_config = DatabaseConfig::default();
    execute_with_deadline(
        query,
        store,
        dict,
        vectors,
        &default_config,
        Some(Instant::now() + Duration::from_secs(timeout_secs)),
    )
}

/// Internal executor that accepts an optional deadline.
fn execute_with_deadline(
    query: &Query,
    store: &TripleStore,
    dict: &TermDictionary,
    vectors: &VectorRegistry,
    config: &DatabaseConfig,
    deadline: Option<Instant>,
) -> Result<QueryResult> {
    let mut ctx = ExecutionContext {
        store,
        dict,
        vectors,
        prefixes: &query.prefixes,
        config,
        deadline,
        pseudo_tables: None,
    };
    execute_query_with_ctx(query, &mut ctx)
}

/// Execute a parsed query without vector support (backward compatible).
pub fn execute(query: &Query, store: &TripleStore, dict: &TermDictionary) -> Result<QueryResult> {
    let empty_registry = VectorRegistry::new();
    execute_with_vectors(query, store, dict, &empty_registry)
}

fn evaluate_pattern(
    pattern: &Pattern,
    current: &[Bindings],
    current_scores: &[HashMap<String, f32>],
    ctx: &mut ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<(Vec<Bindings>, Vec<HashMap<String, f32>>)> {
    // Check timeout before evaluating each pattern
    check_deadline(ctx)?;

    match pattern {
        Pattern::Triple {
            subject,
            predicate: Term::Path { base, modifier },
            object,
        } => {
            // Property path evaluation
            evaluate_property_path(
                subject,
                base,
                modifier,
                object,
                current,
                current_scores,
                ctx,
            )
        }
        Pattern::Triple {
            subject,
            predicate,
            object,
        } => {
            let (rows, _) =
                evaluate_triple_pattern(subject, predicate, object, current, ctx, row_limit)?;
            // Carry forward scores from current rows (expand for each new match)
            let new_scores = expand_scores(current, current_scores, &rows);
            Ok((rows, new_scores))
        }
        Pattern::Optional(inner_patterns) => {
            let mut result = Vec::new();
            let mut result_scores = Vec::new();
            for (i, row) in current.iter().enumerate() {
                let row_score = &current_scores[i];
                let mut inner_results = vec![row.clone()];
                let mut inner_scores = vec![row_score.clone()];
                for p in inner_patterns {
                    let (new_results, new_s) =
                        evaluate_pattern(p, &inner_results, &inner_scores, ctx, None)?;
                    inner_results = new_results;
                    inner_scores = new_s;
                }
                if inner_results.is_empty() {
                    result.push(row.clone());
                    result_scores.push(row_score.clone());
                } else {
                    result.extend(inner_results);
                    result_scores.extend(inner_scores);
                }
            }
            Ok((result, result_scores))
        }
        Pattern::Filter(expr) => {
            let mut filtered = Vec::new();
            let mut filtered_scores = Vec::new();
            for (i, row) in current.iter().enumerate() {
                if evaluate_filter(expr, row, ctx) {
                    filtered.push(row.clone());
                    filtered_scores.push(current_scores[i].clone());
                }
            }
            Ok((filtered, filtered_scores))
        }
        Pattern::VectorSimilar {
            subject,
            predicate,
            query_vector,
            threshold,
            ef_search,
            top_k,
        } => evaluate_vector_similar(
            subject,
            predicate,
            query_vector,
            *threshold,
            *ef_search,
            *top_k,
            current,
            current_scores,
            ctx,
        ),
        Pattern::Union(branches) => {
            let mut result = Vec::new();
            let mut result_scores = Vec::new();
            for branch in branches {
                let mut branch_results = current.to_vec();
                let mut branch_scores = current_scores.to_vec();
                for p in branch {
                    let (new_results, new_s) =
                        evaluate_pattern(p, &branch_results, &branch_scores, ctx, None)?;
                    branch_results = new_results;
                    branch_scores = new_s;
                }
                result.extend(branch_results);
                result_scores.extend(branch_scores);
            }
            Ok((result, result_scores))
        }
        Pattern::Bind {
            expression,
            variable,
        } => {
            // BIND(term AS ?var): resolve the term and add it as a binding
            let mut result = Vec::new();
            let mut result_scores = Vec::new();
            for (i, row) in current.iter().enumerate() {
                let value = resolve_term(expression, row, ctx.dict, ctx.prefixes)?;
                if let Some(id) = value {
                    let mut new_row = row.clone();
                    new_row.insert(variable.clone(), id);
                    result.push(new_row);
                    result_scores.push(current_scores[i].clone());
                } else {
                    // If the expression can't be resolved, keep the row without the binding
                    result.push(row.clone());
                    result_scores.push(current_scores[i].clone());
                }
            }
            Ok((result, result_scores))
        }
        Pattern::Subquery(inner_query) => {
            // Execute the subquery independently, then join results
            let sub_result = execute_query_with_ctx(inner_query, ctx)?;
            let mut result = Vec::new();
            let mut result_scores = Vec::new();
            for (i, outer_row) in current.iter().enumerate() {
                for sub_row in &sub_result.rows {
                    // Check if bindings are compatible
                    let mut compatible = true;
                    let mut merged = outer_row.clone();
                    for (var, &val) in sub_row {
                        if let Some(&existing) = merged.get(var) {
                            if existing != val {
                                compatible = false;
                                break;
                            }
                        } else {
                            merged.insert(var.clone(), val);
                        }
                    }
                    if compatible {
                        result.push(merged);
                        result_scores.push(current_scores[i].clone());
                    }
                }
            }
            Ok((result, result_scores))
        }
        Pattern::Values { variable, values } => {
            // VALUES ?var { val1 val2 ... }: cross-join current rows with each value
            let mut result = Vec::new();
            let mut result_scores = Vec::new();
            for (i, row) in current.iter().enumerate() {
                for value_term in values {
                    if let Some(id) = resolve_term(value_term, row, ctx.dict, ctx.prefixes)? {
                        let mut new_row = row.clone();
                        new_row.insert(variable.clone(), id);
                        result.push(new_row);
                        result_scores.push(current_scores[i].clone());
                    }
                }
            }
            Ok((result, result_scores))
        }
    }
}

/// Execute a VECTOR_SIMILAR pattern against the VectorRegistry.
///
/// Vectors are objects (primitives) in the graph. The HNSW index is keyed
/// by the vector object's TermId. VECTOR_SIMILAR searches the index for
/// matching vector objects, then resolves back through the triple store
/// to find which subjects connect to those vectors.
///
/// This supports the "bank" disambiguation case: two entities can point
/// to the same vector, and VECTOR_SIMILAR finds both.
///
/// Two strategies:
/// - Subject bound: check if the bound subject has a vector above threshold
/// - Subject unbound: search vectors, then find all subjects pointing to them
fn evaluate_vector_similar(
    subject: &Term,
    predicate: &Term,
    query_vector: &[f32],
    threshold: f32,
    ef_search: Option<usize>,
    top_k: Option<usize>,
    current: &[Bindings],
    current_scores: &[HashMap<String, f32>],
    ctx: &mut ExecutionContext<'_>,
) -> Result<(Vec<Bindings>, Vec<HashMap<String, f32>>)> {
    let pred_id = resolve_term(predicate, &HashMap::new(), ctx.dict, ctx.prefixes)?
        .ok_or_else(|| SparqlError::Vector("vector predicate not found in dictionary".into()))?;

    if !ctx.vectors.has_index(pred_id) {
        return Err(SparqlError::Vector(format!(
            "no vector index declared for predicate ID {}",
            pred_id
        )));
    }

    // Higher defaults for better recall across clustered data.
    // ef=500 ensures the beam search explores enough of the graph to
    // bridge between distant clusters. k=500 returns enough candidates
    // before threshold filtering.
    let ef = ef_search.unwrap_or(500);
    let k = top_k.unwrap_or(500);

    let var_name = match subject {
        Term::Variable(name) => name.clone(),
        _ => "_bound".to_string(),
    };
    let score_key = format!("{}:{}", var_name, pred_id);

    let mut results = Vec::new();
    let mut result_scores = Vec::new();

    let subject_var = match subject {
        Term::Variable(name) => Some(name.as_str()),
        _ => None,
    };

    // Run HNSW search once — results are vector object IDs
    let search_results = ctx
        .vectors
        .search(pred_id, query_vector, k, ef)
        .map_err(SparqlError::Hnsw)?;

    for (i, row) in current.iter().enumerate() {
        let subject_bound = subject_var
            .and_then(|name| row.get(name).copied())
            .or_else(|| {
                resolve_term(subject, row, ctx.dict, ctx.prefixes)
                    .ok()
                    .flatten()
            });

        if let Some(bound_subject_id) = subject_bound {
            // Subject is bound: check if this subject has any vector above threshold.
            // Look up all triples where this subject has the vector predicate,
            // then check if any of those vector objects are in the search results.
            let subject_vectors = ctx
                .store
                .find_by_subject_predicate(bound_subject_id, pred_id);
            for triple in &subject_vectors {
                for sr in &search_results {
                    if sr.triple_id == triple.object && sr.score >= threshold {
                        let new_row = row.clone();
                        let mut new_score = current_scores[i].clone();
                        new_score.insert(score_key.clone(), sr.score);
                        results.push(new_row);
                        result_scores.push(new_score);
                        break;
                    }
                }
            }
        } else {
            // Subject is unbound: for each matching vector object, find all
            // subjects that point to it via the predicate (reverse traversal).
            for sr in &search_results {
                if sr.score < threshold {
                    continue;
                }
                // Find subjects that have this vector as object
                let pointing_triples = ctx.store.find_by_predicate_object(pred_id, sr.triple_id);

                if pointing_triples.is_empty() {
                    // Fallback: try binding the vector object ID directly
                    // (for backward compat with tests that don't create triples)
                    let mut new_row = row.clone();
                    let mut new_score = current_scores[i].clone();
                    new_score.insert(score_key.clone(), sr.score);
                    if let Term::Variable(name) = subject {
                        new_row.insert(name.clone(), sr.triple_id);
                    }
                    results.push(new_row);
                    result_scores.push(new_score);
                } else {
                    // Bind each subject that points to this vector
                    for triple in &pointing_triples {
                        let mut new_row = row.clone();
                        let mut new_score = current_scores[i].clone();
                        new_score.insert(score_key.clone(), sr.score);
                        if let Term::Variable(name) = subject {
                            new_row.insert(name.clone(), triple.subject);
                        }
                        results.push(new_row);
                        result_scores.push(new_score);
                    }
                }
            }
        }
    }

    Ok((results, result_scores))
}

/// Apply ORDER BY clauses to the result set.
fn apply_order_by(
    results: &mut Vec<Bindings>,
    scores: &mut Vec<HashMap<String, f32>>,
    order_by: &[crate::parser::OrderClause],
    ctx: &mut ExecutionContext<'_>,
) -> Result<()> {
    // Build index array and sort that
    let mut indices: Vec<usize> = (0..results.len()).collect();

    // For VECTOR_SCORE expressions, compute scores if not already available
    for clause in order_by {
        if let Some(vs) = &clause.vector_score {
            let pred_id = resolve_term(&vs.predicate, &HashMap::new(), ctx.dict, ctx.prefixes)?
                .ok_or_else(|| SparqlError::Vector("VECTOR_SCORE predicate not found".into()))?;

            if ctx.vectors.has_index(pred_id) {
                let var_name = match &vs.subject {
                    Term::Variable(name) => name.clone(),
                    _ => "_bound".to_string(),
                };
                let score_key = format!("{}:{}", var_name, pred_id);

                // Compute scores for rows that don't have them yet
                let search_results = ctx
                    .vectors
                    .search(pred_id, &vs.query_vector, 1000, 200)
                    .map_err(SparqlError::Hnsw)?;

                let score_map: HashMap<TermId, f32> = search_results
                    .into_iter()
                    .map(|sr| (sr.triple_id, sr.score))
                    .collect();

                for (i, row) in results.iter().enumerate() {
                    if !scores[i].contains_key(&score_key) {
                        if let Some(term_id) = row.get(&var_name).copied() {
                            if let Some(&s) = score_map.get(&term_id) {
                                scores[i].insert(score_key.clone(), s);
                            }
                        }
                    }
                }
            }
        }
    }

    indices.sort_by(|&a, &b| {
        for clause in order_by {
            let cmp = if let Some(vs) = &clause.vector_score {
                let var_name = match &vs.subject {
                    Term::Variable(name) => name.clone(),
                    _ => "_bound".to_string(),
                };
                // Look up score from the scores map
                let pred_id_str = format!("{}:", var_name);
                let score_a = scores[a]
                    .iter()
                    .find(|(k, _)| k.starts_with(&pred_id_str))
                    .map(|(_, v)| *v)
                    .unwrap_or(f32::NEG_INFINITY);
                let score_b = scores[b]
                    .iter()
                    .find(|(k, _)| k.starts_with(&pred_id_str))
                    .map(|(_, v)| *v)
                    .unwrap_or(f32::NEG_INFINITY);
                score_a
                    .partial_cmp(&score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else {
                // Sort by variable value
                let val_a = results[a].get(&clause.variable).copied().unwrap_or(0);
                let val_b = results[b].get(&clause.variable).copied().unwrap_or(0);
                val_a.cmp(&val_b)
            };

            let cmp = if clause.descending {
                cmp.reverse()
            } else {
                cmp
            };
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    });

    let sorted_results: Vec<Bindings> = indices.iter().map(|&i| results[i].clone()).collect();
    let sorted_scores: Vec<HashMap<String, f32>> =
        indices.iter().map(|&i| scores[i].clone()).collect();
    *results = sorted_results;
    *scores = sorted_scores;

    Ok(())
}

/// Evaluate a triple pattern against the store, joining with current bindings.
///
/// ## Join strategy selection
///
/// The executor selects between three join strategies based on the
/// intermediate result size and binding pattern. This mirrors SQL engine
/// behavior where the optimizer picks between hash join, merge join, and
/// nested-loop join based on cost estimates.
///
/// | Strategy | When used | Cost model |
/// |----------|-----------|------------|
/// | **Nested-loop** | current < 50 rows | O(N * scan_cost) — cheapest for small N |
/// | **Hash join** | current >= 50, join variable bound | O(N + M) — groups by join key, batch lookup |
/// | **Object hash join** | current >= 50, object variable bound | O(N + M) — same as above, keyed on object |
///
/// The threshold of 50 rows (down from 100 in v0.1) is based on empirical
/// observation that the hash join's O(1) amortized lookup beats nested-loop
/// earlier than expected due to BTreeSet range scan overhead in the store.
///
/// ## Index selection
///
/// Within each join strategy, the most selective index is chosen based on
/// which triple positions are bound:
///
/// | Bound positions | Index used | Selectivity |
/// |-----------------|-----------|-------------|
/// | S + P | SPO prefix scan | Highest (typical: 1-10 results) |
/// | S only | SPO prefix scan | High (typical: 5-50 results) |
/// | P + O | POS prefix scan | Medium (depends on predicate) |
/// | P only | POS prefix scan | Low (can be large for common predicates) |
/// | O only | OSP prefix scan | Medium (reverse traversal) |
/// | None | Full scan | Lowest (avoid if possible) |
fn evaluate_triple_pattern(
    subject: &Term,
    predicate: &Term,
    object: &Term,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<(Vec<Bindings>, Vec<usize>)> {
    // Check if this is a virtual HNSW edge query.
    // When config says Virtual mode, intercept sutra:hnswNeighbor and typed
    // HNSW predicates (hnswHorizontalNeighbor, hnswLayerDescend).
    if ctx.config.hnsw_edge_mode == HnswEdgeMode::Virtual {
        if let Some(result) =
            try_evaluate_hnsw_edge_pattern(subject, predicate, object, current, ctx, row_limit)?
        {
            return Ok(result);
        }
    }

    // --- Pseudo-table acceleration ---
    //
    // Before falling through to the general-purpose SPO/POS/OSP index lookup,
    // check if this pattern can be served by a pseudo-table scan. Pseudo-tables
    // offer columnar storage with zonemap pruning, which is faster for patterns
    // that match a characteristic set (e.g., all Person nodes with name+age+city).
    //
    // The check: if the predicate is a constant (IRI) and maps to a pseudo-table
    // column, and the current result set is small (initial scan or few rows),
    // use the pseudo-table's vectorized column scan instead.
    if let Some(registry) = ctx.pseudo_tables {
        if let Some(result) = try_pseudo_table_scan(
            subject, predicate, object, current, ctx, registry, row_limit,
        )? {
            return Ok(result);
        }
    }

    let mut results = Vec::new();
    let mut source_indices = Vec::new();

    // --- Join strategy selection ---
    //
    // We choose the join strategy based on intermediate result size and
    // which variables are bound. The key insight from SQL query optimization:
    // hash joins amortize their build cost over many probes, so they win
    // when the build side (current rows grouped by join key) is large enough
    // to offset the HashMap overhead.
    //
    // Threshold: 50 rows. Below this, nested-loop with index lookup is faster
    // because there's no HashMap construction overhead.
    const HASH_JOIN_THRESHOLD: usize = 50;

    // Strategy 1: Hash join on subject variable.
    // When the subject variable is bound in all current rows, we can group
    // rows by subject and do a single index lookup per unique subject,
    // then cross-product with all rows sharing that subject. This avoids
    // redundant index scans when many rows share the same subject.
    if current.len() >= HASH_JOIN_THRESHOLD {
        if let Term::Variable(subj_var) = subject {
            if current.iter().all(|r| r.contains_key(subj_var)) {
                return hash_join_on_subject(subj_var, predicate, object, current, ctx, row_limit);
            }
        }

        // Strategy 2: Hash join on object variable.
        // Same idea as above, but keyed on the object position. Useful for
        // reverse traversal patterns like `?s :knows <Alice>` where the
        // object is bound and we want to batch-lookup by object.
        if let Term::Variable(obj_var) = object {
            if current.iter().all(|r| r.contains_key(obj_var)) {
                return hash_join_on_object(subject, predicate, obj_var, current, ctx, row_limit);
            }
        }
    }

    // Strategy 3: Nested-loop join (fallback).
    // For each row in current, resolve bound terms and do an index lookup.
    // This is the simplest strategy and optimal for small intermediate results
    // (< 50 rows) where the per-row overhead is minimal.

    'outer: for (row_idx, row) in current.iter().enumerate() {
        let s_id = resolve_term(subject, row, ctx.dict, ctx.prefixes)?;
        let p_id = resolve_term(predicate, row, ctx.dict, ctx.prefixes)?;
        let o_id = resolve_term(object, row, ctx.dict, ctx.prefixes)?;

        if is_concrete(subject) && s_id.is_none() {
            continue;
        }
        if is_concrete(predicate) && p_id.is_none() {
            continue;
        }
        if is_concrete(object) && o_id.is_none() {
            continue;
        }

        // Pick the most selective index based on which terms are bound
        let candidates: Vec<Triple> = match (s_id, p_id, o_id) {
            (Some(s), Some(p), _) => ctx.store.find_by_subject_predicate(s, p),
            (Some(s), None, _) => ctx.store.find_by_subject(s),
            (None, Some(p), Some(o)) => ctx.store.find_by_predicate_object(p, o),
            (None, Some(p), None) => ctx.store.find_by_predicate(p),
            (None, None, Some(o)) => ctx.store.find_by_object(o),
            (None, None, None) => ctx.store.iter().collect(),
        };

        for triple in candidates {
            if let Some(s) = s_id {
                if triple.subject != s {
                    continue;
                }
            }
            if let Some(p) = p_id {
                if triple.predicate != p {
                    continue;
                }
            }
            if let Some(o) = o_id {
                if triple.object != o {
                    continue;
                }
            }

            let mut new_row = row.clone();
            if let Term::Variable(name) = subject {
                if let Some(&existing) = new_row.get(name) {
                    if existing != triple.subject {
                        continue;
                    }
                } else {
                    new_row.insert(name.clone(), triple.subject);
                }
            }
            if let Term::Variable(name) = predicate {
                if let Some(&existing) = new_row.get(name) {
                    if existing != triple.predicate {
                        continue;
                    }
                } else {
                    new_row.insert(name.clone(), triple.predicate);
                }
            }
            if let Term::Variable(name) = object {
                if let Some(&existing) = new_row.get(name) {
                    if existing != triple.object {
                        continue;
                    }
                } else {
                    new_row.insert(name.clone(), triple.object);
                }
            }

            results.push(new_row);
            source_indices.push(row_idx);

            // Early termination: stop when we have enough rows
            if let Some(limit) = row_limit {
                if results.len() >= limit {
                    break 'outer;
                }
            }
        }
    }

    Ok((results, source_indices))
}

/// Hash join strategy: group current rows by their subject variable binding,
/// then batch-lookup triples for each unique subject.
///
/// This avoids redundant index scans when many rows share the same subject.
/// For example, if 100 rows all have `?person = :Alice`, we do ONE
/// `find_by_subject_predicate(:Alice, :knows)` instead of 100 separate lookups.
///
/// ## Complexity
/// - Build phase: O(N) to group rows by subject → HashMap<TermId, Vec<row_idx>>
/// - Probe phase: O(K * M) where K = unique subjects, M = avg matches per subject
/// - Total: O(N + K*M) vs O(N*M) for nested-loop when K << N
fn hash_join_on_subject(
    subj_var: &str,
    predicate: &Term,
    object: &Term,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<(Vec<Bindings>, Vec<usize>)> {
    let mut results = Vec::new();
    let mut source_indices = Vec::new();

    // Build phase: group rows by their subject binding.
    // This is the "build side" of the hash join — we construct a lookup
    // table mapping each unique subject to the row indices that share it.
    let mut grouped: HashMap<TermId, Vec<usize>> = HashMap::new();
    for (idx, row) in current.iter().enumerate() {
        if let Some(&sid) = row.get(subj_var) {
            grouped.entry(sid).or_default().push(idx);
        }
    }

    // Probe phase: for each unique subject, do a single index lookup
    // and cross-product the results with all rows sharing that subject.
    for (sid, row_indices) in &grouped {
        let p_id_first = resolve_term(predicate, &current[row_indices[0]], ctx.dict, ctx.prefixes)?;
        let candidates = if let Some(pid) = p_id_first {
            ctx.store.find_by_subject_predicate(*sid, pid)
        } else {
            ctx.store.find_by_subject(*sid)
        };

        for triple in &candidates {
            for &row_idx in row_indices {
                let row = &current[row_idx];
                let mut new_row = row.clone();

                if let Term::Variable(name) = predicate {
                    new_row.insert(name.clone(), triple.predicate);
                }
                if let Some(oid) = resolve_term(object, row, ctx.dict, ctx.prefixes)? {
                    if triple.object != oid {
                        continue;
                    }
                }
                if let Term::Variable(name) = object {
                    if let Some(&existing) = new_row.get(name) {
                        if existing != triple.object {
                            continue;
                        }
                    } else {
                        new_row.insert(name.clone(), triple.object);
                    }
                }

                results.push(new_row);
                source_indices.push(row_idx);
                if let Some(limit) = row_limit {
                    if results.len() >= limit {
                        return Ok((results, source_indices));
                    }
                }
            }
        }
    }

    Ok((results, source_indices))
}

/// Hash join strategy keyed on the object variable.
///
/// Symmetric to `hash_join_on_subject` but groups by the object position.
/// This is optimal for reverse-traversal patterns where many rows share
/// the same object binding (e.g., "find all subjects that point to ?x").
///
/// Uses the POS index (predicate-object-subject) for efficient lookup when
/// the predicate is also bound, or the OSP index (object-subject-predicate)
/// when only the object is bound.
fn hash_join_on_object(
    subject: &Term,
    predicate: &Term,
    obj_var: &str,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<(Vec<Bindings>, Vec<usize>)> {
    let mut results = Vec::new();
    let mut source_indices = Vec::new();

    // Build phase: group rows by their object binding.
    let mut grouped: HashMap<TermId, Vec<usize>> = HashMap::new();
    for (idx, row) in current.iter().enumerate() {
        if let Some(&oid) = row.get(obj_var) {
            grouped.entry(oid).or_default().push(idx);
        }
    }

    // Probe phase: for each unique object, lookup via POS or OSP index.
    for (oid, row_indices) in &grouped {
        let p_id_first = resolve_term(predicate, &current[row_indices[0]], ctx.dict, ctx.prefixes)?;

        // Choose the most selective index based on what's bound.
        // If predicate is bound → POS index (predicate + object prefix scan).
        // If predicate is unbound → OSP index (object prefix scan).
        let candidates = if let Some(pid) = p_id_first {
            ctx.store.find_by_predicate_object(pid, *oid)
        } else {
            ctx.store.find_by_object(*oid)
        };

        for triple in &candidates {
            for &row_idx in row_indices {
                let row = &current[row_idx];
                let mut new_row = row.clone();

                // Bind or check subject variable.
                if let Term::Variable(name) = subject {
                    if let Some(&existing) = new_row.get(name) {
                        if existing != triple.subject {
                            continue;
                        }
                    } else {
                        new_row.insert(name.clone(), triple.subject);
                    }
                } else if let Some(sid) = resolve_term(subject, row, ctx.dict, ctx.prefixes)? {
                    if triple.subject != sid {
                        continue;
                    }
                }

                // Bind predicate variable if unbound.
                if let Term::Variable(name) = predicate {
                    if let Some(&existing) = new_row.get(name) {
                        if existing != triple.predicate {
                            continue;
                        }
                    } else {
                        new_row.insert(name.clone(), triple.predicate);
                    }
                }

                results.push(new_row);
                source_indices.push(row_idx);
                if let Some(limit) = row_limit {
                    if results.len() >= limit {
                        return Ok((results, source_indices));
                    }
                }
            }
        }
    }

    Ok((results, source_indices))
}

/// Try to evaluate a triple pattern using pseudo-table columnar scans.
///
/// Returns `Some((results, source_indices))` if the pattern matches a pseudo-table
/// column and the columnar scan is beneficial. Returns `None` to fall through to
/// the general-purpose SPO/POS/OSP index scan.
///
/// ## When pseudo-tables are used
///
/// A pseudo-table scan is attempted when:
/// 1. The predicate is a constant (IRI, not a variable) — we need to know
///    which column to scan.
/// 2. The predicate maps to a column in at least one pseudo-table.
/// 3. The pseudo-table scan is expected to be faster than the triple store scan.
///    Currently, pseudo-tables win for initial scans (current = 1 empty row)
///    because zonemap pruning can skip entire segments.
///
/// ## How it works
///
/// For a pattern like `?s :name ?name`:
/// 1. Find pseudo-tables with a column for Property(:name, Subject).
/// 2. For each matching table, scan all segments using zonemap pruning.
/// 3. For matching rows, bind ?s to the node TermId and ?name to the column value.
fn try_pseudo_table_scan(
    subject: &Term,
    predicate: &Term,
    object: &Term,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
    registry: &PseudoTableRegistry,
    row_limit: Option<usize>,
) -> Result<Option<(Vec<Bindings>, Vec<usize>)>> {
    // Only attempt pseudo-table scan when the predicate is a constant.
    // Variable predicates can't be mapped to a specific column.
    let pred_id = match predicate {
        Term::Variable(_) => return Ok(None),
        _ => resolve_term(predicate, &HashMap::new(), ctx.dict, ctx.prefixes)?,
    };
    let pred_id = match pred_id {
        Some(id) => id,
        None => return Ok(None),
    };

    // Determine the property to search for. The subject position means
    // the node is the subject and we're looking for the object value.
    // (Most common pattern: ?s :predicate ?o)
    let property = Property {
        predicate: pred_id,
        position: PropertyPosition::Subject,
    };

    // Find pseudo-tables with this property as a column.
    let table_matches = registry.find_tables_for_property(&property);
    if table_matches.is_empty() {
        // No pseudo-table has this column — fall through to triple store.
        return Ok(None);
    }

    let mut results = Vec::new();
    let mut source_indices = Vec::new();

    // Use the first matching pseudo-table (largest coverage wins in discovery order).
    let (table_idx, col_idx) = table_matches[0];
    let table = &registry.tables[table_idx];

    for (row_idx, row) in current.iter().enumerate() {
        let s_id = resolve_term(subject, row, ctx.dict, ctx.prefixes)?;
        let o_id = resolve_term(object, row, ctx.dict, ctx.prefixes)?;

        // Scan each segment of the pseudo-table.
        for segment in &table.segments {
            // Determine which rows match based on bound variables.
            let scan_result = if let Some(obj_value) = o_id {
                // Object is bound: equality scan on the column.
                sutra_core::scan_column_eq(segment, col_idx, obj_value)
            } else {
                // Object is unbound: scan for non-null values.
                sutra_core::scan_column_not_null(segment, col_idx)
            };

            for &seg_row_idx in &scan_result.matching_rows {
                let node_id = segment.nodes[seg_row_idx];
                let col_value = segment.columns[col_idx][seg_row_idx];

                // Filter by subject if bound.
                if let Some(bound_s) = s_id {
                    if node_id != bound_s {
                        continue;
                    }
                }

                let mut new_row = row.clone();

                // Bind subject variable.
                if let Term::Variable(name) = subject {
                    if let Some(&existing) = new_row.get(name) {
                        if existing != node_id {
                            continue;
                        }
                    } else {
                        new_row.insert(name.clone(), node_id);
                    }
                }

                // Bind object variable.
                if let Term::Variable(name) = object {
                    if let Some(value) = col_value {
                        if let Some(&existing) = new_row.get(name) {
                            if existing != value {
                                continue;
                            }
                        } else {
                            new_row.insert(name.clone(), value);
                        }
                    } else {
                        continue; // null column value — skip
                    }
                }

                results.push(new_row);
                source_indices.push(row_idx);

                if let Some(limit) = row_limit {
                    if results.len() >= limit {
                        return Ok(Some((results, source_indices)));
                    }
                }
            }
        }
    }

    Ok(Some((results, source_indices)))
}

/// Expand scores from current rows into new results.
/// Each new result row inherits the scores from its source row.
fn expand_scores(
    current: &[Bindings],
    current_scores: &[HashMap<String, f32>],
    new_results: &[Bindings],
) -> Vec<HashMap<String, f32>> {
    // For triple pattern expansion, each new row comes from matching against
    // a current row. We need to figure out which current row produced each new row.
    // Since evaluate_triple_pattern doesn't track this, we match by subset.
    new_results
        .iter()
        .map(|new_row| {
            // Find the source row whose bindings are a subset of new_row
            for (i, old_row) in current.iter().enumerate() {
                if old_row.iter().all(|(k, v)| new_row.get(k) == Some(v)) {
                    return current_scores[i].clone();
                }
            }
            HashMap::new()
        })
        .collect()
}

/// Resolve a Term to a TermId if it's bound (either a concrete term or
/// a variable that's already in the bindings).
fn resolve_term(
    term: &Term,
    bindings: &Bindings,
    dict: &TermDictionary,
    prefixes: &HashMap<String, String>,
) -> Result<Option<TermId>> {
    match term {
        Term::Variable(name) => Ok(bindings.get(name).copied()),
        Term::Iri(iri) => Ok(dict.lookup(iri)),
        Term::PrefixedName { prefix, local } => {
            let base = prefixes
                .get(prefix)
                .ok_or_else(|| SparqlError::UnknownPrefix(prefix.clone()))?;
            let full_iri = format!("{}{}", base, local);
            Ok(dict.lookup(&full_iri))
        }
        Term::A => Ok(dict.lookup("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")),
        Term::Literal(s) => Ok(dict.lookup(&format!("\"{}\"", s))),
        Term::IntegerLiteral(n) => Ok(sutra_core::inline_integer(*n)),
        Term::TypedLiteral { value, datatype } => {
            Ok(dict.lookup(&format!("\"{}\"^^<{}>", value, datatype)))
        }
        Term::VectorLiteral(components) => {
            let vec_str: Vec<String> = components.iter().map(|f| f.to_string()).collect();
            let literal = format!("\"{}\"^^<http://sutra.dev/f32vec>", vec_str.join(" "));
            Ok(dict.lookup(&literal))
        }
        Term::Path { base, .. } => {
            // Path terms are handled at the pattern level, not here
            resolve_term(base, bindings, dict, prefixes)
        }
        Term::QuotedTriple {
            subject,
            predicate,
            object,
        } => {
            // Resolve the inner terms and compute content-addressed ID
            let s = resolve_term(subject, bindings, dict, prefixes)?;
            let p = resolve_term(predicate, bindings, dict, prefixes)?;
            let o = resolve_term(object, bindings, dict, prefixes)?;
            match (s, p, o) {
                (Some(s_id), Some(p_id), Some(o_id)) => {
                    Ok(Some(sutra_core::quoted_triple_id(s_id, p_id, o_id)))
                }
                _ => Ok(None),
            }
        }
    }
}

/// Try to evaluate a triple pattern as a virtual HNSW edge query.
///
/// Returns `Some((results, source_indices))` if the predicate matches any
/// of the HNSW edge predicates, otherwise returns `None` to fall through
/// to normal triple pattern evaluation.
///
/// Supported predicates (checked in order of specificity):
///
/// - `sutra:hnswHorizontalNeighbor` — only horizontal edges (same-layer neighbors)
/// - `sutra:hnswLayerDescend` — only vertical descent edges (layer L → L-1)
/// - `sutra:hnswNeighbor` — ALL edges (backward compatible, matches both types)
///
/// The typed predicates enable SPARQL property paths to express HNSW search:
/// ```sparql
/// ?entry sutra:hnswLayerDescend* / sutra:hnswHorizontalNeighbor+ ?result
/// ```
fn try_evaluate_hnsw_edge_pattern(
    subject: &Term,
    predicate: &Term,
    object: &Term,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<Option<(Vec<Bindings>, Vec<usize>)>> {
    // Resolve the predicate IRI to determine which edge type filter to apply.
    // We check against all three HNSW predicates:
    // - hnswNeighbor (generic, matches all edges)
    // - hnswHorizontalNeighbor (horizontal only)
    // - hnswLayerDescend (vertical only)
    let resolved_iri = resolve_predicate_iri(predicate, current, ctx);
    let edge_type_filter: Option<Option<sutra_hnsw::HnswEdgeType>> = match resolved_iri.as_deref() {
        // Generic predicate: match ALL edge types (no filter)
        Some(iri) if iri == sutra_hnsw::HNSW_NEIGHBOR_IRI => Some(None),
        // Typed predicates: match only the specific edge type
        Some(iri) if iri == sutra_hnsw::HNSW_HORIZONTAL_NEIGHBOR_IRI => {
            Some(Some(sutra_hnsw::HnswEdgeType::Horizontal))
        }
        Some(iri) if iri == sutra_hnsw::HNSW_LAYER_DESCEND_IRI => {
            Some(Some(sutra_hnsw::HnswEdgeType::VerticalDescend))
        }
        // Not an HNSW predicate — fall through to normal evaluation
        _ => None,
    };

    // If the predicate doesn't match any HNSW edge type, this isn't an
    // HNSW edge pattern — return None to let normal triple evaluation handle it.
    let edge_type_filter = match edge_type_filter {
        Some(f) => f,
        None => return Ok(None),
    };

    // If the HNSW neighbor IRI isn't in the dictionary yet, it can't match anything
    let neighbor_pred_id = ctx.dict.lookup(sutra_hnsw::HNSW_NEIGHBOR_IRI).unwrap_or(0);

    let mut results = Vec::new();
    let mut source_indices = Vec::new();

    'outer: for (row_idx, row) in current.iter().enumerate() {
        let s_id = resolve_term(subject, row, ctx.dict, ctx.prefixes)?;
        let o_id = resolve_term(object, row, ctx.dict, ctx.prefixes)?;

        // Generate edge triples based on what's bound
        let edges: Vec<(sutra_core::TermId, sutra_hnsw::HnswEdgeTriple)> = match (s_id, o_id) {
            (Some(source_id), _) => {
                // Source bound: get edges from this source
                ctx.vectors.edge_triples_for_source(source_id)
            }
            (None, Some(target_id)) => {
                // Object bound: get edges to this target
                ctx.vectors.edge_triples_for_target(target_id)
            }
            (None, None) => {
                // Both unbound: get all edges (expensive!)
                ctx.vectors.all_edge_triples()
            }
        };

        for (_pred_id, edge) in &edges {
            // Apply edge type filter: if a specific typed predicate was used
            // (e.g., hnswHorizontalNeighbor), only include edges of that type.
            // If the generic hnswNeighbor was used, include all edges.
            if let Some(required_type) = edge_type_filter {
                if edge.edge_type != required_type {
                    continue;
                }
            }

            // Resolve vector object IDs back to entity IRIs.
            // HNSW nodes are keyed by vector object TermIds, but we want to
            // expose entity IRIs in the virtual triples. Find which entities
            // have these vectors by scanning all vector predicates.
            let source_entities = resolve_vector_to_entities(edge.source, ctx);
            let target_entities = resolve_vector_to_entities(edge.target, ctx);

            for &source_entity in &source_entities {
                // Filter by subject if bound
                if let Some(bound_source) = s_id {
                    if source_entity != bound_source {
                        continue;
                    }
                }

                for &target_entity in &target_entities {
                    // Filter by object if bound
                    if let Some(bound_target) = o_id {
                        if target_entity != bound_target {
                            continue;
                        }
                    }

                    let mut new_row = row.clone();

                    // Bind subject variable
                    if let Term::Variable(name) = subject {
                        if let Some(&existing) = new_row.get(name) {
                            if existing != source_entity {
                                continue;
                            }
                        } else {
                            new_row.insert(name.clone(), source_entity);
                        }
                    }

                    // Bind predicate variable
                    if let Term::Variable(name) = predicate {
                        if neighbor_pred_id != 0 {
                            if let Some(&existing) = new_row.get(name) {
                                if existing != neighbor_pred_id {
                                    continue;
                                }
                            } else {
                                new_row.insert(name.clone(), neighbor_pred_id);
                            }
                        }
                    }

                    // Bind object variable
                    if let Term::Variable(name) = object {
                        if let Some(&existing) = new_row.get(name) {
                            if existing != target_entity {
                                continue;
                            }
                        } else {
                            new_row.insert(name.clone(), target_entity);
                        }
                    }

                    results.push(new_row);
                    source_indices.push(row_idx);

                    if let Some(limit) = row_limit {
                        if results.len() >= limit {
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    Ok(Some((results, source_indices)))
}

/// Resolve a predicate term to its full IRI string.
///
/// Handles all predicate term forms: full IRIs, prefixed names, and bound variables.
/// Returns None if the predicate can't be resolved (e.g., unbound variable, unknown prefix).
///
/// This is used by `try_evaluate_hnsw_edge_pattern` to determine which HNSW edge
/// predicate is being queried, enabling typed edge filtering (horizontal vs vertical).
fn resolve_predicate_iri(
    predicate: &Term,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
) -> Option<String> {
    match predicate {
        Term::Iri(iri) => Some(iri.clone()),
        Term::PrefixedName { prefix, local } => ctx
            .prefixes
            .get(prefix)
            .map(|base| format!("{}{}", base, local)),
        Term::Variable(name) => {
            // Check if the variable is bound to an IRI in the first row.
            // This is a heuristic — if the variable is bound to different IRIs
            // in different rows, we use the first row's binding. This is correct
            // because HNSW edge patterns are typically used with a single
            // predicate binding.
            current
                .first()
                .and_then(|row| row.get(name))
                .and_then(|&id| ctx.dict.resolve(id))
                .map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Resolve a vector object TermId back to the entity IRIs that point to it.
///
/// Scans all vector predicates in the registry, then uses the triple store's
/// POS index to find triples where the object is this vector literal.
/// Returns the subject IRIs of those triples.
fn resolve_vector_to_entities(vector_object_id: TermId, ctx: &ExecutionContext<'_>) -> Vec<TermId> {
    let mut entities = Vec::new();
    for pred_id in ctx.vectors.predicates() {
        let triples = ctx
            .store
            .find_by_predicate_object(pred_id, vector_object_id);
        for triple in triples {
            entities.push(triple.subject);
        }
    }
    // Fallback: if no entity found, return the raw ID (backward compat)
    if entities.is_empty() {
        entities.push(vector_object_id);
    }
    entities
}

fn evaluate_filter(expr: &FilterExpr, row: &Bindings, ctx: &mut ExecutionContext<'_>) -> bool {
    match expr {
        FilterExpr::Equals(left, right) => {
            let l = filter_term_value(left, row);
            let r = filter_term_value(right, row);
            l.is_some() && l == r
        }
        FilterExpr::NotEquals(left, right) => {
            let l = filter_term_value(left, row);
            let r = filter_term_value(right, row);
            l.is_some() && r.is_some() && l != r
        }
        FilterExpr::LessThan(left, right) => {
            let l = filter_term_value(left, row);
            let r = filter_term_value(right, row);
            match (l, r) {
                (Some(a), Some(b)) => a < b,
                _ => false,
            }
        }
        FilterExpr::GreaterThan(left, right) => {
            let l = filter_term_value(left, row);
            let r = filter_term_value(right, row);
            match (l, r) {
                (Some(a), Some(b)) => a > b,
                _ => false,
            }
        }
        FilterExpr::Bound(var) => row.contains_key(var),
        FilterExpr::NotBound(var) => !row.contains_key(var),
        FilterExpr::NotExists(patterns) => {
            // NOT EXISTS: evaluate sub-patterns starting from this row.
            // If any results come back, the filter fails (row is excluded).
            let start = vec![row.clone()];
            let start_scores = vec![HashMap::new()];
            let mut results = start;
            let mut scores = start_scores;
            for p in patterns {
                match evaluate_pattern(p, &results, &scores, ctx, Some(1)) {
                    Ok((new_results, new_scores)) => {
                        results = new_results;
                        scores = new_scores;
                    }
                    Err(_) => return true, // on error, treat as not existing
                }
                if results.is_empty() {
                    return true; // no matches = NOT EXISTS is true
                }
            }
            results.is_empty() // true if no matches found
        }
        FilterExpr::Exists(patterns) => {
            let start = vec![row.clone()];
            let start_scores = vec![HashMap::new()];
            let mut results = start;
            let mut scores = start_scores;
            for p in patterns {
                match evaluate_pattern(p, &results, &scores, ctx, Some(1)) {
                    Ok((new_results, new_scores)) => {
                        results = new_results;
                        scores = new_scores;
                    }
                    Err(_) => return false,
                }
                if results.is_empty() {
                    return false;
                }
            }
            !results.is_empty()
        }
        FilterExpr::And(left, right) => {
            evaluate_filter(left, row, ctx) && evaluate_filter(right, row, ctx)
        }
        FilterExpr::Or(left, right) => {
            evaluate_filter(left, row, ctx) || evaluate_filter(right, row, ctx)
        }
        FilterExpr::Not(inner) => !evaluate_filter(inner, row, ctx),
        FilterExpr::GreaterThanOrEqual(left, right) => {
            let l = filter_term_value(left, row);
            let r = filter_term_value(right, row);
            match (l, r) {
                (Some(a), Some(b)) => a >= b,
                _ => false,
            }
        }
        FilterExpr::LessThanOrEqual(left, right) => {
            let l = filter_term_value(left, row);
            let r = filter_term_value(right, row);
            match (l, r) {
                (Some(a), Some(b)) => a <= b,
                _ => false,
            }
        }
        FilterExpr::Contains(haystack, needle) => {
            string_filter_op(haystack, needle, row, ctx, |h, n| h.contains(n))
        }
        FilterExpr::StrStarts(haystack, prefix) => {
            string_filter_op(haystack, prefix, row, ctx, |h, p| h.starts_with(p))
        }
        FilterExpr::StrEnds(haystack, suffix) => {
            string_filter_op(haystack, suffix, row, ctx, |h, s| h.ends_with(s))
        }
        FilterExpr::Regex(term, pattern) => {
            // Simple substring match (full regex would need a regex crate)
            string_filter_op(term, pattern, row, ctx, |h, p| h.contains(p))
        }
        FilterExpr::LangEquals(var, lang) => {
            if let Some(&id) = row.get(var) {
                if let Some(term_str) = ctx.dict.resolve(id) {
                    // Language-tagged literals look like: "value"@lang
                    if let Some(at_pos) = term_str.rfind('@') {
                        return &term_str[at_pos + 1..] == lang;
                    }
                }
            }
            false
        }
        FilterExpr::IsIri(var) => {
            if let Some(&id) = row.get(var) {
                if sutra_core::is_inline(id) {
                    return false;
                }
                if let Some(term_str) = ctx.dict.resolve(id) {
                    return !term_str.starts_with('"') && !term_str.starts_with("_:");
                }
            }
            false
        }
        FilterExpr::LangMatches(var, lang) => {
            if let Some(&id) = row.get(var) {
                if let Some(term_str) = ctx.dict.resolve(id) {
                    if let Some(at_pos) = term_str.rfind('@') {
                        let term_lang = &term_str[at_pos + 1..];
                        if lang == "*" {
                            return !term_lang.is_empty();
                        }
                        return term_lang.eq_ignore_ascii_case(lang);
                    }
                }
            }
            false
        }
        FilterExpr::StrEquals(var, term) => {
            let var_str = row.get(var).and_then(|&id| {
                if let Some(s) = ctx.dict.resolve(id) {
                    // Strip quotes and language tag
                    if let Some(inner) = s.strip_prefix('"') {
                        let end = inner.find('"').unwrap_or(inner.len());
                        Some(inner[..end].to_string())
                    } else {
                        Some(s.to_string())
                    }
                } else {
                    None
                }
            });
            let term_str = term_to_string(term, row, ctx);
            var_str.is_some() && var_str == term_str
        }
        FilterExpr::DatatypeEquals(var, expected_dt) => {
            if let Some(&id) = row.get(var) {
                if sutra_core::decode_inline_integer(id).is_some() {
                    return expected_dt.contains("integer");
                }
                if sutra_core::decode_inline_boolean(id).is_some() {
                    return expected_dt.contains("boolean");
                }
                if let Some(term_str) = ctx.dict.resolve(id) {
                    // Check for ^^<datatype>
                    if let Some(dt_start) = term_str.find("^^<") {
                        let dt = &term_str[dt_start + 3..term_str.len() - 1];
                        return dt == expected_dt;
                    }
                }
            }
            false
        }
        FilterExpr::IsLiteral(var) => {
            if let Some(&id) = row.get(var) {
                if sutra_core::is_inline(id) {
                    return true; // inline integers/booleans are literals
                }
                if let Some(term_str) = ctx.dict.resolve(id) {
                    return term_str.starts_with('"');
                }
            }
            false
        }
    }
}

/// Helper for string-based filter operations.
fn string_filter_op(
    left: &Term,
    right: &Term,
    row: &Bindings,
    ctx: &ExecutionContext<'_>,
    op: impl FnOnce(&str, &str) -> bool,
) -> bool {
    let left_str = term_to_string(left, row, ctx);
    let right_str = term_to_string(right, row, ctx);
    match (left_str, right_str) {
        (Some(l), Some(r)) => op(&l, &r),
        _ => false,
    }
}

/// Resolve a term to its string value for string operations.
fn term_to_string(term: &Term, row: &Bindings, ctx: &ExecutionContext<'_>) -> Option<String> {
    match term {
        Term::Variable(name) => {
            let &id = row.get(name)?;
            if let Some(n) = sutra_core::decode_inline_integer(id) {
                return Some(n.to_string());
            }
            let resolved = ctx.dict.resolve(id)?;
            // Strip quotes from literals
            if let Some(inner) = resolved.strip_prefix('"') {
                let end = inner.find('"').unwrap_or(inner.len());
                Some(inner[..end].to_string())
            } else {
                Some(resolved.to_string())
            }
        }
        Term::Literal(s) => Some(s.clone()),
        Term::Iri(s) => Some(s.clone()),
        _ => None,
    }
}

/// Apply GROUP BY and aggregate functions.
fn apply_group_by_and_aggregates(
    results: &[Bindings],
    group_by: &[String],
    aggregates: &[Aggregate],
    ctx: &ExecutionContext<'_>,
) -> Result<(Vec<Bindings>, Vec<HashMap<String, f32>>)> {
    // Group rows by the GROUP BY variables
    let mut groups: HashMap<Vec<Option<TermId>>, Vec<&Bindings>> = HashMap::new();

    for row in results {
        let key: Vec<Option<TermId>> = group_by.iter().map(|v| row.get(v).copied()).collect();
        groups.entry(key).or_default().push(row);
    }

    // If no GROUP BY but there are aggregates, treat all rows as one group
    if group_by.is_empty() && !aggregates.is_empty() {
        let all_rows: Vec<&Bindings> = results.iter().collect();
        let mut result_row = HashMap::new();

        for agg in aggregates {
            let value = compute_aggregate(agg, &all_rows, ctx);
            if let Some(id) = sutra_core::inline_integer(value) {
                result_row.insert(agg.alias.clone(), id);
            }
        }

        return Ok((vec![result_row], vec![HashMap::new()]));
    }

    let mut output_rows = Vec::new();
    let mut output_scores = Vec::new();

    for (key, group_rows) in &groups {
        let mut row = HashMap::new();

        // Set GROUP BY variable values from the key
        for (i, var) in group_by.iter().enumerate() {
            if let Some(id) = key[i] {
                row.insert(var.clone(), id);
            }
        }

        // Compute aggregates
        for agg in aggregates {
            let value = compute_aggregate(agg, group_rows, ctx);
            if let Some(id) = sutra_core::inline_integer(value) {
                row.insert(agg.alias.clone(), id);
            }
        }

        output_rows.push(row);
        output_scores.push(HashMap::new());
    }

    Ok((output_rows, output_scores))
}

fn compute_aggregate(agg: &Aggregate, rows: &[&Bindings], _ctx: &ExecutionContext<'_>) -> i64 {
    match agg.function {
        AggregateFunction::Count => {
            if agg.distinct {
                let mut seen = std::collections::HashSet::new();
                for row in rows {
                    let val = match &agg.argument {
                        AggregateArg::Star => Some(format!("{:?}", row)),
                        AggregateArg::Variable(v) => row.get(v).map(|id| id.to_string()),
                    };
                    if let Some(v) = val {
                        seen.insert(v);
                    }
                }
                seen.len() as i64
            } else {
                match &agg.argument {
                    AggregateArg::Star => rows.len() as i64,
                    AggregateArg::Variable(v) => {
                        rows.iter().filter(|r| r.contains_key(v)).count() as i64
                    }
                }
            }
        }
        AggregateFunction::Sum | AggregateFunction::Avg => {
            let values: Vec<i64> = match &agg.argument {
                AggregateArg::Variable(v) => rows
                    .iter()
                    .filter_map(|r| r.get(v))
                    .filter_map(|&id| sutra_core::decode_inline_integer(id))
                    .collect(),
                AggregateArg::Star => vec![],
            };
            if values.is_empty() {
                return 0;
            }
            let sum: i64 = values.iter().sum();
            if agg.function == AggregateFunction::Avg {
                sum / values.len() as i64
            } else {
                sum
            }
        }
        AggregateFunction::Min => match &agg.argument {
            AggregateArg::Variable(v) => rows
                .iter()
                .filter_map(|r| r.get(v))
                .filter_map(|&id| sutra_core::decode_inline_integer(id))
                .min()
                .unwrap_or(0),
            AggregateArg::Star => 0,
        },
        AggregateFunction::Max => match &agg.argument {
            AggregateArg::Variable(v) => rows
                .iter()
                .filter_map(|r| r.get(v))
                .filter_map(|&id| sutra_core::decode_inline_integer(id))
                .max()
                .unwrap_or(0),
            AggregateArg::Star => 0,
        },
    }
}

/// Evaluate a property path pattern (pred+, pred*, pred/pred2).
fn evaluate_property_path(
    subject: &Term,
    base_pred: &Term,
    modifier: &PathModifier,
    object: &Term,
    current: &[Bindings],
    current_scores: &[HashMap<String, f32>],
    ctx: &mut ExecutionContext<'_>,
) -> Result<(Vec<Bindings>, Vec<HashMap<String, f32>>)> {
    match modifier {
        PathModifier::Sequence(next_pred) => {
            // pred1/pred2: ?s pred1 ?mid . ?mid pred2 ?o
            let mid_var = format!("__path_mid_{}", current.len());
            let step1 = Pattern::Triple {
                subject: subject.clone(),
                predicate: base_pred.clone(),
                object: Term::Variable(mid_var.clone()),
            };
            let step2 = Pattern::Triple {
                subject: Term::Variable(mid_var),
                predicate: (*next_pred).as_ref().clone(),
                object: object.clone(),
            };
            let (r1, s1) = evaluate_pattern(&step1, current, current_scores, ctx, None)?;
            evaluate_pattern(&step2, &r1, &s1, ctx, None)
        }
        PathModifier::OneOrMore | PathModifier::ZeroOrMore => {
            // Iterative BFS traversal with cycle detection
            let include_zero = matches!(modifier, PathModifier::ZeroOrMore);
            let max_depth = 50; // prevent infinite loops

            let mut all_results = Vec::new();
            let mut all_scores = Vec::new();

            for (row_idx, row) in current.iter().enumerate() {
                let s_id = resolve_term(subject, row, ctx.dict, ctx.prefixes)?;
                let o_id = resolve_term(object, row, ctx.dict, ctx.prefixes)?;
                let pred_id = resolve_term(base_pred, row, ctx.dict, ctx.prefixes)?
                    .ok_or_else(|| SparqlError::Execution("path predicate not resolved".into()))?;

                // Zero-length path: subject = object
                if include_zero {
                    if let Some(sid) = s_id {
                        if let Term::Variable(o_var) = object {
                            let mut new_row = row.clone();
                            new_row.insert(o_var.clone(), sid);
                            all_results.push(new_row);
                            all_scores.push(current_scores[row_idx].clone());
                        } else if o_id == Some(sid) {
                            all_results.push(row.clone());
                            all_scores.push(current_scores[row_idx].clone());
                        }
                    }
                }

                // BFS from subject
                let start_nodes: Vec<TermId> = if let Some(sid) = s_id {
                    vec![sid]
                } else {
                    continue;
                };

                let mut frontier = start_nodes;
                let mut visited = std::collections::HashSet::new();

                for _depth in 0..max_depth {
                    if frontier.is_empty() {
                        break;
                    }
                    check_deadline(ctx)?;

                    let mut next_frontier = Vec::new();
                    for &node in &frontier {
                        if !visited.insert(node) {
                            continue;
                        }
                        // Find all objects reachable via one step of pred_id from node
                        for triple in ctx.store.find_by_subject_predicate(node, pred_id) {
                            let target = triple.object;
                            // Add to results
                            if let Term::Variable(o_var) = object {
                                let mut new_row = row.clone();
                                new_row.insert(o_var.clone(), target);
                                all_results.push(new_row);
                                all_scores.push(current_scores[row_idx].clone());
                            } else if o_id == Some(target) {
                                all_results.push(row.clone());
                                all_scores.push(current_scores[row_idx].clone());
                            }
                            if !visited.contains(&target) {
                                next_frontier.push(target);
                            }
                        }
                    }
                    frontier = next_frontier;
                }
            }

            Ok((all_results, all_scores))
        }
        PathModifier::ZeroOrOne => {
            // pred?: match zero or one step
            let mut results = Vec::new();
            let mut scores = Vec::new();

            // Zero step: subject = object
            for (i, row) in current.iter().enumerate() {
                if let Some(sid) = resolve_term(subject, row, ctx.dict, ctx.prefixes)? {
                    if let Term::Variable(o_var) = object {
                        let mut new_row = row.clone();
                        new_row.insert(o_var.clone(), sid);
                        results.push(new_row);
                        scores.push(current_scores[i].clone());
                    }
                }
            }

            // One step: normal triple pattern
            let one_step = Pattern::Triple {
                subject: subject.clone(),
                predicate: base_pred.clone(),
                object: object.clone(),
            };
            let (r1, s1) = evaluate_pattern(&one_step, current, current_scores, ctx, None)?;
            results.extend(r1);
            scores.extend(s1);

            Ok((results, scores))
        }
    }
}

/// Check if the query has timed out.
fn check_deadline(ctx: &ExecutionContext<'_>) -> Result<()> {
    if let Some(deadline) = ctx.deadline {
        if Instant::now() > deadline {
            return Err(SparqlError::Timeout);
        }
    }
    Ok(())
}

fn is_concrete(term: &Term) -> bool {
    !matches!(term, Term::Variable(_))
}

fn filter_term_value(term: &Term, row: &Bindings) -> Option<TermId> {
    match term {
        Term::Variable(name) => row.get(name).copied(),
        Term::IntegerLiteral(n) => sutra_core::inline_integer(*n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn setup() -> (TripleStore, TermDictionary) {
        let mut dict = TermDictionary::new();
        let mut store = TripleStore::new();

        let alice = dict.intern("http://example.org/Alice");
        let bob = dict.intern("http://example.org/Bob");
        let charlie = dict.intern("http://example.org/Charlie");
        let knows = dict.intern("http://example.org/knows");
        let name = dict.intern("http://example.org/name");
        let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let person = dict.intern("http://example.org/Person");
        let alice_name = dict.intern("\"Alice\"");
        let bob_name = dict.intern("\"Bob\"");
        let age = dict.intern("http://example.org/age");

        store.insert(Triple::new(alice, rdf_type, person)).unwrap();
        store.insert(Triple::new(bob, rdf_type, person)).unwrap();
        store.insert(Triple::new(alice, knows, bob)).unwrap();
        store.insert(Triple::new(alice, knows, charlie)).unwrap();
        store.insert(Triple::new(bob, knows, alice)).unwrap();
        store.insert(Triple::new(alice, name, alice_name)).unwrap();
        store.insert(Triple::new(bob, name, bob_name)).unwrap();

        let age_30 = sutra_core::inline_integer(30).unwrap();
        let age_25 = sutra_core::inline_integer(25).unwrap();
        store.insert(Triple::new(alice, age, age_30)).unwrap();
        store.insert(Triple::new(bob, age, age_25)).unwrap();

        (store, dict)
    }

    #[test]
    fn select_all_triples() {
        let (store, dict) = setup();
        let q = parser::parse("SELECT * WHERE { ?s ?p ?o }").unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 9);
    }

    #[test]
    fn select_by_predicate() {
        let (store, dict) = setup();
        let q = parser::parse("SELECT ?s ?o WHERE { ?s <http://example.org/knows> ?o }").unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 3);
    }

    #[test]
    fn select_with_bound_subject() {
        let (store, dict) = setup();
        let q = parser::parse(
            "SELECT ?o WHERE { <http://example.org/Alice> <http://example.org/knows> ?o }",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn select_with_a_shorthand() {
        let (store, dict) = setup();
        let q = parser::parse(
            "PREFIX ex: <http://example.org/> \
             SELECT ?person WHERE { ?person a ex:Person }",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn select_with_join() {
        let (store, dict) = setup();
        let q = parser::parse(
            "SELECT ?name WHERE { \
             <http://example.org/Alice> <http://example.org/knows> ?person . \
             ?person <http://example.org/name> ?name \
             }",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn select_with_limit() {
        let (store, dict) = setup();
        let q = parser::parse("SELECT * WHERE { ?s ?p ?o } LIMIT 3").unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 3);
    }

    #[test]
    fn select_with_offset() {
        let (store, dict) = setup();
        let q = parser::parse("SELECT * WHERE { ?s ?p ?o } LIMIT 3 OFFSET 2").unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 3);
    }

    #[test]
    fn select_with_filter() {
        let (store, dict) = setup();
        let q = parser::parse(
            "SELECT ?person WHERE { \
             ?person <http://example.org/age> ?age . \
             FILTER(?age > 26) \
             }",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn empty_result() {
        let (store, dict) = setup();
        let q =
            parser::parse("SELECT ?s WHERE { ?s <http://example.org/nonexistent> ?o }").unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 0);
    }

    #[test]
    fn vector_similar_unbound_subject() {
        use sutra_hnsw::{VectorPredicateConfig, VectorRegistry};

        let mut dict = TermDictionary::new();
        let mut store = TripleStore::new();
        let has_embedding = dict.intern("http://example.org/hasEmbedding");

        let mut vectors = VectorRegistry::new();
        vectors
            .declare(VectorPredicateConfig {
                predicate_id: has_embedding,
                dimensions: 3,
                m: 4,
                ef_construction: 20,
                metric: sutra_hnsw::DistanceMetric::Cosine,
            })
            .unwrap();

        // Insert vectors as proper triples: <doc> <hasEmbedding> <vector_literal>
        // Then insert the vector into HNSW keyed by the object (vector literal) ID
        let doc1 = dict.intern("http://example.org/doc1");
        let doc2 = dict.intern("http://example.org/doc2");
        let doc3 = dict.intern("http://example.org/doc3");
        let vec1_id = dict.intern("\"vec_doc1\"^^<http://sutra.dev/f32vec>");
        let vec2_id = dict.intern("\"vec_doc2\"^^<http://sutra.dev/f32vec>");
        let vec3_id = dict.intern("\"vec_doc3\"^^<http://sutra.dev/f32vec>");

        // Create triples linking docs to their vector objects
        store
            .insert(Triple::new(doc1, has_embedding, vec1_id))
            .unwrap();
        store
            .insert(Triple::new(doc2, has_embedding, vec2_id))
            .unwrap();
        store
            .insert(Triple::new(doc3, has_embedding, vec3_id))
            .unwrap();

        // Insert vectors into HNSW keyed by object ID
        vectors
            .insert(has_embedding, vec![1.0, 0.0, 0.0], vec1_id)
            .unwrap();
        vectors
            .insert(has_embedding, vec![0.9, 0.1, 0.0], vec2_id)
            .unwrap();
        vectors
            .insert(has_embedding, vec![0.0, 0.0, 1.0], vec3_id)
            .unwrap();

        let q = parser::parse(
            "SELECT ?doc WHERE { \
             VECTOR_SIMILAR(?doc <http://example.org/hasEmbedding> \"1.0 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.8) \
             }",
        )
        .unwrap();

        let result = execute_with_vectors(&q, &store, &dict, &mut vectors).unwrap();

        // doc1 (cosine ~1.0) and doc2 (cosine ~0.99) should match; doc3 (cosine ~0.0) should not
        assert!(result.rows.len() >= 2);
        let doc_ids: Vec<TermId> = result.rows.iter().map(|r| *r.get("doc").unwrap()).collect();
        assert!(doc_ids.contains(&doc1));
        assert!(doc_ids.contains(&doc2));
        assert!(!doc_ids.contains(&doc3));
    }

    #[test]
    fn vector_similar_with_graph_join() {
        use sutra_hnsw::{VectorPredicateConfig, VectorRegistry};

        let mut dict = TermDictionary::new();
        let mut store = TripleStore::new();
        let has_embedding = dict.intern("http://example.org/hasEmbedding");
        let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let paper = dict.intern("http://example.org/Paper");
        let title = dict.intern("http://example.org/title");

        let doc1 = dict.intern("http://example.org/doc1");
        let doc2 = dict.intern("http://example.org/doc2");
        let doc3 = dict.intern("http://example.org/doc3");
        let title1 = dict.intern("\"Attention Is All You Need\"");
        let title2 = dict.intern("\"BERT\"");
        let title3 = dict.intern("\"Cooking Recipes\"");

        // doc1 and doc2 are Papers, doc3 is not
        store.insert(Triple::new(doc1, rdf_type, paper)).unwrap();
        store.insert(Triple::new(doc2, rdf_type, paper)).unwrap();
        store.insert(Triple::new(doc1, title, title1)).unwrap();
        store.insert(Triple::new(doc2, title, title2)).unwrap();
        store.insert(Triple::new(doc3, title, title3)).unwrap();

        let mut vectors = VectorRegistry::new();
        vectors
            .declare(VectorPredicateConfig {
                predicate_id: has_embedding,
                dimensions: 3,
                m: 4,
                ef_construction: 20,
                metric: sutra_hnsw::DistanceMetric::Cosine,
            })
            .unwrap();

        // Create vector object IDs and triples linking docs to vectors
        let vec1_id = dict.intern("\"vec_d1\"^^<http://sutra.dev/f32vec>");
        let vec2_id = dict.intern("\"vec_d2\"^^<http://sutra.dev/f32vec>");
        let vec3_id = dict.intern("\"vec_d3\"^^<http://sutra.dev/f32vec>");
        store
            .insert(Triple::new(doc1, has_embedding, vec1_id))
            .unwrap();
        store
            .insert(Triple::new(doc2, has_embedding, vec2_id))
            .unwrap();
        store
            .insert(Triple::new(doc3, has_embedding, vec3_id))
            .unwrap();

        vectors
            .insert(has_embedding, vec![1.0, 0.0, 0.0], vec1_id)
            .unwrap();
        vectors
            .insert(has_embedding, vec![0.9, 0.1, 0.0], vec2_id)
            .unwrap();
        vectors
            .insert(has_embedding, vec![0.8, 0.2, 0.0], vec3_id)
            .unwrap();

        // Query: find Papers similar to query vector
        // The planner should put VECTOR_SIMILAR first (unbound subject → weight 1)
        // then filter by rdf:type Paper
        let q = parser::parse(
            "PREFIX ex: <http://example.org/> \
             SELECT ?doc ?title WHERE { \
             ?doc a ex:Paper . \
             ?doc ex:title ?title . \
             VECTOR_SIMILAR(?doc ex:hasEmbedding \"1.0 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.5) \
             }",
        )
        .unwrap();

        let result = execute_with_vectors(&q, &store, &dict, &mut vectors).unwrap();

        // All 3 docs are similar (>0.5), but only doc1 and doc2 are Papers
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn order_by_variable() {
        let (store, dict) = setup();
        let q = parser::parse(
            "SELECT ?person ?age WHERE { \
             ?person <http://example.org/age> ?age \
             } ORDER BY ASC(?age)",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 2);
        // age 25 should come before age 30
        let ages: Vec<TermId> = result.rows.iter().map(|r| *r.get("age").unwrap()).collect();
        assert!(ages[0] < ages[1]);
    }

    #[test]
    fn union_pattern() {
        let (store, dict) = setup();
        let q = parser::parse(
            "SELECT ?s WHERE { \
             { ?s <http://example.org/name> ?n } \
             UNION \
             { ?s <http://example.org/age> ?a } \
             }",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        // 2 name triples + 2 age triples = 4, but Alice and Bob appear in both
        // UNION doesn't deduplicate, so we get 4
        assert_eq!(result.rows.len(), 4);
    }
}
