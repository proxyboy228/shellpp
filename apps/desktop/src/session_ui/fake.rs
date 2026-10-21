//! A fake session layer for headless tests: [`FakeLauncher`] records every
//! request and command, and keeps the panes so a test can report phases,
//! prompts and host keys to them. No network, no PTY, no keys.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{App, Context, Entity, TestAppContext, WeakEntity};
use secrecy::ExposeSecret as _;

use super::model::{HostKeyReply, PromptReply, SessionControl};
use super::{Failure, Launcher, SessionLauncher, SessionPane};
use crate::sessions::OpenRequest;
use crate::terminal::{TerminalHandle, TerminalSize};

/// What reached the fake session layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Launch(OpenRequest),
    Cancel,
    Reconnect,
    Close,
    /// A prompt answer: the value (tests only use generated values) and the
    /// checkbox.
    Answer(String, bool),
    CancelPrompt,
    /// Allow of a signature question.
    AllowSignature,
    HostKey(HostKeyReply),
}

/// The shared record of a [`FakeLauncher`].
#[derive(Clone, Default)]
pub struct Record {
    calls: Rc<RefCell<Vec<Call>>>,
    panes: Rc<RefCell<Vec<WeakEntity<SessionPane>>>>,
    terminals: Rc<RefCell<Vec<TerminalHandle>>>,
    /// The terminal's size at each launch (what the shell would open with).
    launch_sizes: Rc<RefCell<Vec<TerminalSize>>>,
    /// The reason every launch fails with, if any.
    fail: Rc<Cell<Option<Failure>>>,
}

impl Record {
    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    /// The panes launched so far, oldest first.
    pub fn panes(&self) -> Vec<Entity<SessionPane>> {
        self.panes
            .borrow()
            .iter()
            .filter_map(WeakEntity::upgrade)
            .collect()
    }

    /// The terminals handed to the session layer, oldest first.
    pub fn terminals(&self) -> Vec<TerminalHandle> {
        self.terminals.borrow().clone()
    }

    /// The terminal sizes at the launches, oldest first.
    pub fn launch_sizes(&self) -> Vec<TerminalSize> {
        self.launch_sizes.borrow().clone()
    }

    /// Makes the following launches fail with `fail` (or succeed).
    pub fn set_fail(&self, fail: Option<Failure>) {
        self.fail.set(fail);
    }

    fn push(&self, call: Call) {
        self.calls.borrow_mut().push(call);
    }
}

/// The launcher of the fake session layer; `fail` makes every launch fail.
pub struct FakeLauncher {
    record: Record,
}

impl FakeLauncher {
    /// Installs the fake as the app's launcher and returns its record.
    pub fn install(cx: &mut TestAppContext) -> Record {
        Self::install_with(None, cx)
    }

    pub fn install_with(fail: Option<Failure>, cx: &mut TestAppContext) -> Record {
        let record = Record::default();
        record.set_fail(fail);
        let launcher = Self {
            record: record.clone(),
        };
        cx.update(|cx| cx.set_global(Launcher(Rc::new(launcher))));
        record
    }
}

impl SessionLauncher for FakeLauncher {
    fn launch(
        &self,
        request: OpenRequest,
        terminal: TerminalHandle,
        cx: &mut Context<SessionPane>,
    ) -> Result<Rc<dyn SessionControl>, Failure> {
        self.record.push(Call::Launch(request));
        self.record.panes.borrow_mut().push(cx.weak_entity());
        self.record.launch_sizes.borrow_mut().push(terminal.size());
        self.record.terminals.borrow_mut().push(terminal);
        if let Some(failure) = self.record.fail.get() {
            return Err(failure);
        }
        Ok(Rc::new(FakeControl {
            record: self.record.clone(),
        }))
    }
}

struct FakeControl {
    record: Record,
}

impl SessionControl for FakeControl {
    fn cancel(&self, _: &mut App) {
        self.record.push(Call::Cancel);
    }

    fn reconnect(&self, _: &mut App) {
        self.record.push(Call::Reconnect);
    }

    fn close(&self, _: &mut App) {
        self.record.push(Call::Close);
    }

    fn reply_prompt(&self, reply: PromptReply, _: &mut App) {
        self.record.push(match reply {
            PromptReply::Answer { value, keep } => {
                Call::Answer(value.expose_secret().to_owned(), keep)
            }
            PromptReply::Cancel => Call::CancelPrompt,
            PromptReply::Allow => Call::AllowSignature,
        });
    }

    fn reply_host_key(&self, reply: HostKeyReply, _: &mut App) {
        self.record.push(Call::HostKey(reply));
    }
}
