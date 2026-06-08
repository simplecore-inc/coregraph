window.BENCHMARK_DATA = {
  "lastUpdate": 1780888892301,
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
      }
    ]
  }
}