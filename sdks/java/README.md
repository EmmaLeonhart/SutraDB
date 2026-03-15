# SutraDB Java Client

Java client for [SutraDB](https://github.com/EmmaLeonhart/SutraDB) — an RDF-star triplestore with native HNSW vector indexing.

Requires Java 11+. Uses `java.net.http.HttpClient` (no external HTTP dependencies).

## Installation

### Maven

```xml
<dependency>
    <groupId>dev.sutradb</groupId>
    <artifactId>sutradb-java</artifactId>
    <version>0.1.0</version>
</dependency>
```

### Gradle

```groovy
implementation 'dev.sutradb:sutradb-java:0.1.0'
```

## Usage

```java
import dev.sutradb.SutraClient;
import dev.sutradb.SparqlResults;

SutraClient client = new SutraClient("http://localhost:7878");

// Health check
boolean alive = client.health();

// Insert triples
client.insertTriples("<http://example.org/paper1> <http://example.org/title> \"Graph Databases\" .");

// SPARQL query
SparqlResults results = client.sparql("SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10");
for (var row : results.getBindings()) {
    System.out.println(row.get("s").getValue());
}

// Declare a vector predicate
client.declareVector("http://example.org/hasEmbedding", 1536);

// Insert a vector
double[] embedding = new double[1536];
client.insertVector("http://example.org/hasEmbedding", "http://example.org/paper1", embedding);
```

## License

Apache-2.0
