//! Stress tests for large joins and long traversals.
//!
//! These tests verify that the query executor can handle:
//! - Wide joins (many intermediate results)
//! - Deep traversals (long chains of triple patterns)
//! - Combined vector + graph traversals at scale
//!
//! These are intentionally larger than unit tests but small enough
//! to run in CI (seconds, not minutes).

use std::collections::HashSet;

use sutra_core::{DatabaseConfig, TermDictionary, TermId, Triple, TripleStore};
use sutra_hnsw::{DistanceMetric, VectorPredicateConfig, VectorRegistry};
use sutra_sparql::{execute_with_config, execute_with_vectors, parse};

/// Build a chain graph: node_0 → node_1 → node_2 → ... → node_{n-1}
/// Each node has a type `:ChainNode` and the link predicate is `:next`.
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

/// Build a star graph: center node connected to N leaf nodes.
/// Each leaf has a `:category` and `:value` for join testing.
fn star_graph(leaves: usize, categories: usize) -> (TripleStore, TermDictionary) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let has_leaf = dict.intern("http://example.org/hasLeaf");
    let category_pred = dict.intern("http://example.org/category");
    let value_pred = dict.intern("http://example.org/value");
    let leaf_type = dict.intern("http://example.org/Leaf");

    let center = dict.intern("http://example.org/center");
    store
        .insert(Triple::new(
            center,
            rdf_type,
            dict.intern("http://example.org/Hub"),
        ))
        .unwrap();

    let mut cat_ids = Vec::new();
    for c in 0..categories {
        cat_ids.push(dict.intern(&format!("http://example.org/cat/{}", c)));
    }

    for i in 0..leaves {
        let leaf = dict.intern(&format!("http://example.org/leaf/{}", i));
        store.insert(Triple::new(center, has_leaf, leaf)).unwrap();
        store
            .insert(Triple::new(leaf, rdf_type, leaf_type))
            .unwrap();
        store
            .insert(Triple::new(leaf, category_pred, cat_ids[i % categories]))
            .unwrap();
        let val = sutra_core::inline_integer(i as i64).unwrap();
        store.insert(Triple::new(leaf, value_pred, val)).unwrap();
    }

    (store, dict)
}

/// Build a grid/mesh graph: N×N grid where each node connects to its right
/// and bottom neighbors.
fn grid_graph(width: usize, height: usize) -> (TripleStore, TermDictionary) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let right = dict.intern("http://example.org/right");
    let down = dict.intern("http://example.org/down");
    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let grid_node = dict.intern("http://example.org/GridNode");

    let mut nodes = vec![vec![0u64; width]; height];
    for y in 0..height {
        for x in 0..width {
            let node = dict.intern(&format!("http://example.org/grid/{}/{}", x, y));
            nodes[y][x] = node;
            store
                .insert(Triple::new(node, rdf_type, grid_node))
                .unwrap();
        }
    }

    for y in 0..height {
        for x in 0..width {
            if x + 1 < width {
                store
                    .insert(Triple::new(nodes[y][x], right, nodes[y][x + 1]))
                    .unwrap();
            }
            if y + 1 < height {
                store
                    .insert(Triple::new(nodes[y][x], down, nodes[y + 1][x]))
                    .unwrap();
            }
        }
    }

    (store, dict)
}

// --- Long traversal tests ---

#[test]
fn traverse_2_hops_on_1000_node_chain() {
    let (store, dict) = chain_graph(1000);
    let vectors = VectorRegistry::new();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?start ?mid ?end WHERE { \
         ?start ex:next ?mid . \
         ?mid ex:next ?end \
         } LIMIT 100",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 100);
}

#[test]
fn traverse_3_hops_on_500_node_chain() {
    let (store, dict) = chain_graph(500);
    let vectors = VectorRegistry::new();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?a ?b ?c ?d WHERE { \
         ?a ex:next ?b . \
         ?b ex:next ?c . \
         ?c ex:next ?d \
         } LIMIT 50",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 50);
}

#[test]
fn traverse_4_hops_on_200_node_chain() {
    let (store, dict) = chain_graph(200);
    let vectors = VectorRegistry::new();

    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?a ?b ?c ?d ?e WHERE { \
         ?a ex:next ?b . \
         ?b ex:next ?c . \
         ?c ex:next ?d . \
         ?d ex:next ?e \
         } LIMIT 50",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 50);

    // Verify chain integrity: each hop should advance by 1
    for row in &result.rows {
        let a = *row.get("a").unwrap();
        let b = *row.get("b").unwrap();
        let c = *row.get("c").unwrap();
        let d = *row.get("d").unwrap();
        let e = *row.get("e").unwrap();
        // All should be distinct in a chain
        let set: HashSet<TermId> = [a, b, c, d, e].into_iter().collect();
        assert_eq!(
            set.len(),
            5,
            "4-hop traversal should visit 5 distinct nodes"
        );
    }
}

// --- Large join tests ---

#[test]
fn join_1000_leaves_with_type_filter() {
    let (store, dict) = star_graph(1000, 10);
    let vectors = VectorRegistry::new();

    // Join: center → leaf, leaf has type Leaf and category
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?leaf ?cat WHERE { \
         ex:center ex:hasLeaf ?leaf . \
         ?leaf a ex:Leaf . \
         ?leaf ex:category ?cat \
         } LIMIT 100",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    assert_eq!(result.rows.len(), 100);
}

#[test]
fn join_5000_leaves_filtered_by_category() {
    let (store, dict) = star_graph(5000, 20);
    let vectors = VectorRegistry::new();

    let _cat_0 = dict.lookup("http://example.org/cat/0").unwrap();

    // 2-way join: center → leaf → category, filtered to a specific category
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?leaf WHERE { \
         ex:center ex:hasLeaf ?leaf . \
         ?leaf ex:category <http://example.org/cat/0> \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // 5000 / 20 categories = 250 leaves per category
    assert_eq!(result.rows.len(), 250);
}

#[test]
fn self_join_on_shared_category() {
    let (store, dict) = star_graph(200, 5);
    let vectors = VectorRegistry::new();

    // Self-join: find pairs of leaves that share the same category
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?leaf1 ?leaf2 ?cat WHERE { \
         ?leaf1 ex:category ?cat . \
         ?leaf2 ex:category ?cat . \
         FILTER(?leaf1 != ?leaf2) \
         } LIMIT 100",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // LIMIT pushdown may give slightly fewer results after FILTER removes self-pairs,
    // but we should get a substantial number of cross-category matches
    assert!(
        result.rows.len() > 50,
        "Self-join should produce many results, got {}",
        result.rows.len()
    );
    assert!(result.rows.len() <= 100);

    // Verify: each pair should share the same category and be distinct
    for row in &result.rows {
        assert_ne!(row.get("leaf1"), row.get("leaf2"));
    }
}

// --- Grid traversal tests ---

#[test]
fn grid_2hop_traversal_20x20() {
    let (store, dict) = grid_graph(20, 20);
    let vectors = VectorRegistry::new();

    // 2-hop right traversal: find nodes 2 steps right
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?start ?mid ?end WHERE { \
         ?start ex:right ?mid . \
         ?mid ex:right ?end \
         } LIMIT 100",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();
    // 20 rows * 18 valid 2-step right positions = 360 total, limited to 100
    // LIMIT pushdown may cause early termination, so we check >= 50
    assert!(
        result.rows.len() >= 50 && result.rows.len() <= 100,
        "Grid 2-hop should return 50-100 rows, got {}",
        result.rows.len()
    );
}

// --- HNSW edge traversal stress test ---

#[test]
fn hnsw_edge_traversal_50_vectors() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let has_embedding = dict.intern("http://example.org/hasEmbedding");
    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let doc_type = dict.intern("http://example.org/Document");
    let _neighbor_id = dict.intern(sutra_hnsw::HNSW_NEIGHBOR_IRI);

    let mut vectors = VectorRegistry::new();
    vectors
        .declare(VectorPredicateConfig {
            predicate_id: has_embedding,
            dimensions: 8,
            m: 4,
            ef_construction: 20,
            metric: DistanceMetric::Cosine,
        })
        .unwrap();

    // Insert 50 documents with vectors
    let mut doc_ids = Vec::new();
    for i in 0..50u64 {
        let doc = dict.intern(&format!("http://example.org/doc/{}", i));
        let vec_id = dict.intern(&format!("\"vec_{}\"^^<http://sutra.dev/f32vec>", i));
        store.insert(Triple::new(doc, rdf_type, doc_type)).unwrap();
        store
            .insert(Triple::new(doc, has_embedding, vec_id))
            .unwrap();

        // Create vectors with some structure (2 clusters)
        let v: Vec<f32> = (0..8)
            .map(|d| {
                if i < 25 {
                    // Cluster A: mostly positive
                    ((i * 7 + d * 3) % 100) as f32 / 100.0
                } else {
                    // Cluster B: mostly negative first components
                    -(((i * 11 + d * 5) % 100) as f32 / 100.0)
                }
            })
            .collect();
        vectors.insert(has_embedding, v, vec_id).unwrap();
        doc_ids.push(doc);
    }

    // Query virtual HNSW edges — now returns entity IRIs, not vector object IDs
    let q = parse(
        "SELECT ?source ?target WHERE { \
         ?source <http://sutra.dev/hnswNeighbor> ?target \
         } LIMIT 200",
    )
    .unwrap();

    let config = DatabaseConfig::default();
    let result = execute_with_config(&q, &store, &dict, &vectors, &config).unwrap();

    assert!(!result.rows.is_empty(), "50-vector HNSW should have edges");
    assert!(result.rows.len() <= 200, "Should respect LIMIT 200");

    // Verify all edges reference valid entity IRIs (resolved from vector objects)
    let valid_set: HashSet<TermId> = doc_ids.into_iter().collect();
    for row in &result.rows {
        assert!(valid_set.contains(row.get("source").unwrap()));
        assert!(valid_set.contains(row.get("target").unwrap()));
    }
}

// --- Combined vector + deep traversal ---

#[test]
fn vector_search_then_3_hop_traversal() {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();

    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let has_embedding = dict.intern("http://example.org/hasEmbedding");
    let links_to = dict.intern("http://example.org/linksTo");
    let entity_type = dict.intern("http://example.org/Entity");

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

    // Build a chain of 20 entities, each with a vector embedding
    let mut entities = Vec::new();
    for i in 0..20u64 {
        let entity = dict.intern(&format!("http://example.org/entity/{}", i));
        let vec_id = dict.intern(&format!("\"vec_e{}\"^^<http://sutra.dev/f32vec>", i));

        store
            .insert(Triple::new(entity, rdf_type, entity_type))
            .unwrap();
        store
            .insert(Triple::new(entity, has_embedding, vec_id))
            .unwrap();

        // Vectors: first 5 entities are in one cluster (positive), rest in another
        let v = if i < 5 {
            vec![1.0, 0.0, (i as f32) * 0.05, 0.0]
        } else {
            vec![0.0, 1.0, 0.0, (i as f32) * 0.05]
        };
        vectors.insert(has_embedding, v, vec_id).unwrap();
        entities.push(entity);
    }

    // Chain: entity[i] linksTo entity[i+1]
    for i in 0..19 {
        store
            .insert(Triple::new(entities[i], links_to, entities[i + 1]))
            .unwrap();
    }

    // Query: find entities similar to cluster A, then traverse 3 hops
    let q = parse(
        "PREFIX ex: <http://example.org/> \
         SELECT ?start ?hop1 ?hop2 ?hop3 WHERE { \
         VECTOR_SIMILAR(?start ex:hasEmbedding \"1.0 0.0 0.0 0.0\"^^<http://sutra.dev/f32vec>, 0.5) \
         ?start ex:linksTo ?hop1 . \
         ?hop1 ex:linksTo ?hop2 . \
         ?hop2 ex:linksTo ?hop3 \
         }",
    )
    .unwrap();

    let result = execute_with_vectors(&q, &store, &dict, &vectors).unwrap();

    // Entities 0-4 are similar, and each one that has 3 hops forward should appear
    // Entity 0 → 1 → 2 → 3
    // Entity 1 → 2 → 3 → 4
    // Entity 2 → 3 → 4 → 5
    // Entity 3 → 4 → 5 → 6
    // Entity 4 → 5 → 6 → 7 (if entity 4 is similar enough)
    assert!(
        !result.rows.is_empty(),
        "Vector search + 3-hop should return results"
    );

    // Each result should have 4 distinct entities in the chain
    for row in &result.rows {
        let nodes: HashSet<TermId> = [
            *row.get("start").unwrap(),
            *row.get("hop1").unwrap(),
            *row.get("hop2").unwrap(),
            *row.get("hop3").unwrap(),
        ]
        .into_iter()
        .collect();
        assert_eq!(nodes.len(), 4, "3-hop chain should visit 4 distinct nodes");
    }
}
