//! Database health report: unified metrics for AI agents and humans.
//!
//! This module produces a comprehensive health report covering all subsystems:
//! HNSW vector indexes, pseudo-tables, and storage. The report is designed to
//! be consumed by both AI agents (structured text with context) and humans
//! (via Sutra Studio visualization).
//!
//! # Design philosophy
//!
//! Every metric includes three things:
//! 1. **The value** — what the metric currently is
//! 2. **What "healthy" looks like** — ideal range for this metric
//! 3. **What "unhealthy" looks like** — when to worry
//!
//! This context is critical for AI agents that need to assess database health
//! without prior knowledge of SutraDB internals. An agent should be able to
//! read the health report and immediately know which subsystems need attention.
//!
//! # Example output (CLI, `sutra health`)
//!
//! ```text
//! # SutraDB Health Report
//!
//! Overall status: [HEALTHY]
//!
//! ## Storage
//!   Triple count: 16,234
//!   Term dictionary: 4,891 terms
//!   Unique predicates: 23
//!   [HEALTHY] Storage is within normal parameters.
//!
//! ## HNSW Vector Indexes
//!   ### :hasEmbedding (1536 dimensions, Cosine)
//!     Nodes: 435 active / 435 total
//!     Tombstone ratio: 0.0% [HEALTHY: <10% is ideal, >30% triggers rebuild]
//!     Layers: max=3, distribution=[435, 30, 4, 1]
//!     Avg neighbors (layer 0): 12.3 (min=4, max=16) [HEALTHY: good connectivity]
//!     Entry points: 1 primary + 3 extra [HEALTHY: good cluster diversity]
//!     Edges: 5,220 horizontal + 35 vertical
//!     [HEALTHY] No action needed.
//!
//! ## Pseudo-Tables
//!   Discovered: 2 table(s) covering 420/4891 nodes (8.6%)
//!   ### pseudo_table_0 (420 rows, 7 columns)
//!     Cliff steepness: 14.2 [EXCELLENT: very sharp schema boundary]
//!     Segments: 1, avg tail properties per row: 0.3
//!   [HEALTHY] Good schema consistency across all pseudo-tables.
//!
//! ## Recommended Actions
//!   None. All subsystems are healthy.
//! ```

use std::fmt;

use sutra_core::pseudotable::PseudoTableRegistry;
use sutra_core::{TermDictionary, TripleStore};
use sutra_hnsw::VectorRegistry;

// ---------------------------------------------------------------------------
// Health status levels
// ---------------------------------------------------------------------------

/// Health status for a subsystem or metric.
///
/// These levels are designed to be unambiguous for AI agents:
/// - HEALTHY: no action needed
/// - WARNING: degraded but functional, schedule maintenance
/// - CRITICAL: performance significantly impacted, rebuild recommended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// All metrics within ideal ranges. No action needed.
    Healthy,
    /// Some metrics outside ideal range but not critical.
    /// Schedule maintenance during low-usage period.
    Warning,
    /// Significant performance degradation. Immediate action recommended.
    Critical,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "HEALTHY"),
            HealthStatus::Warning => write!(f, "WARNING"),
            HealthStatus::Critical => write!(f, "CRITICAL"),
        }
    }
}

// ---------------------------------------------------------------------------
// HNSW health metrics
// ---------------------------------------------------------------------------

/// Health metrics for a single HNSW vector index.
///
/// Each metric includes the value and contextual guidance for what
/// constitutes healthy vs unhealthy values.
#[derive(Debug, Clone)]
pub struct HnswHealthMetrics {
    /// Human-readable predicate name (e.g., ":hasEmbedding").
    pub predicate_name: String,
    /// Predicate TermId for programmatic access.
    pub predicate_id: u64,
    /// Vector dimensionality.
    pub dimensions: usize,
    /// Distance metric name.
    pub metric: String,
    /// The configured M parameter (max neighbors per layer).
    pub m_parameter: usize,

    // --- Node counts ---
    /// Total nodes including deleted (tombstoned) nodes.
    pub total_nodes: usize,
    /// Active (non-deleted) nodes.
    pub active_nodes: usize,
    /// Fraction of nodes that are tombstoned (0.0 to 1.0).
    /// Context: <10% is ideal, 10-30% is acceptable, >30% triggers rebuild.
    pub tombstone_ratio: f64,

    // --- Layer distribution ---
    /// Maximum layer in the graph.
    pub max_layer: u8,
    /// Number of active nodes per layer.
    /// Index 0 = layer 0 (all nodes), index 1 = layer 1, etc.
    /// Context: should follow geometric distribution. Layer 0 has all nodes,
    /// each subsequent layer has roughly 1/M of the previous.
    pub layer_distribution: Vec<usize>,

    // --- Connectivity ---
    /// Average number of neighbors per node at layer 0.
    /// Context: should be close to M (the HNSW M parameter).
    /// Significantly below M means poor connectivity (cold spots).
    /// At M = 16: ideal is 12-16, warning below 8.
    pub avg_neighbors_layer0: f64,
    /// Minimum neighbor count at layer 0 (excluding entry points).
    /// Context: should be > 0. Nodes with 0 neighbors are unreachable.
    pub min_neighbors_layer0: usize,
    /// Maximum neighbor count at layer 0.
    pub max_neighbors_layer0: usize,

    // --- Entry points ---
    /// Number of primary entry point(s). Should always be 1.
    pub primary_entry_points: usize,
    /// Number of extra entry points for cross-cluster diversity.
    /// Context: 2-8 is ideal for diverse cluster coverage.
    /// 0 means single-cluster search (may miss distant clusters).
    pub extra_entry_points: usize,

    // --- Edge counts ---
    /// Horizontal edges (same-layer neighbor connections).
    pub horizontal_edges: usize,
    /// Vertical descent edges (layer transitions).
    pub vertical_edges: usize,

    // --- Overall status ---
    /// Computed health status based on all metrics.
    pub status: HealthStatus,
    /// Human/AI-readable explanation of what needs attention.
    pub status_detail: String,
}

impl HnswHealthMetrics {
    /// Format as AI-readable structured text with context annotations.
    ///
    /// Each metric line includes [CONTEXT] showing ideal vs concerning ranges.
    /// AI agents can parse this to determine which indexes need maintenance.
    pub fn to_ai_text(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!(
            "  ### {} ({} dimensions, {}, M={})",
            self.predicate_name, self.dimensions, self.metric, self.m_parameter
        ));

        // Node counts and tombstone ratio
        lines.push(format!(
            "    Nodes: {} active / {} total",
            self.active_nodes, self.total_nodes
        ));

        let tombstone_status = if self.tombstone_ratio < 0.10 {
            "HEALTHY: <10% is ideal"
        } else if self.tombstone_ratio < 0.30 {
            "WARNING: 10-30%, schedule rebuild during low usage"
        } else {
            "CRITICAL: >30%, rebuild recommended now to restore search quality"
        };
        lines.push(format!(
            "    Tombstone ratio: {:.1}% [{}]",
            self.tombstone_ratio * 100.0,
            tombstone_status
        ));

        // Layer distribution
        let layer_str: Vec<String> = self
            .layer_distribution
            .iter()
            .map(|n| n.to_string())
            .collect();
        lines.push(format!(
            "    Layers: max={}, distribution=[{}]",
            self.max_layer,
            layer_str.join(", ")
        ));
        lines.push(
            "    [CONTEXT: Layer distribution should follow geometric decay. Layer 0 has all nodes, each higher layer ~1/M of previous.]"
                .to_string(),
        );

        // Connectivity
        let conn_status = if self.avg_neighbors_layer0 >= 8.0 {
            "HEALTHY: good connectivity"
        } else if self.avg_neighbors_layer0 >= 4.0 {
            "WARNING: below ideal, some cold spots possible"
        } else {
            "CRITICAL: very low connectivity, search quality degraded"
        };
        lines.push(format!(
            "    Avg neighbors (layer 0): {:.1} (min={}, max={}) [{}]",
            self.avg_neighbors_layer0,
            self.min_neighbors_layer0,
            self.max_neighbors_layer0,
            conn_status
        ));

        // Entry points
        let ep_status = if self.extra_entry_points >= 2 && self.extra_entry_points <= 8 {
            "HEALTHY: good cluster diversity"
        } else if self.extra_entry_points == 0 || self.extra_entry_points == 1 {
            "WARNING: limited diversity, may miss distant clusters"
        } else {
            "OK: many entry points"
        };
        lines.push(format!(
            "    Entry points: {} primary + {} extra [{}]",
            self.primary_entry_points, self.extra_entry_points, ep_status
        ));

        // Edge counts
        lines.push(format!(
            "    Edges: {} horizontal + {} vertical",
            self.horizontal_edges, self.vertical_edges
        ));

        // Overall status
        lines.push(format!("    [{}] {}", self.status, self.status_detail));

        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Pseudo-table health metrics
// ---------------------------------------------------------------------------

/// Health metrics for the pseudo-table subsystem.
#[derive(Debug, Clone)]
pub struct PseudoTableHealthMetrics {
    /// Number of discovered pseudo-tables.
    pub table_count: usize,
    /// Total nodes covered by pseudo-tables.
    pub total_coverage: usize,
    /// Total nodes in the store (for coverage ratio).
    pub total_nodes: usize,
    /// Coverage ratio (0.0 to 1.0).
    pub coverage_ratio: f64,
    /// Per-table metrics.
    pub tables: Vec<PseudoTableMetrics>,
    /// Overall status.
    pub status: HealthStatus,
    pub status_detail: String,
}

/// Metrics for a single pseudo-table.
#[derive(Debug, Clone)]
pub struct PseudoTableMetrics {
    pub label: String,
    pub row_count: usize,
    pub column_count: usize,
    pub cliff_steepness: f64,
    pub segment_count: usize,
    pub avg_tail_count: f64,
}

impl PseudoTableHealthMetrics {
    pub fn to_ai_text(&self) -> String {
        let mut lines = Vec::new();

        if self.table_count == 0 {
            lines.push("  No pseudo-tables discovered.".to_string());
            lines.push(
                "  [CONTEXT: Pseudo-tables are auto-discovered from shared predicate patterns."
                    .to_string(),
            );
            lines.push(
                "   A database with no pseudo-tables either has too few triples or no relational structure.]"
                    .to_string(),
            );
            return lines.join("\n");
        }

        lines.push(format!(
            "  Discovered: {} table(s) covering {}/{} nodes ({:.1}%)",
            self.table_count,
            self.total_coverage,
            self.total_nodes,
            self.coverage_ratio * 100.0
        ));

        let coverage_context = if self.coverage_ratio > 0.50 {
            "HEALTHY: >50% of nodes in pseudo-tables, good relational structure"
        } else if self.coverage_ratio > 0.10 {
            "OK: some relational structure detected"
        } else {
            "LOW: <10% coverage, data may be too graph-like for columnar acceleration"
        };
        lines.push(format!("  [{}]", coverage_context));

        for table in &self.tables {
            lines.push(format!(
                "  ### {} ({} rows, {} columns)",
                table.label, table.row_count, table.column_count
            ));

            let cliff_context = if table.cliff_steepness > 10.0 {
                "EXCELLENT: very sharp schema boundary"
            } else if table.cliff_steepness > 3.0 {
                "GOOD: clear schema with some optional properties"
            } else if table.cliff_steepness > 1.0 {
                "FAIR: gradual property distribution, messy schema"
            } else {
                "POOR: no clear schema boundary, pseudo-table may not be useful"
            };
            lines.push(format!(
                "    Cliff steepness: {:.1} [{}]",
                table.cliff_steepness, cliff_context
            ));
            lines.push(format!(
                "    Segments: {}, avg tail properties per row: {:.1}",
                table.segment_count, table.avg_tail_count
            ));
        }

        lines.push(format!("  [{}] {}", self.status, self.status_detail));
        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Storage health metrics
// ---------------------------------------------------------------------------

/// Storage-level health metrics.
#[derive(Debug, Clone)]
pub struct StorageHealthMetrics {
    /// Total number of triples in the store.
    pub triple_count: usize,
    /// Number of interned terms in the dictionary.
    pub term_count: usize,
    /// Number of unique predicates (approximation of schema width).
    pub predicate_count: usize,
    /// Overall status.
    pub status: HealthStatus,
    pub status_detail: String,
}

impl StorageHealthMetrics {
    pub fn to_ai_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("  Triple count: {}", self.triple_count));
        lines.push(format!("  Term dictionary: {} terms", self.term_count));
        lines.push(format!(
            "  Unique predicates: {} [CONTEXT: number of distinct predicate IRIs, indicates schema width]",
            self.predicate_count
        ));
        lines.push(format!("  [{}] {}", self.status, self.status_detail));
        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Full health report
// ---------------------------------------------------------------------------

/// Complete database health report covering all subsystems.
///
/// This is the top-level structure returned by `sutra health`. It aggregates
/// metrics from HNSW, pseudo-tables, and storage into a single report with
/// an overall health status.
#[derive(Debug, Clone)]
pub struct HealthReport {
    /// HNSW vector index health (one entry per vector predicate).
    pub hnsw: Vec<HnswHealthMetrics>,
    /// Pseudo-table subsystem health.
    pub pseudo_tables: PseudoTableHealthMetrics,
    /// Storage engine health.
    pub storage: StorageHealthMetrics,
    /// Overall database health (worst of all subsystems).
    pub overall_status: HealthStatus,
}

impl HealthReport {
    /// Format the full report as AI-readable structured text.
    ///
    /// This output is designed for AI agents that need to assess database
    /// health without GUI access. Every metric includes context annotations
    /// explaining what the ideal range is and what action to take if the
    /// metric is outside that range.
    ///
    /// The output uses Markdown headers for structure so agents can
    /// navigate by section.
    pub fn to_ai_text(&self) -> String {
        let mut sections = Vec::new();

        sections.push("# SutraDB Health Report".to_string());
        sections.push(String::new());
        sections.push(format!("Overall status: [{}]", self.overall_status));
        sections.push(String::new());

        // Storage section
        sections.push("## Storage".to_string());
        sections.push(self.storage.to_ai_text());
        sections.push(String::new());

        // HNSW section
        sections.push("## HNSW Vector Indexes".to_string());
        if self.hnsw.is_empty() {
            sections.push("  No vector indexes declared.".to_string());
            sections.push("  [CONTEXT: Vector indexes are created via sutra:declareVectorPredicate. No indexes = no vector search capability.]".to_string());
        } else {
            for index_health in &self.hnsw {
                sections.push(index_health.to_ai_text());
                sections.push(String::new());
            }
        }

        // Pseudo-tables section
        sections.push("## Pseudo-Tables".to_string());
        sections.push(self.pseudo_tables.to_ai_text());
        sections.push(String::new());

        // Action items — the most important part for AI agents.
        // If an agent reads nothing else, this section tells them what to do.
        let mut actions = Vec::new();
        for h in &self.hnsw {
            if h.status != HealthStatus::Healthy {
                actions.push(format!(
                    "- HNSW index '{}': {} (run `sutra health --rebuild-hnsw` to rebuild all, or target specific predicates)",
                    h.predicate_name, h.status_detail
                ));
            }
        }
        if self.pseudo_tables.status != HealthStatus::Healthy {
            actions.push(format!(
                "- Pseudo-tables: {} (run `sutra health --refresh` to rediscover)",
                self.pseudo_tables.status_detail
            ));
        }

        if !actions.is_empty() {
            sections.push("## Recommended Actions".to_string());
            for action in &actions {
                sections.push(action.clone());
            }
        } else {
            sections.push("## Recommended Actions".to_string());
            sections.push("  None. All subsystems are healthy.".to_string());
        }

        sections.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Report generation
// ---------------------------------------------------------------------------

/// Generate a complete health report from the current database state.
///
/// This is the main entry point for the health system. It collects metrics
/// from all subsystems and computes an overall health status.
///
/// ## Performance
///
/// This function performs read-only scans over the HNSW indexes and store.
/// It does NOT lock anything for writes. For large indexes, layer distribution
/// and connectivity metrics require O(N) scans over the node array.
pub fn generate_health_report(
    store: &TripleStore,
    dict: &TermDictionary,
    vectors: &VectorRegistry,
    pseudo_tables: Option<&PseudoTableRegistry>,
) -> HealthReport {
    // --- HNSW metrics ---
    let mut hnsw_metrics = Vec::new();
    for pred_id in vectors.predicates() {
        if let Some(index) = vectors.get(pred_id) {
            let pred_name = dict.resolve(pred_id).unwrap_or("unknown").to_string();

            let total_nodes = index.len();
            let active_nodes = index.active_count();
            let tombstone_ratio = index.deleted_ratio();
            let m_parameter = index.m_parameter();

            // Layer distribution: count active nodes at each layer.
            let max_layer = index.max_layer();
            let layer_distribution = compute_layer_distribution(index, max_layer);

            // Connectivity metrics at layer 0.
            let (avg_neighbors, min_neighbors, max_neighbors) = compute_connectivity_metrics(index);

            // Entry point counts.
            let primary_entry_points = if total_nodes > 0 { 1 } else { 0 };
            let extra_entry_points = index.extra_entry_point_count();

            // Edge counts.
            let horizontal_edges = index.horizontal_edge_count();
            let vertical_edges = index.vertical_edge_triples().len();

            // Compute overall status for this index.
            let (status, status_detail) = compute_hnsw_status(
                tombstone_ratio,
                avg_neighbors,
                extra_entry_points,
                active_nodes,
            );

            hnsw_metrics.push(HnswHealthMetrics {
                predicate_name: pred_name,
                predicate_id: pred_id,
                dimensions: index.dimensions(),
                metric: format!("{:?}", index.metric()),
                m_parameter,
                total_nodes,
                active_nodes,
                tombstone_ratio,
                max_layer,
                layer_distribution,
                avg_neighbors_layer0: avg_neighbors,
                min_neighbors_layer0: min_neighbors,
                max_neighbors_layer0: max_neighbors,
                primary_entry_points,
                extra_entry_points,
                horizontal_edges,
                vertical_edges,
                status,
                status_detail,
            });
        }
    }

    // --- Pseudo-table metrics ---
    let pseudo_table_metrics = match pseudo_tables {
        Some(registry) => {
            let total_nodes = count_unique_nodes(store);
            let coverage = registry.total_coverage();
            let coverage_ratio = registry.coverage_ratio(total_nodes);

            let tables: Vec<PseudoTableMetrics> = registry
                .tables
                .iter()
                .map(|t| {
                    let avg_tail = if t.total_rows > 0 {
                        t.segments
                            .iter()
                            .flat_map(|s| s.tail_counts.iter())
                            .sum::<usize>() as f64
                            / t.total_rows as f64
                    } else {
                        0.0
                    };
                    PseudoTableMetrics {
                        label: t.label.clone(),
                        row_count: t.total_rows,
                        column_count: t.columns.len(),
                        cliff_steepness: t.cliff_steepness,
                        segment_count: t.segments.len(),
                        avg_tail_count: avg_tail,
                    }
                })
                .collect();

            let (status, detail) = compute_pseudo_table_status(&tables, coverage_ratio);

            PseudoTableHealthMetrics {
                table_count: registry.len(),
                total_coverage: coverage,
                total_nodes,
                coverage_ratio,
                tables,
                status,
                status_detail: detail,
            }
        }
        None => PseudoTableHealthMetrics {
            table_count: 0,
            total_coverage: 0,
            total_nodes: 0,
            coverage_ratio: 0.0,
            tables: Vec::new(),
            status: HealthStatus::Healthy,
            status_detail:
                "Pseudo-tables not yet discovered. Run `sutra health --refresh` to populate."
                    .to_string(),
        },
    };

    // --- Storage metrics ---
    let triple_count = store.len();
    let term_count = dict.len();
    let predicate_count = count_unique_predicates(store);

    let storage_metrics = StorageHealthMetrics {
        triple_count,
        term_count,
        predicate_count,
        status: HealthStatus::Healthy,
        status_detail: "Storage is within normal parameters.".to_string(),
    };

    // --- Overall status ---
    let overall = worst_status(&[
        storage_metrics.status,
        pseudo_table_metrics.status,
        worst_status(&hnsw_metrics.iter().map(|h| h.status).collect::<Vec<_>>()),
    ]);

    HealthReport {
        hnsw: hnsw_metrics,
        pseudo_tables: pseudo_table_metrics,
        storage: storage_metrics,
        overall_status: overall,
    }
}

// ---------------------------------------------------------------------------
// Internal metric computation
// ---------------------------------------------------------------------------

/// Compute how many active nodes exist at each layer.
fn compute_layer_distribution(index: &sutra_hnsw::HnswIndex, max_layer: u8) -> Vec<usize> {
    let mut dist = vec![0usize; (max_layer as usize) + 1];
    for node in index.nodes() {
        if node.deleted {
            continue;
        }
        for layer in 0..=node.layer as usize {
            if layer < dist.len() {
                dist[layer] += 1;
            }
        }
    }
    dist
}

/// Compute average, min, max neighbor count at layer 0.
fn compute_connectivity_metrics(index: &sutra_hnsw::HnswIndex) -> (f64, usize, usize) {
    let mut total_neighbors = 0usize;
    let mut min_neighbors = usize::MAX;
    let mut max_neighbors = 0usize;
    let mut active_count = 0usize;

    for node in index.nodes() {
        if node.deleted {
            continue;
        }
        let n = if !node.neighbors.is_empty() {
            node.neighbors[0].len()
        } else {
            0
        };
        total_neighbors += n;
        min_neighbors = min_neighbors.min(n);
        max_neighbors = max_neighbors.max(n);
        active_count += 1;
    }

    if active_count == 0 {
        return (0.0, 0, 0);
    }

    let avg = total_neighbors as f64 / active_count as f64;
    (avg, min_neighbors, max_neighbors)
}

/// Compute HNSW health status from metrics.
fn compute_hnsw_status(
    tombstone_ratio: f64,
    avg_neighbors: f64,
    extra_entry_points: usize,
    active_nodes: usize,
) -> (HealthStatus, String) {
    if active_nodes == 0 {
        return (
            HealthStatus::Healthy,
            "Empty index — no data to assess.".to_string(),
        );
    }

    if tombstone_ratio > 0.50 {
        return (
            HealthStatus::Critical,
            format!(
                "Tombstone ratio {:.0}% is critically high. Rebuild immediately to restore search quality.",
                tombstone_ratio * 100.0
            ),
        );
    }

    if avg_neighbors < 2.0 && active_nodes > 5 {
        return (
            HealthStatus::Critical,
            format!(
                "Average connectivity {:.1} is critically low. Many nodes may be unreachable during search.",
                avg_neighbors
            ),
        );
    }

    if tombstone_ratio > 0.30 {
        return (
            HealthStatus::Warning,
            format!(
                "Tombstone ratio {:.0}% exceeds 30% threshold. Schedule HNSW rebuild during low usage.",
                tombstone_ratio * 100.0
            ),
        );
    }

    if avg_neighbors < 6.0 && active_nodes > 10 {
        return (
            HealthStatus::Warning,
            format!(
                "Average connectivity {:.1} is below ideal. Consider rebuilding for better search recall.",
                avg_neighbors
            ),
        );
    }

    if extra_entry_points == 0 && active_nodes > 50 {
        return (
            HealthStatus::Warning,
            "No extra entry points. Search may miss distant clusters. Rebuild to generate diverse entry points.".to_string(),
        );
    }

    (HealthStatus::Healthy, "No action needed.".to_string())
}

/// Compute pseudo-table subsystem status.
fn compute_pseudo_table_status(
    tables: &[PseudoTableMetrics],
    _coverage_ratio: f64,
) -> (HealthStatus, String) {
    if tables.is_empty() {
        return (
            HealthStatus::Healthy,
            "No pseudo-tables discovered. This is normal for small or highly graph-like datasets."
                .to_string(),
        );
    }

    let worst_cliff = tables
        .iter()
        .map(|t| t.cliff_steepness)
        .fold(f64::MAX, f64::min);

    if worst_cliff < 1.0 {
        return (
            HealthStatus::Warning,
            format!(
                "Pseudo-table with cliff steepness {:.1} detected. Schema may be too messy for effective columnar indexing.",
                worst_cliff
            ),
        );
    }

    (
        HealthStatus::Healthy,
        "Good schema consistency across all pseudo-tables.".to_string(),
    )
}

/// Count unique subject nodes in the store.
fn count_unique_nodes(store: &TripleStore) -> usize {
    let mut subjects = std::collections::HashSet::new();
    for triple in store.iter() {
        subjects.insert(triple.subject);
    }
    subjects.len()
}

/// Count unique predicates in the store.
fn count_unique_predicates(store: &TripleStore) -> usize {
    let mut predicates = std::collections::HashSet::new();
    for triple in store.iter() {
        predicates.insert(triple.predicate);
    }
    predicates.len()
}

/// Return the worst (most severe) health status from a list.
fn worst_status(statuses: &[HealthStatus]) -> HealthStatus {
    if statuses.iter().any(|s| *s == HealthStatus::Critical) {
        HealthStatus::Critical
    } else if statuses.iter().any(|s| *s == HealthStatus::Warning) {
        HealthStatus::Warning
    } else {
        HealthStatus::Healthy
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sutra_core::Triple;
    use sutra_hnsw::{DistanceMetric, VectorPredicateConfig};

    fn make_test_db() -> (TripleStore, TermDictionary, VectorRegistry) {
        let mut store = TripleStore::new();
        let mut dict = TermDictionary::new();

        let knows = dict.intern("http://example.org/knows");
        let name = dict.intern("http://example.org/name");

        for i in 0..20u64 {
            let s = dict.intern(&format!("http://example.org/person/{}", i));
            let name_val = dict.intern(&format!("\"Person {}\"", i));
            let target = dict.intern(&format!("http://example.org/person/{}", (i + 1) % 20));
            store.insert(Triple::new(s, knows, target)).unwrap();
            store.insert(Triple::new(s, name, name_val)).unwrap();
        }

        let mut vectors = VectorRegistry::new();
        let embedding_pred = dict.intern("http://example.org/hasEmbedding");
        vectors
            .declare(VectorPredicateConfig {
                predicate_id: embedding_pred,
                dimensions: 3,
                m: 4,
                ef_construction: 20,
                metric: DistanceMetric::Cosine,
            })
            .unwrap();

        for i in 0..10u64 {
            let v = vec![i as f32 * 0.1, (10 - i) as f32 * 0.1, 0.5];
            vectors.insert(embedding_pred, v, 1000 + i).unwrap();
        }

        (store, dict, vectors)
    }

    #[test]
    fn generates_health_report() {
        let (store, dict, vectors) = make_test_db();
        let report = generate_health_report(&store, &dict, &vectors, None);

        assert_eq!(report.overall_status, HealthStatus::Healthy);
        assert_eq!(report.hnsw.len(), 1);
        assert!(report.storage.triple_count > 0);
    }

    #[test]
    fn report_shows_hnsw_metrics() {
        let (store, dict, vectors) = make_test_db();
        let report = generate_health_report(&store, &dict, &vectors, None);

        let hnsw = &report.hnsw[0];
        assert_eq!(hnsw.active_nodes, 10);
        assert_eq!(hnsw.total_nodes, 10);
        assert!((hnsw.tombstone_ratio - 0.0).abs() < f64::EPSILON);
        assert!(hnsw.avg_neighbors_layer0 > 0.0);
        assert_eq!(hnsw.dimensions, 3);
        assert_eq!(hnsw.m_parameter, 4);
    }

    #[test]
    fn report_to_ai_text_is_readable() {
        let (store, dict, vectors) = make_test_db();
        let report = generate_health_report(&store, &dict, &vectors, None);
        let text = report.to_ai_text();

        assert!(text.contains("# SutraDB Health Report"));
        assert!(text.contains("## Storage"));
        assert!(text.contains("## HNSW Vector Indexes"));
        assert!(text.contains("## Pseudo-Tables"));
        assert!(text.contains("## Recommended Actions"));
        assert!(text.contains("HEALTHY"));
    }

    #[test]
    fn tombstone_triggers_warning() {
        let (status, _) = compute_hnsw_status(0.35, 12.0, 3, 100);
        assert_eq!(status, HealthStatus::Warning);
    }

    #[test]
    fn high_tombstone_triggers_critical() {
        let (status, _) = compute_hnsw_status(0.55, 12.0, 3, 100);
        assert_eq!(status, HealthStatus::Critical);
    }

    #[test]
    fn healthy_hnsw() {
        let (status, _) = compute_hnsw_status(0.05, 14.0, 4, 100);
        assert_eq!(status, HealthStatus::Healthy);
    }

    #[test]
    fn worst_status_picks_critical() {
        assert_eq!(
            worst_status(&[
                HealthStatus::Healthy,
                HealthStatus::Warning,
                HealthStatus::Critical
            ]),
            HealthStatus::Critical
        );
    }
}
