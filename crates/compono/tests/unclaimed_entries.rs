//! 5-4: saved state nobody can claim any more is dropped at the next save, and
//! state a live router can still claim is kept.

#![cfg(feature = "serde")]
#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::router::stack::{SavedStack, Stack};
use compono::{Commit, Component, Cx, Handle, Host, Persist, Tree};

struct NullHost;
impl Host for NullHost {
    fn commit(&mut self, _commit: &Commit<'_>) {}
}

const SCHEMA: u32 = 1;

struct Note(u32);
impl Component for Note {
    type Msg = u32;
    type Output = ();
    fn update(&mut self, value: u32, _cx: &mut Cx<'_, Self>) {
        self.0 = value;
    }
}
impl Persist for Note {
    type Saved = u32;
    const KIND: &'static str = "unclaimed.note";
    fn save(&self) -> u32 {
        self.0
    }
}

/// The first version keeps many notes, the second none.
struct App {
    _notes: Vec<Handle<Note>>,
}
impl Component for App {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}
impl Persist for App {
    type Saved = ();
    const KIND: &'static str = "unclaimed.app";
    fn save(&self) {}
}

fn first_version(notes: u32) -> impl FnOnce(&mut Cx<'_, App>) -> App {
    move |cx| App {
        _notes: (0..notes)
            .map(|index| {
                cx.persistent_child_keyed(
                    &format!("note {index}"),
                    move |cx| Note(cx.restored().unwrap_or(index)),
                    |_app, (), _cx| {},
                )
            })
            .collect(),
    }
}

#[test]
fn state_nobody_claims_for_a_whole_run_is_dropped() {
    let mut tree = Tree::new(NullHost);
    tree.mount_persistent(first_version(10_000));
    let old = tree.save(SCHEMA).unwrap();

    // The second version has no notes. Its run keeps their state, since a note
    // could still be created later in it.
    let (mut tree, _) = Tree::with_saved(NullHost, &old, SCHEMA);
    tree.mount_persistent(|_cx| App { _notes: Vec::new() });
    let kept = tree.save(SCHEMA).unwrap();
    assert!(
        kept.len() >= old.len(),
        "the first run without notes dropped them"
    );

    // The next run restores it, and nobody claims it there either.
    let (mut tree, _) = Tree::with_saved(NullHost, &kept, SCHEMA);
    tree.mount_persistent(|_cx| App { _notes: Vec::new() });
    let new = tree.save(SCHEMA).unwrap();
    assert!(
        new.len() < 100,
        "{} bytes of {} carried forward",
        new.len(),
        old.len()
    );
}

/// Creates its note only when asked, after the application has started.
struct Lazy {
    note: Option<Handle<Note>>,
}
impl Component for Lazy {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), cx: &mut Cx<'_, Self>) {
        self.note = Some(cx.persistent_child_keyed(
            "lazy",
            |cx| Note(cx.restored().unwrap_or(0)),
            |_lazy, (), _cx| {},
        ));
    }
}
impl Persist for Lazy {
    type Saved = ();
    const KIND: &'static str = "unclaimed.lazy";
    fn save(&self) {}
}

fn lazy_value(tree: &Tree, root: Handle<Lazy>) -> u32 {
    let note = tree.get(root).unwrap().note.unwrap();
    tree.get(note).unwrap().0
}

#[test]
fn a_child_created_only_after_a_save_still_restores_the_earlier_run() {
    // The first run creates the note and saves 41.
    let mut tree = Tree::new(NullHost);
    let root = tree.mount_persistent(|_cx| Lazy { note: None });
    tree.send(root, ()).unwrap();
    let note = tree.get(root).unwrap().note.unwrap();
    tree.send(note, 41).unwrap();
    let first = tree.save(SCHEMA).unwrap();

    // The second run saves before it creates the note, then ends.
    let (mut tree, _) = Tree::with_saved(NullHost, &first, SCHEMA);
    tree.mount_persistent(|_cx| Lazy { note: None });
    let second = tree.save(SCHEMA).unwrap();

    // The third run creates the note: it has the first run's state.
    let (mut tree, _) = Tree::with_saved(NullHost, &second, SCHEMA);
    let root = tree.mount_persistent(|_cx| Lazy { note: None });
    tree.send(root, ()).unwrap();
    assert_eq!(lazy_value(&tree, root), 41);
}

/// Screens of a named stack, restored only when they are shown.
struct Screens {
    stack: Stack<u32, Handle<Note>, Self>,
}
impl Component for Screens {
    type Msg = bool;
    type Output = ();
    fn update(&mut self, push: bool, cx: &mut Cx<'_, Self>) {
        if push {
            let route = self.stack.depth() as u32;
            self.stack.push(route, cx);
        } else {
            let _popped = self.stack.pop(cx);
        }
    }
}
impl Persist for Screens {
    type Saved = SavedStack<u32>;
    const KIND: &'static str = "unclaimed.screens";
    fn save(&self) -> SavedStack<u32> {
        self.stack.snapshot()
    }
}

fn screens(cx: &mut Cx<'_, Screens>) -> Screens {
    let saved = cx.restored();
    let stack = cx.named_routers("screens", |cx| {
        Stack::restore_or(saved, 0, cx, |route, cx| {
            let route = *route;
            cx.persistent_child(
                move |cx| Note(cx.restored().unwrap_or(route)),
                |_screens, (), _cx| {},
            )
        })
    });
    Screens { stack }
}

#[test]
fn state_of_an_entry_a_live_router_restores_later_is_kept() {
    let mut tree = Tree::new(NullHost);
    let root = tree.mount_persistent(screens);
    // The root screen holds 41, then a screen is pushed above it.
    let bottom = *tree.get(root).unwrap().stack.active();
    tree.send(bottom, 41).unwrap();
    tree.send(root, true).unwrap();
    let bytes = tree.save(SCHEMA).unwrap();

    // Restored, only the top screen is built. A save keeps the lower one's state.
    let (mut tree, _) = Tree::with_saved(NullHost, &bytes, SCHEMA);
    tree.mount_persistent(screens);
    let bytes = tree.save(SCHEMA).unwrap();

    let (mut tree, _) = Tree::with_saved(NullHost, &bytes, SCHEMA);
    let root = tree.mount_persistent(screens);
    tree.send(root, false).unwrap();
    let bottom = *tree.get(root).unwrap().stack.active();
    assert_eq!(tree.get(bottom).unwrap().0, 41);
}
