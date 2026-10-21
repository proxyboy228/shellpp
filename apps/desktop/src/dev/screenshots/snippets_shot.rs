//! The Snippets states of the screenshot run (UI 9; plan of run 3, section
//! 8): the screen with a package and snippet cards, a snippet's panel with
//! its targets, the form of a run's values, the confirmation for several
//! hosts and the results table.
//!
//! The snippets are saved in the demo profile for the run. Nothing runs:
//! the results table is filled by hand ([`RunResults::preview`]).

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Value, json};
use shellpp_core::service::snippets::{RecordInput, SnippetView};
use shellpp_ssh::batch::TargetOutcome;
use shellpp_ssh::client::ExecOutput;
use shellpp_ssh::{FailureKind, SshError};
use uuid::Uuid;

use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::shell::{Section, TopTab};
use crate::snippets::{RunResults, RunRow, RunState};
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 7] = [
    ShotState::Snippets,
    ShotState::SnippetPanel,
    ShotState::SnippetVariables,
    ShotState::SnippetConfirm,
    ShotState::SnippetResults,
    ShotState::SnippetLeave,
    ShotState::ShellHistory,
];

/// Commands of the demo's first hosts for Shell History (once).
fn seed_history(state: &Entity<AppState>, cx: &mut gpui::App) -> Option<()> {
    use shellpp_core::service::dto::{HistoryCandidate, HistorySearchInput, RecordRef};
    use shellpp_core::service::host_api::HistoryRecordInput;
    let core = state.read(cx).core()?.clone();
    let vault_id = state.read(cx).vault_id()?;
    let found = core
        .history_search(HistorySearchInput {
            hosts: Vec::new(),
            text: None,
            limit: Some(1),
        })
        .ok()?;
    if !found.is_empty() {
        return Some(());
    }
    let hosts: Vec<Uuid> = state
        .read(cx)
        .hosts()
        .iter()
        .take(3)
        .map(|host| host.record_id)
        .collect();
    let commands = [
        "sudo systemctl restart nginx",
        "journalctl -u nginx --since today",
        "df -h /var",
        "docker compose ps",
        "git -C /srv/app pull --ff-only",
        "tail -f /var/log/syslog",
    ];
    for (index, command) in commands.iter().enumerate() {
        let host = *hosts.get(index % hosts.len().max(1))?;
        core.history_record(HistoryRecordInput {
            host: Some(RecordRef {
                vault_id,
                record_id: host,
            }),
            session_id: Uuid::now_v7(),
            candidate: HistoryCandidate {
                command: (*command).to_owned(),
                prompt_text: "deploy@web:~$".to_owned(),
                typed_printable: true,
                alt_screen: false,
            },
            injected: false,
            record_history: true,
        })
        .ok()?;
    }
    Some(())
}

fn not_reached(state: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(state, reason)
}

/// The demo snippets (`hosts`: the demo's hosts), once.
fn prepare(state: &Entity<AppState>, cx: &mut gpui::App) -> Option<Vec<SnippetView>> {
    let core = state.read(cx).core()?.clone();
    let vault_id = state.read(cx).vault_id()?;
    let existing = core.snippets_list(vault_id).ok()?;
    if !existing.is_empty() {
        return Some(existing);
    }
    let hosts: Vec<Uuid> = state
        .read(cx)
        .hosts()
        .iter()
        .map(|host| host.record_id)
        .collect();
    let first = *hosts.first()?;
    let some: Vec<Uuid> = hosts.iter().copied().take(3).collect();
    let many: Vec<Uuid> = hosts.iter().copied().take(6).collect();
    let input = |data: Value| RecordInput {
        vault_id,
        record_id: None,
        data: data.as_object().cloned().unwrap_or_default(),
    };
    let deploy = core
        .snippet_package_save(input(json!({"name": "Deploy"})))
        .ok()?
        .record
        .record_id;
    let snippets = [
        json!({"label": "Restart nginx",
               "script": "sudo systemctl restart nginx && systemctl status nginx --no-pager # check\nif [ $? -ne 0 ]; then\n  journalctl -u nginx -n 50 \"--since=today\"\nfi",
               "tags": ["web", "nginx"], "target_host_ids": some}),
        json!({"label": "Disk usage", "script": "df -h {{path}}", "target_host_ids": many}),
        json!({"script": "uptime"}),
        json!({"label": "Release", "script": "cd /srv/app && git pull && ./deploy.sh {{version}}",
               "package_id": deploy, "target_host_ids": [first]}),
        json!({"label": "Roll back", "script": "cd /srv/app && ./rollback.sh",
               "package_id": deploy, "target_host_ids": [first]}),
    ];
    for data in snippets {
        core.snippet_save(input(data)).ok()?;
    }
    core.snippets_list(vault_id).ok()
}

fn output(text: &str, code: u32) -> TargetOutcome {
    TargetOutcome::Finished(ExecOutput {
        stdout: text.as_bytes().to_vec(),
        exit_status: Some(code),
        truncated: false,
        timed_out: false,
    })
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, shot)?;
    let snippets = cx
        .update(|cx| prepare(state, cx))
        .ok_or_else(|| not_reached(shot, "the demo snippets could not be saved"))?;
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.set_top_tab(TopTab::Vaults, cx);
            shell.set_section(Section::Snippets, cx);
        })
    });
    let screen = cx.update(|cx| shell.read(cx).snippets_screen().clone());
    if shot == ShotState::ShellHistory {
        cx.update(|cx| seed_history(state, cx))
            .ok_or_else(|| not_reached(shot, "the demo history could not be saved"))?;
    }
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.reload(cx);
            screen.enter_package(None, cx);
            screen.close_panel(cx);
            screen.show_history(shot == ShotState::ShellHistory, window, cx);
        })
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await?;
    let by_label = |label: &str| {
        snippets
            .iter()
            .find(|snippet| snippet.label.as_deref() == Some(label))
            .cloned()
    };
    let (Some(nginx), Some(disk)) = (by_label("Restart nginx"), by_label("Disk usage")) else {
        return Err(not_reached(shot, "the demo snippets are missing"));
    };
    let titles: Vec<(Uuid, String)> = cx.update(|cx| {
        let state = state.read(cx);
        disk.target_host_ids
            .iter()
            .filter_map(|id| state.host(*id).map(|host| (*id, host.title().to_owned())))
            .collect()
    });
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| match shot {
            ShotState::SnippetPanel | ShotState::SnippetLeave => {
                screen.select(Some(nginx.record.record_id), window, cx)
            }
            ShotState::SnippetVariables => screen.run(disk.record.record_id, window, cx),
            ShotState::SnippetConfirm => screen.run(nginx.record.record_id, window, cx),
            ShotState::SnippetResults => {
                let failure = SshError::new(FailureKind::AuthFailed);
                let states = [
                    RunState::Done(output(
                        "Filesystem  Size  Used Avail Use%\n/dev/sda1    40G   18G   20G  48%\n",
                        0,
                    )),
                    RunState::Done(output("df: /data: No such file or directory\n", 1)),
                    RunState::Done(output(
                        "Filesystem  Size  Used Avail Use%\n/dev/vda1    80G   61G   15G  81%\n",
                        0,
                    )),
                    RunState::Done(TargetOutcome::ConnectFailed(failure)),
                    RunState::Running,
                    RunState::Waiting,
                ];
                let rows = titles
                    .iter()
                    .zip(states)
                    .enumerate()
                    .map(|(index, ((host, title), state))| RunRow {
                        host: *host,
                        title: title.clone(),
                        state,
                        expanded: index == 0,
                    })
                    .collect();
                let results = cx.new(|_| RunResults::preview("Disk usage".into(), rows));
                screen.show_results(results, window, cx);
            }
            _ => {}
        })
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await?;
    if shot == ShotState::SnippetLeave {
        // A change typed into the description, then the collapse button.
        cx.update_window(handle, |_, window, cx| {
            window.click("snippet-label", cx);
            window.press("end", cx);
            window.input(" and php-fpm", cx);
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
        settle(cx, handle).await?;
        cx.update_window(handle, |_, window, cx| {
            screen.update(cx, |screen, cx| screen.leave_panel(window, cx))
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
        settle(cx, handle).await?;
    }
    Ok(())
}

/// Checks that the window shows `shot`.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let wanted: &[&str] = match shot {
        ShotState::Snippets => &["snippet-packages", "snippet-list"],
        ShotState::SnippetPanel => &["snippet-panel", "snippet-form-targets"],
        ShotState::SnippetVariables => &["snippet-variables-form"],
        ShotState::SnippetConfirm => &["snippet-confirm"],
        ShotState::SnippetResults => &["snippet-results"],
        ShotState::SnippetLeave => &["panel-note", "snippet-leave-dialog"],
        ShotState::ShellHistory => &["shell-history-list"],
        _ => return Err(not_reached(shot, "not a Snippets state")),
    };
    for id in wanted {
        let shown = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(*id).is_some_and(|found| found.visible())
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
        if !shown {
            return Err(not_reached(shot, &format!("{id} is not shown")));
        }
    }
    Ok(())
}
