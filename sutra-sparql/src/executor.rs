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

use sutra_core::{DatabaseConfig, HnswEdgeMode, TermDictionary, TermId, Triple, TripleStore};
use sutra_hnsw::VectorRegistry;

use crate::error::{Result, SparqlError};
use crate::parser::{FilterExpr, Pattern, Query, Term};

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
    let mut ctx = ExecutionContext {
        store,
        dict,
        vectors,
        prefixes: &query.prefixes,
        config,
    };

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
            evaluate_pattern(pattern, &results, &scores, &mut ctx, pushable_limit)?;
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

    // Apply ORDER BY
    if !query.order_by.is_empty() {
        apply_order_by(&mut results, &mut scores, &query.order_by, &mut ctx)?;
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

/// Execute a parsed query without vector support (backward compatible).
pub fn execute(query: &Query, store: &TripleStore, dict: &TermDictionary) -> Result<QueryResult> {
    let mut empty_registry = VectorRegistry::new();
    execute_with_vectors(query, store, dict, &mut empty_registry)
}

fn evaluate_pattern(
    pattern: &Pattern,
    current: &[Bindings],
    current_scores: &[HashMap<String, f32>],
    ctx: &mut ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<(Vec<Bindings>, Vec<HashMap<String, f32>>)> {
    match pattern {
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

fn evaluate_triple_pattern(
    subject: &Term,
    predicate: &Term,
    object: &Term,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<(Vec<Bindings>, Vec<usize>)> {
    // Check if this is a virtual HNSW edge query.
    // When config says Virtual mode, intercept sutra:hnswNeighbor patterns.
    if ctx.config.hnsw_edge_mode == HnswEdgeMode::Virtual {
        if let Some(result) =
            try_evaluate_hnsw_edge_pattern(subject, predicate, object, current, ctx, row_limit)?
        {
            return Ok(result);
        }
    }

    let mut results = Vec::new();
    let mut source_indices = Vec::new();

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
    }
}

/// Try to evaluate a triple pattern as a virtual HNSW edge query.
///
/// Returns `Some((results, source_indices))` if the predicate is `sutra:hnswNeighbor`,
/// otherwise returns `None` to fall through to normal triple pattern evaluation.
///
/// Virtual HNSW edge triples look like:
///   `?source sutra:hnswNeighbor ?target`
///
/// These are generated on-the-fly from the HNSW graph structure without
/// being stored in the triple store.
fn try_evaluate_hnsw_edge_pattern(
    subject: &Term,
    predicate: &Term,
    object: &Term,
    current: &[Bindings],
    ctx: &ExecutionContext<'_>,
    row_limit: Option<usize>,
) -> Result<Option<(Vec<Bindings>, Vec<usize>)>> {
    // Check if the predicate is sutra:hnswNeighbor
    let is_hnsw_neighbor = match predicate {
        Term::Iri(iri) => iri == sutra_hnsw::HNSW_NEIGHBOR_IRI,
        Term::PrefixedName { prefix, local } => {
            if let Some(base) = ctx.prefixes.get(prefix) {
                let full = format!("{}{}", base, local);
                full == sutra_hnsw::HNSW_NEIGHBOR_IRI
            } else {
                false
            }
        }
        Term::Variable(name) => {
            // If the predicate variable is already bound to hnswNeighbor
            if let Some(&bound_id) = current.first().and_then(|row| row.get(name)) {
                if let Some(iri) = ctx.dict.resolve(bound_id) {
                    iri == sutra_hnsw::HNSW_NEIGHBOR_IRI
                } else {
                    false
                }
            } else {
                false
            }
        }
        _ => false,
    };

    if !is_hnsw_neighbor {
        return Ok(None);
    }

    let neighbor_pred_id = ctx
        .dict
        .lookup(sutra_hnsw::HNSW_NEIGHBOR_IRI)
        .unwrap_or_else(|| {
            // If hnswNeighbor isn't in the dictionary yet, it can't match anything
            0
        });

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
    }
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
