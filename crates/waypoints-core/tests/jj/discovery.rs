use super::*;
use waypoints_core::repository;

#[test]
fn discovery_is_canonical_literal_and_readonly() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Base \"quoted\" 日本語"])?;
        repo.jj(&["bookmark", "create", "main", "feature/auth", "literal*"])?;
        let commit = repo.value("commit_id")?;
        let nested = repo.project.join("nested");
        fs::create_dir(&nested)?;
        fs::write(repo.project.join("dirty.txt"), "not recorded\n")?;
        let head = repo.operation()?;
        let checkout = repo.jj(&["--ignore-working-copy", "debug", "working-copy"])?;
        let value = repository::inspect(&nested).await?;
        assert_eq!(value.project_path, nested.canonicalize()?);
        assert_eq!(value.repository_path, repo.project.canonicalize()?);
        assert_eq!(value.native_operation, head);
        assert_eq!(
            value
                .bookmarks
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["feature/auth", "literal*", "main"]
        );
        for bookmark in value.bookmarks {
            assert!(!bookmark.conflicted);
            assert!(bookmark.removed_targets.is_empty());
            assert_eq!(bookmark.added_targets.len(), 1);
            assert_eq!(bookmark.added_targets[0].revision.commit_id, commit);
            assert!(!bookmark.added_targets[0].divergent);
        }
        assert_eq!(repo.operation()?, head);
        assert_eq!(
            repo.jj(&["--ignore-working-copy", "debug", "working-copy"])?,
            checkout
        );
        assert_eq!(
            fs::read_to_string(repo.project.join("dirty.txt"))?,
            "not recorded\n"
        );
        assert!(repository::inspect(repo.directory.path()).await.is_err());
        assert!(
            repository::inspect(&repo.project.join("missing"))
                .await
                .is_err()
        );
        assert!(
            repository::inspect(&repo.project.join("dirty.txt"))
                .await
                .is_err()
        );
        assert!(repository::inspect(Path::new(".")).await.is_err());
        Ok(())
    })
}

#[test]
fn discovery_preserves_ref_conflicts_and_divergent_changes() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        repo.jj(&["describe", "-m", "Original"])?;
        repo.jj(&["bookmark", "create", "main"])?;
        let id = repo.value("change_id")?;
        let fork = repo.operation()?;
        repo.jj(&["describe", "-r", &id, "-m", "First"])?;
        repo.jj(&["--at-op", &fork, "describe", "-r", &id, "-m", "Second"])?;
        // Discovery refuses concurrent operation heads instead of merging them.
        assert!(repository::inspect(&repo.project).await.is_err());
        repo.jj(&["status"])?;
        let head = repo.operation()?;
        let value = repository::inspect(&repo.project).await?;
        let bookmark = &value.bookmarks[0];
        assert!(bookmark.conflicted);
        assert_eq!(bookmark.added_targets.len(), 2);
        assert_eq!(bookmark.removed_targets.len(), 1);
        assert!(
            bookmark
                .added_targets
                .iter()
                .all(|r| r.divergent && r.revision.change_id == id)
        );
        assert_eq!(repo.operation()?, head);
        Ok(())
    })
}

#[test]
fn discovery_preserves_code_conflicts_and_empty_repositories() -> anyhow::Result<()> {
    run(async {
        let repo = Repository::new()?;
        assert!(
            repository::inspect(&repo.project)
                .await?
                .bookmarks
                .is_empty()
        );
        fs::write(repo.project.join("file.txt"), "base\n")?;
        let base = repo.value("change_id")?;
        repo.jj(&["new", &base])?;
        fs::write(repo.project.join("file.txt"), "left\n")?;
        let left = repo.value("change_id")?;
        repo.jj(&["new", &base])?;
        fs::write(repo.project.join("file.txt"), "right\n")?;
        let right = repo.value("change_id")?;
        repo.jj(&["new", &left, &right])?;
        repo.jj(&["bookmark", "create", "merge"])?;
        let head = repo.operation()?;
        let value = repository::inspect(&repo.project).await?;
        assert!(!value.bookmarks[0].conflicted);
        assert!(value.bookmarks[0].added_targets[0].revision.conflicted);
        assert_eq!(repo.operation()?, head);
        Ok(())
    })
}
