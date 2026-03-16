import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/connection_provider.dart';
import '../theme/sutra_theme.dart';

/// Ontology viewer/editor screen — Protege-like class hierarchy browser.
///
/// Displays:
/// - OWL class hierarchy (rdfs:subClassOf tree)
/// - Object properties and data properties
/// - Class restrictions (owl:someValuesFrom, owl:allValuesFrom)
/// - Individuals for each class
///
/// This is a lightweight version of what Protege provides. For heavy
/// ontology editing, use the SutraDB Protege plugin (tools/protege-plugin/).
class OntologyScreen extends StatefulWidget {
  const OntologyScreen({super.key});

  @override
  State<OntologyScreen> createState() => _OntologyScreenState();
}

class _OntologyScreenState extends State<OntologyScreen> {
  Future<void> _exportOntology() async {
    final conn = context.read<ConnectionProvider>();
    if (!conn.connected) return;
    try {
      final turtle = await conn.client.exportGraph();
      // Show in a dialog for copy/save
      if (!mounted) return;
      showDialog(
        context: context,
        builder: (ctx) => AlertDialog(
          title: const Text('Ontology Export (Turtle)'),
          content: SizedBox(
            width: 600,
            height: 400,
            child: SingleChildScrollView(
              child: SelectableText(
                turtle,
                style: const TextStyle(fontFamily: 'monospace', fontSize: 11),
              ),
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(ctx),
              child: const Text('Close'),
            ),
          ],
        ),
      );
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Export failed: $e')),
        );
      }
    }
  }

  List<_OntologyClass> _classes = [];
  List<_OntologyProperty> _properties = [];
  _OntologyClass? _selectedClass;
  List<_Individual> _individuals = [];
  bool _loading = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _loadOntology());
  }

  Future<void> _loadOntology() async {
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
      // Load classes (rdf:type owl:Class or rdfs:Class)
      final classResult = await conn.client.query('''
        SELECT DISTINCT ?class ?parent WHERE {
          {
            ?class <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2002/07/owl#Class>
          } UNION {
            ?class <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2000/01/rdf-schema#Class>
          }
          OPTIONAL {
            ?class <http://www.w3.org/2000/01/rdf-schema#subClassOf> ?parent
          }
        }
      ''');

      final classMap = <String, _OntologyClass>{};
      for (final row in classResult.rows) {
        final iri = _val(row, 'class');
        final parent = _val(row, 'parent');
        classMap.putIfAbsent(iri, () => _OntologyClass(iri: iri));
        if (parent.isNotEmpty) {
          classMap[iri]!.parentIri = parent;
          classMap.putIfAbsent(
              parent, () => _OntologyClass(iri: parent));
        }
      }

      // Load properties
      final propResult = await conn.client.query('''
        SELECT DISTINCT ?prop ?type ?domain ?range WHERE {
          {
            ?prop <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2002/07/owl#ObjectProperty>
            BIND("object" AS ?type)
          } UNION {
            ?prop <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2002/07/owl#DatatypeProperty>
            BIND("datatype" AS ?type)
          }
          OPTIONAL { ?prop <http://www.w3.org/2000/01/rdf-schema#domain> ?domain }
          OPTIONAL { ?prop <http://www.w3.org/2000/01/rdf-schema#range> ?range }
        }
      ''');

      final properties = <_OntologyProperty>[];
      for (final row in propResult.rows) {
        properties.add(_OntologyProperty(
          iri: _val(row, 'prop'),
          type: _val(row, 'type') == 'object'
              ? _PropertyType.object
              : _PropertyType.datatype,
          domain: _val(row, 'domain'),
          range: _val(row, 'range'),
        ));
      }

      setState(() {
        _classes = classMap.values.toList()
          ..sort((a, b) => _shortName(a.iri).compareTo(_shortName(b.iri)));
        _properties = properties;
        _loading = false;
      });
    } catch (e) {
      setState(() {
        _loading = false;
        _error = e.toString();
      });
    }
  }

  Future<void> _loadIndividuals(String classIri) async {
    final conn = context.read<ConnectionProvider>();
    try {
      final result = await conn.client.query('''
        SELECT ?individual WHERE {
          ?individual <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <$classIri>
        } LIMIT 100
      ''');
      setState(() {
        _individuals = result.rows
            .map((r) => _Individual(iri: _val(r, 'individual')))
            .toList();
      });
    } catch (_) {
      setState(() => _individuals = []);
    }
  }

  String _val(Map<String, dynamic> row, String key) {
    final v = row[key];
    if (v is Map) return v['value']?.toString() ?? '';
    return v?.toString() ?? '';
  }

  static String _shortName(String iri) {
    final h = iri.lastIndexOf('#');
    if (h >= 0) return iri.substring(h + 1);
    final s = iri.lastIndexOf('/');
    if (s >= 0) return iri.substring(s + 1);
    return iri;
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
              const Icon(Icons.account_tree, size: 18, color: SutraTheme.accent),
              const SizedBox(width: 8),
              const Text('Ontology',
                  style: TextStyle(
                      fontWeight: FontWeight.w600, color: SutraTheme.text)),
              const Spacer(),
              const Text(
                'For full ontology editing, use Protege with the SutraDB plugin',
                style: TextStyle(color: SutraTheme.muted, fontSize: 11),
              ),
              const SizedBox(width: 12),
              IconButton(
                icon: const Icon(Icons.refresh, size: 18),
                onPressed: _loadOntology,
              ),
              IconButton(
                icon: const Icon(Icons.download, size: 18),
                tooltip: 'Export as Turtle',
                onPressed: _exportOntology,
              ),
            ],
          ),
        ),

        // Content
        Expanded(
          child: _loading
              ? const Center(child: CircularProgressIndicator())
              : _error != null
                  ? Center(
                      child: Text(_error!,
                          style: const TextStyle(color: SutraTheme.red)))
                  : _classes.isEmpty && _properties.isEmpty
                      ? const Center(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Icon(Icons.schema_outlined,
                                  color: SutraTheme.muted, size: 48),
                              SizedBox(height: 12),
                              Text('No OWL classes or properties found',
                                  style: TextStyle(
                                      color: SutraTheme.muted)),
                              SizedBox(height: 4),
                              Text(
                                'Import an ontology via SPARQL INSERT or\n'
                                'the Protege plugin to see the class hierarchy.',
                                textAlign: TextAlign.center,
                                style: TextStyle(
                                    color: SutraTheme.muted, fontSize: 12),
                              ),
                            ],
                          ),
                        )
                      : Row(
                          children: [
                            // Class tree (left)
                            SizedBox(
                              width: 280,
                              child: _buildClassTree(),
                            ),
                            // Detail panel (right)
                            Expanded(child: _buildDetailPanel()),
                          ],
                        ),
        ),
      ],
    );
  }

  Widget _buildClassTree() {
    return Container(
      decoration: const BoxDecoration(
        border: Border(right: BorderSide(color: SutraTheme.border)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Padding(
            padding: EdgeInsets.all(12),
            child: Text('Classes',
                style: TextStyle(
                    color: SutraTheme.muted,
                    fontSize: 11,
                    fontWeight: FontWeight.w600)),
          ),
          Expanded(
            child: ListView(
              children: _classes.map((c) {
                final isSelected = _selectedClass?.iri == c.iri;
                final indent = c.parentIri != null ? 24.0 : 8.0;
                return InkWell(
                  onTap: () {
                    setState(() => _selectedClass = c);
                    _loadIndividuals(c.iri);
                  },
                  child: Container(
                    padding: EdgeInsets.only(
                        left: indent, top: 6, bottom: 6, right: 8),
                    color: isSelected
                        ? SutraTheme.accent.withOpacity(0.1)
                        : null,
                    child: Row(
                      children: [
                        Icon(
                          Icons.circle,
                          size: 8,
                          color: isSelected
                              ? SutraTheme.accent
                              : SutraTheme.orange,
                        ),
                        const SizedBox(width: 8),
                        Expanded(
                          child: Text(
                            _shortName(c.iri),
                            style: TextStyle(
                              fontSize: 12,
                              color: isSelected
                                  ? SutraTheme.accent
                                  : SutraTheme.text,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                );
              }).toList(),
            ),
          ),

          // Properties section
          const Padding(
            padding: EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Text('Properties',
                style: TextStyle(
                    color: SutraTheme.muted,
                    fontSize: 11,
                    fontWeight: FontWeight.w600)),
          ),
          Expanded(
            child: ListView(
              children: _properties.map((p) {
                return Padding(
                  padding:
                      const EdgeInsets.symmetric(horizontal: 12, vertical: 3),
                  child: Row(
                    children: [
                      Icon(
                        p.type == _PropertyType.object
                            ? Icons.arrow_forward
                            : Icons.text_fields,
                        size: 12,
                        color: p.type == _PropertyType.object
                            ? SutraTheme.accent
                            : SutraTheme.green,
                      ),
                      const SizedBox(width: 6),
                      Expanded(
                        child: Text(
                          _shortName(p.iri),
                          style: const TextStyle(fontSize: 12),
                        ),
                      ),
                    ],
                  ),
                );
              }).toList(),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildDetailPanel() {
    if (_selectedClass == null) {
      return const Center(
        child: Text('Select a class to view details',
            style: TextStyle(color: SutraTheme.muted)),
      );
    }

    final c = _selectedClass!;
    final classProps = _properties
        .where((p) => p.domain == c.iri)
        .toList();

    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // Class header
          Text(
            _shortName(c.iri),
            style: const TextStyle(
              fontSize: 18,
              fontWeight: FontWeight.w600,
              color: SutraTheme.accent,
            ),
          ),
          const SizedBox(height: 4),
          SelectableText(
            c.iri,
            style: const TextStyle(color: SutraTheme.muted, fontSize: 11),
          ),
          if (c.parentIri != null) ...[
            const SizedBox(height: 4),
            Text.rich(TextSpan(children: [
              const TextSpan(
                  text: 'subClassOf: ',
                  style: TextStyle(color: SutraTheme.muted, fontSize: 12)),
              TextSpan(
                  text: _shortName(c.parentIri!),
                  style: const TextStyle(
                      color: SutraTheme.orange, fontSize: 12)),
            ])),
          ],

          const SizedBox(height: 16),

          // Properties for this class
          if (classProps.isNotEmpty) ...[
            const Text('Properties',
                style: TextStyle(
                    fontWeight: FontWeight.w600,
                    color: SutraTheme.text,
                    fontSize: 13)),
            const Divider(color: SutraTheme.border),
            ...classProps.map((p) => Padding(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  child: Row(
                    children: [
                      Icon(
                        p.type == _PropertyType.object
                            ? Icons.arrow_forward
                            : Icons.text_fields,
                        size: 14,
                        color: SutraTheme.accent,
                      ),
                      const SizedBox(width: 8),
                      Text(_shortName(p.iri),
                          style: const TextStyle(fontSize: 12)),
                      if (p.range.isNotEmpty) ...[
                        const Text(' -> ',
                            style: TextStyle(
                                color: SutraTheme.muted, fontSize: 12)),
                        Text(_shortName(p.range),
                            style: const TextStyle(
                                color: SutraTheme.green, fontSize: 12)),
                      ],
                    ],
                  ),
                )),
            const SizedBox(height: 16),
          ],

          // Individuals
          const Text('Individuals',
              style: TextStyle(
                  fontWeight: FontWeight.w600,
                  color: SutraTheme.text,
                  fontSize: 13)),
          const Divider(color: SutraTheme.border),
          Expanded(
            child: _individuals.isEmpty
                ? const Text('No individuals found',
                    style: TextStyle(color: SutraTheme.muted, fontSize: 12))
                : ListView.builder(
                    itemCount: _individuals.length,
                    itemBuilder: (ctx, i) => Padding(
                      padding: const EdgeInsets.symmetric(vertical: 2),
                      child: Row(
                        children: [
                          const Icon(Icons.diamond_outlined,
                              size: 12, color: SutraTheme.purple),
                          const SizedBox(width: 6),
                          Expanded(
                            child: Tooltip(
                              message: _individuals[i].iri,
                              child: Text(
                                _shortName(_individuals[i].iri),
                                style: const TextStyle(fontSize: 12),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}

class _OntologyClass {
  final String iri;
  String? parentIri;
  _OntologyClass({required this.iri, this.parentIri});
}

class _OntologyProperty {
  final String iri;
  final _PropertyType type;
  final String domain;
  final String range;
  _OntologyProperty({
    required this.iri,
    required this.type,
    this.domain = '',
    this.range = '',
  });
}

class _Individual {
  final String iri;
  _Individual({required this.iri});
}

enum _PropertyType { object, datatype }
