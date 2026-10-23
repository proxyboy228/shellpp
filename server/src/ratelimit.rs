//! Rate limits (docs/sync-protocol.md, section 10.1): request rates through
//! `governor` token buckets keyed by client IP or device, and counters of
//! failures (sign-ins, `current_auth_key` checks, recoveries, TOTP codes) in
//! fixed 15-minute windows, which answer `429` until the window ends. All in
//! memory; IP addresses are never stored.

use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use governor::{DefaultKeyedRateLimiter, Quota};
use uuid::Uuid;

use crate::config::ConfigError;

/// The configured limits (defaults of section 10.1; `SHELLPP_RATE_LIMIT_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimits {
    /// Unauthenticated auth endpoints per IP: per minute and burst.
    pub auth_per_minute: u32,
    pub auth_burst: u32,
    /// Authenticated API per device.
    pub api_per_minute: u32,
    pub api_burst: u32,
    /// Push requests per device per minute.
    pub push_per_minute: u32,
    /// Failed sign-ins (and auth key checks, recoveries) per email per 15 minutes.
    pub failures_per_window: u32,
    /// Failed TOTP codes per account per 15 minutes.
    pub totp_failures_per_window: u32,
    /// Bearer tokens that did not authenticate, per IP per 15 minutes.
    pub token_failures_per_window: u32,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            auth_per_minute: 20,
            auth_burst: 10,
            api_per_minute: 300,
            api_burst: 60,
            push_per_minute: 60,
            failures_per_window: 10,
            totp_failures_per_window: 10,
            token_failures_per_window: 60,
        }
    }
}

impl RateLimits {
    pub fn from_env(get: &impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut limits = Self::default();
        let fields: [(&str, &mut u32); 8] = [
            (
                "SHELLPP_RATE_LIMIT_AUTH_PER_MINUTE",
                &mut limits.auth_per_minute,
            ),
            ("SHELLPP_RATE_LIMIT_AUTH_BURST", &mut limits.auth_burst),
            (
                "SHELLPP_RATE_LIMIT_API_PER_MINUTE",
                &mut limits.api_per_minute,
            ),
            ("SHELLPP_RATE_LIMIT_API_BURST", &mut limits.api_burst),
            (
                "SHELLPP_RATE_LIMIT_PUSH_PER_MINUTE",
                &mut limits.push_per_minute,
            ),
            (
                "SHELLPP_RATE_LIMIT_FAILED_SIGNINS",
                &mut limits.failures_per_window,
            ),
            (
                "SHELLPP_RATE_LIMIT_FAILED_TOTP",
                &mut limits.totp_failures_per_window,
            ),
            (
                "SHELLPP_RATE_LIMIT_FAILED_TOKENS",
                &mut limits.token_failures_per_window,
            ),
        ];
        for (name, field) in fields {
            if let Some(value) = get(name).filter(|value| !value.trim().is_empty()) {
                *field = value
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| ConfigError(format!("{name} is not a positive number")))?;
            }
        }
        Ok(limits)
    }

    /// Limits high enough for tests that do many requests.
    pub fn generous() -> Self {
        Self {
            auth_per_minute: 100_000,
            auth_burst: 100_000,
            api_per_minute: 100_000,
            api_burst: 100_000,
            push_per_minute: 100_000,
            failures_per_window: 10,
            totp_failures_per_window: 10,
            token_failures_per_window: 100_000,
        }
    }
}

/// The length of a failure window.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

/// Counts failures per key in fixed windows.
#[derive(Debug)]
pub struct FailureWindows<K: std::hash::Hash + Eq> {
    windows: Mutex<HashMap<K, (Instant, u32)>>,
}

// Not derived: the key needs no `Default` (an `IpAddr` has none).
impl<K: std::hash::Hash + Eq> Default for FailureWindows<K> {
    fn default() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
        }
    }
}

impl<K: std::hash::Hash + Eq + Clone> FailureWindows<K> {
    /// `Err(seconds)` while `key` has `max` failures in its window.
    pub fn check(&self, key: &K, max: u32, now: Instant) -> Result<(), u64> {
        let windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match windows.get(key) {
            Some((start, count)) if now.duration_since(*start) < WINDOW && *count >= max => {
                Err((WINDOW - now.duration_since(*start)).as_secs().max(1))
            }
            _ => Ok(()),
        }
    }

    /// Records a failure.
    pub fn fail(&self, key: &K, now: Instant) {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if windows.len() > 100_000 {
            windows.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
        }
        let entry = windows.entry(key.clone()).or_insert((now, 0));
        if now.duration_since(entry.0) >= WINDOW {
            *entry = (now, 0);
        }
        entry.1 += 1;
    }
}

/// Every limiter of the server.
pub struct Limiters {
    pub limits: RateLimits,
    auth_ip: DefaultKeyedRateLimiter<IpAddr>,
    api_device: DefaultKeyedRateLimiter<Uuid>,
    push_device: DefaultKeyedRateLimiter<Uuid>,
    /// Keyed by normalized email, known or not (section 10.1).
    pub failures: FailureWindows<String>,
    /// Keyed by account.
    pub totp_failures: FailureWindows<Uuid>,
    /// Bearer tokens that did not authenticate, keyed by client IP.
    pub token_failures: FailureWindows<IpAddr>,
}

impl std::fmt::Debug for Limiters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limiters")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

fn quota(per_minute: u32, burst: u32) -> Quota {
    let rate = NonZeroU32::new(per_minute.max(1)).expect("non-zero");
    let burst = NonZeroU32::new(burst.max(1)).expect("non-zero");
    Quota::per_minute(rate).allow_burst(burst)
}

/// Seconds until a governor bucket lets the next request through.
fn wait_seconds(not_until: governor::NotUntil<governor::clock::QuantaInstant>) -> u64 {
    let clock = governor::clock::QuantaClock::default();
    not_until
        .wait_time_from(governor::clock::Clock::now(&clock))
        .as_secs()
        .max(1)
}

impl Limiters {
    /// Drops the state of keys idle long enough to be full again (IPs and
    /// devices seen once would otherwise stay forever); run periodically.
    pub fn prune(&self) {
        self.auth_ip.retain_recent();
        self.api_device.retain_recent();
        self.push_device.retain_recent();
        self.auth_ip.shrink_to_fit();
        self.api_device.shrink_to_fit();
        self.push_device.shrink_to_fit();
    }

    pub fn new(limits: RateLimits) -> Self {
        Self {
            limits,
            auth_ip: DefaultKeyedRateLimiter::keyed(quota(
                limits.auth_per_minute,
                limits.auth_burst,
            )),
            api_device: DefaultKeyedRateLimiter::keyed(quota(
                limits.api_per_minute,
                limits.api_burst,
            )),
            push_device: DefaultKeyedRateLimiter::keyed(quota(
                limits.push_per_minute,
                limits.push_per_minute,
            )),
            failures: FailureWindows::default(),
            totp_failures: FailureWindows::default(),
            token_failures: FailureWindows::default(),
        }
    }

    /// The unauthenticated auth endpoints, per client IP.
    pub fn auth(&self, ip: IpAddr) -> Result<(), u64> {
        self.auth_ip.check_key(&ip).map_err(wait_seconds)
    }

    /// Any authenticated request, per device.
    pub fn api(&self, device: Uuid) -> Result<(), u64> {
        self.api_device.check_key(&device).map_err(wait_seconds)
    }

    /// A push, per device.
    pub fn push(&self, device: Uuid) -> Result<(), u64> {
        self.push_device.check_key(&device).map_err(wait_seconds)
    }

    /// Before a sign-in or an auth key check for `email`.
    pub fn signin_allowed(&self, email: &str) -> Result<(), u64> {
        self.failures.check(
            &email.to_owned(),
            self.limits.failures_per_window,
            Instant::now(),
        )
    }

    pub fn signin_failed(&self, email: &str) {
        self.failures.fail(&email.to_owned(), Instant::now());
    }

    pub fn totp_allowed(&self, user: Uuid) -> Result<(), u64> {
        self.totp_failures
            .check(&user, self.limits.totp_failures_per_window, Instant::now())
    }

    pub fn totp_failed(&self, user: Uuid) {
        self.totp_failures.fail(&user, Instant::now());
    }

    /// Before a bearer token is looked up for a request from `ip`.
    pub fn token_allowed(&self, ip: IpAddr) -> Result<(), u64> {
        self.token_failures
            .check(&ip, self.limits.token_failures_per_window, Instant::now())
    }

    pub fn token_failed(&self, ip: IpAddr) {
        self.token_failures.fail(&ip, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_block_until_the_window_ends() {
        let windows = FailureWindows::<String>::default();
        let start = Instant::now();
        let key = "alice@example.com".to_owned();
        for _ in 0..10 {
            assert!(windows.check(&key, 10, start).is_ok());
            windows.fail(&key, start);
        }
        let wait = windows.check(&key, 10, start).unwrap_err();
        assert!((899..=900).contains(&wait), "{wait}");
        // Another email is not affected (unknown emails count the same way).
        assert!(
            windows
                .check(&"bob@example.com".to_owned(), 10, start)
                .is_ok()
        );
        // After the window, a new one starts.
        assert!(windows.check(&key, 10, start + WINDOW).is_ok());
        windows.fail(&key, start + WINDOW);
        assert!(windows.check(&key, 10, start + WINDOW).is_ok());
    }

    #[test]
    fn bogus_tokens_are_limited_per_ip() {
        let limiters = Limiters::new(RateLimits {
            token_failures_per_window: 3,
            ..RateLimits::default()
        });
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        for _ in 0..3 {
            assert!(limiters.token_allowed(ip).is_ok());
            limiters.token_failed(ip);
        }
        assert!(limiters.token_allowed(ip).unwrap_err() >= 1);
        assert!(limiters.token_allowed("192.0.2.2".parse().unwrap()).is_ok());
    }

    #[test]
    fn request_rates_have_a_burst() {
        let limiters = Limiters::new(RateLimits {
            auth_per_minute: 20,
            auth_burst: 3,
            ..RateLimits::default()
        });
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        for _ in 0..3 {
            assert!(limiters.auth(ip).is_ok());
        }
        let wait = limiters.auth(ip).unwrap_err();
        assert!(wait >= 1);
        assert!(limiters.auth("192.0.2.2".parse().unwrap()).is_ok());
    }

    #[test]
    fn the_environment_sets_limits() {
        let limits = RateLimits::from_env(&|name: &str| {
            (name == "SHELLPP_RATE_LIMIT_API_BURST").then(|| "5".to_owned())
        })
        .unwrap();
        assert_eq!(limits.api_burst, 5);
        assert_eq!(limits.api_per_minute, 300);
        assert!(RateLimits::from_env(&|_: &str| Some("x".to_owned())).is_err());
    }
}
