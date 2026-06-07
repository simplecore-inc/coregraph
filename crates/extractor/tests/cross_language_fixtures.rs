//! Integration tests against `tests/fixtures/cross-lang-matched`.
//!
//! These assertions document the cross-language joins CoreGraph
//! currently produces, so a regression in any pipeline stage is caught.
//! They also serve as executable spec: new capabilities (e.g. extending
//! EnumValueMatch to TS const literals) should flip the corresponding
//! `assert!` here from the current baseline.

use coregraph_core::EdgeKind;
use coregraph_extractor::build_graph;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("cross-lang-matched")
}

#[test]
fn cross_lang_api_path_match_connects_java_controller_and_ts_client() {
    let (graph, _) = build_graph(&fixture_root()).expect("build_graph ok");

    let api_matches = graph
        .edges()
        .filter(|e| e.kind == EdgeKind::ApiPathMatch)
        .count();

    // Two endpoints × two callers = 4 (GET/POST × listUsers/createUser).
    // Baseline: at least 2 matches must exist — GET and POST paths.
    assert!(
        api_matches >= 2,
        "expected ≥2 ApiPathMatch edges, got {}",
        api_matches
    );
}

#[test]
fn cross_lang_api_paths_point_across_language_boundary() {
    let (graph, _) = build_graph(&fixture_root()).expect("build_graph ok");

    let boundaries = graph
        .edges()
        .filter(|e| e.kind == EdgeKind::ApiPathMatch)
        .filter_map(|e| {
            let from = graph.get_node(e.from)?;
            let to = graph.get_node(e.to)?;
            Some((
                from.file.display().to_string(),
                to.file.display().to_string(),
            ))
        })
        .filter(|(a, b)| {
            (a.ends_with(".java") && b.ends_with(".ts"))
                || (a.ends_with(".ts") && b.ends_with(".java"))
        })
        .count();

    assert!(
        boundaries >= 1,
        "expected ≥1 ApiPathMatch crossing .java ↔ .ts, got {}",
        boundaries
    );
}

#[test]
fn cross_lang_fixture_contains_expected_kinds() {
    let (graph, _) = build_graph(&fixture_root()).expect("build_graph ok");

    // Expected kinds for this fixture. Documents current capability so
    // that breakage is loud.
    let mut seen_enum_variant = false;
    let mut seen_string_literal_api = false;
    let mut seen_config_key_yaml = false;

    for n in graph.nodes() {
        use coregraph_core::SymbolKind;
        if matches!(n.kind, SymbolKind::EnumVariant) {
            seen_enum_variant = true;
        }
        if matches!(n.kind, SymbolKind::StringLiteral) && n.name.starts_with("api_path::") {
            seen_string_literal_api = true;
        }
        if matches!(n.kind, SymbolKind::ConfigKey) && !n.name.starts_with("config_ref::") {
            seen_config_key_yaml = true;
        }
    }

    assert!(
        seen_enum_variant,
        "Java enum_constant must produce EnumVariant nodes"
    );
    assert!(
        seen_string_literal_api,
        "api_path:: string literals must be extracted"
    );
    assert!(
        seen_config_key_yaml,
        "YAML ConfigKey definitions must be present"
    );
}
