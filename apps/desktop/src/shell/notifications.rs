//! The notification center (UI 1.5): the list behind the bell of the top
//! bar, newest first, with an unread counter. Opening the list marks every
//! notification as read. Notifications live in memory only (they disappear
//! when the app exits); system notifications are not used.
//!
//! One list per app ([`Notifications::global`]), shown by every window.
//! [`crate::shell::notify`] adds a notification and shows it as a toast.

use gpui::{App, AppContext as _, Context, Entity, Global, SharedString};

/// One notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    id: u64,
    message: SharedString,
    read: bool,
}

impl Notification {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn message(&self) -> &SharedString {
        &self.message
    }

    pub fn is_read(&self) -> bool {
        self.read
    }

    /// Stable element id in the list (`notification-<id>`).
    pub fn element_id(&self) -> SharedString {
        format!("notification-{}", self.id).into()
    }
}

/// The notification list.
#[derive(Debug, Default)]
pub struct Notifications {
    entries: Vec<Notification>,
    last_id: u64,
}

struct GlobalNotifications(Entity<Notifications>);

impl Global for GlobalNotifications {}

impl Notifications {
    /// The app's notification list (created on first use).
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(global) = cx.try_global::<GlobalNotifications>() {
            return global.0.clone();
        }
        let entity = cx.new(|_| Self::default());
        cx.set_global(GlobalNotifications(entity.clone()));
        entity
    }

    /// Newest first.
    pub fn entries(&self) -> &[Notification] {
        &self.entries
    }

    pub fn unread(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.read).count()
    }

    /// Adds an unread notification at the top of the list.
    pub fn push(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) -> u64 {
        self.last_id += 1;
        self.entries.insert(
            0,
            Notification {
                id: self.last_id,
                message: message.into(),
                read: false,
            },
        );
        cx.notify();
        self.last_id
    }

    /// Opening the list marks everything as read.
    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        if self.entries.iter().any(|entry| !entry.read) {
            for entry in &mut self.entries {
                entry.read = true;
            }
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn newest_first_and_opening_marks_read(cx: &mut TestAppContext) {
        let list = cx.update(Notifications::global);
        list.update(cx, |list, cx| {
            list.push("first", cx);
            list.push("second", cx);
            assert_eq!(list.unread(), 2);
            assert_eq!(list.entries()[0].message().as_ref(), "second");
            list.mark_all_read(cx);
            assert_eq!(list.unread(), 0);
            assert!(list.entries().iter().all(Notification::is_read));
        });
        // The same list everywhere.
        let again = cx.update(Notifications::global);
        assert_eq!(again.entity_id(), list.entity_id());
    }
}
