//! Selection of records on a screen (UI 2.3: single click selects, Ctrl
//! toggles, Shift selects a range from the anchor in screen order).

use uuid::Uuid;

/// How a click changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Plain click: only this record.
    Replace,
    /// Ctrl+click (Cmd on macOS): add or remove this record.
    Toggle,
    /// Shift+click: everything between the anchor and this record, in the
    /// order the screen shows them.
    Range,
}

/// The selected record ids, in selection order, and the range anchor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<Uuid>,
    anchor: Option<Uuid>,
}

impl Selection {
    pub fn ids(&self) -> &[Uuid] {
        &self.ids
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn contains(&self, id: &Uuid) -> bool {
        self.ids.contains(id)
    }

    /// The only selected record, if exactly one is selected.
    pub fn single(&self) -> Option<Uuid> {
        match self.ids.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }

    pub fn anchor(&self) -> Option<Uuid> {
        self.anchor
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }

    /// Applies a click on `id`. `order` is the screen order, used by
    /// [`SelectMode::Range`]; without an anchor in `order`, a range click
    /// acts like a plain click.
    pub fn apply(&mut self, id: Uuid, mode: SelectMode, order: &[Uuid]) {
        match mode {
            SelectMode::Replace => {
                self.ids = vec![id];
                self.anchor = Some(id);
            }
            SelectMode::Toggle => {
                if let Some(position) = self.ids.iter().position(|selected| *selected == id) {
                    self.ids.remove(position);
                } else {
                    self.ids.push(id);
                }
                self.anchor = Some(id);
            }
            SelectMode::Range => {
                let from = self
                    .anchor
                    .and_then(|anchor| order.iter().position(|item| *item == anchor));
                let to = order.iter().position(|item| *item == id);
                match (from, to) {
                    (Some(from), Some(to)) => {
                        let (start, end) = if from <= to { (from, to) } else { (to, from) };
                        self.ids = order[start..=end].to_vec();
                    }
                    _ => self.apply(id, SelectMode::Replace, order),
                }
            }
        }
    }

    /// Drops ids that no longer exist (after a reload or a delete).
    pub fn retain(&mut self, exists: impl Fn(&Uuid) -> bool) {
        self.ids.retain(|id| exists(id));
        if self.anchor.is_some_and(|anchor| !exists(&anchor)) {
            self.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<Uuid> {
        (0..n).map(|_| Uuid::now_v7()).collect()
    }

    #[test]
    fn replace_toggle_and_range() {
        let order = ids(5);
        let mut selection = Selection::default();
        selection.apply(order[1], SelectMode::Replace, &order);
        assert_eq!(selection.ids(), &[order[1]]);
        assert_eq!(selection.single(), Some(order[1]));

        selection.apply(order[3], SelectMode::Toggle, &order);
        assert_eq!(selection.ids(), &[order[1], order[3]]);
        assert_eq!(selection.single(), None);
        selection.apply(order[3], SelectMode::Toggle, &order);
        assert_eq!(selection.ids(), &[order[1]]);

        selection.apply(order[1], SelectMode::Replace, &order);
        selection.apply(order[4], SelectMode::Range, &order);
        assert_eq!(selection.ids(), &order[1..=4]);
        selection.apply(order[0], SelectMode::Range, &order);
        assert_eq!(selection.ids(), &order[0..=1]);
    }

    #[test]
    fn range_without_anchor_selects_one() {
        let order = ids(3);
        let mut selection = Selection::default();
        selection.apply(order[2], SelectMode::Range, &order);
        assert_eq!(selection.ids(), &[order[2]]);
    }

    #[test]
    fn retain_drops_missing_ids_and_anchor() {
        let order = ids(3);
        let mut selection = Selection::default();
        selection.apply(order[0], SelectMode::Replace, &order);
        selection.apply(order[1], SelectMode::Toggle, &order);
        selection.retain(|id| *id != order[1]);
        assert_eq!(selection.ids(), &[order[0]]);
        assert_eq!(selection.anchor(), None);
        selection.clear();
        assert!(selection.is_empty());
    }
}
