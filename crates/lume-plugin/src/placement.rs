//! Putting things in order among their neighbours.
//!
//! Wherever plugins add to something (a menu, a section of one, a bar, a group of
//! settings), each thing they add says where it goes: right before or right after a
//! neighbour, or at an order number. The application's own things have orders spaced
//! apart, so a number can fall between two of them. This works out the order of the
//! things in one container:
//!
//! 1. An anchor that is not a neighbour in the same container is dropped, with a
//!    warning, and the thing falls back to its order.
//! 2. Things without an anchor come first by their order, then by the order plugins
//!    load in (the application's own before any plugin's), then by plugin, then by where
//!    each is declared.
//! 3. Each of those is preceded by the things anchored before it and followed by the
//!    things anchored after it, each of those in turn with its own.
//! 4. Things that anchor each other in a circle are never reached that way: their
//!    anchors are dropped, with a warning, and they are placed by their orders.

use std::collections::{BTreeMap, BTreeSet};

/// Which side of its neighbour a thing goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Before,
    After,
}

/// A neighbour a thing goes right next to, by the neighbour's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub side: Side,
    pub key: String,
}

/// One thing to be put in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed<T> {
    /// What names it among its neighbours: `lume:<key>` for the application's own,
    /// `<plugin>:<name>` for a plugin's.
    pub key: String,
    pub anchor: Option<Anchor>,
    pub order: i32,
    /// Where its plugin is in the order plugins load in; the application's own things
    /// come before every plugin's.
    pub rank: i64,
    /// Its plugin, as installed: empty for the application's own.
    pub plugin: String,
    /// Where it is declared among its plugin's things.
    pub index: usize,
    pub value: T,
}

/// The rank of the application's own things.
pub const BUILTIN_RANK: i64 = -1;

impl<T> Placed<T> {
    fn sort_key(&self) -> (i32, i64, &str, usize) {
        (self.order, self.rank, self.plugin.as_str(), self.index)
    }
}

/// Something worth saying about where a thing went: its key and what happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub key: String,
    pub plugin: String,
    pub message: String,
}

/// Puts the things of one container in order. What went other than asked is said in
/// `warnings`.
pub fn arrange<T>(things: Vec<Placed<T>>, warnings: &mut Vec<Warning>) -> Vec<Placed<T>> {
    let mut things = things;
    let keys: BTreeSet<String> = things.iter().map(|thing| thing.key.clone()).collect();
    for thing in &mut things {
        let Some(anchor) = &thing.anchor else {
            continue;
        };
        if anchor.key == thing.key || !keys.contains(&anchor.key) {
            warnings.push(Warning {
                key: thing.key.clone(),
                plugin: thing.plugin.clone(),
                message: format!(
                    "{} is placed by its order: `{}` is not beside it here",
                    thing.key, anchor.key
                ),
            });
            thing.anchor = None;
        }
    }

    let mut ordered: Vec<usize> = Vec::with_capacity(things.len());
    let mut reached = vec![false; things.len()];
    loop {
        ordered.clear();
        reached.iter_mut().for_each(|one| *one = false);
        // What hangs off each thing, before it and after it, in the order of their own.
        let mut before: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        let mut after: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        let mut roots = Vec::new();
        for (at, thing) in things.iter().enumerate() {
            match &thing.anchor {
                None => roots.push(at),
                Some(anchor) => {
                    let side = match anchor.side {
                        Side::Before => &mut before,
                        Side::After => &mut after,
                    };
                    side.entry(anchor.key.as_str()).or_default().push(at);
                }
            }
        }
        let by_key = |list: &mut Vec<usize>| {
            list.sort_by(|a, b| things[*a].sort_key().cmp(&things[*b].sort_key()));
        };
        by_key(&mut roots);
        for list in before.values_mut().chain(after.values_mut()) {
            by_key(list);
        }

        fn emit<T>(
            at: usize,
            things: &[Placed<T>],
            before: &BTreeMap<&str, Vec<usize>>,
            after: &BTreeMap<&str, Vec<usize>>,
            reached: &mut [bool],
            ordered: &mut Vec<usize>,
        ) {
            if reached[at] {
                return;
            }
            reached[at] = true;
            let key = things[at].key.as_str();
            for &one in before.get(key).into_iter().flatten() {
                emit(one, things, before, after, reached, ordered);
            }
            ordered.push(at);
            for &one in after.get(key).into_iter().flatten() {
                emit(one, things, before, after, reached, ordered);
            }
        }

        for root in roots {
            emit(root, &things, &before, &after, &mut reached, &mut ordered);
        }
        if ordered.len() == things.len() {
            break;
        }
        // Everything is placed again once the circle is undone, by the orders of all.
        // What is left is reached only through a circle of anchors. Each thing has one
        // anchor at most, so following them from what is left comes round to the circle.
        let at_key: BTreeMap<&str, usize> = things
            .iter()
            .enumerate()
            .map(|(at, thing)| (thing.key.as_str(), at))
            .collect();
        let mut circle = BTreeSet::new();
        for start in (0..things.len()).filter(|at| !reached[*at]) {
            let mut walked = Vec::new();
            let mut at = start;
            loop {
                if let Some(from) = walked.iter().position(|seen| *seen == at) {
                    circle.extend(walked[from..].iter().copied());
                    break;
                }
                if reached[at] || circle.contains(&at) {
                    break;
                }
                walked.push(at);
                match things[at]
                    .anchor
                    .as_ref()
                    .and_then(|anchor| at_key.get(anchor.key.as_str()))
                {
                    Some(next) => at = *next,
                    None => break,
                }
            }
        }
        let mut circle: Vec<usize> = circle.into_iter().collect();
        if circle.is_empty() {
            // Not to be had, since what is left always comes round; but nothing may
            // be left unplaced, so then every anchor left goes.
            circle = (0..things.len()).filter(|at| !reached[*at]).collect();
        }
        circle.sort_by_key(|at| things[*at].key.clone());
        for at in circle {
            let thing = &mut things[at];
            if thing.anchor.take().is_some() {
                warnings.push(Warning {
                    key: thing.key.clone(),
                    plugin: thing.plugin.clone(),
                    message: format!(
                        "{} is placed by its order: its neighbours are placed against each other in a circle",
                        thing.key
                    ),
                });
            }
        }
    }

    let mut slots: Vec<Option<Placed<T>>> = things.into_iter().map(Some).collect();
    ordered
        .into_iter()
        .filter_map(|at| slots[at].take())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin(key: &str, order: i32) -> Placed<()> {
        Placed {
            key: format!("lume:{key}"),
            anchor: None,
            order,
            rank: BUILTIN_RANK,
            plugin: String::new(),
            index: 0,
            value: (),
        }
    }

    fn plugin(
        plugin: &str,
        name: &str,
        rank: i64,
        index: usize,
        anchor: Option<(Side, &str)>,
        order: i32,
    ) -> Placed<()> {
        Placed {
            key: format!("{plugin}:{name}"),
            anchor: anchor.map(|(side, key)| Anchor {
                side,
                key: key.to_owned(),
            }),
            order,
            rank,
            plugin: plugin.to_owned(),
            index,
            value: (),
        }
    }

    fn keys(things: &[Placed<()>]) -> Vec<&str> {
        things.iter().map(|thing| thing.key.as_str()).collect()
    }

    #[test]
    fn things_go_by_order_then_load_order_then_plugin_then_declaration() {
        let mut warnings = Vec::new();
        let arranged = arrange(
            vec![
                builtin("b", 20),
                plugin("p.two", "x", 1, 0, None, 15),
                plugin("p.one", "y", 0, 1, None, 15),
                plugin("p.one", "x", 0, 0, None, 15),
                builtin("a", 10),
                plugin("p.three", "late", 2, 0, None, 1000),
            ],
            &mut warnings,
        );
        assert_eq!(
            keys(&arranged),
            [
                "lume:a",
                "p.one:x",
                "p.one:y",
                "p.two:x",
                "lume:b",
                "p.three:late"
            ]
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_thing_goes_right_next_to_the_neighbour_it_names() {
        let mut warnings = Vec::new();
        let arranged = arrange(
            vec![
                builtin("a", 10),
                builtin("b", 20),
                plugin("p", "after-a", 0, 0, Some((Side::After, "lume:a")), 0),
                plugin("p", "before-a", 0, 1, Some((Side::Before, "lume:a")), 0),
                plugin(
                    "q",
                    "after-after-a",
                    1,
                    0,
                    Some((Side::After, "p:after-a")),
                    0,
                ),
                plugin("q", "also-after-a", 1, 1, Some((Side::After, "lume:a")), 0),
            ],
            &mut warnings,
        );
        assert_eq!(
            keys(&arranged),
            [
                "p:before-a",
                "lume:a",
                "p:after-a",
                "q:after-after-a",
                "q:also-after-a",
                "lume:b",
            ]
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn an_anchor_that_is_not_a_neighbour_falls_back_to_the_order() {
        let mut warnings = Vec::new();
        let arranged = arrange(
            vec![
                builtin("a", 10),
                plugin("p", "x", 0, 0, Some((Side::Before, "lume:gone")), 15),
                plugin("p", "self", 0, 1, Some((Side::After, "p:self")), 5),
            ],
            &mut warnings,
        );
        assert_eq!(keys(&arranged), ["p:self", "lume:a", "p:x"]);
        assert_eq!(warnings.len(), 2);
        assert!(warnings[0].message.contains("lume:gone"), "{warnings:?}");
    }

    #[test]
    fn things_anchored_in_a_circle_are_placed_by_their_orders() {
        let mut warnings = Vec::new();
        let arranged = arrange(
            vec![
                builtin("a", 10),
                plugin("p", "x", 0, 0, Some((Side::After, "p:y")), 30),
                plugin("p", "y", 0, 1, Some((Side::After, "p:z")), 20),
                plugin("p", "z", 0, 2, Some((Side::Before, "p:x")), 5),
                plugin("p", "w", 0, 3, Some((Side::After, "p:x")), 0),
            ],
            &mut warnings,
        );
        // The circle goes by orders, and what hangs off it keeps its place.
        assert_eq!(keys(&arranged), ["p:z", "lume:a", "p:y", "p:x", "p:w"]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(
            warnings
                .iter()
                .all(|warning| warning.message.contains("circle"))
        );
    }
}
