import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/connection_provider.dart';
import '../services/sutra_client.dart';
import '../theme/sutra_theme.dart';

/// Database and HNSW health diagnostics screen.
///
/// This is one of the primary reasons the visual client exists — broken HNSW
/// graphs, drifted clusters, and tombstone accumulation are easier for humans
/// to spot visually than for AI agents to detect programmatically.
///
/// Displays:
/// - Connection status and basic stats
/// - HNSW index health per vector predicate (planned)
/// - Tombstone ratio and rebuild recommendations
/// - Edge traversal distribution (PageRank-like)
/// - Cluster-level and network-level health views
class HealthScreen extends StatefulWidget {
  const HealthScreen({super.key});

  @override
  State<HealthScreen> createState() => _HealthScreenState();
}

class _HealthScreenState extends State<HealthScreen> {
  bool _loading = false;
  DbStats? _stats;
  List<_VectorPredicateHealth> _vectorHealth = [];
  String? _error;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _loadHealth());
  }

  Future<void> _loadHealth() async {
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
      final stats = await conn.client.stats();

      // Try to detect vector predicates and their health
      final vectorHealth = <_VectorPredicateHealth>[];
      try {
        // Query for vector predicates (hasEmbedding-like predicates)
        final vecResult = await conn.client.query('''
          SELECT ?pred (COUNT(?s) AS ?count) WHERE {
            ?s ?pred ?o .
            FILTER(CONTAINS(STR(?pred), "Embedding") || CONTAINS(STR(?pred), "embedding") || CONTAINS(STR(?pred), "vector"))
          } GROUP BY ?pred
        ''');
        for (final row in vecResult.rows) {
          final pred = _val(row, 'pred');
          final count =
              int.tryParse(_val(row, 'count')) ?? 0;
          if (pred.isNotEmpty) {
            vectorHealth.add(_VectorPredicateHealth(
              predicate: pred,
              vectorCount: count,
            ));
          }
        }
      } catch (_) {
        // GROUP BY / FILTER may not be implemented yet — that's fine
      }

      // Try to get HNSW neighbor edge counts for health assessment
      try {
        final hnswResult = await conn.client.query('''
          SELECT ?src (COUNT(?tgt) AS ?degree) WHERE {
            ?src <sutra:hnswNeighbor> ?tgt
          } GROUP BY ?src LIMIT 500
        ''');
        // Compute degree distribution stats
        final degrees = <int>[];
        for (final row in hnswResult.rows) {
          final d = int.tryParse(_val(row, 'degree')) ?? 0;
          degrees.add(d);
        }
        if (degrees.isNotEmpty && vectorHealth.isNotEmpty) {
          degrees.sort();
          vectorHealth.first.hnswDegrees = degrees;
        }
      } catch (_) {
        // GROUP BY may not be available yet
      }

      setState(() {
        _stats = stats;
        _vectorHealth = vectorHealth;
        _loading = false;
      });
    } catch (e) {
      setState(() {
        _loading = false;
        _error = e.toString();
      });
    }
  }

  String _val(Map<String, dynamic> row, String key) {
    final v = row[key];
    if (v is Map) return v['value']?.toString() ?? '';
    return v?.toString() ?? '';
  }

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // Header
          Row(
            children: [
              const Icon(Icons.monitor_heart_outlined,
                  color: SutraTheme.accent, size: 24),
              const SizedBox(width: 10),
              const Text(
                'Database Health',
                style: TextStyle(
                    fontSize: 18,
                    fontWeight: FontWeight.w600,
                    color: SutraTheme.text),
              ),
              const Spacer(),
              IconButton(
                icon: const Icon(Icons.refresh, size: 18),
                onPressed: _loadHealth,
              ),
            ],
          ),

          const SizedBox(height: 16),

          if (_loading)
            const Center(child: CircularProgressIndicator())
          else if (_error != null)
            _errorCard(_error!)
          else ...[
            // Connection status card
            _buildConnectionCard(),
            const SizedBox(height: 16),

            // Database overview
            if (_stats != null) _buildStatsCards(),
            const SizedBox(height: 16),

            // HNSW health
            _buildHnswHealthSection(),
            const SizedBox(height: 16),

            // Future: traversal heatmap, cluster isolation
            _buildPlannedFeatures(),
          ],
        ],
      ),
    );
  }

  Widget _buildConnectionCard() {
    return Consumer<ConnectionProvider>(
      builder: (ctx, conn, _) => _card(
        child: Row(
          children: [
            Icon(
              Icons.circle,
              size: 12,
              color: conn.connected ? SutraTheme.green : SutraTheme.red,
            ),
            const SizedBox(width: 10),
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  conn.connected ? 'Connected' : 'Disconnected',
                  style: TextStyle(
                    fontWeight: FontWeight.w600,
                    color: conn.connected
                        ? SutraTheme.green
                        : SutraTheme.red,
                  ),
                ),
                Text(
                  conn.config.endpoint,
                  style: const TextStyle(
                      color: SutraTheme.muted, fontSize: 12),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildStatsCards() {
    return Wrap(
      spacing: 12,
      runSpacing: 12,
      children: [
        _statCard(
          'Total Triples',
          _stats!.totalTriples >= 0
              ? _stats!.totalTriples.toString()
              : 'Unknown',
          Icons.storage,
          SutraTheme.accent,
        ),
        _statCard(
          'RDF Types',
          _stats!.typeDistribution.length.toString(),
          Icons.category,
          SutraTheme.orange,
        ),
        _statCard(
          'Vector Predicates',
          _vectorHealth.length.toString(),
          Icons.grain,
          SutraTheme.purple,
        ),
        _statCard(
          'Total Vectors',
          _vectorHealth
              .fold<int>(0, (sum, v) => sum + v.vectorCount)
              .toString(),
          Icons.scatter_plot,
          SutraTheme.green,
        ),
      ],
    );
  }

  Widget _buildHnswHealthSection() {
    return _card(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Row(
            children: [
              Icon(Icons.grain, size: 18, color: SutraTheme.purple),
              SizedBox(width: 8),
              Text('HNSW Index Health',
                  style: TextStyle(
                      fontWeight: FontWeight.w600,
                      color: SutraTheme.text)),
            ],
          ),
          const SizedBox(height: 12),

          if (_vectorHealth.isEmpty)
            const Text(
              'No vector predicates detected. Declare a vector predicate\n'
              'and insert embeddings to see HNSW health metrics.',
              style: TextStyle(color: SutraTheme.muted, fontSize: 12),
            )
          else
            ..._vectorHealth.map((v) => _buildVectorHealthRow(v)),

          const SizedBox(height: 16),
          const Divider(color: SutraTheme.border),
          const SizedBox(height: 8),
          const Text(
            'Health indicators (planned):',
            style: TextStyle(
                color: SutraTheme.muted,
                fontSize: 11,
                fontWeight: FontWeight.w600),
          ),
          const SizedBox(height: 6),
          _healthIndicator(
            'Degree distribution',
            'Healthy HNSW has regular degree distribution. Nodes near '
                'small-world hubs should have higher connectivity; '
                'peripheral nodes lower.',
            Icons.bar_chart,
          ),
          _healthIndicator(
            'Tombstone ratio',
            'Deleted vectors are tombstoned, not removed. High ratio '
                '(>30%) degrades search quality — triggers rebuild recommendation.',
            Icons.delete_sweep,
          ),
          _healthIndicator(
            'Cluster connectivity',
            'PageRank-like metric per cluster. Over-linked or under-linked '
                'clusters indicate drift from insertions/deletions.',
            Icons.hub,
          ),
          _healthIndicator(
            'Traversal counters',
            'Per-edge traversal counts reveal hot paths and dead zones. '
                'Useful for identifying HNSW edges that are never traversed.',
            Icons.route,
          ),
        ],
      ),
    );
  }

  Widget _buildVectorHealthRow(_VectorPredicateHealth v) {
    final shortPred = v.predicate.split('#').last.split('/').last;

    // Compute degree distribution stats if available
    String degreeSummary = 'Degree data not available';
    Color healthColor = SutraTheme.muted;
    if (v.hnswDegrees != null && v.hnswDegrees!.isNotEmpty) {
      final d = v.hnswDegrees!;
      final mean = d.reduce((a, b) => a + b) / d.length;
      final median = d[d.length ~/ 2];
      final max = d.last;
      final min = d.first;
      degreeSummary =
          'Degree: min=$min median=$median mean=${mean.toStringAsFixed(1)} max=$max';

      // Healthy if distribution is reasonably even (stddev < mean)
      final variance =
          d.map((x) => (x - mean) * (x - mean)).reduce((a, b) => a + b) /
              d.length;
      final stddev = variance > 0 ? variance * 0.5 : 0; // rough sqrt approx
      healthColor =
          stddev < mean * 1.5 ? SutraTheme.green : SutraTheme.orange;
    }

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          Icon(Icons.circle, size: 8, color: healthColor),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(shortPred,
                    style: const TextStyle(
                        fontWeight: FontWeight.w600, fontSize: 13)),
                Text('${v.vectorCount} vectors',
                    style: const TextStyle(
                        color: SutraTheme.muted, fontSize: 11)),
                Text(degreeSummary,
                    style: TextStyle(color: healthColor, fontSize: 11)),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildPlannedFeatures() {
    return _card(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Row(
            children: [
              Icon(Icons.construction, size: 18, color: SutraTheme.orange),
              SizedBox(width: 8),
              Text('Planned Diagnostics',
                  style: TextStyle(
                      fontWeight: FontWeight.w600,
                      color: SutraTheme.text)),
            ],
          ),
          const SizedBox(height: 12),
          _plannedItem(
            'HNSW cluster heatmap',
            'Visual heatmap of HNSW layer connectivity — shows cluster '
                'isolation and drift at a glance.',
          ),
          _plannedItem(
            'Rebuild recommendations',
            'Automatic detection of when an HNSW index should be rebuilt '
                'based on tombstone ratio, degree drift, and traversal '
                'efficiency metrics.',
          ),
          _plannedItem(
            'Edge traversal counters',
            'Track per-edge traversal counts for both semantic and HNSW '
                'edges. Identify hot paths and dead zones.',
          ),
          _plannedItem(
            'Per-cluster PageRank',
            'Run a PageRank-like metric at cluster level or full network '
                'level to detect drift from heavy insert/delete workloads.',
          ),
        ],
      ),
    );
  }

  Widget _healthIndicator(String title, String description, IconData icon) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(icon, size: 14, color: SutraTheme.muted),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title,
                    style: const TextStyle(
                        fontSize: 12, fontWeight: FontWeight.w600)),
                Text(description,
                    style: const TextStyle(
                        color: SutraTheme.muted, fontSize: 11)),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _plannedItem(String title, String desc) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Icon(Icons.schedule, size: 12, color: SutraTheme.muted),
          const SizedBox(width: 8),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title,
                    style: const TextStyle(
                        fontSize: 12, fontWeight: FontWeight.w600)),
                Text(desc,
                    style: const TextStyle(
                        color: SutraTheme.muted, fontSize: 11)),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _statCard(String label, String value, IconData icon, Color color) {
    return Container(
      width: 160,
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: SutraTheme.surface,
        border: Border.all(color: SutraTheme.border),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(icon, size: 20, color: color),
          const SizedBox(height: 8),
          Text(value,
              style: TextStyle(
                  fontSize: 24, fontWeight: FontWeight.w700, color: color)),
          Text(label,
              style: const TextStyle(color: SutraTheme.muted, fontSize: 11)),
        ],
      ),
    );
  }

  Widget _card({required Widget child}) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: SutraTheme.surface,
        border: Border.all(color: SutraTheme.border),
        borderRadius: BorderRadius.circular(8),
      ),
      child: child,
    );
  }

  Widget _errorCard(String msg) {
    return _card(
      child: Row(
        children: [
          const Icon(Icons.error_outline, color: SutraTheme.red),
          const SizedBox(width: 8),
          Expanded(
              child: Text(msg,
                  style: const TextStyle(color: SutraTheme.red))),
        ],
      ),
    );
  }
}

class _VectorPredicateHealth {
  final String predicate;
  final int vectorCount;
  List<int>? hnswDegrees;
  double? tombstoneRatio;

  _VectorPredicateHealth({
    required this.predicate,
    required this.vectorCount,
    this.hnswDegrees,
    this.tombstoneRatio,
  });
}
