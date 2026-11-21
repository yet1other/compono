//! The showcase as SwiftUI and Compose see it.
//!
//! [`ShowcaseApp`] owns the tree and has the standard methods the Swift package and
//! the Compose library call. Every screen is a component object, as in Decompose:
//! `RootComponent` gives `TabsComponent`, which gives `CountersComponent`, and so on.
//! Each has a `model` and its actions as methods. Component objects hold the tree
//! weakly and answer with nothing once their component or the application is gone.

mod component;
mod feature;

use boltffi::export;
pub use component::TreeError;
use component::{Ctx, checked, throwing};
use compono::Handle;
use compono_ffi::{NO_NODE, NativeTree, Platform, phase_from_code};
use compono_tokio::TokioSpawner;
use feature::root::{RootComponent, root_component};
use showcase::Services;
use showcase::feature::root::Root;
use std::sync::{Arc, OnceLock};
use tokio::runtime::{Builder, Runtime};

boltffi::scaffolding!();

/// Bumped when saved state stops being readable.
const SCHEMA: u32 = 3;

/// What the native side receives from the tree.
#[export]
pub trait Listener: Send + Sync {
    /// One finished change. `dirty` holds the addresses of changed components.
    fn commit(&self, dirty: Vec<i64>, can_back: bool);
    /// Asks for `ShowcaseApp::pump` on the main thread. Called from any thread.
    fn schedule_pump(&self);
}

struct Forward(Arc<dyn Listener>);

impl Platform for Forward {
    fn commit(&self, dirty: &[u64], can_back: bool) {
        self.0
            .commit(dirty.iter().map(|id| id.cast_signed()).collect(), can_back);
    }

    fn schedule_pump(&self) {
        self.0.schedule_pump();
    }
}

fn runtime() -> Option<&'static Runtime> {
    static RUNTIME: OnceLock<Option<Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(1)
                .enable_time()
                .build()
                .ok()
        })
        .as_ref()
}

/// The application's tree. Create it and call it on the main thread.
#[export]
pub struct ShowcaseApp {
    tree: Arc<NativeTree>,
    root: Option<Handle<Root>>,
}

#[export]
impl ShowcaseApp {
    /// Builds the tree on this thread. `saved` is empty on a first start.
    pub fn new(listener: Arc<dyn Listener>, saved: Vec<u8>) -> Self {
        let platform: Arc<dyn Platform> = Arc::new(Forward(listener));
        let tree = if saved.is_empty() {
            NativeTree::new(platform)
        } else {
            // Unreadable state starts the application from the beginning.
            NativeTree::restore(platform, &saved, SCHEMA).0
        };
        let root = checked(tree.with(|tree| {
            if let Some(runtime) = runtime() {
                tree.set_spawner(TokioSpawner::new(runtime.handle().clone()));
            }
            // The timer is created inside the future, where the runtime is current.
            let services =
                Services::new(|delay| Box::pin(async move { tokio::time::sleep(delay).await }));
            tree.mount_persistent(move |cx| Root::new(cx, services))
        }));
        Self {
            tree: Arc::new(tree),
            root,
        }
    }

    /// Frees trees whose `ShowcaseApp` was released on another thread. Call it on
    /// the main thread, at every pump (6-4).
    pub fn collect_retired() {
        compono_ffi::collect_retired();
    }

    /// The root component.
    pub fn root(&self) -> RootComponent {
        root_component(Ctx::new(&self.tree), self.root)
    }

    /// Delivers work that arrived from other threads.
    ///
    /// # Errors
    ///
    /// Throws a [`TreeError`] when the tree refuses the call.
    pub fn pump(&self) -> Result<(), TreeError> {
        throwing(self.tree.pump()).map(drop)
    }

    /// Sets the phase: 0 for hidden, 1 for visible and 2 for focused.
    ///
    /// # Errors
    ///
    /// Throws [`TreeError::UnknownPhase`] for any other code, or the tree's error.
    pub fn set_phase(&self, code: i32) -> Result<(), TreeError> {
        let phase = u8::try_from(code)
            .ok()
            .and_then(phase_from_code)
            .ok_or(TreeError::UnknownPhase)?;
        throwing(self.tree.set_phase(phase)).map(drop)
    }

    /// Delivers a Back press and returns whether it was handled.
    ///
    /// # Errors
    ///
    /// Throws a [`TreeError`] when the tree refuses the call.
    pub fn back(&self) -> Result<bool, TreeError> {
        Ok(throwing(self.tree.back())?.unwrap_or(false))
    }

    /// Starts a Back gesture and returns its target's address, or `NO_NODE`.
    ///
    /// # Errors
    ///
    /// Throws a [`TreeError`] when the tree refuses the call.
    pub fn back_start(&self) -> Result<i64, TreeError> {
        Ok(throwing(self.tree.back_start())?
            .flatten()
            .map_or(NO_NODE, u64::cast_signed))
    }

    /// Abandons a Back gesture.
    ///
    /// # Errors
    ///
    /// Throws a [`TreeError`] when the tree refuses the call.
    pub fn back_cancel(&self) -> Result<(), TreeError> {
        throwing(self.tree.back_cancel()).map(drop)
    }

    /// Completes a Back gesture and returns whether Back was handled.
    ///
    /// # Errors
    ///
    /// Throws a [`TreeError`] when the tree refuses the call.
    pub fn back_commit(&self) -> Result<bool, TreeError> {
        Ok(throwing(self.tree.back_commit())?.unwrap_or(false))
    }

    /// Encodes navigation and persistent state.
    ///
    /// # Errors
    ///
    /// Throws a [`TreeError`], such as [`TreeError::Poisoned`], when nothing could
    /// be saved. The caller keeps the state it saved before.
    pub fn save(&self) -> Result<Vec<u8>, TreeError> {
        Ok(throwing(self.tree.save(SCHEMA))?.unwrap_or_default())
    }

    /// Returns whether the component at `node` still exists.
    ///
    /// # Errors
    ///
    /// Throws a [`TreeError`] when the tree refuses the call.
    pub fn contains(&self, node: i64) -> Result<bool, TreeError> {
        let node = compono::NodeId::from_bits(node.cast_unsigned());
        Ok(throwing(self.tree.read(|tree| tree.contains(node)))?.unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::{Listener, ShowcaseApp, TreeError};
    use std::sync::Arc;

    struct Silent;
    impl Listener for Silent {
        fn commit(&self, _dirty: Vec<i64>, _can_back: bool) {}
        fn schedule_pump(&self) {}
    }

    fn app() -> ShowcaseApp {
        ShowcaseApp::new(Arc::new(Silent), Vec::new())
    }

    #[test]
    fn a_call_from_another_thread_throws_its_kind() {
        // 6-3: the error reaches the native side, in every build, without a panic.
        let app = app();
        let elsewhere = std::thread::scope(|scope| scope.spawn(|| app.back()).join());
        assert_eq!(elsewhere.ok(), Some(Err(TreeError::WrongThread)));
        let elsewhere = std::thread::scope(|scope| scope.spawn(|| app.save()).join());
        assert_eq!(elsewhere.ok(), Some(Err(TreeError::WrongThread)));
    }

    #[test]
    fn an_unknown_phase_code_throws() {
        assert_eq!(app().set_phase(7), Err(TreeError::UnknownPhase));
    }

    #[test]
    fn a_child_of_another_kind_gives_no_component() {
        let app = app();
        let root = app.root();
        let tabs = root.active().unwrap();
        assert!(root.book(tabs).is_none());
        assert!(root.tabs(tabs).is_some());
    }

    #[test]
    fn no_back_handler_is_reported_as_no_node() {
        let app = app();
        app.set_phase(2).unwrap();
        assert_eq!(app.back_start(), Ok(compono_ffi::NO_NODE));
    }

    #[test]
    fn a_call_on_the_owning_thread_works() {
        let app = app();
        app.set_phase(2).unwrap();
        app.pump().unwrap();
        assert_eq!(app.back(), Ok(false), "the root screen handles no Back");
        assert!(!app.save().unwrap().is_empty());
    }
}

#[cfg(test)]
mod kinds {
    use super::{Listener, ShowcaseApp};
    use crate::component::{ChildRef, Ctx};
    use crate::feature::articles::{ArticlesComponent, PanelChild, PanelKind};
    use crate::feature::root::{ScreenChild, ScreenKind};
    use crate::feature::tabs::{TabKind, TabsChild, TabsComponent};
    use std::sync::Arc;

    struct Silent;
    impl Listener for Silent {
        fn commit(&self, _dirty: Vec<i64>, _can_back: bool) {}
        fn schedule_pump(&self) {}
    }

    const CHILD: ChildRef = ChildRef {
        key: 0,
        node: 0,
        phase: 0,
    };

    #[test]
    fn a_screen_accessor_gives_nothing_for_another_kind() {
        let app = ShowcaseApp::new(Arc::new(Silent), Vec::new());
        let root = app.root();
        for kind in [ScreenKind::Tabs, ScreenKind::Book, ScreenKind::Gallery] {
            let child = ScreenChild { kind, child: CHILD };
            assert_eq!(root.tabs(child).is_some(), kind == ScreenKind::Tabs);
            assert_eq!(root.book(child).is_some(), kind == ScreenKind::Book);
            assert_eq!(root.gallery(child).is_some(), kind == ScreenKind::Gallery);
        }
    }

    #[test]
    fn a_tab_accessor_gives_nothing_for_another_kind() {
        let app = ShowcaseApp::new(Arc::new(Silent), Vec::new());
        let tabs = TabsComponent {
            ctx: Ctx::new(&app.tree),
            node: 0,
        };
        let all = [
            TabKind::Menu,
            TabKind::Counters,
            TabKind::Cards,
            TabKind::Articles,
            TabKind::Feed,
        ];
        for tab in all {
            let child = TabsChild { tab, child: CHILD };
            assert_eq!(tabs.menu(child).is_some(), tab == TabKind::Menu);
            assert_eq!(tabs.counters(child).is_some(), tab == TabKind::Counters);
            assert_eq!(tabs.cards(child).is_some(), tab == TabKind::Cards);
            assert_eq!(tabs.articles(child).is_some(), tab == TabKind::Articles);
            assert_eq!(tabs.feed(child).is_some(), tab == TabKind::Feed);
        }
    }

    #[test]
    fn a_panel_accessor_gives_nothing_for_another_kind() {
        let app = ShowcaseApp::new(Arc::new(Silent), Vec::new());
        let articles = ArticlesComponent {
            ctx: Ctx::new(&app.tree),
            node: 0,
        };
        for kind in [PanelKind::Main, PanelKind::Details, PanelKind::Extra] {
            let panel = PanelChild { kind, child: CHILD };
            assert_eq!(articles.list(panel).is_some(), kind == PanelKind::Main);
            assert_eq!(
                articles.details(panel).is_some(),
                kind == PanelKind::Details
            );
            assert_eq!(articles.author(panel).is_some(), kind == PanelKind::Extra);
        }
    }
}
