use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use waypoints_core::jj::ChangeState;
use waypoints_core::storage::Storage;

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
        match selected.state {
            ChangeState::Current { revision } => assert!(revision.conflicted),
            state => panic!("expected conflicted current revision, got {state:?}"),
        }
        Ok(())
    })
}
