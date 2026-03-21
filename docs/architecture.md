# SutraDB — Architecture

> A lean, high-performance RDF triplestore with native vector indexing and hybrid SPARQL.
> Influenced by Qdrant's vector indexing and Oxigraph's storage architecture, unified into a single system.
> Draft v0.3

---

## 1. Design Philosophy

SutraDB is a single-purpose database. Its only job is to store triples and answer queries over them as fast as possible, at any scale. The database is isomorphic with reality — it stores what you put in, nothing more. OWL reasoning is planned as an opt-in query-time layer that never modifies stored data. RDFS inference is out of scope.

**Four non-negotiable properties:**

1. Any traversal, of any depth, across the entire database, must be expressible in a single query.
2. Vectors are triples. A vector embedding is just an attribute of a node or an edge — stored, indexed, and queried the same way as any other predicate.
3. The database stays lean. Complexity is the enemy of performance. Every feature must justify its presence in terms of query throughput or data model expressiveness.
4. **Serverless by default, server when needed.** Like SQLite, SutraDB can be embedded directly in your application — just open a `.sdb` file. No daemon, no port, no configuration. When you need HTTP access, multi-client concurrency, or a SPARQL endpoint, start it in server mode. Same storage format either way.

---

## 2. Data Model

### 2.1 RDF-star as the Foundation

All data is stored as RDF-star triples: subject, predicate, object — where any of the three positions can itself be a quoted triple. This gives us statements about statements natively, without reification hacks.

RDF-star is a **superset of RDF 1.2** — any valid RDF 1.2 data (triple terms in object position only) is also valid RDF-star. SutraDB additionally allows triple terms in subject position, which is the natural pattern for annotating edges with vector embeddings.

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

SutraDB maintains **four types of indexes**. The first three are covering indexes over interned u64 IDs that ensure any triple access pattern hits an index rather than scanning. The fourth is what makes SutraDB unique — a native vector index that the query planner treats as a first-class access path alongside the triple indexes.

| Index | Key Order | Purpose |
|---|---|---|
| **SPO** | Subject → Predicate → Object | Primary store. Subject-first traversal, star-shaped queries (prefix scan on S gives all of a subject's triples, prefix on S+P gives a specific predicate's values). |
| **POS** | Predicate → Object → Subject | Predicate-first lookups: type queries (`?x rdf:type :Person`), predicate+object reverse lookup for vector resolution. |
| **OSP** | Object → Subject → Predicate | Object-first reverse traversal: "what points to this entity?" |
| **VECTOR(p)** | One HNSW graph per vector predicate | Approximate nearest neighbor search over vector embeddings. Keyed by the vector object's TermId. Returns ranked results that join back through POS for entity resolution. |

All triple index keys are 24 bytes (3 × u64 in big-endian for correct sort order). Since they are sorted, prefix scans serve multiple access patterns — there is no need for separate SP or PO indexes because they are just prefix scans on SPO and POS respectively.

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

Each HNSW index is keyed by the **vector object's TermId** — the vector literal is a primitive value in the graph, like a string or integer. This is the critical integration point:

- **Insert**: the vector literal is interned as a term, a triple `<subject> <predicate> <vector>` is created, and the vector is inserted into the predicate's HNSW index under the object's TermId.
- **Multiple subjects**: two entities can point to the same vector (e.g. "bank" the financial institution and "bank" the riverbank sharing an embedding). The HNSW index stores the vector once; the triple store links multiple subjects to it.
- **Delete (tombstone)**: deleted HNSW nodes are **tombstoned**, not removed. The node is flagged as inactive and excluded from search results, but it remains in the graph structure so it can still be traversed over during greedy descent. This preserves graph connectivity — removing a node would break neighbor links and degrade search quality. Tombstoned nodes are cleaned up only during a full index rebuild.
- **Query**: HNSW returns a ranked list of vector object IDs. The executor joins these back through the triple store's POS index to find which subjects connect to those vectors. A vector never exists without at least one triple pointing to it.

The HNSW index is a first-class index alongside SPO/POS/OSP — the query planner sees it as just another access path, not a foreign system.

### 4.4 Virtual Triples

HNSW neighbor connections are exposed as **virtual triples** using the `sutra:hnswNeighbor` predicate:

```turtle
:entity_A sutra:hnswNeighbor :entity_B .
```

These triples are not stored in SPO/POS/OSP — they exist only in the HNSW graph structure and are generated on-the-fly when queried. This means:

- `SELECT ?neighbor WHERE { :entity sutra:hnswNeighbor ?neighbor }` traverses the HNSW graph like any other triple pattern
- The same SPARQL executor handles both stored triples and virtual HNSW triples — **one unified graph, one traversal process**
- No special API or query syntax needed to explore the vector index structure

### 4.5 Persistence Model

The HNSW graph is **ephemeral by default, rebuildable from triples**:

- **Persisted**: all triples (including vector triples like `<entity> <hasEmbedding> <vector>`) are stored in SPO/POS/OSP indexes in the `.sdb` file, along with the term dictionary.
- **Rebuilt on startup**: HNSW graphs are reconstructed from the stored vector triples when the database opens. This ensures the index is always fresh — no stale neighbor connections, no accumulated tombstones, no degraded entry points.
- **Optional snapshot**: for faster cold start on large databases, the HNSW graph state can optionally be serialized alongside the `.sdb` file. If the snapshot exists and is valid, it is loaded directly instead of rebuilding. If corrupt or stale, the database falls back to rebuilding from triples.

HNSW graphs degrade over time — insertions and deletions shift the vector distribution, neighbor connections become suboptimal, and tombstoned nodes accumulate. A fresh rebuild from the current vector triples always produces a better index than preserving a stale one.

### 4.6 Memory Layout

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

### 4.7 Background Maintenance Cycle

During low-usage periods, SutraDB runs a background optimization cycle that rebuilds indexes without any downtime. The old indexes remain fully operational and in-memory while new ones are constructed — queries continue to hit the old indexes until the rebuild completes, then an atomic swap replaces them.

This cycle handles two things:
1. **HNSW rebuild** — construct a fresh HNSW graph from current vector triples. Eliminates tombstoned nodes, rebalances layer assignments, produces optimal neighbor connections for the current vector distribution.
2. **Pseudo-table discovery and rebuild** — re-scan the graph for emergent relational structure and materialize columnar indexes (see §4.8).

---

## 4.8 Pseudo-Tables (Auto-Discovered Columnar Indexes)

RDF has no tables, but relational structure exists implicitly in the graph. Nodes that share the same predicate-position structure are, in effect, rows of an implicit table. SutraDB auto-discovers these groups and materializes **pseudo-tables** — columnar indexes over groups of structurally similar nodes — to accelerate the SQL-like portions of SPARQL execution (joins, filters, aggregates over uniform data).

### 4.8.1 Property Definition

A "property" is defined by **predicate + position** (subject or object). This matters because being on different ends of the same predicate is semantically distinct:

| Triple | Properties assigned |
|---|---|
| `:Cat :eats :Mouse` | `:Cat` gets `SUB→eats`, `:Mouse` gets `OBJ→eats` |
| `:Mouse :eats :Grain` | `:Mouse` gets `SUB→eats`, `:Grain` gets `OBJ→eats` |

So `:Mouse` has two distinct properties: `SUB→eats` and `OBJ→eats`.

### 4.8.2 Group Discovery

A pseudo-table is formed when a statistically significant cluster of nodes (p < 0.05 vs. random co-occurrence) share enough predicate-position structure:

- **Minimum criteria**: 5 properties each held by ≥50% of the group
- Discovery runs during the background maintenance cycle, not during query time

### 4.8.3 Table Structure

Once a group qualifies:

| Column type | Inclusion rule |
|---|---|
| **Core columns** | Each property held by ≥33% of the group becomes a column |
| **Null values** | If a node lacks a core-column property, the value is null |
| **Tail count** | An integer column counting how many non-core properties ("tail properties") each node has |

The pseudo-table is a columnar index — contiguous memory layout per column, suitable for SIMD-accelerated filtering and vectorized execution.

### 4.8.4 Query Planner Integration

Pseudo-tables are an **accelerator**, not a replacement for the general triple store. The planner must decide per-pattern how to route:

| Situation | Strategy |
|---|---|
| All queried properties are pseudo-table columns | Full columnar scan — maximum acceleration |
| Some properties are columns, some are not | Resolve pseudo-table columns first (fast, columnar), then join remaining properties via regular SPO/POS/OSP lookups on the already-bound subjects |
| No properties match any pseudo-table | Fall back entirely to regular triple store scans |

Properties outside the pseudo-table's columns are resolved as normal SPARQL triple pattern lookups — they get no columnar benefit. This is by design: pseudo-tables accelerate the common case (queries over the shared structure of a group), not the general case. Asking for everything (`SELECT *` in SQL terms) would include tail properties that fall back to regular efficiency. But `SELECT *` is bad practice regardless — you should ask for only what you need.

The mixed case (some columns, some not) is the most common in practice. The planner should resolve the pseudo-table portion first to bind subjects cheaply, then use those bindings to do targeted SPO lookups for the remaining properties — which is fast because the subject is already bound.

### 4.8.5 Data Health Metric

The distribution of properties across a pseudo-table group reveals data quality:

- **Healthy**: sharp "cliff" between core and tail properties. Example: 10 properties held by 100% of the group, every other property held by ≤10%. This indicates well-structured, consistent data.
- **Unhealthy**: gradual slope from core to tail. Many properties at 30–40% coverage, no clear separation. This indicates inconsistent schema usage.

The cliff steepness is a quantifiable metric exposed through the health endpoint and Sutra Studio. It measures how "table-like" a group of nodes actually is — the sharper the cliff, the more benefit the pseudo-table provides.

Note that pseudo-tables also serve as a **data health indicator** beyond query optimization. Well-structured data naturally forms clean pseudo-tables with steep cliffs — the pseudo-table discovery process doubles as a data quality audit. Incomplete or inconsistent data produces shallow cliffs or fails to form pseudo-tables at all. This makes the pseudo-table health metrics a core component of the database health dashboard (see §4.9).

### 4.8.6 Relationship to Prior Work

The concept of grouping RDF subjects by shared predicate structure is known in the literature as **characteristic sets** (Neumann & Moerkotte, 2011; Pham et al., WWW 2015 — "MonetDB/RDF: Discovering and Exploiting the Emergent Schema"). SutraDB's pseudo-tables extend this with:
- Statistical significance testing (p < 0.05) rather than pure frequency thresholds
- Per-column statistics (min/max/null_count/distinct_count) following DuckDB's zonemap pattern
- Segment-level storage (~2048 rows per segment) with per-segment statistics for skip-scan pruning
- The data health cliff metric as a first-class diagnostic
- Background discovery and atomic swap during the maintenance cycle

Implementation references: DataFusion's `ColumnStatistics` and `Precision<T>` pattern for statistics representation, DuckDB's row-group/segment storage hierarchy with zonemaps for skip-scan pruning.

---

## 4.9 Database Health Dashboard

SutraDB exposes comprehensive database health diagnostics through two interfaces:

1. **Sutra Studio** (GUI) — visual health dashboard with charts, heatmaps, and interactive exploration. Aimed at human operators who want visual intuition about database state.

2. **`sutra health`** (CLI) — text-based health report aimed at AI agents. All metrics are output as structured text that an agent can parse and reason about. No GUI required.

Both interfaces expose the same underlying metrics:

| Metric | Source |
|---|---|
| HNSW index health | Tombstone ratio, layer distribution, entry point connectivity, recall estimate |
| Pseudo-table coverage | What percentage of triples fall into pseudo-tables, cliff steepness per group |
| Data structure quality | Characteristic set distribution, property coverage histograms |
| Storage statistics | Triple count, term dictionary size, index sizes, per-predicate cardinality |
| Query performance | Per-pattern latency percentiles, planner decision accuracy (pseudo-table hit rate) |

The health dashboard is a diagnostic tool, not a monitoring system — it reports current state on demand rather than collecting time-series data. For continuous monitoring, export metrics to an external system.

---

## 5. SPARQL+ Extension

SutraDB's query language is called **SPARQL+** — a superset of SPARQL 1.1. Any valid SPARQL 1.1 query works as-is. SPARQL+ adds two categories of extensions that standard SPARQL cannot express: vector search operators and predicate-based exit conditions on property path traversal.

### 5.1 VECTOR_SIMILAR Operator

The extensions below add vector search capabilities:

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

### 5.3 Predicate-Based Exit Conditions

Standard SPARQL property paths are declarative — they return all matches. There is no way to express "traverse this ordered sequence and stop when a condition is met." This is a real expressiveness gap.

**Example:** Traverse American presidents in chronological order until you find the first one who died in office. In standard SPARQL you can find presidents who died in office, or traverse the sequence, but the two don't compose — you'd have to pull all results and filter client-side.

SPARQL+ adds **exit conditions** to property path traversal: a predicate evaluated per-step during traversal, not post-traversal. When the exit condition is met on a branch, that branch terminates and returns the matching node.

```sparql
# Find the first president in succession who died in office
SELECT ?president WHERE {
  :GeorgeWashington :succeededBy+ ?president
  UNTIL { ?president :diedInOffice true }
}
```

**Design considerations:**
- Exit on one branch does not kill other branches (scoping is per-path)
- Only meaningful when traversal order is defined (directed labeled edges provide this naturally)
- HNSW-specific exit condition: "no closer neighbor found" — local optimality termination that maps to the HNSW algorithm's natural stopping criterion

---

## 6. Crate Architecture

```
sutra-core/      # Triple storage, LSM indexes, IRI interning, RDF-star IDs
sutra-hnsw/      # HNSW index, vector literal type, predicate index registry
sutra-sparql/    # SPARQL 1.1 parser, planner, executor, hybrid extension
sutra-proto/     # SPARQL HTTP protocol, Graph Store Protocol, REST API
sutra-cli/       # CLI: serve, query, import, export, health, mcp
sutra-ffi/       # C-compatible FFI layer for embedding in non-Rust apps
```

**Hard dependency rules:**
- `sutra-hnsw` → **no dependency on `sutra-sparql`**. Pure data structure crate.
- `sutra-sparql` → depends on `sutra-core` + `sutra-hnsw`
- `sutra-proto` → depends on `sutra-sparql`
- `sutra-cli` → depends on `sutra-proto` + `sutra-sparql`
- `sutra-ffi` → depends on `sutra-core` + `sutra-hnsw` + `sutra-sparql`. Produces a C shared library (`.dll`/`.so`/`.dylib`).

---

## 7. Sutra Studio

Sutra Studio is a Flutter desktop/web application that provides a visual interface to SutraDB. It is designed for operations that benefit from visual intuition — graph visualization, HNSW health heatmaps, manual emergency editing — while the CLI and MCP server remain the primary interfaces for agents and automation.

### 7.1 Single-Process Architecture

Studio, the MCP server, and the database engine all run in a single process:

```
┌─────────────────────────────────────────┐
│  Sutra Studio (Flutter GUI)             │  ← optional, can be on or off
│  Health │ Graph │ SPARQL │ Ontology      │
├─────────────────────────────────────────┤
│  MCP Server (JSON-RPC stdin/stdout)     │  ← optional, toggleable at runtime
├─────────────────────────────────────────┤
│  sutra-ffi (C ABI shared library)       │  ← the glue layer
│  ┌─────────┬───────────┬──────────────┐ │
│  │sutra-core│sutra-hnsw│sutra-sparql  │ │  ← the database engine
│  └─────────┴───────────┴──────────────┘ │
└─────────────────────────────────────────┘
```

Flutter loads `sutra_ffi.dll`/`.so`/`.dylib` via `dart:ffi`. The shared library contains the full database engine. The MCP server can run on a background thread within the same process. All three layers share the same database handle — zero serialization overhead.

**Two entry points to the same system:**

| Entry point | What runs |
|---|---|
| `sutra mcp` | MCP server + database engine. Headless, no GUI. |
| Sutra Studio | GUI + database engine + optional MCP server. All one process. |

Studio can open `.sdb` files directly (like SQLite browsers), start/stop the MCP server from within the GUI, and optionally start an HTTP server for remote SDK access — all without separate processes.

The HTTP client mode still exists for connecting to remote or already-running instances.

### 7.2 FFI Layer (`sutra-ffi`)

The `sutra-ffi` crate produces a C-compatible shared library that can be loaded by any language with FFI support (Dart, Python, C, C++, etc.). It wraps the core Rust crates behind a stable C ABI.

**Exposed functions:**

```c
// Database lifecycle
sutra_db_t* sutra_db_open(const char* path);       // Open or create .sdb file
void        sutra_db_close(sutra_db_t* db);         // Close and flush

// Triple operations
int      sutra_insert_ntriples(sutra_db_t* db, const char* data);   // Insert N-Triples
uint64_t sutra_triple_count(sutra_db_t* db);                         // Total triple count

// Term dictionary
uint64_t    sutra_intern(sutra_db_t* db, const char* term);          // String → ID
const char* sutra_resolve(sutra_db_t* db, uint64_t id);              // ID → string

// SPARQL query
sutra_result_t* sutra_query(sutra_db_t* db, const char* sparql);     // Execute SPARQL+
void            sutra_result_free(sutra_result_t* result);            // Free result

// Health diagnostics
const char* sutra_health_report(sutra_db_t* db);                     // Full health text
void        sutra_string_free(const char* s);                         // Free returned strings

// Server management
int sutra_serve_start(sutra_db_t* db, uint16_t port, const char* passcode);
int sutra_serve_stop(sutra_db_t* db);
```

**Design principles:**
- All functions are `extern "C"` with `#[no_mangle]`
- Opaque pointer types (`sutra_db_t`, `sutra_result_t`) hide Rust internals
- Strings are passed as `*const c_char` (null-terminated UTF-8) and returned as owned C strings that must be freed with `sutra_string_free`
- Errors return null pointers or negative integers; last error message available via `sutra_last_error()`
- Thread-safe: the opaque handle wraps `Arc<Mutex<...>>` internally

**Build output per platform:**

| Platform | Library | Dart FFI loads via |
|---|---|---|
| Windows | `sutra_ffi.dll` | `DynamicLibrary.open('sutra_ffi.dll')` |
| Linux | `libsutra_ffi.so` | `DynamicLibrary.open('libsutra_ffi.so')` |
| macOS | `libsutra_ffi.dylib` | `DynamicLibrary.open('libsutra_ffi.dylib')` |

The shared library ships alongside the Studio binary in release archives. Studio loads it at startup from the same directory as its own executable.

### 7.3 MCP Server as an FFI Capability

The MCP server is not a separate binary — it is a capability exposed by the FFI layer. Studio can start an MCP server on a background thread via `sutra_mcp_start(db, stdin_fd, stdout_fd)`, sharing the same database handle that the GUI is using. This means an AI agent can connect to Studio's MCP server and both the agent and the human see the same database state in real time.

When running headless (`sutra mcp`), the CLI binary uses the same FFI functions internally.

### 7.4 MCP Studio Tools

The MCP server provides two tools for Studio management:

- **`download_studio`** — Downloads the pre-built Studio binary (including the FFI shared library) from GitHub releases for the current platform.
- **`launch_studio`** — Opens Studio. If not installed, downloads it first. Passes the database path or HTTP endpoint depending on mode.

Auto-update keeps Studio in sync with the CLI version — when the `sutra` binary updates, Studio is also re-downloaded if installed.

### 7.5 Studio Screens

| Screen | Purpose |
|---|---|
| **Health** | HNSW index diagnostics, tombstone ratios, pseudo-table coverage, rebuild controls |
| **Graph** | Force-directed graph visualization (semantic, vector, or all edges) |
| **Triples** | Sortable/filterable triple table with add/delete |
| **SPARQL** | Query editor with syntax highlighting and result display |
| **Ontology** | OWL class hierarchy browser |
| **Auth** | Connection settings, endpoint configuration, authentication |

---

## 8. MCP Server (Model Context Protocol)

SutraDB includes a native MCP server (`sutra mcp`) that allows AI agents to interact with the database over JSON-RPC 2.0 via stdin/stdout. The MCP server operates in two modes:

- **Server mode**: connects to a running `sutra serve` HTTP endpoint
- **Serverless mode**: opens a `.sdb` file directly via library calls (no server needed)

### 8.1 MCP Tools

| Tool | Description |
|---|---|
| `health_report` | Full database diagnostics (HNSW, storage, consistency) |
| `rebuild_hnsw` | Compact and rebuild vector indexes |
| `verify_consistency` | Check SPO/POS/OSP index consistency, auto-repair |
| `database_info` | Triple count, term count, vector index count |
| `sparql_query` | Execute SPARQL+ queries |
| `insert_triples` | Insert N-Triples data |
| `backup` | Create database snapshot |
| `vector_search` | ANN search via VECTOR_SIMILAR |
| `download_studio` | Download and install Sutra Studio |
| `launch_studio` | Open Sutra Studio (downloads first if needed) |
| `check_update` | Check for new SutraDB releases |
| `decline_update` | Cancel pending auto-update |

### 8.2 Auto-Update

On startup, the MCP server checks GitHub releases for a newer version. If found, it notifies the agent and auto-installs after a 2-minute window (cancelable via `decline_update`). When Studio is installed, it is also updated to match.

### 8.3 Resources and Prompts

**Resources:** `sutra://connection` (mode/endpoint info), `sutra://version` (build info), `sutra://schema` (predicates, serverless only).

**Prompts:** `explore_graph` (sample triples + schema), `find_similar` (VECTOR_SIMILAR template), `count_by_type` (GROUP BY rdf:type).

---

## 9. Query Language Policy

**Supported:**
- **SPARQL+** — SPARQL 1.1 superset with VECTOR_SIMILAR, VECTOR_SCORE, and predicate-based exit conditions (UNTIL). The primary query interface.

**Planned:**
- Cypher — as a translation layer/wrapper over SPARQL, not a native execution engine

**Never:**
- SQL — not appropriate for graph data; use a relational database
- MongoDB Query Language — not appropriate for graph data; use a document database
- GraphQL — push to application layer

---

## 10. Explicitly Out of Scope

These will not be implemented without explicit instruction. They cannot be handled better at the database layer than at the application layer:

- RDFS inference
- Built-in graph algorithms (PageRank, community detection, etc.)
- Distributed execution / sharding
- Embedding model metadata enforcement
- Multi-embedding-space / cross-modal queries

---

## 11. Reference Architectures

SutraDB draws from multiple open-source databases across two domains: RDF/vector indexing and SQL query optimization.

### 9.1 RDF & Vector Indexing

- **[Oxigraph](https://github.com/oxigraph/oxigraph)** (Rust) — Closest existing Rust triplestore. Reference for storage (RocksDB), indexing (SPO/POS/OSP), SPARQL pipeline (parser → optimizer → evaluator). SutraDB diverges by adding native HNSW vector indexing and SPARQL+ extensions.
- **[Qdrant](https://github.com/qdrant/qdrant)** (Rust) — Vector database. Reference for HNSW implementation (immutable GraphLayers, thread-local visited pools, per-node RwLock during construction), vector preprocessing (normalize-at-insert for cosine).

### 9.2 SQL-Like Query Optimization

Every operation you can do in SQL you can do in SPARQL — triple pattern matching is fundamentally relational joins over a three-column relation (subject, predicate, object). All SQL execution optimization that operates at the relational join layer is fair game for SPARQL. The part unique to SPARQL/graph (property paths, HNSW traversal) has no SQL analogue.

- **[DataFusion](https://github.com/apache/datafusion)** (Apache, Rust) — Most mature Rust query engine. Primary reference for cost-based planning, join ordering, predicate pushdown, and vectorized execution. Embeddable and extensible, which matches SutraDB's architecture.
- **[GlueSQL](https://github.com/gluesql/gluesql)** (Rust) — Pure Rust, small and readable. Good for understanding query parsing and planning without excessive complexity.
- **[Limbo](https://github.com/tursodatabase/limbo)** (Turso, Rust) — Rust SQLite reimplementation. Reference for storage layer ideas.
- **[DuckDB](https://github.com/duckdb/duckdb)** (C++) — Not Rust, but extremely influential. Columnar, vectorized, analytical. Excellent join ordering and cost model work.
- **[Materialize](https://github.com/MaterializeInc/materialize)** (Rust) — Streaming SQL on Differential Dataflow. Sophisticated execution architecture for incremental computation.

---

## 12. Open Questions

These are unresolved architecture decisions that must be answered before or during implementation of the relevant component:

- ~~**RDF-star vs. RDF 1.2**~~ **Resolved: RDF-star.** The `<< s p o >> :hasEmbedding ...` syntax is the natural way to annotate edges with vectors. RDF 1.2's object-only restriction adds indirection (reification nodes) that doesn't serve the embedding use case. Users working in vector/embedding space will expect direct edge annotation. If RDF 1.2 compatibility is ever needed, a translation layer can handle it.
- **LSM-tree**: build from scratch vs. wrap RocksDB/sled? Wrapping is weeks faster to prototype but hides tuning knobs and adds a dependency. Oxigraph chose RocksDB.
- **HNSW compaction**: lazy deletion degrades index quality over time. What threshold triggers a background compaction pass to clean deleted nodes?
- **SPARQL property paths** (`+`, `*`, `?`): traversal strategy for cycles on large graphs — what prevents unbounded recursion?
- **IRI encoding**: Our current sequential interning vs. Oxigraph's hash-based approach (128-bit SipHash, no collision issues at scale, eliminates need for string→ID index).
- ~~**License**: Apache 2.0 vs MIT?~~ **Resolved: Apache 2.0.**
