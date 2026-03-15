using System.Net.Http.Json;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace SutraDB.Client;

/// <summary>
/// Async client for communicating with a SutraDB instance.
/// </summary>
/// <example>
/// <code>
/// var client = new SutraClient("http://localhost:7878");
/// var healthy = await client.HealthAsync();
/// var results = await client.SparqlAsync("SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10");
/// </code>
/// </example>
public class SutraClient : IDisposable
{
    private readonly HttpClient _http;
    private readonly string _endpoint;
    private readonly JsonSerializerOptions _jsonOptions;
    private bool _disposed;

    /// <summary>
    /// Create a new client pointing at the given SutraDB endpoint.
    /// </summary>
    /// <param name="endpoint">Base URL without trailing slash, e.g. "http://localhost:7878"</param>
    /// <param name="httpClient">Optional pre-configured HttpClient. If null, a new one is created.</param>
    public SutraClient(string endpoint, HttpClient? httpClient = null)
    {
        _endpoint = endpoint.TrimEnd('/');
        _http = httpClient ?? new HttpClient();
        _http.DefaultRequestHeaders.UserAgent.ParseAdd("sutradb-dotnet-sdk/0.1.0");
        _jsonOptions = new JsonSerializerOptions
        {
            PropertyNameCaseInsensitive = true,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        };
    }

    /// <summary>
    /// The base endpoint URL this client is configured with.
    /// </summary>
    public string Endpoint => _endpoint;

    /// <summary>
    /// Check whether the SutraDB instance is reachable and healthy.
    /// </summary>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>True if the server responds with a 2xx status.</returns>
    public async Task<bool> HealthAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            var response = await _http.GetAsync($"{_endpoint}/health", cancellationToken);
            return response.IsSuccessStatusCode;
        }
        catch (HttpRequestException)
        {
            return false;
        }
    }

    /// <summary>
    /// Execute a SPARQL query and return parsed results.
    /// </summary>
    /// <param name="query">A SPARQL query string.</param>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>Parsed SPARQL results.</returns>
    /// <exception cref="SutraException">If the query fails.</exception>
    public async Task<SparqlResults> SparqlAsync(string query, CancellationToken cancellationToken = default)
    {
        var content = new StringContent(query, Encoding.UTF8, "application/sparql-query");
        var request = new HttpRequestMessage(HttpMethod.Post, $"{_endpoint}/sparql")
        {
            Content = content,
        };
        request.Headers.Accept.ParseAdd("application/sparql-results+json");

        var response = await _http.SendAsync(request, cancellationToken);
        await EnsureSuccessAsync(response, cancellationToken);

        var results = await response.Content.ReadFromJsonAsync<SparqlResults>(_jsonOptions, cancellationToken);
        return results ?? throw new SutraException("Empty response from server");
    }

    /// <summary>
    /// Insert triples in N-Triples format.
    /// </summary>
    /// <param name="ntriples">Valid N-Triples data.</param>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>Insert result with count of inserted triples.</returns>
    /// <exception cref="SutraException">If the insertion fails.</exception>
    public async Task<InsertResult> InsertTriplesAsync(string ntriples, CancellationToken cancellationToken = default)
    {
        var content = new StringContent(ntriples, Encoding.UTF8, "application/n-triples");
        var response = await _http.PostAsync($"{_endpoint}/store", content, cancellationToken);
        await EnsureSuccessAsync(response, cancellationToken);

        var result = await response.Content.ReadFromJsonAsync<InsertResult>(_jsonOptions, cancellationToken);
        return result ?? throw new SutraException("Empty response from server");
    }

    /// <summary>
    /// Declare a vector predicate with the given dimensionality.
    /// </summary>
    /// <param name="predicate">The predicate IRI.</param>
    /// <param name="dimensions">Vector dimensionality.</param>
    /// <param name="hnswM">Max connections per node per layer (default: 16).</param>
    /// <param name="hnswEfConstruction">Beam width during index construction (default: 200).</param>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>Declaration result.</returns>
    /// <exception cref="SutraException">If the declaration fails.</exception>
    public async Task<DeclareVectorResult> DeclareVectorAsync(
        string predicate,
        int dimensions,
        int? hnswM = null,
        int? hnswEfConstruction = null,
        CancellationToken cancellationToken = default)
    {
        var body = new DeclareVectorRequest(predicate, dimensions, hnswM, hnswEfConstruction);
        var response = await _http.PostAsJsonAsync($"{_endpoint}/vectors/declare", body, _jsonOptions, cancellationToken);
        await EnsureSuccessAsync(response, cancellationToken);

        var result = await response.Content.ReadFromJsonAsync<DeclareVectorResult>(_jsonOptions, cancellationToken);
        return result ?? throw new SutraException("Empty response from server");
    }

    /// <summary>
    /// Insert a vector for the given subject under the specified predicate.
    /// </summary>
    /// <param name="predicate">The predicate IRI (must be previously declared).</param>
    /// <param name="subject">The subject IRI.</param>
    /// <param name="vector">The embedding vector.</param>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>Insertion result.</returns>
    /// <exception cref="SutraException">If the insertion fails.</exception>
    public async Task<InsertVectorResult> InsertVectorAsync(
        string predicate,
        string subject,
        float[] vector,
        CancellationToken cancellationToken = default)
    {
        var body = new InsertVectorRequest(predicate, subject, vector);
        var response = await _http.PostAsJsonAsync($"{_endpoint}/vectors/insert", body, _jsonOptions, cancellationToken);
        await EnsureSuccessAsync(response, cancellationToken);

        var result = await response.Content.ReadFromJsonAsync<InsertVectorResult>(_jsonOptions, cancellationToken);
        return result ?? throw new SutraException("Empty response from server");
    }

    /// <summary>
    /// Dispose the underlying HttpClient if it was created by this instance.
    /// </summary>
    public void Dispose()
    {
        if (!_disposed)
        {
            _http.Dispose();
            _disposed = true;
        }
        GC.SuppressFinalize(this);
    }

    private static async Task EnsureSuccessAsync(HttpResponseMessage response, CancellationToken cancellationToken)
    {
        if (!response.IsSuccessStatusCode)
        {
            var body = await response.Content.ReadAsStringAsync(cancellationToken);
            throw new SutraException(
                $"SutraDB returned HTTP {(int)response.StatusCode}: {body}",
                (int)response.StatusCode
            );
        }
    }
}
