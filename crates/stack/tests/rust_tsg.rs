//! Isolated cross-file resolution tests for the hand-authored Rust `.tsg`.
//!
//! Rust is NOT wired into the production backend yet (see
//! `crates/stack/rules/rust.tsg`). These tests exercise the rules directly via
//! `LanguageConfiguration::from_sources`, injecting the SAME `(DIR, MOD_NAME)`
//! globals production will use — `coregraph_stack::rust_module_globals` — so the
//! rules can be developed without touching the live pipeline while guaranteeing
//! the test and production derivations never drift (a globals mismatch between
//! test and live is what bit the Go transition).

use std::path::{Path, PathBuf};

use coregraph_stack::rust_module_globals;
use stack_graphs::graph::StackGraph;
use stack_graphs::partial::PartialPaths;
use stack_graphs::stitching::{
    Database, DatabaseCandidates, ForwardPartialPathStitcher, StitcherConfig,
};
use stack_graphs::NoCancellation;
use tree_sitter_stack_graphs::loader::LanguageConfiguration;
use tree_sitter_stack_graphs::{NoCancellation as TsNoCancel, Variables};

const RUST_TSG: &str = include_str!("../rules/rust.tsg");

fn build_config() -> LanguageConfiguration {
    LanguageConfiguration::from_sources(
        tree_sitter_rust::LANGUAGE.into(),
        Some("source.rust".into()),
        None,
        vec!["rs".into()],
        PathBuf::from("rust.tsg"),
        RUST_TSG,
        None,
        None,
        // from_sources wants tree_sitter_stack_graphs' cancellation flag here;
        // the stitcher phases below use stack_graphs' own NoCancellation.
        &TsNoCancel,
    )
    .expect("rust.tsg must compile into a LanguageConfiguration")
}

/// Build a stack graph for `files`, injecting each file's `(DIR, MOD_NAME)`
/// globals exactly as production does (via `rust_module_globals`), stitch
/// complete paths, and return `(cross_file, same_file)` resolution counts.
fn resolution_counts(files: &[(&str, &str)]) -> (usize, usize) {
    let config = build_config();
    let mut sg = StackGraph::new();

    let mut file_handles = Vec::new();
    for (path, source) in files {
        let file = sg.add_file(path).expect("add_file");
        let (dir, mod_name) = rust_module_globals(Path::new(path));
        let mut globals = Variables::new();
        globals
            .add("DIR".into(), dir.into())
            .expect("set DIR global");
        globals
            .add("MOD_NAME".into(), mod_name.into())
            .expect("set MOD_NAME global");
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

    // Consumer-side shadowing (mirrors the production `resolve_language` path):
    // the stitcher returns EVERY complete path and never auto-suppresses a
    // higher-precedence inner binding that hides a lower-precedence outer one.
    // `PartialPath::shadows` walks two paths' edges and reports when a
    // precedence-1 path (a function-local binding) shadows a precedence-0 one
    // (the module / `use` fall-through) for the same reference; we drop the
    // shadowed path so a `let`/param correctly hides an imported or module name.
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
fn rust_resolves_sibling_module_qualified_call() {
    // main.rs calls `helper::greet()`, defined in the sibling-module file
    // helper.rs (same directory). The qualified path must resolve cross-file.
    let (cross, _) = resolution_counts(&[
        ("src/helper.rs", "pub fn greet() {}\n"),
        (
            "src/main.rs",
            "mod helper;\nfn main() { helper::greet(); }\n",
        ),
    ]);
    assert!(
        cross >= 1,
        "helper::greet() in main.rs must resolve cross-file to helper.rs, got {cross}"
    );
}

#[test]
fn rust_qualified_call_is_directory_anchored() {
    // helper.rs lives in `a/`, the caller in `b/`. Even though the module name
    // `helper` matches by text, the directory anchor differs, so there must be
    // NO cross-file edge. This is what prevents two unrelated `helper.rs` files
    // (different crates / directories) from being falsely merged.
    let (cross, _) = resolution_counts(&[
        ("a/helper.rs", "pub fn greet() {}\n"),
        ("b/main.rs", "mod helper;\nfn main() { helper::greet(); }\n"),
    ]);
    assert_eq!(
        cross, 0,
        "a `helper::greet()` call in b/ must not bind to a/helper.rs \
         (resolution is directory-anchored), got {cross}"
    );
}

#[test]
fn rust_resolves_use_import_then_bare_call() {
    // main.rs `use helper::greet;` then calls bare `greet()`. The bare call must
    // resolve through the `use` alias cross-file to helper.rs::greet — this is
    // the idiomatic Rust pattern (import at top, bare name in the body).
    let (cross, _) = resolution_counts(&[
        ("src/helper.rs", "pub fn greet() {}\n"),
        (
            "src/main.rs",
            "mod helper;\nuse helper::greet;\nfn main() { greet(); }\n",
        ),
    ]);
    assert!(
        cross >= 1,
        "a bare greet() after `use helper::greet;` must resolve cross-file to helper.rs, got {cross}"
    );
}

#[test]
fn rust_resolves_use_alias_then_bare_call() {
    // `use helper::greet as g;` then `g()` must resolve cross-file to helper.rs
    // via the aliased local name.
    let (cross, _) = resolution_counts(&[
        ("src/helper.rs", "pub fn greet() {}\n"),
        (
            "src/main.rs",
            "mod helper;\nuse helper::greet as g;\nfn main() { g(); }\n",
        ),
    ]);
    assert!(
        cross >= 1,
        "a bare g() after `use helper::greet as g;` must resolve cross-file, got {cross}"
    );
}

#[test]
fn rust_local_binding_shadows_use_import_no_cross_file_edge() {
    // main.rs imports `greet` but also binds a function-local `let greet = …`,
    // then calls `greet()`. With consumer-side shadowing the call resolves to
    // the LOCAL only — there must be NO cross-file edge to helper.rs::greet.
    let (cross, same) = resolution_counts(&[
        ("src/helper.rs", "pub fn greet() {}\n"),
        (
            "src/main.rs",
            "mod helper;\nuse helper::greet;\nfn main() { let greet = || {}; greet(); }\n",
        ),
    ]);
    assert_eq!(
        cross, 0,
        "a function-local `let greet` must shadow the `use helper::greet` import \
         — expected 0 cross-file resolutions, got cross={cross}"
    );
    assert!(
        same >= 1,
        "the call must still resolve to the same-file local greet, got same={same}"
    );
}

#[test]
fn rust_param_shadows_use_import_no_cross_file_edge() {
    // A parameter named `greet` shadows the `use helper::greet` import for calls
    // inside the function body — no false cross-file edge.
    let (cross, _) = resolution_counts(&[
        ("src/helper.rs", "pub fn greet() {}\n"),
        (
            "src/main.rs",
            "mod helper;\nuse helper::greet;\nfn run(greet: fn()) { greet(); }\n",
        ),
    ]);
    assert_eq!(
        cross, 0,
        "a parameter `greet` must shadow the imported greet — expected 0 \
         cross-file resolutions, got cross={cross}"
    );
}

#[test]
fn rust_resolves_use_import_then_bare_type() {
    // `use helper::Widget;` then a parameter type `Widget` must resolve
    // cross-file to helper.rs::Widget — the idiomatic type-import pattern.
    let (cross, _) = resolution_counts(&[
        ("src/helper.rs", "pub struct Widget { x: i32 }\n"),
        (
            "src/main.rs",
            "mod helper;\nuse helper::Widget;\nfn run(w: Widget) { let _ = w; }\n",
        ),
    ]);
    assert!(
        cross >= 1,
        "a parameter type `Widget` after `use helper::Widget;` must resolve cross-file, got {cross}"
    );
}

#[test]
fn rust_resolves_qualified_type_path() {
    // A parameter type written as a qualified path `helper::Widget` must resolve
    // cross-file without a `use`.
    let (cross, _) = resolution_counts(&[
        ("src/helper.rs", "pub struct Widget { x: i32 }\n"),
        (
            "src/main.rs",
            "mod helper;\nfn run(w: helper::Widget) { let _ = w; }\n",
        ),
    ]);
    assert!(
        cross >= 1,
        "a parameter type `helper::Widget` must resolve cross-file to helper.rs, got {cross}"
    );
}

#[test]
fn rust_value_and_type_namespaces_do_not_collide() {
    // helper.rs has BOTH `fn thing` (value) and `struct thing { … }` (type). A
    // call `helper::thing()` must resolve to ONLY the function via the "%value"
    // marker — not also the struct. Without the namespace discriminator this
    // would produce two cross-file edges.
    let (cross, _) = resolution_counts(&[
        (
            "src/helper.rs",
            "pub fn thing() {}\npub struct thing { x: i32 }\n",
        ),
        (
            "src/main.rs",
            "mod helper;\nfn main() { helper::thing(); }\n",
        ),
    ]);
    assert_eq!(
        cross, 1,
        "helper::thing() must resolve to exactly the value def, not also the same-named struct, got {cross}"
    );
}

#[test]
fn rust_qualified_call_unknown_module_no_edge() {
    // The caller references `missing::greet()` but there is no sibling module
    // `missing` — the reference must simply not resolve (no false edge), leaving
    // it to the syntactic 0.85 layer in production.
    let (cross, _) = resolution_counts(&[
        ("src/helper.rs", "pub fn greet() {}\n"),
        ("src/main.rs", "fn main() { missing::greet(); }\n"),
    ]);
    assert_eq!(
        cross, 0,
        "a call to a non-existent sibling module must not resolve, got {cross}"
    );
}
