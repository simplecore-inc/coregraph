window.BENCHMARK_DATA = {
  "lastUpdate": 1780882505738,
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
      }
    ]
  }
}