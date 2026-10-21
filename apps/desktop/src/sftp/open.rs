//! Open and Open with of the SFTP tab (UI 5.4; D49; stage 4, item 6).
//!
//! - A local file opens in the program of "Program for Open"
//!   (`sftp.open_program`, UI 12.6) or, when it is empty, in the program
//!   the system assigns; "Open with…" asks for the program.
//! - A remote file is downloaded into the app's `tmp/` folder
//!   (`shellpp_ssh::open_with::OpenedCopy`, storage-format 9: a folder of its
//!   own, the original name made safe), opened the same way, and watched:
//!   when the program saves it, the copy is uploaded back and a
//!   notification says so. When the file on the server changed meanwhile,
//!   nothing is written until the user chooses to overwrite it.
//! - Opening a remote file warns once per open that the copy is not
//!   encrypted. Copies are removed when the app exits (`tmp/` is emptied).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, AsyncWindowContext, PathPromptOptions, Task, WeakEntity, Window};
use shellpp_ssh::Sftp;
use shellpp_ssh::open_with::{OpenedCopy, UploadOutcome};

use super::strings;
use crate::sessions::SessionHandle;
use crate::state::AppState;

/// How often an opened copy is checked for a save.
const WATCH: Duration = Duration::from_secs(2);

/// A remote file open in a local program.
pub struct Opened {
    copy: Arc<tokio::sync::Mutex<OpenedCopy>>,
    /// The local copy's path.
    path: PathBuf,
    /// Its session: the channel is taken for each upload, so a reconnect's
    /// new channel is used.
    handle: SessionHandle,
    name: String,
    /// The file on the server changed: uploads wait for the user's answer
    /// (shared by the copies of this handle).
    blocked: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("blocked", &self.is_blocked())
            .finish_non_exhaustive()
    }
}

/// Marks a downloaded copy the way the system marks downloads (security.md
/// 11.4): the Windows Mark of the Web, so the system's checks apply when a
/// program opens it. Elsewhere nothing yet.
pub fn mark_downloaded(path: &Path) {
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        if let Err(error) = std::fs::write(&stream, b"[ZoneTransfer]\r\nZoneId=3\r\n") {
            tracing::warn!(kind = %error.kind(), "the downloaded copy was not marked");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

/// "Open a program file?": Cancel or Open (security.md 11.4).
pub fn confirm_run(
    name: &str,
    extension: &str,
    window: &mut Window,
    cx: &mut App,
    on_open: impl Fn(&mut Window, &mut App) + 'static,
) {
    use gpui::prelude::*;
    use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex};

    use crate::hosts::controls::{ControlButton, ControlKind};
    use crate::theme::metrics;

    let text = strings::run_text(&super::display_name(name), extension);
    let on_open = std::rc::Rc::new(on_open);
    window.open_dialog(cx, move |dialog, _, cx| {
        let on_open = on_open.clone();
        dialog
            .w(gpui::px(420.))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                gpui::div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(strings::RUN_TITLE),
            )
            .child(
                gpui::div()
                    .id("sftp-run-confirm")
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(cx.theme().muted_foreground)
                    .child(text.clone()),
            )
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("sftp-run-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("sftp-run-open", ControlKind::Danger)
                            .label(strings::RUN_OPEN)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                on_open(window, cx);
                            }),
                    ),
            )
    });
}

/// The program of "Program for Open", when one is set.
pub fn open_program(state: &AppState) -> Option<PathBuf> {
    state
        .core()
        .and_then(|core| core.settings_get().ok())
        .and_then(|values| values.get("sftp.open_program").cloned())
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|program| !program.trim().is_empty())
        .map(PathBuf::from)
}

/// Opens `path` in `program`, or in the system's program for its type.
pub fn launch(path: &Path, program: Option<&Path>, window: &mut Window, cx: &mut App) {
    match program {
        Some(program) => {
            if let Err(error) = std::process::Command::new(program).arg(path).spawn() {
                tracing::warn!(kind = %error.kind(), "open program not started");
                crate::shell::show_toast(strings::OPEN_FAILED, window, cx);
            }
        }
        None => cx.open_with_system(path),
    }
}

/// "Open with…": the program, chosen in the system's open dialog.
pub fn choose_program(cx: &mut App) -> Task<Option<PathBuf>> {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    cx.background_spawn(async move {
        match paths.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            _ => None,
        }
    })
}

/// Downloads the remote file `remote` into `root` and returns its copy.
pub async fn download(
    handle: SessionHandle,
    sftp: Arc<Sftp>,
    remote: String,
    root: PathBuf,
) -> Result<OpenedCopy, String> {
    let work = handle.spawn(async move {
        OpenedCopy::open(&sftp, &remote, &root)
            .await
            .map_err(|error| error.to_string())
    });
    work.await
        .unwrap_or_else(|_| Err(strings::OPEN_FAILED.to_owned()))
}

/// "The file changed on the server": Overwrite uploads the copy over it;
/// Keep leaves the server's file (the copy stays open, not uploaded).
fn ask_overwrite(open: Opened, window: &mut Window, cx: &mut App) {
    use gpui::prelude::*;
    use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex};

    use crate::hosts::controls::{ControlButton, ControlKind};
    use crate::theme::metrics;

    let name = open.name.clone();
    let open = Arc::new(open);
    window.open_dialog(cx, move |dialog, _, cx| {
        let open = open.clone();
        dialog
            .w(gpui::px(420.))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                gpui::div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(strings::CHANGED_ON_SERVER_TITLE),
            )
            .child(
                gpui::div()
                    .id("sftp-open-changed")
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(cx.theme().muted_foreground)
                    .child(strings::changed_on_server(&name)),
            )
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("sftp-open-keep", ControlKind::Neutral)
                            .label(strings::KEEP_SERVER_FILE)
                            .on_click({
                                let name = name.clone();
                                move |_, window, cx| {
                                    window.close_dialog(cx);
                                    // Its saves are not uploaded from now on.
                                    crate::shell::show_toast(
                                        strings::kept_server_file(&name),
                                        window,
                                        cx,
                                    );
                                }
                            }),
                    )
                    .child(
                        ControlButton::new("sftp-open-overwrite", ControlKind::Danger)
                            .label(strings::OVERWRITE)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let open = open.clone();
                                let name = open.name.clone();
                                window
                                    .spawn(cx, async move |cx| {
                                        let outcome = open.upload(true).await;
                                        if outcome.is_ok() {
                                            // Later saves upload again.
                                            open.blocked
                                                .store(false, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        let _ = cx.update(|window, cx| match outcome {
                                            Ok(_) => crate::shell::show_toast(
                                                strings::uploaded_back(&name),
                                                window,
                                                cx,
                                            ),
                                            Err(reason) => crate::shell::show_toast(
                                                strings::failed(
                                                    strings::UPLOAD_BACK_FAILED,
                                                    &reason,
                                                ),
                                                window,
                                                cx,
                                            ),
                                        });
                                    })
                                    .detach();
                            }),
                    ),
            )
    });
}

impl Opened {
    /// The same copy, for a task (the watch, a dialog).
    fn clone_parts(&self) -> Self {
        Self {
            copy: self.copy.clone(),
            path: self.path.clone(),
            handle: self.handle.clone(),
            name: self.name.clone(),
            blocked: self.blocked.clone(),
        }
    }

    pub fn new(copy: OpenedCopy, handle: SessionHandle, name: String) -> Self {
        Self {
            path: copy.local_path().to_path_buf(),
            copy: Arc::new(tokio::sync::Mutex::new(copy)),
            handle,
            name,
            blocked: Arc::default(),
        }
    }

    fn is_blocked(&self) -> bool {
        self.blocked.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn local_path(&self) -> &Path {
        &self.path
    }

    /// Uploads the copy when it was saved (`force`: over a file that changed
    /// on the server).
    async fn upload(&self, force: bool) -> Result<UploadOutcome, String> {
        let copy = self.copy.clone();
        let Some(sftp) = self.handle.sftp() else {
            // Not connected now: the next check after a reconnect uploads.
            return Ok(UploadOutcome::Unchanged);
        };
        let work = self.handle.spawn(async move {
            let mut copy = copy.lock().await;
            copy.upload_back(&sftp, force)
                .await
                .map_err(|error| error.to_string())
        });
        work.await
            .unwrap_or_else(|_| Err(strings::UPLOAD_BACK_FAILED.to_owned()))
    }
}

/// Watches the opened copies of `screen`: a saved copy is uploaded back
/// with a notification; a file changed on the server asks first.
pub fn watch<T: 'static>(
    screen: WeakEntity<T>,
    opened: impl Fn(&mut T) -> &mut Vec<Opened> + 'static,
    cx: &mut AsyncWindowContext,
) -> impl std::future::Future<Output = ()> + 'static {
    let mut cx = cx.clone();
    async move {
        loop {
            cx.background_executor().timer(WATCH).await;
            // The copies to check now (not blocked by a question).
            let Ok(copies) = screen.update(&mut cx, |screen, _| {
                opened(screen)
                    .iter()
                    .filter(|open| !open.is_blocked())
                    .map(Opened::clone_parts)
                    .collect::<Vec<_>>()
            }) else {
                return;
            };
            for probe in copies {
                let name = probe.name.clone();
                let outcome = probe.upload(false).await;
                let _ = cx.update(|window, cx| match &outcome {
                    Ok(UploadOutcome::Uploaded) => {
                        crate::shell::show_toast(strings::uploaded_back(&name), window, cx);
                    }
                    Ok(UploadOutcome::Unchanged) => {}
                    Ok(UploadOutcome::RemoteChanged) => {
                        ask_overwrite(probe.clone_parts(), window, cx);
                    }
                    Err(reason) => {
                        crate::shell::show_toast(
                            strings::failed(strings::UPLOAD_BACK_FAILED, reason),
                            window,
                            cx,
                        );
                    }
                });
                if matches!(outcome, Ok(UploadOutcome::RemoteChanged)) {
                    probe
                        .blocked
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }
}
