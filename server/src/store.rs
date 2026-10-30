use rusqlite::Connection;

/// Open a WAL connection with the same lock timeout and referential guarantees for every worker.
pub fn connect(path: &str) -> rusqlite::Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    Ok(db)
}

/// Apply additive schemas to an application database.
pub fn open(path: &str) -> Connection {
    let mut db = connect(path).expect("open db");
    db.execute_batch(include_str!("../schema.sql")).expect("schema");
    db.execute_batch(include_str!("../schema_annotations.sql")).expect("annotations schema");
    // Registries created before the lifecycle column existed keep their old table shape under
    // CREATE TABLE IF NOT EXISTS, so add the column here.
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).expect("schema migration lock");
    let has_lifecycle = tx.prepare("PRAGMA table_info(transcode_artifacts)").expect("artifact columns")
        .query_map([], |row| row.get::<_, String>(1)).expect("artifact column names")
        .any(|name| name.as_deref() == Ok("lifecycle"));
    if !has_lifecycle {
        tx.execute_batch("ALTER TABLE transcode_artifacts ADD COLUMN lifecycle TEXT NOT NULL DEFAULT 'unknown'
            CHECK(lifecycle IN ('unknown','pending','completed','retiring'));").expect("artifact lifecycle migration");
    }
    tx.commit().expect("schema migration commit");
    db
}

#[cfg(test)]
mod tests {
    #[test]
    fn artifact_lifecycle_migration_is_idempotent_and_preserves_unknown_history() {
        let path=std::env::temp_dir().join(format!("frayd-lifecycle-{}.sqlite",uuid::Uuid::new_v4()));
        let db=rusqlite::Connection::open(&path).unwrap();
        let old_schema = include_str!("../schema.sql").replace(
            "  lifecycle TEXT NOT NULL DEFAULT 'unknown' CHECK(lifecycle IN ('unknown','pending','completed','retiring')),\n",
            "",
        );
        db.execute_batch(&old_schema).unwrap();
        db.execute_batch("INSERT INTO media(id,kind,path,title) VALUES(1,'movie','/original','original');
            INSERT INTO transcode_artifacts VALUES('/historical',1,'/original','final','generated');").unwrap();drop(db);
        for _ in 0..2 {
            let db=super::open(path.to_str().unwrap());
            assert_eq!(
                db.query_row(
                    "SELECT owner_media_id,owner_source_path,lifecycle FROM transcode_artifacts",
                    [],
                    |r| Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?
                    ))
                )
                .unwrap(),
                (1, "/original".into(), "unknown".into())
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
