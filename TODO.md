# SutraDB — TODO

## Priority 1: Vector SPARQL Integration

The HNSW index and SPARQL engine both exist — they need to be connected.

- [ ] Add `VECTOR_SIMILAR` as a recognized pattern in the SPARQL parser
- [ ] Add `VECTOR_SCORE` as a recognized function in the SPARQL parser
- [ ] Wire VECTOR_SIMILAR into the query executor (call HnswIndex.search())
- [ ] Planner integration: detect bound/unbound subject for execution order
- [ ] Support `ef:=N` hint parameter for per-query ef_search tuning
- [ ] Support `k:=N` top-K mode (no threshold, return top K)
- [ ] Parse `sutra:f32vec` typed literals into actual float arrays
- [ ] Schema declaration support (`sutra:declareVectorPredicate`)
- [ ] Predicate-to-HnswIndex registry (map predicate IDs to their HNSW indexes)

## Priority 2: SPARQL Completeness

- [ ] ORDER BY clause (ASC/DESC on variables and expressions)
- [ ] UNION patterns
- [ ] BIND / VALUES
- [ ] GROUP BY / HAVING / aggregates (COUNT, SUM, AVG, MIN, MAX)
- [ ] Property paths (`+`, `*`, `?`) for multi-hop traversal
- [ ] Subqueries (nested SELECT)
- [ ] RDF-star quoted triple patterns in SPARQL (`<< ?s ?p ?o >>` syntax)
- [ ] CONSTRUCT queries (return triples instead of bindings)
- [ ] ASK queries (boolean existence check)
- [ ] DESCRIBE queries

## Priority 3: String and Function Support

- [ ] String functions: CONTAINS, STRSTARTS, STRENDS, STRLEN, SUBSTR
- [ ] REGEX filter support
- [ ] LANG() and LANGMATCHES() for language-tagged literals
- [ ] DATATYPE() function
- [ ] STR() cast function
- [ ] COALESCE()
- [ ] IF() conditional
- [ ] Arithmetic in expressions (+, -, *, /)
- [ ] Boolean operators in FILTER (&&, ||, !)

## Priority 4: Storage Engine

- [x] ~~LSM-tree decision~~ → Resolved: using sled for v0.1
- [ ] Integrate PersistentStore with the HTTP server (currently server uses in-memory only)
- [ ] Persistent term dictionary load/save in CLI
- [ ] HNSW index persistence (serialize to disk, memory-map on startup)
- [ ] HNSW compaction: background pass to remove deleted nodes when deleted_ratio > threshold
- [ ] Transaction support (atomic multi-triple inserts)
- [ ] Write-ahead log (WAL) for crash recovery
- [ ] Benchmark sled vs RocksDB for triple workloads

## Priority 5: Data Ingestion

- [ ] Turtle (.ttl) parser for bulk import
- [ ] N-Triples (.nt) parser for bulk import
- [ ] N-Quads (.nq) parser for named graphs
- [ ] RDF/XML parser (or use an existing crate)
- [ ] JSON-LD parser
- [ ] SPARQL Update (INSERT DATA, DELETE DATA, DELETE/INSERT WHERE)
- [ ] Graph Store Protocol (PUT/POST/DELETE graphs via HTTP)
- [ ] Bulk import CLI command (`sutra import data.ttl`)
- [ ] Streaming import (line-by-line for large files)

## Priority 6: HTTP Protocol

- [ ] Content negotiation for SPARQL results (JSON, XML, CSV, TSV)
- [ ] SPARQL results XML format (application/sparql-results+xml)
- [ ] SPARQL results CSV/TSV format
- [ ] Proper CORS configuration
- [ ] Authentication / API keys
- [ ] Rate limiting
- [ ] Query timeout enforcement
- [ ] SPARQL service description endpoint

## Priority 7: OWL Support

OWL is planned as an opt-in query-time layer (not stored inference).

- [ ] OWL class hierarchy resolution (rdfs:subClassOf transitive closure)
- [ ] OWL property hierarchy (rdfs:subPropertyOf)
- [ ] owl:equivalentClass
- [ ] owl:sameAs
- [ ] owl:inverseOf
- [ ] OWL restrictions (someValuesFrom, allValuesFrom)
- [ ] Reasoning toggle per-query (opt-in, not default)
- [ ] Materialization option (precompute inferences into stored triples)

## Priority 8: Performance

- [ ] Benchmarks with criterion (triple insert, query, HNSW search)
- [ ] SIMD distance functions (AVX2/SSE/NEON) for vector operations
- [ ] Visited pool pattern (pre-allocated visited lists for HNSW search)
- [ ] Builder/reader separation for HNSW (immutable index after construction)
- [ ] Parallel HNSW construction (rayon)
- [ ] Query result streaming (don't collect all results before returning)
- [ ] Connection pooling for persistent store
- [ ] Prefix compression for IRI storage (common prefixes stored once)

## Priority 9: Tooling

- [ ] `sutra import` CLI command
- [ ] `sutra export` CLI command (dump to Turtle/N-Triples)
- [ ] `sutra bench` CLI command (built-in benchmarks)
- [ ] `sutra info` CLI command (database stats: triple count, term count, index sizes)
- [ ] Docker image
- [ ] Configuration file (TOML) for server settings
- [ ] Logging configuration (structured JSON logs)

## Open Architecture Questions

- **HNSW compaction threshold**: What deleted_ratio triggers a rebuild? 0.3? 0.5? Should it be configurable?
- **SPARQL property paths**: How to handle cycles on large graphs? Depth limit? Visited set?
- **Adaptive query execution**: Runtime reordering based on intermediate cardinalities — worth the complexity for v0.2?
- **Named graphs**: Support GRAPH clause and quad storage? Adds complexity but needed for provenance.
- **Blank node handling**: Skolemization vs. internal IDs? How to handle blank nodes across imports?

## Resolved

- [x] License → Apache 2.0
- [x] Storage engine → sled for v0.1
- [x] OWL → planned as opt-in query-time, not out of scope
- [x] Naming → SutraDB
