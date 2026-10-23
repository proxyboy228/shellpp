//! Shell++ sync server (docs/sync-protocol.md; docs/architecture.md,
//! section 10.2). AGPL-3.0-only. It stores accounts, encrypted record
//! envelopes and key wraps and never sees a password, a key or a record's
//! plaintext (principle 5).

#![forbid(unsafe_code)]

pub mod accounts;
pub mod admin;
pub mod api;
pub mod config;
pub mod db;
pub mod error;
pub mod migrate;
pub mod model;
pub mod ratelimit;
pub mod records;
pub mod secrets;
pub mod state;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;

pub use config::Config;
pub use db::Db;
pub use state::Shared;

/// Why the server did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("{0}")]
    Config(#[from] config::ConfigError),
    #[error("database: {0}")]
    Db(#[from] db::DbError),
    #[error(
        "the database schema is version {found}; run `shellpp-server migrate` (needs {needed})"
    )]
    OldSchema { found: i64, needed: i64 },
    #[error("cannot listen: {0}")]
    Listen(std::io::Error),
}

/// Opens the database and checks that its schema is current (`serve`
/// refuses an older one; `migrate` upgrades it).
pub fn open_checked(config: &Config) -> Result<Db, StartError> {
    let db = Db::open(&config.database_url)?;
    let found = migrate::version(&db)?;
    if found != migrate::CURRENT {
        return Err(StartError::OldSchema {
            found,
            needed: migrate::CURRENT,
        });
    }
    Ok(db)
}

/// A server listening on `listener`, with the daily purge.
pub async fn serve_on(
    listener: TcpListener,
    shared: Arc<Shared>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let purge_state = shared.clone();
    let purge = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
        loop {
            interval.tick().await;
            let days = purge_state.config.tombstone_retention_days;
            let result = purge_state
                .db
                .run(db::TxKind::Write, move |tx| {
                    accounts::purge(tx, days, model::now())
                })
                .await;
            match result {
                Ok(count) => tracing::info!(tombstones = count, "purge done"),
                Err(error) => tracing::warn!(code = error.code, "purge failed"),
            }
        }
    });
    let prune_state = shared.clone();
    let prune = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10 * 60));
        loop {
            interval.tick().await;
            prune_state.limiters.prune();
        }
    });
    let app = api::router(shared).into_make_service_with_connect_info::<SocketAddr>();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;
    purge.abort();
    prune.abort();
    result
}

/// Binds `config.bind` and serves until `shutdown`.
pub async fn serve(
    config: Config,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), StartError> {
    let db = open_checked(&config)?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(StartError::Listen)?;
    tracing::info!(bind = %config.bind, "listening");
    let shared = Shared::new(db, config);
    serve_on(listener, shared, shutdown)
        .await
        .map_err(StartError::Listen)
}
