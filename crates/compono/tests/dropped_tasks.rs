//! 2-2: a task that the executor drops before it finishes still reports.

#![cfg(feature = "std")]
#![expect(clippy::unwrap_used, reason = "tests assert by failing")]

use compono::{Commit, Component, Cx, Host, Spawner, TaskGuard, TaskId, Tree};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};

type Task = Pin<Box<dyn Future<Output = ()> + Send>>;

/// An executor that keeps its tasks until it shuts down, as Tokio does.
#[derive(Clone, Default)]
struct Executor(Arc<Mutex<Vec<Task>>>);

impl Executor {
    /// Drops every task without finishing it.
    fn shut_down(&self) {
        drop(std::mem::take(&mut *self.0.lock().unwrap()));
    }
}

impl Spawner for Executor {
    fn spawn(&self, task: Task) -> TaskGuard {
        self.0.lock().unwrap().push(task);
        // Cancelling does nothing here; the executor decides when tasks go.
        TaskGuard::new(())
    }
}

#[derive(Default)]
struct Wakes(AtomicUsize);
impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct Counting(Arc<Wakes>);
impl Host for Counting {
    fn commit(&mut self, _commit: &Commit<'_>) {}
    fn waker(&self) -> Waker {
        Waker::from(Arc::clone(&self.0))
    }
}

enum Msg {
    Start,
    Done,
    /// Asks which tasks are still outstanding, through `Cx::cancel`.
    Check,
}

#[derive(Default)]
struct Loader {
    tasks: Vec<TaskId>,
    outstanding: Vec<bool>,
}

impl Component for Loader {
    type Msg = Msg;
    type Output = ();
    fn update(&mut self, msg: Msg, cx: &mut Cx<'_, Self>) {
        match msg {
            Msg::Start => {
                for _ in 0..4 {
                    let id = cx.perform(std::future::pending::<()>(), |()| Msg::Done);
                    self.tasks.push(id.unwrap());
                }
            }
            Msg::Done => {}
            Msg::Check => {
                self.outstanding = self.tasks.iter().map(|task| cx.cancel(*task)).collect();
            }
        }
    }
}

#[test]
fn a_task_dropped_by_its_executor_is_no_longer_outstanding() {
    let wakes = Arc::new(Wakes::default());
    let executor = Executor::default();
    let mut tree = Tree::new(Counting(Arc::clone(&wakes)));
    tree.set_spawner(executor.clone());
    let loader = tree.mount(|_cx| Loader::default());
    tree.send(loader, Msg::Start).unwrap();
    let before = wakes.0.load(Ordering::SeqCst);

    executor.shut_down();
    assert_eq!(
        wakes.0.load(Ordering::SeqCst) - before,
        4,
        "the host is woken once for each task that stopped"
    );
    tree.pump().unwrap();
    tree.send(loader, Msg::Check).unwrap();
    assert_eq!(tree.get(loader).unwrap().outstanding, [false; 4]);
}
