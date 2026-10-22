//! Full-screen programs and text of the terminal compatibility checks (run
//! 4, plan 9.2; docs/compatibility.md, section 2) in the screenshot run:
//! tmux (two panes, and after the app's pane is split beside it), mc, vim,
//! nano, less, emacs, vttest and a sample of Cyrillic, emoji, wide and
//! combining characters. They need the OpenSSH test image (its programs):
//! on the russh test server they are left out of the run.
//!
//! Like the other terminal states, each starts from the reset demo window
//! and opens an SSH tab to the test server; the commands are typed as a
//! user does. Nothing here logs addresses, user names or terminal text.

use gpui::{App, AsyncApp, Entity, Window};

use super::super::{ScreenshotError, ShotState};
use super::{
    HTOP_DRAW, LS_MARKER, Rig, STEP_TIMEOUT, active_pane, clear, render, screen, shows, sleep,
    ssh_session, type_line, wait_until,
};
use crate::app::RootView;
use crate::session_ui::{Phase, SessionPane};
use crate::shell::Shell;
use crate::state::AppState;

/// The states of this module, in capture order.
pub const STATES: [ShotState; 9] = [
    ShotState::TerminalTmux,
    ShotState::TerminalTmuxResize,
    ShotState::TerminalMc,
    ShotState::TerminalVim,
    ShotState::TerminalNano,
    ShotState::TerminalLess,
    ShotState::TerminalEmacs,
    ShotState::TerminalVttest,
    ShotState::TerminalUnicode,
];

/// The file vim, nano and emacs open: a small shell script, for syntax
/// colors. `printf` writes it in the home folder of the test user.
const DEMO_FILE: &str = "printf '#!/bin/sh\\n# Deploy the website.\\nset -eu\\nname=\"shell++\"\\nfor i in 1 2 3; do\\n  echo \"hello $name $i\"\\ndone\\n' > demo.sh";

/// The Unicode sample (typed as `$'\uXXXX'` escapes, so only its output has
/// these characters).
const UNICODE_LINES: [&str; 5] = [
    "Cyrillic: Съешь же ещё этих мягких французских булок, да выпей чаю.",
    "Emoji: 😀 👍 👍🏽 🇺🇦 👨‍👩‍👧 ❤️ ✅ |",
    "Wide: 漢字 かな カナ 한국어 全角ＡＢＣ |",
    "Combining: e\u{301} a\u{308} n\u{303} o\u{302}\u{323} (é ä ñ ộ) |",
    "Box: ┌──┬──┐ │ab│cd│ └──┴──┘",
];
const UNICODE_MARKER: &str = "┌──┬──┐";

/// What each program shows once it has drawn.
const TMUX_MARKER: &str = "[shellpp]";
const TMUX_RESIZE_MARKER: &str = "[resize]";
const MC_MARKER: &str = "Options";
const VIM_MARKER: &str = "\"demo.sh\"";
const NANO_MARKER: &str = "Write Out";
const LESS_MARKER: &str = "lines 1-";
const EMACS_MARKER: &str = "Shell-script";
const VTTEST_MENU_MARKER: &str = "VT100 test program";
// vttest hyphenates "border" on this screen.
const VTTEST_MARKER: &str = "frame of E's";

/// `printf '%s\n' $'…'` for `lines`: non-ASCII characters as `\u` escapes.
fn printf_command(lines: &[&str]) -> String {
    let mut command = String::from("printf '%s\\n'");
    for line in lines {
        command.push_str(" $'");
        for c in line.chars() {
            match c {
                '\\' => command.push_str("\\\\"),
                '\'' => command.push_str("\\'"),
                c if c.is_ascii() => command.push(c),
                c if u32::from(c) <= 0xFFFF => {
                    command.push_str(&format!("\\u{:04x}", u32::from(c)))
                }
                c => command.push_str(&format!("\\U{:08x}", u32::from(c))),
            }
        }
        command.push('\'');
    }
    command
}

/// Types `line` and waits until `marker` is on the screen, then gives the
/// program time to finish drawing.
async fn start(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    pane: &Entity<SessionPane>,
    shot: ShotState,
    line: &str,
    marker: &'static str,
) -> Result<(), ScreenshotError> {
    type_line(cx, handle, pane, line)?;
    wait_until(cx, handle, shot, marker, STEP_TIMEOUT, |cx| {
        Ok(shows(pane, marker, cx))
    })
    .await?;
    sleep(cx, HTOP_DRAW).await;
    render(cx, handle)
}

/// Brings the reset demo window into the program state `shot`.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    rig: &Rig,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    if !rig.real_shell() {
        return Err(ScreenshotError::state(
            shot,
            "needs the OpenSSH test image (its programs)",
        ));
    }
    let (id, pane) = ssh_session(cx, handle, root, state, rig, 0, shot).await?;
    clear(cx, handle, &pane, shot, "clear").await?;
    match shot {
        ShotState::TerminalTmux => {
            // A server of an earlier capture (the other theme) goes first.
            let line = "tmux kill-server 2>/dev/null; tmux new-session -s shellpp \\; split-window -h \\; send-keys 'ls --color' Enter";
            start(cx, handle, &pane, shot, line, TMUX_MARKER).await?;
            wait_until(cx, handle, shot, "ls in tmux", STEP_TIMEOUT, |cx| {
                Ok(shows(&pane, LS_MARKER, cx))
            })
            .await
        }
        ShotState::TerminalTmuxResize => {
            let line =
                "tmux kill-server 2>/dev/null; tmux new-session -s resize \\; split-window -v";
            start(cx, handle, &pane, shot, line, TMUX_RESIZE_MARKER).await?;
            // The app's pane is split: tmux gets half the columns and
            // draws again.
            let shell = super::super::shell_of(cx, root, shot)?;
            let group = super::stage2b::group_of(cx, &shell, id, shot)?;
            let tmux_pane = cx.update(|cx| group.read(cx).active_pane());
            super::stage2b::split_and_open(
                cx,
                handle,
                &group,
                super::stage2b::SPLIT_RIGHT,
                rig,
                shot,
            )
            .await?;
            super::stage2b::activate(cx, handle, &group, tmux_pane)?;
            wait_until(cx, handle, shot, "tmux drawn again", STEP_TIMEOUT, |cx| {
                Ok(status_at_bottom(&pane, TMUX_RESIZE_MARKER, cx))
            })
            .await?;
            sleep(cx, HTOP_DRAW).await;
            render(cx, handle)
        }
        ShotState::TerminalMc => start(cx, handle, &pane, shot, "mc", MC_MARKER).await,
        ShotState::TerminalVim => {
            let line = format!("{DEMO_FILE} && vim demo.sh");
            start(cx, handle, &pane, shot, &line, VIM_MARKER).await
        }
        ShotState::TerminalNano => {
            let line = format!("{DEMO_FILE} && nano demo.sh");
            start(cx, handle, &pane, shot, &line, NANO_MARKER).await
        }
        ShotState::TerminalLess => {
            // The image has no /etc/services: a long pipe instead.
            start(cx, handle, &pane, shot, "seq 1 1000 | less -M", LESS_MARKER).await
        }
        ShotState::TerminalEmacs => {
            let line = format!("{DEMO_FILE} && emacs -nw demo.sh");
            start(cx, handle, &pane, shot, &line, EMACS_MARKER).await
        }
        ShotState::TerminalVttest => {
            start(cx, handle, &pane, shot, "vttest", VTTEST_MENU_MARKER).await?;
            // Test 1, its first screen: a frame of E's in a border.
            start(cx, handle, &pane, shot, "1", VTTEST_MARKER).await
        }
        ShotState::TerminalUnicode => {
            let line = printf_command(&UNICODE_LINES);
            start(cx, handle, &pane, shot, &line, UNICODE_MARKER).await
        }
        _ => Err(ScreenshotError::state(shot, "not a program state")),
    }
}

/// Whether tmux's status line (`marker` first) is the last row on screen.
fn status_at_bottom(pane: &Entity<SessionPane>, marker: &str, cx: &App) -> bool {
    screen(pane, cx)
        .last()
        .is_some_and(|row| row.trim_start().starts_with(marker))
}

/// Checks the program state `shot` before the capture.
pub fn check(
    window: &mut Window,
    cx: &mut App,
    shell: &Entity<Shell>,
    shot: ShotState,
) -> Result<(), &'static str> {
    let _ = &window;
    let pane = active_pane(shell, cx).ok_or("no session tab is active")?;
    if *pane.read(cx).phase() != Phase::Running {
        return Err("the session does not run");
    }
    let need = |ok: bool, reason: &'static str| if ok { Ok(()) } else { Err(reason) };
    match shot {
        ShotState::TerminalTmux => {
            need(shows(&pane, TMUX_MARKER, cx), "no tmux status line")?;
            need(shows(&pane, LS_MARKER, cx), "no ls output in tmux")
        }
        ShotState::TerminalTmuxResize => need(
            status_at_bottom(&pane, TMUX_RESIZE_MARKER, cx),
            "tmux's status line is not on the last row",
        ),
        ShotState::TerminalMc => need(shows(&pane, MC_MARKER, cx), "mc is not on screen"),
        ShotState::TerminalVim => need(shows(&pane, VIM_MARKER, cx), "vim is not on screen"),
        ShotState::TerminalNano => need(shows(&pane, NANO_MARKER, cx), "nano is not on screen"),
        ShotState::TerminalLess => need(shows(&pane, LESS_MARKER, cx), "less is not on screen"),
        ShotState::TerminalEmacs => need(shows(&pane, EMACS_MARKER, cx), "emacs is not on screen"),
        ShotState::TerminalVttest => need(
            shows(&pane, VTTEST_MARKER, cx),
            "vttest's test is not on screen",
        ),
        ShotState::TerminalUnicode => need(
            shows(&pane, UNICODE_MARKER, cx),
            "the sample is not on screen",
        ),
        _ => Err("not a program state"),
    }
}

#[cfg(test)]
mod tests {
    use super::printf_command;

    #[test]
    fn the_unicode_sample_is_typed_as_ascii() {
        let command = printf_command(&["Я 😀 it's \\"]);
        assert!(command.is_ascii());
        assert_eq!(command, "printf '%s\\n' $'\\u042f \\U0001f600 it\\'s \\\\'");
    }
}
