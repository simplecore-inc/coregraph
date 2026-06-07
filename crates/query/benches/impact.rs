//! Impact / orphan benchmarks against a real graph.
//!
//! Tracks the cost of the two heaviest analysis routines as the
//! semantic edge filter and the ExternalPackage trimming evolve.
//! Builds the extractor crate's graph once outside the timed loop
//! so we measure pure query work, not the build step.

use std::path::PathBuf;
use std::sync::OnceLock;

use coregraph_extractor::build_graph;
use coregraph_graph::SymbolGraph;
use coregraph_query::{compute_impact, find_orphans};
use criterion::{criterion_group, criterion_main, Criterion};

fn fixture_graph() -> &'static SymbolGraph {
    // OnceLock so the (~6s) build runs once per benchmark binary
    // rather than once per `b.iter` invocation. Criterion samples
    // hundreds of times — without caching the build dominates.
    static GRAPH: OnceLock<SymbolGraph> = OnceLock::new();
    GRAPH.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("extractor");
        let (g, _) = build_graph(&root).expect("build");
        g
    })
}

fn bench_orphans(c: &mut Criterion) {
    let g = fixture_graph();
    c.bench_function("query/find_orphans", |b| {
        b.iter(|| std::hint::black_box(find_orphans(g)))
    });
}

fn bench_impact(c: &mut Criterion) {
    let g = fixture_graph();
    // Pick a real symbol — `build_graph` is the most-connected node
    // in this fixture, so it stresses the BFS bound.
    let seed = g
        .nodes()
        .find(|n| n.name == "build_graph")
        .expect("build_graph symbol must exist in the fixture");
    let mut group = c.benchmark_group("query/compute_impact");
    for depth in [1usize, 3, 5] {
        group.bench_with_input(format!("depth={}", depth), &depth, |b, &d| {
            b.iter(|| std::hint::black_box(compute_impact(g, seed.id, d)))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_orphans, bench_impact);
criterion_main!(benches);
