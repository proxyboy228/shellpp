//! The heap an SFTP transfer holds (run 12; ROADMAP 2.4: the saw-tooth of
//! private bytes under the soak's SFTP load), against OpenSSH in Docker
//! (`SHELLPP_TEST_SSHD=openssh`; skipped with a printed reason otherwise).
//! A counting allocator measures the peak of live heap bytes while files
//! of the soak's sizes go up and down, above what was live before. With
//! `SHELLPP_SFTP_MEMORY=1` it also prints where the largest allocations
//! were made.

// A global allocator is an unsafe trait: this one passes every call to the
// system allocator and only counts beside it.
#![allow(unsafe_code)]

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use shellpp_ssh::{AuthPlan, Connection, KeyCredential, connect};
use support::openssh::{OpenSshServer, Provider, provider};
use support::*;

/// Live heap bytes, and their peak since the last reset.
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Whether allocations of at least [`LARGE_ALLOCATION`] note where they
/// were made.
static TRACE: AtomicBool = AtomicBool::new(false);
const LARGE_ALLOCATION: usize = 512 * 1024;
/// The sites noted: size and backtrace.
static SITES: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());

thread_local! {
    /// Set while a site is noted: its own allocations are not noted again.
    static NOTING: Cell<bool> = const { Cell::new(false) };
}

struct Counting;

impl Counting {
    fn grew(size: usize) {
        let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
        PEAK.fetch_max(live, Ordering::Relaxed);
        if size >= LARGE_ALLOCATION && TRACE.load(Ordering::Relaxed) {
            note(size);
        }
    }
}

fn note(size: usize) {
    NOTING.with(|noting| {
        if noting.replace(true) {
            return;
        }
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        if let Ok(mut sites) = SITES.lock() {
            sites.push((size, trace));
        }
        noting.set(false);
    });
}

// SAFETY: every call goes to the system allocator unchanged; only counters
// are kept beside it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is passed on.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            Self::grew(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is passed on.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            Self::grew(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract is passed on.
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's contract is passed on.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            Self::grew(new_size);
        }
        moved
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The soak's file sizes (apps/desktop/src/dev/soak.rs).
const SIZES: [usize; 3] = [16 * 1024, 1024 * 1024, 8 * 1024 * 1024];
/// Rounds of the three sizes up and down.
const ROUNDS: usize = 3;
/// The most an upload and a download may hold above what was live before:
/// the 2 MiB in flight each way (crates/ssh/src/sftp.rs), the copy buffer
/// and russh's channel window, with room to spare.
const PEAK_LIMIT: usize = 16 * 1024 * 1024;

async fn key_connection(server: &OpenSshServer) -> Connection {
    let plan = AuthPlan {
        key: Some(KeyCredential {
            label: "sftp memory test key".to_owned(),
            private_key: secret(&server.key_user.1),
            passphrase: None,
        }),
        ..AuthPlan::default()
    };
    connect(
        spec(server.addr, Some(&server.key_user.0), plan),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("key connection")
}

#[allow(clippy::print_stderr)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_transfers_hold_little_heap() {
    if provider() != Provider::OpenSsh {
        eprintln!("skipped: SFTP runs against OpenSSH (SHELLPP_TEST_SSHD=openssh)");
        return;
    }
    let Some(server) = OpenSshServer::start() else {
        return;
    };
    let connection = key_connection(&server).await;
    let sftp = connection.open_sftp().await.expect("sftp");
    let local = tempfile::tempdir().unwrap();
    let home = sftp.canonicalize(".").await.expect("home folder");
    let dir = shellpp_ssh::sftp::join(&home, &random_secret("sftp-memory"));
    sftp.create_dir(&dir).await.expect("scratch folder");
    let mut sources = Vec::new();
    for size in SIZES {
        let path = local.path().join(format!("source-{size}"));
        let bytes: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
        std::fs::write(&path, bytes).unwrap();
        sources.push((size, path));
    }
    // A first small round trip: whatever is made once per session is made
    // before the measurement.
    let (size, first) = &sources[0];
    let remote = shellpp_ssh::sftp::join(&dir, "warm");
    sftp.upload(first, &remote, false, &mut |_| {})
        .await
        .unwrap();
    let back = local.path().join("warm-back");
    assert_eq!(
        sftp.download(&remote, &back, false, &mut |_| {})
            .await
            .unwrap(),
        *size as u64
    );

    let tracing = std::env::var("SHELLPP_SFTP_MEMORY").as_deref() == Ok("1");
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    TRACE.store(tracing, Ordering::Relaxed);
    let mut peaks = Vec::new();
    for round in 0..ROUNDS {
        for (size, source) in &sources {
            let remote = shellpp_ssh::sftp::join(&dir, &format!("file-{size}"));
            let start = LIVE.load(Ordering::Relaxed);
            PEAK.store(start, Ordering::Relaxed);
            sftp.upload(source, &remote, false, &mut |_| {})
                .await
                .unwrap();
            let up = PEAK.load(Ordering::Relaxed) - start;
            let back = local.path().join(format!("back-{size}"));
            let start = LIVE.load(Ordering::Relaxed);
            PEAK.store(start, Ordering::Relaxed);
            let got = sftp
                .download(&remote, &back, false, &mut |_| {})
                .await
                .unwrap();
            let down = PEAK.load(Ordering::Relaxed) - start;
            assert_eq!(got, *size as u64);
            peaks.push((round, *size, up, down));
        }
    }
    TRACE.store(false, Ordering::Relaxed);
    let after = LIVE.load(Ordering::Relaxed);
    for (round, size, up, down) in &peaks {
        eprintln!(
            "sftp memory: round {round} size {size}: upload peak +{:.1} MiB, download peak +{:.1} MiB",
            *up as f64 / 1048576.0,
            *down as f64 / 1048576.0
        );
    }
    eprintln!(
        "sftp memory: live before {:.1} MiB, after {:.1} MiB",
        before as f64 / 1048576.0,
        after as f64 / 1048576.0
    );
    if tracing {
        let sites = std::mem::take(&mut *SITES.lock().unwrap());
        let mut counted: std::collections::BTreeMap<String, (usize, usize)> =
            std::collections::BTreeMap::new();
        for (size, trace) in sites {
            // The frames of this workspace's and its dependencies' code,
            // without the allocator's and std's own.
            let key: String = trace
                .lines()
                .filter(|line| {
                    line.contains("russh")
                        || line.contains("shellpp")
                        || line.contains("tokio")
                        || line.contains("bytes")
                })
                .take(12)
                .collect::<Vec<_>>()
                .join("\n");
            let entry = counted.entry(key).or_default();
            entry.0 += 1;
            entry.1 = entry.1.max(size);
        }
        for (key, (count, largest)) in counted {
            eprintln!("sftp memory: {count} allocations up to {largest} bytes at\n{key}\n");
        }
    }
    let worst = peaks
        .iter()
        .map(|(_, _, up, down)| (*up).max(*down))
        .max()
        .unwrap_or(0);
    sftp.remove_tree(&dir).await.unwrap();
    sftp.close().await;
    connection.disconnect().await;
    drop(server);
    assert!(
        worst <= PEAK_LIMIT,
        "an SFTP transfer held {worst} bytes of heap above what was live before"
    );
}
