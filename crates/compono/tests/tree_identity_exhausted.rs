//! 4-1: with every identity held by a live tree, a new tree addresses nothing,
//! and identities come back when trees are dropped.
//!
//! The test runs alone in its own binary, so no other test meets the full bitmap.

#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::{Commit, Component, Cx, Host, StaleHandle, Tree};

struct NullHost;
impl Host for NullHost {
    fn commit(&mut self, _commit: &Commit<'_>) {}
}

struct Counter(u32);

impl Component for Counter {
    type Msg = u32;
    type Output = ();
    fn update(&mut self, amount: u32, _cx: &mut Cx<'_, Self>) {
        self.0 += amount;
    }
}

#[test]
fn a_tree_past_the_last_identity_addresses_nothing_until_one_is_free() {
    // 4 094 identities can be in use at once.
    let mut live: Vec<(Tree, compono::Handle<Counter>)> = (0..4094)
        .map(|_| {
            let mut tree = Tree::new(NullHost);
            let handle = tree.mount(|_cx| Counter(0));
            (tree, handle)
        })
        .collect();
    assert!(
        live.iter()
            .all(|(tree, handle)| tree.get(*handle).is_some())
    );

    let mut extra = Tree::new(NullHost);
    // It says so from the start, as a poisoned tree does.
    assert!(extra.is_poisoned(), "a tree with no identity looks healthy");
    let handle = extra.mount(|_cx| Counter(0));
    assert!(extra.get(handle).is_none());
    assert_eq!(extra.send(handle, 1), Err(StaleHandle));
    // It reaches none of the live trees' components either.
    for (_tree, other) in &live {
        assert!(extra.get(*other).is_none());
    }
    drop(extra);

    live.truncate(4000);
    let mut again = Tree::new(NullHost);
    assert!(!again.is_poisoned());
    let handle = again.mount(|_cx| Counter(0));
    assert_eq!(again.send(handle, 2), Ok(()));
    assert_eq!(again.get(handle).unwrap().0, 2);
    for (tree, other) in &live {
        assert!(again.get(*other).is_none());
        assert!(tree.get(handle).is_none());
    }
}
