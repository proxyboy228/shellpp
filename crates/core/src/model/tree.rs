//! The resolved group tree (docs/data-model.md, section 7.1).
//!
//! Sync can produce a cycle or a missing parent. While resolving, a group whose
//! parent is missing (absent, not a live group) is top level, and in a cycle
//! the group with the greatest `(hlc, device_id)` is top level. No record is
//! rewritten; every device computes the same tree from the same records.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::model::hlc::Hlc;

/// Maximum nesting depth of groups when writing.
pub const MAX_DEPTH: usize = 32;

/// The input of one group.
#[derive(Clone, Copy, Debug)]
pub struct GroupNode {
    pub id: Uuid,
    /// The raw `parent_id` member.
    pub parent_id: Option<Uuid>,
    pub hlc: Hlc,
    pub device_id: Uuid,
}

/// Parent links after the cycle and missing-parent rules.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    parent: HashMap<Uuid, Option<Uuid>>,
    children: HashMap<Option<Uuid>, Vec<Uuid>>,
}

impl Tree {
    pub fn build(nodes: &[GroupNode]) -> Self {
        let known: HashMap<Uuid, &GroupNode> = nodes.iter().map(|node| (node.id, node)).collect();
        let mut parent: HashMap<Uuid, Option<Uuid>> = nodes
            .iter()
            .map(|node| {
                let parent = node
                    .parent_id
                    .filter(|parent| *parent != node.id && known.contains_key(parent));
                (node.id, parent)
            })
            .collect();
        // Self-references count as a one-node cycle and are already top level.

        // Break cycles: walk each chain; a node met again on the current walk
        // closes a cycle whose greatest (hlc, device_id) member becomes top level.
        let mut done: HashSet<Uuid> = HashSet::new();
        let mut ids: Vec<Uuid> = nodes.iter().map(|node| node.id).collect();
        ids.sort();
        for start in ids {
            if done.contains(&start) {
                continue;
            }
            let mut walk: Vec<Uuid> = Vec::new();
            let mut on_walk: HashSet<Uuid> = HashSet::new();
            let mut current = Some(start);
            while let Some(id) = current {
                if done.contains(&id) {
                    break;
                }
                if on_walk.contains(&id) {
                    let position = walk.iter().position(|item| *item == id).unwrap_or(0);
                    let cycle = &walk[position..];
                    let winner = cycle
                        .iter()
                        .copied()
                        .max_by(|a, b| {
                            let a = known[a];
                            let b = known[b];
                            (a.hlc, a.device_id.as_bytes()).cmp(&(b.hlc, b.device_id.as_bytes()))
                        })
                        .unwrap_or(id);
                    parent.insert(winner, None);
                    break;
                }
                walk.push(id);
                on_walk.insert(id);
                current = parent.get(&id).copied().flatten();
            }
            done.extend(walk);
        }

        let mut children: HashMap<Option<Uuid>, Vec<Uuid>> = HashMap::new();
        for (id, parent) in &parent {
            children.entry(*parent).or_default().push(*id);
        }
        for list in children.values_mut() {
            list.sort();
        }
        Self { parent, children }
    }

    /// `true` for a live group.
    pub fn contains(&self, id: &Uuid) -> bool {
        self.parent.contains_key(id)
    }

    /// The resolved parent (`None` at top level or for an unknown id).
    pub fn parent(&self, id: &Uuid) -> Option<Uuid> {
        self.parent.get(id).copied().flatten()
    }

    /// Direct child groups of `parent` (`None` = top level), sorted by id.
    pub fn children(&self, parent: Option<Uuid>) -> &[Uuid] {
        self.children.get(&parent).map_or(&[], Vec::as_slice)
    }

    /// Ancestors of `id`, top level first, ending with its parent.
    pub fn path(&self, id: &Uuid) -> Vec<Uuid> {
        let mut path = Vec::new();
        let mut current = self.parent(id);
        while let Some(group) = current {
            if path.contains(&group) {
                break;
            }
            path.push(group);
            current = self.parent(&group);
        }
        path.reverse();
        path
    }

    /// Ancestors of a host in `group_id` (the group itself last), or empty
    /// when the group is missing.
    pub fn host_path(&self, group_id: Option<Uuid>) -> Vec<Uuid> {
        match group_id.filter(|id| self.contains(id)) {
            Some(group) => {
                let mut path = self.path(&group);
                path.push(group);
                path
            }
            None => Vec::new(),
        }
    }

    /// `id`, then its parent, grandparent and so on (at most `limit` groups).
    pub fn chain(&self, id: Uuid, limit: usize) -> Vec<Uuid> {
        let mut chain = Vec::new();
        let mut current = Some(id).filter(|id| self.contains(id));
        while let Some(group) = current {
            if chain.len() >= limit || chain.contains(&group) {
                break;
            }
            chain.push(group);
            current = self.parent(&group);
        }
        chain
    }

    /// `true` when `node` is `root` or inside its subtree.
    pub fn in_subtree(&self, node: &Uuid, root: &Uuid) -> bool {
        node == root || self.path(node).contains(root)
    }

    /// 1 for a top-level group.
    pub fn depth(&self, id: &Uuid) -> usize {
        self.path(id).len() + 1
    }

    /// 1 for a group without subgroups.
    pub fn height(&self, id: &Uuid) -> usize {
        let mut best = 1;
        let mut stack: Vec<(Uuid, usize)> = vec![(*id, 1)];
        let mut seen: HashSet<Uuid> = HashSet::new();
        while let Some((group, level)) = stack.pop() {
            if !seen.insert(group) {
                continue;
            }
            best = best.max(level);
            for child in self.children(Some(group)) {
                stack.push((*child, level + 1));
            }
        }
        best
    }

    /// `id` and every group below it.
    pub fn subtree(&self, id: &Uuid) -> Vec<Uuid> {
        let mut out = Vec::new();
        let mut stack = vec![*id];
        let mut seen: HashSet<Uuid> = HashSet::new();
        while let Some(group) = stack.pop() {
            if !seen.insert(group) {
                continue;
            }
            out.push(group);
            stack.extend_from_slice(self.children(Some(group)));
        }
        out
    }

    /// The nearest ancestor of `id` (excluding itself) that is not in `skip`.
    pub fn nearest_ancestor_outside(&self, id: &Uuid, skip: &HashSet<Uuid>) -> Option<Uuid> {
        let mut current = self.parent(id);
        let mut steps = 0;
        while let Some(group) = current {
            if !skip.contains(&group) {
                return Some(group);
            }
            steps += 1;
            if steps > self.parent.len() {
                return None;
            }
            current = self.parent(&group);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn node(n: u128, parent: Option<u128>, hlc: u64) -> GroupNode {
        GroupNode {
            id: id(n),
            parent_id: parent.map(id),
            hlc: Hlc::new(hlc, 0),
            device_id: id(0xd),
        }
    }

    #[test]
    fn paths_depths_and_subtrees() {
        let tree = Tree::build(&[
            node(1, None, 1),
            node(2, Some(1), 1),
            node(3, Some(2), 1),
            node(4, Some(1), 1),
        ]);
        assert_eq!(tree.path(&id(3)), vec![id(1), id(2)]);
        assert_eq!(tree.host_path(Some(id(3))), vec![id(1), id(2), id(3)]);
        assert_eq!(tree.host_path(Some(id(99))), Vec::<Uuid>::new());
        assert_eq!(tree.depth(&id(1)), 1);
        assert_eq!(tree.depth(&id(3)), 3);
        assert_eq!(tree.height(&id(1)), 3);
        assert_eq!(tree.height(&id(3)), 1);
        assert!(tree.in_subtree(&id(3), &id(1)));
        assert!(tree.in_subtree(&id(1), &id(1)));
        assert!(!tree.in_subtree(&id(1), &id(3)));
        let mut sub = tree.subtree(&id(1));
        sub.sort();
        assert_eq!(sub, vec![id(1), id(2), id(3), id(4)]);
        assert_eq!(tree.children(None), &[id(1)]);
        assert_eq!(tree.children(Some(id(1))), &[id(2), id(4)]);
        assert_eq!(tree.chain(id(3), 32), vec![id(3), id(2), id(1)]);
        assert_eq!(tree.chain(id(3), 2), vec![id(3), id(2)]);
    }

    #[test]
    fn missing_parent_is_top_level() {
        let tree = Tree::build(&[node(1, Some(42), 1), node(2, Some(2), 1)]);
        assert_eq!(tree.parent(&id(1)), None);
        assert_eq!(tree.parent(&id(2)), None);
    }

    #[test]
    fn cycle_breaks_at_greatest_stamp() {
        // Parent links 1 -> 3 -> 2 -> 1; group 2 has the greatest HLC.
        let tree = Tree::build(&[
            node(1, Some(3), 5),
            node(2, Some(1), 9),
            node(3, Some(2), 7),
            node(4, Some(3), 1),
        ]);
        assert_eq!(tree.parent(&id(2)), None);
        assert_eq!(tree.parent(&id(3)), Some(id(2)));
        assert_eq!(tree.parent(&id(1)), Some(id(3)));
        assert_eq!(tree.path(&id(4)), vec![id(2), id(3)]);
        assert_eq!(tree.path(&id(1)), vec![id(2), id(3)]);
        // The same records give the same tree in any input order.
        let reversed = Tree::build(&[
            node(4, Some(3), 1),
            node(3, Some(2), 7),
            node(2, Some(1), 9),
            node(1, Some(3), 5),
        ]);
        for n in 1..=4 {
            assert_eq!(tree.parent(&id(n)), reversed.parent(&id(n)));
        }
    }

    #[test]
    fn cycle_tie_uses_device_id() {
        let mut a = node(1, Some(2), 5);
        let mut b = node(2, Some(1), 5);
        a.device_id = id(0x20);
        b.device_id = id(0x10);
        let tree = Tree::build(&[a, b]);
        assert_eq!(tree.parent(&id(1)), None);
        assert_eq!(tree.parent(&id(2)), Some(id(1)));
    }

    #[test]
    fn nearest_ancestor_skips_removed_groups() {
        let tree = Tree::build(&[node(1, None, 1), node(2, Some(1), 1), node(3, Some(2), 1)]);
        let skip: HashSet<Uuid> = [id(2)].into_iter().collect();
        assert_eq!(tree.nearest_ancestor_outside(&id(3), &skip), Some(id(1)));
        let skip: HashSet<Uuid> = [id(1), id(2)].into_iter().collect();
        assert_eq!(tree.nearest_ancestor_outside(&id(3), &skip), None);
    }
}
