//! Schema migrations (docs/architecture.md, section 10.2): ordered SQL
//! scripts per backend, applied by `migrate`; `serve` refuses a schema
//! older than the binary. The first migration also creates the server
//! generation (section 3.4).

use crate::db::{Backend, Db, DbError, TxKind, Ty};
use crate::model;

/// The schema version this binary needs.
pub const CURRENT: i64 = 1;

const POSTGRES: &[&str] = &[include_str!("../migrations/postgres/0001_initial.sql")];
const SQLITE: &[&str] = &[include_str!("../migrations/sqlite/0001_initial.sql")];

const VERSION_TABLE: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)";

/// The applied schema version (0 for an empty database).
pub fn version(db: &Db) -> Result<i64, DbError> {
    db.batch(VERSION_TABLE)?;
    db.tx(TxKind::Read, |tx| -> Result<i64, DbError> {
        Ok(tx
            .query_one(
                "SELECT MAX(version) FROM schema_migrations",
                &[],
                &[Ty::Int],
            )?
            .and_then(|row| row[0].int())
            .unwrap_or(0))
    })
}

/// Applies every missing step; returns the versions applied.
pub fn migrate(db: &Db) -> Result<Vec<i64>, DbError> {
    let scripts = match db.backend() {
        Backend::Postgres => POSTGRES,
        Backend::Sqlite => SQLITE,
    };
    let from = version(db)?;
    if from > scripts.len() as i64 {
        return Err(DbError::Failed(
            "the database schema is newer than this server".to_owned(),
        ));
    }
    let mut applied = Vec::new();
    for (index, script) in scripts.iter().enumerate() {
        let step = index as i64 + 1;
        if step <= from {
            continue;
        }
        // One transaction per step, with its version row.
        db.tx(TxKind::Write, |tx| -> Result<(), DbError> {
            for statement in split_statements(script) {
                tx.exec(&statement, &[])?;
            }
            tx.exec(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?, ?)",
                &[step.into(), crate::db::time_text(model::now()).into()],
            )?;
            if step == 1 {
                tx.exec(
                    "INSERT INTO server_state (id, server_generation, changed_at) VALUES (1, ?, ?)",
                    &[
                        crate::secrets::random_bytes::<16>().to_vec().into(),
                        model::now().into(),
                    ],
                )?;
            }
            Ok(())
        })?;
        tracing::info!(version = step, "migration applied");
        applied.push(step);
    }
    Ok(applied)
}

/// The statements of a script: split at `;` at the end of a line, comments
/// dropped (the scripts hold no `;` inside literals).
fn split_statements(script: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    for line in script.lines() {
        let line = line.split("--").next().unwrap_or_default();
        if line.trim().is_empty() {
            continue;
        }
        current.push_str(line);
        current.push('\n');
        if line.trim_end().ends_with(';') {
            let statement = current.trim().trim_end_matches(';').trim().to_owned();
            if !statement.is_empty() {
                statements.push(statement);
            }
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        statements.push(current.trim().to_owned());
    }
    statements
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_migrates_once_and_has_a_generation() {
        let db = Db::open("sqlite::memory:").unwrap();
        assert_eq!(version(&db).unwrap(), 0);
        assert_eq!(migrate(&db).unwrap(), vec![1]);
        assert_eq!(version(&db).unwrap(), CURRENT);
        assert!(migrate(&db).unwrap().is_empty());
        let generation = db
            .tx(TxKind::Read, |tx| crate::records::server_generation(tx))
            .unwrap()
            .unwrap();
        assert_eq!(generation.len(), 16);
    }

    #[test]
    fn statements_split_at_semicolons() {
        let parts = split_statements(
            "-- c\nCREATE TABLE a (x INTEGER);\nCREATE INDEX i ON a (x) WHERE x > 0;\n",
        );
        assert_eq!(parts.len(), 2);
        assert!(parts[1].starts_with("CREATE INDEX"));
    }
}
