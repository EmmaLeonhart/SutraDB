# SutraDB — TODO

## Remaining Items (33 of 176 total)

### Parser & Format Support
- [x] Turtle (.ttl) parser — handles @prefix, prefixed names, ;/, lists, lang tags, typed literals, blank nodes
- [x] RDF/XML parser — handles rdf:Description, rdf:about, rdf:resource, namespace expansion, literal content
- [x] JSON-LD parser — handles @context, @id, @type, @value/@language, nested nodes, arrays
- [ ] Arithmetic in FILTER expressions (+, -, *, /) — needs expression AST

### Query Performance
- [ ] Parallel HNSW construction (rayon) for faster bulk vector insert
- [ ] Materialized adjacency lists (Neo4j-style node→edge lists)
- [ ] Query result streaming (don't collect all results before returning)
- [x] Adaptive query execution: hash join auto-triggers at >100 rows; planner + cardinality estimation handle most cases statically

### SDK Publishing (needs registry accounts — see docs/SDK_ACCOUNTS_SETUP.md)
- [ ] Python SDK: publish to PyPI
- [ ] TypeScript SDK: publish to npm
- [ ] Rust SDK: publish to crates.io
- [ ] Java SDK: publish to Maven Central
- [ ] C# SDK: publish to NuGet
- [ ] Go SDK: tag for Go modules

### Sutra Studio (Flutter)
- [ ] Flutter graph view: remaining browse.html parity work
- [ ] Per-cluster PageRank health metric
- [ ] Edge traversal counters (per-edge hit counts)
- [ ] HNSW cluster heatmap visualization
- [x] Graph export (screenshot hint; full RepaintBoundary→PNG is future work)
- [x] Dark/light theme toggle (icon in nav rail, SutraStudioApp state)
- [ ] Backup management via Sutra Studio UI
- [ ] Long-term: absorb core Protege functionality into Sutra Studio

### Benchmarking & Evaluation
- [x] Benchmark sled performance baseline (20K inserts/sec, <1ms queries, 40ms full export)
- [x] IRI encoding: sequential interning is simpler, deterministic, and fast enough (0.65ms point lookup). Hash-based adds complexity for marginal benefit at current scale. Revisit at >10M triples.
- [ ] Prefix compression for IRI storage — low priority, dictionary already compact

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

## Completed (143 items)

<details>
<summary>Click to expand completed items</summary>

### Stress Test Fixes
- [x] Type lookup 2s → FIXED (find_by_predicate_object)
- [x] 2-hop joins 18s → FIXED (LIMIT push-down: 18.453s → 0.003s)
- [x] HNSW defaults → FIXED (ef_search 200→500)
- [x] First query cold start → FIXED (HashSet visited list)
- [x] HNSW cross-cluster search → FIXED (multiple entry points)

### Core Infrastructure
- [x] Database configuration model
- [x] HNSW edges as RDF triples (virtual view)
- [x] SPARQL executor intercepts sutra:hnswNeighbor
- [x] VECTOR_SIMILAR + VECTOR_SCORE in parser/executor
- [x] Planner: bound/unbound subject detection, ef/k hints
- [x] Parse sutra:f32vec typed literals
- [x] VectorRegistry (predicate → HnswIndex)
- [x] ORDER BY, UNION, N-Triples parser
- [x] POST /triples, /vectors/declare, /vectors endpoints
- [x] Vector architecture: vectors are graph objects
- [x] find_by_predicate_object for reverse lookup

### Persistence
- [x] PersistentStore (sled) wired to HTTP server
- [x] Persistent term dictionary
- [x] HNSW rebuilt from vector triples on startup
- [x] .sdb is sled directory with all data
- [x] sutra serve/query loads from disk

### Parser & Ingestion
- [x] Blank node support, N-Quads parser
- [x] sutra import/export CLI, streaming import
- [x] SPARQL Update (INSERT DATA, DELETE DATA)
- [x] Schema declaration via SPARQL

### SPARQL Completeness
- [x] BIND/VALUES, GROUP BY/HAVING/aggregates
- [x] Property paths (+, *, ?, /), Subqueries
- [x] RDF-star quoted triple patterns
- [x] CONSTRUCT, DESCRIBE, ASK queries
- [x] String functions, REGEX, LANG/LANGMATCHES
- [x] DATATYPE, STR, COALESCE, IF
- [x] Boolean operators, comparison >=/<=/isIRI/isLiteral
- [x] FILTER NOT EXISTS/EXISTS

### Performance
- [x] SIMD distance functions (AVX2/FMA + SSE)
- [x] Hash joins for large intermediate results
- [x] Cardinality estimation
- [x] Query timeout enforcement
- [x] HNSW compaction (rebuild without tombstones)
- [x] Crash recovery (verify_consistency + repair)
- [x] Named graph support (Triple::quad)
- [x] Wormhole query optimization

### HTTP & Server
- [x] Content negotiation (Accept header → JSON/XML/CSV/TSV)
- [x] SPARQL results XML, CSV, TSV formats
- [x] Passcode authentication, rate limiting
- [x] HNSW health endpoint, service description
- [x] Periodic backups, Graph Store Protocol
- [x] GET /graph (Turtle/N-Triples export)

### SDKs & Ecosystem
- [x] 6 SDK scaffolds + endpoint fixes
- [x] Python OWL validation (domain/range/subclass/disjoint/equivalent/sameAs/inverse)
- [x] OWL verification query generation
- [x] Integration test CI, publish workflow
- [x] LangChain VectorStore, Jupyter %%sparql magic
- [x] MCP server (6 tools), agent installer CLI
- [x] Protege plugin, Dockerfile, install scripts

### Sutra Studio
- [x] Flutter scaffold, Dart client, force-directed graph
- [x] View modes, triple editor, SPARQL editor, ontology viewer
- [x] HNSW health diagnostics from /vectors/health
- [x] IRI shortening, click-to-expand, predicate filtering
- [x] Triple list side panel, Japanese labels
- [x] HNSW virtual edges, persistent settings
- [x] OWL export, Windows desktop support

### Data & Testing
- [x] 82K triples + 79K vectors (embedding-mapping)
- [x] 500K triples + 1M vectors stress test
- [x] 439 Wikidata entities BFS import (16K triples)
- [x] 435 Japanese label embeddings (mxbai-embed-large)
- [x] Benchmark suite (sub-millisecond query performance)
- [x] OWL validation tests (Python SDK)

### Documentation & Website
- [x] Agent setup guide, SDK publishing guide, SDK accounts guide
- [x] README updated, session notes
- [x] Open Graph meta tags on all pages
- [x] AI agent callout on website

</details>

## Benchmark Results (16K triples, 435 vectors)

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
