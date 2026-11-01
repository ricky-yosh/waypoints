use super::*;

fn setup() -> anyhow::Result<(Fixture, PathBuf, PathBuf, PathBuf)> {
    let (fixture, agent, _unused) = run_fixture("unused")?;
    let project = fixture.directory.path().join("project");
    fixture.success(&[
        "workflow",
        "create",
        "--project",
        project.to_str().expect("project"),
        "--name",
        "Deletion",
        "--base-revision",
        "root()",
    ])?;
    fixture.success(&[
        "node",
        "create",
        "--workflow",
        "2",
        "--prompt",
        "complete",
        "--from-base",
    ])?;
    fixture.success(&["node", "create-change", "--workflow", "2", "--node", "2"])?;
    let kept = fixture.directory.path().join("keep-this-workspace");
    fixture.success(&[
        "node",
        "workspace",
        "create",
        "--workflow",
        "2",
        "--node",
        "2",
        "--directory",
        kept.to_str().expect("workspace"),
    ])?;
    Ok((fixture, agent, project, kept))
}

#[test]
fn deleting_a_used_node_retains_dirty_ignored_untracked_files_and_saved_run_review()
-> anyhow::Result<()> {
    let (fixture, agent, project, kept) = setup()?;
    fs::write(kept.join(".gitignore"), "ignored.txt\n")?;
    fs::write(kept.join("tracked.txt"), "recorded\n")?;
    jj(&kept, &["describe", "-m", "Recorded source"])?;
    let started = fixture.success(&[
        "run",
        "start",
        "--workflow",
        "2",
        "--node",
        "2",
        "--executable",
        agent.to_str().expect("agent"),
    ])?;
    let id = started["run"]["id"].to_string();
    let completed = wait_run(&fixture, &id, "completed")?;
    let review = fixture.success(&["run", "review", "--run", &id])?;
    fs::write(kept.join("tracked.txt"), "dirty tracked bytes\n")?;
    fs::write(kept.join("untracked.txt"), "untracked bytes\n")?;
    fs::write(kept.join("ignored.txt"), [0, 255, 1, 0])?;
    let checkout = fs::read(kept.join(".jj/working_copy/checkout"))?;
    let tree = fs::read(kept.join(".jj/working_copy/tree_state"))?;
    let source_checkout = fs::read(project.join(".jj/working_copy/checkout"))?;
    let deleted = fixture.success(&[
        "node",
        "delete",
        "--workflow",
        "2",
        "--node",
        "2",
        "--accept-undo-boundary",
    ])?;
    assert_eq!(deleted["state"], "completed");
    assert_eq!(
        deleted["deletion_workspace"]["workspace"]["directory"],
        kept.canonicalize()?.to_str().expect("path")
    );
    assert!(deleted["deletion_workspace"]["abandoned_operation"].is_string());
    assert!(
        deleted["detail"]
            .as_str()
            .expect("detail")
            .contains("folder and all files left")
    );
    let name = deleted["deletion_workspace"]["workspace"]["name"]
        .as_str()
        .expect("name");
    assert!(
        !jj(
            &project,
            &[
                "--ignore-working-copy",
                "--at-op=@",
                "workspace",
                "list",
                "-T",
                "name ++ \"\\n\""
            ]
        )?
        .lines()
        .any(|n| n == name)
    );
    assert_eq!(
        fs::read_to_string(kept.join("tracked.txt"))?,
        "dirty tracked bytes\n"
    );
    assert_eq!(
        fs::read_to_string(kept.join("untracked.txt"))?,
        "untracked bytes\n"
    );
    assert_eq!(fs::read(kept.join("ignored.txt"))?, [0, 255, 1, 0]);
    assert_eq!(fs::read(kept.join(".jj/working_copy/checkout"))?, checkout);
    assert_eq!(fs::read(kept.join(".jj/working_copy/tree_state"))?, tree);
    assert_eq!(
        fs::read(project.join(".jj/working_copy/checkout"))?,
        source_checkout
    );
    let runs = fixture.success(&["run", "list", "--workflow", "2", "--node", "2"])?;
    assert_eq!(runs[0], completed);
    assert_eq!(fixture.success(&["run", "review", "--run", &id])?, review);
    fixture.stop()?;
    assert_eq!(fixture.success(&["run", "review", "--run", &id])?, review);
    assert!(kept.is_dir());
    Ok(())
}

#[test]
fn real_backend_crashes_do_not_repeat_abandonment_or_workspace_forgetting() -> anyhow::Result<()> {
    for (mutation, applied) in [
        ("abandon", false),
        ("abandon", true),
        ("forget", false),
        ("forget", true),
    ] {
        let (fixture, _agent, project, kept) = setup()?;
        fs::write(kept.join("dirty.txt"), "keep me\n")?;
        let checkout = fs::read(kept.join(".jj/working_copy/checkout"))?;
        fixture.stop()?;
        let native = Command::new("/bin/sh")
            .args(["-c", "command -v jj"])
            .output()?;
        assert!(native.status.success());
        let native = String::from_utf8(native.stdout)?.trim().to_owned();
        let bin = fixture.directory.path().join("intercept");
        fs::create_dir(&bin)?;
        let entered = fixture.directory.path().join("entered");
        let calls = fixture.directory.path().join("calls");
        let wrapper = bin.join("jj");
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nnative='{native}'\nfor arg do\ncase \"$arg\" in abandon|forget) echo \"$arg\" >> '{}' ;; esac\n\
             if [ \"$arg\" = '{mutation}' ]; then\n{}\necho $$ > '{}'\nexec /bin/sleep 15\nfi\ndone\nexec \"$native\" \"$@\"\n",
                calls.display(),
                if applied {
                    "\"$native\" \"$@\" > /dev/null || exit $?"
                } else {
                    ":"
                },
                entered.display()
            ),
        )?;
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
        fixture.json_output(
            fixture
                .list_command()
                .env(
                    "PATH",
                    format!("{}:{}", bin.display(), std::env::var("PATH")?),
                )
                .output()?,
            "intercepted startup",
        )?;
        let backend_pid = fixture.success(&["backend", "status"])?["pid"].to_string();
        let child = fixture
            .command()
            .args([
                "node",
                "delete",
                "--workflow",
                "2",
                "--node",
                "2",
                "--accept-undo-boundary",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        wait_for_file(&entered);
        let native_pid = fs::read_to_string(&entered)?.trim().to_owned();
        // These are freshly launched fixture-owned processes, never historical saved PIDs.
        assert!(
            Command::new("/bin/kill")
                .args(["-KILL", &backend_pid])
                .status()?
                .success()
        );
        assert!(
            Command::new("/bin/kill")
                .args(["-KILL", &native_pid])
                .status()?
                .success()
        );
        assert!(!child.wait_with_output()?.status.success());
        let inspected = fixture.success(&["node", "operation-recovery", "--operation", "1"])?;
        let assessment = if mutation == "abandon" && !applied {
            "not_applied"
        } else if mutation == "forget" && applied {
            "applied"
        } else {
            "deletion_abandoned"
        };
        assert_eq!(inspected["assessment"]["status"], assessment, "{inspected}");
        let command = if assessment == "deletion_abandoned" {
            "resume-deletion"
        } else {
            "recover-operation"
        };
        let resolved = fixture.success(&[
            "node",
            command,
            "--operation",
            "1",
            "--confirm-processes-stopped",
        ])?;
        assert_eq!(
            resolved["state"],
            if assessment == "not_applied" {
                "not_applied"
            } else {
                "completed"
            }
        );
        assert_eq!(
            fs::read_to_string(&calls)?,
            if mutation == "abandon" {
                "abandon\n"
            } else {
                "abandon\nforget\n"
            }
        );
        assert_eq!(fs::read_to_string(kept.join("dirty.txt"))?, "keep me\n");
        assert_eq!(fs::read(kept.join(".jj/working_copy/checkout"))?, checkout);
        let operation = jj(
            &project,
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
        )?;
        assert_eq!(
            fixture.success(&[
                "node",
                command,
                "--operation",
                "1",
                "--confirm-processes-stopped"
            ])?,
            resolved
        );
        assert_eq!(
            jj(
                &project,
                &[
                    "--ignore-working-copy",
                    "--at-op=@",
                    "op",
                    "log",
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    "id"
                ]
            )?,
            operation
        );
    }
    Ok(())
}
