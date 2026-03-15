import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/connection_provider.dart';
import '../services/sutra_client.dart';
import '../theme/sutra_theme.dart';

/// SPARQL query editor with results display.
///
/// Intentionally lightweight — the expectation is that most users
/// interact with SutraDB via AI agents or application code. This
/// editor exists for quick ad-hoc queries and debugging.
class SparqlScreen extends StatefulWidget {
  const SparqlScreen({super.key});

  @override
  State<SparqlScreen> createState() => _SparqlScreenState();
}

class _SparqlScreenState extends State<SparqlScreen> {
  final _queryController = TextEditingController(
    text: 'SELECT ?s ?p ?o WHERE {\n  ?s ?p ?o\n} LIMIT 25',
  );
  SparqlResult? _result;
  bool _loading = false;
  String? _error;
  Duration? _elapsed;

  // Quick query templates
  static const _templates = {
    'All triples': 'SELECT ?s ?p ?o WHERE {\n  ?s ?p ?o\n} LIMIT 100',
    'Types': 'SELECT ?type (COUNT(?s) AS ?count) WHERE {\n'
        '  ?s <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> ?type\n'
        '} GROUP BY ?type',
    'Vector search':
        'SELECT ?doc WHERE {\n  VECTOR_SIMILAR(?doc <http://example.org/hasEmbedding>\n'
            '    "0.1 0.2 0.3"^^<sutra:f32vec>, 0.85)\n}',
    'HNSW neighbors':
        'SELECT ?src ?tgt WHERE {\n  ?src <sutra:hnswNeighbor> ?tgt\n} LIMIT 50',
    'Star annotations':
        'SELECT ?s ?p ?o ?ap ?av WHERE {\n  << ?s ?p ?o >> ?ap ?av\n} LIMIT 50',
  };

  Future<void> _runQuery() async {
    final conn = context.read<ConnectionProvider>();
    if (!conn.connected) {
      setState(() => _error = 'Not connected');
      return;
    }

    setState(() {
      _loading = true;
      _error = null;
      _result = null;
    });

    final sw = Stopwatch()..start();
    try {
      final result = await conn.client.query(_queryController.text);
      sw.stop();
      setState(() {
        _result = result;
        _elapsed = sw.elapsed;
        _loading = false;
      });
    } catch (e) {
      sw.stop();
      setState(() {
        _error = e.toString();
        _elapsed = sw.elapsed;
        _loading = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        // Toolbar
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
          decoration: const BoxDecoration(
            color: SutraTheme.surface,
            border: Border(bottom: BorderSide(color: SutraTheme.border)),
          ),
          child: Row(
            children: [
              const Icon(Icons.code, size: 18, color: SutraTheme.accent),
              const SizedBox(width: 8),
              const Text('SPARQL',
                  style: TextStyle(
                      fontWeight: FontWeight.w600, color: SutraTheme.text)),
              const SizedBox(width: 16),

              // Quick templates
              ...(_templates.entries.map((e) => Padding(
                    padding: const EdgeInsets.only(right: 4),
                    child: OutlinedButton(
                      onPressed: () =>
                          _queryController.text = e.value,
                      style: OutlinedButton.styleFrom(
                        visualDensity: VisualDensity.compact,
                        padding: const EdgeInsets.symmetric(horizontal: 8),
                      ),
                      child: Text(e.key,
                          style: const TextStyle(fontSize: 10)),
                    ),
                  ))),

              const Spacer(),
              ElevatedButton.icon(
                onPressed: _loading ? null : _runQuery,
                icon: const Icon(Icons.play_arrow, size: 16),
                label: const Text('Run'),
              ),
            ],
          ),
        ),

        // Editor + results split
        Expanded(
          child: Row(
            children: [
              // Query editor (left)
              SizedBox(
                width: 400,
                child: Container(
                  decoration: const BoxDecoration(
                    border:
                        Border(right: BorderSide(color: SutraTheme.border)),
                  ),
                  child: TextField(
                    controller: _queryController,
                    maxLines: null,
                    expands: true,
                    style: const TextStyle(
                      fontFamily: 'monospace',
                      fontSize: 13,
                      color: SutraTheme.text,
                    ),
                    decoration: const InputDecoration(
                      border: InputBorder.none,
                      contentPadding: EdgeInsets.all(12),
                      hintText: 'Enter SPARQL query...',
                    ),
                    onSubmitted: (_) => _runQuery(),
                  ),
                ),
              ),

              // Results (right)
              Expanded(
                child: _buildResults(),
              ),
            ],
          ),
        ),

        // Status bar
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
          decoration: const BoxDecoration(
            color: SutraTheme.surface,
            border: Border(top: BorderSide(color: SutraTheme.border)),
          ),
          child: Row(
            children: [
              if (_elapsed != null)
                Text(
                  'Query time: ${_elapsed!.inMilliseconds}ms',
                  style: const TextStyle(
                      color: SutraTheme.muted, fontSize: 11),
                ),
              const Spacer(),
              if (_result != null)
                Text(
                  '${_result!.rows.length} result(s)',
                  style: const TextStyle(
                      color: SutraTheme.muted, fontSize: 11),
                ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _buildResults() {
    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Icon(Icons.error_outline,
                  color: SutraTheme.red, size: 32),
              const SizedBox(height: 8),
              SelectableText(
                _error!,
                style: const TextStyle(color: SutraTheme.red, fontSize: 12),
              ),
            ],
          ),
        ),
      );
    }
    if (_result == null) {
      return const Center(
        child: Text(
          'Run a query to see results\n\n'
          'Tip: Most real work happens via AI agents\n'
          'or application SDKs — this editor is for\n'
          'quick debugging and visual inspection.',
          textAlign: TextAlign.center,
          style: TextStyle(color: SutraTheme.muted, fontSize: 13),
        ),
      );
    }

    final r = _result!;
    if (r.rows.isEmpty) {
      return const Center(
        child: Text('No results',
            style: TextStyle(color: SutraTheme.muted)),
      );
    }

    return SingleChildScrollView(
      scrollDirection: Axis.vertical,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: DataTable(
          columns: r.variables
              .map((v) => DataColumn(label: Text(v)))
              .toList(),
          rows: r.rows.map((row) {
            return DataRow(
              cells: r.variables.map((v) {
                final cell = row[v];
                final value = cell is Map
                    ? cell['value']?.toString() ?? ''
                    : cell?.toString() ?? '';
                return DataCell(
                  Tooltip(
                    message: value,
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 300),
                      child: Text(
                        value,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(fontSize: 12),
                      ),
                    ),
                  ),
                );
              }).toList(),
            );
          }).toList(),
        ),
      ),
    );
  }

  @override
  void dispose() {
    _queryController.dispose();
    super.dispose();
  }
}
