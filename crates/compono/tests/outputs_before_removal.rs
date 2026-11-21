//! 4-3: an output a child emits before it is removed still reaches its parent.

#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::{Commit, Component, Cx, Handle, Host, Phase, Tree};

struct NullHost;
impl Host for NullHost {
    fn commit(&mut self, _commit: &Commit<'_>) {}
}

#[derive(Clone, Debug, PartialEq)]
enum Note {
    Close,
    Saved(u32),
}

/// Asks to be closed and hands over its text in the same update, as a "save and
/// close" button does. It also saves when it is hidden.
struct Editor(u32);

impl Component for Editor {
    type Msg = ();
    type Output = Note;
    fn update(&mut self, (): (), cx: &mut Cx<'_, Self>) {
        cx.emit(Note::Close);
        cx.emit(Note::Saved(self.0));
    }
    fn phase_changed(&mut self, phase: Phase, cx: &mut Cx<'_, Self>) {
        if phase == Phase::Hidden {
            cx.emit(Note::Saved(self.0 + 1));
        }
    }
}

/// Removes the editor as soon as it asks to be closed.
struct Notes {
    editor: Option<Handle<Editor>>,
    received: Vec<Note>,
}

impl Component for Notes {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

fn notes(cx: &mut Cx<'_, Notes>) -> Notes {
    Notes {
        editor: Some(cx.child(
            |_cx| Editor(7),
            |notes: &mut Notes, note, cx| {
                if note == Note::Close
                    && let Some(editor) = notes.editor.take()
                {
                    cx.destroy(editor).unwrap();
                }
                notes.received.push(note);
            },
        )),
        received: Vec::new(),
    }
}

#[test]
fn an_output_emitted_before_removal_reaches_the_parent_in_order() {
    let mut tree = Tree::new(NullHost);
    tree.set_phase(Phase::Focused);
    let root = tree.mount(notes);
    let editor = tree.get(root).unwrap().editor.unwrap();
    tree.send(editor, ()).unwrap();
    assert!(tree.get(editor).is_none(), "the editor was removed");
    let notes = tree.get(root).unwrap();
    // `Close` removed the editor, which saved on being hidden, after `Saved(7)`
    // had already been emitted.
    assert_eq!(
        notes.received,
        [Note::Close, Note::Saved(7), Note::Saved(8)],
        "outputs were lost or reordered"
    );
}

#[test]
fn an_output_whose_parent_is_also_removed_is_dropped_quietly() {
    let mut tree = Tree::new(NullHost);
    tree.set_phase(Phase::Focused);
    let root = tree.mount(notes);
    let editor = tree.get(root).unwrap().editor.unwrap();
    // Replacing the root removes the notes and their editor, which emits while
    // it is hidden. Nobody is left to receive it.
    tree.mount(|_cx| Editor(0));
    assert!(tree.get(root).is_none());
    assert!(tree.get(editor).is_none());
    assert!(!tree.is_poisoned());
}
