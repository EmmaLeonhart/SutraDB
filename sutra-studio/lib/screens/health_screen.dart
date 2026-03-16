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

      // Get real HNSW health data from /vectors/health endpoint
      final vectorHealth = <_VectorPredicateHealth>[];
      try {
        final healthData = await conn.client.vectorsHealth();
        final indexes = healthData['indexes'] as List<dynamic>? ?? [];
        for (final idx in indexes) {
          final m = idx as Map<String, dynamic>;
          vectorHealth.add(_VectorPredicateHealth(
            predicate: m['predicate']?.toString() ?? 'unknown',
            vectorCount: m['total_nodes'] as int? ?? 0,
            activeNodes: m['active_nodes'] as int? ?? 0,
            deletedRatio: (m['deleted_ratio'] as num?)?.toDouble() ?? 0.0,
            dimensions: m['dimensions'] as int? ?? 0,
            metric: m['metric']?.toString() ?? 'unknown',
            needsCompaction: m['needs_compaction'] as bool? ?? false,
          ));
        }
      } catch (_) {
        // /vectors/health may not be available
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

            // HNSW cluster heatmap (simplified — shows layer distribution)
            _buildHnswHeatmap(),
            const SizedBox(height: 16),

            // Backup management
            _buildBackupManagement(),
            const SizedBox(height: 16),

            // Future features
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

    // Determine health color based on deleted ratio
    Color healthColor;
    String healthLabel;
    if (v.needsCompaction) {
      healthColor = SutraTheme.red;
      healthLabel = 'NEEDS COMPACTION';
    } else if (v.deletedRatio > 0.15) {
      healthColor = SutraTheme.orange;
      healthLabel = 'Warning';
    } else {
      healthColor = SutraTheme.green;
      healthLabel = 'Healthy';
    }

    final tombstoneCount = v.vectorCount - v.activeNodes;
    final tombstonePercent = (v.deletedRatio * 100).toStringAsFixed(1);

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(Icons.circle, size: 10, color: healthColor),
              const SizedBox(width: 8),
              Text(shortPred,
                  style: const TextStyle(
                      fontWeight: FontWeight.w600, fontSize: 13)),
              const Spacer(),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                decoration: BoxDecoration(
                  color: healthColor.withValues(alpha: 0.15),
                  borderRadius: BorderRadius.circular(4),
                ),
                child: Text(healthLabel,
                    style: TextStyle(color: healthColor, fontSize: 10, fontWeight: FontWeight.w600)),
              ),
            ],
          ),
          const SizedBox(height: 4),
          Padding(
            padding: const EdgeInsets.only(left: 18),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '${v.activeNodes} active / ${v.vectorCount} total nodes  •  '
                  '${v.dimensions}d  •  ${v.metric}',
                  style: const TextStyle(color: SutraTheme.muted, fontSize: 11),
                ),
                if (tombstoneCount > 0)
                  Text(
                    '$tombstoneCount tombstoned ($tombstonePercent% deleted)',
                    style: TextStyle(color: healthColor, fontSize: 11),
                  ),
                // Tombstone ratio bar
                const SizedBox(height: 4),
                ClipRRect(
                  borderRadius: BorderRadius.circular(2),
                  child: LinearProgressIndicator(
                    value: v.deletedRatio.clamp(0, 1),
                    backgroundColor: SutraTheme.border,
                    valueColor: AlwaysStoppedAnimation(healthColor),
                    minHeight: 4,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildHnswHeatmap() {
    if (_vectorHealth.isEmpty) return const SizedBox.shrink();

    return _card(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Row(
            children: [
              Icon(Icons.grid_on, size: 18, color: SutraTheme.orange),
              SizedBox(width: 8),
              Text('HNSW Layer Distribution',
                  style: TextStyle(
                      fontWeight: FontWeight.w600, color: SutraTheme.text)),
            ],
          ),
          const SizedBox(height: 12),
          ..._vectorHealth.map((v) {
            final total = v.vectorCount;
            final active = v.activeNodes;
            final deleted = total - active;
            final shortPred = v.predicate.split('#').last.split('/').last;

            // Simple heatmap: show ratio bars for active vs deleted
            return Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(shortPred,
                      style: const TextStyle(fontSize: 12, fontWeight: FontWeight.w500)),
                  const SizedBox(height: 4),
                  Row(
                    children: [
                      // Active nodes bar (green)
                      Expanded(
                        flex: active.clamp(1, 1000),
                        child: Container(
                          height: 20,
                          decoration: BoxDecoration(
                            color: SutraTheme.green,
                            borderRadius: const BorderRadius.horizontal(left: Radius.circular(4)),
                          ),
                          alignment: Alignment.center,
                          child: Text('$active active',
                              style: const TextStyle(fontSize: 9, color: Colors.black)),
                        ),
                      ),
                      // Deleted nodes bar (red)
                      if (deleted > 0)
                        Expanded(
                          flex: deleted.clamp(1, 1000),
                          child: Container(
                            height: 20,
                            decoration: BoxDecoration(
                              color: SutraTheme.red,
                              borderRadius: const BorderRadius.horizontal(right: Radius.circular(4)),
                            ),
                            alignment: Alignment.center,
                            child: Text('$deleted deleted',
                                style: const TextStyle(fontSize: 9, color: Colors.white)),
                          ),
                        ),
                    ],
                  ),
                  const SizedBox(height: 2),
                  Text('${v.dimensions}d ${v.metric}',
                      style: const TextStyle(fontSize: 10, color: SutraTheme.muted)),
                ],
              ),
            );
          }),
        ],
      ),
    );
  }

  Widget _buildBackupManagement() {
    return _card(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Row(
            children: [
              Icon(Icons.backup, size: 18, color: SutraTheme.accent),
              SizedBox(width: 8),
              Text('Backup Management',
                  style: TextStyle(
                      fontWeight: FontWeight.w600, color: SutraTheme.text)),
            ],
          ),
          const SizedBox(height: 12),
          const Text(
            'Backups are managed via the CLI:\n\n'
            '  sutra serve --backup-interval 60\n\n'
            'Creates a copy of the database every N minutes\n'
            'in the data-dir/backups/ subdirectory.',
            style: TextStyle(
              color: SutraTheme.muted,
              fontSize: 12,
              fontFamily: 'monospace',
            ),
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              ElevatedButton.icon(
                onPressed: () async {
                  final conn = context.read<ConnectionProvider>();
                  if (!conn.connected) return;
                  // Trigger a manual backup by calling the health endpoint
                  // (in a real implementation, this would call a backup API)
                  ScaffoldMessenger.of(context).showSnackBar(
                    const SnackBar(
                      content: Text('Manual backup: use CLI "sutra serve --backup-interval 1" for one-time backup'),
                    ),
                  );
                },
                icon: const Icon(Icons.save, size: 16),
                label: const Text('Backup Now', style: TextStyle(fontSize: 12)),
                style: ElevatedButton.styleFrom(
                  backgroundColor: SutraTheme.accent,
                  foregroundColor: Colors.black,
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                ),
              ),
              const SizedBox(width: 8),
              ElevatedButton.icon(
                onPressed: () {
                  ScaffoldMessenger.of(context).showSnackBar(
                    const SnackBar(
                      content: Text('View backups in data-dir/backups/ directory'),
                    ),
                  );
                },
                icon: const Icon(Icons.folder_open, size: 16),
                label: const Text('View Backups', style: TextStyle(fontSize: 12)),
                style: ElevatedButton.styleFrom(
                  backgroundColor: SutraTheme.surface,
                  foregroundColor: SutraTheme.text,
                  padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                ),
              ),
            ],
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
  final int activeNodes;
  final double deletedRatio;
  final int dimensions;
  final String metric;
  final bool needsCompaction;
  List<int>? hnswDegrees;

  _VectorPredicateHealth({
    required this.predicate,
    required this.vectorCount,
    this.activeNodes = 0,
    this.deletedRatio = 0.0,
    this.dimensions = 0,
    this.metric = 'unknown',
    this.needsCompaction = false,
    this.hnswDegrees,
  });
}
