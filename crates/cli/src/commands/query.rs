use crate::global_opts::GlobalOpts;
use crate::render::{
    bfs_edges_aggregated, bfs_edges_filtered, decode_cursor, render_symbol, EdgeAtDepth,
};
use clap::{Args, ValueEnum};
use coregraph_core::{EdgeKind, SymbolKind};
use coregraph_extractor::build_graph;

#[derive(Args)]
pub struct QueryArgs {
    /// Symbol name (exact or substring).
    pub symbol: String,

    /// Filter center-symbol kind.
    #[arg(long)]
    pub kind: Option<KindFilter>,

    /// Edge direction.
    #[arg(long, default_value = "both")]
    pub direction: Direction,

    /// Edge kind filter (repeatable).
    #[arg(long = "edge-kind")]
    pub edge_kind: Vec<EdgeFilter>,

    /// Union the neighborhood across every same-name definition instead of
    /// centering on one. Use for genuine overloads or when you want the callers
    /// of every symbol sharing the name (recall over precision).
    #[arg(long, default_value_t = false)]
    pub aggregate: bool,

    /// Traversal depth (overrides global --hop-limit).
    #[arg(long)]
    pub depth: Option<usize>,

    /// Opaque pagination cursor from a previous response.
    #[arg(long)]
    pub cursor: Option<String>,

    /// Page size (hard cap combined with token budget).
    #[arg(long, default_value_t = 50)]
    pub page_size: usize,

    /// Drill-down into a specific node id for detailed context.
    #[arg(long)]
    pub expand: Option<u64>,

    /// Skip on-demand healing before executing the query. When omitted, the
    /// query path checks every evidence file's content hash and re-extracts
    /// stale files within a 200ms budget.
    #[arg(long, default_value_t = false)]
    pub no_heal: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_opts::{ColorMode, LogLevel, OutputFormat};
    use std::path::PathBuf;

    fn base_globals() -> GlobalOpts {
        GlobalOpts {
            project: PathBuf::from("."),
            config: None,
            output_format: OutputFormat::Human,
            color: ColorMode::Auto,
            token_budget: 8000,
            hop_limit: 3,
            min_confidence: 0.70,
            include_stale: false,
            lang: vec![],
            verbose: false,
            quiet: false,
            log_level: LogLevel::Info,
            fast: false,
            standard: false,
            full: false,
            no_auto_start: false,
        }
    }

    fn args_with_symbol(symbol: &str) -> QueryArgs {
        QueryArgs {
            symbol: symbol.to_string(),
            kind: None,
            direction: Direction::Both,
            edge_kind: vec![],
            aggregate: false,
            depth: None,
            cursor: None,
            page_size: 50,
            expand: None,
            no_heal: true,
        }
    }

    #[test]
    fn rejects_empty_symbol() {
        let err = run(args_with_symbol(""), &base_globals()).unwrap_err();
        assert!(
            err.to_string().contains("must not be empty"),
            "expected empty-symbol error, got: {}",
            err
        );
    }

    #[test]
    fn rejects_whitespace_only_symbol() {
        // `coregraph query "   "` used to pass the filter and fall through
        // to the substring search, matching every name. Tabs and spaces
        // alone are never a real symbol.
        let err = run(args_with_symbol("   \t  "), &base_globals()).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }
}

#[derive(Copy, Clone, ValueEnum)]
pub enum Direction {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Copy, Clone, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum KindFilter {
    Function,
    Method,
    Class,
    Struct,
    Interface,
    Trait,
    Enum,
    EnumVariant,
    Constant,
    Variable,
    Field,
    TypeAlias,
    Module,
    Namespace,
    ConfigKey,
    StringLiteral,
    Package,
    ExternalPackage,
}

impl KindFilter {
    fn matches(&self, k: &SymbolKind) -> bool {
        matches!(
            (self, k),
            (KindFilter::Function, SymbolKind::Function)
                | (KindFilter::Method, SymbolKind::Method)
                | (KindFilter::Class, SymbolKind::Class)
                | (KindFilter::Struct, SymbolKind::Struct)
                | (KindFilter::Interface, SymbolKind::Interface)
                | (KindFilter::Trait, SymbolKind::Trait)
                | (KindFilter::Enum, SymbolKind::Enum)
                | (KindFilter::EnumVariant, SymbolKind::EnumVariant)
                | (KindFilter::Constant, SymbolKind::Constant)
                | (KindFilter::Variable, SymbolKind::Variable)
                | (KindFilter::Field, SymbolKind::Field)
                | (KindFilter::TypeAlias, SymbolKind::TypeAlias)
                | (KindFilter::Module, SymbolKind::Module)
                | (KindFilter::Namespace, SymbolKind::Namespace)
                | (KindFilter::ConfigKey, SymbolKind::ConfigKey)
                | (KindFilter::StringLiteral, SymbolKind::StringLiteral)
                | (KindFilter::Package, SymbolKind::Package)
                | (KindFilter::ExternalPackage, SymbolKind::ExternalPackage)
        )
    }
}

#[derive(Copy, Clone, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum EdgeFilter {
    Resolves,
    Calls,
    Implements,
    Extends,
    Overrides,
    References,
    Imports,
    StringMatch,
    Configures,
    DependsOn,
}

impl EdgeFilter {
    pub fn matches(&self, k: &EdgeKind) -> bool {
        matches!(
            (self, k),
            (EdgeFilter::Resolves, EdgeKind::Resolves)
                | (EdgeFilter::Calls, EdgeKind::Calls)
                | (EdgeFilter::Implements, EdgeKind::Implements)
                | (EdgeFilter::Extends, EdgeKind::Extends)
                | (EdgeFilter::Overrides, EdgeKind::Overrides)
                | (EdgeFilter::References, EdgeKind::References)
                | (EdgeFilter::Imports, EdgeKind::Imports)
                | (EdgeFilter::StringMatch, EdgeKind::StringMatch)
                | (EdgeFilter::Configures, EdgeKind::Configures)
                | (EdgeFilter::DependsOn, EdgeKind::DependsOn)
        )
    }

    /// Kebab-case wire name, used to forward the filter across IPC to the
    /// daemon (which re-applies it server-side).
    pub fn as_kebab(&self) -> &'static str {
        match self {
            EdgeFilter::Resolves => "resolves",
            EdgeFilter::Calls => "calls",
            EdgeFilter::Implements => "implements",
            EdgeFilter::Extends => "extends",
            EdgeFilter::Overrides => "overrides",
            EdgeFilter::References => "references",
            EdgeFilter::Imports => "imports",
            EdgeFilter::StringMatch => "string-match",
            EdgeFilter::Configures => "configures",
            EdgeFilter::DependsOn => "depends-on",
        }
    }

    pub fn from_kebab(s: &str) -> Option<EdgeFilter> {
        Some(match s {
            "resolves" => EdgeFilter::Resolves,
            "calls" => EdgeFilter::Calls,
            "implements" => EdgeFilter::Implements,
            "extends" => EdgeFilter::Extends,
            "overrides" => EdgeFilter::Overrides,
            "references" => EdgeFilter::References,
            "imports" => EdgeFilter::Imports,
            "string-match" => EdgeFilter::StringMatch,
            "configures" => EdgeFilter::Configures,
            "depends-on" => EdgeFilter::DependsOn,
            _ => return None,
        })
    }
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Incoming => "incoming",
            Direction::Outgoing => "outgoing",
            Direction::Both => "both",
        }
    }
}

pub fn run(args: QueryArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    // Reject empty / whitespace-only queries. Previously `coregraph query ""`
    // fell through to the fuzzy substring search where `str::contains("")`
    // returns true for every name, handing the caller an arbitrary symbol.
    if args.symbol.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "symbol name must not be empty (got {:?})",
            args.symbol
        ));
    }

    // Thin-client path: if the daemon is running, delegate the whole query
    // to it so multi-project requests hit the ProjectManager cache.
    if args.expand.is_none() && crate::ipc::ensure_running(globals) {
        // Minimal IPC payload — the daemon only needs symbol + project to
        // route to the right ProjectManager slot. The rich response is
        // rendered server-side and streamed back as a text body.
        let params = serde_json::json!({
            "symbol": args.symbol,
            "depth": args.depth.unwrap_or(globals.hop_limit),
            "cursor": args.cursor,
            "page_size": args.page_size,
            "output_format": match globals.output_format {
                crate::global_opts::OutputFormat::Json => "json",
                crate::global_opts::OutputFormat::Llm => "llm",
                crate::global_opts::OutputFormat::Human => "human",
            },
            "token_budget": globals.token_budget,
            "min_confidence": globals.min_confidence,
            "lang": globals.lang,
            "no_heal": args.no_heal,
            // Forward the direction + edge-kind filters so the daemon applies
            // them server-side. Without these the daemon rendered the full
            // neighborhood and `--edge-kind calls` / `--direction incoming`
            // were silently ignored whenever the daemon was running.
            "direction": args.direction.as_str(),
            "edge_kind": args.edge_kind.iter().map(|f| f.as_kebab()).collect::<Vec<_>>(),
            "aggregate": args.aggregate,
        });
        let req = crate::ipc::Request {
            method: "query".to_string(),
            params,
            project: globals.project_root(),
        };
        if let Ok(resp) = crate::ipc::send(&req) {
            if resp.ok {
                println!("{}", resp.body);
                return Ok(());
            }
        }
    }

    let (graph, _) = build_graph(&globals.project)?;

    let start_page = args
        .cursor
        .as_deref()
        .and_then(decode_cursor)
        .map(|c| c.page)
        .unwrap_or(0);

    // Prefer the name index for exact matches (O(1)) and fall back to
    // fuzzy substring search (which walks the index keys, not the entire
    // node set).
    let exact_hits = graph.lookup_by_name(&args.symbol, args.page_size * 4);
    let candidates: Vec<&coregraph_core::SymbolNode> = if !exact_hits.is_empty() {
        exact_hits
    } else {
        graph.lookup_by_name_fuzzy(&args.symbol, args.page_size * 4)
    };
    let matches: Vec<_> = candidates
        .into_iter()
        .filter(|n| args.kind.as_ref().is_none_or(|k| k.matches(&n.kind)))
        .filter(|n| crate::langfilter::match_langs(&globals.lang, &n.file))
        .collect();

    if matches.is_empty() {
        println!("No symbol found for '{}'", args.symbol);
        return Ok(());
    }

    // Center on the requested node (--expand) or the best definition: prefer an
    // exported/Public symbol over module-private or test-local same-name nodes.
    let center_index = if let Some(id) = args.expand {
        matches.iter().position(|n| n.id.0 == id).unwrap_or(0)
    } else {
        crate::render::pick_center_index(&matches)
    };
    let center = matches[center_index];

    let depth = args.depth.unwrap_or(globals.hop_limit).max(1);
    // Default is a *precise* single-symbol neighborhood so distinct same-name
    // symbols stay separated (e.g. a module `request` vs `Session.request`).
    // `--aggregate` opts into unioning the callers of every exact same-name
    // definition (genuine overloads, or recall across same-name symbols).
    let (incoming_all, outgoing_all) = if args.aggregate {
        let center_ids: Vec<coregraph_core::SymbolId> = matches
            .iter()
            .filter(|n| n.name == center.name)
            .map(|n| n.id)
            .collect();
        bfs_edges_aggregated(
            &graph,
            &center_ids,
            depth,
            globals.min_confidence,
            globals.include_stale,
        )
    } else {
        // Precise default keeps the exact prior single-center traversal (no
        // cross-overload union, no edge dedup) — only the center selection and
        // the multi-definition note are new. Aggregation/dedup is opt-in above.
        bfs_edges_filtered(
            &graph,
            center.id,
            depth,
            globals.min_confidence,
            globals.include_stale,
        )
    };

    // Direction + edge-kind filters.
    let edge_filter = args.edge_kind.as_slice();
    let pass_edge = |e: &EdgeAtDepth| -> bool {
        edge_filter.is_empty() || edge_filter.iter().any(|f| f.matches(&e.edge.kind))
    };
    let incoming_all: Vec<EdgeAtDepth> = match args.direction {
        Direction::Outgoing => vec![],
        _ => incoming_all.into_iter().filter(pass_edge).collect(),
    };
    let outgoing_all: Vec<EdgeAtDepth> = match args.direction {
        Direction::Incoming => vec![],
        _ => outgoing_all.into_iter().filter(pass_edge).collect(),
    };

    let rendered = render_symbol(
        &graph,
        center,
        &incoming_all,
        &outgoing_all,
        globals.output_format,
        globals.color,
        start_page,
        args.page_size,
        globals.token_budget,
    );
    println!("{}", rendered.body);

    // Surface the other definitions a precise query did not show, so the view
    // is never silently partial. Suppressed when aggregating (they are already
    // included) and for JSON (consumers re-query structurally).
    if !args.aggregate
        && !matches!(
            globals.output_format,
            crate::global_opts::OutputFormat::Json
        )
    {
        if let Some(note) = crate::render::multi_def_note(&matches, center_index) {
            println!("{note}");
        }
    }
    Ok(())
}
