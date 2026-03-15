package sutradb

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"
)

// SutraClient is an HTTP client for communicating with a SutraDB instance.
type SutraClient struct {
	// Endpoint is the base URL of the SutraDB instance (no trailing slash).
	Endpoint string

	client *http.Client
}

// NewClient creates a new SutraClient pointing at the given endpoint.
//
// The endpoint should be the base URL without a trailing slash,
// e.g. "http://localhost:7878".
func NewClient(endpoint string) *SutraClient {
	return &SutraClient{
		Endpoint: strings.TrimRight(endpoint, "/"),
		client: &http.Client{
			Timeout: 30 * time.Second,
		},
	}
}

// Health checks whether the SutraDB instance is reachable and healthy.
// Returns true if the server responds with a 2xx status code.
func (c *SutraClient) Health() (bool, error) {
	resp, err := c.client.Get(c.Endpoint + "/health")
	if err != nil {
		return false, fmt.Errorf("sutradb: health check failed: %w", err)
	}
	defer resp.Body.Close()
	io.Copy(io.Discard, resp.Body)
	return resp.StatusCode >= 200 && resp.StatusCode < 300, nil
}

// Sparql executes a SPARQL query and returns the parsed JSON result set.
func (c *SutraClient) Sparql(query string) (*SparqlResults, error) {
	req, err := http.NewRequest(
		http.MethodPost,
		c.Endpoint+"/sparql",
		strings.NewReader(query),
	)
	if err != nil {
		return nil, fmt.Errorf("sutradb: failed to create request: %w", err)
	}
	req.Header.Set("Content-Type", "application/sparql-query")
	req.Header.Set("Accept", "application/sparql-results+json")

	resp, err := c.client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("sutradb: SPARQL request failed: %w", err)
	}
	defer resp.Body.Close()

	if err := checkStatus(resp); err != nil {
		return nil, err
	}

	var results SparqlResults
	if err := json.NewDecoder(resp.Body).Decode(&results); err != nil {
		return nil, fmt.Errorf("sutradb: failed to decode SPARQL results: %w", err)
	}
	return &results, nil
}

// InsertTriples inserts triples in N-Triples format.
func (c *SutraClient) InsertTriples(ntriples string) (*InsertResult, error) {
	req, err := http.NewRequest(
		http.MethodPost,
		c.Endpoint+"/triples",
		strings.NewReader(ntriples),
	)
	if err != nil {
		return nil, fmt.Errorf("sutradb: failed to create request: %w", err)
	}
	req.Header.Set("Content-Type", "application/n-triples")

	resp, err := c.client.Do(req)
	if err != nil {
		return nil, fmt.Errorf("sutradb: insert request failed: %w", err)
	}
	defer resp.Body.Close()

	if err := checkStatus(resp); err != nil {
		return nil, err
	}

	var result InsertResult
	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
		return nil, fmt.Errorf("sutradb: failed to decode insert result: %w", err)
	}
	return &result, nil
}

// DeclareVector declares a vector predicate with the given dimensionality.
// Optional HNSW parameters can be provided via VectorOption functions.
func (c *SutraClient) DeclareVector(predicate string, dimensions int, opts ...VectorOption) (*DeclareVectorResult, error) {
	options := &vectorOptions{}
	for _, opt := range opts {
		opt(options)
	}

	body := declareVectorRequest{
		Predicate:          predicate,
		Dimensions:         dimensions,
		HnswM:              options.hnswM,
		HnswEfConstruction: options.hnswEfConstruction,
	}

	var result DeclareVectorResult
	if err := c.postJSON("/vectors/declare", body, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// InsertVector inserts a vector for the given subject under the specified predicate.
// The predicate must have been previously declared with DeclareVector,
// and the vector length must match the declared dimensionality.
func (c *SutraClient) InsertVector(predicate, subject string, vector []float32) (*InsertVectorResult, error) {
	body := insertVectorRequest{
		Predicate: predicate,
		Subject:   subject,
		Vector:    vector,
	}

	var result InsertVectorResult
	if err := c.postJSON("/vectors", body, &result); err != nil {
		return nil, err
	}
	return &result, nil
}

// postJSON sends a POST request with a JSON body and decodes the response.
func (c *SutraClient) postJSON(path string, body interface{}, result interface{}) error {
	jsonBody, err := json.Marshal(body)
	if err != nil {
		return fmt.Errorf("sutradb: failed to marshal request body: %w", err)
	}

	req, err := http.NewRequest(
		http.MethodPost,
		c.Endpoint+path,
		bytes.NewReader(jsonBody),
	)
	if err != nil {
		return fmt.Errorf("sutradb: failed to create request: %w", err)
	}
	req.Header.Set("Content-Type", "application/json")

	resp, err := c.client.Do(req)
	if err != nil {
		return fmt.Errorf("sutradb: request failed: %w", err)
	}
	defer resp.Body.Close()

	if err := checkStatus(resp); err != nil {
		return err
	}

	if err := json.NewDecoder(resp.Body).Decode(result); err != nil {
		return fmt.Errorf("sutradb: failed to decode response: %w", err)
	}
	return nil
}

// checkStatus returns a SutraError if the response status is not 2xx.
func checkStatus(resp *http.Response) error {
	if resp.StatusCode >= 200 && resp.StatusCode < 300 {
		return nil
	}
	body, _ := io.ReadAll(resp.Body)
	return &SutraError{
		StatusCode: resp.StatusCode,
		Message:    string(body),
	}
}
