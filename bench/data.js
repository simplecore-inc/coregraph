window.BENCHMARK_DATA = {
  "lastUpdate": 1781565434615,
  "repoUrl": "https://github.com/simplecore-inc/coregraph",
  "entries": {
    "Benchmark": [
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "061b15a045aac8e373b0938eb0b6efb4d461798f",
          "message": "fix(daemon): stop the file-watcher loop busy-spinning when its channel closes\n\nLikely root cause of the Windows daemon E2E failure: next_changed_files() blocks\nwhile the watcher is healthy, but returns an empty batch immediately once its\nchannel closes/errors. The daemon's watcher loop did `if paths.is_empty() {\ncontinue; }`, busy-spinning a CPU core — on a constrained Windows runner that\nstarves the IPC accept-loop thread, so the daemon stays alive but stops\naccepting connections (the observed \"became unresponsive\"). Back off on empty.\n\nRe-enable daemon_starts_then_responds_to_health_then_stops on Windows (removes\nthe #[cfg_attr(windows, ignore)]) and add a CI step that dumps daemon.log + the\ndaemon PID's liveness on Windows failure, so the cause is observable from CI.\n\nAlso fixes two audit findings: bench.yml auto-push was gated on refs/heads/master\n(default branch is main) so benchmark history never landed; and the pr-review\ncomposite action used @stable instead of the pinned @1.95.0 toolchain.",
          "timestamp": "2026-06-08T10:32:33+09:00",
          "tree_id": "137472aa2c121afe473635ba8229d2bf996ab019",
          "url": "https://github.com/simplecore-inc/coregraph/commit/061b15a045aac8e373b0938eb0b6efb4d461798f"
        },
        "date": 1780882504545,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 478557005,
            "range": "± 36793132",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 49349,
            "range": "± 897",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 4512,
            "range": "± 108",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 96718,
            "range": "± 1355",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 133397,
            "range": "± 1402",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "b97a6a55167ed4034b0c34d89f0b893288d859ba",
          "message": "fix(daemon): break the Windows daemon away from the parent job object\n\nThe CI daemon.log + PID probe proved the Windows failure mode: the detached\ndaemon binds the pipe and becomes ready, then the PROCESS exits (tasklist: no\nsuch PID) — with no panic and no shutdown line — seconds after `server start`\nreturns. That is the classic symptom of a detached process reaped by the\nparent's job object: GitHub's runner wraps each step's process tree in a\nkill-on-close job, and the daemon was spawned with DETACHED_PROCESS |\nCREATE_NEW_PROCESS_GROUP but WITHOUT CREATE_BREAKAWAY_FROM_JOB, so it stayed in\nthe job and was killed. Add CREATE_BREAKAWAY_FROM_JOB, with a fallback spawn for\njob objects that forbid breakaway.",
          "timestamp": "2026-06-08T10:42:59+09:00",
          "tree_id": "df4add717a1bcfef7ec41fae1476702c39c5c83c",
          "url": "https://github.com/simplecore-inc/coregraph/commit/b97a6a55167ed4034b0c34d89f0b893288d859ba"
        },
        "date": 1780883157801,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 625055451,
            "range": "± 7322413",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 61539,
            "range": "± 328",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5287,
            "range": "± 82",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 121898,
            "range": "± 3743",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 174127,
            "range": "± 1724",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "341772712f6c2d77d638106d9486ccc5f248cac9",
          "message": "chore(daemon): temporary Windows death-trace (heartbeat + accept log)\n\nDiagnostic only — pinpoints where the detached Windows daemon silently exits\n(process confirmed dead via tasklist, no Rust panic in daemon.log). A 250ms\nheartbeat shows when the process vanishes; per-accept lines show whether it was\nserving. Reverted once the Windows daemon lifecycle is fixed.",
          "timestamp": "2026-06-08T10:50:47+09:00",
          "tree_id": "beb69f1be5f395fd036afe413ae177819467c9df",
          "url": "https://github.com/simplecore-inc/coregraph/commit/341772712f6c2d77d638106d9486ccc5f248cac9"
        },
        "date": 1780883624958,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 617559850,
            "range": "± 13490576",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 58673,
            "range": "± 304",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5124,
            "range": "± 32",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 125492,
            "range": "± 2154",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 175580,
            "range": "± 1998",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "e1e002abe0154e1622549a98fddfc21301945147",
          "message": "fix(daemon): read the IPC request without try_clone (crashed Windows pipe)\n\nThe CI death-trace pinpointed it: the daemon entered the accept loop, accepted\nexactly one connection, then the process vanished within 250ms — no heartbeat,\nno Rust panic — *before* the handler's own request log. The first connection is\nis_running()'s connect-then-drop readiness probe, and the handler did\n`BufReader::new(stream.try_clone()?)`. Duplicating a half-closed Windows\nnamed-pipe handle crashed the whole process at the C level.\n\nThe handler only reads one line then writes one response, so borrow the stream\nfor the read (`BufReader::new(&mut stream)`) instead of cloning the handle.\n(Diagnostic heartbeat/accept trace from the previous commit is kept for this\nverification run and removed once Windows is green.)",
          "timestamp": "2026-06-08T10:59:26+09:00",
          "tree_id": "d60dbfdf1fcabfad9ab9ca479c836ba1bd753464",
          "url": "https://github.com/simplecore-inc/coregraph/commit/e1e002abe0154e1622549a98fddfc21301945147"
        },
        "date": 1780884114480,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 464490224,
            "range": "± 42446548",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 48682,
            "range": "± 631",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 4457,
            "range": "± 24",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 96992,
            "range": "± 2397",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 133529,
            "range": "± 1132",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "4fccdb71bd8fab0ae05ed8c3627bbc2ec96c4e7f",
          "message": "deps(daemon): bump interprocess 2.4.0 -> 2.4.2\n\nAfter removing try_clone the Windows daemon survives the first connection but\nstill crashes (C-level, no Rust panic) right after reading a request — inside\ninterprocess's Windows named-pipe path. interprocess 2.4.2 changed exactly the\nWindows named-pipe read (recv_bytes), accept (listener), and Win32 wrapper\n(c_wrappers) code versus 2.4.0, i.e. the very paths where the daemon dies, so\nthe remaining crash is plausibly an upstream 2.4.0 Windows bug fixed in 2.4.2.",
          "timestamp": "2026-06-08T11:08:30+09:00",
          "tree_id": "d3b83d16ba3b00525c1f72e8c31ac851e1d5ac82",
          "url": "https://github.com/simplecore-inc/coregraph/commit/4fccdb71bd8fab0ae05ed8c3627bbc2ec96c4e7f"
        },
        "date": 1780884683942,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 601915594,
            "range": "± 46548641",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 62506,
            "range": "± 10358",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5400,
            "range": "± 17",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 120932,
            "range": "± 528",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 169621,
            "range": "± 1105",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "1c03cfc7ebbe9126149e9556063fd4c5897175f7",
          "message": "chore(daemon): remove Windows death-trace diagnostics; de-flake perf test\n\nThe Windows daemon is fixed (read the IPC request without try_clone + interprocess\n2.4.2), so remove the temporary heartbeat/accept/read trace from the accept loop.\n\nAlso de-flake extractor incremental::adding_a_function_parses_quickly_vs_initial:\nits `warm < 10ms` absolute floor was too tight for a loaded shared CI runner and\nflaked on ubuntu. Assert a looser but still-meaningful bound — warm no slower\nthan cold, or under an absolute 50ms.",
          "timestamp": "2026-06-08T11:21:42+09:00",
          "tree_id": "87fc6659d59f01e29854771123d4eeb0ffd5c9fa",
          "url": "https://github.com/simplecore-inc/coregraph/commit/1c03cfc7ebbe9126149e9556063fd4c5897175f7"
        },
        "date": 1780885478587,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 622615149,
            "range": "± 79272491",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 58857,
            "range": "± 11558",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5392,
            "range": "± 26",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 127179,
            "range": "± 1049",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 178449,
            "range": "± 4315",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "2343952c1e7bc19613eb2e979ba80abd9238d74b",
          "message": "Initial commit\n\nCoreGraph: an in-memory code symbol graph for multi-language and monorepo\ncodebases. Combines tree-sitter symbol extraction with stack-graphs name\nresolution, served from a background daemon, with a confidence/trust model\non every edge.\n\nIncludes the Rust workspace (CLI + library crates), the VS Code extension,\nthe npm distribution scaffold, the e2e test suites, and the documentation set.",
          "timestamp": "2026-06-08T11:48:38+09:00",
          "tree_id": "87fc6659d59f01e29854771123d4eeb0ffd5c9fa",
          "url": "https://github.com/simplecore-inc/coregraph/commit/2343952c1e7bc19613eb2e979ba80abd9238d74b"
        },
        "date": 1780887109731,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 606973447,
            "range": "± 6781006",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 62588,
            "range": "± 1331",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5497,
            "range": "± 76",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 121025,
            "range": "± 2241",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 167657,
            "range": "± 4036",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "6aaec80f538eddc9394a17613083c86feeda3948",
          "message": "feat(npm): add win32-arm64 target; repoint repo URLs to simplecore-inc\n\n- Add a sixth distribution target win32-arm64 (aarch64-pc-windows-msvc), built on\n  the native windows-11-arm runner — the same arm64 standard-runner mechanism as\n  the existing ubuntu-24.04-arm linux-arm64 build. config.mjs PLATFORMS,\n  _build-matrix.yml, the launcher's supported-platforms message, and the npm\n  README all list it.\n- Repoint every repository/homepage URL from thkwag/coregraph to\n  simplecore-inc/coregraph after the repo move (Cargo.toml workspace metadata,\n  npm config.mjs REPOSITORY/HOMEPAGE, npm README).\n\nLocal packaging dry-run (verify-local.sh) passes on the host (build → pack →\ninstall → --version/query/mcp). First publish stays at 0.1.0 (never published);\na real release still needs the NPM_TOKEN secret and the win32-arm64 build green.",
          "timestamp": "2026-06-08T12:18:37+09:00",
          "tree_id": "1cf17120123e95ee6a6f83f8f9e54aed9a918e26",
          "url": "https://github.com/simplecore-inc/coregraph/commit/6aaec80f538eddc9394a17613083c86feeda3948"
        },
        "date": 1780888891792,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 626082985,
            "range": "± 70551312",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 62681,
            "range": "± 13649",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5503,
            "range": "± 30",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 120837,
            "range": "± 1175",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 168214,
            "range": "± 1840",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "31829d65cb4694db739c9a7259664742ceb68c1a",
          "message": "ci(release): tag-driven release flow with independent npm/vscode publish\n\nSplit releasing into three independent workflow_dispatch procedures that no longer trigger each other: release.yml cuts the vX.Y.Z tag + notes (enforcing the Cargo workspace, cli crate, and VS Code extension share one version); publish-npm.yml and publish-vscode.yml take a release tag, check out that ref, and publish exactly that version (default dry-run); _build-matrix.yml gains a ref input so npm builds the tagged source.\n\nAlign the VS Code extension to 0.1.0 and add the 0.1.0 changelog.",
          "timestamp": "2026-06-08T15:11:35+09:00",
          "tree_id": "dd3c9931adadc94741032bd12368e4c0af203e46",
          "url": "https://github.com/simplecore-inc/coregraph/commit/31829d65cb4694db739c9a7259664742ceb68c1a"
        },
        "date": 1780899272489,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 600134004,
            "range": "± 59036323",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 62700,
            "range": "± 10378",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5896,
            "range": "± 111",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 121150,
            "range": "± 1794",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 168847,
            "range": "± 1217",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "3fcbc2e5119a62384ea4854272c8b0934f82dd95",
          "message": "ci(release): attach platform binaries to the Release and reuse them for npm\n\nrelease.yml now builds every platform at the released commit, attaches one archive per platform (coregraph-<version>-<os>-<cpu>.tar.gz/.zip) plus SHA256SUMS to the GitHub Release. publish-npm no longer rebuilds: it downloads those release binaries and publishes them, so npm ships the exact bytes the Release does. Platform list and archive names are driven by npm/config.mjs in both workflows.",
          "timestamp": "2026-06-08T15:28:53+09:00",
          "tree_id": "a443f8a5a1be81c5ce2d0facab67d887c2e70e62",
          "url": "https://github.com/simplecore-inc/coregraph/commit/3fcbc2e5119a62384ea4854272c8b0934f82dd95"
        },
        "date": 1780900303943,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 604660162,
            "range": "± 43192541",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 62615,
            "range": "± 10279",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5849,
            "range": "± 40",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 122145,
            "range": "± 3182",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 169121,
            "range": "± 1002",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "b114652e2874cee6533615702b38ac86a76e9820",
          "message": "docs(readme): add highlights diagram, badges, and CodeGraph credit; add LICENSE\n\nEmbed a docs/assets/highlights.svg diagram atop the Highlights section; add npm / license / OS-arch badges; add an 'Inspired by CodeGraph' section crediting the project that popularized the pattern; lead with token-efficiency and speed.\n\nAdd the MIT LICENSE file and ship it inside the npm main package (build-main.mjs now copies LICENSE into the package).",
          "timestamp": "2026-06-08T16:13:41+09:00",
          "tree_id": "d08105db408283ddc6f2163855ee459109056ffc",
          "url": "https://github.com/simplecore-inc/coregraph/commit/b114652e2874cee6533615702b38ac86a76e9820"
        },
        "date": 1780902947853,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 352274243,
            "range": "± 2795325",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 48498,
            "range": "± 124",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 4477,
            "range": "± 156",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 96182,
            "range": "± 956",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 131860,
            "range": "± 1880",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "cf340f94147b79b4f1221434220759e4c5daf8c9",
          "message": "docs(readme): add logo and social-preview image; recolor macOS badge\n\nAdd docs/assets/logo.{svg,png} (graph-mark app icon) and center it atop the README; add docs/assets/social-preview.{svg,png} (1280x640) for the GitHub repo Social preview. Recolor the macOS badge from black to a space-grey slate (334155) so it differs from the default 555 shields label.",
          "timestamp": "2026-06-08T16:24:21+09:00",
          "tree_id": "3c2f67bc2937673f43b1643c0ed22f208025004d",
          "url": "https://github.com/simplecore-inc/coregraph/commit/cf340f94147b79b4f1221434220759e4c5daf8c9"
        },
        "date": 1780903598535,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 458439441,
            "range": "± 13353761",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 59259,
            "range": "± 331",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5532,
            "range": "± 42",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 130545,
            "range": "± 1034",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 184326,
            "range": "± 987",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "edfb6624deb9e9efc3e81c5f450b32f33ee56c04",
          "message": "feat(agents): multi-agent kit — plugin/marketplace, AGENTS.md, Codex/Gemini/opencode\n\nShip coregraph as a capability any AI coding agent can install and use, biased to\nprefer the symbol graph over a raw grep/read sweep for structural questions.\n\n- .claude-plugin/marketplace.json + agents/coregraph plugin (guidance skill +\n  bundled `coregraph mcp` server): `/plugin marketplace add simplecore-inc/coregraph`\n  then `/plugin install coregraph@coregraph`\n- agents/AGENTS.md: thin wrapper consumed by Codex/Gemini/opencode; SKILL.md is the\n  single source of guidance, with cli-reference/analysis-workflow/llm-usage/\n  troubleshooting references\n- agents/{codex,gemini,opencode}: per-agent MCP config; Codex install.sh\n- README: \"Use with AI coding agents\" section; README + docs/integrations MCP tool\n  tables synced to the corrected tool contract",
          "timestamp": "2026-06-09T11:15:33+09:00",
          "tree_id": "f6b9e1e99da9978453861d3755e1ce9eca8cd1b8",
          "url": "https://github.com/simplecore-inc/coregraph/commit/edfb6624deb9e9efc3e81c5f450b32f33ee56c04"
        },
        "date": 1780971495816,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 460947342,
            "range": "± 8626027",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 58840,
            "range": "± 1157",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5136,
            "range": "± 57",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 125074,
            "range": "± 1310",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 176068,
            "range": "± 963",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "31a79e30d907b477d2bf38fc86f42b9db509a801",
          "message": "chore(release): 0.1.1\n\nBump workspace, cli/query/stack crates, and the VS Code extension to 0.1.1, and\nadd the 0.1.1 CHANGELOG section (multi-agent kit; MCP impact `transitive` flag;\ncorrected MCP tool descriptions and `--min-confidence` help).",
          "timestamp": "2026-06-09T11:31:13+09:00",
          "tree_id": "612b64bd04aafe9d6bd48fd87ee884310481d69e",
          "url": "https://github.com/simplecore-inc/coregraph/commit/31a79e30d907b477d2bf38fc86f42b9db509a801"
        },
        "date": 1780972407343,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 447328826,
            "range": "± 10505256",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 62553,
            "range": "± 273",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5728,
            "range": "± 27",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 123262,
            "range": "± 1308",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 171383,
            "range": "± 405",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "54426cc58d8cb6716b9cf19df25ac7902d1b02d8",
          "message": "refactor(release): single-source the version in the Cargo workspace\n\nBumping a release should touch one line. Make every crate inherit the workspace\nversion instead of hardcoding it, and read that one value everywhere downstream.\n\n- crates cli/query/stack: `version = \"x\"` -> `version.workspace = true` (core,\n  graph, manifest, extractor, server, watcher already inherited it)\n- npm/config.mjs `cliVersion()`: read the root Cargo.toml [workspace.package]\n  version instead of crates/cli/Cargo.toml, so the npm package follows the\n  workspace automatically\n- release.yml version check: drop the now-redundant cli-crate check (it inherits\n  the workspace); verify the workspace version + the VS Code extension\n\nThe Cargo workspace version is now the single source for every crate and the npm\nCLI. The VS Code extension keeps its own package.json version (separate ecosystem),\nstill gated by the release check.",
          "timestamp": "2026-06-09T11:41:32+09:00",
          "tree_id": "b6861eb0dfa582b6dc511ef6863c75cea45ec785",
          "url": "https://github.com/simplecore-inc/coregraph/commit/54426cc58d8cb6716b9cf19df25ac7902d1b02d8"
        },
        "date": 1780973028956,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 447095082,
            "range": "± 9162700",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 62663,
            "range": "± 631",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5921,
            "range": "± 45",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 123282,
            "range": "± 462",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 176762,
            "range": "± 1006",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "221d4b1935a1b83ac65893ac933fd10fb0d8597a",
          "message": "feat(agents): install the plugin without cloning the source repo\n\nAdding the marketplace via `owner/repo` git-clones the whole source repo just to\nread marketplace.json. Switch to a no-clone install:\n\n- plugin `source` -> `git-subdir` ({url, path: agents/coregraph, ref: main}), so\n  the plugin is a sparse partial-clone of only agents/coregraph (works for both\n  git- and URL-added marketplaces; relative paths don't work for URL marketplaces)\n- document `/plugin marketplace add <raw marketplace.json URL>`, which downloads\n  only the small catalog (no repo clone); the owner/repo shorthand still works but\n  clones the full source\n\nSingle source of truth — the plugin files stay in agents/coregraph, no duplicate\nmarketplace repo and no sync.",
          "timestamp": "2026-06-09T12:05:07+09:00",
          "tree_id": "7e5e6a5c38f029220c8ebc2e741528235e040c01",
          "url": "https://github.com/simplecore-inc/coregraph/commit/221d4b1935a1b83ac65893ac933fd10fb0d8597a"
        },
        "date": 1780974443647,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 458238923,
            "range": "± 12108268",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 60861,
            "range": "± 192",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5738,
            "range": "± 13",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 132309,
            "range": "± 1022",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 180785,
            "range": "± 677",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "8011c5de9f471e68441659e88aa2ff703914d7db",
          "message": "docs(coregraph): document index vs analysis exclude and the orphans recall ceiling\n\nUpdate the bundled skill and CLI reference: [index].exclude drops a file's nodes and edges (and can turn a symbol referenced only by an excluded file into a false orphan) while [analysis].exclude keeps it indexed but hides its own symbols from dead-code reports; orphans reports only fully-disconnected symbols, so a clean result is triage, not a census.",
          "timestamp": "2026-06-09T16:06:36+09:00",
          "tree_id": "57fa01d143e9f77f47f8e95c33d001514ad20374",
          "url": "https://github.com/simplecore-inc/coregraph/commit/8011c5de9f471e68441659e88aa2ff703914d7db"
        },
        "date": 1780989057315,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 358378122,
            "range": "± 8174718",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 48714,
            "range": "± 1223",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 4633,
            "range": "± 108",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 95890,
            "range": "± 358",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 132347,
            "range": "± 3908",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "602404b6952b289e66ac7593dc160412cf877c71",
          "message": "chore(release): 0.1.2",
          "timestamp": "2026-06-09T16:19:02+09:00",
          "tree_id": "91156faba86b79beff184ee2e665fd94a29d32e4",
          "url": "https://github.com/simplecore-inc/coregraph/commit/602404b6952b289e66ac7593dc160412cf877c71"
        },
        "date": 1780989679632,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 462869688,
            "range": "± 15748574",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 58960,
            "range": "± 1391",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5215,
            "range": "± 277",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 126568,
            "range": "± 990",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 175672,
            "range": "± 818",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "98abe1a07dec9049c2127550008e9067946c7f9f",
          "message": "docs(readme): add npm upgrade instructions to Quick start",
          "timestamp": "2026-06-09T16:30:25+09:00",
          "tree_id": "ea8d120ccee8a2323acd8bce9437825e78204e89",
          "url": "https://github.com/simplecore-inc/coregraph/commit/98abe1a07dec9049c2127550008e9067946c7f9f"
        },
        "date": 1780990363061,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 459121821,
            "range": "± 13143217",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 63216,
            "range": "± 289",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5871,
            "range": "± 47",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 123487,
            "range": "± 2939",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 172489,
            "range": "± 547",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "999fae2ad9af2b9c3e27d60a3a9ae7055b5ceb64",
          "message": "fix(cli): canonicalize in-process project root so daemon on/off output matches\n\nThe five daemon-routed read commands (query, orphans, stats, impact,\ninconsistencies) built their in-process graph from globals.project (the\nraw -C value, default the relative \".\"), while the daemon canonicalizes\nits routing key. So `coregraph impact foo` printed relative ./x paths\nin-process but absolute paths through the daemon — an everyday on/off\ninconsistency, not just a /tmp symlink artifact.\n\nUse globals.project_root() (canonical absolute) for the in-process graph\nbuild and the exclude/test/library classification in all five, so node\npaths and classification match the daemon exactly.\n\nDocs: correct the thin-client command list (add diff, drop inspect which\nnever routes) and widen the on-demand healing description from query-only\nto the daemon-routed read commands (query/impact/inconsistencies/diff).",
          "timestamp": "2026-06-09T19:21:21+09:00",
          "tree_id": "3920f859321e5f2eb25d0fe01d8bf1b885234aa5",
          "url": "https://github.com/simplecore-inc/coregraph/commit/999fae2ad9af2b9c3e27d60a3a9ae7055b5ceb64"
        },
        "date": 1781001344547,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 470519041,
            "range": "± 8114735",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 59167,
            "range": "± 282",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5423,
            "range": "± 18",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 127443,
            "range": "± 779",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 173479,
            "range": "± 3880",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "bb93b3ec74a777236044999760a67cebf014ee42",
          "message": "chore(release): 0.1.3",
          "timestamp": "2026-06-09T20:01:38+09:00",
          "tree_id": "ac69d525558c2a652f5dc0828823e75ffc53769c",
          "url": "https://github.com/simplecore-inc/coregraph/commit/bb93b3ec74a777236044999760a67cebf014ee42"
        },
        "date": 1781003034701,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 460492237,
            "range": "± 11095233",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 59373,
            "range": "± 1033",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 5469,
            "range": "± 128",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 130692,
            "range": "± 587",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 182848,
            "range": "± 1385",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "03d359e88b59e5ac6fd52297fd6c8ae07ed9cdb7",
          "message": "chore(release): 0.2.0",
          "timestamp": "2026-06-10T18:58:46+09:00",
          "tree_id": "deb41593e9eb1c346846e228f5b48f5960791e73",
          "url": "https://github.com/simplecore-inc/coregraph/commit/03d359e88b59e5ac6fd52297fd6c8ae07ed9cdb7"
        },
        "date": 1781085685200,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 471136050,
            "range": "± 8510154",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 50668,
            "range": "± 208",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2694,
            "range": "± 15",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4457,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4298,
            "range": "± 60",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "8f54bb52a7fdac14912dd04e4575fee09de1714e",
          "message": "docs(readme): drop the Inspired-by-CodeGraph section",
          "timestamp": "2026-06-10T19:04:45+09:00",
          "tree_id": "a71a457ddc0adedd09f5270fc00cc37e69a205be",
          "url": "https://github.com/simplecore-inc/coregraph/commit/8f54bb52a7fdac14912dd04e4575fee09de1714e"
        },
        "date": 1781086078752,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 471727996,
            "range": "± 11669942",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 48454,
            "range": "± 733",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2609,
            "range": "± 145",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4502,
            "range": "± 33",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4495,
            "range": "± 69",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "7e2cc32862714e582ac5c4127ce6b29472c396f0",
          "message": "style: apply rustfmt",
          "timestamp": "2026-06-10T19:11:53+09:00",
          "tree_id": "92e015206328f889248256cd6619dc88ee0e23d7",
          "url": "https://github.com/simplecore-inc/coregraph/commit/7e2cc32862714e582ac5c4127ce6b29472c396f0"
        },
        "date": 1781086451116,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 463325550,
            "range": "± 11091537",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 52294,
            "range": "± 1054",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2622,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4556,
            "range": "± 13",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4453,
            "range": "± 44",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "93f35afe34692d58f86123563aeff85b54259ca9",
          "message": "fix(viz): stop static-layout cluster toggles from freezing the engine\n\nAbove STATIC_LAYOUT_THRESHOLD the canvas used cooldownTicks=0, and the\ncluster toggle's reheat raced it: the transition flag is React state, so\nit reached the engine one render after d3ReheatSimulation ran — the\nfirst layoutTick saw cntTicks > 0 and stopped the engine again, leaving\nthe cluster force registered but never applied. Toggling any node\nfilter (e.g. hide top hubs) replaced graphData and ran 80 synchronous\nwarmup ticks, which is why clusters only formed after that.\n\nKeep cooldownTicks=Infinity except in focus mode (instant-settle UX)\nand stop big cosmos graphs via d3AlphaMin=0.03 instead: alpha cooling\nbounds CPU the same way, and a reheat is safe at any moment. The\nclusterTransition state is gone.",
          "timestamp": "2026-06-10T20:22:45+09:00",
          "tree_id": "91cfebc85fc190c4ce8f8110daf0f1186304abb9",
          "url": "https://github.com/simplecore-inc/coregraph/commit/93f35afe34692d58f86123563aeff85b54259ca9"
        },
        "date": 1781090706077,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 465806277,
            "range": "± 10363226",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 48436,
            "range": "± 755",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2564,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4375,
            "range": "± 18",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4226,
            "range": "± 59",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "dbcf2b0182b5173ea809644a42d03df5ff76f09a",
          "message": "docs: document analysis improvements and config recommend\n\nUpdate cli.md, config-example.md, data-flow.md, graph-model.md,\nuse-cases.md and the README for the new behavior: per-package stats\nbreakdown, container-free in-degree, exclude/config lint warnings,\nstring_match_max_files, [inconsistencies] tuning, doc-drift rename\ncandidates, the post-index noise note, and the config recommend\ncommand with its --write mode and MCP tool.",
          "timestamp": "2026-06-12T09:05:46+09:00",
          "tree_id": "4507996026eb8d8942a207d4d4b2b5e865d9a553",
          "url": "https://github.com/simplecore-inc/coregraph/commit/dbcf2b0182b5173ea809644a42d03df5ff76f09a"
        },
        "date": 1781223419555,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 486818274,
            "range": "± 9592118",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 55570,
            "range": "± 201",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2559,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4471,
            "range": "± 11",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4421,
            "range": "± 12",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "3026bce567b7fce86cdb9bb4173e3ceb94e399ef",
          "message": "chore(release): 0.2.1",
          "timestamp": "2026-06-12T09:31:44+09:00",
          "tree_id": "43eb0bb5439dffc2c94e59ca5bef3599db272b23",
          "url": "https://github.com/simplecore-inc/coregraph/commit/3026bce567b7fce86cdb9bb4173e3ceb94e399ef"
        },
        "date": 1781224447096,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 494319239,
            "range": "± 10142269",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 55522,
            "range": "± 284",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2533,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4377,
            "range": "± 68",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4330,
            "range": "± 16",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "a5f388b4517dc9189294e72f4a23c8fa107fe1e0",
          "message": "feat(viz): background mode (--detach / --stop)\n\n--detach re-spawns the atlas server fully detached from the terminal\n(new session via setsid / DETACHED_PROCESS on Windows), returns once\nthe port answers, logs to viz.log in the daemon runtime directory, and\nrecords <pid> <port> in viz.pid. It refuses to start when the port is\nalready serving. --stop terminates the recorded instance and cleans up;\na record whose port no longer answers is treated as stale and removed\nwithout signaling (the pid may have been recycled).\n\nThe detach/terminate mechanics are extracted from the daemon spawn into\nshared helpers (daemon::spawn_detached / daemon::terminate_pid) so the\ndaemon auto-spawn and viz use one implementation, including the log\nrotation and the Windows job-breakaway retry. Also documents the viz\ncommand in docs/cli.md, which had no section for it.",
          "timestamp": "2026-06-12T10:10:22+09:00",
          "tree_id": "4b32b55e26937da839a364abf491687356c24c3a",
          "url": "https://github.com/simplecore-inc/coregraph/commit/a5f388b4517dc9189294e72f4a23c8fa107fe1e0"
        },
        "date": 1781226763986,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 492846968,
            "range": "± 9249530",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 57351,
            "range": "± 2089",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2841,
            "range": "± 44",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4631,
            "range": "± 51",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4557,
            "range": "± 123",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "d9bcf04dd3374ffd1331574b5d5e90dc19370037",
          "message": "chore(release): 0.2.2",
          "timestamp": "2026-06-12T10:26:14+09:00",
          "tree_id": "dcdef0c8da696bb91e6606ea16567115036e49a2",
          "url": "https://github.com/simplecore-inc/coregraph/commit/d9bcf04dd3374ffd1331574b5d5e90dc19370037"
        },
        "date": 1781227714413,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 478836681,
            "range": "± 15158040",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 59424,
            "range": "± 3023",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2589,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4471,
            "range": "± 46",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4463,
            "range": "± 8",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "2903fe4618746b1c47c1149ec6d4c02c12d3aab4",
          "message": "chore(release): 0.2.3",
          "timestamp": "2026-06-16T03:18:29+09:00",
          "tree_id": "66497bc122865d61de03f4e07d5f98d0e9ae4e9c",
          "url": "https://github.com/simplecore-inc/coregraph/commit/2903fe4618746b1c47c1149ec6d4c02c12d3aab4"
        },
        "date": 1781547652619,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 371036100,
            "range": "± 11631016",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 46044,
            "range": "± 590",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2479,
            "range": "± 12",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 3958,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 3935,
            "range": "± 14",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "782e43267a8e9dcab8e1bcc81b24f97cf84afdda",
          "message": "chore(release): 0.2.4",
          "timestamp": "2026-06-16T07:49:46+09:00",
          "tree_id": "050f5d8a28ff334cf4aaabccb57d2a86db00e3e2",
          "url": "https://github.com/simplecore-inc/coregraph/commit/782e43267a8e9dcab8e1bcc81b24f97cf84afdda"
        },
        "date": 1781563935918,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 482213671,
            "range": "± 3058296",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 55612,
            "range": "± 575",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 2960,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 4795,
            "range": "± 14",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 4743,
            "range": "± 71",
            "unit": "ns/iter"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "committer": {
            "email": "thkwag@gmail.com",
            "name": "Taehwan Kwag",
            "username": "thkwag"
          },
          "distinct": true,
          "id": "5617801886c74a6cf007035eb464b68e51116ec7",
          "message": "perf(extractor): cache the doc-comment tree-sitter Query per thread\n\nextract_block_doc_comments recompiled its per-language Query (a static query string) on every file via Query::new — thousands of redundant compilations on a large monorepo (a profiled hot spot). Cache the compiled Query in a thread_local keyed by the query string's address (1:1 with language across all callers), caching None for an un-compilable query so it is not retried. thread_local avoids any Sync bound on Query and fits the rayon one-file-per-task walk. ~0.5s saved indexing a ~900-file project; more on doc-comment-heavy trees. Behaviour unchanged (extractor doc-comment tests pass).",
          "timestamp": "2026-06-16T08:14:25+09:00",
          "tree_id": "680c07fb2e917a4d8b70287e472731e5e4514f29",
          "url": "https://github.com/simplecore-inc/coregraph/commit/5617801886c74a6cf007035eb464b68e51116ec7"
        },
        "date": 1781565433798,
        "tool": "cargo",
        "benches": [
          {
            "name": "build_graph/extractor-crate/cold",
            "value": 424136164,
            "range": "± 14898628",
            "unit": "ns/iter"
          },
          {
            "name": "query/find_orphans",
            "value": 59435,
            "range": "± 1081",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=1",
            "value": 3212,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=3",
            "value": 5050,
            "range": "± 46",
            "unit": "ns/iter"
          },
          {
            "name": "query/compute_impact/depth=5",
            "value": 5044,
            "range": "± 19",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}