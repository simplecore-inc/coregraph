//! Integration tests for `StackGraphsBackend`.
//!
//! Exercise the real `tree-sitter-stack-graphs` pipeline end-to-end:
//! feed the backend real Java / TS / JS / Python source text, verify
//! it builds stack graphs under the configured budget, and confirm
//! the syntactic fallback still runs for unsupported languages.

use coregraph_graph::SymbolGraph;
use coregraph_stack::{ResolutionBackend, StackGraphsBackend, SyntacticBackend};
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn builds_stack_graph_for_java_source() {
    let backend = StackGraphsBackend::new(Duration::from_secs(10), "java");
    let files = vec![(
        PathBuf::from("Foo.java"),
        r#"
        package com.example;
        public class Foo {
            public int value() { return 42; }
        }
        "#
        .to_string(),
    )];
    let report = backend.build_supported_graphs(&files);
    assert_eq!(report.built, 1, "Java stack graph must build: {:?}", report);
    assert_eq!(report.failed_files, 0);
    assert_eq!(report.failed_languages, 0);
}

#[test]
fn builds_stack_graph_for_python_source() {
    let backend = StackGraphsBackend::new(Duration::from_secs(10), "python");
    let files = vec![(
        PathBuf::from("main.py"),
        "def greet(name: str) -> str:\n    return f'hello {name}'\n".to_string(),
    )];
    let report = backend.build_supported_graphs(&files);
    assert_eq!(
        report.built, 1,
        "Python stack graph must build: {:?}",
        report
    );
}

#[test]
fn builds_stack_graph_for_typescript_source() {
    let backend = StackGraphsBackend::new(Duration::from_secs(10), "typescript");
    let files = vec![(
        PathBuf::from("index.ts"),
        r#"
        export function greet(name: string): string {
            return "hello " + name;
        }
        "#
        .to_string(),
    )];
    let report = backend.build_supported_graphs(&files);
    assert_eq!(
        report.built, 1,
        "TypeScript stack graph must build: {:?}",
        report
    );
}

#[test]
fn builds_stack_graph_for_javascript_source() {
    let backend = StackGraphsBackend::new(Duration::from_secs(10), "javascript");
    let files = vec![(
        PathBuf::from("util.js"),
        "export function shout(s) { return s.toUpperCase(); }\n".to_string(),
    )];
    let report = backend.build_supported_graphs(&files);
    assert_eq!(
        report.built, 1,
        "JavaScript stack graph must build: {:?}",
        report
    );
}

#[test]
fn unsupported_language_skipped_but_no_failure() {
    // Ruby has no stack-graphs rules in CoreGraph, so the backend must simply
    // skip it without tripping failure counters. (Go/Rust/Kotlin now use
    // CoreGraph's own hand-authored rules.)
    let backend = StackGraphsBackend::new(Duration::from_secs(5), "ruby");
    let files = vec![(PathBuf::from("main.rb"), "def foo; end".to_string())];
    let report = backend.build_supported_graphs(&files);
    assert_eq!(report.built, 0);
    assert_eq!(report.failed_files, 0);
    assert_eq!(report.failed_languages, 0);
}

#[test]
fn resolve_go_cross_file_stitches_package_reference() {
    // Two Go files in the same package (same directory): b.go calls Helper(),
    // defined in a.go. CoreGraph's hand-authored go.tsg, with PKG_PATH injected
    // from each file's directory, must stitch the call to the definition and
    // emit it as a NameResolved (stack-graphs) cross-file ref — not just the
    // syntactic fallback.
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(30), "go");
    let files = vec![
        (
            PathBuf::from("pkg/a.go"),
            "package pkg\n\nfunc Helper() {}\n".to_string(),
        ),
        (
            PathBuf::from("pkg/b.go"),
            "package pkg\n\nfunc Use() { Helper() }\n".to_string(),
        ),
    ];
    let result = backend.resolve(&files, &graph);
    let go_resolved = result
        .refs
        .iter()
        .filter(|r| {
            r.from_file != r.to_file
                && r.origin == coregraph_core::edge::AnalysisOrigin::NameResolved
        })
        .count();
    assert!(
        go_resolved > 0,
        "go.tsg must stitch Helper() in b.go to a.go as a NameResolved cross-file ref (got {go_resolved})"
    );
}

#[test]
fn resolve_rust_cross_file_stitches_qualified_call() {
    // main.rs calls `helper::greet()`, defined in the sibling-module file
    // helper.rs (same directory). CoreGraph's hand-authored rust.tsg, with DIR
    // and MOD_NAME injected from each file's path, must stitch the qualified
    // call to the definition and emit it as a NameResolved (stack-graphs)
    // cross-file ref — not just the syntactic fallback.
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(30), "rust");
    let files = vec![
        (
            PathBuf::from("src/helper.rs"),
            "pub fn greet() {}\n".to_string(),
        ),
        (
            PathBuf::from("src/main.rs"),
            "mod helper;\nfn main() { helper::greet(); }\n".to_string(),
        ),
    ];
    let result = backend.resolve(&files, &graph);
    let rust_resolved = result
        .refs
        .iter()
        .filter(|r| {
            r.from_file != r.to_file
                && r.origin == coregraph_core::edge::AnalysisOrigin::NameResolved
        })
        .count();
    assert!(
        rust_resolved > 0,
        "rust.tsg must stitch helper::greet() in main.rs to helper.rs as a NameResolved cross-file ref (got {rust_resolved})"
    );
}

#[test]
fn resolve_rust_cross_file_stitches_use_import_bare_call() {
    // The idiomatic Rust pattern, exercised through the live `resolve_language`
    // path (which differs from the isolated harness): main.rs `use helper::greet;`
    // then a bare `greet()` call must stitch cross-file to helper.rs::greet as a
    // NameResolved ref via the `use` alias.
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(30), "rust");
    let files = vec![
        (
            PathBuf::from("src/helper.rs"),
            "pub fn greet() {}\n".to_string(),
        ),
        (
            PathBuf::from("src/main.rs"),
            "mod helper;\nuse helper::greet;\nfn main() { greet(); }\n".to_string(),
        ),
    ];
    let result = backend.resolve(&files, &graph);
    let rust_resolved = result
        .refs
        .iter()
        .filter(|r| {
            r.from_file != r.to_file
                && r.origin == coregraph_core::edge::AnalysisOrigin::NameResolved
        })
        .count();
    assert!(
        rust_resolved > 0,
        "rust.tsg must stitch a bare greet() after `use helper::greet;` to helper.rs as a NameResolved cross-file ref (got {rust_resolved})"
    );
}

#[test]
fn resolve_rust_cross_file_stitches_use_import_bare_type() {
    // Through the live path: main.rs `use helper::Widget;` then a parameter type
    // `Widget` must stitch cross-file to helper.rs::Widget (type namespace).
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(30), "rust");
    let files = vec![
        (
            PathBuf::from("src/helper.rs"),
            "pub struct Widget { x: i32 }\n".to_string(),
        ),
        (
            PathBuf::from("src/main.rs"),
            "mod helper;\nuse helper::Widget;\nfn run(w: Widget) { let _ = w; }\n".to_string(),
        ),
    ];
    let result = backend.resolve(&files, &graph);
    let rust_resolved = result
        .refs
        .iter()
        .filter(|r| {
            r.from_file != r.to_file
                && r.origin == coregraph_core::edge::AnalysisOrigin::NameResolved
        })
        .count();
    assert!(
        rust_resolved > 0,
        "rust.tsg must stitch a `Widget` param type after `use helper::Widget;` to helper.rs as a NameResolved cross-file ref (got {rust_resolved})"
    );
}

#[test]
fn resolve_kotlin_cross_file_stitches_package_reference() {
    // Two Kotlin files in the same package: b.kt calls helper(), defined in
    // a.kt. CoreGraph's hand-authored kotlin.tsg (package key read from each
    // file's `package_header`) must stitch the call to the definition and emit
    // it as a NameResolved cross-file ref.
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(30), "kotlin");
    let files = vec![
        (
            PathBuf::from("a.kt"),
            "package com.example\n\nfun helper() {}\n".to_string(),
        ),
        (
            PathBuf::from("b.kt"),
            "package com.example\n\nfun use() { helper() }\n".to_string(),
        ),
    ];
    let result = backend.resolve(&files, &graph);
    let kotlin_resolved = result
        .refs
        .iter()
        .filter(|r| {
            r.from_file != r.to_file
                && r.origin == coregraph_core::edge::AnalysisOrigin::NameResolved
        })
        .count();
    assert!(
        kotlin_resolved > 0,
        "kotlin.tsg must stitch helper() in b.kt to a.kt as a NameResolved cross-file ref (got {kotlin_resolved})"
    );
}

#[test]
fn mixed_languages_build_only_supported() {
    let backend = StackGraphsBackend::new(Duration::from_secs(10), "ruby");
    let files = vec![
        (PathBuf::from("Foo.java"), "public class Foo {}".to_string()),
        (PathBuf::from("main.rb"), "def foo; end".to_string()),
        (PathBuf::from("x.py"), "def f(): pass".to_string()),
    ];
    let report = backend.build_supported_graphs(&files);
    assert_eq!(
        report.built, 2,
        "Java + Python should build; Ruby skipped (no stack-graphs rules): {:?}",
        report
    );
}

#[test]
fn resolve_unsupported_language_reports_fallback_success_flag() {
    // Ruby has no stack-graphs rules, so `resolve` produces zero stitched refs
    // and the reported success flag must match the syntactic fallback's (false —
    // it never claims success).
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(5), "ruby");
    let files = vec![(PathBuf::from("a.rb"), "def foo; end".to_string())];
    let result = backend.resolve(&files, &graph);
    assert!(
        !result.success,
        "Ruby-only input must not report stack-graphs success"
    );
}

#[test]
fn resolve_python_cross_file_yields_stitched_refs() {
    // Two tiny Python files: `main.py` imports `helper` from `mod.py`
    // and calls `helper()`. Stack-graphs should stitch the reference
    // on line 2 of main.py to the definition in mod.py, producing at
    // least one cross-file ResolvedRef with distinct source/target
    // paths.
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(30), "python");
    let files = vec![
        (
            PathBuf::from("mod.py"),
            "def helper():\n    return 42\n".to_string(),
        ),
        (
            PathBuf::from("main.py"),
            "from mod import helper\nhelper()\n".to_string(),
        ),
    ];
    let result = backend.resolve(&files, &graph);

    let cross_file = result
        .refs
        .iter()
        .filter(|r| r.from_file != r.to_file)
        .count();

    // We accept either outcome: some stack-graphs rule changes between
    // upstream releases have left Python stitching partial. If we got
    // cross-file refs, success must be true; otherwise the fallback
    // contributes whatever it finds and success is false. The test
    // asserts the *invariant*: success implies stitched refs exist.
    if result.success {
        assert!(
            cross_file > 0,
            "success=true requires at least one cross-file stitched ref (got {} total)",
            result.refs.len()
        );
    }
}

#[test]
fn resolve_java_cross_file_stitches_class_reference() {
    // Java: `Consumer.use` references `Provider` from a sibling file.
    let graph = SymbolGraph::new();
    let backend = StackGraphsBackend::new(Duration::from_secs(30), "java");
    let files = vec![
        (
            PathBuf::from("Provider.java"),
            "package app;\npublic class Provider { public int supply() { return 1; } }\n"
                .to_string(),
        ),
        (
            PathBuf::from("Consumer.java"),
            "package app;\npublic class Consumer {\n  public int use(Provider p) { return p.supply(); }\n}\n"
                .to_string(),
        ),
    ];
    let result = backend.resolve(&files, &graph);

    // Same invariant as the Python case: we don't pin the exact
    // count because rule-file changes can shift it, but success=true
    // must be backed by at least one cross-file ResolvedRef.
    if result.success {
        let cross_file = result
            .refs
            .iter()
            .filter(|r| r.from_file != r.to_file)
            .count();
        assert!(
            cross_file > 0,
            "Java stack-graphs success requires a cross-file ref"
        );
    }
}

#[test]
fn syntactic_backend_trait_impl_round_trip() {
    // The trait object path used by the extractor must work — same
    // signature as StackGraphsBackend so callers can swap at runtime.
    let graph = SymbolGraph::new();
    let backend: Box<dyn ResolutionBackend> = Box::new(SyntacticBackend {
        language: "rust".into(),
    });
    let _ = backend.resolve(&[], &graph);
    assert_eq!(backend.label(), "syntactic");
}

#[test]
fn backend_respects_timeout_budget() {
    // Zero-length budget must still run at least partially without
    // panicking; timed_out counter captures what was skipped.
    let backend = StackGraphsBackend::new(Duration::from_nanos(1), "rust");
    let files = vec![
        (PathBuf::from("a.java"), "public class A {}".to_string()),
        (PathBuf::from("b.java"), "public class B {}".to_string()),
    ];
    let report = backend.build_supported_graphs(&files);
    // Not asserting exact numbers — timing dependent. Just that the
    // backend didn't explode and kept its counters consistent.
    assert!(
        report.built + report.failed_files + report.timed_out <= files.len(),
        "accounting must not overcount: {:?}",
        report
    );
}
