package dev.sutradb;

import org.json.JSONObject;

import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;
import java.util.StringJoiner;

/**
 * Synchronous client for communicating with a SutraDB instance.
 *
 * <p>Uses the built-in {@link java.net.http.HttpClient} (Java 11+)
 * and {@link org.json.JSONObject} for JSON handling.</p>
 *
 * <h3>Example</h3>
 * <pre>{@code
 * SutraClient client = new SutraClient("http://localhost:7878");
 * boolean healthy = client.health();
 * SparqlResults results = client.sparql("SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5");
 * }</pre>
 */
public class SutraClient {

    private final String endpoint;
    private final HttpClient httpClient;

    /**
     * Create a new client pointing at the given SutraDB endpoint.
     *
     * @param endpoint base URL without trailing slash, e.g. {@code "http://localhost:7878"}
     */
    public SutraClient(String endpoint) {
        this.endpoint = endpoint.replaceAll("/+$", "");
        this.httpClient = HttpClient.newBuilder()
                .connectTimeout(Duration.ofSeconds(10))
                .build();
    }

    /**
     * Check whether the SutraDB instance is reachable and healthy.
     *
     * @return true if the server returns a 2xx status
     * @throws SutraError if the request fails
     */
    public boolean health() {
        HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/health"))
                .GET()
                .timeout(Duration.ofSeconds(5))
                .build();

        HttpResponse<String> response = send(request);
        return response.statusCode() >= 200 && response.statusCode() < 300;
    }

    /**
     * Execute a SPARQL query and return parsed results.
     *
     * @param query a SPARQL query string
     * @return parsed SPARQL results
     * @throws SutraError if the query fails or the server returns an error
     */
    public SparqlResults sparql(String query) {
        HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/sparql"))
                .header("Content-Type", "application/sparql-query")
                .header("Accept", "application/sparql-results+json")
                .POST(HttpRequest.BodyPublishers.ofString(query))
                .timeout(Duration.ofSeconds(30))
                .build();

        HttpResponse<String> response = send(request);
        requireSuccess(response);
        return new SparqlResults(new JSONObject(response.body()));
    }

    /**
     * Insert triples in N-Triples format.
     *
     * @param ntriples valid N-Triples data
     * @return the server response as a JSONObject
     * @throws SutraError if the insertion fails
     */
    public JSONObject insertTriples(String ntriples) {
        HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/triples"))
                .header("Content-Type", "application/n-triples")
                .POST(HttpRequest.BodyPublishers.ofString(ntriples))
                .timeout(Duration.ofSeconds(30))
                .build();

        HttpResponse<String> response = send(request);
        requireSuccess(response);
        return new JSONObject(response.body());
    }

    /**
     * Declare a vector predicate with the given dimensionality.
     *
     * @param predicate  the predicate IRI
     * @param dimensions the vector dimensionality
     * @return the server response as a JSONObject
     * @throws SutraError if the declaration fails
     */
    public JSONObject declareVector(String predicate, int dimensions) {
        return declareVector(predicate, dimensions, 16, 200);
    }

    /**
     * Declare a vector predicate with full HNSW parameters.
     *
     * @param predicate        the predicate IRI
     * @param dimensions       the vector dimensionality
     * @param hnswM            max connections per node per layer
     * @param hnswEfConstruction beam width during index construction
     * @return the server response as a JSONObject
     * @throws SutraError if the declaration fails
     */
    public JSONObject declareVector(String predicate, int dimensions, int hnswM, int hnswEfConstruction) {
        JSONObject body = new JSONObject();
        body.put("predicate", predicate);
        body.put("dimensions", dimensions);
        body.put("hnswM", hnswM);
        body.put("hnswEfConstruction", hnswEfConstruction);

        HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/vectors/declare"))
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(body.toString()))
                .timeout(Duration.ofSeconds(10))
                .build();

        HttpResponse<String> response = send(request);
        requireSuccess(response);
        return new JSONObject(response.body());
    }

    /**
     * Insert a vector for the given subject under the specified predicate.
     *
     * @param predicate the predicate IRI (must be previously declared)
     * @param subject   the subject IRI
     * @param vector    the embedding vector
     * @return the server response as a JSONObject
     * @throws SutraError if the insertion fails
     */
    public JSONObject insertVector(String predicate, String subject, double[] vector) {
        JSONObject body = new JSONObject();
        body.put("predicate", predicate);
        body.put("subject", subject);

        // Build the vector array manually to avoid JSONArray boxing issues
        StringJoiner sj = new StringJoiner(",", "[", "]");
        for (double v : vector) {
            sj.add(String.valueOf(v));
        }

        // Construct full JSON with raw array to preserve numeric precision
        String json = String.format(
                "{\"predicate\":%s,\"subject\":%s,\"vector\":%s}",
                JSONObject.quote(predicate),
                JSONObject.quote(subject),
                sj.toString()
        );

        HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/vectors"))
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(json))
                .timeout(Duration.ofSeconds(10))
                .build();

        HttpResponse<String> response = send(request);
        requireSuccess(response);
        return new JSONObject(response.body());
    }

    /**
     * Compact and rebuild all HNSW indexes on the server.
     *
     * <p>This operation may take a long time depending on the number
     * of indexed vectors. A 60-second timeout is used.</p>
     *
     * @return the server response as a JSONObject
     * @throws SutraError if the rebuild fails
     */
    public JSONObject rebuildHnsw() {
        HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/vectors/rebuild"))
                .POST(HttpRequest.BodyPublishers.noBody())
                .timeout(Duration.ofSeconds(60))
                .build();

        HttpResponse<String> response = send(request);
        requireSuccess(response);
        return new JSONObject(response.body());
    }

    /**
     * Get a combined health report including general health and vector index status.
     *
     * <p>Calls both {@code GET /health} and {@code GET /vectors/health},
     * returning a single JSON object with keys {@code "healthy"} (boolean)
     * and {@code "vectors"} (vector index details).</p>
     *
     * @return a combined health report as a JSONObject
     * @throws SutraError if either health endpoint fails
     */
    public JSONObject healthReport() {
        // Check general health
        HttpRequest healthReq = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/health"))
                .GET()
                .timeout(Duration.ofSeconds(5))
                .build();

        HttpResponse<String> healthResp = send(healthReq);
        boolean healthy = healthResp.statusCode() >= 200 && healthResp.statusCode() < 300;

        // Get vector health details
        HttpRequest vectorReq = HttpRequest.newBuilder()
                .uri(URI.create(endpoint + "/vectors/health"))
                .GET()
                .header("Accept", "application/json")
                .timeout(Duration.ofSeconds(10))
                .build();

        HttpResponse<String> vectorResp = send(vectorReq);
        requireSuccess(vectorResp);

        JSONObject report = new JSONObject();
        report.put("healthy", healthy);
        report.put("vectors", new JSONObject(vectorResp.body()));
        return report;
    }

    /**
     * Return the base endpoint URL this client is configured with.
     *
     * @return the endpoint URL
     */
    public String getEndpoint() {
        return endpoint;
    }

    // ---- internal helpers ----

    private HttpResponse<String> send(HttpRequest request) {
        try {
            return httpClient.send(request, HttpResponse.BodyHandlers.ofString());
        } catch (IOException e) {
            throw new SutraError("HTTP request failed: " + e.getMessage(), e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new SutraError("HTTP request interrupted", e);
        }
    }

    private void requireSuccess(HttpResponse<String> response) {
        int status = response.statusCode();
        if (status < 200 || status >= 300) {
            throw new SutraError(
                    "SutraDB returned HTTP " + status + ": " + response.body(),
                    status
            );
        }
    }
}
