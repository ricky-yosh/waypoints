//! Transactional upgrade from run-only restoration journals to optional run provenance.

use anyhow::Context;
use turso::transaction::TransactionBehavior;

const TABLE: &str = "CREATE TABLE IF NOT EXISTS restore_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES agent_runs(id),
    node_id INTEGER NOT NULL REFERENCES nodes(id),
    active INTEGER NOT NULL CHECK(active IN (0, 1)),
    session_barrier INTEGER NOT NULL,
    record TEXT NOT NULL
)";

pub(super) async fn initialize(database: &turso::Database) -> anyhow::Result<()> {
    let mut connection = database.connect()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await?;
    let result = async {
        tx.execute(TABLE, ()).await?;
        let mut columns = tx.query("PRAGMA table_info(restore_operations)", ()).await?;
        let mut run_required = false;
        while let Some(row) = columns.next().await? {
            if row.get::<String>(1)? == "run_id" { run_required = row.get::<i64>(3)? != 0; }
        }
        if run_required {
            let mut rows = tx.query(
                "SELECT seq FROM sqlite_sequence WHERE name = 'restore_operations'", (),
            ).await?;
            let sequence = match rows.next().await? {
                Some(row) => row.get::<i64>(0)?,
                None => 0,
            };
            // Never adopt a pre-existing staging table after an uncertain or
            // externally modified schema.
            tx.execute(&TABLE.replace(
                "IF NOT EXISTS restore_operations", "restore_operations_upgrade",
            ), ()).await?;
            tx.execute(
                "INSERT INTO restore_operations_upgrade
                 SELECT id, run_id, node_id, active, session_barrier, record FROM restore_operations", (),
            ).await?;
            tx.execute("DROP TABLE restore_operations", ()).await?;
            tx.execute("ALTER TABLE restore_operations_upgrade RENAME TO restore_operations", ()).await?;
            tx.execute(
                "UPDATE sqlite_sequence SET seq = MAX(seq, ?1) WHERE name = 'restore_operations'",
                [sequence],
            ).await?;
            tx.execute(
                "INSERT INTO sqlite_sequence(name, seq)
                 SELECT 'restore_operations', ?1
                 WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'restore_operations')",
                [sequence],
            ).await?;
        }
        tx.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS one_restore_operation
             ON restore_operations(active) WHERE active = 1", (),
        ).await?;
        Ok::<_, anyhow::Error>(())
    }.await;
    match result {
        Ok(()) => tx.commit().await?,
        Err(error) => {
            tx.rollback().await?;
            return Err(error)
                .context("upgrading restoration journal; prior schema and records retained");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn legacy_database(path: &std::path::Path) -> anyhow::Result<turso::Database> {
        let database = turso::Builder::new_local(path.to_str().unwrap())
            .build()
            .await?;
        let connection = database.connect()?;
        connection
            .execute("CREATE TABLE agent_runs(id INTEGER PRIMARY KEY)", ())
            .await?;
        connection
            .execute("CREATE TABLE nodes(id INTEGER PRIMARY KEY)", ())
            .await?;
        connection
            .execute("INSERT INTO agent_runs VALUES (1)", ())
            .await?;
        connection
            .execute("INSERT INTO nodes VALUES (2)", ())
            .await?;
        connection
            .execute(
                &TABLE.replace(
                    "run_id INTEGER REFERENCES",
                    "run_id INTEGER NOT NULL REFERENCES",
                ),
                (),
            )
            .await?;
        connection.execute(
            "INSERT INTO restore_operations(id, run_id, node_id, active, session_barrier, record)
             VALUES (7, 1, 2, 1, 42, 'original journal bytes')", (),
        ).await?;
        connection.execute(
            "CREATE UNIQUE INDEX one_restore_operation ON restore_operations(active) WHERE active = 1", (),
        ).await?;
        connection
            .execute(
                "UPDATE sqlite_sequence SET seq = 99 WHERE name = 'restore_operations'",
                (),
            )
            .await?;
        Ok(database)
    }

    #[test]
    fn upgrade_preserves_journal_reservation_and_allocation_watermark() -> anyhow::Result<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let directory = tempfile::tempdir()?;
                let database = legacy_database(&directory.path().join("legacy.db")).await?;
                initialize(&database).await?;
                // Reopening must not repeat the rebuild or reset IDs.
                initialize(&database).await?;
                let connection = database.connect()?;
                let mut rows = connection
                    .query("SELECT * FROM restore_operations", ())
                    .await?;
                let row = rows.next().await?.unwrap();
                assert_eq!(row.get::<i64>(0)?, 7);
                assert_eq!(row.get::<i64>(1)?, 1);
                assert_eq!(row.get::<i64>(2)?, 2);
                assert_eq!(row.get::<i64>(3)?, 1);
                assert_eq!(row.get::<i64>(4)?, 42);
                assert_eq!(row.get::<String>(5)?, "original journal bytes");
                assert!(rows.next().await?.is_none());
                assert!(connection.execute(
                "INSERT INTO restore_operations(run_id, node_id, active, session_barrier, record)
                 VALUES (NULL, 2, 1, 43, 'another active restore')", (),
            ).await.is_err(), "unresolved guard must survive migration");
                connection
                    .execute("UPDATE restore_operations SET active = 0", ())
                    .await?;
                connection.execute(
                "INSERT INTO restore_operations(run_id, node_id, active, session_barrier, record)
                 VALUES (NULL, 2, 1, 43, 'manual restore')", (),
            ).await?;
                assert_eq!(connection.last_insert_rowid(), 100);
                let mut rows = connection
                    .query(
                        "SELECT run_id IS NULL FROM restore_operations WHERE id = 100",
                        (),
                    )
                    .await?;
                assert_eq!(rows.next().await?.unwrap().get::<i64>(0)?, 1);
                Ok(())
            })
    }

    #[test]
    fn failed_upgrade_rolls_back_old_schema_rows_and_index() -> anyhow::Result<()> {
        tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
            let directory = tempfile::tempdir()?;
            let database = legacy_database(&directory.path().join("legacy.db")).await?;
            let connection = database.connect()?;
            // Force index creation to fail after copying, dropping and renaming:
            // this deliberately invalid legacy fixture has two active rows.
            connection.execute("DROP INDEX one_restore_operation", ()).await?;
            connection.execute("CREATE INDEX legacy_lookup ON restore_operations(node_id)", ()).await?;
            connection.execute(
                "INSERT INTO restore_operations(id, run_id, node_id, active, session_barrier, record)
                 VALUES (8, 1, 2, 1, 43, 'second active row')", (),
            ).await?;
            assert!(initialize(&database).await.is_err());
            let mut columns = connection.query("PRAGMA table_info(restore_operations)", ()).await?;
            let mut required = false;
            while let Some(row) = columns.next().await? {
                if row.get::<String>(1)? == "run_id" { required = row.get::<i64>(3)? == 1; }
            }
            assert!(required, "failed migration must retain the run-only schema");
            let mut rows = connection.query("SELECT id, record FROM restore_operations", ()).await?;
            let row = rows.next().await?.unwrap();
            assert_eq!(row.get::<i64>(0)?, 7);
            assert_eq!(row.get::<String>(1)?, "original journal bytes");
            let mut indexes = connection.query("PRAGMA index_list(restore_operations)", ()).await?;
            let mut retained = false;
            while let Some(row) = indexes.next().await? {
                retained |= row.get::<String>(1)? == "legacy_lookup";
            }
            assert!(retained, "rollback must restore indexes dropped with the old table");
            connection.execute("DELETE FROM restore_operations WHERE id = 8", ()).await?;
            initialize(&database).await?;
            Ok(())
        })
    }
}
