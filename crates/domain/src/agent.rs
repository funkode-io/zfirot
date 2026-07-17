use serde::{Deserialize, Serialize};

/// A reference to an Agent that can currently be assigned on a repository.
///
/// Carries the display name and the GitHub node ID needed to assign the Agent
/// to a Slice. The set is discovered live per board load and never persisted;
/// the node ID is only ever read back to the adapter that will use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef {
    /// The Agent's login / display name shown in the UI.
    pub name: String,
    /// The GitHub node ID used when assigning this Agent to a Slice's issue.
    pub node_id: String,
}
