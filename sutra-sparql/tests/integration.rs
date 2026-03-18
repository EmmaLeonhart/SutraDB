//! Integration tests for sutra-sparql: full query pipeline with vector support.
//!
//! Tests the complete path: parse → optimize → execute, including
//! VECTOR_SIMILAR, VECTOR_SCORE, ORDER BY, UNION, and graph+vector joins.

use sutra_core::{DatabaseConfig, HnswEdgeMode, TermDictionary, TermId, Triple, TripleStore};
use sutra_hnsw::{DistanceMetric, VectorPredicateConfig, VectorRegistry};
use sutra_sparql::{execute_with_config, execute_with_vectors, optimize, parse};

/// Build a test knowledge graph about academic papers with embeddings.
fn academic_graph() -> (TripleStore, TermDictionary, VectorRegistry) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    // Predicates
    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let has_embedding = dict.intern("http://example.org/hasEmbedding");
    let title = dict.intern("http://example.org/title");
    let author = dict.intern("http://example.org/author");
    let cites = dict.intern("http://example.org/cites");
    let year = dict.intern("http://example.org/year");

    // Classes
    let paper = dict.intern("http://example.org/Paper");
    let person = dict.intern("http://example.org/Person");

    // Papers
    let p1 = dict.intern("http://example.org/paper/attention");
    let p2 = dict.intern("http://example.org/paper/bert");
    let p3 = dict.intern("http://example.org/paper/gpt");
    let p4 = dict.intern("http://example.org/paper/cooking");
    let p5 = dict.intern("http://example.org/paper/gardening");

    // Authors
    let vaswani = dict.intern("http://example.org/person/vaswani");
    let devlin = dict.intern("http://example.org/person/devlin");

    // Titles
    let t1 = dict.intern("\"Attention Is All You Need\"");
    let t2 = dict.intern("\"BERT: Pre-training\"");
    let t3 = dict.intern("\"Language Models are Few-Shot Learners\"");
    let t4 = dict.intern("\"Cooking with Herbs\"");
    let t5 = dict.intern("\"Garden Design Principles\"");

    // Type assertions
    store.insert(Triple::new(p1, rdf_type, paper)).unwrap();
    store.insert(Triple::new(p2, rdf_type, paper)).unwrap();
    store.insert(Triple::new(p3, rdf_type, paper)).unwrap();
    store.insert(Triple::new(p4, rdf_type, paper)).unwrap();
    store.insert(Triple::new(p5, rdf_type, paper)).unwrap();
    store
        .insert(Triple::new(vaswani, rdf_type, person))
        .unwrap();
    store.insert(Triple::new(devlin, rdf_type, person)).unwrap();

    // Titles
    store.insert(Triple::new(p1, title, t1)).unwrap();
    store.insert(Triple::new(p2, title, t2)).unwrap();
    store.insert(Triple::new(p3, title, t3)).unwrap();
    store.insert(Triple::new(p4, title, t4)).unwrap();
    store.insert(Triple::new(p5, title, t5)).unwrap();

    // Authors
    store.insert(Triple::new(p1, author, vaswani)).unwrap();
    store.insert(Triple::new(p2, author, devlin)).unwrap();

    // Citations
    store.insert(Triple::new(p2, cites, p1)).unwrap();
    store.insert(Triple::new(p3, cites, p1)).unwrap();
    store.insert(Triple::new(p3, cites, p2)).unwrap();

    // Years as inline integers
    let y2017 = sutra_core::inline_integer(2017).unwrap();
    let y2018 = sutra_core::inline_integer(2018).unwrap();
    let y2020 = sutra_core::inline_integer(2020).unwrap();
    let y2019 = sutra_core::inline_integer(2019).unwrap();
    let y2021 = sutra_core::inline_integer(2021).unwrap();
    store.insert(Triple::new(p1, year, y2017)).unwrap();
    store.insert(Triple::new(p2, year, y2018)).unwrap();
    store.insert(Triple::new(p3, year, y2020)).unwrap();
    store.insert(Triple::new(p4, year, y2019)).unwrap();
    store.insert(Triple::new(p5, year, y2021)).unwrap();

    // Vector index
    let mut vectors = VectorRegistry::new();
    vectors
        .declare(VectorPredicateConfig {
            predicate_id: has_embedding,
            dimensions: 4,
            m: 4,
            ef_construction: 20,
            metric: DistanceMetric::Cosine,
        })
        .unwrap();

    // Vector objects — each paper links to its vector via a triple
    let v1 = dict.intern("\"vec_p1\"^^<http://sutra.dev/f32vec>");
    let v2 = dict.intern("\"vec_p2\"^^<http://sutra.dev/f32vec>");
    let v3 = dict.intern("\"vec_p3\"^^<http://sutra.dev/f32vec>");
    let v4 = dict.intern("\"vec_p4\"^^<http://sutra.dev/f32vec>");
    let v5 = dict.intern("\"vec_p5\"^^<http://sutra.dev/f32vec>");

    // Triples linking papers to their vector objects
    store.insert(Triple::new(p1, has_embedding, v1)).unwrap();
    store.insert(Triple::new(p2, has_embedding, v2)).unwrap();
    store.insert(Triple::new(p3, has_embedding, v3)).unwrap();
    store.insert(Triple::new(p4, has_embedding, v4)).unwrap();
    store.insert(Triple::new(p5, has_embedding, v5)).unwrap();

    // NLP papers get similar embeddings, non-NLP papers get different ones
    // HNSW is keyed by the vector object ID
    vectors
        .insert(has_embedding, vec![0.9, 0.1, 0.0, 0.0], v1)
        .unwrap(); // attention
    vectors
        .insert(has_embedding, vec![0.85, 0.15, 0.0, 0.0], v2)
        .unwrap(); // bert
    vectors
        .insert(has_embedding, vec![0.8, 0.2, 0.0, 0.0], v3)
        .unwrap(); // gpt
    vectors
        .insert(has_embedding, vec![0.0, 0.0, 0.9, 0.1], v4)
        .unwrap(); // cooking
    vectors
        .insert(has_embedding, vec![0.0, 0.0, 0.1, 0.9], v5)
        .unwrap(); // gardening

    (store, dict, vectors)
}

// --- VECTOR_SIMILAR Tests ---

#[test]
fn vector_similar_finds_similar_papers() {
    let (store, dict, vectors) = academic_graph();

    // Search for papers similar to NLP embedding
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.9 0.1 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.7) \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    // p1, p2, p3 should match (NLP cluster); p4, p5 should not (cooking/gardening)
    assert_eq!(result.rows.len(), 3);
    let paper_ids: Vec<TermId> = result
        .rows
        .iter()
        .map(|r| *r.get("paper").unwrap())
        .collect();
    let p1 = dict.lookup("http://example.org/paper/attention").unwrap();
    let p2 = dict.lookup("http://example.org/paper/bert").unwrap();
    let p3 = dict.lookup("http://example.org/paper/gpt").unwrap();
    assert!(paper_ids.contains(&p1));
    assert!(paper_ids.contains(&p2));
    assert!(paper_ids.contains(&p3));
}

#[test]
fn vector_similar_with_high_threshold() {
    let (store, dict, vectors) = academic_graph();

    // High threshold — only very close matches
    // After normalization, NLP vectors all have cosine >0.99 with each other,
    // so we use a threshold that excludes the non-NLP vectors (cooking/gardening)
    // but query from the cooking direction to get fewer NLP matches
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.0 0.0 0.95 0.05\"^^<http://sutra.dev/f32vec>, 0.9) \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // Only the cooking paper (p4) should be very similar to this query vector
    assert_eq!(result.rows.len(), 1);
    let p4 = dict.lookup("http://example.org/paper/cooking").unwrap();
    assert_eq!(*result.rows[0].get("paper").unwrap(), p4);
}

#[test]
fn vector_similar_with_graph_filter() {
    let (store, dict, vectors) = academic_graph();

    // Find similar papers that were published after 2018
    let mut q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper ?year WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.9 0.1 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.7) \
         ?paper ex:year ?year . \
         FILTER(?year > 2018) \
         }",
    )
    .unwrap();

    optimize(&mut q);
    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    // p3 (GPT, 2020) should match. p1 (2017) and p2 (2018) should not pass filter.
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn vector_similar_with_type_constraint() {
    let (store, dict, vectors) = academic_graph();

    // Combine vector search with rdf:type constraint
    let mut q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper ?title WHERE { \
         ?paper a ex:Paper . \
         ?paper ex:title ?title . \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.0 0.0 0.9 0.1\"^^<http://sutra.dev/f32vec>, 0.7) \
         }",
    )
    .unwrap();

    optimize(&mut q);
    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    // Should find cooking paper (similar vector) that is also a Paper
    assert!(result.rows.len() >= 1);
    let paper_ids: Vec<TermId> = result
        .rows
        .iter()
        .map(|r| *r.get("paper").unwrap())
        .collect();
    let cooking = dict.lookup("http://example.org/paper/cooking").unwrap();
    assert!(paper_ids.contains(&cooking));
}

#[test]
fn vector_similar_scores_are_populated() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.9 0.1 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.5) \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    // Every matching row should have a score
    for score_row in &result.scores {
        assert!(!score_row.is_empty(), "score should be populated");
        for (_, &score) in score_row {
            assert!(score >= 0.5, "score should meet threshold");
        }
    }
}

// --- UNION Tests ---

#[test]
fn union_combines_branches() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?x WHERE { \
         { ?x a ex:Paper } \
         UNION \
         { ?x a ex:Person } \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // 5 papers + 2 people = 7
    assert_eq!(result.rows.len(), 7);
}

#[test]
fn union_with_different_variables() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper ?author WHERE { \
         { ?paper ex:author ?author } \
         UNION \
         { ?paper ex:cites ?cited } \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // 2 author triples + 3 citation triples = 5
    assert_eq!(result.rows.len(), 5);
}

// --- ORDER BY Tests ---

#[test]
fn order_by_ascending() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper ?year WHERE { \
         ?paper ex:year ?year \
         } ORDER BY ASC(?year)",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 5);

    // Years should be in ascending order
    let years: Vec<i64> = result
        .rows
        .iter()
        .map(|r| sutra_core::decode_inline_integer(*r.get("year").unwrap()).unwrap())
        .collect();
    for w in years.windows(2) {
        assert!(w[0] <= w[1], "years should be ascending: {:?}", years);
    }
}

#[test]
fn order_by_descending() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper ?year WHERE { \
         ?paper ex:year ?year \
         } ORDER BY DESC(?year)",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    let years: Vec<i64> = result
        .rows
        .iter()
        .map(|r| sutra_core::decode_inline_integer(*r.get("year").unwrap()).unwrap())
        .collect();
    for w in years.windows(2) {
        assert!(w[0] >= w[1], "years should be descending: {:?}", years);
    }
}

// --- Multi-pattern query tests ---

#[test]
fn join_across_three_patterns() {
    let (store, dict, vectors) = academic_graph();

    // Find titles of papers that cite papers authored by Vaswani
    let mut q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?citing_title WHERE { \
         ?original ex:author <http://example.org/person/vaswani> . \
         ?citing ex:cites ?original . \
         ?citing ex:title ?citing_title \
         }",
    )
    .unwrap();

    optimize(&mut q);
    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    // BERT and GPT cite Attention (authored by Vaswani)
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn optional_with_vector() {
    let (store, dict, vectors) = academic_graph();

    // Get papers with optional author info, filtered by vector similarity
    let mut q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper ?author WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.9 0.1 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.7) \
         OPTIONAL { ?paper ex:author ?author } \
         }",
    )
    .unwrap();

    optimize(&mut q);
    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    // 3 NLP papers match, some have authors, some don't
    assert_eq!(result.rows.len(), 3);

    // Count how many have author bindings
    let with_author = result
        .rows
        .iter()
        .filter(|r| r.contains_key("author"))
        .count();
    // p1 has Vaswani, p2 has Devlin, p3 has no author
    assert_eq!(with_author, 2);
}

#[test]
fn distinct_eliminates_duplicates() {
    let (store, dict, vectors) = academic_graph();

    // Without DISTINCT
    let q1 = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?cited WHERE { \
         ?paper ex:cites ?cited \
         }",
    )
    .unwrap();
    let r1 = execute_with_vectors(&q1, &store, &dict, &vectors).unwrap();

    // With DISTINCT
    let q2 = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT DISTINCT ?cited WHERE { \
         ?paper ex:cites ?cited \
         }",
    )
    .unwrap();
    let r2 = execute_with_vectors(&q2, &store, &dict, &vectors).unwrap();

    // p1 is cited by both p2 and p3, so DISTINCT should reduce count
    assert!(r2.rows.len() <= r1.rows.len());
    assert_eq!(r2.rows.len(), 2); // p1 and p2 (cited)
}

#[test]
fn limit_and_offset() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { \
         ?paper a ex:Paper \
         } LIMIT 2 OFFSET 1",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 2);
}

// --- Edge cases ---

#[test]
fn vector_similar_no_matches() {
    let (store, dict, vectors) = academic_graph();

    // Query vector that matches nothing at high threshold
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.5 0.5 0.5 0.5\"^^<http://sutra.dev/f32vec>, 0.99) \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // Threshold 0.99 is very high — normalized vectors won't hit this
    assert_eq!(result.rows.len(), 0);
}

#[test]
fn empty_graph_query() {
    let store = TripleStore::new();
    let dict = TermDictionary::new();
    let vectors = VectorRegistry::new();

    let q = parse("SELECT * WHERE { ?s ?p ?o }").unwrap();
    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 0);
}

#[test]
fn vector_similar_with_ef_hint() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.9 0.1 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.7, ef:=50) \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // Should still find NLP papers
    assert!(result.rows.len() >= 3);
}

#[test]
fn vector_similar_with_k_hint() {
    let (store, dict, vectors) = academic_graph();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { \
         VECTOR_SIMILAR(?paper ex:hasEmbedding \"0.9 0.1 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.0, k:=2) \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // k:=2 limits to 2 results (threshold 0.0 means all pass)
    assert_eq!(result.rows.len(), 2);
}

// --- Backward compatibility ---

#[test]
fn standard_sparql_still_works() {
    let (store, dict, vectors) = academic_graph();

    // Standard SPARQL 1.1 query — no vector extensions
    let mut q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper ?title WHERE { \
         ?paper a ex:Paper . \
         ?paper ex:title ?title \
         }",
    )
    .unwrap();

    optimize(&mut q);
    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 5);
}

#[test]
fn execute_without_vectors_backward_compat() {
    let (store, dict, _vectors) = academic_graph();

    // Using the old execute() function (no vectors)
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?paper WHERE { ?paper a ex:Paper }",
    )
    .unwrap();

    let result = sutra_sparql::execute(&q, &store, &dict).unwrap();
    assert_eq!(result.rows.len(), 5);
}

// --- Planner integration ---

#[test]
fn planner_puts_vector_first_for_unbound() {
    // Verify that the planner reorders VECTOR_SIMILAR before triple patterns
    // when the subject is unbound and vector pattern has lower weight.
    // Here we use two triple patterns after VECTOR_SIMILAR so the vector
    // pattern (weight 1) clearly beats the 3-unbound triple pattern (weight 3).
    let mut q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?doc ?title WHERE { \
         ?doc ex:title ?title . \
         ?thing ?pred ?obj . \
         VECTOR_SIMILAR(?doc ex:hasEmbedding \"0.9 0.1 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.7) \
         }",
    )
    .unwrap();

    optimize(&mut q);

    // VECTOR_SIMILAR (weight 1, only subject unbound) should come before
    // ?thing ?pred ?obj (weight 3, all unbound)
    assert!(
        matches!(
            q.patterns[0],
            sutra_sparql::parser::Pattern::VectorSimilar { .. }
        ),
        "VECTOR_SIMILAR should be first after optimization, got: {:?}",
        q.patterns[0]
    );
}

// --- RDF-star edge annotation ---

#[test]
fn rdf_star_edge_with_vector() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let alice = dict.intern("http://example.org/Alice");
    let bob = dict.intern("http://example.org/Bob");
    let knows = dict.intern("http://example.org/knows");
    let confidence = dict.intern("http://example.org/confidence");

    // Base triple
    store.insert(Triple::new(alice, knows, bob)).unwrap();

    // RDF-star: annotate the edge with confidence
    let edge_id = sutra_core::quoted_triple_id(alice, knows, bob);
    let conf_val = sutra_core::inline_integer(95).unwrap();
    store
        .insert(Triple::new(edge_id, confidence, conf_val))
        .unwrap();

    // Query the annotation
    let q = parse(
        "SELECT ?conf WHERE { \
         ?edge <http://example.org/confidence> ?conf \
         }",
    )
    .unwrap();

    let vectors = VectorRegistry::new();
    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 1);
    let conf = sutra_core::decode_inline_integer(*result.rows[0].get("conf").unwrap()).unwrap();
    assert_eq!(conf, 95);
}

// --- Virtual HNSW edge triples ---

#[test]
fn virtual_hnsw_edges_queryable_via_sparql() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let has_embedding = dict.intern("http://example.org/hasEmbedding");

    // Register the HNSW neighbor predicate in the dictionary so queries can resolve it
    let _neighbor_id = dict.intern(sutra_hnsw::HNSW_NEIGHBOR_IRI);

    let mut vectors = VectorRegistry::new();
    vectors
        .declare(VectorPredicateConfig {
            predicate_id: has_embedding,
            dimensions: 3,
            m: 4,
            ef_construction: 20,
            metric: DistanceMetric::Cosine,
        })
        .unwrap();

    // Insert vectors — these become nodes in the HNSW graph
    let doc1 = dict.intern("http://example.org/doc1");
    let doc2 = dict.intern("http://example.org/doc2");
    let doc3 = dict.intern("http://example.org/doc3");
    let vec1_id = dict.intern("\"vec_doc1\"^^<http://sutra.dev/f32vec>");
    let vec2_id = dict.intern("\"vec_doc2\"^^<http://sutra.dev/f32vec>");
    let vec3_id = dict.intern("\"vec_doc3\"^^<http://sutra.dev/f32vec>");

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
        .insert(has_embedding, vec![0.0, 0.0, 1.0], vec3_id)
        .unwrap();

    // Query HNSW edges as virtual triples
    let q = parse(
        "SELECT ?source ?target WHERE { \
         ?source <http://sutra.dev/hnswNeighbor> ?target \
         }",
    )
    .unwrap();

    let config = DatabaseConfig {
        hnsw_edge_mode: HnswEdgeMode::Virtual,
        ..Default::default()
    };

    let result = execute_with_config(&q, &store, &dict, &vectors, &config).unwrap();

    // HNSW builds bidirectional edges, so there should be some edges
    assert!(
        !result.rows.is_empty(),
        "Virtual HNSW edge query should return edges"
    );

    // Virtual HNSW edges now resolve back to entity IRIs (not vector object IDs)
    let valid_entities = [doc1, doc2, doc3];
    for row in &result.rows {
        let source = row.get("source").unwrap();
        let target = row.get("target").unwrap();
        assert!(
            valid_entities.contains(source),
            "Source {} not a valid entity ID",
            source
        );
        assert!(
            valid_entities.contains(target),
            "Target {} not a valid entity ID",
            target
        );
    }
}

#[test]
fn virtual_hnsw_edges_with_bound_source() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let has_embedding = dict.intern("http://example.org/hasEmbedding");
    let _neighbor_id = dict.intern(sutra_hnsw::HNSW_NEIGHBOR_IRI);

    let mut vectors = VectorRegistry::new();
    vectors
        .declare(VectorPredicateConfig {
            predicate_id: has_embedding,
            dimensions: 3,
            m: 4,
            ef_construction: 20,
            metric: DistanceMetric::Cosine,
        })
        .unwrap();

    let vec1_id = dict.intern("\"vec1\"^^<http://sutra.dev/f32vec>");
    let vec2_id = dict.intern("\"vec2\"^^<http://sutra.dev/f32vec>");
    let vec3_id = dict.intern("\"vec3\"^^<http://sutra.dev/f32vec>");

    let doc1 = dict.intern("http://example.org/doc1");
    let doc2 = dict.intern("http://example.org/doc2");
    let doc3 = dict.intern("http://example.org/doc3");

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
        .insert(has_embedding, vec![0.0, 0.0, 1.0], vec3_id)
        .unwrap();

    // Query: entity → hnswNeighbor → neighbor entity
    // Virtual edges now connect entities directly (resolved from vector objects)
    let q = parse(&format!(
        "PREFIX ex: <http://example.org/> \
         SELECT ?doc ?neighbor WHERE {{ \
         ?doc <{}> ?neighbor \
         }}",
        sutra_hnsw::HNSW_NEIGHBOR_IRI
    ))
    .unwrap();

    let config = DatabaseConfig {
        hnsw_edge_mode: HnswEdgeMode::Virtual,
        ..Default::default()
    };
    let result = execute_with_config(&q, &store, &dict, &vectors, &config).unwrap();

    // Edges should connect entities (doc1, doc2, doc3), not vector literals
    assert!(
        !result.rows.is_empty(),
        "HNSW edge query should return entity-to-entity results"
    );
    let valid_entities = [doc1, doc2, doc3];
    for row in &result.rows {
        let doc = row.get("doc").unwrap();
        let neighbor = row.get("neighbor").unwrap();
        assert!(valid_entities.contains(doc), "doc should be an entity");
        assert!(
            valid_entities.contains(neighbor),
            "neighbor should be an entity"
        );
    }
}
