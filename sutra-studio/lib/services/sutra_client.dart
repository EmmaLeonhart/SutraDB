import 'dart:convert';
import 'package:http/http.dart' as http;
import '../models/connection_config.dart';
import '../models/triple.dart';

/// Dart client for the SutraDB HTTP API.
///
/// Mirrors the TypeScript SDK interface but adds auth support
/// and connection health monitoring.
class SutraClient {
  ConnectionConfig config;
  final http.Client _http;

  SutraClient({ConnectionConfig? config, http.Client? httpClient})
      : config = config ?? const ConnectionConfig(),
        _http = httpClient ?? http.Client();

  String get _base => config.endpoint.replaceAll(RegExp(r'/+$'), '');

  Map<String, String> get _headers => {
        'User-Agent': 'sutra-studio/0.1.0',
        ...config.authHeaders,
      };

  // ── Health ──────────────────────────────────────────────────────

  /// Check if the server is reachable.
  Future<bool> health() async {
    try {
      final res = await _http
          .get(Uri.parse('$_base/health'), headers: _headers)
          .timeout(config.timeout);
      return res.statusCode >= 200 && res.statusCode < 300;
    } catch (_) {
      return false;
    }
  }

  // ── SPARQL ─────────────────────────────────────────────────────

  /// Execute a SPARQL query and return parsed results.
  Future<SparqlResult> query(String sparql) async {
    final uri = Uri.parse('$_base/sparql').replace(
      queryParameters: {'query': sparql},
    );
    final res = await _http.get(uri, headers: {
      ..._headers,
      'Accept': 'application/sparql-results+json',
    }).timeout(config.timeout);

    if (res.statusCode != 200) {
      throw SutraClientException('Query failed: ${res.statusCode} ${res.body}');
    }

    final json = jsonDecode(res.body) as Map<String, dynamic>;
    return SparqlResult.fromJson(json);
  }

  /// Fetch all triples via SPARQL SELECT * WHERE { ?s ?p ?o } LIMIT [limit].
  Future<List<Triple>> fetchTriples({int limit = 1000, int offset = 0}) async {
    final sparql =
        'SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT $limit OFFSET $offset';
    final result = await query(sparql);
    return result.rows.map((r) => Triple.fromSparqlRow(r)).toList();
  }

  /// Fetch triples filtered by subject.
  Future<List<Triple>> fetchTriplesForSubject(String subject) async {
    final sparql =
        'SELECT ?p ?o WHERE { <$subject> ?p ?o }';
    final result = await query(sparql);
    return result.rows
        .map((r) => Triple(
              subject: subject,
              predicate: Triple.shortName(
                  (r['p'] as Map)['value']?.toString() ?? ''),
              object: (r['o'] as Map)['value']?.toString() ?? '',
            ))
        .toList();
  }

  // ── Triple insertion ──────────────────────────────────────────

  /// Insert triples in N-Triples format.
  Future<InsertResponse> insertTriples(String ntriples) async {
    final res = await _http.post(
      Uri.parse('$_base/triples'),
      headers: {
        ..._headers,
        'Content-Type': 'application/n-triples',
      },
      body: ntriples,
    ).timeout(config.timeout);

    if (res.statusCode != 200) {
      throw SutraClientException(
          'Insert failed: ${res.statusCode} ${res.body}');
    }

    final json = jsonDecode(res.body) as Map<String, dynamic>;
    return InsertResponse(
      inserted: json['inserted'] as int? ?? 0,
      errors: (json['errors'] as List?)?.cast<String>() ?? [],
    );
  }

  // ── Vector operations ─────────────────────────────────────────

  /// Declare a vector predicate with HNSW configuration.
  Future<void> declareVector({
    required String predicate,
    required int dimensions,
    int m = 16,
    int efConstruction = 200,
  }) async {
    final res = await _http.post(
      Uri.parse('$_base/vectors/declare'),
      headers: {..._headers, 'Content-Type': 'application/json'},
      body: jsonEncode({
        'predicate': predicate,
        'dimensions': dimensions,
        'm': m,
        'ef_construction': efConstruction,
      }),
    ).timeout(config.timeout);

    if (res.statusCode != 200) {
      throw SutraClientException(
          'Declare vector failed: ${res.statusCode} ${res.body}');
    }
  }

  /// Insert a single vector embedding.
  Future<void> insertVector({
    required String predicate,
    required String subject,
    required List<double> vector,
  }) async {
    final res = await _http.post(
      Uri.parse('$_base/vectors'),
      headers: {..._headers, 'Content-Type': 'application/json'},
      body: jsonEncode({
        'predicate': predicate,
        'subject': subject,
        'vector': vector,
      }),
    ).timeout(config.timeout);

    if (res.statusCode != 200) {
      throw SutraClientException(
          'Insert vector failed: ${res.statusCode} ${res.body}');
    }
  }

  // ── Graph export ──────────────────────────────────────────────

  /// Export the full graph as Turtle.
  Future<String> exportGraph({String format = 'turtle'}) async {
    final uri = Uri.parse('$_base/graph').replace(
      queryParameters: format == 'ntriples' ? {'format': 'ntriples'} : {},
    );
    final res =
        await _http.get(uri, headers: _headers).timeout(config.timeout);
    if (res.statusCode != 200) {
      throw SutraClientException(
          'Export failed: ${res.statusCode} ${res.body}');
    }
    return res.body;
  }

  // ── Database health diagnostics ───────────────────────────────

  /// Get basic database statistics by counting triples and types.
  Future<DbStats> stats() async {
    try {
      final countResult = await query(
          'SELECT (COUNT(*) AS ?count) WHERE { ?s ?p ?o }');
      // COUNT isn't implemented yet — fall back to fetching a batch
      final total = int.tryParse(
              countResult.rows.firstOrNull?['count']?['value'] ?? '') ??
          -1;

      final typeResult = await query(
          'SELECT ?type (COUNT(?s) AS ?count) WHERE { ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type } GROUP BY ?type');
      final types = <String, int>{};
      for (final row in typeResult.rows) {
        final t = (row['type'] as Map?)?['value']?.toString() ?? '';
        final c =
            int.tryParse((row['count'] as Map?)?['value']?.toString() ?? '') ??
                0;
        if (t.isNotEmpty) types[t] = c;
      }

      return DbStats(totalTriples: total, typeDistribution: types);
    } catch (e) {
      return DbStats(totalTriples: -1, typeDistribution: {});
    }
  }

  void dispose() => _http.close();
}

// ── Response types ────────────────────────────────────────────────

class SparqlResult {
  final List<String> variables;
  final List<Map<String, dynamic>> rows;

  SparqlResult({required this.variables, required this.rows});

  factory SparqlResult.fromJson(Map<String, dynamic> json) {
    final head = json['head'] as Map<String, dynamic>? ?? {};
    final vars = (head['vars'] as List?)?.cast<String>() ?? [];
    final results = json['results'] as Map<String, dynamic>? ?? {};
    final bindings = (results['bindings'] as List?) ?? [];
    return SparqlResult(
      variables: vars,
      rows: bindings.cast<Map<String, dynamic>>(),
    );
  }
}

class InsertResponse {
  final int inserted;
  final List<String> errors;
  const InsertResponse({required this.inserted, required this.errors});
}

class DbStats {
  final int totalTriples;
  final Map<String, int> typeDistribution;
  const DbStats({required this.totalTriples, required this.typeDistribution});
}

class SutraClientException implements Exception {
  final String message;
  const SutraClientException(this.message);
  @override
  String toString() => 'SutraClientException: $message';
}
