use serde::Deserialize;

/// A single binding value returned in a SPARQL result set.
#[derive(Debug, Clone, Deserialize)]
pub struct BindingValue {
    /// The RDF term type: `"uri"`, `"literal"`, or `"bnode"`.
    #[serde(rename = "type")]
    pub value_type: String,

    /// The lexical value of the binding.
    pub value: String,

    /// The datatype IRI, if this is a typed literal.
    #[serde(default)]
    pub datatype: Option<String>,

    /// The language tag, if this is a language-tagged literal.
    #[serde(default, rename = "xml:lang")]
    pub lang: Option<String>,
}

/// The results portion of a SPARQL JSON response.
#[derive(Debug, Clone, Deserialize)]
pub struct ResultSet {
    /// The variable names in the result set.
    pub vars: Vec<String>,
}

/// A single row in a SPARQL result set.
pub type BindingRow = std::collections::HashMap<String, BindingValue>;

/// The full SPARQL JSON results object.
#[derive(Debug, Clone, Deserialize)]
pub struct SparqlResults {
    /// Head section containing variable names.
    pub head: ResultSet,

    /// The result bindings.
    pub results: SparqlBindings,
}

/// The bindings section of a SPARQL JSON response.
#[derive(Debug, Clone, Deserialize)]
pub struct SparqlBindings {
    /// Each row is a map from variable name to binding value.
    pub bindings: Vec<BindingRow>,
}

/// Response from an insert triples operation.
#[derive(Debug, Clone, Deserialize)]
pub struct InsertResult {
    /// Number of triples successfully inserted.
    pub inserted: u64,

    /// Human-readable status message.
    #[serde(default)]
    pub message: Option<String>,
}

/// Response from a declare vector predicate operation.
#[derive(Debug, Clone, Deserialize)]
pub struct DeclareVectorResult {
    /// The predicate that was declared.
    pub predicate: String,

    /// The dimensionality that was set.
    pub dimensions: u32,

    /// Human-readable status message.
    #[serde(default)]
    pub message: Option<String>,
}

/// Response from an insert vector operation.
#[derive(Debug, Clone, Deserialize)]
pub struct InsertVectorResult {
    /// Whether the insertion was successful.
    pub success: bool,

    /// Human-readable status message.
    #[serde(default)]
    pub message: Option<String>,
}
