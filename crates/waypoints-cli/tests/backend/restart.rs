use super::*;
use std::os::unix::fs::MetadataExt;

#[test]
fn the_same_startup_requests_succeed_when_ownership_is_released_after_contention()
-> anyhow::Result<()> {
    for lock_name in ["workflow.db.lock", "backend.sock.lock"] {
        for clients in [1, 4] {
            let fixture = Fixture::new()?;
            let lock_path = fixture.directory.path().join(lock_name);
            let owner = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)?;
            owner.try_lock()?;
            let inode = owner.metadata()?.ino();
            let blocked = fixture.command().arg("serve").output()?;
            assert_eq!(
                blocked.status.code(),
                Some(i32::from(
                    waypoints_backend::OWNERSHIP_UNAVAILABLE_EXIT_CODE
                )),
                "{lock_name}: contention must have a distinguishable startup status"
            );
            let children = (0..clients)
                .map(|_| {
                    fixture
                        .list_command()
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                })
                .collect::<std::io::Result<Vec<_>>>()?;

            // State synchronization, not a settling delay: observe actual failed
            // starters before releasing ownership. Requests are never reissued.
            let deadline = Instant::now() + Duration::from_secs(3);
            let log = fixture.directory.path().join("backend.sock.log");
            loop {
                let contents = fs::read_to_string(&log).unwrap_or_default();
                if contents
                    .matches("Error: backend ownership unavailable at")
                    .count()
                    >= clients
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "starters did not report contention:\n{}",
                    fixture.startup_diagnostics()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!fixture.database.exists(), "ownership was bypassed");
            assert!(
                !fixture.socket.exists(),
                "listener appeared while ownership was held"
            );
            drop(owner);

            // Collect all clients before asserting, so failed starts do not leave
            // outstanding requests using a fixture that is being removed.
            let outputs = children
                .into_iter()
                .map(Child::wait_with_output)
                .collect::<std::io::Result<Vec<_>>>()?;
            for output in outputs {
                assert_eq!(
                    fixture.json_output(output, &format!("{lock_name}, {clients} clients"))?,
                    json!([])
                );
            }
            assert_eq!(
                fs::metadata(&lock_path)?.ino(),
                inode,
                "lock file was replaced"
            );
            let identity = fixture.success(&["backend", "status"])?;
            assert_eq!(fixture.success(&["backend", "status"])?, identity);
            // Neither ownership lock can be acquired by a second writer.
            for name in ["workflow.db.lock", "backend.sock.lock"] {
                let contender = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(fixture.directory.path().join(name))?;
                assert!(
                    matches!(contender.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
                    "{name}: backend has no exclusive ownership"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn startup_does_not_retry_other_process_failures() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let executable = fixture.directory.path().join("starter");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf 'attempt\\n' >> \"$0.calls\"\nexit 1\n",
    )?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    let config = waypoints_backend::Config::new(&fixture.database, &fixture.socket)?;
    let error = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(waypoints_backend::Client::connect_or_start(
            &config,
            &executable,
        ))
        .expect_err("a failed starter with no listener must not report success");
    assert!(
        error.to_string().contains("backend exited during startup"),
        "unexpected error: {error}"
    );
    assert_eq!(
        fs::read_to_string(executable.with_extension("calls"))?,
        "attempt\n",
        "non-ownership failures must not launch repeated starters"
    );
    assert!(!fixture.database.exists());
    assert!(!fixture.socket.exists());
    Ok(())
}
