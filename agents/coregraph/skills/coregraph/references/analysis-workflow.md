# Project analysis workflow

A practical order of operations for analyzing an unfamiliar repository with CoreGraph, with
the purpose, interpretation criteria, and pitfalls of each step.

## Prerequisites — don't skip

```bash
# Binary on PATH (if `which` fails after install, the npm global bin isn't on PATH —
# use an absolute path, e.g. "$(npm prefix -g)/bin/coregraph", or add that dir to PATH):
which coregraph || npm install -g @coregraph/cli

# The target must be a git repo for `diff` to work
git -C <project> rev-parse --is-inside-work-tree

# Existing snapshot? It will be reused
ls <project>/.coregraph/
```

---

## 1. Index

```bash
# Anchor --snapshot to the project: it resolves relative to your cwd (not -C), and the
# daemon only warm-loads <project>/.coregraph/snapshot.bin.
coregraph -C <project> index --stats --snapshot <project>/.coregraph/snapshot.bin
```

**Checkpoints**

- Note the final `Index complete — N files, M symbols, K edges` line; it is your baseline
  for later steps.
- Indexing builds two layers: tree-sitter symbol extraction and stack-graphs cross-file
  name resolution. No external toolchain is invoked.
- The read paths don't all report identical totals: plain `stats` / `server status` show the
  live in-memory graph (use that as the scale figure), while `stats --breakdown` and
  `snapshot load` report a smaller, separately-counted view. A mismatch between them is not
  an error.

**Language mix**

```bash
find <project> -type f \( -name "*.rs" -o -name "*.java" -o -name "*.kt" \
  -o -name "*.ts" -o -name "*.tsx" -o -name "*.py" -o -name "*.go" \) \
  -not -path "*/target/*" -not -path "*/build/*" -not -path "*/node_modules/*" \
  2>/dev/null | awk -F. '{print $NF}' | sort | uniq -c | sort -rn
```

---

## 2. Overview — stats

```bash
coregraph -C <project> stats --breakdown --top 15
```

**What to read**

1. **Language / symbol mix** — the share of Method / Class / Struct. Is it Java-heavy or
   Rust-heavy?
2. **Analysis origins** — five are produced, highest trust first: `CompilerDerived`,
   `NameResolved`, `SyntaxMatched`, `PatternMatched`, `ConventionInferred`. A high combined
   `CompilerDerived` + `NameResolved` share means most edges are compiler/scope-accurate
   (high trust); a high `SyntaxMatched` share means more edges are syntactic guesses.
3. **Trust models** — mostly `SourceEvidenced` is normal. A large `ExternallyMediated`
   share signals many config keys (noise ahead).
4. **Per-crate symbol/edge counts** — the first indicator of which crate/module is central.
   A dominant `other` bucket means large Java/TS monorepo components.
5. **Top in-degree** — reference hubs. But `Module`-level hubs are often big config files
   (Grafana dashboard JSON, substation YAML), not real architecture hubs.
6. **Heaviest files** — symbol-dense files. If they are mostly ops dashboards / spec YAML,
   that is noise — a signal to tune `exclude`.

---

## 3. Identify architecture hubs — query + impact

### 3.1 Pick candidates

- App entry points (often `*App`, `main`, `Bootstrap`, `Application`, `Server`, `Daemon`)
- Data-plane core (often `Pipeline`, `Dispatcher`, `Router`, `Stream`, `Channel`)
- Control plane (often `Authority`, `Service`, `Manager`, `Coordinator`, `Scheduler`)
- Security / policy (often `Gate`, `Guard`, `Authority`, `Policy`, `Verifier`)

Pick 1–2 representative symbols per axis and run impact.

### 3.2 Run impact

```bash
coregraph -C <project> impact <Symbol> --risk
```

**Line-by-line meaning**

```
Impact of 'X': A reachable symbols, B edges, depth 3
  Risk Score: R (Tier)                         # 4-factor weighted sum. 0.85+ Critical
  Blast Radius: Tier (M modules, C callers)    # breadth — look at both module and caller count
  Confidence-Weighted Impact: W                # weighted by edge confidence × distance
  Affected tests: T                            # tests in the impact set; gauge CI re-run scope
```

**Interpretation**

| Score | Tier | Reading |
|---|---|---|
| ≥ 0.85 | Critical | System core. Design review + full regression before changing. |
| 0.65–0.85 | High | Major component. Recommend extra test steps on the PR. |
| 0.40–0.65 | Medium | Local change but referenced by several modules. Touch and review. |
| < 0.40 | Low | In-module change. Standard PR. |

**Tips**

- If `Affected tests` ≫ `reachable symbols`, test coverage is good.
- `Blast Radius: Critical (45 modules, 2439 callers)` with four-digit callers usually means
  an interface or DI factory — consider an adapter layer instead of editing it directly.
- A Critical tier with only tens of reachable symbols means either extremely high edge
  confidence (a tight core) or very high test density.

---

## 4. Dead-code candidates — orphans

```bash
coregraph -C <project> orphans --exclude-tests > /tmp/orphans.log
head -1 /tmp/orphans.log    # e.g. "Orphan symbols (10): 7 likely dead, 3 library API surface, 0 test code"
```

**What orphans actually returns**

`orphans` reports **code symbols** (functions, methods, classes, structs, interfaces, traits,
enums, constants, variables, fields, type-aliases, namespaces) with no semantic incoming or
outgoing edge. Non-code nodes — config keys, string literals, doc nodes, file/module/package
containers — are excluded by design and **never** appear, so there is nothing to pre-filter:
the bracket labels are symbol *kinds* (`[Method]`, `[Function]`, …), never `[ConfigKey]`. The
header line pre-classifies the results into likely-dead / library API surface / test code;
start your review on the likely-dead rows.

**Classify by kind, then by module**

```bash
# Kind distribution of the reported orphans
awk -F'[][]' '/\[/{print $2}' /tmp/orphans.log | sort | uniq -c | sort -rn

# Where they cluster (intentional dirs vary by repo — adjust the pattern to yours)
grep -vE "(_poc|sandbox|experiments)" /tmp/orphans.log | sort
```

Among the candidates:

- Directories like `_poc*/`, `experiments/`, `sandbox/` (illustrative — use your repo's
  conventions) are usually intentional → add them to `[index] exclude`.
- `--public-only` is on by default; pass `--public-only=false` to surface private symbols as
  higher-confidence dead code.
- FFI boundaries (`*Bindings.java`, `extern "C"`), serialization, reflection, dynamic
  dispatch, and macro/derive-generated usage (e.g. clap `#[derive(Args)]` / `ValueEnum`) are
  out-of-graph — confirm any flagged symbol with a targeted read before deleting.

If orphans cluster in one module, that module is likely mid-refactor or slated for removal.

---

## 5. Cross-file inconsistencies

```bash
# Inspect with JSON so you can see each hit's two files (provenance)
coregraph -C <project> inconsistencies --category enum-mismatch --output-format json
coregraph -C <project> inconsistencies --category api-path --output-format json
coregraph -C <project> inconsistencies --category doc-drift
coregraph -C <project> inconsistencies --category config-key --output-format json
```

**Provenance first, then category.** `inconsistencies` has **no `--exclude-tests` flag**, and
on many repos most hits come from test/fixture files. For each hit, read its `a.file`/`b.file`
(the matched value is `a.name`/`b.name`) and **discard pairs where both sides are under
`tests/`, `fixtures/`, `__fixtures__/`, or `*.test.*`**. Count distinct *production* files,
not the raw total.

**Per-category criteria** (project-dependent — rank by inspecting hits, not by a fixed order):

- `api-path` — a **pairwise (O(n²)) match over path-like string literals**. Short
  slash-prefixed strings (including mock paths in test fixtures like `{ file: "/a.rs" }`)
  produce false hits. A real one is a route literal shared between a production client and a
  production server (singular/plural or version drift). Don't assume a hit is a real mismatch.
- `enum-mismatch` — the same variant value/name defined in two different **code** enums (an
  accidental collision or a divergent copy across files/languages, e.g. `Permission.ADMIN` and
  `Role.ADMIN` both `"admin"`). Usually clean; not an external-data comparison.
- `doc-drift` — a `@param` / `:param` naming a parameter the signature no longer has.
- `config-key/unused-key` — config keys with no resolved code binding. Accuracy varies by
  repo (false positives from camelCase↔snake/kebab normalization or reflection binding); it is
  **not** categorically noisier than `api-path`.

To suppress fixture noise at the source, add those paths to `[index] exclude` in
`.coregraph/config.toml` and re-index.

---

## 6. Impact of your git changes — diff

Before opening a PR, see how far your edits propagate:

```bash
coregraph -C <project> diff HEAD~1 --exclude-tests
coregraph -C <project> diff main --to HEAD --max-depth 2 --exclude-tests
```

`diff` summarizes the reachable impact of the **whole change set**, not a single symbol.

To post it as an automated PR comment:

```bash
coregraph -C <project> review --pr <N> --exclude-tests
coregraph -C <project> review --pr <N> --dry-run     # print only, don't comment
```

---

## 7. (Optional) Visualize

```bash
# DOT → Graphviz
coregraph -C <project> export --format dot --subgraph <Symbol> > subgraph.dot
dot -Tsvg subgraph.dot -o subgraph.svg

# Cypher → neo4j
coregraph -C <project> export --format cypher > graph.cypher

# JSON → custom analysis
coregraph -C <project> export --format json-graph > graph.json
jq '.nodes | length, .edges | length' graph.json
```

Always pass `--subgraph` for DOT — the full graph is impractical to render.

---

## 8. Continuous mode — watch

```bash
coregraph -C <project> watch --diff
```

Incremental rebuild + diff on every save. Handy during a large refactor to see how changes
land in the graph in real time.

---

## Report template

A readable structure for the analysis writeup:

```markdown
# <project> analysis

## Identity (from README + verification)
- Role / domain
- Runtime stack

## Graph scale
| files | symbols | edges | index time |
| trust distribution | analysis origins |

## Architecture hubs (impact --risk, top N)
| symbol | risk | blast radius | tests | reading |

## Layering
- apps/ roles
- key packages / crates
- external boundaries (FFI, gRPC, HTTP)

## Dead-code candidates
- distribution by module
- review list

## Inconsistency signals
- production-file hits only (test/fixture pairs discarded by provenance)
- per category, with each hit's two files noted

## Follow-up actions
- harden `.coregraph/config.toml` exclude patterns
- symbols worth a deep `coregraph impact …`
- visualization (`coregraph export …`)
```
