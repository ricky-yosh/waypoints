use super::*;

#[test]
fn repository_inspection_cli_rpc_is_readonly_and_strict() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    jj(&project, &["describe", "-m", "Local base 日本語"])?;
    jj(&project, &["bookmark", "create", "main", "literal*"])?;
    let nested = project.join("nested");
    fs::create_dir(&nested)?;
    let commit = jj(
        &project,
        &[
            "--ignore-working-copy",
            "log",
            "--no-graph",
            "-r",
            "@",
            "-T",
            "commit_id",
        ],
    )?;
    fs::write(project.join("dirty.txt"), "unrecorded\n")?;
    let head = native_head(&project)?;
    let checkout = jj(
        &project,
        &["--ignore-working-copy", "debug", "working-copy"],
    )?;
    let value = fixture.json_output(
        fixture
            .command()
            .current_dir(&project)
            .args(["repository", "inspect", "--project", "nested"])
            .output()?,
        "repository inspection from client-relative path",
    )?;
    assert_eq!(value["project_path"], json!(nested.canonicalize()?));
    assert_eq!(value["repository_path"], json!(project.canonicalize()?));
    assert_eq!(value["native_operation"], head);
    assert_eq!(value["bookmarks"].as_array().expect("bookmarks").len(), 2);
    assert_eq!(value["bookmarks"][0]["name"], "literal*");
    assert_eq!(value["bookmarks"][1]["name"], "main");
    assert_eq!(
        value["bookmarks"][1]["added_targets"][0]["revision"]["commit_id"],
        commit
    );
    assert_eq!(value["bookmarks"][1]["conflicted"], false);
    let rpc = |params: Value| {
        fixture.rpc(
            format!(
                "{}\n",
                json!({"jsonrpc":"2.0","id":1,"method":"repository.inspect","params":params})
            )
            .as_bytes(),
        )
    };
    assert_eq!(rpc(json!({"project_path":nested}))?["result"], value);
    for params in [
        json!({}),
        json!({"project_path":1}),
        json!({"project_path":project,"base":"main"}),
        json!({"project_path":project,"limit":1}),
        json!({"project_path":null}),
        json!([]),
    ] {
        let response = rpc(params)?;
        assert_eq!(response["error"]["code"], -32602, "{response}");
    }
    for path in [
        PathBuf::from("."),
        fixture.directory.path().join("missing"),
        fixture.directory.path().to_path_buf(),
        project.join("dirty.txt"),
    ] {
        let response = rpc(json!({"project_path":path}))?;
        assert_eq!(response["error"]["code"], -32000, "{response}");
    }
    let missing = fixture
        .command()
        .args(["repository", "inspect", "--project"])
        .arg(project.join("missing"))
        .output()?;
    assert!(!missing.status.success());
    assert_eq!(native_head(&project)?, head);
    assert_eq!(
        jj(
            &project,
            &["--ignore-working-copy", "debug", "working-copy"]
        )?,
        checkout
    );
    assert_eq!(
        fs::read_to_string(project.join("dirty.txt"))?,
        "unrecorded\n"
    );
    assert_eq!(
        fixture.success(&[
            "workflow",
            "list",
            "--project",
            project.to_str().expect("path")
        ])?,
        json!([])
    );
    fixture.stop()?;
    assert_eq!(
        fixture.success(&[
            "repository",
            "inspect",
            "--project",
            nested.to_str().expect("path")
        ])?,
        value
    );
    Ok(())
}

fn native_head(project: &Path) -> anyhow::Result<String> {
    jj(
        project,
        &[
            "--ignore-working-copy",
            "--at-op=@",
            "op",
            "log",
            "--no-graph",
            "--limit",
            "1",
            "-T",
            "id",
        ],
    )
}

#[test]
fn repository_inspection_backend_retains_native_conflicts() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    jj(fixture.directory.path(), &["git", "init", "project"])?;
    let project = fixture.directory.path().join("project");
    let path = project.to_str().expect("path");
    jj(&project, &["describe", "-m", "Original"])?;
    jj(&project, &["bookmark", "create", "main"])?;
    let fork = native_head(&project)?;
    jj(&project, &["describe", "-m", "First"])?;
    jj(&project, &["--at-op", &fork, "describe", "-m", "Second"])?;
    let output = fixture
        .command()
        .args(["repository", "inspect", "--project", path])
        .output()?;
    assert!(
        !output.status.success(),
        "must not merge native operation heads"
    );
    jj(&project, &["status"])?;
    let head = native_head(&project)?;
    let value = fixture.success(&["repository", "inspect", "--project", path])?;
    let bookmark = &value["bookmarks"][0];
    assert_eq!(bookmark["conflicted"], true);
    assert_eq!(
        bookmark["added_targets"]
            .as_array()
            .expect("added targets")
            .len(),
        2
    );
    assert_eq!(
        bookmark["removed_targets"]
            .as_array()
            .expect("removed targets")
            .len(),
        1
    );
    for target in bookmark["added_targets"].as_array().expect("targets") {
        assert_eq!(target["divergent"], true);
    }
    assert_eq!(native_head(&project)?, head);
    // A normal ref may independently point to a revision with code conflicts.
    jj(&project, &["new", "root()"])?;
    fs::write(project.join("file.txt"), "base\n")?;
    let base = jj(
        &project,
        &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
    )?;
    jj(&project, &["new", &base])?;
    fs::write(project.join("file.txt"), "left\n")?;
    let left = jj(
        &project,
        &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
    )?;
    jj(&project, &["new", &base])?;
    fs::write(project.join("file.txt"), "right\n")?;
    let right = jj(
        &project,
        &["log", "--no-graph", "-r", "@", "-T", "commit_id"],
    )?;
    jj(&project, &["new", &left, &right])?;
    jj(&project, &["bookmark", "create", "merge"])?;
    let head = native_head(&project)?;
    let value = fixture.success(&["repository", "inspect", "--project", path])?;
    let merge = &value["bookmarks"][1];
    assert_eq!(merge["name"], "merge");
    assert_eq!(merge["conflicted"], false);
    assert_eq!(merge["added_targets"][0]["revision"]["conflicted"], true);
    assert_eq!(native_head(&project)?, head);
    Ok(())
}
