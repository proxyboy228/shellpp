//! `--soak <minutes>` (run 6, plan section 8; `scripts/soak.ps1`): the app
//! under load for a long time, to find what grows.
//!
//! In the main window, on the test server's host:
//!
//! - [`TABS`] session tabs with output: on OpenSSH most print `seq` in a
//!   shell loop (the journal of the scrollback) and the others run `htop`
//!   (the alternate screen, redrawn); the russh test server's shell has no
//!   loops and no `htop`, so the run types `seq` into every tab again and
//!   again;
//! - a local and a dynamic (SOCKS) forwarding rule on the host, with HTTP
//!   downloads through both all the time: from the host's `busybox httpd`
//!   on OpenSSH, from a server of the run itself on russh (the russh server
//!   runs on this machine, so its forwards reach it);
//! - on OpenSSH, an SFTP connection of the session layer that uploads and
//!   downloads files of three sizes in turn (the russh test server has no
//!   SFTP).
//!
//! The script reads the process's memory, handles and threads from outside;
//! the run prints `bench: soak load` when the load runs, its own counts
//! every minute (`bench: soak minute ...`) and `bench: ready` when the time
//! is up, then idles until `quit`.

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, AsyncApp, Entity};
use serde_json::json;

use super::bench_run::{main_shell, open_host_tab, say, sleep, type_line, wait_settled};
use crate::app::RootView;
use crate::sessions::SessionTarget;

/// Session tabs with output.
pub const TABS: usize = 10;
/// Of them, the ones that run `htop` on OpenSSH (the last ones).
const HTOP_TABS: usize = 3;
/// The port of the host's HTTP server on OpenSSH (loopback of the host).
const HTTP_PORT: u16 = 8080;
/// The file the downloads fetch.
const BLOB_BYTES: usize = 1024 * 1024;
/// The sizes of the SFTP files.
const SFTP_SIZES: [usize; 3] = [16 * 1024, 1024 * 1024, 8 * 1024 * 1024];
/// On russh, each tab gets `seq` again this often.
const RUSSH_SEQ_EVERY: Duration = Duration::from_secs(3);
/// The heap line (`bench: soak heap ...`) comes this often.
const HEAP_EVERY: Duration = Duration::from_secs(5);

/// What the load threads count.
#[derive(Default)]
struct Counts {
    http_local: AtomicU64,
    http_socks: AtomicU64,
    http_failed: AtomicU64,
    sftp_files: AtomicU64,
    sftp_failed: AtomicU64,
}

/// Runs the load for `minutes`; the window and its sessions stay open
/// afterwards (the caller idles until `quit`). `real_shell`: the server is
/// OpenSSH (bash, `htop`, `busybox httpd`, SFTP).
pub async fn run(
    cx: &mut AsyncApp,
    window: AnyWindowHandle,
    root: &Entity<RootView>,
    target: SessionTarget,
    fingerprints: &[String],
    minutes: u64,
    real_shell: bool,
) -> Result<(), String> {
    let SessionTarget::Host {
        vault_id,
        record_id: host,
    } = target
    else {
        return Err("the soak target is a saved host".to_owned());
    };
    let shell = main_shell(cx, root, host).await?;
    let mut handles = Vec::with_capacity(TABS);
    for _ in 0..TABS {
        let (_, pane) = open_host_tab(cx, window, &shell, host, fingerprints).await?;
        wait_settled(cx, &pane).await?;
        let handle = cx
            .update(|cx| {
                pane.read(cx)
                    .terminal()
                    .map(|terminal| terminal.read(cx).handle())
            })
            .ok_or("a tab has no terminal")?;
        handles.push(handle);
    }
    say(&format!("soak tabs {TABS}"));

    let stop = Arc::new(AtomicBool::new(false));
    let counts = Arc::new(Counts::default());
    let mut threads = Vec::new();
    // `SHELLPP_SOAK_LOAD=tabs`, `forward` or `sftp` (OpenSSH): that part of
    // the load only, to find what grows; all of it by default. `direct`: the
    // downloads go to the run's own server without the forwards (russh
    // only): what the run itself costs.
    let only = std::env::var("SHELLPP_SOAK_LOAD").unwrap_or_default();
    match (only.as_str(), real_shell) {
        ("" | "tabs" | "forward", _) | ("sftp", true) | ("direct", false) => {}
        ("sftp", false) => return Err("SHELLPP_SOAK_LOAD=sftp needs OpenSSH".to_owned()),
        ("direct", true) => {
            return Err("SHELLPP_SOAK_LOAD=direct is for the russh server".to_owned());
        }
        (other, _) => return Err(format!("unknown SHELLPP_SOAK_LOAD {other:?}")),
    }
    let direct = only == "direct" && !real_shell;
    let tabs_load = matches!(only.as_str(), "" | "tabs");
    let forward_load = !matches!(only.as_str(), "tabs" | "sftp");
    let sftp_load = matches!(only.as_str(), "" | "sftp");

    // The HTTP server behind the forwards, then the output of every tab.
    let http_port = if real_shell {
        if forward_load {
            type_line(
                &handles[0],
                &format!(
                    "mkdir -p ~/soak-www && head -c {BLOB_BYTES} /dev/urandom > ~/soak-www/blob && \
                     busybox httpd -p 127.0.0.1:{HTTP_PORT} -h ~/soak-www && echo httpd-started"
                ),
            );
            sleep(cx, Duration::from_secs(2)).await;
        }
        for (index, handle) in handles.iter().enumerate() {
            if !tabs_load {
                break;
            }
            if index >= TABS - HTOP_TABS {
                type_line(handle, "htop -d 10");
            } else {
                type_line(handle, "while :; do seq 1 20000; sleep 1; done");
            }
        }
        HTTP_PORT
    } else {
        let (port, server) = http_server(stop.clone())?;
        threads.push(server);
        port
    };

    // The two forwarding rules.
    let state = cx.update(|cx| root.read(cx).state().clone());
    let core = cx
        .update(|cx| state.read(cx).core().cloned())
        .ok_or("no core")?;
    let local_port = free_port()?;
    let socks_port = free_port()?;
    let mut rules = Vec::new();
    for data in [
        json!({"type": "local", "host_id": host.to_string(), "label": "Soak local",
               "bind_port": local_port, "dest_address": "127.0.0.1", "dest_port": http_port}),
        json!({"type": "dynamic", "host_id": host.to_string(), "label": "Soak SOCKS",
               "bind_port": socks_port}),
    ] {
        let rule = core
            .forwarding_rule_save(shellpp_core::service::snippets::RecordInput {
                vault_id,
                record_id: None,
                data: data.as_object().cloned().unwrap_or_default(),
            })
            .map_err(|error| format!("a rule was not saved: {}", error.code()))?;
        rules.push(rule);
    }
    for rule in &rules {
        let started = cx
            .update(|cx| {
                crate::sessions::ui::manager(cx).map(|manager| manager.forward_start(rule))
            })
            .ok_or("no session layer")?;
        let status = cx.background_executor().spawn(started).await;
        if !matches!(status, shellpp_ssh::RuleStatus::Running(_)) {
            return Err(format!(
                "the rule {:?} did not start: {status:?}",
                rule.label
            ));
        }
    }
    say(&format!(
        "soak rules local {local_port}, socks {socks_port}"
    ));

    // The SFTP connection (OpenSSH only).
    let mut sftp_session = None;
    if real_shell && sftp_load {
        let opened = cx
            .update(|cx| {
                crate::sessions::ui::manager(cx).map(|manager| manager.open_sftp(target.clone()))
            })
            .ok_or("no session layer")?
            .map_err(|error| format!("SFTP did not open: {error:?}"))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let sftp = loop {
            if let Some(sftp) = opened.handle.sftp() {
                break sftp;
            }
            if Instant::now() > deadline {
                return Err("SFTP did not connect".to_owned());
            }
            sleep(cx, Duration::from_millis(100)).await;
        };
        threads.push(sftp_thread(sftp, stop.clone(), counts.clone())?);
        sftp_session = Some(opened);
    } else if !real_shell {
        say("soak sftp skipped: the russh test server has no SFTP");
    }

    // The downloads, until the time is up.
    if direct {
        for _ in 0..2 {
            threads.push(http_thread(
                Route::Direct(SocketAddr::from(([127, 0, 0, 1], http_port))),
                http_port,
                stop.clone(),
                counts.clone(),
            ));
        }
    } else if forward_load {
        threads.push(http_thread(
            Route::Direct(SocketAddr::from(([127, 0, 0, 1], local_port))),
            http_port,
            stop.clone(),
            counts.clone(),
        ));
        threads.push(http_thread(
            Route::Socks(SocketAddr::from(([127, 0, 0, 1], socks_port))),
            http_port,
            stop.clone(),
            counts.clone(),
        ));
    }
    say("soak load");
    // The heap's first period starts with the load, not at the app's start.
    let _ = super::heap::take_peak();

    let started = Instant::now();
    let end = started + Duration::from_secs(minutes * 60);
    let mut minute = 0;
    let mut next_seq = Instant::now();
    let mut next_heap = Instant::now();
    while Instant::now() < end {
        // The Rust heap beside the process's numbers the script reads
        // (`dev::heap`): live bytes now and their peak since the last line.
        if Instant::now() >= next_heap {
            say(&format!(
                "soak heap live {} peak {}",
                super::heap::live(),
                super::heap::take_peak()
            ));
            next_heap += HEAP_EVERY;
        }
        if !real_shell && tabs_load && Instant::now() >= next_seq {
            for handle in &handles {
                type_line(handle, "seq 1 20000");
            }
            next_seq = Instant::now() + RUSSH_SEQ_EVERY;
        }
        sleep(cx, Duration::from_millis(500)).await;
        let elapsed = started.elapsed().as_secs() / 60;
        if elapsed > minute {
            minute = elapsed;
            say(&format!(
                "soak minute {minute} http {} socks {} http-failed {} sftp {} sftp-failed {}",
                counts.http_local.load(Ordering::Relaxed),
                counts.http_socks.load(Ordering::Relaxed),
                counts.http_failed.load(Ordering::Relaxed),
                counts.sftp_files.load(Ordering::Relaxed),
                counts.sftp_failed.load(Ordering::Relaxed),
            ));
        }
    }
    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        let _ = thread.join();
    }
    if real_shell {
        // The tabs stop their output; the host's HTTP server and files go.
        for handle in &handles {
            handle.send_input(vec![0x03]);
            type_line(handle, "q");
        }
        type_line(
            &handles[0],
            "pkill -f 'busybox httpd' ; rm -rf ~/soak-www ~/soak-*.bin",
        );
    }
    if let Some(opened) = sftp_session {
        opened.handle.close();
    }
    // With SHELLPP_HEAP_TRACE=1: where the large allocations were made.
    let sites = super::heap::sites();
    let shown = sites.len().min(20);
    for (count, largest, frames) in sites.into_iter().take(shown) {
        say(&format!(
            "soak heap site {count} x up to {largest} bytes: {frames}"
        ));
    }
    let left_out = super::heap::sites_left_out();
    if left_out > 0 {
        say(&format!(
            "soak heap sites: {left_out} allocations at places over the map's limit not kept"
        ));
    }
    say("ready");
    Ok(())
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .map_err(|error| format!("no free port: {error}"))
}

/// The HTTP server of a russh run: `GET` of anything answers the blob.
fn http_server(stop: Arc<AtomicBool>) -> Result<(u16, std::thread::JoinHandle<()>), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let blob: Arc<[u8]> = (0..BLOB_BYTES).map(|byte| (byte % 253) as u8).collect();
    let server = std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let blob = blob.clone();
                    std::thread::spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                        let mut request = [0u8; 1024];
                        let _ = stream.read(&mut request);
                        let head =
                            format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", blob.len());
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&blob);
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    Ok((port, server))
}

/// How a download reaches the HTTP server.
#[derive(Clone, Copy)]
enum Route {
    /// The local forwarding rule's port.
    Direct(SocketAddr),
    /// The dynamic rule's SOCKS5 port.
    Socks(SocketAddr),
}

/// Downloads the blob again and again until `stop`.
fn http_thread(
    route: Route,
    http_port: u16,
    stop: Arc<AtomicBool>,
    counts: Arc<Counts>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match download(route, http_port) {
                Ok(bytes) if bytes >= BLOB_BYTES => {
                    let counter = match route {
                        Route::Direct(_) => &counts.http_local,
                        Route::Socks(_) => &counts.http_socks,
                    };
                    counter.fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    counts.http_failed.fetch_add(1, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    })
}

/// One `GET /blob`; the bytes of the body and headers read.
fn download(route: Route, http_port: u16) -> std::io::Result<usize> {
    let timeout = Some(Duration::from_secs(30));
    let mut stream = match route {
        Route::Direct(addr) => TcpStream::connect_timeout(&addr, Duration::from_secs(10))?,
        Route::Socks(addr) => {
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
            stream.set_read_timeout(timeout)?;
            // SOCKS5: no authentication, CONNECT 127.0.0.1:http_port.
            stream.write_all(&[5, 1, 0])?;
            let mut reply = [0u8; 2];
            stream.read_exact(&mut reply)?;
            let port = http_port.to_be_bytes();
            stream.write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port[0], port[1]])?;
            let mut reply = [0u8; 10];
            stream.read_exact(&mut reply)?;
            if reply[1] != 0 {
                return Err(std::io::Error::other("SOCKS refused"));
            }
            stream
        }
    };
    stream.set_read_timeout(timeout)?;
    stream.write_all(b"GET /blob HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")?;
    let mut total = 0;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Ok(total);
        }
        total += read;
    }
}

/// A source file of the SFTP load: `size` bytes of a fixed pattern.
fn write_source(
    folder: &std::path::Path,
    path: &std::path::Path,
    size: usize,
) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let data: Vec<u8> = (0..size).map(|byte| (byte % 251) as u8).collect();
    std::fs::write(path, data)
}

/// An SFTP failure as the log shows it: the error's own text, without a
/// path (a server message that names one is left out).
fn sftp_failure(step: &str, error: &shellpp_ssh::SftpError) -> String {
    let text = match error {
        shellpp_ssh::SftpError::Failure(message) if message.contains(['/', '\\']) => {
            "the server refused (its message names a path)".to_owned()
        }
        other => other.to_string(),
    };
    format!("soak sftp {step} failed: {text}")
}

/// Uploads and downloads files of [`SFTP_SIZES`] in turn until `stop`. A
/// failure is printed when its text differs from the one before
/// (`bench: soak sftp <step> failed: ...`); a source file that something
/// outside the run removed is written again (run 7: a sweep of the
/// temporary folder at minute 60 took them, and every turn after failed).
fn sftp_thread(
    sftp: Arc<shellpp_ssh::Sftp>,
    stop: Arc<AtomicBool>,
    counts: Arc<Counts>,
) -> Result<std::thread::JoinHandle<()>, String> {
    let folder = std::env::temp_dir().join(format!(
        "shellpp-soak-{}",
        crate::profile::random_suffix().map_err(|error| error.to_string())?
    ));
    let mut files = Vec::new();
    for (index, size) in SFTP_SIZES.iter().enumerate() {
        let path = folder.join(format!("soak-{index}.bin"));
        write_source(&folder, &path, *size).map_err(|error| error.to_string())?;
        files.push((path, *size));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    Ok(std::thread::spawn(move || {
        runtime.block_on(async {
            let home = sftp
                .canonicalize(".")
                .await
                .unwrap_or_else(|_| ".".to_owned());
            let mut turn = 0usize;
            let mut last_failure = None;
            while !stop.load(Ordering::Relaxed) {
                let (local, size) = &files[turn % files.len()];
                if !local.exists() {
                    let made = write_source(&folder, local, *size)
                        .map_or_else(|error| error.kind().to_string(), |()| "done".to_owned());
                    say(&format!(
                        "soak sftp source {} was gone, written again: {made}",
                        turn % files.len()
                    ));
                }
                let remote = format!("{home}/soak-{}.bin", turn % files.len());
                let back = local.with_extension("back");
                let mut progress = |_: u64| {};
                let done = async {
                    sftp.upload(local, &remote, false, &mut progress)
                        .await
                        .map_err(|error| sftp_failure("upload", &error))?;
                    let got = sftp
                        .download(&remote, &back, false, &mut progress)
                        .await
                        .map_err(|error| sftp_failure("download", &error))?;
                    if got == *size as u64 {
                        Ok(())
                    } else {
                        Err("soak sftp download failed: a short file".to_owned())
                    }
                }
                .await;
                match done {
                    Ok(()) => {
                        last_failure = None;
                        counts.sftp_files.fetch_add(2, Ordering::Relaxed)
                    }
                    Err(failure) => {
                        if last_failure.as_ref() != Some(&failure) {
                            say(&failure);
                            last_failure = Some(failure);
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        counts.sftp_failed.fetch_add(1, Ordering::Relaxed)
                    }
                };
                let _ = std::fs::remove_file(&back);
                turn += 1;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        let _ = std::fs::remove_dir_all(&folder);
    }))
}
