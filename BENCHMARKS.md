# SutraDB Benchmark Results

Run on Windows 11, Rust 2021 edition, `--release` profile (opt-level 3, thin LTO, codegen-units 1).

Date: 2026-03-16

## sutra-core — Triple Storage Engine

| Benchmark | Size | Time |
|---|---|---|
| Single triple insert | 1 | **238 ns** |
| Bulk insert | 100 triples | **24.1 us** |
| Bulk insert | 1,000 triples | **629 us** |
| Bulk insert | 10,000 triples | **7.13 ms** |
| Lookup by subject | 1K graph | **135 ns** |
| Lookup by subject | 10K graph | **234 ns** |
| Lookup by predicate | 5K graph | **13.0 us** |
| Contains check | 10K graph | **57.6 ns** |
| Remove single triple | 1K graph | **70.5 us** |
| Adjacency (star, 500 edges) | 500 neighbors | **18.3 ns** |
| Term dictionary intern | 10K IRIs | **4.15 ms** |

**Throughput highlights:**
- Insert: ~1.4M triples/sec (bulk 10K)
- Subject lookup: ~4.3M lookups/sec (10K graph)
- Contains: ~17.4M checks/sec
- Adjacency: ~54.6M lookups/sec (O(1) star query)

## sutra-hnsw — HNSW Vector Index

### Insert Performance

| Vectors | Dimensions | Time |
|---|---|---|
| 100 | 128 | **3.28 ms** |
| 1,000 | 128 | **77.7 ms** |
| 5,000 | 128 | **1.15 s** |
| 1,000 | 384 | **247 ms** |

### Search Performance (k=10)

| Index Size | Dimensions | ef_search | Time |
|---|---|---|---|
| 1,000 | 128 | 50 | **73.3 us** |
| 1,000 | 128 | 100 | **109 us** |
| 1,000 | 128 | 200 | **160 us** |
| 5,000 | 128 | 100 | **267 us** |
| 1,000 | 384 | 100 | **206 us** |

### Search by k (5K vectors, 128D, ef=100)

| k | Time |
|---|---|
| 1 | **265 us** |
| 5 | **122 us** |
| 10 | **145 us** |
| 25 | **121 us** |
| 50 | **114 us** |
| 100 | **111 us** |

### Search After Deletions (1K vectors, 128D)

| Deleted % | Time |
|---|---|
| 10% | **145 us** |
| 25% | **144 us** |
| 50% | **130 us** |

### Bulk Insert

| Vectors | Dimensions | Time |
|---|---|---|
| 100 | 128 | **2.99 ms** |
| 500 | 128 | **28.3 ms** |
| 1,000 | 128 | **71.0 ms** |

### Distance Metrics (1K vectors, 128D, k=10)

| Metric | Time |
|---|---|
| Cosine | **55.0 us** |
| Euclidean | **57.5 us** |
| Dot Product | **34.6 us** |

**Throughput highlights:**
- Search (1K, 128D, ef=100): ~9,200 queries/sec
- Search (5K, 128D, ef=100): ~3,750 queries/sec
- Dot product is ~37% faster than cosine (no normalization overhead)

## sutra-sparql — SPARQL Query Engine

### Parse Performance

| Query Type | Time |
|---|---|
| Simple SELECT | **638 ns** |
| Complex (FILTER + ORDER BY) | **1.30 us** |
| VECTOR_SIMILAR | **920 ns** |

### Chain Traversal (LIMIT 50)

| Chain Length | Hops | Time |
|---|---|---|
| 500 nodes | 2 | **52.5 us** |
| 1,000 nodes | 2 | **52.7 us** |
| 500 nodes | 3 | **196 us** |
| 200 nodes | 4 | **214 us** |

### Star Join (LIMIT 100)

| Leaves | Categories | Time |
|---|---|---|
| 1,000 | 10 | **247 us** |
| 5,000 | 20 | **275 us** |
| 1,000 | 5 | **235 us** |

### Hybrid Vector+Graph Search (LIMIT 10)

| Documents | Dimensions | Time |
|---|---|---|
| 100 | 32 | **196 us** |
| 500 | 32 | **2.73 ms** |
| 1,000 | 32 | **1.36 ms** |

**Throughput highlights:**
- Parse: ~1.6M simple queries/sec
- 2-hop traversal is constant-time regardless of chain length (LIMIT pushdown)
- Star join on 5K leaves: ~3,600 queries/sec
- Vector+graph hybrid on 1K docs: ~735 queries/sec

---

*Benchmarks run with Criterion 0.5, 100 samples per measurement. All times are median values.*
