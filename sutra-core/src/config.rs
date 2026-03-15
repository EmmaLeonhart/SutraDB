//! Database configuration: RDF mode, HNSW edge exposure, OWL support.
//!
//! These settings control how the database behaves at a fundamental level.
//! They are set at database creation time and can be changed at runtime
//! for some settings (e.g. HNSW edge mode).

/// How RDF triples are handled — controls whether quoted triples (edges-as-subjects)
/// are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdfMode {
    /// RDF-star: quoted triples can appear in both subject and object positions.
    /// This is the default and recommended mode. Enables `<< s p o >> :meta :value .`
    RdfStar,
    /// RDF 1.2: quoted triples can appear only in the object position.
    /// `<< s p o >>` is valid as an object but not as a subject.
    Rdf12,
    /// Legacy RDF 1.1: no quoted triples at all. Plain triples only.
    /// Some customers with existing RDF 1.1 data may need this.
    Legacy,
}

/// How HNSW neighbor edges are exposed to the query layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HnswEdgeMode {
    /// Virtual: HNSW edges are generated on-the-fly when queried via SPARQL.
    /// Zero write overhead. Read-only view of the HNSW graph structure.
    /// This is the default.
    Virtual,
    /// Materialized: HNSW edges are stored as real triples in the SPO/POS/OSP
    /// indexes. Queryable like any triple, but adds write cost on HNSW mutations.
    Materialized,
}

/// Top-level database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// RDF compatibility mode. Default: RdfStar.
    pub rdf_mode: RdfMode,
    /// How HNSW neighbor connections are exposed. Default: Virtual.
    pub hnsw_edge_mode: HnswEdgeMode,
    /// Whether OWL reasoning is enabled (opt-in, query-time only).
    /// Default: true (enabled but requires explicit schema setup).
    pub owl_enabled: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            rdf_mode: RdfMode::RdfStar,
            hnsw_edge_mode: HnswEdgeMode::Virtual,
            owl_enabled: true,
        }
    }
}

impl DatabaseConfig {
    /// Create a config with all defaults: RDF-star, virtual HNSW edges, OWL enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether quoted triples are allowed in subject position.
    pub fn allows_quoted_subject(&self) -> bool {
        self.rdf_mode == RdfMode::RdfStar
    }

    /// Whether quoted triples are allowed at all.
    pub fn allows_quoted_triples(&self) -> bool {
        self.rdf_mode != RdfMode::Legacy
    }

    /// Whether HNSW edges should be materialized into the triple store.
    pub fn materialize_hnsw_edges(&self) -> bool {
        self.hnsw_edge_mode == HnswEdgeMode::Materialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = DatabaseConfig::default();
        assert_eq!(cfg.rdf_mode, RdfMode::RdfStar);
        assert_eq!(cfg.hnsw_edge_mode, HnswEdgeMode::Virtual);
        assert!(cfg.owl_enabled);
        assert!(cfg.allows_quoted_subject());
        assert!(cfg.allows_quoted_triples());
        assert!(!cfg.materialize_hnsw_edges());
    }

    #[test]
    fn rdf12_mode() {
        let cfg = DatabaseConfig {
            rdf_mode: RdfMode::Rdf12,
            ..Default::default()
        };
        assert!(!cfg.allows_quoted_subject());
        assert!(cfg.allows_quoted_triples());
    }

    #[test]
    fn legacy_mode() {
        let cfg = DatabaseConfig {
            rdf_mode: RdfMode::Legacy,
            ..Default::default()
        };
        assert!(!cfg.allows_quoted_subject());
        assert!(!cfg.allows_quoted_triples());
    }

    #[test]
    fn materialized_hnsw() {
        let cfg = DatabaseConfig {
            hnsw_edge_mode: HnswEdgeMode::Materialized,
            ..Default::default()
        };
        assert!(cfg.materialize_hnsw_edges());
    }
}
