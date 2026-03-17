//! Pseudo-tables: auto-discovered columnar indexes over RDF triple patterns.
//!
//! # What are pseudo-tables?
//!
//! RDF has no tables, but relational structure exists implicitly in the graph.
//! When many nodes share the same set of predicates (like all `Person` nodes
//! having `name`, `age`, `email`), they form a "characteristic set" — a group
//! that behaves like rows in a relational table.
//!
//! Pseudo-tables auto-discover these groups and materialize columnar indexes
//! over them, enabling SQL-like query acceleration for the relational portions
//! of SPARQL queries.
//!
//! # Design
//!
//! ## Property model
//!
//! A "property" is defined by a predicate + position pair:
//! - `SUB→:eats` means the node appears as **subject** of `:eats`
//! - `OBJ→:eats` means the node appears as **object** of `:eats`
//!
//! This distinction is critical: a cat that eats mice has `SUB→:eats`,
//! while the mouse has `OBJ→:eats`. Being on different ends of the same
//! predicate is a fundamentally different property.
//!
//! ## Discovery criteria
//!
//! A group qualifies for a pseudo-table when:
//! 1. A statistically significant cluster of nodes shares 5+ properties
//! 2. Each of those 5+ properties is held by ≥50% of the group
//! 3. The group has enough members to justify the columnar index overhead
//!
//! ## Table structure
//!
//! Each property held by ≥33% of the group becomes a column. If a node
//! doesn't have a property that is a column, the value is null (None).
//! An additional column tracks the count of "tail properties" — properties
//! not included as columns — per node.
//!
//! ## Data health metric
//!
//! The "cliff" between core properties (high coverage) and tail properties
//! (low coverage) indicates schema consistency:
//! - **Sharp cliff**: 10 properties at 100%, everything else at <10% → healthy
//! - **Gradual slope**: properties spread across 20%-80% → messy schema
//!
//! ## Segment-level storage (DuckDB pattern)
//!
//! Rows are stored in segments of ~2048 rows. Each segment maintains
//! per-column zonemaps (min/max) for skip-scan pruning: if a query asks
//! for `?age > 50` and a segment's max age is 30, the entire segment
//! is skipped without examining individual rows.
//!
//! ## Reference architectures
//!
//! - **DataFusion**: `Precision<T>` pattern for column statistics (min/max/null_count/distinct)
//! - **DuckDB**: Segment-level zonemaps for skip-scan pruning, sorted by most selective column

use std::collections::HashMap;

use crate::id::TermId;
use crate::store::TripleStore;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum number of shared properties for a group to qualify as a pseudo-table.
/// Groups with fewer shared properties don't have enough relational structure
/// to benefit from columnar indexing.
const MIN_SHARED_PROPERTIES: usize = 5;

/// Minimum coverage ratio for a property to be considered "core" (part of the
/// characteristic set). A property held by 50% of the group is common enough
/// to define the group's identity.
const CORE_PROPERTY_THRESHOLD: f64 = 0.50;

/// Minimum coverage ratio for a property to become a column in the pseudo-table.
/// Lower than CORE_PROPERTY_THRESHOLD because we want columns for "optional"
/// properties that are common but not universal (like an optional email field).
const COLUMN_INCLUSION_THRESHOLD: f64 = 0.33;

/// Minimum number of nodes in a group for it to justify pseudo-table overhead.
/// A pseudo-table with 3 rows is worse than just scanning triples.
const MIN_GROUP_SIZE: usize = 10;

/// Number of rows per segment. Chosen to balance zonemap granularity against
/// overhead. Too small = too many segments = overhead. Too large = zonemaps
/// too coarse = no pruning benefit.
///
/// 2048 is the DuckDB default and works well for analytical workloads.
const SEGMENT_SIZE: usize = 2048;

// ---------------------------------------------------------------------------
// Property model
// ---------------------------------------------------------------------------

/// A property is a (predicate, position) pair that describes how a node
/// participates in a triple pattern.
///
/// Two nodes with the same predicate but different positions have different
/// properties. For example:
/// - `:Alice :knows :Bob` → Alice has `Property(knows, Subject)`, Bob has `Property(knows, Object)`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Property {
    /// The predicate IRI (interned as TermId).
    pub predicate: TermId,
    /// Which position the node occupies in the triple.
    pub position: PropertyPosition,
}

/// Which position a node occupies in a triple with a given predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyPosition {
    /// The node is the subject of the triple.
    Subject,
    /// The node is the object of the triple.
    Object,
}

/// The set of all properties for a single node — its "property signature."
///
/// Two nodes with the same property set are candidates for the same pseudo-table.
/// The property set is the RDF equivalent of a relational schema: it describes
/// what "columns" a node would have if it were a row in a table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropertySet {
    /// Sorted, deduplicated list of properties for this node.
    /// Sorting enables fast comparison and hashing.
    pub properties: Vec<Property>,
}

impl PropertySet {
    /// Create a new property set from a list of properties.
    /// Automatically sorts and deduplicates.
    pub fn new(mut properties: Vec<Property>) -> Self {
        properties.sort_by_key(|p| (p.predicate, p.position as u8));
        properties.dedup();
        Self { properties }
    }

    /// Check if this property set contains a specific property.
    pub fn contains(&self, property: &Property) -> bool {
        self.properties
            .binary_search_by_key(&(property.predicate, property.position as u8), |p| {
                (p.predicate, p.position as u8)
            })
            .is_ok()
    }

    /// Number of properties in this set.
    pub fn len(&self) -> usize {
        self.properties.len()
    }

    /// Whether this property set is empty.
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Column statistics (DataFusion Precision<T> pattern)
// ---------------------------------------------------------------------------

/// Statistics for a single column in a pseudo-table segment.
///
/// Follows DataFusion's `Precision<T>` pattern: each statistic is either
/// Exact (computed from all values), Approximate (estimated), or Unknown.
///
/// These statistics enable the query planner to estimate selectivity and
/// the executor to skip segments via zonemap pruning.
#[derive(Debug, Clone)]
pub struct ColumnStats {
    /// Minimum value in this column (within a segment or the whole table).
    /// None if the column has no non-null values.
    pub min_value: Option<TermId>,
    /// Maximum value in this column.
    pub max_value: Option<TermId>,
    /// Number of null (absent) values.
    pub null_count: usize,
    /// Number of distinct non-null values.
    /// Exact after full scan, approximate after sampling.
    pub distinct_count: usize,
    /// Total number of rows (null + non-null).
    pub row_count: usize,
}

impl ColumnStats {
    /// Create empty statistics (no data yet).
    fn empty() -> Self {
        Self {
            min_value: None,
            max_value: None,
            null_count: 0,
            distinct_count: 0,
            row_count: 0,
        }
    }

    /// Selectivity estimate for an equality predicate.
    ///
    /// Returns the estimated fraction of rows that match `value = X`.
    /// Uses distinct count for cardinality estimation (uniform distribution assumption).
    pub fn equality_selectivity(&self) -> f64 {
        if self.distinct_count == 0 {
            0.0
        } else {
            1.0 / self.distinct_count as f64
        }
    }

    /// Whether a range query [lo, hi] could match any values in this column.
    ///
    /// This is the zonemap pruning check: if the column's max < lo or min > hi,
    /// no rows in this segment can match and it can be skipped entirely.
    pub fn range_could_match(&self, lo: Option<TermId>, hi: Option<TermId>) -> bool {
        // If column has no values, it can't match anything.
        if self.min_value.is_none() || self.max_value.is_none() {
            return false;
        }
        let col_min = self.min_value.unwrap();
        let col_max = self.max_value.unwrap();

        // Check if the query range overlaps the column's value range.
        // If query's low bound exceeds column's max, no match possible.
        if let Some(lo) = lo {
            if lo > col_max {
                return false;
            }
        }
        // If query's high bound is below column's min, no match possible.
        if let Some(hi) = hi {
            if hi < col_min {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Pseudo-table segment (DuckDB pattern)
// ---------------------------------------------------------------------------

/// A segment of rows in a pseudo-table, with per-column zonemaps.
///
/// Segments are the unit of skip-scan pruning: when a query filter doesn't
/// overlap a segment's zonemap, the entire segment is skipped. This is the
/// same pattern DuckDB uses for analytical queries.
///
/// Each segment holds up to `SEGMENT_SIZE` rows (default 2048).
#[derive(Debug, Clone)]
pub struct Segment {
    /// The node TermIds (row identifiers) in this segment.
    /// Each entry is a node that belongs to this pseudo-table.
    pub nodes: Vec<TermId>,

    /// Column values: columns[col_idx][row_idx] = Some(value) or None.
    /// Outer vec is indexed by column position in PseudoTable::columns.
    /// Inner vec is parallel to `nodes`.
    pub columns: Vec<Vec<Option<TermId>>>,

    /// Tail property count per row: how many properties this node has
    /// that aren't included as columns. High tail counts indicate
    /// the node doesn't fit the pseudo-table schema well.
    pub tail_counts: Vec<usize>,

    /// Per-column statistics for zonemap pruning.
    /// Indexed by column position, parallel to `columns`.
    pub column_stats: Vec<ColumnStats>,
}

impl Segment {
    /// Create a new empty segment with the given number of columns.
    fn new(num_columns: usize) -> Self {
        Self {
            nodes: Vec::new(),
            columns: (0..num_columns).map(|_| Vec::new()).collect(),
            tail_counts: Vec::new(),
            column_stats: (0..num_columns).map(|_| ColumnStats::empty()).collect(),
        }
    }

    /// Number of rows in this segment.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether this segment is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Whether this segment is full (at capacity).
    pub fn is_full(&self) -> bool {
        self.nodes.len() >= SEGMENT_SIZE
    }

    /// Recompute column statistics from the actual data.
    ///
    /// Called after the segment is fully populated. Computes min/max/null_count
    /// for each column, which enables zonemap-based skip-scan pruning.
    pub fn compute_stats(&mut self) {
        for (col_idx, col_data) in self.columns.iter().enumerate() {
            let mut min_val: Option<TermId> = None;
            let mut max_val: Option<TermId> = None;
            let mut null_count = 0usize;
            let mut distinct = std::collections::HashSet::new();

            for value in col_data {
                match value {
                    Some(v) => {
                        distinct.insert(*v);
                        min_val = Some(min_val.map_or(*v, |m: TermId| m.min(*v)));
                        max_val = Some(max_val.map_or(*v, |m: TermId| m.max(*v)));
                    }
                    None => null_count += 1,
                }
            }

            self.column_stats[col_idx] = ColumnStats {
                min_value: min_val,
                max_value: max_val,
                null_count,
                distinct_count: distinct.len(),
                row_count: col_data.len(),
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Pseudo-table
// ---------------------------------------------------------------------------

/// A pseudo-table: a columnar index over a group of RDF nodes that share
/// enough predicate structure to benefit from relational-style query execution.
///
/// This is the core data structure that bridges RDF's flexible graph model
/// with SQL-like columnar execution. Each pseudo-table represents a
/// "characteristic set" — a group of nodes with similar property signatures.
#[derive(Debug, Clone)]
pub struct PseudoTable {
    /// Human-readable label for this pseudo-table (derived from the most
    /// common rdf:type or the dominant predicate pattern).
    pub label: String,

    /// The properties that define this pseudo-table's columns.
    /// Each column corresponds to a Property (predicate + position).
    /// Properties are ordered by coverage (highest first) for tighter
    /// zonemaps when rows are sorted by the most selective column.
    pub columns: Vec<Property>,

    /// Coverage ratio for each column: what fraction of nodes in this group
    /// have this property. Columns are sorted by coverage descending.
    pub column_coverage: Vec<f64>,

    /// Segmented row storage. Each segment holds up to SEGMENT_SIZE rows
    /// with per-column zonemaps for skip-scan pruning.
    pub segments: Vec<Segment>,

    /// Total number of nodes in this pseudo-table (across all segments).
    pub total_rows: usize,

    /// The core property set: properties held by ≥50% of the group.
    /// This defines the group's identity — the "characteristic set."
    pub core_properties: Vec<Property>,

    /// Data health metric: cliff steepness between core and tail properties.
    ///
    /// Computed as the ratio of average core property coverage to average
    /// tail property coverage. Higher = sharper cliff = healthier schema.
    ///
    /// - `cliff_steepness > 10.0`: Excellent schema consistency
    /// - `cliff_steepness 3.0-10.0`: Good, some optional properties
    /// - `cliff_steepness 1.0-3.0`: Messy schema, many optional fields
    /// - `cliff_steepness < 1.0`: No clear schema — pseudo-table may not be useful
    pub cliff_steepness: f64,
}

impl PseudoTable {
    /// Get aggregate statistics for a column across all segments.
    ///
    /// Merges per-segment zonemaps into a single ColumnStats covering
    /// the entire pseudo-table. Used by the query planner for cardinality
    /// estimation.
    pub fn column_stats(&self, col_idx: usize) -> ColumnStats {
        let mut merged = ColumnStats::empty();
        for segment in &self.segments {
            let seg_stats = &segment.column_stats[col_idx];
            merged.row_count += seg_stats.row_count;
            merged.null_count += seg_stats.null_count;
            merged.distinct_count = merged.distinct_count.max(seg_stats.distinct_count);
            if let Some(seg_min) = seg_stats.min_value {
                merged.min_value =
                    Some(merged.min_value.map_or(seg_min, |m: TermId| m.min(seg_min)));
            }
            if let Some(seg_max) = seg_stats.max_value {
                merged.max_value =
                    Some(merged.max_value.map_or(seg_max, |m: TermId| m.max(seg_max)));
            }
        }
        merged
    }

    /// Find the column index for a given property, if it exists.
    pub fn column_index(&self, property: &Property) -> Option<usize> {
        self.columns.iter().position(|p| p == property)
    }

    /// Check if a node (by TermId) is in this pseudo-table.
    pub fn contains_node(&self, node_id: TermId) -> bool {
        self.segments.iter().any(|seg| seg.nodes.contains(&node_id))
    }
}

// ---------------------------------------------------------------------------
// Pseudo-table registry
// ---------------------------------------------------------------------------

/// Registry of all discovered pseudo-tables.
///
/// The registry is the top-level entry point for pseudo-table operations.
/// It holds all discovered tables and provides lookup methods for the
/// query planner to find matching pseudo-tables for SPARQL patterns.
#[derive(Debug, Clone)]
pub struct PseudoTableRegistry {
    /// All discovered pseudo-tables, in discovery order.
    pub tables: Vec<PseudoTable>,
}

impl PseudoTableRegistry {
    /// Create an empty registry with no discovered tables.
    pub fn new() -> Self {
        Self { tables: Vec::new() }
    }

    /// Number of discovered pseudo-tables.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Whether any pseudo-tables have been discovered.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Find pseudo-tables that contain a column matching the given property.
    ///
    /// Used by the query planner to determine if a triple pattern can be
    /// routed through a pseudo-table's columnar index instead of the
    /// general-purpose SPO/POS/OSP indexes.
    pub fn find_tables_for_property(&self, property: &Property) -> Vec<(usize, usize)> {
        let mut matches = Vec::new();
        for (table_idx, table) in self.tables.iter().enumerate() {
            if let Some(col_idx) = table.column_index(property) {
                matches.push((table_idx, col_idx));
            }
        }
        matches
    }

    /// Total number of nodes across all pseudo-tables.
    pub fn total_coverage(&self) -> usize {
        self.tables.iter().map(|t| t.total_rows).sum()
    }

    /// Coverage ratio: what fraction of all nodes in the store are covered
    /// by at least one pseudo-table.
    pub fn coverage_ratio(&self, total_nodes: usize) -> f64 {
        if total_nodes == 0 {
            return 0.0;
        }
        self.total_coverage() as f64 / total_nodes as f64
    }
}

impl Default for PseudoTableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Discovery algorithm
// ---------------------------------------------------------------------------

/// Extract the property set for every node in the triple store.
///
/// Scans all triples to determine which predicates each node participates in
/// and in which position (subject or object). Returns a map from node TermId
/// to its PropertySet.
///
/// This is the first step of pseudo-table discovery: understanding what
/// "schema" each node implicitly follows.
pub fn extract_node_properties(store: &TripleStore) -> HashMap<TermId, PropertySet> {
    let mut node_props: HashMap<TermId, Vec<Property>> = HashMap::new();

    // Scan all triples to collect properties for each node.
    // Each triple contributes two properties:
    // - Subject gets Property(predicate, Subject)
    // - Object gets Property(predicate, Object)
    for triple in store.iter() {
        node_props
            .entry(triple.subject)
            .or_default()
            .push(Property {
                predicate: triple.predicate,
                position: PropertyPosition::Subject,
            });
        node_props.entry(triple.object).or_default().push(Property {
            predicate: triple.predicate,
            position: PropertyPosition::Object,
        });
    }

    // Convert to PropertySets (sorted + deduplicated).
    node_props
        .into_iter()
        .map(|(node, props)| (node, PropertySet::new(props)))
        .collect()
}

/// Discover pseudo-table groups from node property sets.
///
/// Groups nodes by their property signatures, then identifies groups that
/// are large enough and have enough shared properties to form pseudo-tables.
///
/// ## Algorithm
///
/// 1. **Exact grouping**: Group nodes by identical property sets. This finds
///    the tightest characteristic sets — nodes with exactly the same schema.
///
/// 2. **Merge similar groups**: Groups that share ≥80% of properties are
///    merged. This handles optional properties: a Person with email and
///    a Person without email should be in the same pseudo-table.
///
/// 3. **Filter by criteria**: Only keep groups with ≥5 shared properties
///    at ≥50% coverage and ≥10 members.
///
/// 4. **Compute coverage**: For each surviving group, compute per-property
///    coverage ratios and determine which properties become columns (≥33%).
pub fn discover_pseudo_tables(
    node_properties: &HashMap<TermId, PropertySet>,
    store: &TripleStore,
) -> PseudoTableRegistry {
    // Step 1: Group nodes by exact property set.
    // Nodes with identical property signatures form the initial clusters.
    let mut exact_groups: HashMap<Vec<(TermId, u8)>, Vec<TermId>> = HashMap::new();
    for (node_id, prop_set) in node_properties {
        // Create a hashable key from the property set.
        let key: Vec<(TermId, u8)> = prop_set
            .properties
            .iter()
            .map(|p| (p.predicate, p.position as u8))
            .collect();
        exact_groups.entry(key).or_default().push(*node_id);
    }

    // Step 2: Merge similar groups.
    // Groups sharing ≥80% of properties are combined into a single group.
    // This handles the "optional field" pattern where some nodes have extra properties.
    let mut merged_groups: Vec<(Vec<Property>, Vec<TermId>)> = Vec::new();

    let mut exact_vec: Vec<(Vec<Property>, Vec<TermId>)> = exact_groups
        .into_iter()
        .map(|(key, nodes)| {
            let props: Vec<Property> = key
                .into_iter()
                .map(|(pred, pos)| Property {
                    predicate: pred,
                    position: if pos == 0 {
                        PropertyPosition::Subject
                    } else {
                        PropertyPosition::Object
                    },
                })
                .collect();
            (props, nodes)
        })
        .collect();

    // Sort by group size descending so large groups absorb smaller ones.
    exact_vec.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut absorbed = vec![false; exact_vec.len()];

    for i in 0..exact_vec.len() {
        if absorbed[i] {
            continue;
        }

        let mut merged_props = exact_vec[i].0.clone();
        let mut merged_nodes = exact_vec[i].1.clone();

        for j in (i + 1)..exact_vec.len() {
            if absorbed[j] {
                continue;
            }

            // Compute Jaccard similarity between property sets.
            let props_i: std::collections::HashSet<_> = merged_props.iter().cloned().collect();
            let props_j: std::collections::HashSet<_> = exact_vec[j].0.iter().cloned().collect();
            let intersection = props_i.intersection(&props_j).count();
            let union = props_i.union(&props_j).count();

            if union > 0 && (intersection as f64 / union as f64) >= 0.80 {
                // Merge: take the union of properties and all nodes.
                merged_props = props_i.union(&props_j).cloned().collect();
                merged_props.sort_by_key(|p| (p.predicate, p.position as u8));
                merged_nodes.extend(exact_vec[j].1.iter());
                absorbed[j] = true;
            }
        }

        merged_groups.push((merged_props, merged_nodes));
    }

    // Step 3: Filter and build pseudo-tables.
    let mut tables = Vec::new();

    for (all_properties, nodes) in &merged_groups {
        if nodes.len() < MIN_GROUP_SIZE {
            continue;
        }

        // Compute per-property coverage: what fraction of nodes have each property.
        let mut property_coverage: Vec<(Property, f64)> = Vec::new();
        for prop in all_properties {
            let count = nodes
                .iter()
                .filter(|&&node_id| {
                    node_properties
                        .get(&node_id)
                        .is_some_and(|ps| ps.contains(prop))
                })
                .count();
            let coverage = count as f64 / nodes.len() as f64;
            property_coverage.push((*prop, coverage));
        }

        // Sort by coverage descending for column ordering.
        property_coverage.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        // Core properties: ≥50% coverage. These define the group's identity.
        let core: Vec<Property> = property_coverage
            .iter()
            .filter(|(_, cov)| *cov >= CORE_PROPERTY_THRESHOLD)
            .map(|(p, _)| *p)
            .collect();

        // Must have at least MIN_SHARED_PROPERTIES core properties.
        if core.len() < MIN_SHARED_PROPERTIES {
            continue;
        }

        // Columns: properties with ≥33% coverage become columns.
        let column_props: Vec<(Property, f64)> = property_coverage
            .iter()
            .filter(|(_, cov)| *cov >= COLUMN_INCLUSION_THRESHOLD)
            .cloned()
            .collect();

        // Tail properties: everything not included as a column.
        let tail_properties: Vec<(Property, f64)> = property_coverage
            .iter()
            .filter(|(_, cov)| *cov < COLUMN_INCLUSION_THRESHOLD)
            .cloned()
            .collect();

        // Compute cliff steepness: ratio of average core coverage to average tail coverage.
        let avg_core_coverage = if core.is_empty() {
            0.0
        } else {
            property_coverage
                .iter()
                .filter(|(p, _)| core.contains(p))
                .map(|(_, c)| c)
                .sum::<f64>()
                / core.len() as f64
        };
        let avg_tail_coverage = if tail_properties.is_empty() {
            // No tail properties = infinitely sharp cliff = perfect schema.
            0.01 // avoid division by zero
        } else {
            tail_properties.iter().map(|(_, c)| c).sum::<f64>() / tail_properties.len() as f64
        };
        let cliff_steepness = avg_core_coverage / avg_tail_coverage.max(0.01);

        // Build segmented storage.
        let columns: Vec<Property> = column_props.iter().map(|(p, _)| *p).collect();
        let coverage: Vec<f64> = column_props.iter().map(|(_, c)| *c).collect();
        let num_columns = columns.len();

        // Sort nodes by the value of the most selective column for tighter zonemaps.
        // The "most selective" column is the one with the highest distinct_count relative
        // to row count, which gives the tightest min/max ranges per segment.
        let mut sorted_nodes = nodes.clone();
        if let Some(first_col) = columns.first() {
            // Sort by the first column's value (highest coverage = most common = best sort key).
            sorted_nodes
                .sort_by_key(|&node_id| get_property_value(node_id, first_col, store).unwrap_or(0));
        }

        let mut segments = Vec::new();
        let mut current_segment = Segment::new(num_columns);

        for &node_id in &sorted_nodes {
            let node_propset = node_properties.get(&node_id);

            // Fill column values for this row.
            for (col_idx, col_prop) in columns.iter().enumerate() {
                let value = get_property_value(node_id, col_prop, store);
                current_segment.columns[col_idx].push(value);
            }

            // Count tail properties for this node.
            let tail_count = node_propset.map_or(0, |ps| {
                ps.properties
                    .iter()
                    .filter(|p| !columns.contains(p))
                    .count()
            });

            current_segment.nodes.push(node_id);
            current_segment.tail_counts.push(tail_count);

            // Segment full — finalize and start a new one.
            if current_segment.is_full() {
                current_segment.compute_stats();
                segments.push(current_segment);
                current_segment = Segment::new(num_columns);
            }
        }

        // Finalize the last (possibly partial) segment.
        if !current_segment.is_empty() {
            current_segment.compute_stats();
            segments.push(current_segment);
        }

        tables.push(PseudoTable {
            label: format!("pseudo_table_{}", tables.len()),
            columns,
            column_coverage: coverage,
            total_rows: sorted_nodes.len(),
            core_properties: core,
            cliff_steepness,
            segments,
        });
    }

    PseudoTableRegistry { tables }
}

/// Get the value of a property for a specific node.
///
/// Looks up the triple store to find what value a node has for a given property.
/// For Subject properties, returns the object of the triple.
/// For Object properties, returns the subject of the triple.
///
/// If the node has multiple values for this property (multi-valued),
/// returns the first one found. Multi-valued properties are a limitation
/// of the columnar model — the pseudo-table stores only one value per cell.
fn get_property_value(node_id: TermId, property: &Property, store: &TripleStore) -> Option<TermId> {
    match property.position {
        PropertyPosition::Subject => {
            // Node is subject, property is predicate → value is object.
            let triples = store.find_by_subject_predicate(node_id, property.predicate);
            triples.first().map(|t| t.object)
        }
        PropertyPosition::Object => {
            // Node is object, property is predicate → value is subject.
            let triples = store.find_by_predicate_object(property.predicate, node_id);
            triples.first().map(|t| t.subject)
        }
    }
}

// ---------------------------------------------------------------------------
// Vectorized scan operations
// ---------------------------------------------------------------------------

/// Result of a vectorized column scan: matching row indices within a segment.
///
/// Used by the executor to efficiently filter pseudo-table segments without
/// examining individual triples. The executor can then join these row indices
/// back to the node TermIds for the final result.
#[derive(Debug)]
pub struct ScanResult {
    /// Indices into the segment's `nodes` array that passed the filter.
    pub matching_rows: Vec<usize>,
}

/// Scan a segment's column for rows matching an equality predicate.
///
/// This is the vectorized equivalent of `find_by_subject_predicate` — but
/// operates on contiguous columnar data instead of a B-tree index, enabling
/// better cache utilization and potential SIMD acceleration.
///
/// ## Vectorization strategy
///
/// The inner loop processes column values sequentially, which enables
/// auto-vectorization by the compiler (LLVM). For u64 TermId comparison,
/// the compiler can generate AVX2 code that compares 4 values per cycle.
///
/// Future work: explicit SIMD intrinsics for even faster comparison,
/// especially for range predicates on sorted columns.
pub fn scan_column_eq(segment: &Segment, col_idx: usize, value: TermId) -> ScanResult {
    // Zonemap pruning: skip the entire segment if the value can't be present.
    let stats = &segment.column_stats[col_idx];
    if !stats.range_could_match(Some(value), Some(value)) {
        return ScanResult {
            matching_rows: Vec::new(),
        };
    }

    // Vectorized scan: iterate column values and collect matching indices.
    // This loop is auto-vectorizable by LLVM when compiled with -C target-cpu=native.
    let col = &segment.columns[col_idx];
    let matching_rows: Vec<usize> = col
        .iter()
        .enumerate()
        .filter_map(
            |(idx, val)| {
                if *val == Some(value) {
                    Some(idx)
                } else {
                    None
                }
            },
        )
        .collect();

    ScanResult { matching_rows }
}

/// Scan a segment's column for rows matching a range predicate.
///
/// Supports open ranges (lo or hi can be None for unbounded).
/// Uses zonemap pruning to skip segments that can't contain matching values.
pub fn scan_column_range(
    segment: &Segment,
    col_idx: usize,
    lo: Option<TermId>,
    hi: Option<TermId>,
) -> ScanResult {
    // Zonemap pruning: skip if the range doesn't overlap the segment's min/max.
    let stats = &segment.column_stats[col_idx];
    if !stats.range_could_match(lo, hi) {
        return ScanResult {
            matching_rows: Vec::new(),
        };
    }

    let col = &segment.columns[col_idx];
    let matching_rows: Vec<usize> = col
        .iter()
        .enumerate()
        .filter_map(|(idx, val)| {
            if let Some(v) = val {
                let above_lo = lo.is_none_or(|lo| *v >= lo);
                let below_hi = hi.is_none_or(|hi| *v <= hi);
                if above_lo && below_hi {
                    Some(idx)
                } else {
                    None
                }
            } else {
                None // nulls never match range predicates
            }
        })
        .collect();

    ScanResult { matching_rows }
}

/// Scan a segment's column for non-null rows.
///
/// Useful for patterns like `?s :name ?name` where we want all nodes
/// that have the property, regardless of value.
pub fn scan_column_not_null(segment: &Segment, col_idx: usize) -> ScanResult {
    let col = &segment.columns[col_idx];
    let matching_rows: Vec<usize> = col
        .iter()
        .enumerate()
        .filter_map(|(idx, val)| if val.is_some() { Some(idx) } else { None })
        .collect();

    ScanResult { matching_rows }
}

/// Batch scan: intersect results from multiple column scans.
///
/// Used for multi-pattern queries like:
/// ```sparql
/// ?s :name ?name . ?s :age ?age . FILTER(?age > 25)
/// ```
///
/// The executor scans each column independently, then intersects the
/// matching row sets. This is the columnar equivalent of a multi-index
/// lookup in a row store.
///
/// ## SIMD opportunity
///
/// The intersection of sorted row index arrays can be accelerated with
/// SIMD merge operations (similar to merge join). For now, we use a
/// simple set intersection which is O(n log n) via sorted merge.
pub fn intersect_scan_results(results: &[ScanResult]) -> ScanResult {
    if results.is_empty() {
        return ScanResult {
            matching_rows: Vec::new(),
        };
    }

    // Start with the smallest result set (for early termination).
    let mut sorted_results: Vec<&ScanResult> = results.iter().collect();
    sorted_results.sort_by_key(|r| r.matching_rows.len());

    let mut intersection: Vec<usize> = sorted_results[0].matching_rows.clone();

    for result in &sorted_results[1..] {
        let other = &result.matching_rows;
        // Sorted merge intersection: O(n + m) where n, m are the two array sizes.
        let mut new_intersection = Vec::new();
        let mut i = 0;
        let mut j = 0;
        while i < intersection.len() && j < other.len() {
            match intersection[i].cmp(&other[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    new_intersection.push(intersection[i]);
                    i += 1;
                    j += 1;
                }
            }
        }
        intersection = new_intersection;

        // Early termination: if intersection is empty, no point continuing.
        if intersection.is_empty() {
            break;
        }
    }

    ScanResult {
        matching_rows: intersection,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triple::Triple;

    /// Build a test store with person-like nodes that share common predicates.
    fn make_person_store() -> (TripleStore, HashMap<&'static str, TermId>) {
        let mut store = TripleStore::new();
        let mut ids = HashMap::new();

        // Predicates
        let rdf_type = 1;
        let name = 2;
        let age = 3;
        let email = 4;
        let knows = 5;
        let city = 6;
        let person = 7;

        ids.insert("rdf:type", rdf_type);
        ids.insert("name", name);
        ids.insert("age", age);
        ids.insert("email", email);
        ids.insert("knows", knows);
        ids.insert("city", city);
        ids.insert("Person", person);

        // Create 20 person nodes with shared predicates.
        // All have: rdf:type, name, age, city, knows (5 core properties)
        // 15 have: email (75% coverage — optional but common)
        // 5 have: extra properties (tail)
        for i in 0..20u64 {
            let node = 100 + i;
            let name_val = 200 + i;
            let age_val = 300 + i;
            let city_val = 400 + (i % 5);
            let knows_target = 100 + ((i + 1) % 20);

            store.insert(Triple::new(node, rdf_type, person)).unwrap();
            store.insert(Triple::new(node, name, name_val)).unwrap();
            store.insert(Triple::new(node, age, age_val)).unwrap();
            store.insert(Triple::new(node, city, city_val)).unwrap();
            store
                .insert(Triple::new(node, knows, knows_target))
                .unwrap();

            // 15 out of 20 have email
            if i < 15 {
                let email_val = 500 + i;
                store.insert(Triple::new(node, email, email_val)).unwrap();
            }
        }

        // Add some unrelated triples (different schema)
        for i in 0..5u64 {
            let doc = 1000 + i;
            let title = 8;
            let content = 9;
            let title_val = 2000 + i;
            let content_val = 3000 + i;
            store.insert(Triple::new(doc, title, title_val)).unwrap();
            store
                .insert(Triple::new(doc, content, content_val))
                .unwrap();
        }

        (store, ids)
    }

    // -----------------------------------------------------------------------
    // Property extraction tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_properties_captures_both_positions() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);

        // Node 100 is a subject of rdf:type, name, age, city, knows, email
        let node_100_props = &node_props[&100];
        assert!(node_100_props.contains(&Property {
            predicate: ids["rdf:type"],
            position: PropertyPosition::Subject,
        }));
        assert!(node_100_props.contains(&Property {
            predicate: ids["name"],
            position: PropertyPosition::Subject,
        }));

        // Person (id 7) is an object of rdf:type
        let person_props = &node_props[&7];
        assert!(person_props.contains(&Property {
            predicate: ids["rdf:type"],
            position: PropertyPosition::Object,
        }));
    }

    // -----------------------------------------------------------------------
    // Discovery tests
    // -----------------------------------------------------------------------

    #[test]
    fn discovers_person_pseudo_table() {
        let (store, _ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        // Should discover at least one pseudo-table for the person nodes.
        assert!(
            !registry.is_empty(),
            "Should discover at least one pseudo-table for 20 person nodes"
        );

        // The largest table should have ~20 rows (the person nodes).
        let largest = registry.tables.iter().max_by_key(|t| t.total_rows).unwrap();
        assert!(
            largest.total_rows >= 15,
            "Largest pseudo-table should have at least 15 rows, got {}",
            largest.total_rows
        );

        // Should have at least 5 columns (the core properties).
        assert!(
            largest.columns.len() >= 5,
            "Should have at least 5 columns, got {}",
            largest.columns.len()
        );
    }

    #[test]
    fn cliff_steepness_is_positive() {
        let (store, _ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        for table in &registry.tables {
            assert!(
                table.cliff_steepness > 0.0,
                "Cliff steepness should be positive, got {}",
                table.cliff_steepness
            );
        }
    }

    // -----------------------------------------------------------------------
    // Segment and zonemap tests
    // -----------------------------------------------------------------------

    #[test]
    fn segments_have_stats() {
        let (store, _ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        if let Some(table) = registry.tables.first() {
            for segment in &table.segments {
                for (col_idx, stats) in segment.column_stats.iter().enumerate() {
                    assert_eq!(
                        stats.row_count,
                        segment.len(),
                        "Stats row count should match segment length for col {}",
                        col_idx
                    );
                }
            }
        }
    }

    #[test]
    fn zonemap_pruning_works() {
        let mut segment = Segment::new(1);
        // Add values 10, 20, 30
        segment.nodes = vec![1, 2, 3];
        segment.columns = vec![vec![Some(10), Some(20), Some(30)]];
        segment.tail_counts = vec![0, 0, 0];
        segment.compute_stats();

        // Value 20 is within range — should find it.
        let result = scan_column_eq(&segment, 0, 20);
        assert_eq!(result.matching_rows, vec![1]);

        // Value 50 is outside range — zonemap should prune.
        let result = scan_column_eq(&segment, 0, 50);
        assert!(result.matching_rows.is_empty());

        // Range [15, 25] should find value 20.
        let result = scan_column_range(&segment, 0, Some(15), Some(25));
        assert_eq!(result.matching_rows, vec![1]);

        // Range [40, 50] should be pruned by zonemap.
        let result = scan_column_range(&segment, 0, Some(40), Some(50));
        assert!(result.matching_rows.is_empty());
    }

    // -----------------------------------------------------------------------
    // Vectorized scan tests
    // -----------------------------------------------------------------------

    #[test]
    fn scan_not_null() {
        let mut segment = Segment::new(1);
        segment.nodes = vec![1, 2, 3, 4];
        segment.columns = vec![vec![Some(10), None, Some(30), None]];
        segment.tail_counts = vec![0; 4];
        segment.compute_stats();

        let result = scan_column_not_null(&segment, 0);
        assert_eq!(result.matching_rows, vec![0, 2]);
    }

    #[test]
    fn intersect_scans() {
        let scan1 = ScanResult {
            matching_rows: vec![0, 1, 2, 5, 8],
        };
        let scan2 = ScanResult {
            matching_rows: vec![1, 2, 3, 5, 7],
        };
        let scan3 = ScanResult {
            matching_rows: vec![2, 5, 9],
        };

        let result = intersect_scan_results(&[scan1, scan2, scan3]);
        assert_eq!(result.matching_rows, vec![2, 5]);
    }

    #[test]
    fn intersect_empty_result() {
        let scan1 = ScanResult {
            matching_rows: vec![0, 1, 2],
        };
        let scan2 = ScanResult {
            matching_rows: vec![5, 6, 7],
        };

        let result = intersect_scan_results(&[scan1, scan2]);
        assert!(result.matching_rows.is_empty());
    }

    #[test]
    fn column_stats_selectivity() {
        let stats = ColumnStats {
            min_value: Some(1),
            max_value: Some(100),
            null_count: 5,
            distinct_count: 50,
            row_count: 100,
        };

        // Equality selectivity: 1/50 = 0.02
        assert!((stats.equality_selectivity() - 0.02).abs() < 0.001);

        // Range checks
        assert!(stats.range_could_match(Some(50), Some(60))); // within range
        assert!(!stats.range_could_match(Some(200), Some(300))); // above max
        assert!(!stats.range_could_match(None, Some(0))); // below min (hi < min)
    }

    // -----------------------------------------------------------------------
    // Registry tests
    // -----------------------------------------------------------------------

    #[test]
    fn registry_find_tables_for_property() {
        let (store, ids) = make_person_store();
        let node_props = extract_node_properties(&store);
        let registry = discover_pseudo_tables(&node_props, &store);

        // The "name" property as Subject should be in a pseudo-table.
        let name_prop = Property {
            predicate: ids["name"],
            position: PropertyPosition::Subject,
        };
        let matches = registry.find_tables_for_property(&name_prop);

        // Should find at least one table with this property as a column.
        if !registry.is_empty() {
            assert!(
                !matches.is_empty(),
                "Should find pseudo-table with 'name' column"
            );
        }
    }
}
