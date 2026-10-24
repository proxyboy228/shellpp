//! What every request shares: the database, the configuration, the server
//! keys, the rate limiters, the dummy verifier of unknown emails and the
//! slots of Argon2id checks.

use std::num::NonZeroUsize;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::config::Config;
use crate::db::Db;
use crate::ratelimit::Limiters;
use crate::secrets::{self, ServerKeys};

pub struct Shared {
    pub db: Db,
    pub config: Config,
    pub keys: ServerKeys,
    pub limiters: Limiters,
    /// A PHC string of a random key, computed at startup: unknown emails are
    /// checked against it so they take as long as known ones (section 4.4).
    pub dummy_verifier: String,
    /// The Argon2id checks and hashes of requests run outside the database
    /// transaction (D174), at most this many at once: each holds 19 MiB.
    /// A check holds its slot until it ends, also when its request is gone.
    pub argon2_slots: Arc<Semaphore>,
    /// Requests waiting for a slot (bounded, cross review of D174).
    pub argon2_waiting: std::sync::atomic::AtomicUsize,
}

/// How many Argon2id checks run at once: half the CPUs, from 1 to 8, so a
/// burst of sign-ins leaves CPU to the other requests (D174).
pub fn argon2_slot_count() -> usize {
    std::thread::available_parallelism()
        .map_or(2, NonZeroUsize::get)
        .div_ceil(2)
        .clamp(1, 8)
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Shared {
    pub fn new(db: Db, config: Config) -> Arc<Shared> {
        let keys = ServerKeys::derive(&config.server_secret);
        let limiters = Limiters::new(config.rate_limits);
        let dummy_verifier = secrets::make_verifier(&secrets::random_bytes::<32>());
        Arc::new(Shared {
            db,
            config,
            keys,
            limiters,
            dummy_verifier,
            argon2_slots: Arc::new(Semaphore::new(argon2_slot_count())),
            argon2_waiting: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}
