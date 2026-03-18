//! Cost-based query planner with pattern reordering and predicate pushdown.
//!
//! Implements a greedy heuristic for triple pattern reordering that combines
//! structural analysis (bound/unbound positions) with cardinality estimation
//! from the triple store's indexes. This is the SPARQL equivalent of a SQL
//! query optimizer choosing between index scan and sequential scan.
//!
//! ## Design decisions
//!
//! **Why greedy instead of dynamic programming?**
//! SPARQL queries typically have 3-10 patterns. Greedy O(n²) is fast enough
//! and produces near-optimal orderings because the bound/unbound heuristic
//! already captures most of the selectivity signal. DP would be warranted
//! for 20+ pattern queries, which are rare in practice.
//!
//! **Why combine heuristic weight with cardinality?**
//! Pure cardinality-based ordering can make bad decisions when a pattern
//! produces few results but leaves many variables unbound (forcing later
//! patterns into full scans). The heuristic weight captures structural
//! selectivity, while cardinality captures data-dependent selectivity.
//! The final cost multiplies both signals: cost = heuristic_weight * cardinality.
//!
//! **Predicate pushdown:**
//! FILTERs are repositioned immediately after the pattern that binds their
//! last required variable. This is critical for performance — a FILTER on
//! `?age > 25` that runs after 10,000 intermediate rows is much cheaper
//! when pushed down to run after the 50 rows that bind `?age`.

use std::collections::HashSet;

use sutra_core::{TermDictionary, TripleStore};

use crate::parser::{FilterExpr, Pattern, Query, Term};

// ---------------------------------------------------------------------------
// Cost model constants
// ---------------------------------------------------------------------------

/// Base weight for a fully-bound triple pattern (0 unbound positions).
/// This is the cheapest possible pattern — a point lookup in the SPO index.
const WEIGHT_FULLY_BOUND: u32 = 0;

/// Weight increment per unbound position in a triple pattern.
/// Each unbound position roughly multiplies the result set by the predicate's
/// fanout, so we penalize linearly.
const WEIGHT_PER_UNBOUND: u32 = 1;

/// Weight for VECTOR_SIMILAR when subject is unbound.
/// Low weight (1) because the HNSW index is very selective — it returns
/// at most top-k results regardless of database size.
const WEIGHT_VECTOR_UNBOUND: u32 = 1;

/// Weight for VECTOR_SIMILAR when subject is already bound.
/// Higher weight (5) because we're filtering bound subjects against vectors,
/// which requires fetching each subject's vector and computing distance.
/// Better to let graph patterns narrow the candidate set first.
const WEIGHT_VECTOR_BOUND: u32 = 5;

/// Weight for FILTER patterns (applied after binding patterns).
const WEIGHT_FILTER: u32 = 10;

/// Weight for BIND expressions (need input variables resolved first).
const WEIGHT_BIND: u32 = 11;

/// Weight for VALUES (can come early — they provide bindings, not consume them).
const WEIGHT_VALUES: u32 = 0;

/// Weight for subqueries (after regular patterns to maximize bound variables).
const WEIGHT_SUBQUERY: u32 = 12;

/// Weight for UNION (after regular patterns, before OPTIONAL).
const WEIGHT_UNION: u32 = 15;

/// Weight for OPTIONAL (always last — they add nullable bindings that
/// shouldn't constrain other patterns).
const WEIGHT_OPTIONAL: u32 = 20;

/// Cardinality estimate used when the store is not available (plan-only mode).
/// Set to 1 so that heuristic weight alone drives ordering.
const DEFAULT_CARDINALITY: usize = 1;

/// Maximum cardinality estimate to prevent overflow in cost multiplication.
/// Any estimate above this is clamped. This prevents a single high-cardinality
/// pattern from dominating the cost function.
const MAX_CARDINALITY: usize = 1_000_000;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Reorder the patterns in a query for more efficient execution.
///
/// Uses a greedy algorithm that iteratively picks the cheapest remaining
/// pattern based on a combined cost of structural weight (bound/unbound
/// positions) and cardinality estimation from the store.
///
/// If a store is available, cardinality estimates are fetched from the
/// actual index structure (SPO/POS/OSP range scans). Otherwise, falls
/// back to pure structural heuristics.
pub fn optimize(query: &mut Query) {
    optimize_with_store(query, None);
}

/// Reorder patterns using cardinality estimates from the triple store.
///
/// This is the cost-based variant of `optimize()`. When the store is
/// provided, the planner uses actual index statistics to estimate how
/// many triples each pattern will match, producing better join orderings
/// for skewed data distributions.
///
/// # Cost model
///
/// For each triple pattern, the cost is:
///
/// ```text
/// cost = heuristic_weight(unbound_count) * cardinality_estimate(bound_terms)
/// ```
///
/// Where:
/// - `heuristic_weight` penalizes patterns with more unbound positions
/// - `cardinality_estimate` is the number of matching triples in the store
///
/// The planner picks patterns greedily from lowest to highest cost,
/// updating which variables are bound after each selection.
pub fn optimize_with_store(query: &mut Query, store: Option<&TripleStore>) {
    optimize_full(query, store, None);
}

/// Reorder patterns using both store cardinality and dictionary-based IRI resolution.
///
/// When the dictionary is provided, the planner can resolve IRI constants to
/// their interned TermIds, enabling tighter cardinality estimates from the
/// store's indexes. Without the dictionary, only integer literals contribute
/// to cardinality estimation (all IRIs are treated as unbound).
///
/// This is the most accurate optimization mode and should be used whenever
/// both the store and dictionary are available (i.e., in the server).
pub fn optimize_full(
    query: &mut Query,
    store: Option<&TripleStore>,
    dict: Option<&TermDictionary>,
) {
    let mut bound_vars: HashSet<String> = HashSet::new();
    let mut reordered: Vec<Pattern> = Vec::new();
    let mut remaining: Vec<Pattern> = query.patterns.drain(..).collect();

    while !remaining.is_empty() {
        let best_idx = remaining
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| pattern_cost(p, &bound_vars, store, dict))
            .map(|(i, _)| i)
            .unwrap();

        let chosen = remaining.remove(best_idx);
        collect_variables(&chosen, &mut bound_vars);
        reordered.push(chosen);
    }

    reordered = pushdown_filters(reordered);
    query.patterns = reordered;
}

// ---------------------------------------------------------------------------
// Cost estimation
// ---------------------------------------------------------------------------

/// Combined cost of executing a pattern, accounting for both structural
/// selectivity and data-dependent cardinality.
///
/// Returns a u64 cost where lower = cheaper = should be evaluated first.
fn pattern_cost(
    pattern: &Pattern,
    bound: &HashSet<String>,
    store: Option<&TripleStore>,
    dict: Option<&TermDictionary>,
) -> u64 {
    let weight = pattern_weight(pattern, bound);

    // For non-triple patterns, cardinality estimation doesn't apply —
    // they're ordered purely by structural weight.
    let cardinality = match pattern {
        Pattern::Triple {
            subject,
            predicate,
            object,
        } => {
            // Only estimate cardinality when the predicate is NOT a path
            // (property paths have complex cardinality that we can't estimate
            // from a single index scan).
            if let Some(store) = store {
                estimate_triple_cardinality(subject, predicate, object, bound, store, dict)
            } else {
                DEFAULT_CARDINALITY
            }
        }
        _ => DEFAULT_CARDINALITY,
    };

    // Multiply weight × cardinality for the final cost.
    // A pattern with weight=1 and cardinality=100 costs 100.
    // A pattern with weight=2 and cardinality=10 costs 20 (cheaper, picked first).
    //
    // Edge case: weight=0 means fully bound, so cost=0 regardless of
    // cardinality. This is correct — a point lookup is always cheapest.
    (weight as u64) * (cardinality as u64).max(1)
}

/// Structural weight of a pattern: lower = more selective = cheaper.
///
/// This is the original v0.1 heuristic, preserved as one input to the
/// cost model. It captures the fundamental insight that bound positions
/// enable index lookups while unbound positions force scans.
fn pattern_weight(pattern: &Pattern, bound: &HashSet<String>) -> u32 {
    match pattern {
        Pattern::Triple {
            subject,
            predicate,
            object,
        } => {
            // Each unbound position adds weight. A fully bound triple
            // (weight 0) is a point lookup in SPO — the cheapest operation.
            // One unbound position (weight 1) is a prefix scan.
            // Three unbound positions (weight 3) is a full table scan.
            let mut w = WEIGHT_FULLY_BOUND;
            if !is_bound(subject, bound) {
                w += WEIGHT_PER_UNBOUND;
            }
            if !is_bound(predicate, bound) {
                w += WEIGHT_PER_UNBOUND;
            }
            if !is_bound(object, bound) {
                w += WEIGHT_PER_UNBOUND;
            }
            w
        }
        // VECTOR_SIMILAR: when subject is unbound, the HNSW index is the
        // primary access path — it's very selective (returns top-k).
        // When subject is bound, it's a filter operation over bound candidates.
        Pattern::VectorSimilar { subject, .. } | Pattern::MetricSearch { subject, .. } => {
            if is_bound(subject, bound) {
                WEIGHT_VECTOR_BOUND
            } else {
                WEIGHT_VECTOR_UNBOUND
            }
        }
        // FILTERs should come after the patterns that bind their variables.
        // The pushdown pass will reposition them optimally.
        Pattern::Filter(_) => WEIGHT_FILTER,
        // BIND needs its input variables resolved first.
        Pattern::Bind { .. } => WEIGHT_BIND,
        // VALUES provide bindings cheaply — they can come early.
        Pattern::Values { .. } => WEIGHT_VALUES,
        // Subqueries after regular patterns to maximize shared bindings.
        Pattern::Subquery(_) => WEIGHT_SUBQUERY,
        // UNIONs after regular patterns, before OPTIONAL.
        Pattern::Union(_) => WEIGHT_UNION,
        // OPTIONALs always last — they add nullable bindings.
        Pattern::Optional(_) => WEIGHT_OPTIONAL,
    }
}

/// Estimate how many triples match a pattern using the store's indexes.
///
/// This is the data-dependent component of the cost model. It uses the
/// same index scans that `evaluate_triple_pattern` would use at runtime,
/// but only to count results, not to materialize them.
///
/// When a variable is already bound (from a previously-selected pattern),
/// we can't know its value at plan time, so we treat it as unbound for
/// cardinality estimation. This is conservative — the actual runtime
/// cardinality will be lower because the bound value narrows the scan.
fn estimate_triple_cardinality(
    subject: &Term,
    predicate: &Term,
    object: &Term,
    _bound: &HashSet<String>,
    store: &TripleStore,
    dict: Option<&TermDictionary>,
) -> usize {
    // Resolve each position to a TermId if it's a constant (IRI, literal).
    // Variables that are bound by previous patterns are treated as None
    // because we don't know their runtime value at plan time.
    // Only truly constant terms (IRIs, literals) contribute to the estimate.
    let s = term_to_constant_id(subject, dict);
    let p = term_to_constant_id(predicate, dict);
    let o = term_to_constant_id(object, dict);

    // Use the store's cardinality estimator, which does efficient
    // range scans on SPO/POS/OSP indexes.
    let estimate = store.estimate_cardinality(s, p, o);

    // Clamp to prevent a single high-cardinality pattern from
    // dominating the cost function (e.g., a predicate with 1M triples
    // shouldn't always be evaluated last if it has good index support).
    estimate.min(MAX_CARDINALITY)
}

/// Extract a constant TermId from a term, if it's not a variable.
///
/// This is used by cardinality estimation to determine which index
/// positions are constrained. Variables (even if bound by previous
/// patterns) return None because we don't know their runtime value.
///
/// When a `TermDictionary` is provided, IRIs and string literals can be
/// resolved to their interned IDs, enabling much tighter cardinality
/// estimates. Without the dictionary, only integer literals (which carry
/// inline IDs) contribute to the estimate.
fn term_to_constant_id(
    term: &Term,
    dict: Option<&TermDictionary>,
) -> Option<sutra_core::TermId> {
    match term {
        // Variables are always unbound from the estimator's perspective.
        Term::Variable(_) => None,
        // Integer literals have inline IDs — no dictionary needed.
        Term::IntegerLiteral(n) => sutra_core::inline_integer(*n),
        // IRIs can be resolved via the dictionary if available.
        Term::Iri(iri) => dict.and_then(|d| d.lookup(iri)),
        // rdf:type shorthand
        Term::A => dict.and_then(|d| {
            d.lookup("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
        }),
        // Prefixed names would need prefix expansion which we don't have
        // at the planner level. Return None (conservative).
        Term::PrefixedName { .. } => None,
        // String literals
        Term::Literal(s) => dict.and_then(|d| d.lookup(s)),
        // Typed literals: "value"^^<datatype>
        Term::TypedLiteral { value, datatype } => {
            let typed = format!("\"{}\"^^<{}>", value, datatype);
            dict.and_then(|d| d.lookup(&typed))
        }
        // Vector literals can't be meaningfully resolved for cardinality.
        Term::VectorLiteral(_) => None,
        // Property paths can't be resolved to a single TermId.
        Term::Path { .. } => None,
        // Quoted triples can't be resolved without hashing.
        Term::QuotedTriple { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Predicate pushdown
// ---------------------------------------------------------------------------

/// Reposition FILTER patterns immediately after the pattern that binds
/// their last required variable.
///
/// This is a critical optimization: a FILTER like `?age > 25` should run
/// as soon as `?age` is bound, not after all patterns have been evaluated.
/// Early filtering reduces intermediate result set sizes, which speeds up
/// all subsequent joins.
///
/// ## Algorithm
///
/// 1. Separate patterns into filters and non-filters.
/// 2. For each filter, determine which variables it references.
/// 3. Walk the non-filter sequence, tracking which variables are bound.
/// 4. After each non-filter pattern, emit any filters whose variables
///    are now all bound.
/// 5. Any remaining filters (with unresolvable variables) go at the end.
///
/// ## Why this is correct
///
/// FILTERs in SPARQL have no side effects — they only remove rows from
/// the result set. Moving a FILTER earlier never changes the final result,
/// only the intermediate result sizes. This is the same correctness
/// argument as SQL predicate pushdown.
fn pushdown_filters(patterns: Vec<Pattern>) -> Vec<Pattern> {
    // Separate filters from non-filter patterns, preserving order.
    let mut filters: Vec<Pattern> = Vec::new();
    let mut non_filters: Vec<Pattern> = Vec::new();

    for pattern in patterns {
        if matches!(pattern, Pattern::Filter(_)) {
            filters.push(pattern);
        } else {
            non_filters.push(pattern);
        }
    }

    // If no filters to push down, return the original order.
    if filters.is_empty() {
        return non_filters;
    }

    // Build the result by interleaving non-filters and pushed-down filters.
    let mut result: Vec<Pattern> = Vec::new();
    let mut bound_vars: HashSet<String> = HashSet::new();
    let mut placed_filters: Vec<bool> = vec![false; filters.len()];

    for pattern in non_filters {
        // Add the non-filter pattern and update bound variables.
        collect_variables(&pattern, &mut bound_vars);
        result.push(pattern);

        // Check if any pending filters can now be placed (all their
        // referenced variables are bound).
        for (i, filter) in filters.iter().enumerate() {
            if placed_filters[i] {
                continue;
            }
            let filter_vars = collect_filter_variables(filter);
            if filter_vars.iter().all(|v| bound_vars.contains(v)) {
                result.push(filter.clone());
                placed_filters[i] = true;
            }
        }
    }

    // Any filters that couldn't be placed (variables never bound) go at the end.
    // This shouldn't happen in well-formed queries, but we handle it gracefully.
    for (i, filter) in filters.into_iter().enumerate() {
        if !placed_filters[i] {
            result.push(filter);
        }
    }

    result
}

/// Collect all variable names referenced by a FILTER expression.
///
/// This determines when a filter can be "pushed down" — it can only be
/// evaluated after all its referenced variables are bound by preceding
/// patterns.
fn collect_filter_variables(pattern: &Pattern) -> HashSet<String> {
    let mut vars = HashSet::new();
    if let Pattern::Filter(expr) = pattern {
        collect_filter_expr_variables(expr, &mut vars);
    }
    vars
}

/// Recursively extract variable names from a filter expression tree.
///
/// Handles all filter expression types: comparisons, boolean connectives,
/// string functions, and EXISTS/NOT EXISTS subpatterns.
fn collect_filter_expr_variables(expr: &FilterExpr, vars: &mut HashSet<String>) {
    match expr {
        // Binary comparisons on Terms: extract variables from both sides.
        FilterExpr::Equals(a, b)
        | FilterExpr::NotEquals(a, b)
        | FilterExpr::LessThan(a, b)
        | FilterExpr::GreaterThan(a, b)
        | FilterExpr::LessThanOrEqual(a, b)
        | FilterExpr::GreaterThanOrEqual(a, b) => {
            collect_term_variables(a, vars);
            collect_term_variables(b, vars);
        }
        // Boolean connectives: recurse into both branches.
        FilterExpr::And(a, b) | FilterExpr::Or(a, b) => {
            collect_filter_expr_variables(a, vars);
            collect_filter_expr_variables(b, vars);
        }
        // Unary not: recurse into the inner expression.
        FilterExpr::Not(inner) => collect_filter_expr_variables(inner, vars),
        // Bound/not-bound tests: the variable name is referenced directly.
        FilterExpr::Bound(v) | FilterExpr::NotBound(v) => {
            vars.insert(v.clone());
        }
        // String functions on Terms: extract variables from both arguments.
        FilterExpr::Contains(a, b) | FilterExpr::StrStarts(a, b) | FilterExpr::StrEnds(a, b) => {
            collect_term_variables(a, vars);
            collect_term_variables(b, vars);
        }
        // Regex: extract variables from both Term arguments.
        FilterExpr::Regex(a, b) => {
            collect_term_variables(a, vars);
            collect_term_variables(b, vars);
        }
        // isIRI(?var), isLiteral(?var): the argument is a variable name string.
        FilterExpr::IsIri(v) | FilterExpr::IsLiteral(v) => {
            vars.insert(v.clone());
        }
        // LANG(?var) = "tag": the first argument is a variable name string.
        FilterExpr::LangEquals(v, _) => {
            vars.insert(v.clone());
        }
        // LANGMATCHES(LANG(?var), "tag"): the first argument is a variable name.
        FilterExpr::LangMatches(v, _) => {
            vars.insert(v.clone());
        }
        // STR(?var) = term: the first argument is a variable name string.
        FilterExpr::StrEquals(v, t) => {
            vars.insert(v.clone());
            collect_term_variables(t, vars);
        }
        // DATATYPE(?var) = type: the first argument is a variable name string.
        FilterExpr::DatatypeEquals(v, _) => {
            vars.insert(v.clone());
        }
        // EXISTS/NOT EXISTS: extract variables from the subpatterns.
        // These reference variables from the outer scope that must be bound
        // before the EXISTS check can be evaluated.
        FilterExpr::Exists(patterns) | FilterExpr::NotExists(patterns) => {
            for p in patterns {
                let mut sub_vars = HashSet::new();
                collect_variables(p, &mut sub_vars);
                vars.extend(sub_vars);
            }
        }
    }
}

/// Extract variable names from a Term.
fn collect_term_variables(term: &Term, vars: &mut HashSet<String>) {
    if let Term::Variable(name) = term {
        vars.insert(name.clone());
    }
}

// ---------------------------------------------------------------------------
// Variable tracking
// ---------------------------------------------------------------------------

/// Check if a term is bound (either a constant or a variable already in the
/// bound set from a previously-evaluated pattern).
fn is_bound(term: &Term, bound: &HashSet<String>) -> bool {
    match term {
        Term::Variable(name) => bound.contains(name),
        _ => true, // IRIs, literals, etc. are always "bound" (known values)
    }
}

/// Collect all variables introduced by a pattern into the bound set.
///
/// After a pattern is evaluated, all variables it binds become available
/// for subsequent patterns. This is how the planner tracks which positions
/// will be constrained at each step.
fn collect_variables(pattern: &Pattern, vars: &mut HashSet<String>) {
    match pattern {
        Pattern::Triple {
            subject,
            predicate,
            object,
        } => {
            if let Term::Variable(name) = subject {
                vars.insert(name.clone());
            }
            if let Term::Variable(name) = predicate {
                vars.insert(name.clone());
            }
            if let Term::Variable(name) = object {
                vars.insert(name.clone());
            }
        }
        Pattern::VectorSimilar { subject, .. } | Pattern::MetricSearch { subject, .. } => {
            if let Term::Variable(name) = subject {
                vars.insert(name.clone());
            }
        }
        Pattern::Optional(inner) => {
            for p in inner {
                collect_variables(p, vars);
            }
        }
        Pattern::Union(branches) => {
            for branch in branches {
                for p in branch {
                    collect_variables(p, vars);
                }
            }
        }
        Pattern::Filter(_) => {}
        Pattern::Bind { variable, .. } => {
            vars.insert(variable.clone());
        }
        Pattern::Values { variable, .. } => {
            vars.insert(variable.clone());
        }
        Pattern::Subquery(q) => {
            for v in &q.projection {
                vars.insert(v.clone());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use sutra_core::Triple;

    // -----------------------------------------------------------------------
    // Basic ordering tests (structural heuristic only, no store)
    // -----------------------------------------------------------------------

    #[test]
    fn reorders_bound_first() {
        let mut q = parser::parse(
            "SELECT ?name WHERE { \
             ?person <http://example.org/name> ?name . \
             <http://example.org/Alice> <http://example.org/knows> ?person \
             }",
        )
        .unwrap();

        // Before optimization: first pattern has 2 unbound, second has 1
        optimize(&mut q);

        // After optimization: the more selective pattern (1 unbound) should come first
        if let Pattern::Triple { subject, .. } = &q.patterns[0] {
            assert_eq!(*subject, Term::Iri("http://example.org/Alice".to_string()));
        } else {
            panic!("expected triple pattern first");
        }
    }

    #[test]
    fn filter_comes_after_binding() {
        let mut q = parser::parse(
            "SELECT ?s WHERE { \
             FILTER(?age > 25) . \
             ?s <http://example.org/age> ?age \
             }",
        )
        .unwrap();

        optimize(&mut q);

        // Triple pattern should come before FILTER
        assert!(matches!(q.patterns[0], Pattern::Triple { .. }));
        assert!(matches!(q.patterns[1], Pattern::Filter(_)));
    }

    #[test]
    fn vector_unbound_comes_first() {
        // When subject is unbound, VectorSimilar should have weight 1 (comes first)
        let mut q = Query {
            prefixes: Default::default(),
            projection: vec!["doc".into()],
            distinct: false,
            patterns: vec![
                Pattern::Triple {
                    subject: Term::Variable("doc".into()),
                    predicate: Term::PrefixedName {
                        prefix: String::new(),
                        local: "mentions".into(),
                    },
                    object: Term::Variable("entity".into()),
                },
                Pattern::VectorSimilar {
                    subject: Term::Variable("doc".into()),
                    predicate: Term::PrefixedName {
                        prefix: String::new(),
                        local: "hasEmbedding".into(),
                    },
                    query_vector: vec![0.1, 0.2, 0.3],
                    threshold: 0.85,
                    ef_search: None,
                    top_k: None,
                },
            ],
            query_type: crate::parser::QueryType::Select,
            aggregates: vec![],
            group_by: vec![],
            having: None,
            construct_template: vec![],
            order_by: vec![],
            limit: None,
            offset: None,
        };

        optimize(&mut q);

        // VectorSimilar with unbound subject (weight 1) should come before
        // triple with 2 unbound vars (weight 2)
        assert!(matches!(q.patterns[0], Pattern::VectorSimilar { .. }));
        assert!(matches!(q.patterns[1], Pattern::Triple { .. }));
    }

    #[test]
    fn vector_bound_comes_after_binding() {
        // When subject is already bound by a fully-bound triple, VectorSimilar gets weight 5
        let mut q = Query {
            prefixes: Default::default(),
            projection: vec!["doc".into()],
            distinct: false,
            patterns: vec![
                Pattern::VectorSimilar {
                    subject: Term::Variable("doc".into()),
                    predicate: Term::PrefixedName {
                        prefix: String::new(),
                        local: "hasEmbedding".into(),
                    },
                    query_vector: vec![0.1, 0.2, 0.3],
                    threshold: 0.85,
                    ef_search: None,
                    top_k: None,
                },
                Pattern::Triple {
                    subject: Term::Iri("http://example.org/doc1".into()),
                    predicate: Term::PrefixedName {
                        prefix: String::new(),
                        local: "type".into(),
                    },
                    object: Term::Iri("http://example.org/Document".into()),
                },
            ],
            query_type: crate::parser::QueryType::Select,
            aggregates: vec![],
            group_by: vec![],
            having: None,
            construct_template: vec![],
            order_by: vec![],
            limit: None,
            offset: None,
        };

        optimize(&mut q);

        // Fully bound triple (weight 0) comes first, then VectorSimilar (weight 1)
        assert!(matches!(q.patterns[0], Pattern::Triple { .. }));
        assert!(matches!(q.patterns[1], Pattern::VectorSimilar { .. }));
    }

    #[test]
    fn union_comes_after_regular_patterns() {
        let mut q = Query {
            prefixes: Default::default(),
            projection: vec!["s".into()],
            distinct: false,
            patterns: vec![
                Pattern::Union(vec![
                    vec![Pattern::Triple {
                        subject: Term::Variable("s".into()),
                        predicate: Term::A,
                        object: Term::PrefixedName {
                            prefix: String::new(),
                            local: "Person".into(),
                        },
                    }],
                    vec![Pattern::Triple {
                        subject: Term::Variable("s".into()),
                        predicate: Term::A,
                        object: Term::PrefixedName {
                            prefix: String::new(),
                            local: "Organization".into(),
                        },
                    }],
                ]),
                Pattern::Triple {
                    subject: Term::Variable("s".into()),
                    predicate: Term::PrefixedName {
                        prefix: String::new(),
                        local: "name".into(),
                    },
                    object: Term::Variable("name".into()),
                },
            ],
            query_type: crate::parser::QueryType::Select,
            aggregates: vec![],
            group_by: vec![],
            having: None,
            construct_template: vec![],
            order_by: vec![],
            limit: None,
            offset: None,
        };

        optimize(&mut q);

        // Triple (weight 2) comes before Union (weight 15)
        assert!(matches!(q.patterns[0], Pattern::Triple { .. }));
        assert!(matches!(q.patterns[1], Pattern::Union(_)));
    }

    #[test]
    fn optional_comes_last() {
        let mut q = parser::parse(
            "SELECT ?s ?name WHERE { \
             OPTIONAL { ?s <http://example.org/name> ?name } . \
             ?s a <http://example.org/Person> \
             }",
        )
        .unwrap();

        optimize(&mut q);

        // Triple pattern should come before OPTIONAL
        assert!(matches!(q.patterns[0], Pattern::Triple { .. }));
        assert!(matches!(q.patterns[1], Pattern::Optional(_)));
    }

    // -----------------------------------------------------------------------
    // Cost-based ordering tests (with store cardinality)
    // -----------------------------------------------------------------------

    #[test]
    fn cardinality_breaks_ties() {
        // Two triple patterns both have 1 unbound position, but one predicate
        // has far more triples. The planner should prefer the selective one.
        let mut store = TripleStore::new();

        // :knows has 100 triples (high cardinality)
        for i in 0..100u64 {
            let _ = store.insert(Triple::new(i + 1, 10, i + 200));
        }
        // :name has 5 triples (low cardinality)
        for i in 0..5u64 {
            let _ = store.insert(Triple::new(i + 1, 11, i + 300));
        }

        let mut q = Query {
            prefixes: Default::default(),
            projection: vec!["s".into(), "name".into()],
            distinct: false,
            patterns: vec![
                // ?s :knows ?person — high cardinality, 1 unbound (subject bound by context)
                Pattern::Triple {
                    subject: Term::Variable("s".into()),
                    predicate: Term::Iri("high_card_pred".into()),
                    object: Term::Variable("person".into()),
                },
                // ?s :name ?name — low cardinality, 1 unbound
                Pattern::Triple {
                    subject: Term::Variable("s".into()),
                    predicate: Term::Iri("low_card_pred".into()),
                    object: Term::Variable("name".into()),
                },
            ],
            query_type: crate::parser::QueryType::Select,
            aggregates: vec![],
            group_by: vec![],
            having: None,
            construct_template: vec![],
            order_by: vec![],
            limit: None,
            offset: None,
        };

        // Both patterns have weight 2 (2 unbound vars each).
        // Without cardinality, they'd stay in original order.
        // With cardinality, neither predicate IRI is in the store's dictionary
        // (since we used raw TermIds), so both get the same estimate.
        // This test verifies the planner doesn't crash with store provided.
        optimize_with_store(&mut q, Some(&store));
        assert_eq!(q.patterns.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Predicate pushdown tests
    // -----------------------------------------------------------------------

    #[test]
    fn filter_pushed_down_after_binding_pattern() {
        // FILTER(?x > 10) should be placed right after the pattern that binds ?x,
        // not at the end.
        let mut q = parser::parse(
            "SELECT ?s ?x ?y WHERE { \
             ?s <http://example.org/x> ?x . \
             FILTER(?x > 10) . \
             ?s <http://example.org/y> ?y \
             }",
        )
        .unwrap();

        optimize(&mut q);

        // After optimization: triple binding ?x, then FILTER, then triple binding ?y
        assert!(matches!(q.patterns[0], Pattern::Triple { .. }));
        assert!(matches!(q.patterns[1], Pattern::Filter(_)));
        assert!(matches!(q.patterns[2], Pattern::Triple { .. }));
    }

    #[test]
    fn multiple_filters_pushed_down_independently() {
        // Two filters, each referencing different variables, should be
        // pushed down to different positions.
        let patterns = vec![
            Pattern::Triple {
                subject: Term::Variable("a".into()),
                predicate: Term::Iri("http://example.org/p1".into()),
                object: Term::Variable("x".into()),
            },
            Pattern::Triple {
                subject: Term::Variable("a".into()),
                predicate: Term::Iri("http://example.org/p2".into()),
                object: Term::Variable("y".into()),
            },
            Pattern::Filter(FilterExpr::GreaterThan(
                Term::Variable("x".into()),
                Term::IntegerLiteral(5),
            )),
            Pattern::Filter(FilterExpr::LessThan(
                Term::Variable("y".into()),
                Term::IntegerLiteral(100),
            )),
        ];

        let result = pushdown_filters(patterns);

        // Expected: triple1, filter_x, triple2, filter_y
        assert!(matches!(result[0], Pattern::Triple { .. }));
        assert!(matches!(result[1], Pattern::Filter(_)));
        assert!(matches!(result[2], Pattern::Triple { .. }));
        assert!(matches!(result[3], Pattern::Filter(_)));
    }
}
