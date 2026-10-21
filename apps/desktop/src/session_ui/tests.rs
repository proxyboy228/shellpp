//! Headless tests of session tabs (UI 4.1, 4.3, 4.6, 4.8, 10.2) with the
//! fake session layer ([`super::fake`]): a temporary profile, no network,
//! no keys, no real agent. Elements are found by element id (UI 0).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, Bounds, Entity, SharedString, TestAppContext, WindowBounds, WindowOptions, px,
    size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::json;
use shellpp_core::service::dto::SessionTarget;

use super::fake::{Call, FakeLauncher, Record};
use super::*;
use crate::app::{self, RootView};
use crate::shell::{SessionId, SessionKind, SessionRequest, Shell, TopTab};
use crate::state::AppState;
use crate::terminal::{TerminalSink, TerminalSize};
use crate::testing::TestProfile;

pub(crate) struct Ui {
    pub state: Entity<AppState>,
    pub handle: AnyWindowHandle,
    pub root: Entity<RootView>,
    pub record: Record,
    /// The generated master password of the temporary profile.
    pub password: secrecy::SecretString,
}

impl Ui {
    pub fn open(cx: &mut TestAppContext) -> Self {
        Self::open_with(None, cx)
    }

    pub fn open_with(fail: Option<Failure>, cx: &mut TestAppContext) -> Self {
        Self::start(Some(fail), cx)
    }

    /// The window over the real session layer (`crate::sessions`, its own
    /// Tokio runtime) with no key files and no agent: tabs connect through
    /// [`bridge::Bridge`].
    pub fn open_real(cx: &mut TestAppContext) -> Self {
        Self::start(None, cx)
    }

    /// `fake`: the fake session layer (failing every launch with the given
    /// reason, if any); `None`: the real one.
    fn start(fake: Option<Option<Failure>>, cx: &mut TestAppContext) -> Self {
        // Every terminal parses on its own thread and wakes its view from
        // there; the real session layer reports from its runtime's threads.
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            app::root::bind_keys(cx);
            crate::terminal::init(cx);
        });
        let record = match fake {
            Some(fail) => FakeLauncher::install_with(fail, cx),
            None => Record::default(),
        };
        let profile = TestProfile::new();
        let password = profile.password().clone();
        let state = cx.new(|cx| AppState::new(profile.into_unlocked(), cx));
        if fake.is_none() {
            let installed = cx.update(|cx| {
                crate::sessions::ui::install(
                    &state,
                    crate::sessions::ManagerOptions::with_keys(crate::sessions::KeySources::none()),
                    cx,
                )
            });
            assert!(installed, "the session layer starts");
        }
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1280.), px(800.)),
            ))),
            ..Default::default()
        };
        let (handle, root) = cx
            .update(|cx| app::open_window_with(state.clone(), options, cx))
            .expect("test window");
        let ui = Self {
            state,
            handle,
            root,
            record,
            password,
        };
        ui.frame(cx);
        ui
    }

    pub fn shell(&self, cx: &mut TestAppContext) -> Entity<Shell> {
        cx.update(|cx| self.root.read(cx).shell().cloned().expect("shell"))
    }

    pub fn frame(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    pub fn click(&self, id: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.frame(cx);
    }

    pub fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.frame(cx);
    }

    pub fn input(&self, text: &str, cx: &mut TestAppContext) {
        let text = text.to_owned();
        cx.update_window(self.handle, move |_, window, cx| window.input(&text, cx))
            .unwrap();
        self.frame(cx);
    }

    pub fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id.clone())
                .is_some_and(|element| element.visible())
        })
        .unwrap()
    }

    pub fn focused(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id.clone())
                .is_some_and(|element| element.focused() == Some(true))
        })
        .unwrap()
    }

    /// Opens a session tab through the shell and lets it start.
    pub fn open_session(
        &self,
        request: SessionRequest,
        cx: &mut TestAppContext,
    ) -> (SessionId, Entity<SessionPane>) {
        let shell = self.shell(cx);
        let id = shell.update(cx, |shell, cx| shell.open_session(request, cx));
        self.frame(cx);
        let pane = cx.update(|cx| shell.read(cx).pane(id).cloned().expect("pane"));
        assert!(cx.update(|cx| pane.read(cx).is_started()), "started");
        (id, pane)
    }

    /// A saved host with a username, in the personal vault.
    pub fn save_host(&self, cx: &mut TestAppContext) -> uuid::Uuid {
        self.state.update(cx, |state, cx| {
            state
                .save_host(
                    None,
                    json!({"label": "web", "address": "192.0.2.10", "ssh": {"username": "tester", "port": 2222}})
                        .as_object()
                        .cloned()
                        .unwrap(),
                    cx,
                )
                .unwrap()
                .record_id
        })
    }

    pub fn top_tab(&self, cx: &mut TestAppContext) -> TopTab {
        let shell = self.shell(cx);
        cx.update(|cx| shell.read(cx).top_tab())
    }

    /// Waits (in real time, rendering frames) until `check` holds.
    pub fn wait_until(
        &self,
        what: &str,
        cx: &mut TestAppContext,
        mut check: impl FnMut(&mut TestAppContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.frame(cx);
            if check(cx) {
                return;
            }
            assert!(Instant::now() < deadline, "timeout waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Quits like the app: the sessions end and let go of the core, then
    /// the core closes.
    pub fn finish(self, cx: &mut TestAppContext) {
        cx.update(crate::shell::release_all_sessions);
        self.state.update(cx, |state, _| state.shutdown());
    }
}

fn set_phase(
    ui: &Ui,
    pane: &Entity<SessionPane>,
    phase: Phase,
    reconnect: Option<ReconnectInfo>,
    cx: &mut TestAppContext,
) {
    pane.update(cx, |pane, cx| pane.set_phase(phase, reconnect, cx));
    ui.frame(cx);
}

/// A generated value (tests never use fixed secrets).
fn generated() -> String {
    format!("v-{}", uuid::Uuid::new_v4().simple())
}

/// UI 4.8: the connection screen shows the host, the address, the steps
/// with the current one, and Cancel, which stops the attempt; the failure
/// state shows the reason with Retry and Close.
#[gpui_kit::test]
fn connection_screen_steps_cancel_and_failure(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(cx);
    let (id, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let launched = ui.record.calls();
    match &launched[0] {
        Call::Launch(OpenRequest::Ssh(SessionTarget::Host { record_id, .. })) => {
            assert_eq!(*record_id, host)
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(ui.visible("connecting-screen", cx));
    assert!(ui.visible("connecting-title", cx));
    assert!(ui.visible("connecting-address", cx));
    let address = cx.update(|cx| pane.read(cx).info().address().cloned());
    assert_eq!(address.as_deref(), Some("tester@192.0.2.10:2222"));
    for step in ConnectStep::ALL {
        assert!(ui.visible(&format!("connecting-step-{}", step.slug()), cx));
    }
    set_phase(
        &ui,
        &pane,
        Phase::Connecting(ConnectStep::Authenticating),
        None,
        cx,
    );
    assert!(ui.visible("connecting-screen", cx));

    ui.click("connecting-cancel", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Cancel));
    set_phase(&ui, &pane, Phase::Failed(Failure::Cancelled), None, cx);
    assert!(ui.visible("failed-screen", cx));
    assert!(ui.visible("failure-cancelled", cx));
    ui.click("failed-retry", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Reconnect));

    // Network failures read as such (no panics, a clear text).
    for failure in [
        Failure::NotFound,
        Failure::Refused,
        Failure::Timeout,
        Failure::AuthFailed,
    ] {
        set_phase(&ui, &pane, Phase::Failed(failure), None, cx);
        assert!(ui.visible(&format!("failure-{}", failure.code()), cx));
        assert!(ui.visible("failed-retry", cx));
    }

    // Close ends the session and closes the tab (Vaults: no neighbor).
    ui.click("failed-close", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Close));
    let shell = ui.shell(cx);
    assert!(cx.update(|cx| shell.read(cx).sessions().get(id).is_none()));
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.finish(cx);
}

/// UI 10.2: a revoked key blocks the connection with the danger warning
/// and no way to continue (no Retry).
#[gpui_kit::test]
fn revoked_key_blocks_without_retry(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    pane.update(cx, |pane, cx| {
        pane.set_phase(Phase::Failed(Failure::HostKeyRevoked), None, cx);
        pane.set_revoked_key(
            Some(HostKeyInfo {
                address: "[192.0.2.10]:2222".into(),
                key_type: "ssh-ed25519".into(),
                fingerprint: "SHA256:revoked".into(),
            }),
            cx,
        );
    });
    ui.frame(cx);
    assert!(ui.visible("failed-screen", cx));
    assert!(ui.visible("host-key-revoked-warning", cx));
    assert!(ui.visible("host-key-fingerprint", cx));
    assert!(ui.visible("failed-close", cx));
    assert!(!ui.visible("failed-retry", cx));
    ui.finish(cx);
}

/// A session layer that cannot start the session at all shows the failure
/// screen at once.
#[gpui_kit::test]
fn a_launch_error_is_shown_in_the_tab(cx: &mut TestAppContext) {
    let ui = Ui::open_with(Some(Failure::HostMissing), cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    assert_eq!(
        cx.update(|cx| pane.read(cx).phase().clone()),
        Phase::Failed(Failure::HostMissing)
    );
    assert!(ui.visible("failure-host_missing", cx));
    // It fails the same way every time: only Close.
    assert!(ui.visible("failed-close", cx));
    assert!(!ui.visible("failed-retry", cx));
    ui.finish(cx);
}

/// A session that could not start (the app was locked at the launch) is
/// launched again by Retry, in the same tab with the same terminal.
#[gpui_kit::test]
fn retry_launches_a_session_that_never_started(cx: &mut TestAppContext) {
    let ui = Ui::open_with(Some(Failure::Locked), cx);
    let host = ui.save_host(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    assert_eq!(
        cx.update(|cx| pane.read(cx).phase().clone()),
        Phase::Failed(Failure::Locked)
    );
    assert!(ui.visible("failed-retry", cx));
    let terminal = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            pane.read(cx)
                .terminal()
                .map(|terminal| terminal.entity_id())
        })
    };
    let first_terminal = terminal(cx);
    assert!(first_terminal.is_some());

    // Still failing: the failure screen again, with a second launch.
    ui.click("failed-retry", cx);
    let launches = |ui: &Ui| {
        ui.record
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Launch(_)))
            .count()
    };
    assert_eq!(launches(&ui), 2);
    assert!(ui.visible("failure-locked", cx));

    ui.record.set_fail(None);
    ui.click("failed-retry", cx);
    assert_eq!(launches(&ui), 3);
    assert_eq!(
        cx.update(|cx| pane.read(cx).phase().clone()),
        Phase::Connecting(ConnectStep::Resolving)
    );
    assert!(ui.visible("connecting-screen", cx));
    assert_eq!(
        terminal(cx),
        first_terminal,
        "one terminal for every launch"
    );
    // Now the session answers: Cancel reaches it.
    ui.click("connecting-cancel", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Cancel));
    ui.finish(cx);
}

/// UI 2.1: the offer to save the host comes after a successful quick
/// connect: not while connecting, not after a failure, once per tab.
#[gpui_kit::test]
fn a_quick_connect_offers_to_save_the_host_once_connected(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let target = crate::hosts::QuickTarget {
        username: Some("root".into()),
        host: "192.0.2.77".into(),
        port: Some(2222),
    };
    let quick = || SessionRequest::new(SessionKind::Ssh).quick(target.clone());

    // A mistyped or unreachable address: no offer to save it.
    let (_, failed) = ui.open_session(quick(), cx);
    assert!(!ui.visible("toast-save-host", cx));
    set_phase(&ui, &failed, Phase::Failed(Failure::NotFound), None, cx);
    assert!(!ui.visible("toast-save-host", cx));

    let (_, pane) = ui.open_session(quick(), cx);
    set_phase(
        &ui,
        &pane,
        Phase::Connecting(ConnectStep::Authenticating),
        None,
        cx,
    );
    assert!(!ui.visible("toast-save-host", cx));
    set_phase(&ui, &pane, Phase::Running, None, cx);
    assert!(ui.visible("toast-save-host", cx));

    // "Save": the host with the user and the port, its panel in Vaults.
    ui.click("toast-save-host", cx);
    let hosts = cx.update(|cx| ui.state.read(cx).hosts());
    assert!(hosts.iter().any(|host| host.address == "192.0.2.77"
        && host.port == Some(2222)
        && host.username.as_deref() == Some("root")));
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);

    // A reconnect of the same tab does not offer it again.
    set_phase(
        &ui,
        &pane,
        Phase::Dropped(Failure::ConnectionLost),
        None,
        cx,
    );
    set_phase(&ui, &pane, Phase::Running, None, cx);
    assert!(!ui.visible("toast-save-host", cx));
    ui.finish(cx);
}

/// The shell opens at the size the tab has, not 80x24: the terminal is
/// sized from the pane before the launch, and drawing it later keeps that
/// size.
#[gpui_kit::test]
fn the_shell_opens_at_the_size_of_the_tab(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let launched = ui.record.launch_sizes()[0];
    assert!(
        launched.cols > 100 && launched.rows > 30,
        "a 1280x800 window: {}x{}",
        launched.cols,
        launched.rows
    );
    set_phase(&ui, &pane, Phase::Running, None, cx);
    let drawn = ui.record.terminals()[0].size();
    assert_eq!((drawn.cols, drawn.rows), (launched.cols, launched.rows));
    ui.finish(cx);
}

/// UI 4.8: the password form of a saved host has "Save" (off by default);
/// Enter sends the value with the checkbox; a refused password shows the
/// error in the same form; Esc cancels.
#[gpui_kit::test]
fn password_prompt_with_save_error_and_cancel(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let ask = |error: bool| Prompt::Password {
        username: "tester".into(),
        error,
        keep: Keep::Save,
    };
    pane.update(cx, |pane, cx| pane.ask_prompt(ask(false), cx));
    ui.frame(cx);
    assert!(ui.visible("session-prompt", cx));
    assert!(ui.visible("session-prompt-keep", cx));
    assert!(!ui.visible("session-prompt-error", cx));
    assert!(
        ui.focused("session-prompt-input", cx),
        "the field has the focus"
    );

    let first = generated();
    ui.input(&first, cx);
    ui.press("enter", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Answer(first, false)));
    assert!(!ui.visible("session-prompt", cx));

    // Refused: the same form with the error; this time with "Save".
    pane.update(cx, |pane, cx| pane.ask_prompt(ask(true), cx));
    ui.frame(cx);
    assert!(ui.visible("session-prompt-error", cx));
    ui.click("session-prompt-keep", cx);
    ui.click("session-prompt-input", cx);
    let second = generated();
    ui.input(&second, cx);
    ui.click("session-prompt-continue", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Answer(second, true)));

    // Esc cancels the attempt.
    pane.update(cx, |pane, cx| pane.ask_prompt(ask(false), cx));
    ui.frame(cx);
    ui.press("escape", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::CancelPrompt));
    assert!(!ui.visible("session-prompt", cx));
    ui.finish(cx);
}

/// UI 4.8: a quick connect has no "Save"; the username prompt needs a
/// value; a key file's passphrase offers "Remember" for this app session
/// only (D30).
#[gpui_kit::test]
fn quick_connect_username_and_passphrase_prompts(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let target = crate::hosts::parse_address("192.0.2.20").unwrap();
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).quick(target), cx);
    match &ui.record.calls()[0] {
        Call::Launch(OpenRequest::Ssh(SessionTarget::Quick {
            address,
            port,
            username,
        })) => {
            assert_eq!(address, "192.0.2.20");
            assert_eq!(*port, None);
            assert_eq!(*username, None);
        }
        other => panic!("unexpected {other:?}"),
    }
    pane.update(cx, |pane, cx| pane.ask_prompt(Prompt::Username, cx));
    ui.frame(cx);
    assert!(!ui.visible("session-prompt-keep", cx));
    // Empty: Continue does nothing.
    ui.press("enter", cx);
    assert!(ui.visible("session-prompt", cx));
    ui.input("tester", cx);
    ui.press("enter", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::Answer("tester".into(), false))
    );

    pane.update(cx, |pane, cx| {
        pane.ask_prompt(
            Prompt::Password {
                username: "tester".into(),
                error: false,
                keep: Keep::None,
            },
            cx,
        )
    });
    ui.frame(cx);
    assert!(ui.visible("session-prompt", cx));
    assert!(
        !ui.visible("session-prompt-keep", cx),
        "no Save for a quick connect"
    );

    pane.update(cx, |pane, cx| {
        pane.ask_prompt(
            Prompt::Passphrase {
                key_label: "id_ed25519".into(),
                error: false,
                keep: Keep::ThisSession,
            },
            cx,
        )
    });
    ui.frame(cx);
    assert!(ui.visible("session-prompt-keep", cx));
    ui.click("session-prompt-keep", cx);
    ui.click("session-prompt-input", cx);
    let passphrase = generated();
    ui.input(&passphrase, cx);
    ui.press("enter", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::Answer(passphrase, true))
    );
    ui.finish(cx);
}

/// UI 4.8: keyboard-interactive questions come one at a time with the
/// server's name and instruction; the answer is masked when the server
/// marks the question as not echoed.
#[gpui_kit::test]
fn keyboard_interactive_one_question_at_a_time(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    let question = |index: u32, echo: bool| Prompt::KeyboardInteractive {
        name: "Two-factor".into(),
        instruction: "Enter the code from the app".into(),
        prompt: if index == 0 {
            "Code:".into()
        } else {
            "Again:".into()
        },
        echo,
        index,
        count: 2,
        error: false,
    };
    pane.update(cx, |pane, cx| pane.ask_prompt(question(0, false), cx));
    ui.frame(cx);
    assert!(ui.visible("session-prompt-progress", cx));
    let masked = cx.update(|cx| {
        pane.read(cx)
            .prompt_form()
            .map(|form| form.read(cx).prompt().is_masked())
    });
    assert_eq!(masked, Some(true));
    ui.input("123456", cx);
    ui.press("enter", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::Answer("123456".into(), false))
    );
    pane.update(cx, |pane, cx| pane.ask_prompt(question(1, true), cx));
    ui.frame(cx);
    let masked = cx.update(|cx| {
        pane.read(cx)
            .prompt_form()
            .map(|form| form.read(cx).prompt().is_masked())
    });
    assert_eq!(masked, Some(false), "echoed answers are shown");
    ui.finish(cx);
}

/// docs/security.md 11.2: a server cannot dress the keyboard-interactive
/// dialog up as one of the app's own (here, the lock screen). The title,
/// the host line and the field label stay the app's; the server's name,
/// instruction and question appear only in the framed block.
#[gpui_kit::test]
fn keyboard_interactive_server_text_stays_in_its_frame(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    pane.update(cx, |pane, cx| {
        pane.ask_prompt(
            Prompt::KeyboardInteractive {
                name: "Shell++ is locked".into(),
                instruction: "Enter your master password to continue".into(),
                prompt: "Master password".into(),
                echo: false,
                index: 0,
                count: 1,
                error: false,
            },
            cx,
        )
    });
    ui.frame(cx);
    assert!(ui.visible("session-prompt-subtitle", cx));
    assert!(ui.visible("session-prompt-server", cx));
    let (title, subtitle, label, lines, host) = cx.update(|cx| {
        let pane = pane.read(cx);
        let form = pane.prompt_form().expect("prompt form").read(cx);
        (
            form.title(),
            form.subtitle(),
            form.label(),
            form.server_lines(),
            pane.info().host_text(),
        )
    });
    assert_eq!(title.as_ref(), strings::KI_TITLE);
    assert_eq!(subtitle, Some(host));
    assert_eq!(label.as_ref(), strings::KI_ANSWER);
    assert_eq!(
        lines,
        [
            "Shell++ is locked",
            "Enter your master password to continue",
            "Master password",
        ]
    );

    // The app's own prompts have no server block.
    pane.update(cx, |pane, cx| pane.ask_prompt(Prompt::Username, cx));
    ui.frame(cx);
    assert!(ui.visible("session-prompt", cx));
    assert!(!ui.visible("session-prompt-server", cx));
    ui.finish(cx);
}

fn key(fingerprint: &str) -> HostKeyInfo {
    HostKeyInfo {
        address: "[192.0.2.10]:2222".into(),
        key_type: "ssh-ed25519".into(),
        fingerprint: fingerprint.into(),
    }
}

/// UI 10.2: the first connection shows address, key type and fingerprint;
/// Enter is "Accept and save"; "Connect once" and "Cancel" answer too.
#[gpui_kit::test]
fn first_connection_dialog(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    let ask = HostKeyAsk::Unknown(key("SHA256:new"));
    pane.update(cx, |pane, cx| pane.ask_host_key(ask.clone(), cx));
    ui.frame(cx);
    for id in [
        "host-key-unknown",
        "host-key-address",
        "host-key-type",
        "host-key-fingerprint",
        "host-key-accept",
        "host-key-once",
        "host-key-cancel",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    ui.press("enter", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::HostKey(HostKeyReply::AcceptAndSave))
    );
    assert!(!ui.visible("host-key-unknown", cx));

    pane.update(cx, |pane, cx| pane.ask_host_key(ask.clone(), cx));
    ui.frame(cx);
    ui.click("host-key-once", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::HostKey(HostKeyReply::ConnectOnce))
    );
    pane.update(cx, |pane, cx| pane.ask_host_key(ask, cx));
    ui.frame(cx);
    ui.press("escape", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::HostKey(HostKeyReply::Cancel))
    );
    ui.finish(cx);
}

/// UI 10.2: a changed key blocks with the danger warning and both
/// fingerprints; Enter cancels; only "Replace saved key" continues; a
/// first-connection answer never replaces.
#[gpui_kit::test]
fn changed_key_blocks_until_replace(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    let ask = HostKeyAsk::Changed {
        key: key("SHA256:presented"),
        old_key_type: "ssh-ed25519".into(),
        old_fingerprint: "SHA256:saved".into(),
    };
    pane.update(cx, |pane, cx| pane.ask_host_key(ask.clone(), cx));
    ui.frame(cx);
    for id in [
        "host-key-changed",
        "host-key-changed-warning",
        "host-key-old",
        "host-key-new",
        "host-key-replace",
        "host-key-cancel",
    ] {
        assert!(ui.visible(id, cx), "{id}");
    }
    assert!(!ui.visible("host-key-accept", cx));
    assert!(!ui.visible("host-key-once", cx));
    // "Accept and save" is not an answer to a changed key.
    pane.update(cx, |pane, cx| {
        pane.answer_host_key(HostKeyReply::AcceptAndSave, cx)
    });
    assert!(cx.update(|cx| pane.read(cx).host_key().is_some()));
    ui.press("enter", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::HostKey(HostKeyReply::Cancel))
    );
    pane.update(cx, |pane, cx| pane.ask_host_key(ask, cx));
    ui.frame(cx);
    ui.click("host-key-replace", cx);
    assert_eq!(
        ui.record.calls().last(),
        Some(&Call::HostKey(HostKeyReply::Replace))
    );
    ui.finish(cx);
}

/// UI 10.2, owner step 0.3: the changed-key dialog opens with the keyboard
/// focus on Cancel; Enter and Esc press Cancel; Enter on a focused "Replace
/// saved key" does nothing; Tab then Space, or a click, replaces the key.
#[gpui_kit::test]
fn changed_key_keyboard_starts_on_cancel(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    let ask = HostKeyAsk::Changed {
        key: key("SHA256:presented"),
        old_key_type: "ssh-ed25519".into(),
        old_fingerprint: "SHA256:saved".into(),
    };
    let show = |cx: &mut TestAppContext| {
        pane.update(cx, |pane, cx| pane.ask_host_key(ask.clone(), cx));
        ui.frame(cx);
        assert!(ui.visible("host-key-changed", cx));
    };
    let answers = |ui: &Ui| {
        ui.record
            .calls()
            .into_iter()
            .filter(|call| matches!(call, Call::HostKey(_)))
            .collect::<Vec<_>>()
    };

    // Enter on the default focus: Cancel.
    show(cx);
    assert!(
        ui.focused("host-key-cancel", cx),
        "the focus starts on Cancel"
    );
    ui.press("enter", cx);
    assert_eq!(answers(&ui), [Call::HostKey(HostKeyReply::Cancel)]);
    assert!(!ui.visible("host-key-changed", cx));

    // Esc: Cancel.
    show(cx);
    ui.press("escape", cx);
    assert_eq!(
        answers(&ui).last(),
        Some(&Call::HostKey(HostKeyReply::Cancel))
    );
    assert_eq!(answers(&ui).len(), 2);

    // Tab moves to "Replace saved key"; Enter there does nothing.
    show(cx);
    ui.press("tab", cx);
    assert!(ui.focused("host-key-replace", cx), "Tab reaches Replace");
    ui.press("enter", cx);
    assert_eq!(answers(&ui).len(), 2, "Enter on Replace answers nothing");
    assert!(ui.visible("host-key-changed", cx), "the dialog stays");
    assert!(cx.update(|cx| pane.read(cx).host_key().is_some()));
    // Space on it replaces.
    ui.press("space", cx);
    assert_eq!(
        answers(&ui).last(),
        Some(&Call::HostKey(HostKeyReply::Replace))
    );
    assert_eq!(answers(&ui).len(), 3);

    // A click replaces.
    show(cx);
    ui.click("host-key-replace", cx);
    assert_eq!(
        answers(&ui).last(),
        Some(&Call::HostKey(HostKeyReply::Replace))
    );
    assert_eq!(answers(&ui).len(), 4);
    ui.finish(cx);
}

/// UI 4.6: a non-zero exit keeps the tab and the output with "Session
/// ended with code N", Reconnect and Close; it never reconnects by itself.
#[gpui_kit::test]
fn non_zero_exit_keeps_the_tab(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (id, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    set_phase(&ui, &pane, Phase::Running, None, cx);
    assert!(ui.visible("session-terminal", cx));
    set_phase(&ui, &pane, Phase::Ended(ExitStatus::Code(2)), None, cx);
    assert!(ui.visible("session-ended", cx));
    assert!(ui.visible("session-terminal", cx), "the output stays");
    let shell = ui.shell(cx);
    assert!(cx.update(|cx| shell.read(cx).sessions().get(id).is_some()));
    assert!(!ui.record.calls().contains(&Call::Reconnect));
    ui.click("session-ended-reconnect", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Reconnect));
    set_phase(
        &ui,
        &pane,
        Phase::Reconnecting(ConnectStep::Connecting),
        None,
        cx,
    );
    assert!(ui.visible("session-reconnecting", cx));
    assert!(ui.visible("session-reconnecting-step", cx));
    ui.click("session-reconnecting-cancel", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Cancel));
    ui.finish(cx);
}

/// UI 4.6: a drop keeps the output with the reason; with automatic
/// attempts scheduled the countdown and Cancel (which stops them) show;
/// while locked the attempt waits for unlock; without attempts only
/// Reconnect and Close.
#[gpui_kit::test]
fn drop_shows_reason_countdown_and_cancel(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    set_phase(&ui, &pane, Phase::Running, None, cx);
    let scheduled = ReconnectInfo {
        attempt: 1,
        next_at: Some(Instant::now() + Duration::from_secs(30)),
        waiting_for_unlock: false,
    };
    set_phase(
        &ui,
        &pane,
        Phase::Dropped(Failure::KeepaliveTimeout),
        Some(scheduled),
        cx,
    );
    assert!(ui.visible("session-dropped", cx));
    assert!(ui.visible("session-dropped-reason-keepalive_timeout", cx));
    assert!(ui.visible("session-dropped-countdown", cx));
    let left = cx.update(|cx| pane.read(cx).reconnect_countdown());
    assert!(
        left.is_some_and(|left| (28..=30).contains(&left)),
        "{left:?}"
    );
    assert!(ui.visible("session-dropped-cancel", cx));
    assert!(ui.visible("session-terminal", cx), "the output stays");
    ui.click("session-dropped-cancel", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Cancel));

    // Waiting for unlock.
    set_phase(
        &ui,
        &pane,
        Phase::Dropped(Failure::ConnectionLost),
        Some(ReconnectInfo {
            attempt: 2,
            next_at: None,
            waiting_for_unlock: true,
        }),
        cx,
    );
    assert!(ui.visible("session-dropped-countdown", cx));
    assert!(ui.visible("session-dropped-cancel", cx));

    // Stopped: Reconnect and Close only.
    set_phase(
        &ui,
        &pane,
        Phase::Dropped(Failure::ConnectionLost),
        None,
        cx,
    );
    assert!(!ui.visible("session-dropped-countdown", cx));
    assert!(!ui.visible("session-dropped-cancel", cx));
    ui.click("session-dropped-reconnect", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Reconnect));
    ui.click("session-dropped-close", cx);
    assert_eq!(ui.record.calls().last(), Some(&Call::Close));
    ui.finish(cx);
}

/// UI 4.6: exit status 0 closes the tab by itself; the left neighbor (else
/// the right one, else Vaults) becomes active.
#[gpui_kit::test]
fn exit_zero_closes_the_tab_with_the_neighbor_rule(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (first, _) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    let (second, second_pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    let (third, third_pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    // The middle tab, active: its left neighbor.
    let shell = ui.shell(cx);
    shell.update(cx, |shell, cx| {
        shell.set_top_tab(TopTab::Session(second), cx)
    });
    set_phase(&ui, &second_pane, Phase::Closed, None, cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(first));
    assert!(cx.update(|cx| shell.read(cx).sessions().get(second).is_none()));
    assert!(ui.record.calls().contains(&Call::Close));
    // A background tab closes without changing the active one.
    set_phase(&ui, &third_pane, Phase::Closed, None, cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(first));
    assert!(cx.update(|cx| shell.read(cx).sessions().get(third).is_none()));
    ui.finish(cx);
}

/// A sink that keeps what the terminal sends.
#[derive(Default)]
struct Captured(Mutex<Vec<u8>>);

impl TerminalSink for Captured {
    fn send(&self, bytes: Vec<u8>) {
        self.0.lock().unwrap().extend(bytes);
    }

    fn resize(&self, _: TerminalSize) {}
}

/// UI 4.3: a paste with line breaks asks first (line count and preview);
/// Cancel pastes nothing; Paste sends it, wrapped in bracketed paste
/// markers when the program enabled mode 2004, with markers inside the
/// text removed.
#[gpui_kit::test]
fn multi_line_paste_asks_first(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    set_phase(&ui, &pane, Phase::Running, None, cx);
    let captured = Arc::new(Captured::default());
    let handle = ui.record.terminals()[0].clone();
    handle.set_sink(Some(captured.clone()));
    let terminal = cx.update(|cx| pane.read(cx).terminal().cloned().unwrap());

    // One line: no question.
    terminal.update(cx, |terminal, cx| terminal.paste_text("ls -la", cx));
    ui.frame(cx);
    assert!(!ui.visible("paste-dialog", cx));
    assert_eq!(captured.0.lock().unwrap().as_slice(), b"ls -la");
    captured.0.lock().unwrap().clear();

    // Several lines: the dialog; Cancel pastes nothing.
    let text = "echo 1\necho 2\necho 3\necho 4\necho 5\necho 6\necho 7\n";
    terminal.update(cx, |terminal, cx| terminal.paste_text(text, cx));
    ui.frame(cx);
    assert!(ui.visible("paste-dialog", cx));
    assert!(ui.visible("paste-line-count", cx));
    assert!(ui.visible("paste-preview", cx));
    assert!(ui.visible("paste-more", cx), "7 lines, 5 in the preview");
    let lines = cx.update(|cx| {
        pane.read(cx)
            .pending_paste()
            .map(|paste| paste.line_count())
    });
    assert_eq!(lines, Some(7));
    ui.press("escape", cx);
    assert!(!ui.visible("paste-dialog", cx));
    assert!(captured.0.lock().unwrap().is_empty(), "nothing pasted");

    // Bracketed paste: the program enables mode 2004; markers inside the
    // text are removed.
    handle.feed_blocking(b"\x1b[?2004h").unwrap();
    handle.flush_blocking();
    terminal.update(cx, |terminal, cx| {
        terminal.paste_text("a\x1b[201~\nb\x1b[200~", cx)
    });
    ui.frame(cx);
    assert!(ui.visible("paste-dialog", cx));
    ui.press("enter", cx);
    assert!(!ui.visible("paste-dialog", cx));
    assert_eq!(
        captured.0.lock().unwrap().as_slice(),
        b"\x1b[200~a\rb\x1b[201~"
    );
    ui.finish(cx);
}

/// UI 4.1, 12.5: the focus goes into the terminal once the session runs;
/// there Ctrl+L (and other Ctrl+letters) go to the shell, not to the app,
/// while Ctrl+1..9, Alt+Left/Right and Ctrl+Shift+W still switch and close
/// tabs. Outside a terminal Ctrl+L opens a local terminal.
#[gpui_kit::test]
fn terminal_keys_and_app_shortcuts(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let shell = ui.shell(cx);
    // Outside a terminal: Ctrl+L opens a local terminal tab.
    ui.press("ctrl-l", cx);
    let tabs = |cx: &mut TestAppContext| cx.update(|cx| shell.read(cx).sessions().len());
    assert_eq!(tabs(cx), 1);
    let first = cx.update(|cx| shell.read(cx).sessions().tabs()[0].id());
    let pane = cx.update(|cx| shell.read(cx).pane(first).cloned().unwrap());
    set_phase(&ui, &pane, Phase::Running, None, cx);
    let captured = Arc::new(Captured::default());
    ui.record.terminals()[0].set_sink(Some(captured.clone()));
    let terminal_focused = |cx: &mut TestAppContext| {
        cx.update_window(ui.handle, |_, window, cx| {
            let terminal = pane.read(cx).terminal().cloned().unwrap();
            terminal.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap()
    };
    assert!(terminal_focused(cx), "the terminal has the focus (UI 4.1)");

    // In the terminal: Ctrl+L is the shell's (form feed), no new tab.
    ui.press("ctrl-l", cx);
    assert_eq!(tabs(cx), 1);
    assert_eq!(captured.0.lock().unwrap().as_slice(), b"\x0c");

    // App shortcuts that the terminal leaves to the app.
    ui.press("ctrl-1", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.press("ctrl-3", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(first));
    assert!(terminal_focused(cx), "back in the terminal");
    ui.press("alt-left", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Sftp);
    ui.press("alt-right", cx);
    assert_eq!(ui.top_tab(cx), TopTab::Session(first));
    ui.press("ctrl-shift-w", cx);
    assert_eq!(tabs(cx), 0);
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    assert!(ui.record.calls().contains(&Call::Close));
    ui.finish(cx);
}

/// UI 13.3: while locked the session tab stays alive (nothing is told to
/// end) but nothing of it is drawn, the unlock screen covers the window;
/// after unlock the same tab is back and its terminal has the focus.
#[gpui_kit::test]
fn locking_hides_the_session_and_unlock_brings_it_back(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let (id, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    set_phase(&ui, &pane, Phase::Running, None, cx);
    assert!(ui.visible("session-terminal", cx));

    ui.press("ctrl-shift-l", cx);
    let screen = |cx: &mut TestAppContext| cx.update(|cx| ui.root.read(cx).screen(cx));
    assert_eq!(screen(cx), app::root::Screen::Unlock);
    assert!(!ui.visible("session-terminal", cx), "hidden while locked");
    assert!(!ui.visible("session-tab-1", cx));
    assert!(
        !ui.record.calls().contains(&Call::Close),
        "the session lives on"
    );

    let password = ui.password.clone();
    ui.state
        .update(cx, |state, cx| state.unlock(password, cx))
        .detach();
    ui.frame(cx);
    assert_eq!(screen(cx), app::root::Screen::Shell);
    assert_eq!(ui.top_tab(cx), TopTab::Session(id));
    assert!(ui.visible("session-terminal", cx));
    let focused = cx
        .update_window(ui.handle, |_, window, cx| {
            let terminal = pane.read(cx).terminal().cloned().unwrap();
            terminal.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap();
    assert!(focused, "the terminal has the focus again");
    ui.finish(cx);
}

/// A view of the session layer, as `crate::sessions` reports it.
fn layer_view(state: crate::sessions::SessionState) -> crate::sessions::SessionView {
    use shellpp_core::service::dto::{Backspace, SessionKind as LayerKind};
    crate::sessions::SessionView {
        id: 1,
        kind: LayerKind::Ssh,
        state,
        step: None,
        title: "web".into(),
        host: None,
        address: Some("192.0.2.20".into()),
        port: Some(2200),
        username: None,
        os: None,
        theme: "graphite".into(),
        backspace: Backspace::Default,
        prompt: None,
        error: None,
        host_key: None,
        exit_status: None,
        exit_signal: None,
        reconnect: None,
        shell_id: None,
        rev: 0,
    }
}

/// UI 4.4: a local terminal opened with the default shell takes over the
/// shell the session layer resolved, so Duplicate, a split and Restore open
/// that shell even when the default changes later.
#[gpui_kit::test]
fn a_default_local_terminal_pins_its_resolved_shell(cx: &mut TestAppContext) {
    use std::cell::Cell;

    use shellpp_core::service::dto::SessionKind as LayerKind;

    use crate::panes::PaneTarget;
    use crate::sessions::{OpenRequest, SessionState};

    let ui = Ui::open(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Local), cx);
    assert_eq!(
        cx.update(|cx| pane.read(cx).request().clone()),
        OpenRequest::Local { shell_id: None }
    );
    let shown = Cell::new(None);
    let mut view = layer_view(SessionState::Connected);
    view.kind = LayerKind::Local;
    view.address = None;
    view.port = None;
    view.title = "PowerShell 7".into();
    view.shell_id = Some("pwsh".into());
    pane.update(cx, |pane, cx| bridge::apply(pane, &view, &shown, cx));
    let target = cx.update(|cx| PaneTarget::from_open(pane.read(cx).request()));
    assert_eq!(
        target,
        PaneTarget::Local {
            shell_id: Some("pwsh".into())
        }
    );
    // A later view of another shell does not move a pinned request.
    view.shell_id = Some("cmd".into());
    pane.update(cx, |pane, cx| bridge::apply(pane, &view, &shown, cx));
    assert_eq!(
        cx.update(|cx| pane.read(cx).request().clone()),
        OpenRequest::Local {
            shell_id: Some("pwsh".into())
        }
    );
    ui.finish(cx);
}

/// The bridge (`bridge::apply`) turns every view of the session layer into
/// the tab's phase, dialogs, title, OS icon and theme (UI 4.1, 4.6, 4.8,
/// 10.2), and remembers the id of the prompt on display for the answer.
#[gpui_kit::test]
fn bridge_maps_session_views_to_the_tab(cx: &mut TestAppContext) {
    use std::cell::Cell;

    use shellpp_core::service::dto::{Backspace, HostOs, OsFamily, OsSource};

    use crate::sessions::{
        self as layer, PromptKind, ReconnectView, SaveOption, SessionError, SessionPrompt,
        SessionState,
    };

    let ui = Ui::open(cx);
    let target = crate::hosts::parse_address("192.0.2.20:2200").unwrap();
    let (id, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).quick(target), cx);
    let shown = Cell::new(None);
    let apply = |view: layer::SessionView, cx: &mut TestAppContext| {
        pane.update(cx, |pane, cx| bridge::apply(pane, &view, &shown, cx));
        ui.frame(cx);
    };
    let phase = |cx: &mut TestAppContext| cx.update(|cx| pane.read(cx).phase().clone());
    let key = shellpp_core::service::dto::HostKeyInfo {
        address: "192.0.2.20".into(),
        port: 2200,
        key_type: "ssh-ed25519".into(),
        fingerprint: "SHA256:presented".into(),
    };

    // Connecting at a step; the first connection's host key question.
    let mut view = layer_view(SessionState::Connecting);
    view.step = Some(layer::ConnectStep::Authenticating);
    apply(view.clone(), cx);
    assert_eq!(phase(cx), Phase::Connecting(ConnectStep::Authenticating));
    view.step = Some(layer::ConnectStep::HostKey);
    view.prompt = Some(SessionPrompt {
        id: 7,
        kind: PromptKind::HostKeyUnknown { key: key.clone() },
    });
    apply(view.clone(), cx);
    let asked = cx.update(|cx| pane.read(cx).host_key().cloned());
    assert_eq!(
        asked,
        Some(HostKeyAsk::Unknown(HostKeyInfo {
            address: "[192.0.2.20]:2200".into(),
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:presented".into(),
        }))
    );
    assert_eq!(shown.get(), Some(7));
    assert!(ui.visible("host-key-unknown", cx));

    // A refused password: the form with the error and "Save".
    view.prompt = Some(SessionPrompt {
        id: 8,
        kind: PromptKind::Password {
            username: "tester".into(),
            error: true,
            save: SaveOption::Vault,
        },
    });
    apply(view.clone(), cx);
    let prompt = cx.update(|cx| pane.read(cx).prompt().cloned());
    assert_eq!(
        prompt,
        Some(Prompt::Password {
            username: "tester".into(),
            error: true,
            keep: Keep::Save,
        })
    );
    assert_eq!(shown.get(), Some(8));
    assert!(ui.visible("session-prompt-error", cx));

    // Connected: the terminal; the detected OS and the title reach the tab;
    // the host's own theme reaches the terminal.
    let mut view = layer_view(SessionState::Connected);
    view.title = "web renamed".into();
    view.theme = "paper".into();
    view.backspace = Backspace::CtrlH;
    view.os = Some(HostOs {
        family: OsFamily::Linux,
        distro: None,
        version: None,
        source: OsSource::Script,
        detected_at: String::new(),
    });
    apply(view, cx);
    assert_eq!(phase(cx), Phase::Running);
    assert_eq!(shown.get(), None);
    assert!(cx.update(|cx| pane.read(cx).prompt().is_none()));
    let shell = ui.shell(cx);
    let tab = cx.update(|cx| shell.read(cx).sessions().get(id).cloned().unwrap());
    assert_eq!(tab.title().as_ref(), "web renamed");
    assert_eq!(tab.icon(), crate::hosts::layout::os_icon(Some("linux")));
    let slug = cx.update(|cx| pane.read(cx).theme_slug().map(str::to_owned));
    assert_eq!(slug.as_deref(), Some("paper"));
    // The host's Backspace = ^H reaches the terminal (UI 3.1).
    let backspace = cx.update(|cx| {
        pane.read(cx)
            .terminal()
            .map(|terminal| terminal.read(cx).backspace())
    });
    assert_eq!(backspace, Some(crate::terminal::input::BackspaceKey::CtrlH));

    // Ended: a non-zero code, a signal.
    let mut ended = layer_view(SessionState::Ended);
    ended.exit_status = Some(3);
    apply(ended.clone(), cx);
    assert_eq!(phase(cx), Phase::Ended(ExitStatus::Code(3)));
    ended.exit_signal = Some("KILL".into());
    apply(ended, cx);
    assert_eq!(phase(cx), Phase::Ended(ExitStatus::Signal("KILL".into())));

    // Dropped with the countdown; a running attempt over the output.
    let next_at = Instant::now() + Duration::from_secs(4);
    let mut dropped = layer_view(SessionState::Dropped);
    dropped.error = Some(SessionError::ConnectionLost);
    dropped.reconnect = Some(ReconnectView {
        attempt: 2,
        next_at: Some(next_at),
        waiting_for_unlock: false,
        in_progress: false,
    });
    apply(dropped, cx);
    assert_eq!(phase(cx), Phase::Dropped(Failure::ConnectionLost));
    let info = cx.update(|cx| pane.read(cx).reconnect_info().copied());
    assert_eq!(
        info.map(|info| (info.attempt, info.next_at)),
        Some((2, Some(next_at)))
    );
    assert!(ui.visible("session-dropped-countdown", cx));
    let mut attempt = layer_view(SessionState::Connecting);
    attempt.step = Some(layer::ConnectStep::Connecting);
    attempt.reconnect = Some(ReconnectView {
        attempt: 2,
        next_at: None,
        waiting_for_unlock: false,
        in_progress: true,
    });
    apply(attempt, cx);
    assert_eq!(phase(cx), Phase::Reconnecting(ConnectStep::Connecting));
    assert!(ui.visible("session-reconnecting", cx));

    // A revoked key: the failure screen with the presented key.
    let mut revoked = layer_view(SessionState::Failed);
    revoked.error = Some(SessionError::HostKeyRevoked);
    revoked.host_key = Some(key);
    apply(revoked, cx);
    assert_eq!(phase(cx), Phase::Failed(Failure::HostKeyRevoked));
    assert!(ui.visible("host-key-revoked-warning", cx));
    assert!(!ui.visible("failed-retry", cx));

    // Closed: the tab closes.
    apply(layer_view(SessionState::Closed), cx);
    assert!(cx.update(|cx| shell.read(cx).sessions().get(id).is_none()));
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.finish(cx);
}

/// The whole path of a session tab (stage 2a): the real session layer and
/// the russh test server on 127.0.0.1 (in process; no Docker, no key files,
/// no agent). The first connection asks for the host key (accepted and
/// saved), the password is asked and saved, the shell's output reaches the
/// terminal and typed input reaches the shell; a dropped connection comes
/// back by itself (UI 4.6), a non-zero exit keeps the tab, Reconnect runs
/// the session again, and `exit 0` closes the tab.
#[gpui_kit::test]
fn real_session_runs_in_the_tab(cx: &mut TestAppContext) {
    use shellpp_test_sshd::{ServerConfig, TestServer, UserConfig};

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    let password = generated();
    let user =
        UserConfig::new("alice").with_password(secrecy::SecretString::from(password.clone()));
    let server = runtime
        .block_on(TestServer::start(ServerConfig::new(vec![user])))
        .expect("test server");

    let ui = Ui::open_real(cx);
    let port = server.port();
    let host = ui.state.update(cx, |state, cx| {
        let data = json!({"label": "e2e", "address": "127.0.0.1", "ssh": {"username": "alice", "port": port}});
        state
            .save_host(None, data.as_object().cloned().unwrap(), cx)
            .unwrap()
            .record_id
    });
    let (id, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    let shell = ui.shell(cx);
    let phase = |cx: &mut TestAppContext| cx.update(|cx| pane.read(cx).phase().clone());
    let screen = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let terminal = pane.read(cx).terminal().cloned().expect("terminal");
            terminal.read(cx).handle().screen_text()
        })
    };

    // TOFU: the server's key; Enter accepts and saves it.
    ui.wait_until("the host key dialog", cx, |cx| {
        cx.update(|cx| pane.read(cx).host_key().is_some())
    });
    let asked = cx.update(|cx| pane.read(cx).host_key().cloned());
    match asked {
        Some(HostKeyAsk::Unknown(key)) => {
            assert_eq!(key.fingerprint, server.host_key_fingerprint());
            assert_eq!(key.address, format!("[127.0.0.1]:{port}"));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(ui.visible("host-key-unknown", cx));
    ui.press("enter", cx);

    // The password, saved for the automatic reconnection below.
    ui.wait_until("the password form", cx, |cx| {
        cx.update(|cx| pane.read(cx).prompt_form().is_some())
    });
    assert!(ui.visible("session-prompt-keep", cx));
    ui.click("session-prompt-keep", cx);
    ui.click("session-prompt-input", cx);
    ui.input(&password, cx);
    ui.press("enter", cx);

    // Running: the server's output on screen.
    ui.wait_until("the shell", cx, |cx| {
        phase(cx) == Phase::Running
            && screen(cx)
                .iter()
                .any(|line| line.contains("Welcome to shellpp-test-sshd"))
    });
    let known = cx.update(|cx| {
        let core = ui.state.read(cx).core().cloned().unwrap();
        core.known_hosts_list().unwrap().len()
    });
    assert_eq!(known, 1, "the key is saved in known hosts");
    ui.input("echo e2e-marker", cx);
    ui.press("enter", cx);
    ui.wait_until("the echoed marker", cx, |cx| {
        screen(cx)
            .iter()
            .any(|line| line.trim_end() == "e2e-marker")
    });

    // A transport drop: the output stays, the session comes back by itself
    // with the saved key and password.
    ui.input("drop", cx);
    ui.press("enter", cx);
    ui.wait_until("the drop", cx, |cx| {
        matches!(phase(cx), Phase::Dropped(_) | Phase::Reconnecting(_))
    });
    ui.wait_until("the automatic reconnection", cx, |cx| {
        phase(cx) == Phase::Running
    });
    assert!(
        cx.update(|cx| pane.read(cx).prompt().is_none()),
        "nothing asked again"
    );

    // A non-zero exit keeps the tab; Reconnect runs the session again.
    ui.input("exit 3", cx);
    ui.press("enter", cx);
    ui.wait_until("the end", cx, |cx| {
        phase(cx) == Phase::Ended(ExitStatus::Code(3))
    });
    assert!(ui.visible("session-ended", cx));
    ui.click("session-ended-reconnect", cx);
    ui.wait_until("the session again", cx, |cx| phase(cx) == Phase::Running);

    // `exit 0` closes the tab (no neighbor: Vaults).
    ui.input("exit 0", cx);
    ui.press("enter", cx);
    ui.wait_until("the tab to close", cx, |cx| {
        cx.update(|cx| shell.read(cx).sessions().get(id).is_none())
    });
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    let logged = cx.update(|cx| {
        let core = ui.state.read(cx).core().cloned().unwrap();
        core.recent_connections()
            .unwrap()
            .iter()
            .filter(|row| row.host.as_ref().is_some_and(|h| h.record_id == host))
            .count()
    });
    assert!(logged >= 1, "the connection log has the session");
    ui.finish(cx);
    runtime.block_on(server.shutdown());
}

/// The local terminal through the tab (lead decision 9): Ctrl+L outside a
/// terminal opens it on the real session layer with this machine's default
/// shell (Windows PowerShell on Windows, through ConPTY, whose start-up
/// query the terminal view answers); the tab takes the shell's name, typed
/// input runs, and `exit` closes the tab.
#[gpui_kit::test]
fn real_local_terminal_runs_in_the_tab(cx: &mut TestAppContext) {
    let ui = Ui::open_real(cx);
    let shell = ui.shell(cx);
    ui.press("ctrl-l", cx);
    let tab = cx.update(|cx| shell.read(cx).sessions().tabs()[0].clone());
    assert_eq!(tab.kind(), SessionKind::Local);
    let pane = cx.update(|cx| shell.read(cx).pane(tab.id()).cloned().unwrap());
    let screen = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let terminal = pane.read(cx).terminal().cloned().expect("terminal");
            terminal.read(cx).handle().screen_text()
        })
    };
    ui.wait_until("the local shell", cx, |cx| {
        cx.update(|cx| pane.read(cx).phase() == &Phase::Running)
            && screen(cx).iter().any(|line| !line.trim().is_empty())
    });
    if cfg!(windows) {
        let title = cx.update(|cx| {
            shell
                .read(cx)
                .sessions()
                .get(tab.id())
                .unwrap()
                .title()
                .clone()
        });
        assert_eq!(title.as_ref(), "PowerShell");
    }

    // The echo of the typed command never holds the marker in one piece.
    let marker = generated();
    let (head, tail) = marker.split_at(6);
    let command = if cfg!(windows) {
        format!("Write-Output ('{head}' + '{tail}')")
    } else {
        format!("echo {head}''{tail}")
    };
    ui.input(&command, cx);
    ui.press("enter", cx);
    ui.wait_until("the command's output", cx, |cx| {
        screen(cx).iter().any(|line| line.trim_end() == marker)
    });
    ui.input("exit", cx);
    ui.press("enter", cx);
    ui.wait_until("the tab to close", cx, |cx| {
        cx.update(|cx| shell.read(cx).sessions().is_empty())
    });
    assert_eq!(ui.top_tab(cx), TopTab::Vaults);
    ui.finish(cx);
}

/// UI 1.1 (owner's stage 2b step 0.2): a dropped session's terminal is
/// dimmed and has no cursor while it is down; running again restores both.
#[gpui_kit::test]
fn a_dropped_session_dims_its_terminal(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host = ui.save_host(cx);
    let (_, pane) = ui.open_session(SessionRequest::new(SessionKind::Ssh).host(host), cx);
    set_phase(&ui, &pane, Phase::Running, None, cx);
    let dimmed = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            pane.read(cx)
                .terminal()
                .map(|terminal| terminal.read(cx).is_dimmed())
        })
    };
    assert_eq!(dimmed(cx), Some(false));
    set_phase(
        &ui,
        &pane,
        Phase::Dropped(Failure::ConnectionLost),
        None,
        cx,
    );
    assert_eq!(dimmed(cx), Some(true));
    set_phase(
        &ui,
        &pane,
        Phase::Reconnecting(ConnectStep::Authenticating),
        None,
        cx,
    );
    assert_eq!(dimmed(cx), Some(true), "still down while reconnecting");
    set_phase(&ui, &pane, Phase::Running, None, cx);
    assert_eq!(dimmed(cx), Some(false));
    ui.finish(cx);
}
