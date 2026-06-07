//! Scale benchmarks that generate a synthetic multi-file Rust project in
//! a tempdir and measure indexing time / symbol count / query latency.
//!
//! Run with: `cargo test --release -p coregraph-extractor --test scale_benchmark -- --nocapture`
//!
//! Default `cargo test` (debug) includes these under `#[ignore]` so the
//! normal workspace run stays fast; remove the attribute locally if
//! you want them in the default suite.

use coregraph_extractor::build_graph;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::tempdir;

fn write_rust_file(dir: &Path, idx: usize, symbols_per_file: usize) {
    let mut body = String::with_capacity(symbols_per_file * 80);
    body.push_str(&format!("// Generated module {}\n", idx));
    for i in 0..symbols_per_file {
        body.push_str(&format!(
            "pub struct Type{idx}_{i};\n\
             impl Type{idx}_{i} {{\n\
             \tpub fn new() -> Self {{ Self }}\n\
             \tpub fn process(&self, n: u32) -> u32 {{ n * {i} }}\n\
             }}\n",
            idx = idx,
            i = i
        ));
    }
    fs::write(dir.join(format!("mod_{}.rs", idx)), body).unwrap();
}

fn make_synthetic_repo(file_count: usize, symbols_per_file: usize) -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    for i in 0..file_count {
        write_rust_file(&src, i, symbols_per_file);
    }
    // Minimal Cargo.toml so manifest detection registers a package context.
    fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "synthetic"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::write(
        src.join("lib.rs"),
        (0..file_count)
            .map(|i| format!("pub mod mod_{};\n", i))
            .collect::<String>(),
    )
    .unwrap();
    let root = dir.path().to_path_buf();
    (dir, root)
}

fn run_scale_bench(file_count: usize, symbols_per_file: usize, label: &str) {
    let (_tmp, root) = make_synthetic_repo(file_count, symbols_per_file);

    let t0 = Instant::now();
    let (graph, files) = build_graph(&root).expect("build_graph ok");
    let index_elapsed = t0.elapsed();

    let expected_defs = file_count * symbols_per_file * 3; // struct+new+process
    assert!(
        graph.node_count() >= expected_defs,
        "{} — expected ≥{} symbols, got {}",
        label,
        expected_defs,
        graph.node_count()
    );

    let approx_bytes = graph.node_count() * 256 + graph.edge_count() * 128;

    println!(
        "[{label}] files={files} symbols={sym} edges={edges} \
         index_elapsed={idx:.2?} approx_mem={mem_mb:.1}MB",
        label = label,
        files = files,
        sym = graph.node_count(),
        edges = graph.edge_count(),
        idx = index_elapsed,
        mem_mb = approx_bytes as f64 / (1024.0 * 1024.0),
    );

    // Query latency sample: look up a known symbol 100 times.
    let probe_name = format!("Type0_{}", symbols_per_file / 2);
    let t1 = Instant::now();
    let mut hit = 0usize;
    for _ in 0..100 {
        hit += graph.lookup_by_name(&probe_name, 4).len();
    }
    let query_elapsed = t1.elapsed();
    assert!(hit > 0, "{label} — probe lookup returned no results");

    println!(
        "[{label}] lookup_by_name x100 = {:.2?} total ({:.2?} each)",
        query_elapsed,
        query_elapsed / 100
    );
}

#[test]
#[ignore = "scale benchmark — run with --release and --ignored"]
fn scale_small_500_files() {
    run_scale_bench(500, 3, "500 × 3");
}

#[test]
#[ignore = "scale benchmark — run with --release and --ignored"]
fn scale_medium_1000_files() {
    run_scale_bench(1_000, 5, "1000 × 5");
}

#[test]
#[ignore = "scale benchmark — run with --release and --ignored"]
fn scale_large_5000_files() {
    run_scale_bench(5_000, 4, "5000 × 4");
}
