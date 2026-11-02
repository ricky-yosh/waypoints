//! Readonly discovery for an existing project folder and its local base choices.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::jj::{self, Revision};

/// A project discovery failure with the underlying native or filesystem diagnostic.
#[derive(Debug)]
pub struct RepositoryError(anyhow::Error);

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for RepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<anyhow::Error> for RepositoryError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}

/// An existing project and its recorded native bookmark choices; nothing is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RepositoryInspection {
    /// Canonical selected folder, retaining nested-project identity used by workflows.
    pub project_path: PathBuf,
    /// Canonical jj workspace root, matching saved workflow base repository paths.
    pub repository_path: PathBuf,
    /// Exact operation at which all bookmark targets were read.
    pub native_operation: String,
    /// Local bookmarks only, sorted by literal name; remote-only/deleted refs are omitted.
    pub bookmarks: Vec<LocalBookmark>,
}

/// A local bookmark with its complete native ref conflict, not a guessed target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LocalBookmark {
    /// Literal local name suitable for an explicit `BaseSource::Bookmark` selector.
    pub name: String,
    /// Whether the bookmark ref itself has a conflict (distinct from code conflicts).
    pub conflicted: bool,
    /// All positive target terms; an ordinary bookmark has exactly one.
    pub added_targets: Vec<BookmarkRevision>,
    /// All negative target terms retained for a conflicted bookmark.
    pub removed_targets: Vec<BookmarkRevision>,
}

/// An exact bookmark target and native change-divergence fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BookmarkRevision {
    /// Exact target, including description and unresolved code-conflict status.
    pub revision: Revision,
    /// Whether this target's change ID has multiple visible revisions.
    pub divergent: bool,
}

/// Inspects an existing jj project without snapshotting, importing Git, or writing metadata.
///
/// No default base is inferred. The returned paths are local canonical identities,
/// not portable repository UUIDs. Selection later resolves again using existing base rules.
///
/// # Errors
///
/// Returns an error for a missing/non-directory/non-jj project, concurrent native
/// operation heads, unsupported jj templates, timeout, or an oversized response.
pub async fn inspect(project: &Path) -> Result<RepositoryInspection, RepositoryError> {
    // Leave transport headroom inside the ordinary five-second RPC deadline.
    Ok(tokio::time::timeout(Duration::from_secs(4), async {
        anyhow::ensure!(project.is_absolute(), "project path must be absolute");
        let project_path = project
            .canonicalize()
            .context("resolving project directory")?;
        anyhow::ensure!(project_path.is_dir(), "project path must be a directory");
        let repository_path = jj::repository_root(&project_path).await?;
        let native_operation = jj::operation_id(&repository_path).await?;
        let bookmarks = jj::local_bookmarks(&repository_path, &native_operation).await?;
        let inspection = RepositoryInspection {
            project_path,
            repository_path,
            native_operation,
            bookmarks,
        };
        // Native JSON escaping and the RPC envelope must fit the existing 1 MiB frame.
        anyhow::ensure!(
            serde_json::to_vec(&inspection)?.len() <= 900 * 1024,
            "repository inspection exceeds the response limit; inspect bookmarks with native jj"
        );
        Ok(inspection)
    })
    .await
    .context("repository inspection timed out")??)
}
