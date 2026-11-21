use crate::tree::Tree;
use crate::{Component, Cx};
use alloc::boxed::Box;
use core::any::Any;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Names a task started with [`Cx::perform`], so it can be cancelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskId(u64);

impl TaskId {
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Stops its task when dropped.
pub struct TaskGuard {
    _cancel_on_drop: Box<dyn Any>,
}

impl TaskGuard {
    /// Wraps whatever value cancels the task on drop.
    pub fn new(cancel_on_drop: impl Any) -> Self {
        Self {
            _cancel_on_drop: Box::new(cancel_on_drop),
        }
    }
}

impl fmt::Debug for TaskGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TaskGuard")
    }
}

/// Runs background work, so the core stays free of any executor.
pub trait Spawner {
    /// Starts `task`. Dropping the returned guard must cancel it.
    fn spawn(&self, task: Pin<Box<dyn Future<Output = ()> + Send>>) -> TaskGuard;
}

/// No [`Spawner`] was installed on the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoSpawner;

impl fmt::Display for NoSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tree has no spawner")
    }
}
impl core::error::Error for NoSpawner {}

impl Tree {
    /// Installs the executor bridge used by [`Cx::perform`].
    pub fn set_spawner(&mut self, spawner: impl Spawner + 'static) {
        self.spawner = Some(Box::new(spawner));
    }
}

impl<C: Component> Cx<'_, C> {
    /// Stops a task of this component and discards any result it already produced.
    ///
    /// Returns whether the task was still outstanding. This is the counterpart
    /// of a React effect cleanup: cancel the old request before starting a new one.
    pub fn cancel(&mut self, task: TaskId) -> bool {
        let Some(slot) = self.tree.slot_mut(self.me) else {
            return false;
        };
        if let Some(position) = slot.tasks.iter().position(|(id, _)| *id == task.0) {
            drop(slot.tasks.swap_remove(position));
            return true;
        }
        if let Some(position) = slot.local.iter().position(|local| local.id() == task.0) {
            drop(slot.local.swap_remove(position));
            return true;
        }
        false
    }
}

/// Runs a future and gives `None` if a poll panics, instead of unwinding further.
///
/// Built only on `std::panic::catch_unwind`, so it adds no dependency. Under
/// `panic = "abort"` it is a plain forwarding future.
struct CatchUnwind<F>(F);

impl<F: Future + Unpin> Future for CatchUnwind<F> {
    type Output = Option<F::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let future = Pin::new(&mut self.0);
        match catch_unwind(AssertUnwindSafe(|| future.poll(cx))) {
            Ok(Poll::Ready(value)) => Poll::Ready(Some(value)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_panic) => Poll::Ready(None),
        }
    }
}

/// Reports the end of a task once, however it ends (2-2).
///
/// It lives inside the task's future. A task that finishes reports its message, a
/// task that panics reports that it failed, and a task that its executor drops
/// before it finishes, as Tokio does at shutdown, reports from `drop` that it
/// failed. Each report wakes the host once, and the pump releases the guard.
struct Report<C: Component>
where
    C::Msg: Send,
{
    sender: Option<crate::Sender<C>>,
    id: u64,
}

impl<C: Component> Report<C>
where
    C::Msg: Send,
{
    fn finish(mut self, msg: Option<C::Msg>) {
        if let Some(sender) = self.sender.take() {
            let _closed = match msg {
                Some(msg) => sender.push(Some(Box::new(msg)), Some(self.id)),
                None => sender.fail(self.id),
            };
        }
    }
}

impl<C: Component> Drop for Report<C>
where
    C::Msg: Send,
{
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            // A tree already dropped, or a task already cancelled, ignores it.
            let _closed = sender.fail(self.id);
        }
    }
}

impl<C: Component> Cx<'_, C>
where
    C::Msg: Send,
{
    /// Runs `future` in the background and delivers its result as a message.
    ///
    /// The task is cancelled when this component is destroyed, and a late result
    /// is discarded, so a finished request can never reach a removed component.
    ///
    /// If `future` or `to_msg` panics, the panic stops at the task: its guard is
    /// released at the next [`Tree::pump`], and the component receives nothing
    /// from it. Builds with `panic = "abort"` stop the process instead. A task
    /// that the executor drops before it finishes, as at the shutdown of a
    /// runtime, ends the same way: the host is woken once, the next pump releases
    /// its guard, and [`Cx::cancel`] then returns `false` for it. A task cancelled
    /// by the component may wake the host once more, for nothing.
    ///
    /// # Errors
    ///
    /// Returns [`NoSpawner`] if the tree has no executor bridge.
    pub fn perform<T>(
        &mut self,
        future: impl Future<Output = T> + Send + 'static,
        to_msg: impl FnOnce(T) -> C::Msg + Send + 'static,
    ) -> Result<TaskId, NoSpawner> {
        if self.tree.spawner.is_none() {
            return Err(NoSpawner);
        }
        let id = self.tree.next_task;
        self.tree.next_task = id.wrapping_add(1);
        let report = Report::<C> {
            sender: Some(self.sender()),
            id,
        };
        let task = Box::pin(async move {
            let report = report;
            let work = core::pin::pin!(async move { to_msg(future.await) });
            // A task that panicked still reports, so its guard is released.
            let msg = CatchUnwind(work).await;
            report.finish(msg);
        });
        let guard = self.tree.spawner.as_ref().ok_or(NoSpawner)?.spawn(task);
        if let Some(slot) = self.tree.slot_mut(self.me) {
            slot.tasks.push((id, guard));
        }
        Ok(TaskId(id))
    }
}
