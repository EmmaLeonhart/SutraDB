# SutraDB.Client

C# client for [SutraDB](https://github.com/EmmaLeonhart/SutraDB) — an RDF-star triplestore with native HNSW vector indexing.

Targets .NET 8.0. No external dependencies beyond `System.Text.Json`.

## Installation

```sh
dotnet add package SutraDB.Client
```

## Usage

```csharp
using SutraDB.Client;

var client = new SutraClient("http://localhost:7878");

// Health check
bool alive = await client.HealthAsync();

// Insert triples
var insertResult = await client.InsertTriplesAsync(
    "<http://example.org/paper1> <http://example.org/title> \"Graph Databases\" ."
);

// SPARQL query
var results = await client.SparqlAsync("SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10");
foreach (var row in results.Results.Bindings)
{
    Console.WriteLine(row["s"].Value);
}

// Declare a vector predicate
await client.DeclareVectorAsync("http://example.org/hasEmbedding", 1536);

// Insert a vector
var embedding = new float[1536];
await client.InsertVectorAsync("http://example.org/hasEmbedding", "http://example.org/paper1", embedding);
```

## License

Apache-2.0
