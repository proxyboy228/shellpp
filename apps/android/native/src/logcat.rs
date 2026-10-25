//! The core's log and panics in logcat under the tag `Shell++`, so a failure
//! on the phone says why (after run 14 sync answered "network" with no trace
//! anywhere). Info and above, as the desktop's log file; the core never logs
//! secrets (`docs/security.md`). A panic is logged before it unwinds, also
//! on threads of its own such as the sync client's.
// One call into Android's liblog (`__android_log_write`), as jni_api.rs
// calls into the JVM.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::io::{self, Write};
use std::os::raw::{c_char, c_int};
use std::sync::Once;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

const INFO: c_int = 4;
const ERROR: c_int = 6;

fn write(priority: c_int, text: &str) {
    // An inner NUL would end the line early.
    let Ok(text) = CString::new(text.replace('\0', " ")) else {
        return;
    };
    // SAFETY: both pointers are NUL-terminated strings alive for the call.
    unsafe {
        __android_log_write(priority, c"Shell++".as_ptr(), text.as_ptr());
    }
}

/// One event's text, written as one logcat line when it is dropped.
struct Line(Vec<u8>);

impl Write for Line {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0);
        let text = text.trim_end();
        if !text.is_empty() {
            write(INFO, text);
        }
    }
}

/// Once per process, at the engine's start.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_writer(|| Line(Vec::new()))
            .without_time()
            .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
            .try_init();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            write(ERROR, &format!("panic: {info}"));
            previous(info);
        }));
    });
}
