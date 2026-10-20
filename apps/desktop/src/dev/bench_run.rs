//! The app side of the terminal benchmarks ([`super::bench`]): the demo
//! profile with a host for the test server, GPUI with the session layer
//! (no key files, no agent: only the test server's password is used) and
//! the drivers of the three modes.
//!
//! - `--bench-sessions N`: the app's main window with N real session tabs
//!   (the shell's `open_session`, the same panes, terminal views and session
//!   layer as a Connect); after `bench: ready` the app idles until a `quit`
//!   line arrives on stdin (`scripts/measure-terminal.ps1` sends it; closing
//!   the window asks first while sessions run, UI 4.6) or the window closes.
//! - `--bench-output`, `--bench-latency`: a benchmark window that shows one
//!   terminal view (the tabs' view and session layer) and a probe. The
//!   session layer's local backend is the app's PTY backend plus one extra
//!   shell id, [`DIRECT_SHELL_ID`], that starts `seq` itself as the program
//!   (`--bench-output`'s `local-direct` run). The `bare-*` runs of
//!   `--bench-output` ([`super::bench_pty`]) use no window and no terminal;
//!   their PTY has the size of the benchmark window's terminal grid.
//!
//! Timing: a zero-size probe element painted after the terminal records
//! when each frame is drawn (presentation follows at once). A measurement
//! arms the probe with a check of the terminal's screen; the first frame
//! whose paint sees the check pass ends it. The window is activated first;
//! the output says whether it was active (an inactive window may be
//! throttled by the platform).

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, Entity, IntoElement, Keystroke,
    ParentElement as _, Render, Styled as _, Window, WindowBounds, WindowOptions, canvas, div, px,
    size,
};
use shellpp_term_local::{LocalError, ShellInfo, ShellSpec};

use super::bench::{
    BenchArgs, BenchMode, LocalSeq, OutputKind, latency_char, latency_line_start, local_seq,
    millis, percentile, seq_direct_finished, seq_finished, seq_shell_finished,
};
use super::bench_pty::{self, BareRun, GridSize};
use super::demo;
use super::test_server::DevServer;
use crate::app::{self, RootView};
use crate::session_ui::{HostKeyAsk, HostKeyReply, Phase, SessionPane};
use crate::sessions::transport::TermParts;
use crate::sessions::{
    HostKeyChoice, KeySources, LocalBackend, ManagerOptions, OpenRequest, PromptKind, PromptReply,
    PtyBackend, SessionHandle, SessionState, SessionTarget,
};
use crate::shell::{SessionKind, SessionRequest};
use crate::state::AppState;
use crate::terminal::{TerminalOptions, TerminalView};
use crate::theme;

/// Connecting and the shell's first prompt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// One echoed keystroke.
const KEY_TIMEOUT: Duration = Duration::from_secs(10);
/// The whole output benchmark of one terminal.
const OUTPUT_TIMEOUT: Duration = Duration::from_secs(900);
/// The screen must stay unchanged this long to count as settled.
const SETTLE: Duration = Duration::from_millis(400);
/// Pause between keystrokes (no pipelining of echoes).
const KEY_GAP: Duration = Duration::from_millis(30);
/// The benchmark window (the app's default window size).
const WINDOW_WIDTH: f32 = 1280.;
const WINDOW_HEIGHT: f32 = 800.;
/// The local shell id that starts `seq 1 <N>` itself (no shell).
const DIRECT_SHELL_ID: &str = "bench-seq-direct";

#[allow(clippy::print_stdout)]
fn say(line: &str) {
    println!("bench: {line}");
}

/// Runs one benchmark and returns the process exit code.
pub fn run(args: BenchArgs) -> ExitCode {
    let server = match DevServer::start(args.server_json.as_deref()) {
        Ok(server) => server,
        Err(error) => {
            say(&format!("failed test server: {error}"));
            return ExitCode::from(1);
        }
    };
    say(&format!("server {}", server.kind.label()));
    // Only the test server's own host keys are accepted.
    let fingerprints: Rc<[String]> = server.host_key_fingerprints.clone().into();
    let profile = match demo::create_profile() {
        Ok(profile) => profile,
        Err(error) => {
            say(&format!("failed {error}"));
            return ExitCode::from(1);
        }
    };
    crate::logging::init(&profile.dirs().logs);
    let (opened, summary) = match demo::open(profile) {
        Ok(opened) => opened,
        Err(error) => {
            say(&format!("failed {error}"));
            return ExitCode::from(1);
        }
    };
    say(&format!("demo profile with {} hosts", summary.hosts));
    let target = match server.save_host(&opened.core, "Benchmark server") {
        Ok((vault_id, record_id)) => SessionTarget::Host {
            vault_id,
            record_id,
        },
        Err(error) => {
            say(&format!("failed saving the test server host: {error}"));
            return ExitCode::from(1);
        }
    };

    let outcome: Rc<RefCell<Option<Result<(), String>>>> = Rc::new(RefCell::new(None));
    let outcome_in_app = outcome.clone();
    let mode = args.mode.clone();
    let appearance = args.appearance;
    let seq = local_seq();
    let direct = match &mode {
        BenchMode::Output { lines } => seq.direct_program().map(|program| ShellSpec {
            id: Some(DIRECT_SHELL_ID.to_owned()),
            label: "seq".to_owned(),
            program: program.to_path_buf(),
            args: vec!["1".to_owned(), lines.to_string()],
        }),
        BenchMode::Sessions(_) | BenchMode::Latency { .. } => None,
    };
    gpui_kit::application()
        .with_assets(app::assets::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(appearance, cx);
            app::root::bind_keys(cx);
            crate::terminal::init(cx);
            let state = cx.new(|cx| AppState::new(opened, cx));
            // Never the user's ~/.ssh or agent: the host has a password.
            let options = ManagerOptions {
                local: Arc::new(BenchLocal {
                    pty: PtyBackend::new(),
                    direct,
                }),
                ..ManagerOptions::with_keys(KeySources::none())
            };
            let installed = crate::sessions::ui::install(&state, options, cx);
            let windows: Rc<RefCell<Windows>> = Rc::new(RefCell::new(Windows::default()));
            let quit_state = state.clone();
            let quit_windows = windows.clone();
            cx.on_app_quit(move |cx| {
                release_sessions(&quit_windows.borrow(), cx);
                // The core is no longer shared: its files close and the
                // demo profile is deleted.
                quit_state.update(cx, |state, _| state.shutdown());
                async {}
            })
            .detach();
            // Closing a window ends the run.
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            if !installed {
                *outcome_in_app.borrow_mut() = Some(Err("the session layer did not start".into()));
                cx.quit();
                return;
            }
            if let BenchMode::Sessions(count) = mode {
                let Ok((window, root)) = app::open_main_window(state.clone(), cx) else {
                    *outcome_in_app.borrow_mut() = Some(Err("the main window did not open".into()));
                    cx.quit();
                    return;
                };
                windows.borrow_mut().main = Some(root.clone());
                let quit = quit_command();
                cx.activate(true);
                let SessionTarget::Host { record_id, .. } = target else {
                    unreachable!("the benchmark target is a saved host");
                };
                cx.spawn(async move |cx: &mut AsyncApp| {
                    let result =
                        session_tabs(cx, window, &root, count, record_id, &fingerprints).await;
                    let ready = result.is_ok();
                    *outcome_in_app.borrow_mut() = Some(result);
                    if ready && quit.recv().await.is_err() {
                        // No stdin to listen to: the window ends the run.
                        futures::future::pending::<()>().await;
                    }
                    close_tabs(cx, &root);
                    // The closed tabs let go of their sessions first.
                    sleep(cx, Duration::from_millis(300)).await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
                return;
            }
            let bench = match open_bench_window(cx) {
                Ok(bench) => bench,
                Err(error) => {
                    *outcome_in_app.borrow_mut() = Some(Err(error));
                    cx.quit();
                    return;
                }
            };
            windows.borrow_mut().bench = Some(bench.root.clone());
            cx.activate(true);
            cx.spawn(async move |cx: &mut AsyncApp| {
                let result = drive(cx, &bench, &mode, target, &fingerprints, &seq).await;
                *outcome_in_app.borrow_mut() = Some(result);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    drop(server);
    let result = outcome.borrow_mut().take();
    match result {
        Some(Ok(())) => ExitCode::SUCCESS,
        Some(Err(error)) => {
            say(&format!("failed {error}"));
            ExitCode::from(1)
        }
        None => {
            say("failed the run ended early");
            ExitCode::from(1)
        }
    }
}

/// The local backend of the benchmarks: the app's PTY backend, plus
/// [`DIRECT_SHELL_ID`] for `seq` as the program of `--bench-output`'s
/// `local-direct` run.
struct BenchLocal {
    pty: PtyBackend,
    direct: Option<ShellSpec>,
}

impl LocalBackend for BenchLocal {
    fn shells(&self, refresh: bool) -> Vec<ShellInfo> {
        self.pty.shells(refresh)
    }

    fn default_id(&self, shells: &[ShellInfo]) -> Option<String> {
        self.pty.default_id(shells)
    }

    fn resolve(
        &self,
        requested: Option<&str>,
        setting: Option<&str>,
        shells: &[ShellInfo],
    ) -> Result<ShellSpec, LocalError> {
        if requested == Some(DIRECT_SHELL_ID) {
            return self.direct.clone().ok_or(LocalError::NoShell);
        }
        self.pty.resolve(requested, setting, shells)
    }

    fn spawn(
        &self,
        spec: &ShellSpec,
        cols: u16,
        rows: u16,
        term: &str,
    ) -> Result<TermParts, LocalError> {
        self.pty.spawn(spec, cols, rows, term)
    }
}

/// The run's windows, for the release at quit.
#[derive(Default)]
struct Windows {
    /// The main window of `--bench-sessions`.
    main: Option<Entity<RootView>>,
    /// The benchmark window of `--bench-output` and `--bench-latency`.
    bench: Option<Entity<BenchRoot>>,
}

/// Ends every session and lets go of everything that shares the core (the
/// session manager, the sessions' handles in the terminals' sinks), so that
/// the app state can close the core and delete the demo profile: Windows
/// cannot delete an open database.
fn release_sessions(windows: &Windows, cx: &mut App) {
    if let Some(root) = &windows.main
        && let Some(shell) = root.read(cx).shell().cloned()
    {
        shell.update(cx, |shell, cx| shell.shutdown_sessions(cx));
    }
    crate::sessions::ui::shutdown(cx);
    if let Some(root) = &windows.bench {
        let terminals = root.read(cx).terminals.clone();
        for terminal in terminals {
            terminal.read(cx).handle().set_sink(None);
        }
    }
    if cx.has_global::<crate::sessions::ui::Sessions>() {
        drop(cx.remove_global::<crate::sessions::ui::Sessions>());
    }
}

/// A `quit` line on stdin (the measurement script sends it after reading
/// the memory). The channel closes without a message when stdin ends
/// first.
fn quit_command() -> async_channel::Receiver<()> {
    let (sender, receiver) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("bench-stdin".to_owned())
        .spawn(move || {
            use std::io::BufRead as _;
            for line in std::io::stdin().lock().lines() {
                match line {
                    Ok(line) if line.trim() == "quit" => {
                        let _ = sender.send_blocking(());
                        return;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        });
    if spawned.is_err() {
        say("stdin is not read; close the window to end the run");
    }
    receiver
}

/// When frames are painted, and the check a measurement waits for.
#[derive(Default)]
struct Probe {
    frames: u64,
    check: Option<Box<dyn Fn() -> bool>>,
    hit: Option<Instant>,
}

impl Probe {
    /// Called from the paint of every frame.
    fn painted(&mut self) {
        self.frames += 1;
        if self.hit.is_none() && self.check.as_ref().is_some_and(|check| check()) {
            self.hit = Some(Instant::now());
            self.check = None;
        }
    }
}

/// The benchmark window's view: the active terminal, full size, and the
/// probe painted after it.
struct BenchRoot {
    terminals: Vec<Entity<TerminalView>>,
    active: Option<usize>,
    probe: Rc<RefCell<Probe>>,
}

impl Render for BenchRoot {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let probe = self.probe.clone();
        let terminal = self
            .active
            .and_then(|index| self.terminals.get(index).cloned());
        div().size_full().relative().children(terminal).child(
            canvas(
                |_, _, _| {},
                move |_, (), _, _| probe.borrow_mut().painted(),
            )
            .absolute()
            .size_0(),
        )
    }
}

/// The benchmark window and its view.
struct Bench {
    window: AnyWindowHandle,
    root: Entity<BenchRoot>,
    probe: Rc<RefCell<Probe>>,
}

fn open_bench_window(cx: &mut App) -> Result<Bench, String> {
    let probe = Rc::new(RefCell::new(Probe::default()));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(
            size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
            cx,
        )),
        ..WindowOptions::default()
    };
    let root_probe = probe.clone();
    let (window, root) = gpui_kit::open_window(options, cx, move |_, cx| {
        cx.new(|_| BenchRoot {
            terminals: Vec::new(),
            active: None,
            probe: root_probe,
        })
    })
    .map_err(|_| "the benchmark window did not open".to_owned())?;
    Ok(Bench {
        window,
        root,
        probe,
    })
}

/// One terminal of the benchmark: its view and session.
struct BenchTerminal {
    view: Entity<TerminalView>,
    session: SessionHandle,
    events: futures::channel::mpsc::UnboundedReceiver<crate::sessions::SessionEvent>,
}

impl BenchTerminal {
    fn screen(&self, cx: &mut AsyncApp) -> Vec<String> {
        cx.update(|cx| self.view.read(cx).handle().screen_text())
    }
}

async fn sleep(cx: &mut AsyncApp, duration: Duration) {
    cx.background_executor().timer(duration).await;
}

/// Opens a terminal view in the benchmark window, makes it the visible one
/// and opens `request` on it.
async fn open_terminal(
    cx: &mut AsyncApp,
    bench: &Bench,
    request: OpenRequest,
) -> Result<BenchTerminal, String> {
    let view = add_view(cx, bench).await?;
    open_session(cx, view, request)
}

/// A new terminal view in the benchmark window, visible, focused and laid
/// out (its grid has the window's size), without a session.
async fn add_view(cx: &mut AsyncApp, bench: &Bench) -> Result<Entity<TerminalView>, String> {
    let view = cx
        .update_window(bench.window, |_, window, cx| {
            cx.new(|cx| TerminalView::new(TerminalOptions::default(), window, cx))
        })
        .map_err(|_| "the benchmark window closed".to_owned())?;
    cx.update(|cx| {
        bench.root.update(cx, |root, cx| {
            root.terminals.push(view.clone());
            root.active = Some(root.terminals.len() - 1);
            cx.notify();
        });
    });
    // The first layout gives the grid its real size.
    cx.update_window(bench.window, |_, window, cx| {
        view.update(cx, |view, cx| view.focus(window, cx));
        window.refresh();
    })
    .map_err(|_| "the benchmark window closed".to_owned())?;
    sleep(cx, Duration::from_millis(100)).await;
    Ok(view)
}

/// Opens `request` on the terminal `view`.
fn open_session(
    cx: &mut AsyncApp,
    view: Entity<TerminalView>,
    request: OpenRequest,
) -> Result<BenchTerminal, String> {
    let opened = cx.update(|cx| {
        let handle = view.read(cx).handle();
        let size = handle.size();
        let manager = crate::sessions::ui::manager(cx).ok_or("no session manager")?;
        manager
            .open(request, (size.cols, size.rows), Arc::new(handle))
            .map_err(|error| error.message().to_owned())
    })?;
    Ok(BenchTerminal {
        view,
        session: opened.handle,
        events: opened.events,
    })
}

/// Waits until the session is connected, answering the first-connection
/// dialog with "Accept and save" only for one of the test server's own
/// keys (`fingerprints`); any other key ends the run.
async fn wait_connected(
    cx: &mut AsyncApp,
    terminal: &mut BenchTerminal,
    fingerprints: &[String],
) -> Result<(), String> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        while terminal.events.try_recv().is_ok() {}
        let view = terminal.session.view();
        match view.state {
            SessionState::Connected => return Ok(()),
            SessionState::Failed | SessionState::Ended | SessionState::Closed => {
                return Err(format!(
                    "the session did not connect: {}",
                    view.message().unwrap_or_default()
                ));
            }
            _ => {}
        }
        if let Some(prompt) = &view.prompt {
            let reply = match &prompt.kind {
                PromptKind::HostKeyUnknown { key } => {
                    check_host_key(&key.fingerprint, fingerprints)?;
                    PromptReply::HostKey(HostKeyChoice::AcceptAndSave)
                }
                other => return Err(format!("unexpected {} prompt", other.name())),
            };
            let _ = terminal.session.reply(prompt.id, reply);
        }
        if Instant::now() > deadline {
            return Err("timeout while connecting".to_owned());
        }
        sleep(cx, Duration::from_millis(10)).await;
    }
}

/// Waits until the screen shows something and stays unchanged for
/// [`SETTLE`] (the shell's prompt after its start-up output). Returns the
/// last non-empty row (the prompt).
async fn wait_prompt(cx: &mut AsyncApp, terminal: &BenchTerminal) -> Result<String, String> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut last: Vec<String> = Vec::new();
    let mut since = Instant::now();
    loop {
        let screen = terminal.screen(cx);
        if screen != last {
            last = screen;
            since = Instant::now();
        } else if since.elapsed() >= SETTLE
            && let Some(prompt) = last.iter().rev().find(|row| !row.trim().is_empty())
        {
            return Ok(prompt.clone());
        }
        if Instant::now() > deadline {
            return Err("no shell prompt".to_owned());
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
}

/// Arms the probe with `check` and waits for the frame that passes it.
async fn measure_frame(
    cx: &mut AsyncApp,
    bench: &Bench,
    timeout: Duration,
    check: Box<dyn Fn() -> bool>,
    start: impl FnOnce(&mut AsyncApp) -> Result<Instant, String>,
) -> Result<Duration, String> {
    {
        let mut probe = bench.probe.borrow_mut();
        probe.hit = None;
        probe.check = Some(check);
    }
    let t0 = match start(cx) {
        Ok(t0) => t0,
        Err(error) => {
            bench.probe.borrow_mut().check = None;
            return Err(error);
        }
    };
    let deadline = t0 + timeout;
    loop {
        if let Some(hit) = bench.probe.borrow().hit {
            return Ok(hit.saturating_duration_since(t0));
        }
        if Instant::now() > deadline {
            bench.probe.borrow_mut().check = None;
            return Err("timeout waiting for the frame".to_owned());
        }
        // A frame is drawn when the terminal changes; refresh now and then
        // in case the last change came before the check was armed.
        let _ = cx.update_window(bench.window, |_, window, _| window.refresh());
        sleep(cx, Duration::from_millis(50)).await;
    }
}

async fn drive(
    cx: &mut AsyncApp,
    bench: &Bench,
    mode: &BenchMode,
    target: SessionTarget,
    fingerprints: &[String],
    seq: &LocalSeq,
) -> Result<(), String> {
    cx.update_window(bench.window, |_, window, _| window.activate_window())
        .map_err(|_| "the benchmark window closed".to_owned())?;
    match mode {
        BenchMode::Sessions(_) => Err("--bench-sessions runs in the main window".to_owned()),
        BenchMode::Output { lines } => {
            output_runs(cx, bench, target, fingerprints, seq, *lines).await
        }
        BenchMode::Latency { keys } => {
            let mut local = open_terminal(cx, bench, OpenRequest::Local { shell_id: None }).await?;
            wait_connected(cx, &mut local, fingerprints).await?;
            let mut samples = latency(cx, bench, &local, *keys).await?;
            report_latency("local", &mut samples, cx, bench);
            local.session.close();

            let mut ssh = open_terminal(cx, bench, OpenRequest::Ssh(target)).await?;
            wait_connected(cx, &mut ssh, fingerprints).await?;
            let mut samples = latency(cx, bench, &ssh, *keys).await?;
            report_latency("ssh", &mut samples, cx, bench);
            ssh.session.close();
            Ok(())
        }
    }
}

fn active_note(cx: &mut AsyncApp, bench: &Bench) -> String {
    let active = cx
        .update_window(bench.window, |_, window, _| window.is_window_active())
        .unwrap_or(false);
    if active {
        String::new()
    } else {
        " (window not active)".to_owned()
    }
}

fn report_latency(name: &str, samples: &mut [Duration], cx: &mut AsyncApp, bench: &Bench) {
    let p50 = percentile(samples, 50.0).unwrap_or_default();
    let p95 = percentile(samples, 95.0).unwrap_or_default();
    say(&format!(
        "latency {name} p50={} ms p95={} ms (n={}{})",
        millis(p50),
        millis(p95),
        samples.len(),
        active_note(cx, bench).replace(" (", ", ").replace(')', "")
    ));
}

/// `--bench-sessions`: N session tabs in the main window, opened one after
/// the other as a Connect does (the first answers the first-connection
/// dialog with "Accept and save"); then every terminal waits at its prompt
/// and `bench: ready` is printed.
async fn session_tabs(
    cx: &mut AsyncApp,
    window: AnyWindowHandle,
    root: &Entity<RootView>,
    count: usize,
    host: uuid::Uuid,
    fingerprints: &[String],
) -> Result<(), String> {
    let started = Instant::now();
    let shell = cx
        .update(|cx| root.read(cx).shell().cloned())
        .ok_or("the main window shows no shell")?;
    // The app state learns about the saved host through the core's events.
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    while cx.update(|cx| root.read(cx).state().read(cx).host(host).is_none()) {
        if Instant::now() > deadline {
            return Err("the benchmark host did not appear".to_owned());
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
    let mut panes = Vec::with_capacity(count);
    for _ in 0..count {
        let id = cx.update(|cx| {
            shell.update(cx, |shell, cx| {
                shell.open_session(SessionRequest::new(SessionKind::Ssh).host(host), cx)
            })
        });
        // The pane's session starts at the shell's next frame.
        cx.update_window(window, |_, window, _| window.refresh())
            .map_err(|_| "the main window closed".to_owned())?;
        let pane = cx
            .update(|cx| shell.read(cx).pane(id).cloned())
            .ok_or("the tab has no session pane")?;
        wait_running(cx, &pane, fingerprints).await?;
        panes.push(pane);
    }
    for pane in &panes {
        wait_settled(cx, pane).await?;
    }
    say(&format!(
        "sessions connected {count} in {} ms",
        started.elapsed().as_millis()
    ));
    say("ready");
    Ok(())
}

/// Waits until the tab's session runs, answering the first-connection
/// dialog with "Accept and save" only for one of the test server's own
/// keys (`fingerprints`).
async fn wait_running(
    cx: &mut AsyncApp,
    pane: &Entity<SessionPane>,
    fingerprints: &[String],
) -> Result<(), String> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        let (phase, ask) = cx.update(|cx| {
            let pane = pane.read(cx);
            (pane.phase().clone(), pane.host_key().cloned())
        });
        match phase {
            Phase::Running => return Ok(()),
            Phase::Failed(failure) => {
                return Err(format!("the session did not connect ({})", failure.code()));
            }
            Phase::Closed | Phase::Ended(_) | Phase::Dropped(_) => {
                return Err("the session ended".to_owned());
            }
            Phase::Connecting(_) | Phase::Reconnecting(_) => {}
        }
        match ask {
            Some(HostKeyAsk::Unknown(key)) => {
                check_host_key(&key.fingerprint, fingerprints)?;
                cx.update(|cx| {
                    pane.update(cx, |pane, cx| {
                        pane.answer_host_key(HostKeyReply::AcceptAndSave, cx)
                    })
                });
            }
            Some(HostKeyAsk::Changed { .. }) => {
                return Err("the test server's host key changed".to_owned());
            }
            None => {}
        }
        if Instant::now() > deadline {
            return Err("timeout while connecting".to_owned());
        }
        sleep(cx, Duration::from_millis(10)).await;
    }
}

/// Whether `fingerprint` is one of the test server's own host keys; the
/// benchmark never trusts any other server.
fn check_host_key(fingerprint: &str, fingerprints: &[String]) -> Result<(), String> {
    if fingerprints.iter().any(|known| known == fingerprint) {
        Ok(())
    } else {
        Err("the server presented a host key that is not the test server's".to_owned())
    }
}

/// Waits until the tab's terminal shows something and stays unchanged for
/// [`SETTLE`] (the shell's prompt after its start-up output).
async fn wait_settled(cx: &mut AsyncApp, pane: &Entity<SessionPane>) -> Result<(), String> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut last: Vec<String> = Vec::new();
    let mut since = Instant::now();
    loop {
        let screen = cx.update(|cx| {
            pane.read(cx)
                .terminal()
                .map(|terminal| terminal.read(cx).handle().screen_text())
                .unwrap_or_default()
        });
        if screen != last {
            last = screen;
            since = Instant::now();
        } else if since.elapsed() >= SETTLE && last.iter().any(|row| !row.trim().is_empty()) {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("no shell prompt".to_owned());
        }
        sleep(cx, Duration::from_millis(20)).await;
    }
}

/// Closes every session tab of the main window (their sessions end).
fn close_tabs(cx: &mut AsyncApp, root: &Entity<RootView>) {
    cx.update(|cx| {
        if let Some(shell) = root.read(cx).shell().cloned() {
            shell.update(cx, |shell, cx| {
                let ids: Vec<_> = shell.sessions().tabs().iter().map(|tab| tab.id()).collect();
                for id in ids {
                    shell.close_session(id, cx);
                }
            });
        }
    });
}

/// `--bench-output`: the five runs of [`OutputKind`], in order, one output
/// line each.
async fn output_runs(
    cx: &mut AsyncApp,
    bench: &Bench,
    target: SessionTarget,
    fingerprints: &[String],
    seq: &LocalSeq,
    lines: u64,
) -> Result<(), String> {
    let numbers = format!("seq 1 {lines}");

    let mut ssh = open_terminal(cx, bench, OpenRequest::Ssh(target)).await?;
    wait_connected(cx, &mut ssh, fingerprints).await?;
    let size = cx.update(|cx| ssh.view.read(cx).handle().size());
    let grid = GridSize {
        cols: size.cols,
        rows: size.rows,
    };
    let finished = move |rows: &[String]| seq_finished(rows, lines);
    let elapsed = typed(cx, bench, &ssh, &numbers, finished).await?;
    report_output(
        OutputKind::Ssh,
        lines,
        "typed into the SSH session; our terminal",
        elapsed,
        &active_note(cx, bench),
    );
    ssh.session.close();

    if seq.direct_program().is_some() {
        let elapsed = direct(cx, bench, lines).await?;
        report_output(
            OutputKind::LocalDirect,
            lines,
            &format!("{}; our terminal", seq.direct_label()),
            elapsed,
            &active_note(cx, bench),
        );
    } else {
        say(&format!(
            "output {} skipped: {}",
            OutputKind::LocalDirect.token(),
            seq.direct_label()
        ));
    }

    let mut local = open_terminal(cx, bench, OpenRequest::Local { shell_id: None }).await?;
    wait_connected(cx, &mut local, fingerprints).await?;
    let finished = move |rows: &[String]| seq_shell_finished(rows, lines);
    let elapsed = typed(cx, bench, &local, &seq.command(lines), finished).await?;
    report_output(
        OutputKind::LocalShell,
        lines,
        &format!("{}; our terminal", seq.shell_label()),
        elapsed,
        &active_note(cx, bench),
    );
    local.session.close();
    // Nothing of our terminal runs during the baselines.
    cx.update(|cx| {
        bench.root.update(cx, |root, cx| {
            root.active = None;
            cx.notify();
        });
    });

    if let Some(program) = seq.direct_program() {
        let program = program.to_path_buf();
        let run =
            bare(move || bench_pty::run_direct(&program, lines, grid, OUTPUT_TIMEOUT)).await?;
        report_bare(OutputKind::BareDirect, lines, seq.direct_label(), run);
    } else {
        say(&format!(
            "output {} skipped: {}",
            OutputKind::BareDirect.token(),
            seq.direct_label()
        ));
    }
    let shell_seq = seq.clone();
    let run = bare(move || bench_pty::run_shell(&shell_seq, lines, grid, OUTPUT_TIMEOUT)).await?;
    report_bare(OutputKind::BareShell, lines, seq.shell_label(), run);
    Ok(())
}

/// `bench: output <kind> seq 1 <N> (<how>): <seconds> s<note>`.
fn report_output(kind: OutputKind, lines: u64, how: &str, elapsed: Duration, note: &str) {
    say(&format!(
        "output {} seq 1 {lines} ({how}): {:.2} s{note}",
        kind.token(),
        elapsed.as_secs_f64()
    ));
}

fn report_bare(kind: OutputKind, lines: u64, how: &str, run: BareRun) {
    let read = if run.bytes < 1024 * 1024 {
        format!("{:.0} KB", run.bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", run.bytes as f64 / (1024.0 * 1024.0))
    };
    report_output(
        kind,
        lines,
        &format!("{how}; bare PTY reader, no terminal, {read} read"),
        run.elapsed,
        "",
    );
}

/// Runs a blocking baseline on a thread of its own.
async fn bare(
    run: impl FnOnce() -> Result<BareRun, String> + Send + 'static,
) -> Result<BareRun, String> {
    let (sender, receiver) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("bench-bare".to_owned())
        .spawn(move || {
            let _ = sender.send_blocking(run());
        })
        .map_err(|error| format!("baseline thread: {error}"))?;
    receiver
        .recv()
        .await
        .map_err(|_| "the baseline thread ended early".to_owned())?
}

/// Types `command` at the prompt and times it from Enter to the first frame
/// whose screen passes `finished`.
async fn typed(
    cx: &mut AsyncApp,
    bench: &Bench,
    terminal: &BenchTerminal,
    command: &str,
    finished: impl Fn(&[String]) -> bool + 'static,
) -> Result<Duration, String> {
    wait_prompt(cx, terminal).await?;
    write(&terminal.session, command.as_bytes())?;
    sleep(cx, Duration::from_millis(300)).await;
    let handle = cx.update(|cx| terminal.view.read(cx).handle());
    let check = Box::new(move || finished(&handle.screen_text()));
    let session = terminal.session.clone();
    measure_frame(cx, bench, OUTPUT_TIMEOUT, check, move |_| {
        let t0 = Instant::now();
        write(&session, b"\r")?;
        Ok(t0)
    })
    .await
}

/// `local-direct`: a local terminal whose program is `seq 1 <lines>`; timed
/// from the session's start to the first frame that shows the last number.
async fn direct(cx: &mut AsyncApp, bench: &Bench, lines: u64) -> Result<Duration, String> {
    let view = add_view(cx, bench).await?;
    let handle = cx.update(|cx| view.read(cx).handle());
    let check = Box::new(move || seq_direct_finished(&handle.screen_text(), lines));
    let mut opened = None;
    let elapsed = measure_frame(cx, bench, OUTPUT_TIMEOUT, check, |cx| {
        let t0 = Instant::now();
        let request = OpenRequest::Local {
            shell_id: Some(DIRECT_SHELL_ID.to_owned()),
        };
        opened = Some(open_session(cx, view, request)?);
        Ok(t0)
    })
    .await;
    if let Some(terminal) = opened {
        // `seq` ended with 0 (the session closed itself) or is still running.
        terminal.session.close();
    }
    elapsed
}

fn write(session: &SessionHandle, bytes: &[u8]) -> Result<(), String> {
    session.write(bytes).map_err(|error| error.to_string())
}

/// `keys` keystrokes through the window (the platform input path of a
/// typed character), each timed from the key event to the frame that shows
/// it; after every 40 characters, Backspaces clear the line (not timed, no
/// command runs, nothing reaches the shell's history).
async fn latency(
    cx: &mut AsyncApp,
    bench: &Bench,
    terminal: &BenchTerminal,
    keys: usize,
) -> Result<Vec<Duration>, String> {
    let prompt = wait_prompt(cx, terminal).await?;
    let prompt = prompt.trim_end().to_owned();
    let handle = cx.update(|cx| terminal.view.read(cx).handle());
    cx.update_window(bench.window, |_, window, cx| {
        terminal.view.update(cx, |view, cx| view.focus(window, cx));
    })
    .map_err(|_| "the benchmark window closed".to_owned())?;
    let mut typed = String::new();
    let mut samples = Vec::with_capacity(keys);
    for index in 0..keys {
        if latency_line_start(index) && !typed.is_empty() {
            for _ in 0..typed.chars().count() {
                press(cx, bench, "backspace")?;
                sleep(cx, Duration::from_millis(5)).await;
            }
            typed.clear();
            wait_line(cx, &handle, &prompt, "").await?;
        }
        let character = latency_char(index);
        typed.push(character);
        let expected = typed.clone();
        let check_handle = handle.clone();
        let check_prompt = prompt.clone();
        let check =
            Box::new(move || line_shows(&check_handle.screen_text(), &check_prompt, &expected));
        let key = character.to_string();
        let window = bench.window;
        let elapsed = measure_frame(cx, bench, KEY_TIMEOUT, check, move |cx| {
            let t0 = Instant::now();
            cx.update_window(window, |_, window, cx| {
                window.dispatch_keystroke(
                    Keystroke::parse(&key).map_err(|_| "bad key".to_owned())?,
                    cx,
                );
                Ok::<(), String>(())
            })
            .map_err(|_| "the benchmark window closed".to_owned())??;
            Ok(t0)
        })
        .await?;
        samples.push(elapsed);
        sleep(cx, KEY_GAP).await;
    }
    Ok(samples)
}

fn press(cx: &mut AsyncApp, bench: &Bench, key: &str) -> Result<(), String> {
    let keystroke = Keystroke::parse(key).map_err(|_| "bad key".to_owned())?;
    cx.update_window(bench.window, |_, window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    })
    .map_err(|_| "the benchmark window closed".to_owned())
}

/// Whether the last row that starts with the prompt shows `typed` right
/// after it (anything after `typed`, such as a suggestion, is ignored).
fn line_shows(rows: &[String], prompt: &str, typed: &str) -> bool {
    prompt_rest(rows, prompt).is_some_and(|rest| rest.trim_start().starts_with(typed))
}

/// The text after the prompt on the last row that starts with it.
fn prompt_rest<'a>(rows: &'a [String], prompt: &str) -> Option<&'a str> {
    rows.iter().rev().find_map(|row| row.strip_prefix(prompt))
}

async fn wait_line(
    cx: &mut AsyncApp,
    handle: &crate::terminal::TerminalHandle,
    prompt: &str,
    typed: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + KEY_TIMEOUT;
    loop {
        let rows = handle.screen_text();
        let shown = prompt_rest(&rows, prompt).is_some_and(|rest| rest.trim() == typed);
        if shown {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err("the line was not cleared".to_owned());
        }
        sleep(cx, Duration::from_millis(10)).await;
    }
}
