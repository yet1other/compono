//! 4-4: a component never restores bytes that a component of another type saved.

#![cfg(feature = "serde")]
#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::{Commit, Component, Cx, Host, Persist, RestoreOutcome, Tree};

struct NullHost;
impl Host for NullHost {
    fn commit(&mut self, _commit: &Commit<'_>) {}
}

struct Volume(u32);
impl Component for Volume {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}
impl Persist for Volume {
    type Saved = u32;
    const KIND: &'static str = "settings.volume";
    fn save(&self) -> u32 {
        self.0
    }
}

struct Pin(u32);
impl Component for Pin {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}
impl Persist for Pin {
    type Saved = u32;
    const KIND: &'static str = "settings.pin";
    fn save(&self) -> u32 {
        self.0
    }
}

/// Builds its two children in either order.
struct Settings {
    volume: compono::Handle<Volume>,
    pin: compono::Handle<Pin>,
}
impl Component for Settings {
    type Msg = ();
    type Output = ();
    fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
}

fn settings(volume_first: bool) -> impl FnOnce(&mut Cx<'_, Settings>) -> Settings {
    move |cx| {
        let volume = |cx: &mut Cx<'_, Settings>| {
            cx.persistent_child(|cx| Volume(cx.restored().unwrap_or(1)), |_s, (), _cx| {})
        };
        let pin = |cx: &mut Cx<'_, Settings>| {
            cx.persistent_child(|cx| Pin(cx.restored().unwrap_or(2)), |_s, (), _cx| {})
        };
        if volume_first {
            let volume = volume(cx);
            Settings {
                volume,
                pin: pin(cx),
            }
        } else {
            let pin = pin(cx);
            Settings {
                volume: volume(cx),
                pin,
            }
        }
    }
}

#[test]
fn children_built_in_another_order_start_fresh() {
    let mut tree = Tree::new(NullHost);
    tree.mount(|cx| {
        let volume = cx.persistent_child(|_cx| Volume(4242), |_s, (), _cx| {});
        let pin = cx.persistent_child(|_cx| Pin(11), |_s, (), _cx| {});
        Settings { volume, pin }
    });
    let bytes = tree.save(1).unwrap();

    let (mut tree, outcome) = Tree::with_saved(NullHost, &bytes, 1);
    assert_eq!(outcome, RestoreOutcome::Restored);
    let root = tree.mount(settings(false));
    let settings = tree.get(root).unwrap();
    let volume = tree.get(settings.volume).unwrap().0;
    let pin = tree.get(settings.pin).unwrap().0;
    assert_eq!((volume, pin), (1, 2), "volume={volume} pin={pin}");
}
