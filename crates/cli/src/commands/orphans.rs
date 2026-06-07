use crate::global_opts::{GlobalOpts, OutputFormat};
use clap::Args;
use coregraph_extractor::build_graph;
use coregraph_query::find_orphans;

#[derive(Args)]
pub struct OrphansArgs {
    /// Report only public symbols (declared visibility, falling back to the
    /// name shape). Defaults on; pass `--public-only=false` to also include
    /// private symbols (higher-confidence dead code).
    #[arg(
        long,
        default_value_t = true,
        action = clap::ArgAction::Set,
        num_args = 0..=1,
        default_missing_value = "true"
    )]
    pub public_only: bool,

    /// Exclude symbols from test files/directories.
    #[arg(long, default_value_t = false)]
    pub exclude_tests: bool,
}

pub fn run(args: OrphansArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    // Fast path via daemon (if running). Lang filtering still happens client
    // side, so only take it when no lang filter is set; the daemon applies the
    // same public_only/exclude_tests policy as the local path below.
    if globals.lang.is_empty() && crate::ipc::ensure_running(globals) {
        let req = crate::ipc::Request {
            method: "orphans".to_string(),
            params: serde_json::json!({
                "public_only": args.public_only,
                "exclude_tests": args.exclude_tests,
                // Forward the requested format so the daemon honors
                // `--output-format`; without this it defaulted to Human and
                // silently ignored `--output-format json`.
                "output_format": match globals.output_format {
                    OutputFormat::Json => "json",
                    OutputFormat::Llm => "llm",
                    OutputFormat::Human => "human",
                },
            }),
            project: globals.project.clone(),
        };
        if let Ok(resp) = crate::ipc::send(&req) {
            if resp.ok {
                println!("{}", resp.body);
                return Ok(());
            }
        }
    }
    let (graph, _) = build_graph(&globals.project)?;
    let excluder = coregraph_query::PathExcluder::from_project_root(&globals.project);
    // Library-vs-application classifier (manifest-derived, config-overridable).
    // A public symbol owned by a library package is its external API surface —
    // unreferenced *within* the repo does not make it dead code.
    let classifier = coregraph_query::LibraryClassifier::from_project_root(&globals.project);
    let all = find_orphans(&graph);
    let filtered: Vec<_> = all
        .into_iter()
        .filter(|n| !args.exclude_tests || !coregraph_query::is_test_symbol_in(n, &globals.project))
        .filter(|n| !args.public_only || coregraph_query::is_public_symbol(n))
        .filter(|n| crate::langfilter::match_langs(&globals.lang, &n.file))
        .filter(|n| !excluder.is_excluded(&n.file))
        .collect();

    // Bucket each orphan (mutually exclusive, in precedence order):
    //   test code      — harness-invoked, provably not dead (even inline).
    //   library API     — public symbol in a library package; consumed externally.
    //   likely dead     — everything else (the actionable dead-code candidates).
    // Test takes precedence over library so an inline test in a library package
    // is reported as test, not API surface.
    let is_test =
        |n: &coregraph_core::SymbolNode| coregraph_query::is_test_symbol_in(n, &globals.project);
    let is_api = |n: &coregraph_core::SymbolNode| {
        !is_test(n) && classifier.is_library_file(&n.file) && coregraph_query::is_public_symbol(n)
    };
    // Single shared renderer (also used by the daemon path, `cached_orphans`)
    // so `--output-format {human,llm,json}` produces identical output whether
    // the query ran here in-process or through the daemon.
    println!(
        "{}",
        crate::dispatch::render_orphans(
            &filtered,
            globals.output_format,
            &is_test,
            &is_api,
            args.public_only,
            classifier.has_signal(),
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_only_accepts_explicit_bool_value() {
        // The Human-output Note tells users to run `--public-only=false` to also
        // include private symbols, so that exact form must parse. A bare
        // `--public-only` and the absent case both stay true.
        use clap::Parser;
        #[derive(Parser)]
        struct TestCli {
            #[command(flatten)]
            args: OrphansArgs,
        }
        assert!(
            TestCli::try_parse_from(["x"]).unwrap().args.public_only,
            "default is true"
        );
        assert!(
            TestCli::try_parse_from(["x", "--public-only"])
                .unwrap()
                .args
                .public_only,
            "bare flag stays true"
        );
        assert!(
            !TestCli::try_parse_from(["x", "--public-only=false"])
                .unwrap()
                .args
                .public_only,
            "the documented --public-only=false form must disable it"
        );
    }

    #[test]
    fn public_name_filter_is_shared_with_query() {
        // The CLI now delegates to coregraph_query::is_public_name so the local
        // and daemon paths agree.
        assert!(coregraph_query::is_public_name("Foo"));
        assert!(coregraph_query::is_public_name("foo_bar"));
        assert!(!coregraph_query::is_public_name("_private"));
    }
}
