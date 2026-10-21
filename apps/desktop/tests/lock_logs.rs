//! The master password never reaches the log (owner's step 5.1, decision
//! D29): the first run, wrong passwords up to the pause, the unlock and
//! "Start with a new vault" run through the screens with a TRACE-level
//! `tracing` subscriber capturing every line of the app and the core. A
//! generated canary password must not appear in it. Headless (GPUI
//! `TestAppContext`), temporary profile, in-memory secret store.
//!
//! One test in its own binary: `tracing` caches whether a call site is
//! enabled, and a parallel test without a subscriber could leave a call
//! site cached as disabled.

use std::sync::{Arc, Mutex};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, SharedString, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use shellpp_desktop::app::root::Screen;
use shellpp_desktop::app::{self};
use shellpp_desktop::state::AppState;
use shellpp_desktop::testing::TestProfile;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[gpui_kit::test]
fn the_master_password_never_reaches_the_log(cx: &mut TestAppContext) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let suffix = shellpp_desktop::profile::random_suffix().unwrap();
    let canary = format!("Canary-{suffix}-pw");
    let wrong = format!("Wrong-{suffix}-pw");

    cx.update(|cx| {
        shellpp_desktop::testing::init_ui(cx);
        app::root::bind_keys(cx);
    });
    let (opened, _) = TestProfile::new().into_opened();
    let state = cx.new(|cx| AppState::new(opened, cx));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(px(1280.), px(800.)),
        ))),
        ..Default::default()
    };
    let (handle, root) = cx
        .update(|cx| app::open_window_with(state.clone(), options, cx))
        .unwrap();
    let screen = |cx: &mut TestAppContext| cx.update(|cx| root.read(cx).screen(cx));
    let step = |cx: &mut TestAppContext, f: &dyn Fn(&mut gpui_kit::Window, &mut gpui_kit::App)| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    let type_in = |cx: &mut TestAppContext, id: &'static str, text: &str| {
        let text = text.to_owned();
        step(cx, &move |window, cx| {
            window.click(SharedString::from(id), cx);
            window.press("secondary-a", cx);
            window.input(&text, cx);
        });
    };

    // First run with the canary.
    type_in(cx, "first-run-password", &canary);
    type_in(cx, "first-run-repeat", &canary);
    step(cx, &|window, cx| window.press("enter", cx));
    assert_eq!(screen(cx), Screen::Shell);

    // Lock, five wrong passwords (the fifth starts the pause), and the
    // canary during the pause.
    state.update(cx, |state, cx| state.lock(cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(screen(cx), Screen::Unlock);
    for _ in 0..5 {
        type_in(cx, "unlock-password", &wrong);
        step(cx, &|window, cx| window.press("enter", cx));
    }
    type_in(cx, "unlock-password", &canary);
    step(cx, &|window, cx| window.press("enter", cx));
    assert_eq!(screen(cx), Screen::Unlock);

    // "Forgot password?" -> a new vault, again with the canary.
    step(cx, &|window, cx| window.click("unlock-forgot", cx));
    step(cx, &|window, cx| window.click("forgot-new-vault", cx));
    step(cx, &|window, cx| window.click("forgot-start-over", cx));
    assert_eq!(screen(cx), Screen::FirstRun);
    type_in(cx, "first-run-password", &canary);
    type_in(cx, "first-run-repeat", &canary);
    step(cx, &|window, cx| window.press("enter", cx));
    assert_eq!(screen(cx), Screen::Shell);

    state.update(cx, |state, _| state.shutdown());
    let logs = String::from_utf8_lossy(&capture.0.lock().unwrap()).into_owned();
    for expected in [
        "first run completed",
        "wrong master password; unlock paused",
        "data file archived; first run",
    ] {
        assert!(logs.contains(expected), "the log has {expected:?}");
    }
    assert!(!logs.contains(&canary), "the master password is in the log");
    assert!(!logs.contains(&wrong), "a typed password is in the log");
    assert!(!logs.contains(&suffix), "part of a password is in the log");
}
