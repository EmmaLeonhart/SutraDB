package io.github.emmaleonhart.sutradb;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpHandler;
import com.sun.net.httpserver.HttpServer;
import org.json.JSONObject;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Unit tests for {@link SutraClient} using an embedded JDK HTTP server for mocking.
 */
class SutraClientTest {

    private HttpServer server;
    private SutraClient client;

    @BeforeEach
    void setUp() throws IOException {
        server = HttpServer.create(new InetSocketAddress(0), 0);
        server.setExecutor(null);
        server.start();
        int port = server.getAddress().getPort();
        client = new SutraClient("http://localhost:" + port);
    }

    @AfterEach
    void tearDown() {
        server.stop(0);
    }

    // ---- health() tests ----

    @Test
    void healthReturnsTrueOn200() {
        server.createContext("/health", exchange -> {
            respond(exchange, 200, "{\"status\":\"ok\"}");
        });

        assertTrue(client.health());
    }

    @Test
    void healthReturnsFalseOn500() {
        server.createContext("/health", exchange -> {
            respond(exchange, 500, "{\"error\":\"down\"}");
        });

        assertFalse(client.health());
    }

    // ---- sparql() tests ----

    @Test
    void sparqlSendsCorrectContentTypeAndParsesResponse() {
        String sparqlResponse = "{" +
                "\"head\":{\"vars\":[\"s\",\"p\",\"o\"]}," +
                "\"results\":{\"bindings\":[{" +
                "\"s\":{\"type\":\"uri\",\"value\":\"http://example.org/a\"}," +
                "\"p\":{\"type\":\"uri\",\"value\":\"http://example.org/b\"}," +
                "\"o\":{\"type\":\"literal\",\"value\":\"hello\"}" +
                "}]}" +
                "}";

        server.createContext("/sparql", exchange -> {
            // Verify content type
            String contentType = exchange.getRequestHeaders().getFirst("Content-Type");
            assertEquals("application/sparql-query", contentType);

            // Verify request body is the query
            String body = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
            assertEquals("SELECT ?s WHERE { ?s ?p ?o }", body);

            respond(exchange, 200, sparqlResponse);
        });

        SparqlResults results = client.sparql("SELECT ?s WHERE { ?s ?p ?o }");
        assertEquals(3, results.getVariables().size());
        assertEquals(1, results.size());
        assertEquals("http://example.org/a", results.getBindings().get(0).get("s").getValue());
    }

    // ---- insertTriples() tests ----

    @Test
    void insertTriplesSendsNTriplesContentType() {
        String ntriples = "<http://ex.org/s> <http://ex.org/p> \"value\" .";

        server.createContext("/triples", exchange -> {
            String contentType = exchange.getRequestHeaders().getFirst("Content-Type");
            assertEquals("application/n-triples", contentType);

            String body = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
            assertEquals(ntriples, body);

            respond(exchange, 200, "{\"inserted\":1}");
        });

        JSONObject result = client.insertTriples(ntriples);
        assertEquals(1, result.getInt("inserted"));
    }

    // ---- declareVector() tests ----

    @Test
    void declareVectorSendsCorrectJson() {
        server.createContext("/vectors/declare", exchange -> {
            String contentType = exchange.getRequestHeaders().getFirst("Content-Type");
            assertEquals("application/json", contentType);

            String body = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
            JSONObject json = new JSONObject(body);
            assertEquals("http://ex.org/hasEmbed", json.getString("predicate"));
            assertEquals(768, json.getInt("dimensions"));
            assertEquals(16, json.getInt("hnswM"));
            assertEquals(200, json.getInt("hnswEfConstruction"));

            respond(exchange, 200, "{\"ok\":true}");
        });

        JSONObject result = client.declareVector("http://ex.org/hasEmbed", 768);
        assertTrue(result.getBoolean("ok"));
    }

    @Test
    void declareVectorWithCustomHnswParams() {
        server.createContext("/vectors/declare", exchange -> {
            String body = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
            JSONObject json = new JSONObject(body);
            assertEquals(32, json.getInt("hnswM"));
            assertEquals(400, json.getInt("hnswEfConstruction"));

            respond(exchange, 200, "{\"ok\":true}");
        });

        client.declareVector("http://ex.org/hasEmbed", 768, 32, 400);
    }

    // ---- insertVector() tests ----

    @Test
    void insertVectorSendsCorrectJsonWithVectorArray() {
        server.createContext("/vectors", exchange -> {
            String contentType = exchange.getRequestHeaders().getFirst("Content-Type");
            assertEquals("application/json", contentType);

            String body = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
            JSONObject json = new JSONObject(body);
            assertEquals("http://ex.org/pred", json.getString("predicate"));
            assertEquals("http://ex.org/subj", json.getString("subject"));
            assertEquals(3, json.getJSONArray("vector").length());

            respond(exchange, 200, "{\"ok\":true}");
        });

        double[] vec = {0.1, 0.2, 0.3};
        JSONObject result = client.insertVector("http://ex.org/pred", "http://ex.org/subj", vec);
        assertTrue(result.getBoolean("ok"));
    }

    // ---- rebuildHnsw() tests ----

    @Test
    void rebuildHnswCallsPostVectorsRebuild() {
        server.createContext("/vectors/rebuild", exchange -> {
            assertEquals("POST", exchange.getRequestMethod());
            respond(exchange, 200, "{\"rebuilt\":true}");
        });

        JSONObject result = client.rebuildHnsw();
        assertTrue(result.getBoolean("rebuilt"));
    }

    // ---- healthReport() tests ----

    @Test
    void healthReportCombinesHealthAndVectorHealth() {
        server.createContext("/health", exchange -> {
            respond(exchange, 200, "{\"status\":\"ok\"}");
        });
        server.createContext("/vectors/health", exchange -> {
            respond(exchange, 200, "{\"indexes\":2,\"totalVectors\":1000}");
        });

        JSONObject report = client.healthReport();
        assertTrue(report.getBoolean("healthy"));
        assertEquals(2, report.getJSONObject("vectors").getInt("indexes"));
        assertEquals(1000, report.getJSONObject("vectors").getInt("totalVectors"));
    }

    // ---- error handling tests ----

    @Test
    void sparqlThrowsSutraErrorOn400() {
        server.createContext("/sparql", exchange -> {
            respond(exchange, 400, "{\"error\":\"Bad query\"}");
        });

        SutraError error = assertThrows(SutraError.class, () ->
                client.sparql("INVALID QUERY"));
        assertEquals(400, error.getStatusCode());
        assertTrue(error.getMessage().contains("400"));
    }

    @Test
    void insertTriplesThrowsSutraErrorOn500() {
        server.createContext("/triples", exchange -> {
            respond(exchange, 500, "{\"error\":\"Internal error\"}");
        });

        SutraError error = assertThrows(SutraError.class, () ->
                client.insertTriples("<s> <p> <o> ."));
        assertEquals(500, error.getStatusCode());
    }

    @Test
    void connectionRefusedThrowsSutraError() {
        // Use a client pointing at a port that is not listening
        SutraClient badClient = new SutraClient("http://localhost:1");
        assertThrows(SutraError.class, badClient::health);
    }

    @Test
    void getEndpointReturnsConfiguredUrl() {
        assertEquals("http://localhost:" + server.getAddress().getPort(), client.getEndpoint());
    }

    @Test
    void endpointTrailingSlashIsStripped() {
        SutraClient c = new SutraClient("http://localhost:9999/");
        assertEquals("http://localhost:9999", c.getEndpoint());
    }

    // ---- helper ----

    private static void respond(HttpExchange exchange, int status, String body) throws IOException {
        byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
        exchange.getResponseHeaders().set("Content-Type", "application/json");
        exchange.sendResponseHeaders(status, bytes.length);
        try (OutputStream os = exchange.getResponseBody()) {
            os.write(bytes);
        }
    }
}
