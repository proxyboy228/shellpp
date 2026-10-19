//! Debounced autosave of a panel (UI 1.3; 500 ms, docs/architecture.md 2.5).

use gpui::{Context, Task};

use crate::theme::metrics;

/// One pending save of a view `V`. Scheduling again restarts the delay;
/// [`Autosave::take_pending`] tells a flush whether a save is still due.
#[derive(Default)]
pub struct Autosave {
    task: Option<Task<()>>,
    pending: bool,
}

impl std::fmt::Debug for Autosave {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Autosave")
            .field("pending", &self.pending)
            .finish()
    }
}

impl Autosave {
    /// Runs `save` after the debounce delay, unless scheduled again first.
    pub fn schedule<V: 'static>(
        &mut self,
        cx: &mut Context<V>,
        save: impl FnOnce(&mut V, &mut Context<V>) + 'static,
    ) {
        self.pending = true;
        let timer = cx.background_executor().timer(metrics::AUTOSAVE_DEBOUNCE);
        self.task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, save);
        }));
    }

    /// Whether a save is scheduled and has not run yet.
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Marks the save as done (called by the save itself) and returns
    /// whether one was pending.
    pub fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// Drops a scheduled save without running it.
    pub fn cancel(&mut self) {
        self.pending = false;
        self.task = None;
    }
}
