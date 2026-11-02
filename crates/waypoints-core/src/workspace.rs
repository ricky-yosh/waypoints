//! Persisted workspace references and read-only observations of their native state.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::jj::Revision;
use crate::storage::Node;

/// A dedicated native jj workspace allocated explicitly for a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkspaceAssociation {
    /// The original canonical workspace root used to access shared history.
    pub repository_path: PathBuf,
    /// The node's full change ID at allocation time; never silently retargeted.
    pub change_id: String,
    /// The generated native workspace name, persisted before invoking jj.
    pub name: String,
    /// The explicitly chosen absolute directory, with canonical parent.
    pub directory: PathBuf,
}

/// A read-only observation; ready does not snapshot files or update a stale checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkspaceState {
    /// No workspace has been requested for the node.
    Unallocated,
    /// Native creation did not finish with a confirmed successful checkout.
    Incomplete,
    /// The saved directory is absent; no replacement was created.
    MissingDirectory,
    /// The node, directory, or native registry no longer matches the saved reference.
    Mismatched {
        /// The discrepancy, without an automatic repair or reassociation.
        detail: String,
    },
    /// The registered workspace still edits this node's unique native change.
    Ready {
        /// Recorded native state, not unsnapshotted file contents.
        revision: Revision,
    },
}

/// A saved node, optional workspace allocation, and current native workspace state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NodeWorkspace {
    /// The node and its unchanged saved prompt.
    pub node: Node,
    /// The persisted allocation, retained even after external removal or switching.
    pub workspace: Option<WorkspaceAssociation>,
    /// Current state; repository access failures are returned as errors.
    pub state: WorkspaceState,
}

/// Whether an explicit stale-checkout update invoked native jj.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WorkspaceUpdateOutcome {
    /// The checkout already matched; no snapshot or native mutation was performed.
    AlreadyFresh,
    /// Native jj updated the checkout and postflight verified its identity and freshness.
    Updated,
}

/// A checked explicit update result, not permission to continue pre-restore sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkspaceUpdate {
    /// Whether native update-stale was needed.
    pub outcome: WorkspaceUpdateOutcome,
    /// The saved allocation and verified current revision after the request.
    pub workspace: NodeWorkspace,
}
