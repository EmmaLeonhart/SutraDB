# sutradb

Go client for [SutraDB](https://github.com/EmmaLeonhart/SutraDB) — an RDF-star triplestore with native HNSW vector indexing.

Uses only the Go standard library. No external dependencies.

## Installation

```sh
go get github.com/EmmaLeonhart/SutraDB/sdks/go
```

## Usage

```go
package main

import (
	"fmt"
	"log"

	sutradb "github.com/EmmaLeonhart/SutraDB/sdks/go"
)

func main() {
	client := sutradb.NewClient("http://localhost:7878")

	// Health check
	healthy, err := client.Health()
	if err != nil {
		log.Fatal(err)
	}
	fmt.Println("Healthy:", healthy)

	// Insert triples
	_, err = client.InsertTriples(`<http://example.org/paper1> <http://example.org/title> "Graph Databases" .`)
	if err != nil {
		log.Fatal(err)
	}

	// SPARQL query
	results, err := client.Sparql("SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10")
	if err != nil {
		log.Fatal(err)
	}
	for _, row := range results.Results.Bindings {
		fmt.Println(row["s"].Value, row["p"].Value, row["o"].Value)
	}

	// Declare a vector predicate
	_, err = client.DeclareVector("http://example.org/hasEmbedding", 1536,
		sutradb.WithHnswM(16),
		sutradb.WithHnswEfConstruction(200),
	)
	if err != nil {
		log.Fatal(err)
	}

	// Insert a vector
	embedding := make([]float32, 1536)
	_, err = client.InsertVector("http://example.org/hasEmbedding", "http://example.org/paper1", embedding)
	if err != nil {
		log.Fatal(err)
	}
}
```

## License

Apache-2.0
