# npm packaging

This directory packages the `coregraph` CLI for distribution on npm using the
**platform-packages** pattern (the same approach esbuild and swc use): one main
package plus one native-binary package per platform, wired together through
optional dependencies.

```
@coregraph/cli                  main package — JS launcher + bin: coregraph
 ├─ @coregraph/cli-darwin-arm64 ┐
 ├─ @coregraph/cli-darwin-x64   │ optionalDependencies, exact-pinned to the
 ├─ @coregraph/cli-linux-x64    │ same version. npm installs only the one
 ├─ @coregraph/cli-linux-arm64  │ matching the host's os/cpu.
 ├─ @coregraph/cli-win32-x64    │
 └─ @coregraph/cli-win32-arm64  ┘
```

At install time npm picks the single platform package whose `os`/`cpu` match the
host and skips the rest. `bin/coregraph.js` in the main package resolves that
package's native binary and execs it, forwarding argv, stdio, exit code, and
signals. The user-facing command stays `coregraph` regardless of the package
name.

## Files

| Path                          | Role                                                        |
| ----------------------------- | ----------------------------------------------------------- |
| `config.mjs`                  | Single source of truth: package name, platform matrix, version reader (reads the workspace version from the root `Cargo.toml`). |
| `launcher/coregraph.js`       | The `bin` launcher shipped in the main package.             |
| `scripts/build-platform.mjs`  | Assembles one `npm/dist/cli-<os>-<cpu>/` package from a built binary. |
| `scripts/build-main.mjs`      | Assembles `npm/dist/cli/` (main package) with stamped optionalDependencies. |
| `scripts/assemble-from-artifacts.mjs` | Assembles every platform package plus the main package from a staging directory of downloaded release binaries (used by the publish workflow). |
| `scripts/publish.mjs`         | Publishes every `npm/dist/*` package (platforms first, then main). `--dry-run` supported. |
| `scripts/verify-local.sh`     | End-to-end local check on the host platform (build → pack → install → run). |
| `README.npm.md`               | The README shown on the npm package page.                   |

`npm/dist/` is generated output and is git-ignored. Native binaries are never
committed.

## Build & verify locally

```bash
# Build the host platform package + the main package from an existing binary,
# pack, install into a throwaway project, and exercise the launcher end to end.
bash npm/scripts/verify-local.sh
```

Manual equivalent for a single platform:

```bash
cargo build --release -p coregraph
node npm/scripts/build-platform.mjs --os darwin --cpu arm64 --binary target/release/coregraph
node npm/scripts/build-main.mjs
node npm/scripts/publish.mjs --dry-run
```

## Releasing

The per-platform binaries are built by `.github/workflows/release.yml` (via the
reusable `.github/workflows/_build-matrix.yml`), which attaches them to the
GitHub Release as `coregraph-<version>-<os>-<cpu>.tar.gz`/`.zip` assets.
Publishing to npm is then handled by `.github/workflows/publish-npm.yml`, which
**rebuilds nothing** — it reuses the exact bytes already attached to a Release:

1. The publish job downloads each platform's release archive and unpacks it into
   a `staging/bin-<os>-<cpu>/` layout.
2. It runs `scripts/assemble-from-artifacts.mjs staging` to assemble every
   `npm/dist/cli-<os>-<cpu>/` package plus the main package locally in that job
   (not as CI artifacts).
3. It runs `scripts/publish.mjs` (platforms first, then main).

The workflow runs **only** via manual `workflow_dispatch`. It does **not**
trigger automatically when a Release is published. Two inputs control it: a `tag`
input picks which Release's attached binaries to reuse (blank = the latest GitHub
Release), and a `dry_run` input that **defaults to `true`** — so a real upload
requires explicitly setting `dry_run=false`.

A real publish also requires an **`NPM_TOKEN`** repository secret — an npm
**automation** token for an account with publish rights to the `@coregraph`
scope. (A classic/publish token will be rejected by 2FA in CI; an automation
token bypasses the OTP prompt.) If that secret is unset (or `dry_run=true`), the
publish job automatically falls back to a dry-run, so the workflow still
validates packaging end to end.

### Release checklist & caveats

- **Bump the version first.** npm packages are stamped with the workspace
  version read from the **root `Cargo.toml`** (`[workspace.package].version`,
  what `coregraph --version` prints) — `crates/cli/Cargo.toml` only inherits it
  via `version.workspace = true`, so editing that file alone changes nothing.
  The version is **not** derived from the git tag (the tag only selects the
  checkout ref and the release-asset filenames), and npm rejects re-publishing an
  existing version — so bump the root `Cargo.toml` version before cutting the
  release. `release.yml` enforces that the tag matches this version (and that
  `vscode-extension/package.json` matches) before building, so bump those too.
- **All platforms or nothing.** `scripts/publish.mjs` refuses a real publish
  unless every platform in `config.mjs` is assembled (the main package lists
  them all as optional dependencies). The build matrix that produces the release
  binaries lives in `release.yml`/`_build-matrix.yml` and uses `fail-fast: false`,
  so if one platform fails to build, fix it and re-run the Release rather than
  publishing a partial set. `--dry-run` still validates a partial set.
- **musl Linux builds are the highest-risk step and are not exercised by the
  local check.** `verify-local.sh` only covers the host platform; the
  `*-unknown-linux-musl` targets compile tree-sitter's C sources and
  stack-graphs against musl in CI. The first CI run is where those get proven —
  watch the `linux-x64` / `linux-arm64` matrix legs.

## Switching the package name

The published identity lives entirely in `config.mjs` (`SCOPE` / `MAIN_PACKAGE`).
Changing it there updates the main package, every platform package, the
launcher's resolution, and the publish order in one place. The install-command
line in the docs is **not** generated from `config.mjs`, however, so it must be
updated by hand wherever the package name is hard-coded. At minimum this
includes `README.md`, `docs/cli.md`, `vscode-extension/README.md`,
`npm/README.npm.md` (copied verbatim into the published package by
`scripts/build-main.mjs`), `docs/contributing/development.md`,
`agents/README.md`, `agents/AGENTS.md`,
`agents/coregraph/skills/coregraph/SKILL.md`,
`agents/coregraph/skills/coregraph/references/analysis-workflow.md`, and
`agents/codex/install.sh`.
