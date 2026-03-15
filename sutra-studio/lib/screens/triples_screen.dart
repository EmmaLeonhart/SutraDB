import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../models/triple.dart';
import '../services/connection_provider.dart';
import '../theme/sutra_theme.dart';

/// Triple editor screen with table view and add/delete capabilities.
///
/// Supports:
/// - Viewing all triples as a sortable, filterable table
/// - Adding new triples (N-Triples or form-based)
/// - Deleting triples (when SPARQL Update is available)
/// - RDF-star quoted triple display
/// - Type indicator column (semantic / vector / HNSW)
class TriplesScreen extends StatefulWidget {
  const TriplesScreen({super.key});

  @override
  State<TriplesScreen> createState() => _TriplesScreenState();
}

class _TriplesScreenState extends State<TriplesScreen> {
  List<Triple> _triples = [];
  bool _loading = false;
  String? _error;
  String _filter = '';
  int _limit = 500;
  int _offset = 0;
  final _filterController = TextEditingController();
  _SortColumn _sortColumn = _SortColumn.subject;
  bool _sortAsc = true;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _load());
  }

  Future<void> _load() async {
    final conn = context.read<ConnectionProvider>();
    if (!conn.connected) {
      setState(() => _error = 'Not connected');
      return;
    }
    setState(() {
      _loading = true;
      _error = null;
    });

    try {
      _triples =
          await conn.client.fetchTriples(limit: _limit, offset: _offset);
      setState(() => _loading = false);
    } catch (e) {
      setState(() {
        _loading = false;
        _error = e.toString();
      });
    }
  }

  List<Triple> get _filtered {
    var list = _triples;
    if (_filter.isNotEmpty) {
      final q = _filter.toLowerCase();
      list = list
          .where((t) =>
              t.subject.toLowerCase().contains(q) ||
              t.predicate.toLowerCase().contains(q) ||
              t.object.toLowerCase().contains(q))
          .toList();
    }
    list.sort((a, b) {
      int cmp;
      switch (_sortColumn) {
        case _SortColumn.subject:
          cmp = a.subject.compareTo(b.subject);
        case _SortColumn.predicate:
          cmp = a.predicate.compareTo(b.predicate);
        case _SortColumn.object:
          cmp = a.object.compareTo(b.object);
        case _SortColumn.type:
          cmp = classifyTriple(a).name.compareTo(classifyTriple(b).name);
      }
      return _sortAsc ? cmp : -cmp;
    });
    return list;
  }

  void _sort(_SortColumn col) {
    setState(() {
      if (_sortColumn == col) {
        _sortAsc = !_sortAsc;
      } else {
        _sortColumn = col;
        _sortAsc = true;
      }
    });
  }

  Future<void> _showAddDialog() async {
    final subjectCtrl = TextEditingController();
    final predCtrl = TextEditingController();
    final objCtrl = TextEditingController();
    final ntriplesCtrl = TextEditingController();
    bool useRaw = false;

    final result = await showDialog<bool>(
      context: context,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setDialogState) => AlertDialog(
          title: const Text('Add Triple(s)'),
          backgroundColor: SutraTheme.surface,
          content: SizedBox(
            width: 500,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                // Toggle: form vs raw
                Row(
                  children: [
                    ChoiceChip(
                      label: const Text('Form'),
                      selected: !useRaw,
                      onSelected: (_) =>
                          setDialogState(() => useRaw = false),
                    ),
                    const SizedBox(width: 8),
                    ChoiceChip(
                      label: const Text('N-Triples (raw)'),
                      selected: useRaw,
                      onSelected: (_) =>
                          setDialogState(() => useRaw = true),
                    ),
                  ],
                ),
                const SizedBox(height: 16),

                if (!useRaw) ...[
                  TextField(
                    controller: subjectCtrl,
                    decoration: const InputDecoration(
                      labelText: 'Subject (IRI)',
                      hintText: 'http://example.org/entity1',
                    ),
                  ),
                  const SizedBox(height: 8),
                  TextField(
                    controller: predCtrl,
                    decoration: const InputDecoration(
                      labelText: 'Predicate (IRI)',
                      hintText: 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type',
                    ),
                  ),
                  const SizedBox(height: 8),
                  TextField(
                    controller: objCtrl,
                    decoration: const InputDecoration(
                      labelText: 'Object (IRI or literal)',
                      hintText: 'http://example.org/Person or "John Doe"',
                    ),
                  ),
                ] else ...[
                  TextField(
                    controller: ntriplesCtrl,
                    maxLines: 8,
                    decoration: const InputDecoration(
                      labelText: 'N-Triples',
                      hintText:
                          '<http://ex.org/s> <http://ex.org/p> <http://ex.org/o> .\n'
                          '<< <http://ex.org/s> <http://ex.org/p> <http://ex.org/o> >> <http://ex.org/confidence> "0.95" .',
                      alignLabelWithHint: true,
                    ),
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    'Supports RDF-star quoted triples (<< s p o >>)',
                    style: TextStyle(color: SutraTheme.muted, fontSize: 11),
                  ),
                ],
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: const Text('Cancel'),
            ),
            ElevatedButton(
              onPressed: () => Navigator.pop(ctx, true),
              child: const Text('Insert'),
            ),
          ],
        ),
      ),
    );

    if (result != true) return;

    final conn = context.read<ConnectionProvider>();
    try {
      String ntriples;
      if (useRaw) {
        ntriples = ntriplesCtrl.text;
      } else {
        final t = Triple(
          subject: subjectCtrl.text.trim(),
          predicate: predCtrl.text.trim(),
          object: objCtrl.text.trim(),
        );
        ntriples = t.toNTriples();
      }

      final res = await conn.client.insertTriples(ntriples);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('Inserted ${res.inserted} triple(s)'),
            backgroundColor: SutraTheme.green,
          ),
        );
        _load();
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('Error: $e'),
            backgroundColor: SutraTheme.red,
          ),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final filtered = _filtered;
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
              const Icon(Icons.table_rows, size: 18, color: SutraTheme.accent),
              const SizedBox(width: 8),
              const Text('Triples',
                  style: TextStyle(
                      fontWeight: FontWeight.w600, color: SutraTheme.text)),
              const SizedBox(width: 16),

              // Filter
              SizedBox(
                width: 250,
                child: TextField(
                  controller: _filterController,
                  decoration: const InputDecoration(
                    hintText: 'Filter triples...',
                    prefixIcon: Icon(Icons.search, size: 16),
                    isDense: true,
                    contentPadding: EdgeInsets.symmetric(vertical: 8),
                  ),
                  style: const TextStyle(fontSize: 13),
                  onChanged: (v) => setState(() => _filter = v),
                ),
              ),

              const Spacer(),

              // Pagination
              IconButton(
                icon: const Icon(Icons.chevron_left, size: 18),
                onPressed: _offset > 0
                    ? () {
                        _offset = (_offset - _limit).clamp(0, _offset);
                        _load();
                      }
                    : null,
              ),
              Text('${_offset + 1}–${_offset + filtered.length}',
                  style: const TextStyle(
                      color: SutraTheme.muted, fontSize: 12)),
              IconButton(
                icon: const Icon(Icons.chevron_right, size: 18),
                onPressed: () {
                  _offset += _limit;
                  _load();
                },
              ),

              const SizedBox(width: 8),
              ElevatedButton.icon(
                onPressed: _showAddDialog,
                icon: const Icon(Icons.add, size: 16),
                label: const Text('Add Triple'),
                style: ElevatedButton.styleFrom(
                  visualDensity: VisualDensity.compact,
                ),
              ),
              const SizedBox(width: 8),
              IconButton(
                icon: const Icon(Icons.refresh, size: 18),
                onPressed: _load,
              ),
            ],
          ),
        ),

        // Table
        Expanded(
          child: _loading
              ? const Center(child: CircularProgressIndicator())
              : _error != null
                  ? Center(
                      child: Text(_error!,
                          style: const TextStyle(color: SutraTheme.red)))
                  : filtered.isEmpty
                      ? const Center(
                          child: Text('No triples found',
                              style: TextStyle(color: SutraTheme.muted)))
                      : SingleChildScrollView(
                          scrollDirection: Axis.vertical,
                          child: SingleChildScrollView(
                            scrollDirection: Axis.horizontal,
                            child: DataTable(
                              sortColumnIndex: _sortColumn.index,
                              sortAscending: _sortAsc,
                              columns: [
                                DataColumn(
                                  label: const Text('Type'),
                                  onSort: (_, __) =>
                                      _sort(_SortColumn.type),
                                ),
                                DataColumn(
                                  label: const Text('Subject'),
                                  onSort: (_, __) =>
                                      _sort(_SortColumn.subject),
                                ),
                                DataColumn(
                                  label: const Text('Predicate'),
                                  onSort: (_, __) =>
                                      _sort(_SortColumn.predicate),
                                ),
                                DataColumn(
                                  label: const Text('Object'),
                                  onSort: (_, __) =>
                                      _sort(_SortColumn.object),
                                ),
                              ],
                              rows: filtered.map((t) {
                                final type = classifyTriple(t);
                                return DataRow(cells: [
                                  DataCell(_typeChip(type)),
                                  DataCell(
                                    _iriCell(t.subject),
                                  ),
                                  DataCell(
                                    _iriCell(t.predicate),
                                  ),
                                  DataCell(
                                    _objectCell(t),
                                  ),
                                ]);
                              }).toList(),
                            ),
                          ),
                        ),
        ),
      ],
    );
  }

  Widget _typeChip(TripleType type) {
    Color color;
    String label;
    switch (type) {
      case TripleType.semantic:
        color = SutraTheme.accent;
        label = 'SEM';
      case TripleType.vector:
        color = SutraTheme.purple;
        label = 'VEC';
      case TripleType.hnswEdge:
        color = SutraTheme.orange;
        label = 'HNSW';
    }
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      decoration: BoxDecoration(
        color: color.withOpacity(0.15),
        borderRadius: BorderRadius.circular(4),
        border: Border.all(color: color.withOpacity(0.4)),
      ),
      child: Text(label,
          style: TextStyle(
              color: color, fontSize: 10, fontWeight: FontWeight.w600)),
    );
  }

  Widget _iriCell(String iri) {
    return Tooltip(
      message: iri,
      child: Text(
        Triple.shortName(iri),
        style: const TextStyle(fontSize: 12),
        overflow: TextOverflow.ellipsis,
      ),
    );
  }

  Widget _objectCell(Triple t) {
    if (t.isVector) {
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Icon(Icons.grain, size: 12, color: SutraTheme.purple),
          const SizedBox(width: 4),
          const Text('[vector]',
              style: TextStyle(
                  color: SutraTheme.purple,
                  fontSize: 12,
                  fontStyle: FontStyle.italic)),
        ],
      );
    }
    return Tooltip(
      message: t.object,
      child: Text(
        Triple.shortName(t.object),
        style: const TextStyle(fontSize: 12),
        overflow: TextOverflow.ellipsis,
      ),
    );
  }

  @override
  void dispose() {
    _filterController.dispose();
    super.dispose();
  }
}

enum _SortColumn { type, subject, predicate, object }
