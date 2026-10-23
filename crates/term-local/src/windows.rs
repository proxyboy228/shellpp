//! Windows: a helper program runs in a job object, so a timeout ends its whole
//! process tree (D100, item 5). `wsl.exe --list` starts a second `wsl.exe`
//! from `Program Files\WSL`; killing only the first one left the second
//! behind whenever WSL hung. The only `unsafe` code of the crate: kernel32
//! calls on handles this module owns or borrows from a live `Child`.
#![allow(unsafe_code)]

use std::io::{self, Read as _};
use std::os::windows::io::AsRawHandle as _;
use std::os::windows::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::ptr;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};

/// What [`run_in_job`] returns when the program ended in time.
#[derive(Debug)]
pub(crate) struct JobOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
}

/// Why [`run_in_job`] gave up.
#[derive(Debug)]
pub(crate) enum JobError {
    /// The job or the process could not be set up.
    Setup(io::Error),
    /// The program did not end, or its output did not close, in time; the
    /// whole tree was terminated.
    TimedOut,
}

/// A handle closed on drop.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a kernel32 call that returned a new
        // handle owned by this value, and it is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// A job object that kills every process in it when its last handle closes.
struct Job(OwnedHandle);

impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: null attributes and name: an unnamed job with the default
        // security; the returned handle is owned by `OwnedHandle`.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(OwnedHandle(handle));
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(|_| io::Error::other("job limits too large"))?;
        // SAFETY: `limits` is a valid JOBOBJECT_EXTENDED_LIMIT_INFORMATION of
        // `size` bytes that lives across the call; the job handle is open.
        let ok = unsafe {
            SetInformationJobObject(
                job.0.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        // SAFETY: both handles are open: the job is owned by `self`, the
        // process handle is borrowed from `child`, which outlives the call.
        let ok = unsafe { AssignProcessToJobObject(self.0.0, child.as_raw_handle()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Ends every process in the job (the program and all it started);
    /// when that fails, at least `child` itself, so waiting for it returns.
    fn terminate(&self, child: &mut Child) {
        // SAFETY: the job handle is open and owned by `self`.
        if unsafe { TerminateJobObject(self.0.0, 1) } == 0 {
            let _ = child.kill();
        }
    }
}

/// Resumes the threads of a process created suspended (it has one).
fn resume_threads(pid: u32) -> io::Result<()> {
    // SAFETY: a thread snapshot of the system; the handle is owned by
    // `OwnedHandle` (INVALID_HANDLE_VALUE is checked first).
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return Err(io::Error::last_os_error());
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry = THREADENTRY32 {
        dwSize: u32::try_from(size_of::<THREADENTRY32>()).unwrap_or(u32::MAX),
        ..THREADENTRY32::default()
    };
    let mut resumed = 0usize;
    // SAFETY: `entry` is a THREADENTRY32 with `dwSize` set, written by the
    // call; the snapshot handle is open.
    let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: opens the thread by id for resuming; the returned
            // handle is owned by `OwnedHandle` (null is checked first).
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                let thread = OwnedHandle(thread);
                // SAFETY: the thread handle is open with THREAD_SUSPEND_RESUME.
                if unsafe { ResumeThread(thread.0) } != u32::MAX {
                    resumed += 1;
                }
            }
        }
        // SAFETY: as for Thread32First.
        more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other("no thread of the new process was resumed"));
    }
    Ok(())
}

/// Runs `command` (arguments set, stdio set here: no input, standard output
/// read, no error output, no console window) in a new job object, and waits
/// at most `timeout` for it to exit and for its output to close. The process
/// starts suspended and is resumed only once it is in the job, so nothing it
/// starts can escape. On a timeout, or when this returns in any way, the job
/// ends every process left in it.
pub(crate) fn run_in_job(mut command: Command, timeout: Duration) -> Result<JobOutput, JobError> {
    let deadline = Instant::now() + timeout;
    let job = Job::new().map_err(JobError::Setup)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
        .spawn()
        .map_err(JobError::Setup)?;
    if let Err(err) = job.assign(&child).and_then(|()| resume_threads(child.id())) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(JobError::Setup(err));
    }
    let Some(mut stdout) = child.stdout.take() else {
        job.terminate(&mut child);
        let _ = child.wait();
        return Err(JobError::Setup(io::Error::other("no standard output")));
    };
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("shellpp-job-output".to_owned())
        .spawn(move || {
            let mut output = Vec::new();
            let _ = stdout.read_to_end(&mut output);
            let _ = tx.send(output);
        });
    if let Err(err) = reader {
        job.terminate(&mut child);
        let _ = child.wait();
        return Err(JobError::Setup(err));
    }

    let output = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if output.is_ok() && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => break None,
        }
    };
    match (output, status) {
        (Ok(stdout), Some(status)) => Ok(JobOutput {
            success: status.success(),
            stdout,
        }),
        _ => {
            // Ends the program and everything it started; the reader thread
            // then sees its pipe close and exits.
            job.terminate(&mut child);
            let _ = child.wait();
            Err(JobError::TimedOut)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };

    use super::*;

    fn cmd() -> Command {
        let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Command::new(PathBuf::from(system_root).join(r"System32\cmd.exe"))
    }

    /// Every live process: id, parent id and executable name (lowercase).
    fn processes() -> Vec<(u32, u32, String)> {
        // SAFETY: a process snapshot; the handle is owned by `OwnedHandle`.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        assert!(snapshot != INVALID_HANDLE_VALUE && !snapshot.is_null());
        let snapshot = OwnedHandle(snapshot);
        let mut entry = PROCESSENTRY32W {
            dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap(),
            ..PROCESSENTRY32W::default()
        };
        let mut found = Vec::new();
        // SAFETY: `entry` has `dwSize` set and is written by the call.
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            found.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
            // SAFETY: as for Process32FirstW.
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        found
    }

    /// The `ping.exe` processes whose parent is a child of this process.
    fn our_pings() -> Vec<u32> {
        let all = processes();
        let me = std::process::id();
        let children: Vec<u32> = all
            .iter()
            .filter(|(_, parent, _)| *parent == me)
            .map(|(pid, _, _)| *pid)
            .collect();
        all.iter()
            .filter(|(_, parent, name)| children.contains(parent) && name == "ping.exe")
            .map(|(pid, _, _)| *pid)
            .collect()
    }

    #[test]
    fn a_program_that_ends_in_time_gives_its_output() {
        let mut command = cmd();
        command.args(["/d", "/c", "echo shellpp-job"]);
        let output = run_in_job(command, Duration::from_secs(20)).expect("ran");
        assert!(output.success);
        assert!(String::from_utf8_lossy(&output.stdout).contains("shellpp-job"));
    }

    #[test]
    fn a_failing_program_reports_no_success() {
        let mut command = cmd();
        command.args(["/d", "/c", "exit 3"]);
        let output = run_in_job(command, Duration::from_secs(20)).expect("ran");
        assert!(!output.success);
    }

    #[test]
    fn a_timeout_ends_the_whole_tree() {
        // cmd starts ping as its child; the timeout must end both, while the
        // old code killed only cmd and left ping running.
        let mut command = cmd();
        command.args(["/d", "/c", "ping -n 60 127.0.0.1 > nul"]);
        let started = Instant::now();
        // The timeout leaves enough time to see ping running under cmd.
        let handle = std::thread::spawn(move || run_in_job(command, Duration::from_secs(4)));
        let mut pings = Vec::new();
        let wait_until = Instant::now() + Duration::from_secs(4);
        while pings.is_empty() && Instant::now() < wait_until {
            pings = our_pings();
            std::thread::sleep(Duration::from_millis(50));
        }
        let result = handle.join().expect("joined");
        assert!(matches!(result, Err(JobError::TimedOut)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(30));
        assert!(!pings.is_empty(), "ping was never seen running");
        let wait_until = Instant::now() + Duration::from_secs(10);
        let mut alive = pings.clone();
        while !alive.is_empty() && Instant::now() < wait_until {
            let all = processes();
            alive.retain(|pid| all.iter().any(|(live, _, _)| live == pid));
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(alive.is_empty(), "a child outlived the job: {alive:?}");
    }
}
