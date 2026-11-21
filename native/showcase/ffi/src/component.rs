//! What every exported component shares: a weak link to the tree and its address.

use boltffi::{data, error};
use compono::{Component, NodeId, Tree};
use compono_ffi::{NativeError, NativeTree, phase_code};
use std::sync::{Arc, Weak};

/// One child of a router.
#[data]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ChildRef {
    /// Identifies this entry for as long as it exists.
    pub key: i64,
    /// The component's address.
    pub node: i64,
    /// 0 for hidden, 1 for visible and 2 for focused.
    pub phase: i32,
}

/// Why a call into the tree did nothing, as Swift throws it and Kotlin raises it.
///
/// It names the kind of [`NativeError`]. A missing component is normal use and is
/// not an error: those calls give nothing instead.
#[error]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// Called from a thread that does not own the tree.
    WrongThread,
    /// Called from inside another call on the tree.
    Busy,
    /// The tree's state could not be saved.
    Save,
    /// The tree's message queue is closed.
    QueueClosed,
    /// An earlier call panicked, so the tree runs nothing more.
    Poisoned,
    /// A phase code other than 0, 1 or 2.
    UnknownPhase,
    /// Another failure of the tree.
    Other,
}

impl From<NativeError> for TreeError {
    fn from(error: NativeError) -> Self {
        match error {
            NativeError::WrongThread => Self::WrongThread,
            NativeError::Busy => Self::Busy,
            NativeError::Save => Self::Save,
            NativeError::QueueClosed => Self::QueueClosed,
            NativeError::Poisoned => Self::Poisoned,
            _ => Self::Other,
        }
    }
}

/// Returns the value of a tree call, `Ok(None)` when the tree or component is
/// gone, and the error otherwise, for a method that throws it (6-3).
///
/// Nothing here can panic, because it runs inside an `extern "C"` function.
pub(crate) fn throwing<T>(result: Result<T, NativeError>) -> Result<Option<T>, TreeError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(NativeError::Gone) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Returns the value of a tree call, or `None` when it did nothing.
///
/// A component object's methods answer with nothing when their component is
/// gone, as a native UI meets in normal use. A misuse, such as a call from
/// another thread, also gives nothing here, and the root's methods, which every
/// UI calls on each change, throw it.
pub(crate) fn checked<T>(result: Result<T, NativeError>) -> Option<T> {
    result.ok()
}

/// The tree, held weakly so that a component object never keeps it alive.
///
/// A component object that outlives its application answers with nothing.
#[derive(Clone, Debug)]
pub(crate) struct Ctx(Weak<NativeTree>);

impl Ctx {
    pub(crate) fn new(tree: &Arc<NativeTree>) -> Self {
        Self(Arc::downgrade(tree))
    }

    pub(crate) fn tree<R>(&self, read: impl FnOnce(&Tree) -> Option<R>) -> Option<R> {
        checked(self.0.upgrade()?.read(read)).flatten()
    }

    pub(crate) fn read<C: Component, R>(&self, node: u64, read: impl FnOnce(&C) -> R) -> Option<R> {
        self.tree(|tree| {
            let handle = tree.handle_of::<C>(NodeId::from_bits(node))?;
            tree.get(handle).map(read)
        })
    }

    pub(crate) fn send<C: Component>(&self, node: u64, msg: C::Msg) {
        if let Some(tree) = self.0.upgrade() {
            checked(tree.send::<C>(node, msg));
        }
    }

    pub(crate) fn child(&self, key: u64, node: NodeId) -> Option<ChildRef> {
        self.tree(|tree| {
            let phase = tree.node_phase(node)?;
            Some(ChildRef {
                key: key.cast_signed(),
                node: node.to_bits().cast_signed(),
                phase: i32::from(phase_code(phase)),
            })
        })
    }
}

pub(crate) const fn address(child: ChildRef) -> u64 {
    child.node.cast_unsigned()
}

pub(crate) fn to_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

pub(crate) fn to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
