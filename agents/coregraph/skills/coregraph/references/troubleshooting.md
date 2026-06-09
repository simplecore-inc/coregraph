# coregraph troubleshooting

Failure and noise patterns seen in real sessions, with fixes.

## 1. Daemon startup race

Symptom: `coregraph query …` intermittently fails with `Failed to connect to IPC socket`
or `Another daemon is starting up`.

Cause: on first launch, the LSP bridge and another thin-client race to spawn the daemon.

Fix:

```bash
coregraph server status
coregraph server restart

# Avoid the race by starting the daemon up front
coregraph server start --foreground &
```

`COREGRAPH_NO_AUTO_START=1` or `--no-auto-start` makes each command fall back to in-process
execution, but you lose the shared-snapshot benefit.

---

## 2. Filtering noise from analysis results

### 2a. `orphans` false positives (NOT config/string nodes)

`orphans` never reports `ConfigKey` / `StringLiteral` / doc / container nodes — they're
excluded internally, so there is no config-key flood to grep away (the output is already
code-only, labelled by symbol kind like `[Method]`). The real false-positive sources are
**out-of-graph references**: dynamic dispatch, reflection, FFI (`extern "C"`, `*Bindings`),
serialization/deserialization, and entry points called by the runtime/build. Confirm any
flagged symbol with a targeted read before deleting. Narrow with `--exclude-tests` and
`--public-only=false` (private symbols are higher-confidence dead code).

### 2b. `inconsistencies --category config-key` accuracy is project-dependent

Its false positives come mainly from key-name normalization (camelCase↔snake/kebab, e.g.
`fieldConfig` vs `fieldconfig`) and from bindings made via reflection/dynamic lookup the
graph can't see — so in config-heavy repos (Grafana JSON, Prometheus rules,
`gradle/libs.versions.toml`) it can flood, while in others every hit is accurate. It is
**not** categorically noisier than `api-path` (which O(n²)-matches path-like string literals
and is often the loudest category on repos with path-string test fixtures). Judge any
category by **provenance** — drop hits whose two files are both under `tests/` / `fixtures/`
— and by inspecting the actual hits, not by a fixed category ranking.

### 2c. External config files in Top in-degree

When `stats --breakdown` "Top N most-referenced symbols" surfaces Grafana dashboards,
substation YAML, or prometheus rules, those are big config files acting as the container of
all their inner keys — not real **code** hubs. To see code hotspots only, add their paths to
`index.exclude` and reindex:

```toml
[index]
exclude = [
  "target/", "**/target/",
  "build/", "**/build/",
  "node_modules/",
  ".git/",
  "_poc/", "_poc-*/",
  "ops/grafana/", "ops/prometheus/", "ops/alertmanager/",
  "specs/substations/",
]
```

> Note: `exclude` is project-specific. The defaults above (build output, dependency dirs,
> `.git/`) are generic; the ops/poc paths are examples — set what fits *your* repo, don't
> assume them.

---

## 3. Index reports symbols but a query is empty

Symptom: `index --stats` reports 16k symbols, but `query SomeKnownClass` says
`No symbol found`.

Possibilities:

1. The symbol is marked stale → confirm with `--include-stale`.
2. The name needs to match → try a substring instead.
3. The file is under `index.exclude` → read it with `coregraph config index.exclude` (the
   legacy positional read) or open `<project>/.coregraph/config.toml`. **`config show` does
   NOT list `index.exclude`** — it prints only the `limits.*` / `server.*` keys.
4. The parser didn't recognize the file (rare) → `coregraph index -v` for parser warnings.

```bash
coregraph query SomeClass --include-stale
coregraph query --min-confidence 0.0 SomeClass   # remove all filtering
```

---

## 4. Build / rebuild performance

- With a snapshot, `index` does change detection + incremental heal → usually 1–3 s.
- A full rebuild (`index --full`) is ~30–60 s for 1000 files (varies by language mix).
- Once the daemon is up, thin-client commands are milliseconds.

Reduce latency by keeping the daemon resident:

```bash
coregraph server start --auto-stop-minutes 0
```

---

## 5. Reset / clean slate

```bash
cd <project>
coregraph server stop 2>/dev/null
rm -f .coregraph/snapshot.bin
coregraph index --full --stats --snapshot .coregraph/snapshot.bin
```

Don't delete `.coregraph/config.toml` — it regenerates, but you lose your `exclude` tuning.

---

## 6. Debug logging

```bash
coregraph --log-level debug query SomeClass 2>&1 | tee /tmp/cg-debug.log
coregraph -v index --stats        # INFO-level verbose
```

Daemon logs go to the OS default log path; `coregraph server status` prints it.

---

## 7. `coregraph` not found on PATH

Symptom: `which coregraph` fails even though it's installed, and bare `coregraph …` returns
`command not found`. The npm global bin directory isn't on your `PATH`.

```bash
# Find the npm global bin dir and run by absolute path…
"$(npm prefix -g)/bin/coregraph" --version

# …or add it to PATH for the session
export PATH="$(npm prefix -g)/bin:$PATH"
```

A built-from-source binary works the same way by full path (e.g.
`./target/release/coregraph …`) without being on `PATH`.
