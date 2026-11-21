//! 2-3: a scope dropped between operations wakes the host once, and the next
//! operation removes its components.

#![cfg(feature = "std")]
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests assert by failing"
)]

use compono::router::Scope;
use compono::{Commit, Component, Cx, Handle, Host, Tree};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Wake, Waker};

#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Counting(Arc<Wakes>);
impl Host for Counting {
    fn commit(&mut self, _commit: &Commit<'_>) {}
    fn waker(&self) -> Waker {
        Waker::from(Arc::clone(&self.0))
    }
}

struct Leaf;
impl Component for Leaf {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

/// Hands the scopes of its entries to the application, which keeps them outside
/// the tree.
struct Owner {
    leaves: Vec<Handle<Leaf>>,
}

impl Component for Owner {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

#[test]
fn a_scope_dropped_outside_an_operation_wakes_the_host_once() {
    let wakes = Arc::new(Wakes::default());
    let mut tree = Tree::new(Counting(Arc::clone(&wakes)));
    let held: Rc<RefCell<Vec<Scope>>> = Rc::default();
    let outside = Rc::clone(&held);
    let owner = tree.mount(move |cx| {
        let router = cx.router();
        let mut leaves = Vec::new();
        for key in 0..2 {
            let (leaf, scope) =
                cx.scoped(router, key, |cx| cx.child(|_cx| Leaf, |_owner, (), _cx| {}));
            leaves.push(leaf);
            outside.borrow_mut().push(scope);
        }
        Owner { leaves }
    });
    let leaves = tree.get(owner).unwrap().leaves.clone();
    let before = wakes.0.load(Ordering::SeqCst);

    // Between operations, with work for the tree and nothing to do it.
    held.borrow_mut().clear();
    assert_eq!(wakes.0.load(Ordering::SeqCst) - before, 1);
    assert!(
        tree.get(leaves[0]).is_some(),
        "removal waits for an operation"
    );

    tree.pump().unwrap();
    assert!(leaves.iter().all(|leaf| tree.get(*leaf).is_none()));
    assert_eq!(wakes.0.load(Ordering::SeqCst) - before, 1);
}
