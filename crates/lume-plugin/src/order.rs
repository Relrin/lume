//! The order plugins load in, and which of them can be used.
//!
//! A plugin loads after every plugin it depends on, so what it relies on (a service,
//! a menu it adds to) is there first. `[load] before` puts it before others too, where
//! they are installed. Where nothing decides between two plugins, the user's order
//! does (the lock file's `order`), and then their ids.
//!
//! A plugin names those it depends on by the ids their manifests give them. Each name is
//! bound to an install: the one installed under that id, unless the user has bound it to
//! another, such as a fork installed under a name of its own.
//!
//! A plugin whose required dependency is missing, turned off, of a version it does not
//! work with, or not usable itself, is not used; it says why. Plugins that depend on each
//! other in a circle are none of them used. A circle made with `before` only loses those
//! `before`s instead, with a warning.

use std::collections::{BTreeMap, BTreeSet};

use crate::lock::Entry;
use crate::manifest::Manifest;

/// One install, as far as ordering goes.
pub struct Candidate<'a> {
    pub entry: &'a Entry,
    /// Its manifest, if it can be read.
    pub manifest: Option<&'a Manifest>,
    /// Why it cannot be used whatever the others are, if it cannot: it is turned off,
    /// its package changed, its manifest cannot be read.
    pub unusable: Option<String>,
}

/// Whether an install can be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Ready,
    /// It cannot, and why.
    Inactive(String),
}

/// What the order comes to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    /// Every install, in the order they load in.
    pub order: Vec<String>,
    pub states: BTreeMap<String, State>,
    /// For each install, the install each of its dependencies is bound to, where that
    /// one is installed.
    pub bound: BTreeMap<String, BTreeMap<String, String>>,
    /// Things worth saying that do not stop anything, by install.
    pub warnings: Vec<(String, String)>,
}

impl Resolution {
    pub fn is_ready(&self, local: &str) -> bool {
        self.states.get(local) == Some(&State::Ready)
    }

    /// Where an install is in the order: lower loads first.
    pub fn rank(&self, local: &str) -> Option<usize> {
        self.order.iter().position(|other| other == local)
    }
}

/// The install a plugin's dependency on `wanted` is bound to.
fn bound_to<'a>(entry: &'a Entry, wanted: &'a str) -> &'a str {
    entry.bindings.get(wanted).map_or(wanted, String::as_str)
}

/// Works out the order, and which installs can be used. `order` is the user's order.
pub fn resolve(candidates: &[Candidate<'_>], order: &[String]) -> Resolution {
    let index: BTreeMap<&str, usize> = candidates
        .iter()
        .enumerate()
        .map(|(at, candidate)| (candidate.entry.local.as_str(), at))
        .collect();
    let count = candidates.len();
    let rank = |at: usize| {
        let local = &candidates[at].entry.local;
        (
            order
                .iter()
                .position(|other| other == local)
                .unwrap_or(usize::MAX),
            local.clone(),
        )
    };

    let mut resolution = Resolution::default();
    // `hard[a]` holds b where a must load before b because b depends on a.
    let mut hard: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    let mut soft: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    for (at, candidate) in candidates.iter().enumerate() {
        let Some(manifest) = candidate.manifest else {
            continue;
        };
        let mut bound = BTreeMap::new();
        for wanted in manifest.dependencies.keys() {
            let target = bound_to(candidate.entry, wanted);
            if let Some(&other) = index.get(target) {
                hard[other].insert(at);
                bound.insert(wanted.clone(), target.to_owned());
            }
        }
        resolution
            .bound
            .insert(candidate.entry.local.clone(), bound);
        for later in &manifest.load.before {
            let target = bound_to(candidate.entry, later);
            if let Some(&other) = index.get(target)
                && other != at
            {
                soft[at].insert(other);
            }
        }
    }

    // Circles of dependencies.
    let mut in_circle: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for component in components(&hard) {
        let circular = component.len() > 1 || hard[component[0]].contains(&component[0]);
        if circular {
            for &member in &component {
                in_circle.insert(member, component.clone());
            }
        }
    }
    // Edges inside a circle cannot be kept: nothing would come first.
    let mut edges: Vec<BTreeSet<usize>> = (0..count)
        .map(|from| {
            hard[from]
                .iter()
                .copied()
                .filter(|to| !(in_circle.contains_key(&from) && in_circle.contains_key(to)))
                .collect()
        })
        .collect();
    for (from, targets) in soft.iter().enumerate() {
        edges[from].extend(targets.iter().copied());
    }
    // Circles that are left are made with `before`s; those go.
    for component in components(&edges) {
        if component.len() < 2 {
            continue;
        }
        let members: BTreeSet<usize> = component.iter().copied().collect();
        for &member in &component {
            let dropped: Vec<usize> = soft[member]
                .iter()
                .copied()
                .filter(|to| members.contains(to))
                .collect();
            for to in dropped {
                edges[member].remove(&to);
                if !hard[member].contains(&to) {
                    resolution.warnings.push((
                        candidates[member].entry.local.clone(),
                        format!(
                            "its [load] before {} was set aside: the plugins' orders go in a \
                             circle",
                            candidates[to].entry.local
                        ),
                    ));
                }
            }
        }
    }

    // Kahn's algorithm, taking the first in the user's order each time.
    let mut waiting: Vec<usize> = vec![0; count];
    for targets in &edges {
        for &to in targets {
            waiting[to] += 1;
        }
    }
    let mut ready: BTreeSet<((usize, String), usize)> = (0..count)
        .filter(|&at| waiting[at] == 0)
        .map(|at| (rank(at), at))
        .collect();
    let mut sorted = Vec::with_capacity(count);
    while let Some(first) = ready.pop_first() {
        let at = first.1;
        sorted.push(at);
        for &to in &edges[at] {
            waiting[to] -= 1;
            if waiting[to] == 0 {
                ready.insert((rank(to), to));
            }
        }
    }
    // Whatever is left would be in a circle still, which cannot happen; it goes last.
    let mut left: Vec<usize> = (0..count).filter(|at| !sorted.contains(at)).collect();
    left.sort_by_key(|&at| rank(at));
    sorted.extend(left);

    for &at in &sorted {
        let candidate = &candidates[at];
        let local = &candidate.entry.local;
        let state = match (&candidate.unusable, candidate.manifest) {
            (Some(reason), _) => State::Inactive(reason.clone()),
            (None, None) => State::Inactive("its manifest cannot be read".to_owned()),
            (None, Some(manifest)) => {
                if let Some(circle) = in_circle.get(&at) {
                    let names: Vec<&str> = circle
                        .iter()
                        .map(|&member| candidates[member].entry.local.as_str())
                        .collect();
                    State::Inactive(format!(
                        "it depends on itself, through {}",
                        names.join(", ")
                    ))
                } else {
                    dependency_problem(candidate.entry, manifest, &index, candidates, &resolution)
                        .map_or(State::Ready, State::Inactive)
                }
            }
        };
        resolution.states.insert(local.clone(), state);
        resolution.order.push(local.clone());
    }
    resolution
}

/// Why a plugin's dependencies keep it from being used, if they do. The dependencies
/// have their states already: they come first in the order.
fn dependency_problem(
    entry: &Entry,
    manifest: &Manifest,
    index: &BTreeMap<&str, usize>,
    candidates: &[Candidate<'_>],
    resolution: &Resolution,
) -> Option<String> {
    for (wanted, dependency) in &manifest.dependencies {
        let target = bound_to(entry, wanted);
        let shown = if target == wanted {
            wanted.clone()
        } else {
            format!("{wanted} (bound to {target})")
        };
        let Some(&other) = index.get(target) else {
            if dependency.optional() {
                continue;
            }
            return Some(format!("it needs {shown}, which is not installed"));
        };
        let requirement = dependency.requirement();
        let version_fits = candidates[other]
            .manifest
            .is_none_or(|found| requirement.matches(&found.version()));
        if !version_fits {
            return Some(format!(
                "it needs {shown} {requirement}, and {} is installed",
                candidates[other].entry.version
            ));
        }
        if dependency.optional() {
            continue;
        }
        if let Some(State::Inactive(why)) = resolution.states.get(target) {
            return Some(format!("it needs {shown}, which cannot be used: {why}"));
        }
    }
    None
}

/// The strongly connected parts of a graph (Tarjan), each as its members.
fn components(edges: &[BTreeSet<usize>]) -> Vec<Vec<usize>> {
    struct Walk<'a> {
        edges: &'a [BTreeSet<usize>],
        next: usize,
        number: Vec<Option<usize>>,
        low: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        found: Vec<Vec<usize>>,
    }
    impl Walk<'_> {
        fn visit(&mut self, at: usize) {
            self.number[at] = Some(self.next);
            self.low[at] = self.next;
            self.next += 1;
            self.stack.push(at);
            self.on_stack[at] = true;
            for &to in self.edges[at].clone().iter() {
                match self.number[to] {
                    None => {
                        self.visit(to);
                        self.low[at] = self.low[at].min(self.low[to]);
                    }
                    Some(number) if self.on_stack[to] => {
                        self.low[at] = self.low[at].min(number);
                    }
                    Some(_) => {}
                }
            }
            if Some(self.low[at]) == self.number[at] {
                let mut component = Vec::new();
                while let Some(member) = self.stack.pop() {
                    self.on_stack[member] = false;
                    component.push(member);
                    if member == at {
                        break;
                    }
                }
                component.sort_unstable();
                self.found.push(component);
            }
        }
    }
    let count = edges.len();
    let mut walk = Walk {
        edges,
        next: 0,
        number: vec![None; count],
        low: vec![0; count],
        stack: Vec::new(),
        on_stack: vec![false; count],
        found: Vec::new(),
    };
    for at in 0..count {
        if walk.number[at].is_none() {
            walk.visit(at);
        }
    }
    walk.found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::Source;

    fn manifest(id: &str, version: &str, extra: &str) -> Manifest {
        Manifest::parse(&format!(
            "schema = 1\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = 1\n{extra}"
        ))
        .unwrap_or_else(|problems| panic!("{problems:#?}"))
    }

    fn entry(local: &str, manifest: &Manifest) -> Entry {
        Entry {
            local: local.to_owned(),
            manifest_id: manifest.id.clone(),
            name: None,
            version: manifest.version.clone(),
            dir: String::new(),
            source: Source::local("folder", String::new()),
            content_hash: String::new(),
            manifest_hash: String::new(),
            enabled: true,
            grants: toml::Table::new(),
            bindings: BTreeMap::new(),
            services: BTreeMap::new(),
            machines: BTreeMap::new(),
        }
    }

    struct World {
        manifests: Vec<Manifest>,
        entries: Vec<Entry>,
        unusable: BTreeMap<String, String>,
    }

    impl World {
        fn new(plugins: &[(&str, &str, &str)]) -> Self {
            let manifests: Vec<Manifest> = plugins
                .iter()
                .map(|(id, version, extra)| manifest(id, version, extra))
                .collect();
            let entries = manifests.iter().map(|m| entry(&m.id, m)).collect();
            Self {
                manifests,
                entries,
                unusable: BTreeMap::new(),
            }
        }

        fn resolve(&self, order: &[&str]) -> Resolution {
            let candidates: Vec<Candidate<'_>> = self
                .entries
                .iter()
                .zip(&self.manifests)
                .map(|(entry, manifest)| Candidate {
                    entry,
                    manifest: Some(manifest),
                    unusable: self.unusable.get(&entry.local).cloned(),
                })
                .collect();
            let order: Vec<String> = order.iter().map(|&s| s.to_owned()).collect();
            resolve(&candidates, &order)
        }
    }

    #[test]
    fn a_plugin_loads_after_what_it_depends_on_and_otherwise_in_the_users_order() {
        let world = World::new(&[
            ("a.app", "1.0.0", "[dependencies]\n\"c.core\" = \"^1\"\n"),
            ("b.app", "1.0.0", ""),
            ("c.core", "1.4.0", ""),
        ]);
        let resolution = world.resolve(&["a.app", "b.app", "c.core"]);
        assert_eq!(resolution.order, ["b.app", "c.core", "a.app"]);
        assert!(
            resolution
                .states
                .values()
                .all(|state| *state == State::Ready)
        );
        let resolution = world.resolve(&["a.app", "c.core", "b.app"]);
        assert_eq!(resolution.order, ["c.core", "a.app", "b.app"]);
        // With no order of the user's, by id.
        assert_eq!(world.resolve(&[]).order, ["b.app", "c.core", "a.app"]);
    }

    #[test]
    fn before_puts_a_plugin_ahead_of_another_that_is_installed() {
        let world = World::new(&[
            ("a.x", "1.0.0", ""),
            (
                "b.x",
                "1.0.0",
                "[load]\nbefore = [\"a.x\", \"not.installed\"]\n",
            ),
        ]);
        assert_eq!(world.resolve(&[]).order, ["b.x", "a.x"]);
    }

    #[test]
    fn a_missing_or_unusable_dependency_keeps_its_dependents_out() {
        let mut world = World::new(&[
            ("a.x", "1.0.0", "[dependencies]\n\"b.x\" = \"1\"\n"),
            ("b.x", "1.0.0", "[dependencies]\n\"c.x\" = \"1\"\n"),
            ("c.x", "2.0.0", ""),
            (
                "d.x",
                "1.0.0",
                "[dependencies]\n\"gone.x\" = \"1\"\n\"maybe.x\" = { optional = true }\n",
            ),
            (
                "e.x",
                "1.0.0",
                "[dependencies]\n\"maybe.x\" = { optional = true }\n",
            ),
        ]);
        let resolution = world.resolve(&[]);
        let state = |local: &str| match &resolution.states[local] {
            State::Ready => "ready".to_owned(),
            State::Inactive(why) => why.clone(),
        };
        assert_eq!(state("c.x"), "ready");
        assert!(
            state("b.x").contains("c.x ^1, and 2.0.0 is installed"),
            "{}",
            state("b.x")
        );
        assert!(
            state("a.x").contains("needs b.x, which cannot be used"),
            "{}",
            state("a.x")
        );
        assert!(
            state("d.x").contains("gone.x, which is not installed"),
            "{}",
            state("d.x")
        );
        assert_eq!(state("e.x"), "ready");

        world
            .unusable
            .insert("c.x".to_owned(), "it is turned off".to_owned());
        world.manifests[1] = manifest("b.x", "1.0.0", "[dependencies]\n\"c.x\" = \"2\"\n");
        let resolution = world.resolve(&[]);
        assert_eq!(
            resolution.states["b.x"],
            State::Inactive("it needs c.x, which cannot be used: it is turned off".to_owned())
        );
    }

    #[test]
    fn a_dependency_can_be_bound_to_another_install() {
        let mut world = World::new(&[
            ("a.x", "1.0.0", "[dependencies]\n\"core.x\" = \"^2\"\n"),
            ("core.x", "1.0.0", ""),
        ]);
        // A fork of core.x, installed under a name of its own.
        let fork = manifest("core.x", "2.1.0", "");
        let mut fork_entry = entry("me.core", &fork);
        fork_entry.local = "me.core".to_owned();
        world.manifests.push(fork);
        world.entries.push(fork_entry);
        assert!(!world.resolve(&[]).is_ready("a.x"));

        world.entries[0]
            .bindings
            .insert("core.x".to_owned(), "me.core".to_owned());
        let resolution = world.resolve(&[]);
        assert!(resolution.is_ready("a.x"));
        assert_eq!(resolution.bound["a.x"]["core.x"], "me.core");
        assert!(resolution.rank("me.core") < resolution.rank("a.x"));
    }

    #[test]
    fn plugins_that_depend_on_each_other_in_a_circle_are_not_used() {
        let world = World::new(&[
            ("a.x", "1.0.0", "[dependencies]\n\"b.x\" = \"1\"\n"),
            ("b.x", "1.0.0", "[dependencies]\n\"a.x\" = \"1\"\n"),
            ("c.x", "1.0.0", "[dependencies]\n\"a.x\" = \"1\"\n"),
            ("d.x", "1.0.0", ""),
        ]);
        let resolution = world.resolve(&[]);
        for local in ["a.x", "b.x"] {
            assert!(
                matches!(&resolution.states[local], State::Inactive(why) if why.contains("depends on itself")),
                "{local}: {:?}",
                resolution.states[local]
            );
        }
        assert!(matches!(&resolution.states["c.x"], State::Inactive(why) if why.contains("a.x")));
        assert!(resolution.is_ready("d.x"));
        assert_eq!(resolution.order.len(), 4);
    }

    #[test]
    fn a_circle_of_befores_loses_them_with_a_warning() {
        let world = World::new(&[
            ("a.x", "1.0.0", "[load]\nbefore = [\"b.x\"]\n"),
            ("b.x", "1.0.0", "[load]\nbefore = [\"a.x\"]\n"),
        ]);
        let resolution = world.resolve(&["b.x"]);
        assert_eq!(resolution.order, ["b.x", "a.x"]);
        assert!(resolution.is_ready("a.x") && resolution.is_ready("b.x"));
        assert_eq!(resolution.warnings.len(), 2);
    }
}
