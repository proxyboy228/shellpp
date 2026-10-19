//! App commands: `app_info`, `state_snapshot`, `window_open`, `activity`,
//! `app_exit`, `window_close`, `external_open_url`.
//!
//! Exit and window close confirmation are decided here (stage 2 plan,
//! decision 17; UI 4.6; docs/architecture.md, section 3.6): `app_exit` and
//! `window_close` answer `needs_confirmation` while sessions are active (in
//! the app, in the window) unless the call says `confirmed`.

use serde::{Deserialize, Serialize};
use shellpp_core::error::ValidationReason;
use shellpp_core::ids;
use shellpp_core::service::dto::{
    AppInfo, CoreSnapshot, ExitInput, ExitResult, SessionInfo, WindowIntent, WindowOpenInput,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use super::{run_blocking, with_core, with_guard};
use crate::error::CmdError;
use crate::events::deliver_links;
use crate::platform::current_os;
use crate::state::AppState;
use crate::windows;

/// When this process started, as `state_snapshot.launched_at` reports it.
/// Initialized at the start of [`crate::run`].
static LAUNCHED_AT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The start time of this app process (RFC 3339 UTC, milliseconds).
pub fn launched_at() -> &'static str {
    LAUNCHED_AT
        .get_or_init(|| shellpp_core::model::time::format_timestamp(std::time::SystemTime::now()))
}

/// `state_snapshot`: the core snapshot plus the calling window's label, its
/// sessions and, once, the intent it was opened with (stage 2 plan, 5.1).
#[derive(Debug, Serialize)]
pub struct StateSnapshot {
    #[serde(flatten)]
    pub core: CoreSnapshot,
    pub window_label: String,
    /// When this app process started (RFC 3339 UTC): a `session.last` saved
    /// before it is the previous launch's (UI 6 Restore).
    pub launched_at: String,
    /// Sessions owned by the calling window (S5: from the session manager).
    pub sessions: Vec<SessionInfo>,
    /// The intent of a window opened with `window_open({intent})`, delivered once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<WindowIntent>,
}

/// `window_open`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WindowOpenResult {
    pub label: String,
}

/// Where a link to open comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkSource {
    /// A web link or an OSC 8 link in terminal output: `http:` is allowed too
    /// (docs/security.md, sections 12 and 13.3).
    Terminal,
}

/// Input of `external_open_url`.
#[derive(Clone, Deserialize)]
pub struct ExternalOpenInput {
    pub url: String,
    #[serde(default)]
    pub source: Option<LinkSource>,
}

impl std::fmt::Debug for ExternalOpenInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Terminal links come from remote output: the URL is never logged.
        f.debug_struct("ExternalOpenInput")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// Longest URL `external_open_url` accepts.
pub const MAX_URL_LEN: usize = 2048;

/// `https` only (`http` too for terminal links), no credentials, at most
/// 2048 characters (docs/security.md, section 13.3). Returns the normalized URL.
pub fn validate_external_url(text: &str, source: Option<LinkSource>) -> Result<String, CmdError> {
    let refuse = || CmdError::validation("url", ValidationReason::NotAllowed);
    if text.is_empty() || text.len() > MAX_URL_LEN {
        return Err(refuse());
    }
    let url = url::Url::parse(text).map_err(|_| refuse())?;
    let scheme_allowed = match url.scheme() {
        "https" => true,
        "http" => source == Some(LinkSource::Terminal),
        _ => false,
    };
    if !scheme_allowed
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
    let sessions = state
        .sessions
        .as_ref()
        .map(|sessions| sessions.sessions_of(&window_label))
        .unwrap_or_default();
    // The intent of `window_open({intent})`, delivered once.
    let intent = state.windows.take_intent(&window_label);
    Ok(StateSnapshot {
        core,
        window_label,
        launched_at: launched_at().to_owned(),
        sessions,
        intent,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn window_open(
    app: AppHandle,
    state: State<'_, AppState>,
    input: Option<WindowOpenInput>,
) -> Result<WindowOpenResult, CmdError> {
    let intent = input.and_then(|input| input.intent);
    // Windows are created on the async command thread, not the event loop
    // (a synchronous command would deadlock on Windows).
    let webview_dir = state.dirs.as_ref().and_then(|dirs| dirs.webview.as_deref());
    let label = windows::open_window(&app, &state.windows, webview_dir, intent).map_err(|_| {
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
pub async fn app_exit(
    app: AppHandle,
    state: State<'_, AppState>,
    input: Option<ExitInput>,
) -> Result<ExitResult, CmdError> {
    let confirmed = input.is_some_and(|input| input.confirmed);
    let active_sessions = state
        .sessions
        .as_ref()
        .map_or(0, |sessions| sessions.active_count());
    if active_sessions > 0 && !confirmed {
        return Ok(ExitResult {
            needs_confirmation: true,
            active_sessions,
        });
    }
    tracing::info!(active_sessions, "exit requested");
    let handle = app.clone();
    // `shutdown` closes the sessions and blocks on the core, the clipboard
    // and the file system.
    run_blocking("app_exit", move || {
        handle.state::<AppState>().shutdown();
        Ok(())
    })
    .await?;
    app.exit(0);
    Ok(ExitResult {
        needs_confirmation: false,
        active_sessions,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn window_close(
    state: State<'_, AppState>,
    window: WebviewWindow,
    input: Option<ExitInput>,
) -> Result<ExitResult, CmdError> {
    let label = window.label().to_owned();
    let confirmed = input.is_some_and(|input| input.confirmed);
    let sessions = state.sessions.clone();
    let active_sessions = sessions
        .as_ref()
        .map_or(0, |sessions| sessions.active_count_of(&label));
    if active_sessions > 0 && !confirmed {
        return Ok(ExitResult {
            needs_confirmation: true,
            active_sessions,
        });
    }
    if let Some(sessions) = sessions {
        let closing = label.clone();
        // Connection log entries are ended before the window goes away
        // (closing the last window exits the app).
        run_blocking("window_close", move || {
            let logs = sessions.close_window(&closing);
            sessions.end_logs_now(&logs);
            Ok(())
        })
        .await?;
    }
    tracing::info!(label = %label, active_sessions, "window closed");
    window.destroy().map_err(|_| {
        tracing::error!("window could not be closed");
        CmdError::internal()
    })?;
    Ok(ExitResult {
        needs_confirmation: false,
        active_sessions,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn external_open_url(
    state: State<'_, AppState>,
    input: ExternalOpenInput,
) -> Result<(), CmdError> {
    let launcher = state.platform()?.launcher.clone();
    // Lock guard first, then the argument check.
    with_guard(&state, "external_open_url", move |_core| {
        let url = validate_external_url(&input.url, input.source)?;
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
        refused_from(text, None)
    }

    fn refused_from(text: &str, source: Option<LinkSource>) -> bool {
        validate_external_url(text, source)
            == Err(CmdError::validation("url", ValidationReason::NotAllowed))
    }

    #[test]
    fn accepts_https_links() {
        assert_eq!(
            validate_external_url("https://github.com/proxyboy228/shellpp/issues", None).unwrap(),
            "https://github.com/proxyboy228/shellpp/issues"
        );
        for url in [
            ids::DOCUMENTATION_URL,
            ids::ISSUES_URL,
            ids::CHANGELOG_URL,
            ids::REPOSITORY_URL,
        ] {
            assert!(validate_external_url(url, None).is_ok(), "{url}");
        }
    }

    #[test]
    fn terminal_links_may_use_http() {
        let terminal = Some(LinkSource::Terminal);
        assert_eq!(
            validate_external_url("http://example.com/docs", terminal).unwrap(),
            "http://example.com/docs"
        );
        assert!(validate_external_url("https://example.com/docs", terminal).is_ok());
        assert!(refused("http://example.com/docs"));
        for url in [
            "ftp://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://user:pass@example.com/",
            "shellpp://host/0199f6a1-b2c3-7d4e-8f00-112233445566",
            "data:text/html,x",
        ] {
            assert!(refused_from(url, terminal), "{url}");
        }
        let input: ExternalOpenInput = serde_json::from_value(serde_json::json!({
            "url": "http://example.com/secret-path",
            "source": "terminal"
        }))
        .unwrap();
        assert_eq!(input.source, terminal);
        assert!(!format!("{input:?}").contains("secret-path"));
        assert!(
            serde_json::from_value::<ExternalOpenInput>(
                serde_json::json!({"url": "https://example.com", "source": "web"})
            )
            .is_err()
        );
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
        assert!(validate_external_url(&edge, None).is_ok());
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
