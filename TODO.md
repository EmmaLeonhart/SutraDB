# SutraDB — TODO

**Status: 160 of 215 items complete (74%)**

## Remaining (55 items)

### SPARQL+ Query Engine Optimization (algorithmic priority)

These are the core algorithmic improvements that make SutraDB competitive with optimized SQL engines. The goal: every optimization trick that makes SQL fast should apply to triple pattern matching in SPARQL, while property paths and HNSW traversal are our novel territory where no SQL engine operates.

#### HNSW Traversal via SPARQL Property Paths
The core novel contribution. HNSW topology is already exposed as virtual RDF triples (`sutra:hnswNeighbor`), but the SPARQL executor can't query them yet. The insight: encode HNSW layers as labeled directed edges (vertical descent edges vs horizontal neighbor edges) so that property path expressions naturally express HNSW search. The SPARQL executor does the right thing because the RDF representation does the semantic heavy lifting.

- [ ] Make virtual HNSW edge triples queryable in SPARQL patterns (`?a sutra:hnswNeighbor ?b`)
- [ ] Label vertical (layer descent) vs horizontal (neighbor) HNSW edges with distinct predicates
- [ ] Encode directionality so property paths can express "descend from entry, fan out horizontally"
- [ ] Ensure greedy descent + beam search semantics emerge from the graph structure and property path evaluation
- [ ] Test: property path `sutra:hnswNeighbor+` produces correct ANN results

#### Predicate-Based Exit Conditions (SPARQL+ extension)
Standard SPARQL property paths are declarative and return all matches — there is no way to say "traverse this sequence and stop when a condition is met." This is a real expressiveness gap. Example: traverse American presidents in order until you find the first one who died in office — impossible in standard SPARQL.

- [ ] Design syntax for exit conditions on property path traversal (e.g. UNTIL clause)
- [ ] Implement per-step predicate evaluation during property path execution (not post-traversal filtering)
- [ ] Handle interaction with backtracking (exit on one branch shouldn't kill other branches)
- [ ] Handle ordered traversal (exit conditions only make sense when traversal order is defined)
- [ ] HNSW-specific exit condition: "no closer neighbor found" (local optimality termination)
- [ ] Test: ordered traversal with UNTIL produces correct early termination

#### Cost-Based Query Planning
The classic SPARQL bottleneck. `estimate_cardinality()` exists in the store but the planner doesn't use it for join ordering — it just uses static weights based on unbound variable count.

- [ ] Integrate cardinality estimation into planner's join ordering (not just unbound count)
- [ ] HNSW as access path: planner decides "use HNSW index scan" vs "use SPO triple scan" based on cost, like SQL choosing between index scan and seq scan
- [ ] Adaptive execution: observe intermediate result sizes at runtime, reorder mid-query
- [ ] Predicate pushdown: push filters closer to the scan to reduce intermediate result sizes

#### Join Strategy Selection
Currently row-at-a-time volcano model. SQL engines choose between hash joins, merge joins, nested loop joins based on cost estimates.

- [ ] Cost-based selection between hash join, merge join, nested loop join
- [ ] Use cardinality estimates to pick optimal join strategy per pattern pair

#### Background Maintenance Cycle
During low-usage periods, the database runs a background optimization cycle. The old indexes remain fully operational and in-memory while new ones are being built — zero downtime, atomic swap when ready.

- [ ] Low-usage detection heuristic (query rate below threshold for N seconds)
- [ ] Background HNSW rebuild: construct fresh HNSW graph from current vector triples while old graph serves queries
- [ ] Atomic swap: replace old HNSW graph with rebuilt one once construction completes
- [ ] Background pseudo-table discovery and rebuild (see below)

#### Pseudo-Tables (Auto-Discovered Columnar Indexes)
RDF has no tables, but relational structure exists implicitly in the graph. Pseudo-tables auto-discover groups of nodes that share enough predicate-position structure to benefit from columnar indexing, then materialize table-like indexes over them for SQL-like query acceleration.

**Discovery criteria:**
- A "property" is defined by predicate + position (SUB or OBJ). Example: if cats eat mice, then cat has property `SUB→eats` and mice has both `SUB→eats` and `OBJ→eats`. Being on different ends of the same predicate is a distinct property.
- A group qualifies for a pseudo-table if a statistically significant cluster of nodes (p < 0.05) share 5+ properties, where each of those 5+ properties is held by ≥50% of the group.
- Minimum criteria: 5 properties held by ≥50% of the group each.

**Table structure:**
- Each property held by ≥33% of the group becomes a column in the pseudo-table.
- If a node doesn't have a property that is a column, the value is null.
- An additional column: count of "tail properties" (properties not included as columns) per node.

**Data health metric:**
- The "cliff" between core properties and tail properties indicates database health.
- Healthiest: e.g. 10 properties held by 100% of the group, every other property held by ≤10%. Sharp cliff = well-structured data.
- The tail property distribution is a measurable signal of schema consistency.

- [ ] Property extraction: scan graph for predicate-position pairs per node
- [ ] Group discovery: statistical clustering to find nodes sharing 5+ properties at p < 0.05
- [ ] Pseudo-table materialization: columnar index with ≥33% properties as columns, nulls for absent values, tail-property count column
- [ ] Data health metric: compute cliff steepness between core and tail property distributions
- [ ] Query planner integration: recognize when a SPARQL pattern matches a pseudo-table and route through columnar index
- [ ] Expose health metrics via HNSW health endpoint / Sutra Studio
- [ ] Per-column statistics (min/max/null_count/distinct_count) following DataFusion's Precision<T> pattern
- [ ] Segment-level storage (~2048 rows) with per-segment zonemaps for skip-scan pruning (DuckDB pattern)
- [ ] Sort pseudo-table rows by most selective predicate for tighter zonemaps

#### Database Health Dashboard
Two interfaces, same underlying metrics: Sutra Studio (GUI, visual, for humans) and `sutra health` (CLI, structured text, for AI agents). The agent-oriented CLI is critical — agents need to be able to assess database health, pseudo-table coverage, HNSW quality, and data structure quality without ever touching a GUI.

- [ ] `sutra health` CLI command: structured text output of all health metrics
- [ ] HNSW health metrics: tombstone ratio, layer distribution, entry point connectivity, recall estimate
- [ ] Pseudo-table health metrics: coverage percentage, cliff steepness per group, characteristic set distribution
- [ ] Storage metrics: triple count, term dictionary size, index sizes, per-predicate cardinality
- [ ] Query performance metrics: per-pattern latency percentiles, planner decision accuracy
- [ ] Sutra Studio health dashboard: visual charts/heatmaps for all the above

#### Vectorized Execution for Triple Pattern Scans
Lower priority than the above (graph workloads are pointer-chasing, not scan-heavy), but applicable to the relational-join portions of SPARQL queries — and pseudo-tables make this much more viable since they provide the contiguous columnar layout that SIMD needs. SIMD already exists for distance calculations but not for triple matching.

- [ ] Batch triple pattern scans (process multiple candidate triples per iteration)
- [ ] SIMD-accelerated triple ID comparison during index scans
- [ ] Vectorized filtering over pseudo-table columns

### Reference Architectures to Study

**For HNSW / vector indexing:**
- [Qdrant](https://github.com/qdrant/qdrant) — Rust vector database. Reference for HNSW (immutable GraphLayers, thread-local visited pools, per-node RwLock during construction), vector preprocessing (normalize-at-insert for cosine).

**For RDF / SPARQL:**
- [Oxigraph](https://github.com/oxigraph/oxigraph) — Rust RDF triplestore. Reference for storage (RocksDB), indexing (SPO/POS/OSP), SPARQL pipeline (parser → optimizer → evaluator).

**For SQL-like query optimization (joins, planning, execution):**
- [DataFusion](https://github.com/apache/datafusion) (Apache, Rust) — The most mature Rust query engine. Columnar, vectorized, embeddable and extensible. Primary reference for cost-based planning, join ordering, predicate pushdown, and vectorized execution.
- [GlueSQL](https://github.com/gluesql/gluesql) (Rust) — Pure Rust, small and readable. Good for understanding query parsing and planning without drowning in complexity.
- [Limbo](https://github.com/tursodatabase/limbo) (Turso, Rust) — Rust SQLite reimplementation. Interesting for storage layer ideas.
- [DuckDB](https://github.com/duckdb/duckdb) (C++) — Not Rust, but the ideas are extremely influential. Columnar, vectorized, analytical. Excellent join ordering and cost model work.
- [Materialize](https://github.com/MaterializeInc/materialize) (Rust) — Streaming SQL on Differential Dataflow. Different problem domain but sophisticated execution architecture.

### SDK Publishing (needs registry accounts — see docs/SDK_ACCOUNTS_SETUP.md)
- [ ] Python SDK: publish to PyPI
- [ ] TypeScript SDK: publish to npm
- [ ] Rust SDK: publish to crates.io
- [ ] Java SDK: publish to Maven Central
- [ ] C# SDK: publish to NuGet
- [ ] Go SDK: tag for Go modules

### Sutra Studio
- [ ] Flutter graph view: remaining browse.html parity work
- [ ] Long-term: absorb core Protege functionality into Sutra Studio

### Premium Tier (deferred until paying customers)
- [ ] RBAC — role-based access control
- [ ] Encryption at rest
- [ ] TLS / encryption in transit
- [ ] Audit logging
- [ ] Replication
- [ ] Clustering / sharding
- [ ] Multi-tenancy
- [ ] Connection pooling

---

## Completed (160 items)

<details>
<summary>Click to expand</summary>

### Core Engine
- [x] Database configuration model, HNSW edges as virtual RDF triples
- [x] VECTOR_SIMILAR + VECTOR_SCORE, planner integration, ef/k hints
- [x] VectorRegistry, ORDER BY, UNION, N-Triples/N-Quads/Turtle/RDF-XML/JSON-LD parsers
- [x] POST /triples, /vectors/declare, /vectors, /graph, /graph-store endpoints
- [x] PersistentStore (sled), persistent term dictionary, HNSW rebuilt on startup
- [x] SIMD distance functions (AVX2/FMA + SSE), HashSet visited list
- [x] Multiple HNSW entry points, HNSW compaction, parallel bulk_insert (rayon)
- [x] Hash joins, cardinality estimation, materialized adjacency lists
- [x] Named graph support (Triple::quad), crash recovery (verify + repair)
- [x] Query timeout enforcement, LIMIT push-down

### SPARQL Completeness
- [x] SELECT, ASK, CONSTRUCT, DESCRIBE, INSERT DATA, DELETE DATA
- [x] BIND/VALUES, GROUP BY/HAVING, aggregates (COUNT/SUM/AVG/MIN/MAX)
- [x] Property paths (+, *, ?, /), Subqueries, RDF-star quoted triples
- [x] FILTER: =, !=, <, >, <=, >=, &&, ||, !, NOT EXISTS, EXISTS
- [x] String functions: CONTAINS, STRSTARTS, STRENDS, REGEX
- [x] LANG, LANGMATCHES, DATATYPE, STR, COALESCE, IF, isIRI, isLiteral
- [x] Arithmetic expression parsing, OPTIONAL, UNION, DISTINCT, PREFIX

### HTTP & Server
- [x] Content negotiation (Accept → JSON/XML/CSV/TSV)
- [x] Passcode authentication, rate limiting, query timeouts
- [x] HNSW health endpoint, service description, Graph Store Protocol
- [x] Periodic backups (--backup-interval), schema declaration via SPARQL
- [x] GET /graph (Turtle/N-Triples export)

### SDKs & Ecosystem
- [x] 6 SDKs (Python, TypeScript, Go, Rust, Java, .NET) + endpoint fixes
- [x] Python OWL validation (domain/range/subclass/disjoint/equivalent/sameAs/inverse)
- [x] Verification query generation, integration test CI, publish workflow
- [x] LangChain VectorStore, Jupyter %%sparql magic, MCP server (6 tools)
- [x] Agent installer CLI (--launch-studio), Protege plugin, Dockerfile

### Sutra Studio (Flutter)
- [x] Desktop/web scaffold, Dart client, force-directed graph
- [x] View modes, triple editor, SPARQL editor, ontology viewer
- [x] HNSW health diagnostics, heatmap, backup management panel
- [x] IRI shortening, click-to-expand, predicate filtering, triple list panel
- [x] Japanese labels, HNSW virtual edges, dark/light theme, persistent settings
- [x] OWL export, graph export hint, Windows desktop platform

### Data & Benchmarks
- [x] 82K triples + 79K vectors (embedding-mapping), 500K+1M stress test
- [x] 439 Wikidata BFS import (16K triples), 435 Japanese embeddings
- [x] Benchmark suite: <1ms queries, 20K inserts/sec, 40ms full export
- [x] Storage benchmark baseline (sled), IRI encoding evaluation

### Documentation
- [x] Agent setup guide, SDK publishing/accounts guides, session notes
- [x] README, Open Graph meta tags, AI agent website callout

</details>

## Benchmark Results

Benchmark results are tracked automatically by CI. See:
- **[benchmarks/LATEST.md](benchmarks/LATEST.md)** — most recent Criterion results
- **[benchmarks/HISTORY.md](benchmarks/HISTORY.md)** — full history over time

### Baseline (manual, 16K triples, 435 vectors)

| Query | Latency |
|-------|---------|
| Health check | 0.6ms |
| SELECT LIMIT 10 | 0.7ms |
| SELECT LIMIT 1000 | 5.2ms |
| 2-pattern join | 0.6ms |
| GROUP BY aggregate | 0.6ms |
| FILTER CONTAINS | 0.4ms |
| OPTIONAL | 0.7ms |
| INSERT/DELETE DATA | <1ms |
| Full Turtle export (16K) | 41ms |
| N-Triples export | 35ms |
| Bulk insert (2000) | 76ms (20K/sec) |
| Point lookup p50 | 0.61ms |
| Point lookup p99 | 1.25ms |
