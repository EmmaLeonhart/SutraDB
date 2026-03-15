# SutraDB — AI Agent Setup Guide

> **For AI agents setting up SutraDB.** If you are an AI agent (Claude, GPT, Gemini, Copilot, etc.), this document tells you everything you need to install, configure, and operate SutraDB programmatically.

## Quick Start (3 commands)

```bash
# 1. Clone and build
git clone https://github.com/EmmaLeonhart/SutraDB.git
cd SutraDB
cargo build --release -p sutra-cli

# 2. Start the server
./target/release/sutra serve --port 3030

# 3. Verify it's running
curl http://localhost:3030/health
# Returns: ok
```

SutraDB is now running. The SPARQL endpoint is at `http://localhost:3030/sparql`.

## What is SutraDB?

SutraDB is a **unified RDF triplestore + vector database** written in Rust. It stores knowledge graphs (RDF triples) and vector embeddings (HNSW index) in the same database, queryable with a single SPARQL query.

**Key concept:** Vectors are triples. An embedding is just `<entity> <hasEmbedding> "0.1 0.2 ..."^^sutra:f32vec .` — stored in the graph, indexed by HNSW.

## Installation

### Option A: Build from source (recommended)
```bash
git clone https://github.com/EmmaLeonhart/SutraDB.git
cd SutraDB
cargo build --release -p sutra-cli
# Binary at: ./target/release/sutra (or sutra.exe on Windows)
```

### Option B: Install script
```bash
# Linux/macOS
./install.sh
# Windows
install.bat
```
Installs to `~/.sutra/bin/sutra`. Add to PATH.

### Option C: Docker
```bash
docker build -t sutradb .
docker run -p 3030:3030 -v sutra-data:/data sutradb
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `sutra serve` | Start HTTP server (default port 3030) |
| `sutra serve --port 8080` | Custom port |
| `sutra serve --memory-only` | In-memory only, no persistence |
| `sutra serve --data-dir ./my-db` | Custom data directory |
| `sutra query "SELECT ..."` | Run SPARQL query from command line |
| `sutra import data.nt` | Import N-Triples file |
| `sutra import - < data.nt` | Import from stdin |
| `sutra export` | Export all triples to stdout |
| `sutra export -o dump.nt` | Export to file |
| `sutra export -f ttl` | Export as Turtle |
| `sutra info` | Show triple/term counts |

## HTTP API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/sparql` | GET/POST | SPARQL queries (JSON results) |
| `/sparql.csv` | GET/POST | SPARQL queries (CSV results) |
| `/sparql.tsv` | GET/POST | SPARQL queries (TSV results) |
| `/triples` | POST | Insert N-Triples data |
| `/vectors/declare` | POST | Declare a vector predicate |
| `/vectors` | POST | Insert a vector embedding |
| `/graph` | GET | Export all triples as Turtle |
| `/graph?format=nt` | GET | Export as N-Triples |
| `/health` | GET | Health check |
| `/service-description` | GET | SPARQL service description |

## Inserting Data

### Insert triples (N-Triples format)
```bash
curl -X POST http://localhost:3030/triples \
  -H "Content-Type: text/plain" \
  -d '<http://example.org/Alice> <http://example.org/knows> <http://example.org/Bob> .
<http://example.org/Alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Person> .
<http://example.org/Bob> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Person> .'
```

### Insert via SPARQL Update
```bash
curl -X POST http://localhost:3030/sparql \
  -d 'INSERT DATA {
    <http://example.org/Alice> <http://example.org/age> "30"^^<http://www.w3.org/2001/XMLSchema#integer> .
  }'
```

### Declare a vector predicate
```bash
curl -X POST http://localhost:3030/vectors/declare \
  -H "Content-Type: application/json" \
  -d '{"predicate": "http://example.org/hasEmbedding", "dimensions": 1024, "metric": "cosine"}'
```

### Insert a vector embedding
```bash
curl -X POST http://localhost:3030/vectors \
  -H "Content-Type: application/json" \
  -d '{"predicate": "http://example.org/hasEmbedding", "subject": "http://example.org/Alice", "vector": [0.1, 0.2, ...]}'
```

## Querying

### Basic SPARQL
```bash
# All triples
curl "http://localhost:3030/sparql?query=SELECT%20*%20WHERE%20%7B%20%3Fs%20%3Fp%20%3Fo%20%7D%20LIMIT%2010"

# Via POST (easier for complex queries)
curl -X POST http://localhost:3030/sparql \
  -d 'SELECT ?name WHERE { ?person <http://example.org/name> ?name } LIMIT 10'
```

### Vector similarity search
```sparql
SELECT ?doc ?score WHERE {
  VECTOR_SIMILAR(?doc <http://example.org/hasEmbedding> "0.1 0.2 0.3 ..."^^<http://sutra.dev/f32vec>, 0.85)
}
```

### Combined graph + vector query
```sparql
SELECT ?person ?doc WHERE {
  ?person <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Person> .
  ?doc <http://example.org/mentions> ?person .
  VECTOR_SIMILAR(?doc <http://example.org/hasEmbedding> "0.1 0.2 ..."^^<http://sutra.dev/f32vec>, 0.8)
}
```

## Supported SPARQL Features

- SELECT, ASK, CONSTRUCT, DESCRIBE
- INSERT DATA, DELETE DATA
- WHERE with triple patterns
- FILTER (=, !=, <, >, <=, >=)
- FILTER (&&, ||, !)
- FILTER NOT EXISTS / EXISTS
- OPTIONAL, UNION
- BIND, VALUES
- GROUP BY with aggregates (COUNT, SUM, AVG, MIN, MAX)
- ORDER BY (ASC/DESC)
- LIMIT, OFFSET, DISTINCT
- VECTOR_SIMILAR, VECTOR_SCORE
- String functions: CONTAINS, STRSTARTS, STRENDS, REGEX
- LANG(), LANGMATCHES(), isIRI(), isLiteral()
- PREFIX declarations

## SDKs

Official client libraries:

| Language | Install |
|----------|---------|
| Python | `pip install sutradb` (or from `sdks/python/`) |
| TypeScript | `npm install sutradb` (or from `sdks/typescript/`) |
| Go | `go get github.com/EmmaLeonhart/SutraDB/sdks/go` |
| Rust | From `sdks/rust/` |
| Java | From `sdks/java/` (Maven) |
| .NET | From `sdks/dotnet/` (NuGet) |

## Persistence

By default, `sutra serve` persists data to `./sutra-data/` using sled (embedded LSM-tree). Data survives restarts.

Use `--memory-only` for ephemeral testing.

## Architecture Notes for Agents

- **Storage:** Three sorted indexes (SPO, POS, OSP) over 24-byte composite keys of interned u64 IDs
- **Vector index:** One HNSW graph per vector predicate, lives in RAM, keyed by triple object ID
- **Distance metrics:** Cosine (normalize-at-insert + dot product), Euclidean, DotProduct
- **SIMD:** AVX2+FMA and SSE acceleration for distance functions
- **Concurrency:** RwLock for read-heavy SPARQL queries, write-through to sled on mutations
- **Query planner:** Reorders patterns by selectivity, pushes LIMIT down, vector-first for unbound subjects
