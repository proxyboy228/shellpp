//! App windows (docs/architecture.md, section 3.6): all windows are equal and
//! labelled `w-<n>`, starting with `w-1`. Every window is built from the `w-1`
//! entry of `tauri.conf.json` (`"create": false`) with a new label, loads the
//! same bundled frontend and may navigate only within the app's own origin
//! (docs/security.md, section 13.3).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Serialize;
use shellpp_core::service::dto::WindowIntent;
use tauri::webview::NewWindowResponse;
use tauri::{
    AppHandle, Emitter, EventTarget, Manager, Runtime, WebviewWindow, WebviewWindowBuilder,
};
use url::Url;

/// Label prefix of app windows.
pub const LABEL_PREFIX: &str = "w-";
/// Label of the first app window; it also names the window configuration.
pub const FIRST_WINDOW_LABEL: &str = "w-1";

/// Window labels, focus tracking and readiness.
///
/// A window is *ready* once its frontend has called `state_snapshot`: by then
/// it has subscribed to its window events (`link:open`, `menu:action`), so
/// window-targeted events go only to ready windows and wait otherwise.
#[derive(Debug)]
pub struct WindowRegistry {
    next: AtomicU32,
    inner: Mutex<RegistryInner>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    focused: Option<String>,
    ready: BTreeSet<String>,
    /// What a window opened with `window_open({intent})` does first; taken
    /// by its first `state_snapshot` (stage 2 plan, section 5.1).
    intents: BTreeMap<String, WindowIntent>,
}

impl Default for WindowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowRegistry {
    pub fn new() -> Self {
        Self {
            next: AtomicU32::new(1),
            inner: Mutex::new(RegistryInner::default()),
        }
    }

    /// The next free label: `w-1`, `w-2`, ... (labels are never reused).
    pub fn next_label(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        format!("{LABEL_PREFIX}{n}")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RegistryInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A window gained focus.
    pub fn focused(&self, label: &str) {
        self.lock().focused = Some(label.to_owned());
    }

    /// A window's frontend took its first snapshot (see the type docs).
    pub fn mark_ready(&self, label: &str) {
        self.lock().ready.insert(label.to_owned());
    }

    /// A window was destroyed.
    pub fn destroyed(&self, label: &str) {
        let mut inner = self.lock();
        if inner.focused.as_deref() == Some(label) {
            inner.focused = None;
        }
        inner.ready.remove(label);
        inner.intents.remove(label);
    }

    /// Stores the intent of a window about to open.
    pub fn set_intent(&self, label: &str, intent: WindowIntent) {
        self.lock().intents.insert(label.to_owned(), intent);
    }

    /// The intent of a window, once.
    pub fn take_intent(&self, label: &str) -> Option<WindowIntent> {
        self.lock().intents.remove(label)
    }

    /// The label to target among the existing windows (see [`pick_target`]).
    fn target_label(&self, existing: &[String]) -> Option<String> {
        let inner = self.lock();
        pick_target(inner.focused.as_deref(), &inner.ready, existing)
    }

    /// The focused or last focused ready window, else the ready window with
    /// the lowest number.
    pub fn target<R: Runtime>(&self, app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
        let windows = app.webview_windows();
        let existing: Vec<String> = windows.keys().cloned().collect();
        let label = self.target_label(&existing)?;
        windows.get(&label).cloned()
    }

    /// Emits a window-targeted event to [`WindowRegistry::target`]. Returns
    /// false when no ready window exists (the caller keeps the event).
    pub fn emit_to_target<R: Runtime, S: Serialize + Clone>(
        &self,
        app: &AppHandle<R>,
        event: &str,
        payload: S,
    ) -> bool {
        let Some(window) = self.target(app) else {
            return false;
        };
        let label = window.label().to_owned();
        if app
            .emit_to(EventTarget::webview_window(label), event, payload)
            .is_err()
        {
            tracing::warn!(event, "window event not delivered");
        }
        true
    }

    /// The window to bring to the front: the focused or last focused one,
    /// else the app window with the lowest number (readiness does not matter).
    pub fn front_window<R: Runtime>(&self, app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
        let windows = app.webview_windows();
        let focused = self.lock().focused.clone();
        if let Some(label) = focused
            && let Some(window) = windows.get(&label)
        {
            return Some(window.clone());
        }
        let mut labels: Vec<&String> = windows
            .keys()
            .filter(|label| label.starts_with(LABEL_PREFIX))
            .collect();
        labels.sort_by_key(|label| label_number(label));
        labels
            .first()
            .and_then(|label| windows.get(*label).cloned())
    }
}

/// Chooses the window for a window-targeted event: the focused (or last
/// focused) window if it exists and is ready, else the existing ready window
/// with the lowest number.
pub fn pick_target(
    focused: Option<&str>,
    ready: &BTreeSet<String>,
    existing: &[String],
) -> Option<String> {
    let usable = |label: &str| ready.contains(label) && existing.iter().any(|l| l == label);
    if let Some(label) = focused
        && usable(label)
    {
        return Some(label.to_owned());
    }
    let mut candidates: Vec<&String> = ready.iter().filter(|label| usable(label)).collect();
    candidates.sort_by_key(|label| label_number(label));
    candidates.first().map(|label| (*label).clone())
}

fn label_number(label: &str) -> u32 {
    label
        .strip_prefix(LABEL_PREFIX)
        .and_then(|n| n.parse().ok())
        .unwrap_or(u32::MAX)
}

/// Whether a navigation target belongs to the app's own origin: the bundled
/// assets (`tauri://localhost` on macOS and Linux, `http(s)://tauri.localhost`
/// on Windows) or, in development builds, the dev server origin.
pub fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    let bundled = match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" | "https" => url.host_str() == Some("tauri.localhost") && url.port().is_none(),
        _ => false,
    };
    bundled
        || dev_url.is_some_and(|dev| {
            url.scheme() == dev.scheme()
                && url.host_str() == dev.host_str()
                && url.port_or_known_default() == dev.port_or_known_default()
        })
}

/// Builds and shows a new app window with the given label.
/// `webview_dir` is the webview profile directory (`SHELLPP_DATA_DIR` only).
pub fn build_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    webview_dir: Option<&Path>,
) -> tauri::Result<WebviewWindow<R>> {
    let mut config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == FIRST_WINDOW_LABEL)
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;
    config.label = label.to_owned();
    config.create = true;
    let dev_url = if cfg!(dev) {
        app.config().build.dev_url.clone()
    } else {
        None
    };
    let mut builder = WebviewWindowBuilder::from_config(app, &config)?;
    if let Some(dir) = webview_dir {
        builder = builder.data_directory(dir.to_path_buf());
    }
    builder
        .on_navigation(move |url| {
            let allowed = is_app_url(url, dev_url.as_ref());
            if !allowed {
                tracing::warn!(
                    scheme = url.scheme(),
                    "blocked navigation away from the app"
                );
            }
            allowed
        })
        .on_new_window(|url, _features| {
            tracing::warn!(scheme = url.scheme(), "blocked window.open");
            NewWindowResponse::Deny
        })
        .build()
}

/// Window ▸ New window (and Connect with Alt, Duplicate in a new window):
/// opens `w-<n>` with an optional intent and returns its label.
pub fn open_window<R: Runtime>(
    app: &AppHandle<R>,
    registry: &WindowRegistry,
    webview_dir: Option<&Path>,
    intent: Option<WindowIntent>,
) -> tauri::Result<String> {
    let label = registry.next_label();
    if let Some(intent) = intent {
        registry.set_intent(&label, intent);
    }
    let window = match build_window(app, &label, webview_dir) {
        Ok(window) => window,
        Err(error) => {
            registry.take_intent(&label);
            return Err(error);
        }
    };
    let _ = window.set_focus();
    tracing::info!(label = %label, "window opened");
    Ok(label)
}

/// Brings the focused or last focused window to the front (second launch).
pub fn bring_to_front<R: Runtime>(app: &AppHandle<R>, registry: &WindowRegistry) {
    if let Some(window) = registry.front_window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn labels_start_at_one_and_increase() {
        let registry = WindowRegistry::new();
        assert_eq!(registry.next_label(), "w-1");
        assert_eq!(registry.next_label(), "w-2");
        assert_eq!(registry.next_label(), "w-3");
    }

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|label| (*label).to_owned()).collect()
    }

    #[test]
    fn focus_tracking_forgets_destroyed_windows() {
        let registry = WindowRegistry::new();
        registry.focused("w-2");
        registry.mark_ready("w-2");
        registry.destroyed("w-1");
        assert_eq!(registry.lock().focused.as_deref(), Some("w-2"));
        registry.destroyed("w-2");
        assert_eq!(registry.lock().focused.as_deref(), None);
        assert!(registry.lock().ready.is_empty());
    }

    #[test]
    fn targets_only_ready_windows() {
        let registry = WindowRegistry::new();
        let existing = labels(&["w-1", "w-2", "w-10"]);
        assert_eq!(registry.target_label(&existing), None);
        registry.focused("w-2");
        assert_eq!(registry.target_label(&existing), None);
        registry.mark_ready("w-10");
        assert_eq!(registry.target_label(&existing).as_deref(), Some("w-10"));
        registry.mark_ready("w-1");
        assert_eq!(registry.target_label(&existing).as_deref(), Some("w-1"));
        registry.mark_ready("w-2");
        assert_eq!(registry.target_label(&existing).as_deref(), Some("w-2"));
        // A ready label whose window no longer exists is skipped.
        assert_eq!(
            registry.target_label(&labels(&["w-10"])).as_deref(),
            Some("w-10")
        );
    }

    #[test]
    fn intents_are_taken_once() {
        let registry = WindowRegistry::new();
        let intent = WindowIntent::OpenLocal { shell_id: None };
        registry.set_intent("w-2", intent.clone());
        assert_eq!(registry.take_intent("w-1"), None);
        assert_eq!(registry.take_intent("w-2"), Some(intent.clone()));
        assert_eq!(registry.take_intent("w-2"), None);
        registry.set_intent("w-3", intent);
        registry.destroyed("w-3");
        assert_eq!(registry.take_intent("w-3"), None);
    }

    #[test]
    fn label_numbers_sort_numerically() {
        let mut labels = vec!["w-10", "w-2", "w-1", "other"];
        labels.sort_by_key(|label| label_number(label));
        assert_eq!(labels, vec!["w-1", "w-2", "w-10", "other"]);
    }

    #[test]
    fn navigation_stays_in_the_app_origin() {
        let dev = url("http://localhost:5173");
        for allowed in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/",
            "https://tauri.localhost/index.html#x",
        ] {
            assert!(is_app_url(&url(allowed), None), "{allowed}");
        }
        assert!(is_app_url(
            &url("http://localhost:5173/src/main.tsx"),
            Some(&dev)
        ));
        for blocked in [
            "https://example.com/",
            "http://tauri.localhost.example.com/",
            "http://tauri.localhost:8080/",
            "tauri://example.com/",
            "file:///C:/Windows/",
            "javascript:alert(1)",
            "data:text/html,x",
            "http://localhost:5174/",
            "https://localhost:5173/",
        ] {
            assert!(!is_app_url(&url(blocked), Some(&dev)), "{blocked}");
        }
        assert!(!is_app_url(&url("http://localhost:5173/"), None));
    }
}
