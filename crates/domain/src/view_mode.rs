use serde::{Deserialize, Serialize};

/// How the board arranges each PRD Lane's Slices.
///
/// A pure presentation choice over the same read model: `Columns` shows the
/// Ready / WIP / Blocked columns, `Graph` shows the left-to-right Blocked-by
/// dependency graph. `Columns` is the default when nothing has been persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BoardViewMode {
    /// Ready / WIP / Blocked columns (the default).
    #[default]
    Columns,
    /// A left-to-right Blocked-by dependency graph.
    Graph,
}

impl BoardViewMode {
    /// The stable string persisted for this mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Columns => "columns",
            Self::Graph => "graph",
        }
    }

    /// Parse a persisted mode string, or `None` if it is unrecognised.
    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "columns" => Some(Self::Columns),
            "graph" => Some(Self::Graph),
            _ => None,
        }
    }
}
