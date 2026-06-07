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
 └─ @coregraph/cli-win32-x64    ┘
```

At install time npm picks the single platform package whose `os`/`cpu` match the
host and skips the rest. `bin/coregraph.js` in the main package resolves that
package's native binary and execs it, forwarding argv, stdio, exit code, and
signals. The user-facing command stays `coregraph` regardless of the package
name.

## Files

| Path                          | Role                                                        |
| ----------------------------- | ----------------------------------------------------------- |
| `config.mjs`                  | Single source of truth: package name, platform matrix, version reader (reads the cli crate version). |
| `launcher/coregraph.js`       | The `bin` launcher shipped in the main package.             |
| `scripts/build-platform.mjs`  | Assembles one `npm/dist/cli-<os>-<cpu>/` package from a built binary. |
| `scripts/build-main.mjs`      | Assembles `npm/dist/cli/` (main package) with stamped optionalDependencies. |
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

Publishing is automated by `.github/workflows/publish-npm.yml`:

1. A build matrix compiles the release binary for each platform and assembles
   its `npm/dist/cli-<os>-<cpu>/` package as a CI artifact.
2. The publish job downloads all platform packages, assembles the main package,
   and runs `scripts/publish.mjs` (platforms first, then main).

The workflow runs on a published GitHub Release (or manual `workflow_dispatch`)
and requires an **`NPM_TOKEN`** repository secret — an npm **automation** token
for an account with publish rights to the `@coregraph` scope. (A classic/publish
token will be rejected by 2FA in CI; an automation token bypasses the OTP
prompt.) Until that secret is set, the publish job automatically falls back to a
dry-run, so the workflow still validates packaging end to end.

### Release checklist & caveats

- **Bump the version first.** npm packages are stamped from
  `crates/cli/Cargo.toml`'s `[package].version` (what `coregraph --version`
  prints). The workflow triggers on a published Release but does **not** derive
  the version from the tag, and npm rejects re-publishing an existing version —
  so bump `crates/cli/Cargo.toml` (and the workspace version) before cutting the
  release.
- **All platforms or nothing.** `scripts/publish.mjs` refuses a real publish
  unless every platform in `config.mjs` is assembled (the main package lists
  them all as optional dependencies). The build matrix uses `fail-fast: false`,
  so if one platform fails to build, fix it and re-run rather than publishing a
  partial set. `--dry-run` still validates a partial set.
- **musl Linux builds are the highest-risk step and are not exercised by the
  local check.** `verify-local.sh` only covers the host platform; the
  `*-unknown-linux-musl` targets compile tree-sitter's C sources and
  stack-graphs against musl in CI. The first CI run is where those get proven —
  watch the `linux-x64` / `linux-arm64` matrix legs.

## Switching the package name

The published identity lives entirely in `config.mjs` (`SCOPE` / `MAIN_PACKAGE`).
Changing it there updates the main package, every platform package, the
launcher's resolution, and the publish order in one place. The install-command
line in the docs (`README.md`, `docs/cli.md`, `vscode-extension/README.md`) is
the only thing to update by hand.
