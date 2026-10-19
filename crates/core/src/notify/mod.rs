//! In-app notification center kept in memory only: at most 200 items, newest
//! first, unread counter (docs/data-model.md, section 16.4; UI 1.5).
//!
//! Ids increase from 1 within the run. Opening the list marks every item as
//! read. Everything is lost at exit. The facade emits
//! `notifications:changed` for every change.
//!
//! Owner: W2.

use std::collections::VecDeque;

use crate::service::dto::{NewNotification, Notification, NotificationList};

/// Maximum number of kept notifications; the oldest are dropped.
pub const MAX_NOTIFICATIONS: usize = 200;

/// The notification list.
#[derive(Debug)]
pub struct NotificationCenter {
    next_id: u64,
    /// Newest first.
    items: VecDeque<Notification>,
}

impl NotificationCenter {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            items: VecDeque::new(),
        }
    }

    /// Adds a notification created at `created_at` and returns it.
    pub fn add(&mut self, new: NewNotification, created_at: String) -> Notification {
        let notification = Notification {
            id: self.next_id,
            level: new.level,
            message_key: new.message_key,
            params: new.params,
            source: new.source,
            created_at,
            read: false,
        };
        self.next_id += 1;
        self.items.push_front(notification.clone());
        self.items.truncate(MAX_NOTIFICATIONS);
        notification
    }

    /// Marks every item read. Returns `true` when something changed.
    pub fn mark_all_read(&mut self) -> bool {
        let mut changed = false;
        for item in &mut self.items {
            if !item.read {
                item.read = true;
                changed = true;
            }
        }
        changed
    }

    pub fn unread(&self) -> u32 {
        u32::try_from(self.items.iter().filter(|item| !item.read).count()).unwrap_or(u32::MAX)
    }

    /// Newest first.
    pub fn list(&self) -> NotificationList {
        NotificationList {
            items: self.items.iter().cloned().collect(),
            unread: self.unread(),
        }
    }
}

impl Default for NotificationCenter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::dto::{NotificationLevel, NotificationSource, NotificationSourceKind};
    use std::collections::BTreeMap;

    fn new(key: &str) -> NewNotification {
        NewNotification {
            level: NotificationLevel::Info,
            message_key: key.to_owned(),
            params: BTreeMap::new(),
            source: NotificationSource {
                kind: NotificationSourceKind::App,
                reference: None,
            },
        }
    }

    #[test]
    fn newest_first_with_increasing_ids() {
        let mut center = NotificationCenter::new();
        let first = center.add(new("a"), "t1".into());
        let second = center.add(new("b"), "t2".into());
        assert_eq!(first.id, 1);
        assert_eq!(second.id, 2);
        let list = center.list();
        assert_eq!(list.items[0].message_key, "b");
        assert_eq!(list.items[1].message_key, "a");
        assert_eq!(list.unread, 2);
        assert!(center.mark_all_read());
        assert_eq!(center.unread(), 0);
        assert!(!center.mark_all_read());
        assert!(center.list().items.iter().all(|item| item.read));
    }

    #[test]
    fn keeps_at_most_200() {
        let mut center = NotificationCenter::new();
        for i in 0..250 {
            center.add(new(&format!("n{i}")), String::new());
        }
        let list = center.list();
        assert_eq!(list.items.len(), MAX_NOTIFICATIONS);
        assert_eq!(list.items[0].id, 250);
        assert_eq!(list.items.last().unwrap().id, 51);
        assert_eq!(list.unread, 200);
    }
}
