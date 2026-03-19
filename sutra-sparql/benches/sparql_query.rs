use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sutra_core::{TermDictionary, Triple, TripleStore};
use sutra_hnsw::{DistanceMetric, VectorPredicateConfig, VectorRegistry};
use sutra_sparql::{execute_with_vectors, parse};

/// Build a chain graph: node_0 -> node_1 -> ... -> node_{n-1}
fn chain_graph(length: usize) -> (TripleStore, TermDictionary) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let chain_node = dict.intern("http://example.org/ChainNode");
    let next = dict.intern("http://example.org/next");

    let mut node_ids = Vec::with_capacity(length);
    for i in 0..length {
        let node = dict.intern(&format!("http://example.org/node/{}", i));
        store
            .insert(Triple::new(node, rdf_type, chain_node))
            .unwrap();
        node_ids.push(node);
    }
    for i in 0..length - 1 {
        store
            .insert(Triple::new(node_ids[i], next, node_ids[i + 1]))
            .unwrap();
    }
    (store, dict)
}

/// Build a star graph: center -> N leaf nodes with types and categories
fn star_graph(leaves: usize, categories: usize) -> (TripleStore, TermDictionary) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let has_leaf = dict.intern("http://example.org/hasLeaf");
    let category_pred = dict.intern("http://example.org/category");
    let leaf_type = dict.intern("http://example.org/Leaf");
    let center = dict.intern("http://example.org/center");

    store
        .insert(Triple::new(
            center,
            rdf_type,
            dict.intern("http://example.org/Hub"),
        ))
        .unwrap();

    let cat_ids: Vec<_> = (0..categories)
        .map(|c| dict.intern(&format!("http://example.org/cat/{}", c)))
        .collect();

    for i in 0..leaves {
        let leaf = dict.intern(&format!("http://example.org/leaf/{}", i));
        store.insert(Triple::new(center, has_leaf, leaf)).unwrap();
        store
            .insert(Triple::new(leaf, rdf_type, leaf_type))
            .unwrap();
        store
            .insert(Triple::new(leaf, category_pred, cat_ids[i % categories]))
            .unwrap();
    }
    (store, dict)
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparql_parse");

    group.bench_function("simple_select", |b| {
        b.iter(|| {
            let q = parse(black_box(
                "PREFIX ex: <http://example.org/> SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10",
            ))
            .unwrap();
            black_box(q);
        });
    });

    group.bench_function("complex_with_filter", |b| {
        b.iter(|| {
            let q = parse(black_box(
                "PREFIX ex: <http://example.org/> \
                 SELECT ?person ?name WHERE { \
                 ?person a ex:Person . \
                 ?person ex:name ?name . \
                 ?person ex:age ?age . \
                 FILTER(?age > 25) \
                 } ORDER BY ?name LIMIT 50",
            ))
            .unwrap();
            black_box(q);
        });
    });

    group.bench_function("vector_similar", |b| {
        b.iter(|| {
            let q = parse(black_box(
                "PREFIX ex: <http://example.org/> \
                 SELECT ?doc WHERE { \
                 VECTOR_SIMILAR(?doc ex:hasEmbedding \
                 \"0.1 0.2 0.3 0.4\"^^<http://sutra.dev/f32vec>, 0.8) \
                 ?doc a ex:Document \
                 } LIMIT 10",
            ))
            .unwrap();
            black_box(q);
        });
    });

    group.finish();
}

fn bench_chain_traversal(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparql_chain_traversal");

    for &(chain_len, hops) in &[(500, 2), (1_000, 2), (500, 3), (200, 4)] {
        let (store, dict) = chain_graph(chain_len);
        let vectors = VectorRegistry::new();

        let vars: Vec<String> = (0..=hops).map(|i| format!("?v{}", i)).collect();
        let projections = vars.join(" ");
        let patterns: Vec<String> = (0..hops)
            .map(|i| format!("?v{} ex:next ?v{}", i, i + 1))
            .collect();
        let body = patterns.join(" . ");
        let sparql = format!(
            "PREFIX ex: <http://example.org/> SELECT {} WHERE {{ {} }} LIMIT 50",
            projections, body
        );

        group.bench_with_input(
            criterion::BenchmarkId::new(format!("{}_nodes", chain_len), format!("{}_hops", hops)),
            &sparql,
            |b, sparql| {
                let q = parse(sparql).unwrap();
                b.iter(|| {
                    let result =
                        execute_with_vectors(black_box(&q), &store, &dict, &vectors).unwrap();
                    black_box(result);
                });
            },
        );
    }
    group.finish();
}

fn bench_star_join(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparql_star_join");

    for &(leaves, cats) in &[(1_000, 10), (5_000, 20), (1_000, 5)] {
        let (store, dict) = star_graph(leaves, cats);
        let vectors = VectorRegistry::new();

        let sparql = "PREFIX ex: <http://example.org/> \
                      SELECT ?leaf ?cat WHERE { \
                      ex:center ex:hasLeaf ?leaf . \
                      ?leaf a ex:Leaf . \
                      ?leaf ex:category ?cat \
                      } LIMIT 100";

        group.bench_with_input(
            criterion::BenchmarkId::new(format!("{}_leaves", leaves), format!("{}_cats", cats)),
            &sparql,
            |b, sparql| {
                let q = parse(sparql).unwrap();
                b.iter(|| {
                    let result =
                        execute_with_vectors(black_box(&q), &store, &dict, &vectors).unwrap();
                    black_box(result);
                });
            },
        );
    }
    group.finish();
}

fn bench_vector_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparql_vector_search");

    for n in [100, 500, 1_000] {
        // Build graph with vectors
        let mut dict = TermDictionary::new();
        let mut store = TripleStore::new();
        let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let has_embedding = dict.intern("http://example.org/hasEmbedding");
        let doc_type = dict.intern("http://example.org/Document");

        let mut vectors = VectorRegistry::new();
        vectors
            .declare(VectorPredicateConfig {
                predicate_id: has_embedding,
                dimensions: 32,
                m: 16,
                ef_construction: 100,
                metric: DistanceMetric::Cosine,
            })
            .unwrap();

        for i in 0..n {
            let doc = dict.intern(&format!("http://example.org/doc/{}", i));
            let vec_id = dict.intern(&format!("\"vec_{}\"^^<http://sutra.dev/f32vec>", i));
            store.insert(Triple::new(doc, rdf_type, doc_type)).unwrap();
            store
                .insert(Triple::new(doc, has_embedding, vec_id))
                .unwrap();

            let v: Vec<f32> = (0..32)
                .map(|d| ((i * 7 + d * 3) % 100) as f32 / 100.0)
                .collect();
            vectors.insert(has_embedding, v, vec_id).unwrap();
        }

        let sparql = "PREFIX ex: <http://example.org/> \
                      SELECT ?doc WHERE { \
                      VECTOR_SIMILAR(?doc ex:hasEmbedding \
                      \"0.5 0.3 0.1 0.9 0.2 0.4 0.6 0.8 0.1 0.3 0.5 0.7 0.9 0.2 0.4 0.6 \
                       0.8 0.1 0.3 0.5 0.7 0.9 0.2 0.4 0.6 0.8 0.1 0.3 0.5 0.7 0.9 0.2\"\
                      ^^<http://sutra.dev/f32vec>, 0.5) \
                      ?doc a ex:Document \
                      } LIMIT 10";

        let q = parse(sparql).unwrap();

        group.bench_with_input(criterion::BenchmarkId::new("docs", n), &(), |b, _| {
            b.iter(|| {
                let result = execute_with_vectors(black_box(&q), &store, &dict, &vectors).unwrap();
                black_box(result);
            });
        });
    }
    group.finish();
}

/// Graph + vector combined: traverse the graph to find candidates,
/// then filter by vector similarity. This is the core SutraDB use case.
fn bench_graph_then_vector(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparql_graph_vector");

    for &n in &[200, 500] {
        let mut dict = TermDictionary::new();
        let mut store = TripleStore::new();
        let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let has_embedding = dict.intern("http://example.org/hasEmbedding");
        let cites = dict.intern("http://example.org/cites");
        let paper_type = dict.intern("http://example.org/Paper");

        let mut vectors = VectorRegistry::new();
        vectors
            .declare(VectorPredicateConfig {
                predicate_id: has_embedding,
                dimensions: 32,
                m: 16,
                ef_construction: 100,
                metric: DistanceMetric::Cosine,
            })
            .unwrap();

        // Build citation graph with embeddings
        for i in 0..n {
            let paper = dict.intern(&format!("http://example.org/paper/{}", i));
            let vec_id = dict.intern(&format!("\"pvec_{}\"^^<http://sutra.dev/f32vec>", i));
            store.insert(Triple::new(paper, rdf_type, paper_type)).unwrap();
            store.insert(Triple::new(paper, has_embedding, vec_id)).unwrap();
            // Citation chain
            if i > 0 {
                let prev = dict.intern(&format!("http://example.org/paper/{}", i - 1));
                store.insert(Triple::new(paper, cites, prev)).unwrap();
            }
            let v: Vec<f32> = (0..32)
                .map(|d| ((i * 7 + d * 3) % 100) as f32 / 100.0)
                .collect();
            vectors.insert(has_embedding, v, vec_id).unwrap();
        }

        // Query: find papers similar to a query that cite another paper
        let sparql = "PREFIX ex: <http://example.org/> \
                      SELECT ?paper WHERE { \
                      VECTOR_SIMILAR(?paper ex:hasEmbedding \
                      \"0.5 0.3 0.1 0.9 0.2 0.4 0.6 0.8 0.1 0.3 0.5 0.7 0.9 0.2 0.4 0.6 \
                       0.8 0.1 0.3 0.5 0.7 0.9 0.2 0.4 0.6 0.8 0.1 0.3 0.5 0.7 0.9 0.2\"\
                      ^^<http://sutra.dev/f32vec>, 0.3) \
                      ?paper a ex:Paper . \
                      ?paper ex:cites ?cited \
                      } LIMIT 10";

        let q = parse(sparql).unwrap();
        group.bench_with_input(
            criterion::BenchmarkId::new("cite_chain", n),
            &(),
            |b, _| {
                b.iter(|| {
                    let result =
                        execute_with_vectors(black_box(&q), &store, &dict, &vectors).unwrap();
                    black_box(result);
                });
            },
        );
    }
    group.finish();
}

/// OPTIONAL pattern: left outer join semantics.
/// Common in SPARQL for getting optional properties.
fn bench_optional(c: &mut Criterion) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let name = dict.intern("http://example.org/name");
    let email = dict.intern("http://example.org/email");
    let person = dict.intern("http://example.org/Person");

    for i in 0..1_000 {
        let s = dict.intern(&format!("http://example.org/person/{}", i));
        store.insert(Triple::new(s, rdf_type, person)).unwrap();
        let name_val = dict.intern(&format!("\"Person {}\"", i));
        store.insert(Triple::new(s, name, name_val)).unwrap();
        // Only half have emails
        if i % 2 == 0 {
            let email_val = dict.intern(&format!("\"p{}@example.org\"", i));
            store.insert(Triple::new(s, email, email_val)).unwrap();
        }
    }

    let vectors = VectorRegistry::new();
    let sparql = "PREFIX ex: <http://example.org/> \
                  SELECT ?person ?name ?email WHERE { \
                  ?person a ex:Person . \
                  ?person ex:name ?name . \
                  OPTIONAL { ?person ex:email ?email } \
                  } LIMIT 100";
    let q = parse(sparql).unwrap();

    c.bench_function("sparql_optional_1k", |b| {
        b.iter(|| {
            let result = execute_with_vectors(black_box(&q), &store, &dict, &vectors).unwrap();
            black_box(result);
        });
    });
}

/// FILTER with comparison — equivalent to SQL WHERE clause.
fn bench_filter(c: &mut Criterion) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let name = dict.intern("http://example.org/name");
    let score = dict.intern("http://example.org/score");
    let item = dict.intern("http://example.org/Item");

    for i in 0..2_000 {
        let s = dict.intern(&format!("http://example.org/item/{}", i));
        store.insert(Triple::new(s, rdf_type, item)).unwrap();
        let name_val = dict.intern(&format!("\"Item {}\"", i));
        store.insert(Triple::new(s, name, name_val)).unwrap();
        let score_val = sutra_core::inline_integer(i as i64 % 100).unwrap();
        store.insert(Triple::new(s, score, score_val)).unwrap();
    }

    let vectors = VectorRegistry::new();
    let sparql = "PREFIX ex: <http://example.org/> \
                  SELECT ?item ?name WHERE { \
                  ?item a ex:Item . \
                  ?item ex:name ?name . \
                  ?item ex:score ?s . \
                  FILTER(?s > 80) \
                  } LIMIT 50";
    let q = parse(sparql).unwrap();

    c.bench_function("sparql_filter_gt_2k", |b| {
        b.iter(|| {
            let result = execute_with_vectors(black_box(&q), &store, &dict, &vectors).unwrap();
            black_box(result);
        });
    });
}

criterion_group!(
    benches,
    bench_parse,
    bench_chain_traversal,
    bench_star_join,
    bench_vector_search,
    bench_graph_then_vector,
    bench_optional,
    bench_filter,
);
criterion_main!(benches);
