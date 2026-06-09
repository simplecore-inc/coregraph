# Daemon Routing Extension Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (inline recommended — tasks share `dispatch.rs`/`server.rs`/render helpers, so a single context keeps the build coherent). Steps use checkbox (`- [ ]`) syntax for tracking. Commits only when the user explicitly requests (per project rule).

**Goal:** Route `impact` / `diff` / `inconsistencies` through the daemon cache while keeping every consumer's (CLI / MCP / VSCode extension) output byte-identical or unified, and de-duplicate the thin-client pattern into one helper.

**Architecture:** Each command gets a single `render_<cmd>` shared by the in-process CLI path and the daemon `cached_<cmd>` handler (the proven `render_orphans` pattern). A `try_daemon` helper centralizes routing. `impact`/`inconsistencies` reuse their existing daemon methods (output unified); `diff` gets a new `diff_summary` method so the extension's rich `"diff"` (`dispatch_diff_with_git`) is untouched. On-demand healing extends to the three new methods only.

**Tech Stack:** Rust (workspace: `crates/cli`, `crates/query`, `crates/extractor`), `interprocess` IPC, TypeScript (`vscode-extension`).

---

## File Structure

- `crates/cli/src/ipc.rs` — add `try_daemon(method, params, project) -> Option<String>`.
- `crates/cli/src/dispatch.rs` — add `render_impact`, `render_inconsistencies`, `render_diff_summary`; rewrite `cached_impact`, `cached_inconsistencies`; add `cached_diff_summary`; thread `root: Option<&Path>` where needed.
- `crates/cli/src/commands/{impact,inconsistencies,diff}.rs` — add `try_daemon` fast-path; delegate local render to the shared `render_<cmd>`.
- `crates/cli/src/commands/{query,orphans,stats}.rs` — migrate to `try_daemon` (behavior preserved).
- `crates/cli/src/commands/server.rs` — route `impact`/`inconsistencies` through root-aware arms; add `diff_summary` arm; extend healing gate to the three new methods.
- `vscode-extension/src/providers/diagnosticsProvider.ts` (+ response types / `buildDiagnostics`) — adopt canonical inconsistencies JSON.

---

## Task 1: Common `try_daemon` helper + migrate query/orphans/stats

**Files:**
- Modify: `crates/cli/src/ipc.rs` (add helper)
- Modify: `crates/cli/src/commands/query.rs:260-299`, `orphans.rs:29-52`, `stats.rs:23-33`

- [ ] **Step 1** — Add helper to `ipc.rs`:

```rust
/// Try the daemon for a read query. Returns `Some(body)` when the daemon is
/// running, the request succeeds, and `resp.ok`; otherwise `None` so the caller
/// falls back to its in-process path. Routing opt-outs live in `ensure_running`.
pub fn try_daemon(
    globals: &crate::global_opts::GlobalOpts,
    method: &str,
    params: serde_json::Value,
) -> Option<String> {
    if !ensure_running(globals) {
        return None;
    }
    let req = Request { method: method.to_string(), params, project: globals.project_root() };
    match send(&req) {
        Ok(resp) if resp.ok => Some(resp.body),
        _ => None,
    }
}
```

- [ ] **Step 2** — Migrate `query.rs`: replace the inline `ensure_running`/`send` block (keep the `args.expand.is_none()` gate and the JSON payload) with:

```rust
if args.expand.is_none() {
    if let Some(body) = crate::ipc::try_daemon(globals, "query", params) { println!("{body}"); return Ok(()); }
}
```

- [ ] **Step 3** — Migrate `orphans.rs` (keep `globals.lang.is_empty()` gate) and `stats.rs` (keep `!args.breakdown` gate) the same way.
- [ ] **Step 4** — Verify no behavior change: `cargo test -p coregraph` (existing query/orphans/stats tests stay green) + `cargo clippy --workspace --all-targets -- -D warnings`.

Expected: PASS, zero diff in observable output.

---

## Task 2: `render_impact` + rewrite `cached_impact`

**Files:**
- Modify: `crates/cli/src/dispatch.rs` (add `render_impact`, rewrite `cached_impact` to take `root: Option<&Path>`)
- Modify: `crates/cli/src/commands/server.rs` (route `impact` like `orphans`, passing `Some(&target_project)`)
- Test: `crates/cli/src/dispatch.rs` (`#[cfg(test)]`)

- [ ] **Step 1** — Write failing equivalence tests in `dispatch.rs` tests: build a small fixture graph, assert `cached_impact` output == `render_impact` output for the in-process inputs across human/llm/json, plus:
  - non-transitive depth uses forwarded `depth` (not 1);
  - `--lang`/excluder filter reachable + affected_tests;
  - seed substring fallback resolves `parse` → `parse_config`;
  - json has `symbol` key + `nodes[]` + full 4-factor risk.
- [ ] **Step 2** — `cargo test -p coregraph cached_impact` → FAIL.
- [ ] **Step 3** — Extract `render_impact(seed_name, &ImpactResult, Option<&ImpactRisk>, transitive, OutputFormat) -> String` mirroring `impact.rs:72-168` exactly (human/llm/json, `risk_as_json` full shape). Rewrite `cached_impact(params, g, root)` to: resolve seed (exact-then-substring), read `depth` (honor forwarded value), `transitive`, `risk`, `lang`, apply `PathExcluder`/`match_langs` when `root` is `Some`, then call `render_impact`.
- [ ] **Step 4** — Update `dispatch_cached` arm and `server.rs` so `impact` passes `Some(&target_project)` (new root-aware branch, mirroring the `orphans` branch at server.rs:866-871).
- [ ] **Step 5** — `cargo test -p coregraph cached_impact` → PASS; `cargo clippy` clean.

---

## Task 3: Route `impact.rs` through the daemon

**Files:**
- Modify: `crates/cli/src/commands/impact.rs` (add `try_daemon` path; local path calls `render_impact`)
- Test: `crates/cli/tests/` or inline equivalence

- [ ] **Step 1** — Write a test asserting local `impact` output == daemon `impact` output (spin a graph, compare `render_impact` vs a `cached_impact` round-trip) for human/llm/json with and without `--risk`/`--transitive`.
- [ ] **Step 2** — Run → FAIL (no routing yet).
- [ ] **Step 3** — In `impact.rs::run()`, after the empty-symbol guard, add the `try_daemon("impact", params)` fast-path forwarding `symbol/depth/transitive/risk/lang/output_format`. Replace the inline match render with a call to `render_impact` so local and daemon share it.
- [ ] **Step 4** — Run → PASS; `cargo clippy` clean.

---

## Task 4: `render_inconsistencies` + canonical JSON + rewrite `cached_inconsistencies`

**Files:**
- Modify: `crates/cli/src/dispatch.rs` (add `render_inconsistencies`, rewrite `cached_inconsistencies` with `root`, doc-drift support)
- Modify: `crates/cli/src/commands/server.rs` (root-aware `inconsistencies` arm)
- Modify: `crates/cli/src/commands/inconsistencies.rs` (local path calls shared renderer)

**Canonical JSON (single source of truth):**

```json
{
  "count": 2,
  "reports": [
    { "category": "api-path", "shared_value": "/foo",
      "a": { "name": "/foo", "file": "src/a.ts", "line": 12 },
      "b": { "name": "/foo", "file": "src/b.ts", "line": 30 } }
  ]
}
```

Rules: `name` always `strip_marker`-ed; `category` kebab (`category.label()`); `line` 0-based via `node_line`; doc-drift reports use `{ "category": "doc-drift", "symbol", "file", "line", "detail" }` inside the same `reports` array.

- [ ] **Step 1** — Write failing tests: `render_inconsistencies` produces the canonical JSON above; human/llm match `inconsistencies.rs:59-138`; doc-drift category routes through `find_doc_param_drift`; markers stripped in all three formats.
- [ ] **Step 2** — `cargo test -p coregraph cached_inconsistencies` → FAIL.
- [ ] **Step 3** — Extract `render_inconsistencies(&[Report], &[DocDriftReport], OutputFormat) -> String`. Rewrite `cached_inconsistencies(params, g, root)`: parse `category` (incl. `doc-drift` → call `find_doc_param_drift`), apply `lang`/excluder when `root` present, render via shared fn.
- [ ] **Step 4** — Wire `server.rs` `inconsistencies` arm to pass `Some(&target_project)`.
- [ ] **Step 5** — Run → PASS; `cargo clippy` clean.

---

## Task 5: Route `inconsistencies.rs` through the daemon

**Files:**
- Modify: `crates/cli/src/commands/inconsistencies.rs`

- [ ] **Step 1** — Test: local output == daemon output for each category (incl. doc-drift) across human/llm/json.
- [ ] **Step 2** — Run → FAIL.
- [ ] **Step 3** — Add `try_daemon("inconsistencies", params)` forwarding `category/lang/output_format`. Local path delegates to `render_inconsistencies` (and `run_doc_drift` folds into it).
- [ ] **Step 4** — Run → PASS; `cargo clippy` clean.

---

## Task 6: Update extension to canonical inconsistencies JSON

**Files:**
- Modify: `vscode-extension/src/providers/diagnosticsProvider.ts` (+ `InconsistenciesResponse` type + `buildDiagnostics`)

- [ ] **Step 1** — Update `InconsistenciesResponse` to `{ count: number; reports: Array<{ category: string; shared_value?: string; a?: {name;file;line}; b?: {name;file;line}; symbol?; file?; line?; detail? }> }`.
- [ ] **Step 2** — Update `buildDiagnostics` to read `reports[]` (was the old shape) and place a diagnostic at `line` for each side.
- [ ] **Step 3** — `cd vscode-extension && npm run compile` → PASS; run extension unit tests if present (`npm test`).

---

## Task 7: `render_diff_summary` + new `diff_summary` daemon method

**Files:**
- Modify: `crates/cli/src/dispatch.rs` (add `render_diff_summary`, `cached_diff_summary(params, g)`; add `"diff_summary"` arm to `dispatch_cached` and `dispatch`)
- Modify: `crates/cli/src/commands/server.rs` (add `diff_summary` branch; it needs git, so pass `target_project` like the existing `diff` branch)

- [ ] **Step 1** — Test: `cached_diff_summary` reproduces `diff.rs:107-153` output (human/llm/json) given the same changed-file set; honors `to`/`max_depth`/`exclude_tests`.
- [ ] **Step 2** — Run → FAIL.
- [ ] **Step 3** — Extract `render_diff_summary(base, to, changed_len, &touched, reached_len, depth, OutputFormat) -> String` from `diff.rs`. Implement `cached_diff_summary` that runs the touched-union computation (`diff.rs:74-105`) against the cached graph + git, then renders. Keep `dispatch_diff_with_git` (`"diff"`) untouched.
- [ ] **Step 4** — Add `server.rs` `else if request.method == "diff_summary"` branch (root-aware, bypasses the query-only healing banner like `diff`).
- [ ] **Step 5** — Run → PASS; `cargo clippy` clean.

---

## Task 8: Route `diff.rs` through the daemon

**Files:**
- Modify: `crates/cli/src/commands/diff.rs`

- [ ] **Step 1** — Test: local `diff` == daemon `diff_summary` output for human/llm/json with `--to`/`--max-depth`/`--exclude-tests` variations.
- [ ] **Step 2** — Run → FAIL.
- [ ] **Step 3** — In `diff.rs::run()`, after computing `changed` is empty-guarded, add `try_daemon("diff_summary", {base,to,max_depth,exclude_tests,output_format})`. Local path delegates to `render_diff_summary`.
- [ ] **Step 4** — Run → PASS; `cargo clippy` clean.

---

## Task 9: Extend on-demand healing to the three new methods

**Files:**
- Modify: `crates/cli/src/commands/server.rs:786` (healing gate)

- [ ] **Step 1** — Test (`daemon_lifecycle` or dispatch-level): after editing a file, a daemon `impact`/`inconsistencies`/`diff_summary` request reflects the change within the heal budget (not stale until watcher).
- [ ] **Step 2** — Run → FAIL (currently gated to `query`).
- [ ] **Step 3** — Change the gate from `request.method == "query"` to a set `{query, impact, inconsistencies, diff_summary}` (respect `no_heal`). For `diff_summary`, also feed the git-changed files into the heal candidate set so just-edited seeds resolve. Leave `orphans`/`stats` watcher-only.
- [ ] **Step 4** — Run → PASS; `cargo clippy` clean.

---

## Task 10: Full integration verification

- [ ] **Step 1** — `cargo fmt --all -- --check`
- [ ] **Step 2** — `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] **Step 3** — `cargo test --workspace --exclude coregraph && cargo test -p coregraph -- --test-threads=1`
- [ ] **Step 4** — `cd vscode-extension && npm run compile`
- [ ] **Step 5** — Manual smoke: start daemon, run `impact`/`diff`/`inconsistencies` with daemon up vs `--no-auto-start`, diff the outputs (must be identical).

---

## Self-Review

- **Spec coverage:** try_daemon (T1) ✓ · impact render+route (T2,T3) ✓ · inconsistencies render+canonical+route (T4,T5) ✓ · extension (T6) ✓ · diff new method+route (T7,T8) ✓ · healing scoped to 3 (T9) ✓ · orphans/stats unchanged (T1 preserves) ✓ · root injection (T2,T4,T7 via server.rs) ✓.
- **Type consistency:** `render_impact`/`render_inconsistencies`/`render_diff_summary`, `cached_<cmd>(params, g, root)`, method `"diff_summary"` used consistently across tasks.
- **No placeholders:** each task names exact files, the canonical JSON schema is concrete, equivalence assertions are specified.
