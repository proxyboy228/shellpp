//! App commands: `app_info`, `state_snapshot`, `window_open`, `activity`,
//! `app_exit`, `external_open_url`.

use serde::{Deserialize, Serialize};
use shellpp_core::error::ValidationReason;
use shellpp_core::ids;
use shellpp_core::service::dto::{AppInfo, CoreSnapshot};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use super::{run_blocking, with_core, with_guard};
use crate::error::CmdError;
use crate::events::deliver_links;
use crate::platform::current_os;
use crate::state::AppState;
use crate::windows;

/// `state_snapshot`: the core snapshot plus the calling window's label.
#[derive(Debug, Serialize)]
pub struct StateSnapshot {
    #[serde(flatten)]
    pub core: CoreSnapshot,
    pub window_label: String,
}

/// `window_open`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WindowOpenResult {
    pub label: String,
}

/// Input of `external_open_url`.
#[derive(Clone, Debug, Deserialize)]
pub struct ExternalOpenInput {
    pub url: String,
}

/// Longest URL `external_open_url` accepts.
pub const MAX_URL_LEN: usize = 2048;

/// `https` only, no credentials, at most 2048 characters
/// (docs/security.md, section 13.3). Returns the normalized URL.
pub fn validate_external_url(text: &str) -> Result<String, CmdError> {
    let refuse = || CmdError::validation("url", ValidationReason::NotAllowed);
    if text.is_empty() || text.len() > MAX_URL_LEN {
        return Err(refuse());
    }
    let url = url::Url::parse(text).map_err(|_| refuse())?;
    if url.scheme() != "https"
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(refuse());
    }
    let normalized = url.to_string();
    if normalized.len() > MAX_URL_LEN {
        return Err(refuse());
    }
    Ok(normalized)
}

/// The `app_info` value of this build.
pub fn app_info_value() -> AppInfo {
    AppInfo {
        name: ids::DISPLAY_NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        os: current_os(),
        arch: std::env::consts::ARCH.to_owned(),
        debug: cfg!(debug_assertions),
        e2e: cfg!(feature = "e2e-mocks"),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn app_info() -> Result<AppInfo, CmdError> {
    Ok(app_info_value())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn state_snapshot(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<StateSnapshot, CmdError> {
    let window_label = window.label().to_owned();
    let core = with_core(&state, "state_snapshot", |core| Ok(core.snapshot())).await?;
    // The window has subscribed to its window events by now (snapshot
    // protocol): it may receive links that waited for a window.
    state.windows.mark_ready(&window_label);
    let ready = state.links.take_ready();
    if !ready.is_empty() {
        deliver_links(&app, &state.links, &state.windows, ready);
    }
    Ok(StateSnapshot { core, window_label })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn window_open(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WindowOpenResult, CmdError> {
    // Windows are created on the async command thread, not the event loop
    // (a synchronous command would deadlock on Windows).
    let webview_dir = state.dirs.as_ref().and_then(|dirs| dirs.webview.as_deref());
    let label = windows::open_window(&app, &state.windows, webview_dir).map_err(|_| {
        // Webview errors can carry paths; nothing of them goes to the log.
        tracing::error!("window could not be opened");
        CmdError::internal()
    })?;
    Ok(WindowOpenResult { label })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn activity(state: State<'_, AppState>) -> Result<(), CmdError> {
    match state.core() {
        Ok(_) => {
            with_core(&state, "activity", |core| {
                core.activity();
                Ok(())
            })
            .await
        }
        // No core, no auto-lock: input has nothing to reset.
        Err(_) => Ok(()),
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn app_exit(app: AppHandle) -> Result<(), CmdError> {
    tracing::info!("exit requested");
    let handle = app.clone();
    // `shutdown` blocks on the clipboard and the file system.
    run_blocking("app_exit", move || {
        handle.state::<AppState>().shutdown();
        Ok(())
    })
    .await?;
    app.exit(0);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn external_open_url(
    state: State<'_, AppState>,
    input: ExternalOpenInput,
) -> Result<(), CmdError> {
    let launcher = state.platform()?.launcher.clone();
    // Lock guard first, then the argument check.
    with_guard(&state, "external_open_url", move |_core| {
        let url = validate_external_url(&input.url)?;
        launcher.open_url(&url).map_err(|_| CmdError::unavailable())
    })
    .await?;
    tracing::info!("external link opened");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(text: &str) -> bool {
        validate_external_url(text)
            == Err(CmdError::validation("url", ValidationReason::NotAllowed))
    }

    #[test]
    fn accepts_https_links() {
        assert_eq!(
            validate_external_url("https://github.com/proxyboy228/shellpp/issues").unwrap(),
            "https://github.com/proxyboy228/shellpp/issues"
        );
        for url in [
            ids::DOCUMENTATION_URL,
            ids::ISSUES_URL,
            ids::CHANGELOG_URL,
            ids::REPOSITORY_URL,
        ] {
            assert!(validate_external_url(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn refuses_everything_else() {
        for url in [
            "",
            "http://example.com/",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "shellpp://host/0199f6a1-b2c3-7d4e-8f00-112233445566",
            "ftp://example.com/",
            "https://user@example.com/",
            "https://user:pass@example.com/",
            "https://",
            "not a url",
            "data:text/html,x",
        ] {
            assert!(refused(url), "{url}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(refused(&long));
        let edge = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN - 20));
        assert_eq!(edge.len(), MAX_URL_LEN);
        assert!(validate_external_url(&edge).is_ok());
    }

    #[test]
    fn app_info_of_this_build() {
        let info = app_info_value();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["name"], "Shell++");
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            json["os"],
            if cfg!(windows) {
                "windows"
            } else if cfg!(target_os = "macos") {
                "macos"
            } else {
                "linux"
            }
        );
        assert_eq!(json["arch"], std::env::consts::ARCH);
        assert_eq!(json["debug"], cfg!(debug_assertions));
        assert_eq!(json["e2e"], cfg!(feature = "e2e-mocks"));
    }
}
