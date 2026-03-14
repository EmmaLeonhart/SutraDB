//! Query executor.
//!
//! Evaluates parsed SPARQL queries against a TripleStore + TermDictionary.
//! Uses the Volcano/iterator model: each pattern produces a stream of
//! binding rows that are joined together.

use std::collections::HashMap;

use sutra_core::{TermDictionary, TermId, Triple, TripleStore};

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
}

/// Execute a parsed query against an in-memory store.
pub fn execute(query: &Query, store: &TripleStore, dict: &TermDictionary) -> Result<QueryResult> {
    // Start with a single empty binding
    let mut results = vec![HashMap::new()];

    // Evaluate each pattern
    for pattern in &query.patterns {
        results = evaluate_pattern(pattern, &results, store, dict, &query.prefixes)?;
    }

    // Apply DISTINCT
    if query.distinct {
        let mut seen = std::collections::HashSet::new();
        results.retain(|row| {
            let key: Vec<_> = row.iter().collect();
            let key_str = format!("{:?}", key);
            seen.insert(key_str)
        });
    }

    // Apply OFFSET
    if let Some(offset) = query.offset {
        if offset < results.len() {
            results = results[offset..].to_vec();
        } else {
            results.clear();
        }
    }

    // Apply LIMIT
    if let Some(limit) = query.limit {
        results.truncate(limit);
    }

    // Determine columns
    let columns = if query.projection.is_empty() {
        // SELECT * — collect all variables
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
    })
}

fn evaluate_pattern(
    pattern: &Pattern,
    current: &[Bindings],
    store: &TripleStore,
    dict: &TermDictionary,
    prefixes: &HashMap<String, String>,
) -> Result<Vec<Bindings>> {
    match pattern {
        Pattern::Triple {
            subject,
            predicate,
            object,
        } => evaluate_triple_pattern(subject, predicate, object, current, store, dict, prefixes),
        Pattern::Optional(inner_patterns) => {
            let mut result = Vec::new();
            for row in current {
                let mut inner_results = vec![row.clone()];
                for p in inner_patterns {
                    inner_results = evaluate_pattern(p, &inner_results, store, dict, prefixes)?;
                }
                if inner_results.is_empty() {
                    // OPTIONAL: keep the original row if no match
                    result.push(row.clone());
                } else {
                    result.extend(inner_results);
                }
            }
            Ok(result)
        }
        Pattern::Filter(expr) => {
            let filtered: Vec<_> = current
                .iter()
                .filter(|row| evaluate_filter(expr, row))
                .cloned()
                .collect();
            Ok(filtered)
        }
    }
}

fn evaluate_triple_pattern(
    subject: &Term,
    predicate: &Term,
    object: &Term,
    current: &[Bindings],
    store: &TripleStore,
    dict: &TermDictionary,
    prefixes: &HashMap<String, String>,
) -> Result<Vec<Bindings>> {
    let mut results = Vec::new();

    for row in current {
        let s_id = resolve_term(subject, row, dict, prefixes)?;
        let p_id = resolve_term(predicate, row, dict, prefixes)?;
        let o_id = resolve_term(object, row, dict, prefixes)?;

        // If a concrete term (non-variable) doesn't exist in the dictionary,
        // there can be no matches.
        if is_concrete(subject) && s_id.is_none() {
            continue;
        }
        if is_concrete(predicate) && p_id.is_none() {
            continue;
        }
        if is_concrete(object) && o_id.is_none() {
            continue;
        }

        // Choose the best index access pattern based on which terms are bound
        let candidates: Vec<Triple> = match (s_id, p_id) {
            (Some(s), Some(p)) => store.find_by_subject_predicate(s, p),
            (Some(s), None) => store.find_by_subject(s),
            (None, Some(p)) => store.find_by_predicate(p),
            (None, None) => {
                if let Some(o) = o_id {
                    store.find_by_object(o)
                } else {
                    store.iter().collect()
                }
            }
        };

        for triple in candidates {
            // Check that bound terms match
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

            // Build new bindings
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
        }
    }

    Ok(results)
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
    }
}

fn evaluate_filter(expr: &FilterExpr, row: &Bindings) -> bool {
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
    }
}

/// Returns true if the term is concrete (not a variable).
fn is_concrete(term: &Term) -> bool {
    !matches!(term, Term::Variable(_))
}

fn filter_term_value(term: &Term, row: &Bindings) -> Option<TermId> {
    match term {
        Term::Variable(name) => row.get(name).copied(),
        Term::IntegerLiteral(n) => sutra_core::inline_integer(*n),
        _ => None, // Simplified: only variables and integers in filters for now
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

        // Age as inline integer
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
        assert_eq!(result.rows.len(), 3); // Alice→Bob, Alice→Charlie, Bob→Alice
    }

    #[test]
    fn select_with_bound_subject() {
        let (store, dict) = setup();
        let q = parser::parse(
            "SELECT ?o WHERE { <http://example.org/Alice> <http://example.org/knows> ?o }",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 2); // Bob and Charlie
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
        assert_eq!(result.rows.len(), 2); // Alice and Bob
    }

    #[test]
    fn select_with_join() {
        let (store, dict) = setup();
        // Find names of people Alice knows
        let q = parser::parse(
            "SELECT ?name WHERE { \
             <http://example.org/Alice> <http://example.org/knows> ?person . \
             ?person <http://example.org/name> ?name \
             }",
        )
        .unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        // Alice knows Bob and Charlie. Bob has a name, Charlie doesn't.
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
        assert_eq!(result.rows.len(), 1); // Only Alice (age 30)
    }

    #[test]
    fn empty_result() {
        let (store, dict) = setup();
        let q =
            parser::parse("SELECT ?s WHERE { ?s <http://example.org/nonexistent> ?o }").unwrap();
        let result = execute(&q, &store, &dict).unwrap();
        assert_eq!(result.rows.len(), 0);
    }
}
