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

fn bench_high_dimensional(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_high_dim");
    for &dims in &[768, 1536] {
        let index = build_index(500, dims, DistanceMetric::Cosine);
        let query = random_vector(dims, 99999);
        group.bench_with_input(
            criterion::BenchmarkId::new(format!("search_500n_{}d", dims), "k10"),
            &(),
            |b, _| {
                b.iter(|| {
                    let results = index.search(black_box(&query), 10, 100).unwrap();
                    black_box(results);
                });
            },
        );
    }
    // Insert at high dimensions
    for &dims in &[768, 1536] {
        group.bench_with_input(
            criterion::BenchmarkId::new(format!("insert_100n_{}d", dims), ""),
            &dims,
            |b, &dims| {
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
                        let vectors: Vec<_> = (0..100)
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

fn bench_recall(c: &mut Criterion) {
    // Measures recall@10: what fraction of true top-10 nearest neighbors
    // does HNSW find? Not a latency benchmark — measures quality.
    let n = 1_000;
    let dims = 128;
    let index = build_index(n, dims, DistanceMetric::Cosine);
    let query = random_vector(dims, 99999);

    // Brute-force ground truth
    let mut distances: Vec<(u64, f32)> = (0..n as u64)
        .map(|i| {
            let v = random_vector(dims, i);
            let dot: f32 = query.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
            let norm_q: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
            let norm_v: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            let cosine_sim = if norm_q * norm_v > 0.0 { dot / (norm_q * norm_v) } else { 0.0 };
            (i, cosine_sim)
        })
        .collect();
    distances.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let ground_truth: std::collections::HashSet<u64> =
        distances.iter().take(10).map(|(id, _)| *id).collect();

    c.bench_function("hnsw_recall_at_10/1k_128d", |b| {
        b.iter(|| {
            let results = index.search(black_box(&query), 10, 100).unwrap();
            let found: std::collections::HashSet<u64> =
                results.iter().map(|r| r.triple_id).collect();
            let recall = found.intersection(&ground_truth).count() as f32 / 10.0;
            black_box(recall);
        });
    });
}

criterion_group!(
    benches,
    bench_insert,
    bench_search,
    bench_search_varying_k,
    bench_delete_and_search,
    bench_bulk_insert,
    bench_distance_metrics,
    bench_high_dimensional,
    bench_recall,
);
criterion_main!(benches);
