//! Undo of Remove (UI 2.4, owner's step 3.3): every removal through
//! [`AppState::delete_records`] can be undone for [`UNDO_WINDOW`] (the 5 s of
//! the toast that offers Undo). Undo brings the records back under their ids
//! with their fields (the core keeps the removed payloads for 60 s, memory
//! only) and moves the children that left a removed group back into it.
//! After the window the removal is final: [`AppState::restore_records`]
//! fails with `CoreError::NotFound`, and a restart never restores anything.
//!
//! Locking forgets every pending removal (the core wipes its undo buffer
//! with the session).

use std::collections::HashSet;
use std::time::Duration;

use gpui::{Context, Task};
use shellpp_core::CoreError;
use shellpp_core::service::dto::{MovePosition, RecordMoveInput, RecordsRef};
use uuid::Uuid;

use super::{AppState, GroupItem, HostItem};

/// How long a removal can be undone: the Undo toast's 5 seconds
/// (`theme::metrics::TOAST_DURATION`).
pub const UNDO_WINDOW: Duration = Duration::from_secs(5);

/// Children of a removed group: the core moves them to the nearest remaining
/// ancestor; Undo moves them back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedChildren {
    /// The removed group.
    pub group: Uuid,
    /// Its direct subgroups and hosts that were not removed, in manual order
    /// (subgroups first).
    pub children: Vec<Uuid>,
    /// Where the core moved them (`None`: the top level).
    pub moved_to: Option<Uuid>,
}

/// The children every removed group hands up one level (pure: computed
/// from the loaded lists before the removal).
pub fn moved_children(
    removed: &[Uuid],
    hosts: &[HostItem],
    groups: &[GroupItem],
) -> Vec<MovedChildren> {
    let removed_set: HashSet<Uuid> = removed.iter().copied().collect();
    let parent_of = |id: Uuid| {
        groups
            .iter()
            .find(|group| group.record_id == id)
            .and_then(|group| group.parent_id)
    };
    removed
        .iter()
        .copied()
        .filter(|id| groups.iter().any(|group| group.record_id == *id))
        .map(|group| {
            let mut children: Vec<Uuid> = groups
                .iter()
                .filter(|child| child.parent_id == Some(group))
                .map(|child| child.record_id)
                .filter(|id| !removed_set.contains(id))
                .collect();
            children.extend(
                hosts
                    .iter()
                    .filter(|host| host.group_id == Some(group))
                    .map(|host| host.record_id)
                    .filter(|id| !removed_set.contains(id)),
            );
            // The nearest ancestor that stays (data-model 7.1).
            let mut moved_to = parent_of(group);
            let mut guard = 0;
            while let Some(parent) = moved_to.filter(|parent| removed_set.contains(parent)) {
                moved_to = parent_of(parent);
                guard += 1;
                if guard > groups.len() {
                    moved_to = None;
                    break;
                }
            }
            MovedChildren {
                group,
                children,
                moved_to,
            }
        })
        .filter(|moved| !moved.children.is_empty())
        .collect()
}

/// One removal that can still be undone.
pub(super) struct PendingRemoval {
    serial: u64,
    records: Vec<Uuid>,
    children: Vec<MovedChildren>,
    /// Ends the window: drops this entry after [`UNDO_WINDOW`].
    _expiry: Task<()>,
}

/// The removals that can still be undone.
#[derive(Default)]
pub(super) struct PendingRemovals {
    next_serial: u64,
    entries: Vec<PendingRemoval>,
}

impl PendingRemovals {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    fn contains(&self, id: &Uuid) -> bool {
        self.entries.iter().any(|entry| entry.records.contains(id))
    }
}

impl AppState {
    /// Starts the undo window of a removal that just happened.
    pub(super) fn remember_removal(
        &mut self,
        records: Vec<Uuid>,
        children: Vec<MovedChildren>,
        cx: &mut Context<Self>,
    ) {
        let serial = self.removals.next_serial;
        self.removals.next_serial += 1;
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(UNDO_WINDOW).await;
            let _ = this.update(cx, |state, _| {
                state
                    .removals
                    .entries
                    .retain(|entry| entry.serial != serial);
            });
        });
        self.removals.entries.push(PendingRemoval {
            serial,
            records,
            children,
            _expiry: expiry,
        });
    }

    /// Whether every one of `record_ids` was removed less than
    /// [`UNDO_WINDOW`] ago (Undo still works).
    pub fn can_undo_removal(&self, record_ids: &[Uuid]) -> bool {
        !record_ids.is_empty() && record_ids.iter().all(|id| self.removals.contains(id))
    }

    /// Undo of Remove (UI 2.4): brings removed records back under their ids
    /// with their fields, and the children of a removed group back into it.
    /// `CoreError::NotFound` once [`UNDO_WINDOW`] has passed: the removal is
    /// final.
    pub fn restore_records(
        &mut self,
        record_ids: Vec<Uuid>,
        cx: &mut Context<Self>,
    ) -> Result<(), CoreError> {
        let vault_id = self.require_vault()?;
        if !self.can_undo_removal(&record_ids) {
            return Err(CoreError::NotFound);
        }
        let core = self.require_core()?.clone();
        core.record_restore(RecordsRef {
            vault_id,
            record_ids: record_ids.clone(),
        })?;
        let restored: HashSet<Uuid> = record_ids.iter().copied().collect();
        let mut children: Vec<MovedChildren> = Vec::new();
        for entry in &mut self.removals.entries {
            entry.records.retain(|id| !restored.contains(id));
            let (back, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut entry.children)
                .into_iter()
                .partition(|moved| restored.contains(&moved.group));
            entry.children = keep;
            children.extend(back);
        }
        self.removals
            .entries
            .retain(|entry| !entry.records.is_empty());
        // The lists still hold the state after the removal: a child goes back
        // only while it is still where the removal put it.
        self.reload();
        for moved in children {
            let ids: Vec<Uuid> = moved
                .children
                .iter()
                .copied()
                .filter(|id| {
                    self.host(*id)
                        .map(|host| host.group_id)
                        .or_else(|| self.group(*id).map(|group| group.parent_id))
                        == Some(moved.moved_to)
                })
                .collect();
            if ids.is_empty() {
                continue;
            }
            if let Err(error) = core.record_move(RecordMoveInput {
                vault_id,
                record_ids: ids,
                target_group_id: Some(moved.group),
                position: MovePosition::End,
            }) {
                tracing::warn!(code = error.code(), "children not moved back after undo");
            }
        }
        tracing::info!(restored = record_ids.len(), "removal undone");
        self.records_written(cx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use shellpp_core::service::dto::TagColor;

    fn group(id: u128, parent: Option<u128>) -> GroupItem {
        GroupItem {
            record_id: Uuid::from_u128(id),
            vault_id: Uuid::nil(),
            revision: 1,
            name: format!("g{id}"),
            color: TagColor::default(),
            parent_id: parent.map(Uuid::from_u128),
            path: Vec::new(),
            host_count: 0,
            child_count: 0,
            created_at: String::new(),
            modified_at: String::new(),
            data: Value::Null,
        }
    }

    fn host(id: u128, group: Option<u128>) -> HostItem {
        HostItem {
            record_id: Uuid::from_u128(id),
            vault_id: Uuid::nil(),
            revision: 1,
            label: None,
            address: format!("h{id}.example.com"),
            tags: Vec::new(),
            color: TagColor::default(),
            username: None,
            port: None,
            os_family: None,
            group_id: group.map(Uuid::from_u128),
            path: Vec::new(),
            created_at: String::new(),
            modified_at: String::new(),
            data: Value::Null,
            secrets_set: Vec::new(),
        }
    }

    #[test]
    fn undo_lasts_as_long_as_its_toast() {
        assert_eq!(UNDO_WINDOW, crate::theme::metrics::TOAST_DURATION);
    }

    #[test]
    fn children_of_removed_groups_go_to_the_nearest_remaining_ancestor() {
        // 1 > 2 > 3; hosts 10 in 2, 11 in 3, 12 at the top level.
        let groups = vec![group(1, None), group(2, Some(1)), group(3, Some(2))];
        let hosts = vec![host(10, Some(2)), host(11, Some(3)), host(12, None)];
        let id = Uuid::from_u128;

        // Removing 2 hands 3 and 10 to 1.
        assert_eq!(
            moved_children(&[id(2)], &hosts, &groups),
            vec![MovedChildren {
                group: id(2),
                children: vec![id(3), id(10)],
                moved_to: Some(id(1)),
            }]
        );
        // Removing 1 and 2: 3 and 10 leave 2 for the top level; 1 has no
        // other child.
        assert_eq!(
            moved_children(&[id(1), id(2)], &hosts, &groups),
            vec![MovedChildren {
                group: id(2),
                children: vec![id(3), id(10)],
                moved_to: None,
            }]
        );
        // Hosts and empty groups hand nothing up.
        assert!(moved_children(&[id(12), id(3)], &hosts, &groups)[0].children == vec![id(11)]);
        assert!(moved_children(&[id(12)], &hosts, &groups).is_empty());
    }
}
