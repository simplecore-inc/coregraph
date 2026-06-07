//! Build-graph throughput benchmarks.
//!
//! Measures `coregraph_extractor::build_graph` against a fixture tree
//! that exercises the full pipeline: tree-sitter extraction across
//! every supported language, stack-graphs resolution, and the
//! string-match mediators. The benchmark uses the extractor crate's
//! own source as input — large enough to be representative but
//! self-contained so the suite has no external dependencies.

use std::path::PathBuf;

use coregraph_extractor::build_graph;
use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn fixture_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` resolves to `crates/extractor` at compile
    // time. We benchmark that directory itself — a stable, multi-file,
    // multi-language tree (Rust + tree-sitter queries) that ships
    // with the repo.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bench_build_graph(c: &mut Criterion) {
    let root = fixture_root();
    // Pre-warm so filesystem caches don't penalise the first sample.
    let (warmup, file_count) = build_graph(&root).expect("warmup build");
    eprintln!(
        "extractor crate fixture: {} files, {} symbols, {} edges",
        file_count,
        warmup.node_count(),
        warmup.edge_count()
    );

    let mut group = c.benchmark_group("build_graph/extractor-crate");
    // `bytes` throughput so criterion reports MB/s, which travels
    // better across machines than absolute timings.
    let total_bytes = total_source_bytes(&root);
    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.sample_size(20);
    group.bench_function("cold", |b| {
        b.iter(|| {
            let (graph, _) = build_graph(&root).expect("build");
            std::hint::black_box(graph)
        })
    });
    group.finish();
}

fn total_source_bytes(root: &std::path::Path) -> usize {
    let mut total = 0usize;
    for entry in ignore::WalkBuilder::new(root).build().flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total += meta.len() as usize;
            }
        }
    }
    total
}

criterion_group!(benches, bench_build_graph);
criterion_main!(benches);
