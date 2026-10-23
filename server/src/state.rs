//! What every request shares: the database, the configuration, the server
//! keys, the rate limiters and the dummy verifier of unknown emails.

use std::sync::Arc;

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
        })
    }
}
