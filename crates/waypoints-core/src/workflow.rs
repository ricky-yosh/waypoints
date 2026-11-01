//! Explicit native starting points for workflows.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::jj::Revision;

/// An explicit bookmark or revision selector, resolved once when saving a base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum BaseSource {
    /// An exact local bookmark name, not a glob, revset, or remote-tracking bookmark.
    Bookmark {
        /// User-selected local bookmark name, such as main or feature/auth.
        name: String,
    },
    /// A commit/change ID or unique prefix, or the explicit native root().
    Revision {
        /// User-selected identifier, resolved to a full commit and change ID.
        id: String,
    },
}

/// A workflow's saved starting revision, independent of later bookmark movement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkflowBase {
    /// Canonical native workspace root used to resolve the selector.
    pub repository_path: PathBuf,
    /// Original selector, retained as context rather than a moving reference.
    pub source: BaseSource,
    /// Exact native revision observed at selection time; not refreshed automatically.
    pub revision: Revision,
}

/// Explicit initial dependency for a newly saved node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum NodeParent {
    /// Attach directly to the workflow's fixed native base.
    WorkflowBase {},
    /// Build on a saved upstream node in this workflow.
    Node {
        /// Existing upstream node identifier.
        node_id: i64,
    },
}

/// Saved connectivity to the explicit workflow base, not proof of native checkout freshness.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum GraphConnectivity {
    /// Legacy workflow has no selected base.
    #[default]
    Unbased,
    /// Every saved node is reachable from the selected base.
    Rooted,
    /// Existing nodes need explicit dependency adoption; none are inferred.
    NeedsAdoption {
        /// Nodes without a saved path to the workflow base.
        node_ids: Vec<i64>,
    },
}
