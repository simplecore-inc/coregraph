// Public modules — types used by every downstream crate
pub mod arc_path;
pub mod edge;
pub mod exclude;
pub mod file_state;
pub mod graph;
pub mod span;
pub mod symbol;
pub mod text;

// Re-export the most-used types at crate root
pub use edge::{AnalysisOrigin, DirectEdge, EdgeKind, HealingScope, TrustModel};
pub use exclude::DEFAULT_EXCLUDE_PATTERNS;
pub use file_state::{hash_bytes, hash_file, sample_file_state, FileState, NodeStatus};
pub use graph::SymbolGraph;
pub use span::resolve_line_col;
pub use symbol::{Language, SymbolId, SymbolKind, SymbolNode, Visibility};
pub use text::levenshtein;
