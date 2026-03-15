# SutraDB — Claude Code Context

## What This Is

SutraDB is a lean, high-performance RDF-star triplestore written in Rust with native HNSW vector indexing and a hybrid SPARQL extension. It is a single-purpose database: store triples, answer queries, at any scale.

It is **not** a combination of existing databases. It replaces both a vector database (e.g. Qdrant) and a SPARQL triplestore (e.g. Apache Jena Fuseki) with a single unified system where vectors are just triples.

Full architecture: see `docs/architecture.md`.

---

## Workflow Rules

- **Commit early and often.** Every meaningful change gets a commit with a clear message explaining *why*, not just what.
- **Do not enter planning-only modes.** All thinking must produce files and commits. If scope is unclear, create a `planning/` directory and write `.md` files there instead of using an internal planning mode.
- **Keep this file up to date.** As the project takes shape, record architectural decisions, conventions, and anything needed to work effectively in this repo.
- **Update README.md regularly.** It should always reflect the current state of the project for human readers.

---

## Core Philosophy — Read This First

These are non-negotiable. Do not add features that violate them.

1. **Store first, reason second.** The database stores what you put in. OWL support is planned as a query-time layer — the store itself remains a faithful mirror of reality. RDFS inference is out of scope; OWL reasoning will be opt-in and never modify stored data.

2. **Vectors are triples.** A vector embedding is an attribute of a node or edge, stored via a predicate typed `sutra:f32vec`. It is indexed by HNSW, but it is not a separate system — it is just another index alongside SPO/POS/OSP.

3. **Full traversal in a single query.** Any traversal of any depth across the entire database must be expressible in one SPARQL query. This is the whole point of a graph database.

4. **Lean by default.** Every feature must justify itself. Complexity is the enemy of performance. When in doubt, push it to the application layer.

5. **Serverless by default, server when needed.** Like SQLite, SutraDB can be embedded directly — just open a `.sdb` file. No daemon, no config. Server mode (HTTP/SPARQL endpoint) is opt-in via `sutra serve`. Same `.sdb` storage format either way.

---

## Crate Structure

```
sutra-core/      # Triple storage engine, LSM indexes, IRI interning, RDF-star IDs
sutra-hnsw/      # HNSW index, vector literal type, predicate index registry
sutra-sparql/    # SPARQL 1.1 parser, query planner, executor, hybrid extension
sutra-proto/     # SPARQL HTTP protocol, Graph Store Protocol, REST API
sutra-cli/       # CLI tools: import, export, query, benchmark
```

**Dependency rules:**
- `sutra-hnsw` has **zero** dependency on `sutra-sparql`. It is a pure data structure crate.
- `sutra-sparql` depends on both `sutra-core` and `sutra-hnsw`.
- `sutra-proto` depends on `sutra-sparql`.
- `sutra-cli` depends on `sutra-proto` and `sutra-sparql`.

---

## Data Model

### RDF-star
All data is RDF-star triples. Any position (subject, predicate, object) can be a quoted triple. This gives embeddings and metadata on edges natively.

```turtle
# Embedding on a node
:paper_42 :hasEmbedding "0.23 -0.11 0.87 ..."^^sutra:f32vec .

# Embedding on an edge (RDF-star)
<< :paper_42 :discusses :TransformerArchitecture >> :hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec .
<< :paper_42 :discusses :TransformerArchitecture >> :confidence 0.91 .
```

### Vector Literals
- Type: `sutra:f32vec` — a fixed-dimension array of f32
- Dimensionality is declared per predicate at schema time and enforced on insert
- Mismatched dimensions = hard error
- The database is model-agnostic: raw floats only, no embedding model metadata

### Schema Declaration
```turtle
sutra:declareVectorPredicate :hasEmbedding ;
    sutra:dimensions 1536 ;
    sutra:hnswM 16 ;
    sutra:hnswEfConstruction 200 .
```

---

## Storage Engine

### Indexes
Four index types over integer-interned IRI IDs:

| Index | Purpose |
|---|---|
| SPO | Subject → Predicate → Object (primary store, star-shaped queries via prefix scan) |
| POS | Predicate → Object → Subject (type lookups, vector reverse resolution) |
| OSP | Object → Subject → Predicate (reverse traversal) |
| VECTOR(p) | One HNSW index per vector predicate (ANN search, keyed by vector object ID) |

No separate SP or PO indexes needed — they are prefix scans on SPO and POS respectively.

### Implementation Notes
- Underlying storage: LSM-tree (RocksDB or sled TBD — see open questions)
- IRIs and blank nodes interned to u64 at write time
- Quoted triples get a content-addressed u64 ID: hash(S, P, O)
- All index entries operate on u64 IDs, never strings

---

## HNSW Index

### Parameters
- `M`: max connections per node per layer (default 16, range 8–64)
- `ef_construction`: beam width during build (default 200)
- `ef_search`: beam width during query, tunable per-query
- `dimensions`: fixed at predicate declaration, enforced on insert

### Design
- Keyed by vector object's TermId (the vector literal is a graph primitive)
- Insert: vector literal interned, triple created, HNSW entry added under object's TermId
- Delete: **tombstoned** (flagged inactive, still traversable for graph connectivity — never removed until full rebuild)
- Virtual triples: HNSW neighbor edges exposed as `sutra:hnswNeighbor` triples, generated on-the-fly, not stored in SPO/POS/OSP
- Persistence: HNSW is ephemeral — rebuilt from stored vector triples on startup. Optional snapshot for faster cold start.
- Concurrency: search is `&self` (per-call visited list, Qdrant pattern); concurrent reads don't block

### Node layout (per HNSW node)
```rust
struct HnswNode {
    vector: Vec<f32>,          // 4 * dimensions bytes
    layer: u8,
    neighbors: Vec<Vec<u32>>,  // neighbor lists per layer, bounded by M
    triple_id: u64,            // back-reference into triple store
    deleted: bool,
}
```

---

## Hybrid SPARQL Extension

### VECTOR_SIMILAR operator
```sparql
SELECT ?doc ?entity WHERE {
  ?entity rdf:type :Person .
  ?doc :mentions ?entity .
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)
}

# With explicit ef_search hint
VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85, ef:=200)

# Score in ORDER BY
ORDER BY DESC(VECTOR_SCORE(?doc :hasEmbedding "..."^^sutra:f32vec))
```

### Query planner heuristic (v0.1)
- Subject **bound** before VECTOR_SIMILAR: execute graph first, filter by vector
- Subject **unbound**: execute vector search first (top-k), then evaluate graph patterns over candidates
- Adaptive execution (runtime reordering) is future work

---

## Query Language Policy

**Supported:** SPARQL 1.1 (superset — any standard query works; extensions add VECTOR_SIMILAR, VECTOR_SCORE)
**Planned:** Cypher as a translation layer/wrapper over SPARQL
**Never:** SQL, MongoDB Query Language — use the appropriate database for those. GraphQL — push to application layer.

---

## Explicitly Out of Scope

Do not implement these without explicit instruction:

- RDFS inference
- Built-in graph algorithms (PageRank, community detection, etc.)
- SQL or MongoDB query interfaces (see Query Language Policy)
- Distributed execution / sharding
- Embedding model metadata enforcement
- Multi-embedding-space / cross-modal queries
- GraphQL interface

---

## Reference Architectures: Oxigraph + Qdrant

SutraDB draws from two Rust databases:

- **[Oxigraph](https://github.com/oxigraph/oxigraph)** — Rust RDF triplestore. Reference for storage (RocksDB), indexing (SPO/POS/OSP), SPARQL pipeline (parser → optimizer → evaluator), snapshot-based transaction isolation.
- **[Qdrant](https://github.com/qdrant/qdrant)** — Rust vector database. Reference for HNSW implementation (immutable GraphLayers for search, thread-local visited pools, per-node RwLock during construction), vector preprocessing (normalize-at-insert for cosine).

SutraDB's differentiator: unifying both into one system where vectors are triples and the query planner treats HNSW as a 4th index type alongside SPO/POS/OSP.

---

## Open Questions (Unresolved)

- ~~**RDF-star vs. RDF 1.2**~~ **Resolved: RDF-star.** Direct edge annotation (`<< s p o >> :hasEmbedding ...`) is the natural pattern for vector work.
- **LSM-tree**: build from scratch vs. wrap RocksDB/sled? Oxigraph chose RocksDB.
- **IRI encoding**: Sequential interning (current) vs. hash-based (Oxigraph's SipHash approach)?
- **HNSW compaction**: what threshold triggers a background pass to clean deleted nodes?
- **SPARQL property paths** (`+`, `*`, `?`): traversal strategy for cycles on large graphs?
- ~~**License**: Apache 2.0 (patent grant) vs MIT (simplicity)?~~ **Resolved: Apache 2.0.**

---

## Coding Conventions

- Rust edition: 2021
- Use `thiserror` for error types
- Use `tokio` for async runtime in `sutra-proto`
- No `unwrap()` in library code — propagate errors
- All public API must have doc comments
- Benchmarks go in `benches/` using `criterion`
- Tests use `#[cfg(test)]` modules inline, plus integration tests in `tests/`
