//! `--screenshots <out-dir> [--size WxH[,WxH...]]` (cargo feature
//! `screenshots`, owner's steps 4 and 5.2): one launch of the real app that
//! saves PNG screenshots of **our own window only** for each window size,
//! each screen state of that size ([`states_for`]) and each theme (dark,
//! light), then quits.
//!
//! - Data: a fresh demo profile (300 hosts, 5 groups, see [`super::demo`])
//!   created inside `SHELLPP_DATA_DIR` when that variable is set (the
//!   scripts point it at a temporary folder), otherwise in the OS temporary
//!   directory. The first-run, unlock and empty-state screens get their own
//!   empty profiles next to it ([`demo::open_fresh`]). Every profile keeps
//!   the device key in memory (never the OS keychain) and is deleted at quit.
//! - Capture: GPUI's own renderer draws the window's current scene into an
//!   offscreen texture and reads it back (`Window::render_to_image`, the
//!   DirectX renderer on Windows). Nothing is read from the screen, so other
//!   windows, the desktop or an overlapping window can never appear in a
//!   screenshot, and the window does not have to be fully on screen.
//! - Size: `WxH` is the window content size in logical pixels; the PNG has
//!   the device pixels of the window (`W * scale` x `H * scale`, the scale
//!   factor is printed). One window per size; its minimum size is raised to
//!   `WxH` so that Windows does not clamp a window larger than the display.
//!   Several sizes in one launch (`--size 1920x1200,1280x800,960x600`, what
//!   `scripts/ui-screens.ps1` passes) keep the number of app launches at one.
//! - Names: `<W>x<H>-<theme>-<state>.png`, e.g. `1280x800-dark-root.png`.
//! - States are reached the way a user gets there (clicks, typing, keys on
//!   the elements' ids, see [`ShotState`]); each demo state starts from the
//!   same reset (top level, no panel, no selection, empty search, tiles, no
//!   session tabs, no toasts). The pointer is then moved out of the window so
//!   that no hover state is captured, and the state is checked
//!   ([`check_state`]) before the capture.
//! - Exit code 0 when every PNG was written with the expected size, 1 on any
//!   failure; a watchdog ends a run that hangs after [`WATCHDOG`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Entity, InputEvent as _, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollDelta, Size, Window,
    WindowBounds, point, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use uuid::Uuid;

use super::demo::{self, FreshStage};
use crate::app::{self, RootView};
use crate::hosts::group_card_id;
use crate::hosts::tag_filter::tag_option_id;
use crate::profile::TempProfile;
use crate::shell::{Section, Shell, TopTab};
use crate::state::AppState;
use crate::state::hosts_view::HostsLayout;
use crate::theme::{self, Appearance};

/// One window content size in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl WindowSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Parses `1920x1200` (also `1920X1200`); both sides 320..=7680.
    pub fn parse(value: &str) -> Option<Self> {
        let (width, height) = value.trim().split_once(['x', 'X'])?;
        let width: u32 = width.trim().parse().ok()?;
        let height: u32 = height.trim().parse().ok()?;
        let valid = 320..=7680;
        (valid.contains(&width) && valid.contains(&height)).then_some(Self { width, height })
    }

    pub fn label(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    fn logical(&self) -> Size<Pixels> {
        size(px(self.width as f32), px(self.height as f32))
    }
}

impl std::fmt::Display for WindowSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// The large size of the original set.
pub const LARGE_SIZE: WindowSize = WindowSize::new(1920, 1200);
/// The app's default window size: every state is captured at this size.
pub const MAIN_SIZE: WindowSize = WindowSize::new(1280, 800);
/// The app's minimum window size (lead decision 4): the root and the host
/// panel are captured at this size.
pub const MIN_SIZE: WindowSize = WindowSize::new(960, 600);

/// The window sizes of one run (`--size`), in capture order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotSize {
    sizes: Cow<'static, [WindowSize]>,
}

/// Without `--size`: the app's default window size only.
pub const DEFAULT_SIZE: ShotSize = ShotSize {
    sizes: Cow::Borrowed(&[MAIN_SIZE]),
};

/// The full set of `scripts/ui-screens.ps1`.
pub const FULL_SET: ShotSize = ShotSize {
    sizes: Cow::Borrowed(&[LARGE_SIZE, MAIN_SIZE, MIN_SIZE]),
};

/// At most this many sizes in one launch.
const MAX_SIZES: usize = 8;

impl ShotSize {
    /// Parses one size (`1920x1200`) or a comma-separated list of distinct
    /// sizes (`1920x1200,1280x800,960x600`).
    pub fn parse(value: &str) -> Option<Self> {
        let mut sizes = Vec::new();
        for part in value.split(',') {
            let size = WindowSize::parse(part)?;
            if sizes.contains(&size) {
                return None;
            }
            sizes.push(size);
        }
        (!sizes.is_empty() && sizes.len() <= MAX_SIZES).then_some(Self {
            sizes: Cow::Owned(sizes),
        })
    }

    pub fn sizes(&self) -> &[WindowSize] {
        &self.sizes
    }
}

impl std::fmt::Display for ShotSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let labels: Vec<String> = self.sizes.iter().map(WindowSize::label).collect();
        f.write_str(&labels.join(","))
    }
}

/// A run that has not finished after this long is ended with exit code 3.
pub const WATCHDOG: Duration = Duration::from_secs(600);
/// Settling time after a state change before the frame is captured (covers
/// the 180 ms panel motion of the design system and asset loading).
const SETTLE: Duration = Duration::from_millis(700);
/// Device-pixel tolerance when comparing the PNG with the requested size.
const SIZE_TOLERANCE: i64 = 2;

/// The text typed for the address states (owner's step 5.2).
pub const QUICK_CONNECT_TEXT: &str = "root@10.0.0.5:2222";
/// A search that matches some demo hosts (the "web" role).
pub const SEARCH_WITH_RESULTS: &str = "web";
/// A search that matches nothing in the demo data.
pub const SEARCH_WITHOUT_RESULTS: &str = "no-such-host";
/// The tags checked in the tag filter state (the first demo host has both).
pub const TAG_FILTER_EXAMPLE: [&str; 2] = ["prod", "linux"];

/// The command-line options of a screenshot run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotOptions {
    pub out_dir: PathBuf,
    pub size: ShotSize,
}

/// The themes captured, in order.
pub const THEMES: [(Appearance, &str); 2] =
    [(Appearance::Dark, "dark"), (Appearance::Light, "light")];

/// The screen states captured, each in both themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotState {
    /// The Hosts screen at the top level (Groups and Hosts sections).
    Root,
    /// Inside the first group (breadcrumbs, its subgroups and hosts).
    Group,
    /// The first top-level host selected, its panel open.
    HostPanel,
    /// The context menu of the first host card (right click).
    ContextMenu,
    /// [`QUICK_CONNECT_TEXT`] in the search bar, its Connect button active.
    SearchAddress,
    /// A search with results ([`SEARCH_WITH_RESULTS`]).
    SearchResults,
    /// A search without results ([`SEARCH_WITHOUT_RESULTS`]).
    SearchNoResults,
    /// Three host cards selected (Ctrl+click on each).
    ThreeSelected,
    /// The Hosts screen of an empty profile (UI 2.3, empty state).
    Empty,
    /// The list view (UI 2.2 view toggle).
    ListView,
    /// New host: the empty panel, the focus in Address.
    NewHost,
    /// The toast after a quick connect ("Save host?").
    QuickConnectToast,
    /// The tag filter dropdown open (UI 2.2).
    TagFilter,
    /// The example tags on and the host panel open: the narrower action bar
    /// collapses their chips into "+N" (UI 2.2).
    TagChips,
    /// The bottom of the host panel with "Show more" expanded (UI 3.1).
    ShowMore,
    /// The first-run screen (UI 13.1).
    FirstRun,
    /// The unlock screen (UI 13.2).
    Unlock,
}

/// Which app state a screen state is shown on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    /// The demo profile (300 hosts), shared by the demo states.
    Demo,
    /// A fresh empty profile at this lock stage, in its own window.
    Fresh(FreshStage),
}

impl ShotState {
    /// The original three states (every size but the minimum).
    pub const BASE: [Self; 3] = [Self::Root, Self::Group, Self::HostPanel];
    /// Every state, in capture order.
    pub const ALL: [Self; 17] = [
        Self::Root,
        Self::Group,
        Self::HostPanel,
        Self::ContextMenu,
        Self::SearchAddress,
        Self::SearchResults,
        Self::SearchNoResults,
        Self::ThreeSelected,
        Self::Empty,
        Self::ListView,
        Self::NewHost,
        Self::QuickConnectToast,
        Self::TagFilter,
        Self::TagChips,
        Self::ShowMore,
        Self::FirstRun,
        Self::Unlock,
    ];
    /// The states at the minimum window size: the Hosts screen without and
    /// with the details panel, and the tallest lock screen.
    pub const MINIMUM: [Self; 3] = [Self::Root, Self::HostPanel, Self::FirstRun];

    pub fn name(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Group => "group",
            Self::HostPanel => "host-panel",
            Self::ContextMenu => "context-menu",
            Self::SearchAddress => "search-address",
            Self::SearchResults => "search-results",
            Self::SearchNoResults => "search-no-results",
            Self::ThreeSelected => "three-selected",
            Self::Empty => "empty",
            Self::ListView => "list-view",
            Self::NewHost => "new-host",
            Self::QuickConnectToast => "quick-connect-toast",
            Self::TagFilter => "tag-filter",
            Self::TagChips => "tag-chips",
            Self::ShowMore => "show-more",
            Self::FirstRun => "first-run",
            Self::Unlock => "unlock",
        }
    }

    fn scene(self) -> Scene {
        match self {
            Self::Empty => Scene::Fresh(FreshStage::Unlocked),
            Self::FirstRun => Scene::Fresh(FreshStage::FirstRun),
            Self::Unlock => Scene::Fresh(FreshStage::Locked),
            _ => Scene::Demo,
        }
    }
}

/// The states captured at `size`: every state at the app's default size,
/// the root, the host panel and the first run at the minimum size, the
/// original three at any other size.
pub fn states_for(size: WindowSize) -> &'static [ShotState] {
    if size == MAIN_SIZE {
        &ShotState::ALL
    } else if size == MIN_SIZE {
        &ShotState::MINIMUM
    } else {
        &ShotState::BASE
    }
}

/// `<W>x<H>-<theme>-<state>.png`.
pub fn file_name(size: WindowSize, theme: &str, state: ShotState) -> String {
    format!("{}-{theme}-{}.png", size.label(), state.name())
}

/// Every file a run writes, in capture order (size, state, theme).
pub fn expected_files(sizes: &ShotSize) -> Vec<String> {
    let mut files = Vec::new();
    for size in sizes.sizes() {
        for state in states_for(*size) {
            for (_, theme) in THEMES {
                files.push(file_name(*size, theme, *state));
            }
        }
    }
    files
}

/// Why a screenshot run failed.
#[derive(Debug, thiserror::Error)]
pub enum ScreenshotError {
    #[error("the output directory could not be created: {0}")]
    OutDir(#[source] std::io::Error),
    #[error("the demo profile could not be created: {0}")]
    Profile(#[source] std::io::Error),
    #[error("the demo setup failed: {0}")]
    Demo(#[from] demo::DemoError),
    #[error("the main window did not open")]
    Window,
    #[error("the window closed during the run")]
    WindowClosed,
    #[error("{0}: the state was not reached")]
    State(String),
    #[error("{file}: rendering failed: {message}")]
    Render { file: String, message: String },
    #[error("{file}: saving failed: {message}")]
    Save { file: String, message: String },
    #[error(
        "{file}: got {got_width}x{got_height} device pixels, expected {want_width}x{want_height}"
    )]
    Size {
        file: String,
        got_width: u32,
        got_height: u32,
        want_width: u32,
        want_height: u32,
    },
    #[error("the run ended before all screenshots were taken")]
    Incomplete,
}

impl ScreenshotError {
    fn state(state: ShotState, reason: &str) -> Self {
        Self::State(format!("{}: {reason}", state.name()))
    }
}

/// The result of a run: every saved screenshot, or every failure.
type Outcome = Result<Vec<Shot>, Vec<ScreenshotError>>;

/// One saved screenshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// The folder for the run's profiles: `SHELLPP_DATA_DIR` when set (a
/// temporary folder chosen by the caller), else the OS temporary folder.
fn profiles_parent() -> Result<PathBuf, ScreenshotError> {
    match std::env::var_os(shellpp_core::ids::ENV_DATA_DIR) {
        Some(parent) if !parent.is_empty() => {
            let parent = PathBuf::from(parent);
            crate::dirs::create_private_dir(&parent).map_err(ScreenshotError::Profile)?;
            Ok(parent)
        }
        _ => Ok(std::env::temp_dir()),
    }
}

/// A new temporary profile `<parent>/shellpp-demo-<random>`.
fn create_profile(parent: &Path) -> Result<TempProfile, ScreenshotError> {
    demo::create_profile_in(parent).map_err(|error| match error {
        demo::DemoError::Profile(io) => ScreenshotError::Profile(io),
        other => ScreenshotError::Demo(other),
    })
}

#[allow(clippy::print_stdout)]
fn say(line: &str) {
    println!("{line}");
}

#[allow(clippy::print_stderr)]
fn complain(line: &str) {
    eprintln!("shellpp --screenshots: {line}");
}

/// Runs the app once, takes all screenshots and quits. Returns the process
/// exit code (0 success, 1 failure).
pub fn run(options: ScreenshotOptions) -> std::process::ExitCode {
    match run_inner(options) {
        Ok(shots) => {
            say(&format!("Saved {} screenshots.", shots.len()));
            std::process::ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in &errors {
                complain(&error.to_string());
            }
            std::process::ExitCode::from(1)
        }
    }
}

fn start_watchdog(profile_path: PathBuf) {
    std::thread::spawn(move || {
        std::thread::sleep(WATCHDOG);
        complain(&format!(
            "no result after {} s, giving up",
            WATCHDOG.as_secs()
        ));
        // Best effort: the database may still be open.
        let _ = std::fs::remove_dir_all(&profile_path);
        std::process::exit(3);
    });
}

fn run_inner(options: ScreenshotOptions) -> Outcome {
    std::fs::create_dir_all(&options.out_dir)
        .map_err(|error| vec![ScreenshotError::OutDir(error)])?;
    let out_dir = std::path::absolute(&options.out_dir).unwrap_or(options.out_dir.clone());
    let parent = profiles_parent().map_err(|error| vec![error])?;
    let profile = create_profile(&parent).map_err(|error| vec![error])?;
    say(&format!("Demo profile: {}", profile.path().display()));
    start_watchdog(profile.path().to_path_buf());
    crate::logging::init(&profile.dirs().logs);
    let (opened, summary) = demo::open(profile).map_err(|error| vec![error.into()])?;
    tracing::info!(hosts = summary.hosts, "screenshot run");
    let first_group = summary.groups.first().copied();

    let outcome: Rc<RefCell<Option<Outcome>>> = Rc::new(RefCell::new(None));
    let outcome_in_app = outcome.clone();
    let sizes = options.size;

    gpui_kit::application()
        .with_assets(app::assets::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            app::root::bind_keys(cx);
            let state = cx.new(|cx| AppState::new(opened, cx));

            let quit_state = state.clone();
            cx.on_app_quit(move |cx| {
                quit_state.update(cx, |state, _| state.shutdown());
                async {}
            })
            .detach();

            cx.activate(true);
            let run = Run {
                demo: state,
                first_group,
                parent,
                out_dir,
            };
            cx.spawn(async move |cx: &mut AsyncApp| {
                let result = run.capture_all(cx, &sizes).await;
                *outcome_in_app.borrow_mut() = Some(result);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });

    let result = outcome.borrow_mut().take();
    result.unwrap_or_else(|| Err(vec![ScreenshotError::Incomplete]))
}

/// The options of a screenshot window of `size`.
fn window_options(size: WindowSize, cx: &App) -> gpui::WindowOptions {
    let mut options = app::main_window_options(cx);
    options.window_bounds = Some(WindowBounds::centered(size.logical(), cx));
    // Lifts Windows' maximum track size (the display size) to the requested
    // size, so a 1920x1200 window opens on a smaller display.
    options.window_min_size = Some(size.logical());
    options
}

/// Opens a window of `size` over `state`.
fn open_window(
    cx: &mut AsyncApp,
    state: &Entity<AppState>,
    size: WindowSize,
) -> Result<(AnyWindowHandle, Entity<RootView>), ScreenshotError> {
    cx.update(|cx| {
        let options = window_options(size, cx);
        app::open_window_with(state.clone(), options, cx)
    })
    .map_err(|_| ScreenshotError::Window)
}

fn close_window(cx: &mut AsyncApp, handle: AnyWindowHandle) {
    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
}

/// One screenshot run inside the app.
struct Run {
    demo: Entity<AppState>,
    first_group: Option<Uuid>,
    /// Where the fresh profiles go (next to the demo profile).
    parent: PathBuf,
    out_dir: PathBuf,
}

impl Run {
    /// Takes every screenshot; continues after a failed one and reports all.
    async fn capture_all(&self, cx: &mut AsyncApp, sizes: &ShotSize) -> Outcome {
        let mut shots = Vec::new();
        let mut errors = Vec::new();
        let mut previous: Option<AnyWindowHandle> = None;
        for size in sizes.sizes().iter().copied() {
            // The next window opens before the previous one closes: on some
            // platforms closing the last window ends the app.
            let opened = open_window(cx, &self.demo, size);
            if let Some(previous) = previous.take() {
                close_window(cx, previous);
            }
            let (handle, root) = match opened {
                Ok(window) => window,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            previous = Some(handle);
            if let Err(error) = prepare_window(cx, handle, size).await {
                errors.push(error);
                continue;
            }
            for state in states_for(size).iter().copied() {
                let result = match state.scene() {
                    Scene::Demo => {
                        self.capture_demo_state(cx, handle, &root, size, state)
                            .await
                    }
                    Scene::Fresh(stage) => self.capture_fresh_state(cx, size, state, stage).await,
                };
                match result {
                    Ok(mut saved) => shots.append(&mut saved),
                    Err(mut failed) => errors.append(&mut failed),
                }
            }
        }
        if let Some(previous) = previous {
            close_window(cx, previous);
        }
        if errors.is_empty() {
            Ok(shots)
        } else {
            Err(errors)
        }
    }

    /// A demo state in both themes; the state is reached again for each.
    async fn capture_demo_state(
        &self,
        cx: &mut AsyncApp,
        handle: AnyWindowHandle,
        root: &Entity<RootView>,
        size: WindowSize,
        state: ShotState,
    ) -> Outcome {
        let mut shots = Vec::new();
        let mut errors = Vec::new();
        for (appearance, theme_name) in THEMES {
            set_theme(cx, handle, appearance).map_err(|error| vec![error])?;
            let file = file_name(size, theme_name, state);
            let reached = async {
                reach_state(cx, handle, root, &self.demo, self.first_group, state).await?;
                settle(cx, handle).await?;
                check_state(cx, handle, root, state)
            }
            .await;
            if let Err(error) = reached {
                errors.push(error);
                continue;
            }
            match capture(cx, handle, size, &self.out_dir, &file) {
                Ok(shot) => {
                    report(&shot);
                    shots.push(shot);
                }
                Err(error) => errors.push(error),
            }
        }
        if errors.is_empty() {
            Ok(shots)
        } else {
            Err(errors)
        }
    }

    /// A state of a fresh empty profile: its own window and app state, both
    /// themes, then the window closes and the profile is deleted.
    async fn capture_fresh_state(
        &self,
        cx: &mut AsyncApp,
        size: WindowSize,
        state: ShotState,
        stage: FreshStage,
    ) -> Outcome {
        let fresh = open_fresh_state(cx, &self.parent, stage).map_err(|error| vec![error])?;
        let result = async {
            let (handle, root) = open_window(cx, &fresh, size).map_err(|error| vec![error])?;
            let mut shots = Vec::new();
            let mut errors = Vec::new();
            if let Err(error) = prepare_window(cx, handle, size).await {
                errors.push(error);
            } else {
                for (appearance, theme_name) in THEMES {
                    let file = file_name(size, theme_name, state);
                    let ready = async {
                        set_theme(cx, handle, appearance)?;
                        settle(cx, handle).await?;
                        check_state(cx, handle, &root, state)
                    }
                    .await;
                    if let Err(error) = ready {
                        errors.push(error);
                        continue;
                    }
                    match capture(cx, handle, size, &self.out_dir, &file) {
                        Ok(shot) => {
                            report(&shot);
                            shots.push(shot);
                        }
                        Err(error) => errors.push(error),
                    }
                }
            }
            close_window(cx, handle);
            if errors.is_empty() {
                Ok(shots)
            } else {
                Err(errors)
            }
        }
        .await;
        cx.update(|cx| fresh.update(cx, |fresh, _| fresh.shutdown()));
        result
    }
}

/// An app state over a fresh empty profile at `stage` (see
/// [`demo::open_fresh`]).
pub fn open_fresh_state(
    cx: &mut AsyncApp,
    parent: &Path,
    stage: FreshStage,
) -> Result<Entity<AppState>, ScreenshotError> {
    let profile = create_profile(parent)?;
    let opened = demo::open_fresh(profile, stage)?;
    Ok(cx.update(|cx| cx.new(|cx| AppState::new(opened, cx))))
}

fn report(shot: &Shot) {
    say(&format!(
        "Saved {} ({}x{})",
        shot.path.display(),
        shot.width,
        shot.height
    ));
}

/// The first frames of a new window (layout, fonts, icons), and the size
/// check: a window the platform clamped is asked once more.
async fn prepare_window(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    size: WindowSize,
) -> Result<(), ScreenshotError> {
    settle(cx, handle).await?;
    let scale = cx
        .update_window(handle, |_, window, _| {
            if window.viewport_size() != size.logical() {
                window.resize(size.logical());
            }
            window.scale_factor()
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    say(&format!("Window {size} at scale factor {scale}"));
    settle(cx, handle).await
}

fn set_theme(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    appearance: Appearance,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        theme::set_appearance(appearance, Some(window), cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Waits for the UI to settle and draws a fresh frame.
pub async fn settle(cx: &mut AsyncApp, handle: AnyWindowHandle) -> Result<(), ScreenshotError> {
    cx.background_executor().timer(SETTLE / 2).await;
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .map_err(|_| ScreenshotError::WindowClosed)?;
    cx.background_executor().timer(SETTLE / 2).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Pointer out of the window: no hover state in the capture.
        window.dispatch_event(
            MouseMoveEvent {
                position: point(px(-10.), px(-10.)),
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// The shell of the window (the demo windows are always unlocked).
fn shell_of(
    cx: &mut AsyncApp,
    root: &Entity<RootView>,
    state: ShotState,
) -> Result<Entity<Shell>, ScreenshotError> {
    cx.update(|cx| root.read(cx).shell().cloned())
        .ok_or_else(|| ScreenshotError::state(state, "the shell is not shown"))
}

/// Brings the demo window back to the top level of the Hosts screen: no open
/// menu, panel, selection, search text, tag filter, session tab or toast;
/// tiles; the focus in the shell.
pub async fn reset_demo(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, ShotState::Root)?;
    // Open menus and dropdowns close on Escape.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
        window.press("escape", cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    let screen = cx.update(|cx| shell.read(cx).hosts_screen().clone());
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            let sessions: Vec<_> = shell.sessions().tabs().iter().map(|tab| tab.id()).collect();
            for id in sessions {
                shell.close_session(id, cx);
            }
            shell.set_section(Section::Hosts, cx);
            shell.set_top_tab(TopTab::Vaults, cx);
            shell.close_panel(cx);
            let toasts = shell.toasts().clone();
            toasts.update(cx, |toasts, cx| {
                let ids: Vec<u64> = toasts.items().iter().map(|toast| toast.id).collect();
                for id in ids {
                    toasts.dismiss(id, cx);
                }
            });
        });
        screen.update(cx, |screen, cx| {
            screen.set_layout(HostsLayout::Tiles, cx);
            let active: Vec<String> = screen.filter().tags().to_vec();
            for tag in active {
                screen.toggle_tag(&tag, cx);
            }
        });
        state.update(cx, |state, cx| {
            state.clear_selection(cx);
            state.set_location(None, cx);
        });
    });
    // The search bar is on screen again (Vaults tab): clear it like a user.
    let has_text = cx.update(|cx| !screen.read(cx).filter().text().is_empty());
    if has_text {
        type_into(cx, handle, SEARCH, "")?;
    }
    cx.update_window(handle, |_, window, cx| {
        let focus = shell.read(cx).focus_handle().clone();
        focus.focus(window, cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await
}

/// Element ids the states are reached through (UI 0: stable ids).
pub mod ids {
    pub const SEARCH: &str = "search";
    pub const QUICK_CONNECT: &str = "quick-connect";
    pub const VIEW_LIST: &str = "view-list";
    pub const NEW_HOST: &str = "new-host";
    pub const HOST_PANEL: &str = "host-panel";
    pub const HOST_PANEL_BODY: &str = "host-panel-body";
    pub const HOST_ADDRESS: &str = "host-address";
    pub const HOST_SHOW_MORE: &str = "host-show-more";
    pub const HOST_MORE: &str = "host-more";
    pub const POPUP_MENU: &str = "popup-menu";
    pub const BREADCRUMBS: &str = "breadcrumbs";
    pub const HOSTS_SCREEN: &str = "hosts-screen";
    pub const HOSTS_EMPTY: &str = "hosts-empty";
    pub const HOSTS_NO_MATCHES: &str = "hosts-no-matches";
    pub const TAG_FILTER: &str = "tag-filter";
    pub const TAG_FILTER_MENU: &str = "tag-filter-menu";
    pub const TAG_CHIP_MORE: &str = "tag-chip-more";
    pub const FIRST_RUN: &str = "first-run";
    pub const UNLOCK: &str = "unlock";
    /// Host cards: `host-card-<uuid>`.
    pub const HOST_CARD_PREFIX: &str = "host-card-";
    /// Toasts: `toast-<n>`.
    pub const TOAST_PREFIX: &str = "toast-";
}

use ids::*;

/// Brings the demo window into `shot_state` the way a user would, from the
/// reset state.
pub async fn reach_state(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    first_group: Option<Uuid>,
    shot_state: ShotState,
) -> Result<(), ScreenshotError> {
    reset_demo(cx, handle, root, state).await?;
    let not_reached = |reason: &str| ScreenshotError::state(shot_state, reason);
    match shot_state {
        ShotState::Root => Ok(()),
        ShotState::Group => {
            let group = cx.update(|cx| {
                first_group
                    .filter(|id| state.read(cx).group(*id).is_some())
                    .or_else(|| {
                        state
                            .read(cx)
                            .groups_in(None)
                            .first()
                            .map(|group| group.record_id)
                    })
            });
            let Some(group) = group else {
                return Err(not_reached("no groups"));
            };
            if !click_if_visible(cx, handle, group_card_id(group))? {
                // Not on screen: navigate the way the card would.
                cx.update(|cx| state.update(cx, |state, cx| state.set_location(Some(group), cx)));
            }
            Ok(())
        }
        ShotState::HostPanel => click_first_card(cx, handle, shot_state, ClickKind::Left),
        ShotState::ContextMenu => click_first_card(cx, handle, shot_state, ClickKind::Right),
        ShotState::SearchAddress => type_into(cx, handle, SEARCH, QUICK_CONNECT_TEXT),
        ShotState::SearchResults => type_into(cx, handle, SEARCH, SEARCH_WITH_RESULTS),
        ShotState::SearchNoResults => type_into(cx, handle, SEARCH, SEARCH_WITHOUT_RESULTS),
        ShotState::ThreeSelected => {
            let cards = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    visible_cards(window)
                })
                .map_err(|_| ScreenshotError::WindowClosed)?;
            if cards.len() < 3 {
                return Err(not_reached("fewer than 3 host cards on screen"));
            }
            for card in cards.into_iter().take(3) {
                cx.update_window(handle, |_, window, cx| {
                    click_with(window, card, Modifiers::secondary_key(), cx)
                })
                .map_err(|_| ScreenshotError::WindowClosed)?;
            }
            Ok(())
        }
        ShotState::ListView => click(cx, handle, VIEW_LIST),
        ShotState::NewHost => click(cx, handle, NEW_HOST),
        ShotState::QuickConnectToast => {
            type_into(cx, handle, SEARCH, QUICK_CONNECT_TEXT)?;
            press(cx, handle, "enter")
        }
        ShotState::TagFilter => {
            // The dropdown open with two tags checked (they also show as
            // chips next to the button); both are on the first demo host.
            check_example_tags(cx, handle).await?;
            open_tag_filter(cx, handle)
        }
        ShotState::TagChips => {
            // The same two tags, the dropdown closed, then the first match's
            // panel: the bar narrows and the chips collapse into "+2".
            check_example_tags(cx, handle).await?;
            press(cx, handle, "escape")?;
            settle(cx, handle).await?;
            click_first_card(cx, handle, shot_state, ClickKind::Left)
        }
        ShotState::ShowMore => {
            click_first_card(cx, handle, shot_state, ClickKind::Left)?;
            settle(cx, handle).await?;
            // "Show more" sits below the SSH fields: scroll to it, expand,
            // then scroll to the new bottom of the panel.
            scroll_to_bottom(cx, handle, HOST_PANEL_BODY)?;
            settle(cx, handle).await?;
            click(cx, handle, HOST_SHOW_MORE)?;
            settle(cx, handle).await?;
            scroll_to_bottom(cx, handle, HOST_PANEL_BODY)
        }
        ShotState::Empty | ShotState::FirstRun | ShotState::Unlock => {
            Err(not_reached("shown on a fresh profile, not on the demo"))
        }
    }
}

/// Checks that the window shows `shot_state` (the evidence a reviewer looks
/// for in the PNG), so a missed click fails the run instead of saving a
/// wrong picture.
pub fn check_state(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    shot_state: ShotState,
) -> Result<(), ScreenshotError> {
    let failed = |reason: &str| Err(ScreenshotError::state(shot_state, reason));
    let shell = cx.update(|cx| root.read(cx).shell().cloned());
    let facts = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let screen = shell
                .as_ref()
                .map(|shell| shell.read(cx).hosts_screen().clone());
            Facts {
                visible: |id: &str| visible(window, id),
                cards: visible_cards(window).len(),
                toasts: visible_with_prefix(window, TOAST_PREFIX),
                selected: root.read(cx).state().read(cx).selection().ids().len(),
                active_tags: screen
                    .as_ref()
                    .map_or(0, |screen| screen.read(cx).filter().tags().len()),
                list_layout: screen
                    .as_ref()
                    .is_some_and(|screen| screen.read(cx).layout() == HostsLayout::List),
                connect_enabled: screen
                    .as_ref()
                    .is_some_and(|screen| screen.read(cx).connect_enabled()),
                address_focused: window
                    .try_find(HOST_ADDRESS)
                    .and_then(|snapshot| snapshot.focused())
                    == Some(true),
                address_empty: window
                    .try_find(HOST_ADDRESS)
                    .and_then(|snapshot| snapshot.value().map(str::is_empty))
                    == Some(true),
            }
            .evaluate(shot_state)
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    match facts {
        Ok(()) => Ok(()),
        Err(reason) => failed(reason),
    }
}

/// What [`check_state`] reads from a frame.
struct Facts<V: Fn(&str) -> bool> {
    visible: V,
    cards: usize,
    toasts: usize,
    selected: usize,
    active_tags: usize,
    list_layout: bool,
    connect_enabled: bool,
    address_focused: bool,
    address_empty: bool,
}

impl<V: Fn(&str) -> bool> Facts<V> {
    fn evaluate(&self, state: ShotState) -> Result<(), &'static str> {
        let visible = &self.visible;
        let need = |ok: bool, reason: &'static str| if ok { Ok(()) } else { Err(reason) };
        match state {
            ShotState::Root => {
                need(visible(HOSTS_SCREEN) && self.cards > 0, "no host cards")?;
                need(!visible(HOST_PANEL), "a panel is open")
            }
            ShotState::Group => need(visible(BREADCRUMBS), "no breadcrumbs"),
            ShotState::HostPanel => need(visible(HOST_PANEL), "no host panel"),
            ShotState::ContextMenu => need(visible(POPUP_MENU), "no context menu"),
            ShotState::SearchAddress => need(self.connect_enabled, "Connect is not enabled"),
            ShotState::SearchResults => need(self.cards > 0, "no results"),
            ShotState::SearchNoResults => {
                need(self.cards == 0, "results shown")?;
                need(visible(HOSTS_NO_MATCHES), "no \"no matches\" text")
            }
            ShotState::ThreeSelected => need(self.selected == 3, "not 3 cards selected"),
            ShotState::Empty => need(visible(HOSTS_EMPTY), "no empty state"),
            ShotState::ListView => need(self.list_layout && self.cards > 0, "not the list view"),
            ShotState::NewHost => {
                need(visible(HOST_PANEL), "no host panel")?;
                need(self.address_empty, "Address is not empty")?;
                need(self.address_focused, "Address has no focus")
            }
            ShotState::QuickConnectToast => need(self.toasts > 0, "no toast"),
            ShotState::TagFilter => {
                need(visible(TAG_FILTER_MENU), "no tag filter dropdown")?;
                need(
                    self.active_tags == TAG_FILTER_EXAMPLE.len(),
                    "the example tags are not on",
                )
            }
            ShotState::TagChips => {
                need(visible(HOST_PANEL), "no host panel")?;
                need(
                    self.active_tags == TAG_FILTER_EXAMPLE.len(),
                    "the example tags are not on",
                )?;
                need(!visible(TAG_FILTER_MENU), "the tag filter dropdown is open")?;
                need(
                    visible(TAG_CHIP_MORE),
                    "the chips did not collapse into \"+N\"",
                )
            }
            ShotState::ShowMore => need(visible(HOST_MORE), "\"Show more\" is not expanded"),
            ShotState::FirstRun => need(visible(FIRST_RUN), "no first-run screen"),
            ShotState::Unlock => need(visible(UNLOCK), "no unlock screen"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClickKind {
    Left,
    Right,
}

/// Clicks the first host card on screen (top left of the Hosts section).
fn click_first_card(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot_state: ShotState,
    kind: ClickKind,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let Some(card) = visible_cards(window).into_iter().next() else {
            return Err(ScreenshotError::state(shot_state, "no host card on screen"));
        };
        match kind {
            ClickKind::Left => window.click(card, cx),
            ClickKind::Right => window.right_click(card, cx),
        }
        Ok(())
    })
    .map_err(|_| ScreenshotError::WindowClosed)?
}

fn click(cx: &mut AsyncApp, handle: AnyWindowHandle, id: &str) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        if !visible(window, id) {
            return Err(ScreenshotError::State(format!("{id} is not on screen")));
        }
        window.click(gpui::SharedString::from(id.to_owned()), cx);
        Ok(())
    })
    .map_err(|_| ScreenshotError::WindowClosed)?
}

/// Checks the example tags in the tag filter dropdown, one at a time.
async fn check_example_tags(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
) -> Result<(), ScreenshotError> {
    for tag in TAG_FILTER_EXAMPLE {
        open_tag_filter(cx, handle)?;
        settle(cx, handle).await?;
        click(cx, handle, &tag_option_id(tag))?;
        settle(cx, handle).await?;
    }
    Ok(())
}

/// Opens the tag filter dropdown unless it is open.
fn open_tag_filter(cx: &mut AsyncApp, handle: AnyWindowHandle) -> Result<(), ScreenshotError> {
    let open = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            visible(window, TAG_FILTER_MENU)
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    if open {
        Ok(())
    } else {
        click(cx, handle, TAG_FILTER)
    }
}

/// Scrolls the container `id` far past its end (it stops at the bottom).
fn scroll_to_bottom(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: &'static str,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(id, ScrollDelta::Pixels(point(px(0.), px(-20_000.))), cx);
        window.render_frame(cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

fn press(cx: &mut AsyncApp, handle: AnyWindowHandle, key: &str) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| window.press(key, cx))
        .map_err(|_| ScreenshotError::WindowClosed)
}

/// Replaces the text of the input `id` (click, select all, delete, type).
fn type_into(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: &'static str,
    text: &str,
) -> Result<(), ScreenshotError> {
    click(cx, handle, id)?;
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-a", cx);
        window.press("backspace", cx);
        if !text.is_empty() {
            window.input(text, cx);
        }
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Whether the element `id` is on screen.
fn visible(window: &Window, id: &str) -> bool {
    window
        .try_find(gpui::SharedString::from(id.to_owned()))
        .is_some_and(|snapshot| snapshot.visible())
}

/// The number of visible elements whose id is `prefix` and a number.
fn visible_with_prefix(window: &Window, prefix: &str) -> usize {
    gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|snapshot| snapshot.visible())
        .filter_map(|snapshot| snapshot.path().last().map(|id| id.to_string()))
        .filter(|id| {
            id.strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        })
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// The ids of the host cards on screen, top to bottom, then left to right.
fn visible_cards(window: &Window) -> Vec<gpui::SharedString> {
    let mut cards: Vec<(f32, f32, String)> = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|snapshot| snapshot.visible())
        .filter_map(|snapshot| {
            let id = snapshot.path().last()?.to_string();
            let uuid = id.strip_prefix(HOST_CARD_PREFIX)?;
            Uuid::parse_str(uuid).ok()?;
            let origin = snapshot.bounds().origin;
            Some((origin.y.as_f32(), origin.x.as_f32(), id))
        })
        .collect();
    cards.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    cards.dedup_by(|a, b| a.2 == b.2);
    cards.into_iter().map(|(_, _, id)| id.into()).collect()
}

/// Clicks the element `id` (a real pointer event sequence through GPUI's
/// dispatch) when it is on screen.
fn click_if_visible(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: gpui::SharedString,
) -> Result<bool, ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let visible = window
            .try_find(id.clone())
            .is_some_and(|snapshot| snapshot.visible());
        if visible {
            window.click(id, cx);
        }
        visible
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Clicks the element `id` with `modifiers` held (Ctrl+click selection,
/// which gpui-kit's `click` cannot send).
fn click_with(window: &mut Window, id: gpui::SharedString, modifiers: Modifiers, cx: &mut App) {
    window.render_frame(cx);
    let position = window.find(id).bounds().center();
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

/// Renders the current scene offscreen and writes it as a PNG.
fn capture(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    size: WindowSize,
    out_dir: &Path,
    file: &str,
) -> Result<Shot, ScreenshotError> {
    let path = out_dir.join(file);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let scale = window.scale_factor();
        let image = window
            .render_to_image()
            .map_err(|error| ScreenshotError::Render {
                file: file.to_owned(),
                message: format!("{error:#}"),
            })?;
        let (width, height) = (image.width(), image.height());
        image.save(&path).map_err(|error| ScreenshotError::Save {
            file: file.to_owned(),
            message: error.to_string(),
        })?;
        let want_width = (size.width as f32 * scale).round() as u32;
        let want_height = (size.height as f32 * scale).round() as u32;
        let off = |got: u32, want: u32| (i64::from(got) - i64::from(want)).abs() > SIZE_TOLERANCE;
        if off(width, want_width) || off(height, want_height) {
            return Err(ScreenshotError::Size {
                file: file.to_owned(),
                got_width: width,
                got_height: height,
                want_width,
                want_height,
            });
        }
        Ok(Shot {
            path: path.clone(),
            width,
            height,
        })
    })
    .map_err(|_| ScreenshotError::WindowClosed)?
}

#[cfg(test)]
mod tests;
