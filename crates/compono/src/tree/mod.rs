use crate::component::node::Node;
use crate::saved_key;
use crate::{Component, Cx, Handle, Host, NodeId, Phase, StaleHandle};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;

/// Delivers a child's output to its parent. Shared, so that outputs still queued
/// when the child is removed can carry it (4-3).
pub(crate) type Route = alloc::rc::Rc<dyn Fn(&mut dyn Any, Box<dyn Any>, &mut Tree, NodeId)>;

/// The FNV-1a offset basis, where every hash starts.
pub(crate) const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Continues an FNV-1a hash over `bytes`. It is the same on every launch,
/// platform and build.
pub(crate) const fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut index = 0;
    while index < bytes.len() {
        // The loop bound keeps the index in range.
        #[allow(clippy::indexing_slicing, reason = "bounded by the loop condition")]
        let byte = bytes[index];
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
        index += 1;
    }
    hash
}

pub(crate) enum Work {
    Msg {
        to: NodeId,
        payload: Box<dyn Any>,
    },
    Output {
        from: NodeId,
        payload: Box<dyn Any>,
    },
    /// An output of a child removed before it was delivered, still on its way to
    /// the parent.
    Orphan {
        parent: NodeId,
        route: Route,
        payload: Box<dyn Any>,
    },
}

mod back;
mod execution;
pub(crate) mod identity;
mod lifecycle;
mod links;
mod node_slot;
mod routers;

pub(crate) use node_slot::NodeSlot;
use node_slot::OpenScope;

/// The single owner of all components.
///
/// A tree lives on the thread that created it, as a React tree does. Other
/// threads reach it through [`Sender`](crate::Sender).
pub struct Tree {
    /// Unique among live trees, and part of every address this tree issues.
    identity: u32,
    pub(crate) slots: Vec<NodeSlot>,
    free: Vec<u32>,
    work: VecDeque<Work>,
    pub(crate) work_limit: usize,
    /// Set while an operation runs. Still set between operations means one
    /// unwound, and the tree is poisoned.
    running: bool,
    /// Entries of dropped scopes, shared with every scope the tree made.
    graveyard: crate::router::scope::Graveyard,
    buried: Vec<(NodeId, NodeId)>,
    dirty: Vec<NodeId>,
    pub(crate) root: Option<NodeId>,
    root_phase: Phase,
    can_back: bool,
    /// The target of the predictive Back gesture in progress.
    gesture: Option<NodeId>,
    scope: Option<OpenScope>,
    created: Vec<NodeId>,
    /// For each component, the bases of the router names it has given out.
    pub(crate) router_names: hashbrown::HashMap<NodeId, Vec<u64>>,
    /// The parent and key of every live named child, so a repeated name is found
    /// without walking the siblings (3-1). It grows as a table does, by doubling,
    /// not by one allocation per child.
    named_children: hashbrown::HashSet<(NodeId, u64)>,
    /// The component whose router [`Cx::replace_router`] is destroying. Its
    /// entries' saved state stays for the router that replaces it.
    pub(crate) keeping_saved: Option<NodeId>,
    host: Box<dyn Host>,
    #[cfg(feature = "std")]
    pub(crate) inbox: alloc::sync::Arc<crate::concurrency::sender::Inbox>,
    #[cfg(feature = "std")]
    pub(crate) inbox_batch: VecDeque<crate::concurrency::sender::Envelope>,
    #[cfg(feature = "std")]
    pub(crate) spawner: Option<Box<dyn crate::Spawner>>,
    #[cfg(feature = "std")]
    pub(crate) woken: alloc::sync::Arc<crate::concurrency::local::Woken>,
    #[cfg(feature = "std")]
    pub(crate) woken_batch: Vec<(NodeId, u64)>,
    #[cfg(feature = "std")]
    pub(crate) poll_limit: usize,
    #[cfg(feature = "std")]
    pub(crate) next_task: u64,
    #[cfg(feature = "serde")]
    pub(crate) restored: alloc::collections::BTreeMap<Vec<u64>, crate::persist::SavedEntry>,
}

impl Drop for Tree {
    /// Removes components as navigation does: children first, each told it is hidden.
    ///
    /// A poisoned tree tells no component anything, because that is user code
    /// running on half-changed state. Its tasks still stop before the components
    /// they belong to are dropped.
    fn drop(&mut self) {
        // Everything goes now, so a scope dropped on the way asks for nothing.
        self.graveyard.set_operating(true);
        if self.running {
            #[cfg(feature = "std")]
            for slot in &mut self.slots {
                drop(core::mem::take(&mut slot.tasks));
                drop(core::mem::take(&mut slot.local));
            }
        } else if let Some(root) = self.root.take() {
            self.destroy(root);
        }
        #[cfg(feature = "std")]
        self.inbox.close();
        identity::release(self.identity);
    }
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree")
            .field(
                "nodes",
                &self.slots.iter().filter(|slot| slot.node.is_some()).count(),
            )
            .field("root_phase", &self.root_phase)
            .finish_non_exhaustive()
    }
}

impl Tree {
    /// Creates an empty tree bound to `host`.
    ///
    /// Up to 4 094 trees can be alive at once in a process, because every
    /// address names its tree. A tree made while all of them are taken has no
    /// address of its own: it is poisoned from the start
    /// ([`Tree::is_poisoned`] returns `true`), so it runs nothing and every call
    /// on it says so. Dropping other trees frees identities for later trees.
    #[must_use]
    pub fn new(host: impl Host + 'static) -> Self {
        #[cfg(feature = "std")]
        let waker = host.waker();
        let identity = identity::claim();
        Self {
            identity,
            slots: Vec::new(),
            free: Vec::new(),
            work: VecDeque::new(),
            work_limit: execution::DEFAULT_WORK_LIMIT,
            // A tree with no identity could address nothing it builds.
            running: identity == identity::NONE,
            graveyard: crate::router::scope::Grave::new(host.waker()),
            buried: Vec::new(),
            dirty: Vec::new(),
            root: None,
            root_phase: Phase::Hidden,
            can_back: false,
            gesture: None,
            scope: None,
            created: Vec::new(),
            router_names: hashbrown::HashMap::new(),
            named_children: hashbrown::HashSet::new(),
            keeping_saved: None,
            host: Box::new(host),
            #[cfg(feature = "std")]
            woken: crate::concurrency::local::Woken::new(waker.clone()),
            #[cfg(feature = "std")]
            woken_batch: Vec::new(),
            #[cfg(feature = "std")]
            poll_limit: crate::concurrency::local::DEFAULT_POLL_LIMIT,
            #[cfg(feature = "std")]
            inbox: crate::concurrency::sender::Inbox::new(waker),
            #[cfg(feature = "std")]
            inbox_batch: VecDeque::new(),
            #[cfg(feature = "std")]
            spawner: None,
            #[cfg(feature = "std")]
            next_task: 0,
            #[cfg(feature = "serde")]
            restored: alloc::collections::BTreeMap::new(),
        }
    }

    /// Creates the root component, replacing any previous one.
    pub fn mount<C: Component>(&mut self, init: impl FnOnce(&mut Cx<'_, C>) -> C) -> Handle<C> {
        self.operate(Handle::new(NodeId::NONE), |tree| tree.mount_now(init))
    }

    pub(crate) fn mount_now<C: Component>(
        &mut self,
        init: impl FnOnce(&mut Cx<'_, C>) -> C,
    ) -> Handle<C> {
        if let Some(previous) = self.root.take() {
            self.destroy(previous);
        }
        let id = self.allocate(None, None, None);
        self.root = Some(id);
        let handle = self.build(id, init);
        self.assign(id, self.root_phase);
        self.finish();
        handle
    }

    /// Returns whether an operation on this tree unwound before it finished, or
    /// whether the tree was made with no identity left (see [`Tree::new`]).
    ///
    /// A panic in a component that something caught leaves the tree half-changed:
    /// the component that panicked is out of its slot and its queue is half done.
    /// From then on the tree runs no user code and commits nothing. `send`
    /// returns [`StaleHandle`], `back` returns `false`, `pump` returns
    /// [`QueueClosed`](crate::QueueClosed), `save` returns
    /// [`SaveError::Poisoned`](crate::SaveError::Poisoned) without encoding
    /// anything, and `mount` returns a handle that reads as gone. Reads still
    /// work, and dropping the tree still removes its components, without telling
    /// them they are hidden. Applications that use `panic = "abort"` never see
    /// this.
    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.running
    }

    /// Runs one external operation, or gives `refused` on a poisoned tree.
    ///
    /// The mark stays set if `run` unwinds, which is what poisons the tree.
    pub(crate) fn operate<R>(&mut self, refused: R, run: impl FnOnce(&mut Self) -> R) -> R {
        if self.running {
            return refused;
        }
        self.running = true;
        self.graveyard.set_operating(true);
        let value = run(self);
        self.graveyard.set_operating(false);
        self.running = false;
        value
    }

    /// Returns the parent of a component, for hosts that map components to views.
    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.slot(id)?.parent
    }

    /// Returns whether a component still exists.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.slot(id).is_some_and(|slot| slot.node.is_some())
    }

    /// Rebuilds a typed handle from an address that crossed a language boundary.
    ///
    /// Returns `None` if the component is gone or is not a `C`.
    #[must_use]
    pub fn handle_of<C: Component>(&self, id: NodeId) -> Option<Handle<C>> {
        self.slot(id)?
            .node
            .as_ref()?
            .as_any()
            .is::<C>()
            .then(|| Handle::new(id))
    }

    /// Reads a component without copying it.
    #[must_use]
    pub fn get<C: Component>(&self, handle: Handle<C>) -> Option<&C> {
        self.slot(handle.id())?
            .node
            .as_ref()?
            .as_any()
            .downcast_ref()
    }

    /// Delivers one message and everything it causes, then commits once.
    ///
    /// Everything it causes is done within [`Tree::set_work_limit`]. Past the
    /// limit, `send` still returns `Ok(())`, which means the message was
    /// delivered: the work left waits in order, the host is woken once, and the
    /// next [`Tree::pump`](crate::Tree::pump) or other operation goes on with it.
    ///
    /// # Errors
    ///
    /// Returns [`StaleHandle`] if the component no longer exists, belongs to
    /// another tree or is not a `C`, and then runs nothing.
    pub fn send<C: Component>(
        &mut self,
        handle: Handle<C>,
        msg: C::Msg,
    ) -> Result<(), StaleHandle> {
        self.operate(Err(StaleHandle), |tree| tree.send_now(handle, msg))
    }

    fn send_now<C: Component>(
        &mut self,
        handle: Handle<C>,
        msg: C::Msg,
    ) -> Result<(), StaleHandle> {
        let id = handle.id();
        if self.handle_of::<C>(id).is_none() {
            return Err(StaleHandle);
        }
        let delivered = self.with_node(id, |node, tree| {
            if let Some(value) = node.as_any_mut().downcast_mut::<C>() {
                let mut cx = Cx::new(tree, id);
                value.update(msg, &mut cx);
                cx.finish();
            }
        });
        if !delivered {
            return Err(StaleHandle);
        }
        self.finish();
        Ok(())
    }

    /// Sets how present the whole application is.
    pub fn set_phase(&mut self, phase: Phase) {
        self.operate((), |tree| tree.set_phase_now(phase));
    }

    fn set_phase_now(&mut self, phase: Phase) {
        self.root_phase = phase;
        if let Some(root) = self.root {
            self.assign(root, phase);
        }
        self.finish();
    }

    /// Returns the effective phase of a component.
    #[must_use]
    pub fn phase<C>(&self, handle: Handle<C>) -> Option<Phase> {
        self.node_phase(handle.id())
    }

    /// Returns the effective phase of a component known only by its address.
    #[must_use]
    pub fn node_phase(&self, id: NodeId) -> Option<Phase> {
        self.slot(id).map(|slot| slot.effective)
    }

    /// The slot `id` names, if this tree issued `id` and the slot still holds
    /// the same generation.
    pub(crate) fn slot(&self, id: NodeId) -> Option<&NodeSlot> {
        if id.tree() != self.identity {
            return None;
        }
        let index = usize::try_from(id.index()).ok()?;
        self.slots
            .get(index)
            .filter(|slot| slot.generation == id.generation())
    }

    pub(crate) fn slot_mut(&mut self, id: NodeId) -> Option<&mut NodeSlot> {
        if id.tree() != self.identity {
            return None;
        }
        let index = usize::try_from(id.index()).ok()?;
        self.slots
            .get_mut(index)
            .filter(|slot| slot.generation == id.generation())
    }

    pub(crate) fn phase_of(&self, id: NodeId) -> Phase {
        self.slot(id).map_or(Phase::Hidden, |slot| slot.effective)
    }

    pub(crate) fn enqueue(&mut self, work: Work) {
        self.work.push_back(work);
    }

    pub(crate) fn mark_dirty(&mut self, id: NodeId) {
        if let Some(slot) = self.slot_mut(id)
            && !slot.dirty
        {
            slot.dirty = true;
            self.dirty.push(id);
        }
    }

    /// Tells the host about something that works but saves nothing.
    pub(crate) fn diagnose(&mut self, diagnostic: &crate::Diagnostic<'_>) {
        self.host.diagnostic(diagnostic);
    }

    pub(crate) fn allocate(
        &mut self,
        parent: Option<NodeId>,
        route: Option<Route>,
        name: Option<&str>,
    ) -> NodeId {
        let (key, child) = self.new_child_key(parent, name);
        let in_scope = child.is_some();
        let (index, generation) = self.claim_index();
        let slot = NodeSlot::new(generation, parent, route, key, child);
        let position = usize::try_from(index).unwrap_or(usize::MAX);
        match self.slots.get_mut(position) {
            Some(existing) => *existing = slot,
            None => self.slots.push(slot),
        }
        let id = NodeId::new(self.identity, index, generation);
        if let Some(parent) = parent {
            self.link_child(parent, id);
            if !in_scope {
                self.link_visible(parent, id);
            }
        }
        if in_scope {
            self.created.push(id);
        }
        id
    }

    /// Chooses the saved key of a new child of `parent`, and its place within
    /// a router entry.
    ///
    /// Inside a scope `parent` opened, the child takes the entry's key and the
    /// next place in it. Otherwise a named child takes its name's key, unless a
    /// live sibling has it, and any other child the next number.
    fn new_child_key(&mut self, parent: Option<NodeId>, name: Option<&str>) -> (u64, Option<u32>) {
        let scope = self
            .scope
            .as_mut()
            .filter(|scope| parent.is_some() && Some(scope.owner) == parent);
        if let Some(scope) = scope {
            let child = scope.next_child;
            scope.next_child = child.wrapping_add(1);
            return (scope.base, Some(child));
        }
        if let (Some(name), Some(parent)) = (name, parent) {
            let key = saved_key::named_child(name);
            if self.named_children.insert((parent, key)) {
                return (key, None);
            }
            let owner = parent;
            self.diagnose(&crate::Diagnostic::RepeatedChildName { owner, name });
            return (saved_key::UNSAVED, None);
        }
        let key = parent.and_then(|id| self.slot_mut(id)).map_or(0, |slot| {
            let number = slot.next_auto_key;
            slot.next_auto_key = number.saturating_add(1);
            saved_key::numbered_child(number)
        });
        (key, None)
    }

    fn claim_index(&mut self) -> (u32, u32) {
        if let Some(index) = self.free.pop() {
            let position = usize::try_from(index).unwrap_or(usize::MAX);
            let generation = self.slots.get(position).map_or(0, |slot| slot.generation);
            return (index, generation);
        }
        (u32::try_from(self.slots.len()).unwrap_or(u32::MAX), 0)
    }

    pub(crate) fn build<K: Component>(
        &mut self,
        id: NodeId,
        init: impl FnOnce(&mut Cx<'_, K>) -> K,
    ) -> Handle<K> {
        let mut cx = Cx::new(self, id);
        let value = init(&mut cx);
        self.place_node(id, Box::new(value));
        Handle::new(id)
    }

    pub(crate) fn with_node(
        &mut self,
        id: NodeId,
        run: impl FnOnce(&mut dyn Node, &mut Self),
    ) -> bool {
        // The value leaves its slot, so nothing can alias it while it runs.
        let Some(mut node) = self.slot_mut(id).and_then(|slot| slot.node.take()) else {
            return false;
        };
        run(node.as_mut(), self);
        self.place_node(id, node);
        true
    }

    /// Puts a component in its slot and records whether it now handles Back.
    fn place_node(&mut self, id: NodeId, node: Box<dyn Node>) {
        let wants_back = node.wants_back();
        if let Some(slot) = self.slot_mut(id) {
            slot.node = Some(node);
        }
        self.change_back(id, |slot| slot.own_back = wants_back);
    }

    /// Hides, then removes a node. Children go first, newest first.
    ///
    /// The walk uses the tree's own links and no call stack, so its depth is not
    /// limited.
    pub(crate) fn destroy(&mut self, id: NodeId) {
        let mut unlimited = usize::MAX;
        self.destroy_within(id, &mut unlimited);
    }

    /// Hides a node, then removes it and everything below it, one unit of
    /// `budget` per component. Returns whether the node is gone.
    ///
    /// A node the budget stops halfway stays hidden with what is left below
    /// it, and calling this again removes the rest.
    pub(crate) fn destroy_within(&mut self, id: NodeId, budget: &mut usize) -> bool {
        #[cfg(feature = "serde")]
        if self
            .keeping_saved
            .is_none_or(|owner| self.parent(id) != Some(owner))
        {
            self.forget_subtree(id);
        }
        // Everything below is hidden with it, children before parents.
        self.assign(id, Phase::Hidden);
        let mut cursor = id;
        loop {
            if let Some(child) = self.slot(cursor).and_then(|slot| slot.last_child) {
                cursor = child;
                continue;
            }
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            let parent = self.parent(cursor);
            self.remove_leaf(cursor);
            if cursor == id {
                return true;
            }
            let Some(parent) = parent else {
                return true;
            };
            cursor = parent;
        }
    }

    /// Hands the outputs `child` queued and its route over to `parent`, in place,
    /// so they arrive in the order they were emitted although `child` is gone.
    fn readdress_outputs(&mut self, child: NodeId, parent: NodeId, route: &Route) {
        for work in &mut self.work {
            if let Work::Output { from, payload } = work
                && *from == child
            {
                // A box of `()` does not allocate.
                let payload = core::mem::replace(payload, Box::new(()));
                *work = Work::Orphan {
                    parent,
                    route: alloc::rc::Rc::clone(route),
                    payload,
                };
            }
        }
    }

    /// Frees the slot of a hidden node that has no children left.
    fn remove_leaf(&mut self, id: NodeId) {
        let parent = self.parent(id);
        let Some(slot) = self.slot_mut(id) else {
            return;
        };
        let node = slot.node.take();
        let route = slot.route.take();
        let queued_outputs = core::mem::take(&mut slot.queued_outputs);
        let key = slot.key;
        let (prev, next) = (slot.prev_sibling.take(), slot.next_sibling.take());
        #[cfg(feature = "std")]
        let tasks = (
            core::mem::take(&mut slot.tasks),
            core::mem::take(&mut slot.local),
        );
        slot.generation = slot.generation.saturating_add(1);
        // No id is ever made with the last generation, so a slot that reaches it
        // matches nothing and is never reused. Old ids stay stale for good.
        if slot.generation >= crate::component::handle::GENERATION_LIMIT {
            slot.parent = None;
        } else {
            self.free.push(id.index());
        }
        if let Some(parent) = parent {
            self.unlink_child(parent, prev, next);
            if saved_key::is_named_child(key) {
                self.named_children.remove(&(parent, key));
            }
            if queued_outputs > 0
                && let Some(route) = &route
            {
                self.readdress_outputs(id, parent, route);
            }
        }
        if !self.router_names.is_empty() {
            self.router_names.remove(&id);
        }
        // Tasks stop first, so nothing still running can observe the dropped value.
        #[cfg(feature = "std")]
        drop(tasks);
        drop(node);
        drop(route);
    }
}

#[cfg(test)]
mod tests {
    use super::Tree;
    use crate::component::handle::GENERATION_LIMIT;
    use crate::{Commit, Component, Cx, Host, NodeId};

    struct NullHost;
    impl Host for NullHost {
        fn commit(&mut self, _commit: &Commit<'_>) {}
    }

    struct Unit;
    impl Component for Unit {
        type Msg = ();
        type Output = ();
        fn update(&mut self, (): (), _cx: &mut Cx<'_, Self>) {}
    }

    #[test]
    fn a_numbered_child_past_the_limit_saves_nothing() {
        let mut tree = Tree::new(NullHost);
        let root = tree.mount(|_cx| Unit).id();
        if let Some(slot) = tree.slot_mut(root) {
            slot.next_auto_key = crate::saved_key::NUMBERED_CHILD_LIMIT;
        }
        let child = tree.allocate(Some(root), None, None);
        let key = tree.slot(child).map(|slot| slot.key);
        assert_eq!(key, Some(crate::saved_key::UNSAVED));
    }

    #[test]
    fn a_slot_that_runs_out_of_generations_never_matches_its_first_id() {
        let mut tree = Tree::new(NullHost);
        let first = tree.mount(|_cx| Unit).id();
        assert_eq!((first.index(), first.generation()), (0, 0));
        // Stands in for sixteen million reuses of the slot.
        let last = GENERATION_LIMIT - 1;
        if let Some(slot) = tree.slots.first_mut() {
            slot.generation = last;
        }
        tree.root = Some(NodeId::new(tree.identity, 0, last));
        for _ in 0..3 {
            tree.mount(|_cx| Unit);
        }
        assert_eq!(tree.node_phase(first), None);
        // The retired generation cannot even be named for this slot.
        let forged = NodeId::new(tree.identity, 0, GENERATION_LIMIT);
        assert!(!tree.contains(forged));
    }

    #[test]
    fn no_id_a_tree_gives_packs_to_all_ones() {
        let mut tree = Tree::new(NullHost);
        let mut ids = alloc::vec![tree.mount(|_cx| Unit).id()];
        // The slot's last usable generation, then its retirement.
        let last = GENERATION_LIMIT - 1;
        if let Some(slot) = tree.slots.first_mut() {
            slot.generation = last;
        }
        tree.root = Some(NodeId::new(tree.identity, 0, last));
        ids.extend((0..3).map(|_| tree.mount(|_cx| Unit).id()));
        // The promise rests on no id having the last generation.
        assert!(ids.iter().all(|id| id.generation() < GENERATION_LIMIT));
        assert!(ids.iter().all(|id| id.to_bits() != u64::MAX));
    }

    /// A component of another type than `Unit`.
    struct Other(u8);
    impl Component for Other {
        type Msg = u8;
        type Output = ();
        fn update(&mut self, msg: u8, _cx: &mut Cx<'_, Self>) {
            self.0 = msg;
        }
    }

    /// Sends to its child through a handle of the wrong type, as 4-2 describes.
    struct Sender {
        child: crate::Handle<Unit>,
        result: Option<Result<(), crate::NotOwnChild>>,
    }
    impl Component for Sender {
        type Msg = ();
        type Output = ();
        fn update(&mut self, (): (), cx: &mut Cx<'_, Self>) {
            let mistyped = crate::Handle::<Other>::new(self.child.id());
            self.result = Some(cx.send(mistyped, 7));
        }
    }

    #[test]
    fn a_handle_of_the_wrong_type_is_stale_for_tree_send() {
        let mut tree = Tree::new(NullHost);
        let unit = tree.mount(|_cx| Unit);
        let mistyped = crate::Handle::<Other>::new(unit.id());
        assert_eq!(tree.send(mistyped, 7), Err(crate::StaleHandle));
        assert!(tree.get(mistyped).is_none());
        assert!(!tree.is_poisoned());
        assert_eq!(tree.send(unit, ()), Ok(()));
    }

    #[test]
    fn a_handle_of_the_wrong_type_is_refused_by_cx_send() {
        let mut tree = Tree::new(NullHost);
        let root = tree.mount(|cx| Sender {
            child: cx.child(|_cx| Unit, |_parent, (), _cx| {}),
            result: None,
        });
        assert_eq!(tree.send(root, ()), Ok(()));
        assert!(!tree.is_poisoned());
        let result = tree.get(root).and_then(|sender| sender.result);
        assert_eq!(result, Some(Err(crate::NotOwnChild)));
    }
}
