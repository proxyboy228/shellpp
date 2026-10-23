//! One SQL layer over two databases (D101): SQLite through rusqlite and
//! PostgreSQL through the synchronous `postgres` client. Every store
//! operation is a closure over a [`Tx`], run on a blocking thread
//! ([`Db::run`]).
//!
//! - Statements are written once with `?` placeholders; the PostgreSQL
//!   backend numbers them (`$1`, `$2`, ...). No statement here quotes a `?`.
//! - Values travel as [`Val`]. A query names the type of each column it
//!   reads ([`Ty`]), because SQLite stores UUIDs as 16-byte blobs, times as
//!   RFC 3339 text and booleans as integers, while PostgreSQL has native
//!   types (docs/sync-protocol.md, section 14).
//! - Write transactions serialize pushes to a vault: SQLite takes the
//!   database lock at `BEGIN IMMEDIATE`; PostgreSQL locks the vault row
//!   ([`Tx::lock_vault`]). Read transactions are one snapshot (PostgreSQL
//!   `REPEATABLE READ`, a SQLite read transaction in WAL mode).

use std::sync::{Arc, Condvar, Mutex};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

/// A value passed to or read from a statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Int(i64),
    Bool(bool),
    Text(String),
    Bytes(Vec<u8>),
    Uuid(Uuid),
    Time(OffsetDateTime),
    Json(serde_json::Value),
}

/// The type of a column a query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Bool,
    Text,
    Bytes,
    Uuid,
    Time,
    Json,
}

impl Val {
    pub fn int(&self) -> Option<i64> {
        match self {
            Val::Int(v) => Some(*v),
            _ => None,
        }
    }
    pub fn bool(&self) -> Option<bool> {
        match self {
            Val::Bool(v) => Some(*v),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Val::Text(v) => Some(v),
            _ => None,
        }
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Val::Bytes(v) => Some(v),
            _ => None,
        }
    }
    pub fn uuid(&self) -> Option<Uuid> {
        match self {
            Val::Uuid(v) => Some(*v),
            _ => None,
        }
    }
    pub fn time(&self) -> Option<OffsetDateTime> {
        match self {
            Val::Time(v) => Some(*v),
            _ => None,
        }
    }
    pub fn json(&self) -> Option<&serde_json::Value> {
        match self {
            Val::Json(v) => Some(v),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Val::Null)
    }
}

impl From<i64> for Val {
    fn from(v: i64) -> Self {
        Val::Int(v)
    }
}
impl From<bool> for Val {
    fn from(v: bool) -> Self {
        Val::Bool(v)
    }
}
impl From<&str> for Val {
    fn from(v: &str) -> Self {
        Val::Text(v.to_owned())
    }
}
impl From<String> for Val {
    fn from(v: String) -> Self {
        Val::Text(v)
    }
}
impl From<Vec<u8>> for Val {
    fn from(v: Vec<u8>) -> Self {
        Val::Bytes(v)
    }
}
impl From<&[u8]> for Val {
    fn from(v: &[u8]) -> Self {
        Val::Bytes(v.to_vec())
    }
}
impl From<Uuid> for Val {
    fn from(v: Uuid) -> Self {
        Val::Uuid(v)
    }
}
impl From<OffsetDateTime> for Val {
    fn from(v: OffsetDateTime) -> Self {
        Val::Time(v)
    }
}
impl From<serde_json::Value> for Val {
    fn from(v: serde_json::Value) -> Self {
        Val::Json(v)
    }
}
impl<T: Into<Val>> From<Option<T>> for Val {
    fn from(v: Option<T>) -> Self {
        v.map_or(Val::Null, Into::into)
    }
}

/// A failure of the database (never a domain error).
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The database cannot be reached (connection failures): `503`.
    #[error("database unavailable")]
    Unavailable,
    /// A statement failed: `500`. The text never holds values.
    #[error("database error: {0}")]
    Failed(String),
    /// A unique constraint was hit (lets callers turn a race into `409`).
    #[error("unique constraint")]
    Unique,
}

/// The kind of a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxKind {
    /// A consistent snapshot for reads.
    Read,
    /// Writes; serialized with other writes where it matters.
    Write,
}

/// A transaction of either backend.
pub trait Tx {
    /// Runs a statement; the number of rows it changed.
    fn exec(&mut self, sql: &str, params: &[Val]) -> Result<u64, DbError>;
    /// Runs a query; every row has one value per entry of `cols`.
    fn query(&mut self, sql: &str, params: &[Val], cols: &[Ty]) -> Result<Vec<Vec<Val>>, DbError>;
    /// Locks a vault row until the end of the transaction (PostgreSQL); a
    /// no-op in SQLite, whose write transactions hold the database lock.
    fn lock_vault(&mut self, vault_id: Uuid) -> Result<(), DbError>;
    /// The backend, for the few statements that differ.
    fn backend(&self) -> Backend;

    /// The first row of a query, if any.
    fn query_one(
        &mut self,
        sql: &str,
        params: &[Val],
        cols: &[Ty],
    ) -> Result<Option<Vec<Val>>, DbError> {
        Ok(self.query(sql, params, cols)?.into_iter().next())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

/// The database of the server.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("backend", &self.backend())
            .finish()
    }
}

enum Inner {
    Sqlite(Mutex<rusqlite::Connection>),
    Postgres(PgPool),
}

/// RFC 3339 UTC with milliseconds, the text form of times in SQLite.
pub fn time_text(t: OffsetDateTime) -> String {
    let t = t.to_offset(time::UtcOffset::UTC);
    let millis = t.millisecond();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        millis
    )
}

fn parse_time(text: &str) -> Result<OffsetDateTime, DbError> {
    OffsetDateTime::parse(text, &Rfc3339).map_err(|_| DbError::Failed("bad time".to_owned()))
}

impl Db {
    /// Opens `url`: `sqlite://<path>` (`sqlite::memory:` for tests) or
    /// `postgres://...`.
    pub fn open(url: &str) -> Result<Db, DbError> {
        if let Some(rest) = url.strip_prefix("sqlite:") {
            let path = rest.trim_start_matches("//");
            let conn = if path == ":memory:" || path.is_empty() {
                rusqlite::Connection::open_in_memory()
            } else {
                rusqlite::Connection::open(path)
            }
            .map_err(|_| DbError::Unavailable)?;
            conn.execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; \
                 PRAGMA synchronous = FULL; PRAGMA busy_timeout = 5000;",
            )
            .map_err(sqlite_error)?;
            Ok(Db {
                inner: Arc::new(Inner::Sqlite(Mutex::new(conn))),
            })
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Ok(Db {
                inner: Arc::new(Inner::Postgres(PgPool::new(url.to_owned(), 8)?)),
            })
        } else {
            Err(DbError::Failed(
                "SHELLPP_DATABASE_URL must start with sqlite: or postgres://".to_owned(),
            ))
        }
    }

    pub fn backend(&self) -> Backend {
        match &*self.inner {
            Inner::Sqlite(_) => Backend::Sqlite,
            Inner::Postgres(_) => Backend::Postgres,
        }
    }

    /// Runs `f` in a transaction on this thread (call it from a blocking
    /// thread). Commits when `f` returns `Ok`, rolls back otherwise.
    pub fn tx<T, E: From<DbError>>(
        &self,
        kind: TxKind,
        f: impl FnOnce(&mut dyn Tx) -> Result<T, E>,
    ) -> Result<T, E> {
        match &*self.inner {
            Inner::Sqlite(conn) => {
                let mut conn = conn
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let begin = match kind {
                    TxKind::Read => "BEGIN DEFERRED",
                    TxKind::Write => "BEGIN IMMEDIATE",
                };
                conn.execute_batch(begin).map_err(sqlite_error)?;
                let mut tx = SqliteTx { conn: &mut conn };
                match f(&mut tx) {
                    Ok(value) => {
                        conn.execute_batch("COMMIT").map_err(sqlite_error)?;
                        Ok(value)
                    }
                    Err(error) => {
                        let _ = conn.execute_batch("ROLLBACK");
                        Err(error)
                    }
                }
            }
            Inner::Postgres(pool) => {
                let mut client = pool.get()?;
                let result = {
                    let mut builder = client.build_transaction();
                    if kind == TxKind::Read {
                        builder = builder
                            .isolation_level(postgres::IsolationLevel::RepeatableRead)
                            .read_only(true);
                    }
                    let transaction = builder.start().map_err(pg_error)?;
                    let mut tx = PgTx {
                        tx: Some(transaction),
                    };
                    match f(&mut tx) {
                        Ok(value) => {
                            let transaction = tx.tx.take().expect("open");
                            transaction.commit().map_err(pg_error)?;
                            Ok(value)
                        }
                        Err(error) => {
                            if let Some(transaction) = tx.tx.take() {
                                let _ = transaction.rollback();
                            }
                            Err(error)
                        }
                    }
                };
                let broken = client
                    .client
                    .as_ref()
                    .is_none_or(postgres::Client::is_closed);
                if broken {
                    client.discard();
                }
                result
            }
        }
    }

    /// Runs `f` in a transaction on a blocking thread.
    pub async fn run<T, E>(
        &self,
        kind: TxKind,
        f: impl FnOnce(&mut dyn Tx) -> Result<T, E> + Send + 'static,
    ) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<DbError> + Send + 'static,
    {
        let db = self.clone();
        match tokio::task::spawn_blocking(move || db.tx(kind, f)).await {
            Ok(result) => result,
            Err(_) => Err(E::from(DbError::Failed("task failed".to_owned()))),
        }
    }

    /// Runs a script of statements outside a transaction (migrations).
    pub fn batch(&self, sql: &str) -> Result<(), DbError> {
        match &*self.inner {
            Inner::Sqlite(conn) => conn
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .execute_batch(sql)
                .map_err(sqlite_error),
            Inner::Postgres(pool) => {
                let mut client = pool.get()?;
                let result = client
                    .client
                    .as_mut()
                    .expect("checked out")
                    .batch_execute(sql)
                    .map_err(pg_error);
                if result.is_err() {
                    client.discard();
                }
                result
            }
        }
    }
}

fn sqlite_error(error: rusqlite::Error) -> DbError {
    match &error {
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            DbError::Unique
        }
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            DbError::Unavailable
        }
        _ => DbError::Failed(sqlite_kind(&error)),
    }
}

/// The kind of a rusqlite error without the statement's values.
fn sqlite_kind(error: &rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(code, _) => format!("sqlite {:?}", code.code),
        other => format!("sqlite {}", std::any::type_name_of_val(other)),
    }
}

fn pg_error(error: postgres::Error) -> DbError {
    if error.is_closed() {
        return DbError::Unavailable;
    }
    match error.code() {
        Some(code) if *code == postgres::error::SqlState::UNIQUE_VIOLATION => DbError::Unique,
        Some(code) => DbError::Failed(format!("postgres {}", code.code())),
        None => DbError::Unavailable,
    }
}

struct SqliteTx<'a> {
    conn: &'a mut rusqlite::Connection,
}

fn to_sqlite(value: &Val) -> rusqlite::types::Value {
    use rusqlite::types::Value;
    match value {
        Val::Null => Value::Null,
        Val::Int(v) => Value::Integer(*v),
        Val::Bool(v) => Value::Integer(i64::from(*v)),
        Val::Text(v) => Value::Text(v.clone()),
        Val::Bytes(v) => Value::Blob(v.clone()),
        Val::Uuid(v) => Value::Blob(v.as_bytes().to_vec()),
        Val::Time(v) => Value::Text(time_text(*v)),
        Val::Json(v) => Value::Text(v.to_string()),
    }
}

fn from_sqlite(value: rusqlite::types::ValueRef<'_>, ty: Ty) -> Result<Val, DbError> {
    use rusqlite::types::ValueRef;
    let bad = || DbError::Failed(format!("column is not {ty:?}"));
    Ok(match (value, ty) {
        (ValueRef::Null, _) => Val::Null,
        (ValueRef::Integer(v), Ty::Int) => Val::Int(v),
        (ValueRef::Integer(v), Ty::Bool) => Val::Bool(v != 0),
        (ValueRef::Text(v), Ty::Text) => {
            Val::Text(std::str::from_utf8(v).map_err(|_| bad())?.to_owned())
        }
        (ValueRef::Blob(v), Ty::Bytes) => Val::Bytes(v.to_vec()),
        (ValueRef::Blob(v), Ty::Uuid) => Val::Uuid(Uuid::from_slice(v).map_err(|_| bad())?),
        (ValueRef::Text(v), Ty::Time) => {
            Val::Time(parse_time(std::str::from_utf8(v).map_err(|_| bad())?)?)
        }
        (ValueRef::Text(v), Ty::Json) => Val::Json(serde_json::from_slice(v).map_err(|_| bad())?),
        _ => return Err(bad()),
    })
}

impl Tx for SqliteTx<'_> {
    fn exec(&mut self, sql: &str, params: &[Val]) -> Result<u64, DbError> {
        let values: Vec<_> = params.iter().map(to_sqlite).collect();
        let changed = self
            .conn
            .execute(sql, rusqlite::params_from_iter(values))
            .map_err(sqlite_error)?;
        Ok(changed as u64)
    }

    fn query(&mut self, sql: &str, params: &[Val], cols: &[Ty]) -> Result<Vec<Vec<Val>>, DbError> {
        let values: Vec<_> = params.iter().map(to_sqlite).collect();
        let mut statement = self.conn.prepare(sql).map_err(sqlite_error)?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(values))
            .map_err(sqlite_error)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(sqlite_error)? {
            let mut values = Vec::with_capacity(cols.len());
            for (index, ty) in cols.iter().enumerate() {
                let value = row.get_ref(index).map_err(sqlite_error)?;
                values.push(from_sqlite(value, *ty)?);
            }
            out.push(values);
        }
        Ok(out)
    }

    fn lock_vault(&mut self, _vault_id: Uuid) -> Result<(), DbError> {
        Ok(())
    }

    fn backend(&self) -> Backend {
        Backend::Sqlite
    }
}

struct PgTx<'a> {
    tx: Option<postgres::Transaction<'a>>,
}

/// Numbers the `?` placeholders of a statement for PostgreSQL.
pub fn number_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 16);
    let mut n = 0;
    for c in sql.chars() {
        if c == '?' {
            n += 1;
            out.push('$');
            out.push_str(&n.to_string());
        } else {
            out.push(c);
        }
    }
    out
}

fn pg_params(params: &[Val]) -> Vec<Box<dyn postgres::types::ToSql + Sync>> {
    params
        .iter()
        .map(|value| -> Box<dyn postgres::types::ToSql + Sync> {
            match value {
                Val::Null => Box::new(Option::<String>::None),
                Val::Int(v) => Box::new(*v),
                Val::Bool(v) => Box::new(*v),
                Val::Text(v) => Box::new(v.clone()),
                Val::Bytes(v) => Box::new(v.clone()),
                Val::Uuid(v) => Box::new(*v),
                Val::Time(v) => Box::new(*v),
                Val::Json(v) => Box::new(v.clone()),
            }
        })
        .collect()
}

fn from_pg(row: &postgres::Row, index: usize, ty: Ty) -> Result<Val, DbError> {
    let failed = |_| DbError::Failed(format!("column is not {ty:?}"));
    Ok(match ty {
        Ty::Int => match *row.columns()[index].type_() {
            postgres::types::Type::INT4 => row
                .try_get::<_, Option<i32>>(index)
                .map_err(failed)?
                .map_or(Val::Null, |v| Val::Int(i64::from(v))),
            postgres::types::Type::INT2 => row
                .try_get::<_, Option<i16>>(index)
                .map_err(failed)?
                .map_or(Val::Null, |v| Val::Int(i64::from(v))),
            _ => row
                .try_get::<_, Option<i64>>(index)
                .map_err(failed)?
                .map_or(Val::Null, Val::Int),
        },
        Ty::Bool => row
            .try_get::<_, Option<bool>>(index)
            .map_err(failed)?
            .map_or(Val::Null, Val::Bool),
        Ty::Text => row
            .try_get::<_, Option<String>>(index)
            .map_err(failed)?
            .map_or(Val::Null, Val::Text),
        Ty::Bytes => row
            .try_get::<_, Option<Vec<u8>>>(index)
            .map_err(failed)?
            .map_or(Val::Null, Val::Bytes),
        Ty::Uuid => row
            .try_get::<_, Option<Uuid>>(index)
            .map_err(failed)?
            .map_or(Val::Null, Val::Uuid),
        Ty::Time => row
            .try_get::<_, Option<OffsetDateTime>>(index)
            .map_err(failed)?
            .map_or(Val::Null, Val::Time),
        Ty::Json => row
            .try_get::<_, Option<serde_json::Value>>(index)
            .map_err(failed)?
            .map_or(Val::Null, Val::Json),
    })
}

impl Tx for PgTx<'_> {
    fn exec(&mut self, sql: &str, params: &[Val]) -> Result<u64, DbError> {
        let sql = number_placeholders(sql);
        let boxed = pg_params(params);
        let refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            boxed.iter().map(|value| value.as_ref()).collect();
        self.tx
            .as_mut()
            .expect("open")
            .execute(&sql, &refs)
            .map_err(pg_error)
    }

    fn query(&mut self, sql: &str, params: &[Val], cols: &[Ty]) -> Result<Vec<Vec<Val>>, DbError> {
        let sql = number_placeholders(sql);
        let boxed = pg_params(params);
        let refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            boxed.iter().map(|value| value.as_ref()).collect();
        let rows = self
            .tx
            .as_mut()
            .expect("open")
            .query(&sql, &refs)
            .map_err(pg_error)?;
        rows.iter()
            .map(|row| {
                cols.iter()
                    .enumerate()
                    .map(|(index, ty)| from_pg(row, index, *ty))
                    .collect()
            })
            .collect()
    }

    fn lock_vault(&mut self, vault_id: Uuid) -> Result<(), DbError> {
        self.query(
            "SELECT current_epoch FROM vaults WHERE vault_id = ? FOR UPDATE",
            &[Val::Uuid(vault_id)],
            &[Ty::Int],
        )
        .map(|_| ())
    }

    fn backend(&self) -> Backend {
        Backend::Postgres
    }
}

/// A small pool of PostgreSQL connections.
struct PgPool {
    url: String,
    idle: Mutex<Vec<postgres::Client>>,
    available: Condvar,
    open: Mutex<usize>,
    max: usize,
}

struct PgConn<'a> {
    pool: &'a PgPool,
    client: Option<postgres::Client>,
}

impl PgConn<'_> {
    fn build_transaction(&mut self) -> postgres::TransactionBuilder<'_> {
        self.client
            .as_mut()
            .expect("checked out")
            .build_transaction()
    }

    /// Drops a broken connection instead of returning it.
    fn discard(&mut self) {
        if self.client.take().is_some() {
            let mut open = self
                .pool
                .open
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *open = open.saturating_sub(1);
            self.pool.available.notify_one();
        }
    }
}

impl Drop for PgConn<'_> {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            self.pool
                .idle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(client);
            self.pool.available.notify_one();
        }
    }
}

impl PgPool {
    fn new(url: String, max: usize) -> Result<PgPool, DbError> {
        let pool = PgPool {
            url,
            idle: Mutex::new(Vec::new()),
            available: Condvar::new(),
            open: Mutex::new(0),
            max,
        };
        // Fails early on a wrong URL or an unreachable database.
        drop(pool.get()?);
        Ok(pool)
    }

    fn connect(&self) -> Result<postgres::Client, DbError> {
        postgres::Client::connect(&self.url, postgres::NoTls).map_err(|_| DbError::Unavailable)
    }

    fn get(&self) -> Result<PgConn<'_>, DbError> {
        let mut idle = self
            .idle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(client) = idle.pop() {
                if !client.is_closed() {
                    return Ok(PgConn {
                        pool: self,
                        client: Some(client),
                    });
                }
                let mut open = self
                    .open
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *open = open.saturating_sub(1);
                continue;
            }
            {
                let mut open = self
                    .open
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if *open < self.max {
                    *open += 1;
                    drop(open);
                    drop(idle);
                    return match self.connect() {
                        Ok(client) => Ok(PgConn {
                            pool: self,
                            client: Some(client),
                        }),
                        Err(error) => {
                            let mut open = self
                                .open
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            *open = open.saturating_sub(1);
                            Err(error)
                        }
                    };
                }
            }
            let (guard, timeout) = self
                .available
                .wait_timeout(idle, std::time::Duration::from_secs(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            idle = guard;
            if timeout.timed_out() && idle.is_empty() {
                return Err(DbError::Unavailable);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_numbered() {
        assert_eq!(
            number_placeholders("SELECT a FROM t WHERE b = ? AND c = ?"),
            "SELECT a FROM t WHERE b = $1 AND c = $2"
        );
    }

    #[test]
    fn sqlite_round_trips_every_type() {
        let db = Db::open("sqlite::memory:").unwrap();
        db.batch("CREATE TABLE t (i INTEGER, b INTEGER, s TEXT, x BLOB, u BLOB, t TEXT, j TEXT)")
            .unwrap();
        let now = OffsetDateTime::from_unix_timestamp(1_792_510_215).unwrap();
        let id = Uuid::now_v7();
        let row = db
            .tx(TxKind::Write, |tx| -> Result<_, DbError> {
                tx.exec(
                    "INSERT INTO t VALUES (?, ?, ?, ?, ?, ?, ?)",
                    &[
                        Val::Int(7),
                        Val::Bool(true),
                        "a".into(),
                        Val::Bytes(vec![1, 2]),
                        id.into(),
                        now.into(),
                        serde_json::json!({"k": 1}).into(),
                    ],
                )?;
                tx.query_one(
                    "SELECT i, b, s, x, u, t, j FROM t",
                    &[],
                    &[
                        Ty::Int,
                        Ty::Bool,
                        Ty::Text,
                        Ty::Bytes,
                        Ty::Uuid,
                        Ty::Time,
                        Ty::Json,
                    ],
                )
            })
            .unwrap()
            .unwrap();
        assert_eq!(row[0], Val::Int(7));
        assert_eq!(row[1], Val::Bool(true));
        assert_eq!(row[2], Val::Text("a".into()));
        assert_eq!(row[3], Val::Bytes(vec![1, 2]));
        assert_eq!(row[4], Val::Uuid(id));
        assert_eq!(row[5], Val::Time(now));
        assert_eq!(row[6], Val::Json(serde_json::json!({"k": 1})));
        assert_eq!(time_text(now), "2026-10-20T15:30:15.000Z");
    }

    #[test]
    fn a_failed_closure_rolls_back() {
        let db = Db::open("sqlite::memory:").unwrap();
        db.batch("CREATE TABLE t (i INTEGER PRIMARY KEY)").unwrap();
        let result: Result<(), DbError> = db.tx(TxKind::Write, |tx| {
            tx.exec("INSERT INTO t VALUES (1)", &[])?;
            Err(DbError::Failed("stop".into()))
        });
        assert!(result.is_err());
        let count = db
            .tx(TxKind::Read, |tx| -> Result<_, DbError> {
                tx.query_one("SELECT COUNT(*) FROM t", &[], &[Ty::Int])
            })
            .unwrap()
            .unwrap();
        assert_eq!(count[0], Val::Int(0));
        // A duplicate key is reported as such.
        let duplicate: Result<(), DbError> = db.tx(TxKind::Write, |tx| {
            tx.exec("INSERT INTO t VALUES (1)", &[])?;
            tx.exec("INSERT INTO t VALUES (1)", &[])?;
            Ok(())
        });
        assert!(matches!(duplicate, Err(DbError::Unique)));
    }
}
