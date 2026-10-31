//! Workspace allocation is reserved durably before any native or filesystem mutation.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;

use super::{Storage, StorageError};
use crate::jj::{self, ChangeState};
use crate::workspace::{NodeWorkspace, WorkspaceAssociation, WorkspaceState};

// Leave ten seconds for database writes and transport within the workspace RPC deadline.
const CREATION_TIMEOUT: Duration = Duration::from_secs(110);
const STATUS_TIMEOUT: Duration = Duration::from_secs(4);

struct SavedWorkspace {
    association: WorkspaceAssociation,
    ready: bool,
}

impl Storage {
    /// Allocates a dedicated jj workspace at an explicit, previously absent directory.
    ///
    /// The directory must be absolute with an existing parent, and outside registered
    /// workspaces. Existing allocations are reported, never recreated or switched.
    /// The original working copy is neither snapshotted nor updated. A durable
    /// incomplete allocation blocks replacement after any uncertain native outcome.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership, paths, missing/divergent/immutable
    /// changes, another workspace editing the change, native failures, or storage
    /// failures. Interrupted creation preserves its allocation and any native files;
    /// no automatic deletion, rollback, or retargeting is attempted.
    pub async fn create_node_workspace(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        directory: &Path,
    ) -> Result<NodeWorkspace, StorageError> {
        self.node(workflow_id, node_id).await?;
        let directory = normalized_destination(directory)?;
        if let Some(saved) = self.saved_workspace(node_id).await? {
            if directory != saved.association.directory {
                return Err(anyhow::anyhow!(
                    "node already has a workspace allocation at {}; refusing another directory",
                    saved.association.directory.display()
                )
                .into());
            }
            return self.node_workspace(workflow_id, node_id).await;
        }
        tokio::time::timeout(CREATION_TIMEOUT, async {
            let refreshed = self.refresh_node(workflow_id, node_id).await?;
            let ChangeState::Current { revision } = refreshed.state else {
                anyhow::bail!("workspace creation requires an associated, existing, non-divergent change");
            };
            let change = refreshed.association.context("node has no change association")?;
            ensure_absent(&directory)?;
            let workspaces = jj::workspaces(&change.repository_path).await?;
            for workspace in &workspaces {
                anyhow::ensure!(
                    workspace.change_id != change.change_id,
                    "change is already edited by workspace {}; use that workspace or select a different change",
                    workspace.name
                );
                if let Some(root) = &workspace.directory {
                    anyhow::ensure!(
                        !directory.starts_with(root) && !root.starts_with(&directory),
                        "destination must not overlap an existing jj workspace"
                    );
                }
            }
            anyhow::ensure!(
                !directory.starts_with(&change.repository_path),
                "destination must be outside the original workspace"
            );
            jj::ensure_mutable(&change.repository_path, &revision).await?;
            let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let association = WorkspaceAssociation {
                repository_path: change.repository_path,
                change_id: change.change_id,
                name: format!("waypoints-{node_id}-{}-{stamp:x}", std::process::id()),
                directory,
            };
            let connection = self.database.connect().context("connecting to workspace database")?;
            connection.execute(
                "INSERT INTO node_workspaces (node_id, repository_path, change_id, name, directory)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                turso::params![
                    node_id,
                    association.repository_path.to_str().context("repository path must be UTF-8")?,
                    association.change_id.as_str(),
                    association.name.as_str(),
                    association.directory.to_str().context("workspace path must be UTF-8")?
                ],
            ).await.context("reserving workspace allocation")?;
            // Recheck after reserving, so a path that became occupied is never adopted.
            fs::create_dir(&association.directory)
                .context("claiming workspace directory without overwriting an existing path")?;
            jj::create_workspace(&association, &revision).await.with_context(|| format!(
                "workspace creation incomplete at {} (native name {}); inspect jj and node workspace status; no files were removed",
                association.directory.display(), association.name
            ))?;
            connection.execute(
                "UPDATE node_workspaces SET ready = 1 WHERE node_id = ?1",
                [node_id],
            ).await.context("saving completed workspace allocation")?;
            Ok::<_, anyhow::Error>(())
        }).await.context(
            "workspace creation timed out; inspect node workspace status before taking further action"
        )??;
        // Observe rather than assume success if another native writer changed state.
        self.node_workspace(workflow_id, node_id).await
    }

    /// Observes a saved workspace without snapshotting files, updating, or repairing it.
    ///
    /// Ready means that the native registry and recorded working-copy change match;
    /// unsnapshotted edits and stale filesystem checkouts are left to native jj.
    /// Existing references survive missing directories, forgotten workspaces,
    /// external switches, and changed node associations.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership, inaccessible repositories/directories,
    /// native query failures, timeouts, or storage failures. These are not reported
    /// as missing directories.
    pub async fn node_workspace(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<NodeWorkspace, StorageError> {
        let node = self.node(workflow_id, node_id).await?;
        let Some(saved) = self.saved_workspace(node_id).await? else {
            return Ok(NodeWorkspace {
                node,
                workspace: None,
                state: WorkspaceState::Unallocated,
            });
        };
        let state = if !saved.ready {
            WorkspaceState::Incomplete
        } else {
            tokio::time::timeout(
                STATUS_TIMEOUT,
                self.observe_workspace(workflow_id, node_id, &saved.association),
            )
            .await
            .context("workspace status timed out")??
        };
        Ok(NodeWorkspace {
            node,
            workspace: Some(saved.association),
            state,
        })
    }

    async fn saved_workspace(&self, node_id: i64) -> Result<Option<SavedWorkspace>, StorageError> {
        let connection = self
            .database
            .connect()
            .context("connecting to workspace database")?;
        let mut rows = connection
            .query(
                "SELECT repository_path, change_id, name, directory, ready
             FROM node_workspaces WHERE node_id = ?1",
                [node_id],
            )
            .await
            .context("reading saved workspace")?;
        let Some(row) = rows.next().await.context("reading workspace row")? else {
            return Ok(None);
        };
        Ok(Some(SavedWorkspace {
            association: WorkspaceAssociation {
                repository_path: PathBuf::from(
                    row.get::<String>(0).context("reading repository path")?,
                ),
                change_id: row.get(1).context("reading workspace change ID")?,
                name: row.get(2).context("reading native workspace name")?,
                directory: PathBuf::from(
                    row.get::<String>(3)
                        .context("reading workspace directory")?,
                ),
            },
            ready: row.get::<i64>(4).context("reading creation status")? != 0,
        }))
    }

    async fn observe_workspace(
        &self,
        workflow_id: i64,
        node_id: i64,
        saved: &WorkspaceAssociation,
    ) -> anyhow::Result<WorkspaceState> {
        match fs::symlink_metadata(&saved.directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(WorkspaceState::MissingDirectory);
            }
            Err(error) => return Err(error).context("inspecting saved workspace directory"),
            Ok(metadata) if !metadata.is_dir() => {
                return Ok(mismatch(
                    "saved directory is no longer a directory (or is now a symlink)",
                ));
            }
            Ok(_) => {}
        }
        let refreshed = self.refresh_node(workflow_id, node_id).await?;
        if refreshed.association.as_ref().is_none_or(|association| {
            association.change_id != saved.change_id
                || association.repository_path != saved.repository_path
        }) {
            return Ok(mismatch(
                "node's change association no longer matches its saved workspace",
            ));
        }
        let ChangeState::Current { revision } = refreshed.state else {
            return Ok(mismatch(
                "node's change is missing or divergent; no workspace repair attempted",
            ));
        };
        let registry = jj::workspaces(&saved.repository_path).await?;
        let Some(native) = registry
            .iter()
            .find(|workspace| workspace.name == saved.name)
        else {
            return Ok(mismatch("native workspace was forgotten or renamed"));
        };
        if native.change_id != saved.change_id
            || native.directory.as_ref() != Some(&saved.directory)
        {
            return Ok(mismatch(
                "native workspace target or recorded directory changed",
            ));
        }
        if !saved.directory.join(".jj").is_dir() {
            return Ok(mismatch("saved directory is no longer a jj workspace"));
        }
        let root = jj::repository_root(&saved.directory).await?;
        if root != saved.directory {
            return Ok(mismatch(
                "saved directory resolves to another workspace root",
            ));
        }
        let current = jj::current_revision(&saved.directory).await?;
        if current.commit_id != revision.commit_id {
            return Ok(mismatch(
                "directory's recorded working-copy revision does not match the node",
            ));
        }
        Ok(WorkspaceState::Ready { revision: current })
    }
}

fn mismatch(detail: &str) -> WorkspaceState {
    WorkspaceState::Mismatched {
        detail: detail.to_owned(),
    }
}

fn normalized_destination(path: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(path.is_absolute(), "workspace directory must be absolute");
    let Some(Component::Normal(name)) = path.components().next_back() else {
        anyhow::bail!("workspace destination must name a new directory");
    };
    let parent = path
        .parent()
        .context("workspace destination needs a parent directory")?
        .canonicalize()
        .context("workspace parent directory must already exist")?;
    anyhow::ensure!(parent.is_dir(), "workspace parent must be a directory");
    anyhow::ensure!(
        !parent
            .ancestors()
            .any(|ancestor| ancestor.join(".jj").exists()),
        "workspace destination must be outside existing jj workspaces"
    );
    let destination = parent.join(name);
    destination
        .to_str()
        .context("workspace path must be UTF-8")?;
    Ok(destination)
}

fn ensure_absent(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("inspecting workspace destination"),
        Ok(_) => {
            anyhow::bail!("workspace destination already exists; refusing to overwrite or adopt it")
        }
    }
}
