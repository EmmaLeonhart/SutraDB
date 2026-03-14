# SutraDB — TODO

## Priority 1: Vector SPARQL Integration

The HNSW index and SPARQL engine are now connected.

- [x] Add `VECTOR_SIMILAR` as a recognized pattern in the SPARQL parser
- [x] Add `VECTOR_SCORE` as a recognized function in the SPARQL parser
- [x] Wire VECTOR_SIMILAR into the query executor (call HnswIndex.search())
- [x] Planner integration: detect bound/unbound subject for execution order
- [x] Support `ef:=N` hint parameter for per-query ef_search tuning
- [x] Support `k:=N` top-K mode (no threshold, return top K)
- [x] Parse `sutra:f32vec` typed literals into actual float arrays
- [ ] Schema declaration support (`sutra:declareVectorPredicate`) via SPARQL or config
- [x] Predicate-to-HnswIndex registry (map predicate IDs to their HNSW indexes)

## Priority 2: Data Ingestion — Experiment-Ready

Minimum viable path to loading real data (e.g. embedding-mapping project).

- [ ] N-Triples (.nt) parser for bulk import
- [ ] Turtle (.ttl) parser for bulk import
- [ ] NumPy (.npz) vector import — load embeddings from embedding-mapping project
- [ ] `sutra import` CLI command (`sutra import data.nt`, `sutra import --vectors embeddings.npz`)
- [ ] Wire PersistentStore to HTTP server (currently in-memory only)
- [ ] Vector predicate declaration via config file or SPARQL syntax
- [ ] SPARQL Update (INSERT DATA, DELETE DATA, DELETE/INSERT WHERE)
- [ ] Bulk vector insert endpoint (POST vectors via HTTP)
- [ ] Streaming import (line-by-line for large files)

## Priority 3: SPARQL Completeness

- [x] ORDER BY clause (ASC/DESC on variables and expressions)
- [x] UNION patterns
- [ ] BIND / VALUES
- [ ] GROUP BY / HAVING / aggregates (COUNT, SUM, AVG, MIN, MAX)
- [ ] Property paths (`+`, `*`, `?`) for multi-hop traversal
- [ ] Subqueries (nested SELECT)
- [ ] RDF-star quoted triple patterns in SPARQL (`<< ?s ?p ?o >>` syntax)
- [ ] CONSTRUCT queries (return triples instead of bindings)
- [ ] ASK queries (boolean existence check)
- [ ] DESCRIBE queries

## Priority 4: String and Function Support

- [ ] String functions: CONTAINS, STRSTARTS, STRENDS, STRLEN, SUBSTR
- [ ] REGEX filter support
- [ ] LANG() and LANGMATCHES() for language-tagged literals
- [ ] DATATYPE() function
- [ ] STR() cast function
- [ ] COALESCE()
- [ ] IF() conditional
- [ ] Arithmetic in expressions (+, -, *, /)
- [ ] Boolean operators in FILTER (&&, ||, !)

## Priority 5: Storage Engine

- [x] ~~LSM-tree decision~~ → Resolved: using sled for v0.1
- [ ] Integrate PersistentStore with the HTTP server (currently server uses in-memory only)
- [ ] Persistent term dictionary load/save in CLI
- [ ] HNSW index persistence (serialize to disk, memory-map on startup)
- [ ] HNSW compaction: background pass to remove deleted nodes when deleted_ratio > threshold
- [ ] Transaction support (atomic multi-triple inserts)
- [ ] Write-ahead log (WAL) for crash recovery
- [ ] Benchmark sled vs RocksDB for triple workloads

## Priority 6: HTTP Protocol

- [ ] Content negotiation for SPARQL results (JSON, XML, CSV, TSV)
- [ ] SPARQL results XML format (application/sparql-results+xml)
- [ ] SPARQL results CSV/TSV format
- [ ] Proper CORS configuration
- [ ] Authentication / API keys
- [ ] Rate limiting
- [ ] Query timeout enforcement
- [ ] SPARQL service description endpoint

## Priority 7: Distribution & Ecosystem

- [ ] Docker image on Docker Hub — one-command deployment (`docker run sutradb`)
- [ ] Protégé compatibility — OWL ontology editing via Protégé connected to SutraDB
- [ ] N-Quads (.nq) parser for named graphs
- [ ] RDF/XML parser (or use Oxigraph's oxrdfxml crate)
- [ ] JSON-LD parser (or use Oxigraph's oxjsonld crate)
- [ ] Graph Store Protocol (PUT/POST/DELETE graphs via HTTP)

## Priority 8: OWL Support

OWL is planned as an opt-in query-time layer (not stored inference).

- [ ] OWL class hierarchy resolution (rdfs:subClassOf transitive closure)
- [ ] OWL property hierarchy (rdfs:subPropertyOf)
- [ ] owl:equivalentClass
- [ ] owl:sameAs
- [ ] owl:inverseOf
- [ ] OWL restrictions (someValuesFrom, allValuesFrom)
- [ ] Reasoning toggle per-query (opt-in, not default)
- [ ] Materialization option (precompute inferences into stored triples)

## Priority 9: Performance

- [ ] Benchmarks with criterion (triple insert, query, HNSW search)
- [ ] SIMD distance functions (AVX2/SSE/NEON) for vector operations
- [ ] Visited pool pattern (pre-allocated visited lists for HNSW search)
- [ ] Builder/reader separation for HNSW (immutable index after construction)
- [ ] Parallel HNSW construction (rayon)
- [ ] Query result streaming (don't collect all results before returning)
- [ ] Connection pooling for persistent store
- [ ] Prefix compression for IRI storage (common prefixes stored once)

## Priority 10: Tooling

- [ ] `sutra import` CLI command
- [ ] `sutra export` CLI command (dump to Turtle/N-Triples)
- [ ] `sutra bench` CLI command (built-in benchmarks)
- [ ] `sutra info` CLI command (database stats: triple count, term count, index sizes)
- [ ] Docker image
- [ ] Configuration file (TOML) for server settings
- [ ] Logging configuration (structured JSON logs)

## Test Data: embedding-mapping Project

The `embedding-mapping` project (`C:\Users\Immanuelle\Documents\Github\embedding-mapping`) has real data ready to load:

- **triples.nt** — 733KB of N-Triples RDF (Wikidata triples about mountains, shrines, geography)
- **geodesics.ttl** — 3.6MB Turtle RDF (8,832 geodesic objects with distance metrics)
- **embeddings.npz** — 585MB NumPy file (79,318 vectors × 1024 dimensions, mxbai-embed-large model)
- **embedding_index.json** — maps vector positions to (QID, text, type)
- **items.json** — 28,307 Wikidata items with labels, aliases, and triples

This is the first real-world test: load the triples, attach the 1024-dim embeddings, and run combined graph+vector SPARQL queries over the data.

## Open Architecture Questions

- ~~**RDF-star vs. RDF 1.2**~~: **Resolved: RDF-star.** Direct edge annotation (`<< s p o >> :hasEmbedding ...`) is the natural pattern for vector/embedding work. RDF 1.2's object-only restriction adds unnecessary indirection. If compatibility is ever needed, a translation layer can handle it.
- **IRI encoding strategy**: Current sequential interning vs. Oxigraph's hash-based approach (128-bit SipHash — no collision issues at scale, eliminates separate string→ID index). Hash-based is simpler but loses ordering; sequential preserves insertion order for range queries.
- **HNSW compaction threshold**: What deleted_ratio triggers a rebuild? 0.3? 0.5? Should it be configurable?
- **SPARQL property paths**: How to handle cycles on large graphs? Depth limit? Visited set?
- **Adaptive query execution**: Runtime reordering based on intermediate cardinalities — worth the complexity for v0.2?
- **Named graphs**: Support GRAPH clause and quad storage? Adds complexity but needed for provenance. Oxigraph supports named graphs with 6 extra indexes (SPOG, POSG, OSPG, GSPO, GPOS, GOSP).
- **Blank node handling**: Skolemization vs. internal IDs? How to handle blank nodes across imports?
- **RDF parsing crates**: Write our own parsers or use Oxigraph's crates (oxttl, oxrdfxml, oxjsonld) for data ingestion?

## Resolved

- [x] License → Apache 2.0
- [x] Storage engine → sled for v0.1
- [x] OWL → planned as opt-in query-time, not out of scope
- [x] Naming → SutraDB
- [x] Query language policy → SPARQL primary, Cypher planned as wrapper. SQL and MongoQL permanently out of scope.
- [x] Reference architecture → Oxigraph (https://github.com/oxigraph/oxigraph) as implementation reference for storage, indexing, and SPARQL patterns
- [x] RDF data model → RDF-star (superset of RDF 1.2). Triple terms allowed in any position. Direct edge annotation is the natural pattern for vector/embedding work.
- [x] Vector SPARQL integration → VECTOR_SIMILAR, VECTOR_SCORE, VectorRegistry, ORDER BY, UNION all implemented (135→156 tests)
