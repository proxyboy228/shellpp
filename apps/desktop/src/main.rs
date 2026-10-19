// Shell++ desktop entry point. Hides the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `shellpp [--print-data-dir] [--demo] [--theme auto|light|dark] [--help]`
//!
//! Startup (docs/architecture.md, section 4.1):
//! 1. Arguments.
//! 2. Directories: `SHELLPP_DATA_DIR`, else the permanent profile
//!    (`%LOCALAPPDATA%\shellpp` on Windows, see `dirs`). With `SHELLPP_TEST=1`
//!    a missing `SHELLPP_DATA_DIR` is an error. `--demo` uses a temporary
//!    profile instead.
//! 3. Logging into `<logs>/shellpp.log` (no secrets, addresses or paths);
//!    process hardening.
//! 4. The core: opened here; the window shows the lock screens.
//! 5. The GPUI app (`shellpp_desktop::app::run`).

use std::ffi::OsString;
use std::process::ExitCode;
use std::sync::Arc;

use shellpp_core::error::CoreError;
use shellpp_core::store::StoreError;
use shellpp_core::{Core, CoreConfig};
use shellpp_desktop::app::{self, Launch};
use shellpp_desktop::dirs::{self, AppDirs};
use shellpp_desktop::platform::{self, events::event_channel};
use shellpp_desktop::state::OpenedCore;
use shellpp_desktop::theme::Appearance;
use shellpp_desktop::{hardening, i18n, logging};

const USAGE: &str = "\
Usage: shellpp [options]

Options:
  --print-data-dir   print the data directory this run would use and exit
                     (creates nothing)
  --help             show this help
Developer options (builds with the dev-tools feature):
  --demo             run on a temporary profile with generated data
                     (300 hosts, 5 groups); the profile is deleted on exit
  --theme <mode>     interface theme: auto (default), light or dark
  --screenshots <dir>
                     (feature screenshots) save PNGs of the window on demo
                     data, dark and light, three states, then quit
  --size <WxH>       window size for --screenshots (default 1280x800)

Environment:
  SHELLPP_DATA_DIR   data directory to use instead of the permanent profile
  SHELLPP_TEST=1     test run: refuse the permanent profile
";

/// The parsed command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Args {
    print_data_dir: bool,
    help: bool,
    demo: bool,
    appearance: Appearance,
    /// `--screenshots <dir>` (feature `screenshots`).
    screenshots: Option<std::path::PathBuf>,
    /// `--size <WxH>` for `--screenshots`, checked by the screenshot mode.
    size: Option<String>,
}

/// Why the command line was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ArgsError {
    Unknown(String),
    MissingValue(&'static str),
    BadTheme(String),
    NeedsDevTools(&'static str),
    NeedsScreenshots(&'static str),
}

impl std::fmt::Display for ArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(arg) => write!(f, "unknown option {arg}"),
            Self::MissingValue(arg) => write!(f, "{arg} needs a value"),
            Self::BadTheme(value) => write!(f, "--theme takes auto, light or dark, not {value}"),
            Self::NeedsDevTools(arg) => {
                write!(f, "{arg} needs a build with the dev-tools feature")
            }
            Self::NeedsScreenshots(arg) => {
                write!(f, "{arg} needs a build with the screenshots feature")
            }
        }
    }
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, ArgsError> {
    let dev_tools = cfg!(feature = "dev-tools");
    let screenshots = cfg!(feature = "screenshots");
    let mut parsed = Args::default();
    let mut args = args
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--print-data-dir" => parsed.print_data_dir = true,
            "--help" | "-h" => parsed.help = true,
            "--demo" if dev_tools => parsed.demo = true,
            "--demo" => return Err(ArgsError::NeedsDevTools("--demo")),
            "--theme" if dev_tools => {
                let value = args.next().ok_or(ArgsError::MissingValue("--theme"))?;
                parsed.appearance = Appearance::parse(&value).ok_or(ArgsError::BadTheme(value))?;
            }
            "--theme" => return Err(ArgsError::NeedsDevTools("--theme")),
            "--screenshots" if screenshots => {
                let value = args
                    .next()
                    .ok_or(ArgsError::MissingValue("--screenshots"))?;
                parsed.screenshots = Some(value.into());
            }
            "--size" if screenshots => {
                parsed.size = Some(args.next().ok_or(ArgsError::MissingValue("--size"))?);
            }
            "--screenshots" | "--size" => {
                return Err(ArgsError::NeedsScreenshots(if arg == "--size" {
                    "--size"
                } else {
                    "--screenshots"
                }));
            }
            // A `shellpp://` link (UI deep links) arrives in a later stage.
            other => return Err(ArgsError::Unknown(other.to_owned())),
        }
    }
    Ok(parsed)
}

// The console is the only place these lines can go before logging exists.
#[allow(clippy::print_stderr)]
fn fail(message: &str) -> ExitCode {
    eprintln!("shellpp: {message}");
    ExitCode::from(2)
}

#[allow(clippy::print_stdout)]
fn print_line(line: &str) {
    println!("{line}");
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            let _ = fail(&error.to_string());
            return fail(USAGE.trim_end());
        }
    };
    if args.help {
        print_line(USAGE.trim_end());
        return ExitCode::SUCCESS;
    }
    if let Some(out_dir) = args.screenshots {
        return run_screenshots(out_dir, args.size);
    }
    if args.demo {
        return run_demo(args.appearance);
    }

    let dirs = match dirs::resolve_current() {
        Ok(dirs) => dirs,
        Err(error) => return fail(&error.to_string()),
    };
    if args.print_data_dir {
        print_line(&dirs.data.display().to_string());
        return ExitCode::SUCCESS;
    }
    let opened = match dirs.create() {
        Ok(()) => {
            logging::init(&dirs.logs);
            log_start(false);
            empty_tmp(&dirs);
            open_core(dirs)
        }
        Err(_) => {
            logging::init_without_file();
            log_start(false);
            tracing::error!("data directory unavailable");
            Err(i18n::startup::OPEN_FAILED.to_owned())
        }
    };
    app::run(Launch {
        opened,
        appearance: args.appearance,
    });
    ExitCode::SUCCESS
}

fn log_start(demo: bool) {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        debug = cfg!(debug_assertions),
        demo,
        "starting"
    );
    let hardening = hardening::apply();
    tracing::info!(
        core_dumps_disabled = hardening.core_dumps_disabled,
        not_dumpable = hardening.not_dumpable,
        "hardening"
    );
}

fn empty_tmp(dirs: &AppDirs) {
    let failed = dirs.empty_tmp();
    if failed > 0 {
        tracing::warn!(failed, "tmp entries could not be removed");
    }
}

/// Opens the core of a normal run (OS keychain, OS clipboard), or the
/// message the window shows instead of the app.
fn open_core(dirs: AppDirs) -> Result<OpenedCore, String> {
    let (sink, events) = event_channel();
    let (platform, extras) = platform::desktop(sink);
    let config = CoreConfig {
        data_dir: dirs.data.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    match Core::open(config, platform) {
        Ok(core) => {
            tracing::info!(state = ?core.lock_status().state, "core opened");
            try_device_unlock(&core);
            Ok(OpenedCore {
                core: Arc::new(core),
                events,
                extras,
                dirs: Some(dirs),
                temp_profile: None,
            })
        }
        Err(error) => {
            tracing::error!(code = error.code(), "core did not open");
            Err(startup_message(&error).to_owned())
        }
    }
}

fn startup_message(error: &CoreError) -> &'static str {
    match error {
        CoreError::Store(StoreError::InUse) => i18n::startup::DATA_DIR_IN_USE,
        CoreError::NewerSchema { .. } => i18n::startup::NEWER_SCHEMA,
        _ => i18n::startup::OPEN_FAILED,
    }
}

/// "Remember on this device" (UI 13.4): unlock with the device key before
/// the window exists, so it never flashes the unlock screen. Runs on a
/// helper thread (the Linux Secret Service client must not run on the main
/// thread).
fn try_device_unlock(core: &Core) {
    if core.lock_status().state != shellpp_core::service::dto::LockState::Locked {
        return;
    }
    let result = std::thread::scope(|scope| scope.spawn(|| core.try_device_unlock()).join());
    match result {
        Ok(Ok(true)) => tracing::info!("unlocked with the device key"),
        Ok(Ok(false)) => tracing::info!("device key not used"),
        Ok(Err(error)) => tracing::warn!(code = error.code(), "device unlock failed"),
        Err(_) => tracing::error!("device unlock panicked"),
    }
}

#[cfg(feature = "dev-tools")]
fn run_demo(appearance: Appearance) -> ExitCode {
    use shellpp_desktop::dev::demo;

    let profile = match demo::create_profile() {
        Ok(profile) => profile,
        Err(error) => return fail(&error.to_string()),
    };
    print_line(&format!("Demo profile: {}", profile.path().display()));
    let dirs = profile.dirs();
    logging::init(&dirs.logs);
    log_start(true);
    let opened = match demo::open(profile) {
        Ok((opened, summary)) => {
            print_line(&format!(
                "Generated {} hosts and {} groups; unlocked automatically (generated master password).",
                summary.hosts,
                summary.groups.len()
            ));
            Ok(opened)
        }
        Err(error) => {
            tracing::error!("demo setup failed");
            return fail(&error.to_string());
        }
    };
    app::run(Launch { opened, appearance });
    // Reached where the platform returns from its run loop (Windows, Linux);
    // the profile was already deleted at quit.
    ExitCode::SUCCESS
}

#[cfg(not(feature = "dev-tools"))]
fn run_demo(_: Appearance) -> ExitCode {
    fail("--demo needs a build with the dev-tools feature")
}

/// `--screenshots <dir> [--size WxH]`: see `shellpp_desktop::dev::screenshots`.
#[cfg(feature = "screenshots")]
fn run_screenshots(out_dir: std::path::PathBuf, size: Option<String>) -> ExitCode {
    use shellpp_desktop::dev::screenshots::{self, ScreenshotOptions, ShotSize};

    let size = match size {
        None => screenshots::DEFAULT_SIZE,
        Some(value) => match ShotSize::parse(&value) {
            Some(size) => size,
            None => return fail(&format!("--size takes WxH (e.g. 1920x1200), not {value}")),
        },
    };
    screenshots::run(ScreenshotOptions { out_dir, size })
}

#[cfg(not(feature = "screenshots"))]
fn run_screenshots(_: std::path::PathBuf, _: Option<String>) -> ExitCode {
    fail("--screenshots needs a build with the screenshots feature")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, ArgsError> {
        parse_args(args.iter().map(OsString::from))
    }

    #[test]
    fn plain_run_and_diagnostics() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        assert!(parse(&["--print-data-dir"]).unwrap().print_data_dir);
        assert!(parse(&["--help"]).unwrap().help);
        assert_eq!(
            parse(&["--bogus"]),
            Err(ArgsError::Unknown("--bogus".to_owned()))
        );
    }

    #[test]
    fn developer_flags_follow_the_feature() {
        if cfg!(feature = "dev-tools") {
            let args = parse(&["--demo", "--theme", "light"]).unwrap();
            assert!(args.demo);
            assert_eq!(args.appearance, Appearance::Light);
            assert_eq!(parse(&["--theme"]), Err(ArgsError::MissingValue("--theme")));
            assert_eq!(
                parse(&["--theme", "blue"]),
                Err(ArgsError::BadTheme("blue".to_owned()))
            );
        } else {
            assert_eq!(parse(&["--demo"]), Err(ArgsError::NeedsDevTools("--demo")));
            assert_eq!(
                parse(&["--theme", "dark"]),
                Err(ArgsError::NeedsDevTools("--theme"))
            );
        }
    }

    #[test]
    fn screenshot_flags_follow_their_feature() {
        if cfg!(feature = "screenshots") {
            let args = parse(&["--screenshots", "out", "--size", "1920x1200"]).unwrap();
            assert_eq!(
                args.screenshots.as_deref(),
                Some(std::path::Path::new("out"))
            );
            assert_eq!(args.size.as_deref(), Some("1920x1200"));
            assert_eq!(
                parse(&["--screenshots"]),
                Err(ArgsError::MissingValue("--screenshots"))
            );
            assert_eq!(parse(&["--size"]), Err(ArgsError::MissingValue("--size")));
        } else {
            assert_eq!(
                parse(&["--screenshots", "out"]),
                Err(ArgsError::NeedsScreenshots("--screenshots"))
            );
            assert_eq!(
                parse(&["--size", "1x1"]),
                Err(ArgsError::NeedsScreenshots("--size"))
            );
        }
    }
}
