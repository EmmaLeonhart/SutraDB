# Session Notes — 2026-03-15

## Summary
Major development session covering SPARQL completeness, persistence, HNSW fixes, Protégé integration, SDK fixes, and a real-world Wikidata import.

## Commits (23 total)

### Core Engine
1. **SIMD distance functions** — AVX2/FMA + SSE fallback for HNSW dot_product, squared_euclidean, l2_norm
2. **First-query cold start fix** — Replaced dense Vec<bool> visited list with HashSet (eliminated ~2s page fault overhead at 200K+ nodes)
3. **HNSW cross-cluster search** — Multiple entry points (up to 8), score all EPs and start from best. Fixes bias toward first-inserted cluster.
4. **Persistence** — PersistentStore (sled) wired to HTTP server with write-through. In-memory stores hydrated on startup.
5. **Blank node support** — N-Triples parser now handles `_:label` in subject/object positions
6. **Query timeout** — execute_with_timeout() with per-pattern deadline checks, SparqlError::Timeout

### SPARQL Completeness
7. **FILTER NOT EXISTS / EXISTS** — Sub-pattern evaluation with LIMIT 1 push-down
8. **ASK queries** — Returns single boolean row
9. **GROUP BY + aggregates** — COUNT, SUM, AVG, MIN, MAX with DISTINCT support
10. **BIND / VALUES** — BIND(term AS ?var) and VALUES ?var { val1 val2 }
11. **Boolean operators** — &&, ||, ! in FILTER expressions
12. **String functions** — CONTAINS, STRSTARTS, STRENDS, REGEX (substring)
13. **Comparison operators** — >=, <= added
14. **Type checks** — isIRI(), isLiteral()
15. **LANG() / LANGMATCHES()** — Language tag filtering for multilingual data
16. **SPARQL Update** — INSERT DATA { triples } and DELETE DATA { triples }

### CLI & Distribution
17. **sutra import** — Streaming line-by-line N-Triples import to sled
18. **sutra export** — Dump all triples as N-Triples or Turtle
19. **sutra info** — Show triple/term counts
20. **Install scripts** — install.bat (Windows), install.sh (Linux/macOS)
21. **Dockerfile** — Multi-stage build, exposes 3030, /data volume

### HTTP Protocol
22. **GET /graph** — Turtle/N-Triples export (Protégé integration point)
23. **/sparql.csv, /sparql.tsv** — Delimited result formats
24. **Service description** — /service-description endpoint (Turtle)

### Ecosystem
25. **Protégé plugin** — Java OSGi plugin: Connect/Start, Load from SutraDB, Save to SutraDB, OWL Validate
26. **SDK endpoint fixes** — All 4 SDKs (Go, Rust, Java, .NET) had /store and /vectors/insert instead of /triples and /vectors
27. **.gitattributes** — GitHub Linguist now counts SDK languages
28. **Python pyproject.toml** — Added [dev] extras for CI pytest

### Data Import
29. **Wikidata BFS import script** — tools/wikidata_bfs_import.py with Ollama embeddings
30. **Import results** — 439 entities, 16,084 triples, 439 vectors (1024-dim mxbai-embed-large) from Engishiki Jinmyōchō (Q11064932) BFS, 0 errors, 7,316 entities remaining in queue

## TODO Items Completed
- [x] First query cold start ~2s
- [x] HNSW cross-cluster search
- [x] PersistentStore wired to server
- [x] Persistent term dictionary
- [x] Blank node support
- [x] sutra import / export CLI
- [x] Streaming import
- [x] SPARQL Update (INSERT DATA, DELETE DATA)
- [x] SIMD distance functions
- [x] Query timeout enforcement
- [x] GROUP BY / aggregates
- [x] ASK queries
- [x] String functions
- [x] REGEX filter
- [x] Boolean operators in FILTER
- [x] Comparison operators >=, <=
- [x] isIRI / isLiteral
- [x] FILTER NOT EXISTS / EXISTS
- [x] BIND / VALUES
- [x] LANG() / LANGMATCHES()
- [x] SPARQL results CSV/TSV
- [x] Service description endpoint
- [x] Dockerfile
- [x] Protégé plugin
- [x] SDK endpoint fixes

## Remaining Easy Items (identified for next batch)
- Property paths (+, *, ?)
- HAVING clause
- CONSTRUCT queries
- DESCRIBE queries
- Arithmetic in FILTER (+, -, *, /)
- DATATYPE(), STR(), COALESCE(), IF()
- Content negotiation (Accept header)
- SPARQL results XML format
- Prefix compression for IRI storage
- HNSW compaction (background deleted node cleanup)
