//! Rate limits (docs/sync-protocol.md, section 10.1): request rates through
//! `governor` token buckets keyed by client IP or device, counters of
//! failures (sign-ins, `current_auth_key` checks, recoveries) in fixed
//! 15-minute windows, which answer `429` until the window ends, and the
//! growing delays of TOTP codes per account and client network with a soft
//! limit per account (D175). All in memory; IP addresses are never stored.

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
    /// Failed TOTP codes of one account from one client network before its
    /// growing delay starts (D175).
    pub totp_free_failures: u32,
    /// Failed TOTP codes of one account, from any network, before the soft
    /// delay of the account starts (D175).
    pub totp_soft_failures: u32,
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
            totp_free_failures: 3,
            totp_soft_failures: 10,
            token_failures_per_window: 60,
        }
    }
}

impl RateLimits {
    pub fn from_env(get: &impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut limits = Self::default();
        let fields: [(&str, &mut u32); 9] = [
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
                &mut limits.totp_soft_failures,
            ),
            (
                "SHELLPP_RATE_LIMIT_FREE_TOTP",
                &mut limits.totp_free_failures,
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
            totp_free_failures: 3,
            totp_soft_failures: 10,
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

/// More windows than this are pruned: ended ones first, then the oldest
/// down to half, so a flood of keys cannot grow the map without bound and
/// prunes at most once per half of it (D174, cross review).
pub const WINDOWS_MAX: usize = 100_000;

fn cap_windows<K: std::hash::Hash + Eq>(windows: &mut HashMap<K, (Instant, u32)>, now: Instant) {
    if windows.len() <= WINDOWS_MAX {
        return;
    }
    windows.retain(|_, (start, _)| now.duration_since(*start) < WINDOW);
    let keep = WINDOWS_MAX / 2;
    if windows.len() > keep {
        let mut starts: Vec<Instant> = windows.values().map(|(start, _)| *start).collect();
        let dropped = starts.len() - keep;
        let (_, cutoff, _) = starts.select_nth_unstable(dropped);
        let cutoff = *cutoff;
        windows.retain(|_, (start, _)| *start >= cutoff);
    }
}

/// The key of an email in the failure windows: its SHA-256, so a long
/// email costs 32 bytes, and no email is kept in memory (D174).
pub fn email_key(normalized_email: &str) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(normalized_email.as_bytes()).into()
}

impl<K: std::hash::Hash + Eq + Clone> FailureWindows<K> {
    /// The number of keys held.
    pub fn len(&self) -> usize {
        self.windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

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

    /// Counts an attempt for `key` before it is checked: `Err(seconds)`
    /// when `key` already has `max` in its window. [`Self::refund`] takes
    /// an attempt that succeeded back, so checks of one key that run at
    /// once never pass the limit together (D174).
    pub fn charge(&self, key: &K, max: u32, now: Instant) -> Result<(), u64> {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cap_windows(&mut windows, now);
        let entry = windows.entry(key.clone()).or_insert((now, 0));
        if now.duration_since(entry.0) >= WINDOW {
            *entry = (now, 0);
        }
        if entry.1 >= max {
            return Err((WINDOW - now.duration_since(entry.0)).as_secs().max(1));
        }
        entry.1 += 1;
        Ok(())
    }

    /// Takes back an attempt [`Self::charge`] counted, once it succeeded.
    pub fn refund(&self, key: &K) {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = windows.get_mut(key) {
            entry.1 = entry.1.saturating_sub(1);
        }
    }

    /// Records a failure.
    pub fn fail(&self, key: &K, now: Instant) {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cap_windows(&mut windows, now);
        let entry = windows.entry(key.clone()).or_insert((now, 0));
        if now.duration_since(entry.0) >= WINDOW {
            *entry = (now, 0);
        }
        entry.1 += 1;
    }
}

/// The first TOTP delay after the free failures; each failure doubles it.
pub const TOTP_FIRST_DELAY: Duration = Duration::from_secs(2);
/// The longest delay per account and client network.
pub const TOTP_PAIR_MAX_DELAY: Duration = Duration::from_secs(15 * 60);
/// The longest delay of the soft limit per account: the longest one try
/// waits because of failures from other networks. Shorter than a login
/// ticket's 5 minutes, so the ticket outlives one wait.
pub const TOTP_ACCOUNT_MAX_DELAY: Duration = Duration::from_secs(4 * 60);
/// Failures are forgotten this long after the last one.
pub const TOTP_MEMORY: Duration = Duration::from_secs(24 * 60 * 60);
/// A network a right code of the account came from stays known this long
/// after the last one.
pub const TOTP_KNOWN_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// The known networks kept per account, the most recent.
const TOTP_KNOWN_MAX: usize = 8;

/// The wait after `failures` failures of which `free` cost nothing: then
/// [`TOTP_FIRST_DELAY`], doubled by each further failure, up to `max`.
fn totp_delay(failures: u32, free: u32, max: Duration) -> Duration {
    let free = free.max(1);
    if failures < free {
        return Duration::ZERO;
    }
    let doublings = (failures - free).min(30);
    TOTP_FIRST_DELAY
        .checked_mul(1 << doublings)
        .unwrap_or(max)
        .min(max)
}

/// Failures and the time of the last one.
#[derive(Clone, Copy, Debug)]
struct Strikes {
    failures: u32,
    last: Instant,
}

impl Strikes {
    /// The failures that still count at `now`.
    fn at(self, now: Instant) -> u32 {
        if now.saturating_duration_since(self.last) >= TOTP_MEMORY {
            0
        } else {
            self.failures
        }
    }

    /// How long from `now` until the next attempt, with this delay rule.
    fn wait(self, now: Instant, free: u32, max: Duration) -> Duration {
        let delay = totp_delay(self.at(now), free, max);
        (self.last + delay).saturating_duration_since(now)
    }
}

/// The TOTP delays (D175; docs/sync-protocol.md, section 10.1): a growing
/// delay per account and client network, and a soft limit per account
/// whose delay never passes [`TOTP_ACCOUNT_MAX_DELAY`]. At sign-in the soft
/// limit does not apply to a network a right code of the account came from
/// (only a right code marks one), so failures from elsewhere never lock
/// those networks out, even when an attacker takes every turn of the soft
/// limit. A delay is an answer `429` with the seconds to wait, never a wait
/// in the request.
#[derive(Debug, Default)]
pub struct TotpDelays {
    pairs: Mutex<HashMap<(Uuid, IpAddr), Strikes>>,
    accounts: Mutex<HashMap<Uuid, Strikes>>,
    known: Mutex<HashMap<Uuid, Vec<(IpAddr, Instant)>>>,
}

/// More entries than this are pruned, forgotten ones first, then the oldest
/// down to half: an attacker with many addresses cannot grow the maps
/// without bound, and a prune runs at most once per half of them.
const TOTP_MAX_ENTRIES: usize = 100_000;

fn lock<T>(map: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    map.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn add_strike<K: std::hash::Hash + Eq>(map: &mut HashMap<K, Strikes>, key: K, now: Instant) {
    if map.len() > TOTP_MAX_ENTRIES {
        map.retain(|_, strikes| strikes.at(now) > 0);
        let keep = TOTP_MAX_ENTRIES / 2;
        if map.len() > keep {
            // The newest `keep` stay.
            let mut times: Vec<Instant> = map.values().map(|strikes| strikes.last).collect();
            let dropped = times.len() - keep;
            let (_, cutoff, _) = times.select_nth_unstable(dropped);
            let cutoff = *cutoff;
            map.retain(|_, strikes| strikes.last >= cutoff);
        }
    }
    let strikes = map.entry(key).or_insert(Strikes {
        failures: 0,
        last: now,
    });
    *strikes = Strikes {
        failures: strikes.at(now).saturating_add(1),
        last: now,
    };
}

impl TotpDelays {
    /// `Err(seconds)` while `user` must wait before a sign-in code (login,
    /// recovery) from `ip`.
    pub fn check(
        &self,
        user: Uuid,
        ip: IpAddr,
        limits: &RateLimits,
        now: Instant,
    ) -> Result<(), u64> {
        self.wait(user, ip, limits, now, true)
    }

    /// [`Self::check`] for a code of a signed-in request (enable, disable,
    /// account deletion): the soft limit holds every network, known or not,
    /// since whoever holds a token is already past the sign-in.
    pub fn check_signed_in(
        &self,
        user: Uuid,
        ip: IpAddr,
        limits: &RateLimits,
        now: Instant,
    ) -> Result<(), u64> {
        self.wait(user, ip, limits, now, false)
    }

    fn wait(
        &self,
        user: Uuid,
        ip: IpAddr,
        limits: &RateLimits,
        now: Instant,
        known_counts: bool,
    ) -> Result<(), u64> {
        let pair = lock(&self.pairs)
            .get(&(user, network_of(ip)))
            .map_or(Duration::ZERO, |strikes| {
                strikes.wait(now, limits.totp_free_failures, TOTP_PAIR_MAX_DELAY)
            });
        let known = known_counts
            && lock(&self.known).get(&user).is_some_and(|networks| {
                networks.iter().any(|(network, seen)| {
                    *network == network_of(ip)
                        && now.saturating_duration_since(*seen) < TOTP_KNOWN_FOR
                })
            });
        let account = match lock(&self.accounts).get(&user) {
            Some(strikes) if !known => {
                strikes.wait(now, limits.totp_soft_failures, TOTP_ACCOUNT_MAX_DELAY)
            }
            _ => Duration::ZERO,
        };
        let wait = pair.max(account);
        if wait.is_zero() {
            Ok(())
        } else {
            // Rounded up: an answer of 1 s is never early.
            Err(wait.as_secs() + u64::from(wait.subsec_nanos() > 0))
        }
    }

    /// A wrong code of `user` from `ip`.
    pub fn fail(&self, user: Uuid, ip: IpAddr, now: Instant) {
        add_strike(&mut lock(&self.pairs), (user, network_of(ip)), now);
        add_strike(&mut lock(&self.accounts), user, now);
    }

    /// A right code of `user` from `ip`: that network starts over. The
    /// count of the account stays (it fades only after [`TOTP_MEMORY`]),
    /// so a sign-in of the user gives an attacker elsewhere no new tries.
    pub fn succeed(&self, user: Uuid, ip: IpAddr, now: Instant) {
        lock(&self.pairs).remove(&(user, network_of(ip)));
        self.seen(user, ip, now);
    }

    /// A right code of the account came from `ip`: the soft limit no longer
    /// holds that network at sign-in.
    fn seen(&self, user: Uuid, ip: IpAddr, now: Instant) {
        let network = network_of(ip);
        let mut known = lock(&self.known);
        let networks = known.entry(user).or_default();
        networks.retain(|(other, seen)| {
            *other != network && now.saturating_duration_since(*seen) < TOTP_KNOWN_FOR
        });
        if networks.len() >= TOTP_KNOWN_MAX {
            networks.remove(0);
        }
        networks.push((network, now));
    }

    /// Drops what is forgotten.
    pub fn prune(&self, now: Instant) {
        lock(&self.pairs).retain(|_, strikes| strikes.at(now) > 0);
        lock(&self.accounts).retain(|_, strikes| strikes.at(now) > 0);
        lock(&self.known).retain(|_, networks| {
            networks.retain(|(_, seen)| now.saturating_duration_since(*seen) < TOTP_KNOWN_FOR);
            !networks.is_empty()
        });
    }
}

/// The longest soft delay of failed sign-ins of one email (D185): the
/// longest one try waits because of failures from other networks.
pub const SIGNIN_SOFT_MAX_DELAY: Duration = Duration::from_secs(4 * 60);
/// A network a successful sign-in of the email came from stays known this
/// long after the last one.
pub const SIGNIN_KNOWN_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// The known networks kept per email, the most recent.
const SIGNIN_KNOWN_MAX: usize = 8;

/// The soft limit of failed sign-ins (and `current_auth_key` checks and
/// recoveries) of one email from every network together (D185): after
/// the free failures, 2 seconds doubling up to [`SIGNIN_SOFT_MAX_DELAY`];
/// it never holds a network a successful sign-in of the email came from in
/// the last 30 days. Keyed by the email's SHA-256; in memory only.
#[derive(Debug, Default)]
pub struct SigninSoft {
    emails: Mutex<HashMap<[u8; 32], Strikes>>,
    known: Mutex<HashMap<[u8; 32], KnownNetworks>>,
}

/// Networks and the time each was last seen.
type KnownNetworks = Vec<(IpAddr, Instant)>;

impl SigninSoft {
    /// `Err(seconds)` while `email` must wait before a try from `ip`.
    pub fn check(&self, email: &[u8; 32], ip: IpAddr, free: u32, now: Instant) -> Result<(), u64> {
        let network = network_of(ip);
        let known = lock(&self.known).get(email).is_some_and(|networks| {
            networks.iter().any(|(other, seen)| {
                *other == network && now.saturating_duration_since(*seen) < SIGNIN_KNOWN_FOR
            })
        });
        if known {
            return Ok(());
        }
        let wait = lock(&self.emails)
            .get(email)
            .map_or(Duration::ZERO, |strikes| {
                strikes.wait(now, free, SIGNIN_SOFT_MAX_DELAY)
            });
        if wait.is_zero() {
            Ok(())
        } else {
            Err(wait.as_secs() + u64::from(wait.subsec_nanos() > 0))
        }
    }

    /// A failed try for `email`.
    pub fn fail(&self, email: &[u8; 32], now: Instant) {
        add_strike(&mut lock(&self.emails), *email, now);
    }

    /// A successful sign-in of `email` from `ip`: that network is known.
    pub fn succeed(&self, email: &[u8; 32], ip: IpAddr, now: Instant) {
        let network = network_of(ip);
        let mut known = lock(&self.known);
        if known.len() > TOTP_MAX_ENTRIES {
            known.retain(|_, networks| {
                networks
                    .retain(|(_, seen)| now.saturating_duration_since(*seen) < SIGNIN_KNOWN_FOR);
                !networks.is_empty()
            });
        }
        if known.len() > TOTP_MAX_ENTRIES {
            // The emails seen longest ago go, down to half (review finding:
            // refusing new ones left every other email without known
            // networks).
            let mut latest: Vec<Instant> = known
                .values()
                .map(|networks| networks.iter().map(|(_, seen)| *seen).max().unwrap_or(now))
                .collect();
            let keep = TOTP_MAX_ENTRIES / 2;
            let last = latest.len() - 1;
            let dropped = latest.len().saturating_sub(keep).min(last);
            let (_, cutoff, _) = latest.select_nth_unstable(dropped);
            let cutoff = *cutoff;
            known.retain(|_, networks| networks.iter().any(|(_, seen)| *seen >= cutoff));
        }
        let networks = known.entry(*email).or_default();
        networks.retain(|(other, seen)| {
            *other != network && now.saturating_duration_since(*seen) < SIGNIN_KNOWN_FOR
        });
        if networks.len() >= SIGNIN_KNOWN_MAX {
            networks.remove(0);
        }
        networks.push((network, now));
    }

    /// Drops what is forgotten.
    pub fn prune(&self, now: Instant) {
        lock(&self.emails).retain(|_, strikes| strikes.at(now) > 0);
        lock(&self.known).retain(|_, networks| {
            networks.retain(|(_, seen)| now.saturating_duration_since(*seen) < SIGNIN_KNOWN_FOR);
            !networks.is_empty()
        });
    }
}

/// The key of the bearer-token window: an IPv4 address, or the /64 of an
/// IPv6 one (one host usually holds a whole /64, so rotating addresses
/// inside it must not reset the window; review finding).
fn network_of(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => {
            let mut segments = v6.segments();
            segments[4..].fill(0);
            IpAddr::from(segments)
        }
    }
}

/// Every limiter of the server.
pub struct Limiters {
    pub limits: RateLimits,
    auth_ip: DefaultKeyedRateLimiter<IpAddr>,
    api_device: DefaultKeyedRateLimiter<Uuid>,
    push_device: DefaultKeyedRateLimiter<Uuid>,
    /// Keyed by normalized email, known or not (section 10.1).
    pub failures: FailureWindows<([u8; 32], IpAddr)>,
    /// The soft limit of failed sign-ins per email (D185).
    pub signin_soft: SigninSoft,
    /// The TOTP delays, per account and client network and per account.
    pub totp: TotpDelays,
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
        self.totp.prune(Instant::now());
        self.signin_soft.prune(Instant::now());
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
            signin_soft: SigninSoft::default(),
            totp: TotpDelays::default(),
            token_failures: FailureWindows::default(),
        }
    }

    /// The unauthenticated auth endpoints, per client IP; an IPv6 client by
    /// its /64, which one host usually holds whole (cross review of D174).
    pub fn auth(&self, ip: IpAddr) -> Result<(), u64> {
        self.auth_ip
            .check_key(&network_of(ip))
            .map_err(wait_seconds)
    }

    /// Any authenticated request, per device.
    pub fn api(&self, device: Uuid) -> Result<(), u64> {
        self.api_device.check_key(&device).map_err(wait_seconds)
    }

    /// A push, per device.
    pub fn push(&self, device: Uuid) -> Result<(), u64> {
        self.push_device.check_key(&device).map_err(wait_seconds)
    }

    /// Before a sign-in or an auth key check for `email` from `ip` (D185):
    /// the soft delay of the email, then the window of the email on that
    /// network, which counts the try as a failure until
    /// [`Self::signin_succeeded`] takes it back (D174).
    pub fn signin_attempt(&self, email: &str, ip: IpAddr, soft_applies: bool) -> Result<(), u64> {
        let key = email_key(email);
        let now = Instant::now();
        let pair = (key, network_of(ip));
        let max = self.limits.failures_per_window;
        // The longer of the two waits, so the answer is never early.
        let window = self.failures.check(&pair, max, now).err().unwrap_or(0);
        let soft = if soft_applies {
            self.signin_soft
                .check(&key, ip, max, now)
                .err()
                .unwrap_or(0)
        } else {
            0
        };
        if window.max(soft) > 0 {
            return Err(window.max(soft));
        }
        self.failures.charge(&pair, max, now)
    }

    /// The check that [`Self::signin_attempt`] counted passed: the try is
    /// taken back and the network is known for the email.
    pub fn signin_succeeded(&self, email: &str, ip: IpAddr) {
        let key = email_key(email);
        self.failures.refund(&(key, network_of(ip)));
        self.signin_soft.succeed(&key, ip, Instant::now());
    }

    /// The check that [`Self::signin_attempt`] counted failed.
    pub fn signin_failed(&self, email: &str) {
        self.signin_soft.fail(&email_key(email), Instant::now());
    }

    /// Before a TOTP code of `user` from `ip` is checked (D175): at sign-in
    /// (login, recovery), or in a signed-in request.
    pub fn totp_allowed(&self, user: Uuid, ip: IpAddr, sign_in: bool) -> Result<(), u64> {
        if sign_in {
            self.totp.check(user, ip, &self.limits, Instant::now())
        } else {
            self.totp
                .check_signed_in(user, ip, &self.limits, Instant::now())
        }
    }

    pub fn totp_failed(&self, user: Uuid, ip: IpAddr) {
        self.totp.fail(user, ip, Instant::now());
    }

    pub fn totp_succeeded(&self, user: Uuid, ip: IpAddr) {
        self.totp.succeed(user, ip, Instant::now());
    }

    /// Before a bearer token is looked up for a request from `ip`.
    pub fn token_allowed(&self, ip: IpAddr) -> Result<(), u64> {
        self.token_failures.check(
            &network_of(ip),
            self.limits.token_failures_per_window,
            Instant::now(),
        )
    }

    /// A bearer token the server never issued, from `ip`.
    pub fn token_failed(&self, ip: IpAddr) {
        self.token_failures.fail(&network_of(ip), Instant::now());
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

    /// D174: an attempt counts before its check, so checks that run at
    /// once never pass the limit together; a success is taken back.
    #[test]
    fn attempts_are_charged_before_the_check_and_refunded_after_a_success() {
        let windows = FailureWindows::<String>::default();
        let start = Instant::now();
        let key = "alice@example.com".to_owned();
        for _ in 0..3 {
            windows.charge(&key, 3, start).expect("within the limit");
        }
        // Three checks in flight: a fourth is refused before it runs.
        assert!(windows.charge(&key, 3, start).is_err());
        // One of them succeeded: its attempt comes back.
        windows.refund(&key);
        windows
            .charge(&key, 3, start)
            .expect("the refunded attempt");
        assert!(windows.charge(&key, 3, start).is_err());
        assert!(windows.check(&key, 3, start).is_err(), "the same counter");
        // A new window starts after the old one.
        windows
            .charge(&key, 3, start + WINDOW)
            .expect("a new window");
        // A refund never goes below zero.
        let other = "bob@example.com".to_owned();
        windows.refund(&other);
        windows.refund(&other);
        for _ in 0..3 {
            windows.charge(&other, 3, start).expect("within the limit");
        }
        assert!(windows.charge(&other, 3, start).is_err());
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// D175: per account and client network, three free failures, then 2 s
    /// doubling to 15 minutes; each wait counts from the last failure.
    #[test]
    fn totp_delays_grow_per_account_and_network_up_to_15_minutes() {
        let delays = TotpDelays::default();
        let limits = RateLimits::default();
        let user = Uuid::now_v7();
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let mut now = Instant::now();
        for _ in 0..2 {
            delays.check(user, ip, &limits, now).expect("free");
            delays.fail(user, ip, now);
        }
        delays
            .check(user, ip, &limits, now)
            .expect("the third is free");
        delays.fail(user, ip, now);
        let mut expected = Vec::new();
        let mut seen = Vec::new();
        for failure in 3..16u32 {
            let wait = delays.check(user, ip, &limits, now).unwrap_err();
            seen.push(wait);
            expected.push((2u64 << (failure - 3)).min(15 * 60));
            // Not a moment before the wait ends.
            assert!(
                delays
                    .check(
                        user,
                        ip,
                        &limits,
                        now + secs(wait) - Duration::from_millis(1)
                    )
                    .is_err()
            );
            now += secs(wait);
            delays
                .check(user, ip, &limits, now)
                .expect("after the wait");
            delays.fail(user, ip, now);
        }
        // The account soft limit is below the pair's from its 10th failure
        // on; the pair's delay is the larger and decides.
        assert_eq!(seen, expected);
        assert_eq!(*seen.last().unwrap(), 900, "at most 15 minutes");
        // IPv6: one /64 is one network.
        let v6: IpAddr = "2001:db8:0:1::1".parse().unwrap();
        let other = Uuid::now_v7();
        for _ in 0..3 {
            delays.fail(other, v6, now);
        }
        assert!(
            delays
                .check(other, "2001:db8:0:1::ffff".parse().unwrap(), &limits, now)
                .is_err()
        );
        // Forgotten a day after the last failure.
        assert!(delays.check(user, ip, &limits, now + TOTP_MEMORY).is_ok());
        delays.fail(user, ip, now + TOTP_MEMORY);
        assert!(
            delays.check(user, ip, &limits, now + TOTP_MEMORY).is_ok(),
            "counted from one again"
        );
    }

    /// D175: a right code resets its network; the count of the account
    /// stays.
    #[test]
    fn a_right_totp_code_resets_its_network_only() {
        let delays = TotpDelays::default();
        let limits = RateLimits {
            totp_soft_failures: 4,
            ..RateLimits::default()
        };
        let user = Uuid::now_v7();
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let now = Instant::now();
        for _ in 0..3 {
            delays.fail(user, ip, now);
        }
        assert!(delays.check(user, ip, &limits, now).is_err());
        delays.succeed(user, ip, now);
        // The network is free; the account has 3 of its 4 free failures.
        delays.check(user, ip, &limits, now).expect("reset");
        delays.fail(user, ip, now);
        // The network is known now: the soft delay does not apply to it.
        delays
            .check(user, ip, &limits, now)
            .expect("a known network");
        let elsewhere: IpAddr = "203.0.113.9".parse().unwrap();
        assert_eq!(
            delays.check(user, elsewhere, &limits, now),
            Err(2),
            "the account count stays for other networks"
        );
    }

    /// D175: the soft limit does not apply to a network the account is
    /// known from, for 30 days after its last use; the own delay of that
    /// network still does.
    #[test]
    fn known_networks_are_not_held_by_the_soft_limit() {
        let delays = TotpDelays::default();
        let limits = RateLimits::default();
        let user = Uuid::now_v7();
        let home: IpAddr = "192.0.2.1".parse().unwrap();
        let mut now = Instant::now();
        delays.seen(user, home, now);
        for i in 0..20u8 {
            let ip = IpAddr::from([203, 0, 113, i]);
            now += secs(delays.check(user, ip, &limits, now).err().unwrap_or(0));
            delays.fail(user, ip, now);
        }
        let fresh: IpAddr = "198.51.100.1".parse().unwrap();
        assert!(delays.check(user, fresh, &limits, now).is_err());
        delays.check(user, home, &limits, now).expect("known");
        // Its own failures still count.
        for _ in 0..3 {
            delays.fail(user, home, now);
        }
        assert_eq!(delays.check(user, home, &limits, now), Err(2));
        // Known for 30 days after the last use; at most 8 per account.
        let later = now + TOTP_KNOWN_FOR;
        assert!(
            delays.check(user, home, &limits, later).is_ok(),
            "failures forgotten too"
        );
        delays.seen(user, home, now);
        for i in 0..8u8 {
            delays.seen(user, IpAddr::from([10, 9, 0, i]), now);
        }
        delays.fail(user, fresh, now);
        assert!(
            delays.check(user, home, &limits, now + secs(4)).is_err(),
            "the oldest of nine networks went"
        );
    }

    /// D175: failures from many networks put a soft delay on the account
    /// that never passes 4 minutes, while each attacking network waits up
    /// to 15 minutes; another network waits at most the soft delay.
    #[test]
    fn the_soft_limit_of_an_account_never_passes_4_minutes() {
        let delays = TotpDelays::default();
        let limits = RateLimits::default();
        let user = Uuid::now_v7();
        let attacker: IpAddr = "198.51.100.7".parse().unwrap();
        let user_ip: IpAddr = "192.0.2.1".parse().unwrap();
        let mut now = Instant::now();
        // Nine failures from nine networks: no delay for anyone yet.
        for i in 0..9u8 {
            let ip = IpAddr::from([203, 0, 113, i]);
            delays.fail(user, ip, now);
        }
        delays
            .check(user, user_ip, &limits, now)
            .expect("below the soft limit");
        // Many more, from many networks, each as soon as allowed.
        for i in 0..200u32 {
            let ip = IpAddr::from([10, 1, (i / 256) as u8, (i % 256) as u8]);
            if let Err(wait) = delays.check(user, ip, &limits, now) {
                assert!(wait <= 240, "the soft delay is at most 4 minutes: {wait}");
                now += secs(wait);
            }
            delays.fail(user, ip, now);
        }
        let soft = delays.check(user, user_ip, &limits, now).unwrap_err();
        assert_eq!(soft, 240, "at its cap");
        // The attacker's own network also has its growing delay.
        for _ in 0..12 {
            now += secs(
                delays
                    .check(user, attacker, &limits, now)
                    .err()
                    .unwrap_or(0),
            );
            delays.fail(user, attacker, now);
        }
        assert_eq!(delays.check(user, attacker, &limits, now), Err(900));
        assert_eq!(delays.check(user, user_ip, &limits, now), Err(240));
        // After the soft delay the user's network may try; the attacker's
        // still waits.
        now += secs(240);
        delays
            .check(user, user_ip, &limits, now)
            .expect("the user's turn");
        assert_eq!(delays.check(user, attacker, &limits, now), Err(660));
    }

    /// Review finding (D175): past 100,000 entries the oldest go down to
    /// half at once, so a flood costs one prune per 50,000 failures; the
    /// newest entries stay.
    #[test]
    fn totp_maps_are_pruned_to_their_newest_half() {
        let delays = TotpDelays::default();
        let user = Uuid::now_v7();
        let start = Instant::now();
        for i in 0..=TOTP_MAX_ENTRIES as u32 {
            let ip = IpAddr::from(i.to_be_bytes());
            delays.fail(user, ip, start + Duration::from_millis(u64::from(i)));
        }
        let newest = IpAddr::from((TOTP_MAX_ENTRIES as u32).to_be_bytes());
        assert_eq!(lock(&delays.pairs).len(), TOTP_MAX_ENTRIES + 1);
        delays.fail(user, newest, start + secs(200));
        let pairs = lock(&delays.pairs);
        assert!(pairs.len() <= TOTP_MAX_ENTRIES / 2 + 1, "{}", pairs.len());
        assert_eq!(pairs.get(&(user, newest)).map(|s| s.failures), Some(2));
        assert!(!pairs.contains_key(&(user, IpAddr::from(0u32.to_be_bytes()))));
        drop(pairs);
        assert_eq!(
            lock(&delays.accounts).get(&user).map(|s| s.failures),
            Some(TOTP_MAX_ENTRIES as u32 + 2),
            "the account count stays"
        );
    }

    /// Cross review (D174): an email of any length is kept as its 32-byte
    /// hash, and 200,000 different keys keep the map at its cap.
    #[test]
    fn failure_windows_keep_hashed_keys_and_stay_at_their_cap() {
        let limiters = Limiters::new(RateLimits::default());
        let long = "x".repeat(1 << 20);
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        limiters.signin_attempt(&long, ip, true).unwrap();
        assert_eq!(limiters.failures.len(), 1);
        assert!(
            limiters
                .failures
                .check(&(email_key(&long), ip), 1, Instant::now())
                .is_err(),
            "counted under its 32-byte hash"
        );
        let windows = FailureWindows::<u32>::default();
        let start = Instant::now();
        for i in 0..200_000u32 {
            windows.fail(&i, start + Duration::from_millis(u64::from(i) / 100));
            assert!(windows.len() <= WINDOWS_MAX + 1);
        }
        // The newest keys stay.
        assert!(windows.check(&199_999, 1, start + secs(2)).is_err());
    }

    /// Cross review (D174): two addresses of one IPv6 /64 share the
    /// bucket of the auth endpoints; another /64 has its own.
    #[test]
    fn the_auth_limit_keys_ipv6_by_its_64() {
        let limiters = Limiters::new(RateLimits {
            auth_per_minute: 1,
            auth_burst: 2,
            ..RateLimits::default()
        });
        limiters.auth("2001:db8:0:1::1".parse().unwrap()).unwrap();
        limiters.auth("2001:db8:0:1::2".parse().unwrap()).unwrap();
        assert!(
            limiters
                .auth("2001:db8:0:1:ffff::3".parse().unwrap())
                .is_err()
        );
        limiters.auth("2001:db8:0:2::1".parse().unwrap()).unwrap();
    }

    /// D185: the soft delay of an email holds every network but the ones
    /// a successful sign-in came from, and never passes 4 minutes.
    #[test]
    fn the_soft_signin_limit_spares_known_networks_and_stays_below_4_minutes() {
        let soft = SigninSoft::default();
        let email = email_key("alice@example.com");
        let home: IpAddr = "192.0.2.1".parse().unwrap();
        let fresh: IpAddr = "198.51.100.1".parse().unwrap();
        let mut now = Instant::now();
        soft.succeed(&email, home, now);
        for i in 0..40u8 {
            let ip = IpAddr::from([203, 0, 113, i]);
            if let Err(wait) = soft.check(&email, ip, 10, now) {
                assert!(wait <= 240, "{wait}");
                now += secs(wait);
            }
            soft.fail(&email, now);
        }
        assert_eq!(soft.check(&email, fresh, 10, now), Err(240));
        soft.check(&email, home, 10, now).expect("a known network");
        // Known for 30 days after the last success.
        assert!(
            soft.check(&email, home, 10, now + SIGNIN_KNOWN_FOR).is_ok(),
            "forgotten too"
        );
        soft.fail(&email, now + SIGNIN_KNOWN_FOR);
        assert!(
            soft.check(&email, home, 10, now + SIGNIN_KNOWN_FOR).is_ok(),
            "counted from one"
        );
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

        // IPv6: one /64 is one window, another /64 has its own.
        let first: IpAddr = "2001:db8:0:1::1".parse().unwrap();
        for _ in 0..3 {
            limiters.token_failed(first);
        }
        assert!(
            limiters
                .token_allowed("2001:db8:0:1:ffff::2".parse().unwrap())
                .is_err(),
            "another address in the same /64"
        );
        assert!(
            limiters
                .token_allowed("2001:db8:0:2::1".parse().unwrap())
                .is_ok()
        );
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
