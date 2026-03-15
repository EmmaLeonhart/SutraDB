// Package sutradb provides a Go client for SutraDB, an RDF-star triplestore
// with native HNSW vector indexing.
package sutradb

import "fmt"

// SutraError represents an error returned by the SutraDB server.
type SutraError struct {
	StatusCode int
	Message    string
}

func (e *SutraError) Error() string {
	return fmt.Sprintf("sutradb: HTTP %d: %s", e.StatusCode, e.Message)
}

// BindingValue is a single value within a SPARQL result binding row.
type BindingValue struct {
	Type     string `json:"type"`
	Value    string `json:"value"`
	Datatype string `json:"datatype,omitempty"`
	Lang     string `json:"xml:lang,omitempty"`
}

// SparqlHead is the head section of a SPARQL JSON response.
type SparqlHead struct {
	Vars []string `json:"vars"`
}

// SparqlBindings is the results section of a SPARQL JSON response.
type SparqlBindings struct {
	Bindings []map[string]BindingValue `json:"bindings"`
}

// SparqlResults is the full SPARQL JSON results object.
type SparqlResults struct {
	Head    SparqlHead     `json:"head"`
	Results SparqlBindings `json:"results"`
}

// InsertResult is the response from an insert triples operation.
type InsertResult struct {
	Inserted int64  `json:"inserted"`
	Message  string `json:"message,omitempty"`
}

// DeclareVectorResult is the response from a declare vector predicate operation.
type DeclareVectorResult struct {
	Predicate  string `json:"predicate"`
	Dimensions int    `json:"dimensions"`
	Message    string `json:"message,omitempty"`
}

// InsertVectorResult is the response from an insert vector operation.
type InsertVectorResult struct {
	Success bool   `json:"success"`
	Message string `json:"message,omitempty"`
}

// VectorOption configures optional parameters for vector predicate declaration.
type VectorOption func(*vectorOptions)

type vectorOptions struct {
	hnswM              *int
	hnswEfConstruction *int
}

// WithHnswM sets the max connections per node per layer (default: 16).
func WithHnswM(m int) VectorOption {
	return func(o *vectorOptions) {
		o.hnswM = &m
	}
}

// WithHnswEfConstruction sets the beam width during index construction (default: 200).
func WithHnswEfConstruction(ef int) VectorOption {
	return func(o *vectorOptions) {
		o.hnswEfConstruction = &ef
	}
}

// declareVectorRequest is the JSON body for declaring a vector predicate.
type declareVectorRequest struct {
	Predicate          string `json:"predicate"`
	Dimensions         int    `json:"dimensions"`
	HnswM              *int   `json:"hnswM,omitempty"`
	HnswEfConstruction *int   `json:"hnswEfConstruction,omitempty"`
}

// insertVectorRequest is the JSON body for inserting a vector.
type insertVectorRequest struct {
	Predicate string    `json:"predicate"`
	Subject   string    `json:"subject"`
	Vector    []float32 `json:"vector"`
}
