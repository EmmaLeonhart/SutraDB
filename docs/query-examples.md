# SutraDB Query Examples

> A comprehensive catalog of SPARQL queries that SutraDB supports or will support,
> from basic triple patterns to hybrid vector+graph queries.

---

## 1. Basic Triple Patterns

### Select everything

```sparql
SELECT * WHERE { ?s ?p ?o }
```

The simplest possible query. Returns every triple in the database.

### Find all instances of a type

```sparql
SELECT ?shrine WHERE {
  ?shrine a <http://example.org/Shrine>
}
```

`a` is shorthand for `rdf:type`. This hits the POS index (predicate-first).

### Get a specific property

```sparql
SELECT ?name WHERE {
  <http://example.org/IseJingu> <http://example.org/name> ?name
}
```

Subject and predicate are bound — this is a point lookup on the SPO index.

### Reverse lookup (who links to this?)

```sparql
SELECT ?source WHERE {
  ?source <http://example.org/enshrines> <http://example.org/Amaterasu>
}
```

Object is bound — this hits the OSP index (object-first).

---

## 2. Joins (Multi-Pattern Queries)

### Two-hop traversal

```sparql
SELECT ?grandchild WHERE {
  <http://example.org/Amaterasu> <http://example.org/child> ?child .
  ?child <http://example.org/child> ?grandchild
}
```

First pattern binds `?child`, second pattern uses it. The planner evaluates the first pattern (1 unbound) before the second (which becomes a point lookup once `?child` is bound).

### Star pattern (multiple properties of one entity)

```sparql
PREFIX ex: <http://example.org/>

SELECT ?name ?type ?location WHERE {
  ?shrine ex:name ?name .
  ?shrine a ?type .
  ?shrine ex:locatedIn ?location
}
```

All three patterns share `?shrine`. After the first pattern binds it, the remaining two become point lookups.

### Path through multiple entities

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine ?deity ?domain WHERE {
  ?shrine a ex:Shrine .
  ?shrine ex:enshrines ?deity .
  ?deity ex:domain ?domain .
  ?domain a ex:NaturalForce
}
```

Four-way join: finds shrines that enshrine deities whose domains are natural forces.

### Self-join (find pairs)

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine1 ?shrine2 WHERE {
  ?shrine1 ex:enshrines ?deity .
  ?shrine2 ex:enshrines ?deity .
  FILTER(?shrine1 != ?shrine2)
}
```

Finds pairs of shrines that enshrine the same deity.

---

## 3. FILTER Expressions

### Numeric comparison

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine ?year WHERE {
  ?shrine ex:foundedYear ?year .
  FILTER(?year < 800)
}
```

Finds shrines founded before 800 CE. Uses inline integer comparison.

### String matching

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine ?name WHERE {
  ?shrine ex:name ?name .
  FILTER(CONTAINS(?name, "Jingu"))
}
```

String functions CONTAINS, STRSTARTS, STRENDS, and REGEX are implemented.

### Bound/not-bound checks

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine ?name ?altName WHERE {
  ?shrine ex:name ?name .
  OPTIONAL { ?shrine ex:alternateName ?altName } .
  FILTER(!bound(?altName))
}
```

Finds shrines that do NOT have an alternate name.

### Combining conditions

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine WHERE {
  ?shrine ex:foundedYear ?year .
  ?shrine ex:rank ?rank .
  FILTER(?year < 1000) .
  FILTER(?rank = 1)
}
```

Multiple FILTERs are ANDed together.

---

## 4. OPTIONAL (Left Join)

### Optional properties

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine ?name ?description WHERE {
  ?shrine a ex:Shrine .
  ?shrine ex:name ?name .
  OPTIONAL { ?shrine ex:description ?description }
}
```

Returns all shrines with names. If a shrine has a description, include it; if not, `?description` is unbound but the row still appears.

### Multiple optionals

```sparql
PREFIX ex: <http://example.org/>

SELECT ?deity ?name ?domain ?alias WHERE {
  ?deity a ex:Deity .
  ?deity ex:name ?name .
  OPTIONAL { ?deity ex:domain ?domain } .
  OPTIONAL { ?deity ex:alias ?alias }
}
```

Each OPTIONAL is independent — a deity might have a domain but no alias, or vice versa.

---

## 5. Solution Modifiers

### Pagination

```sparql
SELECT ?s ?p ?o WHERE { ?s ?p ?o }
LIMIT 100 OFFSET 200
```

Skip the first 200 results, return the next 100.

### Distinct results

```sparql
PREFIX ex: <http://example.org/>

SELECT DISTINCT ?deity WHERE {
  ?shrine ex:enshrines ?deity
}
```

Removes duplicate deity bindings (if multiple shrines enshrine the same deity).

---

## 6. PREFIX Declarations

### Multiple prefixes

```sparql
PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
PREFIX ex: <http://example.org/>
PREFIX wd: <http://www.wikidata.org/entity/>

SELECT ?shrine ?label WHERE {
  ?shrine a ex:Shrine .
  ?shrine rdfs:label ?label
}
```

### Default prefix (empty prefix)

```sparql
PREFIX : <http://example.org/>

SELECT ?s WHERE {
  ?s :name ?name .
  ?s a :Shrine
}
```

The empty prefix `:name` expands to `<http://example.org/name>`.

---

## 7. RDF-star (Statements About Statements)

### Annotated edges

```turtle
# Data:
<< :IseJingu :enshrines :Amaterasu >> :confidence 0.99 .
<< :IseJingu :enshrines :Amaterasu >> :source :KojikiText .
```

```sparql
PREFIX ex: <http://example.org/>

SELECT ?shrine ?deity ?confidence WHERE {
  ?shrine ex:enshrines ?deity .
  << ?shrine ex:enshrines ?deity >> ex:confidence ?confidence .
  FILTER(?confidence > 0.9)
}
```

Finds high-confidence enshrinement relationships.

### Provenance tracking

```sparql
PREFIX ex: <http://example.org/>

SELECT ?s ?p ?o ?source WHERE {
  ?s ?p ?o .
  << ?s ?p ?o >> ex:source ?source .
  ?source a ex:PrimarySource
}
```

Finds all triples whose provenance is a primary source.

### Temporal metadata on edges

```sparql
PREFIX ex: <http://example.org/>

SELECT ?deity ?domain ?startYear WHERE {
  ?deity ex:hasDomain ?domain .
  << ?deity ex:hasDomain ?domain >> ex:since ?startYear .
  FILTER(?startYear < 500)
}
```

Finds deity-domain associations that were established before 500 CE.

---

## 8. Vector SPARQL

These queries use SutraDB's hybrid SPARQL extension. See `docs/vectorSPARQL.md` for full design.

### Basic similarity search

```sparql
# Find documents whose embedding is similar to a query vector
SELECT ?doc WHERE {
  VECTOR_SIMILAR(?doc :hasEmbedding "0.23 -0.11 0.87 ..."^^sutra:f32vec, 0.85)
}
```

Returns all `?doc` whose `:hasEmbedding` vector has cosine similarity ≥ 0.85 with the query vector.

### Semantic search with type constraint

```sparql
PREFIX ex: <http://example.org/>

SELECT ?paper ?title WHERE {
  ?paper a ex:AcademicPaper .
  ?paper ex:title ?title .
  VECTOR_SIMILAR(?paper :hasEmbedding "0.1 0.2 ..."^^sutra:f32vec, 0.80)
}
```

Graph pattern constrains to academic papers; vector search finds semantically relevant ones.

### Recommendation: "things like this"

```sparql
PREFIX ex: <http://example.org/>

# Given a shrine's embedding, find similar shrines
SELECT ?similar ?name WHERE {
  ?similar a ex:Shrine .
  ?similar ex:name ?name .
  VECTOR_SIMILAR(?similar :descriptionEmbedding "0.5 0.3 ..."^^sutra:f32vec, 0.75)
  FILTER(?similar != <http://example.org/IseJingu>)
}
```

Find shrines whose descriptions are semantically similar to Ise Jingu's, excluding Ise Jingu itself.

### Cold start: natural language entry point

```sparql
# The user doesn't know any URIs. They have a concept embedding.
# The vector search provides the "entry point" into the graph.

SELECT ?entity ?type ?name WHERE {
  VECTOR_SIMILAR(?entity :embedding "..."^^sutra:f32vec, 0.70)
  ?entity a ?type .
  ?entity rdfs:label ?name
}
LIMIT 10
```

This is the cold start solution: the user provides a concept vector (from an embedding model), the HNSW index finds nearest entities, then SPARQL resolves their types and names.

### Graph traversal from vector entry point

```sparql
PREFIX ex: <http://example.org/>

# Start from a semantic search, then traverse the graph
SELECT ?shrine ?deity ?myth WHERE {
  VECTOR_SIMILAR(?shrine :embedding "..."^^sutra:f32vec, 0.80)
  ?shrine ex:enshrines ?deity .
  ?deity ex:appearsIn ?myth .
  ?myth a ex:Myth
}
```

The vector search finds shrines; SPARQL traverses from those shrines to their deities to the myths they appear in.

### Ranked results with VECTOR_SCORE

```sparql
PREFIX ex: <http://example.org/>

SELECT ?doc ?title (VECTOR_SCORE(?doc :embedding "..."^^sutra:f32vec) AS ?relevance) WHERE {
  ?doc a ex:Document .
  ?doc ex:title ?title .
  VECTOR_SIMILAR(?doc :embedding "..."^^sutra:f32vec, 0.60)
}
ORDER BY DESC(?relevance)
LIMIT 20
```

VECTOR_SCORE exposes the actual similarity value so results can be ranked.

### Edge similarity search (RDF-star + vectors)

```sparql
# Find relationships (not nodes) that are semantically similar
# to "causal influence between concepts"
SELECT ?s ?p ?o ?score WHERE {
  << ?s ?p ?o >> :relationEmbedding ?v .
  VECTOR_SIMILAR(<< ?s ?p ?o >> :relationEmbedding "..."^^sutra:f32vec, 0.85)
  BIND(VECTOR_SCORE(<< ?s ?p ?o >> :relationEmbedding "..."^^sutra:f32vec) AS ?score)
}
ORDER BY DESC(?score)
```

Unique to SutraDB: searching over the semantic meaning of relationships, not just nodes.

### Hybrid: vector + graph + filter

```sparql
PREFIX ex: <http://example.org/>

# Find papers about machine learning, published after 2020,
# semantically similar to a query, by authors at Stanford
SELECT ?paper ?title ?author WHERE {
  ?paper a ex:Paper .
  ?paper ex:topic ex:MachineLearning .
  ?paper ex:publishedYear ?year .
  FILTER(?year > 2020) .
  ?paper ex:author ?author .
  ?author ex:affiliation ex:Stanford .
  ?paper ex:title ?title .
  VECTOR_SIMILAR(?paper :abstractEmbedding "..."^^sutra:f32vec, 0.75)
}
ORDER BY DESC(VECTOR_SCORE(?paper :abstractEmbedding "..."^^sutra:f32vec))
LIMIT 10
```

This combines type constraints, property filters, multi-hop traversal, and vector similarity in one query. No glue code. No two-system orchestration.

### Multi-vector query (different embedding spaces)

```sparql
PREFIX ex: <http://example.org/>

# Find entities that are similar in BOTH text and image embedding spaces
SELECT ?entity ?name WHERE {
  ?entity ex:name ?name .
  VECTOR_SIMILAR(?entity :textEmbedding "..."^^sutra:f32vec, 0.80) .
  VECTOR_SIMILAR(?entity :imageEmbedding "..."^^sutra:f32vec, 0.70)
}
```

Multiple VECTOR_SIMILAR clauses on different predicates — each predicate has its own HNSW index. Both must pass their thresholds.

### Knowledge graph completion candidate detection

```sparql
PREFIX ex: <http://example.org/>

# Find protein-disease pairs that are semantically related
# but have no explicit association edge yet
SELECT ?protein ?disease WHERE {
  ?protein a ex:Protein .
  ?disease a ex:Disease .
  VECTOR_SIMILAR(?protein :bioEmbedding "..."^^sutra:f32vec, 0.85) .
  FILTER NOT EXISTS { ?protein ex:associatedWith ?disease }
}
```

Uses vector similarity to identify candidate links, then FILTER NOT EXISTS to confirm no existing edge — surfacing potential new knowledge.

### Tuning search quality with ef_search

```sparql
# Higher ef = better recall but slower
SELECT ?doc WHERE {
  VECTOR_SIMILAR(?doc :embedding "..."^^sutra:f32vec, 0.80, ef:=500)
}
```

```sparql
# Lower ef = faster but might miss some results
SELECT ?doc WHERE {
  VECTOR_SIMILAR(?doc :embedding "..."^^sutra:f32vec, 0.80, ef:=50)
}
```

### Top-K mode (no threshold)

```sparql
# Just give me the 10 most similar, regardless of score
SELECT ?doc WHERE {
  VECTOR_SIMILAR(?doc :embedding "..."^^sutra:f32vec, k:=10)
}
```

---

## 9. Query Patterns by Use Case

### Wikidata-style entity browsing

```sparql
PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX wdt: <http://www.wikidata.org/prop/direct/>

# All Shikinai Ronsha shrines with their locations
SELECT ?shrine ?label ?prefecture WHERE {
  ?shrine wdt:P31 wd:Q135022904 .
  ?shrine rdfs:label ?label .
  OPTIONAL { ?shrine wdt:P131 ?prefecture }
}
```

### Genealogy traversal

```sparql
PREFIX ex: <http://example.org/>

# Find all ancestors up to 5 generations
SELECT ?ancestor ?name WHERE {
  <http://example.org/PersonA> ex:parent+ ?ancestor .
  ?ancestor ex:name ?name
}
```

(Property paths like `+` are future work but this shows the target.)

### Document corpus exploration

```sparql
PREFIX dc: <http://purl.org/dc/elements/1.1/>
PREFIX ex: <http://example.org/>

SELECT ?doc ?title ?author ?year WHERE {
  ?doc a ex:Document .
  ?doc dc:title ?title .
  ?doc dc:creator ?author .
  ?doc dc:date ?year .
  FILTER(?year > 2020)
}
ORDER BY DESC(?year)
LIMIT 50
```

### Social graph analysis

```sparql
PREFIX foaf: <http://xmlns.com/foaf/0.1/>

# Find friends of friends
SELECT DISTINCT ?fof ?name WHERE {
  <http://example.org/Alice> foaf:knows ?friend .
  ?friend foaf:knows ?fof .
  ?fof foaf:name ?name .
  FILTER(?fof != <http://example.org/Alice>)
}
```

### Semantic search + social graph (hybrid)

```sparql
PREFIX foaf: <http://xmlns.com/foaf/0.1/>
PREFIX ex: <http://example.org/>

# Find friends of friends who are interested in similar topics
SELECT ?person ?name ?score WHERE {
  <http://example.org/Alice> foaf:knows ?friend .
  ?friend foaf:knows ?person .
  ?person foaf:name ?name .
  FILTER(?person != <http://example.org/Alice>) .
  VECTOR_SIMILAR(?person :interestEmbedding "..."^^sutra:f32vec, 0.75)
  BIND(VECTOR_SCORE(?person :interestEmbedding "..."^^sutra:f32vec) AS ?score)
}
ORDER BY DESC(?score)
LIMIT 10
```

Graph traversal finds friends-of-friends; vector search ranks them by semantic interest similarity.

---

## Implementation Status

| Query Feature | Status |
|---|---|
| Basic triple patterns | Implemented |
| Multi-pattern joins | Implemented |
| PREFIX declarations | Implemented |
| FILTER (numeric) | Implemented |
| OPTIONAL | Implemented |
| LIMIT / OFFSET | Implemented |
| DISTINCT | Implemented |
| `a` shorthand | Implemented |
| Integer literals / inline | Implemented |
| String literals | Implemented |
| Full IRI syntax | Implemented |
| Prefixed names | Implemented |
| RDF-star quoted triples | Core support (not yet in SPARQL) |
| ORDER BY | Implemented |
| Property paths (+, *, ?) | Not yet |
| UNION | Not yet |
| BIND / VALUES | Implemented |
| String functions (CONTAINS, STRSTARTS, STRENDS, REGEX) | Implemented |
| VECTOR_SIMILAR | Implemented |
| VECTOR_SCORE | Implemented |
| COSINE_SEARCH / EUCLID_SEARCH / DOTPRODUCT_SEARCH | Implemented |
| ef_search hint | Not yet |
| Top-K mode | Not yet |
