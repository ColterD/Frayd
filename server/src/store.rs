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
    let db = connect(path).expect("open db");
    db.execute_batch(include_str!("../schema.sql")).expect("schema");
    db.execute_batch(include_str!("../schema_annotations.sql")).expect("annotations schema");
    db
}
