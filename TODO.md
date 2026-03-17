# SutraDB — TODO

**Status: 185 of 218 items complete (85%)**

---

## Active Work

### HNSW Traversal via SPARQL Property Paths
HNSW topology is exposed as virtual RDF triples (`sutra:hnswNeighbor`) with labeled edges (vertical descent vs horizontal neighbor). The remaining work: make the SPARQL executor's property path evaluation produce correct ANN results by letting greedy descent + beam search emerge from the graph structure.

- [ ] Greedy descent + beam search semantics from graph structure and property path evaluation
- [ ] Test: `sutra:hnswNeighbor+` produces correct ANN results

### Predicate-Based Exit Conditions (UNTIL)
Standard SPARQL property paths can't express "traverse and stop when a condition is met." Example: traverse American presidents in order until the first who died in office.

- [ ] Design UNTIL syntax for exit conditions on property path traversal
- [ ] Per-step predicate evaluation during traversal (not post-filter)
- [ ] Backtracking interaction (exit on one branch doesn't kill others)
- [ ] Ordered traversal (exit conditions require defined traversal order)
- [ ] HNSW-specific exit: "no closer neighbor found" (local optimality termination)
- [ ] Test: ordered traversal with UNTIL produces correct early termination

### Cost-Based Query Planning (remaining)
Cardinality estimation and predicate pushdown are done. Remaining:

- [ ] HNSW as access path: planner chooses "HNSW index scan" vs "SPO triple scan" based on cost
- [ ] Adaptive execution: observe intermediate result sizes at runtime, reorder mid-query

### Background Maintenance Cycle
During low-usage periods, rebuild indexes in the background. Old indexes stay live; atomic swap when ready.

- [ ] Low-usage detection heuristic (query rate below threshold for N seconds)
- [ ] Background HNSW rebuild: fresh graph from current vectors, old graph serves queries until swap
- [ ] Atomic swap: replace old HNSW with rebuilt one
- [ ] Background pseudo-table rediscovery and rebuild

### Pseudo-Tables — Deep Subgraph Columnar Indexes

**Key insight: pseudo-tables are not limited to single-node property bags.** A pseudo-table can represent an entire deep subgraph — multiple connected nodes forming a repeated structural pattern — as long as the pattern appears frequently enough across the graph and the columns can be scanned in parallel. What matters is strong parallelism: if N instances of the same subgraph shape exist, they form N rows, and each column is a position within that shape (a node, an edge label, a literal at depth 2, etc.).

Example: if thousands of papers each have an author node, an institution node, and a funding source, the entire `paper → author → institution` + `paper → fundedBy → source` subgraph is one pseudo-table row, with columns like `paper_iri`, `author_name`, `institution_name`, `funding_source`. This is far more powerful than just "nodes that share predicates" — it captures relational joins that the graph encodes structurally.

**Discovery criteria (revised):**
- A "pattern" is a rooted subgraph shape: a set of paths from a root node through predicates to leaf positions.
- A group qualifies if a statistically significant cluster of subgraph instances (p < 0.05) share the same shape, with each path present in ≥50% of instances.
- Minimum: 5 distinct paths, each held by ≥50% of the group.
- Paths present in ≥33% become columns; absent values are null.
- Tail path count column tracks structural irregularity per instance.

**The cliff metric still applies:** sharp drop-off between core paths and tail paths = well-structured data.

**Depth threshold — geometric scaling:**
Deep pseudo-tables are riskier than shallow ones (more joins baked in, wider invalidation surface), so the qualification threshold should scale geometrically with depth:
- Depth 1 (single-node characteristic set): base threshold (current: 10 nodes minimum)
- Depth 2: threshold × 4 (e.g. 40 instances)
- Depth 3: threshold × 9 (e.g. 90 instances)
- Depth N: threshold × N²

Rationale: deeper materialization is a bigger commitment. It should only happen when the pattern is overwhelmingly common. But paradoxically, very deep and very common patterns tend to be stable — a country→capital→mayor chain rarely changes structure, even if individual mayors change. The geometric threshold ensures we only pay the cost for patterns that justify it.

**Overlap vs. tree-like structures:**
Not all deep subgraphs are equal for pseudo-table materialization. Two categories:

1. **Tree-like (low overlap):** Each root node's subgraph is mostly disjoint from others. Example: `country → capital city → mayor → date of birth`. Each country traces a unique path — materializing these as rows produces no duplication. These are ideal pseudo-table candidates.

2. **DAG/lattice (high overlap):** Subgraphs share interior nodes heavily. Example: genealogies — every person has a mother and father, but those parents are themselves persons in the table, creating massive overlap. Materializing `person → mother → grandmother` as flat rows duplicates the grandmother's data across every grandchild row.

High-overlap structures risk:
- Storage blowup from duplicated interior nodes across many rows
- Update amplification — changing one shared interior node invalidates many rows
- Diminishing returns — the "join elimination" benefit is smaller when the same node appears in many rows anyway (it's likely hot in cache from regular index lookups)

Detection heuristic: during subgraph pattern mining, measure the **fan-in ratio** of interior nodes. If the average interior node appears in >K root instances (high fan-in), the pattern is DAG-like and should be penalized or skipped. Tree-like patterns have fan-in ≈ 1.

**Invalidation model:**
- Depth 1: invalidate when a node's direct predicates change (current behavior)
- Depth N: invalidate when *any node on the materialized path* changes. A country renaming invalidates every person row that traces through it.
- Mitigation: deep pseudo-tables are rebuilt during the background maintenance cycle, not eagerly. Stale rows are acceptable between cycles — the regular triple store is always authoritative.

All existing pseudo-table work (property extraction, group discovery, columnar storage, zonemaps, vectorized scans) is done. The remaining work is evolving discovery from single-node property sets to multi-hop subgraph shapes:

- [ ] Extend property model from `(predicate, position)` pairs to rooted path shapes (multi-hop)
- [ ] Subgraph pattern mining: discover repeated structural motifs across the graph
- [ ] Geometric depth threshold: N² scaling for minimum group size at depth N
- [ ] Fan-in ratio detection: identify high-overlap (DAG/lattice) vs tree-like subgraph patterns
- [ ] Overlap penalty: skip or deprioritize high-fan-in patterns to avoid storage blowup and update amplification
- [ ] Multi-node pseudo-table materialization: columns can reference nodes at any depth
- [ ] Invalidation tracking: flag stale rows when interior nodes change, rebuild during maintenance cycle
- [ ] Update query planner to recognize multi-pattern SPARQL queries that match a subgraph pseudo-table

### Database Health Dashboard (remaining)
CLI (`sutra health`) and Studio share the same metrics. CLI output needs iteration for agent readability.

- [ ] Query performance metrics: per-pattern latency percentiles, planner decision accuracy
- [ ] `sutra health --json` mode for programmatic agent consumption
- [ ] Iterate CLI health output format based on real agent usage
- [ ] Sutra Studio health dashboard as Flutter landing page: overall status, per-index cards, action buttons

### Vectorized Execution
Done. All columnar scan functions now use SIMD-accelerated packed column scanning.

---

## Deferred

### SDK Publishing
Needs registry accounts — see `docs/SDK_ACCOUNTS_SETUP.md`.

- [ ] Python SDK → PyPI
- [ ] TypeScript SDK → npm
- [ ] Rust SDK → crates.io
- [ ] Java SDK → Maven Central
- [ ] C# SDK → NuGet
- [ ] Go SDK → tag for Go modules

### Sutra Studio
- [ ] Flutter graph view: remaining browse.html parity
- [ ] Long-term: absorb core Protege functionality

### Query Language Wrappers
Cypher and GQL as translation layers over SPARQL — the database still speaks SPARQL+ internally, but these graph query language wrappers let users query with familiar syntax. Each incoming query is parsed and transpiled to SPARQL before execution.

SQL and MQL are deliberately excluded: offering them would mislead AI agents into relational/document thinking when graph traversal is the correct approach.

- [ ] Cypher → SPARQL transpiler: MATCH/WHERE/RETURN mapped to SPARQL patterns
- [ ] GQL (ISO 39075) → SPARQL transpiler: ISO standard graph query language mapped to SPARQL
- [ ] Query validation: reject constructs that can't map to the RDF data model

### Premium Tier
Deferred until paying customers.

- [ ] RBAC
- [ ] Encryption at rest
- [ ] TLS
- [ ] Audit logging
- [ ] Replication
- [ ] Clustering / sharding
- [ ] Multi-tenancy
- [ ] Connection pooling

---

## Reference Architectures

| System | Why |
|--------|-----|
| [Qdrant](https://github.com/qdrant/qdrant) | HNSW impl, visited pools, normalize-at-insert |
| [Oxigraph](https://github.com/oxigraph/oxigraph) | RDF storage, SPO/POS/OSP, SPARQL pipeline |
| [DataFusion](https://github.com/apache/datafusion) | Cost-based planning, join ordering, vectorized execution |
| [DuckDB](https://github.com/duckdb/duckdb) | Columnar analytics, zonemap pruning, join ordering |
| [GlueSQL](https://github.com/gluesql/gluesql) | Small readable query engine |
| [Limbo](https://github.com/tursodatabase/limbo) | Rust SQLite reimpl, storage ideas |
| [Materialize](https://github.com/MaterializeInc/materialize) | Streaming SQL on Differential Dataflow |

---

## Completed (185 items)

<details>
<summary>Click to expand</summary>

### Query Engine Optimization
- [x] Cost-based query planning: cardinality estimation integrated into join ordering
- [x] Predicate pushdown: FILTERs repositioned after the pattern that binds their last variable
- [x] HNSW edge labeling: distinct predicates for vertical descent vs horizontal neighbor edges
- [x] HNSW typed edge filtering in executor (hnswHorizontalNeighbor, hnswLayerDescend)
- [x] Join strategy selection: cost-based hash join on subject, hash join on object, nested-loop
- [x] Object hash join: reverse-traversal optimization using POS/OSP indexes
- [x] Hash join threshold lowered from 100 to 50 for earlier amortization
- [x] Directional HNSW edge encoding for SPARQL property path traversal
- [x] Make virtual HNSW edge triples queryable in SPARQL patterns
- [x] Label vertical vs horizontal HNSW edges with distinct predicates
- [x] Encode directionality for property path descent/fan-out

### Database Health Dashboard
- [x] `sutra health` CLI command with AI-readable structured output
- [x] HNSW health: tombstone ratio, layer distribution, avg/min/max connectivity, entry point diversity
- [x] Pseudo-table health: coverage ratio, cliff steepness, segment count, avg tail properties
- [x] Storage health: triple count, term dictionary size, unique predicate count
- [x] HNSW rebuild via `sutra health --rebuild-hnsw`

### Pseudo-Tables & Vectorized Execution
- [x] Property model: predicate + position (Subject/Object) pairs per node
- [x] Property extraction: full graph scan to build PropertySet for every node
- [x] Group discovery: Jaccard-similarity merging of characteristic sets (≥80% overlap)
- [x] Pseudo-table materialization: columnar storage with ≥33% threshold columns, null support
- [x] Tail property tracking: per-row count of properties not in the pseudo-table schema
- [x] Cliff steepness metric: core/tail coverage ratio for schema health assessment
- [x] Per-column statistics: min/max/null_count/distinct_count (DataFusion Precision<T> pattern)
- [x] Segment-level storage: ~2048 rows per segment for zonemap granularity
- [x] Zonemap pruning: per-segment min/max skips entire segments
- [x] Row sorting by most selective column for tighter zonemaps
- [x] Vectorized column scans: scan_column_eq, scan_column_range, scan_column_not_null
- [x] SIMD-accelerated TermId comparison: packed columns (dense u64 + sentinel nulls), AVX2 (4 u64/cycle), SSE2 (2 u64/cycle)
- [x] Batch scan intersection: sorted merge for multi-column predicate evaluation
- [x] Query planner integration: recognize pseudo-table-matching SPARQL patterns
- [x] Expose health metrics via health endpoint / Sutra Studio

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
