//! The allocation budget, measured exactly so it can be enforced in CI.

#![cfg(feature = "std")]
#![expect(unsafe_code, reason = "a counting allocator needs it")]
#![allow(clippy::unwrap_used, reason = "tests assert by failing")]

mod common;

use common::NullHost;
use compono::router::stack::Stack;
use compono::{Component, Cx, Handle, Phase, Tree};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

std::thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    /// Bytes allocated on this thread and not yet freed.
    static LIVE_BYTES: Cell<i64> = const { Cell::new(0) };
}

fn count_bytes(layout: Layout, sign: i64) {
    let size = i64::try_from(layout.size()).unwrap_or(i64::MAX);
    let _during_thread_exit = LIVE_BYTES.try_with(|bytes| bytes.set(bytes.get() + sign * size));
}

// SAFETY: every call forwards unchanged to the system allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _during_thread_exit = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        count_bytes(layout, 1);
        // SAFETY: the caller upholds the contract of `alloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count_bytes(layout, -1);
        // SAFETY: the caller upholds the contract of `dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

fn allocations_in(operation: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.with(Cell::get);
    operation();
    ALLOCATIONS.with(Cell::get) - before
}

struct Screen {
    _payload: [u64; 8],
}

impl Component for Screen {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), cx: &mut Cx<'_, Self>) {
        cx.unchanged();
    }
}

struct Root {
    stack: Stack<u32, Handle<Screen>, Root>,
}

enum Nav {
    Push,
    Pop,
}

impl Component for Root {
    type Msg = Nav;
    type Output = ();
    fn update(&mut self, msg: Nav, cx: &mut Cx<'_, Self>) {
        match msg {
            Nav::Push => self.stack.push(1, cx),
            Nav::Pop => {
                let _at_root = self.stack.pop(cx);
            }
        }
    }
}

fn app() -> (Tree, Handle<Root>) {
    let mut tree = Tree::new(NullHost);
    tree.set_phase(Phase::Focused);
    let root = tree.mount(|cx| Root {
        stack: Stack::new(0, cx, |_route, cx| {
            cx.child(|_cx| Screen { _payload: [0; 8] }, |_root, (), _cx| {})
        }),
    });
    (tree, root)
}

fn cycle_cost(depth: usize) -> u64 {
    let (mut tree, root) = app();
    for _ in 1..depth {
        tree.send(root, Nav::Push).unwrap();
    }
    // The first cycle grows internal buffers once.
    tree.send(root, Nav::Push).unwrap();
    tree.send(root, Nav::Pop).unwrap();
    const CYCLES: u64 = 64;
    let total = allocations_in(|| {
        for _ in 0..CYCLES {
            tree.send(root, Nav::Push).unwrap();
            tree.send(root, Nav::Pop).unwrap();
        }
    });
    total / CYCLES
}

#[test]
fn navigation_stays_within_budget_at_every_depth() {
    let shallow = cycle_cost(1);
    assert!(shallow <= 3, "{shallow} allocations per push and pop");
    assert_eq!(cycle_cost(10), shallow);
    assert_eq!(cycle_cost(100), shallow);
}

#[test]
fn push_and_pop_leave_no_memory_behind() {
    let (mut tree, root) = app();
    for _ in 1..10 {
        tree.send(root, Nav::Push).unwrap();
    }
    // The first cycle grows internal buffers once.
    tree.send(root, Nav::Push).unwrap();
    tree.send(root, Nav::Pop).unwrap();
    let before = LIVE_BYTES.with(Cell::get);
    for _ in 0..10_000 {
        tree.send(root, Nav::Push).unwrap();
        tree.send(root, Nav::Pop).unwrap();
    }
    assert_eq!(LIVE_BYTES.with(Cell::get), before);
}

#[test]
fn a_message_that_changes_nothing_does_not_allocate() {
    let (mut tree, root) = app();
    let screen = *tree.get(root).unwrap().stack.active();
    tree.send(screen, ()).unwrap();
    assert_eq!(
        allocations_in(|| {
            for _ in 0..100 {
                tree.send(screen, ()).unwrap();
            }
        }),
        0
    );
}

#[test]
fn pumping_an_empty_queue_does_not_allocate() {
    let (mut tree, _root) = app();
    tree.pump().unwrap();
    assert_eq!(allocations_in(|| tree.pump().unwrap()), 0);
}

// 3-3: budgets for the other routers, for outputs and for messages to children.

mod routers {
    use super::{NullHost, allocations_in};
    use compono::router::items::Items;
    use compono::router::pages::Pages;
    use compono::router::panels::{PanelTypes, Panels, PanelsMode};
    use compono::router::slot::Slot;
    use compono::{Component, Cx, Handle, Phase, Tree};

    /// Answers every message with an output. Both carry a value, as real ones do.
    struct Leaf;
    impl Component for Leaf {
        type Msg = u32;
        type Output = u32;
        fn update(&mut self, value: u32, cx: &mut Cx<'_, Self>) {
            cx.emit(value);
        }
    }

    fn leaf<P: Component>(cx: &mut Cx<'_, P>) -> Handle<Leaf> {
        cx.child(|_cx| Leaf, |_parent, _value, cx| cx.unchanged())
    }

    struct Three;
    impl PanelTypes for Three {
        type Main = u32;
        type MainChild = Handle<Leaf>;
        type Details = u32;
        type DetailsChild = Handle<Leaf>;
        type Extra = u32;
        type ExtraChild = Handle<Leaf>;
    }

    pub(super) struct Root {
        child: Handle<Leaf>,
        pages: Pages<u32, Handle<Leaf>, Self>,
        items: Items<u32, Handle<Leaf>, Self>,
        panels: Panels<Three, Self>,
        slot: Slot<u32, Handle<Leaf>, Self>,
    }

    pub(super) enum Msg {
        Send,
        Page(usize),
        Scroll(usize),
        Details,
        Dialog,
    }

    impl Component for Root {
        type Msg = Msg;
        type Output = ();
        fn update(&mut self, msg: Msg, cx: &mut Cx<'_, Self>) {
            match msg {
                Msg::Send => cx.send(self.child, 7).unwrap(),
                Msg::Page(index) => {
                    self.pages.select(index, cx);
                }
                Msg::Scroll(start) => {
                    let _unique = self.items.set_window(start..start + 30, cx);
                }
                Msg::Details => {
                    self.panels.set_details(Some(1), cx);
                    self.panels.set_details(None, cx);
                }
                Msg::Dialog => {
                    self.slot.show(1, cx);
                    self.slot.close(cx);
                }
            }
        }
    }

    pub(super) fn app() -> (Tree, Handle<Root>) {
        let mut tree = Tree::new(NullHost);
        tree.set_phase(Phase::Focused);
        let root = tree.mount(|cx: &mut Cx<'_, Root>| Root {
            child: leaf(cx),
            pages: Pages::new((0..10).collect(), 0, 1, cx, |_route, cx| leaf(cx)).unwrap(),
            items: Items::new((0..1000).collect(), cx, |_key, cx| leaf(cx)),
            panels: Panels::new(
                0,
                PanelsMode::Dual,
                cx,
                |_route, cx| leaf(cx),
                |_route, cx| leaf(cx),
                |_route, cx| leaf(cx),
            ),
            slot: Slot::new(cx, |_route, cx| leaf(cx)),
        });
        (tree, root)
    }

    /// The fewest allocations of `rounds` runs of `msg`, after one to warm up.
    pub(super) fn cost(msg: impl Fn(usize) -> Msg) -> u64 {
        let (mut tree, root) = app();
        tree.send(root, msg(0)).unwrap();
        (1..20)
            .map(|round| allocations_in(|| tree.send(root, msg(round)).unwrap()))
            .min()
            .unwrap()
    }

    /// The allocations of an output alone: a leaf that emits once.
    pub(super) fn emit_cost() -> u64 {
        let (mut tree, root) = app();
        let child = tree.get(root).unwrap().child;
        tree.send(child, 7).unwrap();
        (0..20)
            .map(|_| allocations_in(|| tree.send(child, 7).unwrap()))
            .min()
            .unwrap()
    }
}

#[test]
fn every_router_and_message_stays_within_its_budget() {
    use routers::Msg;
    let emit = routers::emit_cost();
    // A message to a child, which answers with an output.
    let send = routers::cost(|_| Msg::Send);
    let page = routers::cost(|round| Msg::Page(round % 10));
    let scroll = routers::cost(Msg::Scroll);
    let details = routers::cost(|_| Msg::Details);
    let dialog = routers::cost(|_| Msg::Dialog);
    // An output and a message are each one box, since the queue erases their
    // types; `send` here also carries the child's answer.
    assert!(emit <= 1, "{emit} allocations per output");
    assert!(send <= 2, "{send} allocations per message and its answer");
    // A page shown again is built again: its node, its route and its scope.
    assert!(page <= 3, "{page} allocations per page selected");
    // One row in and one out of the window of 30.
    assert!(scroll <= 4, "{scroll} allocations per row scrolled");
    assert!(
        details <= 1,
        "{details} allocations per details panel opened and closed"
    );
    assert!(
        dialog <= 1,
        "{dialog} allocations per dialog shown and closed"
    );
}
