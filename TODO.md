# SutraDB — TODO

**Status: 220 of 241 items complete (91%)**

---

## Next Release (v0.3.1) — Gradle Migration, MCP Agentic UX, Maven Central

Merge the Gradle migration (local) and MCP agentic UX work (claude.ai remote session) then cut v0.3.1.

### Release Checklist
- [ ] Merge claude.ai remote branch (MCP agentic UX work) into main
- [ ] Merge Gradle migration + Maven Central publishing setup (local commits)
- [ ] Bump version to 0.3.1 in `sdks/java/build.gradle.kts` and all other SDK configs
- [ ] Set up Maven Central secrets: `MAVEN_USERNAME`, `MAVEN_TOKEN`, `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE`
- [ ] Generate GPG key and upload public key to keyserver
- [ ] Tag `v0.3.1` and push to trigger publish workflow
- [ ] Verify `io.github.emmaleonhart:sutradb:0.3.1` appears on Maven Central

### Java/Kotlin SDK — Maven Central Ready
The SDK is functionally complete (3 classes, ~400 LOC). Build migrated from Maven to Gradle (Kotlin DSL).

- [x] JUnit 5 test suite: 24 unit tests with HTTP mocking for all SutraClient methods
- [x] Add `rebuildHnsw()` method (calls `POST /vectors/rebuild`)
- [x] Add `healthReport()` method (calls `GET /health` + `GET /vectors/health`)
- [x] Bump version to 0.3.0 (match main project)
- [x] Migrate from Maven (pom.xml) to Gradle (Kotlin DSL)
- [x] Switch to Sonatype Central Portal (`central-publishing-maven-plugin` → Gradle `maven-publish`)
- [x] In-memory GPG signing (no GPG binary needed in CI)
- [x] GroupId: `io.github.emmaleonhart`, artifact: `sutradb`
- [ ] Integration test: start SutraDB, insert triples, query, verify round-trip
- [ ] OWL validation (match Python SDK: domain/range/subclass/disjoint/equivalent)
- [ ] Connection retry logic with configurable timeouts
- [ ] First publish to Maven Central

---

## Future Versions

### AI Agent Installer (remaining)
- [ ] End-to-end test: fresh install → insert → query → verify
- [ ] Serverless mode testing (no `--serve`, just create the `.sdb`)
- [ ] Agent-consumable structured output (JSON mode for programmatic setup)

### HNSW Traversal via SPARQL Property Paths
- [ ] Greedy descent + beam search semantics from graph structure and property path evaluation
- [ ] Test: `sutra:hnswNeighbor+` produces correct ANN results

### Predicate-Based Exit Conditions (UNTIL)
- [ ] Design UNTIL syntax for exit conditions on property path traversal
- [ ] Per-step predicate evaluation during traversal (not post-filter)
- [ ] Backtracking interaction (exit on one branch doesn't kill others)
- [ ] Ordered traversal (exit conditions require defined traversal order)
- [ ] HNSW-specific exit: "no closer neighbor found" (local optimality termination)
- [ ] Test: ordered traversal with UNTIL produces correct early termination

### Cost-Based Query Planning (remaining)
- [ ] HNSW as access path: planner chooses "HNSW index scan" vs "SPO triple scan" based on cost
- [ ] Adaptive execution: observe intermediate result sizes at runtime, reorder mid-query

### Background Maintenance Cycle
- [ ] Low-usage detection heuristic (query rate below threshold for N seconds)
- [ ] Background HNSW rebuild: fresh graph from current vectors, old graph serves queries until swap
- [ ] Atomic swap: replace old HNSW with rebuilt one
- [ ] Background pseudo-table rediscovery and rebuild

### Pseudo-Tables (remaining)
- [ ] Invalidation tracking: flag stale rows when interior nodes change, rebuild during maintenance cycle
- [ ] Update query planner to recognize multi-pattern SPARQL queries that match a subgraph pseudo-table

### Database Health Dashboard (remaining)
- [ ] Query performance metrics: per-pattern latency percentiles, planner decision accuracy
- [ ] `sutra health --json` mode for programmatic agent consumption
- [ ] Iterate CLI health output format based on real agent usage
- [ ] Sutra Studio health dashboard as Flutter landing page: overall status, per-index cards, action buttons

### SDK Publishing
- [ ] Python SDK → PyPI
- [ ] TypeScript SDK → npm
- [ ] Rust SDK → crates.io
- [ ] C# SDK → NuGet
- [ ] Go SDK → tag for Go modules

### Sutra Studio
- [ ] Flutter graph view: remaining browse.html parity
- [ ] Long-term: absorb core Protege functionality

### Query Language Wrappers
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

### ACID Compliance
- [x] Atomicity: sled multi-tree transactions for SPO/POS/OSP insert and remove
- [x] Consistency: startup verification (verify_consistency + repair) on persistent store open
- [x] Isolation: PersistentStore wrapped in RwLock; vector inserts hold store+vectors locks together
- [x] Durability: explicit flush() after all server mutation endpoints before returning success
- [x] Error propagation: all persistent write errors reported to caller (no silent `let _ =`)
- [x] GSP DELETE clears persistent store and flushes

### Native MCP Server
- [x] `sutra mcp` command: native Rust MCP server built into the binary (no Python needed)
- [x] Dual-mode: `--url` for server mode, `--data-dir` for serverless mode
- [x] 8 tools: health_report, rebuild_hnsw, verify_consistency, database_info, sparql_query, insert_triples, backup, vector_search

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
