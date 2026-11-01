use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use waypoints_core::jj::ChangeState;
use waypoints_core::storage::{
    DependencyOperationKind, DependencyOperationState, Node, Storage, UndoBoundaryAcknowledgement,
    Workflow,
};
use waypoints_core::workflow::{BaseSource, GraphConnectivity, NodeParent};
use waypoints_core::workspace::WorkspaceState;

#[path = "jj/multiple_parents.rs"]
mod multiple_parents;
#[path = "jj/planning.rs"]
mod planning;
#[path = "jj/restore.rs"]
mod restore;
#[path = "jj/revisions.rs"]
mod revisions;
#[path = "jj/upstream.rs"]
mod upstream;
#[path = "jj/workspace_deletion.rs"]
mod workspace_deletion;

struct Repository {
    directory: TempDir,
    project: PathBuf,
}

impl Repository {
    fn new() -> anyhow::Result<Self> {
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("project");
        let repo = Self { directory, project };
        repo.jj_at(repo.directory.path(), &["git", "init", "project"])?;
        Ok(repo)
    }

    fn jj_at(&self, path: &Path, args: &[&str]) -> anyhow::Result<String> {
        let output = Command::new("jj")
            .current_dir(path)
            .env("JJ_CONFIG", "")
            .args([
                "--config",
                "user.name='Waypoints test'",
                "--config",
                "user.email='test@example.invalid'",
                "--no-pager",
                "--color=never",
            ])
            .args(args)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "jj {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    }

    fn jj(&self, args: &[&str]) -> anyhow::Result<String> {
        self.jj_at(&self.project, args)
    }

    fn value(&self, template: &str) -> anyhow::Result<String> {
        self.jj(&["log", "--no-graph", "-r", "@", "-T", template])
    }

    fn operation(&self) -> anyhow::Result<String> {
        self.jj(&[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--no-graph",
            "--limit",
            "1",
            "-T",
            "id",
        ])
    }
}

fn run(test: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(test)
}

fn recorded_value(repo: &Repository, revision: &str, template: &str) -> anyhow::Result<String> {
    repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "log",
        "--no-graph",
        "-r",
        revision,
        "-T",
        template,
    ])
}

fn recorded_parents(repo: &Repository, revision: &str) -> anyhow::Result<Vec<String>> {
    let output = recorded_value(
        repo,
        revision,
        r#"parents.map(|p| p.change_id()).join("\n")"#,
    )?;
    let mut parents = output.lines().map(str::to_owned).collect::<Vec<_>>();
    parents.sort();
    Ok(parents)
}

async fn reparent_fixture(
    conflict: bool,
) -> anyhow::Result<(Repository, Storage, Workflow, Vec<Node>)> {
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
    let workflow = storage
        .create_workflow_from(
            &repo.project,
            "Reparent",
            &BaseSource::Revision {
                id: repo.value("commit_id")?,
            },
        )
        .await?;
    let mut parent = NodeParent::WorkflowBase {};
    let mut nodes = Vec::new();
    for name in ["interfaces", "authentication", "review", "followup"] {
        let node = storage.create_node_from(workflow.id, name, &parent).await?;
        let native = storage
            .create_node_change_from_dependency(workflow.id, node.id)
            .await?;
        let change = native.association.expect("created association").change_id;
        repo.jj(&["edit", &change])?;
        let file = if conflict && name == "review" {
            "authentication"
        } else {
            name
        };
        fs::write(
            repo.project.join(format!("{file}.txt")),
            format!("{name}\n"),
        )?;
        repo.jj(&["describe", "-m", name])?;
        parent = NodeParent::Node { node_id: node.id };
        nodes.push(node);
    }
    Ok((repo, storage, workflow, nodes))
}

#[test]
fn native_reparent_rebases_descendants_preserves_files_and_starts_new_undo_baseline()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
        let database = repo.directory.path().join("workflow.db");
        let original_checkout = repo.jj(&["--ignore-working-copy", "debug", "working-copy"])?;
        fs::write(repo.project.join("dirty.txt"), "not snapshotted\n")?;
        let previous = storage
            .history_status(&repo.project)
            .await?
            .undo
            .expect("history");
        let before_child = storage.refresh_node(workflow.id, nodes[3].id).await?;
        let operation = storage
            .reparent_node(
                workflow.id,
                nodes[2].id,
                &NodeParent::Node {
                    node_id: nodes[0].id,
                },
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert_eq!(operation.state, DependencyOperationState::Completed);
        assert_eq!(operation.before.len(), 2);
        assert_eq!(operation.after.len(), 2);
        assert!(operation.after.iter().all(|revision| !revision.conflicted));
        for old in &operation.before {
            let new = operation
                .after
                .iter()
                .find(|new| new.change_id == old.change_id)
                .expect("same change identity");
            assert_ne!(old.commit_id, new.commit_id);
        }
        assert_eq!(
            repo.jj(&["--ignore-working-copy", "debug", "working-copy"])?,
            original_checkout
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "not snapshotted\n"
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("authentication.txt"))?,
            "authentication\n"
        );
        let files = repo.jj(&[
            "--ignore-working-copy",
            "--at-op=@",
            "file",
            "list",
            "-r",
            &operation.after[0].change_id,
        ])?;
        assert!(!files.contains("authentication.txt"), "{files}");
        assert!(
            files.contains("interfaces.txt") && files.contains("review.txt"),
            "{files}"
        );
        assert!(!files.contains("dirty.txt"), "{files}");
        let graph = storage.workflow_graph(workflow.id).await?;
        assert!(
            graph
                .connections
                .iter()
                .any(|edge| edge.source_id == nodes[0].id && edge.target_id == nodes[2].id)
        );
        assert!(
            !graph
                .connections
                .iter()
                .any(|edge| edge.source_id == nodes[1].id && edge.target_id == nodes[2].id)
        );
        let new_child = storage.refresh_node(workflow.id, nodes[3].id).await?;
        assert_eq!(before_child.association, new_child.association);
        let status = storage.history_status(&repo.project).await?;
        assert!(status.undo.is_none() && status.redo.is_none());
        assert!(storage.undo(&repo.project, previous.id).await.is_err());
        storage
            .update_node(workflow.id, nodes[2].id, "New prompt")
            .await?;
        let edit = storage
            .history_status(&repo.project)
            .await?
            .undo
            .expect("new action");
        storage.undo(&repo.project, edit.id).await?;
        assert!(storage.history_status(&repo.project).await?.undo.is_none());
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert_eq!(
            storage.dependency_operations(workflow.id).await?,
            vec![operation]
        );
        Ok(())
    })
}

#[test]
fn native_reparent_conflicts_are_completed_outcomes_and_base_is_a_valid_destination()
-> anyhow::Result<()> {
    run(async {
        let (_repo, mut storage, workflow, nodes) = reparent_fixture(true).await?;
        let operation = storage
            .reparent_node(
                workflow.id,
                nodes[2].id,
                &NodeParent::WorkflowBase {},
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert_eq!(operation.state, DependencyOperationState::Completed);
        assert!(operation.after.iter().any(|revision| revision.conflicted));
        let graph = storage.workflow_graph(workflow.id).await?;
        assert!(graph.base_nodes.contains(&nodes[2].id));
        assert!(
            graph
                .connections
                .iter()
                .all(|edge| edge.target_id != nodes[2].id)
        );
        Ok(())
    })
}

#[test]
fn native_reparent_refusals_do_not_mutate_graph_native_state_or_history() -> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let native = repo.operation()?;
        for (parent, acknowledgement) in [
            (NodeParent::WorkflowBase {}, None),
            (
                NodeParent::Node {
                    node_id: nodes[3].id,
                },
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            ),
            (
                NodeParent::Node {
                    node_id: nodes[1].id,
                },
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            ),
            (
                NodeParent::Node { node_id: i64::MAX },
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            ),
        ] {
            assert!(
                storage
                    .reparent_node(workflow.id, nodes[2].id, &parent, acknowledgement)
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
            assert_eq!(storage.history_status(&repo.project).await?, history);
            assert_eq!(repo.operation()?, native);
            assert!(storage.dependency_operations(workflow.id).await?.is_empty());
        }
        let target = storage
            .refresh_node(workflow.id, nodes[2].id)
            .await?
            .association
            .expect("native change")
            .change_id;
        repo.jj(&[
            "--ignore-working-copy",
            "new",
            "--no-edit",
            &target,
            "-m",
            "External descendant",
        ])?;
        let native = repo.operation()?;
        let error = storage
            .reparent_node(
                workflow.id,
                nodes[2].id,
                &NodeParent::WorkflowBase {},
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await
            .expect_err("external descendants must not be silently changed");
        assert!(
            error.to_string().contains("native descendants differ"),
            "{error}"
        );
        assert_eq!(repo.operation()?, native);
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        Ok(())
    })
}

#[test]
fn native_reparent_rejects_external_ancestry_drift_and_immutable_work() -> anyhow::Result<()> {
    run(async {
        for immutable in [false, true] {
            let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
            let target = storage
                .refresh_node(workflow.id, nodes[2].id)
                .await?
                .association
                .expect("change")
                .change_id;
            if immutable {
                repo.jj(&["bookmark", "create", "protected", "-r", &target])?;
                repo.jj(&[
                    "config",
                    "set",
                    "--repo",
                    "revset-aliases.\"immutable_heads()\"",
                    "protected",
                ])?;
            } else {
                repo.jj(&[
                    "--ignore-working-copy",
                    "rebase",
                    "-s",
                    &target,
                    "-d",
                    &workflow.base.as_ref().expect("base").revision.commit_id,
                ])?;
            }
            let native = repo.operation()?;
            let graph = storage.workflow_graph(workflow.id).await?;
            let history = storage.history_status(&repo.project).await?;
            let error = storage
                .reparent_node(
                    workflow.id,
                    nodes[2].id,
                    &NodeParent::Node {
                        node_id: nodes[0].id,
                    },
                    Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
                )
                .await
                .expect_err("unsafe native state");
            assert!(
                error.to_string().contains(if immutable {
                    "immutable"
                } else {
                    "native parents"
                }),
                "{error}"
            );
            assert_eq!(repo.operation()?, native);
            assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
            assert_eq!(storage.history_status(&repo.project).await?, history);
            assert!(storage.dependency_operations(workflow.id).await?.is_empty());
        }
        Ok(())
    })
}

#[test]
fn native_deletion_reconnects_fanout_preserves_descendants_and_supports_roots_and_leaves()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
        let branch = storage
            .create_node_from(
                workflow.id,
                "Sibling",
                &NodeParent::Node {
                    node_id: nodes[1].id,
                },
            )
            .await?;
        storage
            .create_node_change_from_dependency(workflow.id, branch.id)
            .await?;
        let checkout = repo.jj(&["--ignore-working-copy", "debug", "working-copy"])?;
        fs::write(repo.project.join("dirty.txt"), "unrecorded\n")?;
        let source = storage
            .refresh_node(workflow.id, nodes[1].id)
            .await?
            .association
            .expect("native")
            .change_id;
        let operation = storage
            .delete_node(
                workflow.id,
                nodes[1].id,
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert_eq!(operation.kind, DependencyOperationKind::Delete);
        assert_eq!(operation.state, DependencyOperationState::Completed);
        assert_eq!(operation.after.len(), 3);
        assert_eq!(
            recorded_value(&repo, &format!("change_id(\"{source}\")"), "change_id")?,
            ""
        );
        let graph = storage.workflow_graph(workflow.id).await?;
        assert_eq!(graph.connectivity, GraphConnectivity::Rooted);
        assert!(graph.nodes.iter().all(|node| node.id != nodes[1].id));
        for child in [nodes[2].id, branch.id] {
            assert!(
                graph
                    .connections
                    .iter()
                    .any(|edge| edge.source_id == nodes[0].id && edge.target_id == child)
            );
            let native = storage
                .refresh_node(workflow.id, child)
                .await?
                .association
                .expect("native")
                .change_id;
            let files = repo.jj(&["--ignore-working-copy", "file", "list", "-r", &native])?;
            assert!(
                files.contains("interfaces.txt") && !files.contains("authentication.txt"),
                "{files}"
            );
        }
        assert!(storage.node(workflow.id, nodes[1].id).await.is_err());
        assert!(
            storage
                .update_node(workflow.id, nodes[1].id, "Cannot resurrect")
                .await
                .is_err()
        );
        assert!(
            storage
                .associate_node(workflow.id, nodes[1].id, &source)
                .await
                .is_err()
        );
        assert!(
            storage
                .create_node_from(
                    workflow.id,
                    "Cannot depend on deleted node",
                    &NodeParent::Node {
                        node_id: nodes[1].id
                    }
                )
                .await
                .is_err()
        );
        let history = storage.history_status(&repo.project).await?;
        assert!(history.undo.is_none() && history.redo.is_none());
        assert_eq!(
            repo.jj(&["--ignore-working-copy", "debug", "working-copy"])?,
            checkout
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "unrecorded\n"
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("authentication.txt"))?,
            "authentication\n"
        );
        let root_deletion = storage
            .delete_node(
                workflow.id,
                nodes[0].id,
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert_eq!(root_deletion.state, DependencyOperationState::Completed);
        assert_eq!(
            storage.workflow_graph(workflow.id).await?.base_nodes,
            vec![nodes[2].id, branch.id]
        );
        let leaf = storage
            .delete_node(
                workflow.id,
                branch.id,
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert!(leaf.after.is_empty());
        let database = repo.directory.path().join("workflow.db");
        let graph = storage.workflow_graph(workflow.id).await?;
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(storage.dependency_operations(workflow.id).await?.len(), 3);
        Ok(())
    })
}

#[test]
fn native_deletion_reports_child_conflicts_without_losing_change_identity() -> anyhow::Result<()> {
    run(async {
        let (_repo, mut storage, workflow, nodes) = reparent_fixture(true).await?;
        let operation = storage
            .delete_node(
                workflow.id,
                nodes[1].id,
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await?;
        assert_eq!(operation.state, DependencyOperationState::Completed);
        assert!(operation.after.iter().any(|revision| revision.conflicted));
        for revision in &operation.after {
            assert!(
                operation
                    .before
                    .iter()
                    .any(|old| old.change_id == revision.change_id)
            );
        }
        assert_eq!(
            storage.workflow_graph(workflow.id).await?.connectivity,
            GraphConnectivity::Rooted
        );
        Ok(())
    })
}

#[test]
fn native_deletion_refuses_missing_consent_external_descendants_and_unsaved_workspaces()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, nodes) = reparent_fixture(false).await?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let native = repo.operation()?;
        assert!(
            storage
                .delete_node(workflow.id, nodes[1].id, None)
                .await
                .is_err()
        );
        let error = storage
            .delete_node(
                workflow.id,
                nodes[3].id,
                Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary),
            )
            .await
            .expect_err("source is native @");
        assert!(error.to_string().contains("native workspace"), "{error}");
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert_eq!(repo.operation()?, native);
        let source = storage
            .refresh_node(workflow.id, nodes[1].id)
            .await?
            .association
            .expect("change")
            .change_id;
        repo.jj(&[
            "--ignore-working-copy",
            "new",
            "--no-edit",
            &source,
            "-m",
            "Outside workflow",
        ])?;
        let native = repo.operation()?;
        assert!(
            storage
                .delete_node(
                    workflow.id,
                    nodes[1].id,
                    Some(UndoBoundaryAcknowledgement::AcceptUndoBoundary)
                )
                .await
                .expect_err("external child")
                .to_string()
                .contains("native descendants differ")
        );
        assert_eq!(repo.operation()?, native);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert!(storage.dependency_operations(workflow.id).await?.is_empty());
        Ok(())
    })
}

#[test]
fn rooted_nodes_require_dependencies_and_undo_restores_the_whole_attachment() -> anyhow::Result<()>
{
    run(async {
        let repo = Repository::new()?;
        let database = repo.directory.path().join("workflow.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage
            .create_workflow_from(
                &repo.project,
                "Rooted",
                &BaseSource::Revision {
                    id: "root()".to_owned(),
                },
            )
            .await?;
        let unbased = storage.create_workflow(&repo.project, "Legacy").await?;
        let foreign = storage.create_node(unbased.id, "Foreign").await?;
        let operation = repo.operation()?;
        assert!(storage.create_node(workflow.id, "Orphan").await.is_err());
        assert!(
            storage
                .create_node_from(unbased.id, "No base", &NodeParent::WorkflowBase {})
                .await
                .is_err()
        );
        assert!(
            storage
                .create_node_from(
                    workflow.id,
                    "Wrong workflow",
                    &NodeParent::Node {
                        node_id: foreign.id
                    }
                )
                .await
                .is_err()
        );
        let root = storage
            .create_node_from(workflow.id, "Interfaces", &NodeParent::WorkflowBase {})
            .await?;
        let child = storage
            .create_node_from(
                workflow.id,
                "Authentication",
                &NodeParent::Node { node_id: root.id },
            )
            .await?;
        let graph = storage.workflow_graph(workflow.id).await?;
        assert_eq!(graph.connectivity, GraphConnectivity::Rooted);
        assert_eq!(graph.base_nodes, vec![root.id]);
        assert_eq!(graph.connections.len(), 1);
        assert_eq!(
            (
                graph.connections[0].source_id,
                graph.connections[0].target_id
            ),
            (root.id, child.id)
        );
        let edit = storage
            .history_status(&repo.project)
            .await?
            .undo
            .expect("child action");
        storage.undo(&repo.project, edit.id).await?;
        let removed = storage.workflow_graph(workflow.id).await?;
        assert_eq!(removed.nodes, vec![root]);
        assert!(removed.connections.is_empty());
        assert_eq!(removed.connectivity, GraphConnectivity::Rooted);
        drop(storage);
        let mut storage = Storage::open(&database).await?;
        storage.redo(&repo.project, edit.id).await?;
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert!(
            storage
                .connect_nodes(workflow.id, child.id, foreign.id)
                .await
                .is_err()
        );
        assert_eq!(
            repo.operation()?,
            operation,
            "saving/undoing dependencies must not modify native code"
        );
        Ok(())
    })
}

#[test]
fn rooted_native_creation_uses_saved_dependency_and_rejects_independent_base_choices()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Workflow base"])?;
        let commit = recorded_value(&repo, "@", "commit_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let workflow = storage
            .create_workflow_from(
                &repo.project,
                "Native rooted",
                &BaseSource::Revision { id: commit.clone() },
            )
            .await?;
        let root = storage
            .create_node_from(workflow.id, "Interfaces", &NodeParent::WorkflowBase {})
            .await?;
        let child = storage
            .create_node_from(
                workflow.id,
                "Authentication",
                &NodeParent::Node { node_id: root.id },
            )
            .await?;
        let before = repo.operation()?;
        assert!(
            storage
                .create_node_change_from_dependency(workflow.id, child.id)
                .await
                .is_err(),
            "upstream native change is required"
        );
        assert!(
            storage
                .create_node_change(workflow.id, root.id, "root()")
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, before);
        let native_root = storage
            .create_node_change_from_dependency(workflow.id, root.id)
            .await?;
        let root_revision = match native_root.state {
            ChangeState::Current { revision } => revision,
            state => panic!("{state:?}"),
        };
        assert_eq!(
            recorded_parents(&repo, &root_revision.change_id)?,
            vec![
                workflow
                    .base
                    .as_ref()
                    .expect("base")
                    .revision
                    .change_id
                    .clone()
            ]
        );
        let native_child = storage
            .create_node_change_from_dependency(workflow.id, child.id)
            .await?;
        let child_revision = match native_child.state {
            ChangeState::Current { revision } => revision,
            state => panic!("{state:?}"),
        };
        assert_eq!(
            recorded_parents(&repo, &child_revision.change_id)?,
            vec![root_revision.change_id]
        );
        assert_eq!(
            recorded_value(&repo, "@", "commit_id")?,
            commit,
            "original checkout is unchanged"
        );
        let graph = storage.workflow_graph(workflow.id).await?;
        let undo = storage
            .history_status(&repo.project)
            .await?
            .undo
            .expect("child creation");
        assert!(
            storage.undo(&repo.project, undo.id).await.is_err(),
            "native references must block metadata deletion"
        );
        assert_eq!(
            storage.workflow_graph(workflow.id).await?,
            graph,
            "failed undo restores the detached incoming edge too"
        );
        Ok(())
    })
}

#[test]
fn legacy_prompt_graph_adoption_is_explicit_complete_and_preserves_existing_edges()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Legacy").await?;
        let a = storage.create_node(workflow.id, "First root").await?;
        let b = storage.create_node(workflow.id, "Second root").await?;
        let child = storage.create_node(workflow.id, "Child").await?;
        storage.connect_nodes(workflow.id, a.id, child.id).await?;
        storage
            .set_workflow_base(
                workflow.id,
                &BaseSource::Revision {
                    id: "root()".to_owned(),
                },
            )
            .await?;
        let before = storage.workflow_graph(workflow.id).await?;
        assert!(matches!(
            before.connectivity,
            GraphConnectivity::NeedsAdoption { .. }
        ));
        let history = storage.history_status(&repo.project).await?;
        let operation = repo.operation()?;
        assert!(
            storage
                .create_node_from(
                    workflow.id,
                    "Do not extend broken graph",
                    &NodeParent::WorkflowBase {}
                )
                .await
                .is_err()
        );
        for roots in [vec![a.id], vec![a.id, child.id], vec![a.id, a.id, b.id]] {
            assert!(
                storage
                    .adopt_workflow_roots(workflow.id, &roots)
                    .await
                    .is_err()
            );
            assert_eq!(storage.workflow_graph(workflow.id).await?, before);
        }
        let adopted = storage
            .adopt_workflow_roots(workflow.id, &[a.id, b.id])
            .await?;
        assert_eq!(adopted.connectivity, GraphConnectivity::Rooted);
        assert_eq!(adopted.base_nodes, vec![a.id, b.id]);
        assert_eq!(adopted.nodes, before.nodes);
        assert_eq!(adopted.connections, before.connections);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        let edge_edit = history.undo.expect("legacy edge edit");
        assert!(
            storage.undo(&repo.project, edge_edit.id).await.is_err(),
            "legacy undo cannot orphan an adopted node"
        );
        assert_eq!(storage.workflow_graph(workflow.id).await?, adopted);
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn rooted_association_requires_matching_native_parents_and_unique_node_ownership()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Base"])?;
        let base = repo.value("commit_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let workflow = storage
            .create_workflow_from(
                &repo.project,
                "Associations",
                &BaseSource::Revision { id: base },
            )
            .await?;
        let first = storage
            .create_node_from(workflow.id, "First", &NodeParent::WorkflowBase {})
            .await?;
        let second = storage
            .create_node_from(workflow.id, "Second", &NodeParent::WorkflowBase {})
            .await?;
        let original_change = repo.value("change_id")?;
        assert!(
            storage
                .associate_node(workflow.id, first.id, &original_change)
                .await
                .is_err(),
            "the workflow base is not its own child"
        );
        repo.jj(&["new", "-m", "Correct child"])?;
        let candidate = repo.value("change_id")?;
        storage
            .associate_node(workflow.id, first.id, &candidate)
            .await?;
        let legacy = storage
            .create_workflow(&repo.project, "Legacy alias")
            .await?;
        let alias = storage
            .create_node(legacy.id, "Must not share rooted work")
            .await?;
        assert!(
            storage
                .associate_node(legacy.id, alias.id, &candidate)
                .await
                .is_err()
        );
        assert!(
            storage
                .associate_node(workflow.id, second.id, &candidate)
                .await
                .is_err(),
            "two nodes must not share this native change"
        );
        assert!(matches!(
            storage.refresh_node(workflow.id, second.id).await?.state,
            ChangeState::Unassociated
        ));
        Ok(())
    })
}

#[test]
fn legacy_native_graphs_are_not_silently_adopted() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let workflow = storage
            .create_workflow(&repo.project, "Existing native work")
            .await?;
        let node = storage
            .create_node(workflow.id, "Keep existing native work")
            .await?;
        let change = repo.value("change_id")?;
        storage
            .associate_node(workflow.id, node.id, &change)
            .await?;
        storage
            .set_workflow_base(
                workflow.id,
                &BaseSource::Revision {
                    id: "root()".to_owned(),
                },
            )
            .await?;
        let before = storage.workflow_graph(workflow.id).await?;
        assert!(
            storage
                .adopt_workflow_roots(workflow.id, &[node.id])
                .await
                .is_err()
        );
        assert_eq!(storage.workflow_graph(workflow.id).await?, before);
        assert_eq!(
            storage
                .refresh_node(workflow.id, node.id)
                .await?
                .association
                .expect("retained association")
                .change_id,
            change
        );
        Ok(())
    })
}

#[test]
fn workflow_base_pins_bookmark_without_snapshot_and_survives_movement_and_restart()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("baseline.txt"), "main baseline\n")?;
        repo.jj(&["describe", "-m", "Main baseline"])?;
        repo.jj(&["bookmark", "create", "main"])?;
        let commit = recorded_value(&repo, "@", "commit_id")?;
        let change = recorded_value(&repo, "@", "change_id")?;
        fs::write(repo.project.join("dirty.txt"), "Do not snapshot\n")?;
        let operation = repo.operation()?;
        let checkout = fs::read(repo.project.join(".jj/working_copy/tree_state"))?;
        let database = repo.directory.path().join("workflow.db");
        let mut storage = Storage::open(&database).await?;
        let source = BaseSource::Bookmark {
            name: "main".to_owned(),
        };
        let workflow = storage
            .create_workflow_from(&repo.project, "From main", &source)
            .await?;
        let base = workflow.base.as_ref().expect("saved base");
        assert_eq!(base.source, source);
        assert_eq!(base.revision.commit_id, commit);
        assert_eq!(base.revision.change_id, change);
        assert_eq!(base.repository_path, fs::canonicalize(&repo.project)?);
        assert_eq!(
            repo.operation()?,
            operation,
            "selection must not snapshot or write native history"
        );
        assert_eq!(
            fs::read(repo.project.join(".jj/working_copy/tree_state"))?,
            checkout
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "Do not snapshot\n"
        );
        repo.jj(&["new", "-m", "Main advanced"])?;
        repo.jj(&["bookmark", "set", "main", "-r", "@"])?;
        assert_ne!(recorded_value(&repo, "main", "commit_id")?, commit);
        assert!(
            storage
                .set_workflow_base(workflow.id, &source)
                .await
                .is_err()
        );
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert_eq!(storage.workflow(workflow.id).await?, workflow);
        assert_eq!(
            storage.list_workflows(&repo.project).await?,
            vec![workflow.clone()]
        );
        assert_eq!(
            storage.workflow_graph(workflow.id).await?.workflow,
            workflow
        );
        Ok(())
    })
}

#[test]
fn workflow_bases_accept_alternate_bookmarks_and_revision_ids_without_guessing()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Feature base"])?;
        repo.jj(&["bookmark", "create", "feature/auth"])?;
        let commit = recorded_value(&repo, "@", "commit_id")?;
        let change = recorded_value(&repo, "@", "change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        for source in [
            BaseSource::Bookmark {
                name: "feature/auth".to_owned(),
            },
            BaseSource::Revision { id: commit.clone() },
            BaseSource::Revision {
                id: commit[..12].to_owned(),
            },
            BaseSource::Revision { id: change },
        ] {
            let workflow = storage
                .create_workflow_from(&repo.project, "Alternate", &source)
                .await?;
            let base = workflow.base.expect("saved base");
            assert_eq!(base.source, source);
            assert_eq!(base.revision.commit_id, commit);
        }
        let before = storage.list_workflows(&repo.project).await?;
        let operation = repo.operation()?;
        for source in [
            BaseSource::Bookmark {
                name: "missing".to_owned(),
            },
            BaseSource::Bookmark {
                name: "feature/*".to_owned(),
            },
            BaseSource::Bookmark {
                name: "feature/auth\") | all() | bookmarks(\"".to_owned(),
            },
            BaseSource::Bookmark {
                name: " \n".to_owned(),
            },
            BaseSource::Revision { id: "@".to_owned() },
            BaseSource::Revision {
                id: "all()".to_owned(),
            },
            BaseSource::Revision {
                id: "0000000000000000000000000000000000000001".to_owned(),
            },
        ] {
            assert!(
                storage
                    .create_workflow_from(&repo.project, "Invalid", &source)
                    .await
                    .is_err(),
                "{source:?}"
            );
            assert_eq!(storage.list_workflows(&repo.project).await?, before);
        }
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn legacy_base_selection_is_one_time_metadata_only_and_preserves_graph_and_history()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["bookmark", "create", "main"])?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Legacy").await?;
        assert!(workflow.base.is_none());
        let node = storage.create_node(workflow.id, "Keep assignment").await?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let history = storage.history_status(&repo.project).await?;
        let operation = repo.operation()?;
        let source = BaseSource::Bookmark {
            name: "main".to_owned(),
        };
        assert!(storage.set_workflow_base(9999, &source).await.is_err());
        let based = storage.set_workflow_base(workflow.id, &source).await?;
        assert!(based.base.is_some());
        let after = storage.workflow_graph(workflow.id).await?;
        assert_eq!(after.nodes, graph.nodes);
        assert_eq!(after.connections, graph.connections);
        assert_eq!(storage.node(workflow.id, node.id).await?, node);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .set_workflow_base(workflow.id, &source)
                .await
                .is_err()
        );
        assert_eq!(storage.workflow(workflow.id).await?, based);
        Ok(())
    })
}

#[test]
fn workflow_base_selection_rejects_conflicted_bookmarks_without_saving() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["bookmark", "create", "main"])?;
        repo.jj(&["new", "-m", "Left"])?;
        let left = repo.value("change_id")?;
        repo.jj(&["new", "root()", "-m", "Right"])?;
        let right = repo.value("change_id")?;
        let operation = repo.operation()?;
        repo.jj(&["bookmark", "set", "main", "-r", &left])?;
        repo.jj(&[
            "--at-op",
            &operation,
            "bookmark",
            "set",
            "main",
            "-r",
            &right,
            "--allow-backwards",
        ])?;
        repo.jj(&["log", "--no-graph", "-r", "@", "-T", "commit_id"])?;
        let mut storage = Storage::open(&repo.directory.path().join("workflow.db")).await?;
        let error = storage
            .create_workflow_from(
                &repo.project,
                "Conflicted",
                &BaseSource::Bookmark {
                    name: "main".to_owned(),
                },
            )
            .await
            .expect_err("conflicted bookmark must fail");
        assert!(
            error.to_string().contains("missing or conflicted"),
            "{error}"
        );
        assert!(storage.list_workflows(&repo.project).await?.is_empty());
        Ok(())
    })
}

#[test]
fn native_abandon_reconnects_chain_and_removes_only_abandoned_contribution() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    fs::write(repo.project.join("interfaces.txt"), "interfaces\n")?;
    repo.jj(&["describe", "-m", "Interfaces"])?;
    let interfaces = repo.value("change_id")?;
    let base_commit = repo.value("commit_id")?;
    repo.jj(&["new", "-m", "Authentication"])?;
    fs::write(repo.project.join("authentication.txt"), "authentication\n")?;
    repo.jj(&["describe", "-m", "Authentication"])?;
    let authentication = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review"])?;
    fs::write(repo.project.join("review.txt"), "independent review work\n")?;
    repo.jj(&["describe", "-m", "Review"])?;
    let review = repo.value("change_id")?;
    let old_review_commit = repo.value("commit_id")?;

    repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "abandon",
        &authentication,
    ])?;
    assert_eq!(recorded_parents(&repo, &review)?, vec![interfaces.clone()]);
    assert_eq!(
        recorded_value(
            &repo,
            &format!("change_id(\"{authentication}\")"),
            "change_id"
        )?,
        ""
    );
    assert_ne!(
        recorded_value(&repo, &review, "commit_id")?,
        old_review_commit
    );
    assert_eq!(
        recorded_value(&repo, &interfaces, "commit_id")?,
        base_commit
    );
    let files = repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "file",
        "list",
        "-r",
        &review,
    ])?;
    assert_eq!(files, "interfaces.txt\nreview.txt\n");
    assert_eq!(recorded_value(&repo, &review, "conflict")?, "false");
    Ok(())
}

#[test]
fn native_abandon_rebases_all_children_not_only_selected_workflow_branch() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Shared upstream"])?;
    let upstream = repo.value("change_id")?;
    let mut children = Vec::new();
    for label in ["Authentication", "Images", "Outside this workflow"] {
        repo.jj(&["new", &upstream, "-m", label])?;
        children.push(repo.value("change_id")?);
    }
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &upstream])?;
    for child in children {
        assert_eq!(recorded_parents(&repo, &child)?, vec![base.clone()]);
    }
    Ok(())
}

#[test]
fn native_abandon_merge_reconnects_child_to_both_parents() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Images"])?;
    let images = repo.value("change_id")?;
    repo.jj(&["new", &auth, &images, "-m", "Combined"])?;
    let combined = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review"])?;
    let review = repo.value("change_id")?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &combined])?;
    let mut expected = vec![auth, images];
    expected.sort();
    assert_eq!(recorded_parents(&repo, &review)?, expected);
    Ok(())
}

#[test]
fn native_abandon_merge_parent_retains_inherited_parent_even_when_redundant() -> anyhow::Result<()>
{
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", &base, "-m", "Images"])?;
    let images = repo.value("change_id")?;
    repo.jj(&["new", &auth, &images, "-m", "Combined"])?;
    let combined = repo.value("change_id")?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &auth])?;
    // Native jj retains the inherited Base parent even though it is also an
    // ancestor of Images. The graph must reflect the actual native result.
    let mut expected = vec![base, images];
    expected.sort();
    assert_eq!(recorded_parents(&repo, &combined)?, expected);
    Ok(())
}

#[test]
fn native_abandon_records_dependent_child_conflicts_without_losing_identity() -> anyhow::Result<()>
{
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    let base = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Authentication"])?;
    fs::write(repo.project.join("auth.txt"), "authentication API\n")?;
    repo.jj(&["describe", "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review depends on authentication"])?;
    fs::write(
        repo.project.join("auth.txt"),
        "reviewed authentication API\n",
    )?;
    repo.jj(&["describe", "-m", "Review depends on authentication"])?;
    let review = repo.value("change_id")?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &auth])?;
    assert_eq!(recorded_parents(&repo, &review)?, vec![base]);
    assert_eq!(recorded_value(&repo, &review, "change_id")?, review);
    assert_eq!(recorded_value(&repo, &review, "conflict")?, "true");
    Ok(())
}

#[test]
fn native_abandon_does_not_remove_or_clean_a_workspace_editing_the_change() -> anyhow::Result<()> {
    let repo = Repository::new()?;
    repo.jj(&["describe", "-m", "Base"])?;
    repo.jj(&["new", "-m", "Authentication"])?;
    let auth = repo.value("change_id")?;
    repo.jj(&["new", "-m", "Review"])?;
    let workspace = repo.directory.path().join("auth-workspace");
    repo.jj(&[
        "--ignore-working-copy",
        "--at-op=@",
        "workspace",
        "add",
        "--name",
        "auth",
        "--no-colocate",
        "--sparse-patterns",
        "full",
        "-r",
        &auth,
        workspace.to_str().expect("workspace path"),
    ])?;
    repo.jj_at(
        &workspace,
        &["--ignore-working-copy", "--at-op=@", "edit", &auth],
    )?;
    repo.jj_at(&workspace, &["workspace", "update-stale"])?;
    fs::write(workspace.join("unrecorded.txt"), "Do not erase\n")?;
    let checkout = fs::read(workspace.join(".jj/working_copy/tree_state"))?;
    repo.jj(&["--ignore-working-copy", "--at-op=@", "abandon", &auth])?;
    assert!(workspace.is_dir());
    assert_eq!(
        fs::read_to_string(workspace.join("unrecorded.txt"))?,
        "Do not erase\n"
    );
    assert_eq!(
        fs::read(workspace.join(".jj/working_copy/tree_state"))?,
        checkout
    );
    let replacement = repo.jj_at(
        &workspace,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "change_id",
        ],
    )?;
    assert_ne!(
        replacement, auth,
        "jj gives an abandoned working copy a replacement change"
    );
    Ok(())
}

#[test]
fn workspace_verification_preserves_edits_and_rejects_stale_checkouts() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("base.txt"), "base\n")?;
        repo.jj(&["describe", "-m", "Base"])?;
        let base = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage
            .create_workflow(&repo.project, "Verification")
            .await?;
        let node = storage.create_node(workflow.id, "Keep prompt").await?;
        assert!(
            storage
                .verify_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        storage
            .create_node_change(workflow.id, node.id, &base)
            .await?;
        let path = repo.directory.path().join("editing");
        let workspace = storage
            .create_node_workspace(workflow.id, node.id, &path)
            .await?;
        // Unrelated repository operations do not make this checkout stale.
        let other = storage.create_node(workflow.id, "Independent").await?;
        storage
            .create_node_change(workflow.id, other.id, &base)
            .await?;
        fs::write(path.join("base.txt"), "legitimate local modification\n")?;
        fs::write(path.join("new.txt"), "not snapshotted\n")?;
        let operation = repo.operation()?;
        let checkout = fs::read(path.join(".jj/working_copy/checkout"))?;
        let tree = fs::read(path.join(".jj/working_copy/tree_state"))?;
        assert_eq!(
            storage.verify_node_workspace(workflow.id, node.id).await?,
            workspace
        );
        assert_eq!(repo.operation()?, operation);
        assert_eq!(fs::read(path.join(".jj/working_copy/checkout"))?, checkout);
        assert_eq!(fs::read(path.join(".jj/working_copy/tree_state"))?, tree);
        assert_eq!(
            fs::read_to_string(path.join("base.txt"))?,
            "legitimate local modification\n"
        );
        assert_eq!(
            fs::read_to_string(path.join("new.txt"))?,
            "not snapshotted\n"
        );
        // A local deletion is also legitimate; verification must not restore it.
        fs::remove_file(path.join("base.txt"))?;
        assert_eq!(
            storage.verify_node_workspace(workflow.id, node.id).await?,
            workspace
        );
        assert!(!path.join("base.txt").exists());
        let saved = workspace.workspace.expect("saved allocation");
        repo.jj(&[
            "--ignore-working-copy",
            "--at-op=@",
            "describe",
            "-r",
            &saved.change_id,
            "-m",
            "External rewrite",
        ])?;
        let operation = repo.operation()?;
        let before = storage.node_workspace(workflow.id, node.id).await?;
        let error = storage
            .verify_node_workspace(workflow.id, node.id)
            .await
            .expect_err("stale checkout must fail");
        assert!(
            error
                .to_string()
                .contains("checkout is unfinished or stale"),
            "{error}"
        );
        assert_eq!(storage.node_workspace(workflow.id, node.id).await?, before);
        assert_eq!(repo.operation()?, operation);
        assert_eq!(fs::read(path.join(".jj/working_copy/checkout"))?, checkout);
        assert_eq!(fs::read(path.join(".jj/working_copy/tree_state"))?, tree);
        assert!(!path.join("base.txt").exists());
        assert_eq!(
            fs::read_to_string(path.join("new.txt"))?,
            "not snapshotted\n"
        );
        assert!(storage.verify_node_workspace(99999, node.id).await.is_err());
        Ok(())
    })
}

#[test]
fn workspaces_edit_node_changes_independently_and_survive_restart() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("interfaces.txt"), "shared interface\n")?;
        repo.jj(&["describe", "-m", "Interfaces"])?;
        let base = repo.value("change_id")?;
        let original = repo.value("commit_id")?;
        let database = repo.directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage
            .create_workflow(&repo.project, "Parallel work")
            .await?;
        let auth = storage.create_node(workflow.id, "Authentication").await?;
        let images = storage.create_node(workflow.id, "Image uploads").await?;
        let auth_change = storage
            .create_node_change(workflow.id, auth.id, &base)
            .await?;
        let image_change = storage
            .create_node_change(workflow.id, images.id, &base)
            .await?;
        assert!(matches!(
            storage.node_workspace(workflow.id, auth.id).await?.state,
            WorkspaceState::Unallocated
        ));
        fs::write(repo.project.join("dirty.txt"), "original dirty contents\n")?;
        let count = repo.jj(&["--ignore-working-copy", "log", "-r", "all()", "--count"])?;
        let auth_path = repo.directory.path().join("auth");
        let images_path = repo.directory.path().join("images");
        let auth_workspace = storage
            .create_node_workspace(workflow.id, auth.id, &auth_path)
            .await?;
        let image_workspace = storage
            .create_node_workspace(workflow.id, images.id, &images_path)
            .await?;
        assert!(
            matches!(auth_workspace.state, WorkspaceState::Ready { .. }),
            "{auth_workspace:?}"
        );
        assert!(
            matches!(image_workspace.state, WorkspaceState::Ready { .. }),
            "{image_workspace:?}"
        );
        assert_eq!(auth_workspace.node, auth);
        assert_eq!(
            fs::read_to_string(auth_path.join("interfaces.txt"))?,
            "shared interface\n"
        );
        assert_eq!(
            fs::read_to_string(images_path.join("interfaces.txt"))?,
            "shared interface\n"
        );
        assert_eq!(
            repo.jj(&["--ignore-working-copy", "log", "-r", "all()", "--count"])?,
            count,
            "workspace allocation must not leave extra child changes"
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit_id"
            ])?,
            original
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "original dirty contents\n"
        );
        assert!(!auth_path.join("dirty.txt").exists());
        let operation = repo.operation()?;
        assert_eq!(
            storage
                .create_node_workspace(workflow.id, auth.id, &auth_path)
                .await?,
            auth_workspace
        );
        assert!(
            storage
                .create_node_workspace(workflow.id, auth.id, &repo.directory.path().join("other"))
                .await
                .is_err()
        );
        assert_eq!(
            repo.operation()?,
            operation,
            "repeat requests must be observational"
        );
        fs::write(
            auth_path.join("auth.txt"),
            "authentication implementation\n",
        )?;
        // Explicit native jj use records edits. Waypoints status/refresh never snapshots.
        repo.jj_at(&auth_path, &["status"])?;
        let updated = storage.refresh_node(workflow.id, auth.id).await?;
        assert_eq!(updated.association, auth_change.association);
        assert_ne!(updated.state, auth_change.state);
        assert_eq!(
            storage.refresh_node(workflow.id, images.id).await?,
            image_change
        );
        assert!(!images_path.join("auth.txt").exists());
        assert!(!repo.project.join("auth.txt").exists());
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit_id"
            ])?,
            original
        );
        let auth_status = storage.node_workspace(workflow.id, auth.id).await?;
        let operation = repo.operation()?;
        fs::write(auth_path.join("not-recorded.txt"), "keep dirty\n")?;
        assert_eq!(
            storage.node_workspace(workflow.id, auth.id).await?,
            auth_status
        );
        assert_eq!(repo.operation()?, operation);
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert_eq!(
            storage.node_workspace(workflow.id, auth.id).await?,
            auth_status
        );
        assert_eq!(
            storage.node_workspace(workflow.id, images.id).await?,
            image_workspace
        );
        Ok(())
    })
}

#[test]
fn workspace_creation_rejects_unsafe_destinations_and_shared_changes() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Validation").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let destination = repo.directory.path().join("new");
        assert!(
            storage
                .create_node_workspace(workflow.id, node.id, &destination)
                .await
                .is_err()
        );
        assert!(!destination.exists());
        let original = repo.value("change_id")?;
        storage
            .associate_node(workflow.id, node.id, &original)
            .await?;
        let operation = repo.operation()?;
        assert!(
            storage
                .create_node_workspace(workflow.id, node.id, &destination)
                .await
                .is_err(),
            "must not edit the original workspace's change"
        );
        assert_eq!(repo.operation()?, operation);
        let node = storage.create_node(workflow.id, "Dedicated").await?;
        storage
            .create_node_change(workflow.id, node.id, "root()")
            .await?;
        let operation_after_setup = repo.operation()?;
        let occupied = repo.directory.path().join("occupied");
        fs::create_dir(&occupied)?;
        let file = repo.directory.path().join("file");
        fs::write(&file, "preserved")?;
        for invalid in [
            occupied.clone(),
            file.clone(),
            repo.project.join("nested"),
            repo.directory.path().join("missing-parent").join("new"),
            PathBuf::from("relative"),
        ] {
            assert!(
                storage
                    .create_node_workspace(workflow.id, node.id, &invalid)
                    .await
                    .is_err(),
                "{invalid:?}"
            );
        }
        assert!(
            storage
                .create_node_workspace(99999, node.id, &destination)
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(file)?, "preserved");
        assert_eq!(fs::read_dir(occupied)?.count(), 0);
        #[cfg(unix)]
        {
            let link = repo.directory.path().join("dangling");
            std::os::unix::fs::symlink(repo.directory.path().join("absent"), &link)?;
            assert!(
                storage
                    .create_node_workspace(workflow.id, node.id, &link)
                    .await
                    .is_err()
            );
            assert!(fs::symlink_metadata(link)?.file_type().is_symlink());
        }
        let immutable = storage.create_node(workflow.id, "Immutable").await?;
        storage
            .associate_node(
                workflow.id,
                immutable.id,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            )
            .await?;
        assert!(
            storage
                .create_node_workspace(workflow.id, immutable.id, &destination)
                .await
                .is_err()
        );
        assert!(matches!(
            storage
                .node_workspace(workflow.id, immutable.id)
                .await?
                .state,
            WorkspaceState::Unallocated
        ));
        assert!(!destination.exists());
        assert_eq!(repo.operation()?, operation_after_setup);
        assert_ne!(operation_after_setup, operation);
        assert!(matches!(
            storage.node_workspace(workflow.id, node.id).await?.state,
            WorkspaceState::Unallocated
        ));
        storage
            .create_node_workspace(workflow.id, node.id, &destination)
            .await?;
        Ok(())
    })
}

#[test]
fn workspace_status_retains_missing_forgotten_and_switched_allocations() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage
            .create_workflow(&repo.project, "External changes")
            .await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        storage
            .create_node_change(workflow.id, node.id, "root()")
            .await?;
        let path = repo.directory.path().join("workspace");
        let workspace = storage
            .create_node_workspace(workflow.id, node.id, &path)
            .await?;
        repo.jj_at(&path, &["new"])?;
        let operation = repo.operation()?;
        let switched = storage.node_workspace(workflow.id, node.id).await?;
        assert!(matches!(switched.state, WorkspaceState::Mismatched { .. }));
        assert_eq!(switched.workspace, workspace.workspace);
        assert!(
            storage
                .verify_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        assert_eq!(
            storage
                .create_node_workspace(workflow.id, node.id, &path)
                .await?,
            switched
        );
        assert_eq!(repo.operation()?, operation);
        let saved = workspace.workspace.expect("saved workspace");
        let original = repo.jj(&[
            "--ignore-working-copy",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "change_id",
        ])?;
        storage
            .associate_node(workflow.id, node.id, &original)
            .await?;
        let reassociated = storage.node_workspace(workflow.id, node.id).await?;
        assert!(matches!(
            reassociated.state,
            WorkspaceState::Mismatched { .. }
        ));
        assert_eq!(reassociated.workspace.as_ref(), Some(&saved));
        storage
            .associate_node(workflow.id, node.id, &saved.change_id)
            .await?;
        repo.jj(&["--ignore-working-copy", "workspace", "forget", &saved.name])?;
        assert!(matches!(
            storage.node_workspace(workflow.id, node.id).await?.state,
            WorkspaceState::Mismatched { .. }
        ));
        let moved = repo.directory.path().join("moved");
        fs::rename(&path, &moved)?;
        let missing = storage.node_workspace(workflow.id, node.id).await?;
        assert!(matches!(missing.state, WorkspaceState::MissingDirectory));
        assert!(
            storage
                .verify_node_workspace(workflow.id, node.id)
                .await
                .is_err()
        );
        assert_eq!(missing.workspace, Some(saved));
        assert_eq!(
            storage
                .create_node_workspace(workflow.id, node.id, &path)
                .await?,
            missing
        );
        assert!(
            !path.exists(),
            "repeat must not recreate a removed directory"
        );
        Ok(())
    })
}

#[test]
fn creation_preserves_working_copy_and_persists_explicit_sibling_ancestry() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("base.txt"), "base\n")?;
        let base = repo.value("change_id")?;
        let base_commit = repo.value("commit_id")?;
        repo.jj(&["new", &base])?;
        let working_copy = repo.value("commit_id")?;
        fs::write(repo.project.join("dirty.txt"), "not snapshotted\n")?;
        let database = repo.directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage.create_workflow(&repo.project, "Create").await?;
        let first = storage
            .create_node(workflow.id, "Authentication\n日本語")
            .await?;
        let second = storage.create_node(workflow.id, "Images").await?;
        storage
            .connect_nodes(workflow.id, first.id, second.id)
            .await?;
        let created = storage
            .create_node_change(workflow.id, first.id, &base[..12])
            .await?;
        assert_eq!(created.node, first);
        let id = &created.association.as_ref().expect("association").change_id;
        assert_ne!(id, &base);
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                "@",
                "-T",
                "commit_id"
            ])?,
            working_copy
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "not snapshotted\n"
        );
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                id,
                "-T",
                "parents.map(|p| p.commit_id()).join(\" \") ++ \" \" ++ empty"
            ])?,
            format!("{base_commit} true")
        );
        let sibling = storage
            .create_node_change(workflow.id, second.id, &base_commit)
            .await?;
        let sibling_id = &sibling
            .association
            .as_ref()
            .expect("sibling association")
            .change_id;
        assert_eq!(
            repo.jj(&[
                "--ignore-working-copy",
                "log",
                "--no-graph",
                "-r",
                sibling_id,
                "-T",
                "parents.map(|p| p.commit_id()).join(\" \")"
            ])?,
            base_commit
        );
        let operation = repo.operation()?;
        assert!(
            storage
                .create_node_change(workflow.id, first.id, &base)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        drop(storage);
        let mut storage = Storage::open(&database).await?;
        assert_eq!(storage.refresh_node(workflow.id, first.id).await?, created);
        repo.jj(&["--ignore-working-copy", "abandon", id])?;
        let operation = repo.operation()?;
        assert!(
            storage
                .create_node_change(workflow.id, first.id, &base)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        assert!(matches!(
            storage.refresh_node(workflow.id, first.id).await?.state,
            ChangeState::Missing
        ));
        Ok(())
    })
}

#[test]
fn creation_rejects_invalid_bases_and_membership_before_mutating_jj() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Validation").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let operation = repo.operation()?;
        for base in [
            "",
            "@",
            "all()",
            "--help",
            "x | root()",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        ] {
            assert!(
                storage
                    .create_node_change(workflow.id, node.id, base)
                    .await
                    .is_err(),
                "{base}"
            );
            assert_eq!(repo.operation()?, operation);
        }
        assert!(
            storage
                .create_node_change(99999, node.id, "root()")
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        storage
            .create_node_change(workflow.id, node.id, "root()")
            .await?;
        Ok(())
    })
}

#[test]
fn associations_follow_rewrites_survive_restart_and_retain_missing_nodes() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Initial description"])?;
        let id = repo.value("change_id")?;
        let database = repo.directory.path().join("workflows.db");
        let mut storage = Storage::open(&database).await?;
        // A project can be a subdirectory of a jj workspace.
        let project = repo.project.join("nested");
        fs::create_dir(&project)?;
        let workflow = storage
            .create_workflow(&project, "Native association")
            .await?;
        let other_workflow = storage.create_workflow(&project, "Independent").await?;
        let node = storage.create_node(workflow.id, "Keep this prompt").await?;
        assert!(matches!(
            storage.refresh_node(workflow.id, node.id).await?.state,
            ChangeState::Unassociated
        ));
        // A dirty file must not be snapshotted by association or refresh.
        fs::write(repo.project.join("unsnapshotted.txt"), "dirty contents")?;
        let operation = repo.operation()?;
        let selected = storage
            .associate_node(workflow.id, node.id, &id[..12])
            .await?;
        assert_eq!(
            selected
                .association
                .as_ref()
                .expect("saved association")
                .change_id,
            id
        );
        assert_eq!(
            selected
                .association
                .as_ref()
                .expect("saved association")
                .repository_path,
            repo.project.canonicalize()?
        );
        assert_eq!(
            repo.operation()?,
            operation,
            "selection must not snapshot or write an operation"
        );
        let initial_commit = match selected.state {
            ChangeState::Current { revision } => {
                assert_eq!(revision.description, "Initial description\n");
                assert!(!revision.conflicted);
                revision.commit_id
            }
            state => panic!("expected current, got {state:?}"),
        };
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?.node, node);
        assert_eq!(repo.operation()?, operation);

        repo.jj(&[
            "describe",
            "-m",
            "Updated\nwith \"quotes\" and unicode 日本語",
        ])?;
        let operation = repo.operation()?;
        let refreshed = storage.refresh_node(workflow.id, node.id).await?;
        match &refreshed.state {
            ChangeState::Current { revision } => {
                assert_ne!(revision.commit_id, initial_commit);
                assert_eq!(revision.change_id, id);
                assert_eq!(
                    revision.description,
                    "Updated\nwith \"quotes\" and unicode 日本語\n"
                );
            }
            state => panic!("expected current, got {state:?}"),
        }
        assert_eq!(repo.operation()?, operation);
        assert!(
            storage
                .refresh_node(other_workflow.id, node.id)
                .await
                .is_err()
        );
        assert!(
            storage
                .associate_node(other_workflow.id, node.id, &id)
                .await
                .is_err()
        );
        for invalid in [
            "@",
            "all()",
            "--help",
            "",
            "abcdef",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzy",
        ] {
            assert!(
                storage
                    .associate_node(workflow.id, node.id, invalid)
                    .await
                    .is_err(),
                "{invalid:?}"
            );
            assert_eq!(storage.refresh_node(workflow.id, node.id).await?, refreshed);
        }
        drop(storage);
        let mut storage = Storage::open(&database).await?;
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?, refreshed);
        repo.jj(&["abandon", "-r", &id])?;
        let operation = repo.operation()?;
        let missing = storage.refresh_node(workflow.id, node.id).await?;
        assert!(matches!(missing.state, ChangeState::Missing));
        assert_eq!(missing.association, refreshed.association);
        assert_eq!(missing.node, node);
        assert_eq!(repo.operation()?, operation);
        assert_eq!(
            storage.workflow_graph(workflow.id).await?.nodes,
            vec![node.clone()]
        );
        assert!(
            storage
                .associate_node(workflow.id, node.id, &id)
                .await
                .is_err()
        );
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?, missing);
        let replacement = repo.value("change_id")?;
        let replaced = storage
            .associate_node(workflow.id, node.id, &replacement)
            .await?;
        assert_eq!(
            replaced.association.expect("new association").change_id,
            replacement
        );
        assert_eq!(replaced.node, node);
        Ok(())
    })
}

#[test]
fn repository_errors_are_not_missing_changes_and_do_not_destroy_associations() -> anyhow::Result<()>
{
    run(async {
        let repo = Repository::new()?;
        let id = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Repo").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let selected = storage.associate_node(workflow.id, node.id, &id).await?;
        let moved = repo.directory.path().join("moved-project");
        fs::rename(&repo.project, &moved)?;
        assert!(storage.refresh_node(workflow.id, node.id).await.is_err());
        fs::rename(&moved, &repo.project)?;
        assert_eq!(storage.refresh_node(workflow.id, node.id).await?, selected);
        let plain = repo.directory.path().join("plain");
        fs::create_dir(&plain)?;
        let workflow = storage.create_workflow(&plain, "No jj").await?;
        let node = storage.create_node(workflow.id, "Unassociated").await?;
        assert!(matches!(
            storage.refresh_node(workflow.id, node.id).await?.state,
            ChangeState::Unassociated
        ));
        assert!(
            storage
                .create_node_change(workflow.id, node.id, "root()")
                .await
                .is_err()
        );
        assert!(
            storage
                .associate_node(workflow.id, node.id, &id)
                .await
                .is_err()
        );
        assert!(matches!(
            storage.refresh_node(workflow.id, node.id).await?.state,
            ChangeState::Unassociated
        ));
        Ok(())
    })
}

#[test]
fn review_reads_recorded_changes_and_leaves_dirty_files_and_history_untouched() -> anyhow::Result<()>
{
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("modified.txt"), "before\n")?;
        fs::write(repo.project.join("deleted.txt"), "remove me\n")?;
        repo.jj(&["new", "-m", "Review this revision"])?;
        fs::write(repo.project.join("modified.txt"), "recorded\n")?;
        fs::remove_file(repo.project.join("deleted.txt"))?;
        fs::write(repo.project.join("space name.txt"), "added\n")?;
        fs::write(repo.project.join("binary.bin"), [0, 1, 2, 255])?;
        let id = repo.value("change_id")?;
        let database = repo.directory.path().join("review.db");
        let mut storage = Storage::open(&database).await?;
        let workflow = storage.create_workflow(&repo.project, "Review").await?;
        let node = storage.create_node(workflow.id, "Review").await?;
        assert!(storage.review_node(workflow.id, node.id).await.is_err());
        storage.associate_node(workflow.id, node.id, &id).await?;
        let history = storage.history_status(&repo.project).await?;
        let graph = storage.workflow_graph(workflow.id).await?;
        let operation = repo.operation()?;
        let commit = recorded_value(&repo, "@", "commit_id")?;
        fs::write(repo.project.join("modified.txt"), "unsnapshotted\n")?;
        fs::write(repo.project.join("dirty.txt"), "unrecorded\n")?;
        let review = storage.review_node(workflow.id, node.id).await?;
        assert_eq!(review.revision.commit_id, commit);
        assert_eq!(review.parent_commit_ids.len(), 1);
        assert!(
            review.summary.contains("modified.txt"),
            "{}",
            review.summary
        );
        assert!(review.summary.contains("deleted.txt"), "{}", review.summary);
        assert!(
            review.summary.contains("space name.txt"),
            "{}",
            review.summary
        );
        assert!(review.summary.contains("binary.bin"), "{}", review.summary);
        assert!(review.patch.contains("+recorded"), "{}", review.patch);
        assert!(!review.patch.contains("unsnapshotted"));
        assert!(!review.summary.contains("dirty.txt"));
        assert_eq!(repo.operation()?, operation);
        assert_eq!(recorded_value(&repo, "@", "commit_id")?, commit);
        assert_eq!(
            fs::read_to_string(repo.project.join("modified.txt"))?,
            "unsnapshotted\n"
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "unrecorded\n"
        );
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        drop(storage);
        let storage = Storage::open(&database).await?;
        assert_eq!(storage.review_node(workflow.id, node.id).await?, review);
        repo.jj(&["--ignore-working-copy", "abandon", &id])?;
        assert!(storage.review_node(workflow.id, node.id).await.is_err());
        Ok(())
    })
}

#[test]
fn review_of_an_empty_revision_is_empty_and_large_patches_fail_without_mutation()
-> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        let id = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("review.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Review").await?;
        let node = storage.create_node(workflow.id, "Review").await?;
        storage.associate_node(workflow.id, node.id, &id).await?;
        let empty = storage.review_node(workflow.id, node.id).await?;
        assert!(empty.summary.is_empty());
        assert!(empty.patch.is_empty());
        // JSON escaping alone pushes this otherwise bounded native patch over the RPC budget.
        fs::write(repo.project.join("large.txt"), "\t".repeat(500_000))?;
        repo.value("commit_id")?;
        let operation = repo.operation()?;
        let error = storage
            .review_node(workflow.id, node.id)
            .await
            .expect_err("oversized review");
        assert!(error.to_string().contains("response limit"), "{error}");
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn refresh_reports_divergence_without_selecting_a_revision() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Original"])?;
        let id = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Divergence").await?;
        let node = storage.create_node(workflow.id, "Prompt").await?;
        let selected = storage.associate_node(workflow.id, node.id, &id).await?;
        let operation = repo.operation()?;
        repo.jj(&["describe", "-r", &id, "-m", "First rewrite"])?;
        repo.jj(&[
            "--at-op",
            &operation,
            "describe",
            "-r",
            &id,
            "-m",
            "Second rewrite",
        ])?;
        // This explicit native operation integrates the two concurrent operation heads.
        repo.jj(&["status"])?;
        let operation = repo.operation()?;
        let refreshed = storage.refresh_node(workflow.id, node.id).await?;
        assert_eq!(refreshed.association, selected.association);
        assert!(storage.review_node(workflow.id, node.id).await.is_err());
        match refreshed.state {
            ChangeState::Divergent { revisions } => {
                assert_eq!(revisions.len(), 2);
                assert!(revisions.iter().all(|revision| revision.change_id == id));
                assert_ne!(revisions[0].commit_id, revisions[1].commit_id);
            }
            state => panic!("expected divergence, got {state:?}"),
        }
        assert!(
            storage
                .associate_node(workflow.id, node.id, &id)
                .await
                .is_err()
        );
        let unassociated = storage
            .create_node(workflow.id, "Choose an exact base")
            .await?;
        assert!(
            storage
                .create_node_change(workflow.id, unassociated.id, &id)
                .await
                .is_err()
        );
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}

#[test]
fn refresh_reads_native_conflict_status() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        fs::write(repo.project.join("file.txt"), "base\n")?;
        let base = repo.value("change_id")?;
        repo.jj(&["new", &base])?;
        fs::write(repo.project.join("file.txt"), "left\n")?;
        let left = repo.value("change_id")?;
        repo.jj(&["new", &base])?;
        fs::write(repo.project.join("file.txt"), "right\n")?;
        let right = repo.value("change_id")?;
        repo.jj(&["new", &left, &right])?;
        let merge = repo.value("change_id")?;
        let mut storage = Storage::open(&repo.directory.path().join("workflows.db")).await?;
        let workflow = storage.create_workflow(&repo.project, "Conflict").await?;
        let node = storage.create_node(workflow.id, "Resolve later").await?;
        let selected = storage.associate_node(workflow.id, node.id, &merge).await?;
        let operation = repo.operation()?;
        let review = storage.review_node(workflow.id, node.id).await?;
        assert!(review.revision.conflicted);
        assert_eq!(review.parent_commit_ids.len(), 2);
        assert_eq!(repo.operation()?, operation);
        match selected.state {
            ChangeState::Current { revision } => assert!(revision.conflicted),
            state => panic!("expected conflicted current revision, got {state:?}"),
        }
        Ok(())
    })
}
