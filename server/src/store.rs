use rusqlite::Connection;

pub fn open(path: &str) -> Connection {
    let db = Connection::open(path).expect("open db");
    db.execute_batch(include_str!("../schema.sql")).expect("schema");
    db.execute_batch(include_str!("../schema_annotations.sql")).expect("annotations schema");
    db
}
