//! Isolated cross-file resolution tests: the hand-authored Go `.tsg` must resolve a
//! same-package, cross-file top-level reference, AND must let a function-local
//! definition shadow a package-level name (no false cross-file edge).
//!
//! Go is NOT wired into the production backend yet (see
//! `crates/stack/rules/go.tsg`). These tests exercise the rules directly via
//! `LanguageConfiguration::from_sources` so authoring can proceed without
//! touching the live pipeline, keeping production green.

use std::path::PathBuf;

use stack_graphs::graph::StackGraph;
use stack_graphs::partial::PartialPaths;
use stack_graphs::stitching::{
    Database, DatabaseCandidates, ForwardPartialPathStitcher, StitcherConfig,
};
use stack_graphs::NoCancellation;
use tree_sitter_stack_graphs::loader::LanguageConfiguration;
use tree_sitter_stack_graphs::{NoCancellation as TsNoCancel, Variables};

const GO_TSG: &str = include_str!("../rules/go.tsg");

fn build_config() -> LanguageConfiguration {
    LanguageConfiguration::from_sources(
        tree_sitter_go::LANGUAGE.into(),
        Some("source.go".into()),
        None,
        vec!["go".into()],
        PathBuf::from("go.tsg"),
        GO_TSG,
        None,
        None,
        // from_sources wants tree_sitter_stack_graphs' cancellation flag here;
        // the stitcher phases below use stack_graphs' own NoCancellation.
        &TsNoCancel,
    )
    .expect("go.tsg must compile into a LanguageConfiguration")
}

/// Build a stack graph for `files` (all in package directory `pkg_path`),
/// stitch complete paths, and return `(cross_file, same_file)` resolution
/// counts. `cross_file` = a reference resolves to a definition in a *different*
/// file; `same_file` = it resolves to a definition in the *same* file (e.g. a
/// function-local short-var declaration).
fn resolution_counts(files: &[(&str, &str)], pkg_path: &str) -> (usize, usize) {
    let config = build_config();
    let mut sg = StackGraph::new();

    let mut file_handles = Vec::new();
    for (path, source) in files {
        let file = sg.add_file(path).expect("add_file");
        let mut globals = Variables::new();
        globals
            .add("PKG_PATH".into(), pkg_path.into())
            .expect("set PKG_PATH global");
        config
            .sgl
            .build_stack_graph_into(&mut sg, file, source, &globals, &TsNoCancel)
            .expect("build stack graph for file");
        file_handles.push(file);
    }

    let mut partials = PartialPaths::new();
    let mut db = Database::new();
    for file in &file_handles {
        ForwardPartialPathStitcher::find_minimal_partial_path_set_in_file(
            &sg,
            &mut partials,
            *file,
            StitcherConfig::default(),
            &NoCancellation,
            |g, p, path| {
                db.add_partial_path(g, p, path.clone());
            },
        )
        .expect("phase 1 partial paths");
    }

    let references: Vec<_> = sg.iter_nodes().filter(|&h| sg[h].is_reference()).collect();
    let mut all_paths: Vec<stack_graphs::partial::PartialPath> = Vec::new();
    {
        let mut candidates = DatabaseCandidates::new(&sg, &mut partials, &mut db);
        let _ = ForwardPartialPathStitcher::find_all_complete_partial_paths(
            &mut candidates,
            references,
            StitcherConfig::default(),
            &NoCancellation,
            |_, _, path| {
                all_paths.push(path.clone());
            },
        );
    }

    // Consumer-side shadowing. stack-graphs' stitcher returns EVERY complete
    // path — it never auto-suppresses a lexical inner definition that hides an
    // outer one (its built-in shadowing only de-duplicates equal paths to the
    // same definition). We apply `PartialPath::shadows`, which walks two paths'
    // edges and reports that a higher-precedence path (e.g. a function-local
    // binding, precedence 1) shadows a lower-precedence one (the package
    // fall-through, precedence 0) for the same reference. Shadowed paths are
    // dropped so a local binding correctly hides a package-level name.
    let mut cross_file = 0usize;
    let mut same_file = 0usize;
    for (i, p) in all_paths.iter().enumerate() {
        if p.start_node == p.end_node {
            continue;
        }
        let shadowed = all_paths
            .iter()
            .enumerate()
            .any(|(j, q)| i != j && q.shadows(&mut partials, p));
        if shadowed {
            continue;
        }
        let from = sg[p.start_node].file();
        let to = sg[p.end_node].file();
        if let (Some(f), Some(t)) = (from, to) {
            if f != t {
                cross_file += 1;
            } else {
                same_file += 1;
            }
        }
    }
    (cross_file, same_file)
}

#[test]
fn go_resolves_same_package_cross_file_call() {
    // b.go calls Helper(), defined in a.go (same package) → cross-file resolve.
    let (cross, _) = resolution_counts(
        &[
            ("pkg/a.go", "package pkg\n\nfunc Helper() {}\n"),
            ("pkg/b.go", "package pkg\n\nfunc Use() { Helper() }\n"),
        ],
        "pkg",
    );
    assert!(
        cross >= 1,
        "Helper() in b.go must resolve cross-file to its definition in a.go, got {cross}"
    );
}

#[test]
fn go_resolves_function_local_short_var() {
    // a.go exports `other` (returns a func). b.go calls it cross-file AND binds
    // the result to a function-local short-var `loc`, then calls `loc()` — which
    // must resolve to the same-file local. This exercises both top-level
    // cross-file resolution and the local-scope (short_var) modelling.
    //
    // NOTE on shadowing: stack-graphs' built-in shadowing only de-duplicates
    // EQUAL paths to the SAME definition; a function-local that hides a
    // *different* package-level definition is not auto-suppressed at the
    // stitcher. Suppressing that outer edge needs consumer-side precedence
    // selection in the backend — a cross-language concern (Java/TS/Python share
    // it) tracked as a follow-up. Until then Go stays in the dev-only path; this
    // test verifies local resolution happens, not outer-edge suppression.
    let (cross, same) = resolution_counts(
        &[
            (
                "pkg/a.go",
                "package pkg\n\nfunc other() func() { return nil }\n",
            ),
            (
                "pkg/b.go",
                "package pkg\n\nfunc Use() { loc := other(); loc() }\n",
            ),
        ],
        "pkg",
    );
    assert!(
        cross >= 1,
        "other() in b.go must resolve cross-file to a.go, got cross={cross}"
    );
    assert!(
        same >= 1,
        "the local `loc` must resolve same-file (short-var local scoping), got same={same}"
    );
}

#[test]
fn go_local_shadows_package_no_cross_file_edge() {
    // b.go declares a function-local `Helper` (a `:=` binding) that shadows the
    // package-level `Helper` in a.go, then calls it. With consumer-side
    // shadowing applied, the call resolves to the LOCAL only — so there is NO
    // cross-file edge to a.go's package `Helper`.
    let (cross, same) = resolution_counts(
        &[
            ("pkg/a.go", "package pkg\n\nfunc Helper() {}\n"),
            (
                "pkg/b.go",
                "package pkg\n\nfunc Use() { Helper := func() {}; Helper() }\n",
            ),
        ],
        "pkg",
    );
    assert_eq!(
        cross, 0,
        "a function-local Helper must shadow the package Helper — expected 0 \
         cross-file resolutions after shadowing, got cross={cross}"
    );
    assert!(
        same >= 1,
        "the call must still resolve to the same-file local Helper, got same={same}"
    );
}
