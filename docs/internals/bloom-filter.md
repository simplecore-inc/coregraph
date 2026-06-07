# Appendix: Per-File Symbol Bloom Filter

CoreGraph keeps a small Bloom filter for every indexed file so it can answer one
cheap question without scanning the graph:

> Does this file define a symbol named `X`?

A Bloom filter is a probabilistic set. It can give a false positive ("maybe
present") but never a false negative — so a `false` answer is definitive. That
makes it ideal for fast negative lookups: if the filter says a name is absent
from a file, you can skip the expensive scan entirely.

The design is inspired by the Bloom-filter symbol index in
[Metals](https://scalameta.org/metals/) (the Scala language server).

## What it actually does

Source: `crates/graph/src/bloom.rs` (the `SymbolBloom` type) and
`crates/graph/src/symbol_graph.rs` (the per-file map and lookup helper).

| Property | Value |
|---|---|
| Granularity | One filter per file path |
| Stored items | Symbol **names** defined in that file (not references, not sub-strings) |
| Sizing | `Bloom::new_for_fp_rate(100_000, 0.01)` — 100,000 items at a 1% false-positive rate |
| `false` result | Definitive: the file does not define that name |
| `true` result | Possibly present — confirm with a real scan (`nodes_in_file`) |

The map lives on `SymbolGraph`:

```rust
// crates/graph/src/symbol_graph.rs
/// Per-file bloom filter: "does this file define a symbol named X?"
file_blooms: HashMap<PathBuf, SymbolBloom>,
```

Every node insertion updates the owning file's filter. From
`record_indexes`, called by `insert_node`:

```rust
self.file_blooms
    .entry(node.file.clone())
    .or_default()
    .insert(&node.name);
```

So the index is maintained automatically as the graph is built; there is no
separate "build the Bloom index" step.

## API

`SymbolBloom` (`crates/graph/src/bloom.rs`):

```rust
let mut bloom = SymbolBloom::new();      // sized for 100k items @ 1% FPR
bloom.insert("com.example.UserService");
bloom.might_contain("com.example.UserService"); // true  (possibly present)
bloom.might_contain("definitely.not.present");  // false (definitely absent)
```

`SymbolGraph` exposes the per-file lookup and an introspection counter:

```rust
// false is definitive; true means "fall through to nodes_in_file to confirm".
graph.file_might_define(path, name) -> bool;

// number of files with a tracked filter
graph.file_bloom_count() -> usize;
```

## Status

The per-file index is populated on every node insert and survives the
fast-path (incremental) update — it is additive-only and is not cleared on a
partial rebuild. The `file_might_define` helper is in place as the intended
short-circuit before the `nodes_in_file` scan, but no user-facing query path
calls it yet; today its callers are the graph's own tests. So treat this as an
internal index whose negative-lookup helper exists and is correct, not as a
feature that currently speeds up `query`/`impact`.

### Clone caveat

`bloomfilter::Bloom` does not implement `Clone`, so `SymbolBloom::clone` returns
a fresh empty filter rather than copying the bitmap. This lets `SymbolGraph`
derive `Clone` cheaply; a cloned graph must re-index to repopulate its filters.

---
[Back to docs](../README.md)
