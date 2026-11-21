//! Measures whether `ChildSet` reconciliation stays linear in the child count, and
//! whether a named child (`Cx::child_keyed`) costs the same however many siblings
//! it has.

#![allow(clippy::unwrap_used, reason = "a measurement may stop on failure")]

use compono::router::set::ChildSet;
use compono::{Commit, Component, Cx, Handle, Host, Phase, Tree};
use std::time::Instant;

struct NullHost;
impl Host for NullHost {
    fn commit(&mut self, _commit: &Commit<'_>) {}
}

struct Row;
impl Component for Row {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

struct List {
    rows: ChildSet<u32, Handle<Row>, Self>,
}

impl Component for List {
    type Msg = Vec<(u32, Phase)>;
    type Output = ();
    fn update(&mut self, desired: Vec<(u32, Phase)>, cx: &mut Cx<'_, Self>) {
        self.rows.reconcile(desired, cx).unwrap();
    }
}

fn measure(size: u32) {
    let mut tree = Tree::new(NullHost);
    tree.set_phase(Phase::Focused);
    let list = tree.mount(|cx| List {
        rows: ChildSet::new(cx, |_key, cx| cx.child(|_cx| Row, |_list, (), _cx| {})),
    });
    let all: Vec<_> = (0..size).map(|key| (key, Phase::Visible)).collect();
    let reversed: Vec<_> = all.iter().rev().copied().collect();
    let tail = all.get(all.len() / 2..).unwrap().to_vec();

    let per_row = |start: Instant| start.elapsed().as_nanos() / u128::from(size);
    let start = Instant::now();
    tree.send(list, all.clone()).unwrap();
    let create = per_row(start);
    let start = Instant::now();
    tree.send(list, all).unwrap();
    let unchanged = per_row(start);
    let start = Instant::now();
    tree.send(list, reversed).unwrap();
    let reorder = per_row(start);
    let start = Instant::now();
    tree.send(list, tail.clone()).unwrap();
    let remove_half = per_row(start);
    // `reconcile` reads the whole list, so one new row costs as much as the set.
    let mut one_more = tail;
    one_more.push((size, Phase::Visible));
    let start = Instant::now();
    tree.send(list, one_more).unwrap();
    let add_one = start.elapsed().as_micros();
    println!(
        "{size:>8} children: create {create} ns, unchanged {unchanged} ns, \
         reverse {reorder} ns, remove half {remove_half} ns (per child), \
         add one {add_one} µs (whole call)"
    );
}

/// Creates the children named by its message, one by one.
struct Named {
    rows: Vec<Handle<Row>>,
}

impl Component for Named {
    type Msg = Vec<String>;
    type Output = ();
    fn update(&mut self, names: Vec<String>, cx: &mut Cx<'_, Self>) {
        for name in &names {
            let row = cx.child_keyed(name, |_cx| Row, |_named, (), _cx| {});
            self.rows.push(row);
        }
    }
}

fn measure_named(size: u32) {
    let mut tree = Tree::new(NullHost);
    let named = tree.mount(|_cx| Named { rows: Vec::new() });
    let names: Vec<String> = (0..size).map(|index| format!("row {index}")).collect();
    let start = Instant::now();
    tree.send(named, names).unwrap();
    let per_child = start.elapsed().as_nanos() / u128::from(size);
    println!("{size:>8} named children: create {per_child} ns (per child)");
}

fn main() {
    for size in [1_000, 10_000, 100_000, 1_000_000] {
        measure(size);
    }
    for size in [1_000, 10_000, 50_000] {
        measure_named(size);
    }
}
