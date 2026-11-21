use core::fmt;
use core::marker::PhantomData;
use core::num::NonZeroU64;

/// Untyped address of a live or former node.
///
/// It names the tree that issued it, the slot in that tree and the slot's
/// generation, so it matches nothing in any other tree and nothing that later
/// takes its slot. It is 8 bytes, and `Option<NodeId>` is as small, because no
/// address is zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(NonZeroU64);

/// Bits of an address that hold the slot's index.
const INDEX_BITS: u32 = 28;
/// Bits of an address that hold the slot's generation.
const GENERATION_BITS: u32 = 24;
const INDEX_MASK: u64 = (1 << INDEX_BITS) - 1;
const GENERATION_MASK: u64 = (1 << GENERATION_BITS) - 1;

/// The first slot index no address can name. A tree holds fewer components.
pub(crate) const INDEX_LIMIT: u32 = (1 << INDEX_BITS) - 1;
/// The generation at which a slot retires. No address is issued with it.
pub(crate) const GENERATION_LIMIT: u32 = (1 << GENERATION_BITS) - 1;

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeId")
            .field("tree", &self.tree())
            .field("index", &self.index())
            .field("generation", &self.generation())
            .finish()
    }
}

impl NodeId {
    /// An id no component ever has: it belongs to no tree.
    pub(crate) const NONE: Self = Self(NonZeroU64::MAX);

    /// The id of slot `index` at `generation` in the tree `tree`.
    ///
    /// Anything an address cannot hold, such as the retired generation or the
    /// identity of no tree, gives [`NodeId::NONE`].
    pub(crate) const fn new(tree: u32, index: u32, generation: u32) -> Self {
        if tree == 0
            || tree >= crate::tree::identity::NONE
            || index >= INDEX_LIMIT
            || generation >= GENERATION_LIMIT
        {
            return Self::NONE;
        }
        let bits = ((tree as u64) << (INDEX_BITS + GENERATION_BITS))
            | ((generation as u64) << INDEX_BITS)
            | index as u64;
        match NonZeroU64::new(bits) {
            Some(bits) => Self(bits),
            None => Self::NONE,
        }
    }

    /// The identity of the tree that issued this id.
    pub(crate) const fn tree(self) -> u32 {
        (self.0.get() >> (INDEX_BITS + GENERATION_BITS)) as u32
    }

    /// The slot this id names in its tree.
    pub(crate) const fn index(self) -> u32 {
        (self.0.get() & INDEX_MASK) as u32
    }

    /// The generation of the slot this id was issued for.
    pub(crate) const fn generation(self) -> u32 {
        ((self.0.get() >> INDEX_BITS) & GENERATION_MASK) as u32
    }

    /// Packs the id into one integer that can cross a language boundary directly.
    ///
    /// The integer holds the identity of the tree, so bits from one tree name
    /// nothing in another. It never returns `0` or `u64::MAX`, and `u64::MAX`
    /// (`-1` as an `i64`) can stand for no component.
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0.get()
    }

    /// Rebuilds an id from [`NodeId::to_bits`].
    ///
    /// Bits that `to_bits` never returned, such as `0` or `u64::MAX`, give an
    /// id that matches no component.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        match NonZeroU64::new(bits) {
            Some(bits) => Self(bits),
            None => Self::NONE,
        }
    }
}

// The architecture promises 8-byte addresses, which cross language boundaries as
// one `u64`. Growing either type must be a deliberate change.
const _: () = assert!(core::mem::size_of::<NodeId>() == 8);
const _: () = assert!(core::mem::size_of::<Handle<()>>() == 8);
// A missing address costs nothing more, as in each link of a tree slot.
const _: () = assert!(core::mem::size_of::<Option<NodeId>>() == 8);
const _: () = assert!(core::mem::size_of::<Option<Handle<()>>>() == 8);
// The three parts fill the 64 bits exactly.
const _: () = assert!(INDEX_BITS + GENERATION_BITS + crate::tree::identity::BITS == 64);

/// Typed address of a component. It stays safe to hold after the component is gone.
pub struct Handle<C> {
    id: NodeId,
    marker: PhantomData<fn() -> C>,
}

impl<C> Handle<C> {
    pub(crate) const fn new(id: NodeId) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    /// Returns the untyped address.
    #[must_use]
    pub const fn id(self) -> NodeId {
        self.id
    }
}

impl<C> Clone for Handle<C> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<C> Copy for Handle<C> {}
impl<C> PartialEq for Handle<C> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<C> Eq for Handle<C> {}
impl<C> core::hash::Hash for Handle<C> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl<C> fmt::Debug for Handle<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Handle").field(&self.id).finish()
    }
}

/// The addressed component no longer exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaleHandle;

impl fmt::Display for StaleHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("component no longer exists")
    }
}
impl core::error::Error for StaleHandle {}

/// A message went neither to the sending component nor to one of its live
/// children, so it was not queued.
///
/// [`Cx::send`](crate::Cx::send) reaches only those, so that a child cannot
/// message its parent even when it holds the parent's handle. A grandchild is
/// reached through its own parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotOwnChild;

impl fmt::Display for NotOwnChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the target is neither this component nor one of its children")
    }
}
impl core::error::Error for NotOwnChild {}

#[cfg(test)]
mod tests {
    use super::{GENERATION_LIMIT, INDEX_LIMIT, NodeId};

    #[test]
    fn bits_round_trip_for_every_part_issued() {
        for (tree, index, generation) in [
            (1, 0, 0),
            (7, 5, 1 << 20),
            (4094, INDEX_LIMIT - 1, GENERATION_LIMIT - 1),
        ] {
            let id = NodeId::new(tree, index, generation);
            assert_ne!(id, NodeId::NONE);
            assert_eq!(NodeId::from_bits(id.to_bits()), id);
            assert_eq!(
                (id.tree(), id.index(), id.generation()),
                (tree, index, generation)
            );
            assert_ne!(id.to_bits(), u64::MAX);
        }
    }

    #[test]
    fn the_same_slot_in_two_trees_has_two_addresses() {
        assert_ne!(NodeId::new(1, 0, 0), NodeId::new(2, 0, 0));
    }

    #[test]
    fn parts_an_address_cannot_hold_give_no_component() {
        assert_eq!(NodeId::from_bits(u64::MAX), NodeId::NONE);
        assert_eq!(NodeId::from_bits(0), NodeId::NONE);
        assert_eq!(NodeId::new(0, 1, 1), NodeId::NONE);
        assert_eq!(NodeId::new(4095, 1, 1), NodeId::NONE);
        assert_eq!(NodeId::new(1, INDEX_LIMIT, 0), NodeId::NONE);
        assert_eq!(NodeId::new(1, 0, GENERATION_LIMIT), NodeId::NONE);
    }
}
