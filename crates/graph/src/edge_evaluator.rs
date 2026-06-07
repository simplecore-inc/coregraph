use coregraph_core::edge::{AnalysisOrigin, Confidence};
use coregraph_core::EdgeKind;

/// Evaluates the confidence of an edge based on its kind and analysis origin.
pub struct EdgeEvaluator;

impl EdgeEvaluator {
    /// Compute confidence for an edge.
    pub fn evaluate(kind: EdgeKind, origin: AnalysisOrigin) -> Confidence {
        let base_score: f32 = match kind {
            EdgeKind::Resolves => 1.0,
            EdgeKind::Calls => 0.9,
            EdgeKind::Implements => 0.9,
            EdgeKind::Extends => 0.9,
            EdgeKind::Inherits => 0.9,
            EdgeKind::Overrides => 0.85,
            EdgeKind::References => 0.8,
            EdgeKind::Imports => 0.85,
            EdgeKind::TypeOf => 0.85,
            EdgeKind::GenericParam => 0.8,
            EdgeKind::StringMatch => 0.7,
            EdgeKind::EnumValueMatch => 0.75,
            EdgeKind::ApiPathMatch => 0.75,
            EdgeKind::Configures => 0.7,
            EdgeKind::Contains => 1.0,
            EdgeKind::BelongsTo => 1.0,
            EdgeKind::DependsOn => 0.6,
            // A doc comment attached to a definition: the kind itself does not
            // discount (1.0); the analysis origin sets the level (SyntaxMatched
            // 0.85 for dedicated `///`/JSDoc syntax adjacent to the symbol).
            EdgeKind::Documents => 1.0,
            // A doc-text mention (intra-doc link): name-based resolution, level
            // set by origin (PatternMatched 0.60 for `{@link}` / `` [`X`] ``).
            EdgeKind::Mentions => 1.0,
            // A symbol described in an external Markdown section: level set by
            // origin (PatternMatched 0.60 for a backticked `` `Name` `` ref).
            EdgeKind::DescribedIn => 1.0,
        };
        // Origin weight follows the base_score table in docs/graph-model.md §6.2.
        let weight: f32 = origin.base_score() as f32;
        Confidence::new((base_score * weight).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_compiler_derived_is_high_confidence() {
        let c = EdgeEvaluator::evaluate(EdgeKind::Resolves, AnalysisOrigin::CompilerDerived);
        assert!(c.0 > 0.95);
    }

    #[test]
    fn depends_on_syntax_matched_is_low() {
        let c = EdgeEvaluator::evaluate(EdgeKind::DependsOn, AnalysisOrigin::SyntaxMatched);
        assert!(c.0 < 0.6);
    }

    #[test]
    fn higher_origin_same_kind_gives_higher_confidence() {
        let compiler = EdgeEvaluator::evaluate(EdgeKind::Calls, AnalysisOrigin::CompilerDerived);
        let syntax = EdgeEvaluator::evaluate(EdgeKind::Calls, AnalysisOrigin::SyntaxMatched);
        assert!(compiler.0 > syntax.0);
    }

    #[test]
    fn all_edge_kinds_covered() {
        for kind in [
            EdgeKind::Resolves,
            EdgeKind::Calls,
            EdgeKind::Implements,
            EdgeKind::Extends,
            EdgeKind::Overrides,
            EdgeKind::References,
            EdgeKind::Imports,
            EdgeKind::StringMatch,
            EdgeKind::Configures,
            EdgeKind::DependsOn,
        ] {
            let c = EdgeEvaluator::evaluate(kind, AnalysisOrigin::NameResolved);
            assert!(c.0 > 0.0 && c.0 <= 1.0);
        }
    }
}
