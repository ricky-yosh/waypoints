use super::*;
use std::collections::BTreeSet;
use waypoints_core::storage::NodeRevisionEntry;

async fn fixture() -> anyhow::Result<(Repository, Storage, Workflow, Node, String)> {
    let repo = Repository::new()?;
    let mut storage = Storage::open(&repo.directory.path().join("revisions.db")).await?;
    let workflow = storage.create_workflow(&repo.project, "History").await?;
    let node = storage.create_node(workflow.id, "Task").await?;
    let change = repo.value("change_id")?;
    storage
        .associate_node(workflow.id, node.id, &change)
        .await?;
    Ok((repo, storage, workflow, node, change))
}

async fn collect(
    storage: &Storage,
    workflow: i64,
    node: i64,
    size: usize,
) -> anyhow::Result<Vec<NodeRevisionEntry>> {
    let mut cursor = None;
    let mut entries = Vec::new();
    for _ in 0..100 {
        let page = storage
            .node_revisions(workflow, node, size, cursor.as_deref())
            .await?;
        assert!(page.entries.len() <= size);
        entries.extend(page.entries);
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(entries);
        }
    }
    anyhow::bail!("pagination did not terminate");
}

#[test]
fn native_history_pages_are_pinned_readonly_restartable_and_include_manual_rewrites()
-> anyhow::Result<()> {
    run(async {
        let (repo, storage, workflow, node, change) = fixture().await?;
        for index in 0..14 {
            fs::write(repo.project.join("work.txt"), format!("revision {index}\n"))?;
            repo.jj(&["describe", "-m", &format!("manual {index}")])?;
        }
        let expected = repo
            .jj(&[
                "--ignore-working-copy",
                "evolog",
                "--no-graph",
                "-r",
                &change,
                "-T",
                "commit.commit_id() ++ \"\\n\"",
            ])?
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let first = storage
            .node_revisions(workflow.id, node.id, 3, None)
            .await?;
        assert_eq!(first.entries.len(), 3);
        assert!(first.entries[0].is_current);
        assert!(first.entries.iter().all(|entry| entry.run_ids.is_empty()));
        let cursor = first.next_cursor.clone().expect("more pages");
        let before_rewrite = storage
            .node_revisions(workflow.id, node.id, 2, Some(&cursor))
            .await?;
        fs::write(repo.project.join("work.txt"), "newer than pinned history\n")?;
        repo.jj(&["describe", "-m", "after first page"])?;
        let latest = repo.value("commit_id")?;
        fs::write(repo.project.join("dirty.txt"), "do not snapshot\n")?;
        let native = repo.operation()?;
        let history = storage.history_status(&repo.project).await?;
        let graph = storage.workflow_graph(workflow.id).await?;
        assert_eq!(
            storage
                .node_revisions(workflow.id, node.id, 2, Some(&cursor))
                .await?,
            before_rewrite
        );
        drop(storage);
        let storage = Storage::open(&repo.directory.path().join("revisions.db")).await?;
        let mut actual = first.entries;
        let mut cursor = Some(cursor);
        while let Some(next) = cursor {
            let page = storage
                .node_revisions(workflow.id, node.id, 3, Some(&next))
                .await?;
            assert_eq!(page.native_operation, first.native_operation);
            assert_eq!(page.current_commit_ids, first.current_commit_ids);
            actual.extend(page.entries);
            cursor = page.next_cursor;
        }
        assert_eq!(
            actual
                .iter()
                .map(|e| e.evolution.revision.commit_id.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            !actual
                .iter()
                .any(|e| e.evolution.revision.commit_id == latest)
        );
        let refreshed = storage
            .node_revisions(workflow.id, node.id, 1, None)
            .await?;
        assert_eq!(refreshed.current_commit_ids, [latest]);
        let old = &actual.last().unwrap().evolution.revision;
        assert_eq!(
            storage
                .review_node_revision(workflow.id, node.id, &old.commit_id)
                .await?
                .revision,
            *old
        );
        assert_eq!(repo.operation()?, native);
        assert_eq!(storage.history_status(&repo.project).await?, history);
        assert_eq!(storage.workflow_graph(workflow.id).await?, graph);
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "do not snapshot\n"
        );
        Ok(())
    })
}

#[test]
fn divergent_and_squashed_evolution_preserves_all_heads_links_and_convergence() -> anyhow::Result<()>
{
    run(async {
        let (repo, storage, workflow, node, change) = fixture().await?;
        repo.jj(&["describe", "-m", "shared predecessor"])?;
        let operation = repo.operation()?;
        repo.jj(&[
            "--ignore-working-copy",
            "describe",
            "-r",
            &change,
            "-m",
            "branch one",
        ])?;
        repo.jj(&[
            &format!("--at-op={operation}"),
            "describe",
            "-r",
            &change,
            "-m",
            "branch two",
        ])?;
        // Integrate the test's deliberate concurrent operations with native jj.
        repo.jj(&[
            "log",
            "--no-graph",
            "-r",
            &format!("change_id(\"{change}\")"),
        ])?;
        let page = storage
            .node_revisions(workflow.id, node.id, 1, None)
            .await?;
        assert_eq!(page.current_commit_ids.len(), 2);
        let entries = collect(&storage, workflow.id, node.id, 1).await?;
        assert_eq!(entries.iter().filter(|e| e.is_current).count(), 2);
        let ids = entries
            .iter()
            .map(|e| e.evolution.revision.commit_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            ids.len(),
            entries.len(),
            "converging predecessor must appear once"
        );
        for entry in &entries {
            assert!(
                entry
                    .evolution
                    .predecessor_commit_ids
                    .iter()
                    .all(|id| ids.contains(id))
            );
        }
        // Resolve this test's divergence explicitly, then squash a different
        // change into the node. Evolution legitimately includes both change IDs.
        repo.jj(&[
            "--ignore-working-copy",
            "abandon",
            &page.current_commit_ids[1],
        ])?;
        repo.jj(&["edit", &change])?;
        fs::write(repo.project.join("a.txt"), "a\n")?;
        repo.jj(&["new"])?;
        let related_change = repo.value("change_id")?;
        fs::write(repo.project.join("b.txt"), "b\n")?;
        repo.jj(&["squash", "-m", "combined"])?;
        let entries = collect(&storage, workflow.id, node.id, 2).await?;
        assert!(
            entries
                .iter()
                .any(|e| e.evolution.revision.change_id == related_change)
        );
        let ids = entries
            .iter()
            .map(|e| e.evolution.revision.commit_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), entries.len());
        let expected = repo
            .jj(&[
                "--ignore-working-copy",
                "evolog",
                "--no-graph",
                "-r",
                &change,
                "-T",
                "commit.commit_id() ++ \"\\n\"",
            ])?
            .lines()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids, expected);
        let related = entries
            .iter()
            .find(|e| e.evolution.revision.change_id == related_change)
            .unwrap();
        assert_eq!(
            storage
                .review_node_revision(workflow.id, node.id, &related.evolution.revision.commit_id)
                .await?
                .revision,
            related.evolution.revision
        );
        Ok(())
    })
}

#[test]
fn revision_browsing_rejects_bad_cursors_selectors_and_missing_changes_without_mutation()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, change) = fixture().await?;
        repo.jj(&["describe", "-m", "first"])?;
        let page = storage
            .node_revisions(workflow.id, node.id, 1, None)
            .await?;
        let cursor = page.next_cursor.expect("predecessor");
        let other = storage
            .create_node(workflow.id, "No native association")
            .await?;
        let before = repo.operation()?;
        for size in [0, 101, usize::MAX] {
            assert!(
                storage
                    .node_revisions(workflow.id, node.id, size, None)
                    .await
                    .is_err()
            );
        }
        for encoded in ["", "{}", "not-json", &"x".repeat(65537)] {
            assert!(
                storage
                    .node_revisions(workflow.id, node.id, 1, Some(encoded))
                    .await
                    .is_err()
            );
        }
        let mut bad: serde_json::Value = serde_json::from_str(&cursor)?;
        for (field, value) in [
            ("node_id", serde_json::json!(other.id)),
            ("version", serde_json::json!(42)),
            ("operation", serde_json::json!("root()")),
            ("heads", serde_json::json!(["0".repeat(40)])),
            ("frontier", serde_json::json!(["root()"])),
        ] {
            let saved = bad[field].clone();
            bad[field] = value;
            assert!(
                storage
                    .node_revisions(workflow.id, node.id, 1, Some(&bad.to_string()))
                    .await
                    .is_err(),
                "{field}"
            );
            bad[field] = saved;
        }
        assert!(
            storage
                .node_revisions(workflow.id, other.id, 1, None)
                .await
                .is_err()
        );
        for selector in [
            "root()",
            "@",
            &change,
            "0123",
            &"F".repeat(40),
            &"f".repeat(40),
        ] {
            assert!(
                storage
                    .review_node_revision(workflow.id, node.id, selector)
                    .await
                    .is_err()
            );
        }
        assert_eq!(repo.operation()?, before);
        repo.jj(&["--ignore-working-copy", "abandon", &change])?;
        let before = repo.operation()?;
        assert!(
            storage
                .node_revisions(workflow.id, node.id, 1, None)
                .await
                .is_err()
        );
        // A pinned traversal can still read the retained historical operation.
        assert!(
            storage
                .node_revisions(workflow.id, node.id, 1, Some(&cursor))
                .await
                .is_ok()
        );
        assert_eq!(repo.operation()?, before);
        Ok(())
    })
}

#[test]
fn history_includes_native_conflict_flags_and_does_not_require_a_clean_checkout()
-> anyhow::Result<()> {
    run(async {
        let (repo, mut storage, workflow, node, first) = fixture().await?;
        fs::write(repo.project.join("conflict.txt"), "first\n")?;
        repo.jj(&["describe", "-m", "first"])?;
        repo.jj(&["new", "root()"])?;
        fs::write(repo.project.join("conflict.txt"), "second\n")?;
        repo.jj(&["describe", "-m", "second"])?;
        let second = repo.value("change_id")?;
        repo.jj(&["new", &first, &second])?;
        let merged = repo.value("change_id")?;
        storage
            .associate_node(workflow.id, node.id, &merged)
            .await?;
        fs::write(repo.project.join("dirty.txt"), "leave alone\n")?;
        let operation = repo.operation()?;
        let page = storage
            .node_revisions(workflow.id, node.id, 1, None)
            .await?;
        assert!(page.entries[0].evolution.revision.conflicted);
        let review = storage
            .review_node_revision(
                workflow.id,
                node.id,
                &page.entries[0].evolution.revision.commit_id,
            )
            .await?;
        assert!(review.revision.conflicted);
        assert_eq!(repo.operation()?, operation);
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "leave alone\n"
        );
        Ok(())
    })
}

#[test]
fn oversized_native_history_returns_an_error_not_a_truncated_page() -> anyhow::Result<()> {
    run(async {
        let (repo, storage, workflow, node, _) = fixture().await?;
        let description = "x".repeat(180_000);
        for index in 0..7 {
            repo.jj(&["describe", "-m", &format!("{description}{index}")])?;
        }
        let operation = repo.operation()?;
        assert!(
            storage
                .node_revisions(workflow.id, node.id, 100, None)
                .await
                .is_err()
        );
        assert_eq!(
            storage
                .node_revisions(workflow.id, node.id, 1, None)
                .await?
                .entries
                .len(),
            1
        );
        assert_eq!(repo.operation()?, operation);
        Ok(())
    })
}
