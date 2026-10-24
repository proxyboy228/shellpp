//! The sync client of stage 7 (docs/sync-protocol.md; D10-D16, D93): the
//! server origin and certificate trust (`http`, `tls`), the record engine
//! (`engine`), natural keys for duplicates and the merge (`natural`) and
//! Recently deleted (`trash`). The account operations and the sync cycle
//! are methods of [`crate::service::Core`] (`service/sync_ops.rs`).
//!
//! Sync never starts by itself: only after the user connects a server
//! (plan, rule 1.7.4). Without a bound account nothing here runs.

pub(crate) mod engine;
pub mod http;
pub mod natural;
pub mod tls;
pub mod trash;

pub use http::{HttpApi, HttpError, OriginError, normalize_origin};
pub use tls::{CertInfo, SystemRoots, TlsProblem, TrustMode, fingerprint_text};
