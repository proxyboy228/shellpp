//! A count of the Rust heap for the soak run (run 12; ROADMAP 2.4: the
//! saw-tooth of private bytes). In builds with `dev-tools` the binary's
//! global allocator is [`Counting`]: it passes every call to the system
//! allocator and, once [`enable`] was called, counts the live bytes and
//! their peak. The soak prints them beside the process's private bytes,
//! which tells the heap from memory outside it (graphics, the system).
//! With `SHELLPP_HEAP_TRACE=1` an allocation of at least [`LARGE`] also
//! notes where it was made; [`sites`] sums them by place.

// A global allocator is an unsafe trait: this one passes every call to the
// system allocator unchanged and only counts beside it.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
static TRACE: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
/// Allocations from this size on note their place with `SHELLPP_HEAP_TRACE=1`.
pub const LARGE: usize = 1024 * 1024;
/// The places noted: a few frames of the backtrace, how many allocations and
/// the largest. At most [`MAX_SITES`] places.
static SITES: Mutex<BTreeMap<String, (u64, usize)>> = Mutex::new(BTreeMap::new());
const MAX_SITES: usize = 64;

thread_local! {
    /// Set while a place is noted: the noting's own allocations are not.
    static NOTING: Cell<bool> = const { Cell::new(false) };
}

/// The allocator of `dev-tools` builds (see the module).
#[derive(Debug)]
pub struct Counting;

impl Counting {
    #[inline]
    fn grew(size: usize) {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let size = isize::try_from(size).unwrap_or(isize::MAX);
        let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
        PEAK.fetch_max(live, Ordering::Relaxed);
        if size >= LARGE as isize && TRACE.load(Ordering::Relaxed) {
            note(size.unsigned_abs());
        }
    }

    #[inline]
    fn shrank(size: usize) {
        if ENABLED.load(Ordering::Relaxed) {
            LIVE.fetch_sub(
                isize::try_from(size).unwrap_or(isize::MAX),
                Ordering::Relaxed,
            );
        }
    }
}

// SAFETY: every call goes to the system allocator with the caller's
// arguments; the counters beside it never touch the memory.
unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is passed on.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            Self::grew(layout.size());
        }
        pointer
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is passed on.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            Self::grew(layout.size());
        }
        pointer
    }

    #[inline]
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract is passed on.
        unsafe { System.dealloc(pointer, layout) };
        Self::shrank(layout.size());
    }

    #[inline]
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's contract is passed on.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            Self::shrank(layout.size());
            Self::grew(new_size);
        }
        moved
    }
}

/// Starts the count, at the start of `main` for a soak run: the few blocks
/// allocated before and freed later make the count a little low, never
/// more.
pub fn enable() {
    TRACE.store(
        std::env::var("SHELLPP_HEAP_TRACE").as_deref() == Ok("1"),
        Ordering::Relaxed,
    );
    ENABLED.store(true, Ordering::Relaxed);
}

/// The live bytes counted since [`enable`].
pub fn live() -> usize {
    LIVE.load(Ordering::Relaxed).max(0).unsigned_abs()
}

/// The peak of the live bytes since the last call, which starts the next
/// period at the live bytes of now.
pub fn take_peak() -> usize {
    let now = LIVE.load(Ordering::Relaxed);
    PEAK.swap(now, Ordering::Relaxed)
        .max(now)
        .max(0)
        .unsigned_abs()
}

/// The places of large allocations noted so far, the most frequent first:
/// how many, the largest, and the frames.
pub fn sites() -> Vec<(u64, usize, String)> {
    let Ok(sites) = SITES.lock() else {
        return Vec::new();
    };
    let mut list: Vec<_> = sites
        .iter()
        .map(|(frames, (count, largest))| (*count, *largest, frames.clone()))
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    list
}

fn note(size: usize) {
    NOTING.with(|noting| {
        if noting.replace(true) {
            return;
        }
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        // The frames of the app and the libraries that matter here, without
        // the allocator's and std's own; file lines left out.
        let frames: Vec<&str> = trace
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("at "))
            .filter(|line| {
                ["shellpp", "russh", "alacritty", "gpui", "tokio", "bytes"]
                    .iter()
                    .any(|name| line.contains(name))
            })
            .filter(|line| !line.contains("dev::heap"))
            .take(8)
            .collect();
        let key = frames.join(" <- ");
        if let Ok(mut sites) = SITES.lock()
            && (sites.len() < MAX_SITES || sites.contains_key(&key))
        {
            let entry = sites.entry(key).or_default();
            entry.0 += 1;
            entry.1 = entry.1.max(size);
        }
        noting.set(false);
    });
}
