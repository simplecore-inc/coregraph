use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::symbol::SymbolId;

/// Classifies the semantic relationship between two symbols.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EdgeKind {
    // Code relationships
    Calls,
    References,
    Imports,
    /// Class-to-class extends (Java `extends`, Kotlin `:`). Spec §4.2.
    Extends,
    /// Class-to-class / trait-to-trait inheritance when the grammar uses
    /// a separate keyword from interface-implementation (Java `extends`).
    /// In languages without the distinction we still emit `Extends`; this
    /// variant exists for languages that do differentiate.
    Inherits,
    Implements,
    Overrides,
    /// Variable / field / parameter declares a type relationship
    /// (`let x: Foo`, `Foo field`, `function foo(): Bar`).
    TypeOf,
    /// Generic type parameter binding (`List<T>` → `T`).
    GenericParam,
    // Name resolution (stack-graphs output)
    Resolves,
    // Value / config relationships
    StringMatch,
    /// Enum literal ↔ string literal match (split out from StringMatch so
    /// the Bidirectional trust model can target enum mismatches cleanly).
    EnumValueMatch,
    /// API path literal ↔ server route declaration match.
    ApiPathMatch,
    Configures,
    // Structural relationships (docs §4.2 Contains/BelongsTo)
    /// File → Symbol structural containment.
    Contains,
    /// Symbol → Module / Namespace membership.
    BelongsTo,
    // Manifest relationships
    DependsOn,
    // Documentation relationships (docs/graph-model.md §7)
    /// DocComment → Symbol: a doc comment documents a code symbol. Emitted by
    /// the documentation pass from dedicated doc-comment syntax (`///`, `//!`,
    /// `/** */`) immediately attached to a definition. Same-file structural
    /// attachment, so SourceEvidenced.
    Documents,
    /// DocComment → Symbol: a doc comment's text *mentions* a code symbol via an
    /// intra-doc link (`` [`Name`] ``, `{@link Name}`). The link text is the
    /// evidence (in the doc's own file), so SourceEvidenced; resolution is
    /// name-based and may cross files. PatternMatched (0.60).
    Mentions,
    /// Symbol → DocSection: a code symbol is described in an external Markdown
    /// section that references it by a backticked name (`` `Name` ``). Both the
    /// symbol and the (separate) doc file participate, so Bidirectional.
    /// PatternMatched (0.60).
    DescribedIn,
}

impl EdgeKind {
    /// Classify this edge kind under the four-way Trust Model taxonomy.
    /// See `docs/graph-model.md §5.2`.
    pub fn trust_model(&self) -> TrustModel {
        match self {
            // Source-evidenced: relationship is directly observable in code.
            EdgeKind::Calls
            | EdgeKind::References
            | EdgeKind::Imports
            | EdgeKind::Resolves
            | EdgeKind::DependsOn
            | EdgeKind::Contains
            | EdgeKind::BelongsTo
            // A doc comment and the symbol it documents live in the SAME file,
            // so healing only ever re-parses that one file — SourceEvidenced.
            // (Cross-file doc relationships, e.g. external markdown, will be a
            // distinct kind with a Bidirectional / ExternallyMediated model.)
            | EdgeKind::Documents
            // A doc-text mention: the link text is evidence in the doc's own
            // source file, so healing re-parses that file — SourceEvidenced.
            | EdgeKind::Mentions => TrustModel::SourceEvidenced,
            // Contract-dependent: depends on type hierarchy / contract layer.
            EdgeKind::Extends
            | EdgeKind::Inherits
            | EdgeKind::Implements
            | EdgeKind::Overrides
            | EdgeKind::TypeOf
            | EdgeKind::GenericParam => TrustModel::ContractDependent,
            // Bidirectional: both endpoints participate in the same literal match.
            EdgeKind::StringMatch
            | EdgeKind::EnumValueMatch
            | EdgeKind::ApiPathMatch
            // A symbol and the external doc section describing it: the symbol's
            // name and the doc's reference must agree; healing re-parses both.
            | EdgeKind::DescribedIn => TrustModel::Bidirectional,
            // Externally-mediated: a third document binds the two endpoints
            // (config files, DI containers, routing tables).
            EdgeKind::Configures => TrustModel::ExternallyMediated,
        }
    }
}

/// Confidence score in [0.0, 1.0] for an edge.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Confidence(pub f32);

impl Confidence {
    pub const MAX: Confidence = Confidence(1.0);
    pub const MIN: Confidence = Confidence(0.0);

    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }
}

/// Origin layer that produced this edge — the analysis pipeline's confidence
/// in its own output. See `docs/graph-model.md §6.2`.
///
/// Prior to this rename this type was called `TrustModel`. Serde aliases keep
/// old snapshots loadable: the variant `NameResolved` deserializes from the
/// legacy `Resolved` variant, and `ConventionInferred` deserializes from
/// `Asserted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AnalysisOrigin {
    /// 0.99 — structurally certain: a fact the extractor directly observes
    /// in the syntax tree (e.g. file→symbol containment, module membership).
    CompilerDerived,
    /// 0.95 — cross-file name resolution (stack-graphs)
    #[serde(alias = "Resolved")]
    NameResolved,
    /// 0.85 — tree-sitter syntactic match
    SyntaxMatched,
    /// 0.60 — pattern-based inference (API path regexes, framework idioms)
    PatternMatched,
    /// 0.40 — convention- or config-derived (framework mediators)
    #[serde(alias = "Asserted")]
    ConventionInferred,
    /// 0.20 — reflection / dynamic dispatch / eval
    Dynamic,
}

impl AnalysisOrigin {
    /// Baseline confidence for edges of this origin before stale-evidence decay.
    pub fn base_score(&self) -> f64 {
        match self {
            AnalysisOrigin::CompilerDerived => 0.99,
            AnalysisOrigin::NameResolved => 0.95,
            AnalysisOrigin::SyntaxMatched => 0.85,
            AnalysisOrigin::PatternMatched => 0.60,
            AnalysisOrigin::ConventionInferred => 0.40,
            AnalysisOrigin::Dynamic => 0.20,
        }
    }
}

/// The four-way relationship category that determines healing behavior and
/// how endpoints participate in the edge's validity. Distinct from
/// `AnalysisOrigin`: an edge's origin describes *how* it was produced; its
/// trust model describes *which endpoints* provide evidence.
/// See `docs/graph-model.md §5.2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TrustModel {
    /// Source code of the source symbol provides direct evidence (calls,
    /// imports, references, dependencies, resolves). Healing re-parses source only.
    SourceEvidenced,
    /// Evidence depends on a type contract shared between source and target
    /// (inherits, implements, overrides, typeOf, genericParam). Healing
    /// re-parses both endpoints.
    ContractDependent,
    /// Both endpoints must participate in the same literal value
    /// (stringMatch, enumValueMatch, apiPathMatch). Healing re-parses both sides.
    Bidirectional,
    /// A third file (config, container, router) mediates the relationship
    /// (configBinding, dependencyInjection, routeMapping). Healing re-parses
    /// source + target + mediator.
    ExternallyMediated,
}

/// Which endpoints the healing pipeline must re-parse to refresh an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum HealingScope {
    /// Re-parse the source file only.
    SourceOnly,
    /// Re-parse source + target files.
    BothEndpoints,
    /// Re-parse source + target + mediator files.
    EndpointsPlusMediator,
}

impl TrustModel {
    /// The subset of files the healing pipeline must re-parse to refresh an
    /// edge in this trust model. See `docs/graph-model.md §5.2`.
    pub fn healing_scope(&self) -> HealingScope {
        match self {
            TrustModel::SourceEvidenced => HealingScope::SourceOnly,
            TrustModel::ContractDependent | TrustModel::Bidirectional => {
                HealingScope::BothEndpoints
            }
            TrustModel::ExternallyMediated => HealingScope::EndpointsPlusMediator,
        }
    }
}

/// A directed edge in the symbol graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DirectEdge {
    pub from: SymbolId,
    pub to: SymbolId,
    pub kind: EdgeKind,
    /// Analysis origin — how this edge was produced.
    /// Legacy snapshots serialize this field as `trust`; serde aliases below
    /// allow either name to deserialize, preserving backward compatibility.
    #[serde(alias = "trust")]
    pub origin: AnalysisOrigin,
    /// Stored confidence at the moment of creation. Use `confidence()` for the
    /// current decay-adjusted value.
    pub confidence: Confidence,
    /// Source file that provides evidence for this edge.
    ///
    /// `Arc<Path>` so the graph interns it — all edges whose evidence lives in
    /// the same file share one allocation. See `SymbolNode::file`.
    #[serde(with = "crate::arc_path")]
    pub evidence_file: Arc<Path>,
    /// Graph epoch at which this edge was created.
    /// Used to identify stale edges after incremental invalidations.
    /// Absent in legacy snapshots (deserialized as 0).
    #[serde(default)]
    pub created_at_epoch: u64,
    /// Count of evidence-provider files that have become stale since this edge
    /// was created. Multiplies the base confidence by 0.7^n per `docs/graph-model.md §6.3`.
    /// Absent in legacy snapshots (deserialized as 0).
    #[serde(default)]
    pub stale_evidence_count: u32,
}

impl DirectEdge {
    pub fn new(
        from: SymbolId,
        to: SymbolId,
        kind: EdgeKind,
        origin: AnalysisOrigin,
        confidence: Confidence,
        evidence_file: impl Into<PathBuf>,
    ) -> Self {
        Self {
            from,
            to,
            kind,
            origin,
            confidence,
            evidence_file: Arc::from(evidence_file.into()),
            created_at_epoch: 0,
            stale_evidence_count: 0,
        }
    }

    /// Return a clone with `created_at_epoch` set.
    pub fn with_epoch(mut self, epoch: u64) -> Self {
        self.created_at_epoch = epoch;
        self
    }

    /// Return a clone with `stale_evidence_count` set.
    pub fn with_stale_evidence_count(mut self, count: u32) -> Self {
        self.stale_evidence_count = count;
        self
    }

    /// True when this edge predates (or equals) the given "cutoff" epoch —
    /// i.e. was created before the last invalidation.
    pub fn is_stale(&self, current_epoch: u64, cutoff: u64) -> bool {
        current_epoch > 0 && self.created_at_epoch < cutoff
    }

    /// Classify which endpoints must re-parse to refresh this edge.
    pub fn trust_model(&self) -> TrustModel {
        self.kind.trust_model()
    }

    /// Current confidence accounting for stale evidence, per
    /// `docs/graph-model.md §6.3`: `origin_score * 0.7^stale_evidence_count`.
    pub fn current_confidence(&self) -> f64 {
        let base = self.origin.base_score();
        base * 0.7_f64.powi(self.stale_evidence_count as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_construction() {
        let edge = DirectEdge::new(
            SymbolId(1),
            SymbolId(2),
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.8),
            PathBuf::from("src/main.rs"),
        );
        assert_eq!(edge.from, SymbolId(1));
        assert_eq!(edge.to, SymbolId(2));
        assert_eq!(edge.kind, EdgeKind::Calls);
    }

    #[test]
    fn confidence_clamped() {
        let c = Confidence::new(1.5);
        assert_eq!(c.0, 1.0);
        let c2 = Confidence::new(-0.5);
        assert_eq!(c2.0, 0.0);
    }

    #[test]
    fn edge_serde_roundtrip() {
        let edge = DirectEdge::new(
            SymbolId(10),
            SymbolId(20),
            EdgeKind::Imports,
            AnalysisOrigin::CompilerDerived,
            Confidence::MAX,
            PathBuf::from("src/lib.rs"),
        );
        let json = serde_json::to_string(&edge).expect("serialize");
        let back: DirectEdge = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.from, edge.from);
        assert_eq!(back.kind, EdgeKind::Imports);
        assert_eq!(back.origin, AnalysisOrigin::CompilerDerived);
    }

    #[test]
    fn legacy_snapshot_trust_field_deserializes() {
        // Prior snapshots serialized the origin as `trust: "Resolved"`.
        let legacy = r#"{
            "from": 1, "to": 2, "kind": "Calls",
            "trust": "Resolved",
            "confidence": 0.95,
            "evidence_file": "src/main.rs"
        }"#;
        let edge: DirectEdge = serde_json::from_str(legacy).expect("legacy deserialize");
        assert_eq!(edge.origin, AnalysisOrigin::NameResolved);
        assert_eq!(edge.stale_evidence_count, 0);
    }

    #[test]
    fn legacy_asserted_variant_migrates() {
        let legacy = r#"{
            "from": 1, "to": 2, "kind": "Configures",
            "trust": "Asserted",
            "confidence": 0.4,
            "evidence_file": "config.yml"
        }"#;
        let edge: DirectEdge = serde_json::from_str(legacy).expect("legacy deserialize");
        assert_eq!(edge.origin, AnalysisOrigin::ConventionInferred);
    }

    #[test]
    fn confidence_decays_with_stale_evidence() {
        let edge = DirectEdge::new(
            SymbolId(1),
            SymbolId(2),
            EdgeKind::Calls,
            AnalysisOrigin::CompilerDerived,
            Confidence::new(0.99),
            PathBuf::from("src/main.rs"),
        )
        .with_stale_evidence_count(2);
        // 0.99 * 0.7 * 0.7 = 0.4851
        let c = edge.current_confidence();
        assert!((c - 0.4851).abs() < 1e-4, "got {}", c);
    }

    #[test]
    fn trust_model_classifies_edge_kinds() {
        assert_eq!(EdgeKind::Calls.trust_model(), TrustModel::SourceEvidenced);
        assert_eq!(
            EdgeKind::Implements.trust_model(),
            TrustModel::ContractDependent
        );
        assert_eq!(
            EdgeKind::StringMatch.trust_model(),
            TrustModel::Bidirectional
        );
        assert_eq!(
            EdgeKind::Configures.trust_model(),
            TrustModel::ExternallyMediated
        );
    }

    #[test]
    fn healing_scope_matches_trust_model() {
        assert_eq!(
            TrustModel::SourceEvidenced.healing_scope(),
            HealingScope::SourceOnly
        );
        assert_eq!(
            TrustModel::ContractDependent.healing_scope(),
            HealingScope::BothEndpoints
        );
        assert_eq!(
            TrustModel::Bidirectional.healing_scope(),
            HealingScope::BothEndpoints
        );
        assert_eq!(
            TrustModel::ExternallyMediated.healing_scope(),
            HealingScope::EndpointsPlusMediator
        );
    }
}
