# Appendix: Manifest Parsing

Before extracting symbols, CoreGraph reads each project's build manifests to learn
its **structure**: which internal packages exist, what external dependencies they
declare, and whether each package is a **library** (a public API consumed from
outside the repo) or an **application** (an end product with no importable
surface). This is the `coregraph-manifest` crate (`crates/manifest/`).

What this is used for:

- **Library-vs-application classification** drives orphan reporting. A `pub`
  symbol in a *library* package is presumed to be API surface consumed
  downstream, so `orphans` labels it `[library API]` instead of calling it dead
  code. This is the only consumer of the manifest output
  (`crates/query/src/library.rs`).

Note that the parser's `Package` / `ExternalPackage` / `DependsOn` structs are
*not* ingested into the symbol graph — they feed the library classifier only.
The `ExternalPackage` symbols you see in the graph (via
`coregraph query <name> --kind external-package`) come from the extractor's
import resolution (`crates/extractor/src/lib.rs`), not from this crate, and the
graph has no `Package` nodes or `depends-on` edges sourced from the manifest.

---

## Supported build systems

`coregraph-manifest` ships seven parsers. Detection is by **file presence** in the
project root; a monorepo can match several at once and the results are merged.

| Parser name | Detected when root has | Language | Registry tag |
|---|---|---|---|
| `cargo` | `Cargo.toml` with `[workspace]` or `[package]` | Rust | `crates.io` |
| `maven` | `pom.xml` | Java | `maven_central` |
| `gradle` | `build.gradle(.kts)` or `settings.gradle(.kts)` | Java / Kotlin | `maven_central` |
| `go` | `go.mod` | Go | `pkg.go.dev` |
| `python` | `pyproject.toml`, `requirements.txt`, `setup.py`, `setup.cfg`, or `Pipfile` | Python | `pypi` |
| `npm/pnpm/yarn` | `package.json` | JavaScript | `npmjs` |
| `vite/webpack` | a fixed config file (`vite.config.{ts,js,mjs}` / `webpack.config.{js,ts}`), or `vite`/`webpack` in `devDependencies` | TypeScript | — |

Detection order (`crates/manifest/src/detector.rs`): cargo → maven → gradle → go
→ python → npm → vite. The vite parser runs last because a Vite project also has
a `package.json`, so the npm parser classifies the real package first.

---

## What gets produced

Every parser returns a `ProjectManifest`, and `parse_project()` merges them:

```rust
pub struct ProjectManifest {
    pub root: PathBuf,
    pub packages: Vec<Package>,          // internal packages / workspace members
    pub external_deps: Vec<ExternalPackage>,
    pub edges: Vec<DependsOn>,           // package → dependency
}

pub struct Package {
    pub name: String,
    pub version: Option<String>,
    pub path: PathBuf,                   // relative to project root
    pub language: Language,
    pub kind: PackageKind,               // Library | Application | Unknown
}

pub struct ExternalPackage {
    pub name: String,
    pub version_req: String,             // version requirement from the manifest
    pub resolved_version: Option<String>,// from a lockfile, when available
    pub registry: Option<String>,
}

pub struct DependsOn {
    pub from: String,                    // depending package name
    pub to: String,                      // dependency name
    pub dev_only: bool,                  // dev/test-only dependency
    pub optional: bool,
}
```

A dependency is recorded with just two flags — `dev_only` (dev/test scope) and
`optional`. There is no finer scope taxonomy: build dependencies, peer
dependencies, and runtime dependencies are all folded into runtime unless they
are dev/test, in which case `dev_only` is set.

### The parser contract

Each build system implements one small trait (`crates/manifest/src/lib.rs`):

```rust
pub trait ManifestParser: Send + Sync {
    fn name(&self) -> &'static str;
    fn can_parse(&self, root: &Path) -> bool;
    fn parse(&self, root: &Path) -> Result<ProjectManifest, ManifestError>;
}
```

`parse_project()` calls `can_parse()` on each parser, runs `parse()` on the ones
that match, and merges every result. A parser that errors is logged and skipped —
other parsers still contribute, so a broken `pom.xml` does not prevent the Cargo
side of a polyglot repo from being analyzed.

---

## Per-system parsing notes

**Cargo.** Reads `[package]` and walks `[workspace].members` (including simple
`crates/*` globs). Dependencies come from `[dependencies]`, `[build-dependencies]`
(both runtime-scoped), and `[dev-dependencies]` (`dev_only`). A `[dependencies]`
entry with `optional = true` is marked optional.

**npm / pnpm / yarn.** Parses `package.json` with `serde_json`. Reads
`dependencies`, `devDependencies` (`dev_only`), `peerDependencies`, and
`optionalDependencies`. Workspace members are discovered from the `workspaces`
field (array or `{ "packages": [...] }` object) **and** from `pnpm-workspace.yaml`
(`packages:` list, with `!negation` globs skipped). Each member's
library-vs-application kind is read from its own `package.json`.

**Gradle.** Groovy/Kotlin DSL build scripts are not fully evaluated. The parser
line-scans `build.gradle(.kts)` for dependency configurations
(`implementation`, `api`, `compile`, `runtimeOnly`, `compileOnly`,
`annotationProcessor`, `kapt`, plus `testImplementation` and `testCompile`) and
extracts `group:artifact:version` strings. The two `test`-prefixed
configurations are marked `dev_only`; other `test*` variants
(e.g. `testRuntimeOnly`, `testApi`) are not scanned.
Subprojects come from `include` statements in `settings.gradle(.kts)`; the project
name comes from `rootProject.name`. Because this is text matching rather than a
real Gradle evaluation, dependencies declared via version catalogs or computed
expressions may be missed.

**Maven.** Reads `pom.xml` with lightweight tag extraction. Dependencies come
from the `<dependencies>` block (`groupId:artifactId`, `<scope>test</scope>` →
`dev_only`, `<optional>true</optional>` → optional). Submodules in `<modules>`
are parsed recursively.

**Go.** Text-parses `go.mod`: the `module` line names the package, and `require`
(single-line and block form) lines name dependencies. `// indirect` comments are
stripped from version strings.

**Python.** Reads, in order: `pyproject.toml` (both PEP 621 `[project]` and Poetry
`[tool.poetry]` dependency tables, with Poetry `dev-dependencies` marked
`dev_only`), `requirements.txt`, and the dev variants
`requirements-dev.txt` / `requirements_dev.txt` / `requirements-test.txt`
(`dev_only`). The project name comes from `[project].name` or
`[tool.poetry].name`.

**Vite / Webpack.** Detection-only. CoreGraph does not evaluate JavaScript config
files, so it does not read `build.outDir` or `output.path`. The vite parser emits
a single synthetic package named `"<name> (vite)"` / `"<name> (webpack)"` with
kind `Unknown`; the actual dependency graph for that project comes from the npm
parser reading `package.json`.

---

## Library vs application classification

`PackageKind` is `Library`, `Application`, or `Unknown`, derived from
ecosystem-standard manifest signals. `Unknown` means "the manifest gave no
decisive signal" — consumers then make no library-based adjustment, so the
classifier never silently hides dead code unless a library is positively proven.

| Build system | Library when… | Application when… |
|---|---|---|
| Cargo | a `[lib]` table or `src/lib.rs` exists | only `[[bin]]` / `src/main.rs` |
| npm | declares `exports`, `module`, `types`, or `typings` (even if `private`); else a public package with `main` | `private: true` with no API surface, or a `bin`-only CLI |
| Gradle | `java-library`, `maven-publish`, `kotlin("jvm")`, or plain `java` plugin | `application`, Spring Boot, or `com.android.application` plugin |
| Maven | default / `<packaging>jar</packaging>` | `spring-boot-maven-plugin`, or `<packaging>war</packaging>` (`pom` aggregator → `Unknown`) |
| Go | always (a module declares an importable path) | — (command-only modules are not split out at the manifest level) |
| Python | `setup.py` / `setup.cfg`, or `pyproject.toml` declaring `[project].name` or `[tool.poetry]` | only a bare `requirements.txt` / `Pipfile` with no packaging metadata |

### Overriding the classification

When the heuristic is wrong for your project, set `[project] kind` in
`.coregraph/config.toml`. It applies to the whole project and takes precedence
over manifest detection:

```toml
[project]
kind = "library"   # or "application"
```

---

## Excluding generated and vendored files

Manifest parsing tells CoreGraph what a package *is*; deciding which **files** to
actually index is a separate concern handled during source collection
(`crates/extractor/src/lib.rs::collect_sources`). Three mechanisms apply, in
order:

1. **`.gitignore`-respecting walk** — files are discovered with
   `ignore::WalkBuilder`, which honors `.gitignore`. This is what keeps
   `node_modules`, `target`, `dist`, and other ignored trees out of the index —
   there is no hard-coded directory list.

2. **Configured excludes** — any pattern in the `[index] exclude` array of
   `.coregraph/config.toml` (gitignore syntax) is applied on top of the walk.
   Use this for paths specific to your project, such as a custom codegen output
   directory or a fixtures tree:

   ```toml
   [index]
   exclude = ["tests/fixtures/**", "generated/**"]
   ```

3. **Minified-bundle heuristic** — after a file is read, `looks_minified`
   drops it if it is at least 4 KB *and* its average line length exceeds
   ~1000 bytes. This catches minified/packed bundles (e.g. a committed
   `*.min.js`) without relying on filename patterns, while leaving formatted
   source — which averages tens of characters per line — untouched.

When `index` skips files via the minified heuristic it reports them, for
example:

```
coregraph: skipped 1 minified/generated file(s) (e.g. ./vscode-extension/media/cytoscape.min.js)
```

(Note: `crates/manifest/src/filter.rs` defines a separate `FileFilter` with
its own excluded-directory and generated-marker lists, but it is not wired into
the index path — the indexing filter is the walk + config excludes + minified
heuristic described above.)

---

Back to [documentation index](../README.md).
