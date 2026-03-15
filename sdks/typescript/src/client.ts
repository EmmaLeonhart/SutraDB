import {
  SparqlResults,
  InsertResult,
  DeclareVectorResult,
  DeclareVectorOptions,
  InsertVectorResult,
  SutraError,
} from "./types";

/**
 * Client for interacting with a SutraDB server.
 *
 * Uses the built-in `fetch` API (Node 18+). No external dependencies required.
 */
export class SutraClient {
  private readonly endpoint: string;

  /**
   * Create a new SutraDB client.
   *
   * @param endpoint - Base URL of the SutraDB HTTP server.
   *   Defaults to `http://localhost:3030`.
   */
  constructor(endpoint: string = "http://localhost:3030") {
    this.endpoint = endpoint.replace(/\/+$/, "");
  }

  // ------------------------------------------------------------------
  // Internal helpers
  // ------------------------------------------------------------------

  private url(path: string): string {
    return `${this.endpoint}${path}`;
  }

  private async request(
    method: string,
    path: string,
    options: {
      params?: Record<string, string>;
      body?: string;
      json?: unknown;
      headers?: Record<string, string>;
    } = {}
  ): Promise<Response> {
    let url = this.url(path);

    if (options.params) {
      const qs = new URLSearchParams(options.params).toString();
      url = `${url}?${qs}`;
    }

    const headers: Record<string, string> = {
      "User-Agent": "sutradb-typescript/0.1.0",
      ...options.headers,
    };

    let body: string | undefined;
    if (options.json !== undefined) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(options.json);
    } else if (options.body !== undefined) {
      body = options.body;
    }

    let response: Response;
    try {
      response = await fetch(url, { method, headers, body });
    } catch (err) {
      throw new SutraError(
        `Connection error: ${err instanceof Error ? err.message : String(err)}`
      );
    }

    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new SutraError(
        `HTTP ${response.status}: ${text}`,
        response.status
      );
    }

    return response;
  }

  // ------------------------------------------------------------------
  // Public API
  // ------------------------------------------------------------------

  /**
   * Check whether the server is reachable.
   *
   * @returns `true` if `GET /health` returns a 2xx status, `false` otherwise.
   */
  async health(): Promise<boolean> {
    try {
      await this.request("GET", "/health");
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Execute a SPARQL query and return the parsed JSON result.
   *
   * @param query - A SPARQL 1.1 query string.
   * @returns Parsed SPARQL JSON Results.
   */
  async sparql(query: string): Promise<SparqlResults> {
    const response = await this.request("GET", "/sparql", {
      params: { query },
      headers: { Accept: "application/sparql-results+json" },
    });
    return (await response.json()) as SparqlResults;
  }

  /**
   * Insert triples in N-Triples format, optionally in batches.
   *
   * @param ntriples - One or more triples in N-Triples syntax (one per line).
   * @param batchSize - Maximum number of triples per HTTP request. Default: 5000.
   * @returns Summary of insertions and any errors.
   */
  async insertTriples(
    ntriples: string,
    batchSize: number = 5000
  ): Promise<InsertResult> {
    const lines = ntriples
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l.length > 0);

    let totalInserted = 0;
    const errors: string[] = [];

    for (let start = 0; start < lines.length; start += batchSize) {
      const batch = lines.slice(start, start + batchSize).join("\n");
      try {
        const response = await this.request("POST", "/triples", {
          body: batch,
          headers: { "Content-Type": "application/n-triples" },
        });
        const body = (await response.json()) as {
          inserted?: number;
          errors?: string[];
        };
        totalInserted += body.inserted ?? 0;
        if (body.errors) {
          errors.push(...body.errors);
        }
      } catch (err) {
        errors.push(
          err instanceof Error ? err.message : String(err)
        );
      }
    }

    return { inserted: totalInserted, errors };
  }

  /**
   * Declare an HNSW-indexed vector predicate.
   *
   * @param predicate - IRI of the vector predicate.
   * @param dimensions - Fixed dimensionality of vectors for this predicate.
   * @param options - Optional HNSW tuning parameters.
   * @returns Server response with status and predicate ID.
   */
  async declareVector(
    predicate: string,
    dimensions: number,
    options: DeclareVectorOptions = {}
  ): Promise<DeclareVectorResult> {
    const { m = 16, efConstruction = 200, metric = "cosine" } = options;

    const response = await this.request("POST", "/vectors/declare", {
      json: {
        predicate,
        dimensions,
        m,
        ef_construction: efConstruction,
        metric,
      },
    });
    return (await response.json()) as DeclareVectorResult;
  }

  /**
   * Insert a single vector embedding.
   *
   * @param predicate - IRI of the vector predicate.
   * @param subject - IRI of the subject node.
   * @param vector - The embedding as an array of numbers.
   * @returns Server response with status and triple ID.
   */
  async insertVector(
    predicate: string,
    subject: string,
    vector: number[]
  ): Promise<InsertVectorResult> {
    const response = await this.request("POST", "/vectors", {
      json: { predicate, subject, vector },
    });
    return (await response.json()) as InsertVectorResult;
  }
}
