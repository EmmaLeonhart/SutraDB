using System.Text.Json.Serialization;

namespace SutraDB.Client;

/// <summary>
/// A single binding value in a SPARQL result row.
/// </summary>
public record BindingValue(
    [property: JsonPropertyName("type")] string Type,
    [property: JsonPropertyName("value")] string Value,
    [property: JsonPropertyName("datatype")] string? Datatype = null,
    [property: JsonPropertyName("xml:lang")] string? Lang = null
);

/// <summary>
/// The head section of a SPARQL JSON response.
/// </summary>
public record SparqlHead(
    [property: JsonPropertyName("vars")] List<string> Vars
);

/// <summary>
/// The results section of a SPARQL JSON response.
/// </summary>
public record SparqlBindings(
    [property: JsonPropertyName("bindings")] List<Dictionary<string, BindingValue>> Bindings
);

/// <summary>
/// Full SPARQL JSON results.
/// </summary>
public record SparqlResults(
    [property: JsonPropertyName("head")] SparqlHead Head,
    [property: JsonPropertyName("results")] SparqlBindings Results
);

/// <summary>
/// Response from an insert triples operation.
/// </summary>
public record InsertResult(
    [property: JsonPropertyName("inserted")] long Inserted,
    [property: JsonPropertyName("message")] string? Message = null
);

/// <summary>
/// Response from a declare vector predicate operation.
/// </summary>
public record DeclareVectorResult(
    [property: JsonPropertyName("predicate")] string Predicate,
    [property: JsonPropertyName("dimensions")] int Dimensions,
    [property: JsonPropertyName("message")] string? Message = null
);

/// <summary>
/// Response from an insert vector operation.
/// </summary>
public record InsertVectorResult(
    [property: JsonPropertyName("success")] bool Success,
    [property: JsonPropertyName("message")] string? Message = null
);

/// <summary>
/// Request body for declaring a vector predicate.
/// </summary>
internal record DeclareVectorRequest(
    [property: JsonPropertyName("predicate")] string Predicate,
    [property: JsonPropertyName("dimensions")] int Dimensions,
    [property: JsonPropertyName("hnswM")] int? HnswM = null,
    [property: JsonPropertyName("hnswEfConstruction")] int? HnswEfConstruction = null
);

/// <summary>
/// Request body for inserting a vector.
/// </summary>
internal record InsertVectorRequest(
    [property: JsonPropertyName("predicate")] string Predicate,
    [property: JsonPropertyName("subject")] string Subject,
    [property: JsonPropertyName("vector")] float[] Vector
);
