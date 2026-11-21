//! 4-1: a handle addresses a component of the tree that issued it, and of no other.

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
fn a_handle_of_one_tree_is_stale_in_another() {
    let mut alice = Tree::new(NullHost);
    let mut bob = Tree::new(NullHost);
    let alices = alice.mount(|_cx| Counter(1));
    let bobs = bob.mount(|_cx| Counter(2));

    assert!(bob.get(alices).is_none(), "bob read alice's component");
    assert!(bob.phase(alices).is_none());
    assert!(!bob.contains(alices.id()));
    assert!(bob.parent(alices.id()).is_none());
    assert!(bob.handle_of::<Counter>(alices.id()).is_none());
    assert!(bob.node_phase(alices.id()).is_none());
    assert_eq!(bob.send(alices, 40), Err(StaleHandle));
    assert_eq!(bob.get(bobs).unwrap().0, 2, "bob's component changed");

    // The handle still works where it came from.
    assert_eq!(alice.send(alices, 40), Ok(()));
    assert_eq!(alice.get(alices).unwrap().0, 41);
}

#[test]
fn bits_that_crossed_a_language_boundary_stay_in_their_tree() {
    let mut alice = Tree::new(NullHost);
    let mut bob = Tree::new(NullHost);
    let alices = alice.mount(|_cx| Counter(1));
    bob.mount(|_cx| Counter(2));
    let bits = alices.id().to_bits();
    let rebuilt = compono::NodeId::from_bits(bits);
    assert!(bob.handle_of::<Counter>(rebuilt).is_none());
    assert!(alice.handle_of::<Counter>(rebuilt).is_some());
}

#[test]
fn trees_alive_together_never_share_an_identity() {
    // Many trees at once, made and dropped on several threads.
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                let mut trees: Vec<(Tree, compono::Handle<Counter>)> = Vec::new();
                for round in 0..200_u32 {
                    let mut tree = Tree::new(NullHost);
                    let handle = tree.mount(|_cx| Counter(round));
                    trees.push((tree, handle));
                    if round % 3 == 0 {
                        trees.remove(0);
                    }
                }
                for (index, (tree, _)) in trees.iter().enumerate() {
                    for (other, (_, handle)) in trees.iter().enumerate() {
                        assert_eq!(tree.get(*handle).is_some(), index == other);
                    }
                }
                trees
                    .into_iter()
                    .map(|(_, handle)| handle.id())
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut ids: Vec<_> = threads
        .into_iter()
        .flat_map(|thread| thread.join().unwrap())
        .collect();
    // These trees were alive at once, at the end of every thread.
    let count = ids.len();
    ids.sort_by_key(|id| id.to_bits());
    ids.dedup();
    assert_eq!(ids.len(), count, "two live trees issued the same address");
}
