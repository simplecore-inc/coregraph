# What is CoreGraph?

CoreGraph indexes your codebase into a **call graph** and surfaces code intelligence metrics directly inside VS Code — without sending your code to an external service.

## How it works

A background daemon (`coregraph server`) analyzes your project and maintains a live call graph. The VS Code extension talks to this daemon over a local socket and renders the results as CodeLens annotations, hover details, a tree view, and inline gutter decorations.

## Supported languages

| Language | Identifier |
|---|---|
| Rust | `rust` |
| TypeScript / TSX | `typescript`, `typescriptreact` |
| JavaScript / JSX | `javascript`, `javascriptreact` |
| Python | `python` |
| Go | `go` |
| Java | `java` |
| Kotlin | `kotlin` |

Open any file in one of these languages and CoreGraph will activate automatically.

## First run

On first activation, CoreGraph asks whether to enable **commit-time impact warnings**. You can change this later in Settings (`coregraph.warnOnCommit.enabled`).
