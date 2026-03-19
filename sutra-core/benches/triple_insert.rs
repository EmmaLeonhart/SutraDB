use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use sutra_core::{TermDictionary, Triple, TripleStore};

fn bench_single_insert(c: &mut Criterion) {
    c.bench_function("triple_insert_single", |b| {
        b.iter_batched(
            || {
                let mut dict = TermDictionary::new();
                let store = TripleStore::new();
                let s = dict.intern("http://example.org/subject");
                let p = dict.intern("http://example.org/predicate");
                let o = dict.intern("http://example.org/object");
                (store, s, p, o)
            },
            |(mut store, s, p, o)| {
                store.insert(black_box(Triple::new(s, p, o))).unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_bulk_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("triple_bulk_insert");
    for count in [100, 1_000, 10_000] {
        group.bench_with_input(
            criterion::BenchmarkId::new("triples", count),
            &count,
            |b, &count| {
                b.iter_batched(
                    || {
                        let mut dict = TermDictionary::new();
                        let p = dict.intern("http://example.org/knows");
                        let triples: Vec<_> = (0..count)
                            .map(|i| {
                                let s = dict.intern(&format!("http://example.org/person/{}", i));
                                let o = dict.intern(&format!(
                                    "http://example.org/person/{}",
                                    (i + 1) % count
                                ));
                                Triple::new(s, p, o)
                            })
                            .collect();
                        (TripleStore::new(), triples)
                    },
                    |(mut store, triples)| {
                        for t in triples {
                            store.insert(black_box(t)).unwrap();
                        }
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_lookup_by_subject(c: &mut Criterion) {
    let mut group = c.benchmark_group("triple_lookup_subject");
    for count in [1_000, 10_000] {
        group.bench_with_input(
            criterion::BenchmarkId::new("graph_size", count),
            &count,
            |b, &count| {
                let mut dict = TermDictionary::new();
                let mut store = TripleStore::new();
                let p = dict.intern("http://example.org/knows");
                let subjects: Vec<_> = (0..count)
                    .map(|i| {
                        let s = dict.intern(&format!("http://example.org/person/{}", i));
                        let o =
                            dict.intern(&format!("http://example.org/person/{}", (i + 1) % count));
                        store.insert(Triple::new(s, p, o)).unwrap();
                        s
                    })
                    .collect();
                b.iter(|| {
                    let results = store.find_by_subject(black_box(subjects[42]));
                    black_box(results);
                });
            },
        );
    }
    group.finish();
}

fn bench_lookup_by_predicate(c: &mut Criterion) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let knows = dict.intern("http://example.org/knows");
    let likes = dict.intern("http://example.org/likes");
    for i in 0..5_000 {
        let s = dict.intern(&format!("http://example.org/person/{}", i));
        let o = dict.intern(&format!("http://example.org/person/{}", (i + 1) % 5_000));
        let pred = if i % 2 == 0 { knows } else { likes };
        store.insert(Triple::new(s, pred, o)).unwrap();
    }

    c.bench_function("triple_lookup_predicate_5k", |b| {
        b.iter(|| {
            let results = store.find_by_predicate(black_box(knows));
            black_box(results);
        });
    });
}

fn bench_contains(c: &mut Criterion) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let p = dict.intern("http://example.org/knows");
    let mut triples = Vec::new();
    for i in 0..10_000 {
        let s = dict.intern(&format!("http://example.org/person/{}", i));
        let o = dict.intern(&format!("http://example.org/person/{}", (i + 1) % 10_000));
        let t = Triple::new(s, p, o);
        store.insert(t.clone()).unwrap();
        triples.push(t);
    }

    c.bench_function("triple_contains_10k", |b| {
        b.iter(|| {
            black_box(store.contains(black_box(&triples[5_000])));
        });
    });
}

fn bench_remove(c: &mut Criterion) {
    c.bench_function("triple_remove_single", |b| {
        b.iter_batched(
            || {
                let mut dict = TermDictionary::new();
                let mut store = TripleStore::new();
                let p = dict.intern("http://example.org/knows");
                let mut triples = Vec::new();
                for i in 0..1_000 {
                    let s = dict.intern(&format!("http://example.org/person/{}", i));
                    let o = dict.intern(&format!("http://example.org/person/{}", (i + 1) % 1_000));
                    let t = Triple::new(s, p, o);
                    store.insert(t.clone()).unwrap();
                    triples.push(t);
                }
                (store, triples[500].clone())
            },
            |(mut store, triple)| {
                store.remove(black_box(&triple));
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_adjacency(c: &mut Criterion) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let knows = dict.intern("http://example.org/knows");
    let likes = dict.intern("http://example.org/likes");
    let center = dict.intern("http://example.org/center");
    // Star graph: center connected to 500 neighbors via 2 predicates
    for i in 0..500 {
        let o = dict.intern(&format!("http://example.org/node/{}", i));
        let pred = if i % 2 == 0 { knows } else { likes };
        store.insert(Triple::new(center, pred, o)).unwrap();
    }

    c.bench_function("adjacency_star_500", |b| {
        b.iter(|| {
            let adj = store.adjacency(black_box(center));
            black_box(adj);
        });
    });
}

fn bench_intern(c: &mut Criterion) {
    c.bench_function("term_dictionary_intern_10k", |b| {
        b.iter_batched(
            || {
                let iris: Vec<String> = (0..10_000)
                    .map(|i| format!("http://example.org/entity/{}", i))
                    .collect();
                (TermDictionary::new(), iris)
            },
            |(mut dict, iris)| {
                for iri in &iris {
                    dict.intern(black_box(iri));
                }
            },
            BatchSize::SmallInput,
        );
    });
}

/// Pseudo-table discovery: find relational patterns in graph data.
/// This is analogous to scanning a SQL table — SutraDB auto-discovers
/// columnar structure from shared predicate patterns.
fn bench_pseudotable_discover(c: &mut Criterion) {
    let mut group = c.benchmark_group("pseudotable_discover");

    for &n in &[500, 2_000] {
        let mut dict = TermDictionary::new();
        let mut store = TripleStore::new();
        let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
        let name = dict.intern("http://example.org/name");
        let age = dict.intern("http://example.org/age");
        let email = dict.intern("http://example.org/email");
        let person = dict.intern("http://example.org/Person");

        for i in 0..n {
            let s = dict.intern(&format!("http://example.org/person/{}", i));
            store.insert(Triple::new(s, rdf_type, person)).unwrap();
            let name_val = dict.intern(&format!("\"Person {}\"", i));
            store.insert(Triple::new(s, name, name_val)).unwrap();
            let age_val = dict.intern(&format!(
                "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                20 + i % 60
            ));
            store.insert(Triple::new(s, age, age_val)).unwrap();
            let email_val = dict.intern(&format!("\"person{}@example.org\"", i));
            store.insert(Triple::new(s, email, email_val)).unwrap();
        }

        group.bench_with_input(criterion::BenchmarkId::new("persons", n), &(), |b, _| {
            b.iter(|| {
                let node_props = sutra_core::extract_node_properties(&store);
                let registry = sutra_core::discover_pseudo_tables(black_box(&node_props), &store);
                black_box(registry);
            });
        });
    }
    group.finish();
}

/// Scan a pseudo-table column — analogous to WHERE column = value in SQL.
/// Uses SIMD-accelerated bitset operations when available.
fn bench_pseudotable_scan(c: &mut Criterion) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let rdf_type = dict.intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let name = dict.intern("http://example.org/name");
    let category = dict.intern("http://example.org/category");
    let item = dict.intern("http://example.org/Item");

    let cats: Vec<_> = (0..10)
        .map(|c| dict.intern(&format!("http://example.org/cat/{}", c)))
        .collect();

    for i in 0..5_000 {
        let s = dict.intern(&format!("http://example.org/item/{}", i));
        store.insert(Triple::new(s, rdf_type, item)).unwrap();
        let name_val = dict.intern(&format!("\"Item {}\"", i));
        store.insert(Triple::new(s, name, name_val)).unwrap();
        store
            .insert(Triple::new(s, category, cats[i % 10]))
            .unwrap();
    }

    let node_props = sutra_core::extract_node_properties(&store);
    let registry = sutra_core::discover_pseudo_tables(&node_props, &store);

    c.bench_function("pseudotable_scan_eq_5k", |b| {
        b.iter(|| {
            if let Some(table) = registry.tables.first() {
                for seg in &table.segments {
                    for (col_idx, col_prop) in table.columns.iter().enumerate() {
                        if col_prop.predicate == category {
                            let results = sutra_core::pseudotable::scan_column_eq(
                                seg,
                                col_idx,
                                black_box(cats[3]),
                            );
                            black_box(results);
                        }
                    }
                }
            }
        });
    });
}

/// Lookup by object (reverse index) — equivalent to finding all rows
/// where a foreign key points to a specific value. Like SQL JOIN.
fn bench_reverse_lookup(c: &mut Criterion) {
    let mut dict = TermDictionary::new();
    let mut store = TripleStore::new();
    let mentions = dict.intern("http://example.org/mentions");
    let target = dict.intern("http://example.org/entity/42");

    for i in 0..10_000 {
        let s = dict.intern(&format!("http://example.org/doc/{}", i));
        let o = if i % 50 == 0 {
            target
        } else {
            dict.intern(&format!("http://example.org/entity/{}", i % 500))
        };
        store.insert(Triple::new(s, mentions, o)).unwrap();
    }

    c.bench_function("reverse_lookup_10k", |b| {
        b.iter(|| {
            let results = store.find_by_object(black_box(target));
            black_box(results);
        });
    });
}

criterion_group!(
    benches,
    bench_single_insert,
    bench_bulk_insert,
    bench_lookup_by_subject,
    bench_lookup_by_predicate,
    bench_contains,
    bench_remove,
    bench_adjacency,
    bench_intern,
    bench_pseudotable_discover,
    bench_pseudotable_scan,
    bench_reverse_lookup,
);
criterion_main!(benches);
