//! A tree whose operation unwound is poisoned: it runs no more user code and
//! changes nothing, but it can still be read and dropped.

#![expect(clippy::unwrap_used, reason = "tests assert by failing")]
#![expect(clippy::panic, reason = "the test needs a component that panics")]

use compono::{Commit, Component, Cx, Handle, Host, Phase, StaleHandle, Tree};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

type Log = Rc<RefCell<Vec<String>>>;

#[derive(Clone, Default)]
struct Commits(Rc<RefCell<usize>>);

impl Host for Commits {
    fn commit(&mut self, _commit: &Commit<'_>) {
        *self.0.borrow_mut() += 1;
    }
}

/// Panics on `true` and logs its phases.
struct Fragile {
    log: Log,
}

impl Component for Fragile {
    type Msg = bool;
    type Output = ();
    fn update(&mut self, explode: bool, _cx: &mut Cx<'_, Self>) {
        if explode {
            panic!("the component failed");
        }
    }
    fn phase_changed(&mut self, phase: Phase, _cx: &mut Cx<'_, Self>) {
        self.log.borrow_mut().push(format!("{phase:?}"));
    }
    fn back(&self) -> Option<bool> {
        Some(false)
    }
}

/// The root, which holds the fragile component.
struct Holder {
    fragile: Handle<Fragile>,
    name: &'static str,
}

impl Component for Holder {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

fn holder(log: &Log) -> impl FnOnce(&mut Cx<'_, Holder>) -> Holder {
    let log = Rc::clone(log);
    move |cx| Holder {
        fragile: cx.child(move |_cx| Fragile { log }, |_holder, (), _cx| {}),
        name: "holder",
    }
}

/// A tree whose last operation panicked and was caught.
fn poisoned() -> (Tree, Handle<Holder>, Handle<Fragile>, Log, Commits) {
    let log = Log::default();
    let commits = Commits::default();
    let mut tree = Tree::new(commits.clone());
    tree.set_phase(Phase::Focused);
    let root = tree.mount(holder(&log));
    let fragile = tree.get(root).unwrap().fragile;
    assert!(!tree.is_poisoned(), "a healthy tree is not poisoned");
    let caught = catch_unwind(AssertUnwindSafe(|| tree.send(fragile, true)));
    assert!(caught.is_err(), "the panic reached the caller");
    log.borrow_mut().clear();
    (tree, root, fragile, log, commits)
}

#[test]
fn an_operation_that_unwound_poisons_the_tree() {
    let (tree, _root, _fragile, _log, _commits) = poisoned();
    assert!(tree.is_poisoned());
}

#[test]
fn a_poisoned_tree_runs_no_user_code_and_commits_nothing() {
    let (mut tree, root, _fragile, log, commits) = poisoned();
    let before = *commits.0.borrow();

    assert_eq!(tree.send(root, ()), Err(StaleHandle));
    tree.set_phase(Phase::Hidden);
    assert!(!tree.back_start());
    tree.back_cancel();
    assert!(!tree.back());
    assert!(!tree.back_commit());
    // `pump` exists only with `std`, where other threads can queue work.
    #[cfg(feature = "std")]
    assert!(tree.pump().is_err());
    let replacement = tree.mount(holder(&log));

    assert!(
        log.borrow().is_empty(),
        "no phase was told: {:?}",
        log.borrow()
    );
    assert_eq!(*commits.0.borrow(), before, "nothing was committed");
    assert!(tree.get(replacement).is_none(), "nothing was mounted");
}

#[test]
fn a_poisoned_tree_can_still_be_read() {
    let (tree, root, fragile, _log, _commits) = poisoned();
    assert_eq!(tree.get(root).unwrap().name, "holder");
    assert!(
        tree.get(fragile).is_none(),
        "the component that panicked is not given out half-changed"
    );
}

/// A leaf whose state is saved, counting the calls of `Persist::save`.
#[cfg(feature = "serde")]
struct Leaf {
    saves: Rc<RefCell<usize>>,
}

#[cfg(feature = "serde")]
impl Component for Leaf {
    type Msg = bool;
    type Output = ();
    fn update(&mut self, explode: bool, _cx: &mut Cx<'_, Self>) {
        if explode {
            panic!("the leaf failed");
        }
    }
}

#[cfg(feature = "serde")]
impl compono::Persist for Leaf {
    type Saved = u8;
    const KIND: &'static str = "poison.leaf";
    fn save(&self) -> u8 {
        *self.saves.borrow_mut() += 1;
        1
    }
}

#[cfg(feature = "serde")]
struct Pair {
    leaves: [Handle<Leaf>; 2],
}

#[cfg(feature = "serde")]
impl Component for Pair {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

#[cfg(feature = "serde")]
#[test]
fn a_poisoned_tree_saves_nothing_and_says_so() {
    // 2-1: the state of a half-changed tree must not replace the last good save.
    let saves = Rc::new(RefCell::new(0));
    let mut tree = Tree::new(Commits::default());
    let root = tree.mount(|cx| {
        let leaf = |cx: &mut Cx<'_, Pair>| {
            let saves = Rc::clone(&saves);
            cx.persistent_child(move |_cx| Leaf { saves }, |_pair, (), _cx| {})
        };
        Pair {
            leaves: [leaf(cx), leaf(cx)],
        }
    });
    assert!(tree.save(1).is_ok());
    assert_eq!(*saves.borrow(), 2);
    let fragile = tree.get(root).unwrap().leaves[0];
    let caught = catch_unwind(AssertUnwindSafe(|| tree.send(fragile, true)));
    assert!(caught.is_err());

    assert_eq!(tree.save(1), Err(compono::SaveError::Poisoned));
    assert_eq!(*saves.borrow(), 2, "a poisoned tree ran Persist::save");
}

struct Twins {
    first: Handle<Fragile>,
}

impl Component for Twins {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

#[test]
fn dropping_a_poisoned_tree_tells_no_component_it_is_hidden() {
    // 2-1: dropping runs no phase change, which is user code.
    let log = Log::default();
    let mut tree = Tree::new(Commits::default());
    tree.set_phase(Phase::Focused);
    let root = tree.mount(|cx| {
        let (one, two) = (Rc::clone(&log), Rc::clone(&log));
        let first = cx.child(move |_cx| Fragile { log: one }, |_twins, (), _cx| {});
        cx.child(move |_cx| Fragile { log: two }, |_twins, (), _cx| {});
        Twins { first }
    });
    let first = tree.get(root).unwrap().first;
    let caught = catch_unwind(AssertUnwindSafe(|| tree.send(first, true)));
    assert!(caught.is_err());
    log.borrow_mut().clear();
    drop(tree);
    assert!(log.borrow().is_empty(), "told: {:?}", log.borrow());
}
