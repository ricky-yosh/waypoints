use std::path::Path;

use anyhow::Context;

use super::{Storage, StorageError};
use crate::run::{
    InterruptionPhase, RecoveryConfirmation, RecoveryWorkspace, Run, RunRecovery, RunSnapshot,
    RunState, SnapshotState,
};
use crate::workspace::WorkspaceState;

impl Storage {
    pub(super) async fn initialize_runs(&self) -> anyhow::Result<()> {
        let connection = self.database.connect()?;
        connection
            .execute(
                "CREATE TABLE IF NOT EXISTS agent_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                node_id INTEGER NOT NULL REFERENCES nodes(id),
                directory TEXT NOT NULL,
                active INTEGER NOT NULL CHECK(active IN (0, 1)),
                record TEXT NOT NULL
            )",
                (),
            )
            .await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS active_run_node
             ON agent_runs(node_id) WHERE active = 1",
                (),
            )
            .await?;
        connection
            .execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS active_run_directory
             ON agent_runs(directory) WHERE active = 1",
                (),
            )
            .await?;
        Ok(())
    }

    /// Captures a verified assignment and durably reserves its node and workspace.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership, stale/unallocated workspace,
    /// an existing reservation, unsupported executable path, or storage failure.
    pub async fn create_run(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        executable: &Path,
    ) -> Result<Run, StorageError> {
        let run = self.prepare_run(workflow_id, node_id, executable).await?;
        validate_prompt(&run.prompt)?;
        Ok(self.reserve_run(run).await?)
    }

    /// Reserves a follow-up turn using a stopped run's harness-owned session.
    ///
    /// The node assignment is unchanged. The current workspace and change must
    /// match the selected run; continuation loads current harness history, not
    /// a historical snapshot or fork. Select the latest run for that session.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid prompt, active/interrupted source, absent
    /// session, superseded source, mismatched workspace/change, or storage/native failure.
    pub async fn continue_run(
        &mut self,
        source_run_id: i64,
        prompt: &str,
    ) -> Result<Run, StorageError> {
        Ok(async {
            validate_prompt(prompt)?;
            let source = self.run(source_run_id).await?;
            if let crate::run::ContinuationAssessment::Blocked { detail, .. } =
                self.assess_continuation(source_run_id).await?
            {
                anyhow::bail!("{detail}");
            }
            let session = source.session_id.as_ref().filter(|id| !id.is_empty())
                .context("selected run has no saved ACP session; explicitly start a fresh run instead")?;
            let mut run = self.prepare_run(source.workflow_id, source.node_id, &source.executable).await?;
            anyhow::ensure!(
                run.executable == source.executable,
                "saved ACP executable now resolves to a different path; continuation will not switch adapters"
            );
            anyhow::ensure!(
                run.directory == source.directory && run.change_id == source.change_id,
                "node workspace or change differs from the selected run; continuation will not retarget the session"
            );
            run.prompt = prompt.to_owned();
            run.session_id = Some(session.clone());
            run.continued_from = Some(source.id);
            self.reserve_run(run).await
        }.await?)
    }

    async fn prepare_run(
        &mut self,
        workflow_id: i64,
        node_id: i64,
        executable: &Path,
    ) -> anyhow::Result<Run> {
        self.node(workflow_id, node_id).await?;
        self.ensure_node_idle(node_id).await?;
        self.require_ready_workspace(node_id).await?;
        anyhow::ensure!(
            executable.is_absolute(),
            "ACP executable must be an absolute path"
        );
        let executable = executable
            .canonicalize()
            .context("resolving ACP executable")?;
        anyhow::ensure!(executable.is_file(), "ACP executable must be a file");
        let workspace = self.verify_node_workspace(workflow_id, node_id).await?;
        anyhow::ensure!(
            matches!(workspace.state, WorkspaceState::Ready { .. }),
            "workspace is not ready"
        );
        let allocation = workspace
            .workspace
            .context("node has no allocated workspace")?;
        Ok(Run {
            id: 0,
            workflow_id,
            node_id,
            prompt: workspace.node.prompt,
            directory: allocation.directory,
            change_id: allocation.change_id,
            executable,
            session_id: None,
            continued_from: None,
            process_id: None,
            state: RunState::Starting,
            snapshot: SnapshotState::NotRecorded,
            upstream_inputs: self.capture_upstream_inputs(workflow_id, node_id).await,
        })
    }

    /// Verifies a finalizing run's allocated workspace before native snapshotting.
    ///
    /// The returned operation has no database access and can run in a background
    /// task. The caller must retain the reservation until its outcome is saved.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-finalizing run, changed allocation/checkout,
    /// missing or divergent change, or native/storage failure.
    pub async fn prepare_run_snapshot(&mut self, id: i64) -> Result<RunSnapshot, StorageError> {
        Ok(async {
            let run = self.run(id).await?;
            anyhow::ensure!(
                matches!(run.state, RunState::Finalizing),
                "run is not finalizing"
            );
            self.verified_run_workspace(&run).await
        }
        .await?)
    }

    async fn verified_run_workspace(&mut self, run: &Run) -> anyhow::Result<RunSnapshot> {
        // Require already-ready so verification cannot acknowledge an incomplete
        // allocation as a side effect of inspection or recovery.
        self.require_ready_workspace(run.node_id).await?;
        let verified = self
            .verify_node_workspace(run.workflow_id, run.node_id)
            .await?;
        let workspace = verified
            .workspace
            .context("run has no workspace allocation")?;
        anyhow::ensure!(
            workspace.directory == run.directory && workspace.change_id == run.change_id,
            "run workspace or change no longer matches its saved allocation"
        );
        let WorkspaceState::Ready { revision } = verified.state else {
            anyhow::bail!("run workspace is not ready");
        };
        Ok(RunSnapshot {
            workspace,
            revision,
        })
    }

    /// Inspects interrupted/recovered metadata and native state without releasing ownership.
    ///
    /// Verification cannot establish that old processes stopped. No files, readiness
    /// flags, native history, run outcomes, or reservations are changed.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown/ineligible run or unreadable run metadata.
    /// Native verification failures are returned as blocked checks for inspection.
    pub async fn inspect_run_recovery(&mut self, id: i64) -> Result<RunRecovery, StorageError> {
        Ok(async {
            let run = self.run(id).await?;
            anyhow::ensure!(
                matches!(
                    run.state,
                    RunState::Interrupted { .. } | RunState::Recovered { .. }
                ),
                "recovery inspection requires an interrupted or recovered run"
            );
            let workspace = match self.verified_run_workspace(&run).await {
                Ok(verified) => RecoveryWorkspace::Verified {
                    current_revision: verified.revision,
                },
                Err(error) => RecoveryWorkspace::Blocked {
                    detail: format!("{error:#}"),
                },
            };
            Ok::<_, anyhow::Error>(RunRecovery { run, workspace })
        }
        .await?)
    }

    /// Releases an interrupted reservation after explicit attestation and fresh native checks.
    ///
    /// Retains interruption, session, and snapshot metadata without running an
    /// agent, signalling saved PIDs, repairing files, or attributing a revision.
    /// Repeating an already-completed recovery returns its original record, even
    /// if another run has since reserved the node.
    ///
    /// # Errors
    ///
    /// Returns an error for an ineligible/unreserved run, failed verification,
    /// concurrently changed record, or storage failure. Failures do not release ownership.
    pub async fn recover_run(
        &mut self,
        id: i64,
        _confirmation: RecoveryConfirmation,
    ) -> Result<Run, StorageError> {
        Ok(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query("SELECT record, active FROM agent_runs WHERE id = ?1", [id])
                .await?;
            let row = rows
                .next()
                .await?
                .ok_or(super::StorageFailure::MissingRun { run_id: id })?;
            let record: String = row.get(0)?;
            let active: i64 = row.get(1)?;
            drop(rows);
            let mut run: Run = serde_json::from_str(&record)?;
            run.id = id;
            if matches!(run.state, RunState::Recovered { .. }) {
                anyhow::ensure!(
                    active == 0,
                    "recovered run unexpectedly retains a reservation"
                );
                return Ok(run);
            }
            anyhow::ensure!(
                matches!(run.state, RunState::Interrupted { .. }) && active == 1,
                "only an interrupted run retaining its reservation can be recovered"
            );
            self.verified_run_workspace(&run).await?;
            let RunState::Interrupted { detail, phase } = run.state else {
                unreachable!("interruption checked before verification");
            };
            run.state = RunState::Recovered { detail, phase };
            let changed = connection
                .execute(
                    "UPDATE agent_runs SET record = ?1, active = 0
                 WHERE id = ?2 AND active = 1 AND record = ?3",
                    turso::params![serde_json::to_string(&run)?, id, record],
                )
                .await?;
            anyhow::ensure!(
                changed == 1,
                "run changed during recovery; inspect its current status before retrying"
            );
            Ok::<_, anyhow::Error>(run)
        }
        .await?)
    }

    async fn reserve_run(&mut self, mut run: Run) -> anyhow::Result<Run> {
        let connection = self.database.connect()?;
        connection
            .execute(
                "INSERT INTO agent_runs(node_id, directory, active, record) VALUES (?1, ?2, 1, ?3)",
                turso::params![
                    run.node_id,
                    run.directory
                        .to_str()
                        .context("workspace path must be UTF-8")?,
                    serde_json::to_string(&run)?
                ],
            )
            .await
            .context(
                "node or workspace may already be reserved; inspect run status before retrying",
            )?;
        run.id = connection.last_insert_rowid();
        self.save_run(&run).await?;
        Ok(run)
    }

    /// Reads a saved run, including after backend restarts.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown identifier, invalid saved record, or database failure.
    pub async fn run(&self, id: i64) -> Result<Run, StorageError> {
        Ok(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query("SELECT id, record FROM agent_runs WHERE id = ?1", [id])
                .await?;
            let row = rows
                .next()
                .await?
                .ok_or(super::StorageFailure::MissingRun { run_id: id })?;
            let mut run: Run = serde_json::from_str(&row.get::<String>(1)?)?;
            run.id = row.get(0)?;
            Ok::<_, anyhow::Error>(run)
        }
        .await?)
    }

    /// Lists a node's runs in identifier order, validating workflow membership.
    ///
    /// Historical runs remain inspectable after native node deletion.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid membership, invalid records, or database failure.
    pub async fn node_runs(
        &self,
        workflow_id: i64,
        node_id: i64,
    ) -> Result<Vec<Run>, StorageError> {
        Ok(async {
            let connection = self.database.connect()?;
            let mut membership = connection
                .query(
                    "SELECT id FROM nodes WHERE workflow_id = ?1 AND id = ?2",
                    [workflow_id, node_id],
                )
                .await?;
            anyhow::ensure!(
                membership.next().await?.is_some(),
                "node not found in workflow"
            );
            drop(membership);
            let mut rows = connection
                .query(
                    "SELECT id, record FROM agent_runs WHERE node_id = ?1 ORDER BY id",
                    [node_id],
                )
                .await?;
            let mut runs = Vec::new();
            while let Some(row) = rows.next().await? {
                let mut run: Run = serde_json::from_str(&row.get::<String>(1)?)?;
                run.id = row.get(0)?;
                runs.push(run);
            }
            Ok::<_, anyhow::Error>(runs)
        }
        .await?)
    }

    /// Saves lifecycle metadata, releasing reservations only for confirmed stopped runs.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or database writing fails.
    pub async fn save_run(&mut self, run: &Run) -> Result<(), StorageError> {
        Ok(async {
            let active = !matches!(
                run.state,
                RunState::Completed
                    | RunState::Cancelled
                    | RunState::Failed { .. }
                    | RunState::Recovered { .. }
            );
            let connection = self.database.connect()?;
            connection
                .execute(
                    "UPDATE agent_runs SET record = ?1, active = ?2 WHERE id = ?3",
                    turso::params![serde_json::to_string(run)?, i64::from(active), run.id],
                )
                .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await?)
    }

    /// Marks unfinished runs interrupted after acquiring exclusive backend ownership.
    ///
    /// Reservations remain: a backend crash cannot prove that external processes stopped.
    /// No prompt is automatically replayed.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid records or database failure.
    pub async fn interrupt_runs(&mut self) -> Result<(), StorageError> {
        let connection = self
            .database
            .connect()
            .context("connecting to run storage")?;
        let mut rows = connection
            .query("SELECT id FROM agent_runs WHERE active = 1", ())
            .await
            .context("reading unfinished runs")?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await.context("reading unfinished run")? {
            ids.push(row.get::<i64>(0).context("reading run identifier")?);
        }
        drop(rows);
        for id in ids {
            let mut run = self.run(id).await?;
            if matches!(run.state, RunState::Interrupted { .. }) {
                // Preserve the original phase and cause across repeated restarts.
                continue;
            }
            let (phase, detail) = if matches!(run.state, RunState::Finalizing) {
                (
                    InterruptionPhase::Finalization,
                    "backend restarted during finalization; native snapshot outcome was not durably recorded; inspect jj before explicit recovery",
                )
            } else {
                (
                    InterruptionPhase::Execution,
                    "backend restarted without confirmed cleanup; inspect native processes and workspace before explicit recovery",
                )
            };
            run.state = RunState::Interrupted {
                detail: detail.into(),
                phase,
            };
            self.save_run(&run).await?;
        }
        Ok(())
    }

    /// Refuses association changes while a run retains ownership of the node.
    ///
    /// # Errors
    ///
    /// Returns an error for a reserved node or a database failure.
    pub async fn ensure_node_idle(&self, node_id: i64) -> Result<(), StorageError> {
        self.ensure_dependency_operations_clear().await?;
        let connection = self
            .database
            .connect()
            .context("connecting to run storage")?;
        let mut rows = connection
            .query(
                "SELECT id FROM agent_runs WHERE node_id = ?1 AND active = 1",
                [node_id],
            )
            .await
            .context("checking node reservation")?;
        if let Some(row) = rows.next().await.context("checking active run")? {
            let id: i64 = row.get(0).context("reading active run identifier")?;
            return Err(anyhow::Error::new(super::StorageFailure::NodeBusy {
                node_id,
                run_id: id,
            })
            .into());
        }
        Ok(())
    }
}

fn validate_prompt(prompt: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!prompt.trim().is_empty(), "run prompt must not be blank");
    // Status and event pages include the captured prompt alongside bounded output.
    anyhow::ensure!(prompt.len() <= 128 * 1024, "run prompt exceeds 128 KiB");
    Ok(())
}
