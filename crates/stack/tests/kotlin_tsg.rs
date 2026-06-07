//! Isolated cross-file resolution tests for the hand-authored Kotlin `.tsg`.
//!
//! Kotlin is NOT wired into the production backend yet (see
//! `crates/stack/rules/kotlin.tsg`). These tests exercise the rules directly via
//! `LanguageConfiguration::from_sources` so the rules can be developed without
//! touching the live pipeline. Kotlin needs no caller-injected global — the
//! package key is read from each file's `package_header`.

use std::path::PathBuf;

use stack_graphs::graph::StackGraph;
use stack_graphs::partial::PartialPaths;
use stack_graphs::stitching::{
    Database, DatabaseCandidates, ForwardPartialPathStitcher, StitcherConfig,
};
use stack_graphs::NoCancellation;
use tree_sitter_stack_graphs::loader::LanguageConfiguration;
use tree_sitter_stack_graphs::{NoCancellation as TsNoCancel, Variables};

const KOTLIN_TSG: &str = include_str!("../rules/kotlin.tsg");

fn build_config() -> LanguageConfiguration {
    LanguageConfiguration::from_sources(
        tree_sitter_kotlin_ng::LANGUAGE.into(),
        Some("source.kotlin".into()),
        None,
        vec!["kt".into()],
        PathBuf::from("kotlin.tsg"),
        KOTLIN_TSG,
        None,
        None,
        &TsNoCancel,
    )
    .expect("kotlin.tsg must compile into a LanguageConfiguration")
}

/// Build a stack graph for `files`, stitch complete paths, and return
/// `(cross_file, same_file)` resolution counts.
fn resolution_counts(files: &[(&str, &str)]) -> (usize, usize) {
    let config = build_config();
    let mut sg = StackGraph::new();

    let mut file_handles = Vec::new();
    for (path, source) in files {
        let file = sg.add_file(path).expect("add_file");
        // Kotlin needs no globals — the package key comes from the source.
        let globals = Variables::new();
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

    let mut cross_file = 0usize;
    let mut same_file = 0usize;
    for p in all_paths.iter() {
        if p.start_node == p.end_node {
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
fn kotlin_resolves_same_package_cross_file_call() {
    // b.kt calls helper(), defined in a.kt (same package) → cross-file resolve.
    let (cross, _) = resolution_counts(&[
        ("a.kt", "package com.example\n\nfun helper() {}\n"),
        ("b.kt", "package com.example\n\nfun use() { helper() }\n"),
    ]);
    assert!(
        cross >= 1,
        "helper() in b.kt must resolve cross-file to a.kt (same package), got {cross}"
    );
}

#[test]
fn kotlin_different_package_no_cross_file_edge() {
    // helper() is in package `com.a`; the caller is in `com.b`. A bare call in a
    // different package must NOT resolve cross-file (no import modelled).
    let (cross, _) = resolution_counts(&[
        ("a.kt", "package com.a\n\nfun helper() {}\n"),
        ("b.kt", "package com.b\n\nfun use() { helper() }\n"),
    ]);
    assert_eq!(
        cross, 0,
        "a bare helper() in a different package must not resolve cross-file, got {cross}"
    );
}
