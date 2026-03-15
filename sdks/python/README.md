# sutradb

Python client for [SutraDB](https://github.com/EmmaLeonhart/SutraDB) — an RDF-star triplestore with native HNSW vector indexing.

## Installation

```bash
pip install sutradb
```

## Quick Start

```python
from sutradb import SutraClient

client = SutraClient("http://localhost:3030")

# Check server health
if client.health():
    print("SutraDB is running")

# Run a SPARQL query
results = client.sparql("""
    SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10
""")
for binding in results["results"]["bindings"]:
    print(binding["s"]["value"], binding["p"]["value"], binding["o"]["value"])

# Insert triples
client.insert_triples("""
    <http://example.org/paper/1> <http://example.org/title> "Attention Is All You Need" .
    <http://example.org/paper/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/Paper> .
""")

# Declare a vector predicate
client.declare_vector(
    predicate="http://example.org/hasEmbedding",
    dimensions=1536,
)

# Insert a vector
client.insert_vector(
    predicate="http://example.org/hasEmbedding",
    subject="http://example.org/paper/1",
    vector=[0.23, -0.11, 0.87, ...],  # 1536-dimensional vector
)

# Batch insert vectors
entries = [
    ("http://example.org/paper/1", [0.23, -0.11, ...]),
    ("http://example.org/paper/2", [0.45, 0.02, ...]),
]
result = client.insert_vectors_batch("http://example.org/hasEmbedding", entries)
print(f"Inserted {result['inserted']} vectors")
```

## License

Apache-2.0
