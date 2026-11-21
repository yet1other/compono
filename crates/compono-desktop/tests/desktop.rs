//! Saved-state files and Back shortcuts.

#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono_desktop::{Convention, Key, Modifiers, StateFile};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("compono-desktop-{}-{name}", std::process::id()))
        .join("state.bin")
}

#[test]
fn a_missing_file_is_nothing_saved() {
    let file = StateFile::new(scratch("missing"));
    assert_eq!(file.load().unwrap(), None);
    file.clear().unwrap();
}

#[test]
fn store_replaces_and_creates_the_directory() {
    let path = scratch("store");
    let file = StateFile::new(&path);
    file.store(b"first").unwrap();
    file.store(b"second").unwrap();
    assert_eq!(file.load().unwrap().as_deref(), Some(&b"second"[..]));
    assert!(
        leftovers(path.parent().unwrap()).is_empty(),
        "the temporary file is renamed away"
    );
    file.clear().unwrap();
    assert_eq!(file.load().unwrap(), None);
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}

#[test]
fn an_app_dir_ends_in_the_app_name() {
    if let Ok(file) = StateFile::in_app_dir("compono-test") {
        assert!(file.path().ends_with("compono-test/state.bin"));
    }
}

fn with(f: impl FnOnce(&mut Modifiers)) -> Modifiers {
    let mut modifiers = Modifiers::default();
    f(&mut modifiers);
    modifiers
}

#[test]
fn every_system_takes_escape_and_back_buttons() {
    for convention in [Convention::Mac, Convention::Other] {
        assert!(convention.is_back(Key::Escape, Modifiers::default()));
        assert!(!convention.is_back(Key::Escape, with(|m| m.ctrl = true)));
        assert!(convention.is_back(Key::MouseBack, Modifiers::default()));
        assert!(convention.is_back(Key::BrowserBack, Modifiers::default()));
        assert!(!convention.is_back(Key::ArrowLeft, Modifiers::default()));
        assert!(!convention.is_back(Key::Other, Modifiers::default()));
    }
}

#[test]
fn mac_uses_command_and_others_use_alt() {
    let logo = with(|m| m.logo = true);
    let alt = with(|m| m.alt = true);
    assert!(Convention::Mac.is_back(Key::BracketLeft, logo));
    assert!(Convention::Mac.is_back(Key::ArrowLeft, logo));
    assert!(!Convention::Mac.is_back(Key::ArrowLeft, alt));
    assert!(Convention::Other.is_back(Key::ArrowLeft, alt));
    assert!(!Convention::Other.is_back(Key::ArrowLeft, logo));
    assert!(!Convention::Other.is_back(Key::BracketLeft, logo));
    assert!(!Convention::Mac.is_back(
        Key::BracketLeft,
        with(|m| {
            m.logo = true;
            m.shift = true;
        })
    ));
}

#[test]
fn a_failed_store_leaves_no_temporary_file() {
    let path = scratch("failed");
    // A directory with something in it cannot be replaced by a file.
    std::fs::create_dir_all(path.join("occupied")).unwrap();
    let file = StateFile::new(&path);
    assert!(file.store(b"state").is_err());
    assert!(leftovers(path.parent().unwrap()).is_empty());
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

/// Names in `dir` other than the state file itself.
fn leftovers(dir: &std::path::Path) -> Vec<std::ffi::OsString> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "state.bin")
        .collect()
}

#[test]
fn concurrent_stores_never_leave_a_torn_file() {
    // 5-1: two writers, each saving its own value, as two copies of an
    // application or a save on a background thread at exit do.
    let path = scratch("concurrent");
    let dir = path.parent().unwrap().to_path_buf();
    let values: [Vec<u8>; 2] = [vec![b'a'; 256 * 1024], vec![b'b'; 192 * 1024]];
    for round in 0..200 {
        std::thread::scope(|scope| {
            for value in &values {
                let file = StateFile::new(&path);
                scope.spawn(move || {
                    // Either may fail when the other renames at the same moment on
                    // some systems; neither may tear the file.
                    let _stored = file.store(value);
                });
            }
        });
        let read = StateFile::new(&path).load().unwrap().unwrap();
        assert!(
            values.contains(&read),
            "round {round}: a torn file of {} bytes",
            read.len()
        );
    }
    assert_eq!(leftovers(&dir), Vec::<std::ffi::OsString>::new());
    StateFile::new(&path).clear().unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

#[test]
fn a_poisoned_tree_leaves_the_last_state_file_as_it_was() {
    // 2-1: the save at close of a poisoned tree must not replace good state.
    use compono::{Commit, Component, Cx, Host, Tree};
    struct Quiet;
    impl Host for Quiet {
        fn commit(&mut self, _commit: &Commit<'_>) {}
    }
    struct Bomb;
    impl Component for Bomb {
        type Msg = ();
        type Output = ();
        #[expect(clippy::panic, reason = "the test needs a component that panics")]
        fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {
            panic!("the component failed");
        }
    }

    let path = scratch("poisoned");
    let file = StateFile::new(&path);
    file.store(b"last good state").unwrap();
    let mut tree = Tree::new(Quiet);
    let bomb = tree.mount(|_cx| Bomb);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tree.send(bomb, ())));
    assert!(caught.is_err());
    // What every platform crate and showcase does at close.
    if let Ok(bytes) = tree.save(1) {
        file.store(&bytes).unwrap();
    }
    assert_eq!(
        file.load().unwrap().as_deref(),
        Some(&b"last good state"[..])
    );
    file.clear().unwrap();
    std::fs::remove_dir(path.parent().unwrap()).unwrap();
}
