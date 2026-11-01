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
