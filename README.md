# SutraDB

A lean, high-performance RDF-star triplestore written in Rust with native HNSW vector indexing and SPARQL+ query language.

[![CI](https://github.com/EmmaLeonhart/SutraDB/actions/workflows/ci.yml/badge.svg)](https://github.com/EmmaLeonhart/SutraDB/actions/workflows/ci.yml)

**[sutradb.org](https://sutradb.org)** — Documentation, theory, and interactive visualizations.

> **Status: Developer Preview.** Core engine, SPARQL+, vector indexing, HTTP server, ACID compliance, self-update, and MCP server are fully functional. APIs may evolve before 1.0. See the [Roadmap](https://sutradb.org/roadmap/) for what's done and what's next.

> **AI Agents:** If you are an AI agent setting up a database, see **[Agent Setup Guide](docs/AGENT_SETUP.md)** for complete CLI reference, API endpoints, and step-by-step instructions. SutraDB is designed to be fully operable by AI agents without ever touching a GUI.

## What is this?

SutraDB is a single-purpose database: store triples, answer queries, at any scale. It replaces both a vector database (e.g. Qdrant) and a SPARQL triplestore (e.g. Apache Jena Fuseki) with a single unified system where **vectors are just triples**.

The vector indexing architecture is heavily influenced by [Qdrant](https://github.com/qdrant/qdrant), reimplemented from first principles and unified with a triple store. The RDF/SPARQL semantics draw from Apache Jena's TDB2, but without the JVM overhead.

### Core principles

1. **Store first, reason second.** The database stores what you put in. OWL validation happens client-side in SDKs, not in the database.
2. **Vectors are triples.** A vector embedding is an attribute of a node or edge, stored via a typed predicate and indexed by HNSW — not a separate system.
3. **Full traversal in a single query.** Any traversal of any depth must be expressible in one SPARQL query.
4. **Lean by default.** Every feature must justify itself. Complexity is the enemy of performance.
5. **Agent-first, GUI-optional.** The CLI is the primary interface. Sutra Studio (GUI) exists for visual diagnostics.

## Quick Start

```bash
# Build
cargo build --release -p sutra-cli

# Start server (persistent storage)
./target/release/sutra serve

# Insert some data
curl -X POST http://localhost:3030/triples \
  -d '<http://example.org/Alice> <http://example.org/knows> <http://example.org/Bob> .'

# Query
curl -X POST http://localhost:3030/sparql \
  -d 'SELECT * WHERE { ?s ?p ?o } LIMIT 10'
```

## What's New in v0.2

- **ACID compliance** — atomic sled transactions, startup consistency verification, durable flushes
- **Self-update** — `sutra update`, `sutra --version`, startup version check
- **MCP server for AI agents** — dual-mode (serverless + server), 8 maintenance tools
- **HNSW rebuild HTTP endpoint** — `POST /vectors/rebuild` for compacting and rebuilding all HNSW indexes
- **COSINE_SEARCH, EUCLID_SEARCH, DOTPRODUCT_SEARCH** — new SPARQL operators for explicit distance metric selection

## Data Model

All data is **RDF-star** triples. Vectors are stored as `sutra:f32vec` literals:

```turtle
# Embedding on a node
:paper_42 :hasEmbedding "0.23 -0.11 0.87 ..."^^sutra:f32vec .

# Embedding on an edge (RDF-star)
<< :paper_42 :discusses :TransformerArchitecture >> :hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec .
```

## SPARQL+

SutraDB's query language is **SPARQL+** — a superset of SPARQL 1.1 with `VECTOR_SIMILAR` for unified graph + vector queries and `UNTIL` for predicate-based exit conditions on property path traversal:

```sparql
SELECT ?doc ?entity WHERE {
  ?entity rdf:type :Person .
  ?doc :mentions ?entity .
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)
}
```

### Supported SPARQL Features

SELECT, ASK, CONSTRUCT, DESCRIBE | INSERT DATA, DELETE DATA | FILTER (=, !=, <, >, <=, >=, &&, ||, !) | FILTER NOT EXISTS / EXISTS | OPTIONAL, UNION | BIND, VALUES | GROUP BY + COUNT/SUM/AVG/MIN/MAX | ORDER BY, LIMIT, OFFSET, DISTINCT | VECTOR_SIMILAR, VECTOR_SCORE | String functions (CONTAINS, STRSTARTS, STRENDS, REGEX) | LANG(), LANGMATCHES(), isIRI(), isLiteral() | PREFIX declarations

## Architecture

| Crate | Purpose | Status |
|---|---|---|
| `sutra-core` | Triple storage engine, IRI interning, RDF-star IDs, sled persistence | Implemented |
| `sutra-hnsw` | HNSW vector index with SIMD (AVX2/SSE), multiple distance metrics | Implemented |
| `sutra-sparql` | SPARQL 1.1 parser, query planner, executor, hybrid extension | Implemented |
| `sutra-proto` | HTTP server, SPARQL protocol, Graph Store Protocol | Implemented |
| `sutra-cli` | CLI: serve, query, import, export, info | Implemented |

## CLI

```bash
sutra serve                     # Start HTTP server (port 3030)
sutra serve --memory-only       # In-memory only
sutra query "SELECT ..."        # Run SPARQL query
sutra import data.nt            # Import N-Triples
sutra export -o dump.nt         # Export all triples
sutra info                      # Show database stats
```

## SDKs

| Language | Package | Install |
|----------|---------|---------|
| Python | [`sutradb`](https://pypi.org/project/sutradb/) | `pip install sutradb` |
| TypeScript | [`sutradb`](https://www.npmjs.com/package/sutradb) | `npm install sutradb` |
| Go | [`sutradb`](sdks/go/) | `go get github.com/EmmaLeonhart/SutraDB/sdks/go` |
| Rust | [`sutradb`](sdks/rust/) | `cargo add sutradb` |
| Java | [`sutradb-java`](sdks/java/) | Maven dependency |
| .NET | [`SutraDB.Client`](sdks/dotnet/) | `dotnet add package SutraDB.Client` |

## Sutra Studio

Flutter desktop/web client for visual database management. See `sutra-studio/README.md`.

```bash
cd sutra-studio && flutter run -d chrome
```

## Test Suite

256 tests across 5 crates:

```bash
cargo test --workspace
```

## Building

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## Docker

```bash
docker build -t sutradb .
docker run -p 3030:3030 -v sutra-data:/data sutradb
```

## License

Apache 2.0
