use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use sutra_hnsw::{DistanceMetric, HnswConfig, HnswIndex};

fn random_vector(dims: usize, seed: u64) -> Vec<f32> {
    // Simple deterministic pseudo-random using the seed
    let mut v = Vec::with_capacity(dims);
    let mut state = seed;
    for _ in 0..dims {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        v.push(((state >> 33) as f32) / (u32::MAX as f32) * 2.0 - 1.0);
    }
    v
}

fn build_index(n: usize, dims: usize, metric: DistanceMetric) -> HnswIndex {
    let config = HnswConfig {
        dimensions: dims,
        m: 16,
        m0: 32,
        ef_construction: 100,
        metric,
    };
    let mut index = HnswIndex::with_seed(config, 42);
    for i in 0..n {
        index
            .insert(random_vector(dims, i as u64), i as u64)
            .unwrap();
    }
    index
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_insert");
    for &(n, dims) in &[(100, 128), (1_000, 128), (5_000, 128), (1_000, 384)] {
        group.bench_with_input(
            criterion::BenchmarkId::new(format!("{}d", dims), n),
            &(n, dims),
            |b, &(n, dims)| {
                b.iter_batched(
                    || {
                        let config = HnswConfig {
                            dimensions: dims,
                            m: 16,
                            m0: 32,
                            ef_construction: 100,
                            metric: DistanceMetric::Cosine,
                        };
                        let index = HnswIndex::with_seed(config, 42);
                        let vectors: Vec<_> = (0..n)
                            .map(|i| (random_vector(dims, i as u64), i as u64))
                            .collect();
                        (index, vectors)
                    },
                    |(mut index, vectors)| {
                        for (v, id) in vectors {
                            index.insert(black_box(v), black_box(id)).unwrap();
                        }
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_search");
    for &(n, dims, ef) in &[
        (1_000, 128, 50),
        (1_000, 128, 100),
        (1_000, 128, 200),
        (5_000, 128, 100),
        (1_000, 384, 100),
    ] {
        let index = build_index(n, dims, DistanceMetric::Cosine);
        let query = random_vector(dims, 99999);
        group.bench_with_input(
            criterion::BenchmarkId::new(format!("n{}_{}d_ef{}", n, dims, ef), "k10"),
            &(ef,),
            |b, &(ef,)| {
                b.iter(|| {
                    let results = index.search(black_box(&query), 10, ef).unwrap();
                    black_box(results);
                });
            },
        );
    }
    group.finish();
}

fn bench_search_varying_k(c: &mut Criterion) {
    let index = build_index(5_000, 128, DistanceMetric::Cosine);
    let query = random_vector(128, 99999);

    let mut group = c.benchmark_group("hnsw_search_k");
    for k in [1, 5, 10, 25, 50, 100] {
        group.bench_with_input(criterion::BenchmarkId::new("5k_128d", k), &k, |b, &k| {
            b.iter(|| {
                let results = index.search(black_box(&query), k, 100).unwrap();
                black_box(results);
            });
        });
    }
    group.finish();
}

fn bench_delete_and_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_delete_then_search");
    for delete_pct in [10, 25, 50] {
        group.bench_with_input(
            criterion::BenchmarkId::new("1k_128d", format!("{}pct_deleted", delete_pct)),
            &delete_pct,
            |b, &delete_pct| {
                b.iter_batched(
                    || {
                        let mut index = build_index(1_000, 128, DistanceMetric::Cosine);
                        let to_delete = 1_000 * delete_pct / 100;
                        for i in 0..to_delete {
                            index.delete(i as u64);
                        }
                        (index, random_vector(128, 99999))
                    },
                    |(index, query)| {
                        let results = index.search(black_box(&query), 10, 100).unwrap();
                        black_box(results);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_bulk_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_bulk_insert");
    for n in [100, 500, 1_000] {
        group.bench_with_input(criterion::BenchmarkId::new("128d", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    let config = HnswConfig {
                        dimensions: 128,
                        m: 16,
                        m0: 32,
                        ef_construction: 100,
                        metric: DistanceMetric::Cosine,
                    };
                    let index = HnswIndex::with_seed(config, 42);
                    let vectors: Vec<_> = (0..n)
                        .map(|i| (random_vector(128, i as u64), i as u64))
                        .collect();
                    (index, vectors)
                },
                |(mut index, vectors)| {
                    index.bulk_insert(black_box(vectors)).unwrap();
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_distance_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_metrics");
    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
    ] {
        let index = build_index(1_000, 128, metric);
        let query = random_vector(128, 99999);
        group.bench_with_input(
            criterion::BenchmarkId::new("1k_128d", format!("{:?}", metric)),
            &(),
            |b, _| {
                b.iter(|| {
                    let results = index.search(black_box(&query), 10, 100).unwrap();
                    black_box(results);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_insert,
    bench_search,
    bench_search_varying_k,
    bench_delete_and_search,
    bench_bulk_insert,
    bench_distance_metrics,
);
criterion_main!(benches);
