# Schema Versioning Policy

CoreGraph serializes the same core types (`SymbolNode`, `DirectEdge`,
`EdgeKind`, `SymbolKind`, `AnalysisOrigin`, `TrustModel`) across three
surfaces:

- **Snapshots** — the binary `.coregraph/snapshot.bin` (bincode).
- **JSON output** — `--output-format json` (including the `--output-format json` summary line of `stats`).
- **Bridge responses** — IPC (daemon) and the HTTP bridge.

Those types live in `coregraph-core` and are imported by every downstream
crate, so a careless rename ripples through the whole workspace and — worse —
can silently break snapshots written by an older binary. This document is the
playbook for evolving them without that happening.

There are **two independent compatibility mechanisms**, and they govern
different surfaces. Read §0 first — confusing them is the usual mistake.

---

## 0. Two mechanisms, two scopes

| Mechanism | Granularity | Governs | On mismatch |
|---|---|---|---|
| **Snapshot version gate** | Whole file | `.coregraph/snapshot.bin` only | Reject + rebuild from source |
| **serde `alias` / `default`** | Per field/variant | JSON (IPC, HTTP, `--output-format json`) and same-version snapshot loads | Tolerate silently |

The snapshot file carries a coarse version number (§4). On load,
`load_snapshot` checks `on_disk_version != SNAPSHOT_SCHEMA_VERSION` **before**
bincode touches the body. If they differ, it bails — the body is never
deserialized, so serde aliases and defaults never get a chance to run. A
cross-version snapshot is discarded and rebuilt, not patched.

serde `alias` / `default` (§2) is the fine-grained mechanism. Its real
beneficiaries are the JSON surfaces, which have **no** version gate: an old IPC
or HTTP client, or a hand-written test fixture, can send a payload using a
legacy field name and it still deserializes. Aliases also matter for loading a
snapshot written at the *same* schema version that happens to predate a
field-name change within that version.

Rule of thumb: bump the snapshot version for a *structural* change (a new node
or edge kind that old snapshots simply lack); use serde `alias` / `default` for
a *naming or additive* change that old JSON payloads should still satisfy.

---

## 1. Scope

The serde rules in §2 apply to any type that is
`#[derive(Serialize, Deserialize)]` and exposed through:

- `save_snapshot` / `load_snapshot` (bincode body, magic + version header — see §4)
- any `--output-format json` output (including the `--output-format json` summary line of `coregraph stats`)
- IPC request/response bodies (JSON)
- HTTP bridge responses (JSON)

Anything `#[cfg(test)]`-only or purely in-process (e.g. `HashMap` keys) is out
of scope.

---

## 2. serde rules

### 2.1 Enum variant renames

When renaming a variant, keep the old name as a serde alias. This is the real
`AnalysisOrigin` enum from `crates/core/src/edge.rs`:

```rust
#[derive(Serialize, Deserialize)]
pub enum AnalysisOrigin {
    CompilerDerived,
    #[serde(alias = "Resolved")]       // legacy name in old snapshots
    NameResolved,
    SyntaxMatched,
    PatternMatched,
    #[serde(alias = "Asserted")]       // legacy name in old snapshots
    ConventionInferred,
    Dynamic,
}
```

- `alias` is deserialize-only, so **reading** old data works.
- New writes serialize under the new name, so **writers** converge on the
  canonical form.
- Never remove the alias in the same release as the rename. Alias removal is
  its own breaking change; schedule at least one minor version of overlap.

### 2.2 Adding a new field

Default the new field so payloads that lack it still deserialize. The real
`DirectEdge` (from `crates/core/src/edge.rs`) grew two such fields:

```rust
pub struct DirectEdge {
    pub from: SymbolId,
    pub to: SymbolId,
    pub kind: EdgeKind,
    #[serde(alias = "trust")]          // see §2.3
    pub origin: AnalysisOrigin,
    pub confidence: Confidence,
    pub evidence_file: PathBuf,

    #[serde(default)]                  // appended; absent in legacy snapshots
    pub created_at_epoch: u64,

    #[serde(default)]                  // appended; absent in legacy snapshots
    pub stale_evidence_count: u32,
}
```

- `#[serde(default)]` requires `Default` on the field type. Primitives,
  `Option`, `Vec`, `HashMap` already have it.
- Keep the wire name equal to the Rust field name unless §2.3 applies.

### 2.3 Field renames

Keep both names with `#[serde(rename = "new", alias = "old")]`. The real
`DirectEdge::origin` field was once serialized as `trust`:

```rust
pub struct DirectEdge {
    // Old snapshots wrote this as `trust`; new code calls it `origin`.
    #[serde(alias = "trust")]
    pub origin: AnalysisOrigin,
    // ...
}
```

- The canonical serialized name is the Rust field name.
- `alias` reads old data; writers emit the new name.

### 2.4 Removing a field

Do **not** remove a field in the same release that also renames something else.
One breaking change per version. When removing:

1. Release N: mark the field `#[serde(default, skip_serializing)]`. It still
   deserializes from old data (then ignored) but disappears from new writes.
2. Release N+1 (a minor later): remove the field declaration entirely.

### 2.5 Enum representation

All user-visible enums use the default (externally-tagged) serde
representation. Do **not** introduce `#[serde(untagged)]` or
`#[serde(tag = "...", content = "...")]` on a type that was previously
externally tagged — that is a breaking wire change with no safe alias.

### 2.6 bincode format

bincode is used for snapshots. It is **order-sensitive** for struct fields: a
field reorder is a breaking change even if serde names match. Always append new
fields at the end of the struct (as both new `DirectEdge` fields were).

---

## 3. Checklist for a serialized-type change

Before landing a change that touches a serialized type:

- [ ] Kept the old name via `#[serde(alias = "…")]` on the variant/field?
- [ ] Added `#[serde(default)]` on every new field?
- [ ] Appended (not inserted) new fields in structs (bincode is positional)?
- [ ] Bumped `SNAPSHOT_SCHEMA_VERSION` if the change is structural (a new node
      or edge kind that an old snapshot cannot contain)?
- [ ] Kept the serde roundtrip + legacy-payload tests passing
      (`crates/core/src/edge.rs` and `crates/core/src/symbol.rs` each have an
      inline `*_serde_roundtrip` test; legacy-JSON cases live alongside them)?
- [ ] Kept the snapshot header tests passing — `crates/graph/src/snapshot.rs`
      has inline `load_rejects_wrong_schema_version`, `load_rejects_bad_magic`,
      and `load_rejects_truncated_header`?
- [ ] Bumped the CLI `--version` if the change is user-observable?

---

## 4. Snapshot file format and version history

A snapshot is written by `save_snapshot` (`crates/graph/src/snapshot.rs`) with
this layout:

```
bytes 0..4   "CGRH"                       magic — identifies a CoreGraph snapshot
bytes 4..8   SNAPSHOT_SCHEMA_VERSION       u32, little-endian
bytes 8..    bincode-encoded GraphSnapshot the graph body
```

`load_snapshot` validates the magic and the version **before** bincode reads
the body, so a stale or foreign file produces a clear message instead of an
opaque bincode panic. The current version is **6**.

| Schema | Change |
|---|---|
| v1 | Original layout. |
| v2 | Removed the external compiler-index promotion layer. |
| v3 | Added the documentation layer: `DocComment` nodes and `Documents` edges. |
| v4 | Added `Mentions` edges (intra-doc links in doc text). |
| v5 | Added the external-docs layer: `DocSection` nodes and `DescribedIn` edges (Markdown ingestion). |
| v6 | Added the `built_at` field (wall-clock time the graph reflected its source). Enables the daemon to validate a warm-loaded snapshot against source mtimes and rebuild if stale. A structural (bincode-layout) change, so v5 files are rejected and rebuilt. |

### Any version mismatch is rejected and rebuilt

A snapshot whose recorded version differs from the running binary's
`SNAPSHOT_SCHEMA_VERSION` is **discarded and rebuilt from source** — this is a
deliberate exception to the serde-compatibility rules in §2. The check is a
plain inequality, so it covers every cross-version case, not just v1.

```
snapshot .coregraph/snapshot.bin uses schema v2, but this build only reads v6;
rebuild with `coregraph index --snapshot`
```

In practice the rebuild is automatic — a thin-client command that finds a
mismatched snapshot reindexes from source rather than loading partial state.

Why a hard reject instead of an alias? Each version bump added structure that an
older snapshot simply does not contain. The clearest case is the v1 → v2 jump:
v1 snapshots may carry `CompilerDerived` `Resolves` edges that were promoted
from an external compiler index which no longer exists in the pipeline. Because
that producing source is gone, those edges cannot be re-validated, and no serde
alias or default recovers their meaning — so the snapshot is discarded and
rebuilt rather than partially trusted. Cross-file resolution is now produced
entirely by tree-sitter + stack-graphs (`NameResolved`), with syntactic
fallback (`SyntaxMatched`); `CompilerDerived` survives only for
structurally-certain extractor-observed `Contains` / `BelongsTo` edges. The
later bumps (v3–v6) follow the same logic: an older snapshot is missing nodes,
edges, or fields the new pipeline expects, so a clean rebuild is the only correct
outcome.

---

## 5. Current serde aliases and defaulted fields (as of v0.1)

These keep older JSON payloads and same-version snapshot loads working without a
version bump.

| Type | Field / variant | Compatibility |
|---|---|---|
| `AnalysisOrigin` | `NameResolved` | deserialize alias `Resolved` |
| `AnalysisOrigin` | `ConventionInferred` | deserialize alias `Asserted` |
| `DirectEdge` | `origin` | deserialize alias `trust` |
| `DirectEdge` | `created_at_epoch` | added, defaults to `0` |
| `DirectEdge` | `stale_evidence_count` | added, defaults to `0` |
| `SymbolNode` | `qualified_name` | added, defaults to the symbol `name` (empty after default deserialize) |
| `SymbolNode` | `status` | added, defaults to `Verified` |
| `SymbolNode` | `visibility` | added, defaults to `Unknown` |
| `SymbolNode` | `is_test` | added, defaults to `false` |

An alias line becomes removable only after a full migration window where no
supported binary still writes the legacy name. Track each removal as its own PR;
do not combine it with further renames.

---

## 6. What NOT to do

- **Do not** delete a variant that was ever written to disk. If its semantics
  changed, map it to a compatibility variant via custom deserialization.
- **Do not** change a field's *type* while keeping the name (e.g.
  `Confidence(f32)` → `f64`). bincode will misread the bits.
- **Do not** add, remove, or reorder `SymbolKind` / `EdgeKind` variants in the
  middle of the enum without aliases. Snapshots encode the variant discriminant
  positionally — adding a new node or edge kind is exactly the kind of
  structural change that warrants a `SNAPSHOT_SCHEMA_VERSION` bump (see §4).
