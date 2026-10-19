//! `--screenshots <out-dir> [--size WxH]` (cargo feature `screenshots`, owner's
//! step 4): one launch of the real app on demo data that saves PNG
//! screenshots of **our own window only** for each theme (dark, light) and
//! each state (root of the Hosts screen, inside a group, host panel open),
//! then quits.
//!
//! - Data: a fresh demo profile (300 hosts, 5 groups, see [`super::demo`])
//!   created inside `SHELLPP_DATA_DIR` when that variable is set (the
//!   scripts point it at a temporary folder), otherwise in the OS temporary
//!   directory. The profile is deleted at quit, like `--demo`.
//! - Capture: GPUI's own renderer draws the window's current scene into an
//!   offscreen texture and reads it back (`Window::render_to_image`, the
//!   DirectX renderer on Windows). Nothing is read from the screen, so other
//!   windows, the desktop or an overlapping window can never appear in a
//!   screenshot, and the window does not have to be fully on screen.
//! - Size: `WxH` is the window content size in logical pixels; the PNG has
//!   the device pixels of the window (`W * scale` x `H * scale`, the scale
//!   factor is printed). The minimum window size is raised to `WxH` so that
//!   Windows does not clamp a window larger than the display.
//! - Names: `<W>x<H>-<theme>-<state>.png`, e.g. `1920x1200-dark-root.png`.
//! - States are reached the way a user gets there: the Hosts screen at the
//!   top level; a click on the first group card (or the first group); a
//!   click on the first host card at the top level, which selects it and
//!   opens the host panel. The pointer is then moved out of the window so
//!   that no hover state is captured.
//! - Exit code 0 when every PNG was written with the expected size, 1 on any
//!   failure; a watchdog ends a run that hangs after [`WATCHDOG`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Entity, InputEvent as _, MouseMoveEvent,
    Pixels, Size, WindowBounds, point, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use uuid::Uuid;

use super::demo;
use crate::app::{self, RootView};
use crate::hosts::group_card_id;
use crate::profile::TempProfile;
use crate::state::AppState;
use crate::theme::{self, Appearance};

/// Default window size when `--size` is not given (the app's default).
pub const DEFAULT_SIZE: ShotSize = ShotSize {
    width: 1280,
    height: 800,
};
/// A run that has not finished after this long is ended with exit code 3.
pub const WATCHDOG: Duration = Duration::from_secs(180);
/// Settling time after a state change before the frame is captured (covers
/// the 180 ms panel motion of the design system and asset loading).
const SETTLE: Duration = Duration::from_millis(700);
/// Device-pixel tolerance when comparing the PNG with the requested size.
const SIZE_TOLERANCE: i64 = 2;

/// The window content size in logical pixels (`--size WxH`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShotSize {
    pub width: u32,
    pub height: u32,
}

impl ShotSize {
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

impl std::fmt::Display for ShotSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// The command-line options of a screenshot run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotOptions {
    pub out_dir: PathBuf,
    pub size: ShotSize,
}

/// The themes captured, in order.
pub const THEMES: [(Appearance, &str); 2] =
    [(Appearance::Dark, "dark"), (Appearance::Light, "light")];

/// The screen states captured for each theme, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotState {
    /// The Hosts screen at the top level (Groups and Hosts sections).
    Root,
    /// Inside the first group (breadcrumbs, its subgroups and hosts).
    Group,
    /// The first top-level host selected, its panel open.
    HostPanel,
}

impl ShotState {
    pub const ALL: [Self; 3] = [Self::Root, Self::Group, Self::HostPanel];

    pub fn name(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Group => "group",
            Self::HostPanel => "host-panel",
        }
    }
}

/// `<W>x<H>-<theme>-<state>.png`.
pub fn file_name(size: ShotSize, theme: &str, state: ShotState) -> String {
    format!("{}-{theme}-{}.png", size.label(), state.name())
}

/// Every file a run with `size` writes, in capture order.
pub fn expected_files(size: ShotSize) -> Vec<String> {
    THEMES
        .iter()
        .flat_map(|(_, theme)| {
            ShotState::ALL
                .iter()
                .map(move |state| file_name(size, theme, *state))
        })
        .collect()
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
    #[error("{0}: no element to reach this state")]
    State(&'static str),
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

/// The result of a run: every saved screenshot, or every failure.
type Outcome = Result<Vec<Shot>, Vec<ScreenshotError>>;

/// One saved screenshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// The demo profile of a screenshot run: inside `SHELLPP_DATA_DIR` when set
/// (a temporary folder chosen by the caller), else the OS temporary folder.
fn create_profile() -> Result<TempProfile, ScreenshotError> {
    match std::env::var_os(shellpp_core::ids::ENV_DATA_DIR) {
        Some(parent) if !parent.is_empty() => {
            let parent = PathBuf::from(parent);
            crate::dirs::create_private_dir(&parent).map_err(ScreenshotError::Profile)?;
            demo::create_profile_in(&parent).map_err(|error| match error {
                demo::DemoError::Profile(io) => ScreenshotError::Profile(io),
                other => ScreenshotError::Demo(other),
            })
        }
        _ => demo::create_profile().map_err(ScreenshotError::Demo),
    }
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
    let profile = create_profile().map_err(|error| vec![error])?;
    say(&format!("Demo profile: {}", profile.path().display()));
    start_watchdog(profile.path().to_path_buf());
    crate::logging::init(&profile.dirs().logs);
    let (opened, summary) = demo::open(profile).map_err(|error| vec![error.into()])?;
    tracing::info!(hosts = summary.hosts, "screenshot run");
    let first_group = summary.groups.first().copied();

    let outcome: Rc<RefCell<Option<Outcome>>> = Rc::new(RefCell::new(None));
    let outcome_in_app = outcome.clone();
    let size = options.size;

    gpui_kit::application()
        .with_assets(app::assets::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            app::root::bind_keys(cx);
            let state = cx.new(|cx| AppState::new(opened, cx));

            let mut window_options = app::main_window_options(cx);
            window_options.window_bounds = Some(WindowBounds::centered(size.logical(), cx));
            // Lifts Windows' maximum track size (the display size) to the
            // requested size, so a 1920x1200 window opens on a smaller display.
            window_options.window_min_size = Some(size.logical());
            let opened_window = app::open_window_with(state.clone(), window_options, cx);

            let quit_state = state.clone();
            cx.on_app_quit(move |cx| {
                quit_state.update(cx, |state, _| state.shutdown());
                async {}
            })
            .detach();

            let (handle, root) = match opened_window {
                Ok(window) => window,
                Err(_) => {
                    *outcome_in_app.borrow_mut() = Some(Err(vec![ScreenshotError::Window]));
                    cx.quit();
                    return;
                }
            };
            cx.activate(true);
            cx.spawn(async move |cx: &mut AsyncApp| {
                let result =
                    capture_all(cx, handle, &root, &state, first_group, size, &out_dir).await;
                *outcome_in_app.borrow_mut() = Some(result);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });

    let result = outcome.borrow_mut().take();
    result.unwrap_or_else(|| Err(vec![ScreenshotError::Incomplete]))
}

/// Takes every screenshot; continues after a failed one and reports all.
async fn capture_all(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    first_group: Option<Uuid>,
    size: ShotSize,
    out_dir: &Path,
) -> Outcome {
    let mut shots = Vec::new();
    let mut errors = Vec::new();

    // The first frames: layout, fonts, icons.
    settle(cx, handle).await.map_err(|error| vec![error])?;
    let scale = cx
        .update_window(handle, |_, window, _| {
            let viewport = window.viewport_size();
            if viewport != size.logical() {
                // Not the requested size (a platform clamp): ask once more.
                window.resize(size.logical());
            }
            window.scale_factor()
        })
        .map_err(|_| vec![ScreenshotError::WindowClosed])?;
    say(&format!("Window {size} at scale factor {scale}"));

    for (appearance, theme_name) in THEMES {
        cx.update_window(handle, |_, window, cx| {
            theme::set_appearance(appearance, Some(window), cx);
        })
        .map_err(|_| vec![ScreenshotError::WindowClosed])?;
        for shot_state in ShotState::ALL {
            let file = file_name(size, theme_name, shot_state);
            let reached = reach_state(cx, handle, root, state, first_group, shot_state).await;
            if let Err(error) = reached {
                errors.push(error);
                continue;
            }
            if let Err(error) = settle(cx, handle).await {
                errors.push(error);
                break;
            }
            match capture(cx, handle, size, out_dir, &file) {
                Ok(shot) => {
                    say(&format!(
                        "Saved {} ({}x{})",
                        shot.path.display(),
                        shot.width,
                        shot.height
                    ));
                    shots.push(shot);
                }
                Err(error) => errors.push(error),
            }
        }
    }
    if errors.is_empty() {
        Ok(shots)
    } else {
        Err(errors)
    }
}

/// Waits for the UI to settle and draws a fresh frame.
async fn settle(cx: &mut AsyncApp, handle: AnyWindowHandle) -> Result<(), ScreenshotError> {
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

/// Brings the window into `shot_state` the way a user would.
async fn reach_state(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    first_group: Option<Uuid>,
    shot_state: ShotState,
) -> Result<(), ScreenshotError> {
    // Every state starts at the top level with the panel closed.
    let shell = cx.update(|cx| root.read(cx).shell().cloned());
    let Some(shell) = shell else {
        return Err(ScreenshotError::State("the shell is not shown"));
    };
    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.close_panel(cx));
        state.update(cx, |state, cx| {
            state.clear_selection(cx);
            state.set_location(None, cx);
        });
    });
    settle(cx, handle).await?;
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
                return Err(ScreenshotError::State("group: no groups"));
            };
            let clicked = click_if_visible(cx, handle, group_card_id(group))?;
            if !clicked {
                // Not on screen: navigate the way the card would.
                cx.update(|cx| state.update(cx, |state, cx| state.set_location(Some(group), cx)));
            }
            Ok(())
        }
        ShotState::HostPanel => {
            // The first host card on screen (top left of the Hosts section).
            let first = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    first_visible(window, HOST_CARD_PREFIX)
                })
                .map_err(|_| ScreenshotError::WindowClosed)?;
            let Some(card) = first else {
                return Err(ScreenshotError::State("host-panel: no host card on screen"));
            };
            if click_if_visible(cx, handle, card.into())? {
                Ok(())
            } else {
                Err(ScreenshotError::State(
                    "host-panel: the host card is not on screen",
                ))
            }
        }
    }
}

/// The prefix of [`host_card_id`].
const HOST_CARD_PREFIX: &str = "host-card-";

/// The id of the top-most, then left-most visible element whose id starts
/// with `prefix`.
fn first_visible(window: &gpui::Window, prefix: &str) -> Option<String> {
    gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|snapshot| snapshot.visible())
        .filter_map(|snapshot| {
            let id = snapshot.path().last()?.to_string();
            let origin = snapshot.bounds().origin;
            id.starts_with(prefix).then_some((origin.y, origin.x, id))
        })
        .min_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|(_, _, id)| id)
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

/// Renders the current scene offscreen and writes it as a PNG.
fn capture(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    size: ShotSize,
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
mod tests {
    use super::*;
    use crate::hosts::host_card_id;

    #[test]
    fn sizes_parse_and_validate() {
        assert_eq!(
            ShotSize::parse("1920x1200"),
            Some(ShotSize {
                width: 1920,
                height: 1200
            })
        );
        assert_eq!(ShotSize::parse(" 1280X800 "), Some(DEFAULT_SIZE));
        for bad in [
            "",
            "1920",
            "x800",
            "1920x",
            "axb",
            "100x100",
            "1920x99999",
            "-1x800",
        ] {
            assert_eq!(ShotSize::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn files_cover_both_themes_and_three_states() {
        let size = ShotSize::parse("1920x1200").unwrap();
        assert_eq!(
            expected_files(size),
            [
                "1920x1200-dark-root.png",
                "1920x1200-dark-group.png",
                "1920x1200-dark-host-panel.png",
                "1920x1200-light-root.png",
                "1920x1200-light-group.png",
                "1920x1200-light-host-panel.png",
            ]
        );
    }

    #[test]
    fn host_cards_are_found_by_their_id_prefix() {
        let id = Uuid::now_v7();
        assert_eq!(host_card_id(id).as_ref(), format!("{HOST_CARD_PREFIX}{id}"));
    }

    /// The run's data lives inside `SHELLPP_DATA_DIR` when it is set.
    #[test]
    fn demo_profiles_can_live_in_a_given_folder() {
        let parent = TempProfile::create("shellpp-test-shots").unwrap();
        let profile = demo::create_profile_in(parent.path()).unwrap();
        assert!(profile.path().starts_with(parent.path()));
        let path = profile.path().to_path_buf();
        drop(profile);
        assert!(!path.exists());
    }
}
