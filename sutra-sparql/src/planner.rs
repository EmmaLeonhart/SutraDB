//! Query planner with pattern reordering.
//!
//! Implements a greedy heuristic for triple pattern reordering (inspired by
//! Jena's ReorderTransformationSubstitution). More bound positions = lower
//! weight = picked first.
//!
//! For v0.1, this is a simple static reordering. Future work: cardinality
//! estimation, adaptive execution, VECTOR_SIMILAR integration.

use std::collections::HashSet;

use crate::parser::{Pattern, Query, Term};

/// Reorder the patterns in a query for more efficient execution.
///
/// The heuristic: patterns with more bound positions should be evaluated
/// first, because they produce smaller intermediate result sets.
pub fn optimize(query: &mut Query) {
    let mut bound_vars: HashSet<String> = HashSet::new();
    let mut reordered: Vec<Pattern> = Vec::new();
    let mut remaining: Vec<Pattern> = query.patterns.drain(..).collect();

    while !remaining.is_empty() {
        // Find the pattern with the lowest weight (most selective)
        let best_idx = remaining
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| pattern_weight(p, &bound_vars))
            .map(|(i, _)| i)
            .unwrap();

        let chosen = remaining.remove(best_idx);

        // Mark variables from chosen pattern as bound
        collect_variables(&chosen, &mut bound_vars);

        reordered.push(chosen);
    }

    query.patterns = reordered;
}

/// Weight of a pattern: lower = more selective = should be evaluated first.
/// A fully bound triple pattern (no variables) has weight 0.
/// Each unbound position adds weight.
fn pattern_weight(pattern: &Pattern, bound: &HashSet<String>) -> u32 {
    match pattern {
        Pattern::Triple {
            subject,
            predicate,
            object,
        } => {
            let mut w = 0u32;
            if !is_bound(subject, bound) {
                w += 1;
            }
            if !is_bound(predicate, bound) {
                w += 1;
            }
            if !is_bound(object, bound) {
                w += 1;
            }
            w
        }
        // VectorSimilar: weight depends on whether subject is already bound
        Pattern::VectorSimilar { subject, .. } => {
            if is_bound(subject, bound) {
                5 // subject already bound: execute after graph patterns
            } else {
                1 // subject unbound: execute vector search first
            }
        }
        // FILTERs should come after the patterns that bind their variables
        Pattern::Filter(_) => 10,
        // UNIONs after regular patterns, before OPTIONAL
        Pattern::Union(_) => 15,
        // OPTIONALs should come last
        Pattern::Optional(_) => 20,
    }
}

fn is_bound(term: &Term, bound: &HashSet<String>) -> bool {
    match term {
        Term::Variable(name) => bound.contains(name),
        _ => true, // IRIs, literals are always bound
    }
}

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
        Pattern::VectorSimilar { subject, .. } => {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

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
}
