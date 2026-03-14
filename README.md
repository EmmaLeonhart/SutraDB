# SutraDB

A lean, high-performance RDF-star triplestore written in Rust with native HNSW vector indexing and a hybrid SPARQL extension.

[![CI](https://github.com/EmmaLeonhart/SutraDB/actions/workflows/ci.yml/badge.svg)](https://github.com/EmmaLeonhart/SutraDB/actions/workflows/ci.yml)

## What is this?

SutraDB is a single-purpose database: store triples, answer queries, at any scale. It replaces both a vector database (e.g. Qdrant) and a SPARQL triplestore (e.g. Apache Jena Fuseki) with a single unified system where **vectors are just triples**.

The vector indexing architecture is heavily influenced by [Qdrant](https://github.com/qdrant/qdrant), reimplemented from first principles and unified with a triple store. The RDF/SPARQL semantics draw from Apache Jena's TDB2, but without the JVM overhead.

### Core principles

1. **Store first, reason second.** The database stores what you put in. OWL reasoning is planned as an opt-in query-time layer; RDFS inference is out of scope.
2. **Vectors are triples.** A vector embedding is an attribute of a node or edge, stored via a typed predicate and indexed by HNSW — not a separate system.
3. **Full traversal in a single query.** Any traversal of any depth must be expressible in one SPARQL query.
4. **Lean by default.** Every feature must justify itself. Complexity is the enemy of performance.

## Data Model

All data is **RDF-star** triples. Vectors are stored as `sutra:f32vec` literals:

```turtle
# Embedding on a node
:paper_42 :hasEmbedding "0.23 -0.11 0.87 ..."^^sutra:f32vec .

# Embedding on an edge (RDF-star)
<< :paper_42 :discusses :TransformerArchitecture >> :hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec .
<< :paper_42 :discusses :TransformerArchitecture >> :confidence 0.91 .
```

### Inline Literals

Small typed values (integers, booleans) are encoded directly into the 64-bit term ID, avoiding dictionary lookups entirely. This is inspired by [Jena TDB2's inline literal optimization](https://github.com/apache/jena/tree/main/jena-tdb2):

- Bit 63 = 1 marks an inline value
- Bits 62-56 = type tag (integer, boolean, etc.)
- Bits 55-0 = payload (56 bits)

This means numeric filters and comparisons operate on raw IDs without touching the dictionary — critical for SPARQL query performance.

## Hybrid SPARQL

SutraDB extends SPARQL with `VECTOR_SIMILAR` for unified graph + vector queries:

```sparql
SELECT ?doc ?entity WHERE {
  ?entity rdf:type :Person .
  ?doc :mentions ?entity .
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)
}
```

The query planner decides execution order based on binding state:
- Subject **bound** before VECTOR_SIMILAR → graph first, then vector filter
- Subject **unbound** → vector search first (top-k), then graph patterns

## Architecture

### Crate Structure

| Crate | Purpose | Status |
|---|---|---|
| `sutra-core` | Triple storage engine, IRI interning, RDF-star IDs | Implemented |
| `sutra-hnsw` | HNSW vector index with multiple distance metrics | Implemented |
| `sutra-sparql` | SPARQL 1.1 parser, query planner, executor | Stub |
| `sutra-proto` | SPARQL HTTP protocol, Graph Store Protocol, REST API | Stub |
| `sutra-cli` | CLI tools: import, export, query, benchmark | Stub |

**Dependency rules:**
- `sutra-hnsw` has **zero** dependency on `sutra-sparql` — it is a pure data structure crate
- `sutra-sparql` depends on both `sutra-core` and `sutra-hnsw`
- `sutra-proto` depends on `sutra-sparql`
- `sutra-cli` depends on `sutra-proto` and `sutra-sparql`

### Storage Engine (`sutra-core`)

Six covering indexes over integer-interned term IDs:

| Index | Purpose |
|---|---|
| SPO | Subject-first traversal (primary store) |
| POS | Predicate-first, range queries |
| OSP | Object-first, reverse traversal |
| SP | Star-shaped queries |
| PO | Type lookups, range scans |
| VECTOR(p) | One HNSW per vector predicate |

Currently backed by `BTreeSet` for v0.1. Will be replaced by an LSM-tree for persistence and write throughput.

**IRI interning:** All IRIs/blank nodes are interned to `u64` at write time. Quoted triples (RDF-star) get a content-addressed ID via xxHash3 of their (S, P, O) tuple.

### HNSW Index (`sutra-hnsw`)

The HNSW (Hierarchical Navigable Small World) implementation follows patterns from Qdrant:

- **Normalize-then-dot-product** for cosine similarity (normalize at insert time, dot product at search time — avoids redundant normalization on the hot path)
- **Three distance metrics:** Cosine, Euclidean, DotProduct
- **Reusable visited list** to avoid per-search allocation
- **O(1) deletion** via HashMap lookup (triple_id → node index)
- **Lazy deletion** with tombstone flags — deleted nodes are skipped during search
- **`deleted_ratio()`** for compaction threshold decisions
- **Seeded RNG** (xorshift64) for reproducible layer assignment in tests

**Key parameters:**
- `M`: max connections per node per layer (default 16)
- `M0 = 2*M`: max connections at layer 0
- `ef_construction`: beam width during build
- `ef_search`: beam width at query time (tunable per-query)
- `dimensions`: fixed at predicate declaration, enforced on insert

## Building

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Test Suite

66 tests across unit and integration tests:

- **sutra-core** (29 tests): TermDictionary, Triple key encoding, TripleStore CRUD, inline literals, RDF-star quoted triples, bulk operations
- **sutra-hnsw** (37 tests): HNSW insert/search/delete, distance metrics, search quality, normalization, high-dimensional vectors, reproducibility, stress tests

## License

Apache 2.0
