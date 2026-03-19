# SutraDB Java Client

Java client for [SutraDB](https://github.com/EmmaLeonhart/SutraDB) — an RDF-star triplestore with native HNSW vector indexing.

Requires Java 11+. Uses `java.net.http.HttpClient` (no external HTTP dependencies). Built with Gradle (Kotlin DSL).

## Installation

### Gradle (Kotlin DSL)

```kotlin
implementation("dev.sutradb:sutradb-java:0.3.0")
```

### Gradle (Groovy DSL)

```groovy
implementation 'dev.sutradb:sutradb-java:0.3.0'
```

### Maven

```xml
<dependency>
    <groupId>dev.sutradb</groupId>
    <artifactId>sutradb-java</artifactId>
    <version>0.3.0</version>
</dependency>
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

// Rebuild all HNSW indexes
JSONObject rebuildResult = client.rebuildHnsw();

// Get a combined health report (general health + vector index status)
JSONObject report = client.healthReport();
System.out.println("Healthy: " + report.getBoolean("healthy"));
System.out.println("Vector indexes: " + report.getJSONObject("vectors"));
```

## License

Apache-2.0
