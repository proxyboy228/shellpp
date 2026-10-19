//! Process hardening (docs/security.md, section 9.5): no core dumps.
//!
//! - Linux: `prctl(PR_SET_DUMPABLE, 0)` (also blocks unprivileged same-user
//!   `ptrace` attach) and `setrlimit(RLIMIT_CORE, 0)`.
//! - macOS: `setrlimit(RLIMIT_CORE, 0)`.
//! - Windows: nothing; Windows Error Reporting dumps are a residual risk.
//!
//! This is one of the two modules allowed to use `unsafe` (the other is the
//! `gethostname` call in `platform::device_name`).
#![allow(unsafe_code)]

/// What was applied, for one log line at startup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hardening {
    pub core_dumps_disabled: bool,
    pub not_dumpable: bool,
}

/// Applies the hardening of this platform. Failures are reported in the
/// result, never fatal.
pub fn apply() -> Hardening {
    #[allow(unused_mut)]
    let mut result = Hardening::default();
    #[cfg(unix)]
    {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `setrlimit` reads the `rlimit` value we pass by pointer; it
        // stays alive for the duration of the call.
        result.core_dumps_disabled = unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } == 0;
    }
    #[cfg(target_os = "linux")]
    {
        let zero: libc::c_ulong = 0;
        // SAFETY: `PR_SET_DUMPABLE` takes plain integer arguments (passed as
        // `unsigned long`, as the variadic prototype expects); no memory is shared.
        result.not_dumpable =
            unsafe { libc::prctl(libc::PR_SET_DUMPABLE, zero, zero, zero, zero) } == 0;
    }
    result
}
