//! The identity of each tree, which every address it issues carries (4-1).
//!
//! Identities live in a bitmap of the ones in use. A new tree claims the next
//! free bit after a shared counter, and a dropped tree clears its bit. So two
//! trees alive at the same time never share an identity, however many trees the
//! process has made, and an identity is reused only after the counter has gone
//! round every other one.
//!
//! Only the read-modify-write of one word decides who owns a bit, and those are
//! totally ordered on each word whatever the `Ordering`. No other memory is
//! published through a bit, so every operation is `Relaxed`. The bitmap needs
//! nothing but 32-bit atomics, which `thumbv7em-none-eabihf` and every `std`
//! target have.

use core::sync::atomic::{AtomicU32, Ordering};

/// How many bits of an address hold the identity of its tree.
pub(crate) const BITS: u32 = 12;
/// The identity of no tree: [`NodeId::NONE`](crate::NodeId) has it, and a tree
/// made while every other identity is in use takes it and addresses nothing.
pub(crate) const NONE: u32 = (1 << BITS) - 1;
const WORDS: usize = (1 << BITS) / 32;

static IN_USE: [AtomicU32; WORDS] = [const { AtomicU32::new(0) }; WORDS];
static NEXT: AtomicU32 = AtomicU32::new(1);

fn word_and_bit(identity: u32) -> Option<(&'static AtomicU32, u32)> {
    let word = IN_USE.get(usize::try_from(identity / 32).ok()?)?;
    Some((word, 1 << (identity % 32)))
}

/// Claims an identity that no live tree has, or [`NONE`] when all are in use.
pub(crate) fn claim() -> u32 {
    for _ in 0..(1_u32 << BITS) {
        // `1 << BITS` divides 2³², so the sequence stays round when `NEXT` wraps.
        let candidate = NEXT.fetch_add(1, Ordering::Relaxed) % (1 << BITS);
        // 0 keeps a niche in every address, and `NONE` belongs to no tree.
        if candidate == 0 || candidate == NONE {
            continue;
        }
        if let Some((word, bit)) = word_and_bit(candidate)
            && word.fetch_or(bit, Ordering::Relaxed) & bit == 0
        {
            return candidate;
        }
    }
    NONE
}

/// Gives back an identity from [`claim`], once its tree is dropped.
pub(crate) fn release(identity: u32) {
    if identity == NONE {
        return;
    }
    if let Some((word, bit)) = word_and_bit(identity) {
        word.fetch_and(!bit, Ordering::Relaxed);
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::{NONE, claim, release};

    #[test]
    fn identities_held_at_once_are_distinct() {
        let held: alloc::vec::Vec<u32> = (0..100).map(|_| claim()).collect();
        let mut sorted = held.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), held.len());
        assert!(
            held.iter()
                .all(|identity| *identity != 0 && *identity != NONE)
        );
        held.into_iter().for_each(release);
    }
}
