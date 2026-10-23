//! The Keychain states of the screenshot run (night plan of stage 3, items
//! 1-4; UI 7.1-7.3): the screen with the demo keys and identities, the key
//! panel of the first key, the Generate key form, the import form with a
//! text that is not a private key (the type "unknown" with its reason), the
//! sync conflict badges (UI 13.6) on the first key's and first identity's
//! cards (a conflict copy written through the core's test hook, no server;
//! the reset keeps the current versions), and the empty state on a fresh
//! profile.
//!
//! The keys are the demo profile's, generated when the demo is seeded
//! ([`super::demo::seed`]); the "unknown" text is the public key line of the
//! first one (a public key is not a private key). The `~/.ssh` folder is
//! never read: "Import from ~/.ssh" is not opened here.

use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;

use super::{ScreenshotError, ShotState, click, settle, shell_of, type_into, visible};
use crate::app::RootView;
use crate::keychain::{self, Inspection, KeychainLayout, KeychainScreen, KeychainSort};
use crate::shell::Section;

const TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);
/// The label typed into the Generate key form.
pub const GENERATE_LABEL: &str = "Work laptop";

fn not_reached(state: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(state, reason)
}

fn screen_of(
    cx: &mut AsyncApp,
    root: &Entity<RootView>,
    state: ShotState,
) -> Result<Entity<KeychainScreen>, ScreenshotError> {
    let shell = shell_of(cx, root, state)?;
    Ok(cx.update(|cx| shell.read(cx).keychain_screen().clone()))
}

/// Polls `condition` after a fresh frame until it holds.
async fn wait_for(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: ShotState,
    what: &str,
    mut condition: impl FnMut(&mut gpui::App) -> bool,
) -> Result<(), ScreenshotError> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .map_err(|_| ScreenshotError::WindowClosed)?;
        if cx.update(|cx| condition(cx)) {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(not_reached(state, &format!("timeout: {what}")));
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// The first key in the name order (the panel's key).
fn first_key(screen: &KeychainScreen) -> Option<uuid::Uuid> {
    screen.shown_key_ids().first().copied()
}

/// The cards of the conflict state: the first key and the first identity,
/// with their titles.
fn conflict_records(screen: &KeychainScreen) -> Option<[(uuid::Uuid, String); 2]> {
    let key = first_key(screen)?;
    let identity = *screen.shown_identity_ids().first()?;
    let key_title = keychain::key_title(screen.keys().iter().find(|view| view.record_id == key)?);
    let identity_label = screen
        .identities()
        .iter()
        .find(|view| view.record_id == identity)?
        .label
        .clone();
    Some([(key, key_title), (identity, identity_label)])
}

/// From the reset demo window: Keychain in the sidebar, tiles by name, no
/// panel (no state types a search); then the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    click(cx, handle, Section::Keychain.element_id().as_ref())?;
    settle(cx, handle).await?;
    let screen = screen_of(cx, root, shot)?;
    cx.update(|cx| {
        screen.update(cx, |screen, cx| {
            screen.close_panel(cx);
            screen.set_layout(KeychainLayout::Tiles, cx);
            screen.set_sort(KeychainSort::Name, cx);
        })
    });
    wait_for(cx, handle, shot, "the demo keys", |cx| {
        first_key(screen.read(cx)).is_some()
    })
    .await?;
    match shot {
        // The keyboard focus ring on the first key, nothing selected (run 5,
        // plan 11.3): the cards have the focus, the last input a key (Esc
        // with no panel does nothing else).
        ShotState::Keychain => {
            cx.update_window(handle, |_, window, cx| {
                let focus = gpui::Focusable::focus_handle(screen.read(cx), cx);
                focus.focus(window, cx);
                window.press("escape", cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::KeychainPanel => {
            let key = cx
                .update(|cx| first_key(screen.read(cx)))
                .ok_or_else(|| not_reached(shot, "no key"))?;
            click(cx, handle, keychain::key_card_id(key).as_ref())
        }
        ShotState::KeychainGenerate => {
            cx.update_window(handle, |_, window, cx| {
                screen.update(cx, |screen, cx| screen.open_generate(window, cx))
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await?;
            type_into(cx, handle, "kc-gen-label", GENERATE_LABEL)
        }
        ShotState::KeychainImportUnknown => {
            let key = cx
                .update(|cx| first_key(screen.read(cx)))
                .ok_or_else(|| not_reached(shot, "no key"))?;
            let state = cx.update(|cx| root.read(cx).state().clone());
            let public = cx
                .update(|cx| state.read(cx).key_public_key(key))
                .map_err(|_| not_reached(shot, "no public key"))?;
            cx.update_window(handle, |_, window, cx| {
                screen.update(cx, |screen, cx| screen.open_import(window, cx));
                let form = screen.read(cx).import_form().cloned();
                if let Some(form) = form {
                    form.update(cx, |form, cx| form.set_text(&public, window, cx));
                }
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            wait_for(cx, handle, shot, "the inspection", |cx| {
                screen.read(cx).import_form().is_some_and(|form| {
                    matches!(form.read(cx).inspection(), Inspection::Problem(_))
                })
            })
            .await
        }
        // Sync kept another device's version of the first key and the first
        // identity (UI 13.6): their cards show the badge.
        ShotState::KeychainConflict => {
            let records = cx
                .update(|cx| conflict_records(screen.read(cx)))
                .ok_or_else(|| not_reached(shot, "no key or no identity"))?;
            let state = cx.update(|cx| root.read(cx).state().clone());
            let core = cx
                .update(|cx| state.read(cx).core().cloned())
                .ok_or_else(|| not_reached(shot, "no core"))?;
            let changed = std::time::SystemTime::now() - Duration::from_secs(3 * 3600);
            for (record, title) in &records {
                let mut losing = serde_json::Map::new();
                losing.insert("label".into(), format!("{title} (laptop)").into());
                core.sync_conflict_insert_for_test(
                    *record,
                    Some(losing),
                    uuid::Uuid::now_v7(),
                    changed,
                )
                .map_err(|_| not_reached(shot, "the conflict was not written"))?;
            }
            // The badges come with the status the core's event refreshes.
            wait_for(cx, handle, shot, "the badges", |cx| {
                let conflicted = state.read(cx).sync_conflicted();
                records
                    .iter()
                    .all(|(record, _)| conflicted.contains(record))
            })
            .await?;
            settle(cx, handle).await
        }
        _ => Err(not_reached(shot, "not a Keychain state")),
    }
}

/// The empty state on a fresh profile: Keychain in the sidebar.
pub async fn reach_empty(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
) -> Result<(), ScreenshotError> {
    click(cx, handle, Section::Keychain.element_id().as_ref())?;
    settle(cx, handle).await
}

/// Checks the evidence of a Keychain state before the capture.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let screen = screen_of(cx, root, shot)?;
    let result = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let need = |ok: bool, reason: &'static str| if ok { Ok(()) } else { Err(reason) };
            let screen = screen.read(cx);
            need(visible(window, "keychain-screen"), "no Keychain screen")?;
            match shot {
                ShotState::Keychain => {
                    need(!screen.has_panel(), "a panel is open")?;
                    need(
                        visible(window, "kc-heading-keys")
                            && visible(window, "kc-heading-identities"),
                        "no Keys and Identities sections",
                    )?;
                    need(
                        first_key(screen).is_some_and(|key| {
                            visible(window, keychain::key_card_id(key).as_ref())
                        }),
                        "no key card",
                    )
                }
                ShotState::KeychainPanel => {
                    need(visible(window, "kc-key-panel"), "no key panel")?;
                    need(
                        visible(window, "kc-key-hidden-note") && !visible(window, "kc-key-private"),
                        "the private key is not hidden",
                    )
                }
                ShotState::KeychainGenerate => {
                    need(visible(window, "kc-generate"), "no Generate key form")?;
                    need(
                        window
                            .try_find("kc-gen-label")
                            .and_then(|found| found.value().map(|value| value == GENERATE_LABEL))
                            == Some(true),
                        "the label is not typed",
                    )
                }
                ShotState::KeychainImportUnknown => {
                    need(visible(window, "kc-import"), "no import form")?;
                    need(
                        visible(window, "kc-import-unknown") && visible(window, "kc-import-reason"),
                        "no unknown type with its reason",
                    )
                }
                ShotState::KeychainConflict => {
                    need(!screen.has_panel(), "a panel is open")?;
                    let [(key, _), (identity, _)] =
                        conflict_records(screen).ok_or("no key or no identity")?;
                    need(
                        visible(window, keychain::key_conflict_badge_id(key).as_ref())
                            && visible(
                                window,
                                keychain::identity_conflict_badge_id(identity).as_ref(),
                            ),
                        "no conflict badges",
                    )
                }
                ShotState::KeychainEmpty => need(visible(window, "kc-empty"), "no empty state"),
                _ => Err("not a Keychain state"),
            }
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    result.map_err(|reason| not_reached(shot, reason))
}
