//! 3-1: a named child is checked against its live siblings, whatever their number.

#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::{Commit, Component, Cx, Diagnostic, Handle, Host, Tree};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Default)]
struct Repeats(Rc<RefCell<Vec<String>>>);

impl Host for Repeats {
    fn commit(&mut self, _commit: &Commit<'_>) {}
    fn diagnostic(&mut self, diagnostic: &Diagnostic<'_>) {
        if let Diagnostic::RepeatedChildName { name, .. } = diagnostic {
            self.0.borrow_mut().push((*name).to_owned());
        }
    }
}

struct Row;
impl Component for Row {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

enum Msg {
    Add(&'static str),
    Remove(usize),
}

struct Rows(Vec<Handle<Row>>);
impl Component for Rows {
    type Msg = Msg;
    type Output = ();
    fn update(&mut self, msg: Msg, cx: &mut Cx<'_, Self>) {
        match msg {
            Msg::Add(name) => self
                .0
                .push(cx.child_keyed(name, |_cx| Row, |_rows, (), _cx| {})),
            Msg::Remove(index) => {
                let row = self.0.remove(index);
                cx.destroy(row).unwrap();
            }
        }
    }
}

#[test]
fn a_repeated_name_is_refused_while_its_first_child_lives() {
    let repeats = Repeats::default();
    let mut tree = Tree::new(repeats.clone());
    let rows = tree.mount(|_cx| Rows(Vec::new()));
    for name in ["a", "b", "a"] {
        tree.send(rows, Msg::Add(name)).unwrap();
    }
    assert_eq!(*repeats.0.borrow(), ["a"]);
    // A sibling whose name was taken does not free it when it goes.
    tree.send(rows, Msg::Remove(2)).unwrap();
    tree.send(rows, Msg::Add("a")).unwrap();
    assert_eq!(*repeats.0.borrow(), ["a", "a"]);
    // Once the child that has it is gone, the name is free again.
    tree.send(rows, Msg::Remove(0)).unwrap();
    tree.send(rows, Msg::Add("a")).unwrap();
    assert_eq!(*repeats.0.borrow(), ["a", "a"]);
    tree.send(rows, Msg::Add("a")).unwrap();
    assert_eq!(*repeats.0.borrow(), ["a", "a", "a"]);
}

#[test]
fn the_same_name_under_two_parents_is_two_names() {
    let repeats = Repeats::default();
    let mut tree = Tree::new(repeats.clone());
    tree.mount(|cx| {
        let first = cx.child(|_cx| Rows(Vec::new()), |_rows, (), _cx| {});
        let second = cx.child(|_cx| Rows(Vec::new()), |_rows, (), _cx| {});
        cx.send(first, Msg::Add("a")).unwrap();
        cx.send(second, Msg::Add("a")).unwrap();
        Rows(Vec::new())
    });
    assert!(repeats.0.borrow().is_empty());
}
