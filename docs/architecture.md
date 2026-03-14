# SutraDB — Architecture

> A lean, high-performance RDF-star triplestore with native vector indexing and hybrid SPARQL.
> Draft v0.1

---

## 1. Design Philosophy

SutraDB is a single-purpose database. Its only job is to store triples and answer queries over them as fast as possible, at any scale. No inference engine. No reasoning layer. No OWL. The database is isomorphic with reality — it stores what you put in, nothing more. All semantics belong to the application.

**Three non-negotiable properties:**

1. Any traversal, of any depth, across the entire database, must be expressible in a single query.
2. Vectors are triples. A vector embedding is just an attribute of a node or an edge — stored, indexed, and queried the same way as any other predicate.
3. The database stays lean. Complexity is the enemy of performance. Every feature must justify its presence in terms of query throughput or data model expressiveness.

---

## 2. Data Model

### 2.1 RDF-star as the Foundation

All data is stored as RDF-star triples: subject, predicate, object — where any of the three positions can itself be a quoted triple. This gives us statements about statements natively, without reification hacks.

```turtle
# Embedding on a node
:paper_42 :hasEmbedding "0.23 -0.11 0.87 ..."^^sutra:f32vec .

# Embedding on a relationship (RDF-star)
<< :paper_42 :discusses :TransformerArchitecture >> :hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec .
<< :paper_42 :discusses :TransformerArchitecture >> :confidence 0.91 .
```

RDF-star means provenance, confidence scores, and vector embeddings on edges are all first-class citizens. This is the property that most triplestores cannot offer cleanly.

### 2.2 Vector Literals

A new literal type, `sutra:f32vec`, stores a fixed-dimension array of 32-bit floats. The database treats this type specially:

- At schema declaration time, dimensionality is registered for a given predicate.
- An HNSW index is automatically built and maintained over all triples with that predicate.
- The model that produced the vectors is an application-layer concern — the database stores raw floats only.
- Mismatched dimensionality on insert is a hard error.

**Schema declaration:**

```turtle
sutra:declareVectorPredicate :hasEmbedding ;
    sutra:dimensions 1536 ;
    sutra:hnswM 16 ;
    sutra:hnswEfConstruction 200 .
```

---

## 3. Storage Engine

### 3.1 Index Architecture

All triples are stored across six covering indexes to ensure any SPARQL access pattern hits an index rather than scanning. For RDF-star quoted triples, the quoted triple is assigned a deterministic content-addressed ID and treated as a node internally.

| Index | Purpose |
|---|---|
| SPO | Subject → Predicate → Object (primary store) |
| POS | Predicate → Object → Subject (range queries on predicates) |
| OSP | Object → Subject → Predicate (reverse traversal) |
| SP | Subject → Predicate (star-shaped queries) |
| PO | Predicate → Object (type lookups, range scans) |
| VECTOR(p) | One HNSW index per vector predicate (ANN queries) |

The underlying storage for SPO/POS/OSP is an LSM-tree, chosen over B-tree for its write amplification characteristics on bulk ingestion workloads typical in graph construction. (See open questions: build vs. wrap.)

### 3.2 Node Identity

IRIs and blank nodes are interned at write time to 64-bit integer IDs. All indexes operate on integer IDs, not string values. The string-to-ID dictionary is a separate persistent hash map. This keeps index entries small and comparison O(1).

Quoted triples (RDF-star) are hashed as a tuple `(S, P, O) → u64` content ID using xxHash3. Collision probability is negligible at any realistic graph size.

---

## 4. HNSW Vector Index Design

### 4.1 What HNSW Is

Hierarchical Navigable Small World (HNSW) is a graph-based approximate nearest neighbor (ANN) index. It builds a multi-layer proximity graph over your vectors. At query time it starts from an entry point at the top layer and greedily descends toward nearest neighbors, getting more precise at each layer. The result is sub-linear ANN search — typically O(log n) — with a controllable accuracy/speed tradeoff.

HNSW is the correct choice over IVF-flat or ANNOY because it supports **incremental inserts without full index rebuilds**, which is essential for a live database.

### 4.2 Key Parameters

| Parameter | Description |
|---|---|
| `M` | Max connections per node per layer. Higher = better recall, more memory. Typical: 8–64. |
| `ef_construction` | Beam width during index build. Higher = better quality, slower inserts. Typical: 100–400. |
| `ef_search` | Beam width during query. Tunable at query time — higher = better recall, slower search. |
| `dimensions` | Fixed at predicate declaration time. Enforced on every insert. |

### 4.3 Integration with the Triple Store

Each HNSW index is keyed by **triple ID**, not by a standalone vector ID. This is the critical integration point:

- **Insert**: vector is extracted and inserted into the predicate's HNSW index under the triple's SPO identity.
- **Delete**: the corresponding HNSW node is marked deleted (lazy deletion — HNSW supports this natively without graph restructuring).
- **Query**: HNSW returns a ranked list of triple IDs, which are joined against the SPARQL binding table like any other index result.

The HNSW index is a first-class index alongside SPO/POS/OSP — the query planner sees it as just another access path, not a foreign system.

### 4.4 Memory Layout

HNSW graph nodes are stored in a flat arena allocator per predicate index. Each node:

```rust
struct HnswNode {
    vector: Vec<f32>,          // 4 * dimensions bytes
    layer: u8,
    neighbors: Vec<Vec<u32>>,  // per-layer neighbor lists, bounded by M
    triple_id: u64,            // back-reference into triple store
    deleted: bool,
}
```

The arena is memory-mapped to disk. The index is available immediately on startup — no rebuild required.

### 4.5 Concurrency

Fine-grained `RwLock` per node: reads are fully concurrent, writes lock only the nodes being connected. This allows high read concurrency with acceptable write throughput for typical graph workloads.

Future work: lock-free HNSW variants using atomic CAS on neighbor lists for write-heavy workloads.

---

## 5. Hybrid SPARQL Extension

### 5.1 VECTOR_SIMILAR Operator

A new operator added to the SPARQL graph pattern language:

```sparql
# Basic usage
SELECT ?doc ?entity WHERE {
  ?entity rdf:type :Person .
  ?doc :mentions ?entity .
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)
}

# With explicit ef_search hint
VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85, ef:=200)

# Similarity score in ORDER BY
SELECT ?paper WHERE {
  :TransformerArchitecture :influences+ ?concept .
  ?paper :discusses ?concept .
  VECTOR_SIMILAR(?paper :hasEmbedding "..."^^sutra:f32vec, 0.80)
} ORDER BY DESC(VECTOR_SCORE(?paper :hasEmbedding "..."^^sutra:f32vec))
```

`VECTOR_SIMILAR` takes a subject variable, a vector predicate, a query vector literal, and a similarity threshold. It returns all subject IRIs whose embedding exceeds the threshold, ranked by cosine similarity.

### 5.2 Query Planning (v0.1 Heuristic)

The query planner must decide whether to execute `VECTOR_SIMILAR` first or last. Wrong order is expensive.

| Condition | Strategy |
|---|---|
| Subject **bound** before `VECTOR_SIMILAR` | Execute graph first, filter by vector similarity over result set |
| Subject **unbound** at `VECTOR_SIMILAR` | Execute vector search first (top-k), evaluate graph patterns over candidates |

Adaptive execution (runtime reordering based on observed intermediate cardinalities) is the correct long-term solution but is out of scope for v0.1.

---

## 6. Crate Architecture

```
sutra-core/      # Triple storage, LSM indexes, IRI interning, RDF-star IDs
sutra-hnsw/      # HNSW index, vector literal type, predicate index registry
sutra-sparql/    # SPARQL 1.1 parser, planner, executor, hybrid extension
sutra-proto/     # SPARQL HTTP protocol, Graph Store Protocol, REST API
sutra-cli/       # CLI: import, export, query, benchmark
```

**Hard dependency rules:**
- `sutra-hnsw` → **no dependency on `sutra-sparql`**. Pure data structure crate.
- `sutra-sparql` → depends on `sutra-core` + `sutra-hnsw`
- `sutra-proto` → depends on `sutra-sparql`
- `sutra-cli` → depends on `sutra-proto` + `sutra-sparql`

---

## 7. Explicitly Out of Scope

These will not be implemented without explicit instruction. They cannot be handled better at the database layer than at the application layer:

- OWL / RDFS inference or reasoning
- Built-in graph algorithms (PageRank, community detection, etc.)
- Multi-model query compatibility (SQL, Cypher, Gremlin)
- Distributed execution / sharding
- Embedding model metadata enforcement
- Multi-embedding-space / cross-modal queries
- GraphQL interface

---

## 8. Open Questions

These are unresolved architecture decisions that must be answered before or during implementation of the relevant component:

- **LSM-tree**: build from scratch vs. wrap RocksDB/sled? Wrapping is weeks faster to prototype but hides tuning knobs and adds a dependency.
- **HNSW compaction**: lazy deletion degrades index quality over time. What threshold triggers a background compaction pass to clean deleted nodes?
- **SPARQL property paths** (`+`, `*`, `?`): traversal strategy for cycles on large graphs — what prevents unbounded recursion?
- **License**: Apache 2.0 (explicit patent grant, attribution requirements) vs MIT (simpler)? Both permissive.
