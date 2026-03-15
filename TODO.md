# SutraDB — TODO

## Top Priority: Remaining Stress Test Issues

### ~~1. Type lookup 2s~~ — FIXED (index selection: `find_by_predicate_object`)
### ~~2. 2-hop joins 18s~~ — FIXED (LIMIT push-down: 18.453s → 0.003s)
### ~~3. HNSW defaults~~ — FIXED (ef_search 200→500, top_k 100→500)

### ~~4. First query cold start ~2s~~ — FIXED (HashSet visited list instead of dense Vec<bool>)
### ~~5. HNSW cross-cluster search returns 0 rows~~ — FIXED (multiple entry points, best-of-N search start)

## Done

- [x] Database configuration model (RdfMode: Star/1.2/Legacy, HnswEdgeMode: Virtual/Materialized, OWL toggle)
- [x] HNSW edges as RDF triples — virtual view via `sutra:hnswNeighbor` predicate
- [x] HNSW edge generation API: `edge_triples()`, `edge_triples_for_source()`, `edge_triples_for_target()`
- [x] SPARQL executor intercepts `sutra:hnswNeighbor` for virtual HNSW edge queries
- [x] `execute_with_config()` API for full database configuration control
- [x] Stress tests: chain traversals (2/3/4-hop), large joins (1K/5K leaves), grid traversals, self-joins, combined vector+graph 3-hop
- [x] VECTOR_SIMILAR pattern in SPARQL parser
- [x] VECTOR_SCORE function in SPARQL parser
- [x] Wire VECTOR_SIMILAR into executor (calls HnswIndex.search())
- [x] Planner integration: bound/unbound subject detection
- [x] ef:=N and k:=N hint parameters
- [x] Parse sutra:f32vec typed literals into float arrays
- [x] Predicate-to-HnswIndex registry (VectorRegistry)
- [x] ORDER BY clause (ASC/DESC)
- [x] UNION patterns
- [x] N-Triples parser (sutra-core/ntriples.rs)
- [x] POST /triples endpoint (bulk N-Triples insert)
- [x] POST /vectors/declare endpoint
- [x] POST /vectors endpoint (creates triple + HNSW entry)
- [x] Vector architecture: vectors are graph objects, multiple subjects can share a vector
- [x] VECTOR_SIMILAR resolves back through POS index for entity resolution
- [x] find_by_predicate_object() for vector reverse lookup
- [x] First real data load: 82K triples + 79K vectors from embedding-mapping
- [x] Stress test: 500K triples + 1M vectors (128-dim)
- [x] Client SDKs scaffolded: Python, TypeScript, Rust, Java, C#, Go
- [x] CI workflow: core Rust + all 6 SDK builds/tests
- [x] GitHub Pages: landing page + 8 subpages
- [x] Release workflow: cross-platform builds (Windows/Linux/macOS)
- [x] Graph browser: D3 force-directed visualization (tools/browse.html)
- [x] Serverless-by-default philosophy, .sdb file extension
- [x] License: Apache 2.0
- [x] Reference architecture: Oxigraph
- [x] RDF-star (superset of RDF 1.2)
- [x] Query language policy: SPARQL primary, Cypher planned, SQL/MongoQL never
- [x] 4 index types documented: SPO, POS, OSP, VECTOR(p)

## Priority 1: Persistence — Data Survives Restart

The server is currently in-memory only. This is the #1 blocker.

- [x] Wire PersistentStore (sled) to the HTTP server instead of in-memory TripleStore
- [x] Persistent term dictionary: load on startup, save on insert
- [ ] HNSW index persistence: serialize to disk, memory-map on startup
- [ ] The .sdb file should contain all of the above in one directory/file
- [ ] `sutra serve --data my.sdb` loads from disk, writes back on changes
- [ ] `sutra query --data my.sdb` opens serverless (no HTTP)

## Priority 2: Parser & Ingestion Gaps

- [x] Blank node support in N-Triples parser (`_:b0`, `_:genid123`)
- [ ] Turtle (.ttl) parser for bulk import (consider using Oxigraph's oxttl crate)
- [ ] `sutra import` CLI command (`sutra import data.nt --data my.sdb`)
- [ ] `sutra export` CLI command (dump to Turtle/N-Triples)
- [ ] SPARQL Update (INSERT DATA, DELETE DATA, DELETE/INSERT WHERE)
- [ ] Schema declaration via SPARQL (`sutra:declareVectorPredicate`)
- [ ] Streaming import (line-by-line for large files without loading all into memory)

## Priority 3: Query Performance — Stress Test Findings

The 1M-vector stress test revealed specific bottlenecks:

- [x] HNSW search took 281s at 1M scale — needs SIMD distance functions (AVX2/SSE/NEON) ✅ Implemented AVX2+FMA, SSE, scalar fallback
- [ ] 3-hop joins timeout at 500K triples — nested loop join is O(n^3)
  - [ ] Cardinality estimation: count triples per subject/predicate/object for cost-based planning
  - [ ] Hash joins for large intermediate result sets (instead of nested loop)
  - [ ] Index selection: use the most selective index first based on cardinality stats
- [ ] Wormhole queries (vector→graph→graph) need the planner to push vector results into bound positions before graph joins
- [ ] Query timeout enforcement (kill long-running queries after N seconds)
- [ ] Parallel HNSW construction (rayon) for faster bulk vector insert

## Priority 4: SPARQL Completeness

- [ ] BIND / VALUES
- [ ] GROUP BY / HAVING / aggregates (COUNT, SUM, AVG, MIN, MAX)
- [ ] Property paths (`+`, `*`, `?`) for multi-hop traversal
- [ ] Subqueries (nested SELECT)
- [ ] RDF-star quoted triple patterns in SPARQL (`<< ?s ?p ?o >>` syntax)
- [ ] CONSTRUCT queries (return triples instead of bindings)
- [ ] ASK queries (boolean existence check)
- [ ] DESCRIBE queries
- [ ] String functions: CONTAINS, STRSTARTS, STRENDS, STRLEN, SUBSTR
- [ ] REGEX filter support
- [ ] LANG() and LANGMATCHES() for language-tagged literals
- [ ] DATATYPE(), STR(), COALESCE(), IF()
- [ ] Arithmetic in expressions (+, -, *, /)
- [ ] Boolean operators in FILTER (&&, ||, !)

## Priority 5: SDK Quality

SDKs exist but need real integration testing and polish.

- [ ] Integration tests for each SDK against a running SutraDB instance
- [ ] Python SDK: publish to PyPI
- [ ] TypeScript SDK: publish to npm
- [ ] Rust SDK: publish to crates.io
- [ ] Java SDK: publish to Maven Central
- [ ] C# SDK: publish to NuGet
- [ ] Go SDK: tag for Go modules
- [ ] CI: start SutraDB as a service in CI, run SDK integration tests against it

## Priority 6: Distribution & Ecosystem

- [ ] Docker image on Docker Hub (`docker run sutradb/sutra --port 3030`)
- [ ] Protégé plugin — connect OWL ontology editor to SutraDB's SPARQL endpoint
- [ ] Jupyter integration (%%sparql cell magic, inline result rendering)
- [ ] LangChain / LlamaIndex integration (SutraDB as vector store + knowledge graph for RAG)

## Priority 7: OWL Support (blocked on Protégé plugin)

OWL constraints treated as **explicit verification queries**, not automatic inference.
Users model ontologies in Protégé, SutraDB validates data against them at query time.
This avoids reasoner explosion while still letting users verify their models.
Blocked on Priority 6 Protégé plugin — the two should be designed together.

- [ ] OWL class hierarchy resolution (rdfs:subClassOf transitive closure)
- [ ] OWL property hierarchy (rdfs:subPropertyOf)
- [ ] owl:equivalentClass
- [ ] owl:sameAs
- [ ] owl:inverseOf
- [ ] OWL restrictions (someValuesFrom, allValuesFrom)
- [ ] Verification query generation: given an OWL ontology, produce SPARQL queries that check constraint violations
- [ ] Reasoning toggle per-query (opt-in, not default)
- [ ] Materialization option (precompute inferences into stored triples)

## Priority 7.5: Sutra Studio — Flutter Desktop/Web Client

Visual database management tool (like MongoDB Compass). Primary use: human visual
intuition for things AI agents can't easily detect — broken HNSW clusters, graph
drift, tombstone accumulation.

- [x] Flutter project scaffold (`sutra-studio/`)
- [x] Dart HTTP client mirroring TypeScript SDK interface
- [x] Force-directed graph visualization (custom Canvas painter)
- [x] View mode toggle: semantic only / vector only / all
- [x] Triple table editor with add/delete (form + raw N-Triples)
- [x] SPARQL query editor with quick templates
- [x] Ontology viewer (Protege-like class hierarchy browser)
- [x] Authentication settings page (ready for server-side auth)
- [x] Database health dashboard (connection status, stats)
- [ ] HNSW health diagnostics: degree distribution visualization
- [ ] HNSW health diagnostics: tombstone ratio monitoring with rebuild recommendations
- [ ] Per-cluster PageRank health metric (detect drift from heavy insert/delete)
- [ ] Edge traversal counters (per-edge hit counts for HNSW and semantic edges)
- [ ] HNSW cluster heatmap visualization
- [ ] Automatic rebuild recommendation threshold (configurable)
- [ ] Graph export (PNG/SVG of current visualization)
- [ ] Dark/light theme toggle
- [ ] Persistent connection settings (shared_preferences)

## Priority 8: HTTP Protocol

- [ ] Content negotiation for SPARQL results (JSON, XML, CSV, TSV)
- [ ] SPARQL results XML format (application/sparql-results+xml)
- [ ] SPARQL results CSV/TSV format
- [ ] Authentication / API keys (needed for Sutra Studio auth screen)
- [ ] Rate limiting
- [ ] HNSW health endpoint: `/vectors/health` — degree distribution, tombstone ratio, rebuild recommendation
- [ ] SPARQL service description endpoint

## Priority 9: Additional Storage & Format Support

- [ ] N-Quads (.nq) parser for named graphs
- [ ] Named graph support (GRAPH clause, quad storage)
- [ ] RDF/XML parser (or use Oxigraph's oxrdfxml crate)
- [ ] JSON-LD parser (or use Oxigraph's oxjsonld crate)
- [ ] Graph Store Protocol (PUT/POST/DELETE graphs via HTTP)
- [ ] Benchmark sled vs RocksDB for triple workloads
- [ ] IRI encoding: evaluate hash-based (Oxigraph SipHash) vs current sequential interning

## Priority 10: Advanced Performance

- [x] SIMD distance functions (AVX2/SSE/NEON) for vector operations ✅
- [ ] Materialized adjacency lists (Neo4j-style node→edge lists) — currently all traversals use SPO/OSP prefix scans; adjacency lists could close the ~10× gap vs property graph traversal speed
- [ ] Visited pool pattern (pre-allocated visited lists for HNSW search)
- [ ] Builder/reader separation for HNSW (immutable index after construction)
- [ ] Query result streaming (don't collect all results before returning)
- [ ] Prefix compression for IRI storage (common prefixes stored once)
- [ ] HNSW compaction: background pass to remove deleted nodes when deleted_ratio > threshold
- [ ] Write-ahead log (WAL) for crash recovery
- [ ] Adaptive query execution: runtime reordering based on intermediate cardinalities

## Open Architecture Questions

- **HNSW compaction threshold**: What deleted_ratio triggers a rebuild? 0.3? 0.5? Configurable?
- **SPARQL property paths**: How to handle cycles on large graphs? Depth limit? Visited set?
- **Named graphs**: GRAPH clause + quad storage adds complexity but needed for provenance.
- **Blank node handling**: Skolemization vs. internal IDs? How to handle blank nodes across imports?
- **RDF parsing crates**: Write our own parsers or use Oxigraph's crates (oxttl, oxrdfxml, oxjsonld)?
- **IRI encoding**: Sequential interning (current) vs. hash-based (Oxigraph SipHash)?

## Test Data: embedding-mapping Project

The `embedding-mapping` project has real data loaded:

- **82,177 triples** from 28,307 Wikidata items (mountains, shrines, geography)
- **79,318 vectors** (1024-dim, mxbai-embed-large)
- Geodesics not yet loaded (blocked on blank node support in N-Triples parser)
- Import pipeline: `import_to_sutra.py --load-existing`

## Stress Test Results (1M scale)

- **500,303 triples** + **1,000,000 vectors** (128-dim) — zero insertion errors
- Vector insertion: 762/sec sustained
- Type-filtered lookups: 6ms
- 1K result sets: 11ms
- 2-hop traversals: 8.5s at 500K scale
- VECTOR_SIMILAR over 1M vectors: 281s (needs SIMD)
- 3-hop joins: timeout at 500K scale (needs query plan optimization)
