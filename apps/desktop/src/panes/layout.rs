//! The pane tree of a session tab (UI 4.7) as plain data: binary splits that
//! nest, each with the share of its first child, and the geometry the view
//! and the Ctrl+Alt+arrow focus moves need.
//!
//! [`Layout`] is generic over its leaves: the live tree of a tab holds pane
//! ids ([`PaneId`]), the Restore snapshot holds the session targets
//! ([`super::PaneTarget`]). A split of the live tree is addressed by its
//! [`SplitPath`] from the root; the path of a divider stays the same while it
//! is dragged.
//!
//! On disk a tree has the pane shape of docs/data-model.md, section 12
//! (`{"direction", "sizes", "children"}` with 2 to 4 children, a leaf
//! `{"session": ...}`): runs of splits in the same direction are written as
//! one split with up to four children, and read back as a chain of binary
//! splits with the same geometry ([`Layout::to_wire`]).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Identity of a pane within its tab (never reused in the tab).
pub type PaneId = u64;

/// How a split lays out its two panes (docs/data-model.md, section 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Side by side: "Split vertically", the new pane on the right
    /// (Ctrl+Shift+D).
    Row,
    /// Stacked: "Split horizontally", the new pane below (Ctrl+Shift+E).
    Column,
}

/// The direction of a focus move (Ctrl+Alt+arrows, UI 4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Up,
    Down,
}

/// A rectangle in the tab's unit square (0..1 on both axes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const UNIT: Rect = Rect {
        x: 0.,
        y: 0.,
        w: 1.,
        h: 1.,
    };

    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// The two parts of a split of this rectangle.
    fn split(&self, direction: Direction, ratio: f32) -> (Rect, Rect) {
        match direction {
            Direction::Row => {
                let first = self.w * ratio;
                (
                    Rect { w: first, ..*self },
                    Rect {
                        x: self.x + first,
                        w: self.w - first,
                        ..*self
                    },
                )
            }
            Direction::Column => {
                let first = self.h * ratio;
                (
                    Rect { h: first, ..*self },
                    Rect {
                        y: self.y + first,
                        h: self.h - first,
                        ..*self
                    },
                )
            }
        }
    }
}

/// The share of a new split's first pane: the split pane is cut in half
/// (UI 4.7).
pub const EVEN: f32 = 0.5;

/// Shares closer to the edge than this are not kept (a guard against bad
/// snapshot values; the view applies the pixel minimum on top).
const MIN_SHARE: f32 = 0.05;

/// Where a split sits: the children taken from the root (`false` = first,
/// `true` = second).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SplitPath(pub Vec<bool>);

impl SplitPath {
    /// A stable text form for element ids: `r`, `r-0`, `r-0-1`, ...
    pub fn key(&self) -> String {
        let mut key = String::from("r");
        for step in &self.0 {
            key.push_str(if *step { "-1" } else { "-0" });
        }
        key
    }

    fn child(&self, second: bool) -> SplitPath {
        let mut path = self.0.clone();
        path.push(second);
        SplitPath(path)
    }
}

/// A divider of the laid-out tree: its split, its direction and where the
/// split's rectangle is.
#[derive(Clone, Debug, PartialEq)]
pub struct Divider {
    pub path: SplitPath,
    pub direction: Direction,
    /// The rectangle of the whole split (both panes).
    pub area: Rect,
    pub ratio: f32,
}

/// A pane tree: a leaf, or a split of two subtrees.
#[derive(Clone, Debug, PartialEq)]
pub enum Layout<T> {
    Split {
        direction: Direction,
        /// The share of `first` (0..1).
        ratio: f32,
        first: Box<Layout<T>>,
        second: Box<Layout<T>>,
    },
    Leaf {
        pane: T,
    },
}

impl<T> Layout<T> {
    pub fn leaf(pane: T) -> Self {
        Layout::Leaf { pane }
    }

    /// The leaves, left to right and top to bottom (depth first).
    pub fn leaves(&self) -> Vec<&T> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves<'a>(&'a self, out: &mut Vec<&'a T>) {
        match self {
            Layout::Leaf { pane } => out.push(pane),
            Layout::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    pub fn leaf_count(&self) -> usize {
        match self {
            Layout::Leaf { .. } => 1,
            Layout::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// The depth of the deepest leaf (a single leaf: 0).
    pub fn depth(&self) -> usize {
        match self {
            Layout::Leaf { .. } => 0,
            Layout::Split { first, second, .. } => 1 + first.depth().max(second.depth()),
        }
    }

    /// The same tree with every leaf mapped; `None` when `f` drops a leaf
    /// (its sibling then takes the split's place).
    pub fn filter_map<U>(&self, f: &mut impl FnMut(&T) -> Option<U>) -> Option<Layout<U>> {
        match self {
            Layout::Leaf { pane } => f(pane).map(Layout::leaf),
            Layout::Split {
                direction,
                ratio,
                first,
                second,
            } => match (first.filter_map(f), second.filter_map(f)) {
                (Some(first), Some(second)) => Some(Layout::Split {
                    direction: *direction,
                    ratio: *ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    /// The rectangle of every leaf in the unit square.
    pub fn rects(&self) -> Vec<(&T, Rect)> {
        let mut out = Vec::new();
        self.collect_rects(Rect::UNIT, &mut out);
        out
    }

    fn collect_rects<'a>(&'a self, area: Rect, out: &mut Vec<(&'a T, Rect)>) {
        match self {
            Layout::Leaf { pane } => out.push((pane, area)),
            Layout::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (a, b) = area.split(*direction, *ratio);
                first.collect_rects(a, out);
                second.collect_rects(b, out);
            }
        }
    }

    /// Every split with its rectangle (dividers to draw and drag).
    pub fn dividers(&self) -> Vec<Divider> {
        let mut out = Vec::new();
        self.collect_dividers(SplitPath::default(), Rect::UNIT, &mut out);
        out
    }

    fn collect_dividers(&self, path: SplitPath, area: Rect, out: &mut Vec<Divider>) {
        if let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = self
        {
            out.push(Divider {
                path: path.clone(),
                direction: *direction,
                area,
                ratio: *ratio,
            });
            let (a, b) = area.split(*direction, *ratio);
            first.collect_dividers(path.child(false), a, out);
            second.collect_dividers(path.child(true), b, out);
        }
    }

    /// The split at `path`, if any.
    fn split_at_mut(&mut self, path: &SplitPath) -> Option<&mut Layout<T>> {
        let mut node = self;
        for second in &path.0 {
            node = match node {
                Layout::Split {
                    first, second: s, ..
                } => {
                    if *second {
                        s
                    } else {
                        first
                    }
                }
                Layout::Leaf { .. } => return None,
            };
        }
        matches!(node, Layout::Split { .. }).then_some(node)
    }

    /// Sets the share of the first pane of the split at `path` (kept
    /// between 5 % and 95 %). Returns whether it changed.
    pub fn set_ratio(&mut self, path: &SplitPath, value: f32) -> bool {
        let value = clamp_share(value);
        match self.split_at_mut(path) {
            Some(Layout::Split { ratio, .. }) if (*ratio - value).abs() > f32::EPSILON => {
                *ratio = value;
                true
            }
            _ => false,
        }
    }

    /// Keeps every share inside the allowed range (snapshots from disk).
    pub fn sanitize(&mut self) {
        if let Layout::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            *ratio = clamp_share(*ratio);
            first.sanitize();
            second.sanitize();
        }
    }

    /// The node at `path` (a split or a leaf), if any.
    fn node(&self, path: &SplitPath) -> Option<&Layout<T>> {
        let mut node = self;
        for second in &path.0 {
            node = match node {
                Layout::Split {
                    first, second: s, ..
                } => {
                    if *second {
                        s
                    } else {
                        first
                    }
                }
                Layout::Leaf { .. } => return None,
            };
        }
        Some(node)
    }

    /// The smallest extent of this subtree along `direction`'s axis (width
    /// for [`Direction::Row`], height for [`Direction::Column`]) that keeps
    /// every pane at least `min` at the current shares.
    pub fn min_extent(&self, direction: Direction, min: f32) -> f32 {
        match self {
            Layout::Leaf { .. } => min,
            Layout::Split {
                direction: own,
                ratio,
                first,
                second,
            } => {
                let a = first.min_extent(direction, min);
                let b = second.min_extent(direction, min);
                if *own == direction {
                    (a / ratio.max(f32::EPSILON)).max(b / (1. - ratio).max(f32::EPSILON))
                } else {
                    a.max(b)
                }
            }
        }
    }

    /// The range of shares the split at `path` may take when its area is
    /// `extent` long along its axis, so that every pane of both sides keeps
    /// at least `min` (pixels, at the current inner shares), within
    /// 5 %..95 %. `None` when there is no split at `path` or no share fits.
    pub fn share_bounds(&self, path: &SplitPath, extent: f32, min: f32) -> Option<(f32, f32)> {
        let Some(Layout::Split {
            direction,
            first,
            second,
            ..
        }) = self.node(path)
        else {
            return None;
        };
        if extent <= 0. {
            return None;
        }
        let low = (first.min_extent(*direction, min) / extent).max(MIN_SHARE);
        let high = (1. - second.min_extent(*direction, min) / extent).min(1. - MIN_SHARE);
        (low <= high).then_some((low, high))
    }
}

/// A share inside 5 %..95 %; not a number: even.
fn clamp_share(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_SHARE, 1. - MIN_SHARE)
    } else {
        EVEN
    }
}

impl<T: PartialEq + Clone> Layout<T> {
    pub fn contains(&self, target: &T) -> bool {
        self.leaves().into_iter().any(|pane| pane == target)
    }

    /// Splits the leaf `target` in two even parts: `target` stays first,
    /// `new` comes second (right or below). Returns whether `target` was found.
    pub fn split(&mut self, target: &T, direction: Direction, new: T) -> bool {
        let mut new = Some(new);
        self.split_inner(target, direction, &mut new)
    }

    fn split_inner(&mut self, target: &T, direction: Direction, new: &mut Option<T>) -> bool {
        match self {
            Layout::Leaf { pane } if pane == target => {
                let Some(new) = new.take() else {
                    return false;
                };
                let first = Layout::leaf(pane.clone());
                *self = Layout::Split {
                    direction,
                    ratio: EVEN,
                    first: Box::new(first),
                    second: Box::new(Layout::leaf(new)),
                };
                true
            }
            Layout::Leaf { .. } => false,
            Layout::Split { first, second, .. } => {
                first.split_inner(target, direction, new)
                    || second.split_inner(target, direction, new)
            }
        }
    }
}

/// What [`Layout::remove`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Removed<T> {
    /// No such pane.
    NotFound,
    /// The pane is gone; its sibling took its space. `next` is the pane that
    /// should become active: the sibling's pane next to the removed one.
    Removed { next: T },
    /// It was the only pane: the tree is empty (the caller drops it).
    Last,
}

impl<T: PartialEq + Clone> Layout<T> {
    /// Removes the leaf `target`; its sibling subtree takes the split's
    /// place (UI 4.7: the neighbors take its space).
    pub fn remove(&mut self, target: &T) -> Removed<T> {
        if let Layout::Leaf { pane } = self {
            return if pane == target {
                Removed::Last
            } else {
                Removed::NotFound
            };
        }
        match self.remove_inner(target) {
            Some(next) => Removed::Removed { next },
            None => Removed::NotFound,
        }
    }

    fn remove_inner(&mut self, target: &T) -> Option<T> {
        let Layout::Split { first, second, .. } = self else {
            return None;
        };
        let first_is_target = matches!(first.as_ref(), Layout::Leaf { pane } if pane == target);
        let second_is_target = matches!(second.as_ref(), Layout::Leaf { pane } if pane == target);
        if first_is_target || second_is_target {
            // The sibling's pane next to the removed one: its first pane
            // when the removed one was first (left or above), else its last.
            let sibling = if first_is_target {
                std::mem::replace(second.as_mut(), Layout::leaf(target.clone()))
            } else {
                std::mem::replace(first.as_mut(), Layout::leaf(target.clone()))
            };
            let next = if first_is_target {
                sibling.leaves().first().map(|pane| (*pane).clone())
            } else {
                sibling.leaves().last().map(|pane| (*pane).clone())
            };
            *self = sibling;
            return next;
        }
        first
            .remove_inner(target)
            .or_else(|| second.remove_inner(target))
    }

    /// The nearest pane on `side` of `from` on screen (UI 4.7; Ctrl+Alt+
    /// arrows): among the panes that touch the far edge of `from` and
    /// overlap it across, the one that covers the middle of `from`, else the
    /// one that overlaps it most. `None` at the edge of the tab.
    pub fn neighbor(&self, from: &T, side: Side) -> Option<T> {
        const EPS: f32 = 1e-4;
        let rects = self.rects();
        let (_, origin) = rects.iter().find(|(pane, _)| *pane == from)?;
        let origin = *origin;
        let candidates = rects.iter().filter_map(|(pane, rect)| {
            if *pane == from {
                return None;
            }
            let (gap, overlap, covers_middle) = match side {
                Side::Right | Side::Left => {
                    let gap = if side == Side::Right {
                        rect.x - origin.right()
                    } else {
                        origin.x - rect.right()
                    };
                    let overlap = rect.bottom().min(origin.bottom()) - rect.y.max(origin.y);
                    let middle = origin.y + origin.h / 2.;
                    (gap, overlap, rect.y <= middle && middle <= rect.bottom())
                }
                Side::Down | Side::Up => {
                    let gap = if side == Side::Down {
                        rect.y - origin.bottom()
                    } else {
                        origin.y - rect.bottom()
                    };
                    let overlap = rect.right().min(origin.right()) - rect.x.max(origin.x);
                    let middle = origin.x + origin.w / 2.;
                    (gap, overlap, rect.x <= middle && middle <= rect.right())
                }
            };
            (gap > -EPS && overlap > EPS).then_some((gap, covers_middle, overlap, (*pane).clone()))
        });
        candidates
            .min_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.1.cmp(&a.1))
                    .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
            })
            .map(|(_, _, _, pane)| pane)
    }
}

/// Children of one split on disk (docs/data-model.md, section 12).
const WIRE_MAX_CHILDREN: usize = 4;

/// A pane tree in the shape of docs/data-model.md, section 12.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Wire<T> {
    Split {
        direction: Direction,
        sizes: Vec<f64>,
        children: Vec<Wire<T>>,
    },
    Leaf {
        session: T,
    },
}

impl<T> Wire<T> {
    fn depth(&self) -> usize {
        match self {
            Wire::Leaf { .. } => 0,
            Wire::Split { children, .. } => 1 + children.iter().map(Wire::depth).max().unwrap_or(0),
        }
    }
}

impl<T> Layout<T> {
    /// The on-disk form: a run of splits in the same direction becomes one
    /// split with up to four children (the shares multiplied out).
    fn to_wire(&self) -> Wire<&T> {
        match self {
            Layout::Leaf { pane } => Wire::Leaf { session: pane },
            Layout::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let mut items: Vec<(&Layout<T>, f32)> =
                    vec![(first.as_ref(), *ratio), (second.as_ref(), 1. - *ratio)];
                // Opens nested splits of the same direction, left to right,
                // while the split keeps at most four children.
                let mut index = 0;
                while index < items.len() {
                    match items[index] {
                        (
                            Layout::Split {
                                direction: inner,
                                ratio: inner_ratio,
                                first,
                                second,
                            },
                            size,
                        ) if inner == direction && items.len() < WIRE_MAX_CHILDREN => {
                            items.splice(
                                index..=index,
                                [
                                    (first.as_ref(), size * inner_ratio),
                                    (second.as_ref(), size * (1. - inner_ratio)),
                                ],
                            );
                        }
                        _ => index += 1,
                    }
                }
                Wire::Split {
                    direction: *direction,
                    sizes: items.iter().map(|(_, size)| wire_size(*size)).collect(),
                    children: items.iter().map(|(node, _)| node.to_wire()).collect(),
                }
            }
        }
    }

    /// The tree of an on-disk form: a split of n children becomes a chain
    /// of binary splits (the first child, then the rest), the shares kept
    /// inside 5 %..95 %. `None` for a split without children or with a
    /// size per child missing.
    fn from_wire(wire: Wire<T>) -> Option<Self> {
        match wire {
            Wire::Leaf { session } => Some(Layout::leaf(session)),
            Wire::Split {
                direction,
                sizes,
                children,
            } => {
                if children.is_empty() || sizes.len() != children.len() {
                    return None;
                }
                let sizes: Vec<f32> = sizes
                    .into_iter()
                    .map(|size| {
                        if size.is_finite() && size > 0. {
                            size as f32
                        } else {
                            0.
                        }
                    })
                    .collect();
                let mut nodes = children
                    .into_iter()
                    .map(Layout::from_wire)
                    .collect::<Option<Vec<_>>>()?;
                // Folds from the right: the last child, then each one before
                // it with its share of what is left.
                let mut tree = nodes.pop()?;
                let mut rest: f32 = sizes.last().copied().unwrap_or(0.);
                for (node, size) in nodes.into_iter().zip(sizes.iter().copied()).rev() {
                    let total = size + rest;
                    let ratio = if total > 0. { size / total } else { EVEN };
                    tree = Layout::Split {
                        direction,
                        ratio: clamp_share(ratio),
                        first: Box::new(node),
                        second: Box::new(tree),
                    };
                    rest = total;
                }
                Some(tree)
            }
        }
    }

    /// The split depth of the on-disk form (a leaf: 0).
    pub fn wire_depth(&self) -> usize {
        self.to_wire().depth()
    }
}

/// A share as written: four decimals (the shares of a split still add up
/// to 1 within 0.001, docs/data-model.md, section 12).
fn wire_size(share: f32) -> f64 {
    (f64::from(share) * 10_000.).round() / 10_000.
}

impl<T: Serialize> Serialize for Layout<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_wire().serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Layout<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = Wire::<T>::deserialize(deserializer)?;
        Layout::from_wire(wire)
            .ok_or_else(|| serde::de::Error::custom("a split needs one size per child"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn four() -> Layout<PaneId> {
        // 1 | 2       split 1 right (2), then 1 down (3), then 2 down (4):
        // --+--
        // 3 | 4
        let mut tree = Layout::leaf(1);
        assert!(tree.split(&1, Direction::Row, 2));
        assert!(tree.split(&1, Direction::Column, 3));
        assert!(tree.split(&2, Direction::Column, 4));
        tree
    }

    #[test]
    fn splits_halve_the_pane_and_nest() {
        let tree = four();
        assert_eq!(tree.leaves(), vec![&1, &3, &2, &4]);
        assert_eq!(tree.leaf_count(), 4);
        assert_eq!(tree.depth(), 2);
        let rects: Vec<(PaneId, Rect)> = tree.rects().into_iter().map(|(p, r)| (*p, r)).collect();
        let rect = |id| rects.iter().find(|(p, _)| *p == id).unwrap().1;
        assert_eq!(
            rect(1),
            Rect {
                x: 0.,
                y: 0.,
                w: 0.5,
                h: 0.5
            }
        );
        assert_eq!(
            rect(4),
            Rect {
                x: 0.5,
                y: 0.5,
                w: 0.5,
                h: 0.5
            }
        );
        // An unknown pane is not split.
        let mut copy = tree.clone();
        assert!(!copy.split(&9, Direction::Row, 10));
        assert_eq!(copy, tree);
    }

    #[test]
    fn removing_lets_the_sibling_take_the_space() {
        let mut tree = four();
        // 3 goes: 1 takes the whole left column and becomes active.
        assert_eq!(tree.remove(&3), Removed::Removed { next: 1 });
        assert_eq!(tree.leaves(), vec![&1, &2, &4]);
        let left = tree.rects().into_iter().find(|(p, _)| **p == 1).unwrap().1;
        assert_eq!(left.h, 1.);
        // 1 goes (first of the root): the right column's first pane is next.
        assert_eq!(tree.remove(&1), Removed::Removed { next: 2 });
        assert_eq!(tree.leaves(), vec![&2, &4]);
        assert_eq!(tree.remove(&7), Removed::NotFound);
        assert_eq!(tree.remove(&4), Removed::Removed { next: 2 });
        assert_eq!(tree, Layout::leaf(2));
        assert_eq!(tree.remove(&2), Removed::Last);
    }

    #[test]
    fn neighbors_follow_the_screen_geometry() {
        let tree = four();
        assert_eq!(tree.neighbor(&1, Side::Right), Some(2));
        assert_eq!(tree.neighbor(&1, Side::Down), Some(3));
        assert_eq!(tree.neighbor(&1, Side::Left), None);
        assert_eq!(tree.neighbor(&1, Side::Up), None);
        assert_eq!(tree.neighbor(&4, Side::Left), Some(3));
        assert_eq!(tree.neighbor(&4, Side::Up), Some(2));
        assert_eq!(tree.neighbor(&3, Side::Right), Some(4));
        // A tall pane next to two stacked ones: the one at its middle wins,
        // then the larger overlap.
        let mut tall = Layout::leaf(1);
        tall.split(&1, Direction::Row, 2);
        tall.split(&2, Direction::Column, 3);
        tall.set_ratio(&SplitPath(vec![true]), 0.7);
        // 2 covers 0..0.7 of the height, 3 the rest; the middle (0.5) is in 2.
        assert_eq!(tall.neighbor(&1, Side::Right), Some(2));
        assert_eq!(tall.neighbor(&3, Side::Left), Some(1));
        assert_eq!(tall.neighbor(&2, Side::Left), Some(1));
        assert_eq!(tall.neighbor(&9, Side::Left), None);
    }

    #[test]
    fn ratios_are_kept_inside_the_tab() {
        let mut tree = four();
        let root = SplitPath::default();
        assert!(tree.set_ratio(&root, 0.3));
        assert!(!tree.set_ratio(&root, 0.3), "unchanged");
        assert!(tree.set_ratio(&root, -2.));
        assert_eq!(tree.dividers()[0].ratio, MIN_SHARE);
        assert!(tree.set_ratio(&root, f32::NAN));
        assert_eq!(tree.dividers()[0].ratio, EVEN);
        // A path to a leaf is not a split.
        assert!(!tree.set_ratio(&SplitPath(vec![false, false]), 0.4));
        let dividers = tree.dividers();
        assert_eq!(dividers.len(), 3);
        assert_eq!(dividers[0].path.key(), "r");
        assert_eq!(dividers[1].path.key(), "r-0");
        assert_eq!(dividers[2].path.key(), "r-1");
        assert_eq!(dividers[1].direction, Direction::Column);
    }

    #[test]
    fn snapshots_round_trip_and_drop_missing_leaves() {
        let tree = four();
        let mapped = tree
            .filter_map(&mut |pane| (*pane != 3).then(|| format!("p{pane}")))
            .unwrap();
        assert_eq!(mapped.leaves(), vec!["p1", "p2", "p4"]);
        let json = serde_json::to_value(&mapped).unwrap();
        let back: Layout<String> = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, mapped);
        // The shape of docs/data-model.md, section 12.
        assert_eq!(
            json,
            serde_json::json!({"direction": "row", "sizes": [0.5, 0.5], "children": [
                {"session": "p1"},
                {"direction": "column", "sizes": [0.5, 0.5], "children": [
                    {"session": "p2"}, {"session": "p4"}
                ]}
            ]})
        );
        assert!(tree.filter_map(&mut |_| None::<u8>).is_none());
    }

    #[test]
    fn runs_of_one_direction_are_one_split_on_disk() {
        // a | b | c | d | e: four children at most, then a nested split.
        let mut tree = Layout::leaf('a');
        tree.split(&'a', Direction::Row, 'b');
        tree.split(&'b', Direction::Row, 'c');
        tree.split(&'c', Direction::Row, 'd');
        tree.split(&'d', Direction::Row, 'e');
        assert_eq!(tree.depth(), 4);
        assert_eq!(tree.wire_depth(), 2);
        let json = serde_json::to_value(&tree).unwrap();
        assert_eq!(json["children"].as_array().unwrap().len(), 4);
        let sizes: Vec<f64> = json["sizes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|size| size.as_f64().unwrap())
            .collect();
        assert!((sizes.iter().sum::<f64>() - 1.).abs() < 1e-6);
        assert_eq!(sizes[0], 0.5);
        // Back: the same panes at the same places.
        let back: Layout<char> = serde_json::from_value(json).unwrap();
        assert_eq!(back.leaves(), tree.leaves());
        let close = |a: Rect, b: Rect| {
            (a.x - b.x).abs() < 1e-5
                && (a.y - b.y).abs() < 1e-5
                && (a.w - b.w).abs() < 1e-5
                && (a.h - b.h).abs() < 1e-5
        };
        for ((pane, rect), (other, back_rect)) in tree.rects().into_iter().zip(back.rects()) {
            assert_eq!(pane, other);
            assert!(close(rect, back_rect), "{pane}: {rect:?} vs {back_rect:?}");
        }
        // A split nested on the left side opens too (same geometry).
        let mut left = Layout::leaf(1);
        left.split(&1, Direction::Column, 3);
        left.split(&1, Direction::Column, 2);
        assert_eq!(left.wire_depth(), 1);
        assert_eq!(
            serde_json::to_value(&left).unwrap(),
            serde_json::json!({"direction": "column", "sizes": [0.25, 0.25, 0.5], "children": [
                {"session": 1}, {"session": 2}, {"session": 3}
            ]})
        );
    }

    #[test]
    fn bad_snapshots_are_kept_inside_the_tab_or_refused() {
        // Shares that do not add up are normalized, and kept within 5 %.
        let tree: Layout<String> = serde_json::from_value(serde_json::json!({
            "direction": "column", "sizes": [7.0, 0.0],
            "children": [{"session": "a"}, {"session": "b"}]
        }))
        .unwrap();
        assert_eq!(tree.dividers()[0].ratio, 1. - MIN_SHARE);
        let one: Layout<String> = serde_json::from_value(serde_json::json!({
            "direction": "row", "sizes": [1.0], "children": [{"session": "a"}]
        }))
        .unwrap();
        assert_eq!(one, Layout::leaf("a".to_owned()));
        for bad in [
            serde_json::json!({"direction": "row", "sizes": [], "children": []}),
            serde_json::json!({"direction": "row", "sizes": [0.5], "children": [{"session": "a"}, {"session": "b"}]}),
            serde_json::json!({"direction": "diagonal", "sizes": [0.5, 0.5], "children": [{"session": "a"}, {"session": "b"}]}),
            serde_json::json!({"pane": "a"}),
        ] {
            assert!(serde_json::from_value::<Layout<String>>(bad).is_err());
        }
    }

    #[test]
    fn the_pixel_minimum_bounds_the_shares() {
        let mut tree = four();
        let root = SplitPath::default();
        // 1000 px wide, 100 px minimum: 10 %..90 %.
        let (low, high) = tree.share_bounds(&root, 1000., 100.).unwrap();
        assert!((low - 0.1).abs() < 1e-6 && (high - 0.9).abs() < 1e-6);
        // Column splits on each side do not narrow the row's range.
        assert_eq!(tree.min_extent(Direction::Row, 100.), 200.);
        assert_eq!(tree.min_extent(Direction::Column, 100.), 200.);
        // A nested row on the right at 25 %: its first pane needs the right
        // side to be 400 px.
        tree.split(&4, Direction::Row, 5);
        tree.set_ratio(&SplitPath(vec![true, true]), 0.25);
        let (_, high) = tree.share_bounds(&root, 1000., 100.).unwrap();
        assert!((high - 0.6).abs() < 1e-6, "{high}");
        // Too small for both sides, or not a split.
        assert_eq!(tree.share_bounds(&root, 300., 100.), None);
        assert_eq!(
            tree.share_bounds(&SplitPath(vec![false, false]), 1000., 100.),
            None
        );
        assert_eq!(tree.share_bounds(&root, 0., 100.), None);
    }
}
