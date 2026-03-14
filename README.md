# SutraDB

A lean, high-performance RDF-star triplestore written in Rust with native HNSW vector indexing and a hybrid SPARQL extension.

## What is this?

SutraDB is a single-purpose database: store triples, answer queries, at any scale. It replaces both a vector database (e.g. Qdrant) and a SPARQL triplestore (e.g. Apache Jena Fuseki) with a single unified system where **vectors are just triples**.

### Core principles

1. **No inference, no reasoning.** The database stores what you put in. OWL, RDFS, and all reasoning belong in the application layer.
2. **Vectors are triples.** A vector embedding is an attribute of a node or edge, stored via a typed predicate and indexed by HNSW — not a separate system.
3. **Full traversal in a single query.** Any traversal of any depth must be expressible in one SPARQL query.
4. **Lean by default.** Every feature must justify itself. Complexity is the enemy of performance.

## Data Model

All data is **RDF-star** triples. Vectors are stored as `sutra:f32vec` literals:

```turtle
# Embedding on a node
:paper_42 :hasEmbedding "0.23 -0.11 0.87 ..."^^sutra:f32vec .

# Embedding on an edge (RDF-star)
<< :paper_42 :discusses :TransformerArchitecture >> :hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec .
<< :paper_42 :discusses :TransformerArchitecture >> :confidence 0.91 .
```

## Hybrid SPARQL

SutraDB extends SPARQL with `VECTOR_SIMILAR` for unified graph + vector queries:

```sparql
SELECT ?doc ?entity WHERE {
  ?entity rdf:type :Person .
  ?doc :mentions ?entity .
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)
}
```

## Crate Structure

| Crate | Purpose |
|---|---|
| `sutra-core` | Triple storage engine, LSM indexes, IRI interning, RDF-star IDs |
| `sutra-hnsw` | HNSW vector index, vector literal type, predicate index registry |
| `sutra-sparql` | SPARQL 1.1 parser, query planner, executor, hybrid extension |
| `sutra-proto` | SPARQL HTTP protocol, Graph Store Protocol, REST API |
| `sutra-cli` | CLI tools: import, export, query, benchmark |

## Status

**Early development.** Architecture is designed; implementation is beginning.

See `docs/architecture.md` for the full design document.

## License

Apache 2.0 (tentative — see open questions in architecture doc).
