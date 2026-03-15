/** SPARQL JSON Results format (SELECT queries). */
export interface SparqlResults {
  head: { vars: string[] };
  results: {
    bindings: Record<string, { type: string; value: string }>[];
  };
}

/** Result returned by triple insertion operations. */
export interface InsertResult {
  inserted: number;
  errors: string[];
}

/** Result returned by vector predicate declaration. */
export interface DeclareVectorResult {
  status: string;
  predicate_id: number;
}

/** Result returned by single vector insertion. */
export interface InsertVectorResult {
  status: string;
  triple_id: number;
}

/** Options for HNSW vector predicate declaration. */
export interface DeclareVectorOptions {
  /** HNSW M parameter (max connections per node per layer). Default: 16. */
  m?: number;
  /** HNSW ef_construction beam width. Default: 200. */
  efConstruction?: number;
  /** Distance metric. Default: "cosine". */
  metric?: "cosine" | "euclidean" | "dot";
}

/** Error thrown by the SutraDB client. */
export class SutraError extends Error {
  public readonly statusCode?: number;

  constructor(message: string, statusCode?: number) {
    super(message);
    this.name = "SutraError";
    this.statusCode = statusCode;
    // Restore prototype chain (required for extending built-ins in TS).
    Object.setPrototypeOf(this, SutraError.prototype);
  }
}
