# Vector SPARQL — Design Notes and Examples

> How SutraDB extends SPARQL to unify graph traversal and vector similarity search.

---

## The Problem

Standard SPARQL operates over discrete triples — exact matches, pattern matching, graph traversal. Vector databases operate over continuous embedding spaces — approximate nearest neighbors, similarity thresholds. Today these are separate systems:

- **SPARQL triplestore** (Fuseki, Blazegraph): "Find all papers that discuss topic X"
- **Vector database** (Qdrant, Weaviate): "Find documents semantically similar to this query"

Combining them requires application-layer glue: query one, feed results to the other, merge. This is slow, fragile, and loses the ability to express the full query in one shot.

SutraDB eliminates this by making **vectors just another predicate type** and adding vector operators directly to SPARQL.

---

## Core Concept: Vectors Are Triples

In SutraDB, a vector embedding is stored as an RDF triple with a special literal type:

```turtle
# A node has an embedding
:paper_42 :hasEmbedding "0.23 -0.11 0.87 ..."^^sutra:f32vec .

# An edge has an embedding (RDF-star)
<< :paper_42 :discusses :TransformerArchitecture >> :hasEmbedding "0.12 0.45 ..."^^sutra:f32vec .
```

The `sutra:f32vec` literal type is a fixed-dimension array of 32-bit floats. When a predicate is declared as a vector predicate, SutraDB automatically builds and maintains an HNSW index over all triples with that predicate. The vector index is a first-class index alongside SPO/POS/OSP — not a foreign system.

### Schema Declaration

```turtle
sutra:declareVectorPredicate :hasEmbedding ;
    sutra:dimensions 1536 ;
    sutra:hnswM 16 ;
    sutra:hnswEfConstruction 200 .
```

This tells SutraDB:
- Any triple with predicate `:hasEmbedding` has a 1536-dimensional vector as its object
- Build an HNSW index with M=16 and ef_construction=200
- Inserting a vector of the wrong dimensionality is a hard error

---

## VECTOR_SIMILAR Operator

The primary extension to SPARQL. It can appear anywhere a graph pattern can appear:

```sparql
VECTOR_SIMILAR(?subject :predicate "query_vector"^^sutra:f32vec, threshold)
```

**Parameters:**
- `?subject` — the variable to bind to matching subjects
- `:predicate` — which vector predicate to search
- `"..."^^sutra:f32vec` — the query vector (literal)
- `threshold` — minimum cosine similarity (0.0 to 1.0)

### Basic Usage

```sparql
# Find all documents similar to a query embedding
SELECT ?doc WHERE {
  VECTOR_SIMILAR(?doc :hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec, 0.85)
}
```

This returns all `:doc` IRIs whose `:hasEmbedding` vector has cosine similarity ≥ 0.85 with the query vector.

### Combined with Graph Patterns

The real power: mixing vector search with graph traversal in a single query.

```sparql
# Find documents about a topic AND semantically similar to a query
SELECT ?doc ?topic WHERE {
  ?doc :discusses ?topic .
  ?topic rdf:type :MachineLearning .
  VECTOR_SIMILAR(?doc :hasEmbedding "0.23 -0.11 ..."^^sutra:f32vec, 0.80)
}
```

```sparql
# Find people whose profile embedding is similar to mine,
# who also work at the same company
SELECT ?person ?name WHERE {
  ?person :worksAt :Google .
  ?person :name ?name .
  VECTOR_SIMILAR(?person :profileEmbedding "0.5 0.3 ..."^^sutra:f32vec, 0.90)
}
```

```sparql
# Multi-hop: find papers within 3 hops of a concept,
# that are also semantically similar to a query
SELECT ?paper WHERE {
  :TransformerArchitecture :influences+ ?concept .
  ?paper :discusses ?concept .
  VECTOR_SIMILAR(?paper :hasEmbedding "0.1 0.2 ..."^^sutra:f32vec, 0.75)
}
```

---

## VECTOR_SCORE Function

Returns the actual similarity score for use in ORDER BY or SELECT:

```sparql
SELECT ?paper (VECTOR_SCORE(?paper :hasEmbedding "..."^^sutra:f32vec) AS ?score) WHERE {
  ?paper rdf:type :AcademicPaper .
  VECTOR_SIMILAR(?paper :hasEmbedding "..."^^sutra:f32vec, 0.70)
}
ORDER BY DESC(?score)
LIMIT 10
```

---

## Edge Embeddings (RDF-star)

Because SutraDB uses RDF-star, you can embed edges, not just nodes:

```sparql
# Find relationships (edges) similar to "causal influence"
SELECT ?s ?p ?o WHERE {
  << ?s ?p ?o >> :hasEmbedding ?v .
  VECTOR_SIMILAR(<< ?s ?p ?o >> :hasEmbedding "..."^^sutra:f32vec, 0.85)
}
```

This is unique to SutraDB — no other triplestore or vector database can search over relationship embeddings natively.

---

## Query Planner Heuristic

The query planner must decide execution order when VECTOR_SIMILAR appears alongside graph patterns:

| Condition | Strategy | Rationale |
|---|---|---|
| Subject **bound** before VECTOR_SIMILAR | Graph first, then vector filter | Small result set → cheap vector check |
| Subject **unbound** at VECTOR_SIMILAR | Vector search first (top-k), then graph filter | HNSW is O(log n), graph scan could be O(n) |

### Example: Bound Subject

```sparql
SELECT ?doc WHERE {
  ?doc :author <http://example.org/Alice> .    # Binds ?doc to ~10 results
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)  # Filter those 10
}
```

Here `?doc` is bound by the first pattern to a small set. The planner executes the graph pattern first, then checks vector similarity only on the bound results. This is cheap.

### Example: Unbound Subject

```sparql
SELECT ?doc WHERE {
  VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85)  # Top-k from HNSW
  ?doc :publishedIn :Nature .                                    # Filter by graph
}
```

Here `?doc` is unbound. The planner runs the HNSW search first (returns top-k candidates in O(log n)), then evaluates the graph pattern over only those candidates.

### Future: Adaptive Execution

The v0.1 heuristic is static — it decides order before execution begins. The correct long-term solution is **adaptive execution**: start executing, observe intermediate result sizes, and reorder mid-query. This handles cases where the planner's static estimate is wrong (e.g., a "bound" variable actually matches 10 million triples).

---

## Optional Parameters

### ef_search Hint

Control the accuracy/speed tradeoff per-query:

```sparql
VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, 0.85, ef:=200)
```

Higher `ef` = better recall but slower search. Default is the index's configured `ef_search`.

### Top-K Mode

Instead of a threshold, return the top K results:

```sparql
VECTOR_SIMILAR(?doc :hasEmbedding "..."^^sutra:f32vec, k:=10)
```

---

## Real-World Query Examples

### GraphRAG: Semantic + Structural Retrieval

```sparql
# Find relevant context for an LLM prompt:
# 1. Semantically similar documents
# 2. Their structural neighbors in the knowledge graph
SELECT ?chunk ?related ?relationship WHERE {
  VECTOR_SIMILAR(?chunk :embedding "..."^^sutra:f32vec, 0.80)
  ?chunk ?relationship ?related .
  ?related rdf:type :Entity .
}
LIMIT 50
```

### Knowledge Graph Completion

```sparql
# Find candidate links: entities whose embeddings suggest a relationship
# but no explicit triple exists
SELECT ?entity1 ?entity2 WHERE {
  ?entity1 rdf:type :Protein .
  ?entity2 rdf:type :Disease .
  VECTOR_SIMILAR(?entity1 :embedding "..."^^sutra:f32vec, 0.90)
  FILTER NOT EXISTS { ?entity1 :associatedWith ?entity2 }
}
```

### Multimodal Search with Shared Embedding Space

```sparql
# Search for images similar to a text query
# (both embedded in the same CLIP space)
SELECT ?image ?caption WHERE {
  VECTOR_SIMILAR(?image :clipEmbedding "..."^^sutra:f32vec, 0.75)
  ?image :hasCaption ?caption .
}
ORDER BY DESC(VECTOR_SCORE(?image :clipEmbedding "..."^^sutra:f32vec))
LIMIT 20
```

### Wikidata-Scale Entity Resolution

```sparql
# Find Wikidata entities semantically similar to a shrine description,
# filtered by type
PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX wdt: <http://www.wikidata.org/prop/direct/>

SELECT ?shrine ?label WHERE {
  ?shrine wdt:P31 wd:Q135022904 .  # instance of Shikinai Ronsha
  ?shrine rdfs:label ?label .
  VECTOR_SIMILAR(?shrine :descriptionEmbedding "..."^^sutra:f32vec, 0.80)
}
```

---

## Implementation Status

| Feature | Status |
|---|---|
| `sutra:f32vec` literal type | Implemented |
| HNSW index per vector predicate | Implemented |
| Cosine / Euclidean / DotProduct metrics | Implemented |
| VECTOR_SIMILAR in SPARQL parser | Not yet |
| VECTOR_SIMILAR in query executor | Not yet |
| VECTOR_SCORE function | Not yet |
| Query planner vector integration | Not yet |
| ef_search hint | Not yet |
| Top-K mode | Not yet |

The vector index infrastructure exists in `sutra-hnsw`. The SPARQL parser and executor exist in `sutra-sparql`. The integration — making VECTOR_SIMILAR a first-class SPARQL operator that the query planner can reason about — is the next major piece of work.

---

## The Cold Start Problem

In the Semantic Web and Knowledge Graph world, there's a massive "cold start" problem that traditional SPARQL triplestores cannot solve. SutraDB's vector-first approach eliminates it.

### The Three Cold Start Problems

**1. Schema Cold Start (OWL/RDFS)**
In a strict SPARQL database, you can't query until you've defined a rigid ontology. You can't just dump data in and start finding things. It requires a human to architect the schema before the first query can even run.

**2. The Exact Match Trap**
In a traditional RDF database like Fuseki: if you don't know the exact URI or string literal, you can't find anything. Searching for "the guy who started the electric car company" when the graph only contains `:Elon_Musk` returns nothing. The graph is "cold" — you're standing outside a locked library without the specific call number.

**3. The Silo Problem**
If two nodes aren't explicitly connected by an edge, they're invisible to each other. No edge between "SpaceX" and "Boeing" means the relationship doesn't exist, even though they're obviously related.

### How Vector-First Solves It

By making vector indexing a first-class citizen, SutraDB creates **semantic on-ramps**:

1. **Warm Entry**: Query with a natural language concept embedding ("electric car pioneer")
2. **The Hand-off**: The HNSW index finds nearest neighbors (`:ElonMusk`, `:Tesla`, `:Rivian`)
3. **The Jump**: SPARQL takes over for rigid, logical graph traversal from those entry points

The vector index acts as the **receptionist** who understands what you're looking for. The graph/SPARQL layer acts as the **filing system** that provides documented truth.

Even without edges, vector-indexed nodes allow the query engine to "see" relationships through semantic similarity. The database is warm from the moment the first piece of data is ingested, regardless of whether your OWL constraints are mapped out.

### Embedding Generation: Application-Side

SutraDB is **model-agnostic**. Embeddings are provided by the application at insert time as raw `f32` arrays. The database does not know or care what model produced them — it just stores and indexes floats.

This is a deliberate design choice:
- No coupling to any specific embedding model
- Swap models without changing the DB
- Application controls embedding quality, batching, and versioning
- Database stays lean — no ML runtime dependency

A middleware layer could handle query-time embedding (converting natural language queries into vectors before passing to VECTOR_SIMILAR), but that belongs outside the database.

---

## Why This Matters

Every existing GraphRAG system is a kludge. Microsoft's GraphRAG flattens the graph into community summaries and retrieves summaries — throwing away most relational structure. LangChain's graph retrievers do two separate queries and merge in Python.

SutraDB makes the full power of both graph traversal AND vector similarity available in a single, declarative query language. No glue code. No result merging. No two-system orchestration. Just one query that says exactly what you want.

This is what a database should be.
