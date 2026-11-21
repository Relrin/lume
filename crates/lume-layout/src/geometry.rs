//! Where the panes of a tab lie, without pixels.
//!
//! The layout knows ratios, not sizes, yet "the pane to the left of this one" is a
//! question about places. So a tab is laid out on a square that is [`UNITS`] wide and
//! high, with no gaps between panes, and the question is answered there. A page lays
//! the same tree out in a real window with the same ratios, so what is to the left
//! here is to the left there.

use std::cmp::Reverse;

use crate::ids::PaneId;
use crate::intent::Dir;
use crate::model::{Axis, Node, UnitRect};
use crate::tuning::UNITS;

/// The rectangle of each pane of a tree laid out on the whole square, in the order the
/// panes are laid out.
pub fn tab_rects(root: &Node) -> Vec<(PaneId, UnitRect)> {
    let mut rects = Vec::new();
    place(root, UnitRect::FULL, &mut rects);
    rects
}

fn place(node: &Node, rect: UnitRect, rects: &mut Vec<(PaneId, UnitRect)>) {
    let split = match node {
        Node::Pane(pane) => {
            rects.push((pane.id, rect));
            return;
        }
        Node::Split(split) => split,
    };
    let (start, length) = match split.axis {
        Axis::Row => (rect.x, rect.w),
        Axis::Column => (rect.y, rect.h),
    };
    let mut cursor = start;
    let last = split.children.len().saturating_sub(1);
    for (index, child) in split.children.iter().enumerate() {
        let ratio = split.ratios.get(index).copied().unwrap_or(0);
        // The last child takes what is left, so that rounding leaves no sliver unused.
        let share = if index == last {
            (start + length).saturating_sub(cursor)
        } else {
            (u64::from(length) * u64::from(ratio) / u64::from(UNITS)) as u32
        };
        let child_rect = match split.axis {
            Axis::Row => UnitRect {
                x: cursor,
                w: share,
                ..rect
            },
            Axis::Column => UnitRect {
                y: cursor,
                h: share,
                ..rect
            },
        };
        place(child, child_rect, rects);
        cursor = cursor.saturating_add(share);
    }
}

/// How much of two stretches of a line lies in both.
fn overlap(a_start: u32, a_end: u32, b_start: u32, b_end: u32) -> u32 {
    a_end.min(b_end).saturating_sub(a_start.max(b_start))
}

/// The pane next to `from` in a direction, among panes at the given rectangles.
///
/// A pane is next to another when it lies wholly beyond the other's edge and shares
/// some of that edge's extent. Of several, the nearest is taken; of equally near ones,
/// the one that had the keyboard most recently (`history`, most recent first), then
/// the one sharing most of the edge, then the one made first.
pub fn neighbour(
    rects: &[(PaneId, UnitRect)],
    from: PaneId,
    dir: Dir,
    history: &[PaneId],
) -> Option<PaneId> {
    let (_, origin) = rects.iter().find(|(pane, _)| *pane == from)?;
    rects
        .iter()
        .filter(|(pane, _)| *pane != from)
        .filter_map(|(pane, rect)| {
            let (distance, shared) = match dir {
                Dir::Left => (
                    origin.x.checked_sub(rect.right())?,
                    overlap(origin.y, origin.bottom(), rect.y, rect.bottom()),
                ),
                Dir::Right => (
                    rect.x.checked_sub(origin.right())?,
                    overlap(origin.y, origin.bottom(), rect.y, rect.bottom()),
                ),
                Dir::Up => (
                    origin.y.checked_sub(rect.bottom())?,
                    overlap(origin.x, origin.right(), rect.x, rect.right()),
                ),
                Dir::Down => (
                    rect.y.checked_sub(origin.bottom())?,
                    overlap(origin.x, origin.right(), rect.x, rect.right()),
                ),
            };
            if shared == 0 {
                return None;
            }
            let recency = history
                .iter()
                .position(|seen| seen == pane)
                .unwrap_or(usize::MAX);
            Some(((distance, recency, Reverse(shared), *pane), *pane))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, pane)| pane)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::SplitId;
    use crate::model::{Pane, PaneContent, Split, TerminalPane};

    fn pane(number: u32) -> Node {
        Node::Pane(Pane {
            id: PaneId::new(number),
            title: None,
            icon: None,
            content: PaneContent::Terminal(TerminalPane::default()),
        })
    }

    fn split(id: u32, axis: Axis, ratios: &[u32], children: Vec<Node>) -> Node {
        Node::Split(Split {
            id: SplitId::new(id),
            axis,
            ratios: ratios.to_vec(),
            children,
        })
    }

    /// Pane 1 on the left; on the right pane 2 above panes 3 and 4 side by side.
    fn four() -> Node {
        split(
            10,
            Axis::Row,
            &[400_000, 600_000],
            vec![
                pane(1),
                split(
                    11,
                    Axis::Column,
                    &[500_000, 500_000],
                    vec![
                        pane(2),
                        split(12, Axis::Row, &[500_000, 500_000], vec![pane(3), pane(4)]),
                    ],
                ),
            ],
        )
    }

    #[test]
    fn panes_fill_the_square_without_gaps_or_overlap() {
        let rects = tab_rects(&four());
        assert_eq!(rects.len(), 4);
        let area: u64 = rects
            .iter()
            .map(|(_, rect)| u64::from(rect.w) * u64::from(rect.h))
            .sum();
        assert_eq!(area, u64::from(UNITS) * u64::from(UNITS));
        assert_eq!(
            rects[0],
            (
                PaneId::new(1),
                UnitRect {
                    x: 0,
                    y: 0,
                    w: 400_000,
                    h: UNITS
                }
            )
        );
        assert_eq!(
            rects[3].1,
            UnitRect {
                x: 700_000,
                y: 500_000,
                w: 300_000,
                h: 500_000
            }
        );
    }

    #[test]
    fn rounding_leaves_the_last_child_what_is_left() {
        let thirds = split(
            1,
            Axis::Row,
            &[333_333, 333_333, 333_334],
            vec![pane(1), pane(2), pane(3)],
        );
        let rects = tab_rects(&thirds);
        assert_eq!(rects[2].1.right(), UNITS);
        assert_eq!(rects[1].1.x, rects[0].1.right());
    }

    #[test]
    fn the_neighbour_lies_beyond_the_edge_and_alongside_it() {
        let rects = tab_rects(&four());
        let next = |from: u32, dir| neighbour(&rects, PaneId::new(from), dir, &[]);
        assert_eq!(next(1, Dir::Left), None);
        assert_eq!(next(1, Dir::Up), None);
        assert_eq!(next(3, Dir::Left), Some(PaneId::new(1)));
        assert_eq!(next(3, Dir::Right), Some(PaneId::new(4)));
        assert_eq!(next(3, Dir::Up), Some(PaneId::new(2)));
        assert_eq!(next(4, Dir::Down), None);
        // Pane 3 is below pane 2 and so is pane 4: the one made first wins a tie that
        // nothing else decides.
        assert_eq!(next(2, Dir::Down), Some(PaneId::new(3)));
    }

    #[test]
    fn of_two_equally_near_the_one_focused_last_is_chosen() {
        let rects = tab_rects(&four());
        // Panes 2 and 3 both touch pane 1's right edge.
        let seen = [PaneId::new(3), PaneId::new(2)];
        assert_eq!(
            neighbour(&rects, PaneId::new(1), Dir::Right, &seen),
            Some(PaneId::new(3))
        );
        let seen = [PaneId::new(2), PaneId::new(3)];
        assert_eq!(
            neighbour(&rects, PaneId::new(1), Dir::Right, &seen),
            Some(PaneId::new(2))
        );
    }
}
