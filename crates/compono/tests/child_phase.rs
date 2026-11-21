//! 4-5: the caller learns the phase a child takes, which its parent's limits.

#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::{Commit, Component, Cx, Handle, Host, NotOwnChild, Phase, Tree};

struct NullHost;
impl Host for NullHost {
    fn commit(&mut self, _commit: &Commit<'_>) {}
}

struct Leaf;
impl Component for Leaf {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

struct Parent {
    leaf: Handle<Leaf>,
    applied: Option<Result<Phase, NotOwnChild>>,
}
impl Component for Parent {
    type Msg = Phase;
    type Output = ();
    fn update(&mut self, phase: Phase, cx: &mut Cx<'_, Self>) {
        self.applied = Some(cx.set_child_phase(self.leaf, phase));
    }
}

#[test]
fn a_child_phase_above_the_parent_reports_the_phase_applied() {
    let mut tree = Tree::new(NullHost);
    tree.set_phase(Phase::Visible);
    let parent = tree.mount(|cx| Parent {
        leaf: cx.child(|_cx| Leaf, |_parent, (), _cx| {}),
        applied: None,
    });
    tree.send(parent, Phase::Focused).unwrap();
    assert_eq!(tree.get(parent).unwrap().applied, Some(Ok(Phase::Visible)));
    let leaf = tree.get(parent).unwrap().leaf;
    assert_eq!(tree.phase(leaf), Some(Phase::Visible));

    // The grant is kept: when the parent rises, the child follows it up.
    tree.set_phase(Phase::Focused);
    assert_eq!(tree.phase(leaf), Some(Phase::Focused));
    tree.send(parent, Phase::Hidden).unwrap();
    assert_eq!(tree.get(parent).unwrap().applied, Some(Ok(Phase::Hidden)));
}
