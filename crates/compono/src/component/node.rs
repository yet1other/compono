use crate::tree::Tree;
use crate::{Component, Cx, NodeId, Phase, StaleHandle};
use alloc::boxed::Box;
use core::any::Any;

/// The type-erased view of a component that the tree stores.
pub(crate) trait Node {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn accept(&mut self, msg: Box<dyn Any>, tree: &mut Tree, me: NodeId)
    -> Result<(), StaleHandle>;
    fn phase_changed(&mut self, phase: Phase, tree: &mut Tree, me: NodeId);
    fn wants_back(&self) -> bool;
    fn run_back(&mut self, tree: &mut Tree, me: NodeId);
}

impl<C: Component> Node for C {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    /// Delivers a queued message, or refuses one meant for another type (4-2).
    ///
    /// `Cx::send` checks the type before it queues, and a slot's generation
    /// changes before it holds another component, so the refusal is only a
    /// guard. It never panics.
    fn accept(
        &mut self,
        msg: Box<dyn Any>,
        tree: &mut Tree,
        me: NodeId,
    ) -> Result<(), StaleHandle> {
        let msg = msg.downcast::<C::Msg>().map_err(|_other| StaleHandle)?;
        let mut cx = Cx::new(tree, me);
        self.update(*msg, &mut cx);
        cx.finish();
        Ok(())
    }

    fn phase_changed(&mut self, phase: Phase, tree: &mut Tree, me: NodeId) {
        let mut cx = Cx::new(tree, me);
        Component::phase_changed(self, phase, &mut cx);
        cx.finish();
    }

    fn wants_back(&self) -> bool {
        self.back().is_some()
    }

    fn run_back(&mut self, tree: &mut Tree, me: NodeId) {
        if let Some(msg) = self.back() {
            let mut cx = Cx::new(tree, me);
            self.update(msg, &mut cx);
            cx.finish();
        }
    }
}
