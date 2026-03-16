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
                        let o = dict.intern(&format!(
                            "http://example.org/person/{}",
                            (i + 1) % count
                        ));
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
                    let o = dict.intern(&format!(
                        "http://example.org/person/{}",
                        (i + 1) % 1_000
                    ));
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
);
criterion_main!(benches);
